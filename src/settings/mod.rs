//! User settings persistence.
//!
//! Stores user preferences in ~/.thinclaw/settings.json.
//! Settings are loaded with env var > settings.json > default priority.

use std::collections::HashMap;
use std::path::PathBuf;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::tools::policy::ToolPolicyManager;

fn default_true() -> bool {
    true
}

fn default_telegram_subagent_session_mode() -> String {
    "temp_topic".to_string()
}

fn default_telegram_transport_mode() -> String {
    "auto".to_string()
}

fn default_webchat_theme() -> String {
    "system".to_string()
}

fn default_observability_backend() -> String {
    "none".to_string()
}

fn default_experiments_ui_visibility() -> String {
    "hidden_until_enabled".to_string()
}

fn default_experiments_promotion_mode() -> String {
    "branch_pr_draft".to_string()
}

fn default_prompt_project_context_max_tokens() -> usize {
    8_000
}

fn default_extensions_user_tools_dir() -> String {
    crate::platform::resolve_data_dir("user-tools")
        .to_string_lossy()
        .to_string()
}

mod agent;
mod channels;
mod embeddings;
mod heartbeat;
mod learning;
mod onboarding;
mod product;
mod providers;
mod root;
mod runtime;
mod serialization;
mod settings_impl;

pub use agent::*;
pub use channels::*;
pub use embeddings::*;
pub use heartbeat::*;
pub use learning::*;
pub use onboarding::*;
pub use product::*;
pub use providers::*;
pub use root::*;
pub use runtime::*;

use serialization::*;

#[cfg(test)]
mod tests;
