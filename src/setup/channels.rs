//! Channel-specific setup flows.
//!
//! Each channel (Telegram, HTTP, etc.) has its own setup function that:
//! 1. Displays setup instructions
//! 2. Collects configuration (tokens, ports, etc.)
//! 3. Validates the configuration
//! 4. Saves secrets to the database

use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::Url;
use uuid::Uuid;

use crate::pairing::PairingStore;
#[cfg(feature = "postgres")]
use crate::secrets::SecretsCrypto;
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::settings::{Settings, TunnelSettings};
use crate::setup::prompts::{
    PromptUiMode, confirm, current_prompt_ui_mode, input, optional_input, print_blank_line,
    print_error, print_info, print_success, print_warning, secret_input, select_one,
};

/// Typed errors for channel setup flows.
#[derive(Debug, thiserror::Error)]
pub enum ChannelSetupError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Network(String),

    #[error("{0}")]
    Secrets(String),

    #[error("{0}")]
    Validation(String),
}

/// Context for saving secrets during setup.
pub struct SecretsContext {
    store: Arc<dyn SecretsStore>,
    user_id: String,
}

impl SecretsContext {
    /// Create a new secrets context from a trait-object store.
    pub fn from_store(store: Arc<dyn SecretsStore>, user_id: &str) -> Self {
        Self {
            store,
            user_id: user_id.to_string(),
        }
    }

    /// Create a new secrets context from a PostgreSQL pool and crypto.
    #[cfg(feature = "postgres")]
    pub fn new(pool: deadpool_postgres::Pool, crypto: Arc<SecretsCrypto>, user_id: &str) -> Self {
        Self {
            store: Arc::new(crate::secrets::PostgresSecretsStore::new(pool, crypto)),
            user_id: user_id.to_string(),
        }
    }

    /// Save a secret to the database.
    pub async fn save_secret(
        &self,
        name: &str,
        value: &SecretString,
    ) -> Result<(), ChannelSetupError> {
        let params = CreateSecretParams::new(name, value.expose_secret());

        self.store
            .create(&self.user_id, params)
            .await
            .map_err(|e| ChannelSetupError::Secrets(format!("Failed to save secret: {}", e)))?;

        Ok(())
    }

    /// Check if a secret exists.
    pub async fn secret_exists(&self, name: &str) -> bool {
        match self.store.exists(&self.user_id, name).await {
            Ok(exists) => exists,
            Err(e) => {
                tracing::warn!(secret = name, error = %e, "Failed to check if secret exists, assuming absent");
                false
            }
        }
    }

    /// Read a secret from the database (decrypted).
    pub async fn get_secret(&self, name: &str) -> Result<SecretString, ChannelSetupError> {
        let decrypted = self
            .store
            .get_for_injection(
                &self.user_id,
                name,
                crate::secrets::SecretAccessContext::new("setup.channels", "setup_validation"),
            )
            .await
            .map_err(|e| ChannelSetupError::Secrets(format!("Failed to read secret: {}", e)))?;
        Ok(SecretString::from(decrypted.expose().to_string()))
    }
}

