//! Pure routine domain types (DTOs).
//!
//! Moved out of `thinclaw-agent` so persistence (`thinclaw-db`) can depend on
//! these data types without pulling in the agent layer (the worst
//! wrong-direction crate edge). Trigger-evaluation logic (regex/cron/chrono-tz)
//! stays in `thinclaw_agent::routine`, which re-exports these types for path
//! stability.

use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ToolProfile;
use crate::error::RoutineError;

/// Catch-up policy for overdue scheduled routines after downtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RoutineCatchUpMode {
    /// Skip overdue backlog and move directly to the next future slot.
    Skip,
    /// Run at most once from the current state, then move to the next future slot.
    #[default]
    RunOnceNow,
    /// Replay each missed slot (bounded by engine safeguards).
    Replay,
}

/// Delivery/runtime policy for a routine.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutinePolicy {
    /// Catch-up behavior for overdue cron/system schedules.
    #[serde(default)]
    pub catch_up_mode: RoutineCatchUpMode,
    /// Optional max age for replayed durable events before they expire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_event_age_secs: Option<u64>,
}

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
    #[serde(default)]
    pub policy: RoutinePolicy,

    // Runtime state (DB-managed)
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_fire_at: Option<DateTime<Utc>>,
    pub run_count: u64,
    pub consecutive_failures: u32,
    pub state: serde_json::Value,
    #[serde(default = "default_config_version")]
    pub config_version: i64,

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

    /// Effective max age for durable event replay.
    pub fn effective_event_max_age_secs(&self, default_secs: u64) -> u64 {
        self.policy
            .max_event_age_secs
            .unwrap_or(default_secs)
            .max(1)
    }

    /// Priority used when ordering event-triggered routines for the same message.
    pub fn event_priority(&self) -> i32 {
        match &self.trigger {
            Trigger::Event { priority, .. } => *priority,
            _ => 0,
        }
    }
}

