use crate::sidecar::SidecarManager;
use serde::Serialize;
use specta::Type;
use sqlx::SqlitePool;
use tauri::{ipc::Channel, State};
use thinclaw_runtime_contracts::{
    ApiStyle, DirectAttachedDocument, DirectChatMessage, DirectChatPayload, DirectTokenUsage,
};
use tracing::info;

pub type AttachedDoc = DirectAttachedDocument;
pub type Message = DirectChatMessage;
pub type ChatPayload = DirectChatPayload;
pub type TokenUsage = DirectTokenUsage;

#[derive(Debug, Clone, Serialize, Type)]
pub struct StreamChunk {
    pub content: String,
    pub done: bool,
    pub usage: Option<TokenUsage>,
    #[specta(type = Option<Vec<Message>>)] // Explicit type for recursive struct
    pub context_update: Option<Vec<Message>>,
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub kind: crate::rig_lib::unified_provider::ProviderKind,
    pub base_url: String,
    pub model_name: String,
    pub port: u16,
    pub token: String,
    pub context_size: u32,
    pub model_family: Option<String>,
}

fn provider_kind_from_api_style(
    api_style: ApiStyle,
) -> crate::rig_lib::unified_provider::ProviderKind {
    use crate::rig_lib::unified_provider::ProviderKind;
    match api_style {
        ApiStyle::OpenAi => ProviderKind::OpenAI,
        ApiStyle::Anthropic => ProviderKind::Anthropic,
        ApiStyle::OpenAiCompatible => ProviderKind::OpenAI,
        ApiStyle::Ollama => ProviderKind::OpenAI,
    }
}

pub async fn resolve_provider(
    user_config: &crate::config::UserConfig,
    secret_store: &crate::secret_store::SecretStore,
    sidecar_manager: &State<'_, SidecarManager>,
    engine_manager: &State<'_, crate::engine::EngineManager>,
) -> Result<ProviderConfig, String> {
    // Determine provider: prefer new `chat_backend`, fall back to legacy `selected_chat_provider`
    let provider_id = user_config
        .chat_backend
        .as_deref()
        .or(user_config.selected_chat_provider.as_deref())
        .unwrap_or("local");

    // Check if it's a known cloud provider
    if provider_id != "local" {
        if let Some(endpoint) = thinclaw_config::provider_catalog::endpoint_for(provider_id) {
            info!(
                "[resolve_provider] Routing to {} ({})",
                endpoint.display_name, provider_id
            );

            let descriptor =
                thinclaw_runtime_contracts::descriptor_for_secret_name(&endpoint.secret_name)
                    .unwrap_or_else(|| thinclaw_runtime_contracts::SecretDescriptor {
                        canonical_name: endpoint.secret_name.clone(),
                        provider_slug: Some(provider_id.to_string()),
                        env_key_name: Some(endpoint.env_key_name.clone()),
                        legacy_aliases: vec![
                            provider_id.to_string(),
                            endpoint.env_key_name.clone(),
                        ],
                        allowed_consumers: vec![
                            thinclaw_runtime_contracts::SecretConsumer::DirectWorkbench,
                        ],
                    });
            let key = secret_store
                .get_descriptor_secret(&descriptor)
                .ok_or(format!(
                    "{} API key required. Please set it in Settings > Secrets.",
                    endpoint.display_name
                ))?;

            // Model name: prefer UserConfig.inference_models["chat"], then endpoint default
            let model_name = user_config
                .inference_models
                .as_ref()
                .and_then(|m| m.get("chat"))
                .cloned()
                .unwrap_or_else(|| endpoint.default_model.to_string());

            return Ok(ProviderConfig {
                kind: provider_kind_from_api_style(endpoint.api_style),
                base_url: endpoint.base_url.to_string(),
                model_name,
                port: 0,
                token: key,
                // Prefer user-configured context size (from model discovery),
                // fall back to provider-level default.
                context_size: user_config
                    .selected_model_context_size
                    .unwrap_or(endpoint.default_context_size),
                model_family: None,
            });
        }

        // Unknown cloud provider — warn and fall through to local
        info!(
            "[resolve_provider] Unknown provider '{}', falling back to local",
            provider_id
        );
    }

    // ── Local provider ──────────────────────────────────────────────────
    info!("[resolve_provider] Routing to Local Provider");

    let snapshot = crate::engine::local_runtime_snapshot(sidecar_manager, engine_manager).await;
    if let Some(endpoint) = snapshot.endpoint {
        let port: u16 = endpoint
            .base_url
            .trim_end_matches('/')
            .rsplit(':')
            .next()
            .and_then(|p| p.split('/').next())
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);
        info!(
            "[resolve_provider] Using local runtime snapshot: {}, model: {:?}, context: {:?}",
            endpoint.base_url, endpoint.model_id, endpoint.context_size
        );
        return Ok(ProviderConfig {
            kind: crate::rig_lib::unified_provider::ProviderKind::Local,
            base_url: endpoint.base_url,
            model_name: endpoint.model_id.unwrap_or_else(|| "default".to_string()),
            port,
            token: endpoint.api_key.unwrap_or_default(),
            context_size: endpoint.context_size.unwrap_or(4096),
            model_family: endpoint.model_family,
        });
    }

    Err(snapshot.unavailable_reason.unwrap_or_else(|| {
        "No local inference server is running. Select a model in the chat tab — the engine will start automatically.".to_string()
    }))
}

