//! Cloudflare Tunnel via the `cloudflared` binary.

use anyhow::{Result, bail};
use tokio::process::Command;

use crate::tunnel::{
    SharedProcess, SharedUrl, Tunnel, TunnelProcess, drain_tunnel_output, kill_shared,
    new_shared_process, new_shared_url, read_tunnel_output_bounded,
};

/// Wraps `cloudflared` with token-based auth from the Zero Trust dashboard.
pub struct CloudflareTunnel {
    token: String,
    hostname: String,
    proc: SharedProcess,
    url: SharedUrl,
}

impl CloudflareTunnel {
    pub fn new(token: String, hostname: String) -> Self {
        Self {
            token,
            hostname,
            proc: new_shared_process(),
            url: new_shared_url(),
        }
    }
}

#[async_trait::async_trait]
impl Tunnel for CloudflareTunnel {
    fn name(&self) -> &str {
        "cloudflare"
    }

    async fn start(&self, local_host: &str, local_port: u16) -> Result<String> {
        let origin = format!("http://{local_host}:{local_port}");
        let public_url = url::Url::parse(&self.hostname)?;
        if public_url.scheme() != "https" || public_url.host_str().is_none() {
            bail!("managed Cloudflare tunnel hostname must be a valid HTTPS URL");
        }
        let public_url = public_url.as_str().trim_end_matches('/').to_string();
        let mut command = Command::new(crate::tunnel::resolve_binary("cloudflared"));
        command
            .args(["tunnel", "--no-autoupdate", "run", "--url", &origin])
            // Keep the credential out of argv/process listings. cloudflared
            // supports the same tunnel token through TUNNEL_TOKEN.
            .env("TUNNEL_TOKEN", &self.token)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = thinclaw_platform::OwnedChild::spawn(&mut command)?;
        let stdout = child
            .take_stdout()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture cloudflared stdout"))?;
        let stderr = child
            .take_stderr()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture cloudflared stderr"))?;

        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        let output_tasks = match child.try_wait() {
            Ok(Some(status)) => {
                let ((stdout, stdout_exceeded), (stderr, stderr_exceeded)) = tokio::join!(
                    read_tunnel_output_bounded(stdout, 64 * 1024),
                    read_tunnel_output_bounded(stderr, 64 * 1024),
                );
                let detail = if stderr.is_empty() { stdout } else { stderr };
                let detail = String::from_utf8_lossy(&detail);
                bail!(
                    "cloudflared exited during startup ({status}): {}{}",
                    detail.trim(),
                    if stdout_exceeded || stderr_exceeded {
                        " [output truncated]"
                    } else {
                        ""
                    }
                );
            }
            Ok(None) => vec![drain_tunnel_output(stdout), drain_tunnel_output(stderr)],
            Err(error) => {
                child.kill().await.ok();
                bail!("failed to inspect cloudflared startup: {error}");
            }
        };

        if let Ok(mut guard) = self.url.write() {
            *guard = Some(public_url.clone());
        }

        let mut guard = self.proc.lock().await;
        *guard = Some(TunnelProcess {
            child,
            _output_tasks: output_tasks,
        });

        Ok(public_url)
    }

    async fn stop(&self) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_stores_token() {
        let tunnel = CloudflareTunnel::new("cf-token".into(), "https://agent.example.com".into());
        assert_eq!(tunnel.token, "cf-token");
    }

    #[test]
    fn public_url_none_before_start() {
        assert!(
            CloudflareTunnel::new("tok".into(), "https://agent.example.com".into())
                .public_url()
                .is_none()
        );
    }

    #[tokio::test]
    async fn stop_without_start_is_ok() {
        assert!(
            CloudflareTunnel::new("tok".into(), "https://agent.example.com".into())
                .stop()
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn health_false_before_start() {
        assert!(
            !CloudflareTunnel::new("tok".into(), "https://agent.example.com".into())
                .health_check()
                .await
        );
    }
}
