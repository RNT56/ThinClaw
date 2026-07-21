//! The `SignalChannel` runtime: JSON-RPC client, auth/pairing, the `Channel`
//! impl, and the reconnecting SSE listener.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use lru::LruCache;
use reqwest::Client;
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::attachments::{
    cleanup_signal_temp_attachments, collect_signal_attachments, write_signal_temp_attachments,
};
use super::*;
use crate::pairing::PairingStore;
use thinclaw_channels_core::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};
use thinclaw_types::error::ChannelError;

/// Signal channel using signal-cli daemon's native JSON-RPC + SSE API.
pub struct SignalChannel {
    config: SignalConfig,
    client: Client,
    /// LRU cache of reply targets per incoming message, used by `respond()`.
    /// Bounded to `MAX_REPLY_TARGETS` entries; least-recently-used entries
    /// are evicted automatically when the cache is full.
    reply_targets: Arc<RwLock<LruCache<Uuid, String>>>,
    /// Debug mode for verbose tool output (toggled via /debug command).
    debug_mode: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
    sse_task: Mutex<Option<JoinHandle<()>>>,
}

const CHANNEL_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

impl SignalChannel {
    /// Create a new Signal channel with normalized config and fresh client/cache.
    pub fn new(config: SignalConfig) -> Result<Self, ChannelError> {
        let config = Self::validated_config(config)?;
        let client = Self::base_http_client()?;
        Ok(Self::with_client(config, client))
    }

    /// Create the production Signal channel with DNS validation and pinning.
    pub async fn new_pinned(config: SignalConfig) -> Result<Self, ChannelError> {
        let config = Self::validated_config(config)?;
        let client = Self::pinned_http_client(&config.http_url).await?;
        Ok(Self::with_client(config, client))
    }

    fn with_client(config: SignalConfig, client: Client) -> Self {
        let cap = REPLY_TARGETS_CAP;
        let reply_targets = Arc::new(RwLock::new(LruCache::new(cap)));
        let debug_mode = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_notify = Arc::new(Notify::new());

        Self::from_parts(
            config,
            client,
            reply_targets,
            debug_mode,
            shutdown,
            shutdown_notify,
        )
    }

