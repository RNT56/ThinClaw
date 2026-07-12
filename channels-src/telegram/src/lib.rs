// Telegram API types have fields reserved for future use (entities, reply threading, etc.)
#![allow(dead_code)]

//! Telegram Bot API channel for ThinClaw.
//!
//! This WASM component implements the channel interface for handling Telegram
//! webhooks and sending messages back via the Bot API.
//!
//! # Features
//!
//! - Webhook-based message receiving
//! - Private chat (DM) support
//! - Group chat support with @mention triggering
//! - Reply threading support
//! - User name extraction
//!
//! # Security
//!
//! - Bot token is injected by host during HTTP requests
//! - WASM never sees raw credentials
//! - Optional webhook secret validation by host

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};
/// Telegram Update object (webhook payload).
/// https://core.telegram.org/bots/api#update
#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    /// Unique update identifier.
    update_id: i64,

    /// New incoming message.
    message: Option<TelegramMessage>,

    /// Edited message.
    edited_message: Option<TelegramMessage>,

    /// Channel post (we ignore these for now).
    channel_post: Option<TelegramMessage>,
}

/// Telegram Message object.
/// https://core.telegram.org/bots/api#message
#[derive(Debug, Deserialize)]
struct TelegramMessage {
    /// Unique message identifier.
    message_id: i64,

    /// Sender (empty for channel posts).
    from: Option<TelegramUser>,

    /// Chat the message belongs to.
    chat: TelegramChat,

    /// Forum topic thread ID (for supergroups with Topics enabled).
    /// https://core.telegram.org/bots/api#message → message_thread_id
    message_thread_id: Option<i64>,

    /// Message text.
    text: Option<String>,

    /// Caption for media (photo, video, document, etc.).
    #[serde(default)]
    caption: Option<String>,

    /// Original message if this is a reply.
    reply_to_message: Option<Box<TelegramMessage>>,

    /// Bot command entities (for /commands).
    entities: Option<Vec<MessageEntity>>,

    /// Photo message — array of PhotoSize, sorted by size (last = largest).
    #[serde(default)]
    photo: Option<Vec<PhotoSize>>,

    /// Voice message (OGG/Opus audio).
    voice: Option<TelegramVoice>,

    /// Audio file sent as a music file.
    audio: Option<TelegramAudio>,

    /// General file/document.
    document: Option<TelegramDocument>,

    /// Video message.
    video: Option<TelegramVideo>,

    /// Video note (round video message).
    video_note: Option<TelegramVideoNote>,

    /// Sticker.
    sticker: Option<TelegramSticker>,
}

/// Telegram PhotoSize object.
/// https://core.telegram.org/bots/api#photosize
#[derive(Debug, Deserialize)]
struct PhotoSize {
    file_id: String,
    file_unique_id: String,
    width: i64,
    height: i64,
    file_size: Option<i64>,
}

/// Telegram Voice object.
/// https://core.telegram.org/bots/api#voice
#[derive(Debug, Deserialize)]
struct TelegramVoice {
    file_id: String,
    file_unique_id: String,
    duration: i64,
    mime_type: Option<String>,
    file_size: Option<i64>,
}

/// Telegram Audio object.
/// https://core.telegram.org/bots/api#audio
#[derive(Debug, Deserialize)]
struct TelegramAudio {
    file_id: String,
    file_unique_id: String,
    duration: i64,
    performer: Option<String>,
    title: Option<String>,
    file_name: Option<String>,
    mime_type: Option<String>,
    file_size: Option<i64>,
}

/// Telegram Document object.
/// https://core.telegram.org/bots/api#document
#[derive(Debug, Deserialize)]
struct TelegramDocument {
    file_id: String,
    file_unique_id: String,
    file_name: Option<String>,
    mime_type: Option<String>,
    file_size: Option<i64>,
}

/// Telegram Video object.
/// https://core.telegram.org/bots/api#video
#[derive(Debug, Deserialize)]
struct TelegramVideo {
    file_id: String,
    file_unique_id: String,
    width: i64,
    height: i64,
    duration: i64,
    file_name: Option<String>,
    mime_type: Option<String>,
    file_size: Option<i64>,
}

/// Telegram VideoNote object.
/// https://core.telegram.org/bots/api#videonote
#[derive(Debug, Deserialize)]
struct TelegramVideoNote {
    file_id: String,
    file_unique_id: String,
    length: i64,
    duration: i64,
    file_size: Option<i64>,
}

/// Telegram Sticker object (partial).
/// https://core.telegram.org/bots/api#sticker
#[derive(Debug, Deserialize)]
struct TelegramSticker {
    file_id: String,
    file_unique_id: String,
    width: i64,
    height: i64,
    is_animated: Option<bool>,
    is_video: Option<bool>,
    file_size: Option<i64>,
}

