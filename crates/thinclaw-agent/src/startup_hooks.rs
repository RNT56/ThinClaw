//! Startup hook routing and gateway ownership policy.

use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayStartupThreadTarget {
    pub principal_id: String,
    pub actor_id: String,
    pub thread_id: Uuid,
}

pub fn telegram_startup_thread_id(
    hook_name: &str,
    target_channel: &str,
    bootstrap_pending: bool,
) -> Option<&'static str> {
    if target_channel != "telegram" {
        return None;
    }

    match hook_name {
        // During first-run bootstrap we keep the recurring boot hook in the
        // onboarding thread so General is only created once setup is complete.
        "boot" if bootstrap_pending => Some("bootstrap"),
        "boot" => Some("boot"),
        "bootstrap" => Some("bootstrap"),
        _ => None,
    }
}

pub fn heartbeat_gateway_fallback_identity_from_diagnostics(
    diagnostics: Option<&serde_json::Value>,
    fallback_user_id: &str,
) -> (String, String) {
    let principal_id = diagnostics
        .and_then(|value| value.get("user_id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_user_id)
        .to_string();
    let actor_id = diagnostics
        .and_then(|value| value.get("actor_id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(principal_id.as_str())
        .to_string();
    (principal_id, actor_id)
}

pub fn heartbeat_routine_owner_from_gateway_defaults(
    fallback_principal_id: &str,
    fallback_actor_id: &str,
    inferred_user_id: Option<&str>,
) -> (String, String) {
    let user_id = if !fallback_principal_id.trim().is_empty() && fallback_principal_id != "default"
    {
        fallback_principal_id.to_string()
    } else {
        inferred_user_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback_principal_id)
            .to_string()
    };
    let actor_id =
        if fallback_actor_id.trim().is_empty() || fallback_actor_id == fallback_principal_id {
            user_id.clone()
        } else {
            fallback_actor_id.to_string()
        };
    (user_id, actor_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_startup_thread_id_routes_first_run_boots_to_onboarding() {
        assert_eq!(
            telegram_startup_thread_id("boot", "telegram", true),
            Some("bootstrap")
        );
        assert_eq!(
            telegram_startup_thread_id("bootstrap", "telegram", true),
            Some("bootstrap")
        );
        assert_eq!(
            telegram_startup_thread_id("boot", "telegram", false),
            Some("boot")
        );
        assert_eq!(telegram_startup_thread_id("bootstrap", "web", true), None);
    }

    #[test]
    fn heartbeat_gateway_fallback_identity_prefers_gateway_identity() {
        let diagnostics = serde_json::json!({
            "user_id": "household-user",
            "actor_id": "desk-actor",
        });

        let (user_id, actor_id) = heartbeat_gateway_fallback_identity_from_diagnostics(
            Some(&diagnostics),
            "fallback-user",
        );

        assert_eq!(user_id, "household-user");
        assert_eq!(actor_id, "desk-actor");
    }

    #[test]
    fn heartbeat_gateway_fallback_identity_falls_back_to_workspace_user() {
        let diagnostics = serde_json::json!({
            "user_id": "",
            "actor_id": "",
        });

        let (user_id, actor_id) = heartbeat_gateway_fallback_identity_from_diagnostics(
            Some(&diagnostics),
            "workspace-user",
        );

        assert_eq!(user_id, "workspace-user");
        assert_eq!(actor_id, "workspace-user");
    }

    #[test]
    fn heartbeat_owner_uses_inferred_user_for_default_gateway_identity() {
        let (user_id, actor_id) =
            heartbeat_routine_owner_from_gateway_defaults("default", "default", Some("684480568"));

        assert_eq!(user_id, "684480568");
        assert_eq!(actor_id, "684480568");
    }

    #[test]
    fn heartbeat_owner_keeps_distinct_gateway_actor() {
        let (user_id, actor_id) = heartbeat_routine_owner_from_gateway_defaults(
            "household-user",
            "desk-actor",
            Some("inferred"),
        );

        assert_eq!(user_id, "household-user");
        assert_eq!(actor_id, "desk-actor");
    }
}
