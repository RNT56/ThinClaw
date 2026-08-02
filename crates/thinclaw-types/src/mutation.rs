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
#[serde(tag = "effect", content = "policy", rename_all = "snake_case")]
pub enum CliLeafEffect {
    ReadOnly,
    Mutating(MutationExecutionPolicy),
}

/// Classify one generated, canonical Clap leaf under the shared mutation
/// ownership vocabulary. The final verb lists are deliberately closed: adding
/// a command with a new verb makes surface generation and coverage fail until
/// its ownership contract is reviewed.
pub fn canonical_cli_leaf_effect(path: &str) -> Result<CliLeafEffect, String> {
    let verb = path
        .split_whitespace()
        .last()
        .ok_or_else(|| "canonical CLI path is empty".to_string())?;
    const READ_ONLY: &[&str] = &[
        "access",
        "audit",
        "blocked",
        "check",
        "check-config",
        "check-deps",
        "completion",
        "doctor",
        "events",
        "get",
        "hardware-check",
        "health",
        "history",
        "info",
        "inspect",
        "levels",
        "links",
        "lint",
        "list",
        "list-workflows",
        "path",
        "probe",
        "read",
        "repos",
        "runs",
        "search",
        "show",
        "stats",
        "status",
        "summary",
        "tail",
        "templates",
        "test",
        "tools",
        "tree",
        "validate",
        "verify",
    ];
    if READ_ONLY.contains(&verb) {
        return Ok(CliLeafEffect::ReadOnly);
    }
    const MUTATING: &[&str] = &[
        "activate",
        "add",
        "approve",
        "ask",
        "auth",
        "block",
        "cancel",
        "connect",
        "create",
        "delete",
        "disable",
        "edit",
        "enable",
        "enroll",
        "evaluate-now",
        "export",
        "generate",
        "grant",
        "import",
        "init",
        "install",
        "install-defaults",
        "launch",
        "launch-test",
        "link",
        "open",
        "pair",
        "pause",
        "promote",
        "prompt",
        "prune",
        "publish",
        "record",
        "refresh",
        "reissue-lease",
        "reload",
        "remove",
        "rename",
        "reset",
        "restart",
        "resume",
        "review",
        "revoke",
        "rollback",
        "rotate-master",
        "run",
        "screenshot",
        "send",
        "set",
        "set-credential",
        "set-default",
        "set-preferred-channel",
        "setup",
        "snapshot",
        "start",
        "stop",
        "submit",
        "sync",
        "toggle",
        "trigger",
        "trust",
        "tui",
        "unblock",
        "uninstall",
        "unlink",
        "update",
        "write",
    ];
    if !MUTATING.contains(&verb) {
        return Err(format!(
            "canonical CLI leaf '{path}' uses unreviewed final verb '{verb}'"
        ));
    }

    let policy = if matches!(path, "run" | "tui" | "ask")
        || path.starts_with("runtime service ")
        || matches!(
            path,
            "runtime web start" | "runtime web stop" | "runtime web reload"
        )
        || matches!(path, "media comfy launch" | "media comfy stop")
        || matches!(path, "runtime update install" | "runtime update rollback")
    {
        MutationExecutionPolicy::OwnedProcessLifecycle
    } else if path.starts_with("setup")
        || matches!(path, "data backup import" | "config secrets rotate-master")
    {
        MutationExecutionPolicy::StoppedExclusive
    } else if path == "send"
        || path.starts_with("automation jobs ")
        || path == "automation routines trigger"
        || path == "extensions activate"
        || (path.starts_with("labs experiments campaigns ")
            && matches!(
                verb,
                "start" | "cancel" | "pause" | "resume" | "promote" | "reissue-lease"
            ))
    {
        MutationExecutionPolicy::RuntimeRequired
    } else if path.starts_with("extensions ") || path.starts_with("automation projects ") {
        MutationExecutionPolicy::ActiveCoordinated
    } else if path.starts_with("dev ")
        || path.starts_with("media ")
        || path.ends_with(" export")
        || matches!(verb, "screenshot" | "launch-test" | "sync")
    {
        MutationExecutionPolicy::ExternalDirect
    } else {
        MutationExecutionPolicy::DurableImmediate
    };
    Ok(CliLeafEffect::Mutating(policy))
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

    #[test]
    fn canonical_cli_effect_vocabulary_is_closed() {
        assert_eq!(
            canonical_cli_leaf_effect("status tools"),
            Ok(CliLeafEffect::ReadOnly)
        );
        assert_eq!(
            canonical_cli_leaf_effect("setup reset"),
            Ok(CliLeafEffect::Mutating(
                MutationExecutionPolicy::StoppedExclusive
            ))
        );
        assert!(canonical_cli_leaf_effect("config frobnicate").is_err());
    }
}
