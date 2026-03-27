use std::path::PathBuf;

use secrecy::SecretString;

use crate::channels::StreamMode;
use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Channel configurations.
#[derive(Debug, Clone)]
pub struct ChannelsConfig {
    pub cli: CliConfig,
    pub http: Option<HttpConfig>,
    pub gateway: Option<GatewayConfig>,
    pub signal: Option<SignalConfig>,
    pub nostr: Option<NostrConfig>,
    pub telegram: Option<TelegramConfig>,
    pub slack: Option<SlackChannelConfig>,
    pub discord: Option<DiscordChannelConfig>,
    pub gmail: Option<GmailChannelConfig>,
    #[cfg(target_os = "macos")]
    pub imessage: Option<IMessageChannelConfig>,
    #[cfg(target_os = "macos")]
    pub apple_mail: Option<AppleMailChannelConfig>,
    /// Directory containing WASM channel modules (default: ~/.thinclaw/channels/).
    pub wasm_channels_dir: std::path::PathBuf,
    /// Whether WASM channels are enabled.
    pub wasm_channels_enabled: bool,
    /// Telegram owner user ID. When set, the bot only responds to this user.
    pub telegram_owner_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct CliConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
    pub webhook_secret: Option<SecretString>,
    pub user_id: String,
}

/// Web gateway configuration.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub host: String,
    pub port: u16,
    /// Bearer token for authentication. Random hex generated at startup if unset.
    pub auth_token: Option<String>,
    pub user_id: String,
}

/// Signal channel configuration (signal-cli daemon HTTP/JSON-RPC).
#[derive(Debug, Clone)]
pub struct SignalConfig {
    /// Base URL of the signal-cli daemon HTTP endpoint (e.g. `http://127.0.0.1:8080`).
    pub http_url: String,
    /// Signal account identifier (E.164 phone number, e.g. `+1234567890`).
    pub account: String,
    /// Users allowed to interact with the bot in DMs.
    ///
    /// Each entry is one of:
    /// - `*` — allow everyone
    /// - E.164 phone number (e.g. `+1234567890`)
    /// - bare UUID (e.g. `a1b2c3d4-e5f6-7890-abcd-ef1234567890`)
    /// - `uuid:<id>` prefix form (e.g. `uuid:a1b2c3d4-e5f6-7890-abcd-ef1234567890`)
    ///
    /// An empty list denies all senders (secure by default).
    pub allow_from: Vec<String>,
    /// Groups allowed to interact with the bot.
    ///
    /// - Empty list — deny all group messages (DMs only, secure by default).
    /// - `*` — allow all groups.
    /// - Specific group IDs — allow only those groups.
    pub allow_from_groups: Vec<String>,
    /// DM policy: "open", "allowlist", or "pairing". Default: "pairing".
    ///
    /// - "open" — allow all DM senders (ignores allow_from for DMs)
    /// - "allowlist" — only allow senders in allow_from list
    /// - "pairing" — allowlist + send pairing reply to unknown users
    pub dm_policy: String,
    /// Group policy: "allowlist", "open", or "disabled". Default: "allowlist".
    ///
    /// - "disabled" — deny all group messages
    /// - "allowlist" — check allow_from_groups and group_allow_from
    /// - "open" — accept all group messages (respects allow_from_groups for group ID)
    pub group_policy: String,
    /// Allow list for group message senders. If empty, inherits from allow_from.
    pub group_allow_from: Vec<String>,
    /// Skip messages that contain only attachments (no text).
    pub ignore_attachments: bool,
    /// Skip story messages.
    pub ignore_stories: bool,
}

/// Nostr channel configuration (NIP-04 encrypted DMs via nostr-sdk).
#[derive(Debug, Clone)]
pub struct NostrConfig {
    /// Nostr private key in hex or bech32 (nsec) format.
    pub private_key: secrecy::SecretString,
    /// Relay URLs to connect to.
    pub relays: Vec<String>,
    /// Public keys (hex or npub) allowed to interact with the bot.
    /// Empty list denies all senders. `*` allows everyone.
    pub allow_from: Vec<String>,
}

