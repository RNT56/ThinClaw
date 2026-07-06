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
#[cfg(test)]
use crate::channels::IncomingMessage;
use crate::channels::web::handlers::chat::{
    active_thread_id_for_identity, clear_auth_mode_for_identity, gateway_submission_error,
};
use crate::channels::web::identity_helpers::{
    GatewayRequestIdentity, sse_event_visible_to_identity,
};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::{ModelInfo, SseEvent, WsClientMessage, WsServerMessage};
use thinclaw_gateway::web::devices::DeviceScope;
use thinclaw_gateway::web::identity::DeviceContext;
use thinclaw_gateway::web::submission::{build_gateway_message, submit_gateway_message};

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
    browser_origin: Option<String>,
    device_ctx: Option<DeviceContext>,
) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Device-token connections tear down on revocation (spec D-T5). Subscribe
    // synchronously (before any await) so a revoke racing the first frame is
    // still delivered; one guard drives the sender task (stops forwarding
    // events), the other drives the receiver loop (stops accepting frames).
    let device_id = device_ctx.as_ref().map(|d| d.device_id.clone());
    // While a device principal has this socket open it is watching events
    // in-app, so the first-party push notifier suppresses Alert pushes to it
    // (D-N1). The guard is moved into the sender task and dropped when that
    // task ends (client disconnect, revocation, or broadcast close).
    let stream_guard = device_id
        .as_deref()
        .map(|id| state.device_registry.stream_opened(id));
    let sender_revocation = crate::channels::web::handlers::chat::device_revocation_guard(
        device_id.clone(),
        Arc::clone(&state.device_registry),
    );
    let mut receiver_revocation = Box::pin(
        crate::channels::web::handlers::chat::device_revocation_guard(
            device_id,
            Arc::clone(&state.device_registry),
        ),
    );

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
        // Held for the connection's lifetime; dropped when this task exits.
        let _stream_guard = stream_guard;
        tokio::pin!(sender_revocation);
        loop {
            let msg = tokio::select! {
                _ = &mut sender_revocation => break, // device revoked: stop forwarding
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

    // Receiver loop: read client frames and route to agent, ending promptly
    // if the device is revoked so a revoked token can no longer send.
    loop {
        let frame = tokio::select! {
            _ = &mut receiver_revocation => break, // device revoked: stop accepting frames
            frame = ws_stream.next() => frame,
        };
        let Some(Ok(frame)) = frame else { break };
        match frame {
            Message::Text(text) => {
                let parsed: Result<WsClientMessage, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(client_msg) => {
                        handle_client_message(
                            client_msg,
                            &state,
                            &request_identity,
                            device_ctx.as_ref(),
                            &direct_tx,
                            browser_origin.as_deref(),
                        )
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
    device_ctx: Option<&DeviceContext>,
    direct_tx: &mpsc::Sender<WsServerMessage>,
    browser_origin: Option<&str>,
) {
    // Enforce device scopes at the WS-frame level: opening `/api/chat/ws`
    // only requires the `chat` scope, but individual frames map to distinct
    // surfaces. A device may chat and (with the `approvals` scope) approve;
    // every other operation — settings, secrets, extension auth, model
    // listing — is a never-grantable surface (D-T4) even over WS.
    if let Some(ctx) = device_ctx {
        let allowed = match &msg {
            WsClientMessage::Message { .. } => ctx.has_scope(DeviceScope::Chat),
            WsClientMessage::Approval { .. } => ctx.has_scope(DeviceScope::Approvals),
            WsClientMessage::Ping => true,
            _ => false,
        };
        if !allowed {
            let _ = direct_tx
                .send(WsServerMessage::Error {
                    message: "Forbidden: device token lacks the required scope".to_string(),
                })
                .await;
            return;
        }
    }

    match msg {
        WsClientMessage::Message { content, thread_id } => {
            let incoming = build_gateway_message(
                "gateway",
                request_identity,
                content,
                thread_id.as_deref(),
                browser_origin,
            );
            if let Err(error) = submit_gateway_message(state, incoming).await {
                let (_, message) = gateway_submission_error(error);
                let _ = direct_tx.send(WsServerMessage::Error { message }).await;
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

            // D-K4 / D-K3: a watch companion may only approve LOW-risk actions,
            // enforced server-side so a compromised watch client cannot approve
            // a destructive tool over the WebSocket. Mirrors the HTTP
            // `/api/chat/approval` gate; only an approve/always is gated (deny is
            // always allowed) and a cache miss fails closed (treated high-risk).
            if approved && device_ctx.is_some_and(|ctx| ctx.is_watch_companion()) {
                let cached_risk = state
                    .pending_approvals
                    .lock()
                    .ok()
                    .and_then(|cache| cache.get(&request_id).map(|entry| entry.risk));
                let is_low = matches!(
                    cached_risk,
                    Some(thinclaw_gateway::web::devices::ApprovalRisk::Low)
                );
                if !is_low {
                    let _ = direct_tx
                        .send(WsServerMessage::Error {
                            message: "this device may only approve low-risk actions".to_string(),
                        })
                        .await;
                    return;
                }
            }

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

            let msg = build_gateway_message(
                "gateway",
                request_identity,
                content,
                thread_id.as_deref(),
                browser_origin,
            );
            if let Err(error) = submit_gateway_message(state, msg).await {
                let _ = direct_tx
                    .send(WsServerMessage::Error {
                        message: gateway_submission_error(error).1,
                    })
                    .await;
            } else if let Ok(mut cache) = state.pending_approvals.lock() {
                // Drain the pull-endpoint cache so a resolved approval stops
                // showing as pending (mirrors chat_approval_handler).
                cache.remove(&request_id);
            }
        }
        WsClientMessage::AuthToken {
            extension_name,
            token,
        } => {
            if let Some(ref ext_mgr) = state.extension_manager {
                let thread_id = active_thread_id_for_identity(state, request_identity).await;
                match ext_mgr.auth(&extension_name, Some(&token)).await {
                    Ok(result)
                        if result.auth_status == "authenticated"
                            || result.auth_status == "no_auth_required" =>
                    {
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
                                auth_mode: Some(result.auth_mode),
                                auth_status: Some(result.auth_status),
                                shared_auth_provider: result.shared_auth_provider,
                                missing_scopes: result.missing_scopes,
                                thread_id,
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
                                auth_mode: result.auth_mode,
                                auth_status: result.auth_status,
                                shared_auth_provider: result.shared_auth_provider,
                                missing_scopes: result.missing_scopes,
                                thread_id,
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
                        .map(|name| ModelInfo {
                            is_primary: name == active,
                            name,
                        })
                        .collect(),
                    _ => vec![ModelInfo {
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
        handle_client_message(
            WsClientMessage::Ping,
            &state,
            &identity,
            None,
            &direct_tx,
            None,
        )
        .await;

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
            None,
            &direct_tx,
            Some("https://chat.example.com"),
        )
        .await;

        let incoming = agent_rx.recv().await.unwrap();
        assert_eq!(incoming.content, "hello agent");
        assert_eq!(incoming.thread_id.as_deref(), Some("t1"));
        assert_eq!(incoming.channel, "gateway");
        assert_eq!(incoming.user_id, "user1");
        assert_eq!(
            incoming
                .metadata
                .get("browser_origin")
                .and_then(|value| value.as_str()),
            Some("https://chat.example.com")
        );
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
            None,
            &direct_tx,
            None,
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
            None,
            &direct_tx,
            Some("https://chat.example.com"),
        )
        .await;

        let incoming = agent_rx.recv().await.unwrap();
        // The content should be a serialized ExecApproval
        assert!(incoming.content.contains("ExecApproval"));
        // Thread should be forwarded onto the IncomingMessage.
        assert_eq!(incoming.thread_id.as_deref(), Some("thread-42"));
    }

    /// Seed a pending-approval entry with the given risk so the watch-companion
    /// gate can look it up.
    fn seed_pending_approval(
        state: &GatewayState,
        request_id: &str,
        risk: thinclaw_gateway::web::devices::ApprovalRisk,
    ) {
        let entry = thinclaw_gateway::web::types::PendingApprovalEntry {
            request_id: request_id.to_string(),
            tool_name: "execute_shell".to_string(),
            description: "run a command".to_string(),
            parameters: "{}".to_string(),
            risk,
            thread_id: Some("thread-42".to_string()),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        state
            .pending_approvals
            .lock()
            .unwrap()
            .insert(request_id.to_string(), entry);
    }

    fn watch_companion_ctx() -> DeviceContext {
        DeviceContext::with_class(
            "watch-1",
            vec![DeviceScope::Chat, DeviceScope::Approvals],
            thinclaw_gateway::web::devices::DevicePlatform::Watchos,
            true,
        )
    }

    #[tokio::test]
    async fn test_watch_companion_high_risk_ws_approve_is_refused() {
        // D-K4: a watch companion must not be able to approve a HIGH-risk tool
        // over the WebSocket. The gate is server-side, not just UI.
        let (agent_tx, mut agent_rx) = mpsc::channel(16);
        let state = make_test_state(Some(agent_tx)).await;
        let (direct_tx, mut direct_rx) = mpsc::channel(16);
        let identity = test_request_identity("user1");
        let request_id = Uuid::new_v4().to_string();
        seed_pending_approval(
            &state,
            &request_id,
            thinclaw_gateway::web::devices::ApprovalRisk::High,
        );
        let ctx = watch_companion_ctx();

        handle_client_message(
            WsClientMessage::Approval {
                request_id: request_id.clone(),
                action: "approve".to_string(),
                thread_id: Some("thread-42".to_string()),
            },
            &state,
            &identity,
            Some(&ctx),
            &direct_tx,
            None,
        )
        .await;

        // Refused with an error, and NO approval submitted to the agent.
        match direct_rx.recv().await.unwrap() {
            WsServerMessage::Error { message } => {
                assert!(message.contains("low-risk"), "got: {message}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
        assert!(
            agent_rx.try_recv().is_err(),
            "high-risk approval must not reach the agent loop"
        );
    }

    #[tokio::test]
    async fn test_watch_companion_low_risk_ws_approve_and_deny_pass() {
        let (agent_tx, mut agent_rx) = mpsc::channel(16);
        let state = make_test_state(Some(agent_tx)).await;
        let (direct_tx, _direct_rx) = mpsc::channel(16);
        let identity = test_request_identity("user1");
        let ctx = watch_companion_ctx();

        // Low-risk approve is allowed through.
        let low_id = Uuid::new_v4().to_string();
        seed_pending_approval(
            &state,
            &low_id,
            thinclaw_gateway::web::devices::ApprovalRisk::Low,
        );
        handle_client_message(
            WsClientMessage::Approval {
                request_id: low_id,
                action: "approve".to_string(),
                thread_id: Some("thread-42".to_string()),
            },
            &state,
            &identity,
            Some(&ctx),
            &direct_tx,
            None,
        )
        .await;
        assert!(
            agent_rx
                .recv()
                .await
                .unwrap()
                .content
                .contains("ExecApproval"),
            "low-risk approve should reach the agent"
        );

        // Deny is always allowed, even for a HIGH-risk entry.
        let high_id = Uuid::new_v4().to_string();
        seed_pending_approval(
            &state,
            &high_id,
            thinclaw_gateway::web::devices::ApprovalRisk::High,
        );
        handle_client_message(
            WsClientMessage::Approval {
                request_id: high_id,
                action: "deny".to_string(),
                thread_id: Some("thread-42".to_string()),
            },
            &state,
            &identity,
            Some(&ctx),
            &direct_tx,
            None,
        )
        .await;
        assert!(
            agent_rx
                .recv()
                .await
                .unwrap()
                .content
                .contains("ExecApproval"),
            "deny should always reach the agent regardless of risk"
        );
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
            None,
            &direct_tx,
            None,
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
            None,
            &direct_tx,
            None,
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
            context_manager: None,
            scheduler: tokio::sync::RwLock::new(None),
            user_id: "test".to_string(),
            actor_id: "test".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
            llm_provider: None,
            llm_runtime: None,
            skill_registry: None,
            skill_catalog: None,
            skill_remote_hub: None,
            skill_quarantine: None,
            chat_rate_limiter: crate::channels::web::server::RateLimiter::new(30, 60),
            pair_complete_rate_limiter: crate::channels::web::server::RateLimiter::new(10, 300),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: None,
            repo_project_supervisor: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: None,
            channel_manager: None,
            hooks: None,
            cost_tracker: None,
            response_cache: None,
            device_registry: crate::channels::web::server::test_device_registry(),
            pending_approvals: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
        }
    }
}
