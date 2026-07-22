//! Docker execution sandbox for secure command execution.
//!
//! This module provides a complete sandboxing solution for running untrusted commands:
//! - **Container isolation**: Commands run in ephemeral Docker containers
//! - **Network proxy**: All network traffic goes through a validating proxy
//! - **Credential scoping**: Network and worker credentials are per-job and revoked on completion
//! - **Resource limits**: Memory, CPU, and timeout enforcement
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                           Sandbox System                                     │
//! │                                                                              │
//! │  ┌─────────────────────────────────────────────────────────────────────┐    │
//! │  │                        SandboxManager                                │    │
//! │  │                                                                      │    │
//! │  │  • Coordinates container creation and execution                     │    │
//! │  │  • Manages proxy lifecycle                                          │    │
//! │  │  • Enforces resource limits                                         │    │
//! │  └─────────────────────────────────────────────────────────────────────┘    │
//! │           │                              │                                   │
//! │           ▼                              ▼                                   │
//! │  ┌──────────────────┐          ┌───────────────────┐                        │
//! │  │   Container      │          │   Network Proxy   │                        │
//! │  │   Runner         │          │                   │                        │
//! │  │                  │          │  • Allowlist      │                        │
//! │  │  • Create        │◀────────▶│  • Credentials    │                        │
//! │  │  • Execute       │          │  • Logging        │                        │
//! │  │  • Cleanup       │          │                   │                        │
//! │  └──────────────────┘          └───────────────────┘                        │
//! │           │                              │                                   │
//! │           ▼                              ▼                                   │
//! │  ┌──────────────────┐          ┌───────────────────┐                        │
//! │  │     Docker       │          │     Internet      │                        │
//! │  │                  │          │   (allowed hosts) │                        │
//! │  └──────────────────┘          └───────────────────┘                        │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Sandbox Policies
//!
//! | Policy | Filesystem | Network | Use Case |
//! |--------|------------|---------|----------|
//! | `ReadOnly` | Read workspace | Proxied | Explore code, fetch docs |
//! | `WorkspaceWrite` | Read/write workspace | Proxied | Build software, run tests |
//! | `FullAccess` | Full host | Full | Direct execution (no sandbox) |
//!
//! # Example
//!
//! ```rust,no_run
//! use thinclaw::sandbox::{SandboxManager, SandboxManagerBuilder, SandboxPolicy};
//! use std::collections::HashMap;
//! use std::path::Path;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let manager = SandboxManagerBuilder::new()
//!     .enabled(true)
//!     .policy(SandboxPolicy::WorkspaceWrite)
//!     .build();
//!
//! manager.initialize().await?;
//!
//! let result = manager.execute(
//!     "cargo build --release",
//!     Path::new("/workspace/my-project"),
//!     HashMap::new(),
//! ).await?;
//!
//! println!("Exit code: {}", result.exit_code);
//! println!("Output: {}", result.output);
//!
//! manager.shutdown().await;
//! # Ok(())
//! # }
//! ```
//!
//! # Security Properties
//!
//! - **Scoped credentials**: Workers receive explicit grants, ephemeral control/proxy tokens, and only the provider credential required by their selected CLI mode
//! - **Network isolation**: Sandboxed traffic uses an internal bridge and authenticated allowlisting proxy
//! - **Non-root execution**: Containers run as an unprivileged workspace owner
//! - **Read-only root**: Container filesystem is read-only (except workspace mount)
//! - **Capability dropping**: All Linux capabilities are dropped
//! - **Auto-cleanup**: Containers are explicitly removed after execution, including cancellation paths
//! - **Timeout enforcement**: Commands are killed after the timeout

pub mod config;
#[cfg(feature = "docker-sandbox")]
pub mod container;
pub mod detect;
#[cfg(feature = "browser")]
pub mod docker_chromium;
pub mod docker_init;
pub mod error;
mod host_process;
#[cfg(feature = "docker-sandbox")]
pub mod manager;
#[cfg(feature = "docker-sandbox")]
pub(crate) mod network;
pub mod podman;
pub mod proxy;
#[cfg(feature = "docker-sandbox")]
pub mod relay;

pub use config::{ResourceLimits, SandboxConfig, SandboxPolicy};
#[cfg(feature = "docker-sandbox")]
pub use container::{ContainerOutput, ContainerRunner, connect_docker};
#[cfg(not(feature = "docker-sandbox"))]
pub use container_stub::{ContainerOutput, ContainerRunner, connect_docker};
pub use detect::{DockerDetection, DockerStatus, Platform, check_docker};
pub use error::{Result, SandboxError};
#[cfg(feature = "docker-sandbox")]
pub use manager::{ExecOutput, SandboxManager, SandboxManagerBuilder};
#[cfg(not(feature = "docker-sandbox"))]
pub use manager_stub::{ExecOutput, SandboxManager, SandboxManagerBuilder};
#[cfg(feature = "docker-sandbox")]
pub use network::cleanup_stale_sandbox_resources;
pub use proxy::{
    CredentialResolver, DefaultPolicyDecider, DomainAllowlist, EnvCredentialResolver, HttpProxy,
    NetworkDecision, NetworkPolicyDecider, NetworkProxyBuilder, NetworkRequest,
};

