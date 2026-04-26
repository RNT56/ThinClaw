//! Proactive heartbeat system for periodic execution.
//!
//! The heartbeat runner provides a standalone heartbeat mode that executes
//! periodically (default: every 30 minutes). In the typical Tauri integration,
//! heartbeats are executed via [`crate::agent::routine_engine::RoutineEngine`]
//! which is the primary path. This module is used as a fallback or in CLI mode.
//!
//! # Usage
//!
//! Create a HEARTBEAT.md in the workspace with a checklist of things to monitor:
//!
//! ```markdown
//! # Heartbeat Checklist
//!
//! - [ ] Check for unread emails
//! - [ ] Review calendar for upcoming events
//! - [ ] Check project build status
//! ```
//!
//! The agent will process this checklist on each heartbeat and only notify
//! if action is needed.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::channels::OutgoingResponse;
use crate::db::Database;
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider, Reasoning};
use crate::safety::SafetyLayer;
use crate::workspace::Workspace;
use crate::workspace::hygiene::HygieneConfig;

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
            interval: Duration::from_secs(30 * 60), // 30 minutes
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

/// Heartbeat runner for proactive periodic execution.
pub struct HeartbeatRunner {
    config: HeartbeatConfig,
    hygiene_config: HygieneConfig,
    workspace: Arc<Workspace>,
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    response_tx: Option<mpsc::Sender<OutgoingResponse>>,
    consecutive_failures: u32,
    store: Option<Arc<dyn Database>>,
    outcome_user_id: Option<String>,
    /// Shared cost tracker so heartbeat LLM calls appear in the Cost Dashboard.
    cost_tracker: Option<Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>>,
}

impl HeartbeatRunner {
    /// Create a new heartbeat runner.
    pub fn new(
        config: HeartbeatConfig,
        hygiene_config: HygieneConfig,
        workspace: Arc<Workspace>,
        llm: Arc<dyn LlmProvider>,
        safety: Arc<SafetyLayer>,
    ) -> Self {
        Self {
            config,
            hygiene_config,
            workspace,
            llm,
            safety,
            response_tx: None,
            consecutive_failures: 0,
            store: None,
            outcome_user_id: None,
            cost_tracker: None,
        }
    }

    /// Set the response channel for notifications.
    pub fn with_response_channel(mut self, tx: mpsc::Sender<OutgoingResponse>) -> Self {
        self.response_tx = Some(tx);
        self
    }

    /// Attach a shared cost tracker so heartbeat LLM calls are recorded.
    pub fn with_cost_tracker(
        mut self,
        tracker: Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>,
    ) -> Self {
        self.cost_tracker = Some(tracker);
        self
    }

    /// Attach DB context for outcome-review summaries in standalone heartbeat mode.
    pub fn with_outcome_context(
        mut self,
        store: Arc<dyn Database>,
        user_id: impl Into<String>,
    ) -> Self {
        self.store = Some(store);
        self.outcome_user_id = Some(user_id.into());
        self
    }

    /// Run the heartbeat loop.
    ///
    /// This runs forever, checking periodically based on the configured interval.
    pub async fn run(&mut self) {
        if !self.config.enabled {
            tracing::info!("Heartbeat is disabled, not starting loop");
            return;
        }

        tracing::info!(
            "Starting heartbeat loop with interval {:?}",
            self.config.interval
        );

        let mut interval = tokio::time::interval(self.config.interval);
        // Don't run immediately on startup
        interval.tick().await;

        loop {
            interval.tick().await;

            // Run memory hygiene in the background so it never delays the
            // heartbeat checklist. Failures are logged inside run_if_due.
            let hygiene_workspace = Arc::clone(&self.workspace);
            let hygiene_config = self.hygiene_config.clone();
            tokio::spawn(async move {
                let report =
                    crate::workspace::hygiene::run_if_due(&hygiene_workspace, &hygiene_config)
                        .await;
                if report.had_work() {
                    tracing::info!(
                        daily_logs_deleted = report.daily_logs_deleted,
                        "heartbeat: memory hygiene deleted stale documents"
                    );
                }
            });

            match self.check_heartbeat().await {
                HeartbeatResult::Ok => {
                    tracing::debug!("Heartbeat OK");
                    self.consecutive_failures = 0;
                }
                HeartbeatResult::NeedsAttention(message) => {
                    tracing::info!("Heartbeat needs attention: {}", message);
                    self.consecutive_failures = 0;
                    self.send_notification(&message).await;
                }
                HeartbeatResult::Skipped => {
                    tracing::debug!("Heartbeat skipped");
                }
                HeartbeatResult::Failed(error) => {
                    tracing::error!("Heartbeat failed: {}", error);
                    self.consecutive_failures += 1;

                    if self.consecutive_failures >= self.config.max_failures {
                        tracing::error!(
                            "Heartbeat disabled after {} consecutive failures",
                            self.consecutive_failures
                        );
                        break;
                    }
                }
            }
        }
    }

