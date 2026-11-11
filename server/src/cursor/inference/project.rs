//! Projects client-owned model calls into the provider-independent request.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{Map, Value};

use crate::{
    cursor::protocol::{
        json::{prost_json, prost_struct},
        proto::inference as pb,
    },
    model::{
        parse_token_count, ContentPart, ModelRequest, ModelSpec, ProjectedContent,
        ProjectedMessage, PromptSpec, ProviderReplayState, Role, ToolCallContent, ToolDefinition,
        ToolResultContent,
    },
    Error, Result,
};

#[cfg(test)]
mod context_tests {
    use super::*;

    #[test]
    fn context_selection_is_preserved_for_plugin_and_builtin_models() {
        for model_id in ["plugin:test:model", "builtin-model"] {
            let mut input = pb::InferenceStreamRequest::default();
            assert_eq!(
                project(&input, &ModelSpec::new(model_id))
                    .unwrap()
                    .model
                    .context_window_tokens,
                None
            );
            input.requested_model = Some(pb::InferenceRequestedModel {
                model_id: model_id.into(),
                parameters: vec![pb::InferenceModelParameterValue {
                    id: "context".into(),
                    value: "356k".into(),
                }],
                ..Default::default()
            });
            assert_eq!(
                project(&input, &ModelSpec::new(model_id))
                    .unwrap()
                    .model
                    .context_window_tokens,
                Some(356_000)
            );
            input.requested_model.as_mut().unwrap().parameters[0].value = "invalid".into();
            assert!(project(&input, &ModelSpec::new(model_id)).is_err());
        }
    }
}

pub(crate) fn selected_context(model: Option<&pb::InferenceRequestedModel>) -> Result<Option<u64>> {
    model
        .into_iter()
        .flat_map(|model| &model.parameters)
        .filter(|parameter| parameter.id == "context")
        .try_fold(None, |_, parameter| {
            parse_token_count(&parameter.value)
                .map(Some)
                .ok_or_else(|| {
                    Error::Protocol(format!(
                        "invalid Cursor context token count: {}",
                        parameter.value
                    ))
                })
        })
}

pub(crate) fn apply_selection(
    model: &mut ModelSpec,
    selection: Option<&pb::InferenceRequestedModel>,
) -> Result<()> {
    if let Some(selection) = selection {
        // max_mode is a non-optional protobuf bool: false cannot distinguish omission.
        if selection.max_mode {
            model.reasoning.enabled = true;
        }
        for parameter in &selection.parameters {
            crate::cursor::compile::apply_model_parameter(model, &parameter.id, &parameter.value)?;
        }
    }
    Ok(())
}

