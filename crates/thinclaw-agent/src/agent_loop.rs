//! Root-independent agent loop orchestration policy.

use std::time::Duration;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineGuardrails, Trigger, heartbeat_schedule_hint,
    next_fire_for_routine,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatRoutineConfig {
    pub interval_secs: u64,
    pub notify_channel: Option<String>,
    pub notify_user: Option<String>,
    pub light_context: bool,
    pub include_reasoning: bool,
    pub target: String,
    pub active_start_hour: Option<u8>,
    pub active_end_hour: Option<u8>,
    pub prompt: Option<String>,
    pub max_iterations: u32,
}

#[derive(Debug, Clone)]
pub struct HeartbeatRoutineSpec {
    pub schedule: String,
    pub action: RoutineAction,
    pub notify: NotifyConfig,
    pub guardrails: RoutineGuardrails,
}

impl HeartbeatRoutineSpec {
    pub fn from_config(config: &HeartbeatRoutineConfig) -> Self {
        let interval_secs = config.interval_secs.max(1);
        Self {
            schedule: heartbeat_schedule_hint(interval_secs),
            action: RoutineAction::Heartbeat {
                light_context: config.light_context,
                prompt: config.prompt.clone(),
                include_reasoning: config.include_reasoning,
                active_start_hour: config.active_start_hour,
                active_end_hour: config.active_end_hour,
                target: config.target.clone(),
                max_iterations: config.max_iterations,
                interval_secs: Some(interval_secs),
            },
            notify: NotifyConfig {
                channel: config.notify_channel.clone(),
                user: config
                    .notify_user
                    .clone()
                    .unwrap_or_else(|| "default".to_string()),
                on_attention: true,
                on_failure: true,
                on_success: false,
            },
            guardrails: RoutineGuardrails {
                cooldown: Duration::from_secs((config.interval_secs / 2).max(1)),
                max_concurrent: 1,
                dedup_window: None,
            },
        }
    }

    pub fn new_routine(
        &self,
        user_id: &str,
        actor_id: &str,
        user_timezone: Option<&str>,
        now: DateTime<Utc>,
    ) -> Routine {
        let mut routine = Routine {
            id: Uuid::new_v4(),
            name: "__heartbeat__".to_string(),
            description:
                "Periodic background awareness check - reads HEARTBEAT.md and acts on checklist items"
                    .to_string(),
            user_id: user_id.to_string(),
            actor_id: actor_id.to_string(),
            enabled: true,
            trigger: Trigger::Cron {
                schedule: self.schedule.clone(),
            },
            action: self.action.clone(),
            guardrails: self.guardrails.clone(),
            notify: self.notify.clone(),
            policy: Default::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            config_version: 1,
            created_at: now,
            updated_at: now,
        };
        routine.next_fire_at = next_fire_for_routine(&routine, user_timezone, now).unwrap_or(None);
        routine
    }

    pub fn routine_needs_update(&self, routine: &Routine) -> bool {
        let trigger_changed = match &routine.trigger {
            Trigger::Cron { schedule } => *schedule != self.schedule,
            _ => true,
        };
        let notify_changed = routine.notify.channel != self.notify.channel
            || routine.notify.user != self.notify.user
            || routine.notify.on_attention != self.notify.on_attention
            || routine.notify.on_failure != self.notify.on_failure
            || routine.notify.on_success != self.notify.on_success;
        let action_changed = routine.action.type_tag() != self.action.type_tag()
            || routine.action.to_config_json() != self.action.to_config_json();
        let guardrails_changed = routine.guardrails.cooldown != self.guardrails.cooldown
            || routine.guardrails.max_concurrent != self.guardrails.max_concurrent
            || routine.guardrails.dedup_window != self.guardrails.dedup_window;

        trigger_changed
            || notify_changed
            || action_changed
            || guardrails_changed
            || !routine.enabled
            || routine.next_fire_at.is_none()
    }

    pub fn apply_to_routine(
        &self,
        routine: &mut Routine,
        user_id: &str,
        actor_id: &str,
        user_timezone: Option<&str>,
        now: DateTime<Utc>,
    ) {
        routine.user_id = user_id.to_string();
        routine.actor_id = actor_id.to_string();
        routine.trigger = Trigger::Cron {
            schedule: self.schedule.clone(),
        };
        routine.enabled = true;
        routine.action = self.action.clone();
        routine.notify = self.notify.clone();
        routine.guardrails = self.guardrails.clone();
        routine.next_fire_at = next_fire_for_routine(routine, user_timezone, now).unwrap_or(None);
        routine.updated_at = now;
    }
}

pub fn routine_ownership_changed(routine: &Routine, user_id: &str, actor_id: &str) -> bool {
    routine.user_id != user_id || routine.owner_actor_id() != actor_id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> HeartbeatRoutineConfig {
        HeartbeatRoutineConfig {
            interval_secs: 1800,
            notify_channel: Some("telegram".to_string()),
            notify_user: Some("user-1".to_string()),
            light_context: true,
            include_reasoning: false,
            target: "chat".to_string(),
            active_start_hour: Some(8),
            active_end_hour: Some(22),
            prompt: Some("Check in".to_string()),
            max_iterations: 7,
        }
    }

    #[test]
    fn heartbeat_spec_captures_runtime_policy() {
        let spec = HeartbeatRoutineSpec::from_config(&config());
        assert_eq!(spec.notify.channel.as_deref(), Some("telegram"));
        assert_eq!(spec.notify.user, "user-1");
        assert_eq!(spec.guardrails.cooldown, Duration::from_secs(900));
        assert_eq!(
            spec.action.to_config_json()["interval_secs"],
            serde_json::json!(1800)
        );
    }

    #[test]
    fn heartbeat_update_detects_changed_notify_target() {
        let now = Utc::now();
        let spec = HeartbeatRoutineSpec::from_config(&config());
        let mut routine = spec.new_routine("user-1", "user-1", Some("Europe/Berlin"), now);
        assert!(!spec.routine_needs_update(&routine));

        routine.notify.user = "other".to_string();
        assert!(spec.routine_needs_update(&routine));
    }
}
