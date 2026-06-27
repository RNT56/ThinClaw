//! Contract adapter for optional `agentic-flows` consumption.
//!
//! ThinClaw remains an independent project. This module maps a selected
//! `agentic-flows` definition into ThinClaw's existing routine shape and records
//! flow provenance beside operator approval decisions.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use thinclaw_types::ToolProfile;

use crate::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineCatchUpMode, RoutineGuardrails, RoutinePolicy,
    Trigger,
};

/// Stable source metadata for a flow selected from `agentic-flows`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgenticFlowRef {
    pub id: String,
    pub version: String,
    pub title: String,
    pub summary: String,
    pub source: String,
}

impl AgenticFlowRef {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        title: impl Into<String>,
        summary: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            title: title.into(),
            summary: summary.into(),
            source: source.into(),
        }
    }
}

/// Operator decision recorded against a source flow version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowApprovalDecision {
    Approved,
    Rejected,
    Deferred,
}

impl FlowApprovalDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Deferred => "deferred",
        }
    }
}

/// Options for creating a ThinClaw routine from a source flow.
#[derive(Debug, Clone)]
pub struct AgenticRoutineOptions {
    pub user_id: String,
    pub actor_id: Option<String>,
    pub max_iterations: u32,
    pub allowed_tools: Option<Vec<String>>,
    pub allowed_skills: Option<Vec<String>>,
    pub tool_profile: ToolProfile,
}

impl AgenticRoutineOptions {
    pub fn for_user(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            actor_id: None,
            max_iterations: 10,
            allowed_tools: None,
            allowed_skills: None,
            tool_profile: ToolProfile::Restricted,
        }
    }
}

/// Build a manual ThinClaw routine that carries source-flow provenance in state.
pub fn routine_from_agentic_flow(flow: AgenticFlowRef, options: AgenticRoutineOptions) -> Routine {
    let now = Utc::now();
    Routine {
        id: Uuid::new_v4(),
        name: flow.title.clone(),
        description: flow.summary.clone(),
        user_id: options.user_id,
        actor_id: options.actor_id.unwrap_or_default(),
        enabled: true,
        trigger: Trigger::Manual,
        action: RoutineAction::FullJob {
            title: format!("Agentic flow: {}", flow.title),
            description: format!(
                "Run agentic-flows {}@{} from {}.\n\n{}",
                flow.id, flow.version, flow.source, flow.summary
            ),
            max_iterations: options.max_iterations,
            allowed_tools: options.allowed_tools,
            allowed_skills: options.allowed_skills,
            tool_profile: Some(options.tool_profile),
        },
        guardrails: RoutineGuardrails::default(),
        notify: NotifyConfig {
            user: String::new(),
            ..NotifyConfig::default()
        },
        policy: RoutinePolicy {
            catch_up_mode: RoutineCatchUpMode::Skip,
            max_event_age_secs: None,
        },
        last_run_at: None,
        next_fire_at: None,
        run_count: 0,
        consecutive_failures: 0,
        state: flow_provenance_state(&flow),
        config_version: 1,
        created_at: now,
        updated_at: now,
    }
}

/// Metadata to persist with an operator approval decision.
pub fn approval_decision_metadata(
    flow: &AgenticFlowRef,
    decision: FlowApprovalDecision,
) -> serde_json::Value {
    serde_json::json!({
        "source": "agentic-flows",
        "decision": decision.as_str(),
        "flow": {
            "id": flow.id.clone(),
            "version": flow.version.clone(),
            "source": flow.source.clone(),
        }
    })
}

fn flow_provenance_state(flow: &AgenticFlowRef) -> serde_json::Value {
    serde_json::json!({
        "agentic_flow": {
            "id": flow.id.clone(),
            "version": flow.version.clone(),
            "source": flow.source.clone(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_flow() -> AgenticFlowRef {
        AgenticFlowRef::new(
            "general.human-in-the-loop-review",
            "0.1.0",
            "Human review",
            "Review a proposed action before it is finalized.",
            "flows/general/human-in-the-loop-review/flow.yaml",
        )
    }

    #[test]
    fn routine_from_agentic_flow_carries_source_version() {
        let flow = review_flow();
        let mut options = AgenticRoutineOptions::for_user("operator");
        options.allowed_tools = Some(vec!["approval.request".to_string()]);

        let routine = routine_from_agentic_flow(flow, options);

        assert!(matches!(routine.trigger, Trigger::Manual));
        assert_eq!(
            routine.state["agentic_flow"]["id"],
            "general.human-in-the-loop-review"
        );
        assert_eq!(routine.state["agentic_flow"]["version"], "0.1.0");
        assert_eq!(routine.policy.catch_up_mode, RoutineCatchUpMode::Skip);
        match routine.action {
            RoutineAction::FullJob {
                allowed_tools,
                tool_profile,
                ..
            } => {
                assert_eq!(allowed_tools, Some(vec!["approval.request".to_string()]));
                assert_eq!(tool_profile, Some(ToolProfile::Restricted));
            }
            other => panic!("expected full job routine, got {other:?}"),
        }
    }

    #[test]
    fn approval_decision_metadata_carries_flow_version() {
        let metadata = approval_decision_metadata(&review_flow(), FlowApprovalDecision::Approved);

        assert_eq!(metadata["source"], "agentic-flows");
        assert_eq!(metadata["decision"], "approved");
        assert_eq!(metadata["flow"]["id"], "general.human-in-the-loop-review");
        assert_eq!(metadata["flow"]["version"], "0.1.0");
    }
}
