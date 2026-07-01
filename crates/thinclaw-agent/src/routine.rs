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

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use chrono_tz::Tz;
use regex::{Regex, RegexBuilder};
use uuid::Uuid;

use thinclaw_types::error::RoutineError;

// Pure DTOs now live in `thinclaw-types`; re-export for path stability so that
// existing `thinclaw_agent::routine::*` importers keep working unchanged.
pub use thinclaw_types::routine::*;

const RUNTIME_ADVANCED_FOR_RUN_ID_KEY: &str = "runtime_advanced_for_run_id";
const RUNTIME_ADVANCED_AT_KEY: &str = "runtime_advanced_at";
pub const EVENT_PATTERN_MAX_LEN: usize = 512;
const EVENT_PATTERN_SIZE_LIMIT: usize = 256 * 1024;
const EVENT_PATTERN_DFA_SIZE_LIMIT: usize = 2 * 1024 * 1024;

/// Record that this routine's persisted runtime state has been advanced for a run.
pub fn routine_state_with_runtime_advance(
    state: &serde_json::Value,
    run_id: Uuid,
    advanced_at: DateTime<Utc>,
) -> serde_json::Value {
    let mut map = match state {
        serde_json::Value::Object(existing) => existing.clone(),
        _ => serde_json::Map::new(),
    };
    map.insert(
        RUNTIME_ADVANCED_FOR_RUN_ID_KEY.to_string(),
        serde_json::json!(run_id.to_string()),
    );
    map.insert(
        RUNTIME_ADVANCED_AT_KEY.to_string(),
        serde_json::json!(advanced_at.to_rfc3339()),
    );
    serde_json::Value::Object(map)
}

/// Whether persisted runtime state was already advanced for the provided run.
pub fn routine_state_has_runtime_advance_for_run(state: &serde_json::Value, run_id: Uuid) -> bool {
    let run_id = run_id.to_string();
    state
        .get(RUNTIME_ADVANCED_FOR_RUN_ID_KEY)
        .and_then(|value| value.as_str())
        == Some(run_id.as_str())
}

/// Validate and lint an event trigger before it is persisted or compiled.
pub fn validate_event_trigger(
    channel: Option<&str>,
    event_type: Option<&str>,
    actor: Option<&str>,
    metadata: Option<&serde_json::Value>,
    pattern: &str,
    priority: i32,
) -> Result<Vec<String>, RoutineError> {
    let trimmed_pattern = pattern.trim();
    if !(-10_000..=10_000).contains(&priority) {
        return Err(RoutineError::InvalidEventPattern {
            reason: "priority must be between -10000 and 10000".to_string(),
        });
    }

    if let Some(value) = metadata
        && !value.is_object()
    {
        return Err(RoutineError::InvalidEventPattern {
            reason: "metadata filter must be a JSON object".to_string(),
        });
    }

    let has_structured_filter = channel.is_some()
        || event_type.is_some()
        || actor.is_some()
        || metadata.is_some_and(|value| value.as_object().is_some_and(|obj| !obj.is_empty()));
    if trimmed_pattern.is_empty() && !has_structured_filter {
        return Err(RoutineError::InvalidEventPattern {
            reason: "event trigger needs at least one structured filter or a regex pattern"
                .to_string(),
        });
    }

    if !trimmed_pattern.is_empty() {
        if trimmed_pattern.len() > EVENT_PATTERN_MAX_LEN {
            return Err(RoutineError::InvalidEventPattern {
                reason: format!("pattern exceeds {} characters", EVENT_PATTERN_MAX_LEN),
            });
        }
        compile_event_trigger_pattern(trimmed_pattern)?;
    }

    let mut warnings = Vec::new();
    if channel.is_none() {
        warnings.push("matches all channels".to_string());
    }
    if event_type.is_none() {
        warnings.push("matches all event types".to_string());
    }
    if trimmed_pattern.is_empty() {
        warnings
            .push("regex matching disabled; trigger relies on structured fields only".to_string());
    } else if matches!(trimmed_pattern, ".*" | ".+" | "(?s).*") {
        warnings.push("pattern is extremely broad and may fire on most messages".to_string());
    } else if trimmed_pattern.contains(".*") && !trimmed_pattern.starts_with('^') {
        warnings.push("pattern uses broad wildcards without a leading anchor".to_string());
    }
    if trimmed_pattern.len() > 256 {
        warnings.push("pattern is long; prefer a shorter expression when possible".to_string());
    }

    Ok(warnings)
}

