//! Main sandbox manager coordinating proxy and containers.
//!
//! The `SandboxManager` is the primary entry point for sandboxed execution.
//! It coordinates:
//! - Docker container creation and lifecycle
//! - HTTP proxy for network access control
//! - Credential injection for API calls
//! - Resource limits and timeouts
//!
//! # Architecture
//!
//! ```text
//! ┌───────────────────────────────────────────────────────────────────────────┐
//! │                           SandboxManager                                   │
//! │                                                                            │
//! │   execute(cmd, cwd, policy)                                                │
//! │         │                                                                  │
//! │         ▼                                                                  │
//! │   ┌──────────────┐     ┌──────────────┐     ┌──────────────────────────┐  │
//! │   │ Start Proxy  │────▶│ Create       │────▶│ Execute & Collect Output │  │
//! │   │ (if needed)  │     │ Container    │     │                          │  │
//! │   └──────────────┘     └──────────────┘     └──────────────────────────┘  │
//! │                                                        │                   │
//! │                                                        ▼                   │
//! │                                              ┌──────────────────────────┐  │
//! │                                              │ Cleanup Container        │  │
//! │                                              └──────────────────────────┘  │
//! └───────────────────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, Notify, RwLock};

use bollard::Docker;

use crate::sandbox::config::{ResourceLimits, SandboxConfig, SandboxPolicy};
use crate::sandbox::container::{ContainerOutput, ContainerRunner, connect_docker};
use crate::sandbox::error::{Result, SandboxError};
use crate::sandbox::proxy::{HttpProxy, NetworkProxyBuilder, validate_domain_allowlist};
use crate::secrets::SecretsStore;

/// Output from sandbox execution.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    /// Exit code from the command.
    pub exit_code: i64,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// Combined output (stdout + stderr).
    pub output: String,
    /// How long the command ran.
    pub duration: Duration,
    /// Whether output was truncated.
    pub truncated: bool,
}

impl From<ContainerOutput> for ExecOutput {
    fn from(c: ContainerOutput) -> Self {
        let output = if c.stderr.is_empty() {
            c.stdout.clone()
        } else if c.stdout.is_empty() {
            c.stderr.clone()
        } else {
            format!("{}\n\n--- stderr ---\n{}", c.stdout, c.stderr)
        };

        Self {
            exit_code: c.exit_code,
            stdout: c.stdout,
            stderr: c.stderr,
            output,
            duration: c.duration,
            truncated: c.truncated,
        }
    }
}

/// Main sandbox manager.
pub struct SandboxManager {
    config: SandboxConfig,
    runtime_scope: String,
    proxy: Arc<RwLock<Option<HttpProxy>>>,
    docker: Arc<RwLock<Option<Docker>>>,
    initialized: AtomicBool,
    accepting_executions: AtomicBool,
    execution_admission: std::sync::Mutex<()>,
    active_executions: AtomicUsize,
    execution_notify: Notify,
    lifecycle_lock: Mutex<()>,
    /// Optional encrypted-store credential source and the owning user the proxy
    /// resolves credentials for. When set, the network proxy resolves forwarded
    /// credentials from the [`SecretsStore`] instead of process env.
    ///
    /// The proxy is owned per-manager (shared across the jobs that run through
    /// it), so a single owning `user_id` is bound here at construction. On a
    /// single-operator desktop deployment this is the `"default"` owner; a
    /// future multi-tenant proxy would need per-request user context threaded
    /// through `CredentialResolver::resolve` instead.
    credential_store: Option<(Arc<dyn SecretsStore + Send + Sync>, String)>,
}

struct ActiveExecutionGuard<'a> {
    manager: &'a SandboxManager,
}

impl Drop for ActiveExecutionGuard<'_> {
    fn drop(&mut self) {
        self.manager
            .active_executions
            .fetch_sub(1, Ordering::AcqRel);
        self.manager.execution_notify.notify_waiters();
    }
}

