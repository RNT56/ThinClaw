//! Root-independent API response policies shared by gateway handlers.

use axum::http::StatusCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayApiErrorKind {
    InvalidInput,
    SessionNotFound,
    Unavailable,
    FeatureDisabled,
    Agent,
    Serialization,
    UuidParse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureDisabledStatus {
    Forbidden,
    ServiceUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayApiError {
    pub kind: GatewayApiErrorKind,
    pub message: String,
}

impl GatewayApiError {
    pub fn new(kind: GatewayApiErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

pub fn gateway_api_error_status(
    kind: GatewayApiErrorKind,
    feature_disabled_status: FeatureDisabledStatus,
) -> StatusCode {
    match kind {
        GatewayApiErrorKind::InvalidInput | GatewayApiErrorKind::UuidParse => {
            StatusCode::BAD_REQUEST
        }
        GatewayApiErrorKind::SessionNotFound => StatusCode::NOT_FOUND,
        GatewayApiErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        GatewayApiErrorKind::FeatureDisabled => match feature_disabled_status {
            FeatureDisabledStatus::Forbidden => StatusCode::FORBIDDEN,
            FeatureDisabledStatus::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        },
        GatewayApiErrorKind::Agent
        | GatewayApiErrorKind::Serialization
        | GatewayApiErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub fn gateway_api_error(
    kind: GatewayApiErrorKind,
    message: impl Into<String>,
    feature_disabled_status: FeatureDisabledStatus,
) -> (StatusCode, String) {
    (
        gateway_api_error_status(kind, feature_disabled_status),
        message.into(),
    )
}

pub fn gateway_api_error_response(
    error: impl Into<GatewayApiError>,
    feature_disabled_status: FeatureDisabledStatus,
) -> (StatusCode, String) {
    let error = error.into();
    gateway_api_error(error.kind, error.message, feature_disabled_status)
}

pub fn bounded_limit(value: Option<usize>, default: usize, min: usize, max: usize) -> usize {
    value.unwrap_or(default).clamp(min, max)
}

pub fn learning_limit(value: Option<usize>) -> usize {
    bounded_limit(value, 50, 1, 200)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_status_maps_common_kinds() {
        assert_eq!(
            gateway_api_error_status(
                GatewayApiErrorKind::InvalidInput,
                FeatureDisabledStatus::Forbidden
            ),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            gateway_api_error_status(
                GatewayApiErrorKind::SessionNotFound,
                FeatureDisabledStatus::Forbidden
            ),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            gateway_api_error_status(
                GatewayApiErrorKind::Unavailable,
                FeatureDisabledStatus::Forbidden
            ),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn feature_disabled_status_is_domain_specific() {
        assert_eq!(
            gateway_api_error_status(
                GatewayApiErrorKind::FeatureDisabled,
                FeatureDisabledStatus::Forbidden
            ),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            gateway_api_error_status(
                GatewayApiErrorKind::FeatureDisabled,
                FeatureDisabledStatus::ServiceUnavailable
            ),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn api_error_pairs_status_with_message() {
        assert_eq!(
            gateway_api_error(
                GatewayApiErrorKind::FeatureDisabled,
                "learning disabled",
                FeatureDisabledStatus::Forbidden
            ),
            (StatusCode::FORBIDDEN, "learning disabled".to_string())
        );
    }

    #[test]
    fn api_error_response_accepts_portable_error() {
        assert_eq!(
            gateway_api_error_response(
                GatewayApiError::new(GatewayApiErrorKind::Unavailable, "db down"),
                FeatureDisabledStatus::Forbidden,
            ),
            (StatusCode::SERVICE_UNAVAILABLE, "db down".to_string())
        );
    }

    #[test]
    fn bounded_limit_applies_default_and_bounds() {
        assert_eq!(bounded_limit(None, 50, 1, 200), 50);
        assert_eq!(bounded_limit(Some(0), 50, 1, 200), 1);
        assert_eq!(bounded_limit(Some(999), 50, 1, 200), 200);
        assert_eq!(bounded_limit(Some(42), 50, 1, 200), 42);
    }

    #[test]
    fn learning_limit_uses_learning_bounds() {
        assert_eq!(learning_limit(None), 50);
        assert_eq!(learning_limit(Some(250)), 200);
    }
}