/// Validate and lint an event trigger pattern before it is persisted or compiled.
pub fn validate_event_trigger_pattern(
    channel: Option<&str>,
    pattern: &str,
    priority: i32,
) -> Result<Vec<String>, RoutineError> {
    validate_event_trigger(channel, None, None, None, pattern, priority)
}

/// Compile an event trigger regex with explicit resource limits.
pub fn compile_event_trigger_pattern(pattern: &str) -> Result<Regex, RoutineError> {
    RegexBuilder::new(pattern)
        .size_limit(EVENT_PATTERN_SIZE_LIMIT)
        .dfa_size_limit(EVENT_PATTERN_DFA_SIZE_LIMIT)
        .build()
        .map_err(|error| RoutineError::InvalidEventPattern {
            reason: error.to_string(),
        })
}

/// Compute a content hash for event dedup.
pub fn content_hash(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Clone)]
enum ParsedSchedule {
    Cron {
        normalized: String,
        parsed: Box<cron::Schedule>,
    },
    Interval {
        seconds: u64,
        canonical: String,
    },
}

fn canonical_interval_schedule(seconds: u64) -> String {
    let seconds = seconds.max(1);

    if seconds.is_multiple_of(86_400) {
        return format!("every {}d", seconds / 86_400);
    }

    if seconds.is_multiple_of(3_600) {
        return format!("every {}h", seconds / 3_600);
    }

    if seconds.is_multiple_of(60) {
        return format!("every {}m", seconds / 60);
    }

    format!("every {seconds}s")
}

fn interval_unit_multiplier(unit: &str) -> Option<u64> {
    match unit
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "s" | "sec" | "secs" | "second" | "seconds" => Some(1),
        "m" | "min" | "mins" | "minute" | "minutes" => Some(60),
        "h" | "hr" | "hrs" | "hour" | "hours" => Some(3_600),
        "d" | "day" | "days" => Some(86_400),
        _ => None,
    }
}

fn parse_named_interval(expr: &str) -> Option<u64> {
    match expr.trim().to_ascii_lowercase().as_str() {
        "minutely" | "every minute" => Some(60),
        "hourly" | "every hour" => Some(3_600),
        "daily" | "every day" => Some(86_400),
        _ => None,
    }
}

fn checked_interval_seconds(value: u64, multiplier: u64) -> Result<u64, RoutineError> {
    if value == 0 {
        return Err(RoutineError::InvalidCron {
            reason: "interval schedules must be greater than zero".to_string(),
        });
    }

    value
        .checked_mul(multiplier)
        .ok_or_else(|| RoutineError::InvalidCron {
            reason: "interval schedule is too large".to_string(),
        })
}

fn parse_interval_schedule_seconds(expr: &str) -> Result<Option<u64>, RoutineError> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    if let Some(seconds) = parse_named_interval(trimmed) {
        return Ok(Some(seconds));
    }

    let lowered = trimmed.to_ascii_lowercase();
    let candidate = lowered
        .strip_prefix("every ")
        .map(str::trim)
        .unwrap_or(lowered.trim());

    if let Some(seconds) = parse_named_interval(candidate) {
        return Ok(Some(seconds));
    }

    let mut parts = candidate.split_whitespace();
    let Some(first) = parts.next() else {
        return Ok(None);
    };

    let digit_end = first
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .last()
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);

    if digit_end == 0 {
        return Ok(None);
    }

    let value = first[..digit_end]
        .parse::<u64>()
        .map_err(|e| RoutineError::InvalidCron {
            reason: format!("invalid interval value: {e}"),
        })?;

    let mut unit = first[digit_end..].to_string();
    let remainder = parts.collect::<Vec<_>>().join(" ");
    if !remainder.is_empty() {
        unit.push_str(&remainder);
    }

    let Some(multiplier) = interval_unit_multiplier(&unit) else {
        return Ok(None);
    };

    Ok(Some(checked_interval_seconds(value, multiplier)?))
}