const TUNNEL_NGROK_TOKEN_SECRET: &str = "tunnel_ngrok_token";
const TUNNEL_CF_TOKEN_SECRET: &str = "tunnel_cf_token";

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
struct TelegramOwnerCandidate {
    user_id: i64,
    display_name: String,
    username: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TelegramOwnerCapture {
    candidate: TelegramOwnerCandidate,
    acknowledged_offset: i64,
    ignored_update_upper_bound: i64,
}

impl TelegramOwnerCandidate {
    fn summary(&self) -> String {
        if let Some(username) = self.username.as_deref() {
            format!("@{} (ID: {})", username, self.user_id)
        } else {
            format!("{} (ID: {})", self.display_name, self.user_id)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TelegramBindingOutcome {
    Bound(TelegramOwnerCandidate),
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

fn extract_telegram_owner_capture(updates: &[TelegramUpdate]) -> Option<TelegramOwnerCapture> {
    updates.iter().find_map(|update| {
        let message = update.message.as_ref()?;
        let from = message.from.as_ref()?;
        let chat = message.chat.as_ref()?;
        if from.is_bot || chat.chat_type != "private" {
            return None;
        }

        let display_name = from
            .username
            .as_ref()
            .map(|username| format!("@{}", username))
            .unwrap_or_else(|| from.first_name.clone());

        let candidate = TelegramOwnerCandidate {
            user_id: from.id,
            display_name,
            username: from.username.clone(),
        };
        let acknowledged_offset = next_telegram_update_offset(updates)
            .unwrap_or_else(|| update.update_id.saturating_add(1));
        let ignored_update_upper_bound = acknowledged_offset.saturating_sub(1);

        Some(TelegramOwnerCapture {
            candidate,
            acknowledged_offset,
            ignored_update_upper_bound,
        })
    })
}

fn next_telegram_update_offset(updates: &[TelegramUpdate]) -> Option<i64> {
    updates
        .iter()
        .map(|update| update.update_id.saturating_add(1))
        .max()
}

async fn fetch_telegram_updates(
    client: &Client,
    token: &SecretString,
    timeout_secs: u64,
    offset: Option<i64>,
) -> Result<Vec<TelegramUpdate>, ChannelSetupError> {
    let updates_url = format!(
        "https://api.telegram.org/bot{}/getUpdates",
        token.expose_secret()
    );
    let mut query = vec![
        ("timeout".to_string(), timeout_secs.to_string()),
        ("allowed_updates".to_string(), "[\"message\"]".to_string()),
    ];
    if let Some(offset) = offset {
        query.push(("offset".to_string(), offset.to_string()));
    }

    let response = client
        .get(&updates_url)
        .query(&query)
        .send()
        .await
        .map_err(|e| ChannelSetupError::Network(format!("getUpdates request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(ChannelSetupError::Network(format!(
            "getUpdates returned status {}",
            response.status()
        )));
    }

    let body: TelegramGetUpdatesResponse = response.json().await.map_err(|e| {
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
) -> Result<Option<TelegramOwnerCandidate>, ChannelSetupError> {
    let client = Client::builder()
        .timeout(Duration::from_secs(35))
        .build()
        .map_err(|e| ChannelSetupError::Network(format!("Failed to create HTTP client: {}", e)))?;

    let delete_url = format!(
        "https://api.telegram.org/bot{}/deleteWebhook",
        token.expose_secret()
    );
    if let Err(error) = client.post(&delete_url).send().await {
        tracing::warn!("Failed to delete webhook (getUpdates may not work): {error}");
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
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| ChannelSetupError::Network(format!("Failed to create HTTP client: {}", e)))?;

    let delete_url = format!(
        "https://api.telegram.org/bot{}/deleteWebhook",
        token.expose_secret()
    );
    if let Err(error) = client.post(&delete_url).send().await {
        tracing::warn!("Failed to delete webhook while persisting Telegram snapshot: {error}");
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
    let raw = match std::fs::read_to_string(path) {
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

    serde_json::from_str::<HashMap<String, String>>(&raw).unwrap_or_else(|error| {
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

    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, serialized)?;
    std::fs::rename(&tmp_path, path)?;
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

/// Set up a tunnel for exposing the agent to the internet.
///
/// This is shared across all channels that need webhook endpoints.
/// Returns a `TunnelSettings` with provider config (managed tunnel)
/// or a static URL.
pub async fn setup_tunnel(
    settings: &Settings,
    secrets: Option<&SecretsContext>,
) -> Result<TunnelSettings, ChannelSetupError> {
    // Show existing config
    let has_existing = settings.tunnel.public_url.is_some() || settings.tunnel.provider.is_some();
    if has_existing {
        crate::setup::prompts::print_blank_line();
        print_info("Current tunnel configuration:");
        let t = &settings.tunnel;
        let has_ngrok_secret = if let Some(ctx) = secrets {
            ctx.secret_exists(TUNNEL_NGROK_TOKEN_SECRET).await
        } else {
            false
        };
        let has_cf_secret = if let Some(ctx) = secrets {
            ctx.secret_exists(TUNNEL_CF_TOKEN_SECRET).await
        } else {
            false
        };
        match t.provider.as_deref() {
            Some("ngrok") => {
                print_info("  Provider:  ngrok");
                if let Some(ref domain) = t.ngrok_domain {
                    print_info(&format!("  Domain:    {}", domain));
                }
                if t.ngrok_token.is_some() || has_ngrok_secret {
                    print_info("  Auth:      token configured");
                }
            }
            Some("cloudflare") => {
                print_info("  Provider:  Cloudflare Tunnel");
                if t.cf_token.is_some() || has_cf_secret {
                    print_info("  Auth:      token configured");
                }
            }
            Some("tailscale") => {
                let mode = if t.ts_funnel {
                    "Funnel (public)"
                } else {
                    "Serve (tailnet-only)"
                };
                print_info(&format!("  Provider:  Tailscale {}", mode));
                if let Some(ref hostname) = t.ts_hostname {
                    print_info(&format!("  Hostname:  {}", hostname));
                }
            }
            Some("custom") => {
                print_info("  Provider:  Custom command");
                if let Some(ref cmd) = t.custom_command {
                    print_info(&format!("  Command:   {}", cmd));
                }
                if let Some(ref url) = t.custom_health_url {
                    print_info(&format!("  Health:    {}", url));
                }
            }
            Some(other) => {
                print_info(&format!("  Provider:  {}", other));
            }
            None => {}
        }
        if let Some(ref url) = t.public_url {
            print_info(&format!("  URL:       {}", url));
        }
        crate::setup::prompts::print_blank_line();
        if !confirm("Change tunnel configuration?", false)? {
            return Ok(settings.tunnel.clone());
        }
    }

    crate::setup::prompts::print_blank_line();
    print_info("Tunnel Configuration");
    crate::setup::prompts::print_blank_line();
    print_info("Without a tunnel, channels like Telegram use POLLING mode:");
    print_info("  Your agent asks Telegram \"any new messages?\" every ~5 seconds.");
    print_info("  This works reliably from anywhere (home WiFi, VPN, any network).");
    crate::setup::prompts::print_blank_line();
    print_info("With a tunnel, channels switch to WEBHOOK mode:");
    print_info("  Telegram pushes messages to your agent INSTANTLY (< 200ms).");
    print_info("  Also enables: Slack events, Discord interactions, GitHub webhooks.");
    crate::setup::prompts::print_blank_line();
    print_info("Why is a tunnel needed?");
    print_info("  Webhooks require a publicly reachable HTTPS URL. Most home networks");
    print_info("  use NAT/firewall — Telegram's servers simply cannot reach your machine");
    print_info("  without a tunnel creating a public entrypoint.");
    crate::setup::prompts::print_blank_line();
    print_info("Recommended: Tailscale Funnel (free, zero-config, persistent hostname)");
    print_info("  Alternatives: ngrok (free), Cloudflare Tunnel (free), or your own.");
    print_info("  If you're unsure, skip this — polling works perfectly for most users.");
    crate::setup::prompts::print_blank_line();

    if !confirm("Configure a tunnel for instant webhook delivery?", false)? {
        print_info("No tunnel configured. Telegram and other channels will use polling mode.");
        return Ok(TunnelSettings::default());
    }

    let options = &[
        "ngrok         - managed tunnel, starts automatically",
        "Cloudflare    - cloudflared tunnel, starts automatically",
        "Tailscale     - Tailscale Funnel/Serve, starts automatically",
        "Custom        - your own tunnel command",
        "Static URL    - you manage the tunnel yourself",
    ];

    let choice = select_one("Select tunnel provider:", options)?;

    match choice {
        0 => setup_tunnel_ngrok(secrets).await,
        1 => setup_tunnel_cloudflare(secrets).await,
        2 => setup_tunnel_tailscale(),
        3 => setup_tunnel_custom(),
        4 => setup_tunnel_static(),
        _ => Ok(TunnelSettings::default()),
    }
}

async fn setup_tunnel_ngrok(
    secrets: Option<&SecretsContext>,
) -> Result<TunnelSettings, ChannelSetupError> {
    // Check if ngrok is installed
    if !is_binary_installed("ngrok") {
        crate::setup::prompts::print_blank_line();
        print_error("'ngrok' binary not found in PATH.");
        print_info("Install ngrok before starting the agent:");
        print_info("  macOS:   brew install ngrok");
        print_info("  Linux:   snap install ngrok  (or download from https://ngrok.com/download)");
        print_info("  Windows: choco install ngrok");
        crate::setup::prompts::print_blank_line();
        if !confirm(
            "Continue configuring ngrok anyway? (you can install it before starting the agent)",
            false,
        )? {
            return Ok(TunnelSettings::default());
        }
    }

    print_info("Get your auth token from: https://dashboard.ngrok.com/get-started/your-authtoken");
    crate::setup::prompts::print_blank_line();

    let token = secret_input("ngrok auth token")?;
    let domain = optional_input("Custom domain", Some("leave empty for auto-assigned"))?;
    let ngrok_token = if let Some(ctx) = secrets {
        ctx.save_secret(TUNNEL_NGROK_TOKEN_SECRET, &token).await?;
        None
    } else {
        Some(token.expose_secret().to_string())
    };

    print_success("ngrok configured. Tunnel will start automatically at boot.");
    if !is_binary_installed("ngrok") {
        print_info("⚠ Remember to install 'ngrok' before running 'thinclaw run'.");
    }

    Ok(TunnelSettings {
        provider: Some("ngrok".to_string()),
        ngrok_token,
        ngrok_domain: domain,
        ..Default::default()
    })
}

async fn setup_tunnel_cloudflare(
    secrets: Option<&SecretsContext>,
) -> Result<TunnelSettings, ChannelSetupError> {
    // Check if cloudflared is installed
    if !is_binary_installed("cloudflared") {
        crate::setup::prompts::print_blank_line();
        print_error("'cloudflared' binary not found in PATH.");
        print_info("Install cloudflared before starting the agent:");
        print_info("  macOS:   brew install cloudflare/cloudflare/cloudflared");
        print_info(
            "  Linux:   See https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/",
        );
        print_info("  Windows: winget install Cloudflare.cloudflared");
        crate::setup::prompts::print_blank_line();
        if !confirm(
            "Continue configuring cloudflared anyway? (you can install it before starting the agent)",
            false,
        )? {
            return Ok(TunnelSettings::default());
        }
    }

    print_info("Get your tunnel token from the Cloudflare Zero Trust dashboard:");
    print_info("  https://one.dash.cloudflare.com/ > Networks > Tunnels");
    crate::setup::prompts::print_blank_line();

    let token = secret_input("Cloudflare tunnel token")?;
    let cf_token = if let Some(ctx) = secrets {
        ctx.save_secret(TUNNEL_CF_TOKEN_SECRET, &token).await?;
        None
    } else {
        Some(token.expose_secret().to_string())
    };

    print_success("Cloudflare tunnel configured. Tunnel will start automatically at boot.");
    if !is_binary_installed("cloudflared") {
        print_info("⚠ Remember to install 'cloudflared' before running 'thinclaw run'.");
    }

    Ok(TunnelSettings {
        provider: Some("cloudflare".to_string()),
        cf_token,
        ..Default::default()
    })
}

/// Test whether the `tailscale` CLI can actually run without crashing.
///
/// On macOS, the App Store version's CLI wrapper crashes with a
/// `BundleIdentifier` error when spawned from another process.
/// This function catches that by running `tailscale version` and
/// checking for a clean exit.
///
/// Uses `resolve_binary` so that Homebrew-installed CLIs at
/// `/opt/homebrew/bin/tailscale` are found even when that directory
/// is not in `$PATH` (common for processes spawned by launchd/IDEs).
fn test_tailscale_cli() -> bool {
    let binary = crate::util::resolve_binary("tailscale");
    let output = std::process::Command::new(&binary)
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output();

    match output {
        Ok(o) => {
            if o.status.success() {
                return true;
            }
            // Check if it crashed with the known macOS issue
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("BundleIdentifier") || stderr.contains("Fatal error") {
                return false;
            }
            // Other non-zero exit might still mean it's installed (e.g. not logged in)
            // but at least it didn't crash
            true
        }
        Err(_) => false, // binary not found
    }
}

fn setup_tunnel_tailscale() -> Result<TunnelSettings, ChannelSetupError> {
    // Check if tailscale CLI is installed AND working.
    // On macOS, the App Store version installs a CLI shim that crashes with
    // BundleIdentifier errors when spawned from another process.
    let cli_working = test_tailscale_cli();

    if !cli_working {
        crate::setup::prompts::print_blank_line();

        #[cfg(target_os = "macos")]
        {
            if is_binary_installed("tailscale") {
                // CLI exists but crashes — the App Store BundleIdentifier issue
                print_error("Tailscale CLI is installed but crashes when called from ThinClaw.");
                print_info("This is a known issue with the macOS App Store version's CLI.");
                print_info("The standalone Homebrew CLI fixes this and works alongside the app.");
            } else if std::path::Path::new("/Applications/Tailscale.app").exists() {
                print_info("Tailscale app is installed, but the CLI is not available.");
            } else {
                print_error("Tailscale is not installed.");
            }

            crate::setup::prompts::print_blank_line();

            // Check if Homebrew is available for auto-install
            let has_brew = std::process::Command::new("brew")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);

            if has_brew {
                if confirm(
                    "Install Tailscale CLI via Homebrew? (brew install tailscale)",
                    true,
                )? {
                    print_info("Installing tailscale via Homebrew (this may take a minute)...");
                    let install_result = std::process::Command::new("brew")
                        .args(["install", "tailscale"])
                        .status();

                    match install_result {
                        Ok(status) if status.success() => {
                            if test_tailscale_cli() {
                                print_success("Tailscale CLI installed and working!");
                            } else {
                                print_success("Tailscale CLI installed.");
                                print_info(
                                    "You may need to start the service: brew services start tailscale",
                                );
                            }
                        }
                        Ok(_) => {
                            print_error(
                                "Homebrew install failed. Try manually: brew install tailscale",
                            );
                            crate::setup::prompts::print_blank_line();
                            if !confirm("Continue configuring anyway?", false)? {
                                return Ok(TunnelSettings::default());
                            }
                        }
                        Err(e) => {
                            print_error(&format!("Could not run brew: {}", e));
                            if !confirm("Continue configuring anyway?", false)? {
                                return Ok(TunnelSettings::default());
                            }
                        }
                    }
                } else if !confirm(
                    "Continue without installing? (install before starting the agent)",
                    false,
                )? {
                    return Ok(TunnelSettings::default());
                }
            } else {
                // No Homebrew
                print_info("Homebrew is not installed. Install the Tailscale CLI manually:");
                crate::setup::prompts::print_blank_line();
                print_info("  Option 1: Install Homebrew, then: brew install tailscale");
                print_info("  Option 2: Download from https://tailscale.com/download/mac");
                crate::setup::prompts::print_blank_line();
                if !confirm(
                    "Continue configuring anyway? (install before starting the agent)",
                    false,
                )? {
                    return Ok(TunnelSettings::default());
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            print_error("'tailscale' CLI not found in PATH.");
            print_info("Install Tailscale before starting the agent:");
            print_info("  Linux:   curl -fsSL https://tailscale.com/install.sh | sh");
            print_info("  Windows: Download from https://tailscale.com/download/windows");
            crate::setup::prompts::print_blank_line();
            if !confirm(
                "Continue configuring anyway? (install before starting the agent)",
                false,
            )? {
                return Ok(TunnelSettings::default());
            }
        }
    }

    crate::setup::prompts::print_blank_line();
    print_info("Tailscale offers two modes:");
    crate::setup::prompts::print_blank_line();
    print_info("  Funnel (public)  — Makes your agent reachable from the public internet.");
    print_info("                     Required for Telegram/Slack/Discord webhooks.");
    print_info("                     Your hostname (e.g. my-mac.tail1234.ts.net) becomes");
    print_info("                     publicly resolvable with a valid HTTPS certificate.");
    crate::setup::prompts::print_blank_line();
    print_info("  Serve (tailnet)  — Only reachable from devices on YOUR Tailscale network.");
    print_info("                     Great for private Web UI access from your phone/laptop,");
    print_info("                     but Telegram's servers CANNOT reach it (webhooks won't work,");
    print_info("                     Telegram will fall back to polling mode).");
    crate::setup::prompts::print_blank_line();

    let funnel = confirm(
        "Use Tailscale Funnel (public internet — needed for webhooks)?",
        true,
    )?;
    let hostname = optional_input("Hostname override", Some("leave empty for auto-detect"))?;

    let mode = if funnel {
        "Funnel (public)"
    } else {
        "Serve (tailnet-only)"
    };
    print_success(&format!("Tailscale {} configured.", mode));
    if funnel {
        print_info("Make sure Funnel is enabled in your Tailscale admin console:");
        print_info("  1. Visit https://login.tailscale.com/admin/dns → enable HTTPS");
        print_info("  2. Ensure your ACL policy allows Funnel for this machine");
    } else {
        print_info("Note: Telegram and other webhook channels will use polling mode.");
        print_info(
            "You can switch to Funnel later by re-running setup or setting TUNNEL_TS_FUNNEL=true.",
        );
    }
    if !is_binary_installed("tailscale") {
        print_info("⚠ Remember to install 'tailscale' before running 'thinclaw run'.");
    }

    Ok(TunnelSettings {
        provider: Some("tailscale".to_string()),
        ts_funnel: funnel,
        ts_hostname: hostname,
        ..Default::default()
    })
}

fn setup_tunnel_custom() -> Result<TunnelSettings, ChannelSetupError> {
    print_info("Enter a shell command to start your tunnel.");
    print_info("Use {port} and {host} as placeholders.");
    print_info("Example: bore local {port} --to bore.pub");
    crate::setup::prompts::print_blank_line();

    let command = input("Tunnel command")?;
    if command.is_empty() {
        return Err(ChannelSetupError::Validation(
            "Tunnel command cannot be empty".to_string(),
        ));
    }

    let health_url = optional_input("Health check URL", Some("optional"))?;
    let url_pattern = optional_input(
        "URL pattern (substring to match in stdout)",
        Some("optional"),
    )?;

    print_success("Custom tunnel configured.");

    Ok(TunnelSettings {
        provider: Some("custom".to_string()),
        custom_command: Some(command),
        custom_health_url: health_url,
        custom_url_pattern: url_pattern,
        ..Default::default()
    })
}

fn setup_tunnel_static() -> Result<TunnelSettings, ChannelSetupError> {
    print_info("Enter the public URL of your externally managed tunnel.");
    crate::setup::prompts::print_blank_line();

    let tunnel_url = input("Tunnel URL (e.g., https://abc123.ngrok.io)")?;

    if !tunnel_url.starts_with("https://") {
        print_error("URL must start with https:// (webhooks require HTTPS)");
        return Err(ChannelSetupError::Validation(
            "Invalid tunnel URL: must use HTTPS".to_string(),
        ));
    }

    let tunnel_url = tunnel_url.trim_end_matches('/').to_string();

    print_success(&format!("Static tunnel URL configured: {}", tunnel_url));
    print_info("Make sure your tunnel is running before starting the agent.");

    Ok(TunnelSettings {
        public_url: Some(tunnel_url),
        ..Default::default()
    })
}

/// Check if a binary is available in PATH or at a known fallback location.
///
/// Delegates to `resolve_binary` from the tunnel module, which checks
/// PATH first and then falls back to known macOS Homebrew paths
/// (including `/opt/homebrew/bin/tailscale` and `/usr/local/bin/tailscale`).
fn is_binary_installed(name: &str) -> bool {
    let resolved = crate::util::resolve_binary(name);

    // resolve_binary returns the bare name if nothing was found;
    // if it returned an absolute path, the binary exists at that path.
    if resolved != name {
        return true;
    }

    // resolve_binary returned the bare name — check if it's on PATH.
    #[cfg(unix)]
    {
        std::process::Command::new("which")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        std::process::Command::new("where")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Set up Telegram webhook secret for signature validation.
///
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

    let secret = generate_webhook_secret();
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
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| ChannelSetupError::Network(format!("Failed to create HTTP client: {}", e)))?;

    let url = format!(
        "https://api.telegram.org/bot{}/getMe",
        token.expose_secret()
    );

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| ChannelSetupError::Network(format!("Request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(ChannelSetupError::Network(format!(
            "API returned status {}",
            response.status()
        )));
    }

    let body: TelegramGetMeResponse = response
        .json()
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

/// Result of HTTP webhook setup.
#[derive(Debug, Clone)]
pub struct HttpSetupResult {
    pub enabled: bool,
    pub port: u16,
    pub host: String,
}

/// Result of Signal channel setup.
#[derive(Debug, Clone)]
pub struct SignalSetupResult {
    pub enabled: bool,
    pub http_url: String,
    pub account: String,
    pub allow_from: String,
    pub allow_from_groups: String,
    pub dm_policy: String,
    pub group_policy: String,
    pub group_allow_from: String,
}

/// Set up HTTP webhook channel.
pub async fn setup_http(secrets: &SecretsContext) -> Result<HttpSetupResult, ChannelSetupError> {
    print_info("HTTP webhook setup");
    print_blank_line();
    print_info("The HTTP webhook allows external services to send messages to the agent.");
    print_blank_line();

    let port_str = optional_input("Port", Some("default: 8080"))?;
    let port: u16 = port_str
        .as_deref()
        .unwrap_or("8080")
        .parse()
        .map_err(|e| ChannelSetupError::Validation(format!("Invalid port: {}", e)))?;

    if port < 1024 {
        print_info("Note: Ports below 1024 may require root privileges");
    }

    let host =
        optional_input("Host", Some("default: 0.0.0.0"))?.unwrap_or_else(|| "0.0.0.0".to_string());

    // Generate a webhook secret
    if confirm("Generate a webhook secret for authentication?", true)? {
        let secret = generate_webhook_secret();
        secrets
            .save_secret("http_webhook_secret", &SecretString::from(secret))
            .await?;
        print_success("Webhook secret generated and saved to database");
        print_info("Retrieve it later with: thinclaw secret get http_webhook_secret");
    }

    print_success(&format!("HTTP webhook will listen on {}:{}", host, port));

    Ok(HttpSetupResult {
        enabled: true,
        port,
        host,
    })
}

/// Generate a random webhook secret.
pub fn generate_webhook_secret() -> String {
    generate_secret_with_length(32)
}

fn validate_e164(account: &str) -> Result<(), String> {
    if !account.starts_with('+') {
        return Err("E.164 account must start with '+'".to_string());
    }
    let digits = &account[1..];
    if digits.is_empty() {
        return Err("E.164 account must have digits after '+'".to_string());
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err("E.164 account must contain only digits after '+'".to_string());
    }
    if digits.len() < 7 || digits.len() > 15 {
        return Err("E.164 account must be 7-15 digits after '+'".to_string());
    }
    Ok(())
}

fn validate_allow_from_list(list: &str) -> Result<(), String> {
    if list.is_empty() {
        return Ok(());
    }
    for (i, item) in list.split(',').enumerate() {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "*" {
            continue;
        }
        if let Some(uuid_part) = trimmed.strip_prefix("uuid:") {
            if Uuid::parse_str(uuid_part).is_err() {
                return Err(format!(
                    "allow_from[{}]: '{}' is not a valid UUID (after 'uuid:' prefix)",
                    i, trimmed
                ));
            }
            continue;
        }
        if validate_e164(trimmed).is_ok() {
            continue;
        }
        if Uuid::parse_str(trimmed).is_ok() {
            continue;
        }
        return Err(format!(
            "allow_from[{}]: '{}' must be '*', E.164 phone number, UUID, or 'uuid:<id>'",
            i, trimmed
        ));
    }
    Ok(())
}

fn validate_allow_from_groups_list(list: &str) -> Result<(), String> {
    if list.is_empty() {
        return Ok(());
    }
    for item in list.split(',') {
        let trimmed = item.trim();
        if trimmed.is_empty() || trimmed == "*" {
            continue;
        }
    }
    Ok(())
}

/// Set up Signal channel.
/// `Settings` is reserved for future use
pub async fn setup_signal(_settings: &Settings) -> Result<SignalSetupResult, ChannelSetupError> {
    print_info("Signal channel setup");
    print_blank_line();
    print_info("Signal channel connects to a signal-cli daemon running in HTTP mode.");
    print_blank_line();

    let http_url = input("Signal-cli HTTP URL")?;
    match Url::parse(&http_url) {
        Ok(url) if url.scheme() == "http" || url.scheme() == "https" => {}
        Ok(_) => {
            print_error("URL must use http or https scheme");
            return Err(ChannelSetupError::Validation(
                "Invalid HTTP URL: must use http or https scheme".to_string(),
            ));
        }
        Err(e) => {
            print_error(&format!("Invalid URL: {}", e));
            return Err(ChannelSetupError::Validation(format!(
                "Invalid HTTP URL: {}",
                e
            )));
        }
    }

    let account = input("Signal account (E.164)")?;
    if let Err(e) = validate_e164(&account) {
        print_error(&e);
        return Err(ChannelSetupError::Validation(e));
    }

    let allow_from = optional_input(
        "Allow from (comma-separated: E.164 numbers, '*' for anyone, UUIDs or 'uuid:<id>'; empty for self-only)",
        Some(&format!("default: {} (self-only)", account)),
    )?
    .unwrap_or_else(|| account.clone());

    let dm_policy = optional_input(
        "DM policy (open, allowlist, pairing)",
        Some("default: pairing"),
    )?
    .unwrap_or_else(|| "pairing".to_string());

    let allow_from_groups = optional_input(
        "Allow from groups (comma-separated group IDs, '*' for any group; empty for none)",
        Some("default: (none)"),
    )?
    .unwrap_or_default();

    let group_policy = optional_input(
        "Group policy (allowlist, open, disabled)",
        Some("default: allowlist"),
    )?
    .unwrap_or_else(|| "allowlist".to_string());

    let group_allow_from = optional_input(
        "Group allow from (comma-separated member IDs; empty to inherit from allow_from)",
        Some("default: (inherit from allow_from)"),
    )?
    .unwrap_or_default();

    if let Err(e) = validate_allow_from_list(&allow_from) {
        print_error(&e);
        return Err(ChannelSetupError::Validation(e));
    }

    if let Err(e) = validate_allow_from_groups_list(&allow_from_groups) {
        print_error(&e);
        return Err(ChannelSetupError::Validation(e));
    }

    crate::setup::prompts::print_blank_line();
    print_success(&format!(
        "Signal channel configured for account: {}",
        account
    ));
    print_info(&format!("HTTP URL: {}", http_url));
    if allow_from == account {
        print_info("Allow from: self-only");
    } else {
        print_info(&format!("Allow from: {}", allow_from));
    }
    print_info(&format!("DM policy: {}", dm_policy));
    if allow_from_groups.is_empty() {
        print_info("Allow from groups: (none)");
    } else {
        print_info(&format!("Allow from groups: {}", allow_from_groups));
    }
    print_info(&format!("Group policy: {}", group_policy));
    if group_allow_from.is_empty() {
        print_info("Group allow from: (inherits from allow_from)");
    } else {
        print_info(&format!("Group allow from: {}", group_allow_from));
    }

    Ok(SignalSetupResult {
        enabled: true,
        http_url,
        account,
        allow_from,
        allow_from_groups,
        dm_policy,
        group_policy,
        group_allow_from,
    })
}

/// Result of WASM channel setup.
#[derive(Debug, Clone)]
pub struct WasmChannelSetupResult {
    pub enabled: bool,
    pub channel_name: String,
}

/// Set up a WASM channel using its capabilities file setup schema.
///
/// Reads setup requirements from the channel's capabilities file and
/// prompts the user for each required secret.
pub async fn setup_wasm_channel(
    secrets: &SecretsContext,
    channel_name: &str,
    setup: &crate::channels::wasm::SetupSchema,
) -> Result<WasmChannelSetupResult, ChannelSetupError> {
    print_info(&format!("{channel_name} setup"));
    print_blank_line();

    for secret_config in &setup.required_secrets {
        // Check if this secret already exists
        if secrets.secret_exists(&secret_config.name).await {
            print_info(&format!(
                "Existing {} found in database.",
                secret_config.name
            ));
            if !confirm("Replace existing value?", false)? {
                continue;
            }
        }

        // Get the value from user or auto-generate
        let value = if secret_config.optional {
            let input_value =
                optional_input(&secret_config.prompt, Some("leave empty to auto-generate"))?;

            if let Some(v) = input_value {
                if !v.is_empty() {
                    SecretString::from(v)
                } else if let Some(ref auto_gen) = secret_config.auto_generate {
                    let generated = generate_secret_with_length(auto_gen.length);
                    print_info(&format!(
                        "Auto-generated {} ({} bytes)",
                        secret_config.name, auto_gen.length
                    ));
                    SecretString::from(generated)
                } else {
                    continue; // Skip optional secret with no auto-generate
                }
            } else if let Some(ref auto_gen) = secret_config.auto_generate {
                let generated = generate_secret_with_length(auto_gen.length);
                print_info(&format!(
                    "Auto-generated {} ({} bytes)",
                    secret_config.name, auto_gen.length
                ));
                SecretString::from(generated)
            } else {
                continue; // Skip optional secret with no auto-generate
            }
        } else {
            // Required secret
            let input_value = secret_input(&secret_config.prompt)?;

            // Validate if pattern is provided
            if let Some(ref pattern) = secret_config.validation {
                let re = regex::Regex::new(pattern).map_err(|e| {
                    ChannelSetupError::Validation(format!("Invalid validation pattern: {}", e))
                })?;
                if !re.is_match(input_value.expose_secret()) {
                    print_error(&format!(
                        "Value does not match expected format: {}",
                        pattern
                    ));
                    return Err(ChannelSetupError::Validation(
                        "Validation failed".to_string(),
                    ));
                }
            }

            input_value
        };

        // Save the secret
        secrets.save_secret(&secret_config.name, &value).await?;
        print_success(&format!("{} saved to database", secret_config.name));
    }

    // Validate configured credentials by substituting secrets into the
    // validation URL and making a GET request to verify they work.
    if let Some(ref validation_endpoint) = setup.validation_endpoint {
        let mut url = validation_endpoint.clone();

        // Substitute secret placeholders: {{secret_name}} → actual value
        for secret_config in &setup.required_secrets {
            let placeholder = format!("{{{{{}}}}}", secret_config.name);
            if url.contains(&placeholder) {
                match secrets.get_secret(&secret_config.name).await {
                    Ok(value) => {
                        url = url.replace(&placeholder, value.expose_secret());
                    }
                    Err(_) => {
                        // Secret not found — skip validation
                        print_info(&format!(
                            "Skipping validation: secret '{}' not available",
                            secret_config.name
                        ));
                        url.clear();
                        break;
                    }
                }
            }
        }

        if !url.is_empty() {
            print_info("Validating credentials...");
            let client = Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .ok();

            if let Some(client) = client {
                match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        print_success("Credentials validated successfully");
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        print_error(&format!(
                            "Credential validation failed (HTTP {}): {}",
                            status,
                            body.chars().take(200).collect::<String>()
                        ));
                        print_info(
                            "The channel will still be configured, but credentials may be invalid.",
                        );
                    }
                    Err(e) => {
                        print_info(&format!(
                            "Could not reach validation endpoint: {} (channel configured anyway)",
                            e
                        ));
                    }
                }
            }
        }
    }

    print_success(&format!("{} channel configured", channel_name));

    Ok(WasmChannelSetupResult {
        enabled: true,
        channel_name: channel_name.to_string(),
    })
}

/// Generate a random secret of specified length (in bytes).
fn generate_secret_with_length(length: usize) -> String {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut bytes = vec![0u8; length];
    rng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use crate::setup::channels::{
        TelegramOwnerCandidate, TelegramUpdate, TelegramUpdateChat, TelegramUpdateMessage,
        TelegramUpdateUser, extract_telegram_owner_capture, generate_webhook_secret,
        next_telegram_update_offset,
    };

    #[test]
    fn test_generate_webhook_secret() {
        let secret = generate_webhook_secret();
        assert_eq!(secret.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn test_generate_secret_with_length() {
        use super::generate_secret_with_length;

        let s = generate_secret_with_length(16);
        assert_eq!(s.len(), 32); // 16 bytes = 32 hex chars
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));

        let s2 = generate_secret_with_length(1);
        assert_eq!(s2.len(), 2);
    }

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
