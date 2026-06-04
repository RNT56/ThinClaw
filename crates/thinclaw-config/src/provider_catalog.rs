//! Built-in provider endpoint catalog.
//!
//! Maps provider IDs to their API endpoint details, default models, and
//! secret store key names. This catalog enables ThinClaw to work with 20+
//! providers without requiring explicit base_url configuration.
//!
//! ## Loading order
//!
//! 1. **Disk**: `registry/providers.json` resolved via CWD, executable
//!    ancestors, then the workspace root relative to this crate.
//! 2. **Embedded**: compiled-in copy from the workspace registry.
//!
//! ## Usage
//!
//! - Headless: user sets `providers.enabled = ["anthropic", "openai"]` in
//!   `config.toml`, and API keys via `SecretsStore` or env vars.
//! - Scrappy: the bridge writes the same settings from the UI config.
//!
//! The catalog is the single source of truth for provider endpoints across
//! Direct Workbench, ThinClaw Agent Cockpit, and generated client contracts.

use std::collections::HashMap;

pub use thinclaw_runtime_contracts::{ApiStyle, ProviderEndpoint};

/// Lazy-initialized catalog of all known cloud providers.
///
/// The key is the **provider slug** (matches Provider Vault identifiers, UI
/// identifiers, and the `providers.enabled` config values).
///
/// Loading order: disk `registry/providers.json` → embedded fallback.
pub fn catalog() -> &'static HashMap<String, ProviderEndpoint> {
    use std::sync::LazyLock;

    static CATALOG: LazyLock<HashMap<String, ProviderEndpoint>> = LazyLock::new(|| {
        // Try disk first.
        if let Some(dir) = find_registry_dir() {
            let path = dir.join("providers.json");
            if let Ok(contents) = std::fs::read_to_string(&path) {
                match serde_json::from_str::<Vec<ProviderEndpoint>>(&contents) {
                    Ok(entries) if !entries.is_empty() => {
                        tracing::info!(
                            path = %path.display(),
                            count = entries.len(),
                            "Loaded provider catalog from registry"
                        );
                        return entries.into_iter().map(|e| (e.slug.clone(), e)).collect();
                    }
                    Ok(_) => {
                        tracing::warn!(
                            path = %path.display(),
                            "Registry providers.json was empty, using embedded fallback"
                        );
                    }
                    Err(err) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %err,
                            "Failed to parse registry providers.json, using embedded fallback"
                        );
                    }
                }
            }
        }

        // Embedded fallback.
        let fallback = include_str!("../../../registry/providers.json");
        let entries: Vec<ProviderEndpoint> =
            serde_json::from_str(fallback).expect("embedded providers_catalog.json must be valid");
        tracing::info!(
            count = entries.len(),
            "Loaded embedded provider catalog fallback"
        );
        entries.into_iter().map(|e| (e.slug.clone(), e)).collect()
    });

    &CATALOG
}

fn find_registry_dir() -> Option<std::path::PathBuf> {
    if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join("registry");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let mut dir = Some(parent);
        for _ in 0..3 {
            if let Some(d) = dir {
                let candidate = d.join("registry");
                if candidate.is_dir() {
                    return Some(candidate);
                }
                dir = d.parent();
            }
        }
    }

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest_dir.join("../../registry");
    if candidate.is_dir() {
        return Some(candidate);
    }

    None
}

/// Look up a provider's endpoint configuration by its slug.
pub fn endpoint_for(provider_id: &str) -> Option<&'static ProviderEndpoint> {
    catalog().get(provider_id)
}

/// Get all provider slugs.
pub fn all_provider_ids() -> Vec<&'static str> {
    catalog().keys().map(String::as_str).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_not_empty() {
        assert!(!catalog().is_empty());
    }

    #[test]
    fn known_providers_exist() {
        assert!(endpoint_for("anthropic").is_some());
        assert!(endpoint_for("openai").is_some());
        assert!(endpoint_for("groq").is_some());
        assert!(endpoint_for("gemini").is_some());
    }

    #[test]
    fn unknown_provider_returns_none() {
        assert!(endpoint_for("nonexistent").is_none());
    }

    #[test]
    fn anthropic_uses_native_api() {
        let ep = endpoint_for("anthropic").unwrap();
        assert_eq!(ep.api_style, ApiStyle::Anthropic);
    }

    #[test]
    fn openai_uses_native_api() {
        let ep = endpoint_for("openai").unwrap();
        assert_eq!(ep.api_style, ApiStyle::OpenAi);
    }

    #[test]
    fn groq_uses_compatible_api() {
        let ep = endpoint_for("groq").unwrap();
        assert_eq!(ep.api_style, ApiStyle::OpenAiCompatible);
    }

    #[test]
    fn minimax_uses_current_openai_compat_endpoint() {
        let ep = endpoint_for("minimax").unwrap();
        assert_eq!(ep.base_url, "https://api.minimax.io/v1");
        assert_eq!(ep.default_model, "MiniMax-M2.7");
    }

    #[test]
    fn cohere_uses_compatibility_api() {
        let ep = endpoint_for("cohere").unwrap();
        assert_eq!(ep.base_url, "https://api.cohere.ai/compatibility/v1");
        assert_eq!(ep.default_model, "command-a-03-2025");
    }

    #[test]
    fn xiaomi_is_not_in_catalog() {
        assert!(endpoint_for("xiaomi").is_none());
    }

    #[test]
    fn openrouter_has_setup_url() {
        let ep = endpoint_for("openrouter").unwrap();
        assert!(ep.setup_url.is_some());
        assert!(ep.setup_url.as_ref().unwrap().contains("openrouter.ai"));
    }

    #[test]
    fn anthropic_has_suggested_cheap_model() {
        let ep = endpoint_for("anthropic").unwrap();
        assert_eq!(
            ep.suggested_cheap_model.as_deref(),
            Some("claude-sonnet-4-6")
        );
    }

    #[test]
    fn all_entries_have_valid_api_style() {
        for (slug, ep) in catalog() {
            assert!(
                matches!(
                    ep.api_style,
                    ApiStyle::OpenAi
                        | ApiStyle::Anthropic
                        | ApiStyle::OpenAiCompatible
                        | ApiStyle::Ollama
                ),
                "provider '{}' has unexpected api_style: {:?}",
                slug,
                ep.api_style
            );
        }
    }

    #[test]
    fn json_roundtrip_preserves_api_style() {
        let json = r#"{"id":"test","display_name":"Test","base_url":"http://test","api_style":"openai_compatible","default_model":"m","default_context_size":128000,"supports_streaming":true,"env_key_name":"K","secret_name":"s"}"#;
        let ep: ProviderEndpoint = serde_json::from_str(json).unwrap();
        assert_eq!(ep.api_style, ApiStyle::OpenAiCompatible);
        assert_eq!(ep.slug, "test");
    }
}
