//! Docker container lifecycle management.
//!
//! Handles creating, running, and cleaning up containers for sandboxed execution.
//!
//! # Container Setup
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────────────────┐
//! │                          Docker Container                               │
//! │                                                                         │
//! │  Environment:                                                           │
//! │    http_proxy=http://host.docker.internal:PORT                          │
//! │    https_proxy=http://host.docker.internal:PORT                         │
//! │    (No secrets or credentials)                                          │
//! │                                                                         │
//! │  Mounts:                                                                │
//! │    /workspace ─▶ Host working directory (ro or rw based on policy)     │
//! │    /output    ─▶ Output directory for artifacts (rw)                   │
//! │                                                                         │
//! │  Limits:                                                                │
//! │    Memory: 2GB (default)                                                │
//! │    CPU: 1024 shares                                                     │
//! │    No privileged mode                                                   │
//! │    Non-root user (UID 1000)                                             │
//! └────────────────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::time::Duration;

use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, LogOutput, LogsOptions,
    RemoveContainerOptions, StartContainerOptions, WaitContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::HostConfig;
use futures::StreamExt;

use crate::sandbox::config::{ResourceLimits, SandboxPolicy};
use crate::sandbox::error::{Result, SandboxError};

/// Output from container execution.
#[derive(Debug, Clone)]
pub struct ContainerOutput {
    /// Exit code from the command.
    pub exit_code: i64,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// How long the command ran.
    pub duration: Duration,
    /// Whether output was truncated.
    pub truncated: bool,
}

/// Manages Docker container lifecycle.
pub struct ContainerRunner {
    docker: Docker,
    image: String,
    proxy_port: u16,
}

fn container_user_for_workspace(working_dir: &Path) -> String {
    #[cfg(unix)]
    {
        if let Ok(metadata) = std::fs::metadata(working_dir) {
            let uid = metadata.uid();
            let gid = metadata.gid();
            if uid != 0 {
                return format!("{uid}:{gid}");
            }
        }
    }

    "1000:1000".to_string()
}

impl ContainerRunner {
    fn push_log_chunk(target: &mut String, text: &str, half_max: usize, truncated: &mut bool) {
        if target.len() + text.len() > half_max {
            *truncated = true;
            let remaining = half_max.saturating_sub(target.len());
            let safe = crate::util::floor_char_boundary(text, remaining.min(text.len()));
            target.push_str(&text[..safe]);
        } else {
            target.push_str(text);
        }
    }

    /// Create a new container runner.
    pub fn new(docker: Docker, image: String, proxy_port: u16) -> Self {
        Self {
            docker,
            image,
            proxy_port,
        }
    }

    /// Check if the Docker daemon is available.
    pub async fn is_available(&self) -> bool {
        self.docker.ping().await.is_ok()
    }

    /// Check if the sandbox image exists locally.
    pub async fn image_exists(&self) -> bool {
        self.docker.inspect_image(&self.image).await.is_ok()
    }

