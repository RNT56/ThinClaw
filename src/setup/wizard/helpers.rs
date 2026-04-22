//! Wizard helper functions: model fetchers, channel discovery, text utilities.

use std::collections::HashSet;

use crate::channels::wasm::{
    ChannelCapabilitiesFile, available_channel_names, install_bundled_channel,
};
use crate::setup::prompts::print_info;

use super::SetupError;

/// Mask password in a database URL for display.
#[cfg(feature = "postgres")]
pub(super) fn mask_password_in_url(url: &str) -> String {
    // URL format: scheme://user:password@host/database
    // Find "://" to locate start of credentials
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let credentials_start = scheme_end + 3; // After "://"

    // Find "@" to locate end of credentials
    let Some(at_pos) = url[credentials_start..].find('@') else {
        return url.to_string();
    };
    let at_abs = credentials_start + at_pos;

    // Find ":" in the credentials section (separates user from password)
    let credentials = &url[credentials_start..at_abs];
    let Some(colon_pos) = credentials.find(':') else {
        return url.to_string();
    };

    // Build masked URL: scheme://user:****@host/database
    let scheme = &url[..credentials_start]; // "postgres://"
    let username = &credentials[..colon_pos]; // "user"
    let after_at = &url[at_abs..]; // "@localhost/db"

    format!("{}{}:****{}", scheme, username, after_at)
}

/// Fetch models from the Anthropic API.
///
/// Returns `(model_id, display_label)` pairs. Falls back to static defaults on error.
pub(super) async fn fetch_anthropic_models(cached_key: Option<&str>) -> Vec<(String, String)> {
    let static_defaults = vec![
        (
            "claude-opus-4-7".into(),
            "Claude Opus 4.7 (recommended flagship)".into(),
        ),
        (
            "claude-opus-4-6".into(),
            "Claude Opus 4.6 (latest flagship)".into(),
        ),
        ("claude-sonnet-4-6".into(), "Claude Sonnet 4.6".into()),
        ("claude-opus-4-5".into(), "Claude Opus 4.5".into()),
        ("claude-sonnet-4-5".into(), "Claude Sonnet 4.5".into()),
        ("claude-haiku-4-5".into(), "Claude Haiku 4.5 (fast)".into()),
    ];

    let api_key = cached_key
        .map(String::from)
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .filter(|k| !k.is_empty());

    let api_key = match api_key {
        Some(k) => k,
        None => return static_defaults,
    };

    let client = reqwest::Client::new();
    let resp = match client
        .get("https://api.anthropic.com/v1/models")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return static_defaults,
    };

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }

    match resp.json::<ModelsResponse>().await {
        Ok(body) => {
            let mut models: Vec<(String, String)> = body
                .data
                .into_iter()
                .filter(|m| !m.id.contains("embedding") && !m.id.contains("audio"))
                .map(|m| {
                    let label = m.id.clone();
                    (m.id, label)
                })
                .collect();
            if models.is_empty() {
                return static_defaults;
            }
            models.sort_by(|a, b| a.0.cmp(&b.0));
            models
        }
        Err(_) => static_defaults,
    }
}

