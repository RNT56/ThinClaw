use crate::rig_lib::RigManager;
use serde_json::json;
use std::sync::Arc;
use thinclaw_desktop_tools::events::{StatusReporter, ToolEvent};
use thinclaw_desktop_tools::sandbox::Sandbox;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

fn status_tag(attrs: &str) -> String {
    format!("\n<thinclaw_status {attrs} />\n")
}

/// Extract the text-only portion from a message content string.
///
/// When images are present, `content` is a JSON array like:
/// ```json
/// [{"type":"text","text":"What is this?"},{"type":"image_url","image_url":{...}}]
/// ```
///
/// This function extracts and concatenates just the text parts for use in
/// RAG queries and tool routing, where base64 image data would be harmful.
/// For plain text content, returns the string unchanged.
pub(crate) fn extract_text_from_content(content: &str) -> String {
    if content.trim().starts_with('[') {
        serde_json::from_str::<Vec<serde_json::Value>>(content)
            .ok()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| {
                        if p["type"].as_str() == Some("text") {
                            p["text"].as_str().map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| content.to_string())
    } else {
        content.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tool Permissions
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ToolPermissions {
    pub allow_web_search: bool,
    /// When true, the user explicitly toggled the search icon — the LLM should
    /// aggressively search for every query.  When false (auto mode), the LLM
    /// should only search when genuinely needed.
    pub force_web_search: bool,
    pub allow_file_search: bool,
    pub allow_image_gen: bool,
}

// ---------------------------------------------------------------------------
// MCP Configuration
// ---------------------------------------------------------------------------

/// Configuration for the optional remote MCP server connection.
#[derive(Clone, Debug, Default)]
pub struct McpOrchestratorConfig {
    /// Base URL of the FastAPI MCP server (e.g. "https://api.thinclaw.dev")
    pub mcp_base_url: Option<String>,
    /// JWT bearer token for the MCP server
    pub mcp_auth_token: Option<String>,
    /// Whether to enable sandbox execution mode (Rhai code execution)
    pub sandbox_enabled: bool,
}

// ---------------------------------------------------------------------------
// Status Reporter → ProviderEvent bridge
// ---------------------------------------------------------------------------

/// Bridges `ToolEvent` from the sandbox to `ProviderEvent::Content` XML tags
/// that the frontend already knows how to render.
struct OrchestratorStatusReporter {
    tx: mpsc::Sender<Result<crate::rig_lib::unified_provider::ProviderEvent, String>>,
}

#[async_trait::async_trait]
impl StatusReporter for OrchestratorStatusReporter {
    async fn report(&self, event: ToolEvent) {
        use crate::rig_lib::unified_provider::ProviderEvent;
        let xml_tag = match event {
            ToolEvent::ToolActivity {
                tool_name,
                input_summary,
                status,
            } => status_tag(&format!(
                "type=\"tool_call\" name=\"{}\" query=\"{}\" status=\"{}\"",
                tool_name, input_summary, status
            )),
            ToolEvent::Status { msg, .. } => {
                status_tag(&format!("type=\"thinking\" msg=\"{}\"", msg))
            }
            ToolEvent::Progress {
                percentage,
                message,
            } => status_tag(&format!(
                "type=\"progress\" pct=\"{:.0}\" msg=\"{}\"",
                percentage, message
            )),
        };

        if !xml_tag.is_empty() {
            let _ = self.tx.send(Ok(ProviderEvent::Content(xml_tag))).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

pub struct Orchestrator {
    rig: Arc<RigManager>,
    mcp_config: McpOrchestratorConfig,
}

impl Orchestrator {
    /// Construct with optional MCP sandbox configuration.
    pub fn new_with_mcp(rig: Arc<RigManager>, mcp_config: McpOrchestratorConfig) -> Self {
        Self { rig, mcp_config }
    }

    // -----------------------------------------------------------------------
    // Sandbox factory — uses shared factory
    // -----------------------------------------------------------------------
    fn build_sandbox(
        &self,
        tx: &mpsc::Sender<Result<crate::rig_lib::unified_provider::ProviderEvent, String>>,
    ) -> Option<Sandbox> {
        let reporter = Arc::new(OrchestratorStatusReporter { tx: tx.clone() });
        // Use the new shared factory
        // Convert local config to shared config
        let factory_config = crate::rig_lib::sandbox_factory::McpOrchestratorConfig {
            mcp_base_url: self.mcp_config.mcp_base_url.clone(),
            mcp_auth_token: self.mcp_config.mcp_auth_token.clone(),
            sandbox_enabled: self.mcp_config.sandbox_enabled,
            user_skills_path: self.rig.app_handle.as_ref().map(|app| {
                use tauri::Manager;
                app.path()
                    .app_config_dir()
                    .unwrap_or_default()
                    .join("skills")
            }),
            builtin_skills_path: self.rig.app_handle.as_ref().map(|app| {
                use tauri::Manager;
                app.path()
                    .resource_dir()
                    .unwrap_or_default()
                    .join("scrappy-mcp-tools/skills/built_in")
            }),
        };

        crate::rig_lib::sandbox_factory::create_sandbox(self.rig.clone(), &factory_config, reporter)
    }

    /// Build a sandbox unconditionally (ignoring the sandbox_enabled flag).
    /// Used when tools are enabled but no remote MCP server is configured —
    /// the sandbox still hosts local tools (web_search, rag_search, read_file).
    fn build_sandbox_unconditional(
        &self,
        tx: &mpsc::Sender<Result<crate::rig_lib::unified_provider::ProviderEvent, String>>,
    ) -> Option<Sandbox> {
        let reporter = Arc::new(OrchestratorStatusReporter { tx: tx.clone() });
        let factory_config = crate::rig_lib::sandbox_factory::McpOrchestratorConfig {
            mcp_base_url: self.mcp_config.mcp_base_url.clone(),
            mcp_auth_token: self.mcp_config.mcp_auth_token.clone(),
            sandbox_enabled: true, // Force enabled
            user_skills_path: self.rig.app_handle.as_ref().map(|app| {
                use tauri::Manager;
                app.path()
                    .app_config_dir()
                    .unwrap_or_default()
                    .join("skills")
            }),
            builtin_skills_path: self.rig.app_handle.as_ref().map(|app| {
                use tauri::Manager;
                app.path()
                    .resource_dir()
                    .unwrap_or_default()
                    .join("scrappy-mcp-tools/skills/built_in")
            }),
        };

        crate::rig_lib::sandbox_factory::create_sandbox(self.rig.clone(), &factory_config, reporter)
    }

    // -----------------------------------------------------------------------
    // Main entry point
    // -----------------------------------------------------------------------

    pub async fn run_turn(
        &self,
        mut messages: Vec<crate::chat::Message>,
        permissions: ToolPermissions,
        project_id: Option<String>,
        persona_instructions: String,
        conversation_id: Option<String>,
    ) -> Result<
        impl futures::Stream<Item = Result<crate::rig_lib::unified_provider::ProviderEvent, String>>,
        String,
    > {
        use crate::chat::{Message, TokenUsage};
        use crate::rig_lib::unified_provider::ProviderEvent;

        let (tx, rx) = mpsc::channel(100);
        let rig_clone = self.rig.clone();

        // Extract current turn
        let last_msg = messages.pop().ok_or("No messages provided")?;
        let raw_content = last_msg.content.clone();
        let history_clone = messages; // Remaining are history

        // When images are present, content is a JSON array with base64 data.
        // Extract just the text for RAG queries and tool routing.
        // The raw_content (with images) is passed to the actual LLM message.
        let query = extract_text_from_content(&raw_content);

        let project_id_clone = project_id.clone();
        let perms = permissions.clone();
        let persona_instructions = persona_instructions.clone();

        // Pass current turn docs to context collection
        let current_docs = last_msg.attached_docs.clone();
        let conversation_id_clone = conversation_id.clone(); // Clone for spawn

        // Compute before the spawn so it moves as a plain bool (not a borrow of self).
        let has_mcp = !self
            .mcp_config
            .mcp_base_url
            .as_deref()
            .unwrap_or("")
            .is_empty();

        // Build sandbox — always attempt to build one when tools may be needed.
        // If `mcp_config.sandbox_enabled` is true, `build_sandbox` succeeds.
        // Otherwise, fall back to `build_sandbox_unconditional` so that local
        // host tools (web_search, rag_search, read_file) remain available even
        // without a remote MCP server configured.
        let sandbox = self
            .build_sandbox(&tx)
            .or_else(|| self.build_sandbox_unconditional(&tx));

        tokio::spawn(async move {
            info!(
                "[orchestrator] Background task started for conversation: {:?}",
                conversation_id_clone
            );
            // --- 0. Token Check & Auto-Summarization ---
            // Construct a temporary JSON history to count tokens
            let mut check_history: Vec<serde_json::Value> = Vec::new();
            for msg in &history_clone {
                check_history.push(json!({ "role": msg.role, "content": msg.content }));
            }
            check_history.push(json!({ "role": "user", "content": query.clone() }));

            let mut final_history = history_clone.clone();

            // Configurable constants
            let max_context = rig_clone.context_window; // Configured context size
            let threshold = 0.6; // 60% to trigger proactive summarization (hardcoded logic)
            let _summarize_ratio = 0.5; // Summarize oldest 50%

            let mut should_summarize = false;

            // Performance: Use fast heuristic estimate first (~4 chars per token).
            // Only call the expensive tokenizer endpoint if the estimate is within
            // 80% of the threshold (i.e. we might actually need to summarize).
            let heuristic_tokens: u32 = check_history
                .iter()
                .map(|msg| {
                    let content_len = msg["content"].as_str().map_or(0, |s| s.len());
                    (content_len / 4) as u32 + 4 // +4 for role wrapper overhead
                })
                .sum();

            let threshold_tokens = (max_context as f32 * threshold) as u32;
            let needs_precise_count = heuristic_tokens > (threshold_tokens as f32 * 0.8) as u32;

            let token_count_res = if needs_precise_count {
                info!(
                    "[orchestrator] Heuristic estimate {} near threshold {}, performing precise count",
                    heuristic_tokens, threshold_tokens
                );
                rig_clone.provider.count_tokens(check_history.clone()).await
            } else {
                info!(
                    "[orchestrator] Heuristic token estimate: {} (threshold: {}), skipping precise count",
                    heuristic_tokens, threshold_tokens
                );
                Ok(heuristic_tokens)
            };

            match token_count_res {
                Ok(token_count) => {
                    info!("[orchestrator] Token count: {}", token_count);
                    // Send initial usage stats
                    let _ = tx
                        .send(Ok(ProviderEvent::Usage(TokenUsage {
                            prompt_tokens: token_count,
                            completion_tokens: 0,
                            total_tokens: token_count,
                        })))
                        .await;

                    if token_count > threshold_tokens {
                        should_summarize = true;
                        info!("[orchestrator] Token count exceeds threshold.");
                    }
                }
                Err(e) => {
                    warn!(
                        "[orchestrator] Failed to count tokens: {}. Checking message count fallback.",
                        e
                    );
                    if final_history.len() > 20 {
                        should_summarize = true;
                        info!("[orchestrator] Message count > 20. Truncating history.");
                    }
                }
            }

            if should_summarize {
                info!("[orchestrator] Starting summarization...");
                let _ = tx
                    .send(Ok(ProviderEvent::Content(status_tag(
                        "type=\"summarizing\"",
                    ))))
                    .await;

                // Identify chunk to summarize
                let messages_to_keep = final_history.len() / 2;
                let split_idx = final_history.len().saturating_sub(messages_to_keep);

                if split_idx > 0 {
                    let chunk_to_summarize = final_history.drain(0..split_idx).collect::<Vec<_>>();

                    // Prepare summarization prompt
                    let summary_prompt = format!(
                        "Summarize the following conversation history into a concise paragraph. Capture key decisions, user preferences, and important context. \n\nHISTORY:\n{}",
                        chunk_to_summarize
                            .iter()
                            .map(|m| format!("{}: {}", m.role, m.content))
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    );

                    // Call LLM for summary (Quick non-streaming call)
                    let summary_req = vec![json!({ "role": "user", "content": summary_prompt })];

                    info!("[orchestrator] Requesting summary from provider...");
                    let mut summary_text = String::new();
                    if let Ok(mut stream) = rig_clone
                        .provider
                        .stream_raw_completion(summary_req, Some(0.1))
                        .await
                    {
                        use futures::StreamExt;
                        while let Some(res) = stream.next().await {
                            if let Ok(ProviderEvent::Content(s)) = res {
                                summary_text.push_str(&s);
                            }
                        }
                    }

                    if !summary_text.is_empty() {
                        info!("[orchestrator] Summarization complete.");
                        // Create Summary Message
                        let summary_msg = Message {
                            role: "assistant".into(),
                            content: format!("[Summary of earlier conversation] {}", summary_text),
                            images: None,
                            attached_docs: None,
                            is_summary: Some(true),
                            original_messages: Some(chunk_to_summarize),
                        };

                        // Prepend summary
                        final_history.insert(0, summary_msg);

                        let _ = tx
                            .send(Ok(ProviderEvent::ContextUpdate(final_history.clone())))
                            .await;
                    } else {
                        warn!(
                            "[orchestrator] Summarization failed (empty response). History truncated regardless to save context."
                        );
                        let _ = tx
                            .send(Ok(ProviderEvent::ContextUpdate(final_history.clone())))
                            .await;
                    }
                }
            }

            info!("[orchestrator] Proceeding to tool/manual decision...");

            // 1. Context & Document Collection (Used for both Manual and Lead turns)
            let mut all_doc_ids = Vec::new();

            // Collect docs from history
            for msg in &final_history {
                if let Some(docs) = &msg.attached_docs {
                    for d in docs {
                        if !all_doc_ids.contains(&d.id) {
                            all_doc_ids.push(d.id.clone());
                        }
                    }
                }
            }

            // Collect docs from CURRENT message
            if let Some(docs) = &current_docs {
                for d in docs {
                    if !all_doc_ids.contains(&d.id) {
                        all_doc_ids.push(d.id.clone());
                    }
                }
            }

            let any_tools =
                perms.allow_web_search || perms.allow_file_search || perms.allow_image_gen;

            if !any_tools {
                // --- MANUAL MODE RAG & VISUAL INJECTION ---
                let mut manual_context = String::new();
                let mut visual_messages = Vec::new();

                if !all_doc_ids.is_empty() || project_id_clone.is_some() {
                    let _ = tx
                        .send(Ok(ProviderEvent::Content(status_tag(
                            "type=\"rag_search\" query=\"Retrieving context...\"",
                        ))))
                        .await;

                    if let Some(app) = &rig_clone.app_handle {
                        use tauri::Manager;
                        let sidecar = app.state::<crate::sidecar::SidecarManager>();
                        let pool = app.state::<sqlx::SqlitePool>();
                        let store = app.state::<crate::vector_store::VectorStoreManager>();
                        let reranker = app.state::<crate::reranker::RerankerWrapper>();

                        // Get embedding backend from InferenceRouter (if active)
                        let emb_backend = {
                            let router = app.state::<crate::inference::router::InferenceRouter>();
                            router.embedding_backend().await
                        };

                        // 1. Text RAG
                        let context_res = crate::rag::retrieve_context_internal(
                            Some(app.clone()),
                            sidecar.inner(),
                            pool.inner().clone(),
                            store.inner().clone(),
                            reranker.inner(),
                            emb_backend,
                            query.clone(),
                            conversation_id_clone.clone(),
                            if all_doc_ids.is_empty() {
                                None::<Vec<String>>
                            } else {
                                Some(all_doc_ids.clone())
                            },
                            project_id_clone.clone(),
                        )
                        .await;

                        if let Ok(results) = context_res {
                            if !results.is_empty() {
                                manual_context =
                                    format!("\n[ATTACHED CONTEXT]:\n{}\n", results.join("\n\n"));
                            }
                        }

                        // 2. Visual Previews (for Multimodal models)
                        for doc_id in all_doc_ids.iter().take(2) {
                            // Limit to 2 previews to save context
                            let hash_res: Result<String, _> =
                                sqlx::query_scalar("SELECT hash FROM documents WHERE id = ?")
                                    .bind(doc_id)
                                    .fetch_one(pool.inner())
                                    .await;
                            if let Ok(hash) = hash_res {
                                if let Ok(app_data_dir) = app.path().app_data_dir() {
                                    let preview_path =
                                        app_data_dir.join("previews").join(format!("{}.jpg", hash));
                                    if preview_path.exists() {
                                        if let Ok(bytes) = std::fs::read(preview_path) {
                                            use base64::Engine;
                                            let b64 = base64::engine::general_purpose::STANDARD
                                                .encode(bytes);
                                            visual_messages.push(json!({
                                                 "role": "user",
                                                 "content": [
                                                     { "type": "text", "text": "Visual Preview of attached PDF document:" },
                                                     { "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{}", b64) } }
                                                 ]
                                             }));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Always start with feedback
                let _ = tx
                    .send(Ok(ProviderEvent::Content(status_tag("type=\"thinking\""))))
                    .await;

                // 2. Manual Conversation Assembly
                let mut conversation: Vec<serde_json::Value> = Vec::new();

                // System Prompt
                let date = chrono::Local::now().format("%Y-%m-%d").to_string();
                conversation.push(json!({
                    "role": "system",
                    "content": format!("{}. Current Date: {}. {}. Answer the user request directly. Do NOT output internal thoughts, <think> tags, or simulate tool usage. If context is provided, rely on it.", persona_instructions, date, manual_context)
                }));

                // History
                for msg in &final_history {
                    conversation.push(json!({
                        "role": msg.role,
                        "content": msg.content
                    }));
                }

                // Visual Previews (Pre-inject if any)
                for vmsg in visual_messages {
                    conversation.push(vmsg);
                }

                // Final Prompt — use raw_content so multimodal images are preserved.
                // The llama_provider will re-parse JSON array content strings.
                conversation.push(json!({
                    "role": "user",
                    "content": raw_content.clone()
                }));

                // Stream directly using raw completion
                info!(
                    "[orchestrator] Starting Manual Mode stream for model: {}",
                    rig_clone.provider.model
                );

                match rig_clone
                    .provider
                    .stream_raw_completion(conversation, None)
                    .await
                {
                    Ok(mut stream) => {
                        info!("[orchestrator] Stream started successfully.");
                        while let Some(chunk) = futures::StreamExt::next(&mut stream).await {
                            let _ = tx.send(chunk).await;
                        }
                    }
                    Err(e) => {
                        error!("[orchestrator] Failed to start chat stream: {}", e);
                        let _ = tx.send(Err(format!("Chat Error: {}", e))).await;
                    }
                }
                return;
            }

            // ===================================================================
            // AUTO / TOOL MODE — Unified sandbox execution path
            // ===================================================================

            // Always use the sandbox path. A sandbox is always available because
            // `build_sandbox_unconditional` forces `sandbox_enabled = true`.
            let sandbox = sandbox
                .expect("[orchestrator] BUG: sandbox should always be Some when tools are enabled");

            Self::run_sandbox_loop(
                &tx,
                &rig_clone,
                &sandbox,
                &perms,
                has_mcp,
                &final_history,
                &all_doc_ids,
                &current_docs,
                &project_id_clone,
                &conversation_id_clone,
                &persona_instructions,
                &query,
                &raw_content,
            )
            .await;
        });

        Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
    }

    // -----------------------------------------------------------------------
    // Sandbox execution loop (unified tool execution via Rhai)
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    async fn run_sandbox_loop(
        tx: &mpsc::Sender<Result<crate::rig_lib::unified_provider::ProviderEvent, String>>,
        rig: &Arc<RigManager>,
        sandbox: &Sandbox,
        perms: &ToolPermissions,
        has_mcp: bool,
        final_history: &[crate::chat::Message],
        _all_doc_ids: &[String],
        _current_docs: &Option<Vec<crate::chat::AttachedDoc>>,
        _project_id: &Option<String>,
        _conversation_id: &Option<String>,
        persona_instructions: &str,
        query: &str,
        raw_content: &str,
    ) {
        use crate::rig_lib::unified_provider::ProviderEvent;

        // Detect if the current message contains images (multimodal).
        // When images are present the raw_content is a JSON array like:
        //   [{"type":"text","text":"..."}, {"type":"image_url","image_url":{...}}]
        let has_images = raw_content.trim().starts_with('[') && raw_content.contains("image_url");

        // In force_web_search mode the LLM must call web_search ONCE and then synthesise.
        // Giving it more turns causes it to keep refining / re-searching rather than answering.
        // In auto/optional mode we allow more turns for legitimate multi-step reasoning.
        // For vision (image) queries, limit to 1 turn — no tool loop needed.
        let max_turns = if has_images {
            1
        } else if perms.force_web_search {
            2
        } else {
            5
        };
        let mut current_turn = 0;
        let mut conversation: Vec<serde_json::Value> = Vec::new();

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();

        if has_images {
            // ── VISION MODE ──────────────────────────────────────────────
            // Use a compact system prompt without tool instructions.
            // The full tool prompt is ~800+ tokens of Rhai examples and
            // tool descriptions, which overwhelms small VLMs (4B) and
            // confuses them into thinking about tools instead of analyzing
            // the image.
            let system_prompt = format!(
                "{}.\nCurrent Date: {}\n\n\
                 You have vision capabilities. When the user shares images, analyze them \
                 thoroughly and respond to the user's request based on what you see. \
                 Provide detailed, helpful descriptions and observations. \
                 Be direct — describe what you observe, answer questions about the image, \
                 and do not ask the user questions unless you genuinely need clarification.",
                persona_instructions, date
            );

            conversation.push(json!({ "role": "system", "content": system_prompt }));

            // History
            for msg in final_history {
                conversation.push(json!({ "role": msg.role, "content": msg.content }));
            }

            // User message — pass raw multimodal content directly (no tool wrapping)
            if let Ok(parts) = serde_json::from_str::<Vec<serde_json::Value>>(raw_content) {
                info!(
                    "[orchestrator] Vision mode: passing {} multimodal parts directly",
                    parts.len()
                );
                conversation.push(json!({ "role": "user", "content": parts }));
            } else {
                conversation.push(json!({ "role": "user", "content": raw_content }));
            }

            info!(
                "[orchestrator] Vision mode — using simplified prompt, {} turn(s)",
                max_turns
            );
        } else {
            // ── TOOL / AUTO MODE ─────────────────────────────────────────

            // Build available tools description for the system prompt
            let mut tools_desc = String::from("AVAILABLE TOOLS (callable as Rhai functions):\n");

            // Remote MCP tool discovery/dispatch — only advertise when a server is configured.
            if has_mcp {
                tools_desc.push_str("- search_tools(query): Discover all available tools, including Host tools, Skills, and Remote MCP tools. Returns JSON with names, descriptions, and input schemas.\n");
                tools_desc.push_str("- mcp_call(tool_name, args_json): Call any discovered tool (Remote or Skill) by name. Args must be a JSON string. Returns JSON result.\n");
            }

            if perms.allow_web_search {
                tools_desc.push_str(
                    "- web_search(query): Direct internet search. Returns markdown string.\n",
                );
            }
            if perms.allow_file_search {
                tools_desc.push_str("- rag_search(query): Search codebase/docs. Returns string.\n");
                tools_desc.push_str("- read_file(path): Read file content.\n");
            }
            if perms.allow_image_gen {
                tools_desc.push_str(
                    "- generate_image(prompt): Generate an image from a text description.\n",
                );
            }

            // Skills — always available
            tools_desc.push_str("- run_skill(skill_id, args_json): Execute a skill/workflow by ID. Args must be a JSON string.\n");
            tools_desc.push_str("- save_skill(id, script, description): Save a new skill. Script must be valid Rhai code.\n");

            // Calculator — always available (no permissions gate needed)
            tools_desc.push_str("- calculator(expression): Evaluate mathematical expressions with full precision and show work step-by-step. Supports arithmetic (+, -, *, /, ^, %), parentheses, functions (sqrt, abs, round, ceil, floor, log, ln, log2, sin, cos, tan, asin, acos, atan, min, max, pow, exp), constants (pi, e, tau). Supports inline variables: 'x = 3; y = 5; 2*x^2 + y'. Use for ANY numbers — currency conversions, percentages, tips, compound interest.\n");
            tools_desc.push_str("- calculator_with_vars(expression, vars_json): Same as calculator but accepts named variables as JSON, e.g. calculator_with_vars(\"2*x + 1\", `{\"x\": 5}`) → 11.\n");

            // Build search rules. MCP-specific guidance is only included when a server is wired up,
            // so the LLM never reasons about tools that will always return an error.
            let search_rules = if perms.force_web_search {
                if has_mcp {
                    "CORE RULES:\n\
                 1. **ALWAYS SEARCH**: The user has explicitly enabled web search. You MUST use `web_search` for every query that could benefit from external information. Only skip search for pure greetings like 'Hello' or 'Hi'.\n\
                 2. **FORMALIZE QUERIES**: Transform vague user prompts into precise, professional search queries before calling `web_search`.\n\
                 3. For financial data, model info, or domain-specific queries, you may also use `mcp_call` with the appropriate tool after searching.\n\
                 4. If unsure which MCP tools exist, call `search_tools(\"\")` first to discover them."
                } else {
                    "CORE RULES:\n\
                 1. **ALWAYS SEARCH**: The user has explicitly enabled web search. You MUST use `web_search` for every query that could benefit from external information. Only skip search for pure greetings like 'Hello' or 'Hi'.\n\
                 2. **FORMALIZE QUERIES**: Transform vague user prompts into precise, professional search queries before calling `web_search`."
                }
            } else if has_mcp {
                "CORE RULES:\n\
             1. **REPLY DIRECTLY** for greetings, code, creative writing, general knowledge, opinions, or follow-up chat.\n\
             2. **USE TOOLS ONLY** when the user needs real-time information (today's news, live prices, current events), or explicitly asks you to search/look something up.\n\
             3. **IMAGE ANALYSIS**: If the user's message includes images, analyze and describe them directly. Do NOT use tools unless the user explicitly asks for additional external information (e.g. 'search the web for more info about this').\n\
             4. For financial data, model info, or domain-specific queries, use `mcp_call` with the appropriate tool.\n\
             5. If unsure which MCP tools exist, call `search_tools(\"\")` first to discover them.\n\
             6. When in doubt, reply directly. Only call a tool if you are confident the answer requires fresh external data."
            } else {
                "CORE RULES:\n\
             1. **REPLY DIRECTLY** for greetings, code, creative writing, general knowledge, opinions, or follow-up chat.\n\
             2. **USE TOOLS ONLY** when the user needs real-time information (today's news, live prices, current events), or explicitly asks you to search/look something up.\n\
             3. **IMAGE ANALYSIS**: If the user's message includes images, analyze and describe them directly. Do NOT use tools unless the user explicitly asks for additional external information (e.g. 'search the web for more info about this').\n\
             4. When in doubt, reply directly. Only call a tool if you are confident the answer requires fresh external data."
            };

            let system_prompt = format!(
                r#"{}.\nCurrent Date: {}

{}

TOOL USAGE (Code Execution Mode):
To use tools, write a Rhai script inside <rhai_code> tags.
The script has access to the tool functions listed below.
The LAST expression in the script becomes the result.

Example (simple web search):
<rhai_code>
let results = web_search("latest AI news");
results
</rhai_code>

Example (remote MCP tool):
<rhai_code>
let price = mcp_call("get_stock_price", `{{"symbol": "AAPL"}}`);
price
</rhai_code>

Example (multi-step with filtering):
<rhai_code>
let gold = mcp_call("get_stock_price", `{{"symbol": "GLD"}}`);
let silver = mcp_call("get_stock_price", `{{"symbol": "SLV"}}`);
let news = web_search("gold silver price today");
`Gold: ${{gold}}, Silver: ${{silver}}\n\nNews: ${{news}}`
</rhai_code>

After receiving <tool_result>, write your final answer to the user immediately.
**CRITICAL**: Do NOT call web_search or any other tool a second time. One tool call → synthesise → done.
If a script fails, the error message will appear in <tool_result>. Fix your script and try again ONE time only.

{}"#,
                persona_instructions, date, search_rules, tools_desc
            );

            conversation.push(json!({ "role": "system", "content": system_prompt }));

            // History
            for msg in final_history {
                conversation.push(json!({ "role": msg.role, "content": msg.content }));
            }
        } // end else (tool mode)

        // User query (only for non-vision mode — vision already pushed its message above)
        if !has_images {
            let effective_query = if perms.force_web_search {
                format!(
                    "**SEARCH MODE ACTIVE**: Call `web_search` ONCE with a well-formed query, \
                 then write your complete answer using the results. \
                 Do NOT call web_search more than once.\n\nRequest: {}",
                    query
                )
            } else if perms.allow_web_search {
                format!(
                    "Respond to this request. Only use tools if the request genuinely requires \
                 real-time or external data you don't have. Otherwise reply directly.\n\n\
                 Request: {}",
                    query
                )
            } else {
                query.to_string()
            };

            // Build the final user message. If the original content was multimodal
            // (images), combine effective_query text with the image parts so the VLM
            // can see both the instructions and the image data.
            let has_images_in_content = raw_content.trim().starts_with('[');
            if has_images_in_content {
                if let Ok(parts) = serde_json::from_str::<Vec<serde_json::Value>>(raw_content) {
                    let mut content_parts: Vec<serde_json::Value> = Vec::new();
                    // Tool instructions text first
                    content_parts.push(json!({ "type": "text", "text": effective_query }));
                    // Then all image parts from the original message
                    let mut img_count = 0;
                    for part in &parts {
                        if part["type"].as_str() == Some("image_url") {
                            content_parts.push(part.clone());
                            img_count += 1;
                        }
                    }
                    info!(
                        "[orchestrator] Built multimodal user message with {} image part(s)",
                        img_count
                    );
                    conversation.push(json!({ "role": "user", "content": content_parts }));
                } else {
                    info!(
                        "[orchestrator] Image content detected but failed to parse as JSON array"
                    );
                    conversation.push(json!({ "role": "user", "content": effective_query }));
                }
            } else {
                conversation.push(json!({ "role": "user", "content": effective_query }));
            }
        } // end if !has_images

        // Thinking status
        let _ = tx
            .send(Ok(ProviderEvent::Content(status_tag("type=\"thinking\""))))
            .await;

        // ReAct loop
        while current_turn < max_turns {
            if rig.is_cancelled() {
                let _ = tx
                    .send(Ok(ProviderEvent::Content("\n[Stopped]".into())))
                    .await;
                break;
            }
            current_turn += 1;
            eprintln!("[DEBUG ReAct] === Turn {}/{} ===", current_turn, max_turns);

            // Log conversation structure for debugging
            for (i, msg) in conversation.iter().enumerate() {
                let role = msg["role"].as_str().unwrap_or("?");
                let content_len = msg["content"].as_str().map_or(0, |s| s.len());
                eprintln!(
                    "[DEBUG ReAct]   msg[{}]: role={}, len={}",
                    i, role, content_len
                );
            }

            let mut full_response = String::new();
            let mut buffer = String::new();
            let mut code_detected = false;
            let is_last_turn = current_turn == max_turns;

            // On the last turn, force the LLM to synthesize rather than call tools again.
            // Without this, small models often re-emit <rhai_code> and the content gets
            // swallowed because there's no next turn for synthesis.
            // NOTE: We append to the last user message instead of adding a new one,
            // because Mistral's chat template enforces strict user/assistant alternation
            // and throws an exception on consecutive same-role messages.
            if is_last_turn && current_turn > 1 {
                if let Some(last_msg) = conversation.last_mut() {
                    if last_msg["role"] == "user" {
                        let existing = last_msg["content"].as_str().unwrap_or("").to_string();
                        last_msg["content"] = json!(format!(
                            "{}\n\nNow write your final answer to the user based on the information above. Do NOT use any tools or write any code blocks. Respond directly in natural language.",
                            existing
                        ));
                    }
                }
                info!(
                    "[orchestrator] Last turn — injected synthesis instruction into existing message"
                );
            }

            use futures::StreamExt;
            let total_conv_chars: usize = conversation
                .iter()
                .map(|m| m["content"].as_str().map_or(0, |s| s.len()))
                .sum();
            info!(
                "[orchestrator] Sending {} messages ({} total chars) to LLM for Turn {}",
                conversation.len(),
                total_conv_chars,
                current_turn
            );
            let mut stream = match rig
                .provider
                .stream_raw_completion(conversation.clone(), Some(0.1))
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(Err(format!("Provider Error: {}", e))).await;
                    break;
                }
            };

            while let Some(chunk_res) = stream.next().await {
                if rig.is_cancelled() {
                    let _ = tx
                        .send(Ok(ProviderEvent::Content("\n[Stopped]".into())))
                        .await;
                    return;
                }
                match chunk_res {
                    Ok(event) => match event {
                        ProviderEvent::Content(token) => {
                            full_response.push_str(&token);
                            buffer.push_str(&token);

                            if is_last_turn {
                                // On the last turn, forward EVERYTHING — even if the
                                // model tries to emit <rhai_code>, we won't execute it.
                                // Strip the code tags so only natural language reaches the user.
                                let clean =
                                    token.replace("<rhai_code>", "").replace("</rhai_code>", "");
                                if !clean.is_empty() {
                                    let _ = tx.send(Ok(ProviderEvent::Content(clean))).await;
                                }
                            } else if buffer.contains("<rhai_code>") {
                                code_detected = true;
                                if buffer.ends_with("<rhai_code>") {
                                    let _ = tx
                                        .send(Ok(ProviderEvent::Content(status_tag(
                                            "type=\"thinking\"",
                                        ))))
                                        .await;
                                }
                            } else if !code_detected {
                                let _ = tx.send(Ok(ProviderEvent::Content(token))).await;
                            }
                        }
                        ProviderEvent::Usage(u) => {
                            let _ = tx.send(Ok(ProviderEvent::Usage(u))).await;
                        }
                        ProviderEvent::ContextUpdate(c) => {
                            let _ = tx.send(Ok(ProviderEvent::ContextUpdate(c))).await;
                        }
                    },
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                    }
                }
            }

            info!(
                "[orchestrator] Turn {} complete: code_detected={}, response_len={}, first_80_chars={:?}",
                current_turn,
                code_detected,
                full_response.len(),
                &full_response[..std::cmp::min(80, full_response.len())]
            );

            if !code_detected {
                info!(
                    "[orchestrator] Turn {} — LLM answered directly (no code). Breaking loop.",
                    current_turn
                );
                break; // LLM answered directly without code
            }

            // On the last turn, we already forwarded all content and won't execute code.
            if is_last_turn {
                info!(
                    "[orchestrator] Turn {} — last turn reached. Breaking loop (content already forwarded).",
                    current_turn
                );
                break;
            }

            // Parse <rhai_code> block
            let mut code_executed = false;
            if let Some(start) = full_response.find("<rhai_code>") {
                if let Some(end) = full_response.find("</rhai_code>") {
                    let script = full_response[start + 11..end].trim();

                    // Add assistant response to history
                    conversation.push(json!({ "role": "assistant", "content": full_response }));

                    // Execute in sandbox
                    info!(
                        "[orchestrator] Executing Rhai script ({} chars)",
                        script.len()
                    );

                    let _ = tx
                        .send(Ok(ProviderEvent::Content(status_tag(
                            "type=\"tool_call\" query=\"Executing script...\"",
                        ))))
                        .await;

                    match sandbox.execute(script) {
                        Ok(result) => {
                            eprintln!(
                                "[DEBUG sandbox] Output {} chars. First 300: {:?}",
                                result.output.len(),
                                &result.output[..std::cmp::min(300, result.output.len())]
                            );
                            info!(
                                "[orchestrator] Script executed in {}ms",
                                result.execution_time_ms
                            );

                            // Summarize output to prevent context overflow.
                            // IMPORTANT: DDGSearchTool already performs its own Map-Reduce
                            // summarization, so we use a generous limit (8000 chars) here
                            // to avoid double-truncating the carefully curated data.
                            let output_val: serde_json::Value =
                                serde_json::from_str(&result.output)
                                    .unwrap_or(serde_json::Value::String(result.output.clone()));

                            let summarized_val =
                                crate::rig_lib::tool_router::summarize_arbitrary_json(
                                    output_val, 8000, 20,
                                );

                            let summarized_output = match summarized_val {
                                serde_json::Value::String(s) => s,
                                _ => serde_json::to_string(&summarized_val).unwrap_or_default(),
                            };

                            info!(
                                "[orchestrator] Tool result for LLM: {} chars, preview: {:?}",
                                summarized_output.len(),
                                &summarized_output[..std::cmp::min(500, summarized_output.len())]
                            );

                            conversation.push(json!({
                                "role": "user",
                                "content": format!("<tool_result>\n{}\n</tool_result>", summarized_output)
                            }));
                            eprintln!(
                                "[DEBUG tool_result] Injected {} chars into conversation. Total messages: {}",
                                summarized_output.len(),
                                conversation.len()
                            );
                            code_executed = true;
                            info!(
                                "[orchestrator] Tool result injected. Conversation now has {} messages.",
                                conversation.len()
                            );
                        }
                        Err(e) => {
                            warn!("[orchestrator] Script error: {}", e);
                            let feedback = e.to_llm_feedback();
                            conversation.push(json!({
                                "role": "user",
                                "content": format!("<tool_result>\n{}\n</tool_result>", feedback)
                            }));
                            code_executed = true; // Let the LLM retry
                        }
                    }
                }
            }

            if !code_executed {
                break;
            }
        } // End sandbox loop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // extract_text_from_content
    // -----------------------------------------------------------------------

    #[test]
    fn extract_text_plain_string() {
        assert_eq!(
            extract_text_from_content("Hello, how are you?"),
            "Hello, how are you?"
        );
    }

    #[test]
    fn extract_text_from_multimodal_content() {
        let content = r#"[{"type":"text","text":"What is this?"},{"type":"image_url","image_url":{"url":"data:image/png;base64,abc123"}}]"#;
        assert_eq!(extract_text_from_content(content), "What is this?");
    }

    #[test]
    fn extract_text_multiple_text_parts() {
        let content = r#"[{"type":"text","text":"First part"},{"type":"image_url","image_url":{"url":"data:image/png;base64,abc"}},{"type":"text","text":"Second part"}]"#;
        assert_eq!(extract_text_from_content(content), "First part Second part");
    }

    #[test]
    fn extract_text_image_only_no_text() {
        let content =
            r#"[{"type":"image_url","image_url":{"url":"data:image/png;base64,abc123"}}]"#;
        // No text parts → empty string
        assert_eq!(extract_text_from_content(content), "");
    }

    #[test]
    fn extract_text_malformed_json_fallback() {
        // Starts with '[' but isn't valid JSON → returns original string
        let content = "[not valid json";
        assert_eq!(extract_text_from_content(content), "[not valid json");
    }

    #[test]
    fn extract_text_non_json_bracket_string() {
        // A string starting with '[' that is a valid JSON array but not
        // multimodal content format
        let content = "[1, 2, 3]";
        // Valid JSON array of numbers — no "type":"text" parts → empty
        assert_eq!(extract_text_from_content(content), "");
    }

    // -----------------------------------------------------------------------
    // Multimodal message construction (auto mode)
    // -----------------------------------------------------------------------

    #[test]
    fn auto_mode_builds_multimodal_user_message() {
        // Simulate what run_sandbox_loop does when images are present
        let raw_content = r#"[{"type":"text","text":"Describe this"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]"#;
        let effective_query = "Respond to this request. Only use tools if genuinely needed.\n\nRequest: Describe this";

        let has_images = raw_content.trim().starts_with('[');
        assert!(has_images);

        let parts: Vec<serde_json::Value> =
            serde_json::from_str(raw_content).expect("should parse");
        let mut content_parts: Vec<serde_json::Value> = Vec::new();
        content_parts.push(json!({ "type": "text", "text": effective_query }));
        for part in &parts {
            if part["type"].as_str() == Some("image_url") {
                content_parts.push(part.clone());
            }
        }

        let msg = json!({ "role": "user", "content": content_parts });

        // Verify structure
        let content = msg["content"].as_array().expect("should be array");
        assert_eq!(content.len(), 2); // text + image
        assert_eq!(content[0]["type"], "text");
        assert!(content[0]["text"]
            .as_str()
            .unwrap()
            .contains("Respond to this request"));
        assert_eq!(content[1]["type"], "image_url");
        assert!(content[1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image"));
    }

    #[test]
    fn auto_mode_text_only_is_plain_string() {
        // When no images, the message should be a plain string
        let raw_content = "What is the weather today?";
        let effective_query = "Respond to this request.\n\nRequest: What is the weather today?";

        let has_images = raw_content.trim().starts_with('[');
        assert!(!has_images);

        let msg = json!({ "role": "user", "content": effective_query });
        assert!(msg["content"].is_string());
    }
}
