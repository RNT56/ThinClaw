//! Contract test: the client's `wire::SseEvent` must deserialize the real
//! `thinclaw_gateway::web::types::SseEvent` JSON for every variant this client
//! models. Guards against silent drift without a runtime dependency on the
//! gateway crate (dev-dependency only).

use thinclaw_client::SseEvent as ClientEvent;
use thinclaw_gateway::web::types::SseEvent as ServerEvent;

/// Serialize a server event, parse it through the client, and assert the
/// modeled fields survive.
fn roundtrip(server: ServerEvent) -> ClientEvent {
    let json = serde_json::to_value(&server).expect("serialize server event");
    ClientEvent::from_json(json)
}

#[test]
fn client_parses_response_event() {
    let ev = roundtrip(ServerEvent::Response {
        content: "hello".into(),
        thread_id: "t-1".into(),
        attachments: Vec::new(),
    });
    assert_eq!(
        ev,
        ClientEvent::Response {
            content: "hello".into(),
            thread_id: "t-1".into(),
        }
    );
}

#[test]
fn client_parses_tool_events() {
    assert_eq!(
        roundtrip(ServerEvent::ToolStarted {
            name: "shell".into(),
            thread_id: Some("t-2".into()),
        }),
        ClientEvent::ToolStarted {
            name: "shell".into(),
            thread_id: Some("t-2".into()),
        }
    );
    assert_eq!(
        roundtrip(ServerEvent::ToolCompleted {
            name: "shell".into(),
            success: true,
            thread_id: Some("t-2".into()),
        }),
        ClientEvent::ToolCompleted {
            name: "shell".into(),
            success: true,
            thread_id: Some("t-2".into()),
        }
    );
}

#[test]
fn client_parses_status_and_stream_chunk() {
    assert_eq!(
        roundtrip(ServerEvent::Status {
            message: "working".into(),
            thread_id: None,
        }),
        ClientEvent::Status {
            message: "working".into(),
            thread_id: None,
        }
    );
    assert_eq!(
        roundtrip(ServerEvent::StreamChunk {
            content: "tok".into(),
            thread_id: Some("t-3".into()),
        }),
        ClientEvent::StreamChunk {
            content: "tok".into(),
            thread_id: Some("t-3".into()),
        }
    );
}

#[test]
fn unmodeled_server_events_become_unknown_not_errors() {
    // PlanUpdate is a server variant this client does not model; it must
    // degrade to Unknown (with payload) rather than fail to deserialize.
    let json = serde_json::to_value(ServerEvent::PlanUpdate {
        entries: vec![serde_json::json!({"step": "do the thing"})],
        thread_id: None,
    })
    .unwrap();
    let event_type = json
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    match ClientEvent::from_json(json) {
        ClientEvent::Unknown { event_type: t, raw } => {
            assert_eq!(t, event_type);
            assert!(raw.get("entries").is_some());
        }
        other => panic!("expected Unknown for unmodeled server event, got {other:?}"),
    }
}
