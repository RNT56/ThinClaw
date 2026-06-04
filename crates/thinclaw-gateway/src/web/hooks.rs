//! Root-independent hook gateway response policies.

use axum::http::StatusCode;

use crate::web::types::{HookInfo, HookListResponse, HookRegisterResponse};

pub const HOOK_REGISTRY_UNAVAILABLE_MESSAGE: &str = "Hook registry not available";

pub fn hook_registry_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        HOOK_REGISTRY_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn invalid_hook_json_error(error: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, format!("Invalid JSON: {error}"))
}

pub fn invalid_hook_bundle_error(error: impl std::fmt::Display) -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        format!("Invalid hook bundle: {error}"),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookInfoInput {
    pub name: String,
    pub hook_points: Vec<String>,
    pub failure_mode: String,
    pub timeout_ms: u64,
    pub priority: u32,
}

pub fn hook_info(input: HookInfoInput) -> HookInfo {
    HookInfo {
        name: input.name,
        hook_points: input.hook_points,
        failure_mode: input.failure_mode,
        timeout_ms: input.timeout_ms,
        priority: input.priority,
    }
}

pub fn hook_list_response(hooks: Vec<HookInfo>) -> HookListResponse {
    HookListResponse {
        total: hooks.len(),
        hooks,
    }
}

pub fn hook_register_response(
    hooks_registered: usize,
    webhooks_registered: usize,
    errors: usize,
) -> HookRegisterResponse {
    HookRegisterResponse {
        ok: errors == 0,
        hooks_registered,
        webhooks_registered,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_registry_unavailable_error_uses_service_unavailable() {
        assert_eq!(
            hook_registry_unavailable_error(),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                HOOK_REGISTRY_UNAVAILABLE_MESSAGE.to_string()
            )
        );
    }

    #[test]
    fn hook_list_response_counts_rows() {
        let response = hook_list_response(vec![hook_info(HookInfoInput {
            name: "audit".to_string(),
            hook_points: vec!["tool:before".to_string()],
            failure_mode: "warn".to_string(),
            timeout_ms: 1000,
            priority: 2,
        })]);

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "total": 1,
                "hooks": [{
                    "name": "audit",
                    "hook_points": ["tool:before"],
                    "failure_mode": "warn",
                    "timeout_ms": 1000,
                    "priority": 2,
                }]
            })
        );
    }

    #[test]
    fn hook_register_response_sets_ok_from_error_count() {
        assert!(hook_register_response(1, 0, 0).ok);
        assert!(!hook_register_response(1, 0, 1).ok);
    }

    #[test]
    fn hook_validation_errors_preserve_existing_messages() {
        assert_eq!(
            invalid_hook_json_error("bad syntax"),
            (
                StatusCode::BAD_REQUEST,
                "Invalid JSON: bad syntax".to_string()
            )
        );
        assert_eq!(
            invalid_hook_bundle_error("missing name"),
            (
                StatusCode::BAD_REQUEST,
                "Invalid hook bundle: missing name".to_string()
            )
        );
    }
}
