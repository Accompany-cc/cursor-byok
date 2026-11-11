//! Owns one client-loop inference stream per Cursor request ID.
use std::{collections::HashMap, sync::Arc};

use bytes::Bytes;
use futures_util::StreamExt;
use parking_lot::Mutex;
use prost::Message;
use tokio::{
    sync::{mpsc, Mutex as AsyncMutex, Notify},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use crate::{
    cursor::{
        protocol::{connect, proto::inference as pb},
        transport::{OutputHub, TransportRegistry},
    },
    model::{ModelInvocation, ModelSpec, ModelType},
    provider::{ModelEvent, Provider},
    Error, Result,
};

use super::project;

#[derive(Clone)]
pub(crate) struct InferenceService {
    state: Arc<Mutex<HashMap<String, InferenceRoute>>>,
    changed: Arc<Notify>,
    registry: TransportRegistry,
}

#[derive(Clone)]
pub(crate) enum InferenceRoute {
    Local(Arc<Run>),
    Upstream,
}

pub(crate) struct Run {
    input: mpsc::Sender<pb::RunInferenceClientMessage>,
    output: Arc<OutputHub>,
    next_seqno: AsyncMutex<i64>,
}

impl InferenceService {
    pub fn new(registry: TransportRegistry) -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            changed: Arc::new(Notify::new()),
            registry,
        }
    }

    pub fn route(&self, request_id: &str) -> Option<InferenceRoute> {
        self.state.lock().get(request_id).cloned()
    }

    pub async fn wait_route(&self, request_id: &str) -> InferenceRoute {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(route) = self.route(request_id) {
                return route;
            }
            changed.await;
        }
    }

    pub async fn begin(
        &self,
        request_id: &str,
        message: &pb::RunInferenceClientMessage,
    ) -> Result<InferenceRoute> {
        let Some(pb::run_inference_client_message::Message::RunRequest(request)) =
            message.message.as_ref()
        else {
            return Err(Error::Protocol(
                "first RunInference message must start a run".into(),
            ));
        };
        let model_id = request
            .requested_model
            .as_ref()
            .map(|model| model.model_id.as_str())
            .filter(|model| !model.is_empty())
            .ok_or_else(|| Error::Protocol("RunInference model is required".into()))?;
        if request.conversation_id.is_empty() {
            return Err(Error::Protocol(
                "RunInference conversation ID is required".into(),
            ));
        }
        let configured = self.registry.store().model(model_id).await?;
        let context_window = super::project::selected_context(request.requested_model.as_ref())?
            .or_else(|| {
                configured
                    .as_ref()
                    .and_then(|model| model.context_window_tokens)
            });
        let mut selection = ModelSpec::new(model_id);
        selection.context_window_tokens = context_window;
        super::project::apply_selection(&mut selection, request.requested_model.as_ref())?;
        let local = configured.is_some() || model_id.starts_with(crate::plugin::ADAPTER_ID_PREFIX);
        let route = if local {
            tracing::info!(
                request_id,
                model_id,
                "routing Cursor RunInference to BYOK provider"
            );
            let (sender, receiver) = mpsc::channel(64);
            let output = Arc::new(OutputHub::default());
            let run = Arc::new(Run {
                input: sender,
                output: output.clone(),
                next_seqno: AsyncMutex::new(0),
            });
            tokio::spawn(serve(
                receiver,
                output,
                self.registry.provider(),
                request_id.into(),
                request.conversation_id.clone(),
                selection,
                configured.map(|model| (model.display_name, model.model_type)),
            ));
            InferenceRoute::Local(run)
        } else {
            tracing::info!(
                request_id,
                model_id,
                "routing Cursor RunInference to Cursor upstream"
            );
            InferenceRoute::Upstream
        };
        let mut state = self.state.lock();
        let route = state
            .entry(request_id.into())
            .or_insert_with(|| route.clone())
            .clone();
        drop(state);
        self.changed.notify_waiters();
        Ok(route)
    }

    pub fn finish(&self, request_id: &str) {
        self.state.lock().remove(request_id);
    }
}

impl Run {
    pub async fn append(&self, seqno: i64, message: pb::RunInferenceClientMessage) -> Result<()> {
        let mut next = self.next_seqno.lock().await;
        if seqno < *next {
            return Ok(());
        }
        if seqno != *next {
            return Err(Error::Protocol(format!(
                "RunInference append sequence {seqno} differs from expected {}",
                *next
            )));
        }
        self.input
            .send(message)
            .await
            .map_err(|_| Error::Protocol("RunInference stream has ended".into()))?;
        *next += 1;
        Ok(())
    }

    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<Bytes> {
        self.output.subscribe()
    }
}

