//! Lifecycle hook management DTOs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct HookRegisterRequest {
    pub bundle_json: String,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HookInfo {
    pub name: String,
    pub hook_points: Vec<String>,
    pub failure_mode: String,
    pub timeout_ms: u64,
    pub priority: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HookListResponse {
    pub total: usize,
    pub hooks: Vec<HookInfo>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HookRegisterResponse {
    pub ok: bool,
    pub hooks_registered: usize,
    pub webhooks_registered: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HookUnregisterResponse {
    pub ok: bool,
    pub removed: bool,
    pub message: String,
}

impl HookUnregisterResponse {
    pub fn for_hook(name: &str, removed: bool) -> Self {
        Self {
            ok: removed,
            removed,
            message: if removed {
                format!("Hook '{name}' removed")
            } else {
                format!("Hook '{name}' not found")
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_list_response_preserves_existing_json_shape() {
        let response = HookListResponse {
            total: 1,
            hooks: vec![HookInfo {
                name: "audit".to_string(),
                hook_points: vec!["before_tool".to_string()],
                failure_mode: "FailOpen".to_string(),
                timeout_ms: 5000,
                priority: 10,
            }],
        };

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "total": 1,
                "hooks": [{
                    "name": "audit",
                    "hook_points": ["before_tool"],
                    "failure_mode": "FailOpen",
                    "timeout_ms": 5000,
                    "priority": 10
                }]
            })
        );
    }

    #[test]
    fn hook_action_responses_preserve_existing_json_shape() {
        assert_eq!(
            serde_json::to_value(HookRegisterResponse {
                ok: false,
                hooks_registered: 2,
                webhooks_registered: 1,
                errors: 1,
            })
            .unwrap(),
            serde_json::json!({
                "ok": false,
                "hooks_registered": 2,
                "webhooks_registered": 1,
                "errors": 1
            })
        );

        assert_eq!(
            serde_json::to_value(HookUnregisterResponse::for_hook("audit", true)).unwrap(),
            serde_json::json!({
                "ok": true,
                "removed": true,
                "message": "Hook 'audit' removed"
            })
        );
        assert_eq!(
            serde_json::to_value(HookUnregisterResponse::for_hook("audit", false)).unwrap(),
            serde_json::json!({
                "ok": false,
                "removed": false,
                "message": "Hook 'audit' not found"
            })
        );
    }
}
