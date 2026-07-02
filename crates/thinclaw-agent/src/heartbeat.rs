//! Proactive heartbeat runner and prompt assembly.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use thinclaw_llm_core::{ChatMessage, CompletionRequest};
use thinclaw_workspace::Workspace;
use thinclaw_workspace::hygiene::HygieneConfig;

/// Configuration for the heartbeat runner.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Interval between heartbeat checks.
    pub interval: Duration,
    /// Whether heartbeat is enabled.
    pub enabled: bool,
    /// Maximum consecutive failures before disabling.
    pub max_failures: u32,
    /// User ID to notify on heartbeat findings.
    pub notify_user_id: Option<String>,
    /// Channel to notify on heartbeat findings.
    pub notify_channel: Option<String>,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30 * 60),
            enabled: true,
            max_failures: 3,
            notify_user_id: None,
            notify_channel: None,
        }
    }
}

impl HeartbeatConfig {
    /// Create a config with a specific interval.
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Disable heartbeat.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Set the notification target.
    pub fn with_notify(mut self, user_id: impl Into<String>, channel: impl Into<String>) -> Self {
        self.notify_user_id = Some(user_id.into());
        self.notify_channel = Some(channel.into());
        self
    }
}

/// Result of a heartbeat check.
#[derive(Debug)]
pub enum HeartbeatResult {
    /// Nothing needs attention.
    Ok,
    /// Something needs attention, with the message to send.
    NeedsAttention(String),
    /// Heartbeat was skipped (no checklist or disabled).
    Skipped,
    /// Heartbeat failed.
    Failed(String),
}

/// LLM behavior required by heartbeat.
#[async_trait]
pub trait HeartbeatLlmPort: Send + Sync {
    /// Return the active model context length if the provider exposes it.
    async fn context_length(&self) -> Result<Option<u32>, String>;

    /// Complete the heartbeat request and return cleaned content.
    async fn complete_heartbeat(&self, request: CompletionRequest) -> Result<String, String>;
}

/// Optional source for outcome-review context injected into standalone heartbeats.
#[async_trait]
pub trait HeartbeatOutcomeSummaryPort: Send + Sync {
    async fn heartbeat_review_summary(&self) -> Result<Option<String>, String>;
}

/// Heartbeat runner for proactive periodic execution.
pub struct HeartbeatRunner {
    enabled: bool,
    workspace: Arc<Workspace>,
    llm: Arc<dyn HeartbeatLlmPort>,
    outcome_summary: Option<Arc<dyn HeartbeatOutcomeSummaryPort>>,
}

impl HeartbeatRunner {
    /// Create a new heartbeat runner.
    ///
    /// `config.enabled` is honored by [`Self::check_heartbeat`], which
    /// short-circuits to [`HeartbeatResult::Skipped`] without calling the
    /// LLM when the caller has disabled heartbeat.
    ///
    /// `config.interval` and `config.max_failures` are not meaningful at
    /// this layer: this runner performs a single synchronous check per
    /// call rather than owning a polling loop, and it holds no state across
    /// calls to count consecutive failures. Callers that need interval-based
    /// scheduling or failure-count-based backoff must implement that above
    /// this runner (see the routine engine's own heartbeat scheduling path).
    ///
    /// `config.notify_user_id` / `config.notify_channel` are also not used
    /// here: this runner has no delivery port and only returns a
    /// [`HeartbeatResult`] to the caller, which owns response delivery.
    ///
    /// `hygiene_config` is accepted for call-site/API compatibility with
    /// callers that resolve heartbeat and hygiene configuration together,
    /// but hygiene maintenance is a separate workspace subsystem
    /// (`thinclaw_workspace::hygiene::run_if_due`) and is not invoked by
    /// this runner.
    pub fn new(
        config: HeartbeatConfig,
        _hygiene_config: HygieneConfig,
        workspace: Arc<Workspace>,
        llm: Arc<dyn HeartbeatLlmPort>,
    ) -> Self {
        Self {
            enabled: config.enabled,
            workspace,
            llm,
            outcome_summary: None,
        }
    }

    /// Attach outcome-review context for standalone heartbeat mode.
    pub fn with_outcome_summary(mut self, summary: Arc<dyn HeartbeatOutcomeSummaryPort>) -> Self {
        self.outcome_summary = Some(summary);
        self
    }

