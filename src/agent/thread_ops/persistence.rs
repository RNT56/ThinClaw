//! Message/runtime-snapshot persistence, learning-event recording, and
//! context-pressure tracking.

use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::context_monitor::{ContextPressure, pressure_message, pressure_transition};
use crate::agent::learning::{ImprovementClass, LearningEvent, LearningOrchestrator, RiskTier};
use crate::agent::outcomes;
use crate::agent::session::{
    Session, Thread, model_override_to_portable, persisted_subagent_to_portable,
    thread_runtime_state_from_portable,
};
use crate::agent::{load_thread_runtime, mutate_thread_runtime};
use crate::channels::web::types::SseEvent;
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::db::Database;
use crate::identity::ResolvedIdentity;

fn detect_user_correction_signal(role: &str, content: &str) -> u32 {
    thinclaw_agent::thread_ops::detect_user_correction_signal(role, content)
}

impl Agent {
    fn compact_text_preview(text: &str) -> String {
        thinclaw_agent::thread_ops::compact_text_preview(text)
    }

    fn trajectory_learning_metadata(
        thread_id: Uuid,
        session_id: Option<Uuid>,
        turn_number: Option<usize>,
    ) -> serde_json::Value {
        thinclaw_agent::thread_ops::trajectory_learning_metadata(thread_id, session_id, turn_number)
    }

    async fn best_effort_record_learning_event(
        &self,
        store: &Arc<dyn Database>,
        thread_id: Uuid,
        message: &IncomingMessage,
        identity: &ResolvedIdentity,
        role: &str,
        content: &str,
        persisted_message_id: Option<Uuid>,
        trajectory_metadata: Option<serde_json::Value>,
    ) {
        let correction_count = detect_user_correction_signal(role, content);
        let class = if correction_count > 0 {
            ImprovementClass::Skill
        } else {
            ImprovementClass::Memory
        };
        let risk_tier = if correction_count > 0 {
            RiskTier::Medium
        } else {
            RiskTier::Low
        };
        let summary = if correction_count > 0 {
            "Persisted explicit user correction to conversation history".to_string()
        } else {
            format!("Persisted {} message to conversation history", role)
        };
        let target = if correction_count > 0 {
            "workflow_correction"
        } else {
            "conversation_history"
        };

        let job_id = message
            .metadata
            .get("job_id")
            .and_then(|v| v.as_str())
            .and_then(|value| Uuid::parse_str(value).ok());

        let mut learning_metadata = json!({
            "thread_id": thread_id.to_string(),
            "channel": message.channel.clone(),
            "role": role,
            "principal_id": identity.principal_id.clone(),
            "actor_id": identity.actor_id.clone(),
            "conversation_kind": identity.conversation_kind.as_str(),
            "message_id": message.id.to_string(),
            "content_length": content.len(),
            "content_preview": Self::compact_text_preview(content),
            "received_at": message.received_at.to_rfc3339(),
            "correction_count": correction_count,
            "repeated_failures": correction_count,
            "success": !(role.eq_ignore_ascii_case("user") && correction_count > 0),
        });
        if let Some(target) = learning_metadata.as_object_mut()
            && let Some(extra_obj) = trajectory_metadata
                .as_ref()
                .and_then(|value| value.as_object())
        {
            for (key, value) in extra_obj {
                target.insert(key.clone(), value.clone());
            }
        }

        let learning_event = LearningEvent::new(
            format!("thread_ops::persist_{}_message", role),
            class,
            risk_tier,
            summary,
        )
        .with_target(target)
        .with_metadata(learning_metadata);

        let persisted_event = learning_event.into_persisted(
            identity.principal_id.clone(),
            Some(identity.actor_id.clone()),
            Some(message.channel.clone()),
            Some(thread_id.to_string()),
            Some(thread_id),
            persisted_message_id,
            job_id,
        );

        let mut outcome_payload = serde_json::json!({});
        match store.insert_learning_event(&persisted_event).await {
            Ok(event_id) => {
                let orchestrator = LearningOrchestrator::new(
                    Arc::clone(store),
                    self.workspace().cloned(),
                    self.skill_registry().cloned(),
                );
                match orchestrator
                    .handle_event(
                        if role.eq_ignore_ascii_case("assistant") {
                            "assistant_turn_complete"
                        } else {
                            "user_turn_input"
                        },
                        &persisted_event,
                    )
                    .await
                {
                    Ok(outcome) => {
                        outcome_payload = serde_json::json!(outcome);
                    }
                    Err(err) => {
                        tracing::debug!(
                            thread = %thread_id,
                            event_id = %event_id,
                            error = %err,
                            "Learning orchestrator skipped event"
                        );
                    }
                }

                let outcome_result = if role.eq_ignore_ascii_case("assistant") {
                    outcomes::maybe_create_turn_contract(store, &persisted_event).await
                } else {
                    outcomes::observe_user_turn(store, &persisted_event)
                        .await
                        .map(|_| None)
                };
                if let Err(err) = outcome_result {
                    tracing::debug!(
                        thread = %thread_id,
                        event_id = %event_id,
                        error = %err,
                        "Outcome-backed learning hook skipped event"
                    );
                }
            }
            Err(err) => {
                tracing::debug!(
                    thread = %thread_id,
                    error = %err,
                    "Best-effort learning event insert failed"
                );
            }
        }

        let payload = serde_json::to_value(&persisted_event).unwrap_or_else(|_| {
            json!({
                "id": persisted_event.id.to_string(),
                "source": persisted_event.source,
                "event_type": persisted_event.event_type,
                "payload": persisted_event.payload,
                "metadata": persisted_event.metadata,
                "created_at": persisted_event.created_at.to_rfc3339(),
            })
        });

        if let Some(job_id) = job_id
            && let Err(err) = store
                .save_job_event(job_id, "learning_event", &payload)
                .await
        {
            tracing::debug!(
                thread = %thread_id,
                job_id = %job_id,
                error = %err,
                "Best-effort learning event job write failed"
            );
        }

        let summary_payload = serde_json::json!({
            "event": payload,
            "outcome": outcome_payload,
        });
        if let Err(err) = store
            .update_conversation_metadata_field(thread_id, "learning_last_event", &summary_payload)
            .await
        {
            tracing::debug!(
                thread = %thread_id,
                error = %err,
                "Best-effort learning event conversation metadata write failed"
            );
        }
    }

