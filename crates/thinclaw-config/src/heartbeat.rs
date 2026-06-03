//! Heartbeat configuration.

use chrono::Timelike;
use thinclaw_settings::Settings;
use thinclaw_types::error::ConfigError;

use crate::helpers::{optional_env, parse_bool_env, parse_optional_env};

/// Heartbeat configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatConfig {
    /// Whether heartbeat is enabled.
    pub enabled: bool,
    /// Interval between heartbeat checks in seconds.
    pub interval_secs: u64,
    /// Channel to notify on heartbeat findings.
    pub notify_channel: Option<String>,
    /// User ID to notify on heartbeat findings.
    pub notify_user: Option<String>,
    /// Telegram forum topic ID for topic-targeted messages.
    pub notify_topic_id: Option<i64>,
    /// Use lightweight context (only HEARTBEAT.md, no session history).
    pub light_context: bool,
    /// Include LLM reasoning in heartbeat output.
    pub include_reasoning: bool,
    /// Output target: "chat" | "none" | channel name.
    pub target: String,
    /// Start hour for active window (0-23, local time). None = always active.
    pub active_start_hour: Option<u8>,
    /// End hour for active window (0-23, local time). None = always active.
    pub active_end_hour: Option<u8>,
    /// Custom heartbeat prompt body.
    pub prompt: Option<String>,
    /// Maximum tool iterations per heartbeat run.
    pub max_iterations: u32,
    /// User timezone (IANA). Used for active hours window check.
    pub user_timezone: Option<String>,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 1800,
            notify_channel: None,
            notify_user: None,
            notify_topic_id: None,
            light_context: true,
            include_reasoning: false,
            target: "chat".to_string(),
            active_start_hour: None,
            active_end_hour: None,
            prompt: None,
            max_iterations: 10,
            user_timezone: None,
        }
    }
}

impl HeartbeatConfig {
    pub fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("HEARTBEAT_ENABLED", settings.heartbeat.enabled)?,
            interval_secs: parse_optional_env(
                "HEARTBEAT_INTERVAL_SECS",
                settings.heartbeat.interval_secs,
            )?,
            notify_channel: optional_env("HEARTBEAT_NOTIFY_CHANNEL")?
                .or_else(|| settings.heartbeat.notify_channel.clone())
                .or_else(|| settings.notifications.preferred_channel.clone()),
            notify_user: optional_env("HEARTBEAT_NOTIFY_USER")?
                .or_else(|| settings.heartbeat.notify_user.clone())
                .or_else(|| settings.notifications.recipient.clone()),
            notify_topic_id: optional_env("HEARTBEAT_NOTIFY_TOPIC_ID")?
                .and_then(|s| s.parse::<i64>().ok()),
            light_context: settings.heartbeat.light_context,
            include_reasoning: settings.heartbeat.include_reasoning,
            target: settings.heartbeat.target.clone(),
            active_start_hour: settings.heartbeat.active_start_hour,
            active_end_hour: settings.heartbeat.active_end_hour,
            prompt: settings.heartbeat.prompt.clone(),
            max_iterations: settings.heartbeat.max_iterations,
            user_timezone: settings.user_timezone.clone(),
        })
    }

    /// Check if the current time falls within the configured active hours.
    ///
    /// Returns `true` if no active hours are configured (always active) or if
    /// the current hour in the user's timezone is within the [start, end) range.
    /// Falls back to the system timezone if no user timezone is configured.
    pub fn is_within_active_hours(&self) -> bool {
        let (start, end) = match (self.active_start_hour, self.active_end_hour) {
            (Some(s), Some(e)) => (s, e),
            _ => return true,
        };

        let tz = thinclaw_platform::timezone::resolve_timezone(
            None,
            self.user_timezone.as_deref(),
            &thinclaw_platform::timezone::detect_system_timezone().to_string(),
        );
        let now_hour = thinclaw_platform::timezone::now_in_tz(tz).hour() as u8;

        Self::contains_active_hour(start, end, now_hour)
    }

    pub fn contains_active_hour(start: u8, end: u8, hour: u8) -> bool {
        if start <= end {
            hour >= start && hour < end
        } else {
            hour >= start || hour < end
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::lock_env;

    #[test]
    fn resolve_defaults_from_settings() {
        let _guard = lock_env();
        unsafe {
            std::env::remove_var("HEARTBEAT_ENABLED");
            std::env::remove_var("HEARTBEAT_INTERVAL_SECS");
            std::env::remove_var("HEARTBEAT_NOTIFY_CHANNEL");
            std::env::remove_var("HEARTBEAT_NOTIFY_USER");
            std::env::remove_var("HEARTBEAT_NOTIFY_TOPIC_ID");
        }

        let settings = Settings::default();
        let cfg = HeartbeatConfig::resolve(&settings).expect("heartbeat config");
        assert_eq!(cfg.enabled, settings.heartbeat.enabled);
        assert_eq!(cfg.interval_secs, settings.heartbeat.interval_secs);
        assert_eq!(cfg.target, settings.heartbeat.target);
    }

    #[test]
    fn active_hour_ranges_handle_wrapping_windows() {
        assert!(HeartbeatConfig::contains_active_hour(22, 6, 23));
        assert!(HeartbeatConfig::contains_active_hour(22, 6, 5));
        assert!(!HeartbeatConfig::contains_active_hour(22, 6, 12));
        assert!(HeartbeatConfig::contains_active_hour(8, 22, 12));
        assert!(!HeartbeatConfig::contains_active_hour(8, 22, 22));
    }
}
