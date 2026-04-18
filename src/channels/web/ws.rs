//! WebSocket handler for bidirectional client communication.
//!
//! Provides the same event stream as SSE but also accepts incoming messages
//! (chat, approvals) over a single persistent connection for authenticated
//! non-browser clients. The browser UI remains SSE-first.
//!
//! ```text
//! Client ──── WS frame: {"type":"message","content":"hello"} ──► Agent Loop
//!        ◄─── WS frame: {"type":"event","event_type":"response","data":{...}} ── Broadcast
//!        ──── WS frame: {"type":"ping"} ──────────────────────────────────────►
//!        ◄─── WS frame: {"type":"pong"} ──────────────────────────────────────
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::submission::Submission;
use crate::channels::IncomingMessage;
use crate::channels::web::handlers::chat::clear_auth_mode_for_identity;
use crate::channels::web::identity_helpers::{
    GatewayRequestIdentity, sse_event_visible_to_identity,
};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::{SseEvent, WsClientMessage, WsServerMessage};

/// Tracks active WebSocket connections.
pub struct WsConnectionTracker {
    count: AtomicU64,
}

impl WsConnectionTracker {
    pub fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
        }
    }

    pub fn connection_count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    fn increment(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    fn decrement(&self) {
        self.count.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Default for WsConnectionTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle an upgraded WebSocket connection.
///
/// Spawns two tasks:
/// - **sender**: forwards broadcast events to the WebSocket client
/// - **receiver**: reads client frames and routes them to the agent
///
/// When either task ends (client disconnect or broadcast closed), both are
/// cleaned up.
pub async fn handle_ws_connection(
    socket: WebSocket,
    state: Arc<GatewayState>,
    request_identity: GatewayRequestIdentity,
) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Track connection
    if let Some(ref tracker) = state.ws_tracker {
        tracker.increment();
    }
    let tracker_for_drop = state.ws_tracker.clone();

    // Subscribe to broadcast events (same source as SSE).
    // Reject if we've hit the connection limit.
    let Some(raw_stream) = state.sse.subscribe_raw() else {
        tracing::warn!("WebSocket rejected: too many connections");
        // Decrement the WS tracker we already incremented above.
        if let Some(ref tracker) = tracker_for_drop {
            tracker.decrement();
        }
        return;
    };
    let state_for_stream = Arc::clone(&state);
    let identity_for_stream = request_identity.clone();
    let mut event_stream = Box::pin(raw_stream.filter_map(move |event| {
        let state = Arc::clone(&state_for_stream);
        let identity = identity_for_stream.clone();
        async move {
            if sse_event_visible_to_identity(
                state.store.as_ref(),
                state.as_ref(),
                &identity,
                &event,
            )
            .await
            {
                Some(event)
            } else {
                None
            }
        }
    }));

    // Channel for the sender task to receive messages from both
    // the broadcast stream and any direct sends (like Pong)
    let (direct_tx, mut direct_rx) = mpsc::channel::<WsServerMessage>(64);

    // Sender task: forward broadcast events + direct messages to WS client
    let sender_handle = tokio::spawn(async move {
        loop {
            let msg = tokio::select! {
                event = event_stream.next() => {
                    match event {
                        Some(sse_event) => WsServerMessage::from_sse_event(&sse_event),
                        None => break, // Broadcast channel closed
                    }
                }
                direct = direct_rx.recv() => {
                    match direct {
                        Some(msg) => msg,
                        None => break, // Direct channel closed
                    }
                }
            };

            let json = match serde_json::to_string(&msg) {
                Ok(j) => j,
                Err(_) => continue,
            };

            if ws_sink.send(Message::Text(json.into())).await.is_err() {
                break; // Client disconnected
            }
        }
    });

    // Receiver task: read client frames and route to agent
    while let Some(Ok(frame)) = ws_stream.next().await {
        match frame {
            Message::Text(text) => {
                let parsed: Result<WsClientMessage, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(client_msg) => {
                        handle_client_message(client_msg, &state, &request_identity, &direct_tx)
                            .await;
                    }
                    Err(e) => {
                        let _ = direct_tx
                            .send(WsServerMessage::Error {
                                message: format!("Invalid message: {}", e),
                            })
                            .await;
                    }
                }
            }
            Message::Close(_) => break,
            // Ignore binary, ping/pong (axum handles protocol-level pings)
            _ => {}
        }
    }

    // Clean up: abort sender, decrement counter
    sender_handle.abort();
    if let Some(ref tracker) = tracker_for_drop {
        tracker.decrement();
    }
}

