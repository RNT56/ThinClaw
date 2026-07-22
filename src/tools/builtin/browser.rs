//! Compatibility adapter and secure Docker lifecycle for the CDP browser tool.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use async_trait::async_trait;
pub use runtime_browser::{BrowserDockerRuntime, BrowserEgressRuntime, BrowserProxyConfig};
use thinclaw_tools::builtin::browser as runtime_browser;

#[cfg(feature = "docker-sandbox")]
use bollard::Docker;
#[cfg(feature = "docker-sandbox")]
use bollard::errors::Error as BollardError;
#[cfg(feature = "docker-sandbox")]
use bollard::models::{ContainerCreateBody, HostConfig, PortBinding};
#[cfg(feature = "docker-sandbox")]
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, RemoveContainerOptionsBuilder,
};
#[cfg(feature = "docker-sandbox")]
use futures::StreamExt;

use crate::context::JobContext;
use crate::sandbox::SandboxPolicy;
use crate::sandbox::docker_chromium::{DockerChromiumConfig, DockerError};
#[cfg(feature = "docker-sandbox")]
use crate::sandbox::network::{
    CONTAINER_KIND_LABEL, CONTAINER_SCOPE_LABEL, MANAGED_CONTAINER_LABEL, managed_container_labels,
};
use crate::sandbox::proxy::{NetworkProxyBuilder, NoCredentialResolver};
#[cfg(feature = "docker-sandbox")]
use crate::sandbox::relay::{RELAY_PROXY_PORT, RelayForward, SandboxNetworkRelay};
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};

#[derive(Clone, Default)]
pub struct RootBrowserEgressRuntime {
    proxy: Arc<tokio::sync::Mutex<Option<Arc<crate::sandbox::proxy::HttpProxy>>>>,
}

impl RootBrowserEgressRuntime {
    async fn start_inner(&self) -> Result<BrowserProxyConfig, String> {
        let mut guard = self.proxy.lock().await;
        if let Some(proxy) = guard.as_ref()
            && proxy.is_running()
            && let Some(address) = proxy.addr().await
        {
            return Ok(BrowserProxyConfig {
                endpoint: format!("http://127.0.0.1:{}", address.port()),
                username: "thinclaw".to_string(),
                password: proxy.proxy_token(),
            });
        }
        if let Some(stale) = guard.take() {
            stale.stop().await;
        }
        let proxy = Arc::new(
            NetworkProxyBuilder::new()
                .with_policy(SandboxPolicy::FullAccess)
                .with_credentials(Vec::new())
                .with_credential_resolver(Arc::new(NoCredentialResolver))
                .build(),
        );
        // Publish ownership before awaiting the listener so cancellation can be
        // recovered by the cleanup guard or registry shutdown.
        *guard = Some(proxy.clone());
        let address = proxy
            .start_loopback(0)
            .await
            .map_err(|error| format!("failed to start pinned browser proxy: {error}"))?;
        Ok(BrowserProxyConfig {
            endpoint: format!("http://127.0.0.1:{}", address.port()),
            username: "thinclaw".to_string(),
            password: proxy.proxy_token(),
        })
    }

    async fn stop_inner(&self) {
        if let Some(proxy) = self.proxy.lock().await.take() {
            proxy.stop().await;
        }
    }
}

struct EgressStartCleanup {
    runtime: RootBrowserEgressRuntime,
    armed: bool,
}

impl Drop for EgressStartCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Local browser proxy cleanup dropped outside a Tokio runtime");
            return;
        };
        let runtime = self.runtime.clone();
        handle.spawn(async move { runtime.stop_inner().await });
    }
}

#[async_trait]
impl BrowserEgressRuntime for RootBrowserEgressRuntime {
    async fn start(&self) -> Result<BrowserProxyConfig, String> {
        let mut cleanup = EgressStartCleanup {
            runtime: self.clone(),
            armed: true,
        };
        let config = self.start_inner().await?;
        cleanup.armed = false;
        Ok(config)
    }

    async fn stop(&self) -> Result<(), String> {
        self.stop_inner().await;
        Ok(())
    }
}

#[cfg(feature = "docker-sandbox")]
const DOCKER_BROWSER_OPERATION_TIMEOUT: Duration = Duration::from_secs(20);
#[cfg(feature = "docker-sandbox")]
const DOCKER_BROWSER_PULL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[cfg(feature = "docker-sandbox")]
#[derive(Default)]
struct RootBrowserDockerState {
    docker: Option<Docker>,
    container_target: Option<String>,
    proxy: Option<Arc<crate::sandbox::proxy::HttpProxy>>,
    relay: Option<SandboxNetworkRelay>,
}