impl SandboxManager {
    /// Create a new sandbox manager.
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config,
            runtime_scope: crate::runtime_lease::runtime_scope_id_for_path(
                &crate::platform::resolve_data_dir(""),
            ),
            proxy: Arc::new(RwLock::new(None)),
            docker: Arc::new(RwLock::new(None)),
            initialized: AtomicBool::new(false),
            accepting_executions: AtomicBool::new(true),
            execution_admission: std::sync::Mutex::new(()),
            active_executions: AtomicUsize::new(0),
            execution_notify: Notify::new(),
            lifecycle_lock: Mutex::new(()),
            credential_store: None,
        }
    }

    /// Bind containers created by this manager to a durable runtime identity.
    pub fn with_runtime_scope(mut self, runtime_scope: impl Into<String>) -> Self {
        self.runtime_scope = runtime_scope.into();
        self
    }

    /// Attach an encrypted [`SecretsStore`]-backed credential source for the
    /// network proxy, owned by `user_id`.
    ///
    /// When present, [`Self::initialize`] builds the proxy with
    /// [`NetworkProxyBuilder::from_config_with_store`] so credentials forwarded
    /// to allowlisted hosts are resolved from the encrypted store (audited
    /// injection path) rather than the host process environment. Without a
    /// store, the manager falls back to the env-backed resolver.
    pub fn with_credential_store(
        mut self,
        store: Arc<dyn SecretsStore + Send + Sync>,
        user_id: impl Into<String>,
    ) -> Self {
        self.credential_store = Some((store, user_id.into()));
        self
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(SandboxConfig::default())
    }

    fn validate_config_for_policy(&self, policy: SandboxPolicy) -> Result<()> {
        const MAX_TIMEOUT: Duration = Duration::from_secs(7 * 24 * 60 * 60);
        if self.config.timeout.is_zero() || self.config.timeout > MAX_TIMEOUT {
            return Err(SandboxError::Config {
                reason: "sandbox timeout must be between one second and seven days".to_string(),
            });
        }
        if !policy.is_sandboxed() {
            return Ok(());
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
        if self.config.image.trim().is_empty()
            || self.config.image.len() > 512
            || self.config.image.chars().any(char::is_control)
        {
            return Err(SandboxError::Config {
                reason: "sandbox image name is empty, oversized, or invalid".to_string(),
            });
        }
        if self.config.memory_limit_mb < 16
            || self
                .config
                .memory_limit_mb
                .checked_mul(1024 * 1024)
                .and_then(|bytes| i64::try_from(bytes).ok())
                .is_none()
        {
            return Err(SandboxError::Config {
                reason: "sandbox memory limit is outside Docker's supported range".to_string(),
            });
        }
        if !(2..=262_144).contains(&self.config.cpu_shares) {
            return Err(SandboxError::Config {
                reason: "sandbox CPU shares must be between 2 and 262144".to_string(),
            });
        }
        validate_domain_allowlist(&self.config.network_allowlist)
            .map_err(|reason| SandboxError::Config { reason })?;
        if let Some((_, owner)) = self.credential_store.as_ref()
            && (owner.trim().is_empty() || owner.len() > 256 || owner.chars().any(char::is_control))
        {
            return Err(SandboxError::Config {
                reason: "sandbox credential owner identity is invalid".to_string(),
            });
        }
        Ok(())
    }

    /// Check if the sandbox is available (Docker running, etc.).
    pub async fn is_available(&self) -> bool {
        if !self.config.enabled {
            return false;
        }

        match connect_docker().await {
            Ok(docker) => docker.ping().await.is_ok(),
            Err(_) => false,
        }
    }

    /// Initialize the sandbox (connect to Docker, start proxy).
    pub async fn initialize(&self) -> Result<()> {
        let _lifecycle = self.lifecycle_lock.lock().await;
        if !self.accepting_executions.load(Ordering::Acquire) {
            return Err(SandboxError::Config {
                reason: "sandbox manager has been shut down".to_string(),
            });
        }
        if self.initialized.load(Ordering::Acquire) {
            return Ok(());
        }

        if !self.config.enabled {
            return Err(SandboxError::Config {
                reason: "sandbox is disabled".to_string(),
            });
        }
        self.validate_config_for_policy(SandboxPolicy::ReadOnly)?;

        // Connect to Docker
        let docker = connect_docker().await?;

        // Check if Docker is responsive
        docker
            .ping()
            .await
            .map_err(|e| SandboxError::DockerNotAvailable {
                reason: e.to_string(),
            })?;

        // Check for / pull image using a temporary runner
        let checker = ContainerRunner::new(
            docker.clone(),
            self.config.image.clone(),
            self.config.proxy_port,
        );
        if !checker.image_exists().await {
            if self.config.auto_pull_image {
                checker.pull_image().await?;
            } else {
                return Err(SandboxError::ContainerCreationFailed {
                    reason: format!(
                        "image {} not found and auto_pull is disabled",
                        self.config.image
                    ),
                });
            }
        }

        *self.docker.write().await = Some(docker);

        // `execute_with_policy` may request a sandboxed policy even when the
        // manager's default policy is FullAccess. Initialize the sandbox proxy
        // whenever the sandbox runtime itself is initialized, and force its
        // network decider to the allowlist-backed policy.
        //
        // Prefer the encrypted secrets store as the credential source when one
        // was attached (`with_credential_store`); otherwise fall back to the
        // process-env resolver. Resolving from the store keeps API credentials
        // out of the sandbox manager's environment and routes access through the
        // audited injection path.
        let builder = match self.credential_store.as_ref() {
            Some((store, user_id)) => {
                tracing::debug!(
                    owner = %user_id,
                    "Sandbox network proxy using encrypted secrets store for credentials"
                );
                NetworkProxyBuilder::from_config_with_store(
                    &self.config,
                    Arc::clone(store),
                    user_id.clone(),
                )
            }
            None => {
                tracing::debug!(
                    "Sandbox network proxy using process-env credential resolver (no secrets store attached)"
                );
                NetworkProxyBuilder::from_config(&self.config)
            }
        }
        .with_policy(SandboxPolicy::ReadOnly);

        let proxy = builder.build_and_start(self.config.proxy_port).await?;
        *self.proxy.write().await = Some(proxy);

        self.initialized.store(true, Ordering::Release);

        tracing::info!("Sandbox initialized");
        Ok(())
    }

    /// Shutdown the sandbox (stop proxy, clean up).
    pub async fn shutdown(&self) {
        {
            let _admission = self
                .execution_admission
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.accepting_executions.store(false, Ordering::Release);
        }
        let drain = async {
            loop {
                let notified = self.execution_notify.notified();
                if self.active_executions.load(Ordering::Acquire) == 0 {
                    break;
                }
                notified.await;
            }
        };
        if tokio::time::timeout(Duration::from_secs(30), drain)
            .await
            .is_err()
        {
            tracing::warn!(
                remaining = self.active_executions.load(Ordering::Acquire),
                "Timed out draining active sandbox executions"
            );
        }
        let _lifecycle = self.lifecycle_lock.lock().await;
        if let Some(proxy) = self.proxy.write().await.take() {
            proxy.stop().await;
        }

        self.initialized.store(false, Ordering::Release);

        tracing::info!("Sandbox shut down");
    }

    /// Execute a command in the sandbox.
    pub async fn execute(
        &self,
        command: &str,
        cwd: &Path,
        env: HashMap<String, String>,
    ) -> Result<ExecOutput> {
        self.execute_with_policy_and_network(command, cwd, self.config.policy, env, true)
            .await
    }

    /// Execute a command with a specific policy.
    pub async fn execute_with_policy(
        &self,
        command: &str,
        cwd: &Path,
        policy: SandboxPolicy,
        env: HashMap<String, String>,
    ) -> Result<ExecOutput> {
        self.execute_with_policy_and_network(command, cwd, policy, env, true)
            .await
    }

    /// Execute a command with a specific policy and explicit network setting.
    pub async fn execute_with_policy_and_network(
        &self,
        command: &str,
        cwd: &Path,
        policy: SandboxPolicy,
        env: HashMap<String, String>,
        allow_network: bool,
    ) -> Result<ExecOutput> {
        self.validate_config_for_policy(policy)?;
        let _execution = {
            let _admission = self
                .execution_admission
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !self.accepting_executions.load(Ordering::Acquire) {
                return Err(SandboxError::Config {
                    reason: "sandbox manager is shutting down".to_string(),
                });
            }
            self.active_executions.fetch_add(1, Ordering::AcqRel);
            ActiveExecutionGuard { manager: self }
        };

        // FullAccess policy bypasses the sandbox entirely
        if policy == SandboxPolicy::FullAccess {
            return self.execute_direct(command, cwd, env).await;
        }

        // Ensure we're initialized
        if !self.initialized.load(Ordering::Acquire) {
            self.initialize().await?;
        }
        if !self.accepting_executions.load(Ordering::Acquire) {
            return Err(SandboxError::Config {
                reason: "sandbox manager began shutting down during initialization".to_string(),
            });
        }

        // Get proxy port if running
        let (proxy_port, proxy_token) = if let Some(proxy) = self.proxy.read().await.as_ref() {
            (
                proxy
                    .addr()
                    .await
                    .map(|address| address.port())
                    .unwrap_or(0),
                Some(proxy.proxy_token()),
            )
        } else {
            (0, None)
        };

        // Reuse the stored Docker connection, create a runner with the current proxy port
        let docker =
            self.docker
                .read()
                .await
                .clone()
                .ok_or_else(|| SandboxError::DockerNotAvailable {
                    reason: "Docker connection not initialized".to_string(),
                })?;
        let runner = ContainerRunner::new(docker, self.config.image.clone(), proxy_port)
            .with_runtime_scope(self.runtime_scope.clone())
            .with_proxy_token(proxy_token);

        let limits = ResourceLimits {
            memory_bytes: self.config.memory_limit_mb * 1024 * 1024,
            cpu_shares: self.config.cpu_shares,
            timeout: self.config.timeout,
            max_output_bytes: 64 * 1024,
        };

        let container_output = runner
            .execute(command, cwd, policy, &limits, env, allow_network)
            .await?;

        Ok(container_output.into())
    }

    /// Execute a command directly on the host (no sandbox).
    async fn execute_direct(
        &self,
        command: &str,
        cwd: &Path,
        env: HashMap<String, String>,
    ) -> Result<ExecOutput> {
        let result = super::host_process::execute_host_command(
            command,
            cwd,
            env,
            self.config.timeout,
            64 * 1024,
        )
        .await?;
        let combined = if result.stderr.is_empty() {
            result.stdout.clone()
        } else if result.stdout.is_empty() {
            result.stderr.clone()
        } else {
            format!("{}\n\n--- stderr ---\n{}", result.stdout, result.stderr)
        };

        Ok(ExecOutput {
            exit_code: result.exit_code,
            stdout: result.stdout,
            stderr: result.stderr,
            output: combined,
            duration: result.duration,
            truncated: result.truncated,
        })
    }

    /// Execute a build command (convenience method using WorkspaceWrite policy).
    pub async fn build(
        &self,
        command: &str,
        project_dir: &Path,
        env: HashMap<String, String>,
    ) -> Result<ExecOutput> {
        self.execute_with_policy_and_network(
            command,
            project_dir,
            SandboxPolicy::WorkspaceWrite,
            env,
            true,
        )
        .await
    }

    /// Get the current configuration.
    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// Check if the sandbox is initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    /// Get the proxy port if running.
    pub async fn proxy_port(&self) -> Option<u16> {
        if let Some(proxy) = self.proxy.read().await.as_ref() {
            proxy.addr().await.map(|a| a.port())
        } else {
            None
        }
    }
}

