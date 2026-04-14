use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::task::JoinHandle;

use crate::config::{ClaudeCodeConfig, merge_injected_vars};
use crate::settings::{OAuthCredentialSourceConfig, OAuthCredentialSourceKind, ProvidersSettings};

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
        if !providers.oauth_sync_enabled {
            return None;
        }

        let sources = resolved_sources(providers);
        if sources.is_empty() {
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

                let changed = collect_source_updates(&sources, &mut fingerprints);
                if changed.is_empty() {
                    continue;
                }

                let changed_count = merge_injected_vars(changed);
                tracing::info!(
                    changed_count,
                    "External OAuth credential sync updated overlay values; reloading LLM runtime"
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
    if !providers.oauth_sync_enabled {
        return 0;
    }
    let sources = resolved_sources(providers);
    let mut fingerprints = HashMap::new();
    let changed = collect_source_updates(&sources, &mut fingerprints);
    merge_injected_vars(changed)
}

fn resolved_sources(providers: &ProvidersSettings) -> Vec<ResolvedOAuthSource> {
    let mut sources = vec![
        ResolvedOAuthSource {
            source_id: "claude_code".to_string(),
            label: "Claude Code".to_string(),
            env_key: "ANTHROPIC_API_KEY".to_string(),
            kind: OAuthCredentialSourceKind::ClaudeCode,
            path: None,
            json_pointer: None,
        },
        ResolvedOAuthSource {
            source_id: "openai_codex".to_string(),
            label: "OpenAI Codex".to_string(),
            env_key: "OPENAI_API_KEY".to_string(),
            kind: OAuthCredentialSourceKind::OpenAiCodex,
            path: default_codex_auth_path(),
            json_pointer: None,
        },
    ];

    for (idx, source) in providers.oauth_sync_sources.iter().enumerate() {
        sources.push(resolve_custom_source(source, idx));
    }

    sources
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
    Some(dirs::home_dir()?.join(".codex").join("auth.json"))
}

fn current_fingerprints(sources: &[ResolvedOAuthSource]) -> HashMap<String, String> {
    let mut fingerprints = HashMap::new();
    for source in sources {
        if let Ok(Some(token)) = load_source_token(source) {
            fingerprints.insert(source.source_id.clone(), fingerprint(&token));
        }
    }
    fingerprints
}

fn collect_source_updates(
    sources: &[ResolvedOAuthSource],
    fingerprints: &mut HashMap<String, String>,
) -> HashMap<String, String> {
    let mut changed = HashMap::new();

    for source in sources {
        match load_source_token(source) {
            Ok(Some(token)) => {
                let next = fingerprint(&token);
                let previous = fingerprints.get(&source.source_id);
                if previous != Some(&next) {
                    fingerprints.insert(source.source_id.clone(), next);
                    changed.insert(source.env_key.clone(), token);
                }
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

    changed
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
    use crate::config::clear_injected_vars_for_tests;
    use crate::config::helpers::optional_env;

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
    fn collect_source_updates_only_emits_changed_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        std::fs::write(&auth_path, r#"{"access_token":"tok-1"}"#).unwrap();

        let source = ResolvedOAuthSource {
            source_id: "codex".to_string(),
            label: "Codex".to_string(),
            env_key: "OPENAI_API_KEY".to_string(),
            kind: OAuthCredentialSourceKind::OpenAiCodex,
            path: Some(auth_path.clone()),
            json_pointer: None,
        };

        let mut fingerprints = HashMap::new();
        let first = collect_source_updates(std::slice::from_ref(&source), &mut fingerprints);
        assert_eq!(first.get("OPENAI_API_KEY"), Some(&"tok-1".to_string()));

        let second = collect_source_updates(std::slice::from_ref(&source), &mut fingerprints);
        assert!(second.is_empty());

        std::fs::write(&auth_path, r#"{"access_token":"tok-2"}"#).unwrap();
        let third = collect_source_updates(std::slice::from_ref(&source), &mut fingerprints);
        assert_eq!(third.get("OPENAI_API_KEY"), Some(&"tok-2".to_string()));
    }

    #[test]
    fn prime_runtime_oauth_credentials_merges_custom_source_into_overlay() {
        clear_injected_vars_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.path().join("custom-auth.json");
        std::fs::write(&auth_path, r#"{"credentials":{"token":"custom-token"}}"#).unwrap();

        let providers = ProvidersSettings {
            oauth_sync_sources: vec![OAuthCredentialSourceConfig {
                kind: OAuthCredentialSourceKind::JsonFile,
                path: Some(auth_path),
                env_key: Some("THINCLAW_TEST_OAUTH_SYNC".to_string()),
                json_pointer: Some("/credentials/token".to_string()),
            }],
            ..ProvidersSettings::default()
        };

        let count = prime_runtime_oauth_credentials(&providers);
        assert!(count >= 1);
        assert_eq!(
            optional_env("THINCLAW_TEST_OAUTH_SYNC").unwrap(),
            Some("custom-token".to_string())
        );

        clear_injected_vars_for_tests();
    }
}
