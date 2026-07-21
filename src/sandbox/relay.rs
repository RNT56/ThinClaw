//! Narrow network relay between the host and the isolated sandbox bridge.
//!
//! Docker's `internal` bridge still permits access to arbitrary host services
//! through its gateway. The v2 sandbox network removes that gateway address.
//! A small, capability-free container is therefore attached to both the
//! isolated bridge and Docker's ordinary bridge and forwards only the two
//! explicitly configured authenticated ThinClaw endpoints.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bollard::Docker;
use bollard::models::{
    ContainerCreateBody, EndpointSettings, HealthConfig, HealthStatusEnum, HostConfig,
    NetworkConnectRequest,
};
use bollard::query_parameters::{CreateContainerOptionsBuilder, RemoveContainerOptionsBuilder};
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::sandbox::error::{Result, SandboxError};
use crate::sandbox::network::{
    create_sandbox_network, managed_container_labels, remove_sandbox_network,
};

pub const RELAY_PROXY_PORT: u16 = 18_080;
pub const RELAY_ORCHESTRATOR_PORT: u16 = 18_081;
const HOST_TARGET_ALIAS: &str = "host.docker.internal";
const RELAY_READY_FILE: &str = "/tmp/.thinclaw-relay-ready";
const RELAY_START_TIMEOUT: Duration = Duration::from_secs(20);
const RELAY_DOCKER_TIMEOUT: Duration = Duration::from_secs(15);
const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RELAY_CONNECTION_TIMEOUT: Duration = Duration::from_secs(35 * 60);
const MAX_RELAY_CONNECTIONS_PER_PORT: usize = 256;
const MAX_RELAY_FORWARDS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayForward {
    pub listen_port: u16,
    pub target_port: u16,
}

impl RelayForward {
    fn validate(self) -> std::result::Result<Self, String> {
        if !matches!(self.listen_port, RELAY_PROXY_PORT | RELAY_ORCHESTRATOR_PORT)
            || self.target_port == 0
        {
            return Err("relay ports are outside the supported range".to_string());
        }
        Ok(self)
    }

    fn cli_spec(self) -> String {
        format!(
            "{}={HOST_TARGET_ALIAS}:{}",
            self.listen_port, self.target_port
        )
    }

    fn parse_cli_spec(value: &str) -> std::result::Result<Self, String> {
        if value.len() > 128 || value.chars().any(char::is_control) {
            return Err("relay forward specification is invalid".to_string());
        }
        let (listen_port, target) = value
            .split_once('=')
            .ok_or_else(|| "relay forward must use LISTEN=HOST:PORT".to_string())?;
        let (target_host, target_port) = target
            .rsplit_once(':')
            .ok_or_else(|| "relay forward must use LISTEN=HOST:PORT".to_string())?;
        if target_host != HOST_TARGET_ALIAS {
            return Err("relay target host is not permitted".to_string());
        }
        Self {
            listen_port: listen_port
                .parse()
                .map_err(|_| "relay listen port is invalid".to_string())?,
            target_port: target_port
                .parse()
                .map_err(|_| "relay target port is invalid".to_string())?,
        }
        .validate()
    }
}

/// Owned relay-container lifecycle. Dropping an armed relay schedules bounded
/// force-removal, covering cancellation during create/connect/start/readiness.
pub(crate) struct SandboxNetworkRelay {
    docker: Docker,
    target: String,
    gateway_host: String,
    network_name: String,
    armed: bool,
}

