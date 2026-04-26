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

/// Tunnel settings for public webhook endpoints.
///
/// The tunnel URL is shared across all channels that need webhooks.
/// Two modes:
/// - **Static URL**: `public_url` set directly (manual tunnel management).
/// - **Managed provider**: `provider` is set and the agent starts/stops the
///   tunnel process automatically at boot/shutdown.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TunnelSettings {
    /// Public URL from tunnel provider (e.g., "https://abc123.ngrok.io").
    /// When set without a provider, treated as a static (externally managed) URL.
    #[serde(default)]
    pub public_url: Option<String>,

    /// Managed tunnel provider: "ngrok", "cloudflare", "tailscale", "custom".
    #[serde(default)]
    pub provider: Option<String>,

    /// Cloudflare tunnel token.
    #[serde(default)]
    pub cf_token: Option<String>,

    /// ngrok auth token.
    #[serde(default)]
    pub ngrok_token: Option<String>,

    /// ngrok custom domain (paid plans).
    #[serde(default)]
    pub ngrok_domain: Option<String>,

    /// Use Tailscale Funnel (public) instead of Serve (tailnet-only).
    #[serde(default)]
    pub ts_funnel: bool,

    /// Tailscale hostname override.
    #[serde(default)]
    pub ts_hostname: Option<String>,

    /// Shell command for custom tunnel (with `{port}` / `{host}` placeholders).
    #[serde(default)]
    pub custom_command: Option<String>,

    /// Health check URL for custom tunnel.
    #[serde(default)]
    pub custom_health_url: Option<String>,

    /// Substring pattern to extract URL from custom tunnel stdout.
    #[serde(default)]
    pub custom_url_pattern: Option<String>,
}