    fn base_http_client() -> Result<Client, ChannelError> {
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|e| ChannelError::Http(e.to_string()))
    }

    fn validated_config(mut config: SignalConfig) -> Result<SignalConfig, ChannelError> {
        config.http_url = config.http_url.trim_end_matches('/').to_string();
        let parsed = reqwest::Url::parse(&config.http_url)
            .map_err(|_| ChannelError::Configuration("Signal HTTP URL is malformed".to_string()))?;
        let host = parsed.host_str().ok_or_else(|| {
            ChannelError::Configuration("Signal HTTP URL requires a host".to_string())
        })?;
        let local_http_host = host.eq_ignore_ascii_case("localhost")
            || host.to_ascii_lowercase().ends_with(".local")
            || host.to_ascii_lowercase().ends_with(".internal")
            || !host.contains('.')
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| !thinclaw_tools_core::is_public_outbound_ip(ip));
        let lists = [
            &config.allow_from,
            &config.allow_from_groups,
            &config.group_allow_from,
        ];
        if config.http_url.is_empty()
            || config.http_url.len() > MAX_SIGNAL_ENDPOINT_BYTES
            || !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.scheme() == "http" && !local_http_host
            || config.account.is_empty()
            || config.account.len() > 256
            || config.account.chars().any(char::is_control)
            || !matches!(config.dm_policy.as_str(), "open" | "pairing" | "allowlist")
            || !matches!(
                config.group_policy.as_str(),
                "disabled" | "open" | "allowlist"
            )
            || lists.iter().any(|list| {
                list.len() > MAX_SIGNAL_CONFIG_ENTRIES
                    || list.iter().any(|value| {
                        value.is_empty()
                            || value.len() > MAX_SIGNAL_CONFIG_VALUE_BYTES
                            || value.chars().any(char::is_control)
                    })
            })
        {
            return Err(ChannelError::Configuration(
                "Signal channel configuration is malformed or oversized".to_string(),
            ));
        }
        Ok(config)
    }

    async fn pinned_http_client(http_url: &str) -> Result<Client, ChannelError> {
        let parsed = reqwest::Url::parse(http_url)
            .map_err(|_| ChannelError::Configuration("Signal HTTP URL is malformed".to_string()))?;
        let host = parsed.host_str().ok_or_else(|| {
            ChannelError::Configuration("Signal HTTP URL requires a host".to_string())
        })?;
        let port = parsed.port_or_known_default().ok_or_else(|| {
            ChannelError::Configuration("Signal HTTP URL has no port".to_string())
        })?;
        let resolved = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| {
            ChannelError::Configuration("Signal endpoint DNS lookup timed out".to_string())
        })?
        .map_err(|_| {
            ChannelError::Configuration("Signal endpoint DNS lookup failed".to_string())
        })?;
        let mut addresses = resolved.collect::<Vec<_>>();
        addresses.sort_unstable();
        addresses.dedup();
        if addresses.is_empty()
            || addresses.len() > MAX_SIGNAL_DNS_ADDRESSES
            || addresses.iter().any(|address| {
                let ip = address.ip();
                ip.is_unspecified()
                    || ip.is_multicast()
                    || matches!(ip, std::net::IpAddr::V4(ip) if ip.is_broadcast())
                    || parsed.scheme() == "http" && thinclaw_tools_core::is_public_outbound_ip(ip)
            })
        {
            return Err(ChannelError::Configuration(
                "Signal endpoint resolved to an invalid address".to_string(),
            ));
        }
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|e| ChannelError::Http(e.to_string()))
    }

    /// Construct a SignalChannel from pre-validated parts.
    ///
    /// Used by [`new()`][Self::new] after normalization and by [`sse_listener`]
    /// to ensure both code paths use the same constructor.
    fn from_parts(
        config: SignalConfig,
        client: Client,
        reply_targets: Arc<RwLock<LruCache<Uuid, String>>>,
        debug_mode: Arc<AtomicBool>,
        shutdown: Arc<AtomicBool>,
        shutdown_notify: Arc<Notify>,
    ) -> Self {
        Self {
            config,
            client,
            reply_targets,
            debug_mode,
            shutdown,
            shutdown_notify,
            sse_task: Mutex::new(None),
        }
    }

    fn is_debug(&self) -> bool {
        self.debug_mode.load(Ordering::Relaxed)
    }

    fn toggle_debug(&self) -> bool {
        let current = self.debug_mode.load(Ordering::Relaxed);
        self.debug_mode.store(!current, Ordering::Relaxed);
        !current
    }

    /// Effective sender: prefer `sourceNumber` (E.164), fall back to `source`
    /// (UUID for privacy-enabled users).
    fn sender(envelope: &Envelope) -> Option<String> {
        envelope
            .source_number
            .as_deref()
            .or(envelope.source.as_deref())
            .map(String::from)
    }

    fn stable_sender_id(envelope: &Envelope, sender: &str) -> String {
        envelope
            .source_uuid
            .as_deref()
            .unwrap_or(sender)
            .to_string()
    }

    fn conversation_kind(is_group: bool) -> &'static str {
        if is_group { "group" } else { "direct" }
    }

    fn conversation_scope_id(
        is_group: bool,
        sender: &str,
        stable_sender_id: &str,
        group_id: Option<&str>,
    ) -> String {
        if is_group {
            format!("signal:group:{}", group_id.unwrap_or(sender))
        } else {
            format!("signal:direct:{stable_sender_id}")
        }
    }

    fn external_conversation_key(
        is_group: bool,
        sender: &str,
        stable_sender_id: &str,
        group_id: Option<&str>,
    ) -> String {
        if is_group {
            format!("signal://group/{}", group_id.unwrap_or(sender))
        } else {
            format!("signal://direct/{stable_sender_id}")
        }
    }

    /// Normalize an allowlist entry to the bare identifier.
    ///
    /// Strips the `uuid:` prefix if present, so `uuid:<id>` and `<id>` both
    /// match against a bare UUID sender.
    fn normalize_allow_entry(entry: &str) -> &str {
        entry.strip_prefix("uuid:").unwrap_or(entry)
    }

    /// Check whether a sender is in the allowed users list.
    ///
    /// Returns `false` if the sender is on the blocklist (blocklist takes precedence).
    fn is_sender_allowed(&self, sender: &str) -> bool {
        // Check blocklist first (takes precedence)
        let store = PairingStore::new();
        if store
            .is_sender_blocked("signal", sender, None)
            .unwrap_or(false)
        {
            tracing::debug!(sender = %sender, "Signal: sender is blocked, rejecting");
            return false;
        }
        if self.config.allow_from.is_empty() {
            return false;
        }
        self.config.allow_from.iter().any(|entry| {
            entry == "*"
                || Self::normalize_allow_entry(entry) == Self::normalize_allow_entry(sender)
        })
    }

    /// Check if sender is allowed via config allow_from OR pairing store.
    ///
    /// Returns `false` if the sender is on the blocklist (blocklist takes precedence).
    fn is_sender_allowed_with_pairing(&self, sender: &str) -> bool {
        if self.is_sender_allowed(sender) {
            return true;
        }
        let store = PairingStore::new();
        // Blocklist already checked in is_sender_allowed above
        if let Ok(allowed) = store.read_allow_from("signal") {
            return allowed.iter().any(|entry| entry == "*" || entry == sender);
        }
        false
    }

    /// Handle pairing request for unapproved sender.
    /// Returns Ok(true) if message should be allowed (was already paired),
    /// Ok(false) if message was blocked but pairing request was processed.
    fn handle_pairing_request(
        &self,
        sender: &str,
        source_name: Option<&str>,
        stable_sender_id: &str,
        conversation_kind: &str,
        conversation_scope_id: &str,
        external_conversation_key: &str,
    ) -> Result<bool, ()> {
        let store = PairingStore::new();
        let meta = serde_json::json!({
            "sender": sender,
            "name": source_name,
            "raw_sender_id": sender,
            "stable_sender_id": stable_sender_id,
            "conversation_kind": conversation_kind,
            "conversation_scope_id": conversation_scope_id,
            "external_conversation_key": external_conversation_key,
        });

        match store.upsert_request("signal", sender, Some(meta)) {
            Ok(result) => {
                tracing::info!(
                    sender = %sender,
                    code = %result.code,
                    "Signal: pairing request upserted"
                );
                if result.created {
                    let message = format!(
                        "To pair with this bot, run: `thinclaw pairing approve signal {}`. \
                         For a new family member, you can add `--name \"Alex\"` to create and link an actor.",
                        result.code
                    );
                    let http_url = self.config.http_url.clone();
                    let account = self.config.account.clone();
                    let client = self.client.clone();
                    let sender_owned = sender.to_string();
                    let message_owned = message.clone();
                    tokio::spawn(async move {
                        if let Err(e) = Self::send_pairing_reply_async(
                            &client,
                            &http_url,
                            &account,
                            &sender_owned,
                            &message_owned,
                        )
                        .await
                        {
                            tracing::error!(sender = %sender_owned, error = %e, "Signal: failed to send pairing reply");
                        }
                    });
                }
                Ok(false)
            }
            Err(e) => {
                tracing::error!(sender = %sender, error = %e, "Signal: pairing upsert failed");
                Err(())
            }
        }
    }

    /// Send a pairing reply message to the sender (async helper for spawned task).
    async fn send_pairing_reply_async(
        client: &Client,
        http_url: &str,
        account: &str,
        recipient: &str,
        message: &str,
    ) -> Result<(), ChannelError> {
        let target = Self::parse_recipient_target(recipient);
        let params = Self::build_rpc_params_static(http_url, account, &target, Some(message));

        let url = format!("{}/api/v1/rpc", http_url);
        let id = Uuid::new_v4().to_string();

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "send",
            "params": params,
            "id": id,
        });

        let resp = client
            .post(&url)
            .timeout(Duration::from_secs(30))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!(
                    "RPC request failed to {}: {}",
                    Self::redact_url(&url),
                    e.without_url()
                ),
            })?;

        let status = resp.status();
        let is_success = status.is_success();

        if status.as_u16() == 201 {
            return Ok(());
        }

        if !is_success {
            return Err(ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("Signal RPC returned HTTP {}", status.as_u16()),
            });
        }

        Ok(())
    }

    /// Get effective group allow_from list (inherits from allow_from if empty).
    fn effective_group_allow_from(&self) -> &[String] {
        if self.config.group_allow_from.is_empty() {
            &self.config.allow_from
        } else {
            &self.config.group_allow_from
        }
    }

    /// Check whether a group is in the allowed groups list.
    ///
    /// - Empty list — deny all groups (DMs only, secure by default).
    /// - `*` — allow all groups.
    /// - Specific IDs — allow only those groups.
    fn is_group_allowed(&self, group_id: &str) -> bool {
        if self.config.allow_from_groups.is_empty() {
            return false;
        }
        self.config
            .allow_from_groups
            .iter()
            .any(|entry| entry == "*" || entry == group_id)
    }

    /// Check whether a sender is allowed for group messages.
    fn is_group_sender_allowed(&self, sender: &str) -> bool {
        let effective_list = self.effective_group_allow_from();
        if effective_list.is_empty() {
            return false;
        }
        effective_list.iter().any(|entry| {
            entry == "*"
                || Self::normalize_allow_entry(entry) == Self::normalize_allow_entry(sender)
        })
    }

    /// Redact credentials from a URL for safe logging.
    ///
    /// Replaces any embedded username/password with `**REDACTED**` and returns
    /// the sanitised string. Returns `"<invalid-url>"` when parsing fails.
    pub fn redact_url(url: &str) -> String {
        reqwest::Url::parse(url)
            .map(|mut u| {
                if u.password().is_some() || !u.username().is_empty() {
                    let _ = u.set_username("**REDACTED**");
                    let _ = u.set_password(None);
                }
                u.to_string()
            })
            .unwrap_or_else(|_| "<invalid-url>".to_string())
    }

    fn is_e164(recipient: &str) -> bool {
        let Some(number) = recipient.strip_prefix('+') else {
            return false;
        };
        (7..=15).contains(&number.len()) && number.chars().all(|c| c.is_ascii_digit())
    }

    /// Check whether a string is a valid UUID (signal-cli uses these for
    /// privacy-enabled users who have opted out of sharing their phone number).
    fn is_uuid(s: &str) -> bool {
        Uuid::parse_str(s).is_ok()
    }

    /// Generate a deterministic UUID from an identifier (phone number or group ID).
    ///
    /// This ensures that the same phone number or group always produces the same UUID,
    /// allowing conversation history to persist across gateway restarts.
    fn thread_id_from_identifier(identifier: &str) -> String {
        // Use a stable, deterministic UUID v5 derived from the identifier.
        // This avoids relying on `DefaultHasher` implementation details and
        // provides a full 128 bits of entropy.
        Uuid::new_v5(&Uuid::NAMESPACE_URL, identifier.as_bytes()).to_string()
    }

    fn parse_recipient_target(recipient: &str) -> RecipientTarget {
        if let Some(group_id) = recipient.strip_prefix(GROUP_TARGET_PREFIX) {
            return RecipientTarget::Group(group_id.to_string());
        }

        if Self::is_e164(recipient) || Self::is_uuid(recipient) {
            RecipientTarget::Direct(recipient.to_string())
        } else {
            RecipientTarget::Group(recipient.to_string())
        }
    }

    /// Determine the reply target: group id (prefixed) or the sender's identifier.
    fn reply_target(data_msg: &DataMessage, sender: &str) -> String {
        if let Some(group_id) = data_msg
            .group_info
            .as_ref()
            .and_then(|g| g.group_id.as_deref())
        {
            format!("{GROUP_TARGET_PREFIX}{group_id}")
        } else {
            sender.to_string()
        }
    }

    /// Send a JSON-RPC request to signal-cli daemon.
    async fn rpc_request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Option<serde_json::Value>, ChannelError> {
        let url = format!("{}/api/v1/rpc", self.config.http_url);
        let id = Uuid::new_v4().to_string();

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });

        let resp = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(30))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!(
                    "RPC request failed to {}: {}",
                    Self::redact_url(&url),
                    e.without_url()
                ),
            })?;

        // 201 = success with no body (e.g. typing indicators).
        if resp.status().as_u16() == 201 {
            return Ok(None);
        }

        // Reject obviously oversized responses before buffering.
        if let Some(len) = resp.content_length()
            && len as usize > MAX_HTTP_RESPONSE_SIZE
        {
            return Err(ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!(
                    "RPC response Content-Length too large: {} bytes (max {})",
                    len, MAX_HTTP_RESPONSE_SIZE
                ),
            });
        }

        let status = resp.status();
        let mut stream = resp.bytes_stream();
        let mut total_bytes = 0usize;
        let mut body = Vec::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("Failed to read RPC response: {e}"),
            })?;
            let chunk_len = chunk.len();
            total_bytes =
                total_bytes
                    .checked_add(chunk_len)
                    .ok_or_else(|| ChannelError::SendFailed {
                        name: "signal".to_string(),
                        reason: "RPC response size overflow".to_string(),
                    })?;

            if total_bytes > MAX_HTTP_RESPONSE_SIZE {
                return Err(ChannelError::SendFailed {
                    name: "signal".to_string(),
                    reason: format!(
                        "RPC response too large: {} bytes (max {})",
                        total_bytes, MAX_HTTP_RESPONSE_SIZE
                    ),
                });
            }

            body.extend_from_slice(&chunk);
        }

        let bytes = body;

        if bytes.is_empty() {
            return Ok(None);
        }

        // Check for non-success HTTP status codes before parsing as JSON.
        if !status.is_success() {
            return Err(ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("Signal RPC returned HTTP {}", status.as_u16()),
            });
        }

        let parsed: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("Invalid RPC response JSON: {e}"),
            })?;

        if let Some(err) = parsed.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            let msg = msg
                .chars()
                .filter(|character| !character.is_control())
                .take(1024)
                .collect::<String>();
            return Err(ChannelError::SendFailed {
                name: "signal".to_string(),
                reason: format!("Signal RPC error {code}: {msg}"),
            });
        }

        Ok(parsed.get("result").cloned())
    }

    /// Build JSON-RPC params for a send/typing call.
    fn build_rpc_params(
        &self,
        target: &RecipientTarget,
        message: Option<&str>,
    ) -> serde_json::Value {
        match target {
            RecipientTarget::Direct(id) => {
                let mut params = serde_json::json!({
                    "recipient": [id],
                    "account": &self.config.account,
                });
                if let Some(msg) = message {
                    params["message"] = serde_json::Value::String(msg.to_string());
                }
                params
            }
            RecipientTarget::Group(group_id) => {
                let mut params = serde_json::json!({
                    "groupId": group_id,
                    "account": &self.config.account,
                });
                if let Some(msg) = message {
                    params["message"] = serde_json::Value::String(msg.to_string());
                }
                params
            }
        }
    }

    fn add_attachment_params(params: &mut serde_json::Value, paths: &[std::path::PathBuf]) {
        if !paths.is_empty() {
            params["attachments"] = serde_json::Value::Array(
                paths
                    .iter()
                    .map(|path| serde_json::Value::String(path.to_string_lossy().to_string()))
                    .collect(),
            );
        }
    }

    /// Build JSON-RPC params for a send/typing call (static version).
    fn build_rpc_params_static(
        _http_url: &str,
        account: &str,
        target: &RecipientTarget,
        message: Option<&str>,
    ) -> serde_json::Value {
        match target {
            RecipientTarget::Direct(id) => {
                let mut params = serde_json::json!({
                    "recipient": [id],
                    "account": account,
                });
                if let Some(msg) = message {
                    params["message"] = serde_json::Value::String(msg.to_string());
                }
                params
            }
            RecipientTarget::Group(group_id) => {
                let mut params = serde_json::json!({
                    "groupId": group_id,
                    "account": account,
                });
                if let Some(msg) = message {
                    params["message"] = serde_json::Value::String(msg.to_string());
                }
                params
            }
        }
    }

    /// Process a single SSE envelope, returning an `IncomingMessage` if valid.
    fn process_envelope(&self, envelope: &Envelope) -> Option<(IncomingMessage, String)> {
        // Skip story messages when configured.
        if self.config.ignore_stories && envelope.story_message.is_some() {
            tracing::debug!("Signal: dropping story message");
            return None;
        }

        let data_msg = envelope.data_message.as_ref()?;

        // Skip attachment-only messages when configured.
        let has_attachments = data_msg.attachments.as_ref().is_some_and(|a| !a.is_empty());
        let has_message_text = data_msg.message.as_ref().is_some_and(|m| !m.is_empty());
        if self.config.ignore_attachments && has_attachments && !has_message_text {
            tracing::debug!("Signal: dropping attachment-only message");
            return None;
        }

        // Collect media attachments from signal-cli's local file store
        let media_attachments = if has_attachments && !self.config.ignore_attachments {
            collect_signal_attachments(data_msg.attachments.as_deref().unwrap_or_default())
        } else {
            Vec::new()
        };

        // Use message text, or fall back to a media prompt for attachment-only messages
        let text = data_msg
            .message
            .as_deref()
            .filter(|t| !t.is_empty())
            .map(String::from)
            .or_else(|| {
                if has_attachments {
                    Some("[Media received — please analyze the attached content]".to_string())
                } else {
                    None
                }
            })?;
        let sender = Self::sender(envelope)?;

        // Log sender info including UUID if available
        tracing::debug!(
            sender = %sender,
            uuid = ?envelope.source_uuid,
            "Signal: received message"
        );

        let group_id = data_msg
            .group_info
            .as_ref()
            .and_then(|g| g.group_id.as_deref());
        let is_group = group_id.is_some();
        let stable_sender_id = Self::stable_sender_id(envelope, &sender);
        let conversation_kind = Self::conversation_kind(is_group);
        let conversation_scope_id =
            Self::conversation_scope_id(is_group, &sender, &stable_sender_id, group_id);
        let external_conversation_key =
            Self::external_conversation_key(is_group, &sender, &stable_sender_id, group_id);

        // Apply group policy first (before DM policy for group messages)
        if is_group {
            match self.config.group_policy.as_str() {
                "disabled" => {
                    tracing::debug!("Signal: group messages disabled, dropping");
                    return None;
                }
                "open" => {
                    // For "open" policy, check group allowlist but not sender allowlist
                    if let Some(group_id) = group_id
                        && !self.is_group_allowed(group_id)
                    {
                        tracing::debug!(
                            group_id = %group_id,
                            "Signal: group not in allow_from_groups, dropping"
                        );
                        return None;
                    }
                }
                "allowlist" => {
                    // Default to allowlist - check group AND sender
                    if let Some(group_id) = group_id {
                        if !self.is_group_allowed(group_id) {
                            tracing::debug!(
                                group_id = %group_id,
                                "Signal: group not in allow_from_groups, dropping"
                            );
                            return None;
                        }
                        // Also check sender is allowed for group
                        if !self.is_group_sender_allowed(&sender) {
                            tracing::debug!(
                                sender = %sender,
                                group_id = %group_id,
                                "Signal: sender not in group_allow_from, dropping"
                            );
                            return None;
                        }
                    }
                }
                _ => {
                    tracing::warn!("Signal: unknown group policy, dropping message");
                    return None;
                }
            }
        } else {
            // DM message - apply DM policy
            match self.config.dm_policy.as_str() {
                "open" => {}
                "pairing" => {
                    // Pairing policy: check allow_from + pairing store.
                    if !self.is_sender_allowed_with_pairing(&sender) {
                        // Handle pairing request - this will create a request and send reply if new
                        match self.handle_pairing_request(
                            &sender,
                            envelope.source_name.as_deref(),
                            &stable_sender_id,
                            conversation_kind,
                            &conversation_scope_id,
                            &external_conversation_key,
                        ) {
                            Ok(_) => {
                                // Pairing request processed (new or existing), drop the message
                                return None;
                            }
                            Err(()) => {
                                // Error processing pairing, drop message
                                return None;
                            }
                        }
                    }
                }
                "allowlist" => {
                    // Default: check allow_from list.
                    if !self.is_sender_allowed(&sender) {
                        tracing::debug!(sender = %sender, "Signal: sender not in allow_from, dropping");
                        return None;
                    }
                }
                _ => {
                    tracing::warn!("Signal: unknown DM policy, dropping message");
                    return None;
                }
            }
        }

        let target = Self::reply_target(data_msg, &sender);

        let timestamp = data_msg
            .timestamp
            .or(envelope.timestamp)
            .unwrap_or_else(|| {
                u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis(),
                )
                .unwrap_or(u64::MAX)
            });

        // Build metadata with signal-specific routing info.
        let metadata = serde_json::json!({
            "signal_sender": &sender,
            "signal_target": &target,
            "signal_timestamp": timestamp,
            "conversation_kind": conversation_kind,
            "conversation_scope_id": conversation_scope_id,
            "external_conversation_key": external_conversation_key,
            "raw_sender_id": &sender,
            "stable_sender_id": stable_sender_id,
            "group_id": group_id,
        });

        let mut msg = IncomingMessage::new("signal", &sender, text)
            .with_metadata(metadata)
            .with_attachments(media_attachments);

        // Use sourceName as display name if available.
        if let Some(ref name) = envelope.source_name
            && !name.is_empty()
        {
            msg = msg.with_user_name(name);
        }

        // Use a deterministic UUID as thread_id for all conversations.
        // This ensures DMs and groups continue the same thread AND work with
        // maybe_hydrate_thread, enabling conversation history persistence.
        // Priority: source_uuid > generated UUID from phone/group
        if data_msg.group_info.is_some() {
            // For groups, use the group ID to generate a deterministic UUID
            msg = msg.with_thread(Self::thread_id_from_identifier(&target));
        } else if let Some(ref uuid) = envelope.source_uuid {
            // Privacy mode users already have a UUID
            msg = msg.with_thread(uuid.clone());
        } else {
            // For regular DMs, generate a deterministic UUID from the phone number
            msg = msg.with_thread(Self::thread_id_from_identifier(&sender));
        }

        Some((msg, target))
    }
}