    /// Run a single heartbeat check.
    ///
    /// Returns [`HeartbeatResult::Skipped`] immediately if the runner was
    /// constructed with `HeartbeatConfig { enabled: false, .. }`.
    pub async fn check_heartbeat(&self) -> HeartbeatResult {
        if !self.enabled {
            return HeartbeatResult::Skipped;
        }

        let checklist = match self.workspace.heartbeat_checklist().await {
            Ok(Some(content)) if !is_effectively_empty(&content) => content,
            Ok(_) => return HeartbeatResult::Skipped,
            Err(e) => return HeartbeatResult::Failed(format!("Failed to read checklist: {}", e)),
        };

        let daily_context = build_daily_context(&self.workspace).await;
        let logs_note = if daily_context.is_empty() {
            "\n\nNote: No daily logs exist yet (no conversations recorded). \
             Any checklist items that reference daily logs are automatically satisfied. \
             If all items depend on daily logs, reply HEARTBEAT_OK."
        } else {
            ""
        };
        let outcome_summary = match &self.outcome_summary {
            Some(summary) => match summary.heartbeat_review_summary().await {
                Ok(Some(summary)) => format!("\n\n## {}\n", summary),
                _ => String::new(),
            },
            None => String::new(),
        };

        let prompt = format!(
            "Read the HEARTBEAT.md checklist below and follow it strictly. \
             Do not infer or repeat old tasks. Check each item and report findings.\n\
             \n\
             If nothing needs attention, reply EXACTLY with: HEARTBEAT_OK\n\
             \n\
             If something needs attention, provide a short, specific summary of what \
             needs action. Do NOT echo these instructions back - give real findings only.\n\
             \n\
             ## HEARTBEAT.md\n\
             \n\
             {}{}{}{}",
            checklist, daily_context, outcome_summary, logs_note
        );

        let system_prompt = match self.workspace.system_prompt().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to get system prompt for heartbeat: {}", e);
                String::new()
            }
        };

        let messages = if system_prompt.is_empty() {
            vec![ChatMessage::user(&prompt)]
        } else {
            vec![
                ChatMessage::system(&system_prompt),
                ChatMessage::user(&prompt),
            ]
        };

        let max_tokens = match self.llm.context_length().await {
            Ok(context_length) => {
                let from_api = context_length.map(|ctx| ctx / 4).unwrap_or(2048);
                from_api.clamp(512, 2048)
            }
            Err(e) => {
                tracing::warn!(
                    "Could not fetch model metadata, using default max_tokens: {}",
                    e
                );
                2048
            }
        };

        let request = CompletionRequest::new(messages)
            .with_max_tokens(max_tokens)
            .with_temperature(0.3);

        let content = match self.llm.complete_heartbeat(request).await {
            Ok(content) => content,
            Err(e) => return HeartbeatResult::Failed(format!("LLM call failed: {}", e)),
        };

        let content = content.trim();
        if content.is_empty() {
            return HeartbeatResult::Failed("LLM returned empty content.".to_string());
        }

        if content == "HEARTBEAT_OK" || content.contains("HEARTBEAT_OK") {
            return HeartbeatResult::Ok;
        }

        HeartbeatResult::NeedsAttention(content.to_string())
    }
}

/// Check if heartbeat content is effectively empty.
pub fn is_effectively_empty(content: &str) -> bool {
    let without_comments = strip_html_comments(content);

    without_comments.lines().all(|line| {
        let trimmed = line.trim();
        trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed == "- [ ]"
            || trimmed == "- [x]"
            || trimmed == "-"
            || trimmed == "*"
    })
}

/// Remove HTML comments from content.
pub fn strip_html_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("<!--") {
        result.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return result,
        }
    }
    result.push_str(rest);
    result
}

/// Cap a daily log to approximately `max_bytes`, truncating on a line boundary.
pub fn cap_daily_log(content: &str, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }
    let target_start = content.len() - max_bytes;
    let safe_start = content
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| i >= target_start)
        .unwrap_or(content.len());
    let tail = &content[safe_start..];
    match tail.find('\n') {
        Some(idx) => format!("[...truncated...]\n{}", &tail[idx + 1..]),
        None => format!("[...truncated...]\n{}", tail),
    }
}