#[derive(Clone)]
pub struct RootBrowserDockerRuntime {
    config: DockerChromiumConfig,
    host_port: Arc<AtomicU16>,
    proxy_credentials: Arc<std::sync::RwLock<Option<(String, String)>>>,
    #[cfg(feature = "docker-sandbox")]
    state: Arc<tokio::sync::Mutex<RootBrowserDockerState>>,
}

impl RootBrowserDockerRuntime {
    pub fn new(config: DockerChromiumConfig) -> Self {
        Self {
            config,
            host_port: Arc::new(AtomicU16::new(0)),
            proxy_credentials: Arc::new(std::sync::RwLock::new(None)),
            #[cfg(feature = "docker-sandbox")]
            state: Arc::new(tokio::sync::Mutex::new(RootBrowserDockerState::default())),
        }
    }

    #[cfg(feature = "docker-sandbox")]
    async fn ensure_image(docker: &Docker, image: &str, auto_pull: bool) -> Result<String, String> {
        if let Ok(inspect) = docker.inspect_image(image).await {
            return Self::validated_image_id(image, inspect.id);
        }
        if !auto_pull {
            return Err(format!("Docker image `{image}` is not available locally"));
        }
        tracing::info!(%image, "Pulling Docker browser dependency image");
        let pull = async {
            let options = CreateImageOptionsBuilder::new().from_image(image).build();
            let mut stream = docker.create_image(Some(options), None, None);
            while let Some(result) = stream.next().await {
                result.map_err(|error| format!("failed to pull `{image}`: {error}"))?;
            }
            Ok::<(), String>(())
        };
        tokio::time::timeout(DOCKER_BROWSER_PULL_TIMEOUT, pull)
            .await
            .map_err(|_| format!("pulling `{image}` timed out"))??;
        let inspect = docker
            .inspect_image(image)
            .await
            .map_err(|error| format!("pulled image `{image}` cannot be inspected: {error}"))?;
        Self::validated_image_id(image, inspect.id)
    }

