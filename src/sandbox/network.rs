//! Docker network isolation shared by ephemeral and persistent sandboxes.
//!
//! Proxy environment variables are advisory: a process can simply ignore
//! them and open a socket directly. Sandboxed containers therefore join an
//! unique internal Docker bridge whose gateway uses Docker's `isolated` mode.
//! A capability-free dual-homed relay container exposes only the authenticated
//! ThinClaw proxy and orchestrator ports; arbitrary host services and other
//! jobs are not reachable from the bridge.

use std::collections::HashMap;
use std::time::Duration;

use bollard::Docker;
use bollard::errors::Error as DockerError;
use bollard::models::{NetworkCreateRequest, NetworkInspect};

use crate::sandbox::error::{Result, SandboxError};

const SANDBOX_NETWORK_PREFIX: &str = "thinclaw-sandbox-v2";
pub const MANAGED_CONTAINER_LABEL: &str = "com.thinclaw.sandbox.managed";
pub const CONTAINER_SCOPE_LABEL: &str = "com.thinclaw.sandbox.scope";
pub const CONTAINER_KIND_LABEL: &str = "com.thinclaw.sandbox.kind";
pub const CONTAINER_JOB_LABEL: &str = "com.thinclaw.sandbox.job-id";
const NETWORK_LABEL: &str = "com.thinclaw.sandbox-network";
const NETWORK_LABEL_VALUE: &str = "v2-isolated-gateway";
const GATEWAY_MODE_OPTION: &str = "com.docker.network.bridge.gateway_mode_ipv4";
const GATEWAY_MODE_ISOLATED: &str = "isolated";
const DOCKER_NETWORK_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxNetwork {
    pub name: String,
}

pub fn managed_container_labels(
    runtime_scope: &str,
    kind: &str,
    job_id: Option<uuid::Uuid>,
) -> HashMap<String, String> {
    let mut labels = HashMap::from([
        (MANAGED_CONTAINER_LABEL.to_string(), "v1".to_string()),
        (CONTAINER_SCOPE_LABEL.to_string(), runtime_scope.to_string()),
        (CONTAINER_KIND_LABEL.to_string(), kind.to_string()),
    ]);
    if let Some(job_id) = job_id {
        labels.insert(CONTAINER_JOB_LABEL.to_string(), job_id.to_string());
    }
    labels
}

fn network_error(reason: impl Into<String>) -> SandboxError {
    SandboxError::ContainerCreationFailed {
        reason: format!("sandbox network isolation unavailable: {}", reason.into()),
    }
}

