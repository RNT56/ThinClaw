//! Core types for the routines system.
//!
//! A routine is a named, persistent, user-owned task with a trigger and an action.
//! Each routine fires independently when its trigger condition is met, with only
//! that routine's prompt and context sent to the LLM.
//!
//! ```text
//! ┌──────────┐     ┌─────────┐     ┌──────────────────┐
//! │  Trigger  │────▶│ Engine  │────▶│  Execution Mode  │
//! │ cron/event│     │guardrail│     │lightweight│full_job│
//! │ webhook   │     │ check   │     └──────────────────┘
//! │ manual    │     └─────────┘              │
//! └──────────┘                               ▼
//!                                     ┌──────────────┐
//!                                     │  Notify user │
//!                                     │  if needed   │
//!                                     └──────────────┘
//! ```

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::RoutineError;

/// A routine is a named, persistent, user-owned task with a trigger and an action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routine {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub user_id: String,
    #[serde(default)]
    pub actor_id: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub action: RoutineAction,
    pub guardrails: RoutineGuardrails,
    pub notify: NotifyConfig,

    // Runtime state (DB-managed)
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_fire_at: Option<DateTime<Utc>>,
    pub run_count: u64,
    pub consecutive_failures: u32,
    pub state: serde_json::Value,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Routine {
    /// Resolve the owning actor for this routine.
    pub fn owner_actor_id(&self) -> &str {
        if self.actor_id.is_empty() {
            &self.user_id
        } else {
            &self.actor_id
        }
    }
}

/// When a routine should fire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    /// Fire on a cron schedule (e.g. "0 9 * * MON-FRI" or "every 2h").
    Cron { schedule: String },
    /// Fire when a channel message matches a pattern.
    Event {
        /// Optional channel filter (e.g. "telegram", "slack").
        channel: Option<String>,
        /// Regex pattern to match against message content.
        pattern: String,
    },
    /// Fire on incoming webhook POST to /hooks/routine/{id}.
    Webhook {
        /// Optional webhook path suffix (defaults to routine id).
        path: Option<String>,
        /// Optional shared secret for HMAC validation.
        secret: Option<String>,
    },
    /// Only fires via tool call or CLI.
    Manual,
    /// System event: when this trigger fires (via cron), it injects a message
    /// into the heartbeat's system event queue. The heartbeat picks up the
    /// message on its next tick. This enables "check X at 9am" patterns.
    SystemEvent {
        /// The message to inject into the heartbeat queue.
        message: String,
        /// Optional cron schedule for when to inject the event.
        #[serde(default)]
        schedule: Option<String>,
    },
}

impl Trigger {
    /// The string tag stored in the DB trigger_type column.
    pub fn type_tag(&self) -> &'static str {
        match self {
            Trigger::Cron { .. } => "cron",
            Trigger::Event { .. } => "event",
            Trigger::Webhook { .. } => "webhook",
            Trigger::Manual => "manual",
            Trigger::SystemEvent { .. } => "system_event",
        }
    }

    /// Parse a trigger from its DB representation.
    pub fn from_db(trigger_type: &str, config: serde_json::Value) -> Result<Self, RoutineError> {
        match trigger_type {
            "cron" => {
                let schedule = config
                    .get("schedule")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "cron trigger".into(),
                        field: "schedule".into(),
                    })?
                    .to_string();
                Ok(Trigger::Cron { schedule })
            }
            "event" => {
                let pattern = config
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "event trigger".into(),
                        field: "pattern".into(),
                    })?
                    .to_string();
                let channel = config
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                Ok(Trigger::Event { channel, pattern })
            }
            "webhook" => {
                let path = config
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let secret = config
                    .get("secret")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                Ok(Trigger::Webhook { path, secret })
            }
            "manual" => Ok(Trigger::Manual),
            "system_event" => {
                let message = config
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "system_event trigger".into(),
                        field: "message".into(),
                    })?
                    .to_string();
                let schedule = config
                    .get("schedule")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                Ok(Trigger::SystemEvent { message, schedule })
            }
            other => Err(RoutineError::UnknownTriggerType {
                trigger_type: other.to_string(),
            }),
        }
    }

    /// Serialize trigger-specific config to JSON for DB storage.
    pub fn to_config_json(&self) -> serde_json::Value {
        match self {
            Trigger::Cron { schedule } => serde_json::json!({ "schedule": schedule }),
            Trigger::Event { channel, pattern } => serde_json::json!({
                "pattern": pattern,
                "channel": channel,
            }),
            Trigger::Webhook { path, secret } => serde_json::json!({
                "path": path,
                "secret": secret,
            }),
            Trigger::Manual => serde_json::json!({}),
            Trigger::SystemEvent { message, schedule } => serde_json::json!({
                "message": message,
                "schedule": schedule,
            }),
        }
    }
}

