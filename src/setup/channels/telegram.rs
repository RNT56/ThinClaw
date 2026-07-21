//! Telegram bot channel setup: token validation, owner binding (interactive
//! polling + TUI), runtime polling-offset persistence, and webhook secret.

use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::Path,
    time::{Duration, Instant},
};

use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use thinclaw_channels::setup as channel_setup;
use thinclaw_tools_core::{OutboundUrlGuardOptions, validate_outbound_url_pinned_async};

use crate::pairing::PairingStore;
use crate::settings::{Settings, TunnelSettings};
use crate::setup::prompts::{
    PromptUiMode, confirm, current_prompt_ui_mode, optional_input, print_blank_line, print_error,
    print_info, print_success, print_warning, secret_input, select_one,
};

use super::{ChannelSetupError, SecretsContext};

const MAX_TELEGRAM_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_TELEGRAM_TOKEN_BYTES: usize = 256;
const TELEGRAM_API_ORIGIN: &str = "https://api.telegram.org/";

fn telegram_bot_token(token: &SecretString) -> Result<&str, ChannelSetupError> {
    let token = token.expose_secret().trim();
    let mut segments = token.split(':');
    let bot_id = segments.next().unwrap_or_default();
    let secret = segments.next().unwrap_or_default();
    if token.is_empty()
        || token.len() > MAX_TELEGRAM_TOKEN_BYTES
        || segments.next().is_some()
        || bot_id.is_empty()
        || !bot_id.bytes().all(|byte| byte.is_ascii_digit())
        || secret.is_empty()
        || !secret
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(ChannelSetupError::Validation(
            "Telegram bot token is malformed".to_string(),
        ));
    }
    Ok(token)
}

fn telegram_api_url(token: &SecretString, method: &str) -> Result<reqwest::Url, ChannelSetupError> {
    let token = telegram_bot_token(token)?;
    let mut url = reqwest::Url::parse(TELEGRAM_API_ORIGIN)
        .map_err(|_| ChannelSetupError::Network("Telegram API endpoint is invalid".to_string()))?;
    url.set_path(&format!("bot{token}/{method}"));
    Ok(url)
}

async fn telegram_client(timeout: Duration) -> Result<Client, ChannelSetupError> {
    let guarded = validate_outbound_url_pinned_async(
        TELEGRAM_API_ORIGIN,
        &OutboundUrlGuardOptions {
            require_https: true,
            upgrade_http_to_https: false,
            allowlist: vec!["api.telegram.org".to_string()],
        },
    )
    .await
    .map_err(|error| ChannelSetupError::Network(error.to_string()))?;
    let host = guarded
        .url
        .host_str()
        .ok_or_else(|| ChannelSetupError::Network("Telegram API has no host".to_string()))?;
    let mut builder = Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(10)))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if !guarded.pinned_addrs.is_empty() {
        builder = builder.resolve_to_addrs(host, &guarded.pinned_addrs);
    }
    builder
        .build()
        .map_err(|error| ChannelSetupError::Network(format!("HTTP client: {error}")))
}

/// Result of Telegram setup.
#[derive(Debug, Clone)]
pub struct TelegramSetupResult {
    pub enabled: bool,
    pub bot_username: Option<String>,
    pub webhook_secret: Option<String>,
    pub owner_id: Option<i64>,
}

/// Telegram Bot API response for getMe.
#[derive(Debug, Deserialize)]
struct TelegramGetMeResponse {
    ok: bool,
    result: Option<TelegramUser>,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    username: Option<String>,
    #[allow(dead_code)]
    first_name: String,
}