fn parse_step_value(field: &str) -> Option<u64> {
    let (base, step) = field.split_once('/')?;
    if base != "*" && base != "0" {
        return None;
    }
    step.parse::<u64>().ok()
}

fn parse_oversized_step_interval_seconds(schedule: &str) -> Option<u64> {
    let fields: Vec<_> = schedule.split_whitespace().collect();
    if fields.len() != 7 || fields[6] != "*" {
        return None;
    }

    if let Some(step) = parse_step_value(fields[0])
        && step > 59
        && fields[1..6].iter().all(|field| *field == "*")
    {
        return Some(step);
    }

    if fields[0] == "0"
        && let Some(step) = parse_step_value(fields[1])
        && step > 59
        && fields[2..6].iter().all(|field| *field == "*")
    {
        return step.checked_mul(60);
    }

    if fields[0] == "0"
        && fields[1] == "0"
        && let Some(step) = parse_step_value(fields[2])
        && step > 23
        && fields[3..6].iter().all(|field| *field == "*")
    {
        return step.checked_mul(3_600);
    }

    if fields[0] == "0"
        && fields[1] == "0"
        && fields[2] == "0"
        && let Some(step) = parse_step_value(fields[3])
        && step > 31
        && fields[4] == "*"
        && fields[5] == "*"
    {
        return step.checked_mul(86_400);
    }

    None
}

fn parse_schedule_expr(expr: &str) -> Result<ParsedSchedule, RoutineError> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err(RoutineError::InvalidCron {
            reason: "schedule cannot be empty".to_string(),
        });
    }

    if let Some(seconds) = parse_interval_schedule_seconds(trimmed)? {
        return Ok(ParsedSchedule::Interval {
            seconds,
            canonical: canonical_interval_schedule(seconds),
        });
    }

    let normalized = normalize_cron_expr(trimmed);
    match cron::Schedule::from_str(&normalized) {
        Ok(parsed) => Ok(ParsedSchedule::Cron {
            normalized,
            parsed: Box::new(parsed),
        }),
        Err(err) => {
            if let Some(seconds) = parse_oversized_step_interval_seconds(&normalized) {
                return Ok(ParsedSchedule::Interval {
                    seconds,
                    canonical: canonical_interval_schedule(seconds),
                });
            }

            Err(RoutineError::InvalidCron {
                reason: err.to_string(),
            })
        }
    }
}

/// Canonicalize a schedule expression for persistence.
///
/// Supports both cron expressions and interval schedules like `every 2h`,
/// `90 minutes`, or `12800s`. Oversized pure step-cron expressions such as
/// `0 */213 * * * * *` are converted into interval schedules instead of being
/// rejected and left in a broken state.
pub fn canonicalize_schedule_expr(expr: &str) -> Result<String, RoutineError> {
    match parse_schedule_expr(expr)? {
        ParsedSchedule::Cron { normalized, .. } => Ok(normalized),
        ParsedSchedule::Interval { canonical, .. } => Ok(canonical),
    }
}

/// Return whether a schedule depends on the user's timezone.
///
/// Wall-clock cron schedules are timezone-sensitive. Fixed intervals are not.
pub fn schedule_uses_timezone(schedule: &str) -> Result<bool, RoutineError> {
    Ok(matches!(
        parse_schedule_expr(schedule)?,
        ParsedSchedule::Cron { .. }
    ))
}