/// Fetch models from the OpenAI API.
///
/// Returns `(model_id, display_label)` pairs. Falls back to static defaults on error.
pub(super) async fn fetch_openai_models(cached_key: Option<&str>) -> Vec<(String, String)> {
    let static_defaults = vec![
        (
            "gpt-5.3-codex".into(),
            "GPT-5.3 Codex (latest flagship)".into(),
        ),
        ("gpt-5.2-codex".into(), "GPT-5.2 Codex".into()),
        ("gpt-5.2".into(), "GPT-5.2".into()),
        (
            "gpt-5.1-codex-mini".into(),
            "GPT-5.1 Codex Mini (fast)".into(),
        ),
        ("gpt-5".into(), "GPT-5".into()),
        ("gpt-5-mini".into(), "GPT-5 Mini".into()),
        ("gpt-4.1".into(), "GPT-4.1".into()),
        ("gpt-4.1-mini".into(), "GPT-4.1 Mini".into()),
        ("o4-mini".into(), "o4-mini (fast reasoning)".into()),
        ("o3".into(), "o3 (reasoning)".into()),
    ];

    let api_key = cached_key
        .map(String::from)
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .filter(|k| !k.is_empty());

    let api_key = match api_key {
        Some(k) => k,
        None => return static_defaults,
    };

    let client = reqwest::Client::new();
    let resp = match client
        .get("https://api.openai.com/v1/models")
        .bearer_auth(&api_key)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return static_defaults,
    };

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }

    match resp.json::<ModelsResponse>().await {
        Ok(body) => {
            let mut models: Vec<(String, String)> = body
                .data
                .into_iter()
                .filter(|m| is_openai_chat_model(&m.id))
                .map(|m| {
                    let label = m.id.clone();
                    (m.id, label)
                })
                .collect();
            if models.is_empty() {
                return static_defaults;
            }
            sort_openai_models(&mut models);
            models
        }
        Err(_) => static_defaults,
    }
}

// Delegate to the shared implementation in discovery.rs to avoid drift.
pub(super) fn is_openai_chat_model(model_id: &str) -> bool {
    crate::llm::discovery::is_openai_chat_model(model_id)
}

pub(super) fn openai_model_priority(model_id: &str) -> usize {
    crate::llm::discovery::openai_model_priority(model_id)
}

pub(super) fn sort_openai_models(models: &mut [(String, String)]) {
    models.sort_by(|a, b| {
        openai_model_priority(&a.0)
            .cmp(&openai_model_priority(&b.0))
            .then_with(|| a.0.cmp(&b.0))
    });
}

/// Fetch installed models from a local Ollama instance.
///
/// Returns `(model_name, display_label)` pairs. Falls back to static defaults on error.
pub(super) async fn fetch_ollama_models(base_url: &str) -> Vec<(String, String)> {
    let static_defaults = vec![
        ("llama3".into(), "llama3".into()),
        ("mistral".into(), "mistral".into()),
        ("codellama".into(), "codellama".into()),
    ];

    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();

    let resp = match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(_) => return static_defaults,
        Err(_) => {
            print_info("Could not connect to Ollama. Is it running?");
            return static_defaults;
        }
    };

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        name: String,
    }
    #[derive(serde::Deserialize)]
    struct TagsResponse {
        models: Vec<ModelEntry>,
    }

    match resp.json::<TagsResponse>().await {
        Ok(body) => {
            let models: Vec<(String, String)> = body
                .models
                .into_iter()
                .map(|m| {
                    let label = m.name.clone();
                    (m.name, label)
                })
                .collect();
            if models.is_empty() {
                return static_defaults;
            }
            models
        }
        Err(_) => static_defaults,
    }
}

/// Fetch models from an OpenAI-compatible endpoint.
///
/// Returns `(model_id, display_label)` pairs. Falls back to the provided
/// defaults when discovery fails or yields no usable chat models.
pub(super) async fn fetch_openai_compatible_models(
    base_url: &str,
    auth_header: Option<&str>,
    static_defaults: Vec<(String, String)>,
) -> Vec<(String, String)> {
    if base_url.trim().is_empty() {
        return static_defaults;
    }

    let result = crate::llm::discovery::ModelDiscovery::new()
        .discover_openai_compatible(base_url, auth_header)
        .await;

    let mut seen = HashSet::new();
    let mut models: Vec<(String, String)> = result
        .models
        .into_iter()
        .filter(|model| model.is_chat)
        .filter_map(|model| {
            if seen.insert(model.id.clone()) {
                Some((model.id.clone(), model.id))
            } else {
                None
            }
        })
        .collect();

    if models.is_empty() {
        return static_defaults;
    }

    models.sort_by(|a, b| {
        openai_model_priority(&a.0)
            .cmp(&openai_model_priority(&b.0))
            .then_with(|| a.0.cmp(&b.0))
    });
    models
}

