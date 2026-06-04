//! API error type shared across all `thinclaw::api` sub-modules.
//!
//! Provides a framework-agnostic error enum that Tauri (or any other host)
//! can convert into its own error representation.

use thinclaw_gateway::web::api::{GatewayApiError, GatewayApiErrorKind};

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

    /// Feature exists but is disabled in settings.
    #[error("Feature disabled: {0}")]
    FeatureDisabled(String),

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
            Self::FeatureDisabled(_) => "feature_disabled",
            Self::Agent(_) => "agent_error",
            Self::Serialization(_) => "serialization_error",
            Self::UuidParse(_) => "uuid_parse_error",
            Self::Internal(_) => "internal_error",
        }
    }
}

impl From<ApiError> for GatewayApiError {
    fn from(error: ApiError) -> Self {
        match error {
            ApiError::InvalidInput(message) => {
                Self::new(GatewayApiErrorKind::InvalidInput, message)
            }
            ApiError::SessionNotFound(message) => {
                Self::new(GatewayApiErrorKind::SessionNotFound, message)
            }
            ApiError::Unavailable(message) => Self::new(GatewayApiErrorKind::Unavailable, message),
            ApiError::FeatureDisabled(message) => {
                Self::new(GatewayApiErrorKind::FeatureDisabled, message)
            }
            ApiError::Agent(error) => Self::new(GatewayApiErrorKind::Agent, error.to_string()),
            ApiError::Serialization(error) => {
                Self::new(GatewayApiErrorKind::Serialization, error.to_string())
            }
            ApiError::UuidParse(error) => {
                Self::new(GatewayApiErrorKind::UuidParse, error.to_string())
            }
            ApiError::Internal(message) => Self::new(GatewayApiErrorKind::Internal, message),
        }
    }
}

/// Convenience alias.
pub type ApiResult<T> = std::result::Result<T, ApiError>;
