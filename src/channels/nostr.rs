//! Nostr channel via NIP-04 encrypted DMs.
//!
//! Connects to a set of Nostr relays and listens for NIP-04 encrypted
//! direct messages addressed to the bot's public key. Replies are sent
//! back as NIP-04 encrypted DMs.
//!
//! NIP-04 is the legacy DM protocol (kind 4). The newer NIP-17 (gift-wrapped
//! DMs via NIP-59) requires the `nip59` feature which we don't enable.
//! NIP-04 is widely supported and simpler to implement.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nostr_sdk::prelude::*;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::config::NostrConfig;
use crate::error::ChannelError;

/// Nostr channel using NIP-04 encrypted DMs.
pub struct NostrChannel {
    config: NostrConfig,
    client: Arc<RwLock<Option<Client>>>,
}

impl NostrChannel {
    /// Create a new Nostr channel from configuration.
    pub fn new(config: NostrConfig) -> Result<Self, ChannelError> {
        Ok(Self {
            config,
            client: Arc::new(RwLock::new(None)),
        })
    }

    /// Parse the private key from hex or nsec bech32 format.
    fn parse_keys(config: &NostrConfig) -> Result<Keys, ChannelError> {
        use secrecy::ExposeSecret;
        let key_str = config.private_key.expose_secret();

        Keys::parse(key_str)
            .map_err(|e| ChannelError::Configuration(format!("Invalid Nostr private key: {e}")))
    }

    /// Generate a deterministic thread ID from a public key.
    fn thread_id_from_pubkey(pubkey: &PublicKey) -> String {
        Uuid::new_v5(&Uuid::NAMESPACE_URL, pubkey.to_hex().as_bytes()).to_string()
    }

    /// Build and send a NIP-04 encrypted DM event.
    async fn send_nip04_dm(
        client: &Client,
        keys: &Keys,
        recipient: &PublicKey,
        plaintext: &str,
    ) -> Result<(), ChannelError> {
        // Encrypt with NIP-04
        let encrypted = nip04::encrypt(keys.secret_key(), recipient, plaintext).map_err(|e| {
            ChannelError::SendFailed {
                name: "nostr".to_string(),
                reason: format!("NIP-04 encryption failed: {e}"),
            }
        })?;

        // Build kind-4 event with encrypted content and "p" tag
        let builder = EventBuilder::new(Kind::EncryptedDirectMessage, encrypted)
            .tag(Tag::public_key(*recipient));

        // Sign and send
        let event =
            client
                .sign_event_builder(builder)
                .await
                .map_err(|e| ChannelError::SendFailed {
                    name: "nostr".to_string(),
                    reason: format!("Failed to sign event: {e}"),
                })?;

        client
            .send_event(&event)
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "nostr".to_string(),
                reason: format!("Failed to send event: {e}"),
            })?;

        Ok(())
    }
}

