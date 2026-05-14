//! Shared Nostr runtime used by both the owner-control channel and the Nostr tool.

use std::collections::{HashMap, VecDeque};
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

#[derive(Debug, Clone)]
pub struct NostrConfig {
    pub private_key: SecretString,
    pub relays: Vec<String>,
    pub owner_pubkey: Option<String>,
    pub social_dm_enabled: bool,
    pub allow_from: Vec<String>,
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
        f.debug_struct("NostrRuntime")
            .field("relays", &self.relays)
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
        let keys = parse_keys(config)?;
        let owner_pubkey = config
            .owner_pubkey
            .as_deref()
            .map(parse_public_key)
            .transpose()
            .map_err(ChannelError::Configuration)?;
        let state_path = thinclaw_platform::state_paths()
            .home
            .join("nostr-runtime-state.json");
        let state = load_state(&state_path);

        Ok(Self {
            client: Client::new(keys.clone()),
            keys,
            relays: config.relays.clone(),
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
            return Ok(());
        }

        for relay_url in &self.relays {
            if let Err(err) = self.client.add_relay(relay_url.as_str()).await {
                tracing::warn!(relay = %relay_url, error = %err, "Failed to add Nostr relay");
            }
        }

        self.client.connect().await;
        self.client.wait_for_connection(CONNECT_TIMEOUT).await;
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

    pub async fn mark_control_event_seen(&self, event_id: &EventId) -> bool {
        let id = event_id.to_hex();
        let next_state = {
            let mut state = self.state.write().await;
            if state.seen_control_event_ids.iter().any(|seen| seen == &id) {
                return false;
            }
            state.seen_control_event_ids.push_back(id);
            while state.seen_control_event_ids.len() > MAX_SEEN_CONTROL_EVENTS {
                state.seen_control_event_ids.pop_front();
            }
            state.clone()
        };
        let _ = save_state(&self.state_path, &next_state).await;
        true
    }

    pub async fn remember_protocol(
        &self,
        pubkey: &PublicKey,
        protocol: NostrDmProtocol,
    ) -> Result<(), ChannelError> {
        let next_state = {
            let mut state = self.state.write().await;
            state
                .recipient_dm_protocols
                .insert(pubkey.to_hex(), protocol.as_str().to_string());
            while state.recipient_dm_protocols.len() > MAX_PROTOCOL_PREFERENCES {
                if let Some(oldest) = state.recipient_dm_protocols.keys().next().cloned() {
                    state.recipient_dm_protocols.remove(&oldest);
                } else {
                    break;
                }
            }
            state.clone()
        };
        save_state(&self.state_path, &next_state).await
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
        if plaintext.trim().is_empty() {
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
        let UnwrappedGift { sender, rumor } = match self.client.unwrap_gift_wrap(event).await {
            Ok(gift) => gift,
            Err(err) => {
                tracing::warn!(error = %err, "Nostr: failed to unwrap gift-wrap DM");
                return Ok(None);
            }
        };

        if rumor.kind != Kind::PrivateDirectMessage || rumor.content.trim().is_empty() {
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
        Ok(events.into_iter().next())
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

        let mut related_ids: Vec<EventId> = target.tags.event_ids().copied().collect();
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
            for reply in events {
                if !replies
                    .iter()
                    .any(|existing: &Event| existing.id == reply.id)
                {
                    replies.push(reply);
                }
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
                    .limit(limit.max(1))
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
            "mentions": events.into_iter().map(|event| serialize_event(&event)).collect::<Vec<_>>(),
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
                    .limit(limit.max(1)),
                FETCH_TIMEOUT,
            )
            .await
            .map_err(|err| ChannelError::Disconnected {
                name: "nostr".to_string(),
                reason: format!("Failed to fetch NIP-04 inbox: {err}"),
            })?;
        for event in nip04_events {
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
                    .limit(limit.max(1)),
                FETCH_TIMEOUT,
            )
            .await
            .map_err(|err| ChannelError::Disconnected {
                name: "nostr".to_string(),
                reason: format!("Failed to fetch gift-wrap inbox: {err}"),
            })?;
        for event in gift_wrap_events {
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
        self.client.relays().await.len()
    }

    pub async fn shutdown(&self) {
        self.client.shutdown().await;
        *self.connected.lock().await = false;
    }
}

pub fn normalize_public_key(raw: &str) -> Result<String, String> {
    parse_public_key(raw).map(|pubkey| pubkey.to_hex())
}

pub fn parse_public_key(raw: &str) -> Result<PublicKey, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Nostr public key cannot be empty".to_string());
    }
    PublicKey::parse(trimmed)
        .or_else(|_| PublicKey::from_hex(trimmed))
        .map_err(|err| format!("Invalid Nostr public key '{trimmed}': {err}"))
}

fn parse_keys(config: &NostrConfig) -> Result<Keys, ChannelError> {
    Keys::parse(config.private_key.expose_secret())
        .map_err(|err| ChannelError::Configuration(format!("Invalid Nostr private key: {err}")))
}

fn load_state(path: &PathBuf) -> PersistedRuntimeState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

async fn save_state(path: &PathBuf, state: &PersistedRuntimeState) -> Result<(), ChannelError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|err| ChannelError::SendFailed {
                name: "nostr".to_string(),
                reason: format!("Failed to prepare Nostr state directory: {err}"),
            })?;
    }

    let body = serde_json::to_vec_pretty(state).map_err(|err| ChannelError::SendFailed {
        name: "nostr".to_string(),
        reason: format!("Failed to serialize Nostr runtime state: {err}"),
    })?;

    tokio::fs::write(path, body)
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
        assert!(runtime.mark_control_event_seen(&event_id).await);
        assert!(!runtime.mark_control_event_seen(&event_id).await);

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
}
