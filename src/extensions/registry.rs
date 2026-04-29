//! Curated in-memory catalog of known extensions with fuzzy search.
//!
//! The registry holds well-known channels, tools, and MCP servers that can be
//! installed via conversational commands. Online discoveries are cached here too.

use tokio::sync::RwLock;

use crate::extensions::manifest::{
    ContextProviderContribution, MemoryProviderContribution, PluginArtifactKind, PluginManifest,
    verify_plugin_manifest_signature,
};
use crate::extensions::{
    AuthHint, ExtensionKind, ExtensionSource, RegistryEntry, ResultSource, SearchResult,
    validate_plugin_manifest,
};
use crate::settings::ExtensionsSettings;

/// Curated extension registry with fuzzy search.
pub struct ExtensionRegistry {
    /// Built-in curated entries.
    entries: Vec<RegistryEntry>,
    /// Entries contributed by broad plugin manifests at runtime.
    manifest_entries: RwLock<Vec<RegistryEntry>>,
    /// Memory providers contributed by broad plugin manifests at runtime.
    memory_providers: RwLock<Vec<RegisteredMemoryProviderContribution>>,
    /// Context providers contributed by broad plugin manifests at runtime.
    context_providers: RwLock<Vec<RegisteredContextProviderContribution>>,
    /// Cached entries from online discovery (session-lived).
    discovery_cache: RwLock<Vec<RegistryEntry>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegisteredMemoryProviderContribution {
    pub manifest_id: String,
    pub contribution: MemoryProviderContribution,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegisteredContextProviderContribution {
    pub manifest_id: String,
    pub contribution: ContextProviderContribution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifestRegistration {
    pub tools_registered: Vec<String>,
    pub channels_registered: Vec<String>,
    pub memory_providers_registered: Vec<String>,
    pub context_providers_registered: Vec<String>,
    pub native_plugins_available: Vec<String>,
    pub native_plugins_skipped: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PluginManifestRegistrationError {
    #[error("plugin manifest failed validation: {0}")]
    Validation(String),
    #[error("plugin manifest signature check failed: {0}")]
    Signature(String),
}

impl ExtensionRegistry {
    /// Create a new registry populated with known extensions.
    pub fn new() -> Self {
        Self {
            entries: builtin_entries(),
            manifest_entries: RwLock::new(Vec::new()),
            memory_providers: RwLock::new(Vec::new()),
            context_providers: RwLock::new(Vec::new()),
            discovery_cache: RwLock::new(Vec::new()),
        }
    }

    /// Create a new registry merging builtin entries with catalog-provided entries.
    ///
    /// Deduplicates by `(name, kind)` pair -- a builtin MCP "slack" and a registry
    /// WASM "slack" can coexist since they're different kinds.
    pub fn new_with_catalog(catalog_entries: Vec<RegistryEntry>) -> Self {
        let mut entries = builtin_entries();
        for entry in catalog_entries {
            if !entries
                .iter()
                .any(|e| e.name == entry.name && e.kind == entry.kind)
            {
                entries.push(entry);
            }
        }
        Self {
            entries,
            manifest_entries: RwLock::new(Vec::new()),
            memory_providers: RwLock::new(Vec::new()),
            context_providers: RwLock::new(Vec::new()),
            discovery_cache: RwLock::new(Vec::new()),
        }
    }

    /// Register the safe contribution surface from a broad plugin manifest.
    ///
    /// Native plugin contributions are intentionally not loaded here. When native
    /// plugins are disabled, those contributions are ignored for registration
    /// while the non-native contributions still go through normal manifest policy
    /// validation. When native plugins are enabled, the full manifest must pass
    /// native policy validation before non-native contributions are registered.
    pub async fn register_plugin_manifest_contributions(
        &self,
        manifest: &PluginManifest,
        settings: &ExtensionsSettings,
    ) -> Result<PluginManifestRegistration, PluginManifestRegistrationError> {
        let native_plugins: Vec<String> = manifest
            .contributions
            .native_plugins
            .iter()
            .map(|native| native.id.clone())
            .collect();
        let native_plugins_available = if settings.allow_native_plugins {
            native_plugins.clone()
        } else {
            Vec::new()
        };
        let native_plugins_skipped = if settings.allow_native_plugins {
            Vec::new()
        } else {
            native_plugins.clone()
        };

        let validation_manifest;
        let manifest_for_validation = if settings.allow_native_plugins {
            manifest
        } else {
            validation_manifest = manifest_without_native_contributions(manifest);
            &validation_manifest
        };

        let validation = validate_plugin_manifest(manifest_for_validation, settings);
        if !validation.valid {
            return Err(PluginManifestRegistrationError::Validation(
                validation.errors.join("; "),
            ));
        }
        if settings.require_plugin_signatures {
            verify_plugin_manifest_signature(manifest, settings)
                .map_err(PluginManifestRegistrationError::Signature)?;
        }

        let mut entries = Vec::new();
        let mut tools_registered = Vec::new();
        for tool in &manifest.contributions.tools {
            entries.push(RegistryEntry {
                name: tool.id.clone(),
                display_name: tool.name.clone(),
                kind: ExtensionKind::WasmTool,
                description: manifest
                    .description
                    .clone()
                    .unwrap_or_else(|| format!("Tool contributed by {}", manifest.name)),
                keywords: vec![
                    manifest.id.clone(),
                    "plugin".to_string(),
                    "tool".to_string(),
                ],
                source: wasm_contribution_source(manifest, tool.wasm_artifact.as_deref()),
                fallback_source: None,
                auth_hint: if tool.wasm_artifact.is_some() {
                    AuthHint::CapabilitiesAuth
                } else {
                    AuthHint::None
                },
            });
            tools_registered.push(tool.id.clone());
        }

        let mut channels_registered = Vec::new();
        for channel in &manifest.contributions.channels {
            entries.push(RegistryEntry {
                name: channel.id.clone(),
                display_name: channel.name.clone(),
                kind: ExtensionKind::WasmChannel,
                description: manifest
                    .description
                    .clone()
                    .unwrap_or_else(|| format!("Channel contributed by {}", manifest.name)),
                keywords: vec![
                    manifest.id.clone(),
                    "plugin".to_string(),
                    "channel".to_string(),
                ],
                source: wasm_contribution_source(manifest, channel.wasm_artifact.as_deref()),
                fallback_source: None,
                auth_hint: if channel.wasm_artifact.is_some() {
                    AuthHint::CapabilitiesAuth
                } else {
                    AuthHint::None
                },
            });
            channels_registered.push(channel.id.clone());
        }

        if !entries.is_empty() {
            let mut manifest_entries = self.manifest_entries.write().await;
            for entry in entries {
                upsert_entry(&mut manifest_entries, entry);
            }
        }

        let mut memory_providers_registered = Vec::new();
        if !manifest.contributions.memory_providers.is_empty() {
            let mut providers = self.memory_providers.write().await;
            for provider in &manifest.contributions.memory_providers {
                upsert_memory_provider(
                    &mut providers,
                    RegisteredMemoryProviderContribution {
                        manifest_id: manifest.id.clone(),
                        contribution: provider.clone(),
                    },
                );
                memory_providers_registered.push(provider.id.clone());
            }
        }

        let mut context_providers_registered = Vec::new();
        if !manifest.contributions.context_providers.is_empty() {
            let mut providers = self.context_providers.write().await;
            for provider in &manifest.contributions.context_providers {
                upsert_context_provider(
                    &mut providers,
                    RegisteredContextProviderContribution {
                        manifest_id: manifest.id.clone(),
                        contribution: provider.clone(),
                    },
                );
                context_providers_registered.push(provider.id.clone());
            }
        }

        Ok(PluginManifestRegistration {
            tools_registered,
            channels_registered,
            memory_providers_registered,
            context_providers_registered,
            native_plugins_available,
            native_plugins_skipped,
        })
    }

    pub async fn memory_provider_contributions(&self) -> Vec<RegisteredMemoryProviderContribution> {
        self.memory_providers.read().await.clone()
    }

    pub async fn context_provider_contributions(
        &self,
    ) -> Vec<RegisteredContextProviderContribution> {
        self.context_providers.read().await.clone()
    }

    /// Search the registry by query string. Returns results sorted by relevance.
    ///
    /// Splits the query into lowercase tokens and scores each entry by matches
    /// in name, keywords, and description.
    pub async fn search(&self, query: &str) -> Vec<SearchResult> {
        let tokens: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        if tokens.is_empty() {
            // Return all entries when query is empty
            return self
                .entries
                .iter()
                .map(|e| SearchResult {
                    entry: e.clone(),
                    source: ResultSource::Registry,
                    validated: true,
                })
                .collect();
        }

        let mut scored: Vec<(SearchResult, u32)> = Vec::new();

        // Score built-in entries
        for entry in &self.entries {
            let score = score_entry(entry, &tokens);
            if score > 0 {
                scored.push((
                    SearchResult {
                        entry: entry.clone(),
                        source: ResultSource::Registry,
                        validated: true,
                    },
                    score,
                ));
            }
        }

        // Score manifest-contributed entries
        let manifest_entries = self.manifest_entries.read().await;
        for entry in manifest_entries.iter() {
            let score = score_entry(entry, &tokens);
            if score > 0 {
                scored.push((
                    SearchResult {
                        entry: entry.clone(),
                        source: ResultSource::Registry,
                        validated: true,
                    },
                    score,
                ));
            }
        }

        // Score cached discoveries
        let cache = self.discovery_cache.read().await;
        for entry in cache.iter() {
            let score = score_entry(entry, &tokens);
            if score > 0 {
                scored.push((
                    SearchResult {
                        entry: entry.clone(),
                        source: ResultSource::Discovered,
                        validated: true,
                    },
                    score,
                ));
            }
        }

        scored.sort_by_key(|b| std::cmp::Reverse(b.1));
        scored.into_iter().map(|(r, _)| r).collect()
    }

    /// Look up an entry by exact name.
    ///
    /// NOTE: Prefer [`get_with_kind`] when a kind hint is available, to avoid
    /// returning the wrong entry when two entries share a name but differ in kind.
    pub async fn get(&self, name: &str) -> Option<RegistryEntry> {
        if let Some(entry) = self.entries.iter().find(|e| e.name == name) {
            return Some(entry.clone());
        }
        let manifest_entries = self.manifest_entries.read().await;
        if let Some(entry) = manifest_entries.iter().find(|e| e.name == name) {
            return Some(entry.clone());
        }
        let cache = self.discovery_cache.read().await;
        cache.iter().find(|e| e.name == name).cloned()
    }

    /// Look up an entry by exact name, filtering by kind when provided.
    ///
    /// When `kind` is `Some(...)`, only returns an entry matching both name and
    /// kind — never falls back to a different kind. When `kind` is `None`,
    /// returns the first name match (same as [`get`]).
    pub async fn get_with_kind(
        &self,
        name: &str,
        kind: Option<ExtensionKind>,
    ) -> Option<RegistryEntry> {
        if let Some(kind) = kind {
            if let Some(entry) = self
                .entries
                .iter()
                .find(|e| e.name == name && e.kind == kind)
            {
                return Some(entry.clone());
            }
            let manifest_entries = self.manifest_entries.read().await;
            if let Some(entry) = manifest_entries
                .iter()
                .find(|e| e.name == name && e.kind == kind)
            {
                return Some(entry.clone());
            }
            let cache = self.discovery_cache.read().await;
            if let Some(entry) = cache.iter().find(|e| e.name == name && e.kind == kind) {
                return Some(entry.clone());
            }
            // Kind was specified but no entry matches — don't fall back to a
            // different kind, as that would silently misroute the install.
            return None;
        }
        self.get(name).await
    }

    /// Return all registry entries (builtins + cached discoveries).
    pub async fn all_entries(&self) -> Vec<RegistryEntry> {
        let mut entries = self.entries.clone();
        let manifest_entries = self.manifest_entries.read().await;
        for entry in manifest_entries.iter() {
            if !entries
                .iter()
                .any(|e| e.name == entry.name && e.kind == entry.kind)
            {
                entries.push(entry.clone());
            }
        }
        let cache = self.discovery_cache.read().await;
        for entry in cache.iter() {
            if !entries
                .iter()
                .any(|e| e.name == entry.name && e.kind == entry.kind)
            {
                entries.push(entry.clone());
            }
        }
        entries
    }

    /// Add discovered entries to the cache.
    pub async fn cache_discovered(&self, entries: Vec<RegistryEntry>) {
        let mut cache = self.discovery_cache.write().await;
        for entry in entries {
            // Deduplicate by (name, kind) — same pair as new_with_catalog()
            if !cache
                .iter()
                .any(|e| e.name == entry.name && e.kind == entry.kind)
            {
                cache.push(entry);
            }
        }
    }
}

fn manifest_without_native_contributions(manifest: &PluginManifest) -> PluginManifest {
    let mut manifest = manifest.clone();
    manifest.contributions.native_plugins.clear();
    manifest
        .artifacts
        .retain(|artifact| artifact.kind != PluginArtifactKind::NativeDylib);
    manifest
}

fn wasm_contribution_source(
    manifest: &PluginManifest,
    artifact_id: Option<&str>,
) -> ExtensionSource {
    if let Some(artifact_id) = artifact_id
        && let Some(artifact) = manifest.artifacts.iter().find(|artifact| {
            artifact.id == artifact_id && artifact.kind == PluginArtifactKind::Wasm
        })
    {
        return ExtensionSource::WasmDownload {
            wasm_url: artifact.path.clone(),
            capabilities_url: None,
        };
    }
    ExtensionSource::Discovered {
        url: format!("plugin-manifest://{}/contribution", manifest.id),
    }
}

fn upsert_entry(entries: &mut Vec<RegistryEntry>, entry: RegistryEntry) {
    if let Some(existing) = entries
        .iter_mut()
        .find(|existing| existing.name == entry.name && existing.kind == entry.kind)
    {
        *existing = entry;
    } else {
        entries.push(entry);
    }
}

fn upsert_memory_provider(
    providers: &mut Vec<RegisteredMemoryProviderContribution>,
    provider: RegisteredMemoryProviderContribution,
) {
    if let Some(existing) = providers.iter_mut().find(|existing| {
        existing.manifest_id == provider.manifest_id
            && existing.contribution.id == provider.contribution.id
    }) {
        *existing = provider;
    } else {
        providers.push(provider);
    }
}

fn upsert_context_provider(
    providers: &mut Vec<RegisteredContextProviderContribution>,
    provider: RegisteredContextProviderContribution,
) {
    if let Some(existing) = providers.iter_mut().find(|existing| {
        existing.manifest_id == provider.manifest_id
            && existing.contribution.id == provider.contribution.id
    }) {
        *existing = provider;
    } else {
        providers.push(provider);
    }
}

impl Default for ExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Score an entry against search tokens. Higher = better match.
fn score_entry(entry: &RegistryEntry, tokens: &[String]) -> u32 {
    let mut score = 0u32;
    let name_lower = entry.name.to_lowercase();
    let display_lower = entry.display_name.to_lowercase();
    let desc_lower = entry.description.to_lowercase();
    let keywords_lower: Vec<String> = entry.keywords.iter().map(|k| k.to_lowercase()).collect();

    for token in tokens {
        // Exact name match is the strongest signal
        if name_lower == *token {
            score += 100;
        } else if name_lower.contains(token.as_str()) {
            score += 50;
        }

        // Display name match
        if display_lower.contains(token.as_str()) {
            score += 30;
        }

        // Keyword match
        for kw in &keywords_lower {
            if kw == token {
                score += 40;
            } else if kw.contains(token.as_str()) {
                score += 20;
            }
        }

        // Description match (weakest signal)
        if desc_lower.contains(token.as_str()) {
            score += 10;
        }
    }

    score
}

/// Well-known extensions that ship with thinclaw.
fn builtin_entries() -> Vec<RegistryEntry> {
    vec![
        // -- MCP Servers --
        RegistryEntry {
            name: "notion".to_string(),
            display_name: "Notion".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Connect to Notion for reading and writing pages, databases, and comments"
                .to_string(),
            keywords: vec![
                "notes".into(),
                "wiki".into(),
                "docs".into(),
                "pages".into(),
                "database".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.notion.com/mcp".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "linear".to_string(),
            display_name: "Linear".to_string(),
            kind: ExtensionKind::McpServer,
            description:
                "Connect to Linear for issue tracking, project management, and team workflows"
                    .to_string(),
            keywords: vec![
                "issues".into(),
                "tickets".into(),
                "project".into(),
                "tracking".into(),
                "bugs".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.linear.app/sse".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "github".to_string(),
            display_name: "GitHub".to_string(),
            kind: ExtensionKind::McpServer,
            description:
                "Connect to GitHub for repository management, issues, PRs, and code search"
                    .to_string(),
            keywords: vec![
                "git".into(),
                "repos".into(),
                "code".into(),
                "pull-request".into(),
                "issues".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://api.githubcopilot.com/mcp/".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "slack-mcp".to_string(),
            display_name: "Slack MCP".to_string(),
            kind: ExtensionKind::McpServer,
            description:
                "Connect to Slack via MCP for messaging, channel management, and team communication"
                    .to_string(),
            keywords: vec![
                "messaging".into(),
                "chat".into(),
                "channels".into(),
                "team".into(),
                "communication".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.slack.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "sentry".to_string(),
            display_name: "Sentry".to_string(),
            kind: ExtensionKind::McpServer,
            description:
                "Connect to Sentry for error tracking, performance monitoring, and debugging"
                    .to_string(),
            keywords: vec![
                "errors".into(),
                "monitoring".into(),
                "debugging".into(),
                "crashes".into(),
                "performance".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.sentry.dev/mcp".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "stripe".to_string(),
            display_name: "Stripe".to_string(),
            kind: ExtensionKind::McpServer,
            description:
                "Connect to Stripe for payment processing, subscriptions, and financial data"
                    .to_string(),
            keywords: vec![
                "payments".into(),
                "billing".into(),
                "subscriptions".into(),
                "invoices".into(),
                "finance".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.stripe.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "cloudflare".to_string(),
            display_name: "Cloudflare".to_string(),
            kind: ExtensionKind::McpServer,
            description:
                "Connect to Cloudflare for DNS, Workers, KV, and infrastructure management"
                    .to_string(),
            keywords: vec![
                "cdn".into(),
                "dns".into(),
                "workers".into(),
                "hosting".into(),
                "infrastructure".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.cloudflare.com/mcp".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "asana".to_string(),
            display_name: "Asana".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Connect to Asana for task management, projects, and team coordination"
                .to_string(),
            keywords: vec![
                "tasks".into(),
                "projects".into(),
                "management".into(),
                "team".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.asana.com/v2/mcp".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        },
        RegistryEntry {
            name: "intercom".to_string(),
            display_name: "Intercom".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Connect to Intercom for customer messaging, support, and engagement"
                .to_string(),
            keywords: vec![
                "support".into(),
                "customers".into(),
                "messaging".into(),
                "chat".into(),
                "helpdesk".into(),
            ],
            source: ExtensionSource::McpUrl {
                url: "https://mcp.intercom.com/mcp".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        },
        // WASM channels (telegram, slack, discord, whatsapp) come from the embedded
        // registry catalog (registry/channels/*.json) with WasmDownload URLs pointing
        // to GitHub release artifacts. See new_with_catalog() for merging.
    ]
}

#[cfg(test)]
mod tests {
    use crate::extensions::manifest::{
        ChannelContribution, ContextProviderContribution, MemoryProviderContribution,
        NATIVE_PLUGIN_ABI_VERSION, NativePluginAbi, NativePluginContribution,
        PLUGIN_MANIFEST_SCHEMA_VERSION, PluginArtifact, PluginArtifactKind, PluginContributions,
        PluginManifest, PluginSignature, ToolContribution,
    };
    use crate::extensions::registry::{ExtensionRegistry, score_entry};
    use crate::extensions::{AuthHint, ExtensionKind, ExtensionSource, RegistryEntry};
    use crate::settings::ExtensionsSettings;

    fn plugin_manifest() -> PluginManifest {
        PluginManifest {
            schema_version: PLUGIN_MANIFEST_SCHEMA_VERSION,
            id: "plugin.example".to_string(),
            name: "Example Plugin".to_string(),
            version: "1.0.0".to_string(),
            publisher: None,
            description: Some("Example manifest contributions".to_string()),
            permissions: vec!["tools".to_string(), "channels".to_string()],
            contributions: PluginContributions {
                tools: vec![ToolContribution {
                    id: "plugin.example.echo".to_string(),
                    name: "Example Echo".to_string(),
                    wasm_artifact: Some("echo-wasm".to_string()),
                }],
                channels: vec![ChannelContribution {
                    id: "plugin.example.channel".to_string(),
                    name: "Example Channel".to_string(),
                    wasm_artifact: Some("channel-wasm".to_string()),
                }],
                memory_providers: vec![MemoryProviderContribution {
                    id: "plugin.example.memory".to_string(),
                    provider_type: "custom_http".to_string(),
                    config_schema: serde_json::json!({ "type": "object" }),
                }],
                context_providers: vec![ContextProviderContribution {
                    id: "plugin.example.context".to_string(),
                    provider_type: "workspace".to_string(),
                    config_schema: serde_json::json!({ "type": "object" }),
                }],
                native_plugins: Vec::new(),
            },
            artifacts: vec![
                PluginArtifact {
                    id: "echo-wasm".to_string(),
                    kind: PluginArtifactKind::Wasm,
                    path: "https://example.com/echo.wasm".to_string(),
                    sha256: None,
                },
                PluginArtifact {
                    id: "channel-wasm".to_string(),
                    kind: PluginArtifactKind::Wasm,
                    path: "https://example.com/channel.wasm".to_string(),
                    sha256: None,
                },
            ],
            signature: Some(PluginSignature {
                key_id: "test-key".to_string(),
                algorithm: "ed25519".to_string(),
                signature: "aa".repeat(64),
            }),
        }
    }

    fn unsigned_registration_settings() -> ExtensionsSettings {
        ExtensionsSettings {
            require_plugin_signatures: false,
            ..ExtensionsSettings::default()
        }
    }

    #[test]
    fn test_score_exact_name_match() {
        let entry = RegistryEntry {
            name: "notion".to_string(),
            display_name: "Notion".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Workspace tool".to_string(),
            keywords: vec!["notes".into()],
            source: ExtensionSource::McpUrl {
                url: "https://example.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        };

        let score = score_entry(&entry, &["notion".to_string()]);
        assert!(
            score >= 100,
            "Exact name match should score >= 100, got {}",
            score
        );
    }

    #[test]
    fn test_score_partial_name_match() {
        let entry = RegistryEntry {
            name: "google-calendar".to_string(),
            display_name: "Google Calendar".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Calendar management".to_string(),
            keywords: vec!["events".into()],
            source: ExtensionSource::McpUrl {
                url: "https://example.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        };

        let score = score_entry(&entry, &["calendar".to_string()]);
        assert!(
            score > 0,
            "Partial name match should score > 0, got {}",
            score
        );
    }

    #[test]
    fn test_score_keyword_match() {
        let entry = RegistryEntry {
            name: "notion".to_string(),
            display_name: "Notion".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Workspace tool".to_string(),
            keywords: vec!["wiki".into(), "notes".into()],
            source: ExtensionSource::McpUrl {
                url: "https://example.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        };

        let score = score_entry(&entry, &["wiki".to_string()]);
        assert!(
            score >= 40,
            "Exact keyword match should score >= 40, got {}",
            score
        );
    }

    #[test]
    fn test_score_no_match() {
        let entry = RegistryEntry {
            name: "notion".to_string(),
            display_name: "Notion".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Workspace tool".to_string(),
            keywords: vec!["notes".into()],
            source: ExtensionSource::McpUrl {
                url: "https://example.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        };

        let score = score_entry(&entry, &["xyzfoobar".to_string()]);
        assert_eq!(score, 0, "No match should score 0");
    }

    #[tokio::test]
    async fn test_search_returns_sorted() {
        let registry = ExtensionRegistry::new();
        let results = registry.search("notion").await;

        assert!(!results.is_empty(), "Should find notion in registry");
        assert_eq!(results[0].entry.name, "notion");
    }

    #[tokio::test]
    async fn test_search_empty_query_returns_all() {
        let registry = ExtensionRegistry::new();
        let results = registry.search("").await;

        assert!(results.len() > 5, "Empty query should return all entries");
    }

    #[tokio::test]
    async fn test_search_by_keyword() {
        let registry = ExtensionRegistry::new();
        let results = registry.search("issues tickets").await;

        assert!(
            !results.is_empty(),
            "Should find entries matching 'issues tickets'"
        );
        // Linear should be near the top since it has both keywords
        let linear_pos = results.iter().position(|r| r.entry.name == "linear");
        assert!(linear_pos.is_some(), "Linear should appear in results");
    }

    #[tokio::test]
    async fn test_get_exact_name() {
        let registry = ExtensionRegistry::new();

        let entry = registry.get("notion").await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().display_name, "Notion");

        let missing = registry.get("nonexistent").await;
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_cache_discovered() {
        let registry = ExtensionRegistry::new();

        let discovered = RegistryEntry {
            name: "custom-mcp".to_string(),
            display_name: "Custom MCP".to_string(),
            kind: ExtensionKind::McpServer,
            description: "A custom MCP server".to_string(),
            keywords: vec![],
            source: ExtensionSource::McpUrl {
                url: "https://custom.example.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        };

        registry.cache_discovered(vec![discovered]).await;

        let entry = registry.get("custom-mcp").await;
        assert!(entry.is_some());

        let results = registry.search("custom").await;
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_cache_deduplication() {
        let registry = ExtensionRegistry::new();

        let entry = RegistryEntry {
            name: "dup".to_string(),
            display_name: "Dup".to_string(),
            kind: ExtensionKind::McpServer,
            description: "Test".to_string(),
            keywords: vec![],
            source: ExtensionSource::McpUrl {
                url: "https://example.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::None,
        };

        registry.cache_discovered(vec![entry.clone()]).await;
        registry.cache_discovered(vec![entry]).await;

        let results = registry.search("dup").await;
        assert_eq!(results.len(), 1, "Should not duplicate cached entries");
    }

    #[tokio::test]
    async fn plugin_manifest_contributions_register_with_registry() {
        let registry = ExtensionRegistry::new();
        let manifest = plugin_manifest();

        let result = registry
            .register_plugin_manifest_contributions(&manifest, &unsigned_registration_settings())
            .await
            .expect("manifest contributions register");

        assert_eq!(result.tools_registered, vec!["plugin.example.echo"]);
        assert_eq!(result.channels_registered, vec!["plugin.example.channel"]);
        assert_eq!(
            result.memory_providers_registered,
            vec!["plugin.example.memory"]
        );
        assert_eq!(
            result.context_providers_registered,
            vec!["plugin.example.context"]
        );

        let tool = registry
            .get_with_kind("plugin.example.echo", Some(ExtensionKind::WasmTool))
            .await
            .expect("tool contribution registered");
        assert_eq!(tool.display_name, "Example Echo");
        assert!(matches!(
            tool.source,
            ExtensionSource::WasmDownload { ref wasm_url, .. }
                if wasm_url == "https://example.com/echo.wasm"
        ));

        let channel = registry
            .get_with_kind("plugin.example.channel", Some(ExtensionKind::WasmChannel))
            .await
            .expect("channel contribution registered");
        assert_eq!(channel.display_name, "Example Channel");

        let memory = registry.memory_provider_contributions().await;
        assert_eq!(memory.len(), 1);
        assert_eq!(memory[0].contribution.provider_type, "custom_http");
        let context = registry.context_provider_contributions().await;
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].contribution.provider_type, "workspace");
    }

    #[tokio::test]
    async fn plugin_manifest_native_contributions_are_skipped_when_disabled() {
        let registry = ExtensionRegistry::new();
        let mut manifest = plugin_manifest();
        manifest
            .contributions
            .native_plugins
            .push(NativePluginContribution {
                id: "plugin.example.native".to_string(),
                artifact: "native-lib".to_string(),
                abi: NativePluginAbi::CAbiJsonV1,
                abi_version: NATIVE_PLUGIN_ABI_VERSION,
                max_request_bytes: 1024,
                max_response_bytes: 1024,
            });
        manifest.artifacts.push(PluginArtifact {
            id: "native-lib".to_string(),
            kind: PluginArtifactKind::NativeDylib,
            path: "libexample.dylib".to_string(),
            sha256: None,
        });

        let result = registry
            .register_plugin_manifest_contributions(&manifest, &unsigned_registration_settings())
            .await
            .expect("safe contributions register while native stays disabled");

        assert_eq!(result.tools_registered, vec!["plugin.example.echo"]);
        assert_eq!(result.native_plugins_skipped, vec!["plugin.example.native"]);
        assert!(result.native_plugins_available.is_empty());
        assert!(
            registry
                .get_with_kind("plugin.example.echo", Some(ExtensionKind::WasmTool))
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn test_new_with_catalog() {
        let catalog_entries = vec![
            RegistryEntry {
                name: "telegram".to_string(),
                display_name: "Telegram".to_string(),
                kind: ExtensionKind::WasmChannel,
                description: "Telegram Bot API channel".to_string(),
                keywords: vec!["messaging".into(), "bot".into()],
                source: ExtensionSource::WasmBuildable {
                    repo_url: "channels-src/telegram".to_string(),
                    build_dir: Some("channels-src/telegram".to_string()),
                    crate_name: Some("telegram-channel".to_string()),
                },
                fallback_source: None,
                auth_hint: AuthHint::CapabilitiesAuth,
            },
            // This shares a name with the builtin slack-mcp but has a different kind, so both should appear
            RegistryEntry {
                name: "slack-mcp".to_string(),
                display_name: "Slack MCP WASM".to_string(),
                kind: ExtensionKind::WasmTool,
                description: "Slack WASM tool".to_string(),
                keywords: vec!["messaging".into()],
                source: ExtensionSource::WasmBuildable {
                    repo_url: "tools-src/slack".to_string(),
                    build_dir: Some("tools-src/slack".to_string()),
                    crate_name: Some("slack-tool".to_string()),
                },
                fallback_source: None,
                auth_hint: AuthHint::CapabilitiesAuth,
            },
        ];

        let registry = ExtensionRegistry::new_with_catalog(catalog_entries);

        // Should find the new telegram entry
        let results = registry.search("telegram").await;
        assert!(!results.is_empty(), "Should find telegram from catalog");
        assert_eq!(results[0].entry.name, "telegram");

        // Should have both builtin MCP slack-mcp and catalog WASM slack-mcp
        let results = registry.search("slack").await;
        let slack_mcp = results
            .iter()
            .any(|r| r.entry.name == "slack-mcp" && r.entry.kind == ExtensionKind::McpServer);
        let slack_wasm = results
            .iter()
            .any(|r| r.entry.name == "slack-mcp" && r.entry.kind == ExtensionKind::WasmTool);
        assert!(slack_mcp, "Should have builtin MCP slack-mcp");
        assert!(slack_wasm, "Should have catalog WASM slack-mcp");
    }

    #[tokio::test]
    async fn test_new_with_catalog_dedup_same_kind() {
        // A catalog entry with same name AND kind as a builtin should be skipped
        let catalog_entries = vec![RegistryEntry {
            name: "slack-mcp".to_string(),
            display_name: "Slack MCP Override".to_string(),
            kind: ExtensionKind::McpServer, // same kind as builtin slack-mcp
            description: "Should be skipped".to_string(),
            keywords: vec![],
            source: ExtensionSource::McpUrl {
                url: "https://other.slack.com".to_string(),
            },
            fallback_source: None,
            auth_hint: AuthHint::Dcr,
        }];

        let registry = ExtensionRegistry::new_with_catalog(catalog_entries);

        let entry = registry.get("slack-mcp").await;
        assert!(entry.is_some());
        // Should still be the builtin, not the override
        assert_eq!(entry.unwrap().display_name, "Slack MCP");
    }

    #[tokio::test]
    async fn test_get_with_kind_resolves_collision() {
        // Two entries with the same name but different kinds (the telegram collision scenario)
        let catalog_entries = vec![
            RegistryEntry {
                name: "telegram".to_string(),
                display_name: "Telegram Tool".to_string(),
                kind: ExtensionKind::WasmTool,
                description: "Telegram MTProto tool".to_string(),
                keywords: vec!["messaging".into()],
                source: ExtensionSource::WasmBuildable {
                    repo_url: "tools-src/telegram".to_string(),
                    build_dir: Some("tools-src/telegram".to_string()),
                    crate_name: Some("telegram-tool".to_string()),
                },
                fallback_source: None,
                auth_hint: AuthHint::CapabilitiesAuth,
            },
            RegistryEntry {
                name: "telegram".to_string(),
                display_name: "Telegram Channel".to_string(),
                kind: ExtensionKind::WasmChannel,
                description: "Telegram Bot API channel".to_string(),
                keywords: vec!["messaging".into(), "bot".into()],
                source: ExtensionSource::WasmBuildable {
                    repo_url: "channels-src/telegram".to_string(),
                    build_dir: Some("channels-src/telegram".to_string()),
                    crate_name: Some("telegram-channel".to_string()),
                },
                fallback_source: None,
                auth_hint: AuthHint::CapabilitiesAuth,
            },
        ];

        let registry = ExtensionRegistry::new_with_catalog(catalog_entries);

        // Without kind hint, get() returns the first match (WasmTool)
        let entry = registry.get("telegram").await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().kind, ExtensionKind::WasmTool);

        // With kind hint for WasmChannel, get_with_kind() returns the channel entry
        let entry = registry
            .get_with_kind("telegram", Some(ExtensionKind::WasmChannel))
            .await;
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.kind, ExtensionKind::WasmChannel);
        assert_eq!(entry.display_name, "Telegram Channel");

        // With kind hint for WasmTool, get_with_kind() returns the tool entry
        let entry = registry
            .get_with_kind("telegram", Some(ExtensionKind::WasmTool))
            .await;
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.kind, ExtensionKind::WasmTool);
        assert_eq!(entry.display_name, "Telegram Tool");

        // Without kind hint (None), get_with_kind() falls back to first match
        let entry = registry.get_with_kind("telegram", None).await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().kind, ExtensionKind::WasmTool);

        // Kind mismatch: no McpServer named "telegram" exists — must return None,
        // not silently fall back to the WasmTool entry.
        let entry = registry
            .get_with_kind("telegram", Some(ExtensionKind::McpServer))
            .await;
        assert!(
            entry.is_none(),
            "Should return None when kind doesn't match, not fall back to wrong kind"
        );
    }

    #[tokio::test]
    async fn test_get_with_kind_discovery_cache() {
        let registry = ExtensionRegistry::new();

        // Add two entries with the same name but different kinds to the discovery cache
        let tool_entry = RegistryEntry {
            name: "cached-ext".to_string(),
            display_name: "Cached Tool".to_string(),
            kind: ExtensionKind::WasmTool,
            description: "A cached tool".to_string(),
            keywords: vec![],
            source: ExtensionSource::WasmBuildable {
                repo_url: "tools-src/cached".to_string(),
                build_dir: None,
                crate_name: None,
            },
            fallback_source: None,
            auth_hint: AuthHint::None,
        };
        let channel_entry = RegistryEntry {
            name: "cached-ext".to_string(),
            display_name: "Cached Channel".to_string(),
            kind: ExtensionKind::WasmChannel,
            description: "A cached channel".to_string(),
            keywords: vec![],
            source: ExtensionSource::WasmBuildable {
                repo_url: "channels-src/cached".to_string(),
                build_dir: None,
                crate_name: None,
            },
            fallback_source: None,
            auth_hint: AuthHint::None,
        };

        registry
            .cache_discovered(vec![tool_entry, channel_entry])
            .await;

        // Kind-aware lookup should find the channel in the cache
        let entry = registry
            .get_with_kind("cached-ext", Some(ExtensionKind::WasmChannel))
            .await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().display_name, "Cached Channel");

        // Kind-aware lookup should find the tool in the cache
        let entry = registry
            .get_with_kind("cached-ext", Some(ExtensionKind::WasmTool))
            .await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().display_name, "Cached Tool");
    }

    // Channel tests (telegram, slack, discord, whatsapp) require the embedded catalog
    // to be loaded via new_with_catalog(). See test_new_with_catalog for catalog coverage.
}
