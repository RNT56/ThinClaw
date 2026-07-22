//! Shared Nostr runtime used by both the owner-control channel and the Nostr tool.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read as _;
use std::net::IpAddr;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use nostr_sdk::prelude::*;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use thinclaw_types::error::ChannelError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const STARTUP_BACKLOG_LIMIT: usize = 256;
const MAX_SEEN_CONTROL_EVENTS: usize = 512;
const MAX_PROTOCOL_PREFERENCES: usize = 512;
const MAX_NOSTR_STATE_BYTES: usize = 256 * 1024;
const MAX_NOSTR_RELAYS: usize = 32;
const MAX_NOSTR_RELAY_URL_BYTES: usize = 4 * 1024;
const MAX_NOSTR_DNS_ADDRESSES: usize = 64;
const MAX_NOSTR_KEY_BYTES: usize = 1024;
const MAX_NOSTR_ALLOW_FROM: usize = 512;
const MAX_NOSTR_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_NOSTR_ENCRYPTED_EVENT_BYTES: usize = 512 * 1024;
const MAX_NOSTR_EVENT_TAGS: usize = 1024;
const MAX_NOSTR_EVENT_TAG_FIELDS: usize = 64;
const MAX_NOSTR_EVENT_TAG_BYTES: usize = 512 * 1024;
const MAX_NOSTR_THREAD_ROOTS: usize = 32;
const MAX_NOSTR_THREAD_REPLIES: usize = 100;

#[derive(Clone)]
pub struct NostrConfig {
    pub private_key: SecretString,
    pub relays: Vec<String>,
    pub owner_pubkey: Option<String>,
    pub social_dm_enabled: bool,
    pub allow_from: Vec<String>,
}

impl std::fmt::Debug for NostrConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let relays = self
            .relays
            .iter()
            .map(|relay| redacted_relay_label(relay))
            .collect::<Vec<_>>();
        f.debug_struct("NostrConfig")
            .field("private_key", &"[REDACTED]")
            .field("relays", &relays)
            .field("owner_pubkey", &self.owner_pubkey)
            .field("social_dm_enabled", &self.social_dm_enabled)
            .field("allow_from_count", &self.allow_from.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NostrDmProtocol {
    Nip04,
    GiftWrap,
}

impl NostrDmProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nip04 => "nip04",
            Self::GiftWrap => "gift_wrap",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "nip04" => Some(Self::Nip04),
            "gift_wrap" | "giftwrap" | "nip17" | "nip59" => Some(Self::GiftWrap),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NostrInboundDm {
    pub sender: PublicKey,
    pub sender_hex: String,
    pub sender_npub: String,
    pub content: String,
    pub protocol: NostrDmProtocol,
    pub envelope_event_id: String,
    pub dm_event_id: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedRuntimeState {
    #[serde(default)]
    seen_control_event_ids: VecDeque<String>,
    #[serde(default)]
    recipient_dm_protocols: HashMap<String, String>,
}

pub struct NostrRuntime {
    client: Client,
    keys: Keys,
    relays: Vec<String>,
    owner_pubkey: Option<PublicKey>,
    social_dm_enabled: bool,
    state_path: PathBuf,
    connected: Mutex<bool>,
    state: RwLock<PersistedRuntimeState>,
}

impl std::fmt::Debug for NostrRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let relays = self
            .relays
            .iter()
            .map(|relay| redacted_relay_label(relay))
            .collect::<Vec<_>>();
        f.debug_struct("NostrRuntime")
            .field("relays", &relays)
            .field(
                "owner_pubkey",
                &self.owner_pubkey.as_ref().map(PublicKey::to_hex),
            )
            .field("social_dm_enabled", &self.social_dm_enabled)
            .field("state_path", &self.state_path)
            .finish()
    }
}

