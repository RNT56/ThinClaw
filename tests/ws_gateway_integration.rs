//! End-to-end integration tests for the WebSocket gateway.
//!
//! These tests start a real Axum server on a random port, connect a WebSocket
//! client, and verify the full message flow:
//! - WebSocket upgrade with auth
//! - Ping/pong
//! - Client message → agent msg_tx
//! - Broadcast SSE event → WebSocket client
//! - Connection tracking (counter increment/decrement)
//! - Gateway status endpoint

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use thinclaw::channels::IncomingMessage;
use thinclaw::channels::web::server::{GatewayState, start_server};
use thinclaw::channels::web::sse::SseManager;
use thinclaw::channels::web::types::SseEvent;
use thinclaw::channels::web::ws::WsConnectionTracker;
use thinclaw::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};

const AUTH_TOKEN: &str = "test-token-12345";
const TIMEOUT: Duration = Duration::from_secs(5);

/// Start a gateway server on a random port and return the bound address + agent
/// message receiver.
async fn start_test_server() -> (
    SocketAddr,
    Arc<GatewayState>,
    mpsc::Receiver<IncomingMessage>,
) {
    let (agent_tx, agent_rx) = mpsc::channel(64);

    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(Some(agent_tx)),
        sse: SseManager::new(),
        workspace: None,
        session_manager: Some(Arc::new(thinclaw::agent::SessionManager::new())),
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: None,
        job_manager: None,
        prompt_queue: None,
        context_manager: None,
        scheduler: tokio::sync::RwLock::new(None),
        user_id: "test-user".to_string(),
        actor_id: "test-actor".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None,
        llm_runtime: None,
        skill_registry: None,
        skill_catalog: None,
        skill_remote_hub: None,
        skill_quarantine: None,
        chat_rate_limiter: thinclaw::channels::web::rate_limiter::RateLimiter::new(30, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        cost_tracker: None,
        startup_time: std::time::Instant::now(),
        restart_requested: std::sync::atomic::AtomicBool::new(false),
        routine_engine: None,
        secrets_store: None,
        channel_manager: None,
    });

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let bound_addr = start_server(addr, state.clone(), AUTH_TOKEN.to_string(), vec![])
        .await
        .expect("Failed to start test server");

    (bound_addr, state, agent_rx)
}

