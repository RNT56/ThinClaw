//! Local gateway exposure through Tailscale Serve or Funnel.
//!
//! The embedded Desktop runtime mounts an authenticated loopback gateway. This
//! module is the deliberate network-exposure boundary around it: private
//! tailnet access is the default, public Funnel requires a separate explicit
//! confirmation, and no Tailscale auth keys or arbitrary shell input cross IPC.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;
use thinclaw_core::tunnel::{TailscaleTunnel, Tunnel};
use tokio::process::Command;
use tokio::sync::Mutex;

use super::bridge::{gated, BridgeError, RouteMode};
use super::runtime_bridge::ThinClawRuntimeState;
use super::ThinClawManager;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const GATEWAY_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAccessExposure {
    Tailnet,
    Public,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RemoteAccessStartRequest {
    pub exposure: RemoteAccessExposure,
    /// Public Funnel changes the trust boundary and must be acknowledged for
    /// each start. The private tailnet mode never requires this flag.
    pub confirm_public: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RemoteAccessStatus {
    pub runtime_mode: String,
    pub gateway_running: bool,
    pub gateway_port: u16,
    pub gateway_url: String,
    pub tailscale_installed: bool,
    pub tailscale_authenticated: bool,
    pub tailscale_dns_name: Option<String>,
    pub tailscale_error: Option<String>,
    pub tunnel_running: bool,
    pub exposure: Option<RemoteAccessExposure>,
    pub access_url: Option<String>,
}

struct ActiveTunnel {
    tunnel: Arc<TailscaleTunnel>,
    exposure: RemoteAccessExposure,
    access_url: String,
}

/// App-wide Tailscale process owner. Keeping this outside the embedded agent
/// lets app shutdown stop/reset Serve or Funnel even if the agent is restarted.
pub struct RemoteAccessState {
    active: Mutex<Option<ActiveTunnel>>,
}

impl RemoteAccessState {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(None),
        }
    }

    pub async fn shutdown(&self) {
        if let Some(active) = self.active.lock().await.take() {
            if let Err(error) = stop_tunnel(&active.tunnel).await {
                tracing::warn!(%error, "Failed to stop Tailscale exposure during shutdown");
            }
        }
    }
}

async fn stop_tunnel(tunnel: &TailscaleTunnel) -> Result<(), String> {
    tokio::time::timeout(STOP_TIMEOUT, tunnel.stop())
        .await
        .map_err(|_| "Tailscale reset timed out after 12 seconds".to_string())?
        .map_err(|error| error.to_string())
}

impl Default for RemoteAccessState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct TailscaleProbe {
    installed: bool,
    authenticated: bool,
    dns_name: Option<String>,
    error: Option<String>,
}

fn bounded_detail(bytes: &[u8]) -> String {
    let value = String::from_utf8_lossy(bytes).trim().to_string();
    value.chars().take(1_000).collect()
}

