//! Telegram-specific transport behavior for WASM channels.
//!
//! The generic WASM channel host ([`WasmChannel`]) is channel-agnostic, but a
//! handful of operations are platform-specific: Telegram needs direct Bot-API
//! calls for streaming edits, message deletion, and outbound media, plus a
//! webhook/polling health model with a persisted polling-fallback. Those
//! branches used to be interleaved into the generic host; they now live here
//! behind the [`WasmChannelTransport`] adapter trait so the core host no longer
//! hardcodes Telegram knowledge.
//!
//! Behavior is unchanged: the channel-name guards (`self.name == "telegram"`,
//! `"whatsapp"`, etc.) are preserved exactly as the original `Channel`
//! implementation expressed them.

use std::time::Duration;

use chrono::{TimeZone, Utc};

use crate::wasm::host::WorkspaceReader;
use thinclaw_channels_core::{DraftReplyState, OutgoingResponse};
use thinclaw_types::error::ChannelError;

use super::WasmChannel;

#[derive(Debug, serde::Deserialize)]
pub(super) struct TelegramWebhookInfoEnvelope {
    ok: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    result: Option<TelegramWebhookInfo>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct TelegramWebhookInfo {
    #[serde(default)]
    url: String,
    #[serde(default)]
    pending_update_count: u64,
    #[serde(default)]
    last_error_date: Option<i64>,
    #[serde(default)]
    last_error_message: Option<String>,
}

const TELEGRAM_POLLING_OVERRIDE: &str = "polling";

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct PersistedChannelRuntimeState {
    #[serde(default)]
    transport_override: Option<String>,
    #[serde(default)]
    fallback_from_webhook_url: Option<String>,
}

/// Channel-specific outbound transport behavior layered over the generic
/// WASM channel host.
///
/// The generic [`WasmChannel`] routes the `Channel` trait's transport-shaped
/// methods through this adapter. The default implementation here preserves the
/// historical per-channel `match self.name` behavior; concrete platforms
/// (currently Telegram) contribute the direct-API specializations.
#[async_trait::async_trait]
pub(super) trait WasmChannelTransport {
    /// Stream a draft reply (Telegram `sendMessage`/`editMessageText`); other
    /// channels return `None`.
    async fn transport_send_draft(
        &self,
        draft: &DraftReplyState,
        metadata: &serde_json::Value,
    ) -> Result<Option<String>, ChannelError>;

    /// Delete a previously sent message (Telegram `deleteMessage`).
    async fn transport_delete_message(
        &self,
        message_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError>;

    /// Report channel health (Telegram inspects webhook/polling diagnostics).
    async fn transport_health_check(&self) -> Result<(), ChannelError>;

    /// Optional structured diagnostics payload (Telegram only).
    async fn transport_diagnostics(&self) -> Option<serde_json::Value>;

    /// Clear persisted connection/runtime state (Telegram polling fallback).
    async fn transport_reset_connection_state(&self) -> Result<(), ChannelError>;

    /// Send outbound media attachments directly via the platform API.
    async fn transport_send_attachments(
        &self,
        chat_id: i64,
        message_thread_id: Option<i64>,
        attachments: &[thinclaw_media::MediaContent],
    );

    /// Attempt a channel-specialized broadcast; returns `Ok(true)` when the
    /// transport fully handled the broadcast (e.g. WhatsApp routed delivery),
    /// or `Ok(false)` to fall through to the generic broadcast path.
    async fn transport_try_broadcast(
        &self,
        user_id: &str,
        response: &OutgoingResponse,
    ) -> Result<bool, ChannelError>;
}

impl WasmChannel {
    pub(super) fn runtime_state_path(&self) -> std::path::PathBuf {
        thinclaw_platform::state_paths()
            .channels_dir
            .join(format!("{}.runtime.json", self.name))
    }