/// Telegram Bot API response for getUpdates.
#[derive(Debug, Deserialize)]
struct TelegramGetUpdatesResponse {
    ok: bool,
    result: Vec<TelegramUpdate>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramUpdateMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdateMessage {
    from: Option<TelegramUpdateUser>,
    chat: Option<TelegramUpdateChat>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdateUser {
    id: i64,
    first_name: String,
    username: Option<String>,
    #[serde(default)]
    is_bot: bool,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdateChat {
    #[serde(rename = "type")]
    chat_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TelegramBindingOutcome {
    Bound(channel_setup::TelegramOwnerCandidate),
    TimedOut,
    ManualEntryRequested,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TelegramBindingRecovery {
    RetryAutomatic,
    ManualEntry,
    Skip,
}

/// Set up Telegram bot channel.
///
/// Guides the user through:
/// 1. Creating a bot with @BotFather
/// 2. Entering the bot token
/// 3. Validating the token
/// 4. Saving the token to the database
pub async fn setup_telegram(
    secrets: &SecretsContext,
    settings: &Settings,
) -> Result<TelegramSetupResult, ChannelSetupError> {
    print_info("Telegram setup");
    print_blank_line();
    print_info("To create a Telegram bot:");
    print_info("1. Open Telegram and message @BotFather");
    print_info("2. Send /newbot and follow the prompts");
    print_info("3. Copy the bot token (looks like 123456:ABC-DEF...)");
    print_blank_line();

    // Check if token already exists
    if secrets.secret_exists("telegram_bot_token").await {
        print_info("Existing Telegram token found in database.");
        if !confirm("Replace existing token?", false)? {
            // Still offer to configure webhook secret and owner binding
            let webhook_secret = setup_telegram_webhook_secret(secrets, &settings.tunnel).await?;
            let owner_id = bind_telegram_owner_flow(secrets, settings).await?;
            return Ok(TelegramSetupResult {
                enabled: true,
                bot_username: None,
                webhook_secret,
                owner_id,
            });
        }
    }

    loop {
        let token = secret_input("Bot token (from @BotFather)")?;

        // Validate the token
        print_info("Validating bot token...");

        match validate_telegram_token(&token).await {
            Ok(username) => {
                print_success(&format!(
                    "Bot validated: @{}",
                    username.as_deref().unwrap_or("unknown")
                ));

                // Save to database
                secrets.save_secret("telegram_bot_token", &token).await?;
                print_success("Token saved to database");

                // Bind bot to owner's Telegram account
                let owner_id = bind_telegram_owner(&token, username.as_deref()).await?;

                // Offer webhook secret configuration
                let webhook_secret =
                    setup_telegram_webhook_secret(secrets, &settings.tunnel).await?;

                return Ok(TelegramSetupResult {
                    enabled: true,
                    bot_username: username,
                    webhook_secret,
                    owner_id,
                });
            }
            Err(e) => {
                print_error(&format!("Token validation failed: {}", e));

                if !confirm("Try again?", true)? {
                    return Ok(TelegramSetupResult {
                        enabled: false,
                        bot_username: None,
                        webhook_secret: None,
                        owner_id: None,
                    });
                }
            }
        }
    }
}

/// Bind the bot to the owner's Telegram account by having them send a message.
///
/// Polls `getUpdates` until a message arrives, then captures the sender's user ID.
/// Returns `None` if the user declines or the flow times out.
async fn bind_telegram_owner(
    token: &SecretString,
    bot_username: Option<&str>,
) -> Result<Option<i64>, ChannelSetupError> {
    crate::setup::prompts::print_blank_line();
    print_info("Account Binding (recommended):");
    print_info("Binding restricts the bot so only YOU can use it.");
    print_info("Without this, anyone who finds your bot can send it messages.");
    crate::setup::prompts::print_blank_line();

    if !confirm("Bind bot to your Telegram account?", true)? {
        print_info("Skipping account binding. Bot will accept messages from all users.");
        return Ok(None);
    }

    print_info("Send a private message (for example /start) to your bot in Telegram.");
    if let Some(username) = bot_username {
        print_info(&format!("Bot to message: @{}", username));
    }
    print_info("ThinClaw listens for the first NEW private message after this step starts.");

    loop {
        let automatic_result = match current_prompt_ui_mode() {
            PromptUiMode::Tui => match wait_for_telegram_owner_tui(token, bot_username).await {
                Ok(outcome) => outcome,
                Err(ChannelSetupError::Io(error)) => return Err(ChannelSetupError::Io(error)),
                Err(error) => {
                    print_error(&format!(
                        "Automatic Telegram binding could not complete: {}",
                        error
                    ));
                    match prompt_telegram_binding_recovery(
                        "Automatic Telegram binding ran into a network or API error.",
                    )? {
                        TelegramBindingRecovery::RetryAutomatic => continue,
                        TelegramBindingRecovery::ManualEntry => {
                            TelegramBindingOutcome::ManualEntryRequested
                        }
                        TelegramBindingRecovery::Skip => TelegramBindingOutcome::Skipped,
                    }
                }
            },
            PromptUiMode::Cli => {
                print_info("Waiting for your private message (up to 120 seconds)...");
                match capture_telegram_owner_candidate(token).await {
                    Ok(Some(candidate)) => TelegramBindingOutcome::Bound(candidate),
                    Ok(None) => TelegramBindingOutcome::TimedOut,
                    Err(error) => {
                        print_error(&format!(
                            "Automatic Telegram binding could not complete: {}",
                            error
                        ));
                        match prompt_telegram_binding_recovery(
                            "Automatic Telegram binding ran into a network or API error.",
                        )? {
                            TelegramBindingRecovery::RetryAutomatic => continue,
                            TelegramBindingRecovery::ManualEntry => {
                                TelegramBindingOutcome::ManualEntryRequested
                            }
                            TelegramBindingRecovery::Skip => TelegramBindingOutcome::Skipped,
                        }
                    }
                }
            }
        };

        match automatic_result {
            TelegramBindingOutcome::Bound(candidate) => {
                print_success(&format!(
                    "Captured Telegram account: {}",
                    candidate.summary()
                ));
                print_info(
                    "ThinClaw will store this numeric Telegram ID as the bot owner and seed the pairing allowlist.",
                );
                if confirm("Use this Telegram account as the bot owner?", true)? {
                    if let Err(error) = persist_telegram_runtime_polling_snapshot(token).await {
                        tracing::warn!(
                            %error,
                            "Failed to persist Telegram backlog snapshot after automatic owner capture"
                        );
                    }
                    seed_telegram_owner_allowlist(candidate.user_id);
                    return Ok(Some(candidate.user_id));
                }

                match prompt_telegram_binding_recovery(
                    "The captured Telegram account was not accepted.",
                )? {
                    TelegramBindingRecovery::RetryAutomatic => continue,
                    TelegramBindingRecovery::ManualEntry => {
                        if let Err(error) = persist_telegram_runtime_polling_snapshot(token).await {
                            tracing::warn!(
                                %error,
                                "Failed to persist Telegram backlog snapshot before manual owner ID entry"
                            );
                        }
                        if let Some(owner_id) =
                            prompt_manual_telegram_owner_id(Some(candidate.user_id))?
                        {
                            seed_telegram_owner_allowlist(owner_id);
                            return Ok(Some(owner_id));
                        }
                    }
                    TelegramBindingRecovery::Skip => {
                        if let Err(error) = persist_telegram_runtime_polling_snapshot(token).await {
                            tracing::warn!(
                                %error,
                                "Failed to persist Telegram backlog snapshot before skipping owner binding"
                            );
                        }
                    }
                }
            }
            TelegramBindingOutcome::TimedOut => {
                match prompt_telegram_binding_recovery(
                    "No new private Telegram message reached the bot before the timeout.",
                )? {
                    TelegramBindingRecovery::RetryAutomatic => continue,
                    TelegramBindingRecovery::ManualEntry => {
                        if let Err(error) = persist_telegram_runtime_polling_snapshot(token).await {
                            tracing::warn!(
                                %error,
                                "Failed to persist Telegram backlog snapshot before manual owner ID entry"
                            );
                        }
                        if let Some(owner_id) = prompt_manual_telegram_owner_id(None)? {
                            seed_telegram_owner_allowlist(owner_id);
                            return Ok(Some(owner_id));
                        }
                    }
                    TelegramBindingRecovery::Skip => {
                        if let Err(error) = persist_telegram_runtime_polling_snapshot(token).await {
                            tracing::warn!(
                                %error,
                                "Failed to persist Telegram backlog snapshot before skipping owner binding"
                            );
                        }
                    }
                }
            }
            TelegramBindingOutcome::ManualEntryRequested => {
                if let Err(error) = persist_telegram_runtime_polling_snapshot(token).await {
                    tracing::warn!(
                        %error,
                        "Failed to persist Telegram backlog snapshot after manual owner entry request"
                    );
                }
                if let Some(owner_id) = prompt_manual_telegram_owner_id(None)? {
                    seed_telegram_owner_allowlist(owner_id);
                    return Ok(Some(owner_id));
                }
            }
            TelegramBindingOutcome::Skipped => {
                if let Err(error) = persist_telegram_runtime_polling_snapshot(token).await {
                    tracing::warn!(
                        %error,
                        "Failed to persist Telegram backlog snapshot after skipping owner binding"
                    );
                }
            }
        }

        print_info("Skipping owner binding. Bot will accept messages from all users.");
        return Ok(None);
    }
}

/// Bind flow when the token already exists (reads from secrets store).
///
/// Retrieves the saved bot token and delegates to `bind_telegram_owner`.
async fn bind_telegram_owner_flow(
    secrets: &SecretsContext,
    settings: &Settings,
) -> Result<Option<i64>, ChannelSetupError> {
    if settings.channels.telegram_owner_id.is_some() {
        print_info("Bot is already bound to a Telegram account.");
        if !confirm("Re-bind to a different account?", false)? {
            return Ok(settings.channels.telegram_owner_id);
        }
    }

    // We need the token to poll getUpdates
    let token = secrets.get_secret("telegram_bot_token").await?;

    bind_telegram_owner(&token, None).await
}

fn seed_telegram_owner_allowlist(owner_id: i64) {
    let pairing_store = PairingStore::new();
    if let Err(error) = pairing_store.ensure_allow_from("telegram", &owner_id.to_string()) {
        tracing::warn!(
            owner_id,
            %error,
            "Failed to seed Telegram owner into pairing allowlist"
        );
    }
}

fn prompt_telegram_binding_recovery(
    reason: &str,
) -> Result<TelegramBindingRecovery, ChannelSetupError> {
    print_warning(reason);
    print_info(
        "ThinClaw can retry the automatic capture, accept a manual numeric Telegram user ID, or skip owner binding for now.",
    );

    let options = [
        "Retry automatic capture",
        "Enter Telegram user ID manually",
        "Skip owner binding for now",
    ];
    let choice = select_one("How should ThinClaw continue?", &options)?;
    Ok(match choice {
        0 => TelegramBindingRecovery::RetryAutomatic,
        1 => TelegramBindingRecovery::ManualEntry,
        _ => TelegramBindingRecovery::Skip,
    })
}

fn prompt_manual_telegram_owner_id(
    suggested_id: Option<i64>,
) -> Result<Option<i64>, ChannelSetupError> {
    print_info("Enter the numeric Telegram user ID that should own this bot.");
    if suggested_id.is_some() {
        print_info("Press Enter to use the captured ID, or type a different numeric ID.");
    } else {
        print_info("Leave it blank to skip owner binding for now.");
    }

    let hint = suggested_id.map(|id| id.to_string());
    loop {
        let entered = optional_input("Telegram user ID (numeric)", hint.as_deref())?;
        let value = match (entered, suggested_id) {
            (Some(raw), _) => raw,
            (None, Some(id)) => return Ok(Some(id)),
            (None, None) => return Ok(None),
        };

        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        match trimmed.parse::<i64>() {
            Ok(owner_id) if owner_id > 0 => return Ok(Some(owner_id)),
            _ => {
                print_error(
                    "Telegram owner IDs must be positive whole numbers. Example: 684480568",
                );
                if !confirm("Try entering the Telegram ID again?", true)? {
                    return Ok(None);
                }
            }
        }
    }
}

fn extract_telegram_owner_capture(
    updates: &[TelegramUpdate],
) -> Option<channel_setup::TelegramOwnerCapture> {
    let setup_updates = telegram_setup_updates(updates);
    channel_setup::extract_telegram_owner_capture(&setup_updates)
}

fn next_telegram_update_offset(updates: &[TelegramUpdate]) -> Option<i64> {
    let setup_updates = telegram_setup_updates(updates);
    channel_setup::next_telegram_update_offset(&setup_updates)
}

fn telegram_setup_updates(updates: &[TelegramUpdate]) -> Vec<channel_setup::TelegramSetupUpdate> {
    updates
        .iter()
        .map(|update| channel_setup::TelegramSetupUpdate {
            update_id: update.update_id,
            message: update
                .message
                .as_ref()
                .map(|message| channel_setup::TelegramSetupMessage {
                    from: message
                        .from
                        .as_ref()
                        .map(|from| channel_setup::TelegramSetupUser {
                            id: from.id,
                            first_name: from.first_name.clone(),
                            username: from.username.clone(),
                            is_bot: from.is_bot,
                        }),
                    chat: message
                        .chat
                        .as_ref()
                        .map(|chat| channel_setup::TelegramSetupChat {
                            chat_type: chat.chat_type.clone(),
                        }),
                }),
        })
        .collect()
}

async fn fetch_telegram_updates(
    client: &Client,
    token: &SecretString,
    timeout_secs: u64,
    offset: Option<i64>,
) -> Result<Vec<TelegramUpdate>, ChannelSetupError> {
    let updates_url = telegram_api_url(token, "getUpdates")?;
    let mut query = vec![
        ("timeout".to_string(), timeout_secs.to_string()),
        ("allowed_updates".to_string(), "[\"message\"]".to_string()),
    ];
    if let Some(offset) = offset {
        query.push(("offset".to_string(), offset.to_string()));
    }

    let response = client
        .get(updates_url)
        .query(&query)
        .send()
        .await
        .map_err(|e| {
            ChannelSetupError::Network(format!("getUpdates request failed: {}", e.without_url()))
        })?;

    if !response.status().is_success() {
        return Err(ChannelSetupError::Network(format!(
            "getUpdates returned status {}",
            response.status()
        )));
    }

    let body: TelegramGetUpdatesResponse =
        crate::http_response::bounded_json(response, MAX_TELEGRAM_RESPONSE_BYTES)
            .await
            .map_err(|e| {
                ChannelSetupError::Network(format!("Failed to parse getUpdates response: {}", e))
            })?;

    if !body.ok {
        return Err(ChannelSetupError::Network(
            "Telegram API returned error for getUpdates".to_string(),
        ));
    }

    Ok(body.result)
}

async fn capture_telegram_owner_candidate(
    token: &SecretString,
) -> Result<Option<channel_setup::TelegramOwnerCandidate>, ChannelSetupError> {
    let client = telegram_client(Duration::from_secs(35)).await?;

    let delete_url = telegram_api_url(token, "deleteWebhook")?;
    if let Err(error) = client.post(delete_url).send().await {
        tracing::warn!(
            error = %error.without_url(),
            "Failed to delete webhook (getUpdates may not work)"
        );
    }

    let baseline_updates = fetch_telegram_updates(&client, token, 0, None).await?;
    let mut next_offset = next_telegram_update_offset(&baseline_updates);
    let deadline = Instant::now() + Duration::from_secs(120);

    while Instant::now() < deadline {
        let remaining_secs = deadline.saturating_duration_since(Instant::now()).as_secs();
        let timeout_secs = remaining_secs.clamp(1, 30);
        let updates = match fetch_telegram_updates(&client, token, timeout_secs, next_offset).await
        {
            Ok(updates) => updates,
            Err(error) => {
                persist_telegram_runtime_offset_from_next_offset(next_offset);
                return Err(error);
            }
        };

        if let Some(capture) = extract_telegram_owner_capture(&updates) {
            if let Err(error) =
                acknowledge_telegram_update_offset(&client, token, capture.acknowledged_offset)
                    .await
            {
                tracing::warn!(
                    offset = capture.acknowledged_offset,
                    %error,
                    "Failed to acknowledge captured Telegram binding update; relying on local runtime offset fallback"
                );
            }

            if let Err(error) = persist_telegram_runtime_polling_offset(
                capture.acknowledged_offset,
                capture.ignored_update_upper_bound,
            ) {
                tracing::warn!(
                    offset = capture.acknowledged_offset,
                    %error,
                    "Failed to persist Telegram runtime polling offset after owner capture"
                );
            }

            return Ok(Some(capture.candidate));
        }

        if let Some(offset) = next_telegram_update_offset(&updates) {
            next_offset = Some(offset);
        }
    }

    // Even when no candidate is captured (timeout), persist the highest observed
    // offset so any pairing-window messages do not replay into runtime.
    persist_telegram_runtime_offset_from_next_offset(next_offset);

    Ok(None)
}

fn persist_telegram_runtime_offset_from_next_offset(next_offset: Option<i64>) {
    if let Some(offset) = next_offset
        && let Err(error) =
            persist_telegram_runtime_polling_offset(offset, offset.saturating_sub(1))
    {
        tracing::warn!(
            offset,
            %error,
            "Failed to persist Telegram runtime polling offset from observed high-watermark"
        );
    }
}

async fn persist_telegram_runtime_polling_snapshot(
    token: &SecretString,
) -> Result<(), ChannelSetupError> {
    let client = telegram_client(Duration::from_secs(15)).await?;

    let delete_url = telegram_api_url(token, "deleteWebhook")?;
    if let Err(error) = client.post(delete_url).send().await {
        tracing::warn!(
            error = %error.without_url(),
            "Failed to delete webhook while persisting Telegram snapshot"
        );
    }

    let mut next_offset = None;
    let mut high_watermark = None;

    // Drain pending updates in pages so we capture a reliable high-watermark
    // even when Telegram has more than one page buffered.
    for _ in 0..20 {
        let updates = fetch_telegram_updates(&client, token, 0, next_offset).await?;
        if updates.is_empty() {
            break;
        }

        let Some(observed_next_offset) = next_telegram_update_offset(&updates) else {
            break;
        };

        if next_offset == Some(observed_next_offset) {
            break;
        }

        high_watermark = Some(observed_next_offset);
        next_offset = Some(observed_next_offset);
    }

    persist_telegram_runtime_offset_from_next_offset(high_watermark);
    Ok(())
}

async fn acknowledge_telegram_update_offset(
    client: &Client,
    token: &SecretString,
    offset: i64,
) -> Result<(), ChannelSetupError> {
    if offset <= 0 {
        return Ok(());
    }

    let _ = fetch_telegram_updates(client, token, 0, Some(offset)).await?;
    Ok(())
}

fn persist_telegram_runtime_polling_offset(
    offset: i64,
    ignored_update_upper_bound: i64,
) -> Result<(), ChannelSetupError> {
    if offset <= 0 {
        return Ok(());
    }

    let workspace_path = crate::platform::state_paths()
        .channels_dir
        .join("telegram.workspace.json");
    ensure_parent_dir(&workspace_path)?;

    let mut state = read_telegram_workspace_state(&workspace_path);
    let existing_offset = state
        .get("channels/telegram/state/last_update_id")
        .and_then(|raw| raw.parse::<i64>().ok());
    if existing_offset.is_some_and(|current| current >= offset) {
        return Ok(());
    }

    state.insert(
        "channels/telegram/state/last_update_id".to_string(),
        offset.to_string(),
    );
    state.insert(
        "channels/telegram/state/last_emitted_update_id".to_string(),
        offset.saturating_sub(1).to_string(),
    );
    if ignored_update_upper_bound > 0 {
        let existing_ignored_upper_bound = state
            .get("channels/telegram/state/ignore_updates_until_id")
            .and_then(|raw| raw.parse::<i64>().ok());
        if existing_ignored_upper_bound.is_none_or(|current| current < ignored_update_upper_bound) {
            state.insert(
                "channels/telegram/state/ignore_updates_until_id".to_string(),
                ignored_update_upper_bound.to_string(),
            );
        }
    }

    write_telegram_workspace_state(&workspace_path, &state)
}

fn ensure_parent_dir(path: &Path) -> Result<(), ChannelSetupError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn read_telegram_workspace_state(path: &Path) -> HashMap<String, String> {
    let raw = match thinclaw_platform::read_regular_file_bounded(path, 1024 * 1024) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return HashMap::new(),
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "Failed to read Telegram workspace state file; starting with an empty state map"
            );
            return HashMap::new();
        }
    };

    serde_json::from_slice::<HashMap<String, String>>(&raw).unwrap_or_else(|error| {
        tracing::warn!(
            path = %path.display(),
            %error,
            "Failed to parse Telegram workspace state file; starting with an empty state map"
        );
        HashMap::new()
    })
}

fn write_telegram_workspace_state(
    path: &Path,
    state: &HashMap<String, String>,
) -> Result<(), ChannelSetupError> {
    let sorted: BTreeMap<&String, &String> = state.iter().collect();
    let serialized = serde_json::to_vec_pretty(&sorted).map_err(|error| {
        ChannelSetupError::Io(io::Error::other(format!(
            "Failed to serialize Telegram workspace state: {}",
            error
        )))
    })?;

    thinclaw_platform::write_private_file_atomic(path, &serialized, true)?;
    Ok(())
}

async fn wait_for_telegram_owner_tui(
    token: &SecretString,
    bot_username: Option<&str>,
) -> Result<TelegramBindingOutcome, ChannelSetupError> {
    use crossterm::{
        ExecutableCommand, cursor,
        event::{self, Event, KeyCode, KeyModifiers},
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    };
    use ratatui::{
        Terminal,
        backend::CrosstermBackend,
        prelude::*,
        widgets::{Block, Borders, Clear, Paragraph, Wrap},
    };

    use crate::terminal_branding::resolve_cli_skin_name;
    use crate::tui::skin::CliSkin;

    let mut capture_task = tokio::spawn({
        let token = token.clone();
        async move { capture_telegram_owner_candidate(&token).await }
    });

    enable_raw_mode().map_err(ChannelSetupError::Io)?;
    io::stdout()
        .execute(EnterAlternateScreen)
        .map_err(ChannelSetupError::Io)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).map_err(ChannelSetupError::Io)?;
    terminal.hide_cursor().map_err(ChannelSetupError::Io)?;

    let skin = CliSkin::load(&resolve_cli_skin_name());
    let started = Instant::now();
    let timeout = Duration::from_secs(120);
    let spinner_frames = ['|', '/', '-', '\\'];
    let mut frame_index = 0usize;

    let result = loop {
        let elapsed = started.elapsed();
        let remaining = timeout.saturating_sub(elapsed);
        let spinner = spinner_frames[frame_index % spinner_frames.len()];

        terminal
            .draw(|frame| {
                frame.render_widget(Clear, frame.area());

                let width = frame.area().width.saturating_sub(6).clamp(62, 92);
                let area = centered_rect(frame.area(), width, 18);
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(skin.accent_style())
                    .title(Span::styled(" Telegram Binding ", skin.accent_style()));
                let inner = block.inner(area);
                frame.render_widget(block, area);

                let mut lines = vec![
                    Line::from(Span::styled(
                        "ThinClaw is waiting for a fresh private Telegram message.",
                        skin.body_style().bold(),
                    )),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("1. ", skin.accent_style()),
                        Span::styled(
                            "Open Telegram on the account that should own this bot.",
                            skin.body_style(),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("2. ", skin.accent_style()),
                        Span::styled(
                            "Send a private message such as /start to the bot.",
                            skin.body_style(),
                        ),
                    ]),
                ];

                if let Some(username) = bot_username {
                    lines.push(Line::from(vec![
                        Span::styled("Bot: ", skin.accent_soft_style()),
                        Span::styled(format!("@{}", username), skin.body_style()),
                    ]));
                }

                lines.extend([
                    Line::from(""),
                    Line::from(vec![
                        Span::styled(format!("[{}] ", spinner), skin.accent_style()),
                        Span::styled(
                            "Listening for the first new private message after this step started.",
                            skin.body_style(),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Elapsed: ", skin.accent_soft_style()),
                        Span::styled(format!("{}s", elapsed.as_secs()), skin.body_style()),
                        Span::styled("   Remaining: ", skin.accent_soft_style()),
                        Span::styled(format!("{}s", remaining.as_secs()), skin.body_style()),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled(
                        "M manual ID   S skip binding   Esc exit setup   Ctrl+C abort",
                        skin.muted_style(),
                    )),
                ]);

                frame.render_widget(
                    Paragraph::new(Text::from(lines))
                        .wrap(Wrap { trim: false })
                        .alignment(Alignment::Left),
                    inner,
                );
            })
            .map_err(ChannelSetupError::Io)?;

        tokio::select! {
            joined = &mut capture_task => {
                let outcome = match joined {
                    Ok(Ok(Some(candidate))) => TelegramBindingOutcome::Bound(candidate),
                    Ok(Ok(None)) => TelegramBindingOutcome::TimedOut,
                    Ok(Err(error)) => break Err(error),
                    Err(error) => break Err(ChannelSetupError::Network(format!(
                        "Telegram binding task failed: {}",
                        error
                    ))),
                };
                break Ok(outcome);
            }
            _ = tokio::time::sleep(Duration::from_millis(120)) => {
                frame_index = frame_index.wrapping_add(1);
                let mut exit_action = None;
                while event::poll(Duration::from_millis(0)).map_err(ChannelSetupError::Io)? {
                    if let Event::Key(key) = event::read().map_err(ChannelSetupError::Io)? {
                        match (key.modifiers, key.code) {
                            (_, KeyCode::Char('m')) | (_, KeyCode::Char('M')) => {
                                capture_task.abort();
                                exit_action = Some(Ok(TelegramBindingOutcome::ManualEntryRequested));
                                break;
                            }
                            (_, KeyCode::Char('s')) | (_, KeyCode::Char('S')) => {
                                capture_task.abort();
                                exit_action = Some(Ok(TelegramBindingOutcome::Skipped));
                                break;
                            }
                            (_, KeyCode::Esc) => {
                                capture_task.abort();
                                exit_action = Some(Err(ChannelSetupError::Io(io::Error::new(
                                    io::ErrorKind::Interrupted,
                                    "Esc",
                                ))));
                                break;
                            }
                            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                                capture_task.abort();
                                exit_action = Some(Err(ChannelSetupError::Io(io::Error::new(
                                    io::ErrorKind::Interrupted,
                                    "Ctrl-C",
                                ))));
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(exit_action) = exit_action {
                    break exit_action;
                }
            }
        }
    };

    disable_raw_mode().map_err(ChannelSetupError::Io)?;
    io::stdout()
        .execute(LeaveAlternateScreen)
        .map_err(ChannelSetupError::Io)?;
    io::stdout()
        .execute(cursor::Show)
        .map_err(ChannelSetupError::Io)?;
    terminal.show_cursor().map_err(ChannelSetupError::Io)?;

    result
}

fn centered_rect(
    area: ratatui::layout::Rect,
    desired_width: u16,
    desired_height: u16,
) -> ratatui::layout::Rect {
    let width = desired_width.max(24).min(area.width);
    let height = desired_height.max(8).min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    ratatui::layout::Rect::new(x, y, width, height)
}

/// Returns the webhook secret if configured.
async fn setup_telegram_webhook_secret(
    secrets: &SecretsContext,
    tunnel: &TunnelSettings,
) -> Result<Option<String>, ChannelSetupError> {
    if tunnel.public_url.is_none() {
        crate::setup::prompts::print_blank_line();
        print_info("No tunnel configured — Telegram will use polling mode (~5s message delay).");
        print_info("This works perfectly for most users. To switch to instant webhook delivery,");
        print_info("configure a tunnel (Tailscale Funnel, ngrok, or Cloudflare) in setup.");
        return Ok(None);
    }

    crate::setup::prompts::print_blank_line();
    print_info("Telegram Webhook Security:");
    print_info("A webhook secret adds an extra layer of security by validating");
    print_info("that requests actually come from Telegram's servers.");

    if !confirm("Generate a webhook secret?", true)? {
        return Ok(None);
    }

    let secret = channel_setup::generate_webhook_secret();
    secrets
        .save_secret(
            "telegram_webhook_secret",
            &SecretString::from(secret.clone()),
        )
        .await?;
    print_success("Webhook secret generated and saved");

    Ok(Some(secret))
}

/// Validate a Telegram bot token by calling the getMe API.
///
/// Returns the bot's username if valid.
pub async fn validate_telegram_token(
    token: &SecretString,
) -> Result<Option<String>, ChannelSetupError> {
    let client = telegram_client(std::time::Duration::from_secs(10)).await?;

    let url = telegram_api_url(token, "getMe")?;

    let response =
        client.get(url).send().await.map_err(|e| {
            ChannelSetupError::Network(format!("Request failed: {}", e.without_url()))
        })?;

    if !response.status().is_success() {
        return Err(ChannelSetupError::Network(format!(
            "API returned status {}",
            response.status()
        )));
    }

    let body: TelegramGetMeResponse =
        crate::http_response::bounded_json(response, MAX_TELEGRAM_RESPONSE_BYTES)
            .await
            .map_err(|e| ChannelSetupError::Network(format!("Failed to parse response: {}", e)))?;

    if body.ok {
        Ok(body.result.and_then(|u| u.username))
    } else {
        Err(ChannelSetupError::Network(
            "Telegram API returned error".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TelegramUpdate, TelegramUpdateChat, TelegramUpdateMessage, TelegramUpdateUser,
        extract_telegram_owner_capture, next_telegram_update_offset,
    };
    use thinclaw_channels::setup::TelegramOwnerCandidate;

    #[test]
    fn test_extract_telegram_owner_candidate_uses_first_private_human_message() {
        let updates = vec![
            telegram_update(10, 42, "private", "Ada", Some("ada"), false),
            telegram_update(11, 99, "private", "Grace", Some("grace"), false),
        ];

        assert_eq!(
            extract_telegram_owner_capture(&updates).map(|capture| capture.candidate),
            Some(TelegramOwnerCandidate {
                user_id: 42,
                display_name: "@ada".to_string(),
                username: Some("ada".to_string()),
            })
        );
    }

    #[test]
    fn test_extract_telegram_owner_candidate_ignores_group_messages_and_bots() {
        let updates = vec![
            telegram_update(7, 13, "group", "Ignored Group", Some("group_user"), false),
            telegram_update(8, 21, "private", "Ignored Bot", Some("helper_bot"), true),
            telegram_update(9, 34, "private", "Owner", None, false),
        ];

        assert_eq!(
            extract_telegram_owner_capture(&updates).map(|capture| capture.candidate),
            Some(TelegramOwnerCandidate {
                user_id: 34,
                display_name: "Owner".to_string(),
                username: None,
            })
        );
    }

    #[test]
    fn test_extract_telegram_owner_capture_tracks_acknowledged_offset() {
        let updates = vec![
            telegram_update(21, 17, "private", "Owner", Some("owner"), false),
            telegram_update(24, 88, "private", "Another", Some("another"), false),
        ];

        let capture = extract_telegram_owner_capture(&updates)
            .expect("expected capture from private message");
        assert_eq!(
            capture.candidate,
            TelegramOwnerCandidate {
                user_id: 17,
                display_name: "@owner".to_string(),
                username: Some("owner".to_string()),
            }
        );
        assert_eq!(capture.acknowledged_offset, 25);
        assert_eq!(capture.ignored_update_upper_bound, 24);
    }

    #[test]
    fn test_next_telegram_update_offset_tracks_latest_update_plus_one() {
        let updates = vec![
            telegram_update(4, 1, "private", "One", None, false),
            telegram_update(12, 2, "private", "Two", None, false),
            telegram_update(9, 3, "private", "Three", None, false),
        ];

        assert_eq!(next_telegram_update_offset(&updates), Some(13));
    }

    fn telegram_update(
        update_id: i64,
        user_id: i64,
        chat_type: &str,
        first_name: &str,
        username: Option<&str>,
        is_bot: bool,
    ) -> TelegramUpdate {
        TelegramUpdate {
            update_id,
            message: Some(TelegramUpdateMessage {
                from: Some(TelegramUpdateUser {
                    id: user_id,
                    first_name: first_name.to_string(),
                    username: username.map(str::to_string),
                    is_bot,
                }),
                chat: Some(TelegramUpdateChat {
                    chat_type: chat_type.to_string(),
                }),
            }),
        }
    }
}