fn parse_tailscale_status(stdout: &[u8]) -> Result<TailscaleProbe, String> {
    let status: serde_json::Value = serde_json::from_slice(stdout)
        .map_err(|error| format!("Tailscale returned invalid status JSON: {error}"))?;
    let backend_state = status
        .get("BackendState")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let dns_name = status
        .get("Self")
        .and_then(|value| value.get("DNSName"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_end_matches('.').to_string());
    let authenticated = backend_state.eq_ignore_ascii_case("running") && dns_name.is_some();
    Ok(TailscaleProbe {
        installed: true,
        authenticated,
        dns_name,
        error: (!authenticated).then(|| {
            if backend_state.is_empty() {
                "Tailscale is installed but did not report a running backend. Open Tailscale and sign in."
                    .to_string()
            } else {
                format!(
                    "Tailscale backend is '{backend_state}'. Open Tailscale and sign in before enabling remote access."
                )
            }
        }),
    })
}

async fn probe_tailscale() -> TailscaleProbe {
    let binary = thinclaw_core::tunnel::resolve_binary("tailscale");
    let output = match tokio::time::timeout(
        COMMAND_TIMEOUT,
        Command::new(binary)
            .args(["status", "--json"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    {
        Err(_) => {
            return TailscaleProbe {
                error: Some("Tailscale status timed out after 10 seconds".to_string()),
                ..Default::default()
            };
        }
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return TailscaleProbe {
                error: Some(
                    "Tailscale CLI is not installed. Install it, sign in, then refresh."
                        .to_string(),
                ),
                ..Default::default()
            };
        }
        Ok(Err(error)) => {
            return TailscaleProbe {
                error: Some(format!("Could not run Tailscale: {error}")),
                ..Default::default()
            };
        }
        Ok(Ok(output)) => output,
    };

    if !output.status.success() {
        let detail = bounded_detail(&output.stderr);
        return TailscaleProbe {
            installed: true,
            error: Some(if detail.is_empty() {
                format!("Tailscale status exited with {}", output.status)
            } else {
                format!("Tailscale status failed: {detail}")
            }),
            ..Default::default()
        };
    }

    parse_tailscale_status(&output.stdout).unwrap_or_else(|error| TailscaleProbe {
        installed: true,
        error: Some(error),
        ..Default::default()
    })
}

fn validate_start_request(request: &RemoteAccessStartRequest) -> Result<(), BridgeError> {
    if request.exposure == RemoteAccessExposure::Public && !request.confirm_public {
        return Err(BridgeError::Runtime {
            message: "Public Funnel exposes the gateway to the internet. Confirm public exposure before starting it."
                .to_string(),
        });
    }
    Ok(())
}

async fn config_port(manager: &State<'_, ThinClawManager>) -> Result<u16, BridgeError> {
    let config = match manager.get_config().await {
        Some(config) => config,
        None => manager.init_config().await.map_err(BridgeError::from)?,
    };
    Ok(config.port)
}

async fn status_snapshot(
    state: &RemoteAccessState,
    runtime: &ThinClawRuntimeState,
    gateway_port: u16,
) -> RemoteAccessStatus {
    let probe = probe_tailscale().await;
    let (tunnel_running, exposure, access_url) = {
        let active = state.active.lock().await;
        if let Some(active) = active.as_ref() {
            (
                active.tunnel.health_check().await,
                Some(active.exposure),
                Some(active.access_url.clone()),
            )
        } else {
            (false, None, None)
        }
    };
    RemoteAccessStatus {
        runtime_mode: runtime.mode_label().await.to_string(),
        gateway_running: runtime.is_running().await
            && tokio::time::timeout(
                GATEWAY_PROBE_TIMEOUT,
                tokio::net::TcpStream::connect(("127.0.0.1", gateway_port)),
            )
            .await
            .is_ok_and(|result| result.is_ok()),
        gateway_port,
        gateway_url: format!("http://127.0.0.1:{gateway_port}"),
        tailscale_installed: probe.installed,
        tailscale_authenticated: probe.authenticated,
        tailscale_dns_name: probe.dns_name,
        tailscale_error: probe.error,
        tunnel_running,
        exposure,
        access_url,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_remote_access_status(
    state: State<'_, RemoteAccessState>,
    manager: State<'_, ThinClawManager>,
    runtime: State<'_, ThinClawRuntimeState>,
) -> Result<RemoteAccessStatus, BridgeError> {
    let port = config_port(&manager).await?;
    Ok(status_snapshot(&state, &runtime, port).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_remote_access_start(
    state: State<'_, RemoteAccessState>,
    manager: State<'_, ThinClawManager>,
    runtime: State<'_, ThinClawRuntimeState>,
    request: RemoteAccessStartRequest,
) -> Result<RemoteAccessStatus, BridgeError> {
    validate_start_request(&request)?;
    if runtime.is_remote_mode().await {
        return Err(gated(
            "local remote-access control",
            "Desktop is connected to a different ThinClaw gateway and cannot start host processes there",
            "Switch to Local Core, or configure Tailscale on the remote host",
            RouteMode::LocalOnly,
        ));
    }
    if !runtime.is_running().await {
        return Err(gated(
            "authenticated local gateway",
            "The embedded ThinClaw runtime is stopped",
            "Start Local Core in Gateway settings, then retry",
            RouteMode::LocalOnly,
        ));
    }

    let port = config_port(&manager).await?;
    let gateway_ready = tokio::time::timeout(
        GATEWAY_PROBE_TIMEOUT,
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .is_ok_and(|result| result.is_ok());
    if !gateway_ready {
        return Err(BridgeError::Runtime {
            message: format!(
                "The authenticated loopback gateway is not listening on port {port}. Restart Local Core and retry."
            ),
        });
    }

    let probe = probe_tailscale().await;
    if !probe.authenticated {
        return Err(BridgeError::Runtime {
            message: probe.error.unwrap_or_else(|| {
                "Tailscale is not ready. Install it and sign in before enabling remote access."
                    .to_string()
            }),
        });
    }

    let mut active = state.active.lock().await;
    if let Some(previous) = active.take() {
        if let Err(error) = stop_tunnel(&previous.tunnel).await {
            *active = Some(previous);
            return Err(BridgeError::Runtime {
                message: format!("Could not stop the previous Tailscale exposure: {error}"),
            });
        }
    }

    let tunnel = Arc::new(TailscaleTunnel::new(
        request.exposure == RemoteAccessExposure::Public,
        None,
    ));
    let access_url =
        tunnel
            .start("127.0.0.1", port)
            .await
            .map_err(|error| BridgeError::Runtime {
                message: error.to_string(),
            })?;
    *active = Some(ActiveTunnel {
        tunnel,
        exposure: request.exposure,
        access_url,
    });
    drop(active);

    // The user can stop Local Core while the external CLI is starting. Recheck
    // the trust boundary after the slow process launch so we never leave a
    // public/private listener pointed at a gateway that disappeared mid-start.
    let gateway_still_ready = runtime.is_running().await
        && tokio::time::timeout(
            GATEWAY_PROBE_TIMEOUT,
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await
        .is_ok_and(|result| result.is_ok());
    if !gateway_still_ready {
        if let Some(started) = state.active.lock().await.take() {
            let _ = stop_tunnel(&started.tunnel).await;
        }
        return Err(BridgeError::Runtime {
            message: "Local Core stopped while Tailscale was starting; remote access was reset"
                .to_string(),
        });
    }

    Ok(status_snapshot(&state, &runtime, port).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_remote_access_stop(
    state: State<'_, RemoteAccessState>,
    manager: State<'_, ThinClawManager>,
    runtime: State<'_, ThinClawRuntimeState>,
) -> Result<RemoteAccessStatus, BridgeError> {
    if runtime.is_remote_mode().await {
        return Err(gated(
            "local remote-access control",
            "Desktop is connected to a different ThinClaw gateway and cannot stop host processes there",
            "Switch to Local Core, or stop Tailscale on the remote host",
            RouteMode::LocalOnly,
        ));
    }
    let port = config_port(&manager).await?;
    let mut active_slot = state.active.lock().await;
    if let Some(active) = active_slot.take() {
        if let Err(error) = stop_tunnel(&active.tunnel).await {
            *active_slot = Some(active);
            return Err(BridgeError::Runtime {
                message: format!("Could not stop Tailscale remote access: {error}"),
            });
        }
    }
    drop(active_slot);
    Ok(status_snapshot(&state, &runtime, port).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_authenticated_tailscale_status_without_trailing_dot() {
        let probe = parse_tailscale_status(
            br#"{"BackendState":"Running","Self":{"DNSName":"desktop.tailnet.ts.net."}}"#,
        )
        .unwrap();
        assert!(probe.installed);
        assert!(probe.authenticated);
        assert_eq!(probe.dns_name.as_deref(), Some("desktop.tailnet.ts.net"));
        assert_eq!(probe.error, None);
    }

    #[test]
    fn signed_out_tailscale_status_has_actionable_error() {
        let probe = parse_tailscale_status(br#"{"BackendState":"NeedsLogin","Self":{}}"#).unwrap();
        assert!(probe.installed);
        assert!(!probe.authenticated);
        assert!(probe.error.unwrap().contains("NeedsLogin"));
    }

    #[test]
    fn public_exposure_requires_confirmation() {
        let error = validate_start_request(&RemoteAccessStartRequest {
            exposure: RemoteAccessExposure::Public,
            confirm_public: false,
        })
        .unwrap_err();
        assert!(error.to_string().contains("Confirm public exposure"));
        assert!(validate_start_request(&RemoteAccessStartRequest {
            exposure: RemoteAccessExposure::Tailnet,
            confirm_public: false,
        })
        .is_ok());
    }
}
