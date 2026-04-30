//! DM pairing for channels.
//!
//! Gates DMs from unknown senders. Only approved senders can message the agent.
//! Unknown senders receive a pairing code and must be approved via `thinclaw pairing approve`.
//!
//! OpenClaw reference: src/pairing/pairing-store.ts

pub use thinclaw_channels::pairing::{
    PairingRequest, PairingStore, PairingStoreError, UpsertResult,
};
