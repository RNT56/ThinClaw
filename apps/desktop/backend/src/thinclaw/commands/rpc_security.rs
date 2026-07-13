//! Read-only security posture for the Desktop operator surface.

use tauri::State;

use super::types::{
    SandboxSecurityPosture, SecurityPosture, SecurityTelemetryEvent, SecurityTelemetrySummary,
    ToolSecurityPosture, ToolSecuritySummary,
};
use super::ThinClawManager;
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_security_posture(
    runtime: State<'_, ThinClawRuntimeState>,
    manager: State<'_, ThinClawManager>,
) -> Result<SecurityPosture, String> {
    let runtime_mode = runtime.mode_label().await.to_string();
    let auto_approve_enabled = manager
        .get_config()
        .await
        .map(|config| config.auto_approve_tools)
        .unwrap_or(false);

    let Some(evidence) = runtime.local_security_evidence().await else {
        let unavailable_reason = if runtime_mode == "remote" {
            "Security evidence belongs to the remote gateway and is not exposed by the current gateway contract."
        } else {
            "Start the local Agent runtime to inspect its effective safety, sandbox, and tool controls."
        };
        return Ok(SecurityPosture {
            runtime_mode,
            evidence_available: false,
            unavailable_reason: Some(unavailable_reason.to_string()),
            telemetry: SecurityTelemetrySummary::default(),
            sandbox: None,
            tools: ToolSecuritySummary {
                auto_approve_enabled,
                ..ToolSecuritySummary::default()
            },
        });
    };

    let telemetry = SecurityTelemetrySummary {
        sanitized: evidence.telemetry.sanitized,
        redacted: evidence.telemetry.redacted,
        blocked: evidence.telemetry.blocked,
        warned: evidence.telemetry.warned,
        recent_events: evidence
            .telemetry
            .recent_events
            .into_iter()
            .map(|event| SecurityTelemetryEvent {
                occurred_at_ms: event.occurred_at_ms,
                action: event.action.as_str().to_string(),
                source: event.source,
                reason: event.reason,
                severity: event.severity,
            })
            .collect(),
    };

    let registered = evidence.tools.len() as u64;
    let write_capable = evidence
        .tools
        .iter()
        .filter(|tool| tool.side_effect == "write")
        .count() as u64;
    let always_approval = evidence
        .tools
        .iter()
        .filter(|tool| tool.approval_class == "always")
        .count() as u64;
    let conditional_approval = evidence
        .tools
        .iter()
        .filter(|tool| tool.approval_class == "conditional")
        .count() as u64;
    let write_without_coarse_approval = evidence
        .tools
        .iter()
        .filter(|tool| tool.side_effect == "write" && tool.approval_class == "never")
        .count() as u64;
    let reviewed_tools = evidence
        .tools
        .into_iter()
        .filter(|tool| tool.side_effect == "write" || tool.approval_class != "never")
        .map(|tool| ToolSecurityPosture {
            name: tool.name,
            side_effect: tool.side_effect,
            approval_class: tool.approval_class,
            empty_params_requirement: tool.empty_params_requirement,
            sanitizes_output: tool.sanitizes_output,
            reason: tool.reason,
        })
        .collect();

    Ok(SecurityPosture {
        runtime_mode,
        evidence_available: true,
        unavailable_reason: None,
        telemetry,
        sandbox: Some(SandboxSecurityPosture {
            enabled: evidence.sandbox.enabled,
            policy: evidence.sandbox.policy,
            network_allowlist: evidence.sandbox.network_allowlist,
            timeout_secs: evidence.sandbox.timeout_secs,
            memory_limit_mb: evidence.sandbox.memory_limit_mb,
        }),
        tools: ToolSecuritySummary {
            registered,
            write_capable,
            always_approval,
            conditional_approval,
            write_without_coarse_approval,
            auto_approve_enabled,
            reviewed_tools,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posture_command_never_exposes_raw_content_fields() {
        let posture = SecurityPosture {
            runtime_mode: "stopped".to_string(),
            evidence_available: false,
            unavailable_reason: Some("test".to_string()),
            telemetry: SecurityTelemetrySummary::default(),
            sandbox: None,
            tools: ToolSecuritySummary::default(),
        };
        let serialized = serde_json::to_string(&posture).expect("serialize posture");
        for forbidden in ["raw_output", "raw_prompt", "parameters", "secret_value"] {
            assert!(!serialized.contains(forbidden));
        }
    }
}
