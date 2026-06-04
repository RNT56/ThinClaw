//! Tauri channel adapter for ThinClaw.
//!
//! Implements `thinclaw_core::channels::Channel` to bridge the ThinClaw runtime
//! engine with Tauri's event system. StatusUpdate emissions are converted
//! to UiEvent and emitted via `AppHandle::emit`.
//!
//! ## Session routing (multi-session safe)
//!
//! Session routing uses a two-tier strategy:
//!
//! 1. **Primary:** Read `session_key` / `thread_id` from the StatusUpdate
//!    metadata (injected by ThinClaw's agent loop).
//! 2. **Fallback:** If a status variant carries its own thread ID (auth
//!    events do), use that.
//! 3. **Default:** Route unscoped status updates to `agent:main`.
//!
//! This replaces the old single-variable `session_context: Arc<RwLock<String>>`
//! which was racy under concurrent sessions: setting session B's context
//! could misroute session A's in-flight events. Final responses still use the
//! original message thread ID.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Manager, Wry};
use tokio::sync::{mpsc, Mutex, RwLock};

use thinclaw_core::channels::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};
use thinclaw_core::error::ChannelError;

use super::event_mapping::{routing_from_status, status_to_ui_event};
use super::sanitizer::strip_llm_tokens;
use super::ui_types::UiEvent;

/// Channel name used for routing. Must match what `api::chat` hardcodes.
const CHANNEL_NAME: &str = "tauri";

/// Event name emitted to the frontend (matches existing `listen("thinclaw-event")`)
const EMIT_EVENT: &str = "thinclaw-event";

/// Tauri-native channel implementation for ThinClaw.
///
/// The channel holds an `mpsc::Sender` that the bridge uses to inject
/// messages from Tauri commands into the agent's message stream.
pub struct TauriChannel {
    app_handle: AppHandle<Wry>,
    /// Receiver — taken once in `start()` and converted to a MessageStream.
    inject_rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    /// Active session tracking — maps session_key → activation timestamp (ms).
    /// The most recently activated session is used as fallback when metadata
    /// doesn't include a session key.
    active_sessions: Arc<RwLock<HashMap<String, u64>>>,
}

impl TauriChannel {
    /// Create a new TauriChannel.
    ///
    /// Returns `(channel, sender, active_sessions)` — the sender is stored in
    /// `ThinClawRuntimeState` for Tauri commands to inject messages, and the
    /// active_sessions Arc is shared so commands can register sessions.
    pub fn new(
        app_handle: AppHandle<Wry>,
    ) -> (
        Self,
        mpsc::Sender<IncomingMessage>,
        Arc<RwLock<HashMap<String, u64>>>,
    ) {
        let (tx, rx) = mpsc::channel(64);

        // Seed with the default session
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let mut initial_sessions = HashMap::new();
        initial_sessions.insert("agent:main".to_string(), now);
        let active_sessions = Arc::new(RwLock::new(initial_sessions));

        let channel = Self {
            app_handle,
            inject_rx: Mutex::new(Some(rx)),
            active_sessions: active_sessions.clone(),
        };
        (channel, tx, active_sessions)
    }

    /// Emit a UiEvent to the frontend.
    fn emit_ui_event(&self, event: &UiEvent) {
        if let Err(e) = self.app_handle.emit(EMIT_EVENT, event) {
            tracing::warn!("Failed to emit UI event: {}", e);
        }
    }

    /// Set tray icon to active state (with dot badge) and schedule auto-reset.
    fn set_tray_active(&self) {
        if let Some(tray_state) = self
            .app_handle
            .try_state::<std::sync::Arc<crate::setup::tray::TrayState>>()
        {
            let _ = tray_state
                .tray
                .set_icon(Some(tray_state.active_icon.clone()));
            let _ = tray_state
                .tray
                .set_tooltip(Some("ThinClaw Desktop — processing..."));

            // Cancel previous reset timer and schedule a new one
            let tray_arc = std::sync::Arc::clone(&tray_state);
            tokio::spawn(async move {
                // Cancel previous reset
                if let Some(prev) = tray_arc.reset_handle.lock().await.take() {
                    prev.abort();
                }
                // Schedule reset after 3 seconds of no activity
                let tray_reset = std::sync::Arc::clone(&tray_arc);
                let handle = tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    let _ = tray_reset.tray.set_icon(Some(tray_reset.idle_icon.clone()));
                    let _ = tray_reset.tray.set_tooltip(Some("ThinClaw Desktop"));
                });
                *tray_arc.reset_handle.lock().await = Some(handle);
            });
        }
    }

    /// Set tray icon to idle state immediately.
    fn set_tray_idle(&self) {
        if let Some(tray_state) = self
            .app_handle
            .try_state::<std::sync::Arc<crate::setup::tray::TrayState>>()
        {
            // Cancel any pending reset timer
            let tray_arc = std::sync::Arc::clone(&tray_state);
            tokio::spawn(async move {
                if let Some(prev) = tray_arc.reset_handle.lock().await.take() {
                    prev.abort();
                }
                let _ = tray_arc.tray.set_icon(Some(tray_arc.idle_icon.clone()));
                let _ = tray_arc.tray.set_tooltip(Some("ThinClaw Desktop"));
            });
        }
    }

    /// Get the most recently activated session key (fallback for routing).
    async fn most_recent_session(&self) -> String {
        let sessions = self.active_sessions.read().await;
        sessions
            .iter()
            .max_by_key(|(_, ts)| *ts)
            .map(|(key, _)| key.clone())
            .unwrap_or_else(|| "agent:main".to_string())
    }
}

