//! Tailscale tunnel via `tailscale serve` or `tailscale funnel`.

use anyhow::{Result, bail};
use tokio::process::Command;

use crate::tunnel::{
    SharedProcess, SharedUrl, Tunnel, TunnelProcess, drain_tunnel_output, kill_shared,
    new_shared_process, new_shared_url, read_tunnel_output_bounded,
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
            let mut command = Command::new(crate::tunnel::resolve_binary("tailscale"));
            command.args(["status", "--json"]);
            let output = thinclaw_platform::bounded_command_output(
                &mut command,
                tokio::time::Duration::from_secs(10),
                2 * 1024 * 1024,
                64 * 1024,
            )
            .await?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);

                // Detect the known macOS App Store CLI issue: the binary at
                // /Applications/Tailscale.app/Contents/MacOS/Tailscale crashes
                // with a BundleIdentifier error when spawned from another process.
                if stderr.contains("BundleIdentifier") || stderr.contains("Fatal error") {
                    bail!(
                        "The Tailscale CLI crashed with a macOS bundle identity error. \
                         This happens when the App Store version's CLI is spawned from \
                         another process.\n\n\
                         Fix: install the standalone Tailscale CLI via Homebrew:\n\
                         \n  brew install tailscale\n\n\
                         The Homebrew CLI works alongside the Tailscale app and does not \
                         have this crash. After installing, restart ThinClaw.\n\n\
                         ThinClaw will use polling mode for now (Telegram messages ~5s delay)."
                    );
                }

                bail!("tailscale status failed: {}", stderr);
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
        validate_tailscale_hostname(&hostname)?;

        let target = format!("http://{local_host}:{local_port}");

        // Reset any stale serve/funnel configuration from a previous run.
        // Without this, `tailscale funnel` fails with "listener already exists
        // for port 443" if ThinClaw was killed without a clean shutdown.
        let ts_bin = crate::tunnel::resolve_binary("tailscale");
        tracing::debug!("Resetting stale tailscale {subcommand} config before start");
        let mut reset_command = Command::new(&ts_bin);
        reset_command.args([subcommand, "reset"]);
        let reset_output = thinclaw_platform::bounded_command_output(
            &mut reset_command,
            tokio::time::Duration::from_secs(30),
            64 * 1024,
            64 * 1024,
        )
        .await;
        if let Ok(ref out) = reset_output
            && !out.status.success()
        {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // version warnings are harmless — only log real failures
            let real_errors: Vec<&str> = stderr
                .lines()
                .filter(|l| !l.contains("client version") && !l.contains("Warning:"))
                .collect();
            if !real_errors.is_empty() {
                tracing::warn!(
                    "tailscale {subcommand} reset returned non-zero: {}",
                    real_errors.join("; ")
                );
            }
        }

        // Brief pause after reset to let the daemon settle
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Spawn the tailscale serve/funnel process
        let mut command = Command::new(&ts_bin);
        command
            .args([subcommand, &target])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = thinclaw_platform::OwnedChild::spawn(&mut command)?;
        let stdout = child
            .take_stdout()
            .ok_or_else(|| anyhow::anyhow!("failed to capture tailscale stdout"))?;
        let stderr = child
            .take_stderr()
            .ok_or_else(|| anyhow::anyhow!("failed to capture tailscale stderr"))?;

        // Wait briefly to detect early exit (e.g., "Funnel is not enabled",
        // auth errors, permission denied, etc.). A successful `tailscale funnel`
        // runs as a long-lived daemon and won't exit within this window.
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        // Check if the process has exited (which means failure)
        match child.try_wait() {
            Ok(Some(exit_status)) => {
                let ((stdout, stdout_exceeded), (stderr, stderr_exceeded)) = tokio::join!(
                    read_tunnel_output_bounded(stdout, 64 * 1024),
                    read_tunnel_output_bounded(stderr, 64 * 1024),
                );
                let mut stderr_msg = String::from_utf8_lossy(&stderr).trim().to_string();
                if stderr_msg.is_empty() {
                    let stdout_msg = String::from_utf8_lossy(&stdout).trim().to_string();
                    if !stdout_msg.is_empty() {
                        stderr_msg = stdout_msg;
                    }
                }
                if stdout_exceeded || stderr_exceeded {
                    stderr_msg.push_str(" [output truncated]");
                }

                // Filter out version mismatch warnings (non-fatal, noisy)
                let filtered: Vec<&str> = stderr_msg
                    .lines()
                    .filter(|l| {
                        !l.starts_with("Warning: client version")
                            && !l.contains("!= tailscaled server version")
                    })
                    .collect();
                // Log version warning separately if present
                if filtered.len() < stderr_msg.lines().count() {
                    tracing::warn!(
                        "Tailscale client/server version mismatch detected (non-fatal). \
                         Run 'brew upgrade tailscale' to sync versions."
                    );
                }
                let detail = if filtered.is_empty() {
                    format!("exit code: {}", exit_status)
                } else {
                    filtered.join("\n")
                };

                // Check for "listener already exists" — suggest reset
                if detail.contains("listener already exists") {
                    bail!(
                        "tailscale {subcommand} failed: a stale listener is blocking port 443.\n\
                         \n\
                         Fix: run 'tailscale {subcommand} reset' then restart ThinClaw."
                    );
                }

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
                child.kill().await.ok();
                bail!("could not inspect tailscale {subcommand} startup: {e}");
            }
        }

        let public_url = format!("https://{hostname}");

        if let Ok(mut guard) = self.url.write() {
            *guard = Some(public_url.clone());
        }

        let output_tasks = vec![drain_tunnel_output(stdout), drain_tunnel_output(stderr)];
        let mut guard = self.proc.lock().await;
        *guard = Some(TunnelProcess {
            child,
            _output_tasks: output_tasks,
        });

        Ok(public_url)
    }

    async fn stop(&self) -> Result<()> {
        let subcommand = if self.funnel { "funnel" } else { "serve" };
        let mut command = Command::new(crate::tunnel::resolve_binary("tailscale"));
        command.args([subcommand, "reset"]);
        if let Err(e) = thinclaw_platform::bounded_command_output(
            &mut command,
            tokio::time::Duration::from_secs(30),
            64 * 1024,
            64 * 1024,
        )
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
        let mut guard = self.proc.lock().await;
        guard
            .as_mut()
            .is_some_and(|tp| matches!(tp.child.try_wait(), Ok(None)))
    }

    fn public_url(&self) -> Option<String> {
        self.url.read().ok().and_then(|guard| guard.clone())
    }
}

fn validate_tailscale_hostname(hostname: &str) -> Result<()> {
    if hostname.is_empty() || hostname.len() > 253 || hostname.chars().any(char::is_control) {
        bail!("tailscale returned an invalid hostname");
    }
    let parsed = url::Url::parse(&format!("https://{hostname}"))?;
    if parsed.host_str() != Some(hostname)
        || !parsed.path().is_empty() && parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        bail!("tailscale returned an invalid hostname");
    }
    Ok(())
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
