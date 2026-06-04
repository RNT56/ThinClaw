use std::collections::HashMap;

pub fn provider_base_url(config: &HashMap<String, String>) -> Option<String> {
    config
        .get("base_url")
        .or_else(|| config.get("url"))
        .cloned()
        .filter(|v| !v.trim().is_empty())
}

pub fn provider_config_value(config: &HashMap<String, String>, key: &str) -> Option<String> {
    config.get(key).cloned().filter(|v| !v.trim().is_empty())
}

pub fn provider_base_url_or(config: &HashMap<String, String>, default: &str) -> String {
    provider_base_url(config).unwrap_or_else(|| default.to_string())
}

pub fn provider_bool(config: &HashMap<String, String>, key: &str) -> bool {
    config
        .get(key)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}

pub fn provider_scoped_user_id(config: &HashMap<String, String>, user_id: &str) -> String {
    provider_config_value(config, "user_id").unwrap_or_else(|| user_id.to_string())
}

pub fn provider_agent_id(config: &HashMap<String, String>) -> String {
    provider_config_value(config, "agent_id").unwrap_or_else(|| "thinclaw".to_string())
}

pub fn provider_join_url(base_url: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

pub fn provider_path(config: &HashMap<String, String>, key: &str, default: &str) -> String {
    provider_config_value(config, key).unwrap_or_else(|| default.to_string())
}

pub fn provider_path_with_vars(
    config: &HashMap<String, String>,
    key: &str,
    default: &str,
) -> String {
    let mut path = provider_path(config, key, default);
    for (name, value) in config {
        path = path.replace(&format!("{{{name}}}"), value);
    }
    path
}

pub fn payload_text(payload: &serde_json::Value) -> String {
    if let Some(value) = payload.as_str() {
        return value.to_string();
    }
    for key in ["content", "text", "summary", "memory", "user_message"] {
        if let Some(value) = payload.get(key).and_then(|value| value.as_str())
            && !value.trim().is_empty()
        {
            return value.to_string();
        }
    }
    let user = payload
        .get("user")
        .or_else(|| payload.get("user_message"))
        .and_then(|value| value.as_str());
    let assistant = payload
        .get("assistant")
        .or_else(|| payload.get("assistant_response"))
        .and_then(|value| value.as_str());
    match (user, assistant) {
        (Some(user), Some(assistant)) => {
            format!("User: {user}\nAssistant: {assistant}")
        }
        _ => serde_json::to_string(payload).unwrap_or_else(|_| format!("{payload:?}")),
    }
}

pub fn provider_export_messages(
    config: &HashMap<String, String>,
    payload: &serde_json::Value,
) -> Vec<serde_json::Value> {
    if let Some(messages) = payload.get("messages").and_then(|value| value.as_array()) {
        return messages.clone();
    }
    vec![serde_json::json!({
        "role": provider_config_value(config, "export_role").unwrap_or_else(|| "user".to_string()),
        "content": payload_text(payload),
    })]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn base_url_prefers_base_url_even_when_blank_then_filters_it() {
        let values = config(&[("base_url", " https://memory.local/ "), ("url", "fallback")]);
        assert_eq!(
            provider_base_url(&values).as_deref(),
            Some(" https://memory.local/ ")
        );

        let values = config(&[("base_url", " \t "), ("url", "https://fallback.local")]);
        assert_eq!(provider_base_url(&values), None);
    }

    #[test]
    fn bool_parser_accepts_legacy_truthy_values_only() {
        for value in ["1", "true", "TRUE", " yes "] {
            let values = config(&[("enabled", value)]);
            assert!(provider_bool(&values, "enabled"));
        }

        for value in ["0", "false", "enabled", ""] {
            let values = config(&[("enabled", value)]);
            assert!(!provider_bool(&values, "enabled"));
        }
    }

    #[test]
    fn scoped_ids_and_paths_use_defaults_until_overridden() {
        let empty = HashMap::new();
        assert_eq!(provider_scoped_user_id(&empty, "user-1"), "user-1");
        assert_eq!(provider_agent_id(&empty), "thinclaw");
        assert_eq!(provider_path(&empty, "sync_path", "/memories"), "/memories");

        let values = config(&[
            ("user_id", "external-user"),
            ("agent_id", "agent-9"),
            ("sync_path", "/users/{user_id}/agents/{agent_id}/memories"),
        ]);
        assert_eq!(
            provider_path_with_vars(&values, "sync_path", "/fallback"),
            "/users/external-user/agents/agent-9/memories"
        );
    }

    #[test]
    fn join_url_preserves_absolute_paths_and_normalizes_slashes() {
        assert_eq!(
            provider_join_url("https://api.local/", "/v1/search"),
            "https://api.local/v1/search"
        );
        assert_eq!(
            provider_join_url("https://api.local", "v1/search"),
            "https://api.local/v1/search"
        );
        assert_eq!(
            provider_join_url("https://api.local", "https://override.local/health"),
            "https://override.local/health"
        );
    }

    #[test]
    fn payload_text_extracts_legacy_text_shapes() {
        assert_eq!(payload_text(&serde_json::json!("plain")), "plain");
        assert_eq!(
            payload_text(&serde_json::json!({"summary": " memory summary "})),
            " memory summary "
        );
        assert_eq!(
            payload_text(&serde_json::json!({
                "user": "hello",
                "assistant_response": "hi"
            })),
            "User: hello\nAssistant: hi"
        );
        assert_eq!(
            payload_text(&serde_json::json!({"unknown": true})),
            "{\"unknown\":true}"
        );
    }

    #[test]
    fn provider_export_messages_preserves_existing_messages_or_builds_default() {
        let existing = serde_json::json!({
            "messages": [
                {"role": "system", "content": "keep"}
            ]
        });
        assert_eq!(
            provider_export_messages(&HashMap::new(), &existing),
            vec![serde_json::json!({"role": "system", "content": "keep"})]
        );

        let config = config(&[("export_role", "assistant")]);
        assert_eq!(
            provider_export_messages(&config, &serde_json::json!({"content": "hello"})),
            vec![serde_json::json!({"role": "assistant", "content": "hello"})]
        );
    }
}
