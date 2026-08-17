//! Channel management CLI commands.
//!
//! Subcommands:
//! - `channels list` — list all configured channels and their status
//! - `channels info` — show channel details

use clap::Subcommand;
use serde::Serialize;

use super::{CliContext, CliError, CliOutcome, GatewayClient, GatewayClientError};
use crate::channels::catalog::ChannelCatalogEntry;
use thinclaw_app::capabilities::{FactState, HealthState, ProbeOutcome};

const GATEWAY_STATUS_ENDPOINT: &str = "/api/gateway/status";

#[derive(Subcommand, Debug, Clone)]
pub enum ChannelCommand {
    /// List all configured channels and their status
    List {
        /// Deprecated command-local output selector; use global --output-format.
        #[arg(long, hide = true)]
        format: Option<String>,
    },

    /// Show details for a specific channel
    Info {
        /// Channel name (e.g. "telegram", "signal", "gateway")
        channel: String,
        /// Select one exact driver variant (native, wasm, or local_surface).
        #[arg(long)]
        variant: Option<String>,
    },

    /// Check static configuration without making network requests.
    CheckConfig {
        channel: String,
        #[arg(long)]
        variant: Option<String>,
    },

    /// Deprecated alias for `check-config`.
    #[command(hide = true)]
    Validate {
        /// Channel name (e.g. "matrix", "telegram", "twilio_sms")
        channel: String,
    },

    /// Run bounded, side-effect-minimized live health probes.
    Probe {
        channel: Option<String>,
        #[arg(long, conflicts_with = "channel", required_unless_present = "channel")]
        all: bool,
    },
}

/// Run a channels CLI command.
pub async fn run_channels_command(
    cmd: ChannelCommand,
    context: &CliContext,
) -> Result<CliOutcome, CliError> {
    match cmd {
        ChannelCommand::List { format } => list_channels(format.as_deref(), context).await,
        ChannelCommand::Info { channel, variant } => {
            channel_info(&channel, variant.as_deref(), context).await
        }
        ChannelCommand::CheckConfig { channel, variant } => {
            check_channel_config(&channel, variant.as_deref(), context).await
        }
        ChannelCommand::Validate { channel } => {
            if !context.output().format.is_machine() {
                context.output().diagnostic(
                    "warning: `channels validate` is deprecated; use `extensions channels check-config` (removed in 0.19)",
                )?;
            }
            check_channel_config(&channel, None, context).await
        }
        ChannelCommand::Probe { channel, all } => {
            probe_channels(channel.as_deref(), all, context).await
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ChannelStaticReport {
    service_id: String,
    driver_id: String,
    origin: String,
    compiled: FactState,
    configured: FactState,
    installed: FactState,
    registered: FactState,
    health: HealthState,
    description: String,
    reasons: Vec<String>,
}

async fn list_channels(
    legacy_format: Option<&str>,
    context: &CliContext,
) -> Result<CliOutcome, CliError> {
    if let Some(format) = legacy_format {
        if format != "table" && format != "json" {
            return Err(CliError::usage("--format must be table or json"));
        }
        if !context.output().format.is_machine() {
            context.output().diagnostic(
                "warning: command-local --format is deprecated; use global --output-format (removed in 0.19)",
            )?;
        }
    }
    let resolved = context.config().await?;
    let reports = channel_reports(resolved);
    context
        .output()
        .write_record("extensions.channels.list", &reports, |reports| {
            let mut text = format!(
                "{:<20} {:<16} {:<10} {}\n",
                "SERVICE", "DRIVER", "CONFIG", "DESCRIPTION"
            );
            for report in reports {
                text.push_str(&format!(
                    "{:<20} {:<16} {:<10} {}\n",
                    report.service_id,
                    report.driver_id,
                    fact_label(report.configured),
                    report.description
                ));
            }
            text
        })?;
    Ok(CliOutcome::Success)
}

fn channel_reports(config: &crate::config::Config) -> Vec<ChannelStaticReport> {
    let mut catalog = crate::channels::catalog::static_channel_catalog();
    merge_installed_wasm(&mut catalog);
    catalog.sort_by(|left, right| (&left.id, &left.variant).cmp(&(&right.id, &right.variant)));
    catalog
        .into_iter()
        .map(|entry| {
            let configured = channel_configuration_state(config, &entry.id, &entry.variant);
            let installed = if entry.variant == "wasm" {
                bool_fact(wasm_artifact_exists(&entry.id))
            } else {
                FactState::NotApplicable
            };
            ChannelStaticReport {
                service_id: entry.id.clone(),
                driver_id: format!("{}:{}", entry.variant, entry.id),
                origin: entry.origin,
                compiled: bool_fact(entry.compiled),
                configured,
                installed,
                registered: FactState::Unknown,
                health: HealthState::NotProbed,
                description: entry.description,
                reasons: if entry.compiled {
                    Vec::new()
                } else {
                    vec!["not_compiled".to_string()]
                },
            }
        })
        .collect()
}

fn fact_label(value: FactState) -> &'static str {
    match value {
        FactState::Yes => "yes",
        FactState::No => "no",
        FactState::Unknown => "unknown",
        FactState::NotApplicable => "n/a",
    }
}

const fn bool_fact(value: bool) -> FactState {
    if value { FactState::Yes } else { FactState::No }
}

fn wasm_artifact_exists(name: &str) -> bool {
    crate::platform::state_paths()
        .channels_dir
        .join(format!("{name}.wasm"))
        .is_file()
}

fn merge_installed_wasm(catalog: &mut Vec<ChannelCatalogEntry>) {
    let wasm_dir = crate::platform::state_paths().channels_dir;
    let Ok(entries) = std::fs::read_dir(wasm_dir) else {
        return;
    };
    for entry in entries.take(4_096).flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "wasm")
            || !std::fs::symlink_metadata(&path)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        {
            continue;
        }
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if !catalog
            .iter()
            .any(|candidate| candidate.id == name && candidate.variant == "wasm")
        {
            catalog.push(ChannelCatalogEntry {
                id: name,
                variant: "wasm".to_string(),
                origin: "installed".to_string(),
                description: "Installed WASM channel".to_string(),
                compiled: cfg!(feature = "wasm-runtime"),
                local_surface: false,
            });
        }
    }
}

