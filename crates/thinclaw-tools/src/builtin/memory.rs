//! Root-independent memory tool path policy.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use thinclaw_tools_core::{
    Tool, ToolError, ToolMetadata, ToolOutput, ToolRateLimitConfig, ToolRouteIntent,
};
use thinclaw_types::JobContext;
use thinclaw_workspace::paths;
use uuid::Uuid;

#[cfg(test)]
use crate::ports::ToolOperationScope;
use crate::ports::{MemoryToolHostPort, ToolMemoryActionRequest, tool_scope_from_job_context};

/// Files the LLM may only append to, never fully overwrite.
pub const APPEND_ONLY_IDENTITY_FILES: &[&str] = &[];

/// Files protected from deletion through memory_delete.
pub const DELETE_PROTECTED_FILES: &[&str] = &[paths::IDENTITY];

/// Files the agent may fully rewrite.
pub const FREELY_REWRITABLE_IDENTITY_FILES: &[&str] = &[paths::IDENTITY];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryScope {
    Shared,
    Actor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryConversationKind {
    Direct,
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySearchParams {
    pub query: String,
    pub limit: usize,
    pub use_mmr: bool,
    pub use_temporal_decay: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryWriteParams {
    pub content: String,
    pub target: String,
    pub append: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryReadSlice {
    pub requested: bool,
    pub start_line: usize,
    pub num_lines: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryTreeParams {
    pub path: String,
    pub depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryDeleteAction<'a> {
    Delete { normalized_path: &'a str },
    FinalizeBootstrap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSearchParams {
    pub query: String,
    pub limit: usize,
    pub include_current_thread: bool,
    pub all_channels: bool,
    pub summarize_sessions: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSearchScope {
    pub principal_id: String,
    pub actor_id: String,
    pub include_group_history: bool,
    pub conversation_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSearchFilters {
    pub channel: Option<String>,
    pub thread_id: Option<String>,
}

pub fn split_scoped_target(target: &str) -> (Option<MemoryScope>, String) {
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

pub fn actor_scoped_path(actor_id: &str, path: &str) -> String {
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

pub fn shared_root_path(path: &str) -> String {
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

pub fn memory_write_parameters_schema() -> serde_json::Value {
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

pub fn parse_memory_write_params(
    params: &serde_json::Value,
) -> Result<MemoryWriteParams, ToolError> {
    let content = params
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ToolError::InvalidParameters("missing required parameter: content".to_string())
        })?;

    if content.trim().is_empty() {
        return Err(ToolError::InvalidParameters(
            "content cannot be empty".to_string(),
        ));
    }

    let target = params
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("daily_log")
        .to_string();

    let append = params
        .get("append")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    Ok(MemoryWriteParams {
        content: content.to_string(),
        target,
        append,
    })
}

pub fn memory_read_parameters_schema() -> serde_json::Value {
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

pub fn parse_memory_read_slice(params: &serde_json::Value) -> MemoryReadSlice {
    let requested = params.get("start_line").is_some() || params.get("num_lines").is_some();
    let start_line = params
        .get("start_line")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1) as usize;
    let num_lines = params
        .get("num_lines")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    MemoryReadSlice {
        requested,
        start_line,
        num_lines,
    }
}

pub fn apply_memory_read_slice(content: &str, slice: MemoryReadSlice) -> (String, usize) {
    let total_lines = content.lines().count();
    if !slice.requested {
        return (content.to_string(), total_lines);
    }

    let lines: Vec<&str> = content.lines().collect();
    let from = (slice.start_line - 1).min(lines.len());
    let to = match slice.num_lines {
        Some(n) => (from + n).min(lines.len()),
        None => lines.len(),
    };
    (lines[from..to].join("\n"), total_lines)
}

pub fn memory_read_missing_output(path: &str) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "content": "",
        "word_count": 0,
        "exists": false,
    })
}

pub fn memory_read_output(
    path: &str,
    content: &str,
    total_lines: usize,
    updated_at: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "content": content,
        "word_count": content.split_whitespace().count(),
        "total_lines": total_lines,
        "updated_at": updated_at.map_or(serde_json::Value::Null, serde_json::Value::String),
        "exists": true,
    })
}

pub fn memory_delete_output(path: &str) -> serde_json::Value {
    serde_json::json!({
        "status": "deleted",
        "path": path,
    })
}

pub fn memory_write_output(
    status: &str,
    path: &str,
    append: bool,
    content_length: usize,
    note: Option<&str>,
) -> serde_json::Value {
    let mut output = serde_json::json!({
        "status": status,
        "path": path,
        "append": append,
        "content_length": content_length,
    });

    if let Some(note) = note {
        output["note"] = serde_json::Value::String(note.to_string());
    }

    output
}

pub fn memory_write_mirror_payload(path: &str, append: bool, content: &str) -> serde_json::Value {
    serde_json::json!({
        "tool": "memory_write",
        "path": path,
        "append": append,
        "content_preview": content.chars().take(240).collect::<String>(),
    })
}

pub fn memory_search_result_entry(
    path: &str,
    content: &str,
    score: f64,
    document_id: String,
    is_hybrid_match: bool,
) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "content": content,
        "score": score,
        "document_id": document_id,
        "is_hybrid_match": is_hybrid_match,
    })
}

pub fn memory_search_output(query: &str, results: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "query": query,
        "results": results,
        "result_count": results.len(),
    })
}

#[allow(clippy::too_many_arguments)]
pub fn session_metadata_entry(
    conversation_id: impl serde::Serialize,
    user_id: &str,
    actor_id: Option<&str>,
    channel: &str,
    conversation_kind: &str,
    title: Option<&str>,
    message_count: i64,
    started_at: String,
    last_activity: String,
    thread_type: Option<&str>,
    handoff: impl serde::Serialize,
    stable_external_conversation_key: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "conversation_id": conversation_id,
        "user_id": user_id,
        "actor_id": actor_id,
        "channel": channel,
        "conversation_kind": conversation_kind,
        "title": title,
        "message_count": message_count,
        "started_at": started_at,
        "last_activity": last_activity,
        "thread_type": thread_type,
        "handoff": handoff,
        "stable_external_conversation_key": stable_external_conversation_key,
    })
}