#[async_trait]
impl Channel for SignalChannel {
    fn config_schema(&self) -> Option<thinclaw_channels_core::ConfigSchema> {
        use thinclaw_channels_core::{ConfigField, ConfigOption, ConfigSchema};
        Some(ConfigSchema {
            channel_id: "signal".to_string(),
            channel_name: "Signal".to_string(),
            fields: vec![
                ConfigField {
                    id: "http_url".to_string(),
                    label: "signal-cli HTTP URL".to_string(),
                    field_type: "text".to_string(),
                    required: true,
                    help_text: Some("The local signal-cli daemon endpoint.".to_string()),
                    default_value: Some(serde_json::Value::String(self.config.http_url.clone())),
                    options: None,
                },
                ConfigField {
                    id: "account".to_string(),
                    label: "Signal account".to_string(),
                    field_type: "text".to_string(),
                    required: true,
                    help_text: Some("The registered E.164 account number.".to_string()),
                    default_value: Some(serde_json::Value::String(self.config.account.clone())),
                    options: None,
                },
                ConfigField {
                    id: "allow_from".to_string(),
                    label: "Allowed senders".to_string(),
                    field_type: "textarea".to_string(),
                    required: false,
                    help_text: Some(
                        "One sender per line (phone number or UUID). Empty allows all senders."
                            .to_string(),
                    ),
                    default_value: Some(serde_json::Value::String(
                        self.config.allow_from.join("\n"),
                    )),
                    options: None,
                },
                ConfigField {
                    id: "allow_from_groups".to_string(),
                    label: "Allowed groups".to_string(),
                    field_type: "textarea".to_string(),
                    required: false,
                    help_text: Some("One Signal group ID per line. Empty denies groups.".to_string()),
                    default_value: Some(serde_json::Value::String(
                        self.config.allow_from_groups.join("\n"),
                    )),
                    options: None,
                },
                ConfigField {
                    id: "dm_policy".to_string(),
                    label: "Direct-message policy".to_string(),
                    field_type: "select".to_string(),
                    required: true,
                    help_text: Some("Pair unknown senders, require the allowlist, or allow every DM.".to_string()),
                    default_value: Some(serde_json::Value::String(self.config.dm_policy.clone())),
                    options: Some(vec![
                        ConfigOption { value: "pairing".to_string(), label: "Pairing".to_string() },
                        ConfigOption { value: "allowlist".to_string(), label: "Allowlist".to_string() },
                        ConfigOption { value: "open".to_string(), label: "Open".to_string() },
                    ]),
                },
                ConfigField {
                    id: "group_policy".to_string(),
                    label: "Group policy".to_string(),
                    field_type: "select".to_string(),
                    required: true,
                    help_text: Some("Disable groups, require allowlists, or allow every group.".to_string()),
                    default_value: Some(serde_json::Value::String(self.config.group_policy.clone())),
                    options: Some(vec![
                        ConfigOption { value: "disabled".to_string(), label: "Disabled".to_string() },
                        ConfigOption { value: "allowlist".to_string(), label: "Allowlist".to_string() },
                        ConfigOption { value: "open".to_string(), label: "Open".to_string() },
                    ]),
                },
                ConfigField {
                    id: "group_allow_from".to_string(),
                    label: "Allowed group senders".to_string(),
                    field_type: "textarea".to_string(),
                    required: false,
                    help_text: Some("One sender per line. Empty inherits the DM allowlist.".to_string()),
                    default_value: Some(serde_json::Value::String(
                        self.config.group_allow_from.join("\n"),
                    )),
                    options: None,
                },
                ConfigField {
                    id: "ignore_attachments".to_string(),
                    label: "Ignore attachments".to_string(),
                    field_type: "checkbox".to_string(),
                    required: false,
                    help_text: Some("Skip downloading attached files.".to_string()),
                    default_value: Some(serde_json::Value::Bool(
                        self.config.ignore_attachments,
                    )),
                    options: None,
                },
                ConfigField {
                    id: "ignore_stories".to_string(),
                    label: "Ignore stories".to_string(),
                    field_type: "checkbox".to_string(),
                    required: false,
                    help_text: Some("Ignore Signal stories.".to_string()),
                    default_value: Some(serde_json::Value::Bool(self.config.ignore_stories)),
                    options: None,
                },
            ],
            help: Some(
                "Configure the signal-cli endpoint, sender policies, and media behavior. Changes are persisted and apply after a channel restart."
                    .to_string(),
            ),
        })
    }

