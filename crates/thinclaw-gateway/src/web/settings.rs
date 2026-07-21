//! Root-independent settings policies used by gateway handlers.

use std::collections::HashMap;

use axum::http::StatusCode;
use thinclaw_channels_core::StreamMode;

use crate::web::types::{SettingResponse, SettingsExportResponse, SettingsListResponse};

pub const REDACTED_SETTING_VALUE: &str = "[REDACTED]";
pub const SETTINGS_STORE_UNAVAILABLE_STATUS: StatusCode = StatusCode::SERVICE_UNAVAILABLE;
pub const SETTING_NOT_FOUND_STATUS: StatusCode = StatusCode::NOT_FOUND;
pub const SENSITIVE_SETTING_WRITE_FORBIDDEN_STATUS: StatusCode = StatusCode::FORBIDDEN;
pub const TELEGRAM_CHANNEL_NAME: &str = "telegram";
pub const TELEGRAM_AUTO_TRANSPORT_MODE: &str = "auto";
pub const TELEGRAM_SUBAGENT_SESSION_MODE_FIELD: &str = "subagent_session_mode";
pub const TELEGRAM_DEFAULT_SUBAGENT_SESSION_MODE: &str = "temp_topic";
const MAX_SETTING_KEY_BYTES: usize = 2_048;
const MAX_SETTING_KEY_COMPONENTS: usize = 64;
const MAX_SETTING_KEY_COMPONENT_BYTES: usize = 256;
const MAX_SETTING_VALUE_BYTES: usize = 1024 * 1024;
const MAX_SETTING_VALUE_DEPTH: usize = 64;
const MAX_SETTING_VALUE_NODES: usize = 100_000;

pub fn settings_store_unavailable_status() -> StatusCode {
    SETTINGS_STORE_UNAVAILABLE_STATUS
}

pub fn setting_not_found_status() -> StatusCode {
    SETTING_NOT_FOUND_STATUS
}

pub fn sensitive_setting_write_forbidden_status() -> StatusCode {
    SENSITIVE_SETTING_WRITE_FORBIDDEN_STATUS
}