#[async_trait]
impl Channel for TauriChannel {
    fn name(&self) -> &str {
        CHANNEL_NAME
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let rx = self
            .inject_rx
            .lock()
            .await
            .take()
            .ok_or_else(|| ChannelError::StartupFailed {
                name: CHANNEL_NAME.into(),
                reason: "start() already called (receiver consumed)".into(),
            })?;

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Session routing: prefer thread_id from message, fall back to most recent session
        // (must match the same routing logic as send_status)
        let session_key = if let Some(ref tid) = msg.thread_id {
            tid.clone()
        } else {
            self.most_recent_session().await
        };

        let run_id = msg
            .metadata
            .get("run_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // `respond` is called with the final assistant text
        let event = UiEvent::AssistantFinal {
            session_key,
            run_id,
            message_id: msg.id.to_string(),
            text: strip_llm_tokens(&response.content),
            usage: None, // ThinClaw doesn't pass usage through OutgoingResponse
        };
        self.emit_ui_event(&event);

        // Reset tray icon to idle when response is sent
        self.set_tray_idle();

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let (resolved_session, run_id, message_id) = {
            let (session, run_id, message_id) = routing_from_status(&status, metadata);
            (
                session,
                run_id.map(ToString::to_string),
                message_id.to_string(),
            )
        };

        // Animate tray icon on activity events
        if matches!(
            &status,
            StatusUpdate::Thinking(_)
                | StatusUpdate::ToolStarted { .. }
                | StatusUpdate::AgentMessage { .. }
                | StatusUpdate::SubagentSpawned { .. }
                | StatusUpdate::SubagentProgress { .. }
        ) {
            self.set_tray_active();
        }

        // ── Register subagent lifecycle in the sub_agent_registry ────────
        // This ensures automation subagents show up properly in the Presence
        // "Sub-Agents" section and in listChildSessions() rather than as
        // phantom top-level sessions.
        match &status {
            StatusUpdate::SubagentSpawned {
                agent_id,
                name,
                task,
                ..
            } => {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as f64;
                let child = super::commands::types::ChildSessionInfo {
                    session_key: agent_id.clone(),
                    task: format!("[{}] {}", name, task),
                    status: "running".to_string(),
                    spawned_at: now_ms,
                    result_summary: None,
                };
                super::commands::rpc_orchestration::sub_agent_registry::register(
                    &resolved_session,
                    child,
                )
                .await;
            }
            StatusUpdate::SubagentCompleted {
                agent_id,
                success,
                response,
                ..
            } => {
                let status_str = if *success { "completed" } else { "failed" };
                let preview = if response.len() > 200 {
                    let mut end = 200;
                    while !response.is_char_boundary(end) {
                        end -= 1;
                    }
                    Some(format!("{}…", &response[..end]))
                } else if response.is_empty() {
                    None
                } else {
                    Some(response.clone())
                };
                super::commands::rpc_orchestration::sub_agent_registry::update_status(
                    agent_id,
                    status_str,
                    preview.as_deref(),
                )
                .await;
            }
            _ => {}
        }

        if let Some(event) =
            status_to_ui_event(status, &resolved_session, run_id.as_deref(), &message_id)
        {
            self.emit_ui_event(&event);
        }

        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Broadcast as a plain text event — used for heartbeat/self-repair notifications
        let event = UiEvent::AssistantFinal {
            session_key: "system".into(),
            run_id: None,
            message_id: uuid::Uuid::new_v4().to_string(),
            text: strip_llm_tokens(&response.content),
            usage: None,
        };
        self.emit_ui_event(&event);
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        // Tauri is always "healthy" as long as the app is running
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        tracing::info!("TauriChannel shutting down");
        Ok(())
    }
}
