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

use async_trait::async_trait;
use uuid::Uuid;

use crate::agent::session_search::{SessionSearchRender, SessionSearchService};
use crate::context::JobContext;
use crate::db::Database;
use crate::identity::ConversationKind as IdentityConversationKind;
use crate::llm::LlmProvider;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};
use crate::workspace::{SearchConfig, Workspace, paths};

/// Files the LLM may only APPEND to — never fully overwrite.
///
/// Currently empty: IDENTITY.md was moved to freely-rewritable to prevent
/// identity accretion during repeated bootstrap runs. If a file should be
/// strictly append-only in the future, add it here.
const APPEND_ONLY_IDENTITY_FILES: &[&str] = &[];

/// Files protected from deletion through memory_delete.
const DELETE_PROTECTED_FILES: &[&str] = &[paths::IDENTITY];

/// Files the agent may FULLY REWRITE (replace entire content, append: false).
///
/// IDENTITY.md remains writable through memory_write because prompt_manage
/// intentionally excludes it in V1.
const FREELY_REWRITABLE_IDENTITY_FILES: &[&str] = &[paths::IDENTITY];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryScope {
    Shared,
    Actor,
}

fn split_scoped_target(target: &str) -> (Option<MemoryScope>, String) {
    let trimmed = target.trim();
    for (prefix, scope) in [
        ("shared:", MemoryScope::Shared),
        ("root:", MemoryScope::Shared),
        ("household:", MemoryScope::Shared),
        ("actor:", MemoryScope::Actor),
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return (Some(scope), rest.trim().trim_start_matches('/').to_string());
        }
    }
    (None, trimmed.to_string())
}

fn actor_scoped_path(actor_id: &str, path: &str) -> String {
    if path.is_empty() {
        paths::actor_root(actor_id)
    } else if path.eq_ignore_ascii_case("memory") || path.eq_ignore_ascii_case(paths::MEMORY) {
        paths::actor_memory(actor_id)
    } else if path.eq_ignore_ascii_case(paths::USER) {
        paths::actor_user(actor_id)
    } else if path.eq_ignore_ascii_case("profile") || path.eq_ignore_ascii_case(paths::PROFILE) {
        paths::actor_profile(actor_id)
    } else if path.starts_with("actors/") {
        path.to_string()
    } else {
        format!("{}/{}", paths::actor_root(actor_id), path)
    }
}

fn shared_root_path(path: &str) -> String {
    if path.eq_ignore_ascii_case("memory") {
        paths::MEMORY.to_string()
    } else if path.eq_ignore_ascii_case("heartbeat") {
        paths::HEARTBEAT.to_string()
    } else if path.eq_ignore_ascii_case("profile") || path.eq_ignore_ascii_case(paths::PROFILE) {
        paths::PROFILE.to_string()
    } else {
        path.to_string()
    }
}

fn job_conversation_kind(metadata: &serde_json::Value) -> IdentityConversationKind {
    let kind = metadata
        .get("conversation_kind")
        .and_then(|v| v.as_str())
        .or_else(|| metadata.get("chat_type").and_then(|v| v.as_str()))
        .unwrap_or("direct")
        .to_ascii_lowercase();
    match kind.as_str() {
        "group" | "channel" | "supergroup" => IdentityConversationKind::Group,
        _ => IdentityConversationKind::Direct,
    }
}