/// Build heartbeat daily-log context from today's and yesterday's logs.
pub async fn build_daily_context(workspace: &Workspace) -> String {
    let mut daily_context = String::new();
    let today = workspace.local_today();

    if let Ok(doc) = workspace.today_log().await
        && !doc.content.trim().is_empty()
    {
        let capped = cap_daily_log(&doc.content, 3000);
        daily_context.push_str(&format!(
            "\n\n## Daily Log - {} (today)\n\n{}",
            today.format("%Y-%m-%d"),
            capped
        ));
    }

    if let Some(yesterday) = today.pred_opt()
        && let Ok(doc) = workspace.daily_log(yesterday).await
        && !doc.content.trim().is_empty()
    {
        let capped = cap_daily_log(&doc.content, 2000);
        daily_context.push_str(&format!(
            "\n\n## Daily Log - {} (yesterday)\n\n{}",
            yesterday.format("%Y-%m-%d"),
            capped
        ));
    }

    daily_context
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use thinclaw_types::error::WorkspaceError;
    use thinclaw_workspace::{
        MemoryChunk, MemoryDocument, SearchConfig, SearchResult, WorkspaceEntry, WorkspaceStore,
    };
    use uuid::Uuid;

    /// Minimal `WorkspaceStore` stub. Every method panics if called, since
    /// the tests that use it only exercise the disabled-heartbeat
    /// short-circuit, which must return before touching the workspace.
    struct UnreachableWorkspaceStore;

    #[async_trait]
    impl WorkspaceStore for UnreachableWorkspaceStore {
        async fn get_document_by_path(
            &self,
            _user_id: &str,
            _agent_id: Option<Uuid>,
            _path: &str,
        ) -> Result<MemoryDocument, WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }

        async fn get_document_by_id(&self, _id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }

        async fn get_or_create_document_by_path(
            &self,
            _user_id: &str,
            _agent_id: Option<Uuid>,
            _path: &str,
        ) -> Result<MemoryDocument, WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }

        async fn update_document(&self, _id: Uuid, _content: &str) -> Result<(), WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }

        async fn delete_document_by_path(
            &self,
            _user_id: &str,
            _agent_id: Option<Uuid>,
            _path: &str,
        ) -> Result<(), WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }

        async fn list_directory(
            &self,
            _user_id: &str,
            _agent_id: Option<Uuid>,
            _directory: &str,
        ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }

        async fn list_all_paths(
            &self,
            _user_id: &str,
            _agent_id: Option<Uuid>,
        ) -> Result<Vec<String>, WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }

        async fn list_documents(
            &self,
            _user_id: &str,
            _agent_id: Option<Uuid>,
        ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }

        async fn delete_chunks(&self, _document_id: Uuid) -> Result<(), WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }

        async fn insert_chunk(
            &self,
            _document_id: Uuid,
            _chunk_index: i32,
            _content: &str,
            _embedding: Option<&[f32]>,
        ) -> Result<Uuid, WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }

        async fn update_chunk_embedding(
            &self,
            _chunk_id: Uuid,
            _embedding: &[f32],
        ) -> Result<(), WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }

        async fn get_chunks_without_embeddings(
            &self,
            _user_id: &str,
            _agent_id: Option<Uuid>,
            _limit: usize,
        ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }

        async fn hybrid_search(
            &self,
            _user_id: &str,
            _agent_id: Option<Uuid>,
            _query: &str,
            _embedding: Option<&[f32]>,
            _config: &SearchConfig,
        ) -> Result<Vec<SearchResult>, WorkspaceError> {
            unreachable!("disabled heartbeat must not touch the workspace store")
        }
    }

    struct UnreachableLlm;

    #[async_trait]
    impl HeartbeatLlmPort for UnreachableLlm {
        async fn context_length(&self) -> Result<Option<u32>, String> {
            unreachable!("disabled heartbeat must not call the LLM port")
        }

        async fn complete_heartbeat(&self, _request: CompletionRequest) -> Result<String, String> {
            unreachable!("disabled heartbeat must not call the LLM port")
        }
    }

    #[tokio::test]
    async fn disabled_config_short_circuits_without_touching_workspace_or_llm() {
        let workspace = Arc::new(Workspace::new_with_store(
            "test-user",
            Arc::new(UnreachableWorkspaceStore),
        ));
        let llm: Arc<dyn HeartbeatLlmPort> = Arc::new(UnreachableLlm);

        let runner = HeartbeatRunner::new(
            HeartbeatConfig::default().disabled(),
            HygieneConfig::default(),
            workspace,
            llm,
        );

        let result = runner.check_heartbeat().await;
        assert!(matches!(result, HeartbeatResult::Skipped));
    }

    #[tokio::test]
    async fn enabled_config_does_not_short_circuit() {
        // Sanity check for the inverse: an enabled runner proceeds far
        // enough to reach the workspace store (and hits our stub's
        // `unreachable!`), proving the short-circuit is gated on
        // `enabled` specifically rather than always skipping.
        let workspace = Arc::new(Workspace::new_with_store(
            "test-user",
            Arc::new(UnreachableWorkspaceStore),
        ));
        let llm: Arc<dyn HeartbeatLlmPort> = Arc::new(UnreachableLlm);

        let runner = HeartbeatRunner::new(
            HeartbeatConfig::default(),
            HygieneConfig::default(),
            workspace,
            llm,
        );

        let outcome = std::panic::AssertUnwindSafe(runner.check_heartbeat())
            .catch_unwind()
            .await;
        assert!(
            outcome.is_err(),
            "expected enabled runner to reach the workspace store stub"
        );
    }

    #[test]
    fn test_heartbeat_config_defaults() {
        let config = HeartbeatConfig::default();
        assert!(config.enabled);
        assert_eq!(config.interval, Duration::from_secs(30 * 60));
        assert_eq!(config.max_failures, 3);
    }

    #[test]
    fn test_heartbeat_config_builders() {
        let config = HeartbeatConfig::default()
            .with_interval(Duration::from_secs(60))
            .with_notify("user1", "telegram");

        assert_eq!(config.interval, Duration::from_secs(60));
        assert_eq!(config.notify_user_id, Some("user1".to_string()));
        assert_eq!(config.notify_channel, Some("telegram".to_string()));

        let disabled = HeartbeatConfig::default().disabled();
        assert!(!disabled.enabled);
    }

    #[test]
    fn test_strip_html_comments_no_comments() {
        assert_eq!(strip_html_comments("hello world"), "hello world");
    }

    #[test]
    fn test_strip_html_comments_single() {
        assert_eq!(
            strip_html_comments("before<!-- gone -->after"),
            "beforeafter"
        );
    }

    #[test]
    fn test_strip_html_comments_multiple() {
        let input = "a<!-- 1 -->b<!-- 2 -->c";
        assert_eq!(strip_html_comments(input), "abc");
    }

    #[test]
    fn test_strip_html_comments_multiline() {
        let input = "# Title\n<!-- multi\nline\ncomment -->\nreal content";
        assert_eq!(strip_html_comments(input), "# Title\n\nreal content");
    }

    #[test]
    fn test_strip_html_comments_unclosed() {
        let input = "before<!-- never closed";
        assert_eq!(strip_html_comments(input), "before");
    }

    #[test]
    fn test_effectively_empty_empty_string() {
        assert!(is_effectively_empty(""));
    }

    #[test]
    fn test_effectively_empty_whitespace() {
        assert!(is_effectively_empty("   \n\n  \n  "));
    }

    #[test]
    fn test_effectively_empty_headers_only() {
        assert!(is_effectively_empty("# Title\n## Subtitle\n### Section"));
    }

    #[test]
    fn test_effectively_empty_html_comments_only() {
        assert!(is_effectively_empty("<!-- this is a comment -->"));
    }

    #[test]
    fn test_effectively_empty_empty_checkboxes() {
        assert!(is_effectively_empty("# Checklist\n- [ ]\n- [x]"));
    }

    #[test]
    fn test_effectively_empty_bare_list_markers() {
        assert!(is_effectively_empty("-\n*\n-"));
    }

    #[test]
    fn test_effectively_empty_seeded_template() {
        let template = "\
# Heartbeat Checklist

<!-- Keep this file empty to skip heartbeat API calls.
     Add tasks below when you want the agent to check something periodically.

     Example:
     - [ ] Check for unread emails needing a reply
     - [ ] Review today's calendar for upcoming meetings
     - [ ] Check CI build status for main branch
-->";
        assert!(is_effectively_empty(template));
    }

    #[test]
    fn test_effectively_empty_real_checklist() {
        let content = "\
# Heartbeat Checklist

- [ ] Check for unread emails needing a reply
- [ ] Review today's calendar for upcoming meetings";
        assert!(!is_effectively_empty(content));
    }

    #[test]
    fn test_effectively_empty_mixed_real_and_headers() {
        let content = "# Title\n\nDo something important";
        assert!(!is_effectively_empty(content));
    }

    #[test]
    fn test_effectively_empty_comment_plus_real_content() {
        let content = "<!-- comment -->\nActual task here";
        assert!(!is_effectively_empty(content));
    }
}