impl Drop for SandboxManager {
    fn drop(&mut self) {
        // Note: async cleanup should be done via shutdown() before dropping
        if self.initialized.load(Ordering::Acquire) {
            tracing::warn!("SandboxManager dropped without shutdown(), resources may leak");
        }
    }
}

/// Builder for creating a sandbox manager.
pub struct SandboxManagerBuilder {
    config: SandboxConfig,
}

impl SandboxManagerBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            config: SandboxConfig::default(),
        }
    }

    /// Enable the sandbox.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.config.enabled = enabled;
        self
    }

    /// Set the sandbox policy.
    pub fn policy(mut self, policy: SandboxPolicy) -> Self {
        self.config.policy = policy;
        self
    }

    /// Set the command timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Set the memory limit in MB.
    pub fn memory_limit_mb(mut self, mb: u64) -> Self {
        self.config.memory_limit_mb = mb;
        self
    }

    /// Set the Docker image.
    pub fn image(mut self, image: &str) -> Self {
        self.config.image = image.to_string();
        self
    }

    /// Add domains to the network allowlist.
    pub fn allow_domains(mut self, domains: Vec<String>) -> Self {
        self.config.network_allowlist.extend(domains);
        self
    }

    /// Build the sandbox manager.
    pub fn build(self) -> SandboxManager {
        SandboxManager::new(self.config)
    }

    /// Build and initialize the sandbox manager.
    pub async fn build_and_init(self) -> Result<SandboxManager> {
        let manager = self.build();
        manager.initialize().await?;
        Ok(manager)
    }
}