fn is_not_found(error: &DockerError) -> bool {
    matches!(
        error,
        DockerError::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

fn validate_network(
    network: &NetworkInspect,
    expected_name: &str,
    runtime_scope: &str,
) -> Result<SandboxNetwork> {
    if network.name.as_deref() != Some(expected_name) {
        return Err(network_error("Docker returned the wrong network"));
    }
    if network.internal != Some(true) {
        return Err(network_error(format!(
            "network {expected_name:?} is not internal"
        )));
    }
    if network.driver.as_deref() != Some("bridge") {
        return Err(network_error(format!(
            "network {expected_name:?} is not a bridge"
        )));
    }
    if network
        .labels
        .as_ref()
        .and_then(|labels| labels.get(NETWORK_LABEL))
        .map(String::as_str)
        != Some(NETWORK_LABEL_VALUE)
    {
        return Err(network_error(format!(
            "network {expected_name:?} is not managed by ThinClaw"
        )));
    }
    if network
        .labels
        .as_ref()
        .and_then(|labels| labels.get(CONTAINER_SCOPE_LABEL))
        .map(String::as_str)
        != Some(runtime_scope)
    {
        return Err(network_error(format!(
            "network {expected_name:?} belongs to a different runtime"
        )));
    }
    if network
        .options
        .as_ref()
        .and_then(|options| options.get(GATEWAY_MODE_OPTION))
        .map(String::as_str)
        != Some(GATEWAY_MODE_ISOLATED)
    {
        return Err(network_error(format!(
            "network {expected_name:?} does not isolate its host gateway"
        )));
    }

    Ok(SandboxNetwork {
        name: expected_name.to_string(),
    })
}

async fn inspect_network(
    docker: &Docker,
    name: &str,
    runtime_scope: &str,
) -> Result<Option<SandboxNetwork>> {
    match tokio::time::timeout(DOCKER_NETWORK_TIMEOUT, docker.inspect_network(name, None)).await {
        Ok(Ok(network)) => validate_network(&network, name, runtime_scope).map(Some),
        Ok(Err(error)) if is_not_found(&error) => Ok(None),
        Ok(Err(error)) => Err(network_error(format!(
            "failed to inspect {name:?}: {error}"
        ))),
        Err(_) => Err(network_error(format!("timed out inspecting {name:?}"))),
    }
}

/// Create a unique, host-gateway-isolated bridge for one untrusted execution.
/// Per-execution networks prevent jobs from reaching or denying service to
/// other jobs on the same Docker daemon.
pub async fn create_sandbox_network(
    docker: &Docker,
    runtime_scope: &str,
) -> Result<SandboxNetwork> {
    if runtime_scope.is_empty()
        || runtime_scope.len() > 128
        || !runtime_scope
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(network_error("runtime scope is invalid"));
    }

    let name = format!("{SANDBOX_NETWORK_PREFIX}-{}", uuid::Uuid::new_v4().simple());
    let labels = HashMap::from([
        (NETWORK_LABEL.to_string(), NETWORK_LABEL_VALUE.to_string()),
        (CONTAINER_SCOPE_LABEL.to_string(), runtime_scope.to_string()),
    ]);
    let options = HashMap::from([
        (
            "com.docker.network.bridge.enable_ip_masquerade".to_string(),
            "false".to_string(),
        ),
        (
            GATEWAY_MODE_OPTION.to_string(),
            GATEWAY_MODE_ISOLATED.to_string(),
        ),
    ]);
    let request = NetworkCreateRequest {
        name: name.clone(),
        driver: Some("bridge".to_string()),
        internal: Some(true),
        attachable: Some(false),
        enable_ipv4: Some(true),
        enable_ipv6: Some(false),
        options: Some(options),
        labels: Some(labels),
        ..Default::default()
    };

    match tokio::time::timeout(DOCKER_NETWORK_TIMEOUT, docker.create_network(request)).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            return Err(network_error(format!("failed to create {name:?}: {error}")));
        }
        Err(_) => {
            return Err(network_error(format!("timed out creating {name:?}")));
        }
    }

    inspect_network(docker, &name, runtime_scope)
        .await?
        .ok_or_else(|| network_error(format!("{name:?} disappeared immediately after creation")))
}

pub async fn remove_sandbox_network(docker: &Docker, name: &str) -> Result<()> {
    match tokio::time::timeout(DOCKER_NETWORK_TIMEOUT, docker.remove_network(name)).await {
        Ok(Ok(()))
        | Ok(Err(DockerError::DockerResponseServerError {
            status_code: 404, ..
        })) => Ok(()),
        Ok(Err(error)) => Err(network_error(format!(
            "failed to remove network {name:?}: {error}"
        ))),
        Err(_) => Err(network_error(format!(
            "timed out removing network {name:?}"
        ))),
    }
}

