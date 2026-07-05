//! Device identity operator CLI (`thinclaw devices ...`).
//!
//! Talks to the local gateway's `/api/devices/*` HTTP surface — never a
//! direct store/registry dependency — the same way `thinclaw message` and
//! `thinclaw gateway access` do. Design authority:
//! `docs/MOBILE_SECURITY.md` (decisions D-P*/D-T*/D-X*/D-K*, §8 gateway
//! hardening) and `docs/MOBILE_APP.md` (device identity section).
//!
//! Subcommands:
//! - `devices pair` — start a pairing session, render the QR + human code,
//!   poll until the pairing completes or expires.
//! - `devices list` — table of paired devices.
//! - `devices rename` — rename a device by id (or unambiguous id prefix).
//! - `devices revoke` — revoke a device by id (or unambiguous id prefix).

use std::time::Duration;

use clap::Subcommand;
use serde::Deserialize;
// Re-exported server DTO field/type shapes are the wire contract; these are
// server-side `Serialize`-only types (no `Deserialize`), so the CLI mirrors
// their shape locally to parse responses. Field names below are kept in
// lockstep with `thinclaw_gateway::web::devices::types`.
use thinclaw_gateway::web::devices::DeviceScope;

use crate::platform::gateway_access::GatewayAccessInfo;
use crate::settings::Settings;
use crate::terminal_branding::TerminalBranding;

mod qr;

pub use qr::render_qr_unicode;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Mirrors `thinclaw_gateway::web::devices::PairStartResponse` (server DTO is
/// `Serialize`-only; the CLI is a client so it needs `Deserialize`).
#[derive(Debug, Clone, Deserialize)]
struct PairStartResponse {
    qr_uri: String,
    human_code: String,
    expires_at: i64,
    pairing_id: String,
}

/// Mirrors `thinclaw_gateway::web::devices::PendingPairInfo`.
#[derive(Debug, Clone, Deserialize)]
struct PendingPairInfo {
    pairing_id: String,
}

/// Mirrors `thinclaw_gateway::web::devices::PendingPairListResponse`.
#[derive(Debug, Clone, Deserialize)]
struct PendingPairListResponse {
    pending: Vec<PendingPairInfo>,
}

/// Mirrors `thinclaw_gateway::web::devices::DeviceInfo`.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
struct DeviceInfo {
    device_id: String,
    name: String,
    platform: String,
    #[allow(dead_code)]
    created_at: String,
    last_seen_at: String,
    #[allow(dead_code)]
    token_prefix: String,
    scopes: Vec<DeviceScope>,
    #[allow(dead_code)]
    has_pubkey: bool,
    revoked_at: Option<String>,
    #[allow(dead_code)]
    expires_at: Option<String>,
}

/// Mirrors `thinclaw_gateway::web::devices::DeviceListResponse`.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
struct DeviceListResponse {
    devices: Vec<DeviceInfo>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DeviceCommand {
    /// Start a pairing session and wait for a device to complete it
    Pair {
        /// Human label for the new device (shown in the pairing UI)
        #[arg(long, default_value = "New device")]
        name: String,
    },

    /// List paired devices
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Rename a paired device
    Rename {
        /// Device id, or an unambiguous prefix of one
        id: String,

        /// New display name
        name: String,
    },

    /// Revoke a paired device, disconnecting any live sessions
    Revoke {
        /// Device id, or an unambiguous prefix of one
        id: String,
    },
}

/// Run a `thinclaw devices` command.
pub async fn run_devices_command(cmd: DeviceCommand) -> anyhow::Result<()> {
    let settings = Settings::load();
    let access = GatewayAccessInfo::from_env_and_settings(Some(&settings));
    let client = build_client()?;

    match cmd {
        DeviceCommand::Pair { name } => run_pair(&client, &access, &name).await,
        DeviceCommand::List { json } => run_list(&client, &access, json).await,
        DeviceCommand::Rename { id, name } => run_rename(&client, &access, &id, &name).await,
        DeviceCommand::Revoke { id } => run_revoke(&client, &access, &id).await,
    }
}

fn build_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?)
}

fn auth_error() -> anyhow::Error {
    anyhow::anyhow!(
        "GATEWAY_AUTH_TOKEN is not configured. Set it (or configure a gateway token in \
         settings) before managing devices."
    )
}

fn connect_error(base: &str, source: reqwest::Error) -> anyhow::Error {
    if source.is_connect() {
        anyhow::anyhow!(
            "Could not connect to the gateway at {}. Is it running? Start with `thinclaw gateway start`.",
            base
        )
    } else if source.is_timeout() {
        anyhow::anyhow!("Timed out talking to the gateway at {}.", base)
    } else {
        anyhow::anyhow!("Request to the gateway failed: {}", source)
    }
}

