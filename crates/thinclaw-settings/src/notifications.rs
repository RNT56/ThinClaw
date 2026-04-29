use super::*;

/// Global notification routing preferences.
///
/// Controls where proactive messages (heartbeats, routine alerts, self-repair)
/// are delivered. When a routine's own `NotifyConfig` has no channel/user set,
/// these global defaults are used.
///
/// - If only one channel is configured, it's auto-selected.
/// - If multiple channels exist, the user should explicitly set their preference.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotificationSettings {
    /// Preferred channel for proactive notifications.
    /// e.g. "telegram", "imessage", "bluebubbles", "signal", "web".
    /// None = broadcast to web only (safe default).
    #[serde(default)]
    pub preferred_channel: Option<String>,

    /// User identifier on the preferred channel.
    /// - Telegram: numeric chat ID (e.g. "123456789")
    /// - iMessage: phone number or Apple ID (e.g. "+4917612345678")
    /// - Signal: phone number (e.g. "+4917612345678")
    /// - Web: "default" (always works, no setup needed)
    /// None = use "default" (web-only, no external messaging).
    #[serde(default)]
    pub recipient: Option<String>,
}
