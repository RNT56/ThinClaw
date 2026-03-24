//! API error type shared across all `thinclaw::api` sub-modules.
//!
//! Provides a framework-agnostic error enum that Tauri (or any other host)
//! can convert into its own error representation.

/// Unified error for all API functions.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Caller-supplied input failed validation.
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Requested session / thread does not exist.
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    /// A required component (workspace, DB, etc.) is unavailable.
    #[error("Service unavailable: {0}")]
    Unavailable(String),

    /// Internal agent error.
    #[error("Agent error: {0}")]
    Agent(#[from] crate::Error),

    /// JSON (de)serialization failure.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// UUID parse failure (bad session key, thread id, etc.).
    #[error("UUID parse error: {0}")]
    UuidParse(#[from] uuid::Error),

    /// Catch-all for unexpected failures.
    #[error("{0}")]
    Internal(String),
}

impl ApiError {
    /// Machine-readable error code for frontend mapping.
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::SessionNotFound(_) => "session_not_found",
            Self::Unavailable(_) => "unavailable",
            Self::Agent(_) => "agent_error",
            Self::Serialization(_) => "serialization_error",
            Self::UuidParse(_) => "uuid_parse_error",
            Self::Internal(_) => "internal_error",
        }
    }
}

/// Convenience alias.
pub type ApiResult<T> = std::result::Result<T, ApiError>;