/// Show details for a specific channel.
async fn channel_info(
    channel: &str,
    variant: Option<&str>,
    context: &CliContext,
) -> Result<CliOutcome, CliError> {
    validate_channel_id(channel)?;
    let config = context.config().await?;
    let report = select_channel_report(channel_reports(config), channel, variant)?;
    context.output().write_record("extensions.channels.info", &report, |report| {
        format!(
            "Service: {}\nDriver: {}\nCompiled: {}\nConfigured: {}\nInstalled: {}\nRegistered: {}\nHealth: {:?}\nDescription: {}",
            report.service_id,
            report.driver_id,
            fact_label(report.compiled),
            fact_label(report.configured),
            fact_label(report.installed),
            fact_label(report.registered),
            report.health,
            report.description,
        )
    })?;
    Ok(CliOutcome::Success)
}

async fn check_channel_config(
    channel: &str,
    variant: Option<&str>,
    context: &CliContext,
) -> Result<CliOutcome, CliError> {
    validate_channel_id(channel)?;
    let config = context.config().await?;
    let mut report = select_channel_report(channel_reports(config), channel, variant)?;
    if report.compiled == FactState::No {
        report.reasons.push("not_compiled".to_string());
    }
    if report.configured == FactState::No && !report.driver_id.starts_with("local_surface:") {
        report.reasons.push("not_configured".to_string());
    }
    if report.driver_id == "native:linq" {
        let (_, unresolved) = linq_secret_binding_state(config);
        if !unresolved.is_empty() {
            report.reasons.push(format!(
                "unresolved_secret_bindings:{}",
                unresolved.join(",")
            ));
        }
    }
    if report.driver_id.starts_with("wasm:") {
        validate_wasm_channel_installation(channel, &crate::platform::state_paths().channels_dir)
            .map_err(|error| {
            report.reasons.push("invalid_installation".to_string());
            CliError::operational(error.to_string())
        })?;
    }
    if report.driver_id.starts_with("native:") {
        let missing = native_lifecycle_missing_env(channel);
        if !missing.is_empty() {
            report
                .reasons
                .push(format!("missing_bindings:{}", missing.join(",")));
        }
    }
    let healthy = report.compiled != FactState::No
        && report.configured != FactState::No
        && report.reasons.is_empty();
    context
        .output()
        .write_record("extensions.channels.check-config", &report, |report| {
            format!(
                "{} ({})\ncompiled: {}\nconfigured: {}\nstatic check: {}{}",
                report.service_id,
                report.driver_id,
                fact_label(report.compiled),
                fact_label(report.configured),
                if healthy { "ready" } else { "not ready" },
                if report.reasons.is_empty() {
                    String::new()
                } else {
                    format!("\nreasons: {}", report.reasons.join(", "))
                }
            )
        })?;
    Ok(if healthy {
        CliOutcome::Success
    } else {
        CliOutcome::Unhealthy
    })
}

