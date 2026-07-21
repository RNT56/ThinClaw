//! ngrok tunnel via the `ngrok` binary.

use anyhow::{Result, bail};
use tokio::process::Command;

use crate::tunnel::{
    SharedProcess, SharedUrl, Tunnel, TunnelProcess, drain_tunnel_output, kill_shared,
    new_shared_process, new_shared_url,
};
use crate::worker::bridge_common::read_bounded_line;

/// Wraps `ngrok` with optional custom domain support (paid plan).
pub struct NgrokTunnel {
    auth_token: String,
    domain: Option<String>,
    proc: SharedProcess,
    url: SharedUrl,
}

impl NgrokTunnel {
    pub fn new(auth_token: String, domain: Option<String>) -> Self {
        Self {
            auth_token,
            domain,
            proc: new_shared_process(),
            url: new_shared_url(),
        }
    }
}

#[async_trait::async_trait]
impl Tunnel for NgrokTunnel {
    fn name(&self) -> &str {
        "ngrok"
    }

    async fn start(&self, local_host: &str, local_port: u16) -> Result<String> {
        let mut args = vec!["http".to_string(), format!("{local_host}:{local_port}")];
        if let Some(ref domain) = self.domain {
            args.push("--domain".into());
            args.push(domain.clone());
        }
        args.extend(["--log", "stdout", "--log-format", "logfmt"].map(String::from));

        let mut command = Command::new(crate::tunnel::resolve_binary("ngrok"));
        command
            .args(&args)
            .env("NGROK_AUTHTOKEN", &self.auth_token)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = thinclaw_platform::OwnedChild::spawn(&mut command)?;

        let stdout = child
            .take_stdout()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture ngrok stdout"))?;
        let stderr = child
            .take_stderr()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture ngrok stderr"))?;
        let stderr_task = drain_tunnel_output(stderr);

        let mut reader = tokio::io::BufReader::new(stdout);
        let mut public_url = String::new();

        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(15);
        while tokio::time::Instant::now() < deadline {
            let line = tokio::time::timeout(
                tokio::time::Duration::from_secs(3),
                read_bounded_line(&mut reader, 64 * 1024),
            )
            .await;

            match line {
                Ok(Ok(Some(line))) => {
                    let l = line.text;
                    tracing::debug!("ngrok: {l}");
                    // ngrok logfmt: url=https://xxxx.ngrok-free.app
                    if let Some(idx) = l.find("url=https://") {
                        let url_start = idx + 4; // skip "url="
                        let url_part = &l[url_start..];
                        let end = url_part
                            .find(|c: char| c.is_whitespace())
                            .unwrap_or(url_part.len());
                        public_url = url_part[..end].to_string();
                        break;
                    }
                }
                Ok(Ok(None)) => break,
                Ok(Err(e)) => bail!("Error reading ngrok output: {e}"),
                Err(_) => {}
            }
        }

        if public_url.is_empty() {
            child.kill().await.ok();
            bail!("ngrok did not produce a public URL within 15s. Is the auth token valid?");
        }
        let parsed = url::Url::parse(&public_url)?;
        if parsed.scheme() != "https"
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            child.kill().await.ok();
            bail!("ngrok produced an invalid public HTTPS URL");
        }
        if !matches!(child.try_wait(), Ok(None)) {
            child.kill().await.ok();
            bail!("ngrok exited during startup");
        }
        let stdout_task = drain_tunnel_output(reader);

        if let Ok(mut guard) = self.url.write() {
            *guard = Some(public_url.clone());
        }

        let mut guard = self.proc.lock().await;
        *guard = Some(TunnelProcess {
            child,
            _output_tasks: vec![stdout_task, stderr_task],
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
    fn constructor_stores_domain() {
        let tunnel = NgrokTunnel::new("tok".into(), Some("my.ngrok.app".into()));
        assert_eq!(tunnel.domain.as_deref(), Some("my.ngrok.app"));
    }

    #[test]
    fn public_url_none_before_start() {
        assert!(NgrokTunnel::new("tok".into(), None).public_url().is_none());
    }

    #[tokio::test]
    async fn stop_without_start_is_ok() {
        assert!(NgrokTunnel::new("tok".into(), None).stop().await.is_ok());
    }

    #[tokio::test]
    async fn health_false_before_start() {
        assert!(!NgrokTunnel::new("tok".into(), None).health_check().await);
    }
}
