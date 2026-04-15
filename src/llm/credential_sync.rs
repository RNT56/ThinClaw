use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::task::JoinHandle;

use crate::config::{
    ClaudeCodeConfig, CodexCodeConfig, clear_synced_oauth_vars, replace_synced_oauth_vars,
};
use crate::settings::{
    OAuthCredentialSourceConfig, OAuthCredentialSourceKind, ProviderCredentialMode,
    ProvidersSettings,
};

use super::runtime_manager::LlmRuntimeManager;

const MIN_POLL_INTERVAL_SECS: u64 = 5;
const TOKEN_CANDIDATE_KEYS: &[&str] = &[
    "access_token",
    "accessToken",
    "token",
    "api_key",
    "apiKey",
    "id_token",
];

#[derive(Debug, Clone)]
struct ResolvedOAuthSource {
    source_id: String,
    label: String,
    env_key: String,
    kind: OAuthCredentialSourceKind,
    path: Option<PathBuf>,
    json_pointer: Option<String>,
}

pub struct OAuthCredentialSyncHandle {
    join_handle: JoinHandle<()>,
}

impl Drop for OAuthCredentialSyncHandle {
    fn drop(&mut self) {
        self.join_handle.abort();
    }
}

impl OAuthCredentialSyncHandle {
    pub fn start(runtime: Arc<LlmRuntimeManager>, providers: &ProvidersSettings) -> Option<Self> {
        let sources = resolved_sources(providers);
        if sources.is_empty() {
            clear_synced_oauth_vars();
            return None;
        }

        let poll_interval = Duration::from_secs(
            providers
                .oauth_sync_poll_interval_secs
                .max(MIN_POLL_INTERVAL_SECS),
        );
        let mut fingerprints = current_fingerprints(&sources);

        let join_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(poll_interval).await;

                let (snapshot, next_fingerprints) = snapshot_source_state(&sources);
                if next_fingerprints == fingerprints {
                    continue;
                }

                fingerprints = next_fingerprints;
                let synced_count = replace_synced_oauth_vars(snapshot);
                tracing::info!(
                    synced_count,
                    "External OAuth credential sync updated provider auth overlay; reloading LLM runtime"
                );

                if let Err(error) = runtime.reload().await {
                    tracing::warn!(error = %error, "Failed to reload LLM runtime after OAuth credential sync");
                }
            }
        });

        Some(Self { join_handle })
    }
}

pub fn prime_runtime_oauth_credentials(providers: &ProvidersSettings) -> usize {
    let sources = resolved_sources(providers);
    if sources.is_empty() {
        clear_synced_oauth_vars();
        return 0;
    }

    let (snapshot, _) = snapshot_source_state(&sources);
    replace_synced_oauth_vars(snapshot)
}

pub fn provider_oauth_source_kind(slug: &str) -> Option<OAuthCredentialSourceKind> {
    match slug {
        "anthropic" => Some(OAuthCredentialSourceKind::ClaudeCode),
        "openai" => Some(OAuthCredentialSourceKind::OpenAiCodex),
        _ => None,
    }
}

pub fn oauth_source_label(kind: OAuthCredentialSourceKind) -> &'static str {
    match kind {
        OAuthCredentialSourceKind::ClaudeCode => "Claude Code auth",
        OAuthCredentialSourceKind::OpenAiCodex => "Codex auth",
        OAuthCredentialSourceKind::JsonFile => "Custom JSON auth",
    }
}

pub fn oauth_source_location_hint(kind: OAuthCredentialSourceKind) -> String {
    match kind {
        OAuthCredentialSourceKind::ClaudeCode => {
            if cfg!(target_os = "macos") {
                "macOS Keychain service 'Claude Code-credentials'".to_string()
            } else {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("~"))
                    .join(".claude")
                    .join(".credentials.json")
                    .display()
                    .to_string()
            }
        }
        OAuthCredentialSourceKind::OpenAiCodex => default_codex_auth_path()
            .unwrap_or_else(CodexCodeConfig::resolved_auth_file_path)
            .display()
            .to_string(),
        OAuthCredentialSourceKind::JsonFile => "Configured JSON file".to_string(),
    }
}

pub fn oauth_source_available(kind: OAuthCredentialSourceKind) -> bool {
    match kind {
        OAuthCredentialSourceKind::ClaudeCode => ClaudeCodeConfig::extract_oauth_token()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
        OAuthCredentialSourceKind::OpenAiCodex => default_codex_auth_path()
            .map(|path| path.is_file())
            .unwrap_or(false),
        OAuthCredentialSourceKind::JsonFile => false,
    }
}