/// Remove crash-left networks owned by this runtime. Call only during startup,
/// before new sandboxes are admitted.
pub async fn cleanup_sandbox_networks(docker: &Docker, runtime_scope: &str) -> Result<usize> {
    use bollard::query_parameters::ListNetworksOptionsBuilder;

    let filters = HashMap::from([(
        "label".to_string(),
        vec![
            format!("{NETWORK_LABEL}={NETWORK_LABEL_VALUE}"),
            format!("{CONTAINER_SCOPE_LABEL}={runtime_scope}"),
        ],
    )]);
    let networks = tokio::time::timeout(
        DOCKER_NETWORK_TIMEOUT,
        docker.list_networks(Some(
            ListNetworksOptionsBuilder::new().filters(&filters).build(),
        )),
    )
    .await
    .map_err(|_| network_error("timed out listing stale sandbox networks"))?
    .map_err(|error| network_error(format!("failed to list stale sandbox networks: {error}")))?;
    let mut removed = 0usize;
    for network in networks {
        let Some(name) = network.name else {
            continue;
        };
        if remove_sandbox_network(docker, &name).await.is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Startup-only cleanup for containers and per-execution networks left by a
/// crashed process holding the same durable runtime scope.
pub async fn cleanup_stale_sandbox_resources(runtime_scope: &str) -> Result<(usize, usize)> {
    use bollard::query_parameters::{ListContainersOptionsBuilder, RemoveContainerOptionsBuilder};

    if runtime_scope.is_empty()
        || runtime_scope.len() > 128
        || !runtime_scope
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(network_error("runtime scope is invalid"));
    }
    let docker = super::container::connect_docker().await?;
    let filters = HashMap::from([(
        "label".to_string(),
        vec![
            format!("{MANAGED_CONTAINER_LABEL}=v1"),
            format!("{CONTAINER_SCOPE_LABEL}={runtime_scope}"),
        ],
    )]);
    let containers = tokio::time::timeout(
        DOCKER_NETWORK_TIMEOUT,
        docker.list_containers(Some(
            ListContainersOptionsBuilder::new()
                .all(true)
                .filters(&filters)
                .build(),
        )),
    )
    .await
    .map_err(|_| network_error("timed out listing stale sandbox containers"))?
    .map_err(|error| network_error(format!("failed to list stale containers: {error}")))?;
    let mut removed_containers = 0usize;
    for container in containers {
        let Some(id) = container.id else {
            continue;
        };
        match tokio::time::timeout(
            DOCKER_NETWORK_TIMEOUT,
            docker.remove_container(
                &id,
                Some(RemoveContainerOptionsBuilder::new().force(true).build()),
            ),
        )
        .await
        {
            Ok(Ok(()))
            | Ok(Err(DockerError::DockerResponseServerError {
                status_code: 404, ..
            })) => removed_containers += 1,
            Ok(Err(error)) => tracing::warn!(
                container = %id,
                %error,
                "Failed to remove stale sandbox container"
            ),
            Err(_) => tracing::warn!(
                container = %id,
                "Timed out removing stale sandbox container"
            ),
        }
    }
    let removed_networks = cleanup_sandbox_networks(&docker, runtime_scope).await?;
    Ok((removed_containers, removed_networks))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_NAME: &str = "thinclaw-sandbox-v2-test";
    const TEST_SCOPE: &str = "test_scope";

    fn valid_network() -> NetworkInspect {
        NetworkInspect {
            name: Some(TEST_NAME.to_string()),
            driver: Some("bridge".to_string()),
            internal: Some(true),
            labels: Some(HashMap::from([
                (NETWORK_LABEL.to_string(), NETWORK_LABEL_VALUE.to_string()),
                (CONTAINER_SCOPE_LABEL.to_string(), TEST_SCOPE.to_string()),
            ])),
            options: Some(HashMap::from([(
                GATEWAY_MODE_OPTION.to_string(),
                GATEWAY_MODE_ISOLATED.to_string(),
            )])),
            ..Default::default()
        }
    }

    #[test]
    fn validates_internal_managed_bridge_and_gateway() {
        let network =
            validate_network(&valid_network(), TEST_NAME, TEST_SCOPE).expect("valid network");
        assert_eq!(network.name, TEST_NAME);
    }

    #[test]
    fn rejects_same_named_external_network() {
        let mut network = valid_network();
        network.internal = Some(false);
        let error = validate_network(&network, TEST_NAME, TEST_SCOPE)
            .expect_err("external network must fail closed");
        assert!(error.to_string().contains("is not internal"));
    }

    #[test]
    fn rejects_unmanaged_network_name_collision() {
        let mut network = valid_network();
        network.labels = None;
        let error = validate_network(&network, TEST_NAME, TEST_SCOPE)
            .expect_err("unmanaged collision must fail closed");
        assert!(error.to_string().contains("not managed by ThinClaw"));
    }

    #[test]
    fn rejects_internal_bridge_with_host_gateway_access() {
        let mut network = valid_network();
        network.options = None;
        let error = validate_network(&network, TEST_NAME, TEST_SCOPE)
            .expect_err("host gateway must be isolated");
        assert!(
            error
                .to_string()
                .contains("does not isolate its host gateway")
        );
    }
}
