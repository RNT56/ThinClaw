//! Nostr owner-control channel.
//!
//! This channel accepts command ingress only from one explicit owner pubkey over
//! encrypted Nostr DMs. It uses a shared Nostr runtime so outbound DM sending
//! and the `nostr_actions` tool share the same connection, key material, and
//! protocol preferences.

use std::sync::Arc;

use async_trait::async_trait;
use nostr_sdk::prelude::*;
use uuid::Uuid;

use crate::channels::nostr_runtime::{NostrDmProtocol, NostrRuntime, parse_public_key};
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::config::NostrConfig;
use crate::error::ChannelError;

pub struct NostrChannel {
    config: NostrConfig,
    runtime: Arc<NostrRuntime>,
}

impl NostrChannel {
    pub fn new(config: NostrConfig) -> Result<Self, ChannelError> {
        let runtime = Arc::new(NostrRuntime::new(&config)?);
        Self::new_with_runtime(config, runtime)
    }

    pub fn new_with_runtime(
        config: NostrConfig,
        runtime: Arc<NostrRuntime>,
    ) -> Result<Self, ChannelError> {
        Ok(Self { config, runtime })
    }

    pub fn runtime(&self) -> Arc<NostrRuntime> {
        Arc::clone(&self.runtime)
    }

    fn thread_id_from_pubkey(pubkey: &PublicKey) -> String {
        Uuid::new_v5(&Uuid::NAMESPACE_URL, pubkey.to_hex().as_bytes()).to_string()
    }
}