fn resolved_sources(providers: &ProvidersSettings) -> Vec<ResolvedOAuthSource> {
    let mut sources = Vec::new();
    let include_legacy_builtins =
        providers.oauth_sync_enabled && providers.provider_credential_modes.is_empty();
    let selected_builtin_kinds = explicitly_selected_builtin_sources(providers);

    for kind in [
        OAuthCredentialSourceKind::ClaudeCode,
        OAuthCredentialSourceKind::OpenAiCodex,
    ] {
        if include_legacy_builtins || selected_builtin_kinds.contains(&kind) {
            sources.push(builtin_source(kind));
        }
    }

    if providers.oauth_sync_enabled {
        for (idx, source) in providers.oauth_sync_sources.iter().enumerate() {
            sources.push(resolve_custom_source(source, idx));
        }
    }

    sources
}

fn explicitly_selected_builtin_sources(
    providers: &ProvidersSettings,
) -> HashSet<OAuthCredentialSourceKind> {
    providers
        .provider_credential_modes
        .iter()
        .filter_map(|(slug, mode)| {
            (*mode == ProviderCredentialMode::ExternalOAuthSync)
                .then(|| provider_oauth_source_kind(slug))
                .flatten()
        })
        .collect()
}

fn builtin_source(kind: OAuthCredentialSourceKind) -> ResolvedOAuthSource {
    match kind {
        OAuthCredentialSourceKind::ClaudeCode => ResolvedOAuthSource {
            source_id: "claude_code".to_string(),
            label: oauth_source_label(kind).to_string(),
            env_key: "ANTHROPIC_API_KEY".to_string(),
            kind,
            path: None,
            json_pointer: None,
        },
        OAuthCredentialSourceKind::OpenAiCodex => ResolvedOAuthSource {
            source_id: "openai_codex".to_string(),
            label: oauth_source_label(kind).to_string(),
            env_key: "OPENAI_API_KEY".to_string(),
            kind,
            path: default_codex_auth_path(),
            json_pointer: None,
        },
        OAuthCredentialSourceKind::JsonFile => ResolvedOAuthSource {
            source_id: "custom_json".to_string(),
            label: oauth_source_label(kind).to_string(),
            env_key: "LLM_API_KEY".to_string(),
            kind,
            path: None,
            json_pointer: None,
        },
    }
}

fn resolve_custom_source(
    source: &OAuthCredentialSourceConfig,
    index: usize,
) -> ResolvedOAuthSource {
    let default_env_key = match source.kind {
        OAuthCredentialSourceKind::ClaudeCode => "ANTHROPIC_API_KEY",
        OAuthCredentialSourceKind::OpenAiCodex => "OPENAI_API_KEY",
        OAuthCredentialSourceKind::JsonFile => "LLM_API_KEY",
    };

    let default_path = match source.kind {
        OAuthCredentialSourceKind::ClaudeCode => None,
        OAuthCredentialSourceKind::OpenAiCodex => default_codex_auth_path(),
        OAuthCredentialSourceKind::JsonFile => source.path.clone(),
    };

    ResolvedOAuthSource {
        source_id: format!("custom_{index}"),
        label: format!("custom {}", index + 1),
        env_key: source
            .env_key
            .clone()
            .unwrap_or_else(|| default_env_key.to_string()),
        kind: source.kind,
        path: source.path.clone().or(default_path),
        json_pointer: source.json_pointer.clone(),
    }
}

fn default_codex_auth_path() -> Option<PathBuf> {
    Some(CodexCodeConfig::resolved_auth_file_path())
}

fn current_fingerprints(sources: &[ResolvedOAuthSource]) -> HashMap<String, String> {
    snapshot_source_state(sources).1
}

fn snapshot_source_state(
    sources: &[ResolvedOAuthSource],
) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut values = HashMap::new();
    let mut fingerprints = HashMap::new();

    for source in sources {
        match load_source_token(source) {
            Ok(Some(token)) => {
                fingerprints.insert(source.source_id.clone(), fingerprint(&token));
                values.insert(source.env_key.clone(), token);
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    source = %source.label,
                    env_key = %source.env_key,
                    error = %error,
                    "Failed to load watched OAuth credential source"
                );
            }
        }
    }

    (values, fingerprints)
}

