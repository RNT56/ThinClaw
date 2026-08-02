//! Shared mutation ownership, application, and revision contract.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationExecutionPolicy {
    EmbeddedRuntime,
    DurableImmediate,
    ActiveCoordinated,
    RuntimeRequired,
    StoppedExclusive,
    OwnedProcessLifecycle,
    ExternalDirect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationApplication {
    DurableApplied,
    AppliedLive,
    RestartRequired,
    RuntimeNotRunning,
    ProcessLifecycleApplied,
    ExternalEffectApplied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationRequest<T> {
    pub request_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_durable_revision: Option<u64>,
    pub payload: T,
}

impl<T> MutationRequest<T> {
    pub fn new(payload: T) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            expected_durable_revision: None,
            payload,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationReceipt {
    pub request_id: Uuid,
    pub policy: MutationExecutionPolicy,
    pub domain: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub durable_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_instance_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_operation_id: Option<String>,
    pub application: MutationApplication,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restart_reasons: Vec<String>,
    #[serde(default)]
    pub partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
}

impl MutationReceipt {
    pub fn applied_live(
        request_id: Uuid,
        domain: impl Into<String>,
        resource_id: impl Into<String>,
        runtime_revision: u64,
    ) -> Self {
        Self {
            request_id,
            policy: MutationExecutionPolicy::RuntimeRequired,
            domain: domain.into(),
            resource_id: Some(resource_id.into()),
            durable_revision: None,
            runtime_instance_id: None,
            runtime_revision: Some(runtime_revision),
            external_operation_id: None,
            application: MutationApplication::AppliedLive,
            restart_reasons: Vec::new(),
            partial: false,
            recovery: None,
        }
    }

    pub fn revision_coherent(&self) -> bool {
        match self.application {
            MutationApplication::AppliedLive => {
                self.runtime_revision.is_some() && !self.partial && self.restart_reasons.is_empty()
            }
            MutationApplication::RestartRequired => {
                !self.restart_reasons.is_empty()
                    && self.application != MutationApplication::AppliedLive
            }
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applied_live_requires_an_exact_runtime_revision() {
        let receipt = MutationReceipt::applied_live(Uuid::new_v4(), "tools", "example", 9);
        assert!(receipt.revision_coherent());
        let mut broken = receipt;
        broken.runtime_revision = None;
        assert!(!broken.revision_coherent());
    }
}
