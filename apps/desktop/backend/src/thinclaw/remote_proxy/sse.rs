//! Server-Sent Events subscription: background reconnecting loop and stream
//! consumer that re-emits remote gateway events on the desktop Tauri bus.

use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE};
use tracing::{error, info, warn};

use super::core::{ConnectionState, RemoteGatewayProxy};

const MAX_SSE_LINE_BYTES: usize = 512 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 1024 * 1024;

/// Incremental SSE decoder with bounded line and event storage. Keeping the
/// input as bytes until an event boundary preserves UTF-8 code points split
/// across transport chunks.
struct SseDecoder {
    line: Vec<u8>,
    data: Vec<u8>,
    skip_lf: bool,
    first_line: bool,
    max_line_bytes: usize,
    max_event_bytes: usize,
}

impl SseDecoder {
    fn new() -> Self {
        Self {
            line: Vec::new(),
            data: Vec::new(),
            skip_lf: false,
            first_line: true,
            max_line_bytes: MAX_SSE_LINE_BYTES,
            max_event_bytes: MAX_SSE_EVENT_BYTES,
        }
    }

    #[cfg(test)]
    fn with_limits(max_line_bytes: usize, max_event_bytes: usize) -> Self {
        Self {
            max_line_bytes,
            max_event_bytes,
            ..Self::new()
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        let mut events = Vec::new();
        for &byte in chunk {
            if self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }

            match byte {
                b'\r' => {
                    self.process_current_line(&mut events)?;
                    self.skip_lf = true;
                }
                b'\n' => self.process_current_line(&mut events)?,
                _ => {
                    if self.line.len() >= self.max_line_bytes {
                        return Err(format!(
                            "SSE line exceeds the {}-byte limit",
                            self.max_line_bytes
                        ));
                    }
                    self.line.push(byte);
                }
            }
        }
        Ok(events)
    }

    fn finish(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let mut events = Vec::new();
        if !self.line.is_empty() {
            self.process_current_line(&mut events)?;
        }
        self.dispatch_event(&mut events);
        Ok(events)
    }

    fn process_current_line(&mut self, events: &mut Vec<Vec<u8>>) -> Result<(), String> {
        let mut line = std::mem::take(&mut self.line);
        if self.first_line {
            self.first_line = false;
            if line.starts_with(&[0xEF, 0xBB, 0xBF]) {
                line.drain(..3);
            }
        }

        if line.is_empty() {
            self.dispatch_event(events);
            return Ok(());
        }
        if line[0] == b':' {
            return Ok(());
        }

        let (field, mut value) = match line.iter().position(|byte| *byte == b':') {
            Some(index) => (&line[..index], &line[index + 1..]),
            None => (line.as_slice(), &[][..]),
        };
        if value.first() == Some(&b' ') {
            value = &value[1..];
        }
        if field != b"data" {
            return Ok(());
        }

        let projected = self
            .data
            .len()
            .checked_add(value.len())
            .and_then(|length| length.checked_add(1))
            .ok_or_else(|| "SSE event size overflow".to_string())?;
        if projected > self.max_event_bytes {
            return Err(format!(
                "SSE event exceeds the {}-byte limit",
                self.max_event_bytes
            ));
        }
        self.data.extend_from_slice(value);
        self.data.push(b'\n');
        Ok(())
    }

    fn dispatch_event(&mut self, events: &mut Vec<Vec<u8>>) {
        if self.data.is_empty() {
            return;
        }
        self.data.pop(); // Remove the final newline inserted for the last data field.
        events.push(std::mem::take(&mut self.data));
    }
}

