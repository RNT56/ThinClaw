//! Engine configuration generation, writing, loading, and migration
//!
//! Contains: generate_config(), write_config(), deep_migrate(),
//! load_config(), and env_vars() methods for ThinClawConfig.

use tracing::{info, warn};

use super::types::*;

const MAX_ENGINE_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SESSIONS_INDEX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SESSION_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SESSION_FILES: usize = 10_000;
const MAX_SESSION_DIRECTORY_ENTRIES: usize = 50_000;
const MAX_SESSION_INDEX_ENTRIES: usize = 100_000;

fn modified_millis(path: &std::path::Path) -> u128 {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn session_path_field(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "path"
            | "cwd"
            | "workspace"
            | "workspacedir"
            | "workingdirectory"
            | "sessionfile"
            | "filepath"
            | "root"
            | "homedir"
    )
}

fn migrate_session_path_values(value: &mut serde_json::Value, is_path: bool) -> bool {
    match value {
        serde_json::Value::String(text) if is_path => {
            let updated = text
                .replace("Clawdbot", "ThinClaw")
                .replace("moltbot", "thinclaw");
            if updated == *text {
                false
            } else {
                *text = updated;
                true
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().fold(false, |changed, item| {
            migrate_session_path_values(item, is_path) || changed
        }),
        serde_json::Value::Object(object) => {
            object.iter_mut().fold(false, |changed, (key, value)| {
                migrate_session_path_values(value, session_path_field(key)) || changed
            })
        }
        _ => false,
    }
}

fn migrate_session_jsonl_paths(content: &str) -> std::io::Result<Option<String>> {
    let mut output = String::with_capacity(content.len());
    let mut changed = false;
    for segment in content.split_inclusive('\n') {
        let (line_with_optional_cr, newline) = segment
            .strip_suffix('\n')
            .map_or((segment, ""), |line| (line, "\n"));
        let (line, carriage_return) = line_with_optional_cr
            .strip_suffix('\r')
            .map_or((line_with_optional_cr, ""), |line| (line, "\r"));
        let replacement = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|mut value| {
                migrate_session_path_values(&mut value, false)
                    .then(|| serde_json::to_string(&value))
            })
            .transpose()
            .map_err(std::io::Error::other)?;
        if let Some(replacement) = replacement {
            changed = true;
            output.push_str(&replacement);
        } else {
            output.push_str(line);
        }
        output.push_str(carriage_return);
        output.push_str(newline);
        if output.len() > MAX_SESSION_FILE_BYTES as usize {
            return Err(std::io::Error::other(
                "migrated session file exceeds its size limit",
            ));
        }
    }
    Ok(changed.then_some(output))
}

impl ThinClawConfig {
    /// Generate the default ThinClawEngine configuration
    pub fn generate_config(
        &self,
        slack: Option<SlackConfig>,
        telegram: Option<TelegramConfig>,
        local_llm: Option<(u16, String, u32, String)>,
    ) -> ThinClawEngineConfig {
        // Determine primary model and provider content

        let mut agents_list = vec![];

        // Helper: check if a provider is usable (granted + valid non-empty API key)
        let is_provider_granted = |provider: &str| -> bool {
            let has_key = |key: &Option<String>| -> bool {
                key.as_ref().map(|k| !k.trim().is_empty()).unwrap_or(false)
            };
            match provider {
                "anthropic" => self.anthropic_granted && has_key(&self.anthropic_api_key),
                "openai" => self.openai_granted && has_key(&self.openai_api_key),
                "openrouter" => self.openrouter_granted && has_key(&self.openrouter_api_key),
                "gemini" => self.gemini_granted && has_key(&self.gemini_api_key),
                "groq" => self.groq_granted && has_key(&self.groq_api_key),
                "xai" => self.xai_granted && has_key(&self.xai_api_key),
                "mistral" => self.mistral_granted && has_key(&self.mistral_api_key),
                "venice" => self.venice_granted && has_key(&self.venice_api_key),
                "together" => self.together_granted && has_key(&self.together_api_key),
                "moonshot" => self.moonshot_granted && has_key(&self.moonshot_api_key),
                "minimax" => self.minimax_granted && has_key(&self.minimax_api_key),
                "nvidia" => self.nvidia_granted && has_key(&self.nvidia_api_key),
                "qianfan" => self.qianfan_granted && has_key(&self.qianfan_api_key),
                "xiaomi" => self.xiaomi_granted && has_key(&self.xiaomi_api_key),
                "cohere" => self.cohere_granted && has_key(&self.cohere_api_key),
                "voyage" => self.voyage_granted && has_key(&self.voyage_api_key),
                "deepgram" => self.deepgram_granted && has_key(&self.deepgram_api_key),
                "elevenlabs" => self.elevenlabs_granted && has_key(&self.elevenlabs_api_key),
                "stability" => self.stability_granted && has_key(&self.stability_api_key),
                "fal" => self.fal_granted && has_key(&self.fal_api_key),
                "amazon-bedrock" => {
                    self.bedrock_granted
                        && has_key(&self.bedrock_access_key_id)
                        && has_key(&self.bedrock_secret_access_key)
                }
                _ => false,
            }
        };

        // Helper: get the first enabled model for a provider, or the hardcoded default
        // if no enablement data exists yet (first-run compat)
        let first_enabled_model_for = |provider: &str| -> Option<String> {
            self.enabled_cloud_models
                .get(provider)
                .and_then(|models| models.first().cloned())
        };

        // Helper: check if a specific model is in the user's allowlist for a provider
        let is_model_allowed = |provider: &str, model_id: &str| -> bool {
            self.enabled_cloud_models
                .get(provider)
                .map(|models| models.iter().any(|m| m == model_id))
                .unwrap_or(false)
        };

        // Helper: has at least one enabled model for a provider
        let has_enabled_models = |provider: &str| -> bool {
            self.enabled_cloud_models
                .get(provider)
                .map(|models| !models.is_empty())
                .unwrap_or(false)
        };

        let agent_model;

        if self.local_inference_enabled {
            // Local inference explicitly enabled → always prefer local
            agent_model = "local/model".to_string();
        } else {
            // 1. Try the explicitly selected cloud brain (star) if it's granted + has enabled models
            let primary_resolved = if let Some(ref brain) = self.selected_cloud_brain {
                if is_provider_granted(brain) && has_enabled_models(brain) {
                    // Use the selected model if set AND it's in the allowlist
                    let model_part = if let Some(ref sel) = self.selected_cloud_model {
                        if is_model_allowed(brain, sel) {
                            sel.clone()
                        } else {
                            // Selected model is NOT in allowlist — use first enabled model
                            info!(
                                "Selected model {} is not in allowlist for {}, using first enabled",
                                sel, brain
                            );
                            first_enabled_model_for(brain).unwrap_or_else(|| "model".to_string())
                        }
                    } else {
                        // No model explicitly selected — use first enabled model
                        first_enabled_model_for(brain).unwrap_or_else(|| "model".to_string())
                    };
                    Some(format!("{}/{}", brain, model_part))
                } else {
                    None // Selected brain not granted or has no enabled models → fall through
                }
            } else {
                None // No brain selected → fall through
            };

            if let Some(model) = primary_resolved {
                agent_model = model;
            } else {
                // 2. Fallback: try other enabled + granted cloud providers WITH enabled models
                let fallback = self.enabled_cloud_providers.iter().find(|p| {
                    let is_default = self
                        .selected_cloud_brain
                        .as_deref()
                        .map(|b| b == p.as_str())
                        .unwrap_or(false);
                    !is_default && is_provider_granted(p) && has_enabled_models(p)
                });

                if let Some(provider) = fallback {
                    let model_id =
                        first_enabled_model_for(provider).unwrap_or_else(|| "model".to_string());
                    agent_model = format!("{}/{}", provider, model_id);
                } else {
                    // 3. No enabled+granted cloud provider with models → local model
                    agent_model = "local/model".to_string();
                }
            }
        }

        // Build fallback models list from other granted providers with enabled models.
        // The engine tries fallbacks in order when the primary model fails.
        // See: https://docs.thinclaw.ai/concepts/models#how-model-selection-works
        let mut fallback_models: Vec<String> = Vec::new();
        for provider in &self.enabled_cloud_providers {
            if !is_provider_granted(provider) || !has_enabled_models(provider) {
                continue;
            }
            let engine_provider = match provider.as_str() {
                "gemini" => "google",
                _ => provider.as_str(),
            };
            if let Some(model_id) = first_enabled_model_for(provider) {
                let candidate = format!("{}/{}", engine_provider, model_id);
                // Skip: already the primary model
                if candidate != agent_model {
                    fallback_models.push(candidate);
                }
            }
        }
        // Always include local as final fallback (if not already primary)
        if agent_model != "local/model" {
            fallback_models.push("local/model".to_string());
        }

        // =====================================================================
        // IMPLICIT PROVIDER ARCHITECTURE
        //
        // Built-in providers (anthropic, openai, google, groq, openrouter, xai,
        // mistral, etc.) are handled natively by the engine's pi-ai catalog.
        // They need NO explicit `models.providers` entries — only API keys in
        // `auth-profiles.json`. The engine auto-discovers models, base URLs,
        // context windows, maxTokens, and pricing.
        //
        // The model allowlist is enforced via `agents.defaults.models`:
        //   - If non-empty, ONLY listed models can be used by the agent
        //   - If empty, ALL discovered models are allowed (unsafe)
        //
        // Only the LOCAL provider needs an explicit `models.providers` entry
        // because it has a custom baseUrl + port.
        // =====================================================================

        // Build agents.defaults.models allowlist from user's enabled models.
        // This replaces the old filter_models() + explicit provider approach.
        let mut models_allowlist = std::collections::BTreeMap::new();
        for (provider, model_list) in &self.enabled_cloud_models {
            for model_id in model_list {
                // The engine expects "provider/model-id" format.
                // For gemini, the engine uses "google" as the provider name.
                let engine_provider = match provider.as_str() {
                    "gemini" => "google",
                    _ => provider.as_str(),
                };
                let key = format!("{}/{}", engine_provider, model_id);
                models_allowlist.insert(key, serde_json::json!({}));
            }
        }
        // Always allow the local model
        models_allowlist.insert("local/model".to_string(), serde_json::json!({}));

        // Full catalog of known models per provider (superset).
        // NOTE: The hardcoded model catalog was removed (2026-03-06).
        //
        // The frontend's CloudBrainConfigModal now uses dynamic API discovery
        // (via ModelProviderRegistry / useCloudModels hook) to fetch available
        // models from each provider. This means new models (e.g. GPT-5.3-Codex)
        // appear automatically without code changes.
        //
        // The enabled_cloud_models map (set by the user in the UI) still
        // controls which models the agent is ALLOWED to use.

        // Only the local provider needs explicit models.providers config
        let (local_port, local_token, context_size, _model_family) =
            local_llm.unwrap_or((53755, "".into(), 16384, "chatml".into()));
        let mut providers = serde_json::Map::new();

        // Local Provider (llama.cpp) — needs explicit config for custom baseUrl/port
        let local_host = if self.expose_inference {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        };

        // Build local provider config - include apiKey if we have a token from the sidecar
        let mut local_provider = serde_json::json!({
            "baseUrl": format!("http://{}:{}", local_host, local_port),
            "api": "openai-completions",
            "models": [
                {
                    "id": "model",
                    "name": "Local Model",
                    "contextWindow": context_size,
                    "maxTokens": (context_size / 4).clamp(4096, 8192)
                }
            ]
        });

        // Embed the API key so the engine can authenticate against llama-server
        if !local_token.is_empty() {
            if let Some(provider) = local_provider.as_object_mut() {
                provider.insert("apiKey".into(), serde_json::Value::String(local_token));
            }
        }

        // NOTE: Layer 2 stop token injection was removed because the ThinClaw engine's
        // strict config schema (since 2026.1.20) rejects unrecognized keys like "stop",
        // causing the engine to exit with code 1. Stop tokens are still enforced by:
        //   - Layer 1: llama-server's --stop CLI args (set during sidecar spawn)
        //   - API request level: stop tokens injected per-request by the sidecar

        providers.insert("local".into(), local_provider);

        // Add Amazon Bedrock models.providers entry if credentials are present.
        // Unlike implicit providers (OpenAI, Anthropic, Groq, etc.) which are
        // auto-discovered by the pi-ai catalog, Bedrock requires an explicit
        // provider entry with api: "bedrock-converse-stream" and auth: "aws-sdk".
        // See: https://docs.thinclaw.ai/providers/bedrock
        let mut bedrock_discovery: Option<serde_json::Value> = None;
        if self.bedrock_granted {
            if let (Some(ref _ak), Some(ref _sk)) =
                (&self.bedrock_access_key_id, &self.bedrock_secret_access_key)
            {
                let region = self.bedrock_region.as_deref().unwrap_or("us-east-1");
                let base_url = format!("https://bedrock-runtime.{}.amazonaws.com", region);

                // Build explicit model list from user's enabled models for amazon-bedrock
                let bedrock_models: Vec<serde_json::Value> = self
                    .enabled_cloud_models
                    .get("amazon-bedrock")
                    .map(|ids| {
                        ids.iter()
                            .map(|id| {
                                serde_json::json!({
                                    "id": id,
                                    "name": id,
                                    "contextWindow": 200000,
                                    "maxTokens": 8192
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                providers.insert(
                    "amazon-bedrock".into(),
                    serde_json::json!({
                        "baseUrl": base_url,
                        "api": "bedrock-converse-stream",
                        "auth": "aws-sdk",
                        "models": bedrock_models
                    }),
                );

                // Enable automatic model discovery via bedrock:ListFoundationModels
                bedrock_discovery = Some(serde_json::json!({
                    "enabled": true,
                    "region": region,
                    "providerFilter": ["anthropic", "amazon", "meta"],
                    "refreshInterval": 3600,
                    "defaultContextWindow": 200000,
                    "defaultMaxTokens": 8192
                }));
            }
        }

        let models = Some(ModelsConfig {
            providers,
            bedrock_discovery,
        });

        // Define Main Agent explicitly
        agents_list.push(serde_json::json!({
             "id": "main",
             "name": "ThinClaw",
             "model": agent_model,
        }));

        ThinClawEngineConfig {
            gateway: GatewayConfig {
                mode: "local".into(),
                bind: "loopback".into(),
                port: self.port,
                auth: AuthConfig {
                    mode: "token".into(),
                    token: self.auth_token.clone(),
                },
            },
            discovery: DiscoveryConfig {
                mdns: MdnsConfig { mode: "off".into() },
            },
            agents: AgentsConfig {
                defaults: AgentDefaults {
                    workspace: self.workspace_dir().to_string_lossy().to_string(),
                    model: Some(serde_json::json!({
                        "primary": agent_model,
                        "fallbacks": fallback_models
                    })),
                    models: models_allowlist,
                },
                list: agents_list,
            },
            models,
            channels: ChannelsConfig {
                slack: slack.unwrap_or_default(),
                telegram: telegram.unwrap_or_default(),
            },
            meta: MetaConfig {
                last_touched_version: THINCLAW_VERSION.into(),
                last_touched_at: chrono::Utc::now().to_rfc3339(),
            },
        }
    }

    /// Write config to disk
    pub fn write_config(
        &self,
        config: &ThinClawEngineConfig,
        _local_llm: Option<(u16, String, u32, String)>,
    ) -> std::io::Result<()> {
        self.ensure_dirs()?;
        let json = serde_json::to_string_pretty(config).map_err(std::io::Error::other)?;
        if json.len() > MAX_ENGINE_CONFIG_BYTES as usize {
            return Err(std::io::Error::other(
                "generated engine config exceeds its size limit",
            ));
        }
        thinclaw_platform::write_private_file_atomic(&self.config_path(), json.as_bytes(), true)?;

        // NOTE: auth-profiles.json, agent.json, and models.json were consumed
        // by the Node.js ThinClaw engine gateway (replaced by ThinClaw in-process).
        // ThinClaw gets keys via SecretsStore and config via thinclaw.toml/env vars.
        // These files are no longer written.

        // Ensure workspace directory exists
        let workspace_dir = self.workspace_dir();
        std::fs::create_dir_all(&workspace_dir)?;

        Ok(())
    }

    /// Deep migration for sessions and other data that might contain absolute paths
    pub fn deep_migrate(&self) -> std::io::Result<()> {
        let sessions_dir = self.base_dir.join("agents").join("main").join("sessions");
        match std::fs::symlink_metadata(&sessions_dir) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(std::io::Error::other(
                    "sessions path is not a real directory",
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
        let canonical_sessions_dir = sessions_dir.canonicalize()?;

        // Skip if migration has already run successfully
        let marker = sessions_dir.join(".migration_v1_complete");
        match std::fs::symlink_metadata(&marker) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                return Ok(())
            }
            Ok(_) => {
                return Err(std::io::Error::other(
                    "migration marker is not a regular file",
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        let sessions_index_path = sessions_dir.join("sessions.json");
        let mut sessions_index: serde_json::Value =
            match std::fs::symlink_metadata(&sessions_index_path) {
                Ok(_) => {
                    let bytes = thinclaw_platform::read_regular_file_bounded_single_link(
                        &sessions_index_path,
                        MAX_SESSIONS_INDEX_BYTES,
                    )?;
                    serde_json::from_slice(&bytes).map_err(|error| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("sessions index is invalid JSON: {error}"),
                        )
                    })?
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
                Err(error) => return Err(error),
            };
        let index = sessions_index.as_object().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "sessions index root must be a JSON object",
            )
        })?;
        if index.len() > MAX_SESSION_INDEX_ENTRIES {
            return Err(std::io::Error::other(
                "sessions index exceeds the migration entry limit",
            ));
        }

        let mut changed = false;

        // 1. Prune dead entries from index and normalize existing ones
        if let Some(obj) = sessions_index.as_object_mut() {
            let mut keys_to_remove = Vec::new();
            for (key, meta) in obj.iter_mut() {
                let mut path_valid = false;
                if let Some(file_path) = meta.get_mut("sessionFile") {
                    if let Some(s) = file_path.as_str() {
                        let normalized_s = s
                            .replace("Clawdbot", "ThinClaw")
                            .replace("moltbot", "thinclaw");
                        if normalized_s != s {
                            *file_path = serde_json::Value::String(normalized_s.clone());
                            changed = true;
                        }
                        let supplied = std::path::Path::new(&normalized_s);
                        let candidate = if supplied.is_absolute() {
                            supplied.to_path_buf()
                        } else {
                            canonical_sessions_dir.join(supplied)
                        };
                        if let Ok(canonical) = candidate.canonicalize() {
                            if canonical.starts_with(&canonical_sessions_dir) {
                                if let Ok(metadata) = std::fs::symlink_metadata(&candidate) {
                                    path_valid = metadata.is_file()
                                        && !metadata.file_type().is_symlink()
                                        && metadata.len() <= MAX_SESSION_FILE_BYTES;
                                }
                            }
                        }
                    }
                }
                if !path_valid {
                    warn!(
                        "[thinclaw] Pruning dead session entry: {} (file missing)",
                        key
                    );
                    keys_to_remove.push(key.clone());
                    changed = true;
                }
            }
            for k in keys_to_remove {
                obj.remove(&k);
            }
        }

        // 2. Scan for and re-index orphaned .jsonl files, updating their internal paths
        {
            let entries = std::fs::read_dir(&sessions_dir)?;
            let mut found_files = Vec::new();
            for (entry_index, entry) in entries.enumerate() {
                if entry_index >= MAX_SESSION_DIRECTORY_ENTRIES {
                    return Err(std::io::Error::other(
                        "sessions directory exceeds the migration scan limit",
                    ));
                }
                let entry = entry?;
                let path = entry.path();
                let metadata = match std::fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(_) => continue,
                };
                if metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.len() <= MAX_SESSION_FILE_BYTES
                    && path.extension().and_then(|s| s.to_str()) == Some("jsonl")
                {
                    found_files.push(path);
                }
            }
            if found_files.len() > MAX_SESSION_FILES {
                return Err(std::io::Error::other(
                    "sessions directory exceeds the migration file limit",
                ));
            }

            // Sort by modification time to find most recent
            found_files.sort_by(|a, b| {
                let ma = a
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let mb = b
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                mb.cmp(&ma) // Descending
            });

            for path in &found_files {
                let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                let Some(session_id) = file_name.strip_suffix(".jsonl") else {
                    continue;
                };
                if session_id.is_empty()
                    || session_id.len() > 256
                    || session_id.chars().any(char::is_control)
                {
                    warn!("[thinclaw] Skipping session file with an invalid identifier");
                    continue;
                }

                // Update only structured path fields. A global string replace
                // would corrupt ordinary user/assistant content that happens to
                // mention a legacy product name.
                if let Ok(bytes) = thinclaw_platform::read_regular_file_bounded_single_link(
                    path,
                    MAX_SESSION_FILE_BYTES,
                ) {
                    if let Ok(content) = std::str::from_utf8(&bytes) {
                        if let Some(updated_content) = migrate_session_jsonl_paths(content)? {
                            thinclaw_platform::write_private_file_atomic(
                                path,
                                updated_content.as_bytes(),
                                true,
                            )?;
                        }
                    }
                }

                // Ensure it's in the index
                let mut found_in_index = false;
                if let Some(obj) = sessions_index.as_object() {
                    for (_, meta) in obj {
                        if meta.get("sessionId").and_then(|v| v.as_str()) == Some(session_id) {
                            found_in_index = true;
                            break;
                        }
                    }
                }

                if !found_in_index {
                    let mut key = if session_id == "4e9284c4-ffbf-4eeb-9164-3c6c148c5176"
                        || session_id.starts_with("agent-main")
                    {
                        "agent:main".to_string()
                    } else {
                        format!(
                            "agent:main:{}",
                            session_id.chars().take(8).collect::<String>()
                        )
                    };

                    if let Some(obj) = sessions_index.as_object_mut() {
                        if obj.get(&key).is_some_and(|metadata| {
                            metadata.get("sessionId").and_then(|value| value.as_str())
                                != Some(session_id)
                        }) {
                            key = format!("agent:main:{session_id}");
                        }
                        if !obj.contains_key(&key) {
                            info!(
                                "[thinclaw] Recovering orphaned session: {} -> {}",
                                key, session_id
                            );
                            obj.insert(
                                key,
                                serde_json::json!({
                                    "sessionId": session_id,
                                    "updatedAt": modified_millis(path),
                                    "sessionFile": path.to_string_lossy().to_string(),
                                    "chatType": "direct",
                                }),
                            );
                            changed = true;
                        }
                    }
                }
            }

            // Special Case: ensure agent:main is NOT empty if we have at least one file
            if let Some(obj) = sessions_index.as_object_mut() {
                if !obj.contains_key("agent:main") && !found_files.is_empty() {
                    let best_path = &found_files[0];
                    let best_id = best_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .replace(".jsonl", "");
                    info!(
                        "[thinclaw] Assigning most recent session to agent:main: {}",
                        best_id
                    );
                    obj.insert(
                        "agent:main".into(),
                        serde_json::json!({
                            "sessionId": best_id,
                            "updatedAt": modified_millis(best_path),
                            "sessionFile": best_path.to_string_lossy().to_string(),
                            "chatType": "direct",
                        }),
                    );
                    changed = true;
                }
            }
        }

        if changed {
            let json = serde_json::to_string_pretty(&sessions_index)?;
            if json.len() > MAX_SESSIONS_INDEX_BYTES as usize {
                return Err(std::io::Error::other(
                    "migrated sessions index exceeds its size limit",
                ));
            }
            thinclaw_platform::write_private_file_atomic(
                &sessions_index_path,
                json.as_bytes(),
                true,
            )?;
            info!("[thinclaw] deep_migrate completed and index updated.");
        }

        // Write completion marker so we don't re-run on next start
        match thinclaw_platform::write_private_file_atomic(
            &marker,
            format!("completed: {}", chrono::Utc::now().to_rfc3339()).as_bytes(),
            false,
        ) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = std::fs::symlink_metadata(&marker)?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(std::io::Error::other(
                        "concurrent migration published an invalid marker",
                    ));
                }
            }
            Err(error) => return Err(error),
        }

        Ok(())
    }

    /// Load config from disk
    pub fn load_config(&self) -> std::io::Result<ThinClawEngineConfig> {
        let bytes = thinclaw_platform::read_regular_file_bounded_single_link(
            &self.config_path(),
            MAX_ENGINE_CONFIG_BYTES,
        )?;
        serde_json::from_slice(&bytes).map_err(std::io::Error::other)
    }

    /// Get environment variables to pass to ThinClawEngine process
    pub fn env_vars(&self) -> Vec<(String, String)> {
        // THINCLAW_HOME should point to the base directory.
        // ThinClawEngine appends "/state" internally.
        // If we pass ".../ThinClaw/state", it looks in ".../ThinClaw/state/state".
        // We must pass ".../ThinClaw".
        // NOTE: base_dir_str removed — THINCLAW_HOME is no longer set (see comment below)
        let state_dir_str = self.state_dir().to_string_lossy().to_string();
        let config_path_str = self.config_path().to_string_lossy().to_string();

        let mut vars = vec![
            ("THINCLAW_STATE_DIR".into(), state_dir_str.clone()),
            ("CLAWDBOT_STATE_DIR".into(), state_dir_str.clone()),
            // NOTE: Do NOT set THINCLAW_HOME here. The engine uses THINCLAW_HOME as a
            // user home-directory override and appends "/.thinclaw" to derive the state dir.
            // Setting it to the AppData base dir would cause the engine to look in
            // AppData/ThinClaw/.thinclaw — a wrong path. Instead we use THINCLAW_STATE_DIR
            // and THINCLAW_CONFIG_PATH to point directly at the correct locations.
            ("THINCLAW_ENGINE_CONFIG".into(), config_path_str.clone()),
            ("THINCLAW_CONFIG_PATH".into(), config_path_str.clone()),
            ("CLAWDBOT_CONFIG_PATH".into(), config_path_str.clone()),
            ("MOLTBOT_CONFIG".into(), config_path_str.clone()),
            ("THINCLAW_GATEWAY_PORT".into(), self.port.to_string()),
            ("CLAWDBOT_GATEWAY_PORT".into(), self.port.to_string()),
            ("MOLTBOT_GATEWAY_PORT".into(), self.port.to_string()),
            ("THINCLAW_GATEWAY_TOKEN".into(), self.auth_token.clone()),
            ("CLAWDBOT_GATEWAY_TOKEN".into(), self.auth_token.clone()),
            (
                "THINCLAW_CUSTOM_LLM_ENABLED".into(),
                self.custom_llm_enabled.to_string(),
            ),
            (
                "THINCLAW_ENABLED_CLOUD_PROVIDERS".into(),
                self.enabled_cloud_providers.join(","),
            ),
            ("MOLTBOT_GATEWAY_TOKEN".into(), self.auth_token.clone()),
            (
                "THINCLAW_LOCAL_INFERENCE_ENABLED".into(),
                self.local_inference_enabled.to_string(),
            ),
            (
                "MOLTBOT_LOCAL_INFERENCE_ENABLED".into(),
                self.local_inference_enabled.to_string(),
            ),
            (
                "THINCLAW_EXPOSE_INFERENCE".into(),
                self.expose_inference.to_string(),
            ),
            (
                "MOLTBOT_EXPOSE_INFERENCE".into(),
                self.expose_inference.to_string(),
            ),
        ];

        // Only inject custom LLM credentials when the feature is explicitly enabled.
        // The key must not leak to the ThinClaw process when disabled.
        if self.custom_llm_enabled {
            if let Some(ref url) = self.custom_llm_url {
                vars.push(("THINCLAW_CUSTOM_LLM_URL".into(), url.clone()));
            }
            if let Some(ref key) = self.custom_llm_key {
                if !key.trim().is_empty() {
                    vars.push(("THINCLAW_CUSTOM_LLM_KEY".into(), key.clone()));
                }
            }
            if let Some(ref model) = self.custom_llm_model {
                vars.push(("THINCLAW_CUSTOM_LLM_MODEL".into(), model.clone()));
            }
        }

        // Inject Amazon Bedrock AWS credentials as env vars
        if self.bedrock_granted {
            if let Some(ref ak) = self.bedrock_access_key_id {
                if !ak.trim().is_empty() {
                    vars.push(("AWS_ACCESS_KEY_ID".into(), ak.clone()));
                }
            }
            if let Some(ref sk) = self.bedrock_secret_access_key {
                if !sk.trim().is_empty() {
                    vars.push(("AWS_SECRET_ACCESS_KEY".into(), sk.clone()));
                }
            }
            if let Some(ref r) = self.bedrock_region {
                if !r.trim().is_empty() {
                    vars.push(("AWS_REGION".into(), r.clone()));
                    vars.push(("AWS_DEFAULT_REGION".into(), r.clone()));
                }
            }
        }

        vars
    }
}

#[cfg(test)]
mod tests {
    use super::migrate_session_jsonl_paths;

    #[test]
    fn session_migration_only_rewrites_structured_path_fields() {
        let input = concat!(
            "{\"cwd\":\"/Users/me/Clawdbot/.moltbot\",\"content\":\"Clawdbot and moltbot are ordinary user text\"}\n",
            "not-json Clawdbot moltbot\n",
            "{\"nested\":{\"filePath\":\"C:\\\\Clawdbot\\\\moltbot\"}}"
        );
        let migrated = migrate_session_jsonl_paths(input).unwrap().unwrap();
        assert!(migrated.contains("/Users/me/ThinClaw/.thinclaw"));
        assert!(migrated.contains("Clawdbot and moltbot are ordinary user text"));
        assert!(migrated.contains("not-json Clawdbot moltbot"));
        assert!(migrated.contains("ThinClaw\\\\thinclaw"));
    }

    #[test]
    fn session_migration_returns_none_when_no_path_changed() {
        assert!(migrate_session_jsonl_paths("{\"content\":\"moltbot\"}\n")
            .unwrap()
            .is_none());
    }
}