/// Connect a WebSocket client with auth token in query parameter.
async fn connect_ws(
    addr: SocketAddr,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let url = format!("ws://{}/api/chat/ws?token={}", addr, AUTH_TOKEN);
    let mut request = url.into_client_request().unwrap();
    // Server requires an Origin header from localhost to prevent cross-site WS hijacking.
    request.headers_mut().insert(
        "Origin",
        format!("http://127.0.0.1:{}", addr.port()).parse().unwrap(),
    );
    let (stream, _response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("Failed to connect WebSocket");
    stream
}

/// Read the next text frame from the WebSocket, with a timeout.
async fn recv_text(
    stream: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> String {
    let msg = timeout(TIMEOUT, stream.next())
        .await
        .expect("Timed out waiting for WS message")
        .expect("Stream ended")
        .expect("WS error");
    match msg {
        Message::Text(text) => text.to_string(),
        other => panic!("Expected Text frame, got {:?}", other),
    }
}

async fn wait_for_sse_subscribers(state: &GatewayState, expected: u64) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if state.sse.connection_count() >= expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out waiting for {expected} SSE/WS broadcast subscriber(s)"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn create_visible_thread(state: &GatewayState) -> String {
    let session_manager = state
        .session_manager
        .as_ref()
        .expect("test server should have a session manager");
    let identity = ResolvedIdentity {
        principal_id: "test-user".to_string(),
        actor_id: "test-actor".to_string(),
        conversation_scope_id: scope_id_from_key("principal:test-user"),
        conversation_kind: ConversationKind::Direct,
        raw_sender_id: "test-actor".to_string(),
        stable_external_conversation_key: "gateway://direct/test-user/actor/test-actor".to_string(),
    };
    let session = session_manager
        .get_or_create_session_for_identity(&identity)
        .await;
    let mut guard = session.lock().await;
    guard.create_thread().id.to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_ws_ping_pong() {
    let (addr, _state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Send ping
    let ping = r#"{"type":"ping"}"#;
    ws.send(Message::Text(ping.into())).await.unwrap();

    // Expect pong
    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "pong");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_message_reaches_agent() {
    let (addr, _state, mut agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Send a chat message
    let msg = r#"{"type":"message","content":"hello from ws","thread_id":"t42"}"#;
    ws.send(Message::Text(msg.into())).await.unwrap();

    // Verify it arrives on the agent's msg_tx
    let incoming = timeout(TIMEOUT, agent_rx.recv())
        .await
        .expect("Timed out waiting for agent message")
        .expect("Agent channel closed");

    assert_eq!(incoming.content, "hello from ws");
    assert_eq!(incoming.thread_id.as_deref(), Some("t42"));
    assert_eq!(incoming.channel, "gateway");
    assert_eq!(incoming.user_id, "test-user");
    let identity = incoming.resolved_identity();
    assert_eq!(identity.principal_id, "test-user");
    assert_eq!(identity.actor_id, "test-actor");
    assert_eq!(
        identity.stable_external_conversation_key,
        "gateway://direct/test-user/actor/test-actor/thread/t42"
    );

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_approval_preserves_actor_bound_identity() {
    let (addr, _state, mut agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    let request_id = uuid::Uuid::new_v4();
    let msg = serde_json::json!({
        "type": "approval",
        "request_id": request_id,
        "action": "approve",
        "thread_id": "t-approval",
    });
    ws.send(Message::Text(msg.to_string().into()))
        .await
        .unwrap();

    let incoming = timeout(TIMEOUT, agent_rx.recv())
        .await
        .expect("Timed out waiting for approval message")
        .expect("Agent channel closed");

    let identity = incoming.resolved_identity();
    assert_eq!(identity.principal_id, "test-user");
    assert_eq!(identity.actor_id, "test-actor");
    assert_eq!(
        identity.stable_external_conversation_key,
        "gateway://direct/test-user/actor/test-actor/thread/t-approval"
    );

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_broadcast_event_received() {
    let (addr, state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    wait_for_sse_subscribers(&state, 1).await;
    let thread_id = create_visible_thread(&state).await;

    // Broadcast an SSE event (simulates agent sending a response)
    state.sse.broadcast(SseEvent::Response {
        content: "agent says hi".to_string(),
        thread_id,
        attachments: Vec::new(),
    });

    // The WS client should receive it
    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "event");
    assert_eq!(parsed["event_type"], "response");
    assert_eq!(parsed["data"]["content"], "agent says hi");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_thinking_event() {
    let (addr, state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;
    wait_for_sse_subscribers(&state, 1).await;

    state.sse.broadcast(SseEvent::Thinking {
        message: "analyzing...".to_string(),
        thread_id: None,
    });

    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "event");
    assert_eq!(parsed["event_type"], "thinking");
    assert_eq!(parsed["data"]["message"], "analyzing...");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_connection_tracking() {
    let (addr, state, _agent_rx) = start_test_server().await;
    let tracker = state.ws_tracker.as_ref().unwrap();

    assert_eq!(tracker.connection_count(), 0);

    // Connect first client
    let ws1 = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(tracker.connection_count(), 1);

    // Connect second client
    let ws2 = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(tracker.connection_count(), 2);

    // Disconnect first
    drop(ws1);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(tracker.connection_count(), 1);

    // Disconnect second
    drop(ws2);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(tracker.connection_count(), 0);
}

#[tokio::test]
async fn test_ws_invalid_message_returns_error() {
    let (addr, _state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Send invalid JSON
    ws.send(Message::Text("not json".into())).await.unwrap();

    // Should get an error message back
    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "error");
    assert!(
        parsed["message"]
            .as_str()
            .unwrap()
            .contains("Invalid message")
    );

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_unknown_type_returns_error() {
    let (addr, _state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Send valid JSON but unknown message type
    ws.send(Message::Text(r#"{"type":"foobar"}"#.into()))
        .await
        .unwrap();

    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "error");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_gateway_status_endpoint() {
    let (addr, _state, _agent_rx) = start_test_server().await;

    // Connect a WS client
    let _ws = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Hit the status endpoint
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/gateway/status", addr))
        .header("Authorization", format!("Bearer {}", AUTH_TOKEN))
        .send()
        .await
        .expect("Failed to fetch status");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ws_connections"], 1);
    assert!(body["total_connections"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn test_ws_no_auth_rejected() {
    let (addr, _state, _agent_rx) = start_test_server().await;

    // Try to connect without auth token
    let url = format!("ws://{}/api/chat/ws", addr);
    let request = url.into_client_request().unwrap();
    let result = tokio_tungstenite::connect_async(request).await;

    // Should fail (401 from auth middleware before WS upgrade)
    assert!(result.is_err());
}

#[tokio::test]
async fn test_ws_multiple_events_in_sequence() {
    let (addr, state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;
    wait_for_sse_subscribers(&state, 1).await;
    let thread_id = create_visible_thread(&state).await;

    // Broadcast multiple events rapidly
    state.sse.broadcast(SseEvent::Thinking {
        message: "step 1".to_string(),
        thread_id: None,
    });
    state.sse.broadcast(SseEvent::ToolStarted {
        name: "shell".to_string(),
        thread_id: None,
    });
    state.sse.broadcast(SseEvent::ToolCompleted {
        name: "shell".to_string(),
        success: true,
        thread_id: None,
    });
    state.sse.broadcast(SseEvent::Response {
        content: "done".to_string(),
        thread_id,
        attachments: Vec::new(),
    });

    // Receive all 4 in order
    let t1 = recv_text(&mut ws).await;
    let t2 = recv_text(&mut ws).await;
    let t3 = recv_text(&mut ws).await;
    let t4 = recv_text(&mut ws).await;

    let p1: serde_json::Value = serde_json::from_str(&t1).unwrap();
    let p2: serde_json::Value = serde_json::from_str(&t2).unwrap();
    let p3: serde_json::Value = serde_json::from_str(&t3).unwrap();
    let p4: serde_json::Value = serde_json::from_str(&t4).unwrap();

    assert_eq!(p1["event_type"], "thinking");
    assert_eq!(p2["event_type"], "tool_started");
    assert_eq!(p3["event_type"], "tool_completed");
    assert_eq!(p4["event_type"], "response");

    ws.close(None).await.unwrap();
}