async fn require_auth_request(
    client: &reqwest::Client,
    access: &GatewayAccessInfo,
    method: reqwest::Method,
    path: &str,
) -> anyhow::Result<reqwest::RequestBuilder> {
    let token = access.auth_token.as_ref().ok_or_else(auth_error)?;
    let url = format!("{}{}", access.api_base_url(), path);
    Ok(client.request(method, url).bearer_auth(token))
}

async fn send_and_parse<T: serde::de::DeserializeOwned>(
    request: reqwest::RequestBuilder,
    base: &str,
) -> anyhow::Result<T> {
    let response = request.send().await.map_err(|e| connect_error(base, e))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        anyhow::bail!("Gateway returned HTTP {}: {}", status.as_u16(), body);
    }

    serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("Could not parse gateway response: {} (body: {})", e, body))
}

async fn run_pair(
    client: &reqwest::Client,
    access: &GatewayAccessInfo,
    name: &str,
) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let base = access.api_base_url();

    branding.print_banner("Pair device", Some("Start a new device pairing session"));

    let request = require_auth_request(
        client,
        access,
        reqwest::Method::POST,
        "/api/devices/pair/start",
    )
    .await?
    .json(&serde_json::json!({ "name": name }));

    let start: PairStartResponse = send_and_parse(request, &base).await?;

    println!();
    println!("{}", render_qr_unicode(&start.qr_uri));
    println!();
    println!(
        "{}",
        branding.key_value("Human code", start.human_code.clone())
    );
    println!(
        "{}",
        branding.key_value("Expires at", format_unix(start.expires_at))
    );
    println!();
    println!(
        "{}",
        branding.muted("Scan the QR code (or enter the human code) from the ThinClaw app.")
    );
    println!("{}", branding.muted("Waiting for the device to pair..."));

    poll_pairing(client, access, &base, &start.pairing_id, start.expires_at).await
}

async fn poll_pairing(
    client: &reqwest::Client,
    access: &GatewayAccessInfo,
    base: &str,
    pairing_id: &str,
    expires_at: i64,
) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        let request = require_auth_request(
            client,
            access,
            reqwest::Method::GET,
            "/api/devices/pair/pending",
        )
        .await?;
        let pending: PendingPairListResponse = send_and_parse(request, base).await?;

        let still_pending = pending.pending.iter().any(|p| p.pairing_id == pairing_id);
        if !still_pending {
            println!("{}", branding.good("Device paired successfully."));
            return Ok(());
        }

        let now = chrono::Utc::now().timestamp();
        if now >= expires_at {
            println!(
                "{}",
                branding.warn("Pairing expired before it was completed.")
            );
            return Ok(());
        }
    }
}

async fn run_list(
    client: &reqwest::Client,
    access: &GatewayAccessInfo,
    json: bool,
) -> anyhow::Result<()> {
    let base = access.api_base_url();
    let request =
        require_auth_request(client, access, reqwest::Method::GET, "/api/devices").await?;
    let response: DeviceListResponse = send_and_parse(request, &base).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }

    println!("{}", render_devices_table(&response.devices));
    Ok(())
}

async fn run_rename(
    client: &reqwest::Client,
    access: &GatewayAccessInfo,
    id_prefix: &str,
    name: &str,
) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let base = access.api_base_url();
    let device_id = resolve_device_id(client, access, &base, id_prefix).await?;

    let request = require_auth_request(
        client,
        access,
        reqwest::Method::POST,
        &format!("/api/devices/{}/rename", device_id),
    )
    .await?
    .json(&serde_json::json!({ "name": name }));

    let _: serde_json::Value = send_and_parse(request, &base).await.or_else(|err| {
        // Some deployments may return an empty body on success; only bail if the
        // error indicates a real HTTP failure rather than a parse issue on an
        // empty/no-content response.
        if err.to_string().contains("Gateway returned HTTP") {
            Err(err)
        } else {
            Ok(serde_json::Value::Null)
        }
    })?;

    println!(
        "{}",
        branding.good(format!("Renamed device {} to '{}'.", device_id, name))
    );
    Ok(())
}