/// Telegram File object (response from getFile).
/// https://core.telegram.org/bots/api#file
#[derive(Debug, Deserialize)]
struct TelegramFile {
    file_id: String,
    file_unique_id: String,
    file_size: Option<i64>,
    file_path: Option<String>,
}

/// Telegram User object.
/// https://core.telegram.org/bots/api#user
#[derive(Debug, Deserialize)]
struct TelegramUser {
    /// Unique user identifier.
    id: i64,

    /// True if this is a bot.
    is_bot: bool,

    /// User's first name.
    first_name: String,

    /// User's last name.
    last_name: Option<String>,

    /// Username (without @).
    username: Option<String>,

    /// True if private-chat forum topic mode is enabled for the bot.
    #[serde(default)]
    has_topics_enabled: Option<bool>,

    /// True if users may create/delete private-chat topics for the bot.
    #[serde(default)]
    allows_users_to_create_topics: Option<bool>,
}

/// Telegram Chat object.
/// https://core.telegram.org/bots/api#chat
#[derive(Debug, Deserialize)]
struct TelegramChat {
    /// Unique chat identifier.
    id: i64,

    /// Type of chat: private, group, supergroup, or channel.
    #[serde(rename = "type")]
    chat_type: String,

    /// Title for groups/channels.
    title: Option<String>,

    /// Username for private chats.
    username: Option<String>,
}

/// Message entity (for parsing @mentions, commands, etc.).
/// https://core.telegram.org/bots/api#messageentity
#[derive(Debug, Deserialize)]
struct MessageEntity {
    /// Type: mention, bot_command, etc.
    #[serde(rename = "type")]
    entity_type: String,

    /// Offset in UTF-16 code units.
    offset: i64,

    /// Length in UTF-16 code units.
    length: i64,

    /// For "mention" type, the mentioned user.
    user: Option<TelegramUser>,
}

/// Telegram API response wrapper.
#[derive(Debug, Deserialize)]
struct TelegramApiResponse<T> {
    /// True if the request was successful.
    ok: bool,

    /// Error description if not ok.
    description: Option<String>,

    /// Result on success.
    result: Option<T>,
}

/// Response from sendMessage.
#[derive(Debug, Deserialize)]
struct SentMessage {
    message_id: i64,
}

/// Workspace path for storing polling state.
const POLLING_STATE_PATH: &str = "state/last_update_id";
/// Workspace path for the highest Telegram update ID that should be ignored.
/// Setup uses this to suppress the onboarding pairing/binding message if it
/// is redelivered when runtime starts (especially in webhook mode).
const IGNORE_UPDATES_UNTIL_ID_PATH: &str = "state/ignore_updates_until_id";

/// Workspace path for persisting owner_id across WASM callbacks.
const OWNER_ID_PATH: &str = "state/owner_id";

/// Workspace path for persisting dm_policy across WASM callbacks.
const DM_POLICY_PATH: &str = "state/dm_policy";

/// Workspace path for persisting allow_from (JSON array) across WASM callbacks.
const ALLOW_FROM_PATH: &str = "state/allow_from";

/// Channel name for pairing store (used by pairing host APIs).
const CHANNEL_NAME: &str = "telegram";

/// Workspace path for persisting bot_username for mention detection in groups.
const BOT_USERNAME_PATH: &str = "state/bot_username";

/// Workspace path for persisting respond_to_all_group_messages flag.
const RESPOND_TO_ALL_GROUP_PATH: &str = "state/respond_to_all_group_messages";

/// Workspace path for the configured subagent session mode.
const SUBAGENT_SESSION_MODE_PATH: &str = "state/subagent_session_mode";

/// Workspace path for active subagent session routing state.
const SUBAGENT_SESSIONS_PATH: &str = "state/subagent_sessions";

/// Workspace path for remembering last orphan-session GC run.
const SUBAGENT_GC_LAST_RUN_PATH: &str = "state/subagent_gc_last_run";

/// TTL for subagent routing sessions. Stale entries are removed to avoid
/// indefinite growth when subagent completion events are never observed.
const SUBAGENT_SESSION_TTL_SECS: u64 = 6 * 60 * 60;

/// Maximum number of persisted subagent sessions to retain.
const SUBAGENT_SESSION_STORE_CAP: usize = 256;

/// Minimum interval between periodic orphan-session GC runs.
const SUBAGENT_GC_INTERVAL_SECS: u64 = 5 * 60;

/// Telegram limits messages to 4096 UTF-8 characters.
/// https://core.telegram.org/bots/api#sendmessage
const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;