fn default_config_version() -> i64 {
    1
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
        /// Optional structured event type (defaults to channel-specific "message").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_type: Option<String>,
        /// Optional originating sender/actor filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor: Option<String>,
        /// Optional metadata subset that must be present on the event payload.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
        /// Regex pattern to match against message content.
        ///
        /// Empty means the structured filters above are sufficient and regex is
        /// used as an optional secondary filter rather than the primary match.
        #[serde(default)]
        pattern: String,
        /// Higher priority routines are evaluated first when multiple match.
        #[serde(default)]
        priority: i32,
    },
    /// Fire on incoming webhook POST to /hooks/routine/{id}.
    Webhook {
        /// Optional webhook path suffix (defaults to routine id).
        path: Option<String>,
        /// Optional shared secret for HMAC validation.
        secret: Option<String>,
        /// Allow unsigned webhook calls.
        ///
        /// Defaults to `false` so webhook calls are signed by default.
        #[serde(default)]
        allow_unsigned_webhook: bool,
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
                    .unwrap_or("")
                    .to_string();
                let channel = config
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let event_type = config
                    .get("event_type")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let actor = config
                    .get("actor")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let metadata = config
                    .get("metadata")
                    .cloned()
                    .filter(|value| !value.is_null());
                let priority = config.get("priority").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                Ok(Trigger::Event {
                    channel,
                    event_type,
                    actor,
                    metadata,
                    pattern,
                    priority,
                })
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
                let allow_unsigned_webhook = config
                    .get("allow_unsigned_webhook")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                Ok(Trigger::Webhook {
                    path,
                    secret,
                    allow_unsigned_webhook,
                })
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
            Trigger::Event {
                channel,
                event_type,
                actor,
                metadata,
                pattern,
                priority,
            } => serde_json::json!({
                "pattern": pattern,
                "channel": channel,
                "event_type": event_type,
                "actor": actor,
                "metadata": metadata,
                "priority": priority,
            }),
            Trigger::Webhook {
                path,
                secret,
                allow_unsigned_webhook,
            } => serde_json::json!({
                "path": path,
                "secret": secret,
                "allow_unsigned_webhook": allow_unsigned_webhook,
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
        /// Optional execution profile override for this routine's worker/subagent.
        #[serde(default)]
        tool_profile: Option<ToolProfile>,
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
        /// Exact interval for the internal heartbeat scheduler.
        ///
        /// Heartbeats use this persisted interval to compute `next_fire_at`
        /// directly instead of trying to encode arbitrary second intervals
        /// into a cron expression.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interval_secs: Option<u64>,
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
                let parse_optional_allowlist =
                    |key: &str| -> Result<Option<Vec<String>>, RoutineError> {
                        let Some(value) = config.get(key) else {
                            return Ok(None);
                        };
                        if value.is_null() {
                            return Ok(None);
                        }
                        serde_json::from_value::<Vec<String>>(value.clone())
                            .map(Some)
                            .map_err(|e| RoutineError::InvalidCron {
                                reason: format!("invalid full_job.{key}: {e}"),
                            })
                    };
                let allowed_tools = parse_optional_allowlist("allowed_tools")?;
                let allowed_skills = parse_optional_allowlist("allowed_skills")?;
                let tool_profile = config
                    .get("tool_profile")
                    .and_then(|v| v.as_str())
                    .and_then(|value| value.parse::<ToolProfile>().ok());
                Ok(RoutineAction::FullJob {
                    title,
                    description,
                    max_iterations,
                    allowed_tools,
                    allowed_skills,
                    tool_profile,
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
                    interval_secs: config.get("interval_secs").and_then(|v| v.as_u64()),
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
                tool_profile,
            } => {
                let mut config = serde_json::Map::new();
                config.insert(
                    "title".to_string(),
                    serde_json::Value::String(title.clone()),
                );
                config.insert(
                    "description".to_string(),
                    serde_json::Value::String(description.clone()),
                );
                config.insert(
                    "max_iterations".to_string(),
                    serde_json::json!(*max_iterations),
                );
                if let Some(allowed_tools) = allowed_tools {
                    config.insert(
                        "allowed_tools".to_string(),
                        serde_json::json!(allowed_tools),
                    );
                }
                if let Some(allowed_skills) = allowed_skills {
                    config.insert(
                        "allowed_skills".to_string(),
                        serde_json::json!(allowed_skills),
                    );
                }
                if let Some(tool_profile) = tool_profile {
                    config.insert(
                        "tool_profile".to_string(),
                        serde_json::Value::String(tool_profile.as_str().to_string()),
                    );
                }
                serde_json::Value::Object(config)
            }
            RoutineAction::Heartbeat {
                light_context,
                prompt,
                include_reasoning,
                active_start_hour,
                active_end_hour,
                target,
                max_iterations,
                interval_secs,
            } => serde_json::json!({
                "light_context": light_context,
                "prompt": prompt,
                "include_reasoning": include_reasoning,
                "active_start_hour": active_start_hour,
                "active_end_hour": active_end_hour,
                "target": target,
                "max_iterations": max_iterations,
                "interval_secs": interval_secs,
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

    /// Resolve the effective interval for an internal heartbeat routine.
    pub fn heartbeat_interval_secs(&self, guardrails: Option<&RoutineGuardrails>) -> Option<u64> {
        match self {
            RoutineAction::Heartbeat { interval_secs, .. } => interval_secs
                .as_ref()
                .copied()
                .or_else(|| {
                    guardrails.and_then(|g| {
                        let cooldown_secs = g.cooldown.as_secs();
                        (cooldown_secs > 0).then_some(cooldown_secs.saturating_mul(2))
                    })
                })
                .map(|secs| secs.max(1)),
            _ => None,
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
    pub trigger_key: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: RunStatus,
    pub result_summary: Option<String>,
    pub tokens_used: Option<i32>,
    pub job_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// Durable inbox status for persisted event-trigger inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineEventStatus {
    Pending,
    Processing,
    Processed,
    Failed,
}

impl std::fmt::Display for RoutineEventStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoutineEventStatus::Pending => write!(f, "pending"),
            RoutineEventStatus::Processing => write!(f, "processing"),
            RoutineEventStatus::Processed => write!(f, "processed"),
            RoutineEventStatus::Failed => write!(f, "failed"),
        }
    }
}

impl std::str::FromStr for RoutineEventStatus {
    type Err = RoutineError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "processing" => Ok(Self::Processing),
            "processed" => Ok(Self::Processed),
            "failed" => Ok(Self::Failed),
            other => Err(RoutineError::ExecutionFailed {
                reason: format!("unknown routine event status: {other}"),
            }),
        }
    }
}

/// A persisted inbound event waiting to be matched against event routines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineEvent {
    pub id: Uuid,
    pub principal_id: String,
    pub actor_id: String,
    pub channel: String,
    pub event_type: String,
    pub raw_sender_id: String,
    pub conversation_scope_id: String,
    pub stable_external_conversation_key: String,
    pub idempotency_key: String,
    pub content: String,
    pub content_hash: String,
    pub metadata: serde_json::Value,
    pub status: RoutineEventStatus,
    pub diagnostics: serde_json::Value,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub processed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub matched_routines: u32,
    pub fired_routines: u32,
    pub attempt_count: u32,
    pub created_at: DateTime<Utc>,
}

/// Per-routine evaluation result for a single persisted event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineEventDecision {
    Fired,
    IgnoredChannel,
    IgnoredEventType,
    IgnoredActor,
    IgnoredMetadata,
    IgnoredPattern,
    SkippedExpired,
    SkippedDuplicate,
    SkippedCooldown,
    DeferredConcurrency,
    DeferredGlobalCapacity,
}