/// Route a parsed client message to the appropriate handler.
async fn handle_client_message(
    msg: WsClientMessage,
    state: &GatewayState,
    request_identity: &GatewayRequestIdentity,
    direct_tx: &mpsc::Sender<WsServerMessage>,
) {
    match msg {
        WsClientMessage::Message { content, thread_id } => {
            let user_id = request_identity.principal_id.clone();
            let actor_id = request_identity.actor_id.clone();
            let mut incoming = IncomingMessage::new("gateway", &user_id, &content);
            incoming =
                incoming.with_identity(request_identity.resolved_identity(thread_id.as_deref()));
            if let Some(ref tid) = thread_id {
                incoming = incoming.with_thread(tid);
                incoming = incoming.with_metadata(serde_json::json!({
                    "thread_id": tid,
                    "actor_id": actor_id,
                }));
            }

            let tx_guard = state.msg_tx.read().await;
            if let Some(ref tx) = *tx_guard {
                if tx.send(incoming).await.is_err() {
                    let _ = direct_tx
                        .send(WsServerMessage::Error {
                            message: "Channel closed".to_string(),
                        })
                        .await;
                }
            } else {
                let _ = direct_tx
                    .send(WsServerMessage::Error {
                        message: "Channel not started".to_string(),
                    })
                    .await;
            }
        }
        WsClientMessage::Approval {
            request_id,
            action,
            thread_id,
        } => {
            let (approved, always) = match action.as_str() {
                "approve" => (true, false),
                "always" => (true, true),
                "deny" => (false, false),
                other => {
                    let _ = direct_tx
                        .send(WsServerMessage::Error {
                            message: format!("Unknown approval action: {}", other),
                        })
                        .await;
                    return;
                }
            };

            let request_uuid = match Uuid::parse_str(&request_id) {
                Ok(id) => id,
                Err(_) => {
                    let _ = direct_tx
                        .send(WsServerMessage::Error {
                            message: "Invalid request_id (expected UUID)".to_string(),
                        })
                        .await;
                    return;
                }
            };

            let approval = Submission::ExecApproval {
                request_id: request_uuid,
                approved,
                always,
            };
            let content = match serde_json::to_string(&approval) {
                Ok(c) => c,
                Err(e) => {
                    let _ = direct_tx
                        .send(WsServerMessage::Error {
                            message: format!("Failed to serialize approval: {}", e),
                        })
                        .await;
                    return;
                }
            };

            let user_id = request_identity.principal_id.clone();
            let actor_id = request_identity.actor_id.clone();
            let mut msg = IncomingMessage::new("gateway", &user_id, content);
            msg = msg.with_identity(request_identity.resolved_identity(thread_id.as_deref()));
            if let Some(ref tid) = thread_id {
                msg = msg.with_thread(tid);
                msg = msg.with_metadata(serde_json::json!({
                    "thread_id": tid,
                    "actor_id": actor_id,
                }));
            }
            let tx_guard = state.msg_tx.read().await;
            if let Some(ref tx) = *tx_guard {
                let _ = tx.send(msg).await;
            }
        }
        WsClientMessage::AuthToken {
            extension_name,
            token,
        } => {
            if let Some(ref ext_mgr) = state.extension_manager {
                match ext_mgr.auth(&extension_name, Some(&token)).await {
                    Ok(result) if result.status == "authenticated" => {
                        let msg = match ext_mgr.activate(&extension_name).await {
                            Ok(r) => format!(
                                "{} authenticated ({} tools loaded)",
                                extension_name,
                                r.tools_loaded.len()
                            ),
                            Err(e) => format!(
                                "{} authenticated but activation failed: {}",
                                extension_name, e
                            ),
                        };
                        clear_auth_mode_for_identity(state, request_identity).await;
                        let _ = direct_tx
                            .send(WsServerMessage::from_sse_event(&SseEvent::AuthCompleted {
                                extension_name,
                                success: true,
                                message: msg,
                            }))
                            .await;
                    }
                    Ok(result) => {
                        let _ = direct_tx
                            .send(WsServerMessage::from_sse_event(&SseEvent::AuthRequired {
                                extension_name,
                                instructions: result.instructions,
                                auth_url: result.auth_url,
                                setup_url: result.setup_url,
                            }))
                            .await;
                    }
                    Err(e) => {
                        let _ = direct_tx
                            .send(WsServerMessage::Error {
                                message: format!("Auth failed: {}", e),
                            })
                            .await;
                    }
                }
            } else {
                let _ = direct_tx
                    .send(WsServerMessage::Error {
                        message: "Extension manager not available".to_string(),
                    })
                    .await;
            }
        }
        WsClientMessage::AuthCancel { .. } => {
            clear_auth_mode_for_identity(state, request_identity).await;
        }
        WsClientMessage::Ping => {
            let _ = direct_tx.send(WsServerMessage::Pong).await;
        }
        WsClientMessage::Version {
            protocol_version,
            client_name,
        } => {
            let server_version = env!("CARGO_PKG_VERSION").to_string();
            // Simple compatibility: major version must match
            let client_major = protocol_version
                .split('.')
                .next()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0);
            let compatible = client_major == 1; // Protocol v1.x

            tracing::info!(
                "WS version handshake: client={} ({}), server={}",
                protocol_version,
                client_name.as_deref().unwrap_or("unknown"),
                server_version
            );

            let _ = direct_tx
                .send(WsServerMessage::VersionInfo {
                    protocol_version: "1.0.0".to_string(),
                    server_name: "thinclaw".to_string(),
                    server_version,
                    compatible,
                })
                .await;
        }
        WsClientMessage::ConfigSet { key, value } => {
            // Write the setting to the DB-backed settings store
            if let Some(ref store) = state.store {
                match crate::api::config::set_setting(
                    store,
                    &request_identity.principal_id,
                    &key,
                    &value,
                )
                .await
                {
                    Ok(()) => {
                        tracing::info!("WS config.set: key={} updated", key);
                        let _ = direct_tx
                            .send(WsServerMessage::ConfigResult {
                                key,
                                success: true,
                                error: None,
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = direct_tx
                            .send(WsServerMessage::ConfigResult {
                                key,
                                success: false,
                                error: Some(e.to_string()),
                            })
                            .await;
                    }
                }
            } else {
                let _ = direct_tx
                    .send(WsServerMessage::ConfigResult {
                        key,
                        success: false,
                        error: Some("No database configured for settings storage".to_string()),
                    })
                    .await;
            }
        }
        WsClientMessage::SecretSet { key, value } => {
            // Store the API key as a setting in the DB, prefixed with "secret."
            // In Remote Mode, the thin client transmits API keys to the orchestrator
            // which stores them in its local DB for use by the agent.
            if let Some(ref store) = state.store {
                let setting_key = format!("secret.{}", key);
                let setting_value = serde_json::Value::String(value);
                match crate::api::config::set_setting(
                    store,
                    &request_identity.principal_id,
                    &setting_key,
                    &setting_value,
                )
                .await
                {
                    Ok(()) => {
                        tracing::info!("WS secret.set: key={} stored", key);
                        let _ = direct_tx
                            .send(WsServerMessage::SecretResult {
                                key,
                                success: true,
                                error: None,
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = direct_tx
                            .send(WsServerMessage::SecretResult {
                                key,
                                success: false,
                                error: Some(e.to_string()),
                            })
                            .await;
                    }
                }
            } else {
                let _ = direct_tx
                    .send(WsServerMessage::SecretResult {
                        key,
                        success: false,
                        error: Some("No database configured for secret storage".to_string()),
                    })
                    .await;
            }
        }
        WsClientMessage::ModelList => {
            // Return list of available models from LLM provider
            let models = if let Some(ref llm) = state.llm_provider {
                let active = llm.active_model_name();
                match llm.list_models().await {
                    Ok(list) if !list.is_empty() => list
                        .into_iter()
                        .map(|name| crate::api::system::ModelInfo {
                            is_primary: name == active,
                            name,
                        })
                        .collect(),
                    _ => vec![crate::api::system::ModelInfo {
                        name: active,
                        is_primary: true,
                    }],
                }
            } else {
                vec![]
            };
            let _ = direct_tx
                .send(WsServerMessage::ModelListResult { models })
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_request_identity(user_id: &str) -> GatewayRequestIdentity {
        GatewayRequestIdentity::new(
            user_id,
            user_id,
            crate::channels::web::identity_helpers::GatewayAuthSource::BearerHeader,
            false,
        )
    }

    #[test]
    fn test_ws_connection_tracker() {
        let tracker = WsConnectionTracker::new();
        assert_eq!(tracker.connection_count(), 0);

        tracker.increment();
        assert_eq!(tracker.connection_count(), 1);

        tracker.increment();
        assert_eq!(tracker.connection_count(), 2);

        tracker.decrement();
        assert_eq!(tracker.connection_count(), 1);

        tracker.decrement();
        assert_eq!(tracker.connection_count(), 0);
    }

    #[test]
    fn test_ws_connection_tracker_default() {
        let tracker = WsConnectionTracker::default();
        assert_eq!(tracker.connection_count(), 0);
    }

    #[tokio::test]
    async fn test_handle_client_message_ping() {
        // Ping should produce a Pong on the direct channel
        let (direct_tx, mut direct_rx) = mpsc::channel(16);
        let state = make_test_state(None).await;

        let identity = test_request_identity("user1");
        handle_client_message(WsClientMessage::Ping, &state, &identity, &direct_tx).await;

        let response = direct_rx.recv().await.unwrap();
        assert!(matches!(response, WsServerMessage::Pong));
    }

    #[tokio::test]
    async fn test_handle_client_message_sends_to_agent() {
        // A Message should be forwarded to the agent's msg_tx
        let (agent_tx, mut agent_rx) = mpsc::channel(16);
        let state = make_test_state(Some(agent_tx)).await;
        let (direct_tx, _direct_rx) = mpsc::channel(16);
        let identity = test_request_identity("user1");

        handle_client_message(
            WsClientMessage::Message {
                content: "hello agent".to_string(),
                thread_id: Some("t1".to_string()),
            },
            &state,
            &identity,
            &direct_tx,
        )
        .await;

        let incoming = agent_rx.recv().await.unwrap();
        assert_eq!(incoming.content, "hello agent");
        assert_eq!(incoming.thread_id.as_deref(), Some("t1"));
        assert_eq!(incoming.channel, "gateway");
        assert_eq!(incoming.user_id, "user1");
    }

    #[tokio::test]
    async fn test_handle_client_message_no_channel() {
        // When msg_tx is None, should send an error back
        let state = make_test_state(None).await;
        let (direct_tx, mut direct_rx) = mpsc::channel(16);
        let identity = test_request_identity("user1");

        handle_client_message(
            WsClientMessage::Message {
                content: "hello".to_string(),
                thread_id: None,
            },
            &state,
            &identity,
            &direct_tx,
        )
        .await;

        let response = direct_rx.recv().await.unwrap();
        match response {
            WsServerMessage::Error { message } => {
                assert!(message.contains("not started"));
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[tokio::test]
    async fn test_handle_client_approval_approve() {
        let (agent_tx, mut agent_rx) = mpsc::channel(16);
        let state = make_test_state(Some(agent_tx)).await;
        let (direct_tx, _direct_rx) = mpsc::channel(16);
        let identity = test_request_identity("user1");

        let request_id = Uuid::new_v4();
        handle_client_message(
            WsClientMessage::Approval {
                request_id: request_id.to_string(),
                action: "approve".to_string(),
                thread_id: Some("thread-42".to_string()),
            },
            &state,
            &identity,
            &direct_tx,
        )
        .await;

        let incoming = agent_rx.recv().await.unwrap();
        // The content should be a serialized ExecApproval
        assert!(incoming.content.contains("ExecApproval"));
        // Thread should be forwarded onto the IncomingMessage.
        assert_eq!(incoming.thread_id.as_deref(), Some("thread-42"));
    }

    #[tokio::test]
    async fn test_handle_client_approval_invalid_action() {
        let state = make_test_state(None).await;
        let (direct_tx, mut direct_rx) = mpsc::channel(16);
        let identity = test_request_identity("user1");

        handle_client_message(
            WsClientMessage::Approval {
                request_id: Uuid::new_v4().to_string(),
                action: "maybe".to_string(),
                thread_id: None,
            },
            &state,
            &identity,
            &direct_tx,
        )
        .await;

        let response = direct_rx.recv().await.unwrap();
        match response {
            WsServerMessage::Error { message } => {
                assert!(message.contains("Unknown approval action"));
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[tokio::test]
    async fn test_handle_client_approval_invalid_uuid() {
        let state = make_test_state(None).await;
        let (direct_tx, mut direct_rx) = mpsc::channel(16);
        let identity = test_request_identity("user1");

        handle_client_message(
            WsClientMessage::Approval {
                request_id: "not-a-uuid".to_string(),
                action: "approve".to_string(),
                thread_id: None,
            },
            &state,
            &identity,
            &direct_tx,
        )
        .await;

        let response = direct_rx.recv().await.unwrap();
        match response {
            WsServerMessage::Error { message } => {
                assert!(message.contains("Invalid request_id"));
            }
            _ => panic!("Expected Error variant"),
        }
    }

    /// Helper to create a GatewayState for testing.
    async fn make_test_state(msg_tx: Option<mpsc::Sender<IncomingMessage>>) -> GatewayState {
        use crate::channels::web::sse::SseManager;

        GatewayState {
            msg_tx: tokio::sync::RwLock::new(msg_tx),
            sse: SseManager::new(),
            workspace: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: None,
            job_manager: None,
            prompt_queue: None,
            user_id: "test".to_string(),
            actor_id: "test".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
            llm_provider: None,
            llm_runtime: None,
            skill_registry: None,
            skill_catalog: None,
            chat_rate_limiter: crate::channels::web::server::RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: None,
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: None,
            channel_manager: None,
            cost_tracker: None,
        }
    }
}