impl NostrRuntime {
    pub fn new(config: &NostrConfig) -> Result<Self, ChannelError> {
        validate_legacy_allow_from(&config.allow_from)?;
        let keys = parse_keys(config)?;
        let relays = validate_relay_urls(&config.relays)?;
        let owner_pubkey = config
            .owner_pubkey
            .as_deref()
            .map(parse_public_key)
            .transpose()
            .map_err(ChannelError::Configuration)?;
        let state_path = thinclaw_platform::state_paths()
            .home
            .join("nostr-runtime-state.json");
        let state = load_state(&state_path)?;

        Ok(Self {
            client: Client::new(keys.clone()),
            keys,
            relays,
            owner_pubkey,
            social_dm_enabled: config.social_dm_enabled,
            state_path,
            connected: Mutex::new(false),
            state: RwLock::new(state),
        })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    pub fn public_key_hex(&self) -> String {
        self.public_key().to_hex()
    }

    pub fn public_key_npub(&self) -> String {
        self.public_key()
            .to_bech32()
            .unwrap_or_else(|_| self.public_key_hex())
    }

    pub fn owner_pubkey(&self) -> Option<PublicKey> {
        self.owner_pubkey
    }

    pub fn owner_pubkey_hex(&self) -> Option<String> {
        self.owner_pubkey.map(|pubkey| pubkey.to_hex())
    }

    pub fn owner_pubkey_npub(&self) -> Option<String> {
        self.owner_pubkey
            .map(|pubkey| pubkey.to_bech32().unwrap_or_else(|_| pubkey.to_hex()))
    }

    pub fn social_dm_enabled(&self) -> bool {
        self.social_dm_enabled
    }

    pub fn relay_count(&self) -> usize {
        self.relays.len()
    }

    pub async fn ensure_connected(&self) -> Result<(), ChannelError> {
        let mut connected = self.connected.lock().await;
        if *connected {
            if self.connected_relay_count().await > 0 {
                return Ok(());
            }
            *connected = false;
        }

        futures::future::try_join_all(
            self.relays
                .iter()
                .map(|relay_url| validate_relay_destination(relay_url)),
        )
        .await?;

        for relay_url in &self.relays {
            if let Err(err) = self.client.add_relay(relay_url.as_str()).await {
                tracing::warn!(
                    relay = %redacted_relay_label(relay_url),
                    error = %err,
                    "Failed to add Nostr relay"
                );
            }
        }

        if self.client.relays().await.is_empty() {
            return Err(ChannelError::StartupFailed {
                name: "nostr".to_string(),
                reason: "No configured Nostr relay could be added".to_string(),
            });
        }

        self.client.connect().await;
        self.client.wait_for_connection(CONNECT_TIMEOUT).await;
        if self.connected_relay_count().await == 0 {
            self.client.disconnect().await;
            return Err(ChannelError::StartupFailed {
                name: "nostr".to_string(),
                reason: format!(
                    "No Nostr relay connected within {} seconds",
                    CONNECT_TIMEOUT.as_secs()
                ),
            });
        }
        *connected = true;

        Ok(())
    }

    pub fn control_filters(&self) -> Vec<Filter> {
        let public_key = self.public_key();
        vec![
            Filter::new()
                .kind(Kind::EncryptedDirectMessage)
                .pubkey(public_key)
                .limit(STARTUP_BACKLOG_LIMIT),
            Filter::new()
                .kind(Kind::GiftWrap)
                .pubkey(public_key)
                .limit(STARTUP_BACKLOG_LIMIT),
        ]
    }

    pub fn is_supported_dm_kind(&self, kind: Kind) -> bool {
        kind == Kind::EncryptedDirectMessage || kind == Kind::GiftWrap
    }

    pub async fn mark_control_event_seen(&self, event_id: &EventId) -> Result<bool, ChannelError> {
        let id = event_id.to_hex();
        let mut state = self.state.write().await;
        if state.seen_control_event_ids.iter().any(|seen| seen == &id) {
            return Ok(false);
        }
        let mut next_state = state.clone();
        next_state.seen_control_event_ids.push_back(id);
        while next_state.seen_control_event_ids.len() > MAX_SEEN_CONTROL_EVENTS {
            next_state.seen_control_event_ids.pop_front();
        }
        save_state(&self.state_path, &next_state).await?;
        *state = next_state;
        Ok(true)
    }

    pub async fn remember_protocol(
        &self,
        pubkey: &PublicKey,
        protocol: NostrDmProtocol,
    ) -> Result<(), ChannelError> {
        let mut state = self.state.write().await;
        let mut next_state = state.clone();
        next_state
            .recipient_dm_protocols
            .insert(pubkey.to_hex(), protocol.as_str().to_string());
        while next_state.recipient_dm_protocols.len() > MAX_PROTOCOL_PREFERENCES {
            if let Some(oldest) = next_state.recipient_dm_protocols.keys().next().cloned() {
                next_state.recipient_dm_protocols.remove(&oldest);
            } else {
                break;
            }
        }
        save_state(&self.state_path, &next_state).await?;
        *state = next_state;
        Ok(())
    }

    pub async fn preferred_protocol(&self, pubkey: &PublicKey) -> Option<NostrDmProtocol> {
        self.state
            .read()
            .await
            .recipient_dm_protocols
            .get(&pubkey.to_hex())
            .and_then(|value| NostrDmProtocol::parse(value))
    }

    pub async fn decrypt_inbound_dm(
        &self,
        event: &Event,
    ) -> Result<Option<NostrInboundDm>, ChannelError> {
        match event.kind {
            Kind::EncryptedDirectMessage => self.decrypt_nip04(event).await,
            Kind::GiftWrap => self.decrypt_gift_wrap(event).await,
            _ => Ok(None),
        }
    }

    async fn decrypt_nip04(&self, event: &Event) -> Result<Option<NostrInboundDm>, ChannelError> {
        if event.content.len() > MAX_NOSTR_ENCRYPTED_EVENT_BYTES {
            tracing::warn!(
                sender = %event.pubkey.to_hex(),
                "Nostr: dropping oversized encrypted DM"
            );
            return Ok(None);
        }
        let plaintext = match nip04::decrypt(self.keys.secret_key(), &event.pubkey, &event.content)
        {
            Ok(text) => text,
            Err(err) => {
                tracing::warn!(
                    sender = %event.pubkey.to_hex(),
                    error = %err,
                    "Nostr: failed to decrypt NIP-04 DM"
                );
                return Ok(None);
            }
        };
        if plaintext.trim().is_empty() || plaintext.len() > MAX_NOSTR_MESSAGE_BYTES {
            return Ok(None);
        }
        let sender_hex = event.pubkey.to_hex();
        Ok(Some(NostrInboundDm {
            sender: event.pubkey,
            sender_npub: event
                .pubkey
                .to_bech32()
                .unwrap_or_else(|_| sender_hex.clone()),
            sender_hex,
            content: plaintext,
            protocol: NostrDmProtocol::Nip04,
            envelope_event_id: event.id.to_hex(),
            dm_event_id: Some(event.id.to_hex()),
        }))
    }

    async fn decrypt_gift_wrap(
        &self,
        event: &Event,
    ) -> Result<Option<NostrInboundDm>, ChannelError> {
        if event.content.len() > MAX_NOSTR_ENCRYPTED_EVENT_BYTES {
            tracing::warn!("Nostr: dropping oversized gift-wrap DM");
            return Ok(None);
        }
        let UnwrappedGift { sender, rumor } = match self.client.unwrap_gift_wrap(event).await {
            Ok(gift) => gift,
            Err(err) => {
                tracing::warn!(error = %err, "Nostr: failed to unwrap gift-wrap DM");
                return Ok(None);
            }
        };

        if rumor.kind != Kind::PrivateDirectMessage
            || rumor.content.trim().is_empty()
            || rumor.content.len() > MAX_NOSTR_MESSAGE_BYTES
        {
            return Ok(None);
        }

        let sender_hex = sender.to_hex();
        Ok(Some(NostrInboundDm {
            sender,
            sender_npub: sender.to_bech32().unwrap_or_else(|_| sender_hex.clone()),
            sender_hex,
            content: rumor.content,
            protocol: NostrDmProtocol::GiftWrap,
            envelope_event_id: event.id.to_hex(),
            dm_event_id: rumor.id.map(|id| id.to_hex()),
        }))
    }

    pub async fn send_dm(
        &self,
        recipient: &PublicKey,
        plaintext: &str,
        protocol: Option<NostrDmProtocol>,
    ) -> Result<String, ChannelError> {
        validate_outbound_message(plaintext)?;
        self.ensure_connected().await?;
        let protocol = match protocol {
            Some(protocol) => protocol,
            None => self
                .preferred_protocol(recipient)
                .await
                .unwrap_or(NostrDmProtocol::GiftWrap),
        };

        let event_id = match protocol {
            NostrDmProtocol::Nip04 => self.send_nip04_dm(recipient, plaintext).await?,
            NostrDmProtocol::GiftWrap => match self.send_gift_wrap_dm(recipient, plaintext).await {
                Ok(event_id) => event_id,
                Err(err) => {
                    if matches!(
                        self.preferred_protocol(recipient).await,
                        Some(NostrDmProtocol::Nip04)
                    ) {
                        self.send_nip04_dm(recipient, plaintext).await?
                    } else {
                        return Err(err);
                    }
                }
            },
        };

        self.remember_protocol(recipient, protocol).await?;
        Ok(event_id)
    }

    pub async fn send_reply(
        &self,
        recipient: &PublicKey,
        plaintext: &str,
        reply_protocol: Option<NostrDmProtocol>,
    ) -> Result<String, ChannelError> {
        let protocol = reply_protocol.or(Some(NostrDmProtocol::GiftWrap));
        self.send_dm(recipient, plaintext, protocol).await
    }

    async fn send_nip04_dm(
        &self,
        recipient: &PublicKey,
        plaintext: &str,
    ) -> Result<String, ChannelError> {
        let encrypted =
            nip04::encrypt(self.keys.secret_key(), recipient, plaintext).map_err(|err| {
                ChannelError::SendFailed {
                    name: "nostr".to_string(),
                    reason: format!("NIP-04 encryption failed: {err}"),
                }
            })?;

        let builder = EventBuilder::new(Kind::EncryptedDirectMessage, encrypted)
            .tag(Tag::public_key(*recipient));
        let event = self
            .client
            .sign_event_builder(builder)
            .await
            .map_err(|err| ChannelError::SendFailed {
                name: "nostr".to_string(),
                reason: format!("Failed to sign NIP-04 DM: {err}"),
            })?;
        self.client
            .send_event(&event)
            .await
            .map_err(|err| ChannelError::SendFailed {
                name: "nostr".to_string(),
                reason: format!("Failed to send NIP-04 DM: {err}"),
            })?;
        Ok(event.id.to_hex())
    }

    async fn send_gift_wrap_dm(
        &self,
        recipient: &PublicKey,
        plaintext: &str,
    ) -> Result<String, ChannelError> {
        let output = self
            .client
            .send_private_msg(*recipient, plaintext, [])
            .await
            .map_err(|err| ChannelError::SendFailed {
                name: "nostr".to_string(),
                reason: format!("Failed to send gift-wrap DM: {err}"),
            })?;
        Ok(output.val.to_hex())
    }

    pub async fn publish_builder(&self, builder: EventBuilder) -> Result<String, ChannelError> {
        self.ensure_connected().await?;
        let output = self
            .client
            .send_event_builder(builder)
            .await
            .map_err(|err| ChannelError::SendFailed {
                name: "nostr".to_string(),
                reason: format!("Failed to publish event: {err}"),
            })?;
        Ok(output.val.to_hex())
    }

    pub async fn fetch_profile(
        &self,
        pubkey: &PublicKey,
    ) -> Result<Option<serde_json::Value>, ChannelError> {
        self.ensure_connected().await?;
        let metadata = self
            .client
            .fetch_metadata(*pubkey, FETCH_TIMEOUT)
            .await
            .map_err(|err| ChannelError::Disconnected {
                name: "nostr".to_string(),
                reason: format!("Failed to fetch profile: {err}"),
            })?;

        Ok(metadata.map(|metadata| {
            serde_json::json!({
                "pubkey": pubkey.to_hex(),
                "npub": pubkey.to_bech32().unwrap_or_else(|_| pubkey.to_hex()),
                "metadata": serde_json::to_value(metadata).unwrap_or(serde_json::Value::Null),
                "untrusted_external_content": true,
                "content_trust": "untrusted_external_nostr_content",
            })
        }))
    }

    pub async fn fetch_event(&self, event_id: EventId) -> Result<Option<Event>, ChannelError> {
        self.ensure_connected().await?;
        let events = self
            .client
            .fetch_events(Filter::new().id(event_id).limit(1), FETCH_TIMEOUT)
            .await
            .map_err(|err| ChannelError::Disconnected {
                name: "nostr".to_string(),
                reason: format!("Failed to fetch event: {err}"),
            })?;
        Ok(events.into_iter().find(event_is_bounded))
    }

    pub async fn fetch_thread(&self, event_id: EventId) -> Result<serde_json::Value, ChannelError> {
        let target = self.fetch_event(event_id).await?;
        let Some(target) = target else {
            return Ok(serde_json::json!({
                "found": false,
                "event_id": event_id.to_hex(),
                "untrusted_external_content": true,
                "content_trust": "untrusted_external_nostr_content",
            }));
        };

        let mut related_ids: Vec<EventId> = target
            .tags
            .event_ids()
            .copied()
            .take(MAX_NOSTR_THREAD_ROOTS)
            .collect();
        related_ids.push(target.id);

        let mut replies = Vec::new();
        for related in related_ids {
            let events = self
                .client
                .fetch_events(Filter::new().event(related).limit(100), FETCH_TIMEOUT)
                .await
                .map_err(|err| ChannelError::Disconnected {
                    name: "nostr".to_string(),
                    reason: format!("Failed to fetch thread replies: {err}"),
                })?;
            for reply in events.into_iter().filter(event_is_bounded) {
                if !replies
                    .iter()
                    .any(|existing: &Event| existing.id == reply.id)
                {
                    replies.push(reply);
                    if replies.len() >= MAX_NOSTR_THREAD_REPLIES {
                        break;
                    }
                }
            }
            if replies.len() >= MAX_NOSTR_THREAD_REPLIES {
                break;
            }
        }
        replies.sort_by_key(|event| event.created_at.as_secs());

        Ok(serde_json::json!({
            "found": true,
            "event": serialize_event(&target),
            "replies": replies.iter().map(serialize_event).collect::<Vec<_>>(),
            "untrusted_external_content": true,
            "content_trust": "untrusted_external_nostr_content",
        }))
    }

    pub async fn fetch_mentions(
        &self,
        pubkey: &PublicKey,
        limit: usize,
    ) -> Result<serde_json::Value, ChannelError> {
        self.ensure_connected().await?;
        let events = self
            .client
            .fetch_events(
                Filter::new()
                    .pubkey(*pubkey)
                    .limit(limit.clamp(1, 100))
                    .kinds(vec![
                        Kind::TextNote,
                        Kind::Comment,
                        Kind::Reaction,
                        Kind::Repost,
                        Kind::GenericRepost,
                    ]),
                FETCH_TIMEOUT,
            )
            .await
            .map_err(|err| ChannelError::Disconnected {
                name: "nostr".to_string(),
                reason: format!("Failed to fetch mentions: {err}"),
            })?;

        Ok(serde_json::json!({
            "mentions": events.into_iter().filter(event_is_bounded).map(|event| serialize_event(&event)).collect::<Vec<_>>(),
            "untrusted_external_content": true,
            "content_trust": "untrusted_external_nostr_content",
        }))
    }

    pub async fn fetch_dm_inbox(&self, limit: usize) -> Result<serde_json::Value, ChannelError> {
        if !self.social_dm_enabled {
            return Err(ChannelError::Configuration(
                "Nostr social DM reading is disabled".to_string(),
            ));
        }
        self.ensure_connected().await?;

        let public_key = self.public_key();
        let mut messages: Vec<serde_json::Value> = Vec::new();

        let nip04_events = self
            .client
            .fetch_events(
                Filter::new()
                    .kind(Kind::EncryptedDirectMessage)
                    .pubkey(public_key)
                    .limit(limit.clamp(1, 100)),
                FETCH_TIMEOUT,
            )
            .await
            .map_err(|err| ChannelError::Disconnected {
                name: "nostr".to_string(),
                reason: format!("Failed to fetch NIP-04 inbox: {err}"),
            })?;
        for event in nip04_events.into_iter().filter(event_is_bounded) {
            if let Some(dm) = self.decrypt_nip04(&event).await?
                && Some(dm.sender) != self.owner_pubkey
            {
                self.remember_protocol(&dm.sender, dm.protocol).await?;
                messages.push(serialize_dm_message(&dm));
            }
        }

        let gift_wrap_events = self
            .client
            .fetch_events(
                Filter::new()
                    .kind(Kind::GiftWrap)
                    .pubkey(public_key)
                    .limit(limit.clamp(1, 100)),
                FETCH_TIMEOUT,
            )
            .await
            .map_err(|err| ChannelError::Disconnected {
                name: "nostr".to_string(),
                reason: format!("Failed to fetch gift-wrap inbox: {err}"),
            })?;
        for event in gift_wrap_events.into_iter().filter(event_is_bounded) {
            if let Some(dm) = self.decrypt_gift_wrap(&event).await?
                && Some(dm.sender) != self.owner_pubkey
            {
                self.remember_protocol(&dm.sender, dm.protocol).await?;
                messages.push(serialize_dm_message(&dm));
            }
        }

        Ok(serde_json::json!({
            "messages": messages,
            "untrusted_external_content": true,
            "content_trust": "untrusted_external_nostr_content",
        }))
    }

    pub async fn connected_relay_count(&self) -> usize {
        self.client
            .relays()
            .await
            .values()
            .filter(|relay| relay.status() == RelayStatus::Connected)
            .count()
    }

    pub async fn shutdown(&self) {
        // `Client::shutdown` permanently disables the SDK relay pool. Channels
        // are restarted after health failures, so use the reversible lifecycle
        // operations here and let dropping the runtime perform final cleanup.
        self.client.unsubscribe_all().await;
        self.client.disconnect().await;
        *self.connected.lock().await = false;
    }
}

fn validate_relay_urls(relays: &[String]) -> Result<Vec<String>, ChannelError> {
    if relays.is_empty() || relays.len() > MAX_NOSTR_RELAYS {
        return Err(ChannelError::Configuration(format!(
            "Nostr requires between 1 and {MAX_NOSTR_RELAYS} relay URLs"
        )));
    }

    let mut seen = HashSet::new();
    let mut validated = Vec::with_capacity(relays.len());
    for relay in relays {
        let relay = relay.trim();
        if relay.is_empty()
            || relay.len() > MAX_NOSTR_RELAY_URL_BYTES
            || relay.chars().any(char::is_control)
        {
            return Err(ChannelError::Configuration(
                "A Nostr relay URL is empty, malformed, or exceeds its size limit".to_string(),
            ));
        }
        let parsed = url::Url::parse(relay)
            .map_err(|_| ChannelError::Configuration("A Nostr relay URL is invalid".to_string()))?;
        if !matches!(parsed.scheme(), "ws" | "wss")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
            || parsed.host_str().is_none()
            || parsed.port_or_known_default().is_none()
        {
            return Err(ChannelError::Configuration(
                "Nostr relay URLs must use ws:// or wss://, include a host, and contain no credentials or fragments"
                    .to_string(),
            ));
        }
        if let Some(host) = parsed.host_str()
            && let Ok(ip) = host.parse::<IpAddr>()
            && !thinclaw_tools_core::is_public_outbound_ip(ip)
            && !is_safe_private_relay_ip(ip)
        {
            return Err(ChannelError::Configuration(
                "A Nostr relay URL contains an unusable IP address".to_string(),
            ));
        }

        let normalized = parsed.to_string();
        if seen.insert(normalized.clone()) {
            validated.push(normalized);
        }
    }

    if validated.is_empty() {
        return Err(ChannelError::Configuration(
            "Nostr requires at least one distinct relay URL".to_string(),
        ));
    }
    Ok(validated)
}

fn validate_legacy_allow_from(allow_from: &[String]) -> Result<(), ChannelError> {
    if allow_from.len() > MAX_NOSTR_ALLOW_FROM
        || allow_from.iter().any(|entry| {
            entry.is_empty()
                || entry.len() > MAX_NOSTR_KEY_BYTES
                || entry.chars().any(char::is_control)
        })
    {
        return Err(ChannelError::Configuration(
            "Nostr legacy allow-from entries are malformed or exceed their limits".to_string(),
        ));
    }
    Ok(())
}

async fn validate_relay_destination(relay: &str) -> Result<(), ChannelError> {
    let parsed = url::Url::parse(relay).map_err(|_| {
        ChannelError::Configuration("A validated Nostr relay URL became invalid".to_string())
    })?;
    let host = parsed.host_str().ok_or_else(|| {
        ChannelError::Configuration("A validated Nostr relay URL has no host".to_string())
    })?;
    let port = parsed.port_or_known_default().ok_or_else(|| {
        ChannelError::Configuration("A validated Nostr relay URL has no port".to_string())
    })?;
    let addresses = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| ChannelError::StartupFailed {
        name: "nostr".to_string(),
        reason: format!(
            "Relay {} did not resolve within 5 seconds",
            redacted_relay_label(relay)
        ),
    })?
    .map_err(|_| ChannelError::StartupFailed {
        name: "nostr".to_string(),
        reason: format!(
            "Relay {} could not be resolved",
            redacted_relay_label(relay)
        ),
    })?;
    let mut addresses = addresses.collect::<Vec<_>>();
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty() || addresses.len() > MAX_NOSTR_DNS_ADDRESSES {
        return Err(ChannelError::StartupFailed {
            name: "nostr".to_string(),
            reason: format!(
                "Relay {} resolved to an invalid number of addresses",
                redacted_relay_label(relay)
            ),
        });
    }

