//! Session compatibility facade.
//!
//! Core session, thread, turn, approval, and auth domain types live in
//! `thinclaw-agent`. Root-only runtime envelope adapters stay here because
//! they depend on concrete sub-agent, model override, history, and context
//! pressure types.

use serde::{Deserialize, Serialize};
use thinclaw_agent::ports::{
    ModelOverride as PortableModelOverride, PortableSubagentState, ThreadMessage,
    ThreadRuntimeSnapshot,
};
pub use thinclaw_agent::session::{
    PendingApproval, PendingAuth, PendingAuthMode, Session, Thread, ThreadState, Turn, TurnState,
    TurnToolCall,
};
use uuid::Uuid;

use crate::agent::context_monitor::ContextPressure;
use crate::identity::ResolvedIdentity;

fn default_true() -> bool {
    true
}

/// Persisted record of a sub-agent that was active for a conversation thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSubagentState {
    pub agent_id: Uuid,
    pub name: String,
    pub request: crate::agent::subagent_executor::SubagentSpawnRequest,
    pub channel_name: String,
    #[serde(default)]
    pub channel_metadata: serde_json::Value,
    pub parent_user_id: String,
    #[serde(default)]
    pub parent_identity: Option<ResolvedIdentity>,
    pub parent_thread_id: String,
    #[serde(default = "default_true")]
    pub reinject_result: bool,
}

/// Durable runtime envelope stored in conversation metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadRuntimeState {
    #[serde(default)]
    pub state: ThreadState,
    #[serde(default)]
    pub pending_approval: Option<PendingApproval>,
    #[serde(default)]
    pub pending_auth: Option<PendingAuth>,
    #[serde(default)]
    pub owner_agent_id: Option<String>,
    #[serde(default)]
    pub model_override: Option<crate::tools::builtin::llm_tools::ModelOverride>,
    #[serde(default)]
    pub auto_approved_tools: Vec<String>,
    #[serde(default)]
    pub active_subagents: Vec<PersistedSubagentState>,
    #[serde(default)]
    pub last_context_pressure: Option<ContextPressure>,
    #[serde(default)]
    pub post_compaction_context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_workspace_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_provider_system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_snapshot_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral_overlay_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_segment_order: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_context_refs: Vec<String>,
}

/// Compatibility methods that used to be inherent on the root `Thread`.
pub trait ThreadRuntimeStateExt {
    fn runtime_state(
        &self,
        owner_agent_id: Option<String>,
        model_override: Option<crate::tools::builtin::llm_tools::ModelOverride>,
        auto_approved_tools: impl IntoIterator<Item = String>,
        active_subagents: Vec<PersistedSubagentState>,
        last_context_pressure: Option<ContextPressure>,
    ) -> ThreadRuntimeState;

    fn restore_runtime_state(&mut self, runtime: ThreadRuntimeState);

    fn restore_from_conversation_messages(
        &mut self,
        messages: &[crate::history::ConversationMessage],
    );
}

impl ThreadRuntimeStateExt for Thread {
    fn runtime_state(
        &self,
        owner_agent_id: Option<String>,
        model_override: Option<crate::tools::builtin::llm_tools::ModelOverride>,
        auto_approved_tools: impl IntoIterator<Item = String>,
        active_subagents: Vec<PersistedSubagentState>,
        last_context_pressure: Option<ContextPressure>,
    ) -> ThreadRuntimeState {
        let snapshot = self.runtime_snapshot(
            owner_agent_id.clone(),
            model_override.clone().map(model_override_to_portable),
            auto_approved_tools,
            active_subagents
                .iter()
                .cloned()
                .map(persisted_subagent_to_portable)
                .collect(),
            last_context_pressure.and_then(|pressure| serde_json::to_value(pressure).ok()),
        );

        ThreadRuntimeState {
            state: self.state,
            pending_approval: self.pending_approval.clone(),
            pending_auth: self.pending_auth.clone(),
            owner_agent_id: snapshot.owner_agent_id,
            model_override,
            auto_approved_tools: snapshot.auto_approved_tools,
            active_subagents,
            last_context_pressure,
            post_compaction_context: snapshot.post_compaction_context,
            frozen_workspace_prompt: snapshot.frozen_workspace_prompt,
            frozen_provider_system_prompt: snapshot.frozen_provider_system_prompt,
            prompt_snapshot_hash: snapshot.prompt_snapshot_hash,
            ephemeral_overlay_hash: snapshot.ephemeral_overlay_hash,
            prompt_segment_order: snapshot.prompt_segment_order,
            provider_context_refs: snapshot.provider_context_refs,
        }
    }