    #[cfg(feature = "docker-sandbox")]
    fn validated_image_id(image: &str, id: Option<String>) -> Result<String, String> {
        let id = id.ok_or_else(|| format!("Docker image `{image}` has no content-addressed ID"))?;
        let digest = id.strip_prefix("sha256:").ok_or_else(|| {
            format!("Docker image `{image}` returned a malformed content-addressed ID")
        })?;
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!(
                "Docker image `{image}` returned a malformed content-addressed ID"
            ));
        }
        Ok(id)
    }

    #[cfg(feature = "docker-sandbox")]
    fn docker_not_found(error: &BollardError) -> bool {
        matches!(
            error,
            BollardError::DockerResponseServerError {
                status_code: 404,
                ..
            }
        )
    }

    #[cfg(feature = "docker-sandbox")]
    async fn remove_owned_name_collision(
        &self,
        docker: &Docker,
        container_name: &str,
    ) -> Result<(), String> {
        let inspect = match tokio::time::timeout(
            DOCKER_BROWSER_OPERATION_TIMEOUT,
            docker.inspect_container(container_name, None),
        )
        .await
        {
            Err(_) => return Err("timed out inspecting Docker Chromium name".to_string()),
            Ok(Err(error)) if Self::docker_not_found(&error) => return Ok(()),
            Ok(Err(error)) => {
                return Err(format!("failed to inspect Docker Chromium name: {error}"));
            }
            Ok(Ok(inspect)) => inspect,
        };
        let labels = inspect
            .config
            .and_then(|config| config.labels)
            .unwrap_or_default();
        let owned = labels.get(MANAGED_CONTAINER_LABEL).map(String::as_str) == Some("v1")
            && labels.get(CONTAINER_SCOPE_LABEL).map(String::as_str)
                == Some(self.config.runtime_scope.as_str())
            && labels.get(CONTAINER_KIND_LABEL).map(String::as_str) == Some("browser");
        if !owned {
            return Err(format!(
                "Docker container name `{container_name}` is occupied by a resource ThinClaw does not own"
            ));
        }
        tokio::time::timeout(
            DOCKER_BROWSER_OPERATION_TIMEOUT,
            docker.remove_container(
                container_name,
                Some(RemoveContainerOptionsBuilder::new().force(true).build()),
            ),
        )
        .await
        .map_err(|_| "timed out removing stale Docker Chromium container".to_string())?
        .or_else(|error| Self::docker_not_found(&error).then_some(()).ok_or(error))
        .map_err(|error| format!("failed to remove stale Docker Chromium container: {error}"))
    }

    #[cfg(feature = "docker-sandbox")]
    async fn cleanup_state(&self, state: &mut RootBrowserDockerState) -> Vec<String> {
        let mut errors = Vec::new();
        if let (Some(docker), Some(target)) = (state.docker.as_ref(), state.container_target.take())
        {
            match tokio::time::timeout(
                DOCKER_BROWSER_OPERATION_TIMEOUT,
                docker.remove_container(
                    &target,
                    Some(RemoveContainerOptionsBuilder::new().force(true).build()),
                ),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) if Self::docker_not_found(&error) => {}
                Ok(Err(error)) => errors.push(format!("removing browser container: {error}")),
                Err(_) => errors.push("removing browser container timed out".to_string()),
            }
        }
        if let Some(mut relay) = state.relay.take() {
            relay.stop().await;
        }
        if let Some(proxy) = state.proxy.take() {
            proxy.stop().await;
        }
        state.docker = None;
        self.host_port.store(0, Ordering::Release);
        *self
            .proxy_credentials
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        errors
    }

    #[cfg(feature = "docker-sandbox")]
    async fn start_secure(&self) -> Result<(), String> {
        self.config.validate().map_err(|error| error.to_string())?;
        let mut state = self.state.lock().await;
        if state.container_target.is_some()
            && state.proxy.as_ref().is_some_and(|proxy| proxy.is_running())
            && self.host_port.load(Ordering::Acquire) != 0
        {
            return Ok(());
        }
        for error in self.cleanup_state(&mut state).await {
            tracing::warn!(%error, "Cleaning partial Docker browser state before restart");
        }

        let docker = Docker::connect_with_local_defaults()
            .map_err(|error| format!("failed to connect to Docker: {error}"))?;
        tokio::time::timeout(DOCKER_BROWSER_OPERATION_TIMEOUT, docker.ping())
            .await
            .map_err(|_| "Docker ping timed out".to_string())?
            .map_err(|error| format!("Docker is unavailable: {error}"))?;
        state.docker = Some(docker.clone());

        let browser_image_id =
            Self::ensure_image(&docker, &self.config.image, self.config.auto_pull).await?;
        // Mutable worker tags are accepted only for locally built images. Never
        // auto-pull one across a trust boundary; digest-pinned relay images may
        // opt into the configured auto-pull behavior.
        let relay_auto_pull = self.config.auto_pull && self.config.relay_image.contains("@sha256:");
        let relay_image_id =
            Self::ensure_image(&docker, &self.config.relay_image, relay_auto_pull)
                .await
                .map_err(|error| {
                    format!(
                        "{error}. Build the ThinClaw relay image with `docker build -f Dockerfile.worker -t {} .`, or configure SANDBOX_IMAGE to a digest-pinned image containing the `network-relay` subcommand",
                        self.config.relay_image
                    )
                })?;

        let proxy = Arc::new(
            NetworkProxyBuilder::new()
                .with_policy(SandboxPolicy::FullAccess)
                .with_credentials(Vec::new())
                .with_credential_resolver(Arc::new(NoCredentialResolver))
                .build(),
        );
        let proxy_address = proxy
            .start(0)
            .await
            .map_err(|error| format!("failed to start Docker browser proxy: {error}"))?;
        let proxy_token = proxy.proxy_token();
        state.proxy = Some(proxy);
        *self
            .proxy_credentials
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(("thinclaw".to_string(), proxy_token));

        let relay = SandboxNetworkRelay::start(
            docker.clone(),
            &relay_image_id,
            &self.config.runtime_scope,
            "browser-relay",
            &[RelayForward {
                listen_port: RELAY_PROXY_PORT,
                target_port: proxy_address.port(),
            }],
        )
        .await
        .map_err(|error| format!("failed to isolate Docker browser network: {error}"))?;
        let relay_host = relay.gateway_host().to_string();
        let network_name = relay.network_name().to_string();
        state.relay = Some(relay);

        let container_name = self.config.container_name();
        self.remove_owned_name_collision(&docker, &container_name)
            .await?;
        // Record the deterministic name before creation so cancellation cleanup
        // can remove a daemon-side create that finishes after this future drops.
        state.container_target = Some(container_name.clone());

        let cdp_port = format!("{}/tcp", self.config.debug_port);
        let port_bindings = std::collections::HashMap::from([(
            cdp_port.clone(),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                // Empty requests an ephemeral host port from Docker.
                host_port: Some(String::new()),
            }]),
        )]);
        let memory = self
            .config
            .memory_bytes()
            .map_err(|error| error.to_string())?;
        let shm = self.config.shm_bytes().map_err(|error| error.to_string())?;
        let host_config = HostConfig {
            network_mode: Some(network_name),
            port_bindings: Some(port_bindings),
            memory: Some(memory),
            memory_swap: Some(memory),
            memory_swappiness: Some(0),
            nano_cpus: Some(1_000_000_000),
            pids_limit: Some(256),
            init: Some(true),
            auto_remove: Some(false),
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            readonly_rootfs: Some(true),
            tmpfs: Some(std::collections::HashMap::from([(
                "/tmp".to_string(),
                "rw,nosuid,nodev,size=512M,mode=1777".to_string(),
            )])),
            shm_size: Some(shm),
            ..Default::default()
        };
        let body = ContainerCreateBody {
            // Use the content-addressed local ID inspected above so a mutable
            // tag cannot be swapped between validation and container create.
            image: Some(browser_image_id),
            cmd: Some(
                self.config
                    .chrome_args(&relay_host, RELAY_PROXY_PORT)
                    .map_err(|error| error.to_string())?,
            ),
            user: Some("65534:65534".to_string()),
            env: Some(vec!["HOME=/tmp".to_string(), "TMPDIR=/tmp".to_string()]),
            exposed_ports: Some(vec![cdp_port.clone()]),
            labels: Some(managed_container_labels(
                &self.config.runtime_scope,
                "browser",
                None,
            )),
            host_config: Some(host_config),
            ..Default::default()
        };
        let created = tokio::time::timeout(
            DOCKER_BROWSER_OPERATION_TIMEOUT,
            docker.create_container(
                Some(
                    CreateContainerOptionsBuilder::new()
                        .name(&container_name)
                        .build(),
                ),
                body,
            ),
        )
        .await
        .map_err(|_| "creating Docker Chromium timed out".to_string())?
        .map_err(|error| format!("creating Docker Chromium failed: {error}"))?;
        state.container_target = Some(created.id.clone());
        tokio::time::timeout(
            DOCKER_BROWSER_OPERATION_TIMEOUT,
            docker.start_container(&created.id, None),
        )
        .await
        .map_err(|_| "starting Docker Chromium timed out".to_string())?
        .map_err(|error| format!("starting Docker Chromium failed: {error}"))?;

        let inspect = tokio::time::timeout(
            DOCKER_BROWSER_OPERATION_TIMEOUT,
            docker.inspect_container(&created.id, None),
        )
        .await
        .map_err(|_| "inspecting Docker Chromium port timed out".to_string())?
        .map_err(|error| format!("inspecting Docker Chromium port failed: {error}"))?;
        let bindings = inspect
            .network_settings
            .and_then(|settings| settings.ports)
            .and_then(|ports| ports.get(&cdp_port).cloned().flatten())
            .ok_or_else(|| "Docker did not publish the Chromium CDP port".to_string())?;
        let binding = bindings
            .into_iter()
            .find(|binding| binding.host_ip.as_deref() == Some("127.0.0.1"))
            .ok_or_else(|| {
                "Docker Chromium CDP was not restricted to the loopback interface".to_string()
            })?;
        let host_port = binding
            .host_port
            .as_deref()
            .and_then(|port| port.parse::<u16>().ok())
            .filter(|port| *port != 0)
            .ok_or_else(|| "Docker returned an invalid Chromium host port".to_string())?;
        self.host_port.store(host_port, Ordering::Release);
        tracing::info!(
            container = %created.id,
            host_port,
            "Started isolated Docker Chromium"
        );
        Ok(())
    }

    #[cfg(feature = "docker-sandbox")]
    async fn stop_secure(&self) -> Result<(), String> {
        let mut state = self.state.lock().await;
        let errors = self.cleanup_state(&mut state).await;
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

#[cfg(feature = "docker-sandbox")]
struct BrowserStartCleanup {
    runtime: RootBrowserDockerRuntime,
    armed: bool,
}

#[cfg(feature = "docker-sandbox")]
impl Drop for BrowserStartCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Docker browser start cleanup dropped outside a Tokio runtime");
            return;
        };
        let runtime = self.runtime.clone();
        handle.spawn(async move {
            if let Err(error) = runtime.stop_secure().await {
                tracing::error!(%error, "Failed to clean cancelled Docker browser start");
            }
        });
    }
}