impl SandboxNetworkRelay {
    pub(crate) async fn start(
        docker: Docker,
        image: &str,
        runtime_scope: &str,
        kind: &str,
        forwards: &[RelayForward],
    ) -> Result<Self> {
        validate_relay_inputs(image, runtime_scope, kind, forwards)?;
        let network = create_sandbox_network(&docker, runtime_scope).await?;

        let relay_id = uuid::Uuid::new_v4().simple().to_string();
        let container_name = format!("thinclaw-relay-{relay_id}");
        let gateway_host = format!("relay-{relay_id}");
        let mut relay = Self {
            docker: docker.clone(),
            target: container_name.clone(),
            gateway_host: gateway_host.clone(),
            network_name: network.name.clone(),
            armed: true,
        };

        let mut command = vec!["network-relay".to_string()];
        for forward in forwards {
            command.push("--forward".to_string());
            command.push(forward.cli_spec());
        }

        let host_config = HostConfig {
            network_mode: Some("bridge".to_string()),
            extra_hosts: Some(vec![format!("{HOST_TARGET_ALIAS}:host-gateway")]),
            memory: Some(64 * 1024 * 1024),
            memory_swap: Some(64 * 1024 * 1024),
            memory_swappiness: Some(0),
            nano_cpus: Some(100_000_000),
            pids_limit: Some(64),
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            readonly_rootfs: Some(true),
            tmpfs: Some(HashMap::from([(
                "/tmp".to_string(),
                "rw,nosuid,nodev,noexec,size=16M,mode=1777".to_string(),
            )])),
            auto_remove: Some(false),
            ..Default::default()
        };
        let config = ContainerCreateBody {
            image: Some(image.to_string()),
            cmd: Some(command),
            user: Some("65534:65534".to_string()),
            host_config: Some(host_config),
            healthcheck: Some(HealthConfig {
                test: Some(vec![
                    "CMD-SHELL".to_string(),
                    format!("test -f {RELAY_READY_FILE}"),
                ]),
                interval: Some(250_000_000),
                timeout: Some(1_000_000_000),
                retries: Some(20),
                start_period: Some(1_000_000_000),
                start_interval: Some(250_000_000),
            }),
            labels: Some(managed_container_labels(runtime_scope, kind, None)),
            ..Default::default()
        };
        let response = tokio::time::timeout(
            RELAY_DOCKER_TIMEOUT,
            docker.create_container(
                Some(
                    CreateContainerOptionsBuilder::new()
                        .name(&container_name)
                        .build(),
                ),
                config,
            ),
        )
        .await
        .map_err(|_| relay_error("timed out creating the network relay"))?
        .map_err(|error| relay_error(format!("failed to create network relay: {error}")))?;
        relay.target = response.id.clone();

        tokio::time::timeout(
            RELAY_DOCKER_TIMEOUT,
            docker.connect_network(
                &network.name,
                NetworkConnectRequest {
                    container: response.id.clone(),
                    endpoint_config: Some(EndpointSettings {
                        aliases: Some(vec![gateway_host]),
                        ..Default::default()
                    }),
                },
            ),
        )
        .await
        .map_err(|_| relay_error("timed out attaching the network relay"))?
        .map_err(|error| relay_error(format!("failed to attach network relay: {error}")))?;

        tokio::time::timeout(
            RELAY_DOCKER_TIMEOUT,
            docker.start_container(&response.id, None),
        )
        .await
        .map_err(|_| relay_error("timed out starting the network relay"))?
        .map_err(|error| relay_error(format!("failed to start network relay: {error}")))?;

        let readiness = async {
            loop {
                let inspect =
                    docker
                        .inspect_container(&response.id, None)
                        .await
                        .map_err(|error| {
                            relay_error(format!("failed to inspect network relay: {error}"))
                        })?;
                let state = inspect
                    .state
                    .ok_or_else(|| relay_error("network relay has no Docker state"))?;
                if !state.running.unwrap_or(false) {
                    return Err(relay_error(format!(
                        "network relay exited before readiness (code {:?}, error {:?})",
                        state.exit_code, state.error
                    )));
                }
                match state.health.and_then(|health| health.status) {
                    Some(HealthStatusEnum::HEALTHY) => return Ok(()),
                    Some(HealthStatusEnum::UNHEALTHY) => {
                        return Err(relay_error("network relay failed its readiness check"));
                    }
                    _ => tokio::time::sleep(Duration::from_millis(100)).await,
                }
            }
        };
        tokio::time::timeout(RELAY_START_TIMEOUT, readiness)
            .await
            .map_err(|_| relay_error("network relay readiness timed out"))??;

        Ok(relay)
    }