async fn serve(
    mut input: mpsc::Receiver<pb::RunInferenceClientMessage>,
    output: Arc<OutputHub>,
    provider: Arc<dyn Provider>,
    run_id: String,
    conversation_id: String,
    model_selection: ModelSpec,
    model_info: Option<(String, ModelType)>,
) {
    let invocations = Arc::new(Mutex::new(HashMap::<String, CancellationToken>::new()));
    let mut tasks = JoinSet::new();
    let mut finished = false;
    let mut provider_call_index = 0;
    while let Some(message) = input.recv().await {
        use pb::run_inference_client_message::Message;
        match message.message {
            Some(Message::RunRequest(request)) => {
                let ready = run_ready(request.requested_model, model_info.as_ref());
                emit(
                    &output,
                    pb::run_inference_server_message::Message::RunReady(ready),
                );
            }
            Some(Message::InvokeModel(invoke)) => {
                let id = invoke.invocation_id.clone();
                let Some(request) = invoke.request else {
                    invocation_error(&output, id, "RunInference model request is missing");
                    continue;
                };
                if invoke.delta_base.is_some() || invoke.compressed_request.is_some() {
                    invocation_error(
                        &output,
                        id,
                        "RunInference delta or compressed requests are not supported",
                    );
                    continue;
                }
                let request = match project(&request, &model_selection) {
                    Ok(request) => request,
                    Err(error) => {
                        invocation_error(&output, id, &error.to_string());
                        continue;
                    }
                };
                let cancellation = CancellationToken::new();
                invocations.lock().insert(id.clone(), cancellation.clone());
                tasks.spawn(serve_invocation(
                    provider.clone(),
                    output.clone(),
                    invocations.clone(),
                    cancellation,
                    ModelInvocation {
                        call_id: id,
                        run_id: run_id.clone(),
                        conversation_id: conversation_id.clone(),
                        prompt_cache_enabled: true,
                        provider_call_index,
                        request,
                    },
                ));
                provider_call_index += 1;
            }
            Some(Message::CancelInvocation(cancel)) => {
                if let Some(token) = invocations.lock().remove(&cancel.invocation_id) {
                    token.cancel();
                }
            }
            Some(Message::FinishRun(finish)) => {
                finished = true;
                if finish.aborted {
                    for (_, token) in invocations.lock().drain() {
                        token.cancel();
                    }
                }
                break;
            }
            Some(
                Message::ClientHeartbeat(_)
                | Message::AgentEvent(_)
                | Message::RunRequestLog(_)
                | Message::ResolveSubagentModels(_)
                | Message::ClientFirstCallTimings(_),
            )
            | None => {}
        }
    }
    if !finished {
        for (_, token) in invocations.lock().drain() {
            token.cancel();
        }
    }
    while tasks.join_next().await.is_some() {}
    output.emit(connect::encode_end_stream());
    output.close();
}