impl ChannelsConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let http = if optional_env("HTTP_PORT")?.is_some() || optional_env("HTTP_HOST")?.is_some() {
            Some(HttpConfig {
                host: optional_env("HTTP_HOST")?.unwrap_or_else(|| "0.0.0.0".to_string()),
                port: parse_optional_env("HTTP_PORT", 8080)?,
                webhook_secret: optional_env("HTTP_WEBHOOK_SECRET")?.map(SecretString::from),
                user_id: optional_env("HTTP_USER_ID")?.unwrap_or_else(|| "http".to_string()),
            })
        } else {
            None
        };

        let gateway_enabled = parse_bool_env("GATEWAY_ENABLED", true)?;
        let gateway = if gateway_enabled {
            Some(GatewayConfig {
                host: optional_env("GATEWAY_HOST")?.unwrap_or_else(|| "127.0.0.1".to_string()),
                port: parse_optional_env("GATEWAY_PORT", 3000)?,
                auth_token: optional_env("GATEWAY_AUTH_TOKEN")?,
                user_id: optional_env("GATEWAY_USER_ID")?.unwrap_or_else(|| "default".to_string()),
            })
        } else {
            None
        };

        let signal = if let Some(http_url) = optional_env("SIGNAL_HTTP_URL")? {
            let account = optional_env("SIGNAL_ACCOUNT")?.ok_or(ConfigError::InvalidValue {
                key: "SIGNAL_ACCOUNT".to_string(),
                message: "SIGNAL_ACCOUNT is required when SIGNAL_HTTP_URL is set".to_string(),
            })?;
            let allow_from = match std::env::var_os("SIGNAL_ALLOW_FROM") {
                None => vec![account.clone()],
                Some(val) => {
                    let s = val.to_string_lossy();
                    s.split(',')
                        .map(|e| e.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                }
            };
            let dm_policy =
                optional_env("SIGNAL_DM_POLICY")?.unwrap_or_else(|| "pairing".to_string());
            let group_policy =
                optional_env("SIGNAL_GROUP_POLICY")?.unwrap_or_else(|| "allowlist".to_string());
            Some(SignalConfig {
                http_url,
                account,
                allow_from,
                allow_from_groups: optional_env("SIGNAL_ALLOW_FROM_GROUPS")?
                    .map(|s| {
                        s.split(',')
                            .map(|e| e.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
                dm_policy,
                group_policy,
                group_allow_from: optional_env("SIGNAL_GROUP_ALLOW_FROM")?
                    .map(|s| {
                        s.split(',')
                            .map(|e| e.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
                ignore_attachments: optional_env("SIGNAL_IGNORE_ATTACHMENTS")?
                    .map(|s| s.to_lowercase() == "true" || s == "1")
                    .unwrap_or(false),
                ignore_stories: optional_env("SIGNAL_IGNORE_STORIES")?
                    .map(|s| s.to_lowercase() == "true" || s == "1")
                    .unwrap_or(true),
            })
        } else {
            None
        };

        let cli_enabled = optional_env("CLI_ENABLED")?
            .map(|s| s.to_lowercase() != "false" && s != "0")
            .unwrap_or(true);

        Ok(Self {
            cli: CliConfig {
                enabled: cli_enabled,
            },
            http,
            gateway,
            signal,
            nostr: Self::resolve_nostr()?,
            wasm_channels_dir: optional_env("WASM_CHANNELS_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_channels_dir),
            wasm_channels_enabled: parse_bool_env("WASM_CHANNELS_ENABLED", true)?,
            telegram_owner_id: optional_env("TELEGRAM_OWNER_ID")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e: std::num::ParseIntError| ConfigError::InvalidValue {
                    key: "TELEGRAM_OWNER_ID".to_string(),
                    message: format!("must be an integer: {e}"),
                })?
                .or(settings.channels.telegram_owner_id),
            telegram: Self::resolve_telegram(settings)?,
            slack: Self::resolve_slack()?,
            discord: Self::resolve_discord()?,
            gmail: Self::resolve_gmail()?,
            #[cfg(target_os = "macos")]
            imessage: Self::resolve_imessage()?,
            #[cfg(target_os = "macos")]
            apple_mail: Self::resolve_apple_mail(settings)?,
        })
    }

    fn resolve_nostr() -> Result<Option<NostrConfig>, ConfigError> {
        let private_key = match optional_env("NOSTR_PRIVATE_KEY")? {
            Some(k) => secrecy::SecretString::from(k),
            None => return Ok(None),
        };

        let relays = optional_env("NOSTR_RELAYS")?
            .map(|s| {
                s.split(',')
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty())
                    .collect()
            })
            .unwrap_or_else(|| {
                vec![
                    "wss://relay.damus.io".to_string(),
                    "wss://nos.lol".to_string(),
                    "wss://relay.nostr.band".to_string(),
                ]
            });

        let allow_from = optional_env("NOSTR_ALLOW_FROM")?
            .map(|s| {
                s.split(',')
                    .map(|e| e.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        Ok(Some(NostrConfig {
            private_key,
            relays,
            allow_from,
        }))
    }

    fn resolve_telegram(settings: &Settings) -> Result<Option<TelegramConfig>, ConfigError> {
        let bot_token = match optional_env("TELEGRAM_BOT_TOKEN")? {
            Some(t) => SecretString::from(t),
            None => return Ok(None),
        };

        let owner_id = optional_env("TELEGRAM_OWNER_ID")?
            .map(|s| s.parse())
            .transpose()
            .map_err(|e: std::num::ParseIntError| ConfigError::InvalidValue {
                key: "TELEGRAM_OWNER_ID".to_string(),
                message: format!("must be an integer: {e}"),
            })?
            .or(settings.channels.telegram_owner_id);

        let allow_from = optional_env("TELEGRAM_ALLOW_FROM")?
            .map(|s| {
                s.split(',')
                    .map(|e| e.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let stream_mode = optional_env("TELEGRAM_STREAM_MODE")?
            .map(|s| StreamMode::from_str_value(&s))
            .unwrap_or_default();

        Ok(Some(TelegramConfig {
            bot_token,
            owner_id,
            allow_from,
            stream_mode,
        }))
    }
}

/// Get the default channels directory (~/.thinclaw/channels/).
fn default_channels_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".thinclaw")
        .join("channels")
}

/// Telegram bot configuration.
#[derive(Debug, Clone)]
pub struct TelegramConfig {
    /// Bot token from BotFather.
    pub bot_token: SecretString,
    /// Owner user ID (only respond to this user if set).
    pub owner_id: Option<i64>,
    /// Allowed user IDs (empty = allow all; "*" = allow all).
    pub allow_from: Vec<String>,
    /// Stream mode for progressive message rendering.
    pub stream_mode: StreamMode,
}

/// Slack channel configuration.
#[derive(Debug, Clone)]
pub struct SlackChannelConfig {
    /// Bot User OAuth Token (xoxb-...).
    pub bot_token: SecretString,
    /// App-Level Token (xapp-...) for Socket Mode.
    pub app_token: SecretString,
    /// Allowed channel/DM IDs (empty = allow all).
    pub allow_from: Vec<String>,
}

/// Discord channel configuration.
#[derive(Debug, Clone)]
pub struct DiscordChannelConfig {
    /// Bot token.
    pub bot_token: SecretString,
    /// Optional guild ID to restrict to.
    pub guild_id: Option<String>,
    /// Allowed channel IDs (empty = allow all).
    pub allow_from: Vec<String>,
    /// Stream mode for progressive message rendering.
    pub stream_mode: StreamMode,
}

/// iMessage channel configuration (macOS only).
#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub struct IMessageChannelConfig {
    /// Allowed phone numbers / email addresses (empty = allow all).
    pub allow_from: Vec<String>,
    /// Polling interval in seconds.
    pub poll_interval_secs: u64,
}

/// Apple Mail channel configuration (macOS only).
#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub struct AppleMailChannelConfig {
    /// Allowed sender email addresses (empty = allow all).
    pub allow_from: Vec<String>,
    /// Polling interval in seconds.
    pub poll_interval_secs: u64,
    /// Only process unread messages.
    pub unread_only: bool,
    /// Mark messages as read after processing.
    pub mark_as_read: bool,
}

/// Gmail channel configuration.
#[derive(Debug, Clone)]
pub struct GmailChannelConfig {
    /// GCP project ID.
    pub project_id: String,
    /// Pub/Sub subscription ID.
    pub subscription_id: String,
    /// Pub/Sub topic ID.
    pub topic_id: String,
    /// OAuth2 access token (from `thinclaw auth gmail`).
    pub oauth_token: Option<String>,
    /// Email addresses allowed to interact (empty = all).
    pub allowed_senders: Vec<String>,
    /// Gmail label filters (default: INBOX, UNREAD).
    pub label_filters: Vec<String>,
    /// Maximum message body size in bytes.
    pub max_message_size_bytes: usize,
}

impl ChannelsConfig {
    fn resolve_slack() -> Result<Option<SlackChannelConfig>, ConfigError> {
        let bot_token = match optional_env("SLACK_BOT_TOKEN")? {
            Some(t) => SecretString::from(t),
            None => return Ok(None),
        };

        let app_token = match optional_env("SLACK_APP_TOKEN")? {
            Some(t) => SecretString::from(t),
            None => {
                return Err(ConfigError::InvalidValue {
                    key: "SLACK_APP_TOKEN".to_string(),
                    message: "SLACK_APP_TOKEN is required when SLACK_BOT_TOKEN is set".to_string(),
                });
            }
        };

        let allow_from = optional_env("SLACK_ALLOW_FROM")?
            .map(|s| {
                s.split(',')
                    .map(|e| e.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        Ok(Some(SlackChannelConfig {
            bot_token,
            app_token,
            allow_from,
        }))
    }

    fn resolve_discord() -> Result<Option<DiscordChannelConfig>, ConfigError> {
        let bot_token = match optional_env("DISCORD_BOT_TOKEN")? {
            Some(t) => SecretString::from(t),
            None => return Ok(None),
        };

        let guild_id = optional_env("DISCORD_GUILD_ID")?;

        let allow_from = optional_env("DISCORD_ALLOW_FROM")?
            .map(|s| {
                s.split(',')
                    .map(|e| e.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let stream_mode = optional_env("DISCORD_STREAM_MODE")?
            .map(|s| StreamMode::from_str_value(&s))
            .unwrap_or_default();

        Ok(Some(DiscordChannelConfig {
            bot_token,
            guild_id,
            allow_from,
            stream_mode,
        }))
    }

    #[cfg(target_os = "macos")]
    fn resolve_imessage() -> Result<Option<IMessageChannelConfig>, ConfigError> {
        let enabled = parse_bool_env("IMESSAGE_ENABLED", false)?;
        if !enabled {
            return Ok(None);
        }

        let allow_from = optional_env("IMESSAGE_ALLOW_FROM")?
            .map(|s| {
                s.split(',')
                    .map(|e| e.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let poll_interval_secs: u64 = optional_env("IMESSAGE_POLL_INTERVAL")?
            .map(|s| s.parse())
            .transpose()
            .map_err(|e: std::num::ParseIntError| ConfigError::InvalidValue {
                key: "IMESSAGE_POLL_INTERVAL".to_string(),
                message: format!("must be an integer: {e}"),
            })?
            .unwrap_or(3);

        Ok(Some(IMessageChannelConfig {
            allow_from,
            poll_interval_secs,
        }))
    }

    #[cfg(target_os = "macos")]
    fn resolve_apple_mail(settings: &Settings) -> Result<Option<AppleMailChannelConfig>, ConfigError> {
        // DB setting takes priority (from WebUI), env var as fallback
        let enabled = if settings.channels.apple_mail_enabled {
            true
        } else {
            parse_bool_env("APPLE_MAIL_ENABLED", false)?
        };
        if !enabled {
            return Ok(None);
        }

        // allow_from: DB setting > env var > empty (all)
        let allow_from = settings
            .channels
            .apple_mail_allow_from
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| optional_env("APPLE_MAIL_ALLOW_FROM").ok().flatten().as_deref().map(|_| ""))
            .map(|_| {
                // Re-read from whichever source had the value
                let raw = settings
                    .channels
                    .apple_mail_allow_from
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| optional_env("APPLE_MAIL_ALLOW_FROM").ok().flatten())
                    .unwrap_or_default();
                raw.split(',')
                    .map(|e| e.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let poll_interval_secs: u64 = settings
            .channels
            .apple_mail_poll_interval
            .or_else(|| {
                optional_env("APPLE_MAIL_POLL_INTERVAL")
                    .ok()
                    .flatten()
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(10);

        let unread_only = if settings.channels.apple_mail_unread_only {
            true
        } else {
            parse_bool_env("APPLE_MAIL_UNREAD_ONLY", true)?
        };
        let mark_as_read = if settings.channels.apple_mail_mark_as_read {
            true
        } else {
            parse_bool_env("APPLE_MAIL_MARK_AS_READ", true)?
        };

        Ok(Some(AppleMailChannelConfig {
            allow_from,
            poll_interval_secs,
            unread_only,
            mark_as_read,
        }))
    }

    fn resolve_gmail() -> Result<Option<GmailChannelConfig>, ConfigError> {
        let enabled = parse_bool_env("GMAIL_ENABLED", false)?;
        if !enabled {
            return Ok(None);
        }

        let project_id = optional_env("GMAIL_PROJECT_ID")?.ok_or(ConfigError::InvalidValue {
            key: "GMAIL_PROJECT_ID".to_string(),
            message: "GMAIL_PROJECT_ID is required when GMAIL_ENABLED=true".to_string(),
        })?;

        let subscription_id =
            optional_env("GMAIL_SUBSCRIPTION_ID")?.ok_or(ConfigError::InvalidValue {
                key: "GMAIL_SUBSCRIPTION_ID".to_string(),
                message: "GMAIL_SUBSCRIPTION_ID is required when GMAIL_ENABLED=true".to_string(),
            })?;

        let topic_id = optional_env("GMAIL_TOPIC_ID")?.ok_or(ConfigError::InvalidValue {
            key: "GMAIL_TOPIC_ID".to_string(),
            message: "GMAIL_TOPIC_ID is required when GMAIL_ENABLED=true".to_string(),
        })?;

        let oauth_token = optional_env("GMAIL_OAUTH_TOKEN")?;

        let allowed_senders = optional_env("GMAIL_ALLOWED_SENDERS")?
            .map(|s| {
                s.split(',')
                    .map(|e| e.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let label_filters = optional_env("GMAIL_LABEL_FILTERS")?
            .map(|s| {
                s.split(',')
                    .map(|e| e.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_else(|| vec!["INBOX".into(), "UNREAD".into()]);

        let max_message_size_bytes: usize = optional_env("GMAIL_MAX_MESSAGE_SIZE")?
            .map(|s| s.parse())
            .transpose()
            .map_err(|e: std::num::ParseIntError| ConfigError::InvalidValue {
                key: "GMAIL_MAX_MESSAGE_SIZE".to_string(),
                message: format!("must be an integer: {e}"),
            })?
            .unwrap_or(10 * 1024 * 1024);

        Ok(Some(GmailChannelConfig {
            project_id,
            subscription_id,
            topic_id,
            oauth_token,
            allowed_senders,
            label_filters,
            max_message_size_bytes,
        }))
    }
}
