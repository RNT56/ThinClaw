//! The async ThinClaw gateway client.

use std::time::Duration;

use futures::{Stream, StreamExt};
use reqwest::Url;
use uuid::Uuid;

use crate::error::{ClientError, Result};
use crate::sse::SseDecoder;
use crate::wire::{
    ApprovalAction, ApprovalRequest, HistoryResponse, SendMessageRequest, SendMessageResponse,
    SseEvent, ThreadListResponse,
};

const MAX_ERROR_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_COMMAND_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_HISTORY_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_GATEWAY_TOKEN_BYTES: usize = 64 * 1024;

/// Async client for a ThinClaw gateway.
///
/// Wraps the gateway HTTP + SSE surface with bearer-token auth. Cheap to clone
/// (the inner `reqwest::Client` is a connection-pool handle).
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: Url,
    token: String,
}

impl std::fmt::Debug for Client {
    /// Redacts the bearer token so a `{:?}` of the client (in logs, error
    /// context, or a panic payload) can never leak the credential.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.base_url.as_str())
            .field("token", &"<redacted>")
            .finish()
    }
}

impl Client {
    /// Build a client from a base URL (e.g. `http://127.0.0.1:8080`) and the
    /// gateway bearer token.
    pub fn new(base_url: impl AsRef<str>, token: impl Into<String>) -> Result<Self> {
        let base_url = Url::parse(base_url.as_ref())
            .map_err(|e| ClientError::InvalidUrl(format!("{}: {e}", base_url.as_ref())))?;
        if !matches!(base_url.scheme(), "http" | "https")
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(ClientError::InvalidUrl(
                "gateway URL must be HTTP(S) without credentials, query, or fragment".to_string(),
            ));
        }
        let token = token.into();
        if token.is_empty()
            || token.len() > MAX_GATEWAY_TOKEN_BYTES
            || token.chars().any(char::is_control)
        {
            return Err(ClientError::InvalidUrl(
                "gateway token is empty, oversized, or malformed".to_string(),
            ));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()?;
        Ok(Self {
            http,
            base_url,
            token,
        })
    }

    /// Build a client from `THINCLAW_GATEWAY_URL` and `THINCLAW_GATEWAY_TOKEN`.
    pub fn from_env() -> Result<Self> {
        let url = std::env::var("THINCLAW_GATEWAY_URL")
            .map_err(|_| ClientError::MissingEnv("THINCLAW_GATEWAY_URL"))?;
        let token = std::env::var("THINCLAW_GATEWAY_TOKEN")
            .map_err(|_| ClientError::MissingEnv("THINCLAW_GATEWAY_TOKEN"))?;
        Self::new(url, token)
    }

    /// The base URL for pointing an OpenAI-compatible client at ThinClaw:
    /// `{base_url}/v1`. The fastest zero-code integration path for users who
    /// already have an OpenAI-shaped client — at the cost of ThinClaw-native
    /// features (tool-call visibility, approvals, thread management), which are
    /// only available through this client's native methods.
    pub fn openai_compat_base_url(&self) -> Result<Url> {
        self.url("/v1")
    }