impl Default for SandboxManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exec_output_from_container_output() {
        let container = ContainerOutput {
            exit_code: 0,
            stdout: "hello".to_string(),
            stderr: String::new(),
            duration: Duration::from_secs(1),
            truncated: false,
        };

        let exec: ExecOutput = container.into();
        assert_eq!(exec.exit_code, 0);
        assert_eq!(exec.output, "hello");
    }

    #[test]
    fn test_exec_output_combined() {
        let container = ContainerOutput {
            exit_code: 1,
            stdout: "out".to_string(),
            stderr: "err".to_string(),
            duration: Duration::from_secs(1),
            truncated: false,
        };

        let exec: ExecOutput = container.into();
        assert!(exec.output.contains("out"));
        assert!(exec.output.contains("err"));
        assert!(exec.output.contains("stderr"));
    }

    #[test]
    fn test_builder_defaults() {
        let manager = SandboxManagerBuilder::new().build();
        assert!(manager.config.enabled); // Enabled by default (startup check disables if Docker unavailable)
    }

    #[test]
    fn test_default_manager_has_no_credential_store() {
        let manager = SandboxManager::new(SandboxConfig::default());
        // Without a store attached the proxy falls back to the env resolver.
        assert!(manager.credential_store.is_none());
    }

    #[test]
    fn test_with_credential_store_records_owner() {
        use crate::secrets::{InMemorySecretsStore, SecretsCrypto};

        let crypto = Arc::new(
            SecretsCrypto::new(secrecy::SecretString::from(
                "0123456789abcdef0123456789abcdef".to_string(),
            ))
            .expect("test crypto key is valid"),
        );
        let store: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));

        let manager =
            SandboxManager::new(SandboxConfig::default()).with_credential_store(store, "default");

        let (_, owner) = manager
            .credential_store
            .as_ref()
            .expect("credential store should be attached");
        assert_eq!(owner, "default");
    }

    #[test]
    fn test_builder_custom() {
        let manager = SandboxManagerBuilder::new()
            .enabled(true)
            .policy(SandboxPolicy::WorkspaceWrite)
            .timeout(Duration::from_secs(60))
            .memory_limit_mb(1024)
            .image("custom:latest")
            .build();

        assert!(manager.config.enabled);
        assert_eq!(manager.config.policy, SandboxPolicy::WorkspaceWrite);
        assert_eq!(manager.config.timeout, Duration::from_secs(60));
        assert_eq!(manager.config.memory_limit_mb, 1024);
        assert_eq!(manager.config.image, "custom:latest");
    }

    #[test]
    fn sandbox_configuration_fails_closed_before_docker_access() {
        let invalid_image = SandboxManager::new(SandboxConfig {
            policy: SandboxPolicy::FullAccess,
            image: "\n".to_string(),
            ..Default::default()
        });
        assert!(
            invalid_image
                .validate_config_for_policy(SandboxPolicy::FullAccess)
                .is_ok(),
            "host execution should not depend on unused container configuration"
        );
        assert!(
            invalid_image
                .validate_config_for_policy(SandboxPolicy::ReadOnly)
                .is_err()
        );

        let invalid_allowlist = SandboxManager::new(SandboxConfig {
            network_allowlist: vec!["https://example.com".to_string()],
            ..Default::default()
        });
        assert!(
            invalid_allowlist
                .validate_config_for_policy(SandboxPolicy::ReadOnly)
                .is_err()
        );

        let invalid_scope =
            SandboxManager::new(SandboxConfig::default()).with_runtime_scope("scope/escape");
        assert!(
            invalid_scope
                .validate_config_for_policy(SandboxPolicy::ReadOnly)
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_direct_execution() {
        let manager = SandboxManager::new(SandboxConfig {
            enabled: true,
            policy: SandboxPolicy::FullAccess,
            ..Default::default()
        });

        let result = manager
            .execute("echo hello", Path::new("."), HashMap::new())
            .await;

        // This should work even without Docker since FullAccess runs directly
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_direct_execution_truncates_large_output() {
        let manager = SandboxManager::new(SandboxConfig {
            enabled: true,
            policy: SandboxPolicy::FullAccess,
            ..Default::default()
        });

        // Generate output larger than 32KB (half of 64KB limit)
        // printf repeats a 100-char line 400 times = 40KB
        let result = manager
            .execute(
                "printf 'A%.0s' $(seq 1 40000)",
                Path::new("."),
                HashMap::new(),
            )
            .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.truncated);
        assert!(output.stdout.len() <= 32 * 1024);
    }
}