/// Channel-specific settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSettings {
    /// Whether HTTP webhook channel is enabled.
    #[serde(default)]
    pub http_enabled: bool,

    /// Whether ACP stdio mode is enabled for editor integrations.
    #[serde(default)]
    pub acp_enabled: bool,

    /// HTTP webhook port (if enabled).
    #[serde(default)]
    pub http_port: Option<u16>,

    /// HTTP webhook host.
    #[serde(default)]
    pub http_host: Option<String>,

    /// Whether Signal channel is enabled.
    #[serde(default)]
    pub signal_enabled: bool,

    /// Signal HTTP URL (signal-cli daemon endpoint).
    #[serde(default)]
    pub signal_http_url: Option<String>,

    /// Signal account (E.164 phone number).
    #[serde(default)]
    pub signal_account: Option<String>,

    /// Signal allow from list for DMs (comma-separated E.164 phone numbers).
    /// Comma-separated identifiers: E.164 phone numbers, `*`, bare UUIDs, or `uuid:<id>` entries.
    /// Defaults to the configured account.
    #[serde(default)]
    pub signal_allow_from: Option<String>,

    /// Signal allow from groups (comma-separated group IDs).
    #[serde(default)]
    pub signal_allow_from_groups: Option<String>,

    /// Signal DM policy: "open", "allowlist", or "pairing". Default: "pairing".
    #[serde(default)]
    pub signal_dm_policy: Option<String>,

    /// Signal group policy: "allowlist", "open", or "disabled". Default: "allowlist".
    #[serde(default)]
    pub signal_group_policy: Option<String>,

    /// Signal group allow from (comma-separated group member IDs).
    /// If empty, inherits from signal_allow_from.
    #[serde(default)]
    pub signal_group_allow_from: Option<String>,

    /// Telegram owner user ID. When set, the bot only responds to this user.
    /// Captured during setup by having the user message the bot.
    #[serde(default)]
    pub telegram_owner_id: Option<i64>,

    /// Telegram progressive message streaming mode (e.g. "edit" or "status").
    #[serde(default)]
    pub telegram_stream_mode: Option<String>,

    /// Telegram transport mode.
    /// Supported values: "auto" and "polling".
    #[serde(default = "default_telegram_transport_mode")]
    pub telegram_transport_mode: String,

    /// How Telegram should surface temporary subagent sessions.
    /// Supported values: "temp_topic", "reply_chain", "compact_off".
    #[serde(default = "default_telegram_subagent_session_mode")]
    pub telegram_subagent_session_mode: String,

    // === Discord ===
    /// Whether Discord channel is enabled.
    #[serde(default)]
    pub discord_enabled: bool,

    /// Discord bot token.
    #[serde(default)]
    pub discord_bot_token: Option<String>,

    /// Discord guild ID (optional, restrict to single server).
    #[serde(default)]
    pub discord_guild_id: Option<String>,

    /// Discord allowed channel IDs (comma-separated, empty = all).
    #[serde(default)]
    pub discord_allow_from: Option<String>,

    /// Discord progressive message streaming mode (e.g. "edit" or "status").
    #[serde(default)]
    pub discord_stream_mode: Option<String>,

    // === Slack ===
    /// Whether Slack channel is enabled.
    #[serde(default)]
    pub slack_enabled: bool,

    /// Slack Bot User OAuth Token (xoxb-...).
    #[serde(default)]
    pub slack_bot_token: Option<String>,

    /// Slack App-Level Token (xapp-...) for Socket Mode.
    #[serde(default)]
    pub slack_app_token: Option<String>,

    /// Slack allowed channel/DM IDs (comma-separated, empty = all).
    #[serde(default)]
    pub slack_allow_from: Option<String>,

    // === Nostr ===
    /// Whether Nostr channel is enabled.
    #[serde(default)]
    pub nostr_enabled: bool,

    /// Nostr relay URLs (comma-separated).
    #[serde(default)]
    pub nostr_relays: Option<String>,

    /// Nostr owner public key (hex or npub) authorized to control the agent.
    #[serde(default)]
    pub nostr_owner_pubkey: Option<String>,

    /// Whether non-owner Nostr DMs are readable through the tool layer.
    #[serde(default)]
    pub nostr_social_dm_enabled: bool,

    /// Nostr public keys allowed to interact (comma-separated hex/npub, or '*').
    /// Deprecated for command authorization; kept for backward compatibility.
    #[serde(default)]
    pub nostr_allow_from: Option<String>,

    // === Gmail ===
    /// Whether Gmail channel is enabled.
    #[serde(default)]
    pub gmail_enabled: bool,

    /// GCP project ID for Gmail.
    #[serde(default)]
    pub gmail_project_id: Option<String>,

    /// Pub/Sub subscription ID for Gmail.
    #[serde(default)]
    pub gmail_subscription_id: Option<String>,

    /// Pub/Sub topic ID for Gmail.
    #[serde(default)]
    pub gmail_topic_id: Option<String>,

    /// Gmail allowed senders (comma-separated, empty = all).
    #[serde(default)]
    pub gmail_allowed_senders: Option<String>,

    // === BlueBubbles (cross-platform iMessage bridge) ===
    /// Whether BlueBubbles channel is enabled.
    #[serde(default)]
    pub bluebubbles_enabled: bool,

    /// BlueBubbles server URL (e.g. "http://192.168.1.50:1234").
    #[serde(default)]
    pub bluebubbles_server_url: Option<String>,

    /// BlueBubbles server password.
    #[serde(default)]
    pub bluebubbles_password: Option<String>,

    /// BlueBubbles webhook listen host (default: "127.0.0.1").
    #[serde(default)]
    pub bluebubbles_webhook_host: Option<String>,

    /// BlueBubbles webhook listen port (default: 8645).
    #[serde(default)]
    pub bluebubbles_webhook_port: Option<u16>,

    /// BlueBubbles webhook URL path (default: "/bluebubbles-webhook").
    #[serde(default)]
    pub bluebubbles_webhook_path: Option<String>,

    /// BlueBubbles allowed contacts (comma-separated phone/email, empty = all).
    #[serde(default)]
    pub bluebubbles_allow_from: Option<String>,

    /// Whether to send read receipts via BlueBubbles (default: true).
    #[serde(default)]
    pub bluebubbles_send_read_receipts: Option<bool>,

    // === iMessage (macOS only) ===
    /// Whether iMessage channel is enabled.
    #[serde(default)]
    pub imessage_enabled: bool,

    /// iMessage allowed contacts (comma-separated phone/email, empty = all).
    #[serde(default)]
    pub imessage_allow_from: Option<String>,

    /// iMessage polling interval in seconds.
    #[serde(default)]
    pub imessage_poll_interval: Option<u64>,

    // === Apple Mail (macOS only) ===
    /// Whether Apple Mail channel is enabled.
    #[serde(default)]
    pub apple_mail_enabled: bool,

    /// Apple Mail allowed sender addresses (comma-separated email, empty = all).
    #[serde(default)]
    pub apple_mail_allow_from: Option<String>,

    /// Apple Mail polling interval in seconds.
    #[serde(default)]
    pub apple_mail_poll_interval: Option<u64>,

    /// Only process unread messages.
    #[serde(default = "default_true")]
    pub apple_mail_unread_only: bool,

    /// Mark messages as read after processing.
    #[serde(default = "default_true")]
    pub apple_mail_mark_as_read: bool,

    // === Web Gateway ===
    /// Whether the Web Gateway is enabled.
    #[serde(default)]
    pub gateway_enabled: Option<bool>,

    /// Web Gateway bind host (default: 127.0.0.1).
    #[serde(default)]
    pub gateway_host: Option<String>,

    /// Web Gateway port (default: 3000).
    #[serde(default)]
    pub gateway_port: Option<u16>,

    /// Web Gateway auth token.
    #[serde(default)]
    pub gateway_auth_token: Option<String>,

    /// Whether the interactive CLI/REPL channel is enabled.
    #[serde(default)]
    pub cli_enabled: Option<bool>,

    /// Enabled WASM channels by name.
    /// Channels not in this list but present in the channels directory will still load.
    /// This is primarily used by the setup wizard to track which channels were configured.
    #[serde(default)]
    pub wasm_channels: Vec<String>,

    /// Whether WASM channels are enabled.
    #[serde(default = "default_true")]
    pub wasm_channels_enabled: bool,

    /// Directory containing WASM channel modules.
    #[serde(default)]
    pub wasm_channels_dir: Option<PathBuf>,
}