#[derive(Debug, Serialize)]
struct ChannelProbeReport {
    service_id: String,
    driver_id: String,
    probe: ProbeOutcome,
}

async fn probe_channels(
    channel: Option<&str>,
    all: bool,
    context: &CliContext,
) -> Result<CliOutcome, CliError> {
    if !all && channel.is_none() {
        return Err(CliError::usage("provide CHANNEL or --all"));
    }
    if let Some(channel) = channel {
        validate_channel_id(channel)?;
    }
    let config = context.config().await?;
    let selected: Vec<ChannelStaticReport> = channel_reports(config)
        .into_iter()
        .filter(|report| all || channel == Some(report.service_id.as_str()))
        .collect();
    if selected.is_empty() {
        return Err(CliError::usage("unknown channel or driver"));
    }

    let client = GatewayClient::resolve_from_config(None, None, config)
        .map_err(|error| CliError::operational(error.to_string()))?;
    let checked_at = chrono::Utc::now().to_rfc3339();
    let status = fetch_gateway_channel_status(&client).await;
    let mut reports = Vec::with_capacity(selected.len());
    let mut unhealthy = false;
    match status {
        Ok(status) => {
            for report in selected {
                let (health, reason) = live_channel_health(&status, &report.service_id);
                unhealthy |= health == HealthState::Unhealthy;
                reports.push(ChannelProbeReport {
                    service_id: report.service_id,
                    driver_id: report.driver_id,
                    probe: ProbeOutcome {
                        health,
                        origin: "running_gateway".to_string(),
                        checked_at: Some(checked_at.clone()),
                        reason,
                    },
                });
            }
        }
        Err(error) => {
            unhealthy = true;
            for report in selected {
                reports.push(ChannelProbeReport {
                    service_id: report.service_id,
                    driver_id: report.driver_id,
                    probe: ProbeOutcome {
                        health: HealthState::Unhealthy,
                        origin: "running_gateway".to_string(),
                        checked_at: Some(checked_at.clone()),
                        reason: Some(format!("gateway_unreachable:{error}")),
                    },
                });
            }
        }
    }
    context
        .output()
        .write_record("extensions.channels.probe", &reports, |reports| {
            reports
                .iter()
                .map(|report| {
                    format!(
                        "{} ({}) {:?}{}",
                        report.service_id,
                        report.driver_id,
                        report.probe.health,
                        report
                            .probe
                            .reason
                            .as_deref()
                            .map(|reason| format!(" — {reason}"))
                            .unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })?;
    Ok(if unhealthy {
        CliOutcome::Unhealthy
    } else {
        CliOutcome::Success
    })
}

async fn fetch_gateway_channel_status(
    client: &GatewayClient,
) -> Result<serde_json::Value, GatewayClientError> {
    client
        .get_json(GATEWAY_STATUS_ENDPOINT, &[] as &[(&str, &str)])
        .await
}

fn live_channel_health(status: &serde_json::Value, channel: &str) -> (HealthState, Option<String>) {
    if channel == "gateway" {
        return (HealthState::Healthy, None);
    }
    let Some(setup) = status
        .get("channel_setup")
        .and_then(|setup| setup.get(channel))
    else {
        return (
            HealthState::NotSupported,
            Some("safe_probe_not_supported".to_string()),
        );
    };
    if setup.get("configured").and_then(serde_json::Value::as_bool) != Some(true) {
        return (HealthState::Unknown, Some("not_configured".to_string()));
    }
    if channel == "linq" {
        return if setup.get("tool_ready").and_then(serde_json::Value::as_bool) == Some(true) {
            (HealthState::Healthy, None)
        } else {
            (
                HealthState::Unhealthy,
                Some("runtime_not_active".to_string()),
            )
        };
    }
    if let Some(relay_health) = setup
        .get("relay_health")
        .and_then(serde_json::Value::as_str)
    {
        if matches!(relay_health, "healthy" | "ok" | "connected") {
            return (HealthState::Healthy, None);
        }
        return (
            HealthState::Unhealthy,
            Some(format!("relay:{relay_health}")),
        );
    }
    if setup.get("tool_ready").and_then(serde_json::Value::as_bool) == Some(true)
        || setup
            .get("control_ready")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    {
        return (HealthState::Healthy, None);
    }
    (
        HealthState::Unknown,
        Some("configured_but_no_safe_health_probe".to_string()),
    )
}

fn select_channel_report(
    reports: Vec<ChannelStaticReport>,
    channel: &str,
    variant: Option<&str>,
) -> Result<ChannelStaticReport, CliError> {
    let mut matches: Vec<_> = reports
        .into_iter()
        .filter(|report| {
            report.service_id == channel
                && variant
                    .is_none_or(|variant| report.driver_id.starts_with(&format!("{variant}:")))
        })
        .collect();
    if matches.is_empty() {
        return Err(CliError::usage("unknown channel or driver variant"));
    }
    if matches.len() > 1 {
        let drivers = matches
            .iter()
            .map(|report| report.driver_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(CliError::usage(format!(
            "channel is ambiguous; select --variant ({drivers})"
        )));
    }
    Ok(matches.remove(0))
}

fn validate_channel_id(channel: &str) -> Result<(), CliError> {
    if channel.is_empty()
        || channel.len() > 128
        || !channel.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        return Err(CliError::usage(
            "channel must be a bounded lowercase ASCII identifier",
        ));
    }
    Ok(())
}

fn validate_wasm_channel_installation(
    channel: &str,
    wasm_dir: &std::path::Path,
) -> anyhow::Result<()> {
    if channel.is_empty()
        || channel.len() > 128
        || !channel.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        anyhow::bail!("channel name is not a bounded lowercase ASCII identifier");
    }
    let wasm_path = wasm_dir.join(format!("{channel}.wasm"));
    let caps_path = wasm_dir.join(format!("{channel}.capabilities.json"));
    let wasm =
        thinclaw_platform::read_regular_file_bounded_single_link(&wasm_path, 64 * 1024 * 1024)
            .map_err(|_| anyhow::anyhow!("WASM artifact is missing or invalid"))?;
    if wasm.len() < 8 || !wasm.starts_with(b"\0asm") {
        anyhow::bail!("WASM artifact has an invalid module header");
    }

    let raw = thinclaw_platform::read_regular_file_bounded_single_link(&caps_path, 1024 * 1024)
        .map_err(|_| anyhow::anyhow!("capabilities file is missing or invalid"))?;
    let caps = crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(&raw)?;
    // Static validation deliberately checks only the declared schema and
    // artifact. Credential values are resolved only by the owning runtime.
    for secret in &caps.setup.required_secrets {
        if secret.name.is_empty()
            || secret.name.len() > 128
            || secret.name.chars().any(char::is_control)
        {
            anyhow::bail!("capabilities file declares an invalid secret binding name");
        }
    }
    Ok(())
}

fn channel_is_configured(config: &crate::config::Config, name: &str, variant: &str) -> bool {
    if variant == "local_surface" {
        return true;
    }
    if variant == "wasm" {
        return wasm_artifact_exists(name);
    }
    match name {
        "gateway" => config.channels.gateway.is_some(),
        "signal" => config.channels.signal.is_some(),
        "matrix" => config.channels.matrix_enabled,
        "voice-call" => config.channels.voice_call_enabled && config.channels.voice_call_available,
        "apns" => config.channels.apns_enabled,
        "browser-push" => {
            config.channels.browser_push_enabled && config.channels.browser_push_available
        }
        "nostr" => config.channels.nostr.is_some(),
        "http" => config.channels.http.is_some(),
        "telegram" => config.channels.telegram.is_some(),
        "slack" => config.channels.slack.is_some(),
        "discord" => config.channels.discord.is_some(),
        "gmail" => config.channels.gmail.is_some(),
        "linq" => config.channels.linq.is_some(),
        "bluebubbles" => config.channels.bluebubbles.is_some(),
        "imessage" => {
            #[cfg(target_os = "macos")]
            {
                config.channels.imessage.is_some()
            }
            #[cfg(not(target_os = "macos"))]
            {
                false
            }
        }
        "apple_mail" => {
            #[cfg(target_os = "macos")]
            {
                config.channels.apple_mail.is_some()
            }
            #[cfg(not(target_os = "macos"))]
            {
                false
            }
        }
        _ => false,
    }
}

fn channel_configuration_state(
    config: &crate::config::Config,
    name: &str,
    variant: &str,
) -> FactState {
    if name == "linq" && variant == "native" {
        return linq_secret_binding_state(config).0;
    }
    bool_fact(channel_is_configured(config, name, variant))
}

fn linq_secret_binding_state(config: &crate::config::Config) -> (FactState, Vec<&'static str>) {
    let Some(linq) = config.channels.linq.as_ref() else {
        return (FactState::No, Vec::new());
    };
    // Presence is sufficient for this no-I/O static check; validation and
    // injection remain owned by the runtime credential consumer.
    let api_key_present = linq.api_key.is_some();
    let webhook_secret_present = linq.webhook_secret.is_some();
    linq_secret_binding_state_from_presence(api_key_present, webhook_secret_present)
}

fn linq_secret_binding_state_from_presence(
    api_key_present: bool,
    webhook_secret_present: bool,
) -> (FactState, Vec<&'static str>) {
    let mut unresolved = Vec::new();
    if !api_key_present {
        unresolved.push(crate::channels::LINQ_API_KEY_SECRET);
    }
    if !webhook_secret_present {
        unresolved.push(crate::channels::LINQ_WEBHOOK_SECRET);
    }
    let state = if unresolved.is_empty() {
        FactState::Yes
    } else {
        // `check-config` is deliberately static and must not open the encrypted
        // store (which may be remote). Absence from env/config is unresolved,
        // not proof that the runtime-scoped credential is missing.
        FactState::Unknown
    };
    (state, unresolved)
}

fn native_lifecycle_missing_env(name: &str) -> Vec<String> {
    let required: &[(&str, &[&str])] = match name {
        "matrix" => &[
            ("MATRIX_HOMESERVER", &["MATRIX_HOMESERVER"]),
            ("MATRIX_ACCESS_TOKEN", &["MATRIX_ACCESS_TOKEN"]),
            ("MATRIX_WEBHOOK_SECRET", &["MATRIX_WEBHOOK_SECRET"]),
        ],
        "voice-call" => &[
            ("VOICE_CALL_RESPONSE_URL", &["VOICE_CALL_RESPONSE_URL"]),
            ("VOICE_CALL_WEBHOOK_SECRET", &["VOICE_CALL_WEBHOOK_SECRET"]),
        ],
        "apns" => &[
            ("APNS_TEAM_ID", &["APNS_TEAM_ID"]),
            ("APNS_KEY_ID", &["APNS_KEY_ID"]),
            ("APNS_BUNDLE_ID", &["APNS_BUNDLE_ID"]),
            (
                "APNS_PRIVATE_KEY or APNS_PRIVATE_KEY_PATH",
                &["APNS_PRIVATE_KEY", "APNS_PRIVATE_KEY_PATH"],
            ),
            ("APNS_REGISTRATION_SECRET", &["APNS_REGISTRATION_SECRET"]),
        ],
        "browser-push" => &[
            (
                "BROWSER_PUSH_VAPID_PUBLIC_KEY",
                &["BROWSER_PUSH_VAPID_PUBLIC_KEY"],
            ),
            (
                "BROWSER_PUSH_VAPID_PRIVATE_KEY or BROWSER_PUSH_VAPID_PRIVATE_KEY_PATH",
                &[
                    "BROWSER_PUSH_VAPID_PRIVATE_KEY",
                    "BROWSER_PUSH_VAPID_PRIVATE_KEY_PATH",
                ],
            ),
            (
                "BROWSER_PUSH_VAPID_SUBJECT",
                &["BROWSER_PUSH_VAPID_SUBJECT"],
            ),
            (
                "BROWSER_PUSH_WEBHOOK_SECRET",
                &["BROWSER_PUSH_WEBHOOK_SECRET"],
            ),
        ],
        _ => &[],
    };
    required
        .iter()
        .filter(|(_, alternatives)| {
            !alternatives.iter().any(|env_var| {
                crate::config::helpers::optional_env(env_var)
                    .ok()
                    .flatten()
                    .is_some_and(|value| !value.trim().is_empty())
            })
        })
        .map(|(label, _)| (*label).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        GATEWAY_STATUS_ENDPOINT, fetch_gateway_channel_status,
        linq_secret_binding_state_from_presence, live_channel_health,
        validate_wasm_channel_installation,
    };
    use crate::cli::{GatewayClient, GatewayRequestBudget};
    use thinclaw_app::capabilities::{FactState, HealthState};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn probe_uses_the_authenticated_gateway_status_contract() {
        assert_eq!(GATEWAY_STATUS_ENDPOINT, "/api/gateway/status");
    }

    #[tokio::test]
    async fn probe_status_request_uses_the_gateway_contract_path() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback listener");
        let address = listener.local_addr().expect("listener address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let mut request = vec![0_u8; 4096];
            let read = stream.read(&mut request).await.expect("read request");
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("GET /api/gateway/status HTTP/1.1\r\n"));
            let body = r#"{"channel_setup":{}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });
        let client = GatewayClient::new(
            url::Url::parse(&format!("http://{address}/")).expect("valid loopback URL"),
            None,
            GatewayRequestBudget::control_plane(),
        )
        .expect("gateway client");

        let status = fetch_gateway_channel_status(&client)
            .await
            .expect("status response");
        assert_eq!(status, serde_json::json!({"channel_setup": {}}));
        server.await.expect("mock server task");
    }

    #[test]
    fn linq_static_config_does_not_claim_store_backed_secrets_are_present() {
        let (state, unresolved) = linq_secret_binding_state_from_presence(false, false);
        assert_eq!(state, FactState::Unknown);
        assert_eq!(unresolved, vec!["linq_api_key", "linq_webhook_secret"]);

        let (state, unresolved) = linq_secret_binding_state_from_presence(true, false);
        assert_eq!(state, FactState::Unknown);
        assert_eq!(unresolved, vec!["linq_webhook_secret"]);

        let (state, unresolved) = linq_secret_binding_state_from_presence(true, true);
        assert_eq!(state, FactState::Yes);
        assert!(unresolved.is_empty());
    }

    #[test]
    fn linq_live_status_requires_runtime_diagnostics() {
        let configured = serde_json::json!({
            "channel_setup": {
                "linq": {"enabled": true, "configured": true, "tool_ready": true}
            }
        });
        assert_eq!(
            live_channel_health(&configured, "linq"),
            (HealthState::Healthy, None)
        );

        let inactive = serde_json::json!({
            "channel_setup": {
                "linq": {"enabled": true, "configured": true}
            }
        });
        assert_eq!(
            live_channel_health(&inactive, "linq"),
            (
                HealthState::Unhealthy,
                Some("runtime_not_active".to_string())
            )
        );

        let legacy = serde_json::json!({"channel_setup": {}});
        assert_eq!(
            live_channel_health(&legacy, "linq"),
            (
                HealthState::NotSupported,
                Some("safe_probe_not_supported".to_string())
            )
        );
    }

    #[test]
    fn wasm_channel_validation_requires_artifact_and_capabilities() {
        let temp = tempfile::tempdir().expect("temp dir");
        let err = validate_wasm_channel_installation("missing", temp.path())
            .expect_err("missing artifact should fail");
        assert!(err.to_string().contains("WASM artifact"));
    }

    #[test]
    fn wasm_channel_static_validation_does_not_read_secret_values() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(temp.path().join("demo.wasm"), b"\0asm\x01\0\0\0").expect("write wasm");
        std::fs::write(
            temp.path().join("demo.capabilities.json"),
            r#"{
                "name": "demo",
                "setup": {
                    "required_secrets": [
                        {"name": "demo_missing_token", "prompt": "Token"},
                        {"name": "demo_optional", "prompt": "Optional", "optional": true}
                    ]
                }
            }"#,
        )
        .expect("write capabilities");

        validate_wasm_channel_installation("demo", temp.path())
            .expect("static check validates declarations without reading secret values");
    }

    #[test]
    fn wasm_channel_validation_rejects_path_like_names() {
        let temp = tempfile::tempdir().expect("temp dir");
        assert!(validate_wasm_channel_installation("../../outside", temp.path()).is_err());
    }
}