    let local_wss = parsed.scheme() == "wss" && relay_host_is_explicitly_local(host);
    let valid_addresses = if parsed.scheme() == "ws" || local_wss {
        addresses
            .iter()
            .all(|address| is_safe_private_relay_ip(address.ip()))
    } else {
        addresses
            .iter()
            .all(|address| thinclaw_tools_core::is_public_outbound_ip(address.ip()))
    };
    if !valid_addresses {
        return Err(ChannelError::StartupFailed {
            name: "nostr".to_string(),
            reason: format!(
                "Relay {} resolved outside its permitted network boundary",
                redacted_relay_label(relay)
            ),
        });
    }
    Ok(())
}

fn relay_host_is_explicitly_local(host: &str) -> bool {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_safe_private_relay_ip(ip);
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || !host.contains('.')
}

fn is_safe_private_relay_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_loopback(),
    }
}

fn redacted_relay_label(relay: &str) -> String {
    let Ok(parsed) = url::Url::parse(relay) else {
        return "<invalid-relay>".to_string();
    };
    let Some(host) = parsed.host_str() else {
        return "<invalid-relay>".to_string();
    };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    match parsed.port() {
        Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
        None => format!("{}://{host}", parsed.scheme()),
    }
}

fn validate_outbound_message(message: &str) -> Result<(), ChannelError> {
    if message.trim().is_empty() {
        return Err(ChannelError::InvalidMessage(
            "Nostr messages cannot be empty".to_string(),
        ));
    }
    if message.len() > MAX_NOSTR_MESSAGE_BYTES {
        return Err(ChannelError::MessageTooLong {
            channel: "nostr".to_string(),
            length: message.chars().count(),
            max: MAX_NOSTR_MESSAGE_BYTES,
        });
    }
    Ok(())
}