async fn run_revoke(
    client: &reqwest::Client,
    access: &GatewayAccessInfo,
    id_prefix: &str,
) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let base = access.api_base_url();
    let device_id = resolve_device_id(client, access, &base, id_prefix).await?;

    let request = require_auth_request(
        client,
        access,
        reqwest::Method::POST,
        &format!("/api/devices/{}/revoke", device_id),
    )
    .await?;

    let _: serde_json::Value = send_and_parse(request, &base).await.or_else(|err| {
        if err.to_string().contains("Gateway returned HTTP") {
            Err(err)
        } else {
            Ok(serde_json::Value::Null)
        }
    })?;

    println!(
        "{}",
        branding.good(format!("Revoked device {}.", device_id))
    );
    Ok(())
}

/// Resolve an unambiguous device id prefix to a full device id by listing
/// devices and matching. Errors if zero or more than one device matches.
async fn resolve_device_id(
    client: &reqwest::Client,
    access: &GatewayAccessInfo,
    base: &str,
    id_prefix: &str,
) -> anyhow::Result<String> {
    let request =
        require_auth_request(client, access, reqwest::Method::GET, "/api/devices").await?;
    let response: DeviceListResponse = send_and_parse(request, base).await?;

    let matches: Vec<&DeviceInfo> = response
        .devices
        .iter()
        .filter(|d| d.device_id.starts_with(id_prefix))
        .collect();

    match matches.as_slice() {
        [] => anyhow::bail!("No device matches id prefix '{}'.", id_prefix),
        [single] => Ok(single.device_id.clone()),
        multiple => {
            let ids: Vec<&str> = multiple.iter().map(|d| d.device_id.as_str()).collect();
            anyhow::bail!(
                "Device id prefix '{}' is ambiguous, matches: {}",
                id_prefix,
                ids.join(", ")
            )
        }
    }
}

fn format_unix(unix_seconds: i64) -> String {
    chrono::DateTime::from_timestamp(unix_seconds, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| unix_seconds.to_string())
}

/// Render a table of devices for terminal display.
fn render_devices_table(devices: &[DeviceInfo]) -> String {
    if devices.is_empty() {
        return "No paired devices.".to_string();
    }

    let mut out = String::new();
    out.push_str(&format!(
        "{:<10} {:<20} {:<10} {:<28} {:<24} {:<7}\n",
        "ID", "NAME", "PLATFORM", "SCOPES", "LAST SEEN", "REVOKED"
    ));
    for device in devices {
        let id_prefix: String = device.device_id.chars().take(8).collect();
        let scopes = device
            .scopes
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&format!(
            "{:<10} {:<20} {:<10} {:<28} {:<24} {:<7}\n",
            id_prefix,
            truncate(&device.name, 20),
            device.platform,
            scopes,
            device.last_seen_at,
            device.revoked_at.is_some(),
        ));
    }
    // Drop the trailing newline; println! in callers adds one.
    out.trim_end_matches('\n').to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        truncated.push('…');
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_devices_command_parse() {
        #[derive(clap::Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: DeviceCommand,
        }
        TestCli::command().debug_assert();
    }

    fn sample_device(id: &str, name: &str, revoked: bool) -> DeviceInfo {
        DeviceInfo {
            device_id: id.to_string(),
            name: name.to_string(),
            platform: "ios".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            last_seen_at: "2026-01-02T00:00:00Z".to_string(),
            token_prefix: "tcd_abcd".to_string(),
            scopes: DeviceScope::default_grant(),
            has_pubkey: false,
            revoked_at: if revoked {
                Some("2026-01-03T00:00:00Z".to_string())
            } else {
                None
            },
            expires_at: None,
        }
    }

    #[test]
    fn test_render_devices_table_empty() {
        assert_eq!(render_devices_table(&[]), "No paired devices.");
    }

    #[test]
    fn test_render_devices_table_contains_devices() {
        let devices = vec![
            sample_device(
                "11111111-aaaa-bbbb-cccc-000000000000",
                "Alice's iPhone",
                false,
            ),
            sample_device("22222222-aaaa-bbbb-cccc-000000000000", "Watch", true),
        ];
        let table = render_devices_table(&devices);
        assert!(table.contains("Alice's iPhone"));
        assert!(table.contains("Watch"));
        assert!(table.contains("11111111"));
        assert!(table.contains("22222222"));
        assert!(table.contains("ios"));
        assert!(table.contains("true"));
        assert!(table.contains("false"));
    }

    #[test]
    fn test_truncate_short_string_unchanged() {
        assert_eq!(truncate("short", 20), "short");
    }

    #[test]
    fn test_truncate_long_string_ellipsis() {
        let truncated = truncate("this is a very long device name", 10);
        assert_eq!(truncated.chars().count(), 10);
        assert!(truncated.ends_with('…'));
    }
}