    pub(in crate::agent) async fn persist_thread_runtime_snapshot(
        &self,
        message: &IncomingMessage,
        session: &Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) {
        let (thread, auto_approved_tools) = {
            let sess = session.lock().await;
            (
                sess.threads.get(&thread_id).cloned(),
                Some(sess.auto_approved_tools.iter().cloned().collect::<Vec<_>>()),
            )
        };
        let Some(thread) = thread else {
            return;
        };
        self.persist_thread_runtime_with_thread(message, thread_id, &thread, auto_approved_tools)
            .await;
    }

    async fn persist_thread_runtime_with_thread(
        &self,
        message: &IncomingMessage,
        thread_id: Uuid,
        thread: &Thread,
        auto_approved_tools: Option<Vec<String>>,
    ) {
        let identity = message.resolved_identity();
        let Some(store) = self
            .ensure_persisted_conversation(thread_id, message, &identity)
            .await
        else {
            return;
        };

        let owner_agent_id = match self.session_manager.get_thread_owner(thread_id).await {
            Some(owner) => Some(owner),
            None => self.agent_router.get_thread_owner(thread_id).await,
        };
        let model_override = if let Some(ref overrides) = self.deps.model_override {
            overrides.get(&format!("thread:{thread_id}")).await
        } else {
            None
        };
        let existing_runtime = match load_thread_runtime(&store, thread_id).await {
            Ok(runtime) => runtime,
            Err(err) => {
                tracing::debug!(
                    thread = %thread_id,
                    error = %err,
                    "Failed to load thread runtime before snapshot; preserving defaults"
                );
                None
            }
        };

        if let Err(err) = mutate_thread_runtime(&store, thread_id, |runtime| {
            let active_subagents = runtime.active_subagents.clone();
            let portable_existing = existing_runtime.as_ref().map(|runtime| {
                thinclaw_agent::ports::ThreadRuntimeSnapshot {
                    state: runtime.state.into(),
                    pending_approval: runtime.pending_approval.clone().map(Into::into),
                    pending_auth: runtime.pending_auth.clone().map(Into::into),
                    owner_agent_id: runtime.owner_agent_id.clone(),
                    model_override: runtime
                        .model_override
                        .clone()
                        .map(model_override_to_portable),
                    auto_approved_tools: runtime.auto_approved_tools.clone(),
                    active_subagents: runtime
                        .active_subagents
                        .iter()
                        .cloned()
                        .map(persisted_subagent_to_portable)
                        .collect(),
                    last_context_pressure: runtime
                        .last_context_pressure
                        .and_then(|pressure| serde_json::to_value(pressure).ok()),
                    post_compaction_context: runtime.post_compaction_context.clone(),
                    frozen_workspace_prompt: runtime.frozen_workspace_prompt.clone(),
                    frozen_provider_system_prompt: runtime.frozen_provider_system_prompt.clone(),
                    prompt_snapshot_hash: runtime.prompt_snapshot_hash.clone(),
                    ephemeral_overlay_hash: runtime.ephemeral_overlay_hash.clone(),
                    prompt_segment_order: runtime.prompt_segment_order.clone(),
                    provider_context_refs: runtime.provider_context_refs.clone(),
                }
            });
            let snapshot = thinclaw_agent::thread_ops::runtime_snapshot_for_persistence(
                thread,
                owner_agent_id.clone(),
                model_override.clone().map(model_override_to_portable),
                auto_approved_tools.clone(),
                portable_existing
                    .as_ref()
                    .map(|runtime| runtime.active_subagents.clone())
                    .unwrap_or_default(),
                portable_existing.as_ref(),
            );
            *runtime = thread_runtime_state_from_portable(
                snapshot,
                model_override.clone(),
                active_subagents,
            );
        })
        .await
        {
            tracing::warn!(
                thread = %thread_id,
                error = %err,
                "Failed to persist thread runtime snapshot"
            );
        }
    }