#[tauri::command]
#[specta::specta]
pub async fn direct_chat_stream(
    app: tauri::AppHandle,
    state: State<'_, SidecarManager>,
    config: State<'_, crate::config::ConfigManager>,
    secret_store: State<'_, crate::secret_store::SecretStore>,
    engine_manager: State<'_, crate::engine::EngineManager>,
    rig_cache: State<'_, crate::rig_cache::RigManagerCache>,
    pool: State<'_, SqlitePool>,
    payload: ChatPayload,
    on_event: Channel<StreamChunk>,
) -> Result<(), String> {
    info!("[direct_chat_stream] Starting direct_chat_stream command...");

    // Acquisition of Global Generation Lock (Queuing)
    let _guard = state.generation_lock.lock().await;
    info!("[direct_chat_stream] Generation lock acquired.");

    // Reset Cancellation Token for the CURRENT active job
    state
        .cancellation_token
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // Clone messages so we can modify them
    let mut processing_messages = payload.messages.clone();

    // General Knowledge Injection
    let user_config = config.get_config();

    // Provider Routing Logic — keys from SecretStore, model from UserConfig
    let provider_cfg =
        resolve_provider(&user_config, &secret_store, &state, &engine_manager).await?;
    let kind = provider_cfg.kind;
    let base_url = provider_cfg.base_url;
    let model_name = provider_cfg.model_name;
    let _port = provider_cfg.port;
    let token = provider_cfg.token;
    let context_size = provider_cfg.context_size;
    let model_family = provider_cfg.model_family;

    // Collect enabled knowledge bits
    let gk_content = user_config
        .knowledge_bits
        .iter()
        .filter(|bit| bit.enabled)
        .map(|bit| format!("- [{}] {}", bit.label, bit.content))
        .collect::<Vec<String>>()
        .join("\n");

    // Manual injection removed - passed to Agent Preamble instead

    // Verify the last message is from the user
    if processing_messages.is_empty() {
        return Err("No messages provided".into());
    }
    let last_idx = processing_messages.len() - 1;
    if processing_messages[last_idx].role != "user" {
        return Err("Last message must be from user".into());
    }

    use tauri::Emitter;

    #[derive(Serialize, Clone, Type)]
    struct WebSearchStatus {
        id: String,
        step: String, // "thinking", "searching", "scraping", "analyzing"
        message: String,
    }

    // Build multimodal content for user messages that contain images.
    // CRITICAL: Only embed actual base64 image data for the LAST user message.
    // For older messages in the history, replace with a text placeholder to avoid
    // context bloat — a single 1024×1024 JPEG is ~100KB base64 = ~25K tokens,
    // which would fill a 32K context window on the second turn.
    let last_user_idx = processing_messages.iter().rposition(|m| m.role == "user");

    for (idx, msg) in processing_messages.iter_mut().enumerate() {
        if msg.role != "user" {
            continue;
        }

        let mut image_ids = msg.images.clone().unwrap_or_default();
        if let Some(asset_refs) = msg.assets.clone() {
            for asset_ref in asset_refs {
                if let Ok(record) =
                    crate::direct_assets::DirectAssetStore::get(pool.inner(), &asset_ref).await
                {
                    if matches!(
                        record.kind,
                        thinclaw_runtime_contracts::AssetKind::Image
                            | thinclaw_runtime_contracts::AssetKind::GeneratedImage
                    ) && !image_ids.iter().any(|id| id == &record.reference.id)
                    {
                        image_ids.push(record.reference.id);
                    }
                }
            }
        }

        if !image_ids.is_empty() {
            let is_current_turn = Some(idx) == last_user_idx;

            if is_current_turn {
                // Current turn: embed full base64 image data
                info!(
                    "[chat] Building multimodal parts for {} image(s) (current turn)",
                    image_ids.len()
                );
                let mut parts = Vec::new();
                parts.push(serde_json::json!({
                    "type": "text",
                    "text": msg.content
                }));

                for id in &image_ids {
                    match crate::images::load_image_as_base64_with_mime(&app, id).await {
                        Ok((b64, mime)) => {
                            info!(
                                "[chat] Image {} loaded as {}, base64 length: {}",
                                id,
                                mime,
                                b64.len()
                            );
                            parts.push(serde_json::json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:{};base64,{}", mime, b64)
                                }
                            }));
                        }
                        Err(e) => eprintln!("Failed to load image {}: {}", id, e),
                    }
                }

                info!(
                    "[chat] Multimodal parts: {} total ({} image_url parts)",
                    parts.len(),
                    parts.len() - 1
                );
                if let Ok(json_str) = serde_json::to_string(&parts) {
                    msg.content = json_str;
                }
            } else {
                // Older turn: replace with text placeholder to save context
                let n = image_ids.len();
                let original_text = msg.content.clone();
                msg.content = format!(
                    "{}\n[User shared {} image(s) in this message]",
                    original_text, n
                );
                info!(
                    "[chat] Stripped {} image(s) from history message (turn {})",
                    n, idx
                );
            }
        }
    }

    // --- Image History Filtering (User Requested) ---
    // If the user is NOT explicitly talking about images/pictures in their latest prompt,
    // we strip previous image generation turns to keep the LLM focused on chat and save context.
    let last_prompt_lower = processing_messages
        .last()
        .map(|m| m.content.to_lowercase())
        .unwrap_or_default();
    let has_assistant_images = processing_messages
        .iter()
        .any(|m| m.role == "assistant" && m.images.as_ref().is_some_and(|i| !i.is_empty()));
    let is_referencing_image = last_prompt_lower.contains("image")
        || last_prompt_lower.contains("picture")
        || last_prompt_lower.contains("draw")
        || last_prompt_lower.contains("this one")
        || last_prompt_lower.contains("that one")
        || (has_assistant_images
            && (last_prompt_lower.contains("edit it")
                || last_prompt_lower.contains("change it")
                || last_prompt_lower.contains("modify it")
                || last_prompt_lower.contains("redo it")
                || last_prompt_lower.contains("make it")
                || last_prompt_lower.contains("the same")));

    if !is_referencing_image {
        let mut filtered = Vec::new();
        let mut i = 0;
        while i < processing_messages.len() {
            let msg = &processing_messages[i];

            // Check if this turn resulted in an image (Assistant has images)
            let is_image_turn = if i + 1 < processing_messages.len() {
                let next_msg = &processing_messages[i + 1];
                next_msg.role == "assistant"
                    && next_msg.images.as_ref().is_some_and(|img| !img.is_empty())
            } else {
                false
            };

            if is_image_turn {
                // Skip the user prompt AND the assistant image response
                i += 2;
                continue;
            }

            // Also skip if it IS an assistant image response (sanity check)
            if msg.role == "assistant" && msg.images.as_ref().is_some_and(|img| !img.is_empty()) {
                i += 1;
                continue;
            }

            filtered.push(msg.clone());
            i += 1;
        }
        processing_messages = filtered;
    } else {
        // If they ARE referencing the image, keep the turn but replace the
        // assistant's "Generated image for: [Super Long Prompt]" with something cleaner
        for msg in processing_messages.iter_mut() {
            if msg.role == "assistant" && msg.images.as_ref().is_some_and(|img| !img.is_empty()) {
                if msg.content.contains("Generated image for:") {
                    msg.content = "[Assistant generated an image based on the prompt]".to_string();
                }
            }
        }
    }

    // Use Orchestrator for All Chats (Text & Multimodal)
    use crate::rig_lib::RigManager;
    use futures::StreamExt;

    // Support Legacy Web Search Icon: Treat it as Auto Mode
    let has_context = payload.project_id.is_some()
        || processing_messages
            .iter()
            .any(|m| m.attached_docs.as_ref().is_some_and(|d| !d.is_empty()));

    // We use the Agent if Auto Mode is ON, OR if we have context (RAG/Files), OR if we have images (since Lrama/Rig handling is unified here)
    // Actually, Orchestrator is our default pipeline now.
    let effective_auto_mode = payload.auto_mode || payload.web_search_enabled || has_context;
    let enable_tools = effective_auto_mode; // Or always true? Tools are gated by permissions anyway.

    info!(
        "[direct_chat_stream] Getting RigManager for model: {}",
        &model_name
    );

    let gk_content_for_key = if gk_content.trim().is_empty() {
        String::new()
    } else {
        gk_content.clone()
    };

    let cache_key = crate::rig_cache::RigManagerKey::from_parts(
        &kind,
        &base_url,
        &model_name,
        &token,
        context_size as usize,
        enable_tools,
        &gk_content_for_key,
        model_family.as_deref(),
    );

    // Clone all values needed inside the closure before moving them.
    let app_clone = app.clone();
    let kind_c = kind;
    let base_url_c = base_url;
    let model_name_c = model_name.clone();
    let token_c = token.clone();
    let gk_opt = if gk_content_for_key.is_empty() {
        None
    } else {
        Some(gk_content_for_key)
    };
    let conv_id_c = payload.conversation_id.clone();
    let mf_c = model_family;

    let manager = rig_cache
        .get_or_build(cache_key, move || {
            RigManager::new(
                kind_c,
                base_url_c,
                model_name_c,
                Some(app_clone),
                Some(token_c),
                context_size as usize,
                None,
                enable_tools,
                gk_opt,
                conv_id_c,
                mf_c,
            )
        })
        .await;

    // Emit "Thinking" Status
    if let Some(id) = &payload.conversation_id {
        if payload.auto_mode {
            let _ = app.emit(
                "web_search_status",
                WebSearchStatus {
                    id: id.clone(),
                    step: "thinking".into(),
                    message: "Auto Mode Active...".into(),
                },
            );
        } else if payload.web_search_enabled {
            let _ = app.emit(
                "web_search_status",
                WebSearchStatus {
                    id: id.clone(),
                    step: "thinking".into(),
                    message: "Preparing web search...".into(),
                },
            );
        } else if has_context {
            let _ = app.emit(
                "web_search_status",
                WebSearchStatus {
                    id: id.clone(),
                    step: "thinking".into(),
                    message: "Using Project Context...".into(),
                },
            );
        }
    }

    // Use the Orchestrator — sandbox mode is activated when MCP config is present
    let mcp_config = crate::rig_lib::orchestrator::McpOrchestratorConfig {
        mcp_base_url: user_config.mcp_base_url.clone(),
        mcp_auth_token: user_config.mcp_auth_token.clone(),
        sandbox_enabled: user_config.mcp_sandbox_enabled && user_config.mcp_base_url.is_some(),
    };
    if mcp_config.sandbox_enabled {
        info!(
            "[direct_chat_stream] Sandbox mode ENABLED — MCP server: {}",
            mcp_config.mcp_base_url.as_deref().unwrap_or("(none)")
        );
    } else {
        info!("[direct_chat_stream] Sandbox mode (local-only, no remote MCP)");
    }
    let orchestrator = crate::rig_lib::orchestrator::Orchestrator::new_with_mcp(
        std::sync::Arc::new(manager),
        mcp_config,
    );

    let permissions = crate::rig_lib::orchestrator::ToolPermissions {
        allow_web_search: payload.auto_mode || payload.web_search_enabled,
        force_web_search: payload.web_search_enabled,
        allow_file_search: payload.auto_mode || has_context,
        allow_image_gen: payload.auto_mode,
    };

    // Pass permissions
    let persona_name = user_config.selected_persona.clone();

    // Check if it's a custom persona first, then fallback to built-in
    let persona_instructions = user_config
        .custom_personas
        .iter()
        .find(|p| p.id == persona_name)
        .map(|p| p.instructions.clone())
        .unwrap_or_else(|| crate::personas::get_persona_instructions(&persona_name).to_string());

    info!("[direct_chat_stream] Starting orchestrator turn...");
    match orchestrator
        .run_turn(
            processing_messages,
            permissions,
            payload.project_id.clone(),
            persona_instructions,
            payload.conversation_id.clone(),
        )
        .await
    {
        Ok(mut stream) => {
            info!("[direct_chat_stream] Orchestrator turn started.");
            // NOTE: Do NOT emit "done" status here — the stream hasn't produced
            // content yet. The frontend's onmessage handler already transitions
            // searchStatus to "done" when the first content token arrives, and
            // the DDGSearchTool emits "generating" at the right time.

            // Consume stream and emit chunks — with batching to reduce IPC overhead.
            // During fast local inference, llama.cpp can emit 30-100+ tokens/sec.
            // Sending each as a separate IPC message floods the webview event loop
            // and causes UI lag. Instead, we buffer text content and flush when:
            //   (a) the buffer reaches 20 chars, OR
            //   (b) 30ms have elapsed since the last flush, OR
            //   (c) a non-content event (Usage, ContextUpdate) arrives.
            let mut content_buffer = String::new();
            let mut last_flush = std::time::Instant::now();
            const FLUSH_CHAR_THRESHOLD: usize = 20;
            const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(30);

            while let Some(chunk_res) = stream.next().await {
                // Check Cancellation
                if state
                    .cancellation_token
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    // Flush any buffered content before sending stop
                    if !content_buffer.is_empty() {
                        let _ = on_event.send(StreamChunk {
                            content: std::mem::take(&mut content_buffer),
                            done: false,
                            usage: None,
                            context_update: None,
                        });
                    }
                    let _ = on_event.send(StreamChunk {
                        content: "\n[Stopped]".into(),
                        done: true,
                        usage: None,
                        context_update: None,
                    });
                    return Ok(());
                }

                match chunk_res {
                    Ok(event) => {
                        use crate::rig_lib::unified_provider::ProviderEvent;
                        match event {
                            ProviderEvent::Content(text) => {
                                content_buffer.push_str(&text);
                                let elapsed = last_flush.elapsed();
                                if content_buffer.len() >= FLUSH_CHAR_THRESHOLD
                                    || elapsed >= FLUSH_INTERVAL
                                {
                                    let _ = on_event.send(StreamChunk {
                                        content: std::mem::take(&mut content_buffer),
                                        done: false,
                                        usage: None,
                                        context_update: None,
                                    });
                                    last_flush = std::time::Instant::now();
                                }
                            }
                            ProviderEvent::Usage(u) => {
                                // Flush any buffered text before sending metadata
                                if !content_buffer.is_empty() {
                                    let _ = on_event.send(StreamChunk {
                                        content: std::mem::take(&mut content_buffer),
                                        done: false,
                                        usage: None,
                                        context_update: None,
                                    });
                                    last_flush = std::time::Instant::now();
                                }
                                let _ = on_event.send(StreamChunk {
                                    content: "".into(),
                                    done: false,
                                    usage: Some(u),
                                    context_update: None,
                                });
                            }
                            ProviderEvent::ContextUpdate(c) => {
                                // Flush any buffered text before sending metadata
                                if !content_buffer.is_empty() {
                                    let _ = on_event.send(StreamChunk {
                                        content: std::mem::take(&mut content_buffer),
                                        done: false,
                                        usage: None,
                                        context_update: None,
                                    });
                                    last_flush = std::time::Instant::now();
                                }
                                let _ = on_event.send(StreamChunk {
                                    content: "".into(),
                                    done: false,
                                    usage: None,
                                    context_update: Some(c),
                                });
                            }
                        }
                    }
                    Err(e) => {
                        // Flush buffer before error
                        if !content_buffer.is_empty() {
                            let _ = on_event.send(StreamChunk {
                                content: std::mem::take(&mut content_buffer),
                                done: false,
                                usage: None,
                                context_update: None,
                            });
                        }
                        eprintln!("Error in stream: {}", e);
                        let _ = on_event.send(StreamChunk {
                            content: format!("\n[Error: {}]", e),
                            done: true,
                            usage: None,
                            context_update: None,
                        });
                    }
                }
            }

            // Flush any remaining buffered content before sending done
            if !content_buffer.is_empty() {
                let _ = on_event.send(StreamChunk {
                    content: content_buffer,
                    done: false,
                    usage: None,
                    context_update: None,
                });
            }

            let _ = on_event.send(StreamChunk {
                content: "".into(),
                done: true,
                usage: None,
                context_update: None,
            });
            return Ok(());
        }
        Err(e) => {
            let _ = on_event.send(StreamChunk {
                content: format!("⚠️ Orchestrator Error: {}", e),
                done: true,
                usage: None,
                context_update: None,
            });
            return Ok(());
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn direct_chat_count_tokens(
    app: tauri::AppHandle,
    state: State<'_, SidecarManager>,
    engine_manager: State<'_, crate::engine::EngineManager>,
    conversation_id: String,
) -> Result<TokenUsage, String> {
    use tauri::Manager;

    // 1. Fetch Messages from DB
    let pool = app.state::<sqlx::SqlitePool>();

    #[derive(sqlx::FromRow)]
    struct DbMessage {
        role: String,
        content: String,
    }

    let messages = sqlx::query_as::<_, DbMessage>(
        "SELECT role, content FROM messages WHERE conversation_id = ? ORDER BY created_at ASC",
    )
    .bind(conversation_id)
    .fetch_all(pool.inner())
    .await
    .map_err(|e| format!("DB Error: {}", e))?;

    // 2. Try precise count via the shared local runtime snapshot when available.
    let snapshot = crate::engine::local_runtime_snapshot(&state, &engine_manager).await;
    if let Some(endpoint) = snapshot.endpoint {
        let mut check_history: Vec<serde_json::Value> = Vec::new();
        for msg in &messages {
            check_history.push(serde_json::json!({ "role": msg.role, "content": msg.content }));
        }

        let base_url = endpoint.base_url.trim_end_matches('/').to_string();
        let token = endpoint.api_key.unwrap_or_default();
        let model_family = endpoint
            .model_family
            .unwrap_or_else(|| "unknown".to_string());
        let provider = crate::rig_lib::llama_provider::LlamaProvider::new(
            &base_url,
            &token,
            "default",
            &model_family,
        );

        if let Ok(count) = provider.count_tokens(check_history).await {
            return Ok(TokenUsage {
                prompt_tokens: count,
                completion_tokens: 0,
                total_tokens: count,
            });
        }
    }

    // 3. Fallback: heuristic estimate (chars / 4) for MLX, Ollama, cloud
    let total_chars: u32 = messages.iter().map(|m| m.content.len() as u32).sum();
    let estimate = total_chars / 4;
    tracing::debug!(
        "[direct_chat_count_tokens] Using heuristic estimate (chars/4): {} chars → ~{} tokens",
        total_chars,
        estimate
    );
    Ok(TokenUsage {
        prompt_tokens: estimate,
        completion_tokens: 0,
        total_tokens: estimate,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn direct_chat_completion(
    _app: tauri::AppHandle,
    state: State<'_, SidecarManager>,
    config: State<'_, crate::config::ConfigManager>,
    secret_store: State<'_, crate::secret_store::SecretStore>,
    engine_manager: State<'_, crate::engine::EngineManager>,
    payload: ChatPayload,
) -> Result<String, String> {
    info!("[direct_chat_completion] Starting direct_chat_completion...");

    let user_config = config.get_config();

    // Resolve provider — keys from SecretStore, model from UserConfig
    let provider_cfg =
        resolve_provider(&user_config, &secret_store, &state, &engine_manager).await?;

    let provider = crate::rig_lib::unified_provider::UnifiedProvider::new(
        provider_cfg.kind,
        &provider_cfg.base_url,
        &provider_cfg.token,
        &provider_cfg.model_name,
        provider_cfg.model_family,
    );

    // Construct the request
    if payload.messages.is_empty() {
        return Err("No messages provided".into());
    }
    let mut history = Vec::new();
    let mut system_preamble = None;

    for msg in &payload.messages[..payload.messages.len() - 1] {
        if msg.role == "system" {
            system_preamble = Some(msg.content.clone());
        } else {
            history.push(rig::completion::Message {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }
    }

    let last_msg = payload.messages.last().ok_or("No messages")?;

    let request = rig::completion::CompletionRequest {
        preamble: system_preamble,
        chat_history: history,
        prompt: last_msg.content.clone(),
        documents: vec![],
        tools: Vec::new(),
        temperature: Some(payload.temperature as f64),
        max_tokens: None,
        additional_params: None,
    };

    use rig::completion::CompletionModel;
    let response = provider
        .completion(request)
        .await
        .map_err(|e| format!("Completion failed: {}", e))?;

    match response.choice {
        rig::completion::ModelChoice::Message(content) => Ok(content),
        _ => Err("Received tool call instead of message".into()),
    }
}
