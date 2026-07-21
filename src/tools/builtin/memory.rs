//! Memory tools for persistent workspace memory.
//!
//! These tools allow the agent to:
//! - Search past memories, decisions, and context
//! - Read and write files in the workspace
//!
//! # Usage
//!
//! The agent should use `memory_search` before answering questions about
//! prior work, decisions, dates, people, preferences, or todos.
//!
//! Use `memory_write` to persist important facts that should be remembered
//! across sessions.

use std::sync::Arc;

use crate::agent::session_search::{SessionSearchRender, SessionSearchService};
use crate::context::JobContext;
use crate::db::Database;
use crate::llm::LlmProvider;
use crate::tools::tool::{Tool, ToolError, ToolMetadata, ToolOutput, ToolRouteIntent, require_str};
use crate::workspace::{AuthorizedWorkspace, SearchConfig, Workspace, paths};
use async_trait::async_trait;
use thinclaw_tools::builtin::memory as memory_policy;
use thinclaw_tools::ports::{
    MemoryToolHostPort, ToolHostError, ToolMemoryActionRequest, ToolMemoryActionResult,
    ToolMemoryEntry, ToolMemoryReadRequest, ToolMemoryScope, ToolMemorySearchRequest,
    ToolMemoryWriteRequest, ToolOperationScope, job_context_from_tool_scope,
};

fn workspace_for_ctx(base: &Arc<Workspace>, ctx: &JobContext) -> Workspace {
    let agent_workspace_id =
        memory_policy::workspace_agent_id_from_metadata(&ctx.metadata, base.agent_id());
    base.scoped_clone(ctx.user_id.clone(), agent_workspace_id)
}

fn identity_for_ctx(ctx: &JobContext) -> Result<crate::identity::ResolvedIdentity, ToolError> {
    let actor_id = ctx.owner_actor_id().to_string();
    let conversation_kind = ctx
        .metadata
        .get("conversation_kind")
        .and_then(|value| value.as_str())
        .and_then(crate::identity::parse_conversation_kind_hint)
        .unwrap_or(crate::identity::ConversationKind::Direct);
    let explicit_scope = ctx
        .metadata
        .get("conversation_scope_id")
        .and_then(|value| value.as_str())
        .and_then(|value| uuid::Uuid::parse_str(value).ok());
    let stable_external_conversation_key = ctx
        .metadata
        .get("stable_external_conversation_key")
        .and_then(|value| value.as_str());

    crate::identity::resolved_identity_from_carried_context(
        &ctx.principal_id,
        &actor_id,
        conversation_kind,
        explicit_scope,
        stable_external_conversation_key,
    )
    .map_err(|error| ToolError::NotAuthorized(error.to_string()))
}

fn authorized_workspace_for_ctx(
    base: &Arc<Workspace>,
    ctx: &JobContext,
) -> Result<AuthorizedWorkspace, ToolError> {
    let raw = workspace_for_ctx(base, ctx);
    let identity = identity_for_ctx(ctx)?;
    let channel = ctx
        .metadata
        .get("channel")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    Ok(AuthorizedWorkspace::conversation(&raw, &identity, channel))
}

pub struct RootMemoryToolHost {
    workspace: Arc<Workspace>,
    orchestrator: Option<Arc<crate::agent::learning::LearningOrchestrator>>,
    sse_sender: Option<tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>>,
}

impl RootMemoryToolHost {
    pub fn new(
        workspace: Arc<Workspace>,
        orchestrator: Option<Arc<crate::agent::learning::LearningOrchestrator>>,
        sse_sender: Option<tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>>,
    ) -> Self {
        Self {
            workspace,
            orchestrator,
            sse_sender,
        }
    }
}

pub fn root_memory_tool_host(
    workspace: Arc<Workspace>,
    orchestrator: Option<Arc<crate::agent::learning::LearningOrchestrator>>,
    sse_sender: Option<tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>>,
) -> Arc<dyn MemoryToolHostPort> {
    Arc::new(RootMemoryToolHost::new(workspace, orchestrator, sse_sender))
}

fn tool_host_error_from_tool(error: ToolError) -> ToolHostError {
    match error {
        ToolError::InvalidParameters(reason) => ToolHostError::InvalidRequest { reason },
        ToolError::NotAuthorized(reason) => ToolHostError::PermissionDenied { reason },
        ToolError::ExternalService(service) => ToolHostError::Unavailable { service },
        other => ToolHostError::OperationFailed {
            reason: other.to_string(),
        },
    }
}