    /// Pull the sandbox image.
    pub async fn pull_image(&self) -> Result<()> {
        use bollard::image::CreateImageOptions;

        tracing::info!("Pulling sandbox image: {}", self.image);

        let options = CreateImageOptions {
            from_image: self.image.clone(),
            ..Default::default()
        };

        let mut stream = self.docker.create_image(Some(options), None, None);

        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(status) = info.status {
                        tracing::debug!("Pull status: {}", status);
                    }
                }
                Err(e) => {
                    return Err(SandboxError::ContainerCreationFailed {
                        reason: format!("image pull failed: {}", e),
                    });
                }
            }
        }

        tracing::info!("Successfully pulled image: {}", self.image);
        Ok(())
    }

    /// Execute a command in a new container.
    pub async fn execute(
        &self,
        command: &str,
        working_dir: &Path,
        policy: SandboxPolicy,
        limits: &ResourceLimits,
        env: HashMap<String, String>,
        allow_network: bool,
    ) -> Result<ContainerOutput> {
        let start_time = std::time::Instant::now();

        // Create the container
        let container_id = self
            .create_container(command, working_dir, policy, limits, env, allow_network)
            .await?;

        // Start the container
        self.docker
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| SandboxError::ContainerStartFailed {
                reason: e.to_string(),
            })?;

        // Wait for completion with timeout
        let result = tokio::time::timeout(limits.timeout, async {
            self.wait_for_container(&container_id, limits.max_output_bytes)
                .await
        })
        .await;

        // Always clean up the container
        let _ = self
            .docker
            .remove_container(
                &container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        match result {
            Ok(Ok(mut output)) => {
                output.duration = start_time.elapsed();
                Ok(output)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(SandboxError::Timeout(limits.timeout)),
        }
    }

    /// Execute a command in an existing container using exec.
    pub async fn exec_in_container(
        &self,
        container_id: &str,
        command: &str,
        working_dir: &str,
        limits: &ResourceLimits,
    ) -> Result<ContainerOutput> {
        let start_time = std::time::Instant::now();

        let exec = self
            .docker
            .create_exec(
                container_id,
                CreateExecOptions {
                    cmd: Some(vec!["sh", "-c", command]),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    working_dir: Some(working_dir),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| SandboxError::ExecutionFailed {
                reason: format!("exec create failed: {}", e),
            })?;

        let result = tokio::time::timeout(
            limits.timeout,
            self.run_exec(&exec.id, limits.max_output_bytes),
        )
        .await;

        match result {
            Ok(Ok(mut output)) => {
                output.duration = start_time.elapsed();
                Ok(output)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(SandboxError::Timeout(limits.timeout)),
        }
    }

    /// Create a container with the appropriate configuration.
    async fn create_container(
        &self,
        command: &str,
        working_dir: &Path,
        policy: SandboxPolicy,
        limits: &ResourceLimits,
        env: HashMap<String, String>,
        allow_network: bool,
    ) -> Result<String> {
        let working_dir_str = working_dir.display().to_string();

        // Build environment variables
        let mut env_vec: Vec<String> = env
            .into_iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        // Add proxy environment (uses host.docker.internal for Mac/Windows, 172.17.0.1 for Linux)
        let proxy_host = if cfg!(target_os = "linux") {
            "172.17.0.1"
        } else {
            "host.docker.internal"
        };

        if allow_network && self.proxy_port > 0 && policy.is_sandboxed() {
            env_vec.push(format!(
                "http_proxy=http://{}:{}",
                proxy_host, self.proxy_port
            ));
            env_vec.push(format!(
                "https_proxy=http://{}:{}",
                proxy_host, self.proxy_port
            ));
            env_vec.push(format!(
                "HTTP_PROXY=http://{}:{}",
                proxy_host, self.proxy_port
            ));
            env_vec.push(format!(
                "HTTPS_PROXY=http://{}:{}",
                proxy_host, self.proxy_port
            ));
        }

        // Build volume mounts based on policy
        let binds = match policy {
            SandboxPolicy::ReadOnly => {
                vec![format!("{}:/workspace:ro", working_dir_str)]
            }
            SandboxPolicy::WorkspaceWrite => {
                vec![format!("{}:/workspace:rw", working_dir_str)]
            }
            SandboxPolicy::FullAccess => {
                // Full access - mount more of the host
                vec![
                    format!("{}:/workspace:rw", working_dir_str),
                    "/tmp:/tmp:rw".to_string(),
                ]
            }
        };

        let host_config = HostConfig {
            binds: Some(binds),
            memory: Some((limits.memory_bytes) as i64),
            cpu_shares: Some(limits.cpu_shares as i64),
            // Keep the container around until after log collection completes.
            // Fast-running commands can otherwise disappear before `docker logs`
            // has a chance to read their stdout/stderr.
            auto_remove: Some(false),
            network_mode: Some(if allow_network {
                "bridge".to_string()
            } else {
                "none".to_string()
            }),
            // Security: drop all capabilities and add back only what's needed
            cap_drop: Some(vec!["ALL".to_string()]),
            cap_add: Some(vec!["CHOWN".to_string()]),
            // Prevent privilege escalation
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            // Read-only root filesystem (workspace is still writable if policy allows)
            readonly_rootfs: Some(policy != SandboxPolicy::FullAccess),
            // Tmpfs mounts for /tmp and cargo cache
            tmpfs: Some(
                [
                    ("/tmp".to_string(), "size=512M".to_string()),
                    (
                        "/home/sandbox/.cargo/registry".to_string(),
                        "size=1G".to_string(),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let user = container_user_for_workspace(working_dir);

        let config = Config {
            image: Some(self.image.clone()),
            cmd: Some(vec![
                "sh".to_string(),
                "-c".to_string(),
                command.to_string(),
            ]),
            working_dir: Some("/workspace".to_string()),
            env: Some(env_vec),
            host_config: Some(host_config),
            // Match the bind-mounted workspace owner on Unix so Linux CI and
            // rootless Docker workspaces remain writable without using root.
            user: Some(user),
            ..Default::default()
        };

        let options = CreateContainerOptions {
            name: format!("sandbox-{}", uuid::Uuid::new_v4()),
            ..Default::default()
        };

        let response = self
            .docker
            .create_container(Some(options), config)
            .await
            .map_err(|e| SandboxError::ContainerCreationFailed {
                reason: e.to_string(),
            })?;

        Ok(response.id)
    }

    /// Wait for a container to complete and collect output.
    async fn wait_for_container(
        &self,
        container_id: &str,
        max_output: usize,
    ) -> Result<ContainerOutput> {
        // Wait for the container to finish
        let mut wait_stream = self.docker.wait_container(
            container_id,
            Some(WaitContainerOptions {
                condition: "not-running",
            }),
        );

        let exit_code = match wait_stream.next().await {
            Some(Ok(response)) => response.status_code,
            Some(Err(e)) => {
                tracing::warn!(
                    error = %e,
                    container_id,
                    "Docker wait stream failed; falling back to inspect polling"
                );
                self.poll_container_exit(container_id).await?
            }
            None => {
                tracing::warn!(
                    container_id,
                    "Docker wait stream ended unexpectedly; falling back to inspect polling"
                );
                self.poll_container_exit(container_id).await?
            }
        };

        // Collect logs
        let (stdout, stderr, truncated) = self.collect_logs(container_id, max_output).await?;

        Ok(ContainerOutput {
            exit_code,
            stdout,
            stderr,
            duration: Duration::ZERO, // Will be set by caller
            truncated,
        })
    }

    async fn poll_container_exit(&self, container_id: &str) -> Result<i64> {
        loop {
            let inspect = self
                .docker
                .inspect_container(container_id, None::<InspectContainerOptions>)
                .await
                .map_err(|e| SandboxError::ExecutionFailed {
                    reason: format!("container inspect failed after wait error: {}", e),
                })?;

            if let Some(state) = inspect.state {
                let running = state.running.unwrap_or(false);
                let restarting = state.restarting.unwrap_or(false);
                if !running && !restarting {
                    return Ok(state.exit_code.unwrap_or(-1));
                }
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Collect stdout and stderr from a container.
    async fn collect_logs(
        &self,
        container_id: &str,
        max_output: usize,
    ) -> Result<(String, String, bool)> {
        let options = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            follow: false,
            ..Default::default()
        };

        let mut stream = self.docker.logs(container_id, Some(options));

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut truncated = false;
        let half_max = max_output / 2;

        while let Some(result) = stream.next().await {
            match result {
                Ok(LogOutput::StdOut { message }) => {
                    let text = String::from_utf8_lossy(&message);
                    Self::push_log_chunk(&mut stdout, &text, half_max, &mut truncated);
                }
                Ok(LogOutput::StdErr { message }) => {
                    let text = String::from_utf8_lossy(&message);
                    Self::push_log_chunk(&mut stderr, &text, half_max, &mut truncated);
                }
                Ok(LogOutput::Console { message }) => {
                    let text = String::from_utf8_lossy(&message);
                    Self::push_log_chunk(&mut stdout, &text, half_max, &mut truncated);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Error reading container logs: {}", e);
                }
            }
        }

        Ok((stdout, stderr, truncated))
    }

    /// Run an exec and collect output.
    async fn run_exec(&self, exec_id: &str, max_output: usize) -> Result<ContainerOutput> {
        let start_result = self.docker.start_exec(exec_id, None).await.map_err(|e| {
            SandboxError::ExecutionFailed {
                reason: format!("exec start failed: {}", e),
            }
        })?;

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut truncated = false;
        let half_max = max_output / 2;

        if let StartExecResults::Attached { mut output, .. } = start_result {
            while let Some(result) = output.next().await {
                match result {
                    Ok(LogOutput::StdOut { message }) => {
                        let text = String::from_utf8_lossy(&message);
                        Self::push_log_chunk(&mut stdout, &text, half_max, &mut truncated);
                    }
                    Ok(LogOutput::StdErr { message }) => {
                        let text = String::from_utf8_lossy(&message);
                        Self::push_log_chunk(&mut stderr, &text, half_max, &mut truncated);
                    }
                    Ok(LogOutput::Console { message }) => {
                        let text = String::from_utf8_lossy(&message);
                        Self::push_log_chunk(&mut stdout, &text, half_max, &mut truncated);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("Error reading exec output: {}", e);
                    }
                }
            }
        }

        // Get exec exit code
        let inspect =
            self.docker
                .inspect_exec(exec_id)
                .await
                .map_err(|e| SandboxError::ExecutionFailed {
                    reason: format!("exec inspect failed: {}", e),
                })?;

        let exit_code = inspect.exit_code.unwrap_or(-1);

        Ok(ContainerOutput {
            exit_code,
            stdout,
            stderr,
            duration: Duration::ZERO,
            truncated,
        })
    }
}

/// Connect to the Docker daemon.
///
/// Tries these locations in order:
/// 1. `DOCKER_HOST` env var (bollard default)
/// 2. `/var/run/docker.sock` (Linux default; also used by OrbStack and Podman Desktop on macOS)
/// 3. `~/.docker/run/docker.sock` (Docker Desktop 4.13+ on macOS — primary user-owned socket)
/// 4. `~/.colima/default/docker.sock` (Colima — popular lightweight Docker Desktop alternative)
/// 5. `~/.rd/docker.sock` (Rancher Desktop on macOS)
/// 6. `$XDG_RUNTIME_DIR/docker.sock` (common rootless Docker socket on Linux)
/// 7. `/run/user/$UID/docker.sock` (rootless Docker fallback on Linux)
pub async fn connect_docker() -> Result<Docker> {
    /// Per-probe timeout for Docker daemon pings during startup detection.
    /// The bollard default is 120 seconds — far too long for a startup probe.
    /// 5 seconds is generous for a local Unix-socket ping.
    const PROBE_TIMEOUT: u64 = 5;

    /// Timeout applied to the returned Docker client for actual runtime
    /// operations (container create, image pull, exec).  Must be long enough
    /// for heavy operations but short enough to surface real daemon problems.
    const RUNTIME_TIMEOUT: u64 = 30;

    // First try bollard defaults (checks DOCKER_HOST env var, then /var/run/docker.sock).
    // This covers Linux, OrbStack (updates the /var/run symlink), and any user with
    // DOCKER_HOST set to their runtime's socket.
    if let Ok(docker) = Docker::connect_with_local_defaults()
        .map(|d| d.with_timeout(Duration::from_secs(PROBE_TIMEOUT)))
        && docker.ping().await.is_ok()
    {
        // Widen timeout for runtime use before returning.
        return Ok(docker.with_timeout(Duration::from_secs(RUNTIME_TIMEOUT)));
    }

    #[cfg(unix)]
    {
        // Try well-known user-owned socket locations for desktop and rootless runtimes.
        // Docker Desktop 4.13+ (stabilised in 4.18) stopped creating the
        // /var/run/docker.sock symlink by default and moved the API socket
        // to ~/.docker/run/docker.sock.
        for sock in unix_socket_candidates() {
            if sock.exists() {
                let sock_str = sock.to_string_lossy();
                if let Ok(docker) = Docker::connect_with_socket(
                    &sock_str,
                    PROBE_TIMEOUT,
                    bollard::API_DEFAULT_VERSION,
                ) && docker.ping().await.is_ok()
                {
                    return Ok(docker.with_timeout(Duration::from_secs(RUNTIME_TIMEOUT)));
                }
            }
        }
    }

    Err(SandboxError::DockerNotAvailable {
        reason: "Could not connect to Docker daemon. Tried: $DOCKER_HOST, \
            /var/run/docker.sock, ~/.docker/run/docker.sock, \
            ~/.colima/default/docker.sock, ~/.rd/docker.sock, \
            $XDG_RUNTIME_DIR/docker.sock, /run/user/$UID/docker.sock"
            .to_string(),
    })
}

#[cfg(unix)]
fn unix_socket_candidates() -> Vec<PathBuf> {
    unix_socket_candidates_from_env(
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var("UID").ok(),
    )
}

#[cfg(unix)]
fn unix_socket_candidates_from_env(
    home: Option<PathBuf>,
    xdg_runtime_dir: Option<PathBuf>,
    uid: Option<String>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut push_unique = |path: PathBuf| {
        if !candidates.iter().any(|existing| existing == &path) {
            candidates.push(path);
        }
    };

    if let Some(home) = home {
        push_unique(home.join(".docker/run/docker.sock")); // Docker Desktop 4.13+
        push_unique(home.join(".colima/default/docker.sock")); // Colima
        push_unique(home.join(".rd/docker.sock")); // Rancher Desktop
    }

    if let Some(xdg_runtime_dir) = xdg_runtime_dir {
        push_unique(xdg_runtime_dir.join("docker.sock"));
    }

    if let Some(uid) = uid.filter(|value| !value.is_empty()) {
        push_unique(PathBuf::from(format!("/run/user/{uid}/docker.sock")));
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn test_unix_socket_candidates_include_rootless_paths() {
        let candidates = unix_socket_candidates_from_env(
            Some(PathBuf::from("/home/tester")),
            Some(PathBuf::from("/run/user/1000")),
            Some("1000".to_string()),
        );

        assert!(candidates.contains(&PathBuf::from("/home/tester/.docker/run/docker.sock")));
        assert!(candidates.contains(&PathBuf::from("/home/tester/.colima/default/docker.sock")));
        assert!(candidates.contains(&PathBuf::from("/home/tester/.rd/docker.sock")));
        assert!(candidates.contains(&PathBuf::from("/run/user/1000/docker.sock")));
    }

    #[tokio::test]
    async fn test_docker_connection() {
        // This test requires Docker to be running
        let result = connect_docker().await;
        // Don't fail if Docker isn't available, just skip
        if result.is_err() {
            eprintln!("Skipping Docker test: Docker not available");
            return;
        }

        let docker = result.unwrap();
        let runner = ContainerRunner::new(docker, "alpine:latest".to_string(), 0);
        // Just check that we can query Docker (result doesn't matter for CI)
        let _available = runner.is_available().await;
    }
}
