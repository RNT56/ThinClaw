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
//! │    http_proxy=http://thinclaw:TOKEN@thinclaw.host.internal:PORT         │
//! │    https_proxy=http://thinclaw:TOKEN@thinclaw.host.internal:PORT        │
//! │    (Ephemeral authenticated-proxy credential only)                      │
//! │                                                                         │
//! │  Mounts:                                                                │
//! │    /workspace ─▶ Host working directory (ro or rw based on policy)     │
//! │    /output    ─▶ Output directory for artifacts (rw)                   │
//! │                                                                         │
//! │  Limits:                                                                │
//! │    Memory: 2GB (default)                                                │
//! │    CPU: 1024 shares plus a derived one-CPU hard quota                   │
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
use bollard::container::LogOutput;
use bollard::models::{ContainerCreateBody, HostConfig, Mount, MountType, ResourcesUlimits};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, LogsOptionsBuilder, RemoveContainerOptionsBuilder,
    WaitContainerOptionsBuilder,
};
use futures::StreamExt;

use crate::sandbox::config::{ResourceLimits, SandboxPolicy};
use crate::sandbox::error::{Result, SandboxError};
use crate::sandbox::network::managed_container_labels;
use crate::sandbox::relay::{RELAY_PROXY_PORT, RelayForward, SandboxNetworkRelay};

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
    proxy_token: Option<String>,
    runtime_scope: String,
}

/// Cancellation-safe cleanup for an ephemeral container. The deterministic
/// random name is known before Docker creation starts, so dropping a future
/// while the daemon is still processing `create` can still remove the eventual
/// container. Normal completion disarms the guard after synchronous cleanup.
struct EphemeralContainerGuard {
    docker: Docker,
    target: String,
    armed: bool,
}

impl EphemeralContainerGuard {
    fn new(docker: Docker, container_name: String) -> Self {
        Self {
            docker,
            target: container_name,
            armed: true,
        }
    }

    fn set_container_id(&mut self, container_id: String) {
        self.target = container_id;
    }

    async fn cleanup_now(&mut self) {
        match self
            .docker
            .remove_container(
                &self.target,
                Some(RemoveContainerOptionsBuilder::new().force(true).build()),
            )
            .await
        {
            Ok(()) => self.armed = false,
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                // A create request may still be in flight. Keep the guard armed
                // so Drop performs bounded retries by the preselected name.
            }
            Err(error) => tracing::warn!(
                container = %self.target,
                %error,
                "Failed to synchronously remove ephemeral sandbox"
            ),
        }
    }
}