fn run_ready(
    requested_model: Option<pb::InferenceRequestedModel>,
    model_info: Option<&(String, ModelType)>,
) -> pb::RunInferenceRunReady {
    let (display_name, model_type) = model_info
        .map(|(name, kind)| (name.clone(), *kind))
        .unwrap_or_else(|| (String::new(), ModelType::OpenAi));
    let anthropic = model_type == ModelType::Anthropic;
    let name = display_name.to_ascii_lowercase();
    let claude = anthropic && name.contains("claude");
    let gpt5 = !anthropic && name.starts_with("gpt-5");
    pb::RunInferenceRunReady {
        resolved_model: requested_model,
        routed_model_display_name: (!display_name.is_empty()).then_some(display_name),
        prompt_model_metadata: Some(pb::RunInferencePromptModelMetadata {
            vendor: if anthropic { "anthropic" } else { "openai" }.into(),
            prompt_version: "latest".into(),
            is_claude_4x: claude,
            is_sonnet4: claude && name.contains("sonnet"),
            is_gpt5_family: gpt5,
            is_gpt5: gpt5,
            persona: "auto".into(),
            self_identity_name: "Cursor".into(),
            agent_mode_configs: agent_mode_configs(),
            ask_question_config: Some(pb::RunInferenceAskQuestionConfig {
                tool_name: "AskQuestion".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        accepts_aborted_finish: true,
        ..Default::default()
    }
}

async fn serve_invocation(
    provider: Arc<dyn Provider>,
    output: Arc<OutputHub>,
    invocations: Arc<Mutex<HashMap<String, CancellationToken>>>,
    cancellation: CancellationToken,
    invocation: ModelInvocation,
) {
    let id = invocation.call_id.clone();
    let context_window = invocation.request.model.context_window_tokens;
    let mut stream = provider.stream(invocation, cancellation);
    let mut tool_calls = HashMap::<usize, (String, String, String)>::new();
    let mut pending_tools = Vec::new();
    let mut terminal = false;
    let mut error = None;
    while let Some(next) = stream.next().await {
        match next {
            Ok(event) => {
                if terminal {
                    error = Some("provider emitted an event after Done".into());
                    break;
                }
                if matches!(
                    event,
                    ModelEvent::ToolCallStart { .. }
                        | ModelEvent::ToolCallArgumentsDelta { .. }
                        | ModelEvent::ToolCallEnd { .. }
                ) {
                    pending_tools.push(event);
                    continue;
                }
                if let ModelEvent::Done(finish) = event {
                    terminal = true;
                    if let Err(message) = validate_tool_delivery(&pending_tools, finish) {
                        error = Some(message.into());
                        break;
                    }
                    if !pending_tools.is_empty() {
                        for event in pending_tools.drain(..) {
                            if let Some(response) =
                                model_response(event, &mut tool_calls, context_window)
                            {
                                emit(
                                    &output,
                                    pb::run_inference_server_message::Message::InvocationResponse(
                                        pb::RunInferenceInvocationResponse {
                                            invocation_id: id.clone(),
                                            response: Some(response),
                                        },
                                    ),
                                );
                            }
                        }
                    }
                    continue;
                }
                if let Some(response) = model_response(event, &mut tool_calls, context_window) {
                    emit(
                        &output,
                        pb::run_inference_server_message::Message::InvocationResponse(
                            pb::RunInferenceInvocationResponse {
                                invocation_id: id.clone(),
                                response: Some(response),
                            },
                        ),
                    );
                }
            }
            Err(value) => {
                error = Some(value.to_string());
                break;
            }
        }
    }
    if error.is_none() && !terminal && !pending_tools.is_empty() {
        error = Some("provider stream ended before authorizing tool calls".into());
    }
    invocations.lock().remove(&id);
    let message = pb::RunInferenceInvocationEnd {
        invocation_id: id,
        error: error.map(|message| pb::RunInferenceInvocationError {
            code: 13,
            message,
            ..Default::default()
        }),
        ..Default::default()
    };
    emit(
        &output,
        pb::run_inference_server_message::Message::InvocationEnd(message),
    );
}

fn validate_tool_delivery(
    events: &[ModelEvent],
    finish: crate::provider::FinishReason,
) -> std::result::Result<(), &'static str> {
    use crate::provider::FinishReason;
    if events.is_empty() {
        return if finish == FinishReason::ToolUse {
            Err("provider reported tool use without tool calls")
        } else {
            Ok(())
        };
    }
    if finish != FinishReason::ToolUse {
        return Err("provider did not authorize streamed tool calls");
    }
    let mut calls = HashMap::new();
    for event in events {
        match event {
            ModelEvent::ToolCallStart { index, .. } => {
                if calls.insert(*index, false).is_some() {
                    return Err("provider reused a tool index");
                }
            }
            ModelEvent::ToolCallArgumentsDelta { index, .. } => {
                if calls.get(index) != Some(&false) {
                    return Err("provider emitted arguments for an unknown or ended tool");
                }
            }
            ModelEvent::ToolCallEnd { index } => match calls.get_mut(index) {
                Some(ended) if !*ended => *ended = true,
                _ => return Err("provider ended an unknown or ended tool"),
            },
            _ => {}
        }
    }
    if calls.is_empty() || calls.values().any(|ended| !ended) {
        return Err("provider completed with unfinished tool calls");
    }
    Ok(())
}

fn model_response(
    event: ModelEvent,
    tool_calls: &mut HashMap<usize, (String, String, String)>,
    context_window: Option<u64>,
) -> Option<pb::InferenceStreamResponse> {
    use pb::inference_stream_response::Response;
    let response = match event {
        ModelEvent::TextDelta(text) => Response::TextPart(pb::InferenceTextStreamPart {
            text,
            is_final: false,
        }),
        ModelEvent::TextEnd => Response::TextPart(pb::InferenceTextStreamPart {
            text: String::new(),
            is_final: true,
        }),
        ModelEvent::ThinkingDelta(text) => {
            Response::ThinkingPart(pb::InferenceThinkingStreamPart {
                text,
                is_final: false,
                ..Default::default()
            })
        }
        ModelEvent::ThinkingEnd => Response::ThinkingPart(pb::InferenceThinkingStreamPart {
            text: String::new(),
            is_final: true,
            ..Default::default()
        }),
        ModelEvent::ThinkingSignature(signature) => {
            Response::ThinkingPart(pb::InferenceThinkingStreamPart {
                text: String::new(),
                signature: Some(signature),
                is_final: true,
                ..Default::default()
            })
        }
        ModelEvent::RedactedThinking(data) => {
            Response::ThinkingPart(pb::InferenceThinkingStreamPart {
                text: String::new(),
                redacted_data: Some(data),
                is_final: true,
                ..Default::default()
            })
        }
        ModelEvent::ToolCallStart {
            index,
            call_id,
            name,
        } => {
            tool_calls.insert(index, (call_id.clone(), name.clone(), String::new()));
            Response::ToolCallPart(pb::InferenceToolCallStreamPart {
                tool_call_id: call_id,
                tool_name: name,
                args: String::new(),
                is_complete: false,
                tool_index: i32::try_from(index).ok(),
                ..Default::default()
            })
        }
        ModelEvent::ToolCallArgumentsDelta { index, delta } => {
            let (call_id, _, arguments) = tool_calls.get_mut(&index)?;
            arguments.push_str(&delta);
            Response::ToolCallPart(pb::InferenceToolCallStreamPart {
                tool_call_id: call_id.clone(),
                tool_name: String::new(),
                args: delta,
                is_complete: false,
                tool_index: i32::try_from(index).ok(),
                ..Default::default()
            })
        }
        ModelEvent::ToolCallEnd { index } => {
            let (call_id, name, arguments) = tool_calls.remove(&index)?;
            Response::ToolCallPart(pb::InferenceToolCallStreamPart {
                tool_call_id: call_id,
                tool_name: name,
                args: arguments,
                is_complete: true,
                tool_index: i32::try_from(index).ok(),
                ..Default::default()
            })
        }
        ModelEvent::Usage(usage) => Response::ExtendedUsage(pb::InferenceExtendedUsageInfo {
            input_tokens: count(usage.context_input_tokens.or(usage.input_tokens)),
            output_tokens: count(usage.output_tokens),
            cache_read_tokens: count(usage.cache_read_tokens),
            cache_write_tokens: count(usage.cache_write_tokens),
            max_tokens: count(context_window),
            reasoning_tokens: usage.reasoning_tokens.map(|tokens| count(Some(tokens))),
        }),
        ModelEvent::Start { model_call_id } => {
            Response::InvocationId(pb::InferenceInvocationIdInfo {
                invocation_id: model_call_id,
            })
        }
        ModelEvent::TextStart
        | ModelEvent::ThinkingStart
        | ModelEvent::ProviderReplayState(_)
        | ModelEvent::Done(_) => return None,
    };
    Some(pb::InferenceStreamResponse {
        response: Some(response),
        ..Default::default()
    })
}

fn agent_mode_configs() -> Vec<pb::RunInferenceAgentModeConfig> {
    const MODES: [&str; 6] = ["agent", "chat", "plan", "debug", "triage", "multitask"];
    const SWITCH: [&str; 5] = ["agent", "chat", "plan", "debug", "multitask"];
    MODES
        .into_iter()
        .map(|mode| pb::RunInferenceAgentModeConfig {
            mode_id: mode.into(),
            enable_ask_question_tool: true,
            switch_mode_tool_config: (mode == "agent").then(|| {
                pb::RunInferenceSwitchModeToolConfig {
                    target_modes: SWITCH.into_iter().map(str::to_owned).collect(),
                }
            }),
        })
        .collect()
}

fn count(value: Option<u64>) -> i32 {
    value.unwrap_or_default().min(i32::MAX as u64) as i32
}

fn invocation_error(output: &OutputHub, id: String, message: &str) {
    emit(
        output,
        pb::run_inference_server_message::Message::InvocationEnd(pb::RunInferenceInvocationEnd {
            invocation_id: id,
            error: Some(pb::RunInferenceInvocationError {
                code: 3,
                message: message.into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
    );
}

fn emit(output: &OutputHub, message: pb::run_inference_server_message::Message) {
    let message = pb::RunInferenceServerMessage {
        message: Some(message),
    };
    if let Ok(frame) = connect::encode_message(&message) {
        output.emit(frame);
    }
}

pub(crate) fn decode_append(data: &[u8]) -> Result<pb::RunInferenceClientMessage> {
    pb::RunInferenceClientMessage::decode(data).map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Usage;

    #[tokio::test]
    async fn anthropic_tool_end_before_done_is_delivered_only_for_authorized_turns() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (reason, message_stop, allowed) in [
            ("tool_use", true, true),
            ("end_turn", true, true),
            ("end_turn", false, true),
            ("refusal", true, false),
            ("refusal", false, false),
            ("max_tokens", true, false),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let body = format!(
                "data: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"tool_use\",\"id\":\"tool\",\"name\":\"Shell\"}}}}\n\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":\"{{}}\"}}}}\n\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"{reason}\"}}}}\n\n{}",
                if message_stop { "data: {\"type\":\"message_stop\"}\n\n" } else { "" }
            );
            tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 8192];
                let _ = socket.read(&mut request).await;
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}", body.len(), body).as_bytes()).await.unwrap();
            });
            let provider = crate::provider::AnthropicProvider::new(
                reqwest::Client::new(),
                crate::config::ProviderConfig {
                    kind: crate::config::ProviderKind::Anthropic,
                    request_url: format!("http://{address}"),
                    api_key: "test".into(),
                    custom_headers: Default::default(),
                    max_output_tokens: None,
                    request_timeout: std::time::Duration::from_secs(5),
                    allowed_body_fields: None,
                },
            );
            let output = Arc::new(OutputHub::default());
            let mut receiver = output.subscribe();
            serve_invocation(
                Arc::new(provider),
                output,
                Arc::new(Mutex::new(HashMap::new())),
                CancellationToken::new(),
                ModelInvocation {
                    call_id: "call".into(),
                    run_id: "run".into(),
                    conversation_id: "conversation".into(),
                    prompt_cache_enabled: true,
                    provider_call_index: 0,
                    request: crate::model::ModelRequest {
                        prompt: crate::model::PromptSpec {
                            instructions: String::new(),
                            tools: vec![],
                        },
                        model: ModelSpec::new("test"),
                        history: vec![],
                    },
                },
            )
            .await;
            let mut tool_parts = Vec::new();
            let mut ended = false;
            while let Ok(frame) = receiver.try_recv() {
                let (_, bytes) = connect::decode_frames(&frame).unwrap().pop().unwrap();
                match pb::RunInferenceServerMessage::decode(bytes)
                    .unwrap()
                    .message
                    .unwrap()
                {
                    pb::run_inference_server_message::Message::InvocationResponse(response) => {
                        if let Some(pb::inference_stream_response::Response::ToolCallPart(part)) =
                            response.response.and_then(|response| response.response)
                        {
                            tool_parts.push(part);
                        }
                    }
                    pb::run_inference_server_message::Message::InvocationEnd(end) => {
                        ended = true;
                        assert_eq!(
                            end.error.is_none(),
                            allowed,
                            "{reason}, message_stop={message_stop}"
                        );
                    }
                    _ => {}
                }
            }
            assert!(ended);
            assert_eq!(tool_parts.len(), if allowed { 3 } else { 0 });
            if allowed {
                assert!(tool_parts.last().unwrap().is_complete);
                assert_eq!(tool_parts.last().unwrap().args, "{}");
            }
        }
    }

    #[test]
    fn tool_delivery_requires_complete_authorized_calls_but_plain_stops_are_valid() {
        use crate::provider::FinishReason;
        let events = vec![
            ModelEvent::ToolCallStart {
                index: 0,
                call_id: "tool".into(),
                name: "Shell".into(),
            },
            ModelEvent::ToolCallArgumentsDelta {
                index: 0,
                delta: "{}".into(),
            },
            ModelEvent::ToolCallEnd { index: 0 },
        ];
        assert!(validate_tool_delivery(&events, FinishReason::ToolUse).is_ok());
        assert!(validate_tool_delivery(&events, FinishReason::Stop).is_err());
        assert!(validate_tool_delivery(&events, FinishReason::Length).is_err());
        assert!(validate_tool_delivery(&events[..2], FinishReason::ToolUse).is_err());
        assert!(validate_tool_delivery(&events[1..], FinishReason::ToolUse).is_err());
        assert!(validate_tool_delivery(&[], FinishReason::Stop).is_ok());
        assert!(validate_tool_delivery(&[], FinishReason::Length).is_ok());
        assert!(validate_tool_delivery(&[], FinishReason::ToolUse).is_err());
    }

    #[tokio::test]
    async fn refused_inference_does_not_deliver_tools() {
        struct Refused;
        impl Provider for Refused {
            fn stream(
                &self,
                _: ModelInvocation,
                _: CancellationToken,
            ) -> crate::provider::ProviderStream {
                Box::pin(tokio_stream::iter(vec![
                    Ok(ModelEvent::Start {
                        model_call_id: "call".into(),
                    }),
                    Ok(ModelEvent::ToolCallStart {
                        index: 0,
                        call_id: "tool".into(),
                        name: "Shell".into(),
                    }),
                    Ok(ModelEvent::ToolCallArgumentsDelta {
                        index: 0,
                        delta: "{}".into(),
                    }),
                    Ok(ModelEvent::ToolCallEnd { index: 0 }),
                    Ok(ModelEvent::Done(crate::provider::FinishReason::Stop)),
                ]))
            }
        }
        let output = Arc::new(OutputHub::default());
        let mut receiver = output.subscribe();
        serve_invocation(
            Arc::new(Refused),
            output,
            Arc::new(Mutex::new(HashMap::new())),
            CancellationToken::new(),
            ModelInvocation {
                call_id: "call".into(),
                run_id: "run".into(),
                conversation_id: "conversation".into(),
                prompt_cache_enabled: true,
                provider_call_index: 0,
                request: crate::model::ModelRequest {
                    prompt: crate::model::PromptSpec {
                        instructions: String::new(),
                        tools: vec![],
                    },
                    model: ModelSpec::new("test"),
                    history: vec![],
                },
            },
        )
        .await;
        let start = receiver.try_recv().unwrap();
        let (_, bytes) = connect::decode_frames(&start).unwrap().pop().unwrap();
        let message = pb::RunInferenceServerMessage::decode(bytes).unwrap();
        assert!(matches!(
            message.message,
            Some(pb::run_inference_server_message::Message::InvocationResponse(_))
        ));
        let frame = receiver.try_recv().unwrap();
        let (_, bytes) = connect::decode_frames(&frame).unwrap().pop().unwrap();
        let message = pb::RunInferenceServerMessage::decode(bytes).unwrap();
        assert!(
            matches!(message.message, Some(pb::run_inference_server_message::Message::InvocationEnd(end)) if end.error.is_some())
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn usage_preserves_context_counts_and_selected_window() {
        let usage = Usage {
            input_tokens: Some(100),
            context_input_tokens: Some(65_000),
            cache_read_tokens: Some(64_900),
            reasoning_tokens: Some(12),
            ..Default::default()
        };
        let response =
            model_response(ModelEvent::Usage(usage), &mut HashMap::new(), Some(356_000)).unwrap();
        let Some(pb::inference_stream_response::Response::ExtendedUsage(info)) = response.response
        else {
            panic!("expected extended usage");
        };
        assert_eq!(info.input_tokens, 65_000);
        assert_eq!(info.max_tokens, 356_000);
        assert_eq!(info.cache_read_tokens, 64_900);
        assert_eq!(info.reasoning_tokens, Some(12));
    }

    #[test]
    fn unknown_window_is_not_invented_and_counts_saturate() {
        let response = model_response(
            ModelEvent::Usage(Usage {
                input_tokens: Some(u64::MAX),
                ..Default::default()
            }),
            &mut HashMap::new(),
            None,
        )
        .unwrap();
        let Some(pb::inference_stream_response::Response::ExtendedUsage(info)) = response.response
        else {
            panic!("expected extended usage");
        };
        assert_eq!(info.input_tokens, i32::MAX);
        assert_eq!(info.max_tokens, 0);
        assert_eq!(count(Some(u64::MAX)), i32::MAX);
        assert!(model_response(
            ModelEvent::Done(crate::provider::FinishReason::Stop),
            &mut HashMap::new(),
            Some(356_000)
        )
        .is_none());
    }
}
