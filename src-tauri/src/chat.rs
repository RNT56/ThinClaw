use crate::sidecar::SidecarManager;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{ipc::Channel, State};

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
    clawdbot: State<'_, crate::clawdbot::commands::ClawdbotManager>,
    payload: ChatPayload,
    on_event: Channel<StreamChunk>,
) -> Result<(), String> {
    // Acquisition of Global Generation Lock (Queuing)
    let _guard = state.generation_lock.lock().await;

    // Default to Local Sidecar
    let (port, mut token, mut context_size) =
        state.get_chat_config().ok_or("Chat server not running")?;

    // Reset Cancellation Token for the CURRENT active job
    state
        .cancellation_token
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // Clone messages so we can modify them
    let mut processing_messages = payload.messages.clone();

    // General Knowledge Injection
    let user_config = config.get_config();

    // Provider Routing Logic
    let (_base_url, model_name) = match user_config.selected_chat_provider.as_deref() {
        Some("anthropic") => {
            let claw_cfg = clawdbot
                .get_config()
                .await
                .ok_or("Clawdbot config not found")?;
            let key = claw_cfg
                .anthropic_api_key
                .ok_or("Anthropic API key required. Please set it in Settings > Secrets.")?;
            token = key;
            context_size = 200000; // Cloud context
            (
                "https://api.anthropic.com/v1".to_string(),
                "claude-3-5-sonnet-latest".to_string(),
            )
        }
        Some("openai") => {
            let claw_cfg = clawdbot
                .get_config()
                .await
                .ok_or("Clawdbot config not found")?;
            let key = claw_cfg
                .openai_api_key
                .ok_or("OpenAI API key required. Please set it in Settings > Secrets.")?;
            token = key;
            context_size = 128000;
            (
                "https://api.openai.com/v1".to_string(),
                "gpt-4o".to_string(),
            )
        }
        Some("openrouter") => {
            let claw_cfg = clawdbot
                .get_config()
                .await
                .ok_or("Clawdbot config not found")?;
            let key = claw_cfg
                .openrouter_api_key
                .ok_or("OpenRouter API key required. Please set it in Settings > Secrets.")?;
            token = key;
            context_size = 128000;
            (
                "https://openrouter.ai/api/v1".to_string(),
                "anthropic/claude-3.5-sonnet".to_string(),
            )
        }
        _ => {
            // Local case - ensure local and cloud are checked
            let _ = state.get_chat_config().ok_or("Local Neural Link is not running. Please start it or select a Cloud Brain in Settings > Chat Provider.")?;
            (
                format!("http://127.0.0.1:{}/v1", port),
                "default".to_string(),
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

    let base_url = format!("http://127.0.0.1:{}/v1", port);

    // Support Legacy Web Search Icon: Treat it as Auto Mode
    let has_context = payload.project_id.is_some()
        || processing_messages
            .iter()
            .any(|m| m.attached_docs.as_ref().is_some_and(|d| !d.is_empty()));

    // We use the Agent if Auto Mode is ON, OR if we have context (RAG/Files), OR if we have images (since Lrama/Rig handling is unified here)
    // Actually, Orchestrator is our default pipeline now.
    let effective_auto_mode = payload.auto_mode || payload.web_search_enabled || has_context;
    let enable_tools = effective_auto_mode; // Or always true? Tools are gated by permissions anyway.

    let manager = RigManager::new(
        base_url,
        model_name,
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

    // Use the Orchestrator
    let orchestrator =
        crate::rig_lib::orchestrator::Orchestrator::new(std::sync::Arc::new(manager));

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

            // Consume stream and emit chunks
            while let Some(chunk_res) = stream.next().await {
                // Check Cancellation
                if state
                    .cancellation_token
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    let _ = on_event.send(StreamChunk {
                        content: "\n[Stopped]".into(), // Visual indicator
                        done: true,
                        usage: None,
                        context_update: None,
                    });
                    return Ok(());
                }

                match chunk_res {
                    Ok(event) => {
                        use crate::rig_lib::llama_provider::ProviderEvent;
                        match event {
                            ProviderEvent::Content(text) => {
                                let _ = on_event.send(StreamChunk {
                                    content: text,
                                    done: false,
                                    usage: None,
                                    context_update: None,
                                });
                            }
                            ProviderEvent::Usage(u) => {
                                let _ = on_event.send(StreamChunk {
                                    content: "".into(),
                                    done: false,
                                    usage: Some(u),
                                    context_update: None,
                                });
                            }
                            ProviderEvent::ContextUpdate(c) => {
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
    let (port, token, _) = state.get_chat_config().ok_or("Chat server not running")?;

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
    let provider = crate::rig_lib::llama_provider::LlamaProvider::new(&base_url, &token);

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
