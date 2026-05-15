//! Engine configuration generation, writing, loading, and migration
//!
//! Contains: generate_config(), write_config(), deep_migrate(),
//! load_config(), and env_vars() methods for ThinClawConfig.

use tracing::{info, warn};

use super::types::*;

impl ThinClawConfig {
    /// Generate the default ThinClawEngine configuration
    pub fn generate_config(
        &self,
        slack: Option<SlackConfig>,
        telegram: Option<TelegramConfig>,
        local_llm: Option<(u16, String, u32, String)>,
    ) -> ThinClawEngineConfig {
        // Determine primary model and provider content

        let models;
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
        // (via CloudModelRegistry / useCloudModels hook) to fetch available
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
                    "maxTokens": std::cmp::max(4096, std::cmp::min(8192, context_size / 4))
                }
            ]
        });

        // Embed the API key so the engine can authenticate against llama-server
        if !local_token.is_empty() {
            local_provider
                .as_object_mut()
                .unwrap()
                .insert("apiKey".into(), serde_json::Value::String(local_token));
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

        models = Some(ModelsConfig {
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
        let json = serde_json::to_string_pretty(config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(self.config_path(), json)?;

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
        if !sessions_dir.exists() {
            return Ok(());
        }

        // Skip if migration has already run successfully
        let marker = sessions_dir.join(".migration_v1_complete");
        if marker.exists() {
            return Ok(());
        }

        let sessions_index_path = sessions_dir.join("sessions.json");
        let mut sessions_index: serde_json::Value = if sessions_index_path.exists() {
            let content = std::fs::read_to_string(&sessions_index_path)?;
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

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
                        if std::path::Path::new(&normalized_s).exists() {
                            path_valid = true;
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
        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            let mut found_files = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                    found_files.push(path);
                }
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
                let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                let session_id = file_name.replace(".jsonl", "");

                // Update internal paths in the .jsonl file defensively
                if let Ok(content) = std::fs::read_to_string(path) {
                    if content.contains("Clawdbot") || content.contains("moltbot") {
                        let updated_content = content
                            .replace("Clawdbot", "ThinClaw")
                            .replace("moltbot", "thinclaw");
                        if updated_content != content {
                            let _ = std::fs::write(path, updated_content);
                        }
                    }
                }

                // Ensure it's in the index
                let mut found_in_index = false;
                if let Some(obj) = sessions_index.as_object() {
                    for (_, meta) in obj {
                        if meta.get("sessionId").and_then(|v| v.as_str()) == Some(&session_id) {
                            found_in_index = true;
                            break;
                        }
                    }
                }

                if !found_in_index {
                    let key = if session_id == "4e9284c4-ffbf-4eeb-9164-3c6c148c5176"
                        || session_id.starts_with("agent-main")
                    {
                        "agent:main".to_string()
                    } else {
                        format!(
                            "agent:main:{}",
                            &session_id[..std::cmp::min(8, session_id.len())]
                        )
                    };

                    if let Some(obj) = sessions_index.as_object_mut() {
                        if !obj.contains_key(&key) {
                            info!(
                                "[thinclaw] Recovering orphaned session: {} -> {}",
                                key, session_id
                            );
                            obj.insert(key, serde_json::json!({
                                "sessionId": session_id,
                                "updatedAt": path.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::now()).duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
                                "sessionFile": path.to_string_lossy().to_string(),
                                "chatType": "direct",
                            }));
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
                    obj.insert("agent:main".into(), serde_json::json!({
                        "sessionId": best_id,
                        "updatedAt": best_path.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::now()).duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
                        "sessionFile": best_path.to_string_lossy().to_string(),
                        "chatType": "direct",
                    }));
                    changed = true;
                }
            }
        }

        if changed {
            let json = serde_json::to_string_pretty(&sessions_index)?;
            std::fs::write(&sessions_index_path, json)?;
            info!("[thinclaw] deep_migrate completed and index updated.");
        }

        // Write completion marker so we don't re-run on next start
        let _ = std::fs::write(
            &marker,
            format!("completed: {}", chrono::Utc::now().to_rfc3339()),
        );

        Ok(())
    }

    /// Load config from disk
    pub fn load_config(&self) -> std::io::Result<ThinClawEngineConfig> {
        let json = std::fs::read_to_string(self.config_path())?;
        serde_json::from_str(&json).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
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
