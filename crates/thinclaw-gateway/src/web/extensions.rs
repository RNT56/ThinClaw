//! Root-independent extension presentation policy for the web gateway.

use axum::http::StatusCode;
use thinclaw_types::IntegrationSetupStatus;

use crate::web::types::{
    ActionResponse, ExtensionInfo, ExtensionListResponse, ExtensionSetupResponse,
    RegistryEntryInfo, RegistrySearchResponse, SecretFieldInfo, ToolInfo, ToolListResponse,
};

pub const EXTENSION_KIND_MCP_SERVER: &str = "mcp_server";
pub const EXTENSION_KIND_WASM_TOOL: &str = "wasm_tool";
pub const EXTENSION_KIND_WASM_CHANNEL: &str = "wasm_channel";
pub const TELEGRAM_EXTENSION_NAME: &str = "telegram";
pub const EXTENSION_MANAGER_UNAVAILABLE_MESSAGE: &str =
    "Extension manager not available (secrets store required)";
pub const TOOL_REGISTRY_UNAVAILABLE_MESSAGE: &str = "Tool registry not available";
pub const CHANNEL_MANAGER_UNAVAILABLE_MESSAGE: &str = "Channel manager not available";

pub fn extension_manager_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::NOT_IMPLEMENTED,
        EXTENSION_MANAGER_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn tool_registry_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        TOOL_REGISTRY_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn channel_manager_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        CHANNEL_MANAGER_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn extension_internal_error(error: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionKindHint {
    McpServer,
    WasmTool,
    WasmChannel,
}

pub fn parse_extension_kind_hint(kind: Option<&str>) -> Option<ExtensionKindHint> {
    match kind {
        Some(EXTENSION_KIND_MCP_SERVER) => Some(ExtensionKindHint::McpServer),
        Some(EXTENSION_KIND_WASM_TOOL) => Some(ExtensionKindHint::WasmTool),
        Some(EXTENSION_KIND_WASM_CHANNEL) => Some(ExtensionKindHint::WasmChannel),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmChannelActivationStatusInput<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub authenticated: bool,
    pub active: bool,
    pub activation_error: bool,
    pub has_paired: bool,
}

pub fn classify_wasm_channel_activation_status(
    input: WasmChannelActivationStatusInput<'_>,
) -> Option<&'static str> {
    if input.kind != EXTENSION_KIND_WASM_CHANNEL {
        return None;
    }

    Some(if input.activation_error {
        "failed"
    } else if !input.authenticated {
        "installed"
    } else if input.active && input.name == TELEGRAM_EXTENSION_NAME {
        if input.has_paired {
            "active"
        } else {
            "pairing"
        }
    } else if input.active {
        "active"
    } else {
        "configured"
    })
}

pub fn wasm_channel_activation_status_needs_pairing_state(
    input: WasmChannelActivationStatusInput<'_>,
) -> bool {
    input.kind == EXTENSION_KIND_WASM_CHANNEL
        && input.name == TELEGRAM_EXTENSION_NAME
        && input.authenticated
        && input.active
        && !input.activation_error
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionReconnectSupportInput<'a> {
    pub kind: &'a str,
    pub name: &'a str,
}

pub fn classify_extension_reconnect_support(input: ExtensionReconnectSupportInput<'_>) -> bool {
    input.kind == EXTENSION_KIND_WASM_CHANNEL && input.name == TELEGRAM_EXTENSION_NAME
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionRegistryEntrySource {
    WasmBuildable,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionInstallFallbackInput<'a> {
    pub name: &'a str,
    pub registry_source: Option<ExtensionRegistryEntrySource>,
}

pub fn extension_manager_unavailable_install_message(
    input: ExtensionInstallFallbackInput<'_>,
) -> String {
    match input.registry_source {
        Some(ExtensionRegistryEntrySource::WasmBuildable) => format!(
            "'{}' requires building from source. Run `thinclaw registry install {}` from the CLI.",
            input.name, input.name
        ),
        Some(ExtensionRegistryEntrySource::Other) => format!(
            "Extension manager not available (secrets store required). Configure DATABASE_URL or a secrets backend to enable installation of '{}'.",
            input.name
        ),
        None => EXTENSION_MANAGER_UNAVAILABLE_MESSAGE.to_string(),
    }
}

pub fn extension_manager_unavailable_install_response(
    input: ExtensionInstallFallbackInput<'_>,
) -> ActionResponse {
    ActionResponse::fail(extension_manager_unavailable_install_message(input))
}

pub fn extension_action_success_response(message: impl Into<String>) -> ActionResponse {
    ActionResponse::ok(message)
}

pub fn extension_action_error_response(message: impl Into<String>) -> ActionResponse {
    ActionResponse::fail(message)
}

pub fn activation_error_needs_auth(error: &str) -> bool {
    error.contains("authentication") || error.contains("401") || error.contains("Unauthorized")
}

pub fn extension_auth_status_is_authenticated(status: &str) -> bool {
    status == "authenticated"
}

pub fn extension_auth_status_allows_activation_retry(status: &str) -> bool {
    matches!(status, "authenticated" | "no_auth_required")
}

#[derive(Debug, Clone)]
pub struct ExtensionAuthRequiredResponseInput<'a> {
    pub extension_name: &'a str,
    pub auth_url: Option<String>,
    pub setup_url: Option<String>,
    pub auth_mode: Option<String>,
    pub auth_status: Option<String>,
    pub awaiting_token: bool,
    pub instructions: Option<String>,
    pub shared_auth_provider: Option<String>,
    pub missing_scopes: Vec<String>,
}

pub fn extension_auth_required_response(
    input: ExtensionAuthRequiredResponseInput<'_>,
) -> ActionResponse {
    let mut response = ActionResponse::fail(
        input
            .instructions
            .clone()
            .unwrap_or_else(|| format!("'{}' requires authentication.", input.extension_name)),
    );
    response.auth_url = input.auth_url;
    response.setup_url = input.setup_url;
    response.auth_mode = input.auth_mode;
    response.auth_status = input.auth_status;
    response.awaiting_token = Some(input.awaiting_token);
    response.instructions = input.instructions;
    response.shared_auth_provider = input.shared_auth_provider;
    response.missing_scopes = input.missing_scopes;
    response
}

pub fn extension_authentication_failed_response(error: impl ToString) -> ActionResponse {
    ActionResponse::fail(format!("Authentication failed: {}", error.to_string()))
}

pub fn extension_reconnect_refresh_failed_response(
    name: &str,
    error: impl ToString,
) -> ActionResponse {
    ActionResponse::fail(format!(
        "Failed to refresh '{}': {}",
        name,
        error.to_string()
    ))
}

pub fn extension_reconnect_success_response(name: &str) -> ActionResponse {
    ActionResponse::ok(format!("Reconnected '{}'", name))
}

pub fn extension_reconnect_failed_response(name: &str, error: impl ToString) -> ActionResponse {
    ActionResponse::fail(format!(
        "Reconnect failed for '{}': {}",
        name,
        error.to_string()
    ))
}

pub fn extension_setup_save_response(
    message: impl Into<String>,
    activated: bool,
) -> ActionResponse {
    let mut response = ActionResponse::ok(message);
    response.activated = Some(activated);
    if !activated {
        response.needs_restart = Some(true);
    }
    response
}

#[derive(Debug)]
pub struct ExtensionInfoInput {
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
    pub url: Option<String>,
    pub authenticated: bool,
    pub auth_mode: String,
    pub auth_status: String,
    pub active: bool,
    pub tools: Vec<String>,
    pub needs_setup: bool,
    pub shared_auth_provider: Option<String>,
    pub missing_scopes: Vec<String>,
    pub activation_status: Option<String>,
    pub activation_error: Option<String>,
    pub channel_diagnostics: Option<serde_json::Value>,
    pub reconnect_supported: bool,
    pub setup: IntegrationSetupStatus,
}

pub fn extension_info(input: ExtensionInfoInput) -> ExtensionInfo {
    ExtensionInfo {
        name: input.name,
        kind: input.kind,
        description: input.description,
        url: input.url,
        authenticated: input.authenticated,
        auth_mode: input.auth_mode,
        auth_status: input.auth_status,
        active: input.active,
        tools: input.tools,
        needs_setup: input.needs_setup,
        shared_auth_provider: input.shared_auth_provider,
        missing_scopes: input.missing_scopes,
        activation_status: input.activation_status,
        activation_error: input.activation_error,
        channel_diagnostics: input.channel_diagnostics,
        reconnect_supported: input.reconnect_supported,
        setup: input.setup,
    }
}

#[derive(Debug)]
pub struct InstalledExtensionInfoInput {
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
    pub url: Option<String>,
    pub authenticated: bool,
    pub auth_mode: String,
    pub auth_status: String,
    pub active: bool,
    pub tools: Vec<String>,
    pub needs_setup: bool,
    pub shared_auth_provider: Option<String>,
    pub missing_scopes: Vec<String>,
    pub activation_error: Option<String>,
    pub has_paired: bool,
    pub channel_diagnostics: Option<serde_json::Value>,
    pub setup: IntegrationSetupStatus,
}

pub fn extension_info_needs_channel_diagnostics(kind: &str) -> bool {
    kind == EXTENSION_KIND_WASM_CHANNEL
}

pub fn installed_extension_info(input: InstalledExtensionInfoInput) -> ExtensionInfo {
    let activation_status =
        classify_wasm_channel_activation_status(WasmChannelActivationStatusInput {
            kind: &input.kind,
            name: &input.name,
            authenticated: input.authenticated,
            active: input.active,
            activation_error: input.activation_error.is_some(),
            has_paired: input.has_paired,
        })
        .map(str::to_string);
    let reconnect_supported =
        classify_extension_reconnect_support(ExtensionReconnectSupportInput {
            kind: &input.kind,
            name: &input.name,
        });

    extension_info(ExtensionInfoInput {
        name: input.name,
        kind: input.kind,
        description: input.description,
        url: input.url,
        authenticated: input.authenticated,
        auth_mode: input.auth_mode,
        auth_status: input.auth_status,
        active: input.active,
        tools: input.tools,
        needs_setup: input.needs_setup,
        shared_auth_provider: input.shared_auth_provider,
        missing_scopes: input.missing_scopes,
        activation_status,
        activation_error: input.activation_error,
        channel_diagnostics: input.channel_diagnostics,
        reconnect_supported,
        setup: input.setup,
    })
}

pub fn extension_list_response(extensions: Vec<ExtensionInfo>) -> ExtensionListResponse {
    ExtensionListResponse { extensions }
}

pub fn extension_list_response_from_installed_inputs(
    extensions: impl IntoIterator<Item = InstalledExtensionInfoInput>,
) -> ExtensionListResponse {
    extension_list_response(
        extensions
            .into_iter()
            .map(installed_extension_info)
            .collect(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInfoInput {
    pub name: String,
    pub description: String,
}

pub fn tool_info(name: impl Into<String>, description: impl Into<String>) -> ToolInfo {
    ToolInfo {
        name: name.into(),
        description: description.into(),
    }
}

pub fn tool_info_from_input(input: ToolInfoInput) -> ToolInfo {
    tool_info(input.name, input.description)
}

pub fn tool_list_response(tools: Vec<ToolInfo>) -> ToolListResponse {
    ToolListResponse { tools }
}

pub fn tool_list_response_from_inputs(
    tools: impl IntoIterator<Item = ToolInfoInput>,
) -> ToolListResponse {
    tool_list_response(tools.into_iter().map(tool_info_from_input).collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryEntrySearchInput<'a> {
    pub name: &'a str,
    pub display_name: &'a str,
    pub description: &'a str,
    pub keywords: &'a [String],
}

pub fn registry_entry_matches_query(entry: RegistryEntrySearchInput<'_>, query: &str) -> bool {
    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();
    if tokens.is_empty() {
        return true;
    }

    let name = entry.name.to_lowercase();
    let display = entry.display_name.to_lowercase();
    let desc = entry.description.to_lowercase();
    tokens.iter().any(|token| {
        name.contains(token)
            || display.contains(token)
            || desc.contains(token)
            || entry
                .keywords
                .iter()
                .any(|keyword| keyword.to_lowercase().contains(token))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntryInfoInput {
    pub name: String,
    pub display_name: String,
    pub kind: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntryProjectionInput {
    pub name: String,
    pub display_name: String,
    pub kind: String,
    pub description: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledExtensionRegistryKey {
    pub name: String,
    pub kind: String,
}

pub fn registry_entry_is_installed(
    entry: &RegistryEntryProjectionInput,
    installed: &[InstalledExtensionRegistryKey],
) -> bool {
    installed
        .iter()
        .any(|installed| installed.name == entry.name && installed.kind == entry.kind)
}

pub fn registry_entry_info(input: RegistryEntryInfoInput) -> RegistryEntryInfo {
    RegistryEntryInfo {
        name: input.name,
        display_name: input.display_name,
        kind: input.kind,
        description: input.description,
        keywords: input.keywords,
        installed: input.installed,
    }
}

pub fn registry_entry_info_from_projection(
    input: RegistryEntryProjectionInput,
    installed: &[InstalledExtensionRegistryKey],
) -> RegistryEntryInfo {
    let installed = registry_entry_is_installed(&input, installed);
    registry_entry_info(RegistryEntryInfoInput {
        name: input.name,
        display_name: input.display_name,
        kind: input.kind,
        description: input.description,
        keywords: input.keywords,
        installed,
    })
}

pub fn registry_search_response(entries: Vec<RegistryEntryInfo>) -> RegistrySearchResponse {
    RegistrySearchResponse { entries }
}

pub fn registry_search_response_from_inputs(
    entries: impl IntoIterator<Item = RegistryEntryProjectionInput>,
    installed: &[InstalledExtensionRegistryKey],
    query: &str,
) -> RegistrySearchResponse {
    registry_search_response(
        entries
            .into_iter()
            .filter(|entry| {
                registry_entry_matches_query(
                    RegistryEntrySearchInput {
                        name: &entry.name,
                        display_name: &entry.display_name,
                        description: &entry.description,
                        keywords: &entry.keywords,
                    },
                    query,
                )
            })
            .map(|entry| registry_entry_info_from_projection(entry, installed))
            .collect(),
    )
}

#[derive(Debug, Clone)]
pub struct ExtensionSetupResponseInput {
    pub name: String,
    pub kind: String,
    pub mode: String,
    pub auth_status: String,
    pub fields: Vec<SecretFieldInfo>,
    pub auth_url: Option<String>,
    pub instructions: Option<String>,
    pub setup_url: Option<String>,
    pub validation_url: Option<String>,
    pub shared_auth_provider: Option<String>,
    pub missing_scopes: Vec<String>,
}

pub fn extension_setup_response(input: ExtensionSetupResponseInput) -> ExtensionSetupResponse {
    ExtensionSetupResponse {
        name: input.name,
        kind: input.kind,
        mode: input.mode,
        auth_status: input.auth_status,
        fields: input.fields,
        auth_url: input.auth_url,
        instructions: input.instructions,
        setup_url: input.setup_url,
        validation_url: input.validation_url,
        shared_auth_provider: input.shared_auth_provider,
        missing_scopes: input.missing_scopes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activation_input(
        kind: &'static str,
        name: &'static str,
    ) -> WasmChannelActivationStatusInput<'static> {
        WasmChannelActivationStatusInput {
            kind,
            name,
            authenticated: true,
            active: false,
            activation_error: false,
            has_paired: false,
        }
    }

    #[test]
    fn extension_activation_status_classifies_wasm_channels() {
        let mut input = activation_input(EXTENSION_KIND_WASM_CHANNEL, "search");
        assert_eq!(
            classify_wasm_channel_activation_status(input),
            Some("configured")
        );

        input.activation_error = true;
        assert_eq!(
            classify_wasm_channel_activation_status(input),
            Some("failed")
        );

        input.activation_error = false;
        input.authenticated = false;
        assert_eq!(
            classify_wasm_channel_activation_status(input),
            Some("installed")
        );

        input.authenticated = true;
        input.active = true;
        assert_eq!(
            classify_wasm_channel_activation_status(input),
            Some("active")
        );
    }

    #[test]
    fn extension_activation_status_classifies_telegram_pairing() {
        let mut input = activation_input(EXTENSION_KIND_WASM_CHANNEL, TELEGRAM_EXTENSION_NAME);
        input.active = true;

        assert!(wasm_channel_activation_status_needs_pairing_state(input));
        assert_eq!(
            classify_wasm_channel_activation_status(input),
            Some("pairing")
        );

        input.has_paired = true;
        assert_eq!(
            classify_wasm_channel_activation_status(input),
            Some("active")
        );

        input.activation_error = true;
        assert!(!wasm_channel_activation_status_needs_pairing_state(input));
    }

    #[test]
    fn extension_activation_status_ignores_non_wasm_channels() {
        assert_eq!(
            classify_wasm_channel_activation_status(activation_input("wasm_tool", "search")),
            None
        );
    }

    #[test]
    fn extension_install_fallback_messages_match_web_contract() {
        assert_eq!(
            extension_manager_unavailable_install_message(ExtensionInstallFallbackInput {
                name: "telegram",
                registry_source: Some(ExtensionRegistryEntrySource::WasmBuildable),
            }),
            "'telegram' requires building from source. Run `thinclaw registry install telegram` from the CLI."
        );

        assert_eq!(
            extension_manager_unavailable_install_message(ExtensionInstallFallbackInput {
                name: "notion",
                registry_source: Some(ExtensionRegistryEntrySource::Other),
            }),
            "Extension manager not available (secrets store required). Configure DATABASE_URL or a secrets backend to enable installation of 'notion'."
        );

        assert_eq!(
            extension_manager_unavailable_install_message(ExtensionInstallFallbackInput {
                name: "missing",
                registry_source: None,
            }),
            EXTENSION_MANAGER_UNAVAILABLE_MESSAGE
        );

        let response =
            extension_manager_unavailable_install_response(ExtensionInstallFallbackInput {
                name: "missing",
                registry_source: None,
            });
        assert!(!response.success);
        assert_eq!(response.message, EXTENSION_MANAGER_UNAVAILABLE_MESSAGE);
    }

    #[test]
    fn extension_unavailable_errors_match_web_statuses() {
        assert_eq!(
            extension_manager_unavailable_error(),
            (
                StatusCode::NOT_IMPLEMENTED,
                EXTENSION_MANAGER_UNAVAILABLE_MESSAGE.to_string()
            )
        );
        assert_eq!(
            tool_registry_unavailable_error(),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                TOOL_REGISTRY_UNAVAILABLE_MESSAGE.to_string()
            )
        );
        assert_eq!(
            channel_manager_unavailable_error(),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                CHANNEL_MANAGER_UNAVAILABLE_MESSAGE.to_string()
            )
        );
        assert_eq!(
            extension_internal_error("boom"),
            (StatusCode::INTERNAL_SERVER_ERROR, "boom".to_string())
        );
    }

    #[test]
    fn extension_kind_hint_parses_supported_wire_values() {
        assert_eq!(
            parse_extension_kind_hint(Some(EXTENSION_KIND_MCP_SERVER)),
            Some(ExtensionKindHint::McpServer)
        );
        assert_eq!(
            parse_extension_kind_hint(Some(EXTENSION_KIND_WASM_TOOL)),
            Some(ExtensionKindHint::WasmTool)
        );
        assert_eq!(
            parse_extension_kind_hint(Some(EXTENSION_KIND_WASM_CHANNEL)),
            Some(ExtensionKindHint::WasmChannel)
        );
        assert_eq!(parse_extension_kind_hint(Some("unknown")), None);
        assert_eq!(parse_extension_kind_hint(None), None);
    }

    #[test]
    fn auth_status_helpers_preserve_api_and_web_retry_policy() {
        assert!(extension_auth_status_is_authenticated("authenticated"));
        assert!(!extension_auth_status_is_authenticated("no_auth_required"));
        assert!(extension_auth_status_allows_activation_retry(
            "authenticated"
        ));
        assert!(extension_auth_status_allows_activation_retry(
            "no_auth_required"
        ));
        assert!(!extension_auth_status_allows_activation_retry(
            "awaiting_authorization"
        ));
    }

    #[test]
    fn auth_required_response_preserves_optional_web_fields() {
        let response = extension_auth_required_response(ExtensionAuthRequiredResponseInput {
            extension_name: "calendar",
            auth_url: Some("https://auth.example".to_string()),
            setup_url: Some("https://setup.example".to_string()),
            auth_mode: Some("oauth".to_string()),
            auth_status: Some("awaiting_authorization".to_string()),
            awaiting_token: true,
            instructions: None,
            shared_auth_provider: Some("google".to_string()),
            missing_scopes: vec!["calendar.read".to_string()],
        });

        assert!(!response.success);
        assert_eq!(response.message, "'calendar' requires authentication.");
        assert_eq!(response.auth_url.as_deref(), Some("https://auth.example"));
        assert_eq!(response.setup_url.as_deref(), Some("https://setup.example"));
        assert_eq!(response.auth_mode.as_deref(), Some("oauth"));
        assert_eq!(
            response.auth_status.as_deref(),
            Some("awaiting_authorization")
        );
        assert_eq!(response.awaiting_token, Some(true));
        assert_eq!(response.shared_auth_provider.as_deref(), Some("google"));
        assert_eq!(response.missing_scopes, vec!["calendar.read"]);
    }

    #[test]
    fn auth_and_reconnect_response_messages_match_web_contract() {
        assert_eq!(
            extension_action_success_response("installed").message,
            "installed"
        );
        assert_eq!(extension_action_error_response("failed").message, "failed");
        assert_eq!(
            extension_authentication_failed_response("denied").message,
            "Authentication failed: denied"
        );
        assert_eq!(
            extension_reconnect_refresh_failed_response("telegram", "expired").message,
            "Failed to refresh 'telegram': expired"
        );
        assert_eq!(
            extension_reconnect_success_response("telegram").message,
            "Reconnected 'telegram'"
        );
        assert_eq!(
            extension_reconnect_failed_response("telegram", "offline").message,
            "Reconnect failed for 'telegram': offline"
        );
    }

    #[test]
    fn setup_save_response_marks_restart_only_when_activation_failed() {
        let activated = extension_setup_save_response("saved", true);
        assert!(activated.success);
        assert_eq!(activated.activated, Some(true));
        assert_eq!(activated.needs_restart, None);

        let needs_restart = extension_setup_save_response("saved", false);
        assert!(needs_restart.success);
        assert_eq!(needs_restart.activated, Some(false));
        assert_eq!(needs_restart.needs_restart, Some(true));
    }

    #[test]
    fn installed_extension_projection_derives_wasm_channel_policy() {
        let response =
            extension_list_response_from_installed_inputs(vec![InstalledExtensionInfoInput {
                name: TELEGRAM_EXTENSION_NAME.to_string(),
                kind: EXTENSION_KIND_WASM_CHANNEL.to_string(),
                description: Some("Chat bridge".to_string()),
                url: Some("https://example.test/telegram.wasm".to_string()),
                authenticated: true,
                auth_mode: "oauth".to_string(),
                auth_status: "authenticated".to_string(),
                active: true,
                tools: vec!["telegram.send".to_string()],
                needs_setup: true,
                shared_auth_provider: Some("telegram".to_string()),
                missing_scopes: vec!["messages.write".to_string()],
                activation_error: None,
                has_paired: false,
                channel_diagnostics: Some(serde_json::json!({"connected": true})),
                setup: IntegrationSetupStatus::default(),
            }]);

        let extension = response.extensions.first().expect("projected extension");
        assert_eq!(extension.name, TELEGRAM_EXTENSION_NAME);
        assert_eq!(extension.activation_status.as_deref(), Some("pairing"));
        assert!(extension.reconnect_supported);
        assert_eq!(
            extension.channel_diagnostics,
            Some(serde_json::json!({"connected": true}))
        );
        assert_eq!(
            serde_json::to_value(&response).expect("serialize extension list"),
            serde_json::json!({
                "extensions": [{
                    "name": TELEGRAM_EXTENSION_NAME,
                    "kind": EXTENSION_KIND_WASM_CHANNEL,
                    "description": "Chat bridge",
                    "url": "https://example.test/telegram.wasm",
                    "authenticated": true,
                    "auth_mode": "oauth",
                    "auth_status": "authenticated",
                    "active": true,
                    "tools": ["telegram.send"],
                    "needs_setup": true,
                    "shared_auth_provider": "telegram",
                    "missing_scopes": ["messages.write"],
                    "activation_status": "pairing",
                    "channel_diagnostics": {"connected": true},
                    "reconnect_supported": true,
                    "setup": {
                        "state": "installed_unconfigured",
                        "auth_mode": "none",
                        "actions": ["validate"]
                    }
                }]
            })
        );
    }

    #[test]
    fn registry_search_matches_any_token_across_entry_fields() {
        let keywords = vec!["oauth".to_string(), "chat".to_string()];
        let entry = RegistryEntrySearchInput {
            name: "telegram",
            display_name: "Telegram",
            description: "Message users",
            keywords: &keywords,
        };

        assert!(registry_entry_matches_query(entry, ""));
        assert!(registry_entry_matches_query(entry, "gram"));
        assert!(registry_entry_matches_query(entry, "message"));
        assert!(registry_entry_matches_query(entry, "oauth"));
        assert!(registry_entry_matches_query(entry, "missing oauth"));
        assert!(!registry_entry_matches_query(entry, "slack"));
    }

    #[test]
    fn extension_projection_responses_preserve_json_shapes() {
        let tools = tool_list_response_from_inputs(vec![ToolInfoInput {
            name: "memory.search".to_string(),
            description: "Search memory".to_string(),
        }]);
        let tools_value = serde_json::to_value(tools).expect("serialize tools");
        assert_eq!(
            tools_value,
            serde_json::json!({
                "tools": [{
                    "name": "memory.search",
                    "description": "Search memory",
                }]
            })
        );

        let registry = registry_search_response_from_inputs(
            vec![
                RegistryEntryProjectionInput {
                    name: "telegram".to_string(),
                    display_name: "Telegram".to_string(),
                    kind: EXTENSION_KIND_WASM_CHANNEL.to_string(),
                    description: "Chat bridge".to_string(),
                    keywords: vec!["chat".to_string()],
                },
                RegistryEntryProjectionInput {
                    name: "notes".to_string(),
                    display_name: "Notes".to_string(),
                    kind: EXTENSION_KIND_WASM_TOOL.to_string(),
                    description: "Capture notes".to_string(),
                    keywords: vec!["writing".to_string()],
                },
            ],
            &[InstalledExtensionRegistryKey {
                name: "telegram".to_string(),
                kind: EXTENSION_KIND_WASM_CHANNEL.to_string(),
            }],
            "chat",
        );
        let registry_value = serde_json::to_value(registry).expect("serialize registry");
        assert_eq!(
            registry_value,
            serde_json::json!({
                "entries": [{
                    "name": "telegram",
                    "display_name": "Telegram",
                    "kind": EXTENSION_KIND_WASM_CHANNEL,
                    "description": "Chat bridge",
                    "keywords": ["chat"],
                    "installed": true,
                }]
            })
        );
    }
}