fn has_event_stream_content_type(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
}

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
        let mut handle_slot = self.inner.sse_handle.lock().await;
        if let Some(handle) = handle_slot.take() {
            handle.abort();
        }

        *self.inner.state.write().await = ConnectionState::Connected;

        let proxy = self.clone();
        let handle = tokio::spawn(async move {
            proxy.sse_loop(app_handle).await;
        });

        *handle_slot = Some(handle);
        info!(
            "[remote_proxy] SSE subscription started for {}",
            self.inner.base_url
        );
        Ok(())
    }

    /// Stop the background SSE subscription task.
    pub async fn stop_sse_subscription(&self) {
        if let Some(handle) = self.inner.sse_handle.lock().await.take() {
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
                .sse_client
                .get(&url)
                .header(AUTHORIZATION, self.auth_header())
                .header("Accept", "text/event-stream")
                .header(CACHE_CONTROL, "no-cache")
                .send()
                .await;

            match result {
                Err(e) => {
                    warn!(
                        "[remote_proxy] SSE connection failed: {}. Retrying in {}s",
                        e, backoff_secs
                    );
                }
                Ok(response)
                    if response.status() == reqwest::StatusCode::UNAUTHORIZED
                        || response.status() == reqwest::StatusCode::FORBIDDEN =>
                {
                    error!("[remote_proxy] SSE authorization was rejected; subscription stopped");
                    *self.inner.state.write().await = ConnectionState::Disconnected;
                    let _ = app_handle.emit(
                        "thinclaw-event",
                        &crate::thinclaw::ui_types::UiEvent::Disconnected {
                            reason: "Remote gateway authorization was rejected".to_string(),
                        },
                    );
                    return;
                }
                Ok(response) if !response.status().is_success() => {
                    let status = response.status();
                    error!(
                        "[remote_proxy] SSE endpoint returned HTTP {}. Retrying in {}s",
                        status, backoff_secs
                    );
                }
                Ok(response) if !has_event_stream_content_type(&response) => {
                    error!(
                        "[remote_proxy] SSE endpoint returned a non-event-stream response. Retrying in {}s",
                        backoff_secs
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
        let mut decoder = SseDecoder::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("SSE stream read error: {}", e))?;
            for data in decoder.push(&chunk)? {
                emit_sse_data(&data, app_handle);
            }
        }
        for data in decoder.finish()? {
            emit_sse_data(&data, app_handle);
        }

        Ok(())
    }
}

fn emit_sse_data(data: &[u8], app_handle: &tauri::AppHandle) {
    use tauri::Emitter;

    if data.is_empty() || data == b"[DONE]" {
        return;
    }
    let data = match std::str::from_utf8(data) {
        Ok(data) => data,
        Err(_) => {
            warn!("[remote_proxy] Discarded a non-UTF-8 SSE event");
            return;
        }
    };

    // Prefer UiEvent for remote gateways that already speak the desktop
    // contract. Otherwise normalize gateway (`type`) events into the same bus.
    match serde_json::from_str::<crate::thinclaw::ui_types::UiEvent>(data) {
        Ok(event) => {
            if let Err(error) = app_handle.emit("thinclaw-event", &event) {
                warn!("[remote_proxy] Failed to emit Tauri event: {error}");
            }
        }
        Err(_) => match serde_json::from_str::<serde_json::Value>(data) {
            Ok(raw_json) => {
                for event in crate::thinclaw::event_mapping::gateway_sse_to_ui_events(raw_json) {
                    if let Err(error) = app_handle.emit("thinclaw-event", &event) {
                        warn!("[remote_proxy] Failed to emit mapped gateway event: {error}");
                    }
                }
            }
            Err(error) => warn!("[remote_proxy] Failed to parse SSE data as JSON: {error}"),
        },
    }
}

#[cfg(test)]
mod decoder_tests {
    use super::SseDecoder;

    #[test]
    fn preserves_utf8_split_across_transport_chunks() {
        let input = "data: {\"content\":\"hello 🦀\"}\n\n".as_bytes();
        let crab = input
            .windows(4)
            .position(|window| window == "🦀".as_bytes())
            .expect("crab bytes");
        let mut decoder = SseDecoder::new();
        assert!(decoder.push(&input[..crab + 1]).unwrap().is_empty());
        let events = decoder.push(&input[crab + 1..]).unwrap();
        assert_eq!(events, vec!["{\"content\":\"hello 🦀\"}".as_bytes()]);
    }

    #[test]
    fn combines_multiline_data_and_supports_all_line_endings() {
        let mut decoder = SseDecoder::new();
        let events = decoder
            .push(b": keepalive\rdata: {\"value\":\r\ndata: 1}\n\r")
            .unwrap();
        assert_eq!(events, vec![b"{\"value\":\n1}".to_vec()]);
    }

    #[test]
    fn strips_bom_and_dispatches_final_event_at_eof() {
        let mut decoder = SseDecoder::new();
        assert!(decoder
            .push(b"\xEF\xBB\xBFdata:{\"ok\":true}")
            .unwrap()
            .is_empty());
        assert_eq!(decoder.finish().unwrap(), vec![b"{\"ok\":true}".to_vec()]);
    }

    #[test]
    fn enforces_line_and_event_limits() {
        let mut line_limited = SseDecoder::with_limits(4, 32);
        assert!(line_limited.push(b"12345").unwrap_err().contains("line"));

        let mut event_limited = SseDecoder::with_limits(32, 5);
        assert!(event_limited
            .push(b"data: 12345\n")
            .unwrap_err()
            .contains("event"));
    }
}