    fn url(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path)
            .map_err(|e| ClientError::InvalidUrl(format!("{path}: {e}")))
    }

    async fn error_for_status(resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = thinclaw_types::http_response::bounded_text(resp, MAX_ERROR_RESPONSE_BYTES)
            .await
            .unwrap_or_default();
        let truncated: String = body.chars().take(500).collect();
        Err(ClientError::Status {
            status: status.as_u16(),
            body: truncated,
        })
    }

    /// Send a chat message. Returns immediately with the accepted message id;
    /// the assistant's reply arrives over [`Client::events`], correlated by
    /// `thread_id`. See [`Client::send_and_wait`] for a blocking convenience.
    pub async fn send_message(
        &self,
        content: impl Into<String>,
        thread_id: Option<Uuid>,
    ) -> Result<SendMessageResponse> {
        let body = SendMessageRequest {
            content: content.into(),
            thread_id: thread_id.map(|id| id.to_string()),
            user_id: None,
            actor_id: None,
        };
        let resp = self
            .http
            .post(self.url("/api/chat/send")?)
            .bearer_auth(&self.token)
            .json(&body)
            .timeout(Duration::from_secs(60))
            .send()
            .await?;
        let resp = Self::error_for_status(resp).await?;
        thinclaw_types::http_response::bounded_json(resp, MAX_COMMAND_RESPONSE_BYTES)
            .await
            .map_err(|error| ClientError::Response(error.to_string()))
    }

    /// Fetch the transcript for a thread.
    pub async fn history(
        &self,
        thread_id: Uuid,
        limit: Option<u32>,
        before: Option<&str>,
    ) -> Result<HistoryResponse> {
        let mut url = self.url("/api/chat/history")?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("thread_id", &thread_id.to_string());
            if let Some(limit) = limit {
                q.append_pair("limit", &limit.to_string());
            }
            if let Some(before) = before {
                q.append_pair("before", before);
            }
        }
        let resp = self
            .http
            .get(url)
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(60))
            .send()
            .await?;
        let resp = Self::error_for_status(resp).await?;
        thinclaw_types::http_response::bounded_json(resp, MAX_HISTORY_RESPONSE_BYTES)
            .await
            .map_err(|error| ClientError::Response(error.to_string()))
    }

    /// List conversation threads.
    pub async fn list_threads(&self) -> Result<ThreadListResponse> {
        let resp = self
            .http
            .get(self.url("/api/chat/threads")?)
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(60))
            .send()
            .await?;
        let resp = Self::error_for_status(resp).await?;
        thinclaw_types::http_response::bounded_json(resp, MAX_COMMAND_RESPONSE_BYTES)
            .await
            .map_err(|error| ClientError::Response(error.to_string()))
    }

    /// Create a new conversation thread. Returns the raw JSON response (the
    /// server shape is a small command-ack object).
    pub async fn new_thread(&self) -> Result<serde_json::Value> {
        let resp = self
            .http
            .post(self.url("/api/chat/thread/new")?)
            .bearer_auth(&self.token)
            .json(&serde_json::json!({}))
            .timeout(Duration::from_secs(60))
            .send()
            .await?;
        let resp = Self::error_for_status(resp).await?;
        thinclaw_types::http_response::bounded_json(resp, MAX_COMMAND_RESPONSE_BYTES)
            .await
            .map_err(|error| ClientError::Response(error.to_string()))
    }

    /// Resolve a pending tool-approval request.
    pub async fn resolve_approval(
        &self,
        request_id: impl Into<String>,
        action: ApprovalAction,
        thread_id: Option<Uuid>,
    ) -> Result<()> {
        let body = ApprovalRequest {
            request_id: request_id.into(),
            action: action.as_str().to_string(),
            thread_id: thread_id.map(|id| id.to_string()),
            user_id: None,
            actor_id: None,
        };
        let resp = self
            .http
            .post(self.url("/api/chat/approval")?)
            .bearer_auth(&self.token)
            .json(&body)
            .timeout(Duration::from_secs(60))
            .send()
            .await?;
        Self::error_for_status(resp).await?;
        Ok(())
    }

    /// Abort the in-flight turn for a thread.
    pub async fn abort(&self, thread_id: Uuid) -> Result<()> {
        let resp = self
            .http
            .post(self.url("/api/chat/abort")?)
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "thread_id": thread_id.to_string() }))
            .timeout(Duration::from_secs(60))
            .send()
            .await?;
        Self::error_for_status(resp).await?;
        Ok(())
    }

    /// Open the server-sent event stream (`GET /api/chat/events`).
    ///
    /// Yields decoded [`SseEvent`]s. Unknown event types surface as
    /// [`SseEvent::Unknown`] rather than errors, so a newer server never breaks
    /// an older client. The stream ends when the connection closes.
    pub async fn events(&self) -> Result<impl Stream<Item = Result<SseEvent>>> {
        let resp = self
            .http
            .get(self.url("/api/chat/events")?)
            .bearer_auth(&self.token)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await?;
        let resp = Self::error_for_status(resp).await?;

        let mut decoder = SseDecoder::new();
        let byte_stream = resp.bytes_stream();

        // Flat-map raw byte chunks into decoded events. Each chunk may complete
        // zero or more frames; each frame's data payload is parsed to JSON then
        // into an SseEvent.
        let stream = byte_stream.flat_map(move |chunk| {
            let events: Vec<Result<SseEvent>> = match chunk {
                Ok(bytes) => match decoder.push(&bytes) {
                    Ok(payloads) => payloads
                        .into_iter()
                        .filter_map(|payload| {
                            // Ignore keep-alive/empty payloads.
                            let trimmed = payload.trim();
                            if trimmed.is_empty() {
                                return None;
                            }
                            Some(
                                serde_json::from_str::<serde_json::Value>(trimmed)
                                    .map(SseEvent::from_json)
                                    .map_err(ClientError::from),
                            )
                        })
                        .collect(),
                    Err(error) => vec![Err(ClientError::Response(error.to_string()))],
                },
                Err(e) => vec![Err(ClientError::from(e))],
            };
            futures::stream::iter(events)
        });

        Ok(stream)
    }

    /// Send a message and block until the first matching `Response` (or `Error`)
    /// event for its thread arrives, or `timeout` elapses.
    ///
    /// Convenience for simple request/response usage; for tool-call visibility
    /// or streaming tokens, drive [`Client::events`] directly. Note: because the
    /// stream is opened *after* the send returns its `thread_id`, a reply that
    /// arrives in the gap is not observed — acceptable for interactive use, but
    /// prefer opening `events()` first for guaranteed capture.
    pub async fn send_and_wait(
        &self,
        content: impl Into<String>,
        thread_id: Option<Uuid>,
        timeout: Duration,
    ) -> Result<String> {
        let accepted = self.send_message(content, thread_id).await?;
        let _ = accepted; // message_id is not needed for correlation; thread_id is.
        let want_thread = thread_id.map(|id| id.to_string());

        let stream = self.events().await?;
        futures::pin_mut!(stream);

        let deadline = tokio::time::sleep(timeout);
        futures::pin_mut!(deadline);

        loop {
            tokio::select! {
                _ = &mut deadline => return Err(ClientError::Timeout(timeout)),
                next = stream.next() => match next {
                    None => return Err(ClientError::StreamClosed),
                    Some(Err(e)) => return Err(e),
                    Some(Ok(event)) => {
                        // If we know the target thread, only accept matching events.
                        let matches = want_thread
                            .as_deref()
                            .map(|t| event.thread_id() == Some(t))
                            .unwrap_or(true);
                        if !matches {
                            continue;
                        }
                        match event {
                            SseEvent::Response { content, .. } => return Ok(content),
                            SseEvent::Error { message, .. } => {
                                return Err(ClientError::Status { status: 0, body: message });
                            }
                            _ => continue,
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_urls_from_base() {
        let c = Client::new("http://127.0.0.1:8080", "tok").unwrap();
        assert_eq!(
            c.url("/api/chat/send").unwrap().as_str(),
            "http://127.0.0.1:8080/api/chat/send"
        );
        assert_eq!(
            c.openai_compat_base_url().unwrap().as_str(),
            "http://127.0.0.1:8080/v1"
        );
    }

    #[test]
    fn rejects_bad_base_url() {
        assert!(matches!(
            Client::new("not a url", "tok"),
            Err(ClientError::InvalidUrl(_))
        ));
    }

    #[test]
    fn debug_redacts_the_bearer_token() {
        let c = Client::new("http://127.0.0.1:8080", "super-secret-token").unwrap();
        let rendered = format!("{c:?}");
        assert!(
            !rendered.contains("super-secret-token"),
            "token leaked: {rendered}"
        );
        assert!(rendered.contains("<redacted>"));
        assert!(rendered.contains("127.0.0.1:8080"));
    }
}
