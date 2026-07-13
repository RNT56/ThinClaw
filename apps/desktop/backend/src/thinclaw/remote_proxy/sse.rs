//! Server-Sent Events subscription: background reconnecting loop and stream
//! consumer that re-emits remote gateway events on the desktop Tauri bus.

use std::time::Duration;

use tracing::{error, info, warn};

use super::core::{ConnectionState, RemoteGatewayProxy};

const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;

impl RemoteGatewayProxy {
    /// Subscribe to the remote gateway's SSE event stream and re-emit
    /// all events as Tauri `thinclaw-event` emissions.
    ///
    /// This runs as a background task. Events from the remote ThinClaw
    /// are forwarded directly to the frontend — the UI sees no difference
    /// between local and remote agent events.
    ///
    /// Auto-reconnects on disconnect (exponential backoff, max 30s).
    pub async fn start_sse_subscription(&self, app_handle: tauri::AppHandle) -> Result<(), String> {
        // Stop existing subscription first
        self.stop_sse_subscription().await;

        *self.inner.state.write().await = ConnectionState::Connected;

        let proxy = self.clone();
        let handle = tokio::spawn(async move {
            proxy.sse_loop(app_handle).await;
        });

        *self.inner.sse_handle.write().await = Some(handle);
        info!(
            "[remote_proxy] SSE subscription started for {}",
            self.inner.base_url
        );
        Ok(())
    }

    /// Stop the background SSE subscription task.
    pub async fn stop_sse_subscription(&self) {
        if let Some(handle) = self.inner.sse_handle.write().await.take() {
            handle.abort();
            info!("[remote_proxy] SSE subscription stopped");
        }
        *self.inner.state.write().await = ConnectionState::Disconnected;
    }

    /// Current connection state.
    pub async fn connection_state(&self) -> ConnectionState {
        self.inner.state.read().await.clone()
    }

    /// Background SSE loop with auto-reconnect.
    async fn sse_loop(&self, app_handle: tauri::AppHandle) {
        use tauri::Emitter;

        let mut backoff_secs: u64 = 1;
        const MAX_BACKOFF: u64 = 30;

        loop {
            *self.inner.state.write().await = ConnectionState::Reconnecting;

            let url = self.url("/api/chat/events");
            info!("[remote_proxy] Connecting to SSE: {}", url);

            let result = self
                .inner
                .client
                .get(&url)
                .header(reqwest::header::AUTHORIZATION, self.auth_header())
                .header("Accept", "text/event-stream")
                .header("Cache-Control", "no-cache")
                // No global timeout — SSE is a long-lived connection
                .timeout(Duration::MAX)
                .send()
                .await;

            match result {
                Err(e) => {
                    warn!(
                        "[remote_proxy] SSE connection failed: {}. Retrying in {}s",
                        e, backoff_secs
                    );
                }
                Ok(response) if !response.status().is_success() => {
                    let status = response.status();
                    error!(
                        "[remote_proxy] SSE endpoint returned HTTP {}. Retrying in {}s",
                        status, backoff_secs
                    );
                }
                Ok(response) => {
                    *self.inner.state.write().await = ConnectionState::Connected;
                    backoff_secs = 1; // Reset backoff on successful connect

                    info!("[remote_proxy] SSE stream connected");

                    // Emit Connected event to frontend
                    let _ = app_handle.emit(
                        "thinclaw-event",
                        &crate::thinclaw::ui_types::UiEvent::Connected { protocol: 1 },
                    );

                    // Stream SSE events
                    let stream_result = self.consume_sse_stream(response, &app_handle).await;

                    match stream_result {
                        Ok(()) => {
                            info!("[remote_proxy] SSE stream ended cleanly. Reconnecting...");
                        }
                        Err(e) => {
                            warn!("[remote_proxy] SSE stream error: {}. Reconnecting...", e);
                        }
                    }

                    // Emit Disconnected to frontend on stream end
                    let _ = app_handle.emit(
                        "thinclaw-event",
                        &crate::thinclaw::ui_types::UiEvent::Disconnected {
                            reason: "Remote stream ended — reconnecting".to_string(),
                        },
                    );
                }
            }

            *self.inner.state.write().await = ConnectionState::Reconnecting;
            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF);
        }
    }

    /// Consume a live SSE response stream and forward events to Tauri.
    ///
    /// The ThinClaw gateway sends events in SSE format:
    ///   data: {"type":"stream_chunk","thread_id":"...","content":"..."}\n\n
    ///
    /// We normalize each `data:` line onto the desktop `UiEvent` contract and
    /// re-emit it on the same Tauri bus as local mode.
    async fn consume_sse_stream(
        &self,
        response: reqwest::Response,
        app_handle: &tauri::AppHandle,
    ) -> Result<(), String> {
        use futures_util::StreamExt;

        let mut stream = response.bytes_stream();
        let mut buffer = Vec::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("SSE stream read error: {}", e))?;
            // Split before appending so a large network chunk containing many
            // small events never creates one large intermediate buffer. Bytes
            // are decoded only after a complete line arrives, preserving UTF-8
            // sequences that cross transport chunk boundaries.
            for segment in chunk.split_inclusive(|byte| *byte == b'\n') {
                if buffer.len().saturating_add(segment.len()) > MAX_SSE_LINE_BYTES {
                    return Err(format!(
                        "SSE event exceeded the {MAX_SSE_LINE_BYTES}-byte safety limit"
                    ));
                }
                buffer.extend_from_slice(segment);
                if segment.ends_with(b"\n") {
                    forward_sse_line(&buffer, app_handle)?;
                    buffer.clear();
                }
            }
        }

        if !buffer.is_empty() {
            forward_sse_line(&buffer, app_handle)?;
        }

        Ok(())
    }
}

fn forward_sse_line(line: &[u8], app_handle: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Emitter;

    let mut end = line.len();
    if end > 0 && line[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && line[end - 1] == b'\r' {
        end -= 1;
    }
    let line = std::str::from_utf8(&line[..end])
        .map_err(|_| "Remote SSE stream contained invalid UTF-8".to_string())?;
    let Some(data) = line.strip_prefix("data:").map(str::trim) else {
        return Ok(());
    };
    if data.is_empty() || data == "[DONE]" {
        return Ok(());
    }

    // Prefer UiEvent for remote gateways that already speak the desktop
    // contract. Otherwise normalize root gateway (`type`) events.
    match serde_json::from_str::<crate::thinclaw::ui_types::UiEvent>(data) {
        Ok(event) => {
            if let Err(error) = app_handle.emit("thinclaw-event", &event) {
                warn!("[remote_proxy] Failed to emit Tauri event: {}", error);
            }
        }
        Err(_) => match serde_json::from_str::<serde_json::Value>(data) {
            Ok(raw_json) => {
                for event in crate::thinclaw::event_mapping::gateway_sse_to_ui_events(raw_json) {
                    if let Err(error) = app_handle.emit("thinclaw-event", &event) {
                        warn!(
                            "[remote_proxy] Failed to emit mapped gateway event: {}",
                            error
                        );
                    }
                }
            }
            Err(error) => warn!("[remote_proxy] Failed to parse SSE data as JSON: {}", error),
        },
    }
    Ok(())
}