/// Discover WASM channels in a directory.
///
/// Returns a list of (channel_name, capabilities_file) pairs.
pub(super) async fn discover_wasm_channels(
    dir: &std::path::Path,
) -> Vec<(String, ChannelCapabilitiesFile)> {
    let mut channels = Vec::new();

    if !dir.is_dir() {
        return channels;
    }

    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return channels,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();

        // Look for .capabilities.json files
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if !filename.ends_with(".capabilities.json") {
            continue;
        }

        // Extract channel name
        let name = filename.trim_end_matches(".capabilities.json").to_string();
        if name.is_empty() {
            continue;
        }

        // Check if corresponding .wasm file exists
        let wasm_path = dir.join(format!("{}.wasm", name));
        if !wasm_path.exists() {
            continue;
        }

        // Parse capabilities file
        match tokio::fs::read(&path).await {
            Ok(bytes) => match ChannelCapabilitiesFile::from_bytes(&bytes) {
                Ok(cap_file) => {
                    channels.push((name, cap_file));
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to parse channel capabilities file"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to read channel capabilities file"
                );
            }
        }
    }

    // Sort by name for consistent ordering
    channels.sort_by(|a, b| a.0.cmp(&b.0));
    channels
}

/// Mask an API key for display: show first 6 + last 4 chars.
///
/// Uses char-based indexing to avoid panicking on multi-byte UTF-8.
pub(super) fn mask_api_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() < 12 {
        let prefix: String = chars.iter().take(4).collect();
        return format!("{prefix}...");
    }
    let prefix: String = chars[..6].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}...{suffix}")
}

/// Capitalize the first letter of a string.
pub(super) fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
pub(super) async fn install_missing_bundled_channels(
    channels_dir: &std::path::Path,
    already_installed: &HashSet<String>,
) -> Result<Vec<String>, SetupError> {
    let mut installed = Vec::new();

    for name in available_channel_names().iter().copied() {
        if already_installed.contains(name) {
            continue;
        }

        install_bundled_channel(name, channels_dir, false)
            .await
            .map_err(SetupError::Channel)?;
        installed.push(name.to_string());
    }

    Ok(installed)
}

/// Build channel options from discovered channels + bundled + registry catalog.
///
/// Returns a deduplicated, sorted list of channel names available for selection.
pub(super) fn build_channel_options(
    discovered: &[(String, ChannelCapabilitiesFile)],
) -> Vec<String> {
    let mut names: Vec<String> = discovered.iter().map(|(name, _)| name.clone()).collect();

    // Add channels embedded in the binary (--features bundled-wasm)
    for embedded in crate::registry::bundled_wasm::bundled_channel_names() {
        if !names.iter().any(|n| n == embedded) {
            names.push(embedded.to_string());
        }
    }

    // Add bundled channels (pre-compiled in channels-src/)
    for bundled in available_channel_names().iter().copied() {
        if !names.iter().any(|name| name == bundled) {
            names.push(bundled.to_string());
        }
    }

    // Add registry channels
    if let Some(catalog) = load_registry_catalog() {
        for manifest in catalog.list(Some(crate::registry::manifest::ManifestKind::Channel), None) {
            if !names.iter().any(|n| n == &manifest.name) {
                names.push(manifest.name.clone());
            }
        }
    }

    names.sort();
    names
}

/// Try to load the registry catalog. Falls back to embedded manifests when
/// the `registry/` directory cannot be found (e.g. running from an installed binary).
pub(super) fn load_registry_catalog() -> Option<crate::registry::catalog::RegistryCatalog> {
    crate::registry::catalog::RegistryCatalog::load_or_embedded().ok()
}