pub(crate) fn project(
    input: &pb::InferenceStreamRequest,
    selection: &ModelSpec,
) -> Result<ModelRequest> {
    let mut model = selection.clone();
    if let Some(config) = &input.model_config {
        model.max_output_tokens = config
            .max_tokens
            .and_then(|value| u64::try_from(value).ok());
        let mut extra = Map::new();
        if let Some(value) = config.temperature {
            extra.insert("temperature".into(), Value::from(value));
        }
        if let Some(value) = config.top_p {
            extra.insert("top_p".into(), Value::from(value));
        }
        if !config.stop_sequences.is_empty() {
            extra.insert("stop".into(), Value::from(config.stop_sequences.clone()));
        }
        model.extra_params = Value::Object(extra);
    }
    model.context_window_tokens =
        selected_context(input.requested_model.as_ref())?.or(model.context_window_tokens);
    apply_selection(&mut model, input.requested_model.as_ref())?;

    let tools = input
        .tools
        .iter()
        .map(|tool| {
            let parameters = tool
                .parameters
                .as_ref()
                .map(prost_struct)
                .unwrap_or_else(|| serde_json::json!({}));
            ToolDefinition {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: parameters.get("jsonSchema").cloned().unwrap_or(parameters),
            }
        })
        .collect();

    let mut instructions = Vec::new();
    let mut history = Vec::new();
    let mut system_prefix = true;
    for (index, message) in input.messages.iter().enumerate() {
        let role = match message.role {
            1 => Role::User,
            2 => Role::Assistant,
            3 => Role::Tool,
            4 => Role::System,
            value => {
                return Err(Error::Protocol(format!(
                    "RunInference message {index} has unsupported role {value}"
                )))
            }
        };
        let parts = parts(message.content.as_ref())?;
        if role == Role::System && system_prefix {
            instructions.push(text_parts(&parts));
            continue;
        }
        system_prefix = false;

        if let Some(pb::inference_core_message::Content::ToolContent(content)) =
            message.content.as_ref()
        {
            for (part_index, part) in content.parts.iter().enumerate() {
                let provider_parts = parts_from_wire(&part.experimental_content)?;
                let content = part.result.as_ref().map(prost_json).unwrap_or(Value::Null);
                history.push(ProjectedMessage {
                    message_id: format!("cursor:{index}:tool:{part_index}"),
                    role: Role::Tool,
                    content: ProjectedContent::ToolResult(ToolResultContent {
                        call_id: part.tool_call_id.clone(),
                        name: part.tool_name.clone(),
                        content: match &content {
                            Value::String(text) => text.clone(),
                            value => value.to_string(),
                        },
                        is_error: part.is_error,
                        image: None,
                        provider_parts,
                    }),
                });
            }
            continue;
        }

        let content = if role == Role::Assistant {
            let calls = message
                .tool_calls
                .iter()
                .enumerate()
                .map(|(index, call)| {
                    let arguments = match &call.raw_tool_call_args {
                        Some(raw) => serde_json::from_str(raw)?,
                        None => call
                            .args
                            .as_ref()
                            .map(prost_struct)
                            .unwrap_or_else(|| serde_json::json!({})),
                    };
                    Ok(ToolCallContent {
                        index,
                        call_id: call.tool_call_id.clone(),
                        name: call.tool_name.clone(),
                        arguments,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let thinking = message
                .reasoning_parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>();
            let blocks = message
                .reasoning_parts
                .iter()
                .filter_map(|part| {
                    if let Some(signature) = &part.signature {
                        Some(serde_json::json!({
                            "type": "thinking", "thinking": part.text, "signature": signature
                        }))
                    } else {
                        part.redacted_data.as_ref().map(
                            |data| serde_json::json!({"type": "redacted_thinking", "data": data}),
                        )
                    }
                })
                .collect::<Vec<_>>();
            let replay_state = (!blocks.is_empty()).then(|| ProviderReplayState {
                provider_kind: "anthropic".into(),
                value: serde_json::json!({"blocks": blocks}),
            });
            ProjectedContent::Assistant {
                text: text_parts(&parts),
                thinking,
                replay_state,
                calls,
            }
        } else {
            ProjectedContent::Parts(parts)
        };
        history.push(ProjectedMessage {
            message_id: format!("cursor:{index}"),
            role,
            content,
        });
    }
    Ok(ModelRequest {
        prompt: PromptSpec {
            instructions: instructions.join("\n\n"),
            tools,
        },
        model,
        history,
    })
}

fn parts(content: Option<&pb::inference_core_message::Content>) -> Result<Vec<ContentPart>> {
    match content {
        Some(pb::inference_core_message::Content::Text(text)) => {
            Ok(vec![ContentPart::Text { text: text.clone() }])
        }
        Some(pb::inference_core_message::Content::Parts(parts)) => parts_from_wire(&parts.parts),
        Some(pb::inference_core_message::Content::ToolContent(_)) | None => Ok(Vec::new()),
    }
}

fn parts_from_wire(parts: &[pb::InferenceContentPart]) -> Result<Vec<ContentPart>> {
    parts
        .iter()
        .map(|part| match part.part.as_ref() {
            Some(pb::inference_content_part::Part::Text(text)) => Ok(ContentPart::Text {
                text: text.text.clone(),
            }),
            Some(pb::inference_content_part::Part::Image(image)) => {
                let (mime_type, data) = image_data(&image.data, image.mime_type.as_deref())?;
                Ok(ContentPart::Image { mime_type, data })
            }
            Some(pb::inference_content_part::Part::File(file)) => Err(Error::Protocol(format!(
                "RunInference file content is not supported: {}",
                file.filename.as_deref().unwrap_or(&file.media_type)
            ))),
            None => Err(Error::Protocol("RunInference content part is empty".into())),
        })
        .collect()
}

fn image_data(value: &str, fallback_mime: Option<&str>) -> Result<(String, Vec<u8>)> {
    let (mime_type, encoded) = if let Some((metadata, data)) = value
        .strip_prefix("data:")
        .and_then(|value| value.split_once(","))
    {
        (metadata.trim_end_matches(";base64"), data)
    } else {
        (fallback_mime.unwrap_or("image/png"), value)
    };
    let data = STANDARD
        .decode(encoded)
        .map_err(|error| Error::Protocol(format!("invalid RunInference image: {error}")))?;
    Ok((mime_type.into(), data))
}

fn text_parts(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            ContentPart::Image { .. } => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use prost_types::{value::Kind, Struct, Value as ProtoValue};

    use super::*;

    #[test]
    fn run_selection_reaches_plugin_wire_and_invocation_can_override_it() {
        use crate::model::{ModelInvocation, ModelLatency};
        let mut run = ModelSpec::new("plugin:test:model");
        let selection = |parameters: &[(&str, &str)]| pb::InferenceRequestedModel {
            parameters: parameters
                .iter()
                .map(|(id, value)| pb::InferenceModelParameterValue {
                    id: (*id).into(),
                    value: (*value).into(),
                })
                .collect(),
            ..Default::default()
        };
        apply_selection(
            &mut run,
            Some(&selection(&[("effort", " high "), ("fast", "true")])),
        )
        .unwrap();
        run.context_window_tokens = Some(256_000);
        for requested_model in [None, Some(selection(&[]))] {
            let request = project(
                &pb::InferenceStreamRequest {
                    requested_model,
                    ..Default::default()
                },
                &run,
            )
            .unwrap();
            assert_eq!(request.model.reasoning.effort.as_deref(), Some("high"));
            assert!(request.model.reasoning.enabled);
            assert_eq!(request.model.context_window_tokens, Some(256_000));
            let wire = crate::plugin::llm_request(&ModelInvocation {
                call_id: "call".into(),
                run_id: "run".into(),
                conversation_id: "conversation".into(),
                prompt_cache_enabled: true,
                provider_call_index: 0,
                request,
            })
            .unwrap();
            assert_eq!(
                wire["reasoning"],
                serde_json::json!({"enabled": true, "effort": "high"})
            );
            assert_eq!(wire["latency"], "fast");
        }
        for (params, expected, enabled) in [
            (
                vec![("reasoning", "low"), ("fast", "false")],
                Some("low"),
                true,
            ),
            (vec![("thinking", "false"), ("fast", "false")], None, false),
            (vec![("effort", "none")], None, false),
            (vec![("effort", " ")], None, false),
            (vec![("thinking", "true")], Some("high"), true),
        ] {
            let request = project(
                &pb::InferenceStreamRequest {
                    requested_model: Some(selection(&params)),
                    ..Default::default()
                },
                &run,
            )
            .unwrap();
            assert_eq!(request.model.reasoning.effort.as_deref(), expected);
            assert_eq!(request.model.reasoning.enabled, enabled);
            assert!(request.model.reasoning.explicit);
            if params
                .iter()
                .any(|(id, value)| *id == "fast" && *value == "false")
            {
                assert_eq!(request.model.latency, ModelLatency::Standard);
            }
        }
    }

    #[test]
    fn selection_rejects_invalid_booleans_and_does_not_invent_effort() {
        let mut model = ModelSpec::new("test");
        apply_selection(
            &mut model,
            Some(&pb::InferenceRequestedModel {
                max_mode: true,
                ..Default::default()
            }),
        )
        .unwrap();
        assert!(model.reasoning.enabled);
        assert_eq!(model.reasoning.effort, None);
        for id in ["fast", "thinking"] {
            assert!(apply_selection(
                &mut model,
                Some(&pb::InferenceRequestedModel {
                    parameters: vec![pb::InferenceModelParameterValue {
                        id: id.into(),
                        value: "invalid".into()
                    }],
                    ..Default::default()
                })
            )
            .is_err());
        }
    }

    #[test]
    fn unchanged_client_history_remains_an_exact_provider_prefix() {
        let mut input = pb::InferenceStreamRequest {
            messages: vec![text(4, "You are an assistant"), text(1, "First question")],
            ..Default::default()
        };
        let first = project(&input, &ModelSpec::new("test-model")).unwrap();
        input.messages.push(text(2, "First answer"));
        input.messages.push(text(1, "Second question"));
        let second = project(&input, &ModelSpec::new("test-model")).unwrap();

        assert_eq!(first.prompt.instructions, second.prompt.instructions);
        assert_eq!(first.history, second.history[..first.history.len()]);
    }

    #[test]
    fn tool_parameters_use_the_nested_json_schema() {
        let schema = Struct {
            fields: [(
                "type".into(),
                ProtoValue {
                    kind: Some(Kind::StringValue("object".into())),
                },
            )]
            .into_iter()
            .collect(),
        };
        let input = pb::InferenceStreamRequest {
            tools: vec![pb::InferenceAgentTool {
                name: "Shell".into(),
                parameters: Some(Struct {
                    fields: [(
                        "jsonSchema".into(),
                        ProtoValue {
                            kind: Some(Kind::StructValue(schema)),
                        },
                    )]
                    .into_iter()
                    .collect(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let request = project(&input, &ModelSpec::new("test-model")).unwrap();
        assert_eq!(
            request.prompt.tools[0].parameters,
            serde_json::json!({"type":"object"})
        );
    }

    #[test]
    fn protobuf_float_integers_serialize_as_json_integers() {
        // minItems: 2.0 should become minItems: 2, not minItems: 2.0
        let schema = Struct {
            fields: [
                (
                    "type".into(),
                    ProtoValue {
                        kind: Some(Kind::StringValue("array".into())),
                    },
                ),
                (
                    "minItems".into(),
                    ProtoValue {
                        kind: Some(Kind::NumberValue(2.0)),
                    },
                ),
            ]
            .into_iter()
            .collect(),
        };

        let input = pb::InferenceStreamRequest {
            tools: vec![pb::InferenceAgentTool {
                name: "TodoWrite".into(),
                parameters: Some(schema),
                ..Default::default()
            }],
            ..Default::default()
        };

        let request = project(&input, &ModelSpec::new("test-model")).unwrap();
        let params = &request.prompt.tools[0].parameters;

        // Verify that minItems is an integer, not a float
        assert_eq!(params.get("minItems"), Some(&serde_json::json!(2)));

        // Serialize and verify the JSON doesn't contain "2.0"
        let serialized = serde_json::to_string(params).unwrap();
        assert!(serialized.contains("\"minItems\":2"));
        assert!(!serialized.contains("2.0"));
    }

    fn text(role: i32, content: &str) -> pb::InferenceCoreMessage {
        pb::InferenceCoreMessage {
            role,
            content: Some(pb::inference_core_message::Content::Text(content.into())),
            ..Default::default()
        }
    }
}