pub fn validate_settings_key(key: &str) -> Result<(), StatusCode> {
    let mut component_count = 0usize;
    if key.trim().is_empty() || key.len() > MAX_SETTING_KEY_BYTES || key.contains('\0') {
        return Err(StatusCode::BAD_REQUEST);
    }
    for component in key.split('.') {
        component_count = component_count.saturating_add(1);
        if component.is_empty()
            || component.len() > MAX_SETTING_KEY_COMPONENT_BYTES
            || component.chars().any(char::is_control)
        {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    if component_count > MAX_SETTING_KEY_COMPONENTS {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}

/// Bound a setting before it reaches either database backend. This is kept in
/// the shared gateway crate so HTTP imports and non-HTTP settings ports cannot
/// diverge on malformed keys, deep JSON, or oversized values.
pub fn validate_setting_entry(key: &str, value: &serde_json::Value) -> Result<(), StatusCode> {
    validate_settings_key(key)?;

    let mut stack = vec![(value, 0usize)];
    let mut nodes = 0usize;
    let mut text_bytes = 0usize;
    while let Some((current, depth)) = stack.pop() {
        if depth > MAX_SETTING_VALUE_DEPTH {
            return Err(StatusCode::BAD_REQUEST);
        }
        nodes = nodes.saturating_add(1);
        if nodes > MAX_SETTING_VALUE_NODES {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        match current {
            serde_json::Value::String(text) => {
                if text.contains('\0') {
                    return Err(StatusCode::BAD_REQUEST);
                }
                text_bytes = text_bytes.saturating_add(text.len());
            }
            serde_json::Value::Array(values) => {
                if values.len() > MAX_SETTING_VALUE_NODES.saturating_sub(nodes) {
                    return Err(StatusCode::PAYLOAD_TOO_LARGE);
                }
                stack.extend(values.iter().map(|value| (value, depth.saturating_add(1))));
            }
            serde_json::Value::Object(values) => {
                if values.len() > MAX_SETTING_VALUE_NODES.saturating_sub(nodes) {
                    return Err(StatusCode::PAYLOAD_TOO_LARGE);
                }
                for (nested_key, nested_value) in values {
                    if nested_key.contains('\0') {
                        return Err(StatusCode::BAD_REQUEST);
                    }
                    text_bytes = text_bytes.saturating_add(nested_key.len());
                    stack.push((nested_value, depth.saturating_add(1)));
                }
            }
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            }
        }
        if text_bytes > MAX_SETTING_VALUE_BYTES {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
    }

    let encoded_len = serde_json::to_vec(value)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .len();
    if encoded_len > MAX_SETTING_VALUE_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct GatewaySettingRow {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: String,
}

pub fn setting_response_from_row(row: GatewaySettingRow) -> SettingResponse {
    SettingResponse {
        value: redact_setting_value(&row.key, row.value),
        key: row.key,
        updated_at: row.updated_at,
    }
}

pub fn settings_list_response_from_rows(
    rows: impl IntoIterator<Item = GatewaySettingRow>,
) -> SettingsListResponse {
    SettingsListResponse {
        settings: rows.into_iter().map(setting_response_from_row).collect(),
    }
}

pub fn settings_export_response_from_map(
    settings: HashMap<String, serde_json::Value>,
) -> SettingsExportResponse {
    SettingsExportResponse {
        settings: settings
            .into_iter()
            .map(|(key, value)| {
                let value = redact_setting_value(&key, value);
                (key, value)
            })
            .collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCodeSettingsUpdate {
    /// `None` means this setting was not part of the update. `Some(None)`
    /// resets it to the process-start default.
    pub model: Option<Option<String>>,
    /// `None` means this setting was not part of the update. `Some(None)`
    /// resets it to the process-start default.
    pub max_turns: Option<Option<u32>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelStreamModeUpdate {
    pub channel_name: &'static str,
    pub mode: StreamMode,
}

pub fn is_sensitive_settings_key(key: &str) -> bool {
    key.starts_with("secret.")
        || matches!(
            key,
            "database_url"
                | "libsql_url"
                | "tunnel.ngrok_token"
                | "tunnel.cf_token"
                | "channels.discord_bot_token"
                | "channels.slack_bot_token"
                | "channels.slack_app_token"
                | "channels.bluebubbles_password"
                | "channels.gateway_auth_token"
                | "channels.gateway_principals"
                | "channels.nostr_private_key"
        )
}

pub fn redact_setting_value(key: &str, value: serde_json::Value) -> serde_json::Value {
    if is_sensitive_settings_key(key) {
        serde_json::Value::String(REDACTED_SETTING_VALUE.to_string())
    } else {
        value
    }
}

pub fn sanitize_imported_settings(
    settings: HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    settings
        .into_iter()
        .filter(|(key, _)| !is_sensitive_settings_key(key))
        .collect()
}

pub fn parse_timezone_setting_value(
    value: &serde_json::Value,
    is_valid_timezone: impl Fn(&str) -> bool,
) -> Result<Option<String>, StatusCode> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            if is_valid_timezone(trimmed) {
                Ok(Some(trimmed.to_string()))
            } else {
                Err(StatusCode::BAD_REQUEST)
            }
        }
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

pub fn is_nostr_settings_key(key: &str) -> bool {
    key.starts_with("channels.nostr_") || key.starts_with("nostr_")
}

pub fn requires_llm_runtime_reload(key: &str) -> bool {
    key.starts_with("providers.")
        || matches!(
            key,
            "llm_backend" | "selected_model" | "openai_compatible_base_url" | "ollama_base_url"
        )
}

pub fn claude_code_settings_update(
    key: &str,
    value: &serde_json::Value,
) -> Option<ClaudeCodeSettingsUpdate> {
    match key {
        "claude_code_model" => Some(ClaudeCodeSettingsUpdate {
            model: Some(value.as_str().map(str::to_string)),
            max_turns: None,
        }),
        "claude_code_max_turns" => Some(ClaudeCodeSettingsUpdate {
            model: None,
            max_turns: Some(value.as_u64().and_then(|value| u32::try_from(value).ok())),
        }),
        _ => None,
    }
}

pub fn codex_code_model_update(key: &str, value: &serde_json::Value) -> Option<Option<String>> {
    match key {
        "codex_code_model" => Some(value.as_str().map(str::to_string)),
        _ => None,
    }
}

/// Validate persisted settings that are consumed as sandbox process command
/// arguments. Keeping this check at the write boundary prevents malformed or
/// oversized values from reaching either the live runtime cache or a later
/// process restart.
pub fn validate_sandbox_code_setting(
    key: &str,
    value: &serde_json::Value,
) -> Result<(), StatusCode> {
    const MAX_MODEL_BYTES: usize = 256;
    const MAX_TURNS: u64 = 1_000;

    let valid_model = |model: &str| {
        !model.trim().is_empty()
            && model.len() <= MAX_MODEL_BYTES
            && !model.chars().any(char::is_control)
    };

    match key {
        "claude_code_model" | "codex_code_model" => match value {
            serde_json::Value::Null => Ok(()),
            serde_json::Value::String(model) if valid_model(model) => Ok(()),
            _ => Err(StatusCode::BAD_REQUEST),
        },
        "claude_code_max_turns" => match value {
            serde_json::Value::Null => Ok(()),
            serde_json::Value::Number(number) => number
                .as_u64()
                .filter(|turns| (1..=MAX_TURNS).contains(turns))
                .map(|_| ())
                .ok_or(StatusCode::BAD_REQUEST),
            _ => Err(StatusCode::BAD_REQUEST),
        },
        _ => Ok(()),
    }
}

pub fn channel_stream_mode_update(
    key: &str,
    value: &serde_json::Value,
) -> Option<ChannelStreamModeUpdate> {
    match key {
        "telegram_stream_mode" | "channels.telegram_stream_mode" => Some(ChannelStreamModeUpdate {
            channel_name: "telegram",
            mode: value
                .as_str()
                .map(StreamMode::from_str_value)
                .unwrap_or_default(),
        }),
        "discord_stream_mode" | "channels.discord_stream_mode" => Some(ChannelStreamModeUpdate {
            channel_name: "discord",
            mode: value
                .as_str()
                .map(StreamMode::from_str_value)
                .unwrap_or_default(),
        }),
        _ => None,
    }
}

pub fn telegram_subagent_session_mode_update(
    key: &str,
    value: &serde_json::Value,
) -> Option<String> {
    is_telegram_subagent_session_mode_key(key).then(|| {
        value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(TELEGRAM_DEFAULT_SUBAGENT_SESSION_MODE)
            .to_string()
    })
}

pub fn telegram_default_transport_mode() -> &'static str {
    TELEGRAM_AUTO_TRANSPORT_MODE
}

pub fn telegram_subagent_session_mode_reset_updates() -> HashMap<String, serde_json::Value> {
    HashMap::from([(
        TELEGRAM_SUBAGENT_SESSION_MODE_FIELD.to_string(),
        serde_json::Value::String(TELEGRAM_DEFAULT_SUBAGENT_SESSION_MODE.to_string()),
    )])
}

pub fn is_telegram_subagent_session_mode_key(key: &str) -> bool {
    matches!(
        key,
        "telegram_subagent_session_mode" | "channels.telegram_subagent_session_mode"
    )
}

pub fn is_telegram_transport_mode_key(key: &str) -> bool {
    matches!(
        key,
        "telegram_transport_mode" | "channels.telegram_transport_mode"
    )
}

pub fn telegram_transport_runtime_updates(
    diagnostics: Option<&serde_json::Value>,
    transport_mode: &str,
) -> HashMap<String, serde_json::Value> {
    let host_tunnel_url = diagnostics
        .and_then(|diag| diag.get("host_tunnel_url"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let host_webhook_capable = diagnostics
        .and_then(|diag| diag.get("host_webhook_capable"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let host_transport_reason = diagnostics
        .and_then(|diag| diag.get("host_transport_reason"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let tunnel_url_value = if transport_mode == TELEGRAM_AUTO_TRANSPORT_MODE && host_webhook_capable
    {
        host_tunnel_url
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::Null
    };
    let transport_reason_value = match transport_mode {
        "polling" => serde_json::Value::String("operator forced polling".to_string()),
        _ => host_transport_reason
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    };

    HashMap::from([
        (
            "transport_preference".to_string(),
            serde_json::Value::String(transport_mode.to_string()),
        ),
        ("tunnel_url".to_string(), tunnel_url_value),
        ("transport_reason".to_string(), transport_reason_value),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_settings_are_redacted() {
        assert!(is_sensitive_settings_key("channels.gateway_auth_token"));
        assert!(is_sensitive_settings_key("channels.gateway_principals"));
        assert!(is_sensitive_settings_key("channels.bluebubbles_password"));
        assert!(is_sensitive_settings_key("secret.OPENAI_API_KEY"));
        assert_eq!(
            redact_setting_value(
                "channels.gateway_auth_token",
                serde_json::Value::String("secret".to_string())
            ),
            serde_json::Value::String(REDACTED_SETTING_VALUE.to_string())
        );
        assert_eq!(
            redact_setting_value(
                "channels.gateway_principals",
                serde_json::json!([
                    {"token": "principal-secret", "principal_id": "alice", "role": "operator"}
                ])
            ),
            serde_json::Value::String(REDACTED_SETTING_VALUE.to_string())
        );
        assert_eq!(
            redact_setting_value(
                "display_name",
                serde_json::Value::String("ThinClaw".to_string())
            ),
            serde_json::Value::String("ThinClaw".to_string())
        );
    }

    #[test]
    fn setting_entry_validation_rejects_malformed_and_oversized_values() {
        assert!(validate_setting_entry("agent.name", &serde_json::json!("ok")).is_ok());
        assert_eq!(
            validate_setting_entry("agent..name", &serde_json::json!("bad")),
            Err(StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            validate_setting_entry("agent.name", &serde_json::json!("\0bad")),
            Err(StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            validate_setting_entry(
                "agent.name",
                &serde_json::Value::String("x".repeat(MAX_SETTING_VALUE_BYTES + 1))
            ),
            Err(StatusCode::PAYLOAD_TOO_LARGE)
        );
    }

    #[test]
    fn settings_status_helpers_preserve_existing_statuses() {
        assert_eq!(
            settings_store_unavailable_status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(setting_not_found_status(), StatusCode::NOT_FOUND);
        assert_eq!(
            sensitive_setting_write_forbidden_status(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn setting_response_projection_redacts_sensitive_rows() {
        let response = setting_response_from_row(GatewaySettingRow {
            key: "channels.gateway_auth_token".to_string(),
            value: serde_json::json!("secret"),
            updated_at: "now".to_string(),
        });

        assert_eq!(response.key, "channels.gateway_auth_token");
        assert_eq!(response.value, serde_json::json!(REDACTED_SETTING_VALUE));
        assert_eq!(response.updated_at, "now");
    }

    #[test]
    fn settings_list_response_projects_all_rows() {
        let response = settings_list_response_from_rows(vec![
            GatewaySettingRow {
                key: "display_name".to_string(),
                value: serde_json::json!("ThinClaw"),
                updated_at: "one".to_string(),
            },
            GatewaySettingRow {
                key: "channels.slack_bot_token".to_string(),
                value: serde_json::json!("secret"),
                updated_at: "two".to_string(),
            },
        ]);

        assert_eq!(response.settings.len(), 2);
        assert_eq!(response.settings[0].value, serde_json::json!("ThinClaw"));
        assert_eq!(
            response.settings[1].value,
            serde_json::json!(REDACTED_SETTING_VALUE)
        );
    }

    #[test]
    fn settings_export_response_redacts_sensitive_values() {
        let response = settings_export_response_from_map(HashMap::from([
            ("selected_model".to_string(), serde_json::json!("gpt-test")),
            (
                "channels.gateway_auth_token".to_string(),
                serde_json::json!("secret"),
            ),
        ]));

        assert_eq!(
            response.settings["selected_model"],
            serde_json::json!("gpt-test")
        );
        assert_eq!(
            response.settings["channels.gateway_auth_token"],
            serde_json::json!(REDACTED_SETTING_VALUE)
        );
    }

    #[test]
    fn imported_settings_drop_sensitive_keys() {
        let sanitized = sanitize_imported_settings(HashMap::from([
            (
                "channels.slack_bot_token".to_string(),
                serde_json::Value::String("secret".to_string()),
            ),
            (
                "channels.gateway_principals".to_string(),
                serde_json::json!([
                    {"token": "principal-secret", "principal_id": "alice", "role": "operator"}
                ]),
            ),
            (
                "selected_model".to_string(),
                serde_json::Value::String("gpt-test".to_string()),
            ),
        ]));

        assert!(!sanitized.contains_key("channels.slack_bot_token"));
        assert!(!sanitized.contains_key("channels.gateway_principals"));
        assert_eq!(
            sanitized.get("selected_model"),
            Some(&serde_json::Value::String("gpt-test".to_string()))
        );
    }

    #[test]
    fn timezone_setting_parser_accepts_null_blank_and_valid_strings() {
        assert_eq!(
            parse_timezone_setting_value(&serde_json::Value::Null, |_| false),
            Ok(None)
        );
        assert_eq!(
            parse_timezone_setting_value(&serde_json::json!("  "), |_| false),
            Ok(None)
        );
        assert_eq!(
            parse_timezone_setting_value(&serde_json::json!(" Europe/Berlin "), |value| value
                == "Europe/Berlin"),
            Ok(Some("Europe/Berlin".to_string()))
        );
    }

    #[test]
    fn timezone_setting_parser_rejects_invalid_shapes_and_values() {
        assert_eq!(
            parse_timezone_setting_value(&serde_json::json!("Mars/Base"), |_| false),
            Err(StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            parse_timezone_setting_value(&serde_json::json!(42), |_| true),
            Err(StatusCode::BAD_REQUEST)
        );
    }

    #[test]
    fn nostr_setting_prefixes_are_detected() {
        assert!(is_nostr_settings_key("channels.nostr_relay_url"));
        assert!(is_nostr_settings_key("nostr_public_key"));
        assert!(!is_nostr_settings_key("channels.telegram_transport_mode"));
    }

    #[test]
    fn llm_reload_keys_are_detected() {
        assert!(requires_llm_runtime_reload("providers.enabled"));
        assert!(requires_llm_runtime_reload("selected_model"));
        assert!(requires_llm_runtime_reload("openai_compatible_base_url"));
        assert!(!requires_llm_runtime_reload("telegram_stream_mode"));
    }

    #[test]
    fn job_code_settings_updates_are_classified() {
        assert_eq!(
            claude_code_settings_update(
                "claude_code_model",
                &serde_json::Value::String("claude-test".to_string())
            ),
            Some(ClaudeCodeSettingsUpdate {
                model: Some(Some("claude-test".to_string())),
                max_turns: None
            })
        );
        assert_eq!(
            claude_code_settings_update("claude_code_max_turns", &serde_json::json!(12)),
            Some(ClaudeCodeSettingsUpdate {
                model: None,
                max_turns: Some(Some(12))
            })
        );
        assert_eq!(
            claude_code_settings_update("claude_code_model", &serde_json::Value::Null),
            Some(ClaudeCodeSettingsUpdate {
                model: Some(None),
                max_turns: None,
            })
        );
        assert_eq!(
            codex_code_model_update(
                "codex_code_model",
                &serde_json::Value::String("gpt-code".to_string())
            ),
            Some(Some("gpt-code".to_string()))
        );
        assert_eq!(
            codex_code_model_update("codex_code_model", &serde_json::Value::Null),
            Some(None)
        );
    }

    #[test]
    fn sandbox_code_settings_are_validated_before_persistence() {
        assert_eq!(
            validate_sandbox_code_setting("claude_code_model", &serde_json::json!("claude-test")),
            Ok(())
        );
        assert_eq!(
            validate_sandbox_code_setting("claude_code_model", &serde_json::json!("\n")),
            Err(StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            validate_sandbox_code_setting("codex_code_model", &serde_json::json!(42)),
            Err(StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            validate_sandbox_code_setting("claude_code_max_turns", &serde_json::json!(0)),
            Err(StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            validate_sandbox_code_setting("claude_code_max_turns", &serde_json::json!(1001)),
            Err(StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            validate_sandbox_code_setting("claude_code_max_turns", &serde_json::Value::Null),
            Ok(())
        );
    }

    #[test]
    fn channel_stream_mode_updates_are_classified() {
        assert_eq!(
            channel_stream_mode_update(
                "channels.telegram_stream_mode",
                &serde_json::Value::String("event_chunks".to_string())
            ),
            Some(ChannelStreamModeUpdate {
                channel_name: "telegram",
                mode: StreamMode::EventChunks
            })
        );
        assert_eq!(
            channel_stream_mode_update(
                "discord_stream_mode",
                &serde_json::Value::String("status_line".to_string())
            ),
            Some(ChannelStreamModeUpdate {
                channel_name: "discord",
                mode: StreamMode::StatusLine
            })
        );
        assert_eq!(
            channel_stream_mode_update("selected_model", &serde_json::Value::Null),
            None
        );
    }

    #[test]
    fn telegram_session_mode_defaults_to_temp_topic() {
        assert_eq!(
            telegram_subagent_session_mode_update(
                "channels.telegram_subagent_session_mode",
                &serde_json::Value::String(" persistent ".to_string())
            ),
            Some("persistent".to_string())
        );
        assert_eq!(
            telegram_subagent_session_mode_update(
                "telegram_subagent_session_mode",
                &serde_json::Value::String(" ".to_string())
            ),
            Some(TELEGRAM_DEFAULT_SUBAGENT_SESSION_MODE.to_string())
        );
        assert_eq!(
            telegram_subagent_session_mode_reset_updates()
                .get(TELEGRAM_SUBAGENT_SESSION_MODE_FIELD),
            Some(&serde_json::Value::String(
                TELEGRAM_DEFAULT_SUBAGENT_SESSION_MODE.to_string()
            ))
        );
        assert_eq!(
            telegram_subagent_session_mode_update("selected_model", &serde_json::Value::Null),
            None
        );
        assert!(is_telegram_subagent_session_mode_key(
            "channels.telegram_subagent_session_mode"
        ));
        assert!(!is_telegram_subagent_session_mode_key("selected_model"));
    }

    #[test]
    fn telegram_transport_keys_are_detected() {
        assert!(is_telegram_transport_mode_key("telegram_transport_mode"));
        assert!(is_telegram_transport_mode_key(
            "channels.telegram_transport_mode"
        ));
        assert!(!is_telegram_transport_mode_key("telegram_stream_mode"));
    }

    #[test]
    fn telegram_auto_transport_uses_capable_host_tunnel() {
        let updates = telegram_transport_runtime_updates(
            Some(&serde_json::json!({
                "host_tunnel_url": " https://example.test/hook ",
                "host_webhook_capable": true,
                "host_transport_reason": "public tunnel available"
            })),
            "auto",
        );

        assert_eq!(
            updates.get("transport_preference"),
            Some(&serde_json::Value::String("auto".to_string()))
        );
        assert_eq!(
            updates.get("tunnel_url"),
            Some(&serde_json::Value::String(
                "https://example.test/hook".to_string()
            ))
        );
        assert_eq!(
            updates.get("transport_reason"),
            Some(&serde_json::Value::String(
                "public tunnel available".to_string()
            ))
        );
    }

    #[test]
    fn telegram_polling_transport_forces_reason_and_clears_tunnel() {
        let updates = telegram_transport_runtime_updates(
            Some(&serde_json::json!({
                "host_tunnel_url": "https://example.test/hook",
                "host_webhook_capable": true,
                "host_transport_reason": "public tunnel available"
            })),
            "polling",
        );

        assert_eq!(updates.get("tunnel_url"), Some(&serde_json::Value::Null));
        assert_eq!(
            updates.get("transport_reason"),
            Some(&serde_json::Value::String(
                "operator forced polling".to_string()
            ))
        );
    }
}
