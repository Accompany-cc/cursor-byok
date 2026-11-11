//! Verifies BidiAppend payload selection and Agent message decoding.
use super::*;

fn message(model_id: &str) -> agent::AgentClientMessage {
    agent::AgentClientMessage {
        message: Some(agent::agent_client_message::Message::RunRequest(
            agent::AgentRunRequest {
                conversation_id: Some("encoding-conversation".into()),
                requested_model: Some(agent::RequestedModel {
                    model_id: model_id.into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
    }
}

fn request(data: String, data_binary: Vec<u8>) -> ai::BidiAppendRequest {
    ai::BidiAppendRequest {
        request_id: Some(ai::BidiRequestId {
            request_id: "encoding-request".into(),
        }),
        append_seqno: 42,
        data,
        data_binary,
    }
}

fn assert_decoded(request: &ai::BidiAppendRequest, expected: &agent::AgentClientMessage) {
    let decoded = decode(request).unwrap();
    assert_eq!(decoded.request_id, "encoding-request");
    assert_eq!(decoded.seqno, 42);
    assert_eq!(&decoded.message, expected);
    assert_eq!(decoded.model_id(), Some("binary-model"));
    assert_eq!(decoded.conversation_id(), Some("encoding-conversation"));
}

#[test]
fn accepts_binary_payload() {
    let message = message("binary-model");
    let payload = message.encode_to_vec();
    assert!(!payload.is_empty(), "fixture must encode a real message");
    assert_decoded(&request(String::new(), payload), &message);
}

#[test]
fn accepts_legacy_hex_payload() {
    let message = message("binary-model");
    assert_decoded(
        &request(hex::encode(message.encode_to_vec()), Vec::new()),
        &message,
    );
}

#[test]
fn rejects_both_payloads_even_if_identical_or_hex_is_invalid() {
    let payload = message("binary-model").encode_to_vec();
    for data in [
        hex::encode(&payload),
        hex::encode(message("hex-model").encode_to_vec()),
        "not hexadecimal".into(),
    ] {
        assert!(matches!(
            decode(&request(data, payload.clone())),
            Err(Error::Protocol(error)) if error == "BidiAppend contains both data and data_binary"
        ));
    }
}

#[test]
fn rejects_empty_payload() {
    assert!(matches!(
        decode(&request(String::new(), Vec::new())),
        Err(Error::Protocol(error)) if error == "BidiAppend contains no AgentClientMessage"
    ));
}

#[test]
fn rejects_invalid_hex_without_binary_payload() {
    assert!(matches!(
        decode(&request("not hexadecimal".into(), Vec::new())),
        Err(Error::Protocol(error)) if error.starts_with("invalid BidiAppend hex:")
    ));
}

#[test]
fn rejects_malformed_binary_payload() {
    assert!(decode(&request(String::new(), vec![0x0a])).is_err());
}

#[test]
fn rejects_malformed_protobuf_in_hex_payload() {
    assert!(decode(&request("0a".into(), Vec::new())).is_err());
}

#[test]
fn rejects_missing_request_id_with_either_encoding() {
    let payload = message("binary-model").encode_to_vec();
    for mut request in [
        request(String::new(), payload.clone()),
        request(hex::encode(payload), Vec::new()),
    ] {
        request.request_id = None;
        assert!(matches!(
            decode(&request),
            Err(Error::Protocol(error)) if error == "BidiAppend request_id is required"
        ));
    }
}

#[test]
fn rejects_empty_request_id() {
    let mut request = request(String::new(), message("binary-model").encode_to_vec());
    request.request_id.as_mut().unwrap().request_id.clear();
    assert!(matches!(
        decode(&request),
        Err(Error::Protocol(error)) if error == "BidiAppend request_id is required"
    ));
}
