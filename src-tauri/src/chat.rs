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

#[tauri::command]
#[specta::specta]
pub async fn chat_stream(
    app: tauri::AppHandle,
    state: State<'_, SidecarManager>,
    config: State<'_, crate::config::ConfigManager>,
    openclaw: State<'_, crate::openclaw::commands::OpenClawManager>,
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

    // Provider Routing Logic
    let (kind, base_url, model_name, _port, token, context_size, model_family) = match user_config
        .selected_chat_provider
        .as_deref()
    {
        Some("anthropic") => {
            info!("[chat_stream] Routing to Anthropic");
            let claw_cfg = openclaw
                .get_config()
                .await
                .ok_or("OpenClaw config not found")?;
            let key = claw_cfg
                .anthropic_api_key
                .ok_or("Anthropic API key required. Please set it in Settings > Secrets.")?;
            (
                crate::rig_lib::unified_provider::ProviderKind::Anthropic,
                "https://api.anthropic.com/v1".to_string(),
                claw_cfg
                    .selected_cloud_model
                    .unwrap_or_else(|| "claude-3-5-sonnet-latest".to_string()),
                0,
                key,
                200000,
                None,
            )
        }
        Some("openai") => {
            info!("[chat_stream] Routing to OpenAI");
            let claw_cfg = openclaw
                .get_config()
                .await
                .ok_or("OpenClaw config not found")?;
            let key = claw_cfg
                .openai_api_key
                .ok_or("OpenAI API key required. Please set it in Settings > Secrets.")?;
            (
                crate::rig_lib::unified_provider::ProviderKind::OpenAI,
                "https://api.openai.com/v1".to_string(),
                claw_cfg
                    .selected_cloud_model
                    .unwrap_or_else(|| "gpt-4o".to_string()),
                0,
                key,
                128000,
                None,
            )
        }
        Some("openrouter") => {
            info!("[chat_stream] Routing to OpenRouter");
            let claw_cfg = openclaw
                .get_config()
                .await
                .ok_or("OpenClaw config not found")?;
            let key = claw_cfg
                .openrouter_api_key
                .ok_or("OpenRouter API key required. Please set it in Settings > Secrets.")?;
            (
                crate::rig_lib::unified_provider::ProviderKind::OpenRouter,
                "https://openrouter.ai/api/v1".to_string(),
                claw_cfg
                    .selected_cloud_model
                    .unwrap_or_else(|| "moonshotai/kimi-k2.5".to_string()),
                0,
                key,
                128000,
                None,
            )
        }
        Some("gemini") => {
            info!("[chat_stream] Routing to Gemini");
            let claw_cfg = openclaw
                .get_config()
                .await
                .ok_or("OpenClaw config not found")?;
            let key = claw_cfg
                .gemini_api_key
                .ok_or("Gemini API key required. Please set it in Settings > Secrets.")?;
            (
                crate::rig_lib::unified_provider::ProviderKind::Gemini,
                "https://generativelanguage.googleapis.com/v1beta/models".to_string(),
                claw_cfg
                    .selected_cloud_model
                    .unwrap_or_else(|| "gemini-2.0-flash".to_string()),
                0,
                key,
                128000,
                None,
            )
        }
        Some("groq") => {
            info!("[chat_stream] Routing to Groq");
            let claw_cfg = openclaw
                .get_config()
                .await
                .ok_or("OpenClaw config not found")?;
            let key = claw_cfg
                .groq_api_key
                .ok_or("Groq API key required. Please set it in Settings > Secrets.")?;
            (
                crate::rig_lib::unified_provider::ProviderKind::OpenAI,
                "https://api.groq.com/openai/v1".to_string(),
                claw_cfg
                    .selected_cloud_model
                    .unwrap_or_else(|| "llama-3.3-70b-versatile".to_string()),
                0,
                key,
                128000,
                None,
            )
        }
        _ => {
            info!("[chat_stream] Routing to Local Provider");
            let cfg = state.get_chat_config().ok_or("Local Neural Link is not running. Please start it or select a Cloud Brain in Settings > Chat Provider.")?;
            (
                crate::rig_lib::unified_provider::ProviderKind::Local,
                format!("http://127.0.0.1:{}/v1", cfg.0),
                "default".to_string(),
                cfg.0,
                cfg.1,
                cfg.2,
                Some(cfg.3),
            )
        }
    };

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

    // Check if images are present (Rig doesn't support multimodal yet in our impl, so we hack it via content string)
    for msg in processing_messages.iter_mut() {
        // CRITICAL: Only send images for USER messages.
        // Assistant-generated images should NOT be sent back as base64 in history,
        // as they cause massive context bloat and tokenization errors.
        if msg.role != "user" {
            continue;
        }

        if let Some(image_ids) = &msg.images {
            if !image_ids.is_empty() {
                // Construct multimodal parts
                let mut parts = Vec::new();
                parts.push(serde_json::json!({
                    "type": "text",
                    "text": msg.content
                }));

                for id in image_ids {
                    match crate::images::load_image_as_base64(&app, id).await {
                        Ok(b64) => {
                            parts.push(serde_json::json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:image/jpeg;base64,{}", b64)
                                }
                            }));
                        }
                        Err(e) => eprintln!("Failed to load image {}: {}", id, e),
                    }
                }

                // Pack into content string
                if let Ok(json_str) = serde_json::to_string(&parts) {
                    msg.content = json_str;
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
        "[chat_stream] Creating RigManager for model: {}",
        &model_name
    );
    let manager = RigManager::new(
        kind,
        base_url,
        model_name.clone(),
        Some(app.clone()),
        Some(token.clone()),
        context_size as usize,
        None,
        enable_tools,
        if gk_content.trim().is_empty() {
            None
        } else {
            Some(gk_content)
        },
        payload.conversation_id.clone(),
        model_family,
    );

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
        info!("[chat_stream] Legacy tool mode (sandbox disabled)");
    }
    let orchestrator = crate::rig_lib::orchestrator::Orchestrator::new_with_mcp(
        std::sync::Arc::new(manager),
        mcp_config,
    );

    let permissions = crate::rig_lib::orchestrator::ToolPermissions {
        allow_web_search: payload.auto_mode || payload.web_search_enabled,
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
            if payload.auto_mode || has_context {
                if let Some(id) = &payload.conversation_id {
                    let _ = app.emit(
                        "web_search_status",
                        WebSearchStatus {
                            id: id.clone(),
                            step: "done".into(),
                            message: "Responding...".into(),
                        },
                    );
                }
            }

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
    openclaw: State<'_, crate::openclaw::commands::OpenClawManager>,
    payload: ChatPayload,
) -> Result<String, String> {
    info!("[chat_completion] Starting chat_completion...");

    let user_config = config.get_config();

    // Re-use the provider routing logic from chat_stream
    let (kind, base_url, model_name, _port, token, _context_size, model_family) =
        match user_config.selected_chat_provider.as_deref() {
            Some("anthropic") => {
                let claw_cfg = openclaw
                    .get_config()
                    .await
                    .ok_or("OpenClaw config not found")?;
                let key = claw_cfg
                    .anthropic_api_key
                    .ok_or("Anthropic API key required")?;
                (
                    crate::rig_lib::unified_provider::ProviderKind::Anthropic,
                    "https://api.anthropic.com/v1".to_string(),
                    claw_cfg
                        .selected_cloud_model
                        .unwrap_or_else(|| "claude-3-5-sonnet-latest".to_string()),
                    0,
                    key,
                    200000,
                    None,
                )
            }
            Some("openai") => {
                let claw_cfg = openclaw
                    .get_config()
                    .await
                    .ok_or("OpenClaw config not found")?;
                let key = claw_cfg.openai_api_key.ok_or("OpenAI API key required")?;
                (
                    crate::rig_lib::unified_provider::ProviderKind::OpenAI,
                    "https://api.openai.com/v1".to_string(),
                    claw_cfg
                        .selected_cloud_model
                        .unwrap_or_else(|| "gpt-4o".to_string()),
                    0,
                    key,
                    128000,
                    None,
                )
            }
            Some("gemini") => {
                let claw_cfg = openclaw
                    .get_config()
                    .await
                    .ok_or("OpenClaw config not found")?;
                let key = claw_cfg.gemini_api_key.ok_or("Gemini API key required")?;
                (
                    crate::rig_lib::unified_provider::ProviderKind::Gemini,
                    "https://generativelanguage.googleapis.com/v1beta/models".to_string(),
                    claw_cfg
                        .selected_cloud_model
                        .unwrap_or_else(|| "gemini-2.0-flash".to_string()),
                    0,
                    key,
                    128000,
                    None,
                )
            }
            // ... (can add others if needed, but Local is the main standard)
            _ => {
                let cfg = state
                    .get_chat_config()
                    .ok_or("Local Neural Link not running")?;
                (
                    crate::rig_lib::unified_provider::ProviderKind::Local,
                    format!("http://127.0.0.1:{}/v1", cfg.0),
                    "default".to_string(),
                    cfg.0,
                    cfg.1,
                    cfg.2,
                    Some(cfg.3),
                )
            }
        };

    let provider = crate::rig_lib::unified_provider::UnifiedProvider::new(
        kind,
        &base_url,
        &token,
        &model_name,
        model_family,
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