/// What happens when a routine fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoutineAction {
    /// Single LLM call, no tools. Cheap and fast.
    Lightweight {
        /// The prompt sent to the LLM.
        prompt: String,
        /// Workspace paths to load as context (e.g. ["context/priorities.md"]).
        #[serde(default)]
        context_paths: Vec<String>,
        /// Max output tokens (default: 4096).
        #[serde(default = "default_max_tokens")]
        max_tokens: u32,
    },
    /// Full multi-turn worker job with tool access.
    FullJob {
        /// Job title for the scheduler.
        title: String,
        /// Job description / initial prompt.
        description: String,
        /// Max reasoning iterations (default: 10).
        #[serde(default = "default_max_iterations")]
        max_iterations: u32,
        /// Optional tool allowlist for this routine's worker/subagent.
        #[serde(default)]
        allowed_tools: Option<Vec<String>>,
        /// Optional skill allowlist for this routine's worker/subagent.
        #[serde(default)]
        allowed_skills: Option<Vec<String>>,
    },
    /// Periodic heartbeat: reads HEARTBEAT.md and runs a full agent turn.
    ///
    /// When `light_context` is true, runs as an isolated worker job with
    /// only HEARTBEAT.md + daily logs as context (cheap, no session history).
    /// When false, injects into the main session for full conversational
    /// context and tool access within that session.
    Heartbeat {
        /// When true, run in isolation with only HEARTBEAT.md context.
        /// When false, inject into the main session for full context.
        #[serde(default = "default_true")]
        light_context: bool,
        /// Custom heartbeat prompt body. None = default prompt.
        #[serde(default)]
        prompt: Option<String>,
        /// Include LLM reasoning chain in the output.
        #[serde(default)]
        include_reasoning: bool,
        /// Start hour of active window (0-23, local). None = always active.
        #[serde(default)]
        active_start_hour: Option<u8>,
        /// End hour of active window (0-23, local). None = always active.
        #[serde(default)]
        active_end_hour: Option<u8>,
        /// Output target: "chat" | "none" | channel name.
        #[serde(default = "default_heartbeat_target")]
        target: String,
        /// Maximum tool iterations for this heartbeat run.
        #[serde(default = "default_max_iterations")]
        max_iterations: u32,
    },
    /// Start or resume an experiment campaign.
    ExperimentCampaign {
        /// Experiment project to run.
        project_id: Uuid,
        /// Optional runner profile override.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        runner_profile_id: Option<Uuid>,
        /// Optional max trials override.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_trials_override: Option<u32>,
    },
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_max_iterations() -> u32 {
    10
}

fn default_true() -> bool {
    true
}

fn default_heartbeat_target() -> String {
    "chat".to_string()
}

