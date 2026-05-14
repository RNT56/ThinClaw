pub use thinclaw_channels::nostr::*;

#[cfg(feature = "nostr")]
pub fn runtime_config_from_resolved(
    config: crate::config::NostrConfig,
) -> thinclaw_channels::NostrConfig {
    thinclaw_channels::NostrConfig {
        private_key: config.private_key,
        relays: config.relays,
        owner_pubkey: config.owner_pubkey,
        social_dm_enabled: config.social_dm_enabled,
        allow_from: config.allow_from,
    }
}

#[cfg(feature = "nostr")]
pub fn runtime_config_from_resolved_ref(
    config: &crate::config::NostrConfig,
) -> thinclaw_channels::NostrConfig {
    thinclaw_channels::NostrConfig {
        private_key: config.private_key.clone(),
        relays: config.relays.clone(),
        owner_pubkey: config.owner_pubkey.clone(),
        social_dm_enabled: config.social_dm_enabled,
        allow_from: config.allow_from.clone(),
    }
}