    fn restore_runtime_state(&mut self, runtime: ThreadRuntimeState) {
        self.restore_runtime_snapshot(ThreadRuntimeSnapshot {
            state: runtime.state.into(),
            pending_approval: runtime.pending_approval.map(Into::into),
            pending_auth: runtime.pending_auth.map(Into::into),
            owner_agent_id: runtime.owner_agent_id,
            model_override: runtime.model_override.map(model_override_to_portable),
            auto_approved_tools: runtime.auto_approved_tools,
            active_subagents: runtime
                .active_subagents
                .into_iter()
                .map(persisted_subagent_to_portable)
                .collect(),
            last_context_pressure: runtime
                .last_context_pressure
                .and_then(|pressure| serde_json::to_value(pressure).ok()),
            post_compaction_context: runtime.post_compaction_context,
            frozen_workspace_prompt: runtime.frozen_workspace_prompt,
            frozen_provider_system_prompt: runtime.frozen_provider_system_prompt,
            prompt_snapshot_hash: runtime.prompt_snapshot_hash,
            ephemeral_overlay_hash: runtime.ephemeral_overlay_hash,
            prompt_segment_order: runtime.prompt_segment_order,
            provider_context_refs: runtime.provider_context_refs,
        });
    }

    fn restore_from_conversation_messages(
        &mut self,
        messages: &[crate::history::ConversationMessage],
    ) {
        let portable = messages
            .iter()
            .map(|message| ThreadMessage {
                id: message.id,
                conversation_id: Uuid::nil(),
                role: message.role.clone(),
                content: message.content.clone(),
                actor_id: message.actor_id.clone(),
                actor_display_name: message.actor_display_name.clone(),
                raw_sender_id: message.raw_sender_id.clone(),
                metadata: message.metadata.clone(),
                created_at: message.created_at,
            })
            .collect::<Vec<_>>();
        self.restore_from_thread_messages(&portable);
    }
}

pub(crate) fn model_override_to_portable(
    value: crate::tools::builtin::llm_tools::ModelOverride,
) -> PortableModelOverride {
    PortableModelOverride {
        model_spec: value.model_spec,
        reason: value.reason,
    }
}

pub(crate) fn persisted_subagent_to_portable(
    value: PersistedSubagentState,
) -> PortableSubagentState {
    PortableSubagentState {
        agent_id: value.agent_id,
        name: value.name,
        request: serde_json::to_value(value.request).unwrap_or(serde_json::Value::Null),
        channel_name: value.channel_name,
        channel_metadata: value.channel_metadata,
        parent_user_id: value.parent_user_id,
        parent_thread_id: value.parent_thread_id,
        reinject_result: value.reinject_result,
    }
}

pub(crate) fn thread_runtime_state_from_portable(
    snapshot: ThreadRuntimeSnapshot,
    model_override: Option<crate::tools::builtin::llm_tools::ModelOverride>,
    active_subagents: Vec<PersistedSubagentState>,
) -> ThreadRuntimeState {
    ThreadRuntimeState {
        state: snapshot.state.into(),
        pending_approval: snapshot.pending_approval.map(Into::into),
        pending_auth: snapshot.pending_auth.map(Into::into),
        owner_agent_id: snapshot.owner_agent_id,
        model_override,
        auto_approved_tools: snapshot.auto_approved_tools,
        active_subagents,
        last_context_pressure: snapshot
            .last_context_pressure
            .and_then(|pressure| serde_json::from_value(pressure).ok()),
        post_compaction_context: snapshot.post_compaction_context,
        frozen_workspace_prompt: snapshot.frozen_workspace_prompt,
        frozen_provider_system_prompt: snapshot.frozen_provider_system_prompt,
        prompt_snapshot_hash: snapshot.prompt_snapshot_hash,
        ephemeral_overlay_hash: snapshot.ephemeral_overlay_hash,
        prompt_segment_order: snapshot.prompt_segment_order,
        provider_context_refs: snapshot.provider_context_refs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    #[test]
    fn test_restore_from_conversation_messages_preserves_startup_visibility() {
        let mut thread = Thread::new(Uuid::new_v4());
        let now: DateTime<Utc> = Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "boot prompt".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({"hide_from_webui_chat": true}),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "boot reply".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({"synthetic_origin": "startup_hook"}),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
        ];

        thread.restore_from_conversation_messages(&messages);

        assert_eq!(thread.turns.len(), 1);
        assert!(thread.turns[0].hide_user_input_from_ui);
        assert_eq!(thread.turns[0].response.as_deref(), Some("boot reply"));
    }
}