    pub(in crate::agent) async fn record_context_pressure_state(
        &self,
        thread_id: Uuid,
        usage_percent: f64,
    ) -> Option<ContextPressure> {
        let current_pressure = self.context_monitor.check_pressure(usage_percent as f32);
        let store = self.runtime_ports().threads.as_ref().map(Arc::clone)?;

        let previous_pressure =
            match thinclaw_agent::thread_ops::load_last_context_pressure(store.as_ref(), thread_id)
                .await
            {
                Ok(Some(value)) => serde_json::from_value::<ContextPressure>(value).ok(),
                Ok(None) => None,
                Err(err) => {
                    tracing::debug!(
                        thread = %thread_id,
                        error = %err,
                        "Failed to load thread runtime for context pressure tracking"
                    );
                    None
                }
            };

        if previous_pressure == Some(current_pressure) {
            return Some(current_pressure);
        }

        let persisted_pressure = if current_pressure == ContextPressure::None {
            None
        } else {
            serde_json::to_value(current_pressure).ok()
        };
        if let Err(err) = thinclaw_agent::thread_ops::set_last_context_pressure(
            store.as_ref(),
            thread_id,
            persisted_pressure,
        )
        .await
        {
            tracing::debug!(
                thread = %thread_id,
                error = %err,
                "Failed to persist context pressure state"
            );
        }

        Some(current_pressure)
    }

    pub(in crate::agent) async fn sync_context_pressure_warning(
        &self,
        message: &IncomingMessage,
        thread_id: Uuid,
        usage_percent: f64,
    ) {
        let current_pressure = self.context_monitor.check_pressure(usage_percent as f32);
        let Some(store) = self.runtime_ports().threads.as_ref().map(Arc::clone) else {
            return;
        };

        let previous_pressure =
            match thinclaw_agent::thread_ops::load_last_context_pressure(store.as_ref(), thread_id)
                .await
            {
                Ok(Some(value)) => serde_json::from_value::<ContextPressure>(value).ok(),
                Ok(None) => None,
                Err(err) => {
                    tracing::debug!(
                        thread = %thread_id,
                        error = %err,
                        "Failed to load thread runtime for context pressure warning"
                    );
                    None
                }
            };

        let warning_level = pressure_transition(previous_pressure, current_pressure);
        if let Some(level) = warning_level
            && let Some(status) = pressure_message(level)
        {
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status(status),
                    &message.metadata,
                )
                .await;
        }

