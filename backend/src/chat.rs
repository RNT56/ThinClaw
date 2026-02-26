use crate::sidecar::SidecarManager;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{ipc::Channel, State};
use tracing::info;

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct AttachedDoc {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct Message {
    pub role: String,
    pub content: String,
    pub images: Option<Vec<String>>,
    pub attached_docs: Option<Vec<AttachedDoc>>,
    // New fields for summarization
    pub is_summary: Option<bool>,
    pub original_messages: Option<Vec<Message>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ChatPayload {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub top_p: f32,
    #[serde(default)]
    pub web_search_enabled: bool, // Legacy: Map this to auto_mode on frontend if needed
    #[serde(default)]
    pub auto_mode: bool, // New Flag
    pub project_id: Option<String>,
    pub conversation_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

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

pub async fn resolve_provider(
    user_config: &crate::config::UserConfig,
    secret_store: &crate::secret_store::SecretStore,
    openclaw: &State<'_, crate::openclaw::commands::OpenClawManager>,
    sidecar_manager: &State<'_, SidecarManager>,
    engine_manager: &State<'_, crate::engine::EngineManager>,
) -> Result<ProviderConfig, String> {
    match user_config.selected_chat_provider.as_deref() {
        Some("anthropic") => {
            info!("[resolve_provider] Routing to Anthropic");
            let key = secret_store
                .get("anthropic")
                .ok_or("Anthropic API key required. Please set it in Settings > Secrets.")?;
            // selected_cloud_model is a non-secret preference on OpenClawIdentity.
            // Reading it from OpenClawConfig is fine — it's not an API key.
            let model_name = openclaw
                .get_config()
                .await
                .and_then(|cfg| cfg.selected_cloud_model.clone())
                .unwrap_or_else(|| "claude-3-5-sonnet-latest".to_string());
            Ok(ProviderConfig {
                kind: crate::rig_lib::unified_provider::ProviderKind::Anthropic,
                base_url: "https://api.anthropic.com/v1".to_string(),
                model_name,
                port: 0,
                token: key,
                context_size: 200000,
                model_family: None,
            })
        }
        Some("openai") => {
            info!("[resolve_provider] Routing to OpenAI");
            let key = secret_store
                .get("openai")
                .ok_or("OpenAI API key required. Please set it in Settings > Secrets.")?;
            let model_name = openclaw
                .get_config()
                .await
                .and_then(|cfg| cfg.selected_cloud_model.clone())
                .unwrap_or_else(|| "gpt-4o".to_string());
            Ok(ProviderConfig {
                kind: crate::rig_lib::unified_provider::ProviderKind::OpenAI,
                base_url: "https://api.openai.com/v1".to_string(),
                model_name,
                port: 0,
                token: key,
                context_size: 128000,
                model_family: None,
            })
        }
        Some("openrouter") => {
            info!("[resolve_provider] Routing to OpenRouter");
            let key = secret_store
                .get("openrouter")
                .ok_or("OpenRouter API key required. Please set it in Settings > Secrets.")?;
            let model_name = openclaw
                .get_config()
                .await
                .and_then(|cfg| cfg.selected_cloud_model.clone())
                .unwrap_or_else(|| "moonshotai/kimi-k2.5".to_string());
            Ok(ProviderConfig {
                kind: crate::rig_lib::unified_provider::ProviderKind::OpenRouter,
                base_url: "https://openrouter.ai/api/v1".to_string(),
                model_name,
                port: 0,
                token: key,
                context_size: 128000,
                model_family: None,
            })
        }
        Some("gemini") => {
            info!("[resolve_provider] Routing to Gemini");
            let key = secret_store
                .get("gemini")
                .ok_or("Gemini API key required. Please set it in Settings > Secrets.")?;
            let model_name = openclaw
                .get_config()
                .await
                .and_then(|cfg| cfg.selected_cloud_model.clone())
                .unwrap_or_else(|| "gemini-2.0-flash".to_string());
            Ok(ProviderConfig {
                kind: crate::rig_lib::unified_provider::ProviderKind::Gemini,
                base_url: "https://generativelanguage.googleapis.com/v1beta/models".to_string(),
                model_name,
                port: 0,
                token: key,
                context_size: 128000,
                model_family: None,
            })
        }
        Some("groq") => {
            info!("[resolve_provider] Routing to Groq");
            let key = secret_store
                .get("groq")
                .ok_or("Groq API key required. Please set it in Settings > Secrets.")?;
            let model_name = openclaw
                .get_config()
                .await
                .and_then(|cfg| cfg.selected_cloud_model.clone())
                .unwrap_or_else(|| "llama-3.3-70b-versatile".to_string());
            Ok(ProviderConfig {
                kind: crate::rig_lib::unified_provider::ProviderKind::OpenAI,
                base_url: "https://api.groq.com/openai/v1".to_string(),
                model_name,
                port: 0,
                token: key,
                context_size: 128000,
                model_family: None,
            })
        }
        _ => {
            info!("[resolve_provider] Routing to Local Provider");

            // Primary: llama.cpp sidecar (always present in llamacpp builds)
            if let Some(cfg) = sidecar_manager.get_chat_config() {
                return Ok(ProviderConfig {
                    kind: crate::rig_lib::unified_provider::ProviderKind::Local,
                    base_url: format!("http://127.0.0.1:{}/v1", cfg.0),
                    model_name: "default".to_string(),
                    port: cfg.0,
                    token: cfg.1,
                    context_size: cfg.2,
                    model_family: Some(cfg.3),
                });
            }

            // Fallback: non-llamacpp engine (MLX, vLLM, Ollama) running via EngineManager.
            // start_engine() must have been called first (done by useAutoStart).
            {
                let guard = engine_manager.engine.lock().await;
                if let Some(engine) = guard.as_ref() {
                    if let Some(url) = engine.base_url() {
                        let model_name = engine.model_id().unwrap_or_else(|| "default".to_string());
                        let context_size = engine.max_context().unwrap_or(4096);
                        info!(
                            "[resolve_provider] Using EngineManager base_url: {}, model: {}, context: {}",
                            url, model_name, context_size
                        );
                        // Parse port from URL like "http://127.0.0.1:PORT/v1"
                        let port: u16 = url
                            .trim_end_matches('/')
                            .rsplit(':')
                            .next()
                            .and_then(|p| p.split('/').next())
                            .and_then(|p| p.parse().ok())
                            .unwrap_or(8080);
                        return Ok(ProviderConfig {
                            kind: crate::rig_lib::unified_provider::ProviderKind::Local,
                            base_url: url,
                            model_name,
                            port,
                            // mlx_lm.server runs unauthenticated by default
                            token: String::new(),
                            context_size,
                            model_family: None,
                        });
                    }
                }
            }

            Err("No local inference server is running. \
                 Select a model in the chat tab — the engine will start automatically."
                .to_string())
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn chat_stream(
    app: tauri::AppHandle,
    state: State<'_, SidecarManager>,
    config: State<'_, crate::config::ConfigManager>,
    secret_store: State<'_, crate::secret_store::SecretStore>,
    openclaw: State<'_, crate::openclaw::commands::OpenClawManager>,
    engine_manager: State<'_, crate::engine::EngineManager>,
    rig_cache: State<'_, crate::rig_cache::RigManagerCache>,
    payload: ChatPayload,
    on_event: Channel<StreamChunk>,
) -> Result<(), String> {
    info!("[chat_stream] Starting chat_stream command...");

    // Acquisition of Global Generation Lock (Queuing)
    let _guard = state.generation_lock.lock().await;
    info!("[chat_stream] Generation lock acquired.");

    // Reset Cancellation Token for the CURRENT active job
    state
        .cancellation_token
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // Clone messages so we can modify them
    let mut processing_messages = payload.messages.clone();

    // General Knowledge Injection
    let user_config = config.get_config();

    // Provider Routing Logic — keys from SecretStore, model selection from OpenClawConfig
    let provider_cfg = resolve_provider(
        &user_config,
        &secret_store,
        &openclaw,
        &state,
        &engine_manager,
    )
    .await?;
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

    // (Optional) We could use this to check strict RAG mode or other logic
    let _last_user_content = processing_messages[last_idx].content.clone();

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

        if let Some(image_ids) = &msg.images {
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

                    for id in image_ids {
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
    }

    // --- Image History Filtering (User Requested) ---
    // If the user is NOT explicitly talking about images/pictures in their latest prompt,
    // we strip previous image generation turns to keep the LLM focused on chat and save context.
    let last_prompt_lower = processing_messages
        .last()
        .map(|m| m.content.to_lowercase())
        .unwrap_or_default();
    let is_referencing_image = last_prompt_lower.contains("image")
        || last_prompt_lower.contains("picture")
        || last_prompt_lower.contains("draw")
        || last_prompt_lower.contains("this one")
        || last_prompt_lower.contains("that one")
        || last_prompt_lower.contains("it")
            && processing_messages
                .iter()
                .any(|m| m.role == "assistant" && m.images.as_ref().is_some_and(|i| !i.is_empty()));

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
        "[chat_stream] Getting RigManager for model: {}",
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
                    step: "searching".into(),
                    message: "Web Search Active...".into(),
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
            "[chat_stream] Sandbox mode ENABLED — MCP server: {}",
            mcp_config.mcp_base_url.as_deref().unwrap_or("(none)")
        );
    } else {
        info!("[chat_stream] Sandbox mode (local-only, no remote MCP)");
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

    info!("[chat_stream] Starting orchestrator turn...");
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
            info!("[chat_stream] Orchestrator turn started.");
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
                            done: false,
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
pub async fn count_tokens(
    app: tauri::AppHandle,
    state: State<'_, SidecarManager>,
    conversation_id: String,
) -> Result<TokenUsage, String> {
    use tauri::Manager;
    // 1. Get Chat Config
    let (port, token, _, model_family) =
        state.get_chat_config().ok_or("Chat server not running")?;

    // 2. Fetch Messages from DB Directly
    let pool = app.state::<sqlx::SqlitePool>();

    // Define minimal Message struct for query
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

    // 3. Convert to JSON for Rig
    let mut check_history: Vec<serde_json::Value> = Vec::new();
    for msg in messages {
        check_history.push(serde_json::json!({ "role": msg.role, "content": msg.content }));
    }

    // 4. Initialize ephemeral Rig/Provider to count
    let base_url = format!("http://127.0.0.1:{}/v1", port);
    // Token is already a String based on SidecarManager signature
    let provider = crate::rig_lib::llama_provider::LlamaProvider::new(
        &base_url,
        &token,
        "default",
        &model_family,
    );

    let count = provider
        .count_tokens(check_history)
        .await
        .map_err(|e| e.to_string())?;

    Ok(TokenUsage {
        prompt_tokens: count,
        completion_tokens: 0,
        total_tokens: count,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn chat_completion(
    _app: tauri::AppHandle,
    state: State<'_, SidecarManager>,
    config: State<'_, crate::config::ConfigManager>,
    secret_store: State<'_, crate::secret_store::SecretStore>,
    openclaw: State<'_, crate::openclaw::commands::OpenClawManager>,
    engine_manager: State<'_, crate::engine::EngineManager>,
    payload: ChatPayload,
) -> Result<String, String> {
    info!("[chat_completion] Starting chat_completion...");

    let user_config = config.get_config();

    // Resolve provider — keys from SecretStore, model selection from OpenClawConfig
    let provider_cfg = resolve_provider(
        &user_config,
        &secret_store,
        &openclaw,
        &state,
        &engine_manager,
    )
    .await?;

    let provider = crate::rig_lib::unified_provider::UnifiedProvider::new(
        provider_cfg.kind,
        &provider_cfg.base_url,
        &provider_cfg.token,
        &provider_cfg.model_name,
        provider_cfg.model_family,
    );

    // Construct the request
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