#[async_trait]
impl MemoryToolHostPort for RootMemoryToolHost {
    async fn read_memory(
        &self,
        _request: ToolMemoryReadRequest,
    ) -> Result<ToolMemoryEntry, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "memory_read_structured".to_string(),
        })
    }

    async fn write_memory(
        &self,
        _request: ToolMemoryWriteRequest,
    ) -> Result<ToolMemoryEntry, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "memory_write_structured".to_string(),
        })
    }

    async fn search_memory(
        &self,
        _request: ToolMemorySearchRequest,
    ) -> Result<Vec<ToolMemoryEntry>, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "memory_search_structured".to_string(),
        })
    }

    async fn delete_memory(
        &self,
        _scope: ToolOperationScope,
        _path: String,
        _memory_scope: ToolMemoryScope,
    ) -> Result<(), ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "memory_delete_structured".to_string(),
        })
    }

    async fn search_memory_action(
        &self,
        request: ToolMemoryActionRequest,
    ) -> Result<ToolMemoryActionResult, ToolHostError> {
        let ctx = job_context_from_tool_scope(request.scope, "memory_search");
        let tool = MemorySearchTool::new(Arc::clone(&self.workspace));
        let output = tool
            .execute(request.params, &ctx)
            .await
            .map_err(tool_host_error_from_tool)?;
        Ok(ToolMemoryActionResult {
            output: output.result,
        })
    }

    async fn write_memory_action(
        &self,
        request: ToolMemoryActionRequest,
    ) -> Result<ToolMemoryActionResult, ToolHostError> {
        let ctx = job_context_from_tool_scope(request.scope, "memory_write");
        let tool = MemoryWriteTool::new(Arc::clone(&self.workspace), self.orchestrator.clone());
        let output = tool
            .execute(request.params, &ctx)
            .await
            .map_err(tool_host_error_from_tool)?;
        Ok(ToolMemoryActionResult {
            output: output.result,
        })
    }

    async fn read_memory_action(
        &self,
        request: ToolMemoryActionRequest,
    ) -> Result<ToolMemoryActionResult, ToolHostError> {
        let ctx = job_context_from_tool_scope(request.scope, "memory_read");
        let tool = MemoryReadTool::new(Arc::clone(&self.workspace));
        let output = tool
            .execute(request.params, &ctx)
            .await
            .map_err(tool_host_error_from_tool)?;
        Ok(ToolMemoryActionResult {
            output: output.result,
        })
    }

    async fn tree_memory_action(
        &self,
        request: ToolMemoryActionRequest,
    ) -> Result<ToolMemoryActionResult, ToolHostError> {
        let ctx = job_context_from_tool_scope(request.scope, "memory_tree");
        let tool = MemoryTreeTool::new(Arc::clone(&self.workspace));
        let output = tool
            .execute(request.params, &ctx)
            .await
            .map_err(tool_host_error_from_tool)?;
        Ok(ToolMemoryActionResult {
            output: output.result,
        })
    }

    async fn delete_memory_action(
        &self,
        request: ToolMemoryActionRequest,
    ) -> Result<ToolMemoryActionResult, ToolHostError> {
        let ctx = job_context_from_tool_scope(request.scope, "memory_delete");
        let mut tool = MemoryDeleteTool::new(Arc::clone(&self.workspace));
        if let Some(sender) = self.sse_sender.clone() {
            tool = tool.with_sse_sender(sender);
        }
        let output = tool
            .execute(request.params, &ctx)
            .await
            .map_err(tool_host_error_from_tool)?;
        Ok(ToolMemoryActionResult {
            output: output.result,
        })
    }
}

/// Tool for searching workspace memory.
///
/// Performs hybrid BM25 + vector semantic search over MEMORY.md and daily logs.
/// Applies MMR re-ranking and temporal decay by default for better result quality.
pub struct MemorySearchTool {
    workspace: Arc<Workspace>,
}

impl MemorySearchTool {
    /// Create a new memory search tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search past memories, decisions, and context. MUST be called before answering \
         questions about prior work, decisions, dates, people, preferences, or todos. \
         Returns relevant snippets with relevance scores and source paths. \
         Results are MMR-diversified (no near-duplicate daily notes) and recency-weighted."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        memory_policy::memory_search_parameters_schema()
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata::authoritative(ToolRouteIntent::MemoryRecall)
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let workspace = authorized_workspace_for_ctx(&self.workspace, ctx)?;

        let parsed = memory_policy::parse_memory_search_params(&params)?;
        let query = parsed.query.as_str();

        // MMR re-ranking on by default — reduces near-duplicate daily notes.
        // Lambda 0.7 = slight relevance bias (matches openclaw recommendation).
        let use_mmr = parsed.use_mmr;

        // Temporal decay on by default — 30-day half-life so older notes don't
        // crowd out recent ones on equal semantic similarity.
        let use_decay = parsed.use_temporal_decay;

        let mut config = SearchConfig::default().with_limit(parsed.limit);
        if use_mmr {
            config = config.with_mmr(0.7);
        }
        if use_decay {
            // 30-day half-life: today = 1.0×, 1 month ago = 0.5×, 3 months ago = 0.125×
            config = config.with_temporal_decay(30.0);
        }

        let results = workspace
            .search(query, config)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Search failed: {}", e)))?;

        let output = memory_policy::memory_search_output(
            query,
            results
                .iter()
                .map(|result| {
                    memory_policy::memory_search_result_entry(
                        &result.path,
                        &result.content,
                        result.score.into(),
                        result.document_id.to_string(),
                        result.is_hybrid(),
                    )
                })
                .collect(),
        );

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true // Durable memory may contain user-supplied or recalled untrusted text.
    }
}

/// Tool for searching DB-backed conversation transcripts.
///
/// This is intentionally transcript-only: it queries conversation history from
/// the database, not workspace documents or memory files.
pub struct SessionSearchTool {
    store: Arc<dyn Database>,
    service: SessionSearchService,
}