        let _ = self
            .record_context_pressure_state(thread_id, usage_percent)
            .await;
    }

    /// Persist the user message to the DB at turn start (before the agentic loop).
    ///
    /// This ensures the user message is durable even if the process crashes
    /// mid-response. Call this right after `thread.start_turn()`.
    fn emit_conversation_sync_event(
        &self,
        thread_id: Uuid,
        reason: &'static str,
        channel: Option<&str>,
    ) {
        if let Some(ref sender) = self.deps.sse_sender {
            let _ = sender.send(SseEvent::ConversationUpdated {
                thread_id: thread_id.to_string(),
                reason: reason.to_string(),
                channel: channel.map(str::to_string),
            });
        }
    }

    pub(in crate::agent) async fn persist_user_message(
        &self,
        thread_id: Uuid,
        message: &IncomingMessage,
        user_input: &str,
    ) {
        let identity = message.resolved_identity();
        let Some(store) = self
            .ensure_persisted_conversation(thread_id, message, &identity)
            .await
        else {
            return;
        };

        let persisted_message_id = match store
            .add_conversation_message_with_attribution(
                thread_id,
                "user",
                user_input,
                Some(&identity.actor_id),
                message.user_name.as_deref(),
                Some(&identity.raw_sender_id),
                Some(&message.metadata),
            )
            .await
        {
            Ok(message_id) => Some(message_id),
            Err(e) => {
                tracing::warn!("Failed to persist user message: {}", e);
                return;
            }
        };

        self.best_effort_record_learning_event(
            &store,
            thread_id,
            message,
            &identity,
            "user",
            user_input,
            persisted_message_id,
            None,
        )
        .await;
        self.emit_conversation_sync_event(thread_id, "user_message", Some(&message.channel));
    }

    /// Persist the assistant response to the DB after the agentic loop completes.
    ///
    /// Re-ensures the conversation row exists so that assistant responses are
    /// still persisted even if `persist_user_message` failed transiently at
    /// turn start (e.g. a brief DB blip that resolved before response time).
    pub(in crate::agent) async fn persist_assistant_response(
        &self,
        thread_id: Uuid,
        message: &IncomingMessage,
        response: &str,
        session_id: Uuid,
        turn_number: usize,
    ) {
        let identity = message.resolved_identity();
        let Some(store) = self
            .ensure_persisted_conversation(thread_id, message, &identity)
            .await
        else {
            return;
        };

        let persisted_message_id = match store
            .add_conversation_message_with_attribution(
                thread_id,
                "assistant",
                response,
                None,
                None,
                None,
                Some(&message.metadata),
            )
            .await
        {
            Ok(message_id) => Some(message_id),
            Err(e) => {
                tracing::warn!("Failed to persist assistant message: {}", e);
                return;
            }
        };

        self.best_effort_record_learning_event(
            &store,
            thread_id,
            message,
            &identity,
            "assistant",
            response,
            persisted_message_id,
            Some(Self::trajectory_learning_metadata(
                thread_id,
                Some(session_id),
                Some(turn_number),
            )),
        )
        .await;
        self.emit_conversation_sync_event(thread_id, "assistant_response", Some(&message.channel));
    }
}

#[cfg(test)]
mod tests {
    use super::detect_user_correction_signal;

    #[test]
    fn detects_correction_prefixes() {
        assert_eq!(
            detect_user_correction_signal("user", "Actually, please use this endpoint."),
            1
        );
        assert_eq!(
            detect_user_correction_signal("user", "No, that's incorrect."),
            1
        );
    }

    #[test]
    fn ignores_non_correction_messages() {
        assert_eq!(
            detect_user_correction_signal("user", "Can you summarize this for me?"),
            0
        );
        assert_eq!(
            detect_user_correction_signal("assistant", "Actually this is fine."),
            0
        );
    }
}