/// Workspace path for transport mode telemetry.
const TRANSPORT_MODE_PATH: &str = "state/transport_mode";
/// Workspace path for the configured transport preference.
const TRANSPORT_PREFERENCE_PATH: &str = "state/transport_preference";
/// Workspace path for a human-readable transport selection reason.
const TRANSPORT_REASON_PATH: &str = "state/transport_reason";
/// Workspace path for the expected webhook URL.
const EXPECTED_WEBHOOK_URL_PATH: &str = "state/expected_webhook_url";
/// Workspace path for last webhook registration timestamp.
const LAST_WEBHOOK_REGISTER_AT_PATH: &str = "state/last_webhook_register_at";
/// Workspace path for last webhook registration error text.
const LAST_WEBHOOK_REGISTER_ERROR_PATH: &str = "state/last_webhook_register_error";
/// Workspace path for last polling start timestamp.
const LAST_POLL_STARTED_AT_PATH: &str = "state/last_poll_started_at";
/// Workspace path for last polling success timestamp.
const LAST_POLL_SUCCESS_AT_PATH: &str = "state/last_poll_success_at";
/// Workspace path for last polling error.
const LAST_POLL_ERROR_PATH: &str = "state/last_poll_error";
/// Workspace path for last inbound update timestamp.
const LAST_INBOUND_AT_PATH: &str = "state/last_inbound_at";
/// Workspace path for last emitted update identifier.
const LAST_EMITTED_UPDATE_ID_PATH: &str = "state/last_emitted_update_id";
/// Workspace path for the most recent transport error.
const LAST_TRANSPORT_ERROR_PATH: &str = "state/last_transport_error";
/// Workspace path for whether private bot topics are enabled.
const PRIVATE_TOPICS_ENABLED_PATH: &str = "state/private_topics_enabled";
/// Workspace path for whether users may create private-chat topics.
const PRIVATE_TOPICS_ALLOW_USER_CREATE_PATH: &str = "state/private_topics_allow_user_create";
/// Workspace path for managed durable private-chat topic mappings.
const MANAGED_PRIVATE_TOPICS_PATH: &str = "state/managed_private_topics";
/// Workspace path prefix for last active thread per chat.
/// Key: `state/last_active_thread/{chat_id}`, value: `message_thread_id`.
/// Used as a fallback for sendChatAction when the metadata lacks a thread ID
/// (Telegram quirk: the General topic omits message_thread_id).
const LAST_ACTIVE_THREAD_PREFIX: &str = "state/last_active_thread/";

// ============================================================================
// Channel Metadata
// ============================================================================

/// Metadata stored with emitted messages for response routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelegramMessageMetadata {
    /// Chat ID where the message was received.
    chat_id: i64,

    /// Original message ID (for reply_to_message_id).
    message_id: i64,

    /// User ID who sent the message.
    user_id: i64,

    /// Whether this is a private (DM) chat.
    is_private: bool,

    /// Forum topic thread ID (for supergroups with Topics enabled).
    /// When present, all replies must include this to target the correct topic.
    #[serde(default)]
    message_thread_id: Option<i64>,

    /// Normalized conversation kind for downstream resolver logic.
    #[serde(default)]
    conversation_kind: Option<String>,

    /// Stable scope identifier for the direct chat or group topic.
    #[serde(default)]
    conversation_scope_id: Option<String>,

    /// Stable external conversation key used for cross-channel continuity.
    #[serde(default)]
    external_conversation_key: Option<String>,

    /// Raw sender identifier from Telegram.
    #[serde(default)]
    raw_sender_id: Option<String>,

    /// Stable sender identifier used for continuity within Telegram.
    #[serde(default)]
    stable_sender_id: Option<String>,

    /// Optional per-message override for subagent session rendering mode.
    #[serde(default, alias = "telegram_subagent_session_mode")]
    subagent_session_mode: Option<String>,
}

/// Channel configuration injected by host.
///
/// The host injects runtime values like tunnel_url and webhook_secret.
/// Telegram transport is selected from the host-provided preference plus the
/// effective tunnel URL that survived suitability checks.
#[derive(Debug, Deserialize)]
struct TelegramConfig {
    /// Bot username (without @) for mention detection in groups.
    #[serde(default)]
    bot_username: Option<String>,

    /// Telegram user ID of the bot owner. When set, only messages from this
    /// user are processed. All others are silently dropped.
    #[serde(default)]
    owner_id: Option<i64>,

    /// DM policy: "pairing" (default), "allowlist", or "open".
    #[serde(default)]
    dm_policy: Option<String>,

    /// Allowed sender IDs/usernames from config (merged with pairing-approved store).
    #[serde(default)]
    allow_from: Option<Vec<String>>,

    /// Whether to respond to all group messages (not just mentions).
    #[serde(default)]
    respond_to_all_group_messages: bool,

    /// Public tunnel URL for webhook mode (injected by host from global settings).
    /// When set and transport_preference allows it, webhook mode is enabled.
    #[serde(default)]
    tunnel_url: Option<String>,