/// Return whether a routine's next fire time should be refreshed when the user's
/// timezone changes.
pub fn routine_schedule_uses_timezone(routine: &Routine) -> Result<bool, RoutineError> {
    if routine
        .action
        .heartbeat_interval_secs(Some(&routine.guardrails))
        .is_some()
    {
        return Ok(false);
    }

    match &routine.trigger {
        Trigger::Cron { schedule } => schedule_uses_timezone(schedule),
        Trigger::SystemEvent {
            schedule: Some(schedule),
            ..
        } => schedule_uses_timezone(schedule),
        Trigger::SystemEvent { schedule: None, .. }
        | Trigger::Event { .. }
        | Trigger::Webhook { .. }
        | Trigger::Manual => Ok(false),
    }
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

/// Parse a schedule expression and compute the next fire time from now.
pub fn next_schedule_fire(schedule: &str) -> Result<Option<DateTime<Utc>>, RoutineError> {
    next_schedule_fire_after_in_tz(schedule, Tz::UTC, Utc::now())
}

/// Parse a schedule expression and compute the next fire time from a base time.
pub fn next_schedule_fire_after_in_tz(
    schedule: &str,
    tz: Tz,
    from: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, RoutineError> {
    match parse_schedule_expr(schedule)? {
        ParsedSchedule::Cron { parsed, .. } => Ok(parsed
            .after(&from.with_timezone(&tz))
            .next()
            .map(|next| next.with_timezone(&Utc))),
        ParsedSchedule::Interval { seconds, .. } => {
            let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
            Ok(Some(from + ChronoDuration::seconds(seconds)))
        }
    }
}

/// Parse a schedule expression and compute the next fire time from now in a
/// specific timezone.
pub fn next_schedule_fire_in_tz(
    schedule: &str,
    tz: Tz,
) -> Result<Option<DateTime<Utc>>, RoutineError> {
    next_schedule_fire_after_in_tz(schedule, tz, Utc::now())
}

/// Parse a schedule expression and compute the next fire time in the user's
/// effective timezone.
pub fn next_schedule_fire_for_user(
    schedule: &str,
    user_id: &str,
    user_setting: Option<&str>,
) -> Result<Option<DateTime<Utc>>, RoutineError> {
    let tz = thinclaw_platform::timezone::resolve_effective_timezone(Some(user_id), user_setting);
    next_schedule_fire_in_tz(schedule, tz)
}

/// Parse a schedule expression and compute the next fire time from a base time
/// in the user's effective timezone.
pub fn next_schedule_fire_for_user_after(
    schedule: &str,
    user_id: &str,
    user_setting: Option<&str>,
    from: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, RoutineError> {
    let tz = thinclaw_platform::timezone::resolve_effective_timezone(Some(user_id), user_setting);
    next_schedule_fire_after_in_tz(schedule, tz, from)
}

/// Backward-compatible wrapper around [`next_schedule_fire`].
pub fn next_cron_fire(schedule: &str) -> Result<Option<DateTime<Utc>>, RoutineError> {
    next_schedule_fire(schedule)
}

/// Backward-compatible wrapper around [`next_schedule_fire_in_tz`].
pub fn next_cron_fire_in_tz(schedule: &str, tz: Tz) -> Result<Option<DateTime<Utc>>, RoutineError> {
    next_schedule_fire_in_tz(schedule, tz)
}

/// Backward-compatible wrapper around [`next_schedule_fire_for_user`].
pub fn next_cron_fire_for_user(
    schedule: &str,
    user_id: &str,
    user_setting: Option<&str>,
) -> Result<Option<DateTime<Utc>>, RoutineError> {
    next_schedule_fire_for_user(schedule, user_id, user_setting)
}

/// Produce a valid cron hint for heartbeat routines.
///
/// Heartbeat scheduling uses `interval_secs` directly; this cron string is a
/// compatibility hint for storage and diagnostics so the internal routine never
/// persists an invalid cron expression.
pub fn heartbeat_schedule_hint(interval_secs: u64) -> String {
    let secs = interval_secs.max(1);

    if secs < 60 {
        return format!("*/{} * * * * * *", secs);
    }

    if secs.is_multiple_of(60) {
        let minutes = secs / 60;
        if minutes <= 59 {
            return format!("0 */{} * * * * *", minutes);
        }
    }

    if secs.is_multiple_of(3600) {
        let hours = secs / 3600;
        if hours <= 23 {
            return format!("0 0 */{} * * * *", hours);
        }
    }

    if secs.is_multiple_of(86_400) {
        let days = secs / 86_400;
        if days <= 31 {
            return format!("0 0 0 */{} * * *", days);
        }
    }

    "0 * * * * * *".to_string()
}

/// Compute the next fire time for a routine from the provided base time.
pub fn next_fire_for_routine(
    routine: &Routine,
    user_setting: Option<&str>,
    from: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, RoutineError> {
    if let Some(interval_secs) = routine
        .action
        .heartbeat_interval_secs(Some(&routine.guardrails))
    {
        let seconds = i64::try_from(interval_secs).unwrap_or(i64::MAX);
        return Ok(Some(from + ChronoDuration::seconds(seconds)));
    }

    match &routine.trigger {
        Trigger::Cron { schedule } => {
            next_schedule_fire_for_user_after(schedule, &routine.user_id, user_setting, from)
        }
        Trigger::SystemEvent {
            schedule: Some(schedule),
            ..
        } => next_schedule_fire_for_user_after(schedule, &routine.user_id, user_setting, from),
        Trigger::SystemEvent { schedule: None, .. }
        | Trigger::Event { .. }
        | Trigger::Webhook { .. }
        | Trigger::Manual => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{Duration as ChronoDuration, TimeZone, Utc};
    use chrono_tz::Tz;
    use uuid::Uuid;

    use crate::routine::{
        EVENT_PATTERN_MAX_LEN, NotifyConfig, Routine, RoutineAction, RoutineGuardrails, RunStatus,
        Trigger, canonicalize_schedule_expr, content_hash, heartbeat_schedule_hint, next_cron_fire,
        next_fire_for_routine, next_schedule_fire_after_in_tz, normalize_cron_expr,
        routine_schedule_uses_timezone, validate_event_trigger_pattern,
    };
    use thinclaw_types::ToolProfile;

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
            event_type: None,
            actor: None,
            metadata: None,
            pattern: r"deploy\s+\w+".to_string(),
            priority: 25,
        };
        let json = trigger.to_config_json();
        let parsed = Trigger::from_db("event", json).expect("parse event");
        assert!(
            matches!(parsed, Trigger::Event { channel, pattern, priority, .. }
            if channel == Some("telegram".to_string())
                && pattern == r"deploy\s+\w+"
                && priority == 25)
        );
    }

    #[test]
    fn test_event_pattern_validation_rejects_overly_long_pattern() {
        let pattern = "a".repeat(EVENT_PATTERN_MAX_LEN + 1);
        let err = validate_event_trigger_pattern(Some("slack"), &pattern, 0)
            .expect_err("long patterns should be rejected");
        assert!(err.to_string().contains("exceeds"));
    }

    #[test]
    fn test_event_pattern_validation_warns_on_broad_match() {
        let warnings =
            validate_event_trigger_pattern(None, ".*", 0).expect("pattern should compile");
        assert!(warnings.iter().any(|warning| warning.contains("broad")));
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("all channels"))
        );
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
            tool_profile: Some(ToolProfile::Restricted),
        };
        let json = action.to_config_json();
        let parsed = RoutineAction::from_db("full_job", json).expect("parse full_job");
        assert!(
            matches!(parsed, RoutineAction::FullJob { title, max_iterations, allowed_tools, allowed_skills, tool_profile, .. }
            if title == "Deploy review"
                && max_iterations == 5
                && allowed_tools == Some(vec!["shell".to_string()])
                && allowed_skills == Some(vec!["github".to_string()])
                && tool_profile == Some(ToolProfile::Restricted))
        );
    }

    #[test]
    fn test_action_full_job_accepts_null_allowlists() {
        let json = serde_json::json!({
            "title": "Deploy review",
            "description": "Review and deploy pending changes",
            "max_iterations": 5,
            "allowed_tools": null,
            "allowed_skills": null,
            "tool_profile": "restricted"
        });
        let parsed = RoutineAction::from_db("full_job", json).expect("parse full_job");
        assert!(
            matches!(parsed, RoutineAction::FullJob { allowed_tools, allowed_skills, tool_profile, .. }
            if allowed_tools.is_none()
                && allowed_skills.is_none()
                && tool_profile == Some(ToolProfile::Restricted))
        );
    }

    #[test]
    fn test_action_full_job_omits_empty_allowlists_in_json() {
        let action = RoutineAction::FullJob {
            title: "Deploy review".to_string(),
            description: "Review and deploy pending changes".to_string(),
            max_iterations: 5,
            allowed_tools: None,
            allowed_skills: None,
            tool_profile: None,
        };
        let json = action.to_config_json();
        assert!(json.get("allowed_tools").is_none());
        assert!(json.get("allowed_skills").is_none());
        assert!(json.get("tool_profile").is_none());
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

    #[test]
    fn test_canonicalize_interval_schedule() {
        assert_eq!(
            canonicalize_schedule_expr("90 minutes").expect("interval should parse"),
            "every 90m"
        );
        assert_eq!(
            canonicalize_schedule_expr("12800s").expect("seconds interval should parse"),
            "every 12800s"
        );
    }

    #[test]
    fn test_canonicalize_oversized_cron_step_to_interval() {
        assert_eq!(
            canonicalize_schedule_expr("0 */213 * * * * *").expect("oversized step should convert"),
            "every 213m"
        );
        assert_eq!(
            canonicalize_schedule_expr("0 0 */36 * * * *")
                .expect("oversized hour step should convert"),
            "every 36h"
        );
    }

    #[test]
    fn test_heartbeat_schedule_hint_stays_valid_for_large_interval() {
        assert_eq!(heartbeat_schedule_hint(12_800), "0 * * * * * *");
        assert_eq!(heartbeat_schedule_hint(1_800), "0 */30 * * * * *");
    }

    #[test]
    fn test_next_fire_for_heartbeat_uses_persisted_interval() {
        let base = Utc
            .with_ymd_and_hms(2026, 4, 16, 8, 0, 0)
            .single()
            .expect("valid timestamp");
        let routine = Routine {
            id: Uuid::new_v4(),
            name: "__heartbeat__".to_string(),
            description: "internal".to_string(),
            user_id: "default".to_string(),
            actor_id: "default".to_string(),
            enabled: true,
            trigger: Trigger::Cron {
                schedule: "0 * * * * * *".to_string(),
            },
            action: RoutineAction::Heartbeat {
                light_context: true,
                prompt: None,
                include_reasoning: false,
                active_start_hour: None,
                active_end_hour: None,
                target: "chat".to_string(),
                max_iterations: 10,
                interval_secs: Some(12_800),
            },
            guardrails: RoutineGuardrails {
                cooldown: Duration::from_secs(6_400),
                max_concurrent: 1,
                dedup_window: None,
            },
            notify: NotifyConfig {
                channel: None,
                user: "default".to_string(),
                on_success: false,
                on_failure: true,
                on_attention: true,
            },
            policy: Default::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            config_version: 1,
            created_at: base,
            updated_at: base,
        };

        let next = next_fire_for_routine(&routine, None, base)
            .expect("heartbeat next fire")
            .expect("heartbeat should schedule");
        assert_eq!(next, base + ChronoDuration::seconds(12_800));
    }

    #[test]
    fn test_next_fire_for_heartbeat_falls_back_to_guardrail_interval() {
        let base = Utc
            .with_ymd_and_hms(2026, 4, 16, 8, 0, 0)
            .single()
            .expect("valid timestamp");
        let routine = Routine {
            id: Uuid::new_v4(),
            name: "__heartbeat__".to_string(),
            description: "internal".to_string(),
            user_id: "default".to_string(),
            actor_id: "default".to_string(),
            enabled: true,
            trigger: Trigger::Cron {
                schedule: "0 */213 * * * * *".to_string(),
            },
            action: RoutineAction::Heartbeat {
                light_context: true,
                prompt: None,
                include_reasoning: false,
                active_start_hour: None,
                active_end_hour: None,
                target: "chat".to_string(),
                max_iterations: 10,
                interval_secs: None,
            },
            guardrails: RoutineGuardrails {
                cooldown: Duration::from_secs(6_400),
                max_concurrent: 1,
                dedup_window: None,
            },
            notify: NotifyConfig {
                channel: None,
                user: "default".to_string(),
                on_success: false,
                on_failure: true,
                on_attention: true,
            },
            policy: Default::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            config_version: 1,
            created_at: base,
            updated_at: base,
        };

        let next = next_fire_for_routine(&routine, None, base)
            .expect("heartbeat next fire")
            .expect("heartbeat should schedule");
        assert_eq!(next, base + ChronoDuration::seconds(12_800));
    }

    #[test]
    fn test_next_schedule_fire_after_uses_explicit_base_time() {
        let base = Utc
            .with_ymd_and_hms(2026, 4, 16, 13, 30, 0)
            .single()
            .expect("valid timestamp");
        let next = next_schedule_fire_after_in_tz("0 0 12 * * * *", Tz::UTC, base)
            .expect("cron should parse")
            .expect("cron should produce next fire");
        assert_eq!(
            next,
            Utc.with_ymd_and_hms(2026, 4, 17, 12, 0, 0)
                .single()
                .expect("valid timestamp")
        );
    }

    #[test]
    fn test_next_fire_for_interval_trigger_uses_interval_schedule() {
        let base = Utc
            .with_ymd_and_hms(2026, 4, 16, 8, 0, 0)
            .single()
            .expect("valid timestamp");
        let routine = Routine {
            id: Uuid::new_v4(),
            name: "interval-check".to_string(),
            description: "fixed interval".to_string(),
            user_id: "default".to_string(),
            actor_id: "default".to_string(),
            enabled: true,
            trigger: Trigger::Cron {
                schedule: "every 90m".to_string(),
            },
            action: RoutineAction::Lightweight {
                prompt: "Check status".to_string(),
                context_paths: Vec::new(),
                max_tokens: 256,
            },
            guardrails: RoutineGuardrails::default(),
            notify: NotifyConfig::default(),
            policy: Default::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            config_version: 1,
            created_at: base,
            updated_at: base,
        };

        let next = next_fire_for_routine(&routine, None, base)
            .expect("interval next fire")
            .expect("interval should schedule");
        assert_eq!(next, base + ChronoDuration::minutes(90));
    }

    #[test]
    fn test_routine_schedule_uses_timezone_for_cron_and_interval() {
        let base = Utc
            .with_ymd_and_hms(2026, 4, 16, 8, 0, 0)
            .single()
            .expect("valid timestamp");

        let cron_routine = Routine {
            id: Uuid::new_v4(),
            name: "cron".to_string(),
            description: String::new(),
            user_id: "default".to_string(),
            actor_id: "default".to_string(),
            enabled: true,
            trigger: Trigger::Cron {
                schedule: "0 0 9 * * * *".to_string(),
            },
            action: RoutineAction::Lightweight {
                prompt: "Check".to_string(),
                context_paths: Vec::new(),
                max_tokens: 64,
            },
            guardrails: RoutineGuardrails::default(),
            notify: NotifyConfig::default(),
            policy: Default::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            config_version: 1,
            created_at: base,
            updated_at: base,
        };
        assert!(routine_schedule_uses_timezone(&cron_routine).expect("cron routine should parse"));

        let interval_routine = Routine {
            trigger: Trigger::Cron {
                schedule: "every 2h".to_string(),
            },
            ..cron_routine
        };
        assert!(
            !routine_schedule_uses_timezone(&interval_routine)
                .expect("interval routine should parse")
        );
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
                event_type: None,
                actor: None,
                metadata: None,
                pattern: String::new(),
                priority: 0,
            }
            .type_tag(),
            "event"
        );
        assert_eq!(
            Trigger::Webhook {
                path: None,
                secret: None,
                allow_unsigned_webhook: false,
            }
            .type_tag(),
            "webhook"
        );
        assert_eq!(Trigger::Manual.type_tag(), "manual");
    }
}