    /// Run a single heartbeat check.
    pub async fn check_heartbeat(&self) -> HeartbeatResult {
        // Get the heartbeat checklist
        let checklist = match self.workspace.heartbeat_checklist().await {
            Ok(Some(content)) if !is_effectively_empty(&content) => content,
            Ok(_) => return HeartbeatResult::Skipped,
            Err(e) => return HeartbeatResult::Failed(format!("Failed to read checklist: {}", e)),
        };

        // ── Read daily logs so the LLM has real context ─────────────────
        // Without this, the agent can't evaluate checklist items that
        // reference "daily logs" or "recent activity". We inject today's
        // and yesterday's logs (capped to avoid token explosion).
        // IC-013: Use shared function to build daily log context
        let daily_context = build_daily_context(&self.workspace).await;

        // Build the heartbeat prompt
        let logs_note = if daily_context.is_empty() {
            "\n\nNote: No daily logs exist yet (no conversations recorded). \
             Any checklist items that reference daily logs are automatically satisfied. \
             If all items depend on daily logs, reply HEARTBEAT_OK."
        } else {
            ""
        };
        let outcome_summary = match (&self.store, &self.outcome_user_id) {
            (Some(store), Some(user_id)) => {
                match crate::agent::outcomes::heartbeat_review_summary(store, user_id).await {
                    Ok(Some(summary)) => format!("\n\n## {}\n", summary),
                    _ => String::new(),
                }
            }
            _ => String::new(),
        };

        let prompt = format!(
            "Read the HEARTBEAT.md checklist below and follow it strictly. \
             Do not infer or repeat old tasks. Check each item and report findings.\n\
             \n\
             If nothing needs attention, reply EXACTLY with: HEARTBEAT_OK\n\
             \n\
             If something needs attention, provide a short, specific summary of what \
             needs action. Do NOT echo these instructions back — give real findings only.\n\
             \n\
             ## HEARTBEAT.md\n\
             \n\
             {}{}{}{}",
            checklist, daily_context, outcome_summary, logs_note
        );

        // Get the system prompt for context
        let system_prompt = match self.workspace.system_prompt().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to get system prompt for heartbeat: {}", e);
                String::new()
            }
        };

        // Run the agent turn
        let messages = if system_prompt.is_empty() {
            vec![ChatMessage::user(&prompt)]
        } else {
            vec![
                ChatMessage::system(&system_prompt),
                ChatMessage::user(&prompt),
            ]
        };

        // Use a modest max_tokens for heartbeat responses. Heartbeats are
        // short status checks — they never need more than a few hundred tokens.
        // Previously used ctx/2 which was wasteful on large-context models (Bug 27).
        let max_tokens = match self.llm.model_metadata().await {
            Ok(meta) => {
                // Use at most 2048, but respect smaller models
                let from_api = meta.context_length.map(|ctx| ctx / 4).unwrap_or(2048);
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

        let mut reasoning = Reasoning::new(self.llm.clone(), self.safety.clone());
        if let Some(ref tracker) = self.cost_tracker {
            reasoning = reasoning.with_cost_tracker(std::sync::Arc::clone(tracker));
        }
        let (content, _usage) = match reasoning.complete(request).await {
            Ok(r) => r,
            Err(e) => return HeartbeatResult::Failed(format!("LLM call failed: {}", e)),
        };

        let content = content.trim();

        // Guard against empty content. Reasoning models (e.g. GLM-4.7) may
        // burn all output tokens on chain-of-thought and return content: null.
        if content.is_empty() {
            return HeartbeatResult::Failed("LLM returned empty content.".to_string());
        }

        // Check if nothing needs attention
        if content == "HEARTBEAT_OK" || content.contains("HEARTBEAT_OK") {
            return HeartbeatResult::Ok;
        }

        HeartbeatResult::NeedsAttention(content.to_string())
    }

    /// Send a notification about heartbeat findings.
    async fn send_notification(&self, message: &str) {
        let Some(ref tx) = self.response_tx else {
            tracing::debug!("No response channel configured for heartbeat notifications");
            return;
        };

        let response = OutgoingResponse {
            content: format!("🔔 *Heartbeat Alert*\n\n{}", message),
            thread_id: None,
            metadata: serde_json::json!({
                "source": "heartbeat",
            }),
            attachments: Vec::new(),
        };

        if let Err(e) = tx.send(response).await {
            tracing::error!("Failed to send heartbeat notification: {}", e);
        }
    }
}