    /// Original host tunnel URL before Telegram-specific suitability filtering.
    #[serde(default)]
    host_tunnel_url: Option<String>,

    /// Whether the host determined that webhook ingress is publicly reachable.
    #[serde(default)]
    host_webhook_capable: bool,

    /// Host explanation for why webhook transport is not currently suitable.
    #[serde(default)]
    host_transport_reason: Option<String>,

    /// Secret token for webhook validation (injected by host from secrets store).
    /// Telegram will include this in the X-Telegram-Bot-Api-Secret-Token header.
    #[serde(default)]
    webhook_secret: Option<String>,

    /// How Telegram ingress should be chosen.
    /// Supported values: "auto" and "polling".
    #[serde(
        default,
        alias = "telegram_transport_mode",
        alias = "channels.telegram_transport_mode"
    )]
    transport_preference: Option<String>,

    /// Human-readable explanation for the chosen transport when polling is forced.
    #[serde(default)]
    transport_reason: Option<String>,

    /// How subagent activity should be surfaced in Telegram.
    /// Supported values: "temp_topic", "reply_chain", "compact_off".
    #[serde(
        default,
        alias = "telegram_subagent_session_mode",
        alias = "channels.telegram_subagent_session_mode"
    )]
    subagent_session_mode: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedPrivateTopicKind {
    Onboarding,
    General,
}

