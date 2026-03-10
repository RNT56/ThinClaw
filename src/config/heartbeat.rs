use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Heartbeat configuration.
#[derive(Debug, Clone)]
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

    // ── Phase 3: Enhanced config fields ────────────────────────────
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
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 1800, // 30 minutes
            notify_channel: None,
            notify_user: None,
            notify_topic_id: None,
            light_context: true,
            include_reasoning: false,
            target: "chat".to_string(),
            active_start_hour: None,
            active_end_hour: None,
            prompt: None,
        }
    }
}

impl HeartbeatConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("HEARTBEAT_ENABLED", settings.heartbeat.enabled)?,
            interval_secs: parse_optional_env(
                "HEARTBEAT_INTERVAL_SECS",
                settings.heartbeat.interval_secs,
            )?,
            notify_channel: optional_env("HEARTBEAT_NOTIFY_CHANNEL")?
                .or_else(|| settings.heartbeat.notify_channel.clone()),
            notify_user: optional_env("HEARTBEAT_NOTIFY_USER")?
                .or_else(|| settings.heartbeat.notify_user.clone()),
            notify_topic_id: optional_env("HEARTBEAT_NOTIFY_TOPIC_ID")?
                .and_then(|s| s.parse::<i64>().ok()),
            // Phase 3 fields — read from settings (no env var override needed)
            light_context: settings.heartbeat.light_context,
            include_reasoning: settings.heartbeat.include_reasoning,
            target: settings.heartbeat.target.clone(),
            active_start_hour: settings.heartbeat.active_start_hour,
            active_end_hour: settings.heartbeat.active_end_hour,
            prompt: settings.heartbeat.prompt.clone(),
        })
    }

    /// Check if the current time falls within the configured active hours.
    ///
    /// Returns `true` if no active hours are configured (always active)
    /// or if the current local hour is within the [start, end) range.
    pub fn is_within_active_hours(&self) -> bool {
        let (start, end) = match (self.active_start_hour, self.active_end_hour) {
            (Some(s), Some(e)) => (s, e),
            _ => return true, // No restriction
        };

        let now_hour = chrono::Local::now().hour() as u8;

        if start <= end {
            // Normal range: e.g. 8..22
            now_hour >= start && now_hour < end
        } else {
            // Wrapping range: e.g. 22..6 (overnight)
            now_hour >= start || now_hour < end
        }
    }
}

use chrono::Timelike;
