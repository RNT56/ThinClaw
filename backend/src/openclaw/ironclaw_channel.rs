//! Tauri channel adapter for IronClaw.
//!
//! Implements `ironclaw::channels::Channel` to bridge the IronClaw agent
//! engine with Tauri's event system. StatusUpdate emissions are converted
//! to UiEvent and emitted via `AppHandle::emit`.

use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Wry};
use tokio::sync::{mpsc, Mutex};

use ironclaw::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use ironclaw::error::ChannelError;

use super::ironclaw_types::status_to_ui_event;
use super::sanitizer::strip_llm_tokens;
use super::ui_types::UiEvent;

/// Channel name used for routing. Must match what `api::chat` hardcodes.
const CHANNEL_NAME: &str = "tauri";

/// Event name emitted to the frontend (matches existing `listen("openclaw-event")`)
const EMIT_EVENT: &str = "openclaw-event";

/// Tauri-native channel implementation for IronClaw.
///
/// The channel holds an `mpsc::Sender` that the bridge uses to inject
/// messages from Tauri commands into the agent's message stream.
pub struct TauriChannel {
    app_handle: AppHandle<Wry>,
    /// Receiver — taken once in `start()` and converted to a MessageStream.
    inject_rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
}

impl TauriChannel {
    /// Create a new TauriChannel.
    ///
    /// Returns `(channel, sender)` — the sender is stored in `IronClawState`
    /// for Tauri commands to inject messages.
    pub fn new(app_handle: AppHandle<Wry>) -> (Self, mpsc::Sender<IncomingMessage>) {
        let (tx, rx) = mpsc::channel(64);
        let channel = Self {
            app_handle,
            inject_rx: Mutex::new(Some(rx)),
        };
        (channel, tx)
    }

    /// Emit a UiEvent to the frontend.
    fn emit_ui_event(&self, event: &UiEvent) {
        if let Err(e) = self.app_handle.emit(EMIT_EVENT, event) {
            tracing::warn!("Failed to emit UI event: {}", e);
        }
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
        // Extract session_key from the message's thread_id
        let session_key = msg.thread_id.as_deref().unwrap_or("default");

        let run_id = msg
            .metadata
            .get("run_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // `respond` is called with the final assistant text
        let event = UiEvent::AssistantFinal {
            session_key: session_key.to_string(),
            run_id,
            message_id: msg.id.to_string(),
            text: strip_llm_tokens(&response.content),
            usage: None, // IronClaw doesn't pass usage through OutgoingResponse
        };
        self.emit_ui_event(&event);

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Extract routing info from metadata
        let session_key = metadata
            .get("thread_id")
            .or_else(|| metadata.get("session_key"))
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let run_id = metadata.get("run_id").and_then(|v| v.as_str());

        let message_id = metadata
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        if let Some(event) = status_to_ui_event(status, session_key, run_id, message_id) {
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
