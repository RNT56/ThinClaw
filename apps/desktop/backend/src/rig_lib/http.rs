use futures::StreamExt;
use serde::de::DeserializeOwned;
use std::pin::Pin;
use std::time::Duration;

pub const MAX_LLM_JSON_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_LLM_ERROR_BYTES: usize = 16 * 1024;
pub const MAX_LLM_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 2 * 1024 * 1024;
const MAX_SSE_STREAM_BYTES: usize = 64 * 1024 * 1024;

pub fn client(local_only: bool, streaming: bool) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(if streaming {
            Duration::from_secs(30 * 60)
        } else {
            Duration::from_secs(5 * 60)
        })
        .redirect(reqwest::redirect::Policy::none());
    if local_only {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|error| format!("Could not build LLM HTTP client: {error}"))
}

pub fn transport_error(context: &str, error: reqwest::Error) -> String {
    format!("{context}: {}", error.without_url())
}

fn safe_error_excerpt(text: &str) -> String {
    let mut excerpt = String::with_capacity(text.len().min(2_048));
    for character in text.chars() {
        if excerpt.len() >= 2_048 {
            break;
        }
        if !character.is_control() || matches!(character, '\n' | '\r' | '\t') {
            excerpt.push(character);
        }
    }
    excerpt.trim().to_string()
}

pub async fn checked_response(
    response: reqwest::Response,
    provider: &str,
) -> Result<reqwest::Response, String> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if matches!(status.as_u16(), 401 | 403) {
        return Err(format!(
            "{provider} rejected the configured API credential (HTTP {status})"
        ));
    }
    let detail = thinclaw_core::http_response::bounded_text(response, MAX_LLM_ERROR_BYTES)
        .await
        .ok()
        .map(|text| safe_error_excerpt(&text))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "no bounded error detail".to_string());
    Err(format!(
        "{provider} request failed with HTTP {status}: {detail}"
    ))
}

pub async fn bounded_json<T: DeserializeOwned>(
    response: reqwest::Response,
    provider: &str,
) -> Result<T, String> {
    thinclaw_core::http_response::bounded_json(response, MAX_LLM_JSON_BYTES)
        .await
        .map_err(|error| format!("Invalid bounded {provider} response: {error}"))
}

pub fn bounded_json_body<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("Could not serialize LLM request: {error}"))?;
    if bytes.len() > MAX_LLM_REQUEST_BYTES {
        return Err(format!(
            "LLM request exceeds the {MAX_LLM_REQUEST_BYTES}-byte limit"
        ));
    }
    Ok(bytes)
}

#[derive(Default)]
struct SseLimitState {
    event_bytes: usize,
    total_bytes: usize,
    line_has_data: bool,
    previous_was_cr: bool,
    failed: bool,
}

impl SseLimitState {
    fn observe(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.total_bytes = self
            .total_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| "LLM event stream size overflow".to_string())?;
        if self.total_bytes > MAX_SSE_STREAM_BYTES {
            return Err(format!(
                "LLM event stream exceeds the {MAX_SSE_STREAM_BYTES}-byte limit"
            ));
        }
        for byte in bytes {
            self.event_bytes = self
                .event_bytes
                .checked_add(1)
                .ok_or_else(|| "LLM event size overflow".to_string())?;
            match *byte {
                b'\r' => {
                    if self.line_has_data {
                        self.line_has_data = false;
                    } else {
                        self.event_bytes = 0;
                    }
                    self.previous_was_cr = true;
                }
                b'\n' if self.previous_was_cr => {
                    // The LF is the second half of a CRLF terminator. If the
                    // preceding CR ended a blank line, `event_bytes` was reset
                    // and this byte must not start a phantom one-byte event.
                    if self.event_bytes == 1 {
                        self.event_bytes = 0;
                    }
                    self.previous_was_cr = false;
                }
                b'\n' => {
                    if self.line_has_data {
                        self.line_has_data = false;
                    } else {
                        self.event_bytes = 0;
                    }
                }
                _ => {
                    self.previous_was_cr = false;
                    self.line_has_data = true;
                }
            }
            if self.event_bytes > MAX_SSE_EVENT_BYTES {
                return Err(format!(
                    "LLM event exceeds the {MAX_SSE_EVENT_BYTES}-byte limit"
                ));
            }
        }
        Ok(())
    }
}

pub fn bounded_sse_bytes(
    response: reqwest::Response,
) -> Pin<Box<dyn futures::Stream<Item = Result<bytes::Bytes, String>> + Send>> {
    Box::pin(
        response
            .bytes_stream()
            .scan(SseLimitState::default(), |state, item| {
                std::future::ready(if state.failed {
                    None
                } else {
                    match item {
                        Ok(bytes) => match state.observe(&bytes) {
                            Ok(()) => Some(Ok(bytes)),
                            Err(error) => {
                                state.failed = true;
                                Some(Err(error))
                            }
                        },
                        Err(error) => {
                            state.failed = true;
                            Some(Err(transport_error("LLM event stream failed", error)))
                        }
                    }
                })
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_limit_resets_only_at_blank_lines() {
        let mut state = SseLimitState::default();
        state.observe(b"data: one\r\n").unwrap();
        assert!(state.event_bytes > 0);
        state.observe(b"\r\n").unwrap();
        assert_eq!(state.event_bytes, 0);
        state.observe(b"data: split\r").unwrap();
        let bytes_before_lf = state.event_bytes;
        state.observe(b"\n").unwrap();
        assert!(state.event_bytes >= bytes_before_lf);
        state.observe(b"\r").unwrap();
        state.observe(b"\n").unwrap();
        assert_eq!(state.event_bytes, 0);
        state.observe(b"data: two\n\n").unwrap();
        assert_eq!(state.event_bytes, 0);
    }
}
