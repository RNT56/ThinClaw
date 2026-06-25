//! Channel-specific setup flows.
//!
//! Each channel (Telegram, HTTP, etc.) has its own setup function that:
//! 1. Displays setup instructions
//! 2. Collects configuration (tokens, ports, etc.)
//! 3. Validates the configuration
//! 4. Saves secrets to the database
//!
//! This module is a façade: the shared error/secrets types live in
//! [`error`]/[`secrets_context`], and each channel concern owns its own
//! submodule. Public paths (`crate::setup::channels::...`) are preserved via
//! the `pub use` re-exports below.

mod error;
mod http;
mod secrets_context;
mod signal;
mod telegram;
mod tunnel;
mod wasm;

pub use error::ChannelSetupError;
pub use http::setup_http;
pub use secrets_context::SecretsContext;
pub use signal::setup_signal;
pub use telegram::{setup_telegram, validate_telegram_token};
pub use tunnel::setup_tunnel;
pub use wasm::{WasmChannelSetupResult, setup_wasm_channel};

// Result types are part of the public setup-function signatures. They are
// re-exported here so callers can still name them via `setup::channels::...`,
// even though no in-crate caller imports them by that path today.
#[allow(unused_imports)]
pub use http::HttpSetupResult;
#[allow(unused_imports)]
pub use signal::SignalSetupResult;
#[allow(unused_imports)]
pub use telegram::TelegramSetupResult;