    pub(crate) fn gateway_host(&self) -> &str {
        &self.gateway_host
    }

    pub(crate) fn network_name(&self) -> &str {
        &self.network_name
    }

    pub(crate) fn container_id(&self) -> &str {
        &self.target
    }

    pub(crate) async fn stop(&mut self) {
        let container_removed = match tokio::time::timeout(
            RELAY_DOCKER_TIMEOUT,
            self.docker.remove_container(
                &self.target,
                Some(RemoveContainerOptionsBuilder::new().force(true).build()),
            ),
        )
        .await
        {
            Ok(Ok(()))
            | Ok(Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            })) => true,
            Ok(Err(error)) => {
                tracing::warn!(
                    container = %self.target,
                    %error,
                    "Failed to remove sandbox network relay"
                );
                false
            }
            Err(_) => {
                tracing::warn!(
                    container = %self.target,
                    "Timed out removing sandbox network relay"
                );
                false
            }
        };
        if container_removed {
            match remove_sandbox_network(&self.docker, &self.network_name).await {
                Ok(()) => self.armed = false,
                Err(error) => tracing::warn!(
                    network = %self.network_name,
                    %error,
                    "Failed to remove sandbox execution network"
                ),
            }
        }
    }
}

impl Drop for SandboxNetworkRelay {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::error!(
                container = %self.target,
                "Sandbox network relay dropped outside a Tokio runtime"
            );
            return;
        };
        let docker = self.docker.clone();
        let target = self.target.clone();
        let network_name = self.network_name.clone();
        runtime.spawn(async move {
            let mut container_removed = false;
            for attempt in 0..10 {
                match docker
                    .remove_container(
                        &target,
                        Some(RemoveContainerOptionsBuilder::new().force(true).build()),
                    )
                    .await
                {
                    Ok(())
                    | Err(bollard::errors::Error::DockerResponseServerError {
                        status_code: 404,
                        ..
                    }) => {
                        container_removed = true;
                        break;
                    }
                    Err(error) if attempt < 9 => {
                        tracing::debug!(%target, %error, "Retrying network relay cleanup");
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    Err(error) => {
                        tracing::error!(%target, %error, "Exhausted network relay cleanup retries");
                    }
                }
            }
            if container_removed
                && let Err(error) = remove_sandbox_network(&docker, &network_name).await
            {
                tracing::error!(%network_name, %error, "Failed to clean dropped sandbox network");
            }
        });
    }
}

fn relay_error(reason: impl Into<String>) -> SandboxError {
    SandboxError::ContainerCreationFailed {
        reason: reason.into(),
    }
}

fn validate_relay_inputs(
    image: &str,
    runtime_scope: &str,
    kind: &str,
    forwards: &[RelayForward],
) -> Result<()> {
    if image.trim().is_empty()
        || image.len() > 512
        || image.chars().any(char::is_control)
        || runtime_scope.is_empty()
        || runtime_scope.len() > 128
        || !runtime_scope
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        || kind.is_empty()
        || kind.len() > 64
        || !kind
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        || forwards.is_empty()
        || forwards.len() > MAX_RELAY_FORWARDS
    {
        return Err(relay_error(
            "sandbox network relay configuration is invalid",
        ));
    }
    let mut ports = HashSet::new();
    for forward in forwards {
        forward.validate().map_err(relay_error)?;
        if !ports.insert(forward.listen_port) {
            return Err(relay_error("sandbox network relay port is duplicated"));
        }
    }
    Ok(())
}