impl ManagedPrivateTopicKind {
    fn from_response_thread_id(thread_id: Option<&str>) -> Option<Self> {
        match thread_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some("bootstrap") | Some("onboarding") => Some(Self::Onboarding),
            Some("boot") | Some("general") => Some(Self::General),
            _ => None,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Onboarding => "Onboarding",
            Self::General => "General",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct ManagedPrivateTopicRegistry {
    #[serde(default)]
    chats: std::collections::HashMap<String, ManagedPrivateTopicState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct ManagedPrivateTopicState {
    #[serde(default)]
    onboarding_thread_id: Option<i64>,
    #[serde(default)]
    general_thread_id: Option<i64>,
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct TelegramChannel;

#[derive(Debug, Clone, PartialEq, Eq)]
enum TelegramStatusAction {
    Typing,
    Notify(String),
    Subagent(SubagentEvent),
}

const TELEGRAM_STATUS_MAX_CHARS: usize = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum TelegramSubagentSessionMode {
    #[default]
    TempTopic,
    ReplyChain,
    CompactOff,
}

impl TelegramSubagentSessionMode {
    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "temp_topic" | "temptopic" | "topic" => Some(Self::TempTopic),
            "reply_chain" | "replychain" | "reply" => Some(Self::ReplyChain),
            "compact_off" | "compact" | "off" => Some(Self::CompactOff),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::TempTopic => "temp_topic",
            Self::ReplyChain => "reply_chain",
            Self::CompactOff => "compact_off",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum TelegramTransportPreference {
    #[default]
    Auto,
    Polling,
}

impl TelegramTransportPreference {
    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" | "automatic" | "webhook" => Some(Self::Auto),
            "polling" | "poll" | "off" | "disabled" => Some(Self::Polling),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Polling => "polling",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SubagentEvent {
    Spawned {
        agent_id: String,
        name: String,
        task: String,
    },
    Progress {
        agent_id: String,
        category: String,
        message: String,
    },
    Completed {
        agent_id: String,
        name: String,
        success: bool,
        response: Option<String>,
        duration_ms: Option<u64>,
        iterations: Option<usize>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StoredSubagentSession {
    chat_id: i64,
    parent_message_id: i64,
    parent_thread_id: Option<i64>,
    topic_thread_id: Option<i64>,
    mode: String,
    #[serde(default = "now_epoch_secs")]
    last_touched_epoch_secs: u64,
}

#[derive(Debug, Deserialize)]
struct ForumTopic {
    message_thread_id: i64,
}

impl Guest for TelegramChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Telegram channel config: {}", config_json),
        );

        let raw_config: serde_json::Value = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config JSON: {}", e))?;
        let config: TelegramConfig = serde_json::from_value(raw_config.clone())
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "Telegram channel starting");

        if let Some(ref username) = config.bot_username {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Bot username: @{}", username),
            );
        }

        // Persist owner_id so subsequent callbacks (on_http_request, on_poll) can read it
        if let Some(owner_id) = config.owner_id {
            if let Err(e) = channel_host::workspace_write(OWNER_ID_PATH, &owner_id.to_string()) {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to persist owner_id: {}", e),
                );
            }
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Owner restriction enabled: user {}", owner_id),
            );
        } else {
            // Clear any stale owner_id from a previous config
            let _ = channel_host::workspace_write(OWNER_ID_PATH, "");
            channel_host::log(
                channel_host::LogLevel::Warn,
                "No owner_id configured, bot is open to all users",
            );
        }

        // Persist dm_policy and allow_from for DM pairing in handle_message
        let dm_policy = config.dm_policy.as_deref().unwrap_or("pairing").to_string();
        let _ = channel_host::workspace_write(DM_POLICY_PATH, &dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        // Persist bot_username and respond_to_all_group_messages for group handling
        let _ = channel_host::workspace_write(
            BOT_USERNAME_PATH,
            &config.bot_username.unwrap_or_default(),
        );
        let _ = channel_host::workspace_write(
            RESPOND_TO_ALL_GROUP_PATH,
            &config.respond_to_all_group_messages.to_string(),
        );

        let configured_subagent_mode = config
            .subagent_session_mode
            .clone()
            .or_else(|| extract_subagent_session_mode_from_value(&raw_config));
        let subagent_mode = configured_subagent_mode
            .as_deref()
            .and_then(TelegramSubagentSessionMode::from_str)
            .unwrap_or_default();
        let _ = channel_host::workspace_write(SUBAGENT_SESSION_MODE_PATH, subagent_mode.as_str());
        run_subagent_session_gc(true);

        let transport_preference = config
            .transport_preference
            .as_deref()
            .and_then(TelegramTransportPreference::from_str)
            .unwrap_or_default();
        let transport_reason =
            config
                .transport_reason
                .clone()
                .or_else(|| match transport_preference {
                    TelegramTransportPreference::Polling => {
                        Some("operator forced polling".to_string())
                    }
                    TelegramTransportPreference::Auto
                        if config.host_webhook_capable
                            && config.host_tunnel_url.is_some()
                            && config.tunnel_url.is_none() =>
                    {
                        Some(
                            "runtime fallback forced polling after webhook instability".to_string(),
                        )
                    }
                    TelegramTransportPreference::Auto if !config.host_webhook_capable => {
                        config.host_transport_reason.clone().or_else(|| {
                            Some("no suitable public HTTPS webhook URL is available".to_string())
                        })
                    }
                    TelegramTransportPreference::Auto => None,
                });

        let mut webhook_mode = transport_preference == TelegramTransportPreference::Auto
            && config.tunnel_url.is_some();
        let expected_webhook_url = if webhook_mode {
            config
                .tunnel_url
                .as_ref()
                .map(|url| format!("{}/webhook/telegram", url.trim_end_matches('/')))
        } else {
            None
        };

        write_workspace_state(
            TRANSPORT_MODE_PATH,
            if webhook_mode { "webhook" } else { "polling" },
        );
        write_workspace_state(TRANSPORT_PREFERENCE_PATH, transport_preference.as_str());
        if let Some(reason) = transport_reason.as_deref() {
            write_workspace_state(TRANSPORT_REASON_PATH, reason);
        } else {
            clear_workspace_state(TRANSPORT_REASON_PATH);
        }
        write_workspace_state(
            EXPECTED_WEBHOOK_URL_PATH,
            expected_webhook_url.as_deref().unwrap_or(""),
        );
        clear_workspace_state(LAST_WEBHOOK_REGISTER_ERROR_PATH);
        clear_workspace_state(LAST_POLL_ERROR_PATH);
        clear_transport_error();
        clear_workspace_state(PRIVATE_TOPICS_ENABLED_PATH);
        clear_workspace_state(PRIVATE_TOPICS_ALLOW_USER_CREATE_PATH);
        probe_private_topic_settings();

        if webhook_mode {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Webhook mode enabled (public ingress available)",
            );

            // Register webhook with Telegram API
            if let Some(ref tunnel_url) = config.tunnel_url {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!("Registering webhook: {}/webhook/telegram", tunnel_url),
                );

                if let Err(e) = register_webhook(tunnel_url, config.webhook_secret.as_deref()) {
                    write_workspace_state(LAST_WEBHOOK_REGISTER_AT_PATH, &now_millis_string());
                    write_workspace_state(LAST_WEBHOOK_REGISTER_ERROR_PATH, &e);
                    set_transport_error(&e);
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!(
                            "Failed to register webhook: {} — falling back to polling mode",
                            e
                        ),
                    );
                    // Fall back to polling mode — delete any stale webhook so
                    // getUpdates works, then flip the mode flag.
                    let _ = delete_webhook();
                    webhook_mode = false;
                    write_workspace_state(TRANSPORT_MODE_PATH, "polling");
                } else {
                    write_workspace_state(LAST_WEBHOOK_REGISTER_AT_PATH, &now_millis_string());
                    clear_workspace_state(LAST_WEBHOOK_REGISTER_ERROR_PATH);
                    clear_transport_error();
                }
            }
        } else {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!(
                    "Polling mode enabled ({})",
                    transport_reason
                        .as_deref()
                        .unwrap_or("no suitable public webhook URL")
                ),
            );