#[async_trait]
impl BrowserDockerRuntime for RootBrowserDockerRuntime {
    fn image_label(&self) -> String {
        self.config.image.clone()
    }

    fn http_endpoint(&self) -> String {
        DockerChromiumConfig::http_endpoint_for_host_port(self.host_port.load(Ordering::Acquire))
    }

    fn is_available(&self) -> bool {
        #[cfg(feature = "docker-sandbox")]
        {
            self.config.validate().is_ok() && DockerChromiumConfig::is_docker_available()
        }
        #[cfg(not(feature = "docker-sandbox"))]
        false
    }

    fn proxy_credentials(&self) -> Option<(String, String)> {
        self.proxy_credentials
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    async fn start(&self) -> Result<(), String> {
        #[cfg(feature = "docker-sandbox")]
        {
            let mut cleanup = BrowserStartCleanup {
                runtime: self.clone(),
                armed: true,
            };
            self.start_secure().await?;
            cleanup.armed = false;
            return Ok(());
        }
        #[cfg(not(feature = "docker-sandbox"))]
        Err("Docker Chromium requires ThinClaw's `docker-sandbox` feature so its network can be isolated".to_string())
    }

    async fn wait_for_ready(&self, timeout: Duration) -> Result<(), String> {
        let port = self.host_port.load(Ordering::Acquire);
        if port == 0 {
            return Err("Docker Chromium has no assigned loopback CDP port".to_string());
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("Docker Chromium was not ready after {timeout:?}"));
            }
            match tokio::time::timeout(
                Duration::from_secs(1),
                tokio::net::TcpStream::connect(("127.0.0.1", port)),
            )
            .await
            {
                Ok(Ok(_)) => return Ok(()),
                Ok(Err(_)) | Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
    }

    async fn stop(&self) -> Result<(), String> {
        #[cfg(feature = "docker-sandbox")]
        {
            return self.stop_secure().await;
        }
        #[cfg(not(feature = "docker-sandbox"))]
        {
            self.host_port.store(0, Ordering::Release);
            *self
                .proxy_credentials
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            Ok(())
        }
    }
}

/// Browser automation tool preserving the historical root constructor API.
pub struct BrowserTool {
    inner: runtime_browser::BrowserTool,
}

impl BrowserTool {
    pub fn new(profile_dir: PathBuf) -> Self {
        Self {
            inner: runtime_browser::BrowserTool::new_with_egress(
                profile_dir,
                Arc::new(RootBrowserEgressRuntime::default()),
            ),
        }
    }