/// Container-side relay entrypoint. The host only passes target ports for its
/// own authenticated services; target host and listener shape are validated
/// again in the untrusted container process.
pub async fn run_network_relay(forward_specs: &[String]) -> std::result::Result<(), String> {
    if forward_specs.is_empty() || forward_specs.len() > MAX_RELAY_FORWARDS {
        return Err("network relay requires a bounded forward list".to_string());
    }
    let mut forwards = Vec::with_capacity(forward_specs.len());
    let mut ports = HashSet::new();
    for spec in forward_specs {
        let forward = RelayForward::parse_cli_spec(spec)?;
        if !ports.insert(forward.listen_port) {
            return Err("network relay listen port is duplicated".to_string());
        }
        forwards.push(forward);
    }

    let mut listeners = Vec::with_capacity(forwards.len());
    for forward in forwards {
        let listener = TcpListener::bind(("0.0.0.0", forward.listen_port))
            .await
            .map_err(|error| {
                format!("failed to bind relay port {}: {error}", forward.listen_port)
            })?;
        listeners.push((listener, forward));
    }
    thinclaw_platform::write_private_file_atomic_async(
        std::path::PathBuf::from(RELAY_READY_FILE),
        b"ready\n".to_vec(),
        true,
    )
    .await
    .map_err(|error| format!("failed to publish relay readiness: {error}"))?;

    let mut servers = JoinSet::new();
    for (listener, forward) in listeners {
        servers.spawn(serve_forward(listener, forward));
    }
    match servers.join_next().await {
        Some(Ok(Ok(()))) => Err("network relay listener exited unexpectedly".to_string()),
        Some(Ok(Err(error))) => Err(error),
        Some(Err(error)) => Err(format!("network relay listener task failed: {error}")),
        None => Err("network relay started no listeners".to_string()),
    }
}

async fn serve_forward(
    listener: TcpListener,
    forward: RelayForward,
) -> std::result::Result<(), String> {
    let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_RELAY_CONNECTIONS_PER_PORT));
    let mut connections = JoinSet::new();
    loop {
        while let Some(result) = connections.try_join_next() {
            if let Err(error) = result
                && !error.is_cancelled()
            {
                tracing::debug!(%error, "Network relay connection task failed");
            }
        }
        let (mut client, _) = listener
            .accept()
            .await
            .map_err(|error| format!("network relay accept failed: {error}"))?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            let _ = client.shutdown().await;
            continue;
        };
        connections.spawn(async move {
            let _permit = permit;
            let server = tokio::time::timeout(
                RELAY_CONNECT_TIMEOUT,
                TcpStream::connect((HOST_TARGET_ALIAS, forward.target_port)),
            )
            .await;
            let Ok(Ok(mut server)) = server else {
                let _ = client.shutdown().await;
                return;
            };
            let _ = tokio::time::timeout(
                RELAY_CONNECTION_TIMEOUT,
                tokio::io::copy_bidirectional(&mut client, &mut server),
            )
            .await;
            let _ = client.shutdown().await;
            let _ = server.shutdown().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_forward_specs_are_strict_and_round_trip() {
        let forward = RelayForward {
            listen_port: RELAY_PROXY_PORT,
            target_port: 49_321,
        };
        assert_eq!(
            RelayForward::parse_cli_spec(&forward.cli_spec()).unwrap(),
            forward
        );
        for invalid in [
            "",
            "8080=host.docker.internal:80",
            "18080=evil.internal:80",
            "18080=host.docker.internal:0",
            "18080=host.docker.internal:not-a-port",
        ] {
            assert!(RelayForward::parse_cli_spec(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn relay_configuration_rejects_duplicate_listener_ports() {
        let forwards = [
            RelayForward {
                listen_port: RELAY_PROXY_PORT,
                target_port: 40_001,
            },
            RelayForward {
                listen_port: RELAY_PROXY_PORT,
                target_port: 40_002,
            },
        ];
        assert!(validate_relay_inputs("image:latest", "scope_1", "relay", &forwards).is_err());
    }
}