    pub(super) fn load_runtime_state(&self) -> PersistedChannelRuntimeState {
        let path = self.runtime_state_path();
        let Ok(content) = std::fs::read_to_string(&path) else {
            return PersistedChannelRuntimeState::default();
        };

        serde_json::from_str(&content).unwrap_or_else(|error| {
            tracing::warn!(
                channel = %self.name,
                path = %path.display(),
                error = %error,
                "Failed to parse persisted channel runtime state, ignoring"
            );
            PersistedChannelRuntimeState::default()
        })
    }

    fn save_runtime_state(
        &self,
        state: &PersistedChannelRuntimeState,
    ) -> Result<(), std::io::Error> {
        let path = self.runtime_state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let serialized = serde_json::to_vec_pretty(state)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let tmp_path = path.with_extension("runtime.json.tmp");
        std::fs::write(&tmp_path, serialized)?;
        std::fs::rename(&tmp_path, &path)?;
        Ok(())
    }

    pub(super) fn clear_runtime_state(&self) {
        let path = self.runtime_state_path();
        if let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                channel = %self.name,
                path = %path.display(),
                error = %error,
                "Failed to clear persisted channel runtime state"
            );
        }
    }

    fn telegram_webhook_url_from_tunnel_url(tunnel_url: &str) -> String {
        format!("{}/webhook/telegram", tunnel_url.trim_end_matches('/'))
    }

    fn tunnel_url_from_config(config_json: &str) -> Option<String> {
        serde_json::from_str::<serde_json::Value>(config_json)
            .ok()
            .and_then(|value| {
                value
                    .get("tunnel_url")
                    .and_then(|entry| entry.as_str())
                    .map(|value| value.trim().to_string())
            })
            .filter(|value| !value.is_empty())
    }

    pub(super) fn apply_telegram_runtime_state(
        &self,
        config_json: String,
        state: &PersistedChannelRuntimeState,
    ) -> String {
        if state.transport_override.as_deref() != Some(TELEGRAM_POLLING_OVERRIDE) {
            return config_json;
        }

        let current_webhook_url = Self::tunnel_url_from_config(&config_json)
            .map(|url| Self::telegram_webhook_url_from_tunnel_url(&url));
        if let (Some(expected_previous), Some(current)) = (
            state.fallback_from_webhook_url.as_deref(),
            current_webhook_url.as_deref(),
        ) && expected_previous != current
        {
            tracing::info!(
                channel = %self.name,
                previous = %expected_previous,
                current = %current,
                "Telegram webhook URL changed, clearing persisted polling fallback"
            );
            self.clear_runtime_state();
            return config_json;
        }

        let mut value = serde_json::from_str::<serde_json::Value>(&config_json)
            .unwrap_or_else(|_| serde_json::json!({}));
        if !value.is_object() {
            value = serde_json::json!({});
        }
        let object = value
            .as_object_mut()
            .expect("fallback configuration should be a JSON object");
        object.insert(
            "transport_override".to_string(),
            serde_json::Value::String(TELEGRAM_POLLING_OVERRIDE.to_string()),
        );
        object.insert("tunnel_url".to_string(), serde_json::Value::Null);

        serde_json::to_string(&value).unwrap_or(config_json)
    }