    pub fn new_with_docker(profile_dir: PathBuf, docker_config: DockerChromiumConfig) -> Self {
        Self {
            inner: runtime_browser::BrowserTool::new_with_docker_and_egress(
                profile_dir,
                Arc::new(RootBrowserDockerRuntime::new(docker_config)),
                Arc::new(RootBrowserEgressRuntime::default()),
            ),
        }
    }

    pub fn new_with_cloud(profile_dir: PathBuf, cloud_provider: Option<String>) -> Self {
        Self {
            inner: runtime_browser::BrowserTool::new_with_cloud(profile_dir, cloud_provider),
        }
    }

    pub fn from_runtime(inner: runtime_browser::BrowserTool) -> Self {
        Self { inner }
    }

    pub fn into_runtime(self) -> runtime_browser::BrowserTool {
        self.inner
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.inner.parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        self.inner.execute(params, ctx).await
    }

    async fn shutdown(&self) -> Result<(), ToolError> {
        self.inner.shutdown().await;
        Ok(())
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        self.inner.requires_approval(params)
    }

    fn requires_sanitization(&self) -> bool {
        self.inner.requires_sanitization()
    }

    fn execution_timeout(&self) -> Duration {
        self.inner.execution_timeout()
    }
}

impl From<DockerError> for ToolError {
    fn from(error: DockerError) -> Self {
        ToolError::ExecutionFailed(error.to_string())
    }
}