pub fn session_recent_output(
    query: &str,
    recent_sessions: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "query": query,
        "result_count": recent_sessions.len(),
        "recent_sessions": recent_sessions,
        "summarized": false,
    })
}

pub fn session_search_output(
    query: &str,
    results: Vec<serde_json::Value>,
    summarized: bool,
    fallback: bool,
) -> serde_json::Value {
    let mut output = serde_json::json!({
        "query": query,
        "result_count": results.len(),
        "results": results,
        "summarized": summarized,
    });
    if fallback {
        output["fallback"] = serde_json::json!(true);
    }
    output
}

pub fn memory_tree_parameters_schema() -> serde_json::Value {
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

pub fn parse_memory_tree_params(params: &serde_json::Value) -> MemoryTreeParams {
    let path = params
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let depth = params
        .get("depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .clamp(1, 10) as usize;
    MemoryTreeParams { path, depth }
}

pub fn memory_delete_parameters_schema() -> serde_json::Value {
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

pub fn resolve_memory_delete_action(path: &str) -> Result<MemoryDeleteAction<'_>, ToolError> {
    let normalized = path.trim_start_matches('/');
    if [paths::SOUL, paths::SOUL_LOCAL, paths::AGENTS, paths::USER]
        .iter()
        .any(|p| normalized.eq_ignore_ascii_case(p))
    {
        return Err(ToolError::NotAuthorized(format!(
            "'{}' cannot be deleted. Use prompt_manage to rewrite or refine prompt-managed identity files.",
            path
        )));
    }

    if DELETE_PROTECTED_FILES
        .iter()
        .any(|p| normalized.eq_ignore_ascii_case(p))
    {
        return Err(ToolError::NotAuthorized(format!(
            "'{}' cannot be deleted. Use memory_write to edit identity content. \
             To restructure SOUL.md / SOUL.local.md / AGENTS.md / USER.md entirely, use \
             prompt_manage instead of deleting.",
            path
        )));
    }

    if normalized.eq_ignore_ascii_case(paths::BOOTSTRAP) {
        Ok(MemoryDeleteAction::FinalizeBootstrap)
    } else {
        Ok(MemoryDeleteAction::Delete {
            normalized_path: normalized,
        })
    }
}

pub fn memory_search_parameters_schema() -> serde_json::Value {
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

pub fn session_search_parameters_schema() -> serde_json::Value {
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
                "description": "If true, constrain search to the current thread when thread metadata is available. In direct chats the default is false so linked history can be searched.",
                "default": true
            },
            "all_channels": {
                "type": "boolean",
                "description": "If true, search all channels for this actor/user scope. In direct chats linked cross-channel history is searched by default.",
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

pub fn parse_memory_search_params(
    params: &serde_json::Value,
) -> Result<MemorySearchParams, ToolError> {
    let query = params
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ToolError::InvalidParameters("missing required parameter: query".to_string())
        })?
        .to_string();

    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(6)
        .min(20) as usize;

    let use_mmr = params.get("mmr").and_then(|v| v.as_bool()).unwrap_or(true);
    let use_temporal_decay = params
        .get("temporal_decay")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    Ok(MemorySearchParams {
        query,
        limit,
        use_mmr,
        use_temporal_decay,
    })
}

pub fn parse_session_search_params(
    params: &serde_json::Value,
    direct_scope: bool,
    summarizer_configured: bool,
) -> Result<SessionSearchParams, ToolError> {
    let query = params
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ToolError::InvalidParameters("missing required parameter: query".to_string())
        })?
        .to_string();
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(8)
        .clamp(1, 25) as usize;
    let include_current_thread = params
        .get("include_current_thread")
        .and_then(|v| v.as_bool())
        .unwrap_or(!direct_scope);
    let all_channels = params
        .get("all_channels")
        .and_then(|v| v.as_bool())
        .unwrap_or(direct_scope);
    let summarize_sessions = params
        .get("summarize_sessions")
        .and_then(|v| v.as_bool())
        .unwrap_or(summarizer_configured);

    Ok(SessionSearchParams {
        query,
        limit,
        include_current_thread,
        all_channels,
        summarize_sessions,
    })
}