impl std::fmt::Display for RoutineEventDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoutineEventDecision::Fired => write!(f, "fired"),
            RoutineEventDecision::IgnoredChannel => write!(f, "ignored_channel"),
            RoutineEventDecision::IgnoredEventType => write!(f, "ignored_event_type"),
            RoutineEventDecision::IgnoredActor => write!(f, "ignored_actor"),
            RoutineEventDecision::IgnoredMetadata => write!(f, "ignored_metadata"),
            RoutineEventDecision::IgnoredPattern => write!(f, "ignored_pattern"),
            RoutineEventDecision::SkippedExpired => write!(f, "skipped_expired"),
            RoutineEventDecision::SkippedDuplicate => write!(f, "skipped_duplicate"),
            RoutineEventDecision::SkippedCooldown => write!(f, "skipped_cooldown"),
            RoutineEventDecision::DeferredConcurrency => write!(f, "deferred_concurrency"),
            RoutineEventDecision::DeferredGlobalCapacity => {
                write!(f, "deferred_global_capacity")
            }
        }
    }
}

impl std::str::FromStr for RoutineEventDecision {
    type Err = RoutineError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fired" => Ok(Self::Fired),
            "ignored_channel" => Ok(Self::IgnoredChannel),
            "ignored_event_type" => Ok(Self::IgnoredEventType),
            "ignored_actor" => Ok(Self::IgnoredActor),
            "ignored_metadata" => Ok(Self::IgnoredMetadata),
            "ignored_pattern" => Ok(Self::IgnoredPattern),
            "skipped_expired" => Ok(Self::SkippedExpired),
            "skipped_duplicate" => Ok(Self::SkippedDuplicate),
            "skipped_cooldown" => Ok(Self::SkippedCooldown),
            "deferred_concurrency" => Ok(Self::DeferredConcurrency),
            "deferred_global_capacity" => Ok(Self::DeferredGlobalCapacity),
            other => Err(RoutineError::ExecutionFailed {
                reason: format!("unknown routine event decision: {other}"),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineEventEvaluation {
    pub id: Uuid,
    pub event_id: Uuid,
    pub routine_id: Uuid,
    pub decision: RoutineEventDecision,
    pub reason: Option<String>,
    pub details: serde_json::Value,
    pub sequence_num: u32,
    pub channel: String,
    pub content_preview: String,
    pub created_at: DateTime<Utc>,
}

/// Durable queue status for scheduled routine triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineTriggerStatus {
    Pending,
    Processing,
    Processed,
    Failed,
}

impl std::fmt::Display for RoutineTriggerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoutineTriggerStatus::Pending => write!(f, "pending"),
            RoutineTriggerStatus::Processing => write!(f, "processing"),
            RoutineTriggerStatus::Processed => write!(f, "processed"),
            RoutineTriggerStatus::Failed => write!(f, "failed"),
        }
    }
}

