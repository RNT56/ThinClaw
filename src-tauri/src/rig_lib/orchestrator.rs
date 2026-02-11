use crate::rig_lib::RigManager;
use scrappy_mcp_tools::events::{StatusReporter, ToolEvent};
use scrappy_mcp_tools::sandbox::Sandbox;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Tool Permissions (unchanged)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ToolPermissions {
    pub allow_web_search: bool,
    pub allow_file_search: bool,
    pub allow_image_gen: bool,
}

// ---------------------------------------------------------------------------
// MCP Configuration
// ---------------------------------------------------------------------------

/// Configuration for the optional remote MCP server connection.
/// When `None`, the orchestrator operates in legacy mode (no sandbox).
#[derive(Clone, Debug, Default)]
pub struct McpOrchestratorConfig {
    /// Base URL of the FastAPI MCP server (e.g. "https://api.scrappy.dev")
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
            } => {
                format!(
                    "\n<scrappy_status type=\"tool_call\" name=\"{}\" query=\"{}\" status=\"{}\" />\n",
                    tool_name, input_summary, status
                )
            }
            ToolEvent::Status { msg, .. } => {
                format!("\n<scrappy_status type=\"thinking\" msg=\"{}\" />\n", msg)
            }
            ToolEvent::Progress {
                percentage,
                message,
            } => {
                format!(
                    "\n<scrappy_status type=\"progress\" pct=\"{:.0}\" msg=\"{}\" />\n",
                    percentage, message
                )
            }
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
    pub fn new(rig: Arc<RigManager>) -> Self {
        Self {
            rig,
            mcp_config: McpOrchestratorConfig::default(),
        }
    }

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
        let query = last_msg.content.clone();
        let history_clone = messages; // Remaining are history

        let project_id_clone = project_id.clone();
        let perms = permissions.clone();
        let persona_instructions = persona_instructions.clone();

        // Pass current turn docs to context collection
        let current_docs = last_msg.attached_docs.clone();
        let conversation_id_clone = conversation_id.clone(); // Clone for spawn

        // Build sandbox (if configured)
        let sandbox = self.build_sandbox(&tx);

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

            let token_count_res = rig_clone.provider.count_tokens(check_history.clone()).await;

            let mut should_summarize = false;

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

                    if token_count > (max_context as f32 * threshold) as u32 {
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
                    .send(Ok(ProviderEvent::Content(
                        "\n<scrappy_status type=\"summarizing\" />\n".into(),
                    )))
                    .await;

                // Identify chunk to summarize
                let messages_to_keep = final_history.len() / 2;
                let split_idx = final_history.len().saturating_sub(messages_to_keep);

                if split_idx > 0 {
                    let chunk_to_summarize = final_history.drain(0..split_idx).collect::<Vec<_>>();

                    // Prepare summarization prompt
                    let summary_prompt = format!(
                             "Summarize the following conversation history into a concise paragraph. Capture key decisions, user preferences, and important context. \n\nHISTORY:\n{}",
                             chunk_to_summarize.iter().map(|m| format!("{}: {}", m.role, m.content)).collect::<Vec<_>>().join("\n\n")
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
                            role: "system".into(), // Or "user" with special marker, but "system" is safer for context injection
                            content: format!("Previous conversation summary: {}", summary_text),
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
                        warn!("[orchestrator] Summarization failed (empty response). History truncated regardless to save context.");
                        // Even if summary failed, we already drained the history, so we implicitly truncated it.
                        // This is desired behavior if we are OOM.
                        let _ = tx
                            .send(Ok(ProviderEvent::ContextUpdate(final_history.clone())))
                            .await;
                    }
                }
            }

            info!("[orchestrator] Proceeding to tool/manual decision...");

            // 1. Context & Document Collection (Used for both Manual and Lead turns)
            let mut all_doc_ids = Vec::new();
            let mut all_doc_names = Vec::new();

            // ... (Rest of logic uses `final_history` instead of `history_clone`)

            // Collect docs from history
            for msg in &final_history {
                if let Some(docs) = &msg.attached_docs {
                    for d in docs {
                        if !all_doc_ids.contains(&d.id) {
                            all_doc_ids.push(d.id.clone());
                            all_doc_names.push(d.name.clone());
                        }
                    }
                }
            }

            // Collect docs from CURRENT message
            if let Some(docs) = &current_docs {
                for d in docs {
                    if !all_doc_ids.contains(&d.id) {
                        all_doc_ids.push(d.id.clone());
                        all_doc_names.push(d.name.clone());
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
                    let _ = tx.send(Ok(ProviderEvent::Content("\n<scrappy_status type=\"rag_search\" query=\"Retrieving context...\" />\n".into()))).await;

                    if let Some(app) = &rig_clone.app_handle {
                        use tauri::Manager;
                        let sidecar = app.state::<crate::sidecar::SidecarManager>();
                        let pool = app.state::<sqlx::SqlitePool>();
                        let store = app.state::<crate::vector_store::VectorStore>();
                        let reranker = app.state::<crate::reranker::RerankerWrapper>();

                        // 1. Text RAG
                        let context_res = crate::rag::retrieve_context_internal(
                            Some(app.clone()),
                            sidecar.inner(),
                            pool.inner().clone(),
                            store.inner().clone(),
                            reranker.inner(),
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
                        // ... (Keeping existing visual logic)
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
                    .send(Ok(ProviderEvent::Content(
                        "\n<scrappy_status type=\"thinking\" />\n".into(),
                    )))
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

                // Final Prompt
                conversation.push(json!({
                    "role": "user",
                    "content": query.clone()
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
            // AUTO / TOOL MODE
            // ===================================================================

            // --- SANDBOX MODE (new) ---
            // If sandbox is available, we use a different execution strategy:
            // 1. Emit <rhai_code> blocks instead of <tool_code>
            // 2. Execute them in the sandboxed Rhai engine
            // 3. Feed results back to the LLM
            if let Some(sandbox) = sandbox {
                Self::run_sandbox_loop(
                    &tx,
                    &rig_clone,
                    &sandbox,
                    &perms,
                    &final_history,
                    &all_doc_ids,
                    &current_docs,
                    &project_id_clone,
                    &conversation_id_clone,
                    &persona_instructions,
                    &query,
                )
                .await;
                return;
            }

            // --- LEGACY TOOL MODE (existing <tool_code> parsing) ---
            Self::run_legacy_tool_loop(
                &tx,
                &rig_clone,
                &perms,
                &final_history,
                &all_doc_ids,
                &all_doc_names,
                &current_docs,
                &project_id_clone,
                &conversation_id_clone,
                &persona_instructions,
                &query,
            )
            .await;
        });

        Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
    }

    // -----------------------------------------------------------------------
    // Sandbox execution loop (new MCP code-execution mode)
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    async fn run_sandbox_loop(
        tx: &mpsc::Sender<Result<crate::rig_lib::unified_provider::ProviderEvent, String>>,
        rig: &Arc<RigManager>,
        sandbox: &Sandbox,
        perms: &ToolPermissions,
        final_history: &[crate::chat::Message],
        _all_doc_ids: &[String],
        _current_docs: &Option<Vec<crate::chat::AttachedDoc>>,
        _project_id: &Option<String>,
        _conversation_id: &Option<String>,
        persona_instructions: &str,
        query: &str,
    ) {
        use crate::rig_lib::unified_provider::ProviderEvent;

        let max_turns = 5;
        let mut current_turn = 0;
        let mut conversation: Vec<serde_json::Value> = Vec::new();

        // Build available tools description for the system prompt
        let mut tools_desc = String::from("AVAILABLE TOOLS (callable as Rhai functions):\n");
        // Unified Discovery
        tools_desc.push_str("- search_tools(query): Discover all available tools, including Host tools, Skills, and Remote MCP tools. Returns JSON with names, descriptions, and input schemas.\n");
        tools_desc.push_str("- mcp_call(tool_name, args_json): Call any discovered tool (Remote or Skill) by name. Args must be a JSON string. Returns JSON result.\n");

        if perms.allow_web_search {
            tools_desc.push_str(
                "- web_search(query): Direct internet search. Returns markdown string.\n",
            );
        }
        if perms.allow_file_search {
            tools_desc.push_str("- rag_search(query): Search codebase/docs. Returns string.\n");
            tools_desc.push_str("- read_file(path): Read file content.\n");
        }

        // Dedicated Skills
        tools_desc.push_str("- run_skill(skill_id, args_json): Execute a skill/workflow by ID. Args must be a JSON string.\n");
        tools_desc.push_str("- save_skill(id, script, description): Save a new skill. Script must be valid Rhai code.\n");

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();

        let system_prompt = format!(
            r#"{}
Current Date: {}

CORE RULES:
1. ALWAYS use tools for factual queries.
2. If the request is purely creative or conversational, answer directly.
3. If uncertainty exists about any fact, specific entity, or current event, use `web_search`.
4. For financial data, model info, or domain-specific queries, use `mcp_call` with the appropriate tool.
5. If unsure which MCP tools exist, call `search_tools("")` first to discover them.

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

After receiving <tool_result>, use the data to synthesize a helpful answer for the user.
If a script fails, the error message will appear in <tool_result>. Fix your script and try again.

{}"#,
            persona_instructions, date, tools_desc
        );

        conversation.push(json!({ "role": "system", "content": system_prompt }));

        // History
        for msg in final_history {
            conversation.push(json!({ "role": msg.role, "content": msg.content }));
        }

        // User query
        let effective_query = if perms.allow_web_search {
            format!(
                "**INSTRUCTION**: Check if this request requires external research.\n\
                 - If asking for facts, news, or specific data -> Call `web_search`.\n\
                 - If greeting, chatting, or asking for code/logic -> Answer directly.\n\n\
                 Request: {}",
                query
            )
        } else {
            query.to_string()
        };

        conversation.push(json!({ "role": "user", "content": effective_query }));

        // Thinking status
        let _ = tx
            .send(Ok(ProviderEvent::Content(
                "\n<scrappy_status type=\"thinking\" />\n".into(),
            )))
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

            let mut full_response = String::new();
            let mut buffer = String::new();
            let mut code_detected = false;

            use futures::StreamExt;
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

                            if buffer.contains("<rhai_code>") {
                                code_detected = true;
                                if buffer.ends_with("<rhai_code>") {
                                    let _ = tx
                                        .send(Ok(ProviderEvent::Content(
                                            "\n<scrappy_status type=\"thinking\" />\n".into(),
                                        )))
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

            if !code_detected {
                break; // LLM answered directly without code
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
                        .send(Ok(ProviderEvent::Content(
                            "\n<scrappy_status type=\"tool_call\" query=\"Executing script...\" />\n"
                                .into(),
                        )))
                        .await;

                    match sandbox.execute(script) {
                        Ok(result) => {
                            info!(
                                "[orchestrator] Script executed in {}ms",
                                result.execution_time_ms
                            );

                            // Summarize output to prevent context overflow
                            let output_val: serde_json::Value =
                                serde_json::from_str(&result.output)
                                    .unwrap_or(serde_json::Value::String(result.output.clone()));

                            let summarized_val =
                                crate::rig_lib::tool_router::summarize_arbitrary_json(
                                    output_val, 2000, 20,
                                );

                            let summarized_output = match summarized_val {
                                serde_json::Value::String(s) => s,
                                _ => serde_json::to_string(&summarized_val).unwrap_or_default(),
                            };

                            conversation.push(json!({
                                "role": "user",
                                "content": format!("<tool_result>\n{}\n</tool_result>", summarized_output)
                            }));
                            code_executed = true;
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

    // -----------------------------------------------------------------------
    // Legacy tool loop (existing <tool_code> parsing — unchanged logic)
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    async fn run_legacy_tool_loop(
        tx: &mpsc::Sender<Result<crate::rig_lib::unified_provider::ProviderEvent, String>>,
        rig: &Arc<RigManager>,
        perms: &ToolPermissions,
        final_history: &[crate::chat::Message],
        all_doc_ids: &[String],
        _all_doc_names: &[String],
        _current_docs: &Option<Vec<crate::chat::AttachedDoc>>,
        project_id: &Option<String>,
        conversation_id: &Option<String>,
        persona_instructions: &str,
        query: &str,
    ) {
        use crate::rig_lib::unified_provider::ProviderEvent;

        let max_turns = 5;
        let mut current_turn = 0;
        let mut conversation: Vec<serde_json::Value> = Vec::new();

        // 1. Dynamic System Prompt
        let mut tools_desc = String::from("AVAILABLE TOOLS:\n");
        if perms.allow_web_search {
            tools_desc.push_str("- web_search(query: str): Search internet for real-time info.\n");
        }
        if perms.allow_file_search {
            tools_desc.push_str("- rag_search(query: str): Search project documents/codebase.\n");
            tools_desc.push_str(
                "- read_file(path: str, force_ocr: bool?): Read file content. Set force_ocr to true only if standard text extraction is failing or garbage.\n",
            );
        }
        if perms.allow_image_gen {
            tools_desc.push_str("- generate_image(prompt: str): Generate an image.\n");
        }

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut grounding_rules = String::new();
        if perms.allow_web_search {
            grounding_rules.push_str("\n**RESEARCH RULES**:\n1. **Analyze Request**: Decide if the user needs external information or just conversation.\n2. **Search for Facts**: Use `web_search` for news, data, and specific entities.\n3. **Chat Directly**: For greetings, creative writing, or general questions, answer directly without tools.\n");
        }
        let system_prompt = format!(
            r#"{}. 
Current Date: {}

CORE RULES:
1. ALWAYS use tools for factual queries.
2. If the request is purely creative or conversational, answer directly.
3. If uncertainty exists about any fact, specific entity, or current event, use `web_search`.

TOOL USAGE:
To use a tool, output valid JSON inside <tool_code> tags.
Example:
<tool_code>
{{
  "name": "web_search",
  "arguments": {{ "query": "..." }}
}}
</tool_code>

{}"#,
            persona_instructions, date, tools_desc
        );

        conversation.push(json!({
            "role": "system",
            "content": system_prompt
        }));

        // 2. History Conversion
        for msg in final_history {
            conversation.push(json!({
                "role": msg.role,
                "content": msg.content
            }));
        }

        // 3. Current User Query and Context Collection (Resolving Paths for Tools)
        let mut doc_info = Vec::new();
        let mut visual_messages = Vec::new();
        if !all_doc_ids.is_empty() {
            if let Some(app) = &rig.app_handle {
                use tauri::Manager;
                let pool = app.state::<sqlx::SqlitePool>();
                // Build dynamic IN query
                let placeholders = all_doc_ids
                    .iter()
                    .map(|_| "?")
                    .collect::<Vec<_>>()
                    .join(",");
                let query_str = format!(
                    "SELECT id, path, hash FROM documents WHERE id IN ({})",
                    placeholders
                );
                let mut db_query = sqlx::query_as::<_, (String, String, String)>(&query_str);
                for id in all_doc_ids {
                    db_query = db_query.bind(id);
                }

                match db_query.fetch_all(pool.inner()).await {
                    Ok(docs) => {
                        for (_id, path, hash) in docs {
                            let name = std::path::Path::new(&path)
                                .file_name()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| "unknown_file".to_string());
                            doc_info.push(format!("{} (at {})", name, path));

                            // 1. Check if the file itself is an image
                            let path_lower = path.to_lowercase();
                            let is_direct_image = path_lower.ends_with(".png")
                                || path_lower.ends_with(".jpg")
                                || path_lower.ends_with(".jpeg")
                                || path_lower.ends_with(".webp");

                            let mut image_injected = false;
                            if is_direct_image {
                                if let Ok(bytes) = std::fs::read(&path) {
                                    use base64::Engine;
                                    let b64 =
                                        base64::engine::general_purpose::STANDARD.encode(bytes);
                                    let mime = if path_lower.ends_with(".png") {
                                        "image/png"
                                    } else {
                                        "image/jpeg"
                                    };
                                    visual_messages.push(json!({
                                          "role": "user",
                                          "content": [
                                              { "type": "text", "text": format!("Attached Image ({}):", path) },
                                              { "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", mime, b64) } }
                                          ]
                                      }));
                                    image_injected = true;
                                }
                            }

                            if !image_injected {
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
                                                     { "type": "text", "text": format!("Visual Preview of attached document ({}):", path) },
                                                     { "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{}", b64) } }
                                                 ]
                                             }));
                                        }
                                    }
                                }
                            }

                            // 3. Auto-Injection (simplified)
                            if !path_lower.ends_with(".pdf") && !is_direct_image {
                                if let Ok(content) = std::fs::read_to_string(&path) {
                                    if content.len() < 12000 {
                                        doc_info.push(format!(
                                            "\n[Direct Content of {}]:\n{}\n",
                                            name, content
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => eprintln!("[orchestrator] Error resolving doc paths: {}", e),
                }
            }
        }

        let mut effective_query = query.to_string();

        if let Some(pid) = project_id {
            let mut context_str = format!("Project Context ID: {}\n", pid);
            if perms.allow_file_search {
                if let Some(app) = &rig.app_handle {
                    use tauri::Manager;
                    let pool = app.state::<sqlx::SqlitePool>();
                    let files = crate::rag::list_project_files(pool.inner(), pid).await;
                    if !files.is_empty() {
                        let list = if files.len() > 50 {
                            let subset = files[..50].join("\n- ");
                            format!("- {}\n... ({} more files)", subset, files.len() - 50)
                        } else {
                            files.join("\n- ")
                        };
                        context_str
                            .push_str(&format!("\n[AVAILABLE PROJECT FILES]:\n- {}\n", list));
                    }
                }
            }
            effective_query = format!("{}\nRequest: {}", context_str, query);
        }

        if !doc_info.is_empty() {
            effective_query = format!(
                "[CURRENT CHAT ATTACHMENTS]:\n{}\n\n{}",
                doc_info.join("\n"),
                effective_query
            );
        }

        // Inject Visual Previews
        for vmsg in visual_messages {
            conversation.push(vmsg);
        }

        // Start turn with a strong grounding injection if searching is allowed
        let final_query = if perms.allow_web_search {
            format!(
                "**INSTRUCTION**: Check if the user's request requires external knowledge.\n\
                 - If it's a Greeting, Code execution, or General Chat -> Reply directly.\n\
                 - If it's about News, Facts, or Specific Data -> Use `web_search`.\n\n\
                 Request: {}",
                effective_query
            )
        } else {
            effective_query
        };

        conversation.push(json!({
            "role": "user",
            "content": final_query
        }));

        // 4. ReAct Loop
        let mut _final_answer_streaming = false;
        let _ = tx
            .send(Ok(ProviderEvent::Content(
                "\n<scrappy_status type=\"thinking\" />\n".into(),
            )))
            .await;

        while current_turn < max_turns {
            if rig.is_cancelled() {
                let _ = tx
                    .send(Ok(ProviderEvent::Content("\n[Stopped]".into())))
                    .await;
                break;
            }
            current_turn += 1;
            let mut full_response = String::new();
            let mut buffer = String::new();
            let mut tool_detected = false;

            use futures::StreamExt;
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

                            if buffer.contains("<tool_code>") {
                                tool_detected = true;
                                if buffer.ends_with("<tool_code>") {
                                    let _ = tx
                                        .send(Ok(ProviderEvent::Content(
                                            "\n<scrappy_status type=\"thinking\" />\n".into(),
                                        )))
                                        .await;
                                }
                            } else {
                                if !tool_detected {
                                    let _ = tx.send(Ok(ProviderEvent::Content(token))).await;
                                    _final_answer_streaming = true;
                                }
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

            if !tool_detected {
                break;
            }

            // Parse Tool
            let mut tool_executed = false;
            if let Some(start) = full_response.find("<tool_code>") {
                if let Some(end) = full_response.find("</tool_code>") {
                    let json_str = &full_response[start + 11..end].trim();
                    let json_str = if json_str.starts_with("```json") {
                        json_str
                            .trim_start_matches("```json")
                            .trim_end_matches("```")
                            .trim()
                    } else if json_str.starts_with("```") {
                        json_str
                            .trim_start_matches("```")
                            .trim_end_matches("```")
                            .trim()
                    } else {
                        json_str
                    };

                    let tool_call = match serde_json::from_str::<serde_json::Value>(json_str) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("Failed to parse tool JSON: {} in Turn {}", e, current_turn);
                            if !_final_answer_streaming {
                                let _ = tx
                                    .send(Ok(ProviderEvent::Content(
                                        "\n[Tool Parse Error - Proceeding with answer]\n".into(),
                                    )))
                                    .await;
                            }
                            break;
                        }
                    };

                    conversation.push(json!({
                        "role": "assistant",
                        "content": full_response
                    }));

                    // Tool execution
                    let name = tool_call["name"].as_str().unwrap_or("");
                    let args = tool_call["arguments"].clone();
                    let allowed_web = perms.allow_web_search;
                    let allowed_file = perms.allow_file_search;
                    let allowed_img = perms.allow_image_gen;
                    let result = match name {
                        "web_search" if allowed_web => {
                            let q = args["query"].as_str().unwrap_or("");
                            let _ = tx
                                .send(Ok(ProviderEvent::Content(
                                    format!(
                                        "\n<scrappy_status type=\"web_search\" query=\"{}\" />\n",
                                        q
                                    )
                                    .into(),
                                )))
                                .await;
                            rig.explicit_search(q).await
                        }
                        "rag_search" if allowed_file => {
                            let q = args["query"].as_str().unwrap_or("");
                            let _ = tx
                                .send(Ok(ProviderEvent::Content(
                                    format!(
                                        "\n<scrappy_status type=\"rag_search\" query=\"{}\" />\n",
                                        q
                                    )
                                    .into(),
                                )))
                                .await;
                            if let Some(app) = &rig.app_handle {
                                use tauri::Manager;
                                let context_res = crate::rag::retrieve_context_internal(
                                    rig.app_handle.clone(),
                                    app.state::<crate::sidecar::SidecarManager>().inner(),
                                    app.state::<sqlx::SqlitePool>().inner().clone(),
                                    app.state::<crate::vector_store::VectorStore>()
                                        .inner()
                                        .clone(),
                                    app.state::<crate::reranker::RerankerWrapper>().inner(),
                                    q.to_string(),
                                    conversation_id.clone(),
                                    if all_doc_ids.is_empty() {
                                        None
                                    } else {
                                        Some(all_doc_ids.to_vec())
                                    },
                                    project_id.clone(),
                                )
                                .await;
                                match context_res {
                                    Ok(r) => r.join("\n\n"),
                                    Err(e) => format!("Error: {}", e),
                                }
                            } else {
                                "App state missing".into()
                            }
                        }
                        "read_file" if allowed_file => {
                            let path = args["path"].as_str().unwrap_or("");
                            let _ = tx.send(Ok(ProviderEvent::Content(format!("\n<scrappy_status type=\"tool_call\" query=\"Reading {}\" />\n", path).into()))).await;
                            if std::path::Path::new(path).exists() {
                                if let Ok(c) = std::fs::read_to_string(path) {
                                    if c.len() > 20000 {
                                        format!("{}... (truncated)", &c[..20000])
                                    } else {
                                        c
                                    }
                                } else {
                                    "Read failed".into()
                                }
                            } else {
                                "File not found".into()
                            }
                        }
                        "generate_image" if allowed_img => {
                            let _ = tx
                                .send(Ok(ProviderEvent::Content(
                                    "\n<scrappy_status type=\"image_gen\" />\n".into(),
                                )))
                                .await;
                            "Image Generation Triggered".to_string()
                        }
                        _ => "Unknown tool or permission denied".to_string(),
                    };

                    conversation.push(json!({
                        "role": "user",
                        "content": format!("<tool_result>\n{}\n</tool_result>", result)
                    }));
                    tool_executed = true;
                }
            }

            if !tool_executed {
                break;
            }
        } // End Loop
    }
}