/// Install selected channels from the registry that aren't already on disk
/// and weren't handled by the bundled installer.
pub(super) async fn install_selected_registry_channels(
    channels_dir: &std::path::Path,
    selected_channels: &[String],
    already_installed: &HashSet<String>,
    bundled_installed: &HashSet<String>,
) -> Vec<String> {
    let catalog = match load_registry_catalog() {
        Some(c) => c,
        None => return Vec::new(),
    };

    let repo_root = catalog
        .root()
        .parent()
        .unwrap_or(catalog.root())
        .to_path_buf();

    let bundled_fs: HashSet<&str> = available_channel_names().iter().copied().collect();
    let installer = crate::registry::installer::RegistryInstaller::new(
        repo_root.clone(),
        dirs::home_dir().unwrap_or_default().join(".thinclaw/tools"),
        channels_dir.to_path_buf(),
    );
    let mut installed = Vec::new();

    for name in selected_channels {
        // Skip if already installed or successfully handled by bundled installer
        if already_installed.contains(name)
            || bundled_installed.contains(name)
            || bundled_fs.contains(name.as_str())
        {
            continue;
        }

        // Check if already on disk (may have been installed between bundled and here)
        let wasm_on_disk = channels_dir.join(format!("{}.wasm", name)).exists()
            || channels_dir.join(format!("{}-channel.wasm", name)).exists();
        if wasm_on_disk {
            continue;
        }

        // Look up in registry
        let manifest = match catalog.get(&format!("channels/{}", name)) {
            Some(m) => m,
            None => continue,
        };

        match installer
            .install_with_source_fallback(manifest, false)
            .await
        {
            Ok(outcome) => {
                for warning in &outcome.warnings {
                    crate::setup::prompts::print_info(&format!("{}: {}", name, warning));
                }
                installed.push(name.clone());
            }
            Err(e) => {
                tracing::warn!(
                    channel = %name,
                    error = %e,
                    "Failed to install channel from registry"
                );
                crate::setup::prompts::print_error(&format!(
                    "Failed to install channel '{}': {}",
                    name, e
                ));
            }
        }
    }

    installed
}

/// Discover which tools are already installed in the tools directory.
///
/// Returns a set of tool names (the stem of .wasm files).
pub(super) async fn discover_installed_tools(tools_dir: &std::path::Path) -> HashSet<String> {
    let mut names = HashSet::new();

    if !tools_dir.is_dir() {
        return names;
    }

    let mut entries = match tokio::fs::read_dir(tools_dir).await {
        Ok(e) => e,
        Err(_) => return names,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("wasm")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.insert(stem.to_string());
        }
    }

    names
}

pub(super) async fn install_selected_bundled_channels(
    channels_dir: &std::path::Path,
    selected_channels: &[String],
    already_installed: &HashSet<String>,
) -> Result<Option<Vec<String>>, SetupError> {
    let mut installed = Vec::new();
    let bundled_on_disk: HashSet<&str> = available_channel_names().iter().copied().collect();

    for name in selected_channels {
        if already_installed.contains(name) {
            continue;
        }

        // Priority 1: Extract from binary-embedded WASM (--features bundled-wasm)
        if crate::registry::bundled_wasm::is_bundled(name) {
            match crate::registry::bundled_wasm::extract_bundled(name, channels_dir).await {
                Ok(()) => {
                    installed.push(name.clone());
                    continue;
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %name,
                        error = %e,
                        "Bundled WASM extraction failed, trying filesystem artifacts"
                    );
                    // Fall through to filesystem path
                }
            }
        }

        // Priority 2: Copy from pre-built filesystem artifacts (channels-src/)
        if bundled_on_disk.contains(name.as_str()) {
            match install_bundled_channel(name, channels_dir, false).await {
                Ok(()) => {
                    installed.push(name.clone());
                    continue;
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %name,
                        error = %e,
                        "Filesystem bundled install failed, will try registry"
                    );
                    // Fall through to registry installer
                }
            }
        }
        // Channels not found via either bundled path will be tried by
        // install_selected_registry_channels next.
    }

    installed.sort();
    if installed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(installed))
    }
}