fn event_is_bounded(event: &Event) -> bool {
    if event.content.len() > MAX_NOSTR_MESSAGE_BYTES || event.tags.len() > MAX_NOSTR_EVENT_TAGS {
        return false;
    }
    let mut total_tag_bytes = 0usize;
    for tag in event.tags.iter() {
        let fields = tag.as_slice();
        if fields.len() > MAX_NOSTR_EVENT_TAG_FIELDS {
            return false;
        }
        for field in fields {
            let Some(next) = total_tag_bytes.checked_add(field.len()) else {
                return false;
            };
            total_tag_bytes = next;
            if total_tag_bytes > MAX_NOSTR_EVENT_TAG_BYTES {
                return false;
            }
        }
    }
    true
}

pub fn normalize_public_key(raw: &str) -> Result<String, String> {
    parse_public_key(raw).map(|pubkey| pubkey.to_hex())
}

pub fn parse_public_key(raw: &str) -> Result<PublicKey, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Nostr public key cannot be empty".to_string());
    }
    if trimmed.len() > MAX_NOSTR_KEY_BYTES || trimmed.chars().any(char::is_control) {
        return Err("Nostr public key is malformed or exceeds its size limit".to_string());
    }
    PublicKey::parse(trimmed)
        .or_else(|_| PublicKey::from_hex(trimmed))
        .map_err(|_| "Invalid Nostr public key".to_string())
}

