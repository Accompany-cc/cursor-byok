//! Exercises Bidi encodings across Agent and client-owned inference streams.
use std::sync::{Arc, Mutex};

use super::{router_with_proxy, CursorProxy};
use crate::{
    cursor::{
        prompting::{PromptAssets, PromptCompiler},
        protocol::{
            connect,
            proto::{agent::v1 as agent, aiserver::v1 as ai, inference as pb},
        },
        services::knowledge::KnowledgeService,
        transport::TransportRegistry,
    },
    model::{
        ModelConfigInput, ModelInvocation, ModelRequest, ModelType, ProjectedContent,
        OPENAI_CHAT_ENDPOINT,
    },
    network::NetworkClients,
    provider::{FinishReason, ModelEvent, Provider, ProviderStream},
    store::Store,
};
use axum::{
    body::{Body, Bytes},
    http::{header, Request, StatusCode},
};
use futures_util::{stream, StreamExt};
use prost::Message;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

#[tokio::test]
async fn client_loop_receives_ready_text_tool_call_and_completion() {
    assert_client_loop(false).await;
}

#[tokio::test]
async fn binary_client_loop_receives_ready_text_tool_call_and_completion() {
    assert_client_loop(true).await;
}

async fn assert_client_loop(binary: bool) {
    let (_directory, store) = temp_store().await;
    let model = store.create_model(&model_input()).await.unwrap();
    let provider = FakeProvider::default();
    provider.push(vec![
        ModelEvent::Start {
            model_call_id: "invoke-1".into(),
        },
        ModelEvent::ThinkingStart,
        ModelEvent::ThinkingDelta("Checking the file".into()),
        ModelEvent::ThinkingEnd,
        ModelEvent::ThinkingSignature("signature-1".into()),
        ModelEvent::TextStart,
        ModelEvent::TextDelta("Let me check".into()),
        ModelEvent::TextEnd,
        ModelEvent::ToolCallStart {
            index: 0,
            call_id: "call-1".into(),
            name: "Read".into(),
        },
        ModelEvent::ToolCallArgumentsDelta {
            index: 0,
            delta: "{\"path\":\"a.rs\"}".into(),
        },
        ModelEvent::ToolCallEnd { index: 0 },
        ModelEvent::Done(FinishReason::ToolUse),
    ]);
    let (_router_directory, router) = router(store, provider.clone());
    let request_id = "inference-test";
    let ambiguous_payload = pb::RunInferenceClientMessage {
        message: Some(pb::run_inference_client_message::Message::RunRequest(
            pb::RunInferenceRunRequest {
                conversation_id: "conversation-test".into(),
                requested_model: Some(pb::InferenceRequestedModel {
                    model_id: model.model_hash.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
    }
    .encode_to_vec();
    let ambiguous = ai::BidiAppendRequest {
        request_id: Some(ai::BidiRequestId {
            request_id: request_id.into(),
        }),
        data: hex::encode(&ambiguous_payload),
        data_binary: ambiguous_payload,
        ..Default::default()
    };
    let rejected = router
        .clone()
        .oneshot(
            Request::post("/aiserver.v1.BidiService/BidiAppend")
                .header(header::CONTENT_TYPE, "application/proto")
                .body(Body::from(ambiguous.encode_to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    assert!(provider.requests().is_empty());
    append(
        &router,
        request_id,
        0,
        pb::RunInferenceClientMessage {
            message: Some(pb::run_inference_client_message::Message::RunRequest(
                pb::RunInferenceRunRequest {
                    conversation_id: "conversation-test".into(),
                    requested_model: Some(pb::InferenceRequestedModel {
                        model_id: model.model_hash.clone(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
        },
        binary,
    )
    .await;
    let response = router
        .clone()
        .oneshot(
            Request::post("/aiserver.v1.InferenceService/RunInferenceSSE")
                .header(header::CONTENT_TYPE, "application/proto")
                .body(Body::from(
                    connect::encode_message(&agent::BidiRequestId {
                        request_id: request_id.into(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let ready = next_message(&mut stream).await;
    let Some(pb::run_inference_server_message::Message::RunReady(ready)) = ready.message else {
        panic!("expected run_ready");
    };
    assert_eq!(ready.resolved_model.unwrap().model_id, model.model_hash);
    let metadata = ready.prompt_model_metadata.unwrap();
    assert!(!metadata.is_gpt5);
    let modes: Vec<&str> = metadata
        .agent_mode_configs
        .iter()
        .map(|config| config.mode_id.as_str())
        .collect();
    assert_eq!(
        modes,
        ["agent", "chat", "plan", "debug", "triage", "multitask"]
    );
    assert!(metadata
        .agent_mode_configs
        .iter()
        .all(|config| config.enable_ask_question_tool));
    let agent = metadata
        .agent_mode_configs
        .iter()
        .find(|config| config.mode_id == "agent")
        .unwrap();
    let switch_targets = agent.switch_mode_tool_config.as_ref().map(|config| {
        config
            .target_modes
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    });
    assert_eq!(
        switch_targets,
        Some(vec!["agent", "chat", "plan", "debug", "multitask"])
    );
    assert!(metadata
        .agent_mode_configs
        .iter()
        .filter(|config| config.mode_id != "agent")
        .all(|config| config.switch_mode_tool_config.is_none()));
    assert_eq!(
        metadata.ask_question_config.unwrap().tool_name,
        "AskQuestion"
    );

    append(
        &router,
        request_id,
        1,
        pb::RunInferenceClientMessage {
            message: Some(pb::run_inference_client_message::Message::InvokeModel(
                pb::RunInferenceInvokeModel {
                    invocation_id: "invoke-1".into(),
                    request: Some(pb::InferenceStreamRequest {
                        messages: vec![pb::InferenceCoreMessage {
                            role: 1,
                            content: Some(pb::inference_core_message::Content::Text(
                                "Read a.rs".into(),
                            )),
                            ..Default::default()
                        }],
                        tools: vec![pb::InferenceAgentTool {
                            name: "Read".into(),
                            description: "Read a file".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
        },
        binary,
    )
    .await;

    let mut text = String::new();
    let mut tool = String::new();
    let mut completed_tool = None;
    let mut signature = None;
    let mut invocation_id = None;
    loop {
        let message = next_message(&mut stream).await;
        match message.message.unwrap() {
            pb::run_inference_server_message::Message::InvocationResponse(response) => {
                match response.response.unwrap().response.unwrap() {
                    pb::inference_stream_response::Response::TextPart(part) => {
                        text.push_str(&part.text)
                    }
                    pb::inference_stream_response::Response::ToolCallPart(part) => {
                        if part.is_complete {
                            completed_tool = Some(part.args);
                        } else if part.tool_name.is_empty() {
                            tool.push_str(&part.args);
                        }
                    }
                    pb::inference_stream_response::Response::ThinkingPart(part) => {
                        signature = signature.or(part.signature);
                    }
                    pb::inference_stream_response::Response::InvocationId(info) => {
                        invocation_id = Some(info.invocation_id);
                    }
                    _ => {}
                }
            }
            pb::run_inference_server_message::Message::InvocationEnd(end) => {
                assert_eq!(end.invocation_id, "invoke-1");
                assert!(end.error.is_none());
                break;
            }
            _ => {}
        }
    }
    assert_eq!(text, "Let me check");
    assert_eq!(tool, "{\"path\":\"a.rs\"}");
    assert_eq!(completed_tool.as_deref(), Some(tool.as_str()));
    assert_eq!(signature.as_deref(), Some("signature-1"));
    assert_eq!(invocation_id.as_deref(), Some("invoke-1"));
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].history.len(), 1);
    assert_eq!(requests[0].prompt.tools[0].name, "Read");
    assert!(matches!(
        &requests[0].history[0].content,
        ProjectedContent::Parts(_)
    ));

    append(
        &router,
        request_id,
        2,
        pb::RunInferenceClientMessage {
            message: Some(pb::run_inference_client_message::Message::FinishRun(
                pb::RunInferenceFinishRun {
                    aborted: false,
                    ..Default::default()
                },
            )),
        },
        binary,
    )
    .await;
    let end = tokio::time::timeout(std::time::Duration::from_secs(3), stream.next())
        .await
        .expect("end frame timed out")
        .expect("inference stream ended before its end frame")
        .unwrap();
    assert_eq!(
        connect::decode_frames(&end).unwrap()[0].0,
        connect::END_STREAM_FLAG
    );
}

#[tokio::test]
async fn binary_agent_run_reaches_the_provider() {
    assert_agent_run(true).await;
}

#[tokio::test]
async fn hex_agent_run_still_reaches_the_provider() {
    assert_agent_run(false).await;
}

async fn assert_agent_run(binary: bool) {
    let (_directory, store) = temp_store().await;
    let model = store.create_model(&model_input()).await.unwrap();
    let provider = FakeProvider::default();
    provider.push(vec![
        ModelEvent::Start {
            model_call_id: "agent-call".into(),
        },
        ModelEvent::TextStart,
        ModelEvent::TextDelta("binary-compatible answer".into()),
        ModelEvent::TextEnd,
        ModelEvent::Done(FinishReason::Stop),
    ]);
    let (_router_directory, router) = router(store, provider.clone());
    let request_id = "agent-encoding-test";
    append(
        &router,
        request_id,
        0,
        agent::AgentClientMessage {
            message: Some(agent::agent_client_message::Message::RunRequest(
                agent::AgentRunRequest {
                    conversation_id: Some("agent-encoding-conversation".into()),
                    requested_model: Some(agent::RequestedModel {
                        model_id: model.model_hash,
                        ..Default::default()
                    }),
                    action: Some(agent::ConversationAction {
                        action: Some(agent::conversation_action::Action::UserMessageAction(
                            agent::UserMessageAction {
                                user_message: Some(agent::UserMessage {
                                    text: "verify binary agent input".into(),
                                    message_id: "encoding-user".into(),
                                    mode: agent::AgentMode::Agent as i32,
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                        )),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
        },
        binary,
    )
    .await;
    let response = router
        .clone()
        .oneshot(
            Request::post("/agent.v1.AgentService/RunSSE")
                .header(header::CONTENT_TYPE, "application/proto")
                .body(Body::from(
                    connect::encode_message(&agent::BidiRequestId {
                        request_id: request_id.into(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let mut seqno = 1;
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .expect("Agent stream timed out")
            .expect("Agent stream ended")
            .unwrap();
        let (flags, payload) = connect::decode_frames(&frame).unwrap().pop().unwrap();
        assert_eq!(
            flags,
            0,
            "Agent run failed: {}",
            String::from_utf8_lossy(&payload)
        );
        let message = agent::AgentServerMessage::decode(payload).unwrap();
        match message.message {
            Some(agent::agent_server_message::Message::KvServerMessage(kv)) => {
                append(
                    &router,
                    request_id,
                    seqno,
                    agent::AgentClientMessage {
                        message: Some(agent::agent_client_message::Message::KvClientMessage(
                            agent::KvClientMessage {
                                id: kv.id,
                                message: Some(agent::kv_client_message::Message::SetBlobResult(
                                    agent::SetBlobResult { error: None },
                                )),
                            },
                        )),
                    },
                    binary,
                )
                .await;
                seqno += 1;
            }
            Some(agent::agent_server_message::Message::InteractionUpdate(update))
                if matches!(
                    update.message,
                    Some(agent::interaction_update::Message::TurnEnded(_))
                ) =>
            {
                break
            }
            _ => {}
        }
    }
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert!(serde_json::to_string(&requests[0].history)
        .unwrap()
        .contains("verify binary agent input"));
}

#[tokio::test]
async fn invalid_bidi_payloads_return_bad_request_without_invoking_a_provider() {
    let (_directory, store) = temp_store().await;
    let provider = FakeProvider::default();
    let (_router_directory, router) = router(store, provider.clone());
    for (data, data_binary) in [
        (String::new(), Vec::new()),
        ("not hexadecimal".into(), Vec::new()),
        (String::new(), vec![0x0a]),
        ("0a".into(), vec![0x0a]),
        ("not hexadecimal".into(), vec![0x0a]),
    ] {
        let request = ai::BidiAppendRequest {
            request_id: Some(ai::BidiRequestId {
                request_id: "bad-input".into(),
            }),
            data,
            data_binary,
            ..Default::default()
        };
        let response = router
            .clone()
            .oneshot(
                Request::post("/aiserver.v1.BidiService/BidiAppend")
                    .header(header::CONTENT_TYPE, "application/proto")
                    .body(Body::from(request.encode_to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert!(provider.requests().is_empty());
}

async fn append(
    router: &axum::Router,
    request_id: &str,
    seqno: i64,
    message: impl Message,
    binary: bool,
) {
    let payload = message.encode_to_vec();
    assert!(!payload.is_empty(), "fixture must encode a real message");
    let request = ai::BidiAppendRequest {
        request_id: Some(ai::BidiRequestId {
            request_id: request_id.into(),
        }),
        data: if binary {
            String::new()
        } else {
            hex::encode(&payload)
        },
        data_binary: if binary { payload } else { Vec::new() },
        append_seqno: seqno,
    };
    let response = router
        .clone()
        .oneshot(
            Request::post("/aiserver.v1.BidiService/BidiAppend")
                .header(header::CONTENT_TYPE, "application/proto")
                .body(Body::from(request.encode_to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

async fn next_message<S>(stream: &mut S) -> pb::RunInferenceServerMessage
where
    S: futures_util::Stream<Item = Result<Bytes, axum::Error>> + Unpin,
{
    let frame = tokio::time::timeout(std::time::Duration::from_secs(3), stream.next())
        .await
        .expect("inference frame timed out")
        .expect("inference stream ended")
        .unwrap();
    let (flags, bytes) = connect::decode_frames(&frame).unwrap().pop().unwrap();
    assert_eq!(flags, 0);
    pb::RunInferenceServerMessage::decode(bytes).unwrap()
}

fn router(store: crate::store::Store, provider: FakeProvider) -> (tempfile::TempDir, axum::Router) {
    let directory = tempfile::tempdir().unwrap();
    let assets = PromptAssets::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("prompt/cursor")
            .as_path(),
    )
    .unwrap();
    let clients = NetworkClients::new(store.clone());
    let registry = TransportRegistry::new(store, Arc::new(provider), PromptCompiler::new(assets));
    let proxy = CursorProxy::cursor(clients);
    let knowledge = KnowledgeService::with_root(directory.path().join("rules")).unwrap();
    (directory, router_with_proxy(registry, proxy, knowledge))
}

async fn temp_store() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", directory.path().join("test.db").display());
    let store = Store::connect(&url).await.unwrap();
    (directory, store)
}

#[derive(Clone, Default)]
struct FakeProvider {
    requests: Arc<Mutex<Vec<ModelRequest>>>,
    events: Arc<Mutex<Vec<ModelEvent>>>,
}

impl FakeProvider {
    fn push(&self, events: Vec<ModelEvent>) {
        *self.events.lock().unwrap() = events;
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Provider for FakeProvider {
    fn stream(
        &self,
        invocation: ModelInvocation,
        _cancellation: CancellationToken,
    ) -> ProviderStream {
        self.requests.lock().unwrap().push(invocation.request);
        let events = std::mem::take(&mut *self.events.lock().unwrap());
        Box::pin(stream::iter(events.into_iter().map(Ok)))
    }
}

fn model_input() -> ModelConfigInput {
    ModelConfigInput {
        sort_order: 1,
        display_name: "Test model".into(),
        group_name: None,
        model_type: ModelType::OpenAi,
        base_url: "https://example.com/v1".into(),
        use_full_url: false,
        api_key: "test-key".into(),
        tooltip_data: "Test model".into(),
        model_id: "test-model".into(),
        reasoning_effort: None,
        openai_endpoint: OPENAI_CHAT_ENDPOINT.into(),
        openai_extra_params_enabled: false,
        openai_extra_params: serde_json::json!({}),
        custom_headers_enabled: false,
        custom_headers: serde_json::json!({}),
        anthropic_extra_params_enabled: false,
        anthropic_extra_params: serde_json::json!({}),
        context_window_tokens: None,
        max_completion_tokens: None,
        anthropic_max_tokens: None,
        anthropic_thinking_effort: None,
        thinking_budget_tokens: None,
    }
}