impl FromStr for RoutineTriggerStatus {
    type Err = RoutineError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "processing" => Ok(Self::Processing),
            "processed" => Ok(Self::Processed),
            "failed" => Ok(Self::Failed),
            other => Err(RoutineError::ExecutionFailed {
                reason: format!("unknown routine trigger status: {other}"),
            }),
        }
    }
}

/// Scheduled trigger kind persisted in the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineTriggerKind {
    Cron,
    SystemEvent,
}

impl std::fmt::Display for RoutineTriggerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoutineTriggerKind::Cron => write!(f, "cron"),
            RoutineTriggerKind::SystemEvent => write!(f, "system_event"),
        }
    }
}

impl FromStr for RoutineTriggerKind {
    type Err = RoutineError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "cron" => Ok(Self::Cron),
            "system_event" => Ok(Self::SystemEvent),
            other => Err(RoutineError::ExecutionFailed {
                reason: format!("unknown routine trigger kind: {other}"),
            }),
        }
    }
}

/// Processing outcome for a scheduled queued trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineTriggerDecision {
    Fired,
    SkippedCatchUp,
    SkippedDisabled,
    SkippedDuplicate,
    DeferredCooldown,
    DeferredConcurrency,
    DeferredGlobalCapacity,
}

impl std::fmt::Display for RoutineTriggerDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoutineTriggerDecision::Fired => write!(f, "fired"),
            RoutineTriggerDecision::SkippedCatchUp => write!(f, "skipped_catch_up"),
            RoutineTriggerDecision::SkippedDisabled => write!(f, "skipped_disabled"),
            RoutineTriggerDecision::SkippedDuplicate => write!(f, "skipped_duplicate"),
            RoutineTriggerDecision::DeferredCooldown => write!(f, "deferred_cooldown"),
            RoutineTriggerDecision::DeferredConcurrency => write!(f, "deferred_concurrency"),
            RoutineTriggerDecision::DeferredGlobalCapacity => {
                write!(f, "deferred_global_capacity")
            }
        }
    }
}

impl FromStr for RoutineTriggerDecision {
    type Err = RoutineError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fired" => Ok(Self::Fired),
            "skipped_catch_up" => Ok(Self::SkippedCatchUp),
            "skipped_disabled" => Ok(Self::SkippedDisabled),
            "skipped_duplicate" => Ok(Self::SkippedDuplicate),
            "deferred_cooldown" => Ok(Self::DeferredCooldown),
            "deferred_concurrency" => Ok(Self::DeferredConcurrency),
            "deferred_global_capacity" => Ok(Self::DeferredGlobalCapacity),
            other => Err(RoutineError::ExecutionFailed {
                reason: format!("unknown routine trigger decision: {other}"),
            }),
        }
    }
}

/// Durable scheduled trigger queue row for cron/system event routines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineTrigger {
    pub id: Uuid,
    pub routine_id: Uuid,
    pub trigger_kind: RoutineTriggerKind,
    pub trigger_label: Option<String>,
    pub due_at: DateTime<Utc>,
    pub status: RoutineTriggerStatus,
    pub decision: Option<RoutineTriggerDecision>,
    pub active_key: Option<String>,
    pub idempotency_key: String,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub processed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub diagnostics: serde_json::Value,
    pub coalesced_count: u32,
    pub backlog_collapsed: bool,
    pub routine_config_version: i64,
    pub created_at: DateTime<Utc>,
}