fn resolve_memory_write_path(ctx: &JobContext, target: &str) -> (String, bool) {
    let (explicit_scope, bare_target) = split_scoped_target(target);
    let actor_id = ctx
        .metadata
        .get("actor_id")
        .or_else(|| ctx.metadata.get("actor"))
        .and_then(|v| v.as_str());
    let direct_actor = job_conversation_kind(&ctx.metadata) == IdentityConversationKind::Direct
        && actor_id.is_some();

    match explicit_scope {
        Some(MemoryScope::Shared) => (shared_root_path(&bare_target), false),
        Some(MemoryScope::Actor) => {
            let actor_id = actor_id.unwrap_or("unknown");
            (actor_scoped_path(actor_id, &bare_target), true)
        }
        None if direct_actor
            && (bare_target.eq_ignore_ascii_case("memory")
                || bare_target.eq_ignore_ascii_case(paths::MEMORY)
                || bare_target.eq_ignore_ascii_case(paths::USER)
                || bare_target.eq_ignore_ascii_case(paths::PROFILE)) =>
        {
            let actor_id = actor_id.expect("checked is_some above");
            (actor_scoped_path(actor_id, &bare_target), true)
        }
        None if direct_actor && bare_target.starts_with("actors/") => {
            let actor_id = actor_id.expect("checked is_some above");
            (actor_scoped_path(actor_id, &bare_target), true)
        }
        None => (shared_root_path(&bare_target), false),
    }
}