    fn name(&self) -> &str {
        "signal"
    }

    fn formatting_hints(&self) -> Option<String> {
        Some(
            "Signal renders plain text only. Do not use markdown formatting. Keep messages concise."
                .to_string(),
        )
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        if let Some(handle) = self.sse_task.lock().await.take() {
            self.shutdown.store(true, Ordering::Relaxed);
            self.shutdown_notify.notify_waiters();
            drain_channel_task(handle, "signal-sse").await;
        }
        self.shutdown.store(false, Ordering::Relaxed);

        let config = self.config.clone();
        let client = self.client.clone();
        let reply_targets = Arc::clone(&self.reply_targets);
        let debug_mode = Arc::clone(&self.debug_mode);
        let shutdown = Arc::clone(&self.shutdown);
        let shutdown_notify = Arc::clone(&self.shutdown_notify);

        let handle = tokio::spawn(async move {
            if let Err(e) = sse_listener(
                config,
                client,
                tx,
                reply_targets,
                debug_mode,
                shutdown,
                shutdown_notify,
            )
            .await
            {
                tracing::error!("Signal SSE listener exited with error: {e}");
            }
        });
        *self.sse_task.lock().await = Some(handle);

        // Log the URL with credentials redacted (if any).
        let safe_url = Self::redact_url(&self.config.http_url);
        tracing::info!(
            url = %safe_url,
            "Signal channel started"
        );

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Resolve reply target from stored metadata.
        let target_str = {
            let targets = self.reply_targets.read().await;
            targets.peek(&msg.id).cloned()
        }
        .or_else(|| {
            // Fall back to metadata if not in the map.
            msg.metadata
                .get("signal_target")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| msg.user_id.clone());

        let target = Self::parse_recipient_target(&target_str);
        let temp_paths = write_signal_temp_attachments(&response.attachments).await?;
        let mut params = self.build_rpc_params(&target, Some(&response.content));
        Self::add_attachment_params(&mut params, &temp_paths);
        let result = self.rpc_request("send", params).await;
        cleanup_signal_temp_attachments(&temp_paths).await;
        result?;

        // Clean up stored target.
        self.reply_targets.write().await.pop(&msg.id);

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Send typing indicator for thinking status.
        if matches!(status, StatusUpdate::Thinking(_))
            && let Some(target_str) = metadata.get("signal_target").and_then(|v| v.as_str())
        {
            let target = Self::parse_recipient_target(target_str);
            let params = self.build_rpc_params(&target, None);
            let _ = self.rpc_request("sendTyping", params).await;
        }

        // Send approval prompt to user
        if let StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description: _,
            parameters,
        } = &status
            && let Some(target_str) = metadata.get("signal_target").and_then(|v| v.as_str())
        {
            let params_json = serde_json::to_string_pretty(parameters).unwrap_or_default();
            let message = format!(
                "⚠️ *Approval Required*\n\n\
                 *Request ID:* `{}`\n\
                 *Tool:* {}\n\
                 *Parameters:*\n```\n{}\n```\n\n\
                 Reply with:\n\
                 • `yes` or `y` - Approve this request\n\
                 • `always` or `a` - Approve and auto-approve future {} requests\n\
                 • `no` or `n` - Deny",
                request_id, tool_name, params_json, tool_name
            );
            self.send_status_message(target_str, &message).await;
        }

        // Filter out well-known UX/terminal status messages to avoid redundant updates.
        let should_forward_status = |msg: &str| {
            let normalized = msg.trim();
            !normalized.eq_ignore_ascii_case("done")
                && !normalized.eq_ignore_ascii_case("awaiting approval")
                && !normalized.eq_ignore_ascii_case("rejected")
        };
        // Filter/send status messages
        if let StatusUpdate::Status(msg) = &status
            && let Some(target_str) = metadata.get("signal_target").and_then(|v| v.as_str())
            && should_forward_status(msg)
        {
            self.send_status_message(target_str, msg).await;
        }

        // Send tool result previews to user (debug mode only)
        if self.is_debug()
            && let StatusUpdate::ToolResult { name, preview, .. } = &status
            && let Some(target_str) = metadata.get("signal_target").and_then(|v| v.as_str())
        {
            let truncated = if preview.chars().count() > 500 {
                let s: String = preview.chars().take(500).collect();
                format!("{s}...")
            } else {
                preview.clone()
            };
            let message = format!("Tool '{}' result:\n{}", name, truncated);
            self.send_status_message(target_str, &message).await;
        }

        // Send tool started notification (debug mode only)
        if self.is_debug()
            && let StatusUpdate::ToolStarted { name, .. } = &status
            && let Some(target_str) = metadata.get("signal_target").and_then(|v| v.as_str())
        {
            let message = format!("\u{25CB} Running tool: {}", name);
            self.send_status_message(target_str, &message).await;
        }

        // Send tool completed notification (debug mode only)
        if self.is_debug()
            && let StatusUpdate::ToolCompleted { name, success, .. } = &status
            && let Some(target_str) = metadata.get("signal_target").and_then(|v| v.as_str())
        {
            let (icon, color) = if *success {
                ("\u{25CF}", "success")
            } else {
                ("\u{2717}", "failed")
            };
            let message = format!("{} Tool '{}' completed ({})", icon, name, color);
            self.send_status_message(target_str, &message).await;
        }

        // Send job started notification (sandbox jobs)
        if let StatusUpdate::JobStarted {
            job_id,
            title,
            browse_url,
        } = &status
            && let Some(target_str) = metadata.get("signal_target").and_then(|v| v.as_str())
        {
            let message = format!(
                "\u{1F680} Job started: {}\nID: {}\nURL: {}",
                title, job_id, browse_url
            );
            self.send_status_message(target_str, &message).await;
        }

        // Send auth required notification
        if let StatusUpdate::AuthRequired {
            extension_name,
            instructions,
            auth_url,
            setup_url,
            ..
        } = &status
            && let Some(target_str) = metadata.get("signal_target").and_then(|v| v.as_str())
        {
            let mut message = format!("\u{1F512} Authentication required for: {}", extension_name);
            if let Some(instr) = instructions {
                message.push_str(&format!("\n\n{}", instr));
            }
            if let Some(url) = auth_url {
                message.push_str(&format!("\n\nAuth URL: {}", url));
            }
            if let Some(url) = setup_url {
                message.push_str(&format!("\nSetup URL: {}", url));
            }
            self.send_status_message(target_str, &message).await;
        }

        // Send auth completed notification
        if let StatusUpdate::AuthCompleted {
            extension_name,
            success,
            message: msg,
            ..
        } = &status
            && let Some(target_str) = metadata.get("signal_target").and_then(|v| v.as_str())
        {
            let icon = if *success { "\u{2705}" } else { "\u{274C}" };
            let mut message = format!(
                "{} Authentication {} for {}",
                icon,
                if *success { "completed" } else { "failed" },
                extension_name
            );
            if !msg.is_empty() {
                message.push_str(&format!("\n{}", msg));
            }
            self.send_status_message(target_str, &message).await;
        }

        // Send agent progress messages to user
        if let StatusUpdate::AgentMessage {
            content,
            message_type,
        } = &status
            && let Some(target_str) = metadata.get("signal_target").and_then(|v| v.as_str())
        {
            let prefix = match message_type.as_str() {
                "warning" => "⚠️ ",
                "question" => "❓ ",
                "interim_result" => "📋 ",
                _ => "💬 ",
            };
            let message = format!("{}{}", prefix, content);
            self.send_status_message(target_str, &message).await;
        }

        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Only send to valid E.164 phone numbers or UUIDs.
        // Proactive notifications may arrive with user_id="default" which
        // is not a valid Signal recipient.
        if !Self::is_e164(user_id) && !Self::is_uuid(user_id) {
            tracing::debug!(
                recipient = user_id,
                "Signal: skipping broadcast — recipient is not an E.164 number or UUID"
            );
            return Ok(());
        }
        let target = Self::parse_recipient_target(user_id);
        let temp_paths = write_signal_temp_attachments(&response.attachments).await?;
        let mut params = self.build_rpc_params(&target, Some(&response.content));
        Self::add_attachment_params(&mut params, &temp_paths);
        let result = self.rpc_request("send", params).await;
        cleanup_signal_temp_attachments(&temp_paths).await;
        result?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.shutdown.store(true, Ordering::Relaxed);
        self.shutdown_notify.notify_waiters();
        if let Some(handle) = self.sse_task.lock().await.take() {
            drain_channel_task(handle, "signal-sse").await;
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        let url = format!("{}{}", self.config.http_url, SIGNAL_HEALTH_ENDPOINT);
        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ChannelError::HealthCheckFailed {
                name: format!("signal ({}): {e}", Self::redact_url(&url)),
            })?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: format!("signal: HTTP {}", resp.status()),
            })
        }
    }
}