pub fn session_search_direct_scope_for_context(ctx: &JobContext) -> bool {
    memory_conversation_kind(&ctx.metadata) == MemoryConversationKind::Direct
}

pub fn parse_session_search_params_for_context(
    params: &serde_json::Value,
    ctx: &JobContext,
    summarizer_configured: bool,
) -> Result<SessionSearchParams, ToolError> {
    parse_session_search_params(
        params,
        session_search_direct_scope_for_context(ctx),
        summarizer_configured,
    )
}

pub fn session_search_filters_for_context(
    ctx: &JobContext,
    params: &SessionSearchParams,
) -> SessionSearchFilters {
    let channel = if params.all_channels {
        None
    } else {
        ctx.metadata
            .get("channel")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let thread_id = if params.include_current_thread {
        ctx.metadata
            .get("thread_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    } else {
        None
    };
    SessionSearchFilters { channel, thread_id }
}

pub fn memory_conversation_kind(metadata: &serde_json::Value) -> MemoryConversationKind {
    let kind = metadata
        .get("conversation_kind")
        .and_then(|v| v.as_str())
        .or_else(|| metadata.get("chat_type").and_then(|v| v.as_str()))
        .unwrap_or("direct")
        .to_ascii_lowercase();
    match kind.as_str() {
        "group" | "channel" | "supergroup" => MemoryConversationKind::Group,
        _ => MemoryConversationKind::Direct,
    }
}

pub fn resolve_session_search_scope(
    metadata: &serde_json::Value,
    context_principal_id: &str,
    context_actor_id: Option<&str>,
    context_conversation_id: Option<Uuid>,
) -> SessionSearchScope {
    let principal_id = metadata
        .get("principal_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| context_principal_id.to_string());
    let actor_id = metadata
        .get("actor_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| context_actor_id.map(str::to_string))
        .unwrap_or_else(|| principal_id.clone());
    let include_group_history = memory_conversation_kind(metadata) == MemoryConversationKind::Group;
    let conversation_id = context_conversation_id.or_else(|| {
        metadata
            .get("conversation_id")
            .or_else(|| metadata.get("thread_id"))
            .and_then(|v| v.as_str())
            .and_then(|value| Uuid::parse_str(value).ok())
    });

    SessionSearchScope {
        principal_id,
        actor_id,
        include_group_history,
        conversation_id,
    }
}

pub fn resolve_session_search_scope_for_context(ctx: &JobContext) -> SessionSearchScope {
    resolve_session_search_scope(
        &ctx.metadata,
        &ctx.principal_id,
        ctx.actor_id.as_deref(),
        ctx.conversation_id,
    )
}

pub fn workspace_agent_id_from_metadata(
    metadata: &serde_json::Value,
    fallback_agent_id: Option<Uuid>,
) -> Option<Uuid> {
    metadata
        .get("agent_workspace_id")
        .and_then(|v| v.as_str())
        .and_then(|v| Uuid::parse_str(v).ok())
        .or(fallback_agent_id)
}

pub fn resolve_memory_write_path(
    metadata: &serde_json::Value,
    actor_id: Option<&str>,
    target: &str,
) -> (String, bool) {
    let (explicit_scope, bare_target) = split_scoped_target(target);
    let direct_actor =
        memory_conversation_kind(metadata) == MemoryConversationKind::Direct && actor_id.is_some();

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
                || bare_target.eq_ignore_ascii_case("profile")
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

pub fn resolve_memory_write_path_for_context(ctx: &JobContext, target: &str) -> (String, bool) {
    let actor_id = ctx
        .metadata
        .get("actor_id")
        .or_else(|| ctx.metadata.get("actor"))
        .and_then(|v| v.as_str())
        .or(ctx.actor_id.as_deref());
    resolve_memory_write_path(&ctx.metadata, actor_id, target)
}

async fn execute_memory_action<F, Fut>(
    host: &Arc<dyn MemoryToolHostPort>,
    params: serde_json::Value,
    ctx: &JobContext,
    action: F,
) -> Result<ToolOutput, ToolError>
where
    F: FnOnce(Arc<dyn MemoryToolHostPort>, ToolMemoryActionRequest) -> Fut,
    Fut: std::future::Future<
            Output = Result<crate::ports::ToolMemoryActionResult, crate::ports::ToolHostError>,
        >,
{
    let start = Instant::now();
    let request = ToolMemoryActionRequest {
        scope: tool_scope_from_job_context(ctx),
        params,
    };
    let result = action(Arc::clone(host), request)
        .await
        .map_err(|error| ToolError::ExecutionFailed(error.to_string()))?;
    Ok(ToolOutput::success(result.output, start.elapsed()))
}

macro_rules! memory_host_tool {
    (
        $tool:ident,
        $name:literal,
        $description:literal,
        $schema:ident,
        $parse:expr,
        $method:ident,
        $metadata:expr,
        $rate_limit:expr
    ) => {
        pub struct $tool {
            host: Arc<dyn MemoryToolHostPort>,
        }

        impl $tool {
            pub fn new(host: Arc<dyn MemoryToolHostPort>) -> Self {
                Self { host }
            }
        }

        #[async_trait]
        impl Tool for $tool {
            fn name(&self) -> &str {
                $name
            }

            fn description(&self) -> &str {
                $description
            }

            fn parameters_schema(&self) -> serde_json::Value {
                $schema()
            }

            fn metadata(&self) -> ToolMetadata {
                $metadata
            }

            async fn execute(
                &self,
                params: serde_json::Value,
                ctx: &JobContext,
            ) -> Result<ToolOutput, ToolError> {
                $parse(&params)?;
                execute_memory_action(&self.host, params, ctx, |host, request| async move {
                    host.$method(request).await
                })
                .await
            }

            fn requires_sanitization(&self) -> bool {
                false
            }

            fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
                $rate_limit
            }
        }
    };
}

fn validate_memory_read_params(params: &serde_json::Value) -> Result<(), ToolError> {
    params
        .get("path")
        .and_then(|value| value.as_str())
        .map(|_| ())
        .ok_or_else(|| ToolError::InvalidParameters("missing required parameter: path".to_string()))
}

fn validate_memory_delete_params(params: &serde_json::Value) -> Result<(), ToolError> {
    let path = params
        .get("path")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            ToolError::InvalidParameters("missing required parameter: path".to_string())
        })?;
    let _ = resolve_memory_delete_action(path)?;
    Ok(())
}

fn validate_memory_tree_params(params: &serde_json::Value) -> Result<(), ToolError> {
    let _ = parse_memory_tree_params(params);
    Ok(())
}

memory_host_tool!(
    MemorySearchHostTool,
    "memory_search",
    "Search past memories, decisions, and context.",
    memory_search_parameters_schema,
    parse_memory_search_params,
    search_memory_action,
    ToolMetadata::authoritative(ToolRouteIntent::MemoryRecall),
    None
);

memory_host_tool!(
    MemoryWriteHostTool,
    "memory_write",
    "Write to persistent memory.",
    memory_write_parameters_schema,
    parse_memory_write_params,
    write_memory_action,
    ToolMetadata::default(),
    Some(ToolRateLimitConfig::new(20, 200))
);

memory_host_tool!(
    MemoryReadHostTool,
    "memory_read",
    "Read a durable ThinClaw memory file.",
    memory_read_parameters_schema,
    validate_memory_read_params,
    read_memory_action,
    ToolMetadata::authoritative(ToolRouteIntent::MemoryRecall),
    None
);

memory_host_tool!(
    MemoryTreeHostTool,
    "memory_tree",
    "View the workspace memory structure as a tree.",
    memory_tree_parameters_schema,
    validate_memory_tree_params,
    tree_memory_action,
    ToolMetadata::default(),
    None
);

memory_host_tool!(
    MemoryDeleteHostTool,
    "memory_delete",
    "Delete a file from workspace memory.",
    memory_delete_parameters_schema,
    validate_memory_delete_params,
    delete_memory_action,
    ToolMetadata::default(),
    None
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{
        ToolHostError, ToolMemoryActionResult, ToolMemoryEntry, ToolMemoryReadRequest,
        ToolMemoryScope, ToolMemorySearchRequest, ToolMemoryWriteRequest,
    };

    struct StubMemoryHost;

    impl StubMemoryHost {
        fn output(action: &str, request: ToolMemoryActionRequest) -> ToolMemoryActionResult {
            ToolMemoryActionResult {
                output: serde_json::json!({
                    "action": action,
                    "principal_id": request.scope.principal_id,
                    "actor_id": request.scope.actor_id,
                    "thread_id": request.scope.thread_id,
                    "params": request.params,
                }),
            }
        }
    }

    #[async_trait]
    impl MemoryToolHostPort for StubMemoryHost {
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
            Ok(Self::output("search", request))
        }

        async fn write_memory_action(
            &self,
            request: ToolMemoryActionRequest,
        ) -> Result<ToolMemoryActionResult, ToolHostError> {
            Ok(Self::output("write", request))
        }

        async fn read_memory_action(
            &self,
            request: ToolMemoryActionRequest,
        ) -> Result<ToolMemoryActionResult, ToolHostError> {
            Ok(Self::output("read", request))
        }

        async fn tree_memory_action(
            &self,
            request: ToolMemoryActionRequest,
        ) -> Result<ToolMemoryActionResult, ToolHostError> {
            Ok(Self::output("tree", request))
        }

        async fn delete_memory_action(
            &self,
            request: ToolMemoryActionRequest,
        ) -> Result<ToolMemoryActionResult, ToolHostError> {
            Ok(Self::output("delete", request))
        }
    }

    fn stub_memory_host() -> Arc<dyn MemoryToolHostPort> {
        Arc::new(StubMemoryHost)
    }

    #[test]
    fn scoped_targets_route_to_shared_or_actor_paths() {
        assert_eq!(
            resolve_memory_write_path(&serde_json::json!({}), Some("actor-1"), "shared:memory"),
            (paths::MEMORY.to_string(), false)
        );
        assert_eq!(
            resolve_memory_write_path(&serde_json::json!({}), Some("actor-1"), "actor:profile"),
            (paths::actor_profile("actor-1"), true)
        );
    }

    #[test]
    fn direct_memory_targets_use_actor_scope() {
        assert_eq!(
            resolve_memory_write_path(
                &serde_json::json!({ "conversation_kind": "direct" }),
                Some("actor-1"),
                "memory"
            ),
            (paths::actor_memory("actor-1"), true)
        );
    }

    #[test]
    fn group_memory_targets_stay_shared_without_explicit_actor_scope() {
        assert_eq!(
            resolve_memory_write_path(
                &serde_json::json!({ "conversation_kind": "group" }),
                Some("actor-1"),
                "memory"
            ),
            (paths::MEMORY.to_string(), false)
        );
    }

    #[test]
    fn context_memory_write_path_prefers_metadata_actor() {
        let mut ctx = JobContext::with_identity("user-1", "ctx-actor", "memory", "test");
        ctx.metadata = serde_json::json!({
            "conversation_kind": "direct",
            "actor_id": "metadata-actor"
        });

        assert_eq!(
            resolve_memory_write_path_for_context(&ctx, "memory"),
            (paths::actor_memory("metadata-actor"), true)
        );
    }

    #[test]
    fn context_memory_write_path_uses_context_actor_without_metadata_actor() {
        let mut ctx = JobContext::with_identity("user-1", "ctx-actor", "memory", "test");
        ctx.metadata = serde_json::json!({
            "conversation_kind": "direct"
        });

        assert_eq!(
            resolve_memory_write_path_for_context(&ctx, "profile"),
            (paths::actor_profile("ctx-actor"), true)
        );
    }

    #[test]
    fn session_scope_for_context_uses_context_identity_by_default() {
        let conversation_id = Uuid::new_v4();
        let mut ctx = JobContext::with_identity("principal", "actor", "session", "test");
        ctx.conversation_id = Some(conversation_id);
        ctx.metadata = serde_json::json!({ "conversation_kind": "direct" });

        let scope = resolve_session_search_scope_for_context(&ctx);

        assert_eq!(scope.principal_id, "principal");
        assert_eq!(scope.actor_id, "actor");
        assert_eq!(scope.conversation_id, Some(conversation_id));
        assert!(!scope.include_group_history);
    }

    #[test]
    fn session_scope_for_context_allows_metadata_overrides() {
        let thread_id = Uuid::new_v4();
        let mut ctx = JobContext::with_identity("principal", "actor", "session", "test");
        ctx.metadata = serde_json::json!({
            "principal_id": "metadata-principal",
            "actor_id": "metadata-actor",
            "conversation_kind": "group",
            "thread_id": thread_id.to_string(),
        });

        let scope = resolve_session_search_scope_for_context(&ctx);

        assert_eq!(scope.principal_id, "metadata-principal");
        assert_eq!(scope.actor_id, "metadata-actor");
        assert_eq!(scope.conversation_id, Some(thread_id));
        assert!(scope.include_group_history);
    }

    #[test]
    fn session_search_params_for_context_default_to_cross_channel_direct_history() {
        let mut ctx = JobContext::with_identity("principal", "actor", "session", "test");
        ctx.metadata = serde_json::json!({ "conversation_kind": "direct" });

        let params = parse_session_search_params_for_context(
            &serde_json::json!({ "query": "recent decisions" }),
            &ctx,
            true,
        )
        .unwrap();

        assert!(!params.include_current_thread);
        assert!(params.all_channels);
        assert!(params.summarize_sessions);
    }

    #[test]
    fn session_search_params_for_context_default_to_current_group_thread() {
        let mut ctx = JobContext::with_identity("principal", "actor", "session", "test");
        ctx.metadata = serde_json::json!({ "conversation_kind": "group" });

        let params = parse_session_search_params_for_context(
            &serde_json::json!({ "query": "recent decisions" }),
            &ctx,
            false,
        )
        .unwrap();

        assert!(params.include_current_thread);
        assert!(!params.all_channels);
        assert!(!params.summarize_sessions);
    }

    #[test]
    fn session_search_filters_omit_channel_for_all_channel_searches() {
        let mut ctx = JobContext::with_identity("principal", "actor", "session", "test");
        ctx.metadata = serde_json::json!({
            "channel": "telegram",
            "thread_id": "thread-1"
        });
        let params = SessionSearchParams {
            query: "recent decisions".to_string(),
            limit: 8,
            include_current_thread: true,
            all_channels: true,
            summarize_sessions: false,
        };

        let filters = session_search_filters_for_context(&ctx, &params);

        assert_eq!(filters.channel, None);
        assert_eq!(filters.thread_id.as_deref(), Some("thread-1"));
    }

    #[test]
    fn session_search_filters_omit_thread_when_current_thread_excluded() {
        let mut ctx = JobContext::with_identity("principal", "actor", "session", "test");
        ctx.metadata = serde_json::json!({
            "channel": "telegram",
            "thread_id": "thread-1"
        });
        let params = SessionSearchParams {
            query: "recent decisions".to_string(),
            limit: 8,
            include_current_thread: false,
            all_channels: false,
            summarize_sessions: false,
        };

        let filters = session_search_filters_for_context(&ctx, &params);

        assert_eq!(filters.channel.as_deref(), Some("telegram"));
        assert_eq!(filters.thread_id, None);
    }

    #[test]
    fn workspace_agent_id_from_metadata_prefers_valid_explicit_id() {
        let explicit = Uuid::new_v4();
        let fallback = Uuid::new_v4();

        assert_eq!(
            workspace_agent_id_from_metadata(
                &serde_json::json!({ "agent_workspace_id": explicit.to_string() }),
                Some(fallback),
            ),
            Some(explicit)
        );
    }

    #[test]
    fn workspace_agent_id_from_metadata_falls_back_for_missing_or_invalid_id() {
        let fallback = Uuid::new_v4();

        assert_eq!(
            workspace_agent_id_from_metadata(&serde_json::json!({}), Some(fallback)),
            Some(fallback)
        );
        assert_eq!(
            workspace_agent_id_from_metadata(
                &serde_json::json!({ "agent_workspace_id": "not-a-uuid" }),
                Some(fallback),
            ),
            Some(fallback)
        );
    }

    #[test]
    fn memory_search_params_apply_defaults_and_limits() {
        assert_eq!(
            memory_search_parameters_schema()["properties"]["limit"]["maximum"],
            20
        );

        let params = parse_memory_search_params(&serde_json::json!({
            "query": "previous decisions",
            "limit": 100,
            "mmr": false
        }))
        .unwrap();
        assert_eq!(params.query, "previous decisions");
        assert_eq!(params.limit, 20);
        assert!(!params.use_mmr);
        assert!(params.use_temporal_decay);

        assert!(parse_memory_search_params(&serde_json::json!({})).is_err());
    }

    #[tokio::test]
    async fn memory_host_tools_delegate_action_outputs() {
        let mut ctx = JobContext::with_identity("user-1", "actor-1", "memory", "test");
        ctx.metadata = serde_json::json!({ "thread_id": "thread-1" });
        let host = stub_memory_host();

        let search = MemorySearchHostTool::new(Arc::clone(&host));
        assert_eq!(search.name(), "memory_search");
        assert!(!search.requires_sanitization());
        assert!(
            search
                .metadata()
                .route_intents
                .contains(&ToolRouteIntent::MemoryRecall)
        );
        let output = search
            .execute(
                serde_json::json!({
                    "query": "decision",
                    "limit": 2
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(output.result["action"], "search");
        assert_eq!(output.result["principal_id"], "user-1");
        assert_eq!(output.result["actor_id"], "actor-1");
        assert_eq!(output.result["thread_id"], "thread-1");
        assert_eq!(output.result["params"]["query"], "decision");

        let write = MemoryWriteHostTool::new(Arc::clone(&host));
        assert!(write.rate_limit_config().is_some());
        let output = write
            .execute(serde_json::json!({ "content": "remember this" }), &ctx)
            .await
            .unwrap();
        assert_eq!(output.result["action"], "write");
        assert_eq!(output.result["params"]["content"], "remember this");

        let read = MemoryReadHostTool::new(Arc::clone(&host));
        assert!(
            read.metadata()
                .route_intents
                .contains(&ToolRouteIntent::MemoryRecall)
        );
        let output = read
            .execute(serde_json::json!({ "path": "MEMORY.md" }), &ctx)
            .await
            .unwrap();
        assert_eq!(output.result["action"], "read");
        assert_eq!(output.result["params"]["path"], "MEMORY.md");

        let tree = MemoryTreeHostTool::new(Arc::clone(&host));
        let output = tree
            .execute(serde_json::json!({ "depth": 2 }), &ctx)
            .await
            .unwrap();
        assert_eq!(output.result["action"], "tree");
        assert_eq!(output.result["params"]["depth"], 2);

        let delete = MemoryDeleteHostTool::new(host);
        let output = delete
            .execute(serde_json::json!({ "path": "daily/today.md" }), &ctx)
            .await
            .unwrap();
        assert_eq!(output.result["action"], "delete");
        assert_eq!(output.result["params"]["path"], "daily/today.md");
    }

    #[test]
    fn session_search_params_apply_scope_sensitive_defaults() {
        assert_eq!(
            session_search_parameters_schema()["properties"]["limit"]["maximum"],
            25
        );
        let direct = parse_session_search_params(
            &serde_json::json!({ "query": "launch notes", "limit": 100 }),
            true,
            true,
        )
        .unwrap();
        assert_eq!(direct.limit, 25);
        assert!(!direct.include_current_thread);
        assert!(direct.all_channels);
        assert!(direct.summarize_sessions);

        let group =
            parse_session_search_params(&serde_json::json!({ "query": "" }), false, false).unwrap();
        assert!(group.include_current_thread);
        assert!(!group.all_channels);
        assert!(!group.summarize_sessions);
    }

    #[test]
    fn session_search_scope_uses_metadata_over_context() {
        let conversation_id = Uuid::new_v4();
        let scope = resolve_session_search_scope(
            &serde_json::json!({
                "principal_id": "metadata-principal",
                "actor_id": "metadata-actor",
                "conversation_kind": "group",
                "thread_id": conversation_id.to_string(),
            }),
            "context-principal",
            Some("context-actor"),
            None,
        );
        assert_eq!(scope.principal_id, "metadata-principal");
        assert_eq!(scope.actor_id, "metadata-actor");
        assert!(scope.include_group_history);
        assert_eq!(scope.conversation_id, Some(conversation_id));
    }

    #[test]
    fn memory_write_params_apply_defaults_and_reject_empty_content() {
        assert_eq!(memory_write_parameters_schema()["required"][0], "content");
        assert_eq!(memory_read_parameters_schema()["required"][0], "path");
        assert_eq!(
            memory_tree_parameters_schema()["properties"]["depth"]["maximum"],
            10
        );
        assert_eq!(memory_delete_parameters_schema()["required"][0], "path");

        let params = parse_memory_write_params(&serde_json::json!({
            "content": "Remember this"
        }))
        .unwrap();
        assert_eq!(params.content, "Remember this");
        assert_eq!(params.target, "daily_log");
        assert!(params.append);

        assert!(parse_memory_write_params(&serde_json::json!({ "content": "  " })).is_err());
    }

    #[test]
    fn memory_tree_params_apply_defaults_and_limits() {
        assert_eq!(parse_memory_tree_params(&serde_json::json!({})).depth, 1);
        let params = parse_memory_tree_params(&serde_json::json!({
            "path": "daily",
            "depth": 99
        }));
        assert_eq!(params.path, "daily");
        assert_eq!(params.depth, 10);
    }

    #[test]
    fn memory_delete_action_protects_identity_files() {
        assert!(resolve_memory_delete_action(paths::IDENTITY).is_err());
        assert!(resolve_memory_delete_action(paths::SOUL).is_err());
        assert_eq!(
            resolve_memory_delete_action(paths::BOOTSTRAP).unwrap(),
            MemoryDeleteAction::FinalizeBootstrap
        );
        assert_eq!(
            resolve_memory_delete_action("daily/today.md").unwrap(),
            MemoryDeleteAction::Delete {
                normalized_path: "daily/today.md"
            }
        );
    }

    #[test]
    fn memory_read_slice_extracts_requested_lines() {
        let slice = parse_memory_read_slice(&serde_json::json!({
            "start_line": 2,
            "num_lines": 2
        }));
        let (content, total_lines) = apply_memory_read_slice("one\ntwo\nthree\nfour", slice);
        assert_eq!(content, "two\nthree");
        assert_eq!(total_lines, 4);

        let slice = parse_memory_read_slice(&serde_json::json!({}));
        let (content, total_lines) = apply_memory_read_slice("one\ntwo", slice);
        assert_eq!(content, "one\ntwo");
        assert_eq!(total_lines, 2);
    }

    #[test]
    fn memory_read_and_delete_outputs_are_stable() {
        assert_eq!(memory_read_missing_output("MISSING.md")["exists"], false);

        let read = memory_read_output("MEMORY.md", "hello world", 1, Some("now".to_string()));
        assert_eq!(read["path"], "MEMORY.md");
        assert_eq!(read["word_count"], 2);
        assert_eq!(read["updated_at"], "now");

        let home_soul = memory_read_output(paths::SOUL, "hello", 1, None);
        assert!(home_soul["updated_at"].is_null());

        assert_eq!(memory_delete_output("BOOTSTRAP.md")["status"], "deleted");
    }

    #[test]
    fn memory_write_outputs_are_stable() {
        let output = memory_write_output("written", "MEMORY.md", true, 12, None);
        assert_eq!(output["status"], "written");
        assert_eq!(output["path"], "MEMORY.md");
        assert_eq!(output["append"], true);
        assert_eq!(output["content_length"], 12);
        assert!(output.get("note").is_none());

        let noted = memory_write_output("appended", "IDENTITY.md", true, 5, Some("updated"));
        assert_eq!(noted["note"], "updated");

        let payload = memory_write_mirror_payload("MEMORY.md", false, &"x".repeat(300));
        assert_eq!(payload["tool"], "memory_write");
        assert_eq!(payload["append"], false);
        assert_eq!(payload["content_preview"].as_str().unwrap().len(), 240);

        let search = memory_search_output(
            "query",
            vec![memory_search_result_entry(
                "MEMORY.md",
                "hit",
                0.7,
                "doc".to_string(),
                true,
            )],
        );
        assert_eq!(search["result_count"], 1);

        let recent = session_recent_output("query", vec![serde_json::json!({"id": 1})]);
        assert_eq!(recent["summarized"], false);

        let searched = session_search_output("query", vec![], true, true);
        assert_eq!(searched["summarized"], true);
        assert_eq!(searched["fallback"], true);
    }
}