fn parse_keys(config: &NostrConfig) -> Result<Keys, ChannelError> {
    let private_key = config.private_key.expose_secret().trim();
    if private_key.is_empty()
        || private_key.len() > MAX_NOSTR_KEY_BYTES
        || private_key.chars().any(char::is_control)
    {
        return Err(ChannelError::Configuration(
            "Nostr private key is malformed or exceeds its size limit".to_string(),
        ));
    }
    Keys::parse(private_key)
        .map_err(|_| ChannelError::Configuration("Invalid Nostr private key".to_string()))
}

fn state_sidecar_path(path: &Path) -> PathBuf {
    path.with_extension("json.thinclaw-unused-sidecar")
}

fn load_state(path: &Path) -> Result<PersistedRuntimeState, ChannelError> {
    let sidecar = state_sidecar_path(path);
    thinclaw_platform::recover_file_pair_sync(path, &sidecar).map_err(|error| {
        ChannelError::Configuration(format!("Failed to recover Nostr runtime state: {error}"))
    })?;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PersistedRuntimeState::default());
        }
        Err(error) => {
            return Err(ChannelError::Configuration(format!(
                "Failed to inspect Nostr runtime state: {error}"
            )));
        }
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_NOSTR_STATE_BYTES as u64
    {
        return Err(ChannelError::Configuration(
            "Nostr runtime state is not a bounded regular file".to_string(),
        ));
    }

    let _guard = thinclaw_platform::acquire_artifact_read_lock_sync(path).map_err(|error| {
        ChannelError::Configuration(format!("Failed to lock Nostr runtime state: {error}"))
    })?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).map_err(|error| {
        ChannelError::Configuration(format!("Failed to open Nostr runtime state: {error}"))
    })?;
    let opened_metadata = file.metadata().map_err(|error| {
        ChannelError::Configuration(format!("Failed to inspect open Nostr state: {error}"))
    })?;
    if !opened_metadata.is_file() || opened_metadata.len() > MAX_NOSTR_STATE_BYTES as u64 {
        return Err(ChannelError::Configuration(
            "Opened Nostr runtime state is not a bounded regular file".to_string(),
        ));
    }
    let mut body = Vec::new();
    file.by_ref()
        .take((MAX_NOSTR_STATE_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|error| {
            ChannelError::Configuration(format!("Failed to read Nostr runtime state: {error}"))
        })?;
    if body.len() > MAX_NOSTR_STATE_BYTES {
        return Err(ChannelError::Configuration(
            "Nostr runtime state exceeds its size limit".to_string(),
        ));
    }
    let state: PersistedRuntimeState = serde_json::from_slice(&body).map_err(|error| {
        ChannelError::Configuration(format!("Nostr runtime state is malformed: {error}"))
    })?;
    validate_state(&state)?;
    Ok(state)
}