    fn read_workspace_state(&self, path: &str) -> Option<String> {
        self.workspace_store
            .read(&self.capabilities.prefix_workspace_path(path))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn read_workspace_state_u64(&self, path: &str) -> Option<u64> {
        self.read_workspace_state(path)?.parse::<u64>().ok()
    }

    fn iso_timestamp_from_millis(millis: Option<u64>) -> Option<String> {
        let millis = millis?;
        let millis = i64::try_from(millis).ok()?;
        Utc.timestamp_millis_opt(millis)
            .single()
            .map(|ts| ts.to_rfc3339())
    }

    fn iso_timestamp_from_seconds(seconds: Option<i64>) -> Option<String> {
        let seconds = seconds?;
        Utc.timestamp_opt(seconds, 0)
            .single()
            .map(|ts| ts.to_rfc3339())
    }

    async fn telegram_live_webhook_info(&self) -> Result<Option<TelegramWebhookInfo>, String> {
        let bot_token = self
            .credentials
            .read()
            .await
            .get("TELEGRAM_BOT_TOKEN")
            .cloned()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "Missing TELEGRAM_BOT_TOKEN".to_string())?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| format!("Failed to build Telegram client: {}", error))?;
        let response = client
            .get(format!(
                "https://api.telegram.org/bot{}/getWebhookInfo",
                bot_token
            ))
            .send()
            .await
            .map_err(|error| format!("getWebhookInfo request failed: {}", error))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("Failed to read getWebhookInfo response: {}", error))?;
        if !status.is_success() {
            return Err(format!("getWebhookInfo returned {}: {}", status, body));
        }

        let envelope: TelegramWebhookInfoEnvelope = serde_json::from_str(&body)
            .map_err(|error| format!("Failed to parse getWebhookInfo: {}", error))?;
        if !envelope.ok {
            return Err(envelope
                .description
                .unwrap_or_else(|| "Telegram webhook lookup failed".to_string()));
        }
        Ok(envelope.result)
    }

    fn telegram_polling_unhealthy_reason(
        now_ms: u64,
        last_poll_success_ms: Option<u64>,
        last_poll_started_ms: Option<u64>,
        last_poll_error: Option<&str>,
        poll_stale_after_ms: u64,
    ) -> Option<String> {
        match last_poll_success_ms {
            Some(last_success_ms)
                if now_ms.saturating_sub(last_success_ms) > poll_stale_after_ms =>
            {
                Some(match last_poll_error {
                    Some(error) if !error.trim().is_empty() => {
                        format!("polling stalled: {}", error.trim())
                    }
                    _ => "polling stalled with no recent successful poll".to_string(),
                })
            }
            None if last_poll_started_ms.is_none() => {
                Some("polling has not started yet".to_string())
            }
            None => last_poll_error
                .filter(|error| !error.trim().is_empty())
                .map(|error| format!("polling has not completed successfully: {}", error.trim())),
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn telegram_webhook_unhealthy_reason(
        now_ms: u64,
        expected_webhook_url: Option<&str>,
        registered_webhook_url: Option<&str>,
        last_webhook_register_error: Option<&str>,
        registered_webhook_error: Option<&str>,
        pending_updates: Option<u64>,
        last_webhook_register_ms: Option<u64>,
        last_inbound_ms: Option<u64>,
    ) -> Option<String> {
        if let Some(error) = last_webhook_register_error.filter(|error| !error.trim().is_empty()) {
            return Some(format!("webhook registration failed: {}", error.trim()));
        }

        if let Some(error) = registered_webhook_error.filter(|error| !error.trim().is_empty()) {
            return Some(format!("Telegram webhook error: {}", error.trim()));
        }

        match (expected_webhook_url, registered_webhook_url) {
            (Some(expected), Some(registered)) if expected != registered => {
                return Some(format!(
                    "webhook URL mismatch (expected {}, registered {})",
                    expected, registered
                ));
            }
            (Some(_), None) => {
                return Some("Telegram webhook is not registered".to_string());
            }
            (None, _) => {
                return Some("missing expected webhook URL".to_string());
            }
            _ => {}
        }

        let pending_updates = pending_updates.unwrap_or(0);
        if pending_updates == 0 {
            return None;
        }

        let pending_backlog_stale_after_ms = 90_000;
        if let Some(last_inbound_ms) = last_inbound_ms {
            if now_ms.saturating_sub(last_inbound_ms) > pending_backlog_stale_after_ms {
                return Some(format!(
                    "Telegram has {} pending webhook update(s) but inbound delivery is stalled",
                    pending_updates
                ));
            }
            return None;
        }

        let registration_grace_ms = 30_000;
        let registered_long_enough = last_webhook_register_ms
            .map(|registered_at| now_ms.saturating_sub(registered_at) > registration_grace_ms)
            .unwrap_or(true);
        if registered_long_enough {
            return Some(format!(
                "Telegram has {} pending webhook update(s) but ThinClaw has not received any inbound webhook events",
                pending_updates
            ));
        }

        None
    }

    async fn telegram_diagnostics_payload(&self) -> serde_json::Value {
        let runtime_state = self.load_runtime_state();
        let config_snapshot =
            serde_json::from_str::<serde_json::Value>(&self.config_json.read().await.clone())
                .unwrap_or_else(|_| serde_json::json!({}));
        let transport_preference = config_snapshot
            .get("transport_preference")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let transport_reason = config_snapshot
            .get("transport_reason")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let host_tunnel_url = config_snapshot
            .get("host_tunnel_url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let host_webhook_capable = config_snapshot
            .get("host_webhook_capable")
            .and_then(|value| value.as_bool());
        let host_transport_reason = config_snapshot
            .get("host_transport_reason")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let transport_mode = self
            .read_workspace_state("state/transport_mode")
            .unwrap_or_else(|| "unknown".to_string());
        let expected_webhook_url = self.read_workspace_state("state/expected_webhook_url");
        let last_webhook_register_ms =
            self.read_workspace_state_u64("state/last_webhook_register_at");
        let last_webhook_register_at = Self::iso_timestamp_from_millis(last_webhook_register_ms);
        let last_poll_started_at = Self::iso_timestamp_from_millis(
            self.read_workspace_state_u64("state/last_poll_started_at"),
        );
        let last_poll_success_at = Self::iso_timestamp_from_millis(
            self.read_workspace_state_u64("state/last_poll_success_at"),
        );
        let last_inbound_at =
            Self::iso_timestamp_from_millis(self.read_workspace_state_u64("state/last_inbound_at"));
        let last_webhook_register_error =
            self.read_workspace_state("state/last_webhook_register_error");
        let last_poll_error = self.read_workspace_state("state/last_poll_error");
        let last_transport_error = self.read_workspace_state("state/last_transport_error");
        let last_update_id = self
            .read_workspace_state("state/last_emitted_update_id")
            .and_then(|value| value.parse::<i64>().ok());

        let mut registered_webhook_url = None;
        let mut registered_webhook_error = None;
        let mut registered_webhook_error_at = None;
        let mut pending_updates = None;

        if transport_mode == "webhook" {
            match self.telegram_live_webhook_info().await {
                Ok(Some(info)) => {
                    registered_webhook_url =
                        (!info.url.trim().is_empty()).then(|| info.url.trim().to_string());
                    registered_webhook_error = info
                        .last_error_message
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty());
                    registered_webhook_error_at =
                        Self::iso_timestamp_from_seconds(info.last_error_date);
                    pending_updates = Some(info.pending_update_count);
                }
                Ok(None) => {}
                Err(error) => {
                    registered_webhook_error = Some(error);
                }
            }
        }

        let last_inbound_ms = self.read_workspace_state_u64("state/last_inbound_at");
        let now_ms = Utc::now().timestamp_millis().max(0) as u64;
        let poll_interval_ms = self
            .channel_config
            .read()
            .await
            .as_ref()
            .and_then(|config| config.poll.as_ref().map(|poll| u64::from(poll.interval_ms)))
            .unwrap_or(5_000);
        let poll_stale_after_ms = poll_interval_ms.saturating_mul(6).max(90_000);
        let last_poll_success_ms = self.read_workspace_state_u64("state/last_poll_success_at");
        let last_poll_started_ms = self.read_workspace_state_u64("state/last_poll_started_at");

        let unhealthy_reason = if self.message_tx.read().await.is_none() {
            Some("transport not started".to_string())
        } else if transport_mode == "polling" {
            Self::telegram_polling_unhealthy_reason(
                now_ms,
                last_poll_success_ms,
                last_poll_started_ms,
                last_poll_error.as_deref(),
                poll_stale_after_ms,
            )
        } else if transport_mode == "webhook" {
            Self::telegram_webhook_unhealthy_reason(
                now_ms,
                expected_webhook_url.as_deref(),
                registered_webhook_url.as_deref(),
                last_webhook_register_error.as_deref(),
                registered_webhook_error.as_deref(),
                pending_updates,
                last_webhook_register_ms,
                last_inbound_ms,
            )
        } else {
            None
        };

        serde_json::json!({
            "transport_mode": transport_mode,
            "transport_preference": transport_preference,
            "transport_reason": transport_reason,
            "transport_override": runtime_state.transport_override,
            "fallback_from_webhook_url": runtime_state.fallback_from_webhook_url,
            "host_tunnel_url": host_tunnel_url,
            "host_webhook_capable": host_webhook_capable,
            "host_transport_reason": host_transport_reason,
            "expected_webhook_url": expected_webhook_url,
            "registered_webhook_url": registered_webhook_url,
            "registered_webhook_error": registered_webhook_error,
            "registered_webhook_error_at": registered_webhook_error_at,
            "pending_update_count": pending_updates,
            "last_webhook_register_at": last_webhook_register_at,
            "last_webhook_register_error": last_webhook_register_error,
            "last_poll_started_at": last_poll_started_at,
            "last_poll_success_at": last_poll_success_at,
            "last_poll_error": last_poll_error,
            "last_inbound_at": last_inbound_at,
            "last_transport_error": last_transport_error,
            "last_update_id": last_update_id,
            "unhealthy_reason": unhealthy_reason,
        })
    }

    fn arm_telegram_polling_fallback(&self, diagnostics: &serde_json::Value) {
        if self.name != "telegram" {
            return;
        }

        let transport_mode = diagnostics
            .get("transport_mode")
            .and_then(|value| value.as_str())
            .map(str::trim);
        let unhealthy_reason = diagnostics
            .get("unhealthy_reason")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if transport_mode != Some("webhook") || unhealthy_reason.is_none() {
            return;
        }

        let mut state = self.load_runtime_state();
        if state.transport_override.as_deref() == Some(TELEGRAM_POLLING_OVERRIDE) {
            return;
        }

        state.transport_override = Some(TELEGRAM_POLLING_OVERRIDE.to_string());
        state.fallback_from_webhook_url = diagnostics
            .get("expected_webhook_url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        match self.save_runtime_state(&state) {
            Ok(()) => {
                tracing::warn!(
                    channel = %self.name,
                    reason = %unhealthy_reason.unwrap_or("unknown"),
                    expected_webhook_url = ?state.fallback_from_webhook_url,
                    "Telegram webhook unhealthy; forcing polling fallback on next restart"
                );
            }
            Err(error) => {
                tracing::error!(
                    channel = %self.name,
                    error = %error,
                    "Failed to persist Telegram polling fallback state"
                );
            }
        }
    }

    // ── Telegram outbound media attachments ─────────────────────────

    /// Send outbound media attachments to a Telegram chat.
    ///
    /// Uses the Telegram Bot API directly (bypassing the WASM sandbox),
    /// matching the pattern used by `send_draft` and `delete_message`.
    /// Each attachment is sent via the appropriate endpoint based on media
    /// type: `sendPhoto` for images, `sendAudio` for audio, `sendVideo`
    /// for video, and `sendDocument` for everything else.
    ///
    /// Failures are logged but do not abort the response (best-effort).
    pub(super) async fn send_telegram_attachments(
        &self,
        chat_id: i64,
        message_thread_id: Option<i64>,
        attachments: &[thinclaw_media::MediaContent],
    ) {
        if self.name != "telegram" || attachments.is_empty() {
            return;
        }

        // Get bot token from credentials
        let creds = self.credentials.read().await;
        let token = match creds.get("TELEGRAM_BOT_TOKEN").cloned() {
            Some(t) => t,
            None => {
                tracing::debug!("send_telegram_attachments: no TELEGRAM_BOT_TOKEN, skipping");
                return;
            }
        };
        drop(creds);

        let client = reqwest::Client::new();

        for attachment in attachments {
            use thinclaw_media::MediaType;

            // Pick the right Telegram API endpoint based on media type
            let (api_method, field_name) = match attachment.media_type {
                MediaType::Image => ("sendPhoto", "photo"),
                MediaType::Audio => ("sendAudio", "audio"),
                MediaType::Video => ("sendVideo", "video"),
                // PDFs, documents, unknown — all go through sendDocument
                _ => ("sendDocument", "document"),
            };

            let url = format!("https://api.telegram.org/bot{}/{}", token, api_method);

            let filename = attachment
                .filename
                .as_deref()
                .unwrap_or("attachment")
                .to_string();

            let file_part = match reqwest::multipart::Part::bytes(attachment.data.clone())
                .file_name(filename.clone())
                .mime_str(&attachment.mime_type)
            {
                Ok(part) => part,
                Err(e) => {
                    tracing::warn!(
                        channel = %self.name,
                        error = %e,
                        mime = %attachment.mime_type,
                        "Telegram: invalid MIME for attachment, skipping"
                    );
                    continue;
                }
            };

            let mut form = reqwest::multipart::Form::new()
                .text("chat_id", chat_id.to_string())
                .part(field_name, file_part);

            if let Some(thread_id) = message_thread_id {
                form = form.text("message_thread_id", thread_id.to_string());
            }

            match client
                .post(&url)
                .multipart(form)
                .timeout(Duration::from_secs(120))
                .send()
                .await
            {
                Ok(resp) => {
                    if resp.status().is_success() {
                        tracing::info!(
                            channel = %self.name,
                            chat_id = chat_id,
                            method = api_method,
                            filename = %filename,
                            size = attachment.data.len(),
                            "Telegram: attachment sent successfully"
                        );
                    } else {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        tracing::warn!(
                            channel = %self.name,
                            chat_id = chat_id,
                            method = api_method,
                            status = %status,
                            body = %body,
                            "Telegram: attachment send returned error"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %self.name,
                        chat_id = chat_id,
                        method = api_method,
                        error = %e,
                        "Telegram: attachment HTTP request failed"
                    );
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl WasmChannelTransport for WasmChannel {
    async fn transport_send_draft(
        &self,
        draft: &DraftReplyState,
        metadata: &serde_json::Value,
    ) -> Result<Option<String>, ChannelError> {
        // Only Telegram channels support streaming via edit
        if self.name != "telegram" {
            return Ok(None);
        }

        // Extract chat_id and optional message_thread_id from metadata
        let chat_id = metadata.get("chat_id").and_then(|v| v.as_i64());
        let message_thread_id = metadata.get("message_thread_id").and_then(|v| v.as_i64());

        let Some(chat_id) = chat_id else {
            tracing::debug!("send_draft: no chat_id in metadata, skipping");
            return Ok(None);
        };

        // Get bot token from credentials
        let creds = self.credentials.read().await;
        let token = creds.get("TELEGRAM_BOT_TOKEN").cloned();
        drop(creds);

        let Some(token) = token else {
            tracing::debug!("send_draft: no TELEGRAM_BOT_TOKEN in credentials, skipping");
            return Ok(None);
        };

        let client = reqwest::Client::new();

        // Strategy: sendMessage (first call) → editMessageText (subsequent)
        // This is the standard, reliable approach for streaming in Telegram.
        // sendMessageDraft is unreliable (RANDOM_ID_INVALID errors).
        if !draft.posted {
            // ── First chunk: send a new message ──────────────────────────
            let html = crate::wasm::telegram_html::markdown_to_telegram_html(&draft.accumulated);
            let mut payload = serde_json::json!({
                "chat_id": chat_id,
                "text": html,
                "parse_mode": "HTML",
            });

            if let Some(thread_id) = message_thread_id {
                payload["message_thread_id"] = serde_json::json!(thread_id);
            }

            let url = format!("https://api.telegram.org/bot{}/sendMessage", token);

            match client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&payload)
                .timeout(Duration::from_secs(10))
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    if !status.is_success() {
                        tracing::warn!(
                            channel = %self.name,
                            status = %status,
                            body = %body,
                            "send_draft: initial sendMessage failed"
                        );
                        return Ok(None);
                    }

                    // Extract message_id from the Telegram response
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body)
                        && let Some(msg_id) = parsed
                            .get("result")
                            .and_then(|r| r.get("message_id"))
                            .and_then(|v| v.as_i64())
                    {
                        tracing::debug!(
                            channel = %self.name,
                            chat_id = chat_id,
                            message_id = msg_id,
                            thread_id = ?message_thread_id,
                            text_len = draft.accumulated.len(),
                            "send_draft: initial message sent"
                        );
                        // Return the message_id as string so DraftReplyState can
                        // track it for subsequent editMessageText calls
                        return Ok(Some(msg_id.to_string()));
                    }
                    tracing::warn!(
                        "send_draft: could not extract message_id from sendMessage response"
                    );
                    Ok(None)
                }
                Err(e) => {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "send_draft: sendMessage HTTP request failed (non-fatal)"
                    );
                    Ok(None)
                }
            }
        } else {
            // ── Subsequent chunks: edit the existing message ─────────────
            let Some(ref msg_id_str) = draft.message_id else {
                // No message_id to edit — skip
                return Ok(None);
            };

            let msg_id: i64 = match msg_id_str.parse() {
                Ok(id) => id,
                Err(_) => return Ok(None),
            };

            let html = crate::wasm::telegram_html::markdown_to_telegram_html(&draft.accumulated);

            // Telegram's max message length is 4096 chars. Use a lower
            // threshold (3800) to account for HTML tag expansion and
            // avoid edge-case truncation. When exceeded, signal overflow
            // so the dispatcher falls back to on_respond() which splits.
            const TELEGRAM_MAX_SAFE_EDIT_LENGTH: usize = 3800;
            if html.len() > TELEGRAM_MAX_SAFE_EDIT_LENGTH {
                tracing::info!(
                    channel = %self.name,
                    html_len = html.len(),
                    "send_draft: response exceeds Telegram limit, signaling overflow"
                );
                return Err(ChannelError::MessageTooLong {
                    channel: self.name.clone(),
                    length: html.len(),
                    max: TELEGRAM_MAX_SAFE_EDIT_LENGTH,
                });
            }

            let payload = serde_json::json!({
                "chat_id": chat_id,
                "message_id": msg_id,
                "text": html,
                "parse_mode": "HTML",
            });

            let url = format!("https://api.telegram.org/bot{}/editMessageText", token);

            match client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&payload)
                .timeout(Duration::from_secs(10))
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        let body = resp.text().await.unwrap_or_default();
                        // 400 "message is not modified" is expected when text hasn't
                        // changed enough — treat as non-fatal
                        if body.contains("message is not modified") {
                            return Ok(Some(msg_id_str.clone()));
                        }
                        tracing::debug!(
                            channel = %self.name,
                            status = %status,
                            body = %body,
                            "send_draft: editMessageText failed (non-fatal)"
                        );
                        return Ok(Some(msg_id_str.clone()));
                    }
                    tracing::trace!(
                        channel = %self.name,
                        chat_id = chat_id,
                        message_id = msg_id,
                        text_len = draft.accumulated.len(),
                        "send_draft: message edited"
                    );
                    Ok(Some(msg_id_str.clone()))
                }
                Err(e) => {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "send_draft: editMessageText HTTP request failed (non-fatal)"
                    );
                    Ok(Some(msg_id_str.clone()))
                }
            }
        }
    }

    async fn transport_delete_message(
        &self,
        message_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Only Telegram channels support message deletion in this context
        if !self.name.starts_with("telegram") {
            return Ok(());
        }

        // Get bot token from credentials (same pattern as send_draft)
        let creds = self.credentials.read().await;
        let token = creds.get("TELEGRAM_BOT_TOKEN").cloned();
        drop(creds);

        let Some(token) = token else {
            return Ok(());
        };

        // Extract chat_id from metadata (same pattern as send_draft)
        let chat_id = metadata.get("chat_id").and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        });

        let Some(chat_id) = chat_id else {
            return Ok(());
        };

        let msg_id: i64 = match message_id.parse() {
            Ok(id) => id,
            Err(_) => return Ok(()),
        };

        let client = reqwest::Client::new();
        let url = format!("https://api.telegram.org/bot{}/deleteMessage", token);
        let payload = serde_json::json!({
            "chat_id": chat_id,
            "message_id": msg_id,
        });

        match client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) => {
                if resp.status().is_success() {
                    tracing::debug!(
                        channel = %self.name,
                        message_id = msg_id,
                        "delete_message: message deleted successfully"
                    );
                } else {
                    let body = resp.text().await.unwrap_or_default();
                    tracing::debug!(
                        channel = %self.name,
                        message_id = msg_id,
                        body = %body,
                        "delete_message: deleteMessage API failed (non-fatal)"
                    );
                }
            }
            Err(e) => {
                tracing::debug!(
                    channel = %self.name,
                    error = %e,
                    "delete_message: HTTP request failed (non-fatal)"
                );
            }
        }

        Ok(())
    }

    async fn transport_health_check(&self) -> Result<(), ChannelError> {
        if self.name == "telegram" {
            let diagnostics = self.telegram_diagnostics_payload().await;
            self.arm_telegram_polling_fallback(&diagnostics);
            if diagnostics
                .get("unhealthy_reason")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some()
            {
                return Err(ChannelError::HealthCheckFailed {
                    name: self.name.clone(),
                });
            }
            return Ok(());
        }

        if self.message_tx.read().await.is_some() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: self.name.clone(),
            })
        }
    }

    async fn transport_diagnostics(&self) -> Option<serde_json::Value> {
        if self.name == "telegram" {
            Some(self.telegram_diagnostics_payload().await)
        } else {
            None
        }
    }

    async fn transport_reset_connection_state(&self) -> Result<(), ChannelError> {
        if self.name == "telegram" {
            self.clear_runtime_state();
        }
        Ok(())
    }

    async fn transport_send_attachments(
        &self,
        chat_id: i64,
        message_thread_id: Option<i64>,
        attachments: &[thinclaw_media::MediaContent],
    ) {
        self.send_telegram_attachments(chat_id, message_thread_id, attachments)
            .await;
    }

    async fn transport_try_broadcast(
        &self,
        user_id: &str,
        response: &OutgoingResponse,
    ) -> Result<bool, ChannelError> {
        if self.name == "whatsapp" {
            let metadata =
                super::conversions::merged_response_metadata(&serde_json::Value::Null, response);
            let has_route = metadata
                .get("phone_number_id")
                .and_then(|value| value.as_str())
                .is_some()
                && metadata
                    .get("recipient_phone")
                    .and_then(|value| value.as_str())
                    .is_some();

            if has_route {
                let metadata_json = serde_json::to_string(&metadata).unwrap_or_default();
                self.call_on_respond(
                    uuid::Uuid::new_v4(),
                    &response.content,
                    response.thread_id.as_deref(),
                    &metadata_json,
                )
                .await
                .map_err(|e| ChannelError::SendFailed {
                    name: self.name.clone(),
                    reason: format!("broadcast via on_respond: {}", e),
                })?;
                return Ok(true);
            }

            tracing::warn!(
                channel = %self.name,
                user_id = %user_id,
                "WASM broadcast: WhatsApp requires explicit route metadata"
            );
            return Ok(true);
        }

        Ok(false)
    }
}