impl SignalChannel {
    async fn send_status_message(&self, target: &str, message: &str) {
        let target = Self::parse_recipient_target(target);
        let params = self.build_rpc_params(&target, Some(message));
        if let Err(e) = self.rpc_request("send", params).await {
            tracing::warn!("Signal: failed to send status message: {}", e);
        }
    }
}

/// Long-running SSE listener that reconnects with exponential backoff.
async fn sse_listener(
    config: SignalConfig,
    client: Client,
    tx: tokio::sync::mpsc::Sender<IncomingMessage>,
    reply_targets: Arc<RwLock<LruCache<Uuid, String>>>,
    debug_mode: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
) -> Result<(), ChannelError> {
    let channel = SignalChannel::from_parts(
        config,
        client,
        Arc::clone(&reply_targets),
        Arc::clone(&debug_mode),
        Arc::clone(&shutdown),
        Arc::clone(&shutdown_notify),
    );

    let mut url = reqwest::Url::parse(&format!("{}/api/v1/events", channel.config.http_url))
        .map_err(|e| ChannelError::StartupFailed {
            name: "signal".to_string(),
            reason: format!("Invalid SSE URL: {e}"),
        })?;
    url.query_pairs_mut()
        .append_pair("account", &channel.config.account);

    let mut retry_delay = Duration::from_secs(2);
    let max_delay = Duration::from_secs(60);

    loop {
        if shutdown.load(Ordering::Relaxed) {
            return Ok(());
        }

        let resp = tokio::select! {
            resp = channel
                .client
                .get(url.clone())
                .header("Accept", "text/event-stream")
                .send() => resp,
            _ = shutdown_notify.notified() => {
                if shutdown.load(Ordering::Relaxed) {
                    return Ok(());
                }
                continue;
            }
        };

        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                let status = r.status();
                tracing::warn!("Signal SSE returned {status}");
                if sleep_or_signal_shutdown(&shutdown, &shutdown_notify, retry_delay).await {
                    return Ok(());
                }
                retry_delay = (retry_delay * 2).min(max_delay);
                continue;
            }
            Err(e) => {
                let safe_url = SignalChannel::redact_url(url.as_str());
                tracing::warn!(
                    error = %e.without_url(),
                    "Signal SSE connect error to {safe_url}; retrying"
                );
                if sleep_or_signal_shutdown(&shutdown, &shutdown_notify, retry_delay).await {
                    return Ok(());
                }
                retry_delay = (retry_delay * 2).min(max_delay);
                continue;
            }
        };

        // Connection succeeded — reset backoff.
        retry_delay = Duration::from_secs(2);
        tracing::info!("Signal SSE connected");

        let mut bytes_stream = resp.bytes_stream();
        let mut buffer = String::with_capacity(8192);
        let mut current_data = String::with_capacity(4096);
        // Holds trailing bytes from the previous chunk that form an incomplete
        // multi-byte UTF-8 sequence. At most 3 bytes (the longest incomplete
        // leading sequence for a 4-byte character).
        let mut utf8_carry: Vec<u8> = Vec::with_capacity(4);

        loop {
            let next_chunk = tokio::select! {
                chunk = bytes_stream.next() => chunk,
                _ = shutdown_notify.notified() => {
                    if shutdown.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    continue;
                }
            };
            let Some(chunk) = next_chunk else {
                break;
            };
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!(error = %e.without_url(), "Signal SSE chunk error; reconnecting");
                    break;
                }
            };

            // Prepend any leftover bytes from the previous chunk.
            let decode_buf = if utf8_carry.is_empty() {
                chunk.to_vec()
            } else {
                let mut combined = std::mem::take(&mut utf8_carry);
                combined.extend_from_slice(&chunk);
                combined
            };

            // Decode as much valid UTF-8 as possible, carrying over any
            // incomplete trailing sequence to the next iteration.
            let (valid_len, carry_start) = match std::str::from_utf8(&decode_buf) {
                Ok(_) => (decode_buf.len(), decode_buf.len()),
                Err(e) => {
                    let valid_up_to = e.valid_up_to();
                    match e.error_len() {
                        Some(bad_len) => {
                            // Genuinely invalid byte sequence (not just incomplete).
                            // Skip the bad byte(s) and keep going with what we have.
                            tracing::debug!(
                                "Signal SSE invalid UTF-8 byte at offset {valid_up_to}, \
                                 skipping"
                            );
                            // Advance past the bad byte(s); remaining data (if any)
                            // will be carried over to the next chunk.
                            (valid_up_to, valid_up_to + bad_len)
                        }
                        None => {
                            // Incomplete multi-byte sequence at the end – carry it over.
                            (valid_up_to, valid_up_to)
                        }
                    }
                }
            };

            use std::borrow::Cow;

            debug_assert!(
                std::str::from_utf8(&decode_buf[..valid_len]).is_ok(),
                "valid_len {} should be a valid UTF-8 boundary (buffer len: {})",
                valid_len,
                decode_buf.len()
            );

            let text: Cow<str> = match std::str::from_utf8(&decode_buf[..valid_len]) {
                Ok(s) => Cow::Borrowed(s),
                Err(_) => {
                    tracing::warn!(
                        "Signal SSE: unexpected invalid UTF-8 boundary at valid_len {}, \
                         falling back to lossy conversion",
                        valid_len
                    );
                    Cow::Owned(String::from_utf8_lossy(&decode_buf[..valid_len]).into_owned())
                }
            };

            if buffer.len() + text.len() > MAX_SSE_BUFFER_SIZE {
                tracing::warn!(
                    "Signal SSE buffer overflow, resetting: buffer_len={} text_len={} max={}",
                    buffer.len(),
                    text.len(),
                    MAX_SSE_BUFFER_SIZE
                );
                buffer.clear();
                utf8_carry.clear();
                current_data.clear();
                continue;
            }
            buffer.push_str(&text);

            // Preserve any trailing incomplete bytes for the next chunk.
            if carry_start < decode_buf.len() {
                utf8_carry.extend_from_slice(&decode_buf[carry_start..]);
            }

            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                buffer.drain(..=newline_pos);

                // Skip SSE comments (keepalive).
                if line.starts_with(':') {
                    continue;
                }

                if line.is_empty() {
                    // Empty line = event boundary, dispatch accumulated data.
                    if !current_data.is_empty() {
                        match serde_json::from_str::<SseEnvelope>(&current_data) {
                            Ok(sse) => {
                                if let Some(ref envelope) = sse.envelope
                                    && let Some((msg, target)) = channel.process_envelope(envelope)
                                {
                                    // Handle /debug command locally (same as REPL).
                                    let content_lower = msg.content.trim().to_lowercase();
                                    if content_lower == "/debug" {
                                        let new_state = channel.toggle_debug();
                                        let response = if new_state {
                                            "Debug mode enabled. Tool execution will be shown in chat."
                                        } else {
                                            "Debug mode disabled. Tool execution will be hidden from chat."
                                        };
                                        let reply_params = channel.build_rpc_params(
                                            &SignalChannel::parse_recipient_target(&target),
                                            Some(response),
                                        );
                                        let _ = channel.rpc_request("send", reply_params).await;
                                        // Don't send the /debug command to the agent.
                                        continue;
                                    }

                                    // Store reply target for respond().
                                    // LruCache automatically evicts the
                                    // least-recently-used entry when full.
                                    {
                                        let mut targets = reply_targets.write().await;
                                        targets.put(msg.id, target);
                                    }
                                    if tx.send(msg).await.is_err() {
                                        tracing::debug!("Signal SSE: receiver dropped, exiting");
                                        return Ok(());
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::debug!("Signal SSE parse skip: {e}");
                            }
                        }
                        current_data.clear();
                    }
                } else if let Some(data) = line.strip_prefix("data:") {
                    if current_data.len() + data.len() > MAX_SSE_EVENT_SIZE {
                        tracing::warn!("Signal SSE event too large, dropping");
                        current_data.clear();
                        continue;
                    }
                    if !current_data.is_empty() {
                        current_data.push('\n');
                    }
                    current_data.push_str(data.trim_start());
                }
                // Ignore "event:", "id:", "retry:" lines.
            }
        }

        // Process any trailing data before reconnect.
        if !current_data.is_empty()
            && let Ok(sse) = serde_json::from_str::<SseEnvelope>(&current_data)
            && let Some(ref envelope) = sse.envelope
            && let Some((msg, target)) = channel.process_envelope(envelope)
        {
            reply_targets.write().await.put(msg.id, target);
            let _ = tx.send(msg).await;
        }

        tracing::debug!("Signal SSE stream ended, reconnecting with backoff...");
        if sleep_or_signal_shutdown(&shutdown, &shutdown_notify, retry_delay).await {
            return Ok(());
        }
        retry_delay = std::cmp::min(retry_delay * 2, max_delay);
    }
}

async fn sleep_or_signal_shutdown(
    shutdown: &Arc<AtomicBool>,
    shutdown_notify: &Arc<Notify>,
    duration: Duration,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => shutdown.load(Ordering::Relaxed),
        _ = shutdown_notify.notified() => shutdown.load(Ordering::Relaxed),
    }
}

async fn drain_channel_task(mut handle: JoinHandle<()>, name: &'static str) {
    tokio::select! {
        result = &mut handle => {
            if let Err(error) = result {
                tracing::warn!(channel = "signal", task = name, error = %error, "Signal channel task exited with error");
            }
        }
        _ = tokio::time::sleep(CHANNEL_TASK_SHUTDOWN_TIMEOUT) => {
            handle.abort();
            let _ = handle.await;
            tracing::warn!(channel = "signal", task = name, "Signal channel task did not drain before timeout; aborted");
        }
    }
}

#[cfg(test)]
mod tests;