impl RoutineAction {
    /// The string tag stored in the DB action_type column.
    pub fn type_tag(&self) -> &'static str {
        match self {
            RoutineAction::Lightweight { .. } => "lightweight",
            RoutineAction::FullJob { .. } => "full_job",
            RoutineAction::Heartbeat { .. } => "heartbeat",
            RoutineAction::ExperimentCampaign { .. } => "experiment_campaign",
        }
    }

    /// Parse an action from its DB representation.
    pub fn from_db(action_type: &str, config: serde_json::Value) -> Result<Self, RoutineError> {
        match action_type {
            "lightweight" => {
                let prompt = config
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "lightweight action".into(),
                        field: "prompt".into(),
                    })?
                    .to_string();
                let context_paths = config
                    .get("context_paths")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let max_tokens = config
                    .get("max_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(default_max_tokens() as u64) as u32;
                Ok(RoutineAction::Lightweight {
                    prompt,
                    context_paths,
                    max_tokens,
                })
            }
            "full_job" => {
                let title = config
                    .get("title")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "full_job action".into(),
                        field: "title".into(),
                    })?
                    .to_string();
                let description = config
                    .get("description")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "full_job action".into(),
                        field: "description".into(),
                    })?
                    .to_string();
                let max_iterations = config
                    .get("max_iterations")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(default_max_iterations() as u64)
                    as u32;
                let allowed_tools = config
                    .get("allowed_tools")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let allowed_skills = config
                    .get("allowed_skills")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                Ok(RoutineAction::FullJob {
                    title,
                    description,
                    max_iterations,
                    allowed_tools,
                    allowed_skills,
                })
            }
            "heartbeat" => {
                let light_context = config
                    .get("light_context")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let prompt = config
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let include_reasoning = config
                    .get("include_reasoning")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let active_start_hour = config
                    .get("active_start_hour")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u8);
                let active_end_hour = config
                    .get("active_end_hour")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u8);
                let target = config
                    .get("target")
                    .and_then(|v| v.as_str())
                    .unwrap_or("chat")
                    .to_string();
                Ok(RoutineAction::Heartbeat {
                    light_context,
                    prompt,
                    include_reasoning,
                    active_start_hour,
                    active_end_hour,
                    target,
                    max_iterations: config
                        .get("max_iterations")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(10) as u32,
                })
            }
            "experiment_campaign" => {
                let project_id = config
                    .get("project_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RoutineError::MissingField {
                        context: "experiment_campaign action".into(),
                        field: "project_id".into(),
                    })?;
                Ok(RoutineAction::ExperimentCampaign {
                    project_id: Uuid::parse_str(project_id).map_err(|e| {
                        RoutineError::InvalidCron {
                            reason: format!("invalid experiment project UUID: {e}"),
                        }
                    })?,
                    runner_profile_id: config
                        .get("runner_profile_id")
                        .and_then(|v| v.as_str())
                        .map(Uuid::parse_str)
                        .transpose()
                        .map_err(|e| RoutineError::InvalidCron {
                            reason: format!("invalid runner profile UUID: {e}"),
                        })?,
                    max_trials_override: config
                        .get("max_trials_override")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32),
                })
            }
            other => Err(RoutineError::UnknownActionType {
                action_type: other.to_string(),
            }),
        }
    }

    /// Serialize action config to JSON for DB storage.
    pub fn to_config_json(&self) -> serde_json::Value {
        match self {
            RoutineAction::Lightweight {
                prompt,
                context_paths,
                max_tokens,
            } => serde_json::json!({
                "prompt": prompt,
                "context_paths": context_paths,
                "max_tokens": max_tokens,
            }),
            RoutineAction::FullJob {
                title,
                description,
                max_iterations,
                allowed_tools,
                allowed_skills,
            } => serde_json::json!({
                "title": title,
                "description": description,
                "max_iterations": max_iterations,
                "allowed_tools": allowed_tools,
                "allowed_skills": allowed_skills,
            }),
            RoutineAction::Heartbeat {
                light_context,
                prompt,
                include_reasoning,
                active_start_hour,
                active_end_hour,
                target,
                max_iterations,
            } => serde_json::json!({
                "light_context": light_context,
                "prompt": prompt,
                "include_reasoning": include_reasoning,
                "active_start_hour": active_start_hour,
                "active_end_hour": active_end_hour,
                "target": target,
                "max_iterations": max_iterations,
            }),
            RoutineAction::ExperimentCampaign {
                project_id,
                runner_profile_id,
                max_trials_override,
            } => serde_json::json!({
                "project_id": project_id,
                "runner_profile_id": runner_profile_id,
                "max_trials_override": max_trials_override,
            }),
        }
    }
}

/// Guardrails to prevent runaway execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineGuardrails {
    /// Minimum time between fires.
    pub cooldown: Duration,
    /// Max simultaneous runs of this routine.
    pub max_concurrent: u32,
    /// Window for content-hash dedup (event triggers). None = no dedup.
    pub dedup_window: Option<Duration>,
}

impl Default for RoutineGuardrails {
    fn default() -> Self {
        Self {
            cooldown: Duration::from_secs(300),
            max_concurrent: 1,
            dedup_window: None,
        }
    }
}

/// Notification preferences for a routine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyConfig {
    /// Channel to notify on (None = default/broadcast all).
    pub channel: Option<String>,
    /// User to notify.
    pub user: String,
    /// Notify when routine produces actionable output.
    pub on_attention: bool,
    /// Notify when routine errors.
    pub on_failure: bool,
    /// Notify when routine runs with no findings.
    pub on_success: bool,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            channel: None,
            user: "default".to_string(),
            on_attention: true,
            on_failure: true,
            on_success: false,
        }
    }
}