impl Default for ChannelSettings {
    fn default() -> Self {
        Self {
            http_enabled: false,
            acp_enabled: false,
            http_port: None,
            http_host: None,
            signal_enabled: false,
            signal_http_url: None,
            signal_account: None,
            signal_allow_from: None,
            signal_allow_from_groups: None,
            signal_dm_policy: None,
            signal_group_policy: None,
            signal_group_allow_from: None,
            telegram_owner_id: None,
            telegram_stream_mode: None,
            telegram_transport_mode: default_telegram_transport_mode(),
            telegram_subagent_session_mode: default_telegram_subagent_session_mode(),
            discord_enabled: false,
            discord_bot_token: None,
            discord_guild_id: None,
            discord_allow_from: None,
            discord_stream_mode: None,
            slack_enabled: false,
            slack_bot_token: None,
            slack_app_token: None,
            slack_allow_from: None,
            nostr_enabled: false,
            nostr_relays: None,
            nostr_owner_pubkey: None,
            nostr_social_dm_enabled: false,
            nostr_allow_from: None,
            gmail_enabled: false,
            gmail_project_id: None,
            gmail_subscription_id: None,
            gmail_topic_id: None,
            gmail_allowed_senders: None,
            bluebubbles_enabled: false,
            bluebubbles_server_url: None,
            bluebubbles_password: None,
            bluebubbles_webhook_host: None,
            bluebubbles_webhook_port: None,
            bluebubbles_webhook_path: None,
            bluebubbles_allow_from: None,
            bluebubbles_send_read_receipts: None,
            imessage_enabled: false,
            imessage_allow_from: None,
            imessage_poll_interval: None,
            apple_mail_enabled: false,
            apple_mail_allow_from: None,
            apple_mail_poll_interval: None,
            apple_mail_unread_only: true,
            apple_mail_mark_as_read: true,
            gateway_enabled: None,
            gateway_host: None,
            gateway_port: None,
            gateway_auth_token: None,
            cli_enabled: None,
            wasm_channels: Vec::new(),
            wasm_channels_enabled: true,
            wasm_channels_dir: None,
        }
    }
}