            // Delete any existing webhook before polling
            // Telegram doesn't allow getUpdates while a webhook is active
            if let Err(e) = delete_webhook() {
                set_transport_error(&e);
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to delete webhook (may not exist): {}", e),
                );
            }
        }

        // Configure polling only if not in webhook mode
        let poll = if !webhook_mode {
            Some(PollConfig {
                interval_ms: 5000, // 5 seconds between poll ticks
                enabled: true,
            })
        } else {
            None
        };

        // Webhook secret validation is handled by the host
        let require_secret = config.webhook_secret.is_some();

        Ok(ChannelConfig {
            display_name: "Telegram".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/telegram".to_string(),
                methods: vec!["POST".to_string()],
                require_secret,
            }],
            poll,
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Keep orphan-session GC active in webhook mode as well.
        // The interval gate in run_subagent_session_gc avoids per-request overhead.
        run_subagent_session_gc(false);

        // Check if webhook secret validation passed (if required)
        // The host validates X-Telegram-Bot-Api-Secret-Token header and sets secret_validated
        // If require_secret was true in config but validation failed, secret_validated will be false
        if !req.secret_validated {
            // This means require_secret was set but the secret didn't match
            // We still check the field even though the host should have already rejected invalid requests
            // This is defense in depth
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Webhook request with invalid or missing secret token",
            );
            return json_response(
                401,
                serde_json::json!({"error": "Invalid or missing webhook secret token"}),
            );
        }

        // Parse the request body as UTF-8
        let body_str = match std::str::from_utf8(&req.body) {
            Ok(s) => s,
            Err(_) => {
                return json_response(400, serde_json::json!({"error": "Invalid UTF-8 body"}));
            }
        };

        // Parse as Telegram Update
        let update: TelegramUpdate = match serde_json::from_str(body_str) {
            Ok(u) => u,
            Err(e) => {
                set_transport_error(&format!("Invalid webhook payload: {}", e));
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse Telegram update: {}", e),
                );
                // Still return 200 to prevent Telegram from retrying
                return json_response(200, serde_json::json!({"ok": true}));
            }
        };

        // Handle the update
        handle_update(update);
        clear_transport_error();

        // Always respond 200 quickly (Telegram expects fast responses)
        json_response(200, serde_json::json!({"ok": true}))
    }

    fn on_poll() {
        run_subagent_session_gc(false);
        write_workspace_state(LAST_POLL_STARTED_AT_PATH, &now_millis_string());
        // Read last offset from workspace storage
        let offset = match channel_host::workspace_read(POLLING_STATE_PATH) {
            Some(s) => s.parse::<i64>().unwrap_or(0),
            None => 0,
        };

        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Polling getUpdates with offset {}", offset),
        );

        let headers_json = serde_json::json!({}).to_string();
        // The WASM host enforces a 30s callback_timeout on on_poll.
        // Use a 20s long-poll so the full cycle (WASM startup + HTTP + processing)
        // fits comfortably within the 30s window.  Previous value of 30s caused
        // the callback to ALWAYS time out, leaving orphaned HTTP requests and
        // triggering 409 conflicts on subsequent getUpdates calls.
        let primary_url = get_updates_url(offset, 20);

        // 25s HTTP timeout outlives Telegram's 20s server-side long-poll.
        let result = match channel_host::http_request(
            "GET",
            &primary_url,
            &headers_json,
            None,
            Some(25_000),
        ) {
            Ok(response) => Ok(response),
            Err(primary_err) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!(
                        "getUpdates request failed ({}), retrying once immediately",
                        primary_err
                    ),
                );

                let retry_url = get_updates_url(offset, 3);
                channel_host::http_request("GET", &retry_url, &headers_json, None, Some(8_000))
                    .map_err(|retry_err| {
                        format!("primary error: {}; retry error: {}", primary_err, retry_err)
                    })
            }
        };

        match result {
            Ok(response) => {
                if response.status != 200 {
                    let body_str = String::from_utf8_lossy(&response.body);
                    let error = format!("getUpdates returned {}: {}", response.status, body_str);
                    write_workspace_state(LAST_POLL_ERROR_PATH, &error);
                    set_transport_error(&error);
                    channel_host::log(channel_host::LogLevel::Error, &error);
                    return;
                }

                // Parse response
                let api_response: Result<TelegramApiResponse<Vec<TelegramUpdate>>, _> =
                    serde_json::from_slice(&response.body);

                match api_response {
                    Ok(resp) if resp.ok => {
                        write_workspace_state(LAST_POLL_SUCCESS_AT_PATH, &now_millis_string());
                        clear_workspace_state(LAST_POLL_ERROR_PATH);
                        clear_transport_error();
                        if let Some(updates) = resp.result {
                            let mut new_offset = offset;

                            for update in updates {
                                // Track highest update_id for next poll
                                if update.update_id >= new_offset {
                                    new_offset = update.update_id + 1;
                                }

                                // Process the update (emits messages)
                                handle_update(update);
                            }

                            // Save new offset if it changed
                            if new_offset != offset {
                                if let Err(e) = channel_host::workspace_write(
                                    POLLING_STATE_PATH,
                                    &new_offset.to_string(),
                                ) {
                                    channel_host::log(
                                        channel_host::LogLevel::Error,
                                        &format!("Failed to save polling offset: {}", e),
                                    );
                                }
                            }
                        }
                    }
                    Ok(resp) => {
                        let error = format!(
                            "Telegram API error: {}",
                            resp.description.unwrap_or_else(|| "unknown".to_string())
                        );
                        write_workspace_state(LAST_POLL_ERROR_PATH, &error);
                        set_transport_error(&error);
                        channel_host::log(channel_host::LogLevel::Error, &error);
                    }
                    Err(e) => {
                        let error = format!("Failed to parse getUpdates response: {}", e);
                        write_workspace_state(LAST_POLL_ERROR_PATH, &error);
                        set_transport_error(&error);
                        channel_host::log(channel_host::LogLevel::Error, &error);
                    }
                }
            }
            Err(e) => {
                let error = format!("getUpdates request failed: {}", e);
                write_workspace_state(LAST_POLL_ERROR_PATH, &error);
                set_transport_error(&error);
                channel_host::log(channel_host::LogLevel::Error, &error);
            }
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: TelegramMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;
        let mut target_thread_id =
            resolve_outgoing_message_thread_id(&metadata, response.thread_id.as_deref());

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "on_respond: chat_id={}, metadata_thread_id={:?}, target_thread_id={:?}, requested_thread_id={:?}, is_private={}, metadata_json={}",
                metadata.chat_id,
                metadata.message_thread_id,
                target_thread_id,
                response.thread_id,
                metadata.is_private,
                response.metadata_json
            ),
        );

        // Convert standard Markdown (from LLM output) to Telegram-safe HTML
        let html_content = markdown_to_telegram_html(&response.content);
        let chunks = split_message(&html_content, TELEGRAM_MAX_MESSAGE_LENGTH);

        // Track whether we've already attempted a thread recovery for this
        // response. We only retry once to avoid infinite loops.
        let mut thread_recovered = false;

        for (i, chunk) in chunks.iter().enumerate() {
            // Unconditionally set reply_to = None. If we reply_to a message directly, Telegram
            // clients implicitly group it into a sub-thread 'Replies' view which isolates the
            // conversation from the main flow. Passing None keeps it in the active chat/topic.
            let reply_to = None;

            // Try sending with HTML first; fall back to plain text if Telegram
            // can't parse the entities.
            let result = send_message(
                metadata.chat_id,
                chunk,
                reply_to,
                Some("HTML"),
                target_thread_id,
            );

            match result {
                Ok(msg_id) => {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!(
                            "Sent message chunk {}/{} to chat {}: message_id={}",
                            i + 1,
                            chunks.len(),
                            metadata.chat_id,
                            msg_id
                        ),
                    );
                }
                Err(SendError::ThreadNotFound(detail)) if !thread_recovered => {
                    // The managed topic was deleted by the user. Invalidate the
                    // cached thread_id and re-resolve (which creates a new topic).
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!(
                            "Managed thread not found ({}), invalidating and recreating",
                            detail
                        ),
                    );

                    // Invalidate both managed topic kinds for this chat — we
                    // don't know which one was targeted, but they share the chat.
                    if metadata.is_private {
                        if let Some(kind) = ManagedPrivateTopicKind::from_response_thread_id(
                            response.thread_id.as_deref(),
                        ) {
                            invalidate_managed_private_topic(metadata.chat_id, kind);
                        }
                    }

                    // Re-resolve the thread_id (this will create a new topic)
                    target_thread_id = resolve_outgoing_message_thread_id(
                        &metadata,
                        response.thread_id.as_deref(),
                    );
                    thread_recovered = true;

                    // Retry this chunk with the new thread_id
                    send_message(
                        metadata.chat_id,
                        chunk,
                        reply_to,
                        Some("HTML"),
                        target_thread_id,
                    )
                    .map_err(|e| format!("Retry after thread recreation also failed: {}", e))?;
                }
                Err(SendError::ParseEntities(detail)) => {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("HTML parse failed ({}), retrying as plain text", detail),
                    );
                    // Fall back to original plain-text content for this chunk
                    let plain_chunks =
                        split_message(&response.content, TELEGRAM_MAX_MESSAGE_LENGTH);
                    let plain_chunk = plain_chunks.get(i).map(|s| s.as_str()).unwrap_or(chunk);
                    send_message(
                        metadata.chat_id,
                        plain_chunk,
                        reply_to,
                        None,
                        target_thread_id,
                    )
                    .map_err(|e| format!("Plain-text retry also failed: {}", e))?;
                }
                Err(e) => return Err(e.to_string()),
            }
        }

        Ok(())
    }

    fn on_status(update: StatusUpdate) {
        let action = match classify_status_update(&update) {
            Some(action) => action,
            None => return,
        };

        // Parse chat_id from metadata
        let metadata: TelegramMessageMetadata = match parse_telegram_metadata(&update.metadata_json)
        {
            Ok(m) => m,
            Err(_) => {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    "on_status: no valid Telegram metadata, skipping status update",
                );
                return;
            }
        };

        if should_ensure_general_topic_after_status(&metadata, &update) {
            let _ =
                ensure_managed_private_topic(metadata.chat_id, ManagedPrivateTopicKind::General);
        }

        match action {
            TelegramStatusAction::Typing => {
                // POST /sendChatAction with action "typing"
                let mut payload = serde_json::json!({
                    "chat_id": metadata.chat_id,
                    "action": "typing"
                });

                // Resolve the forum thread ID for the typing indicator.
                // Telegram's "General" topic quirk: the API omits
                // message_thread_id for messages in the General topic, so
                // metadata.message_thread_id may be None even though the
                // conversation is happening inside a thread.
                //
                // Universal fallback: use the last active thread for this
                // chat (persisted on every incoming message). This covers
                // the General topic, user-created threads, and bot-created
                // threads — any case where the metadata lacks a thread ID.
                let effective_thread_id = metadata.message_thread_id.or_else(|| {
                    let path = format!("{}{}", LAST_ACTIVE_THREAD_PREFIX, metadata.chat_id);
                    channel_host::workspace_read(&path).and_then(|v| v.parse::<i64>().ok())
                });

                if let Some(thread_id) = effective_thread_id {
                    payload["message_thread_id"] = serde_json::json!(thread_id);
                }

                let payload_bytes = match serde_json::to_vec(&payload) {
                    Ok(b) => b,
                    Err(_) => return,
                };

                let headers = serde_json::json!({
                    "Content-Type": "application/json"
                });

                let result = channel_host::http_request(
                    "POST",
                    "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendChatAction",
                    &headers.to_string(),
                    Some(&payload_bytes),
                    None,
                );

                if let Err(e) = result {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!("sendChatAction failed: {}", e),
                    );
                }
            }
            TelegramStatusAction::Notify(prompt) => {
                // Send user-visible status updates for actionable events.
                if let Err(first_err) = send_message(
                    metadata.chat_id,
                    &prompt,
                    Some(metadata.message_id),
                    None,
                    metadata.message_thread_id,
                ) {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!(
                            "Failed to send status reply ({}), retrying without reply context",
                            first_err
                        ),
                    );

                    if let Err(retry_err) = send_message(
                        metadata.chat_id,
                        &prompt,
                        None,
                        None,
                        metadata.message_thread_id,
                    ) {
                        channel_host::log(
                            channel_host::LogLevel::Debug,
                            &format!(
                                "Failed to send status message without reply context: {}",
                                retry_err
                            ),
                        );
                    }
                }
            }
            TelegramStatusAction::Subagent(event) => {
                handle_subagent_status(&metadata, event);
            }
        }
    }

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "Telegram channel shutting down",
        );
    }
}

// ============================================================================
// Send Message Helper
// ============================================================================

/// Errors from send_message, split so callers can match on parse-entity failures.
enum SendError {
    /// Telegram returned 400 with "can't parse entities" (Markdown issue).
    ParseEntities(String),
    /// Telegram returned 400 with "message thread not found" — the forum topic
    /// was deleted. Callers should invalidate the cached thread_id and retry.
    ThreadNotFound(String),
    /// Any other failure.
    Other(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::ParseEntities(detail) => write!(f, "parse entities error: {}", detail),
            SendError::ThreadNotFound(detail) => write!(f, "thread not found: {}", detail),
            SendError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

/// Maximum file size we'll attempt to download (20 MB — matches Telegram Bot API limit).
const MAX_DOWNLOAD_SIZE: i64 = 20 * 1024 * 1024;

/// A descriptor for a media file to download.
struct MediaDescriptor {
    file_id: String,
    mime_type: String,
    filename: Option<String>,
}

mod dispatch;
mod media;
mod sessions;

pub(crate) use dispatch::*;
pub(crate) use media::*;
pub(crate) use sessions::*;

#[cfg(test)]
mod tests;

export!(TelegramChannel);
