//! User settings persistence.
//!
//! Stores user preferences in ~/.thinclaw/settings.json.
//! Settings are loaded with env var > settings.json > default priority.

use std::collections::HashMap;
use std::path::PathBuf;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::tools::policy::ToolPolicyManager;

mod agent;
mod builder;
mod channels;
mod desktop_autonomy;
mod embeddings;
mod experiments;
mod extensions;
mod heartbeat;
mod learning;
mod notifications;
mod onboarding;
mod prompt;
mod providers;
mod root;
mod safety;
mod sandbox;
mod serialization;
mod settings_impl;
mod tunnel;
mod wasm;

pub use agent::*;
pub use builder::*;
pub use channels::*;
pub use desktop_autonomy::*;
pub use embeddings::*;
pub use experiments::*;
pub use extensions::*;
pub use heartbeat::*;
pub use learning::*;
pub use notifications::*;
pub use onboarding::*;
pub use prompt::*;
pub use providers::*;
pub use root::*;
pub use safety::*;
pub use sandbox::*;
pub use tunnel::*;
pub use wasm::*;

use serialization::*;

#[cfg(test)]
mod tests;