/// Default allowlist getter (re-export for convenience).
pub fn default_allowlist() -> Vec<String> {
    config::default_allowlist()
}

/// Default credential mappings getter (re-export for convenience).
pub fn default_credential_mappings() -> Vec<crate::secrets::CredentialMapping> {
    config::default_credential_mappings()
}

#[cfg(not(feature = "docker-sandbox"))]
mod container_stub {
    use std::time::Duration;

    use super::{Result, SandboxError};

    #[derive(Debug, Clone)]
    pub struct ContainerOutput {
        pub exit_code: i64,
        pub stdout: String,
        pub stderr: String,
        pub duration: Duration,
        pub truncated: bool,
    }

    pub struct ContainerRunner;

    pub async fn connect_docker() -> Result<()> {
        Err(SandboxError::DockerNotAvailable {
            reason: "ThinClaw was built without the docker-sandbox feature".to_string(),
        })
    }
}

#[cfg(not(feature = "docker-sandbox"))]
mod manager_stub {
    use std::collections::HashMap;
    use std::path::Path;
    use std::time::Duration;

    use super::{Result, SandboxConfig, SandboxError, SandboxPolicy};

    #[derive(Debug, Clone)]
    pub struct ExecOutput {
        pub exit_code: i64,
        pub stdout: String,
        pub stderr: String,
        pub output: String,
        pub duration: Duration,
        pub truncated: bool,
    }

    #[derive(Debug)]
    pub struct SandboxManager {
        config: SandboxConfig,
        initialized: std::sync::atomic::AtomicBool,
    }

    impl SandboxManager {
        pub fn new(config: SandboxConfig) -> Self {
            Self {
                config,
                initialized: std::sync::atomic::AtomicBool::new(false),
            }
        }

        pub fn with_defaults() -> Self {
            Self::new(SandboxConfig::default())
        }

        /// API-compatible no-op with the docker-sandbox manager. Reduced
        /// builds never create managed containers, so there is no resource
        /// scope to bind.
        pub fn with_runtime_scope(self, _runtime_scope: impl Into<String>) -> Self {
            self
        }

        /// API-compatible no-op with the docker-sandbox manager.
        ///
        /// Without the `docker-sandbox` feature there is no network proxy to
        /// resolve credentials for, so the store is ignored. The signature is
        /// kept identical so callers compile against either build.
        pub fn with_credential_store(
            self,
            _store: std::sync::Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
            _user_id: impl Into<String>,
        ) -> Self {
            self
        }

        pub async fn is_available(&self) -> bool {
            false
        }

        pub async fn initialize(&self) -> Result<()> {
            if !self.config.enabled {
                return Err(SandboxError::Config {
                    reason: "sandbox is disabled".to_string(),
                });
            }
            Err(SandboxError::DockerNotAvailable {
                reason: "ThinClaw was built without the docker-sandbox feature".to_string(),
            })
        }

        pub async fn shutdown(&self) {
            self.initialized
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }

        pub async fn execute(
            &self,
            command: &str,
            cwd: &Path,
            env: HashMap<String, String>,
        ) -> Result<ExecOutput> {
            self.execute_with_policy_and_network(command, cwd, self.config.policy, env, true)
                .await
        }

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

        pub async fn execute_with_policy_and_network(
            &self,
            command: &str,
            cwd: &Path,
            policy: SandboxPolicy,
            env: HashMap<String, String>,
            _allow_network: bool,
        ) -> Result<ExecOutput> {
            if policy == SandboxPolicy::FullAccess {
                return self.execute_direct(command, cwd, env).await;
            }
            Err(SandboxError::DockerNotAvailable {
                reason: "ThinClaw was built without the docker-sandbox feature".to_string(),
            })
        }

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

        pub fn config(&self) -> &SandboxConfig {
            &self.config
        }

        pub fn is_initialized(&self) -> bool {
            self.initialized.load(std::sync::atomic::Ordering::SeqCst)
        }

        pub async fn proxy_port(&self) -> Option<u16> {
            None
        }
    }

    pub struct SandboxManagerBuilder {
        config: SandboxConfig,
    }

    impl SandboxManagerBuilder {
        pub fn new() -> Self {
            Self {
                config: SandboxConfig::default(),
            }
        }

        pub fn enabled(mut self, enabled: bool) -> Self {
            self.config.enabled = enabled;
            self
        }

        pub fn policy(mut self, policy: SandboxPolicy) -> Self {
            self.config.policy = policy;
            self
        }

        pub fn timeout(mut self, timeout: Duration) -> Self {
            self.config.timeout = timeout;
            self
        }

        pub fn memory_limit_mb(mut self, mb: u64) -> Self {
            self.config.memory_limit_mb = mb;
            self
        }

        pub fn image(mut self, image: &str) -> Self {
            self.config.image = image.to_string();
            self
        }

        pub fn allow_domains(mut self, domains: Vec<String>) -> Self {
            self.config.network_allowlist.extend(domains);
            self
        }

        pub fn build(self) -> SandboxManager {
            SandboxManager::new(self.config)
        }

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
}
