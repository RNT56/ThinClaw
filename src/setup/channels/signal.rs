//! Signal channel setup.

use thinclaw_channels::setup as channel_setup;
use url::Url;

use crate::settings::Settings;
use crate::setup::prompts::{
    input, optional_input, print_blank_line, print_error, print_info, print_success,
};

use super::ChannelSetupError;

/// Result of Signal channel setup.
#[derive(Debug, Clone)]
pub struct SignalSetupResult {
    pub enabled: bool,
    pub http_url: String,
    pub account: String,
    pub allow_from: String,
    pub allow_from_groups: String,
    pub dm_policy: String,
    pub group_policy: String,
    pub group_allow_from: String,
}

/// `Settings` is reserved for future use
pub async fn setup_signal(_settings: &Settings) -> Result<SignalSetupResult, ChannelSetupError> {
    print_info("Signal channel setup");
    print_blank_line();
    print_info("Signal channel connects to a signal-cli daemon running in HTTP mode.");
    print_blank_line();

    let http_url = input("Signal-cli HTTP URL")?;
    match Url::parse(&http_url) {
        Ok(url) if url.scheme() == "http" || url.scheme() == "https" => {}
        Ok(_) => {
            print_error("URL must use http or https scheme");
            return Err(ChannelSetupError::Validation(
                "Invalid HTTP URL: must use http or https scheme".to_string(),
            ));
        }
        Err(e) => {
            print_error(&format!("Invalid URL: {}", e));
            return Err(ChannelSetupError::Validation(format!(
                "Invalid HTTP URL: {}",
                e
            )));
        }
    }

    let account = input("Signal account (E.164)")?;
    if let Err(e) = channel_setup::validate_e164_account(&account) {
        print_error(&e);
        return Err(ChannelSetupError::Validation(e));
    }

    let allow_from = optional_input(
        "Allow from (comma-separated: E.164 numbers, '*' for anyone, UUIDs or 'uuid:<id>'; empty for self-only)",
        Some(&format!("default: {} (self-only)", account)),
    )?
    .unwrap_or_else(|| account.clone());

    let dm_policy = optional_input(
        "DM policy (open, allowlist, pairing)",
        Some("default: pairing"),
    )?
    .unwrap_or_else(|| "pairing".to_string());

    let allow_from_groups = optional_input(
        "Allow from groups (comma-separated group IDs, '*' for any group; empty for none)",
        Some("default: (none)"),
    )?
    .unwrap_or_default();

    let group_policy = optional_input(
        "Group policy (allowlist, open, disabled)",
        Some("default: allowlist"),
    )?
    .unwrap_or_else(|| "allowlist".to_string());

    let group_allow_from = optional_input(
        "Group allow from (comma-separated member IDs; empty to inherit from allow_from)",
        Some("default: (inherit from allow_from)"),
    )?
    .unwrap_or_default();

    if let Err(e) = channel_setup::validate_allow_from_list(&allow_from) {
        print_error(&e);
        return Err(ChannelSetupError::Validation(e));
    }

    if let Err(e) = channel_setup::validate_allow_from_groups_list(&allow_from_groups) {
        print_error(&e);
        return Err(ChannelSetupError::Validation(e));
    }

    crate::setup::prompts::print_blank_line();
    print_success(&format!(
        "Signal channel configured for account: {}",
        account
    ));
    print_info(&format!("HTTP URL: {}", http_url));
    if allow_from == account {
        print_info("Allow from: self-only");
    } else {
        print_info(&format!("Allow from: {}", allow_from));
    }
    print_info(&format!("DM policy: {}", dm_policy));
    if allow_from_groups.is_empty() {
        print_info("Allow from groups: (none)");
    } else {
        print_info(&format!("Allow from groups: {}", allow_from_groups));
    }
    print_info(&format!("Group policy: {}", group_policy));
    if group_allow_from.is_empty() {
        print_info("Group allow from: (inherits from allow_from)");
    } else {
        print_info(&format!("Group allow from: {}", group_allow_from));
    }

    Ok(SignalSetupResult {
        enabled: true,
        http_url,
        account,
        allow_from,
        allow_from_groups,
        dm_policy,
        group_policy,
        group_allow_from,
    })
}
