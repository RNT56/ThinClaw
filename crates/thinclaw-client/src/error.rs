//! Client error type.

/// Errors returned by the ThinClaw client.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The base URL could not be parsed or a request URL could not be built.
    #[error("invalid gateway URL: {0}")]
    InvalidUrl(String),

    /// A required environment variable was missing (for `from_env`).
    #[error("missing environment variable: {0}")]
    MissingEnv(&'static str),

    /// The HTTP transport failed (connect/timeout/TLS/etc.).
    #[error("http transport error: {0}")]
    Http(#[from] reqwest::Error),

    /// The gateway returned a non-success status.
    #[error("gateway returned {status}: {body}")]
    Status {
        /// HTTP status code.
        status: u16,
        /// Response body (truncated).
        body: String,
    },

    /// A response or SSE frame could not be decoded.
    #[error("decode error: {0}")]
    Decode(#[from] serde_json::Error),

    /// `send_and_wait` timed out before a matching response arrived.
    #[error("timed out waiting for a response after {0:?}")]
    Timeout(std::time::Duration),

    /// The event stream ended before a matching response arrived.
    #[error("event stream closed before a response arrived")]
    StreamClosed,
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, ClientError>;