/// Status of a routine run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Ok,
    Attention,
    Failed,
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunStatus::Running => write!(f, "running"),
            RunStatus::Ok => write!(f, "ok"),
            RunStatus::Attention => write!(f, "attention"),
            RunStatus::Failed => write!(f, "failed"),
        }
    }
}

impl FromStr for RunStatus {
    type Err = RoutineError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "running" => Ok(RunStatus::Running),
            "ok" => Ok(RunStatus::Ok),
            "attention" => Ok(RunStatus::Attention),
            "failed" => Ok(RunStatus::Failed),
            other => Err(RoutineError::UnknownRunStatus {
                status: other.to_string(),
            }),
        }
    }
}

/// A single execution of a routine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineRun {
    pub id: Uuid,
    pub routine_id: Uuid,
    pub trigger_type: String,
    pub trigger_detail: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: RunStatus,
    pub result_summary: Option<String>,
    pub tokens_used: Option<i32>,
    pub job_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// Compute a content hash for event dedup.
pub fn content_hash(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// Normalize a cron expression to the 7-field format required by the `cron` crate.
///
/// The `cron` crate (v0.13) uses: `sec min hour dom month dow year`
///
/// LLMs almost universally produce standard 5-field Unix cron: `min hour dom month dow`
/// or occasionally 6-field AWS/Quartz cron: `sec min hour dom month dow`
///
/// Mapping:
/// - 5 fields → prepend `0` (seconds) and append `*` (any year)
/// - 6 fields → append `*` (any year)
/// - 7 fields → pass through unchanged
/// - Other → pass through and let the parser reject it with a clear error
pub fn normalize_cron_expr(expr: &str) -> String {
    let trimmed = expr.trim();
    let field_count = trimmed.split_whitespace().count();
    match field_count {
        5 => format!("0 {trimmed} *"), // prepend sec=0, append year=*
        6 => format!("{trimmed} *"),   // append year=*
        _ => trimmed.to_string(),      // 7 or invalid — pass through
    }
}

/// Parse a cron expression and compute the next fire time from now.
pub fn next_cron_fire(schedule: &str) -> Result<Option<DateTime<Utc>>, RoutineError> {
    let cron_schedule =
        cron::Schedule::from_str(schedule).map_err(|e| RoutineError::InvalidCron {
            reason: e.to_string(),
        })?;
    Ok(cron_schedule.upcoming(Utc).next())
}

#[cfg(test)]
mod tests {
    use crate::agent::routine::{
        RoutineAction, RoutineGuardrails, RunStatus, Trigger, content_hash, next_cron_fire,
        normalize_cron_expr,
    };

    #[test]
    fn test_trigger_roundtrip() {
        let trigger = Trigger::Cron {
            schedule: "0 9 * * MON-FRI".to_string(),
        };
        let json = trigger.to_config_json();
        let parsed = Trigger::from_db("cron", json).expect("parse cron");
        assert!(matches!(parsed, Trigger::Cron { schedule } if schedule == "0 9 * * MON-FRI"));
    }

    #[test]
    fn test_event_trigger_roundtrip() {
        let trigger = Trigger::Event {
            channel: Some("telegram".to_string()),
            pattern: r"deploy\s+\w+".to_string(),
        };
        let json = trigger.to_config_json();
        let parsed = Trigger::from_db("event", json).expect("parse event");
        assert!(matches!(parsed, Trigger::Event { channel, pattern }
            if channel == Some("telegram".to_string()) && pattern == r"deploy\s+\w+"));
    }

    #[test]
    fn test_action_lightweight_roundtrip() {
        let action = RoutineAction::Lightweight {
            prompt: "Check PRs".to_string(),
            context_paths: vec!["context/priorities.md".to_string()],
            max_tokens: 2048,
        };
        let json = action.to_config_json();
        let parsed = RoutineAction::from_db("lightweight", json).expect("parse lightweight");
        assert!(
            matches!(parsed, RoutineAction::Lightweight { prompt, context_paths, max_tokens }
            if prompt == "Check PRs" && context_paths.len() == 1 && max_tokens == 2048)
        );
    }

    #[test]
    fn test_action_full_job_roundtrip() {
        let action = RoutineAction::FullJob {
            title: "Deploy review".to_string(),
            description: "Review and deploy pending changes".to_string(),
            max_iterations: 5,
            allowed_tools: Some(vec!["shell".to_string()]),
            allowed_skills: Some(vec!["github".to_string()]),
        };
        let json = action.to_config_json();
        let parsed = RoutineAction::from_db("full_job", json).expect("parse full_job");
        assert!(
            matches!(parsed, RoutineAction::FullJob { title, max_iterations, allowed_tools, allowed_skills, .. }
            if title == "Deploy review"
                && max_iterations == 5
                && allowed_tools == Some(vec!["shell".to_string()])
                && allowed_skills == Some(vec!["github".to_string()]))
        );
    }

    #[test]
    fn test_run_status_display_parse() {
        for status in [
            RunStatus::Running,
            RunStatus::Ok,
            RunStatus::Attention,
            RunStatus::Failed,
        ] {
            let s = status.to_string();
            let parsed: RunStatus = s.parse().expect("parse status");
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = content_hash("deploy production");
        let h2 = content_hash("deploy production");
        assert_eq!(h1, h2);

        let h3 = content_hash("deploy staging");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_next_cron_fire_valid() {
        // Every minute should always have a next fire (7-field)
        let next = next_cron_fire("* * * * * * *").expect("valid cron");
        assert!(next.is_some());
    }

    #[test]
    fn test_next_cron_fire_invalid() {
        let result = next_cron_fire("not a cron");
        assert!(result.is_err());
    }

    // ── normalize_cron_expr tests ─────────────────────────────────────────────

    #[test]
    fn test_normalize_5field_to_7field() {
        // Standard Unix cron: "min hour dom month dow"  → "0 min hour dom month dow *"
        let result = normalize_cron_expr("30 9 * * MON-FRI");
        assert_eq!(result, "0 30 9 * * MON-FRI *");
    }

    #[test]
    fn test_normalize_6field_to_7field() {
        // 6-field (with seconds, no year): "sec min hour dom month dow" → append "*"
        let result = normalize_cron_expr("0 30 9 * * MON-FRI");
        assert_eq!(result, "0 30 9 * * MON-FRI *");
    }

    #[test]
    fn test_normalize_7field_passthrough() {
        // Already 7-field — unchanged
        let expr = "0 30 9 * * MON-FRI *";
        let result = normalize_cron_expr(expr);
        assert_eq!(result, expr);
    }

    #[test]
    fn test_normalize_then_validate_5field() {
        // A 5-field expression normalized to 7-field must parse and fire
        let normalized = normalize_cron_expr("0 9 * * MON-FRI");
        let next = next_cron_fire(&normalized).expect("valid after normalization");
        assert!(next.is_some());
    }

    #[test]
    fn test_normalize_every_2h() {
        // "0 */2 * * *" (5-field every 2 hours) → 7-field
        let normalized = normalize_cron_expr("0 */2 * * *");
        assert_eq!(normalized, "0 0 */2 * * * *");
        let next = next_cron_fire(&normalized).expect("valid");
        assert!(next.is_some());
    }

    #[test]
    fn test_normalize_sunday_weekly() {
        // "0 10 * * SUN" common LLM output for "every Sunday at 10am"
        let normalized = normalize_cron_expr("0 10 * * SUN");
        assert_eq!(normalized, "0 0 10 * * SUN *");
        let next = next_cron_fire(&normalized).expect("valid");
        assert!(next.is_some());
    }

    #[test]
    fn test_guardrails_default() {
        let g = RoutineGuardrails::default();
        assert_eq!(g.cooldown.as_secs(), 300);
        assert_eq!(g.max_concurrent, 1);
        assert!(g.dedup_window.is_none());
    }

    #[test]
    fn test_trigger_type_tag() {
        assert_eq!(
            Trigger::Cron {
                schedule: String::new()
            }
            .type_tag(),
            "cron"
        );
        assert_eq!(
            Trigger::Event {
                channel: None,
                pattern: String::new()
            }
            .type_tag(),
            "event"
        );
        assert_eq!(
            Trigger::Webhook {
                path: None,
                secret: None
            }
            .type_tag(),
            "webhook"
        );
        assert_eq!(Trigger::Manual.type_tag(), "manual");
    }
}