fn validate_state(state: &PersistedRuntimeState) -> Result<(), ChannelError> {
    let valid_event_ids = state.seen_control_event_ids.len() <= MAX_SEEN_CONTROL_EVENTS
        && state
            .seen_control_event_ids
            .iter()
            .all(|id| id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let valid_protocols = state.recipient_dm_protocols.len() <= MAX_PROTOCOL_PREFERENCES
        && state
            .recipient_dm_protocols
            .iter()
            .all(|(pubkey, protocol)| {
                pubkey.len() == 64
                    && PublicKey::from_hex(pubkey).is_ok()
                    && NostrDmProtocol::parse(protocol).is_some()
            });
    if !valid_event_ids || !valid_protocols {
        return Err(ChannelError::Configuration(
            "Nostr runtime state contains invalid or oversized entries".to_string(),
        ));
    }
    Ok(())
}

async fn save_state(path: &Path, state: &PersistedRuntimeState) -> Result<(), ChannelError> {
    validate_state(state)?;

    let body = serde_json::to_vec_pretty(state).map_err(|err| ChannelError::SendFailed {
        name: "nostr".to_string(),
        reason: format!("Failed to serialize Nostr runtime state: {err}"),
    })?;

    if body.len() > MAX_NOSTR_STATE_BYTES {
        return Err(ChannelError::SendFailed {
            name: "nostr".to_string(),
            reason: "Serialized Nostr runtime state exceeds its size limit".to_string(),
        });
    }
    thinclaw_platform::publish_file_pair(
        path.to_path_buf(),
        state_sidecar_path(path),
        body,
        None,
        thinclaw_platform::ExistingPairPolicy::Replace,
    )
    .await
    .map_err(|err| ChannelError::SendFailed {
        name: "nostr".to_string(),
        reason: format!("Failed to persist Nostr runtime state: {err}"),
    })
}

pub fn serialize_event(event: &Event) -> serde_json::Value {
    serde_json::json!({
        "id": event.id.to_hex(),
        "kind": event.kind.as_u16(),
        "kind_name": event.kind.to_string(),
        "author_pubkey": event.pubkey.to_hex(),
        "author_npub": event.pubkey.to_bech32().unwrap_or_else(|_| event.pubkey.to_hex()),
        "created_at": event.created_at.as_secs(),
        "content": event.content,
        "tags": event.tags.iter().map(|tag| tag.as_slice().to_vec()).collect::<Vec<_>>(),
    })
}

fn serialize_dm_message(message: &NostrInboundDm) -> serde_json::Value {
    serde_json::json!({
        "sender_pubkey": message.sender_hex,
        "sender_npub": message.sender_npub,
        "content": message.content,
        "protocol": message.protocol.as_str(),
        "envelope_event_id": message.envelope_event_id,
        "dm_event_id": message.dm_event_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn normalize_public_key_accepts_hex() {
        let normalized = normalize_public_key(
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        )
        .unwrap();
        assert_eq!(
            normalized,
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        );
    }

    #[test]
    fn runtime_config_rejects_invalid_relay_boundaries() {
        let base = NostrConfig {
            private_key: secrecy::SecretString::from(
                "0000000000000000000000000000000000000000000000000000000000000001",
            ),
            relays: vec![],
            owner_pubkey: None,
            social_dm_enabled: false,
            allow_from: vec![],
        };
        assert!(NostrRuntime::new(&base).is_err());

        let mut credentials = base.clone();
        credentials.relays = vec!["wss://user:secret@relay.example/".to_string()];
        assert!(NostrRuntime::new(&credentials).is_err());

        let mut fragment = base;
        fragment.relays = vec!["wss://relay.example/#secret".to_string()];
        assert!(NostrRuntime::new(&fragment).is_err());
    }

    #[test]
    fn nostr_debug_output_redacts_relay_secrets() {
        let config = NostrConfig {
            private_key: secrecy::SecretString::from("private-secret"),
            relays: vec!["wss://relay.example/private-path?token=secret".to_string()],
            owner_pubkey: None,
            social_dm_enabled: false,
            allow_from: vec![],
        };
        let debug = format!("{config:?}");
        assert!(debug.contains("wss://relay.example"));
        assert!(!debug.contains("private-secret"));
        assert!(!debug.contains("private-path"));
        assert!(!debug.contains("token"));
        assert!(!debug.contains("secret"));
    }

    #[test]
    fn public_key_errors_do_not_echo_input() {
        let error = parse_public_key("sensitive-invalid-key").unwrap_err();
        assert!(!error.contains("sensitive-invalid-key"));
        assert!(parse_public_key(&"a".repeat(MAX_NOSTR_KEY_BYTES + 1)).is_err());
    }

    #[tokio::test]
    async fn mark_control_event_seen_deduplicates_and_persists() {
        let dir = tempdir().unwrap();
        let state_path = dir.path().join("nostr-runtime-state.json");
        let keys = Keys::generate();

        let runtime = NostrRuntime {
            client: Client::new(keys.clone()),
            keys,
            relays: vec![],
            owner_pubkey: None,
            social_dm_enabled: false,
            state_path: state_path.clone(),
            connected: Mutex::new(false),
            state: RwLock::new(PersistedRuntimeState::default()),
        };

        let event_id = EventId::from_hex(&"f".repeat(64)).expect("should parse event id");
        assert!(runtime.mark_control_event_seen(&event_id).await.unwrap());
        assert!(!runtime.mark_control_event_seen(&event_id).await.unwrap());

        let persisted = std::fs::read_to_string(state_path).unwrap();
        assert!(persisted.contains(&event_id.to_hex()));
    }

    #[tokio::test]
    async fn remember_protocol_updates_state() {
        let dir = tempdir().unwrap();
        let state_path = dir.path().join("nostr-runtime-state.json");
        let keys = Keys::generate();
        let runtime = NostrRuntime {
            client: Client::new(keys.clone()),
            keys,
            relays: vec![],
            owner_pubkey: None,
            social_dm_enabled: false,
            state_path,
            connected: Mutex::new(false),
            state: RwLock::new(PersistedRuntimeState::default()),
        };
        let pubkey =
            PublicKey::from_hex("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .unwrap();

        runtime
            .remember_protocol(&pubkey, NostrDmProtocol::Nip04)
            .await
            .unwrap();

        assert_eq!(
            runtime.preferred_protocol(&pubkey).await,
            Some(NostrDmProtocol::Nip04)
        );
    }

    #[test]
    fn load_state_rejects_corruption_and_unknown_fields() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, b"not-json").unwrap();
        assert!(load_state(&path).is_err());

        std::fs::write(
            &path,
            br#"{"seen_control_event_ids":[],"recipient_dm_protocols":{},"unexpected":true}"#,
        )
        .unwrap();
        assert!(load_state(&path).is_err());
    }

    #[tokio::test]
    async fn failed_replay_state_write_does_not_mutate_memory() {
        let dir = tempdir().unwrap();
        let keys = Keys::generate();
        let runtime = NostrRuntime {
            client: Client::new(keys.clone()),
            keys,
            relays: vec![],
            owner_pubkey: None,
            social_dm_enabled: false,
            // Publishing a regular state file over this directory must fail.
            state_path: dir.path().to_path_buf(),
            connected: Mutex::new(false),
            state: RwLock::new(PersistedRuntimeState::default()),
        };
        let event_id = EventId::from_hex(&"a".repeat(64)).unwrap();
        assert!(runtime.mark_control_event_seen(&event_id).await.is_err());
        assert!(runtime.state.read().await.seen_control_event_ids.is_empty());
    }

    #[tokio::test]
    async fn concurrent_replay_updates_are_all_persisted() {
        let dir = tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let keys = Keys::generate();
        let runtime = std::sync::Arc::new(NostrRuntime {
            client: Client::new(keys.clone()),
            keys,
            relays: vec![],
            owner_pubkey: None,
            social_dm_enabled: false,
            state_path: state_path.clone(),
            connected: Mutex::new(false),
            state: RwLock::new(PersistedRuntimeState::default()),
        });

        let mut handles = Vec::new();
        for index in 1_u64..=16 {
            let runtime = std::sync::Arc::clone(&runtime);
            let event_id = EventId::from_hex(&format!("{index:064x}")).unwrap();
            handles.push(tokio::spawn(async move {
                runtime.mark_control_event_seen(&event_id).await
            }));
        }
        for handle in handles {
            assert!(handle.await.unwrap().unwrap());
        }

        let persisted = load_state(&state_path).unwrap();
        assert_eq!(persisted.seen_control_event_ids.len(), 16);
    }
}