impl Drop for EphemeralContainerGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::error!(
                container = %self.target,
                "Ephemeral sandbox cleanup dropped outside a Tokio runtime"
            );
            return;
        };
        let docker = self.docker.clone();
        let target = self.target.clone();
        runtime.spawn(async move {
            const RETRIES: usize = 10;
            for attempt in 0..RETRIES {
                match docker
                    .remove_container(
                        &target,
                        Some(RemoveContainerOptionsBuilder::new().force(true).build()),
                    )
                    .await
                {
                    Ok(()) => return,
                    Err(bollard::errors::Error::DockerResponseServerError {
                        status_code: 404,
                        ..
                    }) if attempt + 1 < RETRIES => {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    Err(bollard::errors::Error::DockerResponseServerError {
                        status_code: 404,
                        ..
                    }) => return,
                    Err(error) if attempt + 1 < RETRIES => {
                        tracing::debug!(
                            container = %target,
                            %error,
                            "Retrying ephemeral sandbox cleanup"
                        );
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    Err(error) => {
                        tracing::error!(
                            container = %target,
                            %error,
                            "Exhausted ephemeral sandbox cleanup retries"
                        );
                        return;
                    }
                }
            }
        });
    }
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
    fn validate_runner_config(&self) -> Result<()> {
        if self.image.trim().is_empty()
            || self.image.len() > 512
            || self.image.chars().any(char::is_control)
        {
            return Err(SandboxError::Config {
                reason: "sandbox image name is empty, oversized, or invalid".to_string(),
            });
        }
        if self.runtime_scope.is_empty()
            || self.runtime_scope.len() > 128
            || !self.runtime_scope.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            return Err(SandboxError::Config {
                reason: "sandbox runtime scope is invalid".to_string(),
            });
        }
        if self.proxy_token.as_deref().is_some_and(|token| {
            token.is_empty() || token.len() > 512 || token.chars().any(char::is_control)
        }) {
            return Err(SandboxError::Config {
                reason: "sandbox proxy credential is invalid".to_string(),
            });
        }
        Ok(())
    }

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
            proxy_token: None,
            runtime_scope: crate::runtime_lease::runtime_scope_id_for_path(
                &crate::platform::resolve_data_dir(""),
            ),
        }
    }

    /// Bind managed-container labels to the owning runtime state directory.
    pub fn with_runtime_scope(mut self, runtime_scope: impl Into<String>) -> Self {
        self.runtime_scope = runtime_scope.into();
        self
    }

    /// Attach the ephemeral credential required by the host proxy.
    pub fn with_proxy_token(mut self, proxy_token: Option<String>) -> Self {
        self.proxy_token = proxy_token;
        self
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
        use bollard::query_parameters::CreateImageOptionsBuilder;

        self.validate_runner_config()?;

        tracing::info!("Pulling sandbox image: {}", self.image);

        let options = CreateImageOptionsBuilder::new()
            .from_image(self.image.as_str())
            .build();

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
        let container_name = format!("sandbox-{}", uuid::Uuid::new_v4());
        let mut cleanup = EphemeralContainerGuard::new(self.docker.clone(), container_name.clone());
        let mut relay = if allow_network && policy.is_sandboxed() {
            if self.proxy_port == 0 || self.proxy_token.as_deref().is_none_or(str::is_empty) {
                return Err(SandboxError::ProxyError {
                    reason: "authenticated proxy is unavailable for networked sandbox".to_string(),
                });
            }
            Some(
                SandboxNetworkRelay::start(
                    self.docker.clone(),
                    &self.image,
                    &self.runtime_scope,
                    "ephemeral-relay",
                    &[RelayForward {
                        listen_port: RELAY_PROXY_PORT,
                        target_port: self.proxy_port,
                    }],
                )
                .await?,
            )
        } else {
            None
        };

        // Create the container
        let container_id = self
            .create_container(
                &container_name,
                command,
                working_dir,
                policy,
                limits,
                env,
                allow_network,
                relay.as_ref(),
            )
            .await?;
        cleanup.set_container_id(container_id.clone());

        // Start the container
        if let Err(error) = self.docker.start_container(&container_id, None).await {
            cleanup.cleanup_now().await;
            return Err(SandboxError::ContainerStartFailed {
                reason: error.to_string(),
            });
        }

        // Wait for completion with timeout
        let result = tokio::time::timeout(limits.timeout, async {
            self.wait_for_container(&container_id, limits.max_output_bytes)
                .await
        })
        .await;

        // Always clean up the container. If this call fails, the armed guard
        // retries after the future returns or is cancelled.
        cleanup.cleanup_now().await;
        if let Some(relay) = relay.as_mut() {
            relay.stop().await;
        }

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
        container_name: &str,
        command: &str,
        working_dir: &Path,
        policy: SandboxPolicy,
        limits: &ResourceLimits,
        env: HashMap<String, String>,
        allow_network: bool,
        relay: Option<&SandboxNetworkRelay>,
    ) -> Result<String> {
        const MAX_COMMAND_BYTES: usize = 1024 * 1024;
        const MAX_ENVIRONMENT_ENTRIES: usize = 256;
        const MAX_ENVIRONMENT_BYTES: usize = 256 * 1024;
        const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
        const MAX_TIMEOUT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

        self.validate_runner_config()?;
        if !policy.is_sandboxed() {
            return Err(SandboxError::Config {
                reason: "FullAccess commands must use the owned host-process backend".to_string(),
            });
        }
        if limits.memory_bytes < 16 * 1024 * 1024 {
            return Err(SandboxError::ResourceLimitExceeded {
                resource: "memory".to_string(),
                limit: "Docker sandbox memory must be at least 16 MiB (zero disables the limit)"
                    .to_string(),
            });
        }
        if !(2..=262_144).contains(&limits.cpu_shares) {
            return Err(SandboxError::ResourceLimitExceeded {
                resource: "cpu_shares".to_string(),
                limit: "Docker CPU shares must be between 2 and 262144".to_string(),
            });
        }
        if command.len() > MAX_COMMAND_BYTES || command.contains('\0') {
            return Err(SandboxError::Config {
                reason: "sandbox command is oversized or contains NUL".to_string(),
            });
        }
        if limits.timeout.is_zero() || limits.timeout > MAX_TIMEOUT {
            return Err(SandboxError::ResourceLimitExceeded {
                resource: "timeout".to_string(),
                limit: "sandbox timeout must be between one second and seven days".to_string(),
            });
        }
        if !(1..=MAX_OUTPUT_BYTES).contains(&limits.max_output_bytes) {
            return Err(SandboxError::ResourceLimitExceeded {
                resource: "output".to_string(),
                limit: format!("sandbox output limit must be between 1 and {MAX_OUTPUT_BYTES}"),
            });
        }
        if env.len() > MAX_ENVIRONMENT_ENTRIES {
            return Err(SandboxError::Config {
                reason: format!(
                    "sandbox environment has too many entries (maximum {MAX_ENVIRONMENT_ENTRIES})"
                ),
            });
        }
        let mut environment_bytes = 0usize;
        for (name, value) in &env {
            let valid_name = !name.is_empty()
                && name.len() <= 256
                && name.bytes().enumerate().all(|(index, byte)| match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'_' => true,
                    b'0'..=b'9' => index > 0,
                    _ => false,
                });
            if !valid_name || value.contains('\0') {
                return Err(SandboxError::Config {
                    reason: "sandbox environment contains an invalid entry".to_string(),
                });
            }
            environment_bytes = environment_bytes
                .checked_add(name.len())
                .and_then(|size| size.checked_add(value.len()))
                .and_then(|size| size.checked_add(1))
                .ok_or_else(|| SandboxError::Config {
                    reason: "sandbox environment size overflow".to_string(),
                })?;
        }
        if environment_bytes > MAX_ENVIRONMENT_BYTES {
            return Err(SandboxError::Config {
                reason: format!(
                    "sandbox environment exceeds the {MAX_ENVIRONMENT_BYTES} byte limit"
                ),
            });
        }
        let canonical_working_dir =
            working_dir
                .canonicalize()
                .map_err(|error| SandboxError::Config {
                    reason: format!("failed to resolve sandbox workspace: {error}"),
                })?;
        if !canonical_working_dir.is_dir() {
            return Err(SandboxError::Config {
                reason: "sandbox workspace is not a directory".to_string(),
            });
        }
        let working_dir_str = canonical_working_dir
            .to_str()
            .ok_or_else(|| SandboxError::Config {
                reason: "sandbox workspace path is not valid UTF-8".to_string(),
            })?
            .to_string();
        if working_dir_str.len() > 4096 {
            return Err(SandboxError::Config {
                reason: "sandbox workspace path is oversized".to_string(),
            });
        }
        let memory_limit = i64::try_from(limits.memory_bytes).map_err(|_| {
            SandboxError::ResourceLimitExceeded {
                resource: "memory".to_string(),
                limit: format!(
                    "{} bytes exceeds Docker's signed limit",
                    limits.memory_bytes
                ),
            }
        })?;

        // Build environment variables
        let mut env_vec: Vec<String> = env
            .into_iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        // Environment-level proxy configuration is not a security boundary:
        // arbitrary code can ignore it. An internal Docker network below
        // removes direct egress, while this authenticated proxy provides the
        // only intended path back out.
        let sandbox_network = if allow_network && policy.is_sandboxed() {
            if self.proxy_token.as_deref().is_none_or(str::is_empty) || relay.is_none() {
                return Err(SandboxError::ProxyError {
                    reason: "authenticated proxy relay is unavailable for networked sandbox"
                        .to_string(),
                });
            }
            relay
        } else {
            None
        };

        if let (Some(relay), Some(proxy_token)) = (sandbox_network, self.proxy_token.as_deref()) {
            let proxy_url = format!(
                "http://thinclaw:{proxy_token}@{gateway_host}:{}",
                RELAY_PROXY_PORT,
                gateway_host = relay.gateway_host(),
            );
            env_vec.push(format!("http_proxy={proxy_url}"));
            env_vec.push(format!("https_proxy={proxy_url}"));
            env_vec.push(format!("HTTP_PROXY={proxy_url}"));
            env_vec.push(format!("HTTPS_PROXY={proxy_url}"));
        }

        // Build volume mounts based on policy
        let mounts = vec![Mount {
            target: Some("/workspace".to_string()),
            source: Some(working_dir_str),
            typ: Some(MountType::BIND),
            read_only: Some(policy == SandboxPolicy::ReadOnly),
            ..Default::default()
        }];
        let nano_cpus = i64::from(limits.cpu_shares)
            .saturating_mul(1_000_000_000)
            .checked_div(1024)
            .unwrap_or(1_000_000_000)
            .max(10_000_000);

        let host_config = HostConfig {
            mounts: Some(mounts),
            memory: Some(memory_limit),
            memory_swap: Some(memory_limit),
            memory_swappiness: Some(0),
            cpu_shares: Some(limits.cpu_shares as i64),
            // Shares alone are only relative scheduling weight. Derive an
            // explicit quota where the default 1024 shares equals one CPU.
            nano_cpus: Some(nano_cpus),
            pids_limit: Some(512),
            ulimits: Some(vec![ResourcesUlimits {
                name: Some("nofile".to_string()),
                soft: Some(4096),
                hard: Some(4096),
            }]),
            // Keep the container around until after log collection completes.
            // Fast-running commands can otherwise disappear before `docker logs`
            // has a chance to read their stdout/stderr.
            auto_remove: Some(false),
            network_mode: Some(match sandbox_network.as_ref() {
                Some(relay) => relay.network_name().to_string(),
                None if allow_network => "bridge".to_string(),
                None => "none".to_string(),
            }),
            // Security: the sandbox user needs no Linux capabilities.
            cap_drop: Some(vec!["ALL".to_string()]),
            // Prevent privilege escalation
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            // Read-only root filesystem (workspace is still writable if policy allows)
            readonly_rootfs: Some(true),
            // Tmpfs mounts for /tmp and cargo cache
            tmpfs: Some(
                [
                    (
                        "/tmp".to_string(),
                        "rw,nosuid,nodev,size=512M,mode=1777".to_string(),
                    ),
                    (
                        "/home/sandbox/.cargo/registry".to_string(),
                        "rw,nosuid,size=1G,mode=1777".to_string(),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let user = container_user_for_workspace(&canonical_working_dir);

        let config = ContainerCreateBody {
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
            labels: Some(managed_container_labels(
                &self.runtime_scope,
                "ephemeral",
                None,
            )),
            ..Default::default()
        };

        let options = CreateContainerOptionsBuilder::new()
            .name(container_name)
            .build();

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
            Some(
                WaitContainerOptionsBuilder::new()
                    .condition("not-running")
                    .build(),
            ),
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
                .inspect_container(container_id, None)
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
        let options = LogsOptionsBuilder::new()
            .stdout(true)
            .stderr(true)
            .follow(false)
            .build();

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