impl SessionSearchTool {
    /// Create a new session search tool.
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self {
            store,
            service: SessionSearchService::new(),
        }
    }

    /// Configure an optional summarizer model for transcript condensation.
    pub fn with_summarizer(mut self, summarizer: Arc<dyn LlmProvider>) -> Self {
        self.service = self.service.with_summarizer(summarizer);
        self
    }

    async fn recent_conversation_metadata(
        &self,
        principal_id: &str,
        actor_id: &str,
        channel: Option<&str>,
        exact_conversation_id: Option<uuid::Uuid>,
        direct_scope: bool,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, ToolError> {
        let recent = if direct_scope && channel.is_none() {
            self.store
                .list_actor_conversations_for_recall(principal_id, actor_id, false, limit as i64)
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("Transcript listing failed: {}", e))
                })?
        } else {
            let Some(channel) = channel else {
                return Ok(Vec::new());
            };
            let recent = self
                .store
                .list_conversations_with_preview(principal_id, channel, limit as i64)
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("Transcript listing failed: {}", e))
                })?;
            if direct_scope {
                recent
                    .into_iter()
                    .filter(|conversation| {
                        conversation.conversation_kind == crate::history::ConversationKind::Direct
                            && match conversation
                                .actor_id
                                .as_deref()
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                            {
                                Some(conversation_actor_id) => conversation_actor_id == actor_id,
                                None => actor_id == principal_id,
                            }
                    })
                    .collect()
            } else if let Some(exact_conversation_id) = exact_conversation_id {
                recent
                    .into_iter()
                    .filter(|conversation| conversation.id == exact_conversation_id)
                    .collect()
            } else {
                Vec::new()
            }
        };
        Ok(recent
            .into_iter()
            .map(|conversation| {
                memory_policy::session_metadata_entry(
                    conversation.id,
                    &conversation.user_id,
                    conversation.actor_id.as_deref(),
                    &conversation.channel,
                    conversation.conversation_kind.as_str(),
                    conversation.title.as_deref(),
                    conversation.message_count,
                    conversation.started_at.to_rfc3339(),
                    conversation.last_activity.to_rfc3339(),
                    conversation.thread_type.as_deref(),
                    conversation.handoff,
                    conversation.stable_external_conversation_key.as_deref(),
                )
            })
            .collect())
    }
}

#[async_trait]
impl Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn description(&self) -> &str {
        "Search DB-backed conversation transcripts for prior messages, decisions, and workflow history. \
         Use before answering questions about prior work or repeated conversations. \
         In direct chats this follows the actor's linked history across channels by default. \
         This searches conversation history only, not workspace documents."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        memory_policy::session_search_parameters_schema()
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata::authoritative(ToolRouteIntent::TranscriptHistory)
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let resolved_identity = identity_for_ctx(ctx)?;
        let direct_scope =
            resolved_identity.conversation_kind == crate::identity::ConversationKind::Direct;
        let exact_group_conversation_id = if direct_scope {
            None
        } else {
            let conversation_id = ctx.conversation_id.ok_or_else(|| {
                ToolError::NotAuthorized(
                    "group transcript recall requires the canonical conversation id".to_string(),
                )
            })?;
            let belongs = self
                .store
                .conversation_belongs_to_identity(
                    conversation_id,
                    &resolved_identity.principal_id,
                    &resolved_identity.actor_id,
                    resolved_identity.conversation_scope_id,
                    crate::history::ConversationKind::Group,
                )
                .await
                .map_err(|error| {
                    ToolError::ExecutionFailed(format!(
                        "Group transcript authorization failed: {error}"
                    ))
                })?;
            if !belongs {
                return Err(ToolError::NotAuthorized(
                    "the current identity does not own the requested group transcript".to_string(),
                ));
            }
            Some(conversation_id)
        };
        let parsed = memory_policy::parse_session_search_params_for_context(
            &params,
            ctx,
            self.service.summarizer_configured(),
        )?;
        let query = parsed.query.as_str();

        let scope = memory_policy::resolve_session_search_scope_for_context(ctx);
        let filters = memory_policy::session_search_filters_for_context(ctx, &parsed);

        if query.trim().is_empty() {
            let recent = self
                .recent_conversation_metadata(
                    &scope.principal_id,
                    &scope.actor_id,
                    filters.channel.as_deref(),
                    exact_group_conversation_id,
                    direct_scope,
                    parsed.limit,
                )
                .await?;
            let output = memory_policy::session_recent_output(query, recent);
            return Ok(ToolOutput::success(output, start.elapsed()));
        }

        let mut hits = self
            .store
            .search_conversation_messages(
                &scope.principal_id,
                query,
                direct_scope.then_some(scope.actor_id.as_str()),
                filters.channel.as_deref(),
                filters.thread_id.as_deref(),
                parsed.limit as i64,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Transcript search failed: {}", e)))?;
        if let Some(exact_conversation_id) = exact_group_conversation_id {
            hits.retain(|hit| hit.conversation_id == exact_conversation_id);
        }
        let SessionSearchRender {
            results,
            summarized,
            fallback,
        } = self
            .service
            .render_results(&self.store, query, hits, parsed.summarize_sessions)
            .await;

        let output = memory_policy::session_search_output(query, results, summarized, fallback);

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }
}