#[async_trait]
impl Channel for NostrChannel {
    fn name(&self) -> &str {
        "nostr"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        self.runtime.ensure_connected().await?;

        tracing::info!(
            relays = self.config.relays.len(),
            pubkey = %self.runtime.public_key_npub(),
            owner_pubkey = ?self.runtime.owner_pubkey_npub(),
            social_dm_enabled = self.runtime.social_dm_enabled(),
            "Nostr channel connected"
        );

        for filter in self.runtime.control_filters() {
            if let Err(err) = self.runtime.client().subscribe(filter, None).await {
                tracing::error!(error = %err, "Failed to subscribe to Nostr control DMs");
            }
        }

        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let runtime = Arc::clone(&self.runtime);
        let owner_pubkey = runtime.owner_pubkey_hex();

        tokio::spawn(async move {
            let result = runtime
                .client()
                .handle_notifications(|notification| {
                    let tx = tx.clone();
                    let runtime = Arc::clone(&runtime);
                    let owner_pubkey = owner_pubkey.clone();

                    async move {
                        if let RelayPoolNotification::Event { event, .. } = notification {
                            if !runtime.is_supported_dm_kind(event.kind) {
                                return Ok(false);
                            }

                            if !runtime.mark_control_event_seen(&event.id).await {
                                return Ok(false);
                            }

                            let inbound = match runtime.decrypt_inbound_dm(&event).await {
                                Ok(Some(dm)) => dm,
                                Ok(None) => return Ok(false),
                                Err(err) => {
                                    tracing::warn!(error = %err, "Nostr: failed to decode inbound DM");
                                    return Ok(false);
                                }
                            };

                            if owner_pubkey.as_deref() != Some(inbound.sender_hex.as_str()) {
                                tracing::debug!(
                                    sender = %inbound.sender_hex,
                                    "Nostr: dropping DM from non-owner sender"
                                );
                                return Ok(false);
                            }

                            if let Err(err) = runtime
                                .remember_protocol(&inbound.sender, inbound.protocol)
                                .await
                            {
                                tracing::warn!(error = %err, "Nostr: failed to persist DM protocol preference");
                            }

                            let metadata = serde_json::json!({
                                "nostr_pubkey": inbound.sender_hex,
                                "nostr_sender_npub": inbound.sender_npub,
                                "nostr_dm_protocol": inbound.protocol.as_str(),
                                "nostr_envelope_event_id": inbound.envelope_event_id,
                                "nostr_event_id": inbound.dm_event_id,
                            });

                            let message = IncomingMessage::new(
                                "nostr",
                                &inbound.sender_hex,
                                inbound.content,
                            )
                            .with_thread(NostrChannel::thread_id_from_pubkey(&inbound.sender))
                            .with_metadata(metadata)
                            .with_user_name(inbound.sender_npub);

                            if tx.send(message).await.is_err() {
                                tracing::debug!("Nostr: message channel closed");
                                return Ok(true);
                            }
                        }

                        Ok(false)
                    }
                })
                .await;

            if let Err(err) = result {
                tracing::error!(error = %err, "Nostr notification handler exited");
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let recipient = msg
            .metadata
            .get("nostr_pubkey")
            .and_then(|value| value.as_str())
            .unwrap_or(&msg.user_id);
        let recipient_pubkey =
            parse_public_key(recipient).map_err(|message| ChannelError::SendFailed {
                name: "nostr".to_string(),
                reason: message,
            })?;
        let protocol = msg
            .metadata
            .get("nostr_dm_protocol")
            .and_then(|value| value.as_str())
            .and_then(NostrDmProtocol::parse);

        self.runtime
            .send_reply(&recipient_pubkey, &response.content, protocol)
            .await?;
        Ok(())
    }

    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    fn formatting_hints(&self) -> Option<String> {
        Some(
            "- Nostr clients often render plain text only. Keep formatting light.\n\
- Use send_message(platform=\"nostr\") for DMs and nostr_actions for public posts or social interactions."
                .to_string(),
        )
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let recipient = parse_public_key(user_id).map_err(|message| ChannelError::SendFailed {
            name: "nostr".to_string(),
            reason: message,
        })?;

        self.runtime
            .send_dm(&recipient, &response.content, None)
            .await?;
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        if self.runtime.connected_relay_count().await == 0 {
            return Err(ChannelError::NotConnected(
                "No Nostr relays connected".to_string(),
            ));
        }
        Ok(())
    }

    async fn diagnostics(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "public_key_hex": self.runtime.public_key_hex(),
            "public_key_npub": self.runtime.public_key_npub(),
            "owner_pubkey_hex": self.runtime.owner_pubkey_hex(),
            "owner_pubkey_npub": self.runtime.owner_pubkey_npub(),
            "relay_count": self.runtime.relay_count(),
            "connected_relay_count": self.runtime.connected_relay_count().await,
            "control_ready": self.runtime.owner_pubkey().is_some(),
            "social_dm_enabled": self.runtime.social_dm_enabled(),
        }))
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.runtime.shutdown().await;
        tracing::info!("Nostr channel shut down");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    fn sample_config() -> NostrConfig {
        NostrConfig {
            private_key: SecretString::from(
                "0000000000000000000000000000000000000000000000000000000000000001",
            ),
            relays: vec!["wss://relay.example".into()],
            owner_pubkey: None,
            social_dm_enabled: false,
            allow_from: vec![],
        }
    }

    #[test]
    fn thread_id_is_deterministic() {
        let pubkey =
            PublicKey::from_hex("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .unwrap();
        assert_eq!(
            NostrChannel::thread_id_from_pubkey(&pubkey),
            NostrChannel::thread_id_from_pubkey(&pubkey)
        );
    }

    #[test]
    fn name_is_nostr() {
        let channel = NostrChannel::new(sample_config()).unwrap();
        assert_eq!(channel.name(), "nostr");
    }

    #[test]
    fn runtime_is_shared() {
        let config = sample_config();
        let runtime = Arc::new(NostrRuntime::new(&config).unwrap());
        let channel = NostrChannel::new_with_runtime(config, Arc::clone(&runtime)).unwrap();
        assert_eq!(channel.runtime().public_key_hex(), runtime.public_key_hex());
    }
}
