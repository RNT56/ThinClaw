//! Tailscale tunnel via `tailscale serve` or `tailscale funnel`.

use anyhow::{Result, bail};
use tokio::process::Command;

use crate::tunnel::{
    SharedProcess, SharedUrl, Tunnel, TunnelProcess, kill_shared, new_shared_process,
    new_shared_url,
};

/// Uses `tailscale serve` (tailnet-only) or `tailscale funnel` (public).
///
/// Requires Tailscale installed and authenticated (`tailscale up`).
pub struct TailscaleTunnel {
    funnel: bool,
    hostname: Option<String>,
    proc: SharedProcess,
    url: SharedUrl,
}

impl TailscaleTunnel {
    pub fn new(funnel: bool, hostname: Option<String>) -> Self {
        Self {
            funnel,
            hostname,
            proc: new_shared_process(),
            url: new_shared_url(),
        }
    }
}

#[async_trait::async_trait]
impl Tunnel for TailscaleTunnel {
    fn name(&self) -> &str {
        "tailscale"
    }

    async fn start(&self, local_host: &str, local_port: u16) -> Result<String> {
        let subcommand = if self.funnel { "funnel" } else { "serve" };

        let hostname = if let Some(ref h) = self.hostname {
            h.clone()
        } else {
            let output = tokio::time::timeout(
                tokio::time::Duration::from_secs(10),
                Command::new("tailscale")
                    .args(["status", "--json"])
                    .output(),
            )
            .await
            .map_err(|_| anyhow::anyhow!("tailscale status --json timed out after 10s"))??;

            if !output.status.success() {
                bail!(
                    "tailscale status failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }

            let status: serde_json::Value = serde_json::from_slice(&output.stdout)
                .map_err(|e| anyhow::anyhow!("Failed to parse tailscale status JSON: {e}"))?;
            let dns_name = status["Self"]["DNSName"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("tailscale status missing Self.DNSName field"))?
                .trim_end_matches('.')
                .to_string();

            if dns_name.is_empty() {
                bail!(
                    "Tailscale DNSName is empty — is Tailscale logged in? \
                     Run `tailscale login` or `tailscale up` to authenticate."
                );
            }

            dns_name
        };

        let target = format!("http://{local_host}:{local_port}");

        // Spawn the tailscale serve/funnel process
        let mut child = Command::new("tailscale")
            .args([subcommand, &target])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        // Wait briefly to detect early exit (e.g., "Funnel is not enabled",
        // auth errors, permission denied, etc.). A successful `tailscale funnel`
        // runs as a long-lived daemon and won't exit within this window.
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        // Check if the process has exited (which means failure)
        match child.try_wait() {
            Ok(Some(exit_status)) => {
                // Process already exited — read stderr for the error message
                let mut stderr_msg = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    use tokio::io::AsyncReadExt;
                    let mut buf = Vec::new();
                    let _ = stderr.read_to_end(&mut buf).await;
                    stderr_msg = String::from_utf8_lossy(&buf).trim().to_string();
                }
                if stderr_msg.is_empty() {
                    let mut stdout_msg = String::new();
                    if let Some(mut stdout) = child.stdout.take() {
                        use tokio::io::AsyncReadExt;
                        let mut buf = Vec::new();
                        let _ = stdout.read_to_end(&mut buf).await;
                        stdout_msg = String::from_utf8_lossy(&buf).trim().to_string();
                    }
                    if !stdout_msg.is_empty() {
                        stderr_msg = stdout_msg;
                    }
                }
                let detail = if stderr_msg.is_empty() {
                    format!("exit code: {}", exit_status)
                } else {
                    stderr_msg
                };
                bail!(
                    "tailscale {subcommand} failed to start: {detail}\n\
                     \n\
                     If you see 'Funnel is not enabled', visit your Tailscale \
                     admin console to enable it, or switch to a different tunnel \
                     provider (ngrok, cloudflare) in ThinClaw settings."
                );
            }
            Ok(None) => {
                // Process is still running — good, funnel is active
                tracing::info!(
                    subcommand,
                    hostname = %hostname,
                    "Tailscale {subcommand} process started and still running"
                );
            }
            Err(e) => {
                tracing::warn!("Could not check tailscale {subcommand} status: {e}");
            }
        }

        let public_url = format!("https://{hostname}");

        if let Ok(mut guard) = self.url.write() {
            *guard = Some(public_url.clone());
        }

        let mut guard = self.proc.lock().await;
        *guard = Some(TunnelProcess { child });

        Ok(public_url)
    }

    async fn stop(&self) -> Result<()> {
        let subcommand = if self.funnel { "funnel" } else { "serve" };
        if let Err(e) = Command::new("tailscale")
            .args([subcommand, "reset"])
            .output()
            .await
        {
            tracing::warn!("tailscale {subcommand} reset failed: {e}");
        }

        if let Ok(mut guard) = self.url.write() {
            *guard = None;
        }
        kill_shared(&self.proc).await
    }

    async fn health_check(&self) -> bool {
        let guard = self.proc.lock().await;
        guard.as_ref().is_some_and(|tp| tp.child.id().is_some())
    }

    fn public_url(&self) -> Option<String> {
        self.url.read().ok().and_then(|guard| guard.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_stores_hostname_and_mode() {
        let tunnel = TailscaleTunnel::new(true, Some("myhost.ts.net".into()));
        assert!(tunnel.funnel);
        assert_eq!(tunnel.hostname.as_deref(), Some("myhost.ts.net"));
    }

    #[test]
    fn public_url_none_before_start() {
        assert!(TailscaleTunnel::new(false, None).public_url().is_none());
    }

    #[tokio::test]
    async fn health_false_before_start() {
        assert!(!TailscaleTunnel::new(false, None).health_check().await);
    }

    #[tokio::test]
    async fn stop_without_start_is_ok() {
        assert!(TailscaleTunnel::new(false, None).stop().await.is_ok());
    }
}