#[async_trait]
impl Channel for NostrChannel {
    fn name(&self) -> &str {
        "nostr"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let keys = Self::parse_keys(&self.config)?;
        let our_pubkey = keys.public_key();

        let client = Client::new(keys.clone());

        // Add relays
        for relay_url in &self.config.relays {
            if let Err(e) = client.add_relay(relay_url.as_str()).await {
                tracing::warn!(relay = %relay_url, error = %e, "Failed to add Nostr relay");
            }
        }

        // Connect to all relays
        client.connect().await;

        // Wait for at least one connection
        client.wait_for_connection(Duration::from_secs(10)).await;

        tracing::info!(
            relays = self.config.relays.len(),
            pubkey = %our_pubkey.to_bech32().unwrap_or_else(|_| our_pubkey.to_hex()),
            "Nostr channel connected"
        );

        // Subscribe to NIP-04 encrypted DMs addressed to us
        // subscribe() takes a single Filter, not a Vec
        let dm_filter = Filter::new()
            .kind(Kind::EncryptedDirectMessage)
            .pubkey(our_pubkey)
            .since(Timestamp::now());

        if let Err(e) = client.subscribe(dm_filter, None).await {
            tracing::error!(error = %e, "Failed to subscribe to Nostr DMs");
        }

        // Store client for respond()
        *self.client.write().await = Some(client.clone());

        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let allow_from = self.config.allow_from.clone();

        // Spawn listener
        tokio::spawn(async move {
            let result = client
                .handle_notifications(|notification| {
                    let tx = tx.clone();
                    let keys = keys.clone();
                    let allow_from = allow_from.clone();

                    async move {
                        if let RelayPoolNotification::Event { event, .. } = notification {
                            // Only handle NIP-04 encrypted DMs (kind 4)
                            if event.kind != Kind::EncryptedDirectMessage {
                                return Ok(false);
                            }

                            let sender_pubkey = event.pubkey;
                            let sender_hex = sender_pubkey.to_hex();

                            // Check allowlist (empty = accept all, matching other channels)
                            let allowed = if allow_from.is_empty() {
                                true
                            } else {
                                let bech32 = sender_pubkey.to_bech32().unwrap_or_default();
                                allow_from
                                    .iter()
                                    .any(|e| e == "*" || e == &sender_hex || e == &bech32)
                            };

                            if !allowed {
                                tracing::debug!(
                                    sender = %sender_hex,
                                    "Nostr: sender not in allow_from, dropping"
                                );
                                return Ok(false);
                            }

                            // Decrypt NIP-04 message
                            let secret_key = keys.secret_key();
                            let plaintext =
                                match nip04::decrypt(secret_key, &sender_pubkey, &event.content) {
                                    Ok(text) => text,
                                    Err(e) => {
                                        tracing::warn!(
                                            sender = %sender_hex,
                                            error = %e,
                                            "Nostr: failed to decrypt NIP-04 message"
                                        );
                                        return Ok(false);
                                    }
                                };

                            if plaintext.is_empty() {
                                return Ok(false);
                            }

                            let thread_id = NostrChannel::thread_id_from_pubkey(&sender_pubkey);

                            let metadata = serde_json::json!({
                                "nostr_pubkey": sender_hex,
                                "nostr_event_id": event.id.to_hex(),
                            });

                            let msg = IncomingMessage::new("nostr", &sender_hex, plaintext)
                                .with_thread(thread_id)
                                .with_metadata(metadata)
                                .with_user_name(
                                    sender_pubkey
                                        .to_bech32()
                                        .unwrap_or_else(|_| sender_hex.clone()),
                                );

                            if tx.send(msg).await.is_err() {
                                tracing::debug!("Nostr: message channel closed");
                                return Ok(true); // Stop
                            }
                        }
                        Ok(false) // Continue
                    }
                })
                .await;

            if let Err(e) = result {
                tracing::error!(error = %e, "Nostr notification handler exited");
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let client_guard = self.client.read().await;

        let client = client_guard
            .as_ref()
            .ok_or_else(|| ChannelError::NotConnected("Nostr client not connected".to_string()))?;

        // Get recipient pubkey from metadata
        let recipient_hex = msg
            .metadata
            .get("nostr_pubkey")
            .and_then(|v| v.as_str())
            .unwrap_or(&msg.user_id);

        let recipient_pubkey =
            PublicKey::from_hex(recipient_hex).map_err(|e| ChannelError::SendFailed {
                name: "nostr".to_string(),
                reason: format!("Invalid recipient pubkey: {e}"),
            })?;

        // We need keys to encrypt the reply
        let keys = Self::parse_keys(&self.config)?;

        Self::send_nip04_dm(client, &keys, &recipient_pubkey, &response.content).await
    }

    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Nostr doesn't support typing indicators
        Ok(())
    }

    fn formatting_hints(&self) -> Option<String> {
        Some(
            "- Nostr clients often render plain text only. Keep formatting light.\n\
- Prefer concise paragraphs and avoid tables or heavy markdown."
                .to_string(),
        )
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Validate recipient: must be a valid Nostr public key (hex or npub).
        // Skip gracefully for non-pubkey identifiers like "default".
        let recipient_pubkey = if let Ok(pk) = PublicKey::from_hex(user_id) {
            pk
        } else if let Ok(pk) = PublicKey::parse(user_id) {
            pk
        } else {
            tracing::debug!(
                recipient = user_id,
                "Nostr: skipping broadcast — recipient is not a valid pubkey"
            );
            return Ok(());
        };

        let client_guard = self.client.read().await;
        let client = client_guard
            .as_ref()
            .ok_or_else(|| ChannelError::NotConnected("Nostr client not connected".to_string()))?;

        let keys = Self::parse_keys(&self.config)?;
        Self::send_nip04_dm(client, &keys, &recipient_pubkey, &response.content).await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        let client = self.client.read().await;
        match client.as_ref() {
            Some(c) => {
                let relays = c.relays().await;
                if relays.is_empty() {
                    return Err(ChannelError::NotConnected(
                        "No Nostr relays connected".to_string(),
                    ));
                }
                Ok(())
            }
            None => Err(ChannelError::NotConnected(
                "Nostr client not initialized".to_string(),
            )),
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        if let Some(client) = self.client.write().await.take() {
            client.shutdown().await;
            tracing::info!("Nostr channel shut down");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    #[test]
    fn test_thread_id_deterministic() {
        let pubkey =
            PublicKey::from_hex("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .unwrap();
        let id1 = NostrChannel::thread_id_from_pubkey(&pubkey);
        let id2 = NostrChannel::thread_id_from_pubkey(&pubkey);
        assert_eq!(id1, id2);
        assert!(Uuid::parse_str(&id1).is_ok());
    }

    #[test]
    fn test_name_is_nostr() {
        let config = NostrConfig {
            private_key: SecretString::from("0000000000000000000000000000000000000000000000000000000000000001"),
            relays: vec!["wss://relay.example".into()],
            allow_from: vec![],
        };
        let channel = NostrChannel::new(config).unwrap();
        assert_eq!(channel.name(), "nostr");
    }

    #[test]
    fn test_parse_keys_rejects_invalid_secret() {
        let config = NostrConfig {
            private_key: SecretString::from("not-a-secret"),
            relays: vec![],
            allow_from: vec![],
        };
        assert!(NostrChannel::parse_keys(&config).is_err());
    }

    #[test]
    fn test_parse_keys_accepts_valid_hex_private_key() {
        let config = NostrConfig {
            private_key: SecretString::from("0000000000000000000000000000000000000000000000000000000000000001"),
            relays: vec![],
            allow_from: vec![],
        };
        assert!(NostrChannel::parse_keys(&config).is_ok());
    }

    #[test]
    fn test_thread_id_for_different_pubkeys() {
        let pubkey1 =
            PublicKey::from_hex("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .unwrap();
        let pubkey2 =
            PublicKey::from_hex("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81799")
                .unwrap();
        assert_ne!(
            NostrChannel::thread_id_from_pubkey(&pubkey1),
            NostrChannel::thread_id_from_pubkey(&pubkey2)
        );
    }

    #[tokio::test]
    async fn test_broadcast_skips_non_pubkey_recipients() {
        let channel = NostrChannel::new(NostrConfig {
            private_key: SecretString::from("0000000000000000000000000000000000000000000000000000000000000001"),
            relays: vec![],
            allow_from: vec![],
        })
        .unwrap();

        let result = channel.broadcast("default", OutgoingResponse::text("hello")).await;
        assert!(result.is_ok());
    }
}