fn load_source_token(source: &ResolvedOAuthSource) -> Result<Option<String>, String> {
    match source.kind {
        OAuthCredentialSourceKind::ClaudeCode => {
            Ok(ClaudeCodeConfig::extract_oauth_token().filter(|value| !value.trim().is_empty()))
        }
        OAuthCredentialSourceKind::OpenAiCodex | OAuthCredentialSourceKind::JsonFile => {
            let path = source
                .path
                .as_ref()
                .ok_or_else(|| format!("{} source is missing a file path", source.label))?;
            if !path.is_file() {
                return Ok(None);
            }
            let raw = std::fs::read_to_string(path)
                .map_err(|error| format!("failed to read '{}': {}", path.display(), error))?;
            let value: Value = serde_json::from_str(&raw)
                .map_err(|error| format!("invalid JSON in '{}': {}", path.display(), error))?;
            Ok(extract_token_from_json(
                &value,
                source.json_pointer.as_deref(),
            ))
        }
    }
}

fn extract_token_from_json(value: &Value, json_pointer: Option<&str>) -> Option<String> {
    if let Some(pointer) = json_pointer {
        return value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }

    extract_token_recursively(value)
}

fn extract_token_recursively(value: &Value) -> Option<String> {
    match value {
        Value::String(token) if !token.trim().is_empty() => None,
        Value::Object(map) => {
            for candidate in TOKEN_CANDIDATE_KEYS {
                if let Some(token) = map.get(*candidate).and_then(Value::as_str) {
                    let trimmed = token.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }

            for child in map.values() {
                if let Some(token) = extract_token_recursively(child) {
                    return Some(token);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(token) = extract_token_recursively(item) {
                    return Some(token);
                }
            }
            None
        }
        _ => None,
    }
}

fn fingerprint(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        clear_injected_vars_for_tests, helpers::synced_oauth_env, replace_synced_oauth_vars,
    };

    #[test]
    fn extract_token_recursively_prefers_common_access_token_keys() {
        let value = serde_json::json!({
            "profile": {
                "auth": {
                    "access_token": "tok-123"
                }
            }
        });

        assert_eq!(
            extract_token_from_json(&value, None),
            Some("tok-123".to_string())
        );
    }

    #[test]
    fn extract_token_from_json_honors_pointer_override() {
        let value = serde_json::json!({
            "credentials": {
                "value": "tok-abc"
            }
        });

        assert_eq!(
            extract_token_from_json(&value, Some("/credentials/value")),
            Some("tok-abc".to_string())
        );
    }

    #[test]
    fn resolved_sources_only_includes_selected_builtin_modes() {
        let mut providers = ProvidersSettings::default();
        providers.provider_credential_modes.insert(
            "openai".to_string(),
            ProviderCredentialMode::ExternalOAuthSync,
        );

        let sources = resolved_sources(&providers);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].env_key, "OPENAI_API_KEY");
    }

    #[test]
    fn resolved_sources_respects_legacy_global_toggle() {
        let providers = ProvidersSettings {
            oauth_sync_enabled: true,
            ..ProvidersSettings::default()
        };

        let sources = resolved_sources(&providers);
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn prime_runtime_oauth_credentials_replaces_overlay() {
        clear_injected_vars_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.path().join("custom-auth.json");
        std::fs::write(&auth_path, r#"{"credentials":{"token":"custom-token"}}"#).unwrap();

        replace_synced_oauth_vars(HashMap::from([(
            "THINCLAW_STALE_SYNC".to_string(),
            "stale-token".to_string(),
        )]));

        let mut provider_credential_modes = HashMap::new();
        provider_credential_modes.insert("openai".to_string(), ProviderCredentialMode::ApiKey);

        let providers = ProvidersSettings {
            oauth_sync_enabled: true,
            oauth_sync_sources: vec![OAuthCredentialSourceConfig {
                kind: OAuthCredentialSourceKind::JsonFile,
                path: Some(auth_path),
                env_key: Some("THINCLAW_TEST_OAUTH_SYNC".to_string()),
                json_pointer: Some("/credentials/token".to_string()),
            }],
            provider_credential_modes,
            ..ProvidersSettings::default()
        };

        let count = prime_runtime_oauth_credentials(&providers);
        assert_eq!(count, 1);
        assert_eq!(
            synced_oauth_env("THINCLAW_TEST_OAUTH_SYNC"),
            Some("custom-token".to_string())
        );
        assert_eq!(synced_oauth_env("THINCLAW_STALE_SYNC"), None);

        clear_injected_vars_for_tests();
    }
}