/// Tool for writing to workspace memory.
///
/// Use this to persist important information that should be remembered
/// across sessions: decisions, preferences, facts, lessons learned.
pub struct MemoryWriteTool {
    workspace: Arc<Workspace>,
    orchestrator: Option<Arc<crate::agent::learning::LearningOrchestrator>>,
}

impl MemoryWriteTool {
    /// Create a new memory write tool.
    pub fn new(
        workspace: Arc<Workspace>,
        orchestrator: Option<Arc<crate::agent::learning::LearningOrchestrator>>,
    ) -> Self {
        Self {
            workspace,
            orchestrator,
        }
    }
}

#[async_trait]
impl Tool for MemoryWriteTool {
    fn name(&self) -> &str {
        "memory_write"
    }

    fn description(&self) -> &str {
        "Write to persistent memory (database-backed, NOT the local filesystem). \
         Use for facts, decisions, preferences, or lessons to remember across sessions. \
         Targets: 'memory' (MEMORY.md, long-term facts), 'daily_log' (timestamped notes), \
         'heartbeat' (HEARTBEAT.md checklist), and 'IDENTITY.md'. \
         For SOUL.md / SOUL.local.md / AGENTS.md / USER.md use prompt_manage instead of memory_write. \
         or a custom path. In direct DMs, memory/user/profile writes default to the actor overlay. \
         Principal-shared knowledge is read-only from conversation tools and must be curated \
         through an explicitly authorized operator surface. \
         ALWAYS write well-structured markdown: use ## headers for sections, bullet points, \
         and clear prose. Never dump raw unformatted text into identity files."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        memory_policy::memory_write_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let workspace = authorized_workspace_for_ctx(&self.workspace, ctx)?;

        let parsed = memory_policy::parse_memory_write_params(&params)?;
        let content = parsed.content.as_str();
        let target = parsed.target.as_str();
        let append = parsed.append;

        let write_plan = memory_policy::resolve_memory_write_plan_for_context(ctx, target);
        let path = write_plan.path.as_str();

        // IDENTITY.md policy is crate-owned; root only performs the selected write.
        match write_plan.file_policy {
            memory_policy::MemoryWriteFilePolicy::AppendOnlyIdentity => {
                if !append {
                    return Err(ToolError::NotAuthorized(format!(
                        "'{}' is append-only. Add an '## Update' section with your changes \
                         instead of overwriting. To fully restructure SOUL.md / SOUL.local.md / \
                         AGENTS.md / USER.md, use prompt_manage instead.",
                        target,
                    )));
                }
                workspace
                    .append(path, content)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                let output = memory_policy::memory_write_output(
                    "appended",
                    path,
                    true,
                    content.len(),
                    Some("Identity file updated (append-only)"),
                );
                return Ok(ToolOutput::success(output, start.elapsed()));
            }
            memory_policy::MemoryWriteFilePolicy::PromptManagedIdentity => {
                return Err(ToolError::NotAuthorized(format!(
                    "'{}' must be managed through prompt_manage (bounded prompt mutation).",
                    target
                )));
            }
            memory_policy::MemoryWriteFilePolicy::FreelyRewritableIdentity => {
                if append {
                    workspace
                        .append(path, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                } else {
                    workspace
                        .write(path, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                }
                let output = memory_policy::memory_write_output(
                    if append { "appended" } else { "rewritten" },
                    path,
                    append,
                    content.len(),
                    Some(if append {
                        "Personality file updated (new section appended)"
                    } else {
                        "Personality file fully restructured — well-formed markdown expected"
                    }),
                );
                return Ok(ToolOutput::success(output, start.elapsed()));
            }
            memory_policy::MemoryWriteFilePolicy::Regular => {}
        }

        // Resolve the mirror identity before mutating the canonical workspace.
        // The previous post-write `?` could report failure after the durable
        // write had already succeeded, inviting a retry and duplicate append.
        let mirror = if let Some(orchestrator) = self.orchestrator.as_ref() {
            let channel = ctx
                .metadata
                .get("channel")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let access = identity_for_ctx(ctx)?.access_context(channel);
            let payload = memory_policy::memory_write_mirror_payload(path, append, content);
            Some((Arc::clone(orchestrator), access, payload))
        } else {
            None
        };

        let path = match write_plan.target_kind {
            memory_policy::MemoryWriteTargetKind::Memory => {
                if path.eq_ignore_ascii_case(paths::MEMORY) {
                    let doc = if append {
                        workspace
                            .append_with_separator(paths::MEMORY, content, "\n\n")
                            .await
                            .map_err(|e| {
                                ToolError::ExecutionFailed(format!("Write failed: {}", e))
                            })?
                    } else {
                        workspace.write(paths::MEMORY, content).await.map_err(|e| {
                            ToolError::ExecutionFailed(format!("Write failed: {}", e))
                        })?
                    };
                    doc.path
                } else if append {
                    let doc = workspace
                        .append_with_separator(path, content, "\n\n")
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                    doc.path
                } else {
                    workspace
                        .write(path, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?
                        .path
                }
            }
            memory_policy::MemoryWriteTargetKind::DailyLog => workspace
                .append_daily_log(content)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?,
            memory_policy::MemoryWriteTargetKind::Heartbeat => {
                if append {
                    workspace
                        .append_with_separator(path, content, "\n")
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?
                        .path
                } else {
                    workspace
                        .write(path, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?
                        .path
                }
            }
            memory_policy::MemoryWriteTargetKind::Other => {
                if append {
                    workspace
                        .append_with_separator(path, content, "\n\n")
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?
                        .path
                } else {
                    workspace
                        .write(path, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?
                        .path
                }
            }
        };

        let output =
            memory_policy::memory_write_output("written", &path, append, content.len(), None);

        if let Some((orchestrator, access, mut payload)) = mirror {
            // The persisted document can normalize/redirect the requested
            // path, so report its authoritative path to the provider mirror.
            if let Some(object) = payload.as_object_mut() {
                object.insert("path".to_string(), serde_json::json!(path));
            }
            if tokio::time::timeout(
                std::time::Duration::from_secs(10),
                orchestrator.mirror_workspace_write(&access, &payload),
            )
            .await
            .is_err()
            {
                tracing::warn!(path = %path, "Learning-provider workspace mirror timed out");
            }
        }

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(20, 200))
    }
}

/// Tool for reading workspace files, with optional line-range slicing.
///
/// Degrades gracefully when the target file doesn't exist — returns empty
/// content with `"exists": false` instead of an error. This matches openclaw's
/// `memory_get` behaviour so agents can safely probe today's daily log before
/// the first write without wrapping the call in try/catch logic.
pub struct MemoryReadTool {
    workspace: Arc<Workspace>,
}

impl MemoryReadTool {
    /// Create a new memory read tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for MemoryReadTool {
    fn name(&self) -> &str {
        "memory_read"
    }

    fn description(&self) -> &str {
        "Read a durable memory file inside the current actor or conversation namespace. Canonical prompt files such as SOUL.md and AGENTS.md are runtime-managed and are not exposed through this tool. Returns empty content (not an error) if an authorized file does not exist yet."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        memory_policy::memory_read_parameters_schema()
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata::authoritative(ToolRouteIntent::MemoryRecall)
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let workspace = authorized_workspace_for_ctx(&self.workspace, ctx)?;

        let path = require_str(&params, "path")?;
        let read_slice = memory_policy::parse_memory_read_slice(&params);

        // Graceful degradation: missing file → empty content, not an error.
        // Matches openclaw memory_get: { text: "", path } on ENOENT.
        let doc = match workspace.read(path).await {
            Ok(doc) => doc,
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => {
                return Ok(ToolOutput::success(
                    memory_policy::memory_read_missing_output(path),
                    start.elapsed(),
                ));
            }
            Err(e) => return Err(ToolError::ExecutionFailed(format!("Read failed: {}", e))),
        };

        // Optional line-range slicing.
        let (content, total_lines) =
            memory_policy::apply_memory_read_slice(&doc.content, read_slice);
        let output = memory_policy::memory_read_output(
            &doc.path,
            &content,
            total_lines,
            Some(doc.updated_at.to_rfc3339()),
        );

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }
}

/// Tool for viewing workspace structure as a tree.
///
/// Returns a hierarchical view of files and directories with configurable depth.
pub struct MemoryTreeTool {
    workspace: Arc<Workspace>,
}

impl MemoryTreeTool {
    /// Create a new memory tree tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }

    /// Recursively build tree structure.
    ///
    /// Returns a compact format where directories end with `/` and may have children.
    async fn build_tree(
        &self,
        workspace: &AuthorizedWorkspace,
        path: &str,
        current_depth: usize,
        max_depth: usize,
    ) -> Result<Vec<serde_json::Value>, ToolError> {
        if current_depth > max_depth {
            return Ok(Vec::new());
        }

        let entries = workspace
            .list(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Tree failed: {}", e)))?;

        let mut result = Vec::new();
        for entry in entries {
            // Directories end with `/`, files don't
            let display_path = if entry.is_directory {
                format!("{}/", entry.name())
            } else {
                entry.name().to_string()
            };

            if entry.is_directory && current_depth < max_depth {
                let children =
                    Box::pin(self.build_tree(workspace, &entry.path, current_depth + 1, max_depth))
                        .await?;
                if children.is_empty() {
                    result.push(serde_json::Value::String(display_path));
                } else {
                    result.push(serde_json::json!({ display_path: children }));
                }
            } else {
                result.push(serde_json::Value::String(display_path));
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl Tool for MemoryTreeTool {
    fn name(&self) -> &str {
        "memory_tree"
    }

    fn description(&self) -> &str {
        "View the workspace memory structure as a tree (database-backed storage). \
         Use memory_read to read files shown here, NOT read_file. \
         The workspace is separate from the local filesystem."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        memory_policy::memory_tree_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let workspace = authorized_workspace_for_ctx(&self.workspace, ctx)?;

        let parsed = memory_policy::parse_memory_tree_params(&params);

        let tree = self
            .build_tree(&workspace, parsed.path.as_str(), 1, parsed.depth)
            .await?;

        // Compact output: just the tree array
        Ok(ToolOutput::success(
            serde_json::Value::Array(tree),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }
}

/// Tool for deleting a file from workspace memory.
///
/// Use this to clean up temporary files like BOOTSTRAP.md after setup,
/// or remove outdated notes. Identity files are protected.
pub struct MemoryDeleteTool {
    workspace: Arc<Workspace>,
    /// Optional SSE sender for broadcasting lifecycle events (e.g. BootstrapCompleted).
    sse_sender: Option<tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>>,
}

impl MemoryDeleteTool {
    /// Create a new memory delete tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self {
            workspace,
            sse_sender: None,
        }
    }

    /// Attach an SSE sender to enable lifecycle event emission.
    pub fn with_sse_sender(
        mut self,
        sender: tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>,
    ) -> Self {
        self.sse_sender = Some(sender);
        self
    }
}

#[async_trait]
impl Tool for MemoryDeleteTool {
    fn name(&self) -> &str {
        "memory_delete"
    }

    fn description(&self) -> &str {
        "Delete a file from workspace memory (database-backed storage). \
         Cannot delete IDENTITY.md (append to it instead). \
         SOUL.md / SOUL.local.md / AGENTS.md / USER.md can be fully rewritten with prompt_manage \
         rather than deleted. \
         Primary use-case: memory_delete('BOOTSTRAP.md') after the identity ritual completes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        memory_policy::memory_delete_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let workspace = authorized_workspace_for_ctx(&self.workspace, ctx)?;

        let path = require_str(&params, "path")?;

        let delete_action = memory_policy::resolve_memory_delete_action(path)?;
        let is_bootstrap = matches!(
            delete_action,
            memory_policy::MemoryDeleteAction::FinalizeBootstrap
        );
        if is_bootstrap {
            let is_direct = memory_policy::memory_conversation_kind(&ctx.metadata)
                == memory_policy::MemoryConversationKind::Direct;
            let is_principal_admin = ctx
                .metadata
                .get("principal_admin")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if !is_direct || !is_principal_admin {
                return Err(ToolError::NotAuthorized(
                    "BOOTSTRAP.md can only be finalized from an authenticated principal-admin direct session"
                        .to_string(),
                ));
            }
            let principal_workspace = workspace_for_ctx(&self.workspace, ctx);
            // Keep a non-empty completion sentinel so workspace seeding does not
            // recreate BOOTSTRAP.md on next startup.
            principal_workspace
                .write(
                    crate::workspace::paths::BOOTSTRAP,
                    "<!-- bootstrap completed -->",
                )
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!(
                        "Failed to finalize BOOTSTRAP.md completion: {}",
                        e
                    ))
                })?;

            // The pre-authentication startup flow uses the reserved `default`
            // workspace, while the authenticated conversation uses the resolved
            // principal workspace. Finalize both with the same idempotent sentinel
            // so a restart cannot re-enter onboarding through the legacy scope.
            // A mirror failure is surfaced (rather than logged as success), making
            // retry safe and ensuring callers never observe a false completion.
            if principal_workspace.user_id() != "default" {
                principal_workspace
                    .scoped_clone("default", principal_workspace.agent_id())
                    .write(
                        crate::workspace::paths::BOOTSTRAP,
                        "<!-- bootstrap completed -->",
                    )
                    .await
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!(
                            "Failed to finalize default BOOTSTRAP.md completion: {}",
                            e
                        ))
                    })?;
            }
        } else {
            workspace
                .delete(path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Delete failed: {}", e)))?;
        }

        // If BOOTSTRAP.md was completed, notify the bridge to update frontend state.
        if is_bootstrap && let Some(ref tx) = self.sse_sender {
            let _ = tx.send(crate::channels::web::types::SseEvent::BootstrapCompleted);
            tracing::info!("[memory_delete] Emitted BootstrapCompleted SSE event");
        }

        Ok(ToolOutput::success(
            memory_policy::memory_delete_output(path),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;

    fn make_test_workspace() -> Arc<Workspace> {
        Arc::new(Workspace::new(
            "test_user",
            deadpool_postgres::Pool::builder(deadpool_postgres::Manager::new(
                tokio_postgres::Config::new(),
                tokio_postgres::NoTls,
            ))
            .build()
            .unwrap(),
        ))
    }

    #[test]
    fn test_memory_search_schema() {
        let workspace = make_test_workspace();
        let tool = MemorySearchTool::new(workspace);

        assert_eq!(tool.name(), "memory_search");
        assert!(tool.requires_sanitization());

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["query"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&"query".into())
        );
    }

    #[test]
    fn test_memory_write_schema() {
        let workspace = make_test_workspace();
        let tool = MemoryWriteTool::new(workspace, None);

        assert_eq!(tool.name(), "memory_write");

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["content"].is_object());
        assert!(schema["properties"]["target"].is_object());
        assert!(schema["properties"]["append"].is_object());
    }

    #[test]
    fn test_memory_read_schema() {
        let workspace = make_test_workspace();
        let tool = MemoryReadTool::new(workspace);

        assert_eq!(tool.name(), "memory_read");

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&"path".into())
        );
    }

    #[test]
    fn test_memory_tree_schema() {
        let workspace = make_test_workspace();
        let tool = MemoryTreeTool::new(workspace);

        assert_eq!(tool.name(), "memory_tree");

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["depth"].is_object());
        assert_eq!(schema["properties"]["depth"]["default"], 1);
    }

    #[test]
    fn test_memory_write_routes_authoritative_job_actor_to_overlay() {
        let mut ctx = JobContext::with_user_and_actor("default", "actor-123", "chat", "test");
        ctx.metadata = serde_json::json!({
            "conversation_kind": "direct",
            "actor_id": "spoofed-actor",
        });

        let (path, is_actor_scoped) =
            memory_policy::resolve_memory_write_path_for_context(&ctx, "memory");
        assert!(is_actor_scoped);
        assert_eq!(path, "actors/actor-123/MEMORY.md");
    }

    #[test]
    fn test_memory_write_shared_prefix_names_the_shared_namespace() {
        let mut ctx = JobContext::with_user("default", "chat", "test");
        ctx.metadata = serde_json::json!({
            "conversation_kind": "direct",
            "actor_id": "actor-123",
        });

        let (path, is_actor_scoped) =
            memory_policy::resolve_memory_write_path_for_context(&ctx, "shared:memory");
        assert!(!is_actor_scoped);
        assert_eq!(path, "shared/MEMORY.md");
    }

    #[test]
    fn test_memory_write_uses_job_actor_without_metadata_actor_id() {
        let mut ctx = JobContext::with_user_and_actor("default", "actor-456", "chat", "test");
        ctx.metadata = serde_json::json!({
            "conversation_kind": "direct"
        });

        let (path, is_actor_scoped) =
            memory_policy::resolve_memory_write_path_for_context(&ctx, "profile");
        assert!(is_actor_scoped);
        assert_eq!(path, "actors/actor-456/context/profile.json");
    }

    #[test]
    fn test_group_identity_without_canonical_scope_fails_closed() {
        let mut ctx = JobContext::with_user_and_actor("household", "actor-456", "chat", "test");
        ctx.metadata = serde_json::json!({
            "conversation_kind": "group"
        });

        let error = identity_for_ctx(&ctx).expect_err("group scope must be required");
        assert!(matches!(error, ToolError::NotAuthorized(_)));
    }
}

#[cfg(all(test, feature = "libsql"))]
mod session_search_smoke_tests {
    use std::sync::Arc;

    use super::*;
    use crate::context::JobContext;
    use crate::tools::Tool;

    fn make_ctx() -> JobContext {
        let mut ctx = JobContext::with_user("user-1", "chat", "session-search-test");
        ctx.metadata = serde_json::json!({
            "channel": "repl",
            "thread_id": "thread-1",
            "conversation_kind": "direct",
        });
        ctx
    }

    #[tokio::test]
    async fn session_search_smoke_without_summarizer_returns_raw_results() {
        let (db, _guard) = crate::testing::test_db().await;
        let conversation_id = db
            .create_conversation("repl", "user-1", Some("thread-1"))
            .await
            .expect("create conversation");
        db.add_conversation_message(conversation_id, "user", "build error after deploy")
            .await
            .expect("insert transcript message");

        let tool = SessionSearchTool::new(Arc::clone(&db));
        let output = tool
            .execute(
                serde_json::json!({
                    "query": "build error",
                    "summarize_sessions": true
                }),
                &make_ctx(),
            )
            .await
            .expect("session_search should succeed");

        assert_eq!(output.result["summarized"], serde_json::json!(false));
        assert!(output.result.get("fallback").is_none());
        assert!(
            output.result["results"]
                .as_array()
                .and_then(|items| items.first())
                .and_then(|entry| entry.get("message_id"))
                .is_some()
        );
    }

    #[tokio::test]
    async fn session_search_smoke_with_summarizer_returns_summaries() {
        let (db, _guard) = crate::testing::test_db().await;
        let conversation_id = db
            .create_conversation("repl", "user-1", Some("thread-1"))
            .await
            .expect("create conversation");
        db.add_conversation_message(
            conversation_id,
            "assistant",
            "Build failed, then fixed after config rollback.",
        )
        .await
        .expect("insert transcript message");

        let summarizer = Arc::new(crate::testing::StubLlm::new("summary bullet"));
        let tool = SessionSearchTool::new(Arc::clone(&db))
            .with_summarizer(Arc::clone(&summarizer) as Arc<dyn crate::llm::LlmProvider>);
        let output = tool
            .execute(
                serde_json::json!({
                    "query": "failed fixed",
                    "summarize_sessions": true
                }),
                &make_ctx(),
            )
            .await
            .expect("session_search should succeed");

        assert_eq!(output.result["summarized"], serde_json::json!(true));
        assert!(output.result.get("fallback").is_none());
        assert!(
            output.result["results"]
                .as_array()
                .and_then(|items| items.first())
                .and_then(|entry| entry.get("summary"))
                .is_some()
        );
        assert!(summarizer.calls() >= 1);
    }

    #[tokio::test]
    async fn bootstrap_completion_also_finalizes_default_workspace() {
        let (db, _guard) = crate::testing::test_db().await;
        let base_workspace = Arc::new(Workspace::new_with_db("default", Arc::clone(&db)));
        base_workspace
            .seed_if_empty(Some("ThinClaw"), Some("balanced"))
            .await
            .expect("seed default workspace");

        let user_workspace = base_workspace.scoped_clone("user-42", None);
        user_workspace
            .seed_if_empty(Some("ThinClaw"), Some("balanced"))
            .await
            .expect("seed user workspace");

        let tool = MemoryDeleteTool::new(Arc::clone(&base_workspace));
        let mut ctx = JobContext::with_user("user-42", "chat", "bootstrap-finish");
        ctx.metadata = serde_json::json!({
            "conversation_kind": "direct",
            "principal_admin": true,
            "channel": "test",
        });
        tool.execute(
            serde_json::json!({
                "path": crate::workspace::paths::BOOTSTRAP
            }),
            &ctx,
        )
        .await
        .expect("memory_delete should finalize bootstrap");

        let user_bootstrap = user_workspace
            .read(crate::workspace::paths::BOOTSTRAP)
            .await
            .expect("user bootstrap should still exist as sentinel");
        let default_bootstrap = base_workspace
            .read(crate::workspace::paths::BOOTSTRAP)
            .await
            .expect("default bootstrap should still exist as sentinel");

        assert_eq!(user_bootstrap.content, "<!-- bootstrap completed -->");
        assert_eq!(default_bootstrap.content, "<!-- bootstrap completed -->");
    }

    #[tokio::test]
    async fn session_search_direct_defaults_to_cross_channel_actor_history() {
        let (db, _guard) = crate::testing::test_db().await;

        let telegram_conversation = db
            .create_conversation("telegram", "user-1", Some("tg-thread"))
            .await
            .expect("create telegram conversation");
        db.update_conversation_identity(
            telegram_conversation,
            Some("user-1"),
            Some("user-1"),
            Some(crate::identity::scope_id_from_key("principal:user-1")),
            crate::history::ConversationKind::Direct,
            Some("telegram://direct/user-1"),
        )
        .await
        .expect("set telegram identity");
        db.add_conversation_message(telegram_conversation, "user", "telegram ping from mobile")
            .await
            .expect("insert telegram message");

        let gateway_conversation = db
            .create_conversation("gateway", "user-1", Some("gateway-thread"))
            .await
            .expect("create gateway conversation");
        db.update_conversation_identity(
            gateway_conversation,
            Some("user-1"),
            Some("user-1"),
            Some(crate::identity::scope_id_from_key("principal:user-1")),
            crate::history::ConversationKind::Direct,
            Some("gateway://direct/user-1/actor/user-1/thread/gateway-thread"),
        )
        .await
        .expect("set gateway identity");
        db.add_conversation_message(gateway_conversation, "user", "local web note")
            .await
            .expect("insert gateway message");

        let mut ctx = make_ctx();
        ctx.metadata = serde_json::json!({
            "channel": "gateway",
            "thread_id": "gateway-thread",
            "conversation_kind": "direct",
        });

        let tool = SessionSearchTool::new(Arc::clone(&db));
        let output = tool
            .execute(
                serde_json::json!({
                    "query": "telegram ping"
                }),
                &ctx,
            )
            .await
            .expect("session_search should succeed");

        let first = output.result["results"]
            .as_array()
            .and_then(|items| items.first())
            .expect("expected at least one result");
        assert_eq!(first["channel"], serde_json::json!("telegram"));
    }

    #[tokio::test]
    async fn session_search_group_is_bound_to_the_exact_conversation() {
        let (db, _guard) = crate::testing::test_db().await;
        let scope_a = crate::identity::scope_id_from_key("discord:guild:room-a");
        let scope_b = crate::identity::scope_id_from_key("discord:guild:room-b");
        let conversation_a = db
            .create_conversation("discord", "house", Some("room-a"))
            .await
            .unwrap();
        let conversation_b = db
            .create_conversation("discord", "house", Some("room-b"))
            .await
            .unwrap();
        for (conversation_id, scope_id, key) in [
            (conversation_a, scope_a, "discord://guild/room-a"),
            (conversation_b, scope_b, "discord://guild/room-b"),
        ] {
            db.update_conversation_identity(
                conversation_id,
                Some("house"),
                Some("alice"),
                Some(scope_id),
                crate::history::ConversationKind::Group,
                Some(key),
            )
            .await
            .unwrap();
        }
        db.add_conversation_message(conversation_a, "user", "shared needle allowed room")
            .await
            .unwrap();
        db.add_conversation_message(conversation_b, "user", "shared needle private room")
            .await
            .unwrap();

        let mut ctx = JobContext::with_identity("house", "alice", "chat", "group search");
        ctx.conversation_id = Some(conversation_a);
        ctx.metadata = serde_json::json!({
            "channel": "discord",
            "thread_id": "room-a",
            "conversation_kind": "group",
            "conversation_scope_id": scope_a.to_string(),
            "stable_external_conversation_key": "discord://guild/room-a"
        });
        let tool = SessionSearchTool::new(Arc::clone(&db));

        let searched = tool
            .execute(serde_json::json!({"query": "shared needle"}), &ctx)
            .await
            .unwrap();
        let serialized = searched.result.to_string();
        assert!(serialized.contains("allowed room"));
        assert!(!serialized.contains("private room"));

        let recent = tool
            .execute(serde_json::json!({"query": ""}), &ctx)
            .await
            .unwrap();
        let recent = recent.result["recent_sessions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(recent.len(), 1);
        assert_eq!(
            recent[0]["conversation_id"],
            serde_json::json!(conversation_a)
        );
    }
}