fn workspace_for_ctx(base: &Arc<Workspace>, ctx: &JobContext) -> Workspace {
    let agent_workspace_id = ctx
        .metadata
        .get("agent_workspace_id")
        .and_then(|v| v.as_str())
        .and_then(|v| Uuid::parse_str(v).ok())
        .or_else(|| base.agent_id());
    base.scoped_clone(ctx.user_id.clone(), agent_workspace_id)
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query. Use natural language to describe what you're looking for."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 6, max: 20)",
                    "default": 6,
                    "minimum": 1,
                    "maximum": 20
                },
                "mmr": {
                    "type": "boolean",
                    "description": "Enable MMR diversity re-ranking (default: true). Set false only when you want raw ranked results.",
                    "default": true
                },
                "temporal_decay": {
                    "type": "boolean",
                    "description": "Downweight older notes (default: true). Set false to treat all notes equally regardless of age.",
                    "default": true
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let workspace = workspace_for_ctx(&self.workspace, ctx);

        let query = require_str(&params, "query")?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(6)
            .min(20) as usize;

        // MMR re-ranking on by default — reduces near-duplicate daily notes.
        // Lambda 0.7 = slight relevance bias (matches openclaw recommendation).
        let use_mmr = params.get("mmr").and_then(|v| v.as_bool()).unwrap_or(true);

        // Temporal decay on by default — 30-day half-life so older notes don't
        // crowd out recent ones on equal semantic similarity.
        let use_decay = params
            .get("temporal_decay")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut config = SearchConfig::default().with_limit(limit);
        if use_mmr {
            config = config.with_mmr(0.7);
        }
        if use_decay {
            // 30-day half-life: today = 1.0×, 1 month ago = 0.5×, 3 months ago = 0.125×
            config = config.with_temporal_decay(30.0);
        }

        let results = workspace
            .search_with_config(query, config)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Search failed: {}", e)))?;

        let output = serde_json::json!({
            "query": query,
            "results": results.iter().map(|r| {
                serde_json::json!({
                    "path": r.path.clone(),
                    "content": r.content,
                    "score": r.score,
                    "document_id": r.document_id.to_string(),
                    "is_hybrid_match": r.is_hybrid(),
                })
            }).collect::<Vec<_>>(),
            "result_count": results.len(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal memory, trusted content
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

    fn current_scope_filters(&self, ctx: &JobContext) -> (String, String, bool, Option<Uuid>) {
        let principal_id = ctx
            .metadata
            .get("principal_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| ctx.principal_id.clone());
        let actor_id = ctx
            .metadata
            .get("actor_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| ctx.actor_id.clone())
            .unwrap_or_else(|| principal_id.clone());
        let include_group_history =
            job_conversation_kind(&ctx.metadata) == IdentityConversationKind::Group;
        let conversation_id = ctx
            .conversation_id
            .or_else(|| {
                ctx.metadata
                    .get("conversation_id")
                    .or_else(|| ctx.metadata.get("thread_id"))
                    .and_then(|v| v.as_str())
                    .and_then(|value| Uuid::parse_str(value).ok())
            })
            .or_else(|| {
                ctx.metadata
                    .get("conversation_id")
                    .and_then(|v| v.as_str())
                    .and_then(|value| Uuid::parse_str(value).ok())
            });
        (
            principal_id,
            actor_id,
            include_group_history,
            conversation_id,
        )
    }

    async fn recent_conversation_metadata(
        &self,
        principal_id: &str,
        channel: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, ToolError> {
        let Some(channel) = channel else {
            return Ok(Vec::new());
        };
        let recent = self
            .store
            .list_conversations_with_preview(principal_id, channel, limit as i64)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Transcript listing failed: {}", e)))?;
        Ok(recent
            .into_iter()
            .map(|conversation| {
                serde_json::json!({
                    "conversation_id": conversation.id,
                    "user_id": conversation.user_id,
                    "actor_id": conversation.actor_id,
                    "channel": conversation.channel,
                    "conversation_kind": conversation.conversation_kind.as_str(),
                    "title": conversation.title,
                    "message_count": conversation.message_count,
                    "started_at": conversation.started_at.to_rfc3339(),
                    "last_activity": conversation.last_activity.to_rfc3339(),
                    "thread_type": conversation.thread_type,
                    "handoff": conversation.handoff,
                    "stable_external_conversation_key": conversation.stable_external_conversation_key,
                })
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
         This searches conversation history only, not workspace documents."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The transcript search query."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 8, max: 25)",
                    "default": 8,
                    "minimum": 1,
                    "maximum": 25
                },
                "include_current_thread": {
                    "type": "boolean",
                    "description": "If true, constrain search to the current thread when thread metadata is available.",
                    "default": true
                },
                "all_channels": {
                    "type": "boolean",
                    "description": "If true, search all channels for this actor/user scope. If false (default), search is limited to the current channel.",
                    "default": false
                },
                "summarize_sessions": {
                    "type": "boolean",
                    "description": "If true, summarize matching sessions with the auxiliary/cheap model when available. Defaults to true only when a cheap model is configured.",
                    "default": false
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let query = require_str(&params, "query")?;
        let result_limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(8)
            .clamp(1, 25) as usize;
        let include_current_thread = params
            .get("include_current_thread")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let all_channels = params
            .get("all_channels")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let (principal_id, actor_id, _include_group_history, _conversation_id) =
            self.current_scope_filters(ctx);

        let channel_filter = if all_channels {
            None
        } else {
            ctx.metadata
                .get("channel")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        let thread_filter = if include_current_thread {
            ctx.metadata
                .get("thread_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        } else {
            None
        };

        let summarize_sessions = params
            .get("summarize_sessions")
            .and_then(|v| v.as_bool())
            .unwrap_or(self.service.summarizer_configured());

        if query.trim().is_empty() {
            let recent = self
                .recent_conversation_metadata(
                    &principal_id,
                    channel_filter.as_deref(),
                    result_limit,
                )
                .await?;
            let output = serde_json::json!({
                "query": query,
                "result_count": recent.len(),
                "recent_sessions": recent,
                "summarized": false,
            });
            return Ok(ToolOutput::success(output, start.elapsed()));
        }

        let hits = self
            .store
            .search_conversation_messages(
                &principal_id,
                query,
                Some(&actor_id),
                channel_filter.as_deref(),
                thread_filter.as_deref(),
                result_limit as i64,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Transcript search failed: {}", e)))?;
        let SessionSearchRender {
            results,
            summarized,
            fallback,
        } = self
            .service
            .render_results(&self.store, query, hits, summarize_sessions)
            .await;

        let mut output = serde_json::json!({
            "query": query,
            "result_count": results.len(),
            "results": results,
            "summarized": summarized,
        });
        if fallback {
            output["fallback"] = serde_json::json!(true);
        }

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for writing to workspace memory.
///
/// Use this to persist important information that should be remembered
/// across sessions: decisions, preferences, facts, lessons learned.
pub struct MemoryWriteTool {
    workspace: Arc<Workspace>,
}

impl MemoryWriteTool {
    /// Create a new memory write tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
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
         For SOUL.md / AGENTS.md / USER.md use prompt_manage instead of memory_write. \
         or a custom path. In direct DMs, memory/user/profile writes default to the actor overlay; \
         prefix with 'shared:' to force the household root. \
         ALWAYS write well-structured markdown: use ## headers for sections, bullet points, \
         and clear prose. Never dump raw unformatted text into identity files."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The content to write to memory. Be concise but include relevant context."
                },
                "target": {
                    "type": "string",
                    "description": "Where to write: 'memory' for MEMORY.md, 'daily_log' for today's log, 'heartbeat' for HEARTBEAT.md checklist, 'shared:...' to force the household root, or a path like 'projects/alpha/notes.md'",
                    "default": "daily_log"
                },
                "append": {
                    "type": "boolean",
                    "description": "If true, append to existing content. If false, replace entirely.",
                    "default": true
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let workspace = workspace_for_ctx(&self.workspace, ctx);

        let content = require_str(&params, "content")?;

        if content.trim().is_empty() {
            return Err(ToolError::InvalidParameters(
                "content cannot be empty".to_string(),
            ));
        }

        let target = params
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("daily_log");

        let append = params
            .get("append")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let (path, _is_actor_scoped) = resolve_memory_write_path(ctx, target);
        let normalized_path = path.trim_start_matches('/');
        let file_name = normalized_path
            .rsplit('/')
            .next()
            .unwrap_or(normalized_path);

        // IDENTITY.md is append-only to protect the agent's established name/creature.
        if APPEND_ONLY_IDENTITY_FILES.contains(&file_name) {
            if !append {
                return Err(ToolError::NotAuthorized(format!(
                    "'{}' is append-only. Add an '## Update' section with your changes \
                     instead of overwriting. To fully restructure SOUL.md / AGENTS.md / \
                     USER.md, use those targets with append: false.",
                    target,
                )));
            }
            workspace
                .append(&path, content)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
            let output = serde_json::json!({
                "status": "appended",
                "path": path,
                "append": true,
                "content_length": content.len(),
                "note": "Identity file updated (append-only)",
            });
            return Ok(ToolOutput::success(output, start.elapsed()));
        }

        // SOUL.md / AGENTS.md / USER.md must be mutated through prompt_manage.
        if [paths::SOUL, paths::AGENTS, paths::USER]
            .iter()
            .any(|p| file_name.eq_ignore_ascii_case(p))
        {
            return Err(ToolError::NotAuthorized(format!(
                "'{}' must be managed through prompt_manage (bounded prompt mutation).",
                target
            )));
        }

        // IDENTITY.md remains freely rewritable through memory_write.
        if FREELY_REWRITABLE_IDENTITY_FILES
            .iter()
            .any(|p| file_name.eq_ignore_ascii_case(p))
        {
            if append {
                workspace
                    .append(&path, content)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
            } else {
                workspace
                    .write(&path, content)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
            }
            let output = serde_json::json!({
                "status": if append { "appended" } else { "rewritten" },
                "path": path,
                "append": append,
                "content_length": content.len(),
                "note": if append {
                    "Personality file updated (new section appended)"
                } else {
                    "Personality file fully restructured — well-formed markdown expected"
                },
            });
            return Ok(ToolOutput::success(output, start.elapsed()));
        }

        let path =
            match target.trim() {
                t if t.eq_ignore_ascii_case("memory") => {
                    if path.eq_ignore_ascii_case(paths::MEMORY) {
                        if append {
                            workspace.append_memory(content).await.map_err(|e| {
                                ToolError::ExecutionFailed(format!("Write failed: {}", e))
                            })?;
                        } else {
                            workspace.write(paths::MEMORY, content).await.map_err(|e| {
                                ToolError::ExecutionFailed(format!("Write failed: {}", e))
                            })?;
                        }
                    } else if append {
                        let doc = workspace.read(&path).await.ok();
                        let new_content = match doc {
                            Some(doc) if !doc.content.is_empty() => {
                                format!("{}\n\n{}", doc.content, content)
                            }
                            _ => content.to_string(),
                        };
                        workspace.write(&path, &new_content).await.map_err(|e| {
                            ToolError::ExecutionFailed(format!("Write failed: {}", e))
                        })?;
                    } else {
                        workspace.write(&path, content).await.map_err(|e| {
                            ToolError::ExecutionFailed(format!("Write failed: {}", e))
                        })?;
                    }
                    path
                }
                t if t.eq_ignore_ascii_case("daily_log") => {
                    workspace
                        .append_daily_log(content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                    format!("daily/{}.md", chrono::Utc::now().format("%Y-%m-%d"))
                }
                t if t.eq_ignore_ascii_case("heartbeat") => {
                    if append {
                        workspace.append(&path, content).await.map_err(|e| {
                            ToolError::ExecutionFailed(format!("Write failed: {}", e))
                        })?;
                    } else {
                        workspace.write(&path, content).await.map_err(|e| {
                            ToolError::ExecutionFailed(format!("Write failed: {}", e))
                        })?;
                    }
                    path
                }
                _ => {
                    if append {
                        let doc = workspace.read(&path).await.ok();
                        let new_content = match doc {
                            Some(doc) if !doc.content.is_empty() => {
                                format!("{}\n\n{}", doc.content, content)
                            }
                            _ => content.to_string(),
                        };
                        workspace.write(&path, &new_content).await.map_err(|e| {
                            ToolError::ExecutionFailed(format!("Write failed: {}", e))
                        })?;
                    } else {
                        workspace.write(&path, content).await.map_err(|e| {
                            ToolError::ExecutionFailed(format!("Write failed: {}", e))
                        })?;
                    }
                    path
                }
            };

        let output = serde_json::json!({
            "status": "written",
            "path": path,
            "append": append,
            "content_length": content.len(),
        });

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
        "Read a file from the workspace memory (database-backed storage). \
         Use this to read files shown by memory_tree. NOT for local filesystem files \
         (use read_file for those). Works with identity files, heartbeat checklist, \
         memory, daily logs, or any custom workspace path. \
         Returns empty content (not an error) if the file does not exist yet."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file (e.g., 'MEMORY.md', 'daily/2024-01-15.md', 'TOOLS.md')"
                },
                "start_line": {
                    "type": "integer",
                    "description": "1-indexed line to start reading from (optional). Useful for large files like MEMORY.md.",
                    "minimum": 1
                },
                "num_lines": {
                    "type": "integer",
                    "description": "Maximum number of lines to return (optional). Use with start_line for targeted reads.",
                    "minimum": 1
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let workspace = workspace_for_ctx(&self.workspace, ctx);

        let path = require_str(&params, "path")?;

        // Graceful degradation: missing file → empty content, not an error.
        // Matches openclaw memory_get: { text: "", path } on ENOENT.
        let doc = match workspace.read(path).await {
            Ok(doc) => doc,
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => {
                let output = serde_json::json!({
                    "path": path,
                    "content": "",
                    "word_count": 0,
                    "exists": false,
                });
                return Ok(ToolOutput::success(output, start.elapsed()));
            }
            Err(e) => return Err(ToolError::ExecutionFailed(format!("Read failed: {}", e))),
        };

        // Optional line-range slicing.
        let content = if params.get("start_line").is_some() || params.get("num_lines").is_some() {
            let start_line = params
                .get("start_line")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                .max(1) as usize;
            let num_lines = params
                .get("num_lines")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);

            let lines: Vec<&str> = doc.content.lines().collect();
            let total_lines = lines.len();

            // Convert 1-indexed start_line to 0-indexed.
            let from = (start_line - 1).min(total_lines);
            let to = match num_lines {
                Some(n) => (from + n).min(total_lines),
                None => total_lines,
            };

            lines[from..to].join("\n")
        } else {
            doc.content.clone()
        };

        let total_lines = doc.content.lines().count();
        let output = serde_json::json!({
            "path": doc.path,
            "content": content,
            "word_count": content.split_whitespace().count(),
            "total_lines": total_lines,
            "updated_at": doc.updated_at.to_rfc3339(),
            "exists": true,
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal memory
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
        workspace: &Workspace,
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Root path to start from (empty string for workspace root)",
                    "default": ""
                },
                "depth": {
                    "type": "integer",
                    "description": "Maximum depth to traverse (1 = immediate children only)",
                    "default": 1,
                    "minimum": 1,
                    "maximum": 10
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let workspace = workspace_for_ctx(&self.workspace, ctx);

        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");

        let depth = params
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .clamp(1, 10) as usize;

        let tree = self.build_tree(&workspace, path, 1, depth).await?;

        // Compact output: just the tree array
        Ok(ToolOutput::success(
            serde_json::Value::Array(tree),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
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
         SOUL.md / AGENTS.md / USER.md can be fully rewritten with prompt_manage \
         rather than deleted. \
         Primary use-case: memory_delete('BOOTSTRAP.md') after the identity ritual completes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to delete (e.g. 'BOOTSTRAP.md', 'daily/2024-01-15.md')"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let workspace = workspace_for_ctx(&self.workspace, ctx);

        let path = require_str(&params, "path")?;

        // Only IDENTITY.md is delete-protected.
        // SOUL/AGENTS/USER should be restructured with prompt_manage instead.
        let normalized = path.trim_start_matches('/');
        if DELETE_PROTECTED_FILES
            .iter()
            .any(|p| normalized.eq_ignore_ascii_case(p))
        {
            return Err(ToolError::NotAuthorized(format!(
                "'{}' cannot be deleted. Use memory_write to edit identity content. \
                 To restructure SOUL.md / AGENTS.md / USER.md entirely, use \
                 prompt_manage instead of deleting.",
                path
            )));
        }

        workspace
            .delete(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Delete failed: {}", e)))?;

        // If BOOTSTRAP.md was deleted, notify the bridge to update frontend state.
        let is_bootstrap = normalized.eq_ignore_ascii_case(crate::workspace::paths::BOOTSTRAP);
        if is_bootstrap && let Some(ref tx) = self.sse_sender {
            let _ = tx.send(crate::channels::web::types::SseEvent::BootstrapCompleted);
            tracing::info!("[memory_delete] Emitted BootstrapCompleted SSE event");
        }

        let output = serde_json::json!({
            "status": "deleted",
            "path": path,
        });

        Ok(ToolOutput::success(output, start.elapsed()))
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
        assert!(!tool.requires_sanitization());

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
        let tool = MemoryWriteTool::new(workspace);

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
    fn test_memory_write_routes_direct_actor_memory_to_overlay() {
        let mut ctx = JobContext::with_user("default", "chat", "test");
        ctx.metadata = serde_json::json!({
            "conversation_kind": "direct",
            "actor_id": "actor-123",
        });

        let (path, is_actor_scoped) = resolve_memory_write_path(&ctx, "memory");
        assert!(is_actor_scoped);
        assert_eq!(path, "actors/actor-123/MEMORY.md");
    }

    #[test]
    fn test_memory_write_shared_prefix_forces_root() {
        let mut ctx = JobContext::with_user("default", "chat", "test");
        ctx.metadata = serde_json::json!({
            "conversation_kind": "direct",
            "actor_id": "actor-123",
        });

        let (path, is_actor_scoped) = resolve_memory_write_path(&ctx, "shared:memory");
        assert!(!is_actor_scoped);
        assert_eq!(path, "MEMORY.md");
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
}