/// Check if heartbeat content is effectively empty.
///
/// Returns true if the content contains only:
/// - Whitespace
/// - Markdown headers (lines starting with #)
/// - HTML comments (`<!-- ... -->`)
/// - Empty list items (`- [ ]`, `- [x]`, `-`, `*`)
///
/// This skips the LLM call when the user hasn't added real tasks yet,
/// saving API costs.
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
            None => return result, // unclosed comment, treat rest as comment
        }
    }
    result.push_str(rest);
    result
}

/// Cap a daily log to approximately `max_bytes`, truncating on a line boundary.
///
/// Keeps the most recent entries (end of file) since daily logs are
/// append-only with newest entries last.
pub fn cap_daily_log(content: &str, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }
    // Find the nearest char boundary at or after (len - max_bytes) so we
    // never slice into a multi-byte UTF-8 character.
    let target_start = content.len() - max_bytes;
    let safe_start = content
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| i >= target_start)
        .unwrap_or(content.len());
    let tail = &content[safe_start..];
    // Find the first newline to start on a clean line boundary
    match tail.find('\n') {
        Some(idx) => format!("[...truncated...]\n{}", &tail[idx + 1..]),
        None => format!("[...truncated...]\n{}", tail),
    }
}

/// IC-013: Shared daily log context builder for heartbeat.
///
/// Reads today's and yesterday's daily logs from the workspace, caps them
/// to avoid token explosion, and returns a formatted string.
pub async fn build_daily_context(workspace: &crate::workspace::Workspace) -> String {
    let mut daily_context = String::new();
    let today = workspace.local_today();

    if let Ok(doc) = workspace.today_log().await
        && !doc.content.trim().is_empty()
    {
        let capped = cap_daily_log(&doc.content, 3000);
        daily_context.push_str(&format!(
            "\n\n## Daily Log — {} (today)\n\n{}",
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
            "\n\n## Daily Log — {} (yesterday)\n\n{}",
            yesterday.format("%Y-%m-%d"),
            capped
        ));
    }

    daily_context
}

/// Spawn the heartbeat runner as a background task.
///
/// Returns a handle that can be used to stop the runner.
pub fn spawn_heartbeat(
    config: HeartbeatConfig,
    hygiene_config: HygieneConfig,
    workspace: Arc<Workspace>,
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    response_tx: Option<mpsc::Sender<OutgoingResponse>>,
    cost_tracker: Option<Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>>,
) -> tokio::task::JoinHandle<()> {
    let mut runner = HeartbeatRunner::new(config, hygiene_config, workspace, llm, safety);
    if let Some(tx) = response_tx {
        runner = runner.with_response_channel(tx);
    }
    if let Some(tracker) = cost_tracker {
        runner = runner.with_cost_tracker(tracker);
    }

    tokio::spawn(async move {
        runner.run().await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ==================== strip_html_comments ====================

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

    // ==================== is_effectively_empty ====================

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
