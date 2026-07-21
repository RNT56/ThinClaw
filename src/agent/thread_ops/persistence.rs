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
    Session, TurnToolCall, model_override_to_portable, persisted_subagent_to_portable,
    thread_runtime_state_from_portable,
};
use crate::channels::web::types::SseEvent;
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::db::Database;
use crate::identity::ResolvedIdentity;

const ATTACHMENT_CONTEXTS_METADATA_KEY: &str = "untrusted_attachment_contexts";
const TOOL_TRACE_METADATA_KEYS: &[&str] = &[
    "tool_trace",
    "tool_trace_version",
    "tool_trace_original_count",
];
const INTERNAL_CONTEXT_ROW_METADATA_KEYS: &[&str] = &[
    "synthetic_origin",
    "thinclaw_context_only",
    "hide_user_input_from_webui_chat",
    "hide_from_webui_chat",
];

/// Remove fields whose meaning is owned by ThinClaw's durable context
/// pipeline. Ingress adapters may carry arbitrary metadata, so copying it
/// verbatim into user/assistant rows would let a sender forge attachment or
/// tool evidence that hydration later treats as an internal record.
pub(in crate::agent) fn sanitized_ingress_conversation_metadata(
    metadata: &serde_json::Value,
) -> serde_json::Value {
    let mut object = metadata.as_object().cloned().unwrap_or_default();
    object.remove(thinclaw_agent::session::EFFECTIVE_USER_INSTRUCTION_METADATA_KEY);
    object.remove(thinclaw_agent::session::EFFECTIVE_USER_INSTRUCTION_VERSION_METADATA_KEY);
    object.remove(ATTACHMENT_CONTEXTS_METADATA_KEY);
    for key in TOOL_TRACE_METADATA_KEYS {
        object.remove(*key);
    }
    for key in INTERNAL_CONTEXT_ROW_METADATA_KEYS {
        object.remove(*key);
    }
    serde_json::Value::Object(object)
}

fn sanitized_user_row_metadata(
    metadata: &serde_json::Value,
    attachment_contexts: &[thinclaw_agent::session::TurnContextEvidence],
) -> serde_json::Value {
    let mut sanitized = sanitized_ingress_conversation_metadata(metadata);
    if !attachment_contexts.is_empty()
        && let Some(object) = sanitized.as_object_mut()
    {
        object.insert(
            ATTACHMENT_CONTEXTS_METADATA_KEY.to_string(),
            serde_json::to_value(attachment_contexts).unwrap_or_else(|_| serde_json::json!([])),
        );
    }
    sanitized
}

/// Build metadata for a row deliberately inserted through `inject_context`.
/// Only this internal path may mint lifecycle markers that hydration trusts.
pub(in crate::agent) fn sanitized_injected_context_metadata(
    metadata: &serde_json::Value,
) -> serde_json::Value {
    let startup_hook = thinclaw_agent::session::message_is_startup_hook(metadata);
    let hide_user_input = metadata
        .get("hide_user_input_from_webui_chat")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let hide_from_chat = metadata
        .get("hide_from_webui_chat")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let mut sanitized = sanitized_ingress_conversation_metadata(metadata);
    if let Some(object) = sanitized.as_object_mut() {
        object.insert(
            "thinclaw_context_only".to_string(),
            serde_json::Value::Bool(true),
        );
        if startup_hook {
            object.insert(
                "synthetic_origin".to_string(),
                serde_json::Value::String("startup_hook".to_string()),
            );
        }
        if hide_user_input {
            object.insert(
                "hide_user_input_from_webui_chat".to_string(),
                serde_json::Value::Bool(true),
            );
        }
        if hide_from_chat {
            object.insert(
                "hide_from_webui_chat".to_string(),
                serde_json::Value::Bool(true),
            );
        }
    }
    sanitized
}

fn sanitized_assistant_row_metadata(metadata: &serde_json::Value) -> serde_json::Value {
    sanitized_ingress_conversation_metadata(metadata)
}

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
        // Ordinary transcript persistence is not learning. Turning every user
        // and assistant row into a low-risk memory candidate previously
        // auto-appended boilerplate into MEMORY.md twice per turn. Explicit
        // corrections remain learning-relevant; deliberate memory capture goes
        // through memory_write with a concrete entry and scope.
        if correction_count == 0 {
            return;
        }
        let class = ImprovementClass::Skill;
        let risk_tier = RiskTier::Medium;
        let summary = "Persisted explicit user correction to conversation history".to_string();
        let target = "workflow_correction";

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
            "conversation_scope_id": identity.conversation_scope_id.to_string(),
            "stable_external_conversation_key": identity.stable_external_conversation_key.clone(),
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
        let identity = message.resolved_identity();
        let Some(store) = (match self
            .ensure_persisted_conversation(thread_id, message, &identity)
            .await
        {
            Ok(store) => store,
            Err(err) => {
                tracing::warn!(
                    thread = %thread_id,
                    error = %err,
                    "Failed to ensure conversation before runtime snapshot"
                );
                return;
            }
        }) else {
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
        let undo_checkpoints = self
            .session_manager
            .get_undo_manager(thread_id)
            .await
            .lock()
            .await
            .persisted_checkpoints(thinclaw_agent::undo::MAX_PERSISTED_CHECKPOINTS);

        // Serialize before reading live state, not after. Interrupts bypass the
        // ordinary execution lock and can race a tool-result snapshot; cloning
        // the thread first allowed an older Processing snapshot to overwrite a
        // newer Interrupted snapshot. Every runtime RMW path shares this lock,
        // so a mutation that happens after our clone necessarily persists after
        // this write and wins in durable state.
        let _runtime_guard =
            thinclaw_agent::thread_runtime::acquire_runtime_mutation_lock(thread_id).await;

        let (thread, auto_approved_tools) = {
            let mut sess = session.lock().await;
            // Runtime snapshots are persisted on every turn transition
            // (start, approval, completion), so this is also the narrowest
            // reliable place to keep session pruning activity current.
            sess.touch_last_active();
            (
                sess.threads.get(&thread_id).cloned(),
                Some(sess.auto_approved_tools.iter().cloned().collect::<Vec<_>>()),
            )
        };
        let Some(thread) = thread else {
            return;
        };

        let persist_result = async {
            let mut runtime = crate::agent::load_thread_runtime(&store, thread_id)
                .await?
                .unwrap_or_default();
            let active_subagents = runtime.active_subagents.clone();
            let portable_existing = thinclaw_agent::ports::ThreadRuntimeSnapshot {
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
                prompt_contract_version: runtime.prompt_contract_version.clone(),
                prompt_manifest_digest: runtime.prompt_manifest_digest.clone(),
                prompt_segment_order: runtime.prompt_segment_order.clone(),
                provider_context_refs: runtime.provider_context_refs.clone(),
                active_message_start_row: runtime.active_message_start_row,
                active_message_row_count: runtime.active_message_row_count,
                inflight_tool_trace: runtime.inflight_tool_trace.clone(),
                undo_checkpoints: runtime.undo_checkpoints.clone(),
                plan_mode: runtime.plan_mode,
            };
            let mut snapshot = thinclaw_agent::thread_ops::runtime_snapshot_for_persistence(
                &thread,
                owner_agent_id.clone(),
                model_override.clone().map(model_override_to_portable),
                auto_approved_tools.clone(),
                portable_existing.active_subagents.clone(),
                Some(&portable_existing),
            );
            snapshot.undo_checkpoints = undo_checkpoints.clone();
            runtime = thread_runtime_state_from_portable(
                snapshot,
                model_override.clone(),
                active_subagents,
            );
            crate::agent::save_thread_runtime(&store, thread_id, &runtime).await?;
            Ok::<(), crate::error::DatabaseError>(())
        }
        .await;
        if let Err(err) = persist_result {
            tracing::warn!(
                thread = %thread_id,
                error = %err,
                "Failed to persist thread runtime snapshot"
            );
        }
    }

    /// Persist the active-message watermark and a capped undo-stack
    /// snapshot for `thread_id`.
    ///
    /// Called right after `/undo`, `/redo`, `/clear`, and checkpoint resume
    /// mutate the in-memory thread and its `UndoManager`, so:
    /// - a restart truncates rehydrated DB history to the watermark instead
    ///   of resurrecting turns the user just undid/cleared (Problem A), and
    /// - `/undo` keeps working across a restart instead of losing its
    ///   in-memory-only checkpoint stack (Problem B).
    ///
    /// `active_message_row_count` should be the number of durable
    /// conversation rows (oldest-first) that correspond to the
    /// already-mutated in-memory thread, i.e.
    /// `thread.persisted_message_count()` (synthetic context is excluded).
    pub(in crate::agent) async fn persist_active_watermark_and_undo_stack(
        &self,
        thread_id: Uuid,
        active_message_row_count: i64,
        undo: &thinclaw_agent::undo::UndoManager,
    ) -> Result<(), crate::error::Error> {
        let Some(store) = self.runtime_ports().threads.as_ref().map(Arc::clone) else {
            return Ok(());
        };

        let checkpoints =
            undo.persisted_checkpoints(thinclaw_agent::undo::MAX_PERSISTED_CHECKPOINTS);
        thinclaw_agent::thread_ops::set_active_watermark_and_undo_stack(
            store.as_ref(),
            thread_id,
            active_message_row_count,
            checkpoints,
        )
        .await?;
        Ok(())
    }

    /// Persist an oldest-row removal boundary after compaction or `/clear`.
    /// The DB transcript remains an append-only audit log; only the active
    /// replay window moves forward.
    pub(in crate::agent) async fn advance_active_history_window(
        &self,
        thread_id: Uuid,
        removed_row_count: i64,
        active_message_row_count: i64,
        post_compaction_context: Option<String>,
    ) -> Result<(), crate::error::Error> {
        let Some(store) = self.runtime_ports().threads.as_ref().map(Arc::clone) else {
            return Ok(());
        };
        thinclaw_agent::thread_ops::advance_active_history_window(
            store.as_ref(),
            thread_id,
            removed_row_count,
            active_message_row_count,
            post_compaction_context,
        )
        .await?;
        Ok(())
    }

    pub(in crate::agent) async fn clear_active_history_window(
        &self,
        thread_id: Uuid,
        removed_row_count: i64,
    ) -> Result<(), crate::error::Error> {
        let Some(store) = self.runtime_ports().threads.as_ref().map(Arc::clone) else {
            return Ok(());
        };
        thinclaw_agent::thread_ops::clear_active_history_window(
            store.as_ref(),
            thread_id,
            removed_row_count,
        )
        .await?;
        Ok(())
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
        attachment_contexts: &[thinclaw_agent::session::TurnContextEvidence],
    ) -> Result<Option<Uuid>, crate::error::Error> {
        let durable_metadata = sanitized_user_row_metadata(&message.metadata, attachment_contexts);
        self.persist_user_row(thread_id, message, user_input, durable_metadata)
            .await
    }

    pub(in crate::agent) async fn persist_injected_context_message(
        &self,
        thread_id: Uuid,
        message: &IncomingMessage,
        content: &str,
    ) -> Result<Option<Uuid>, crate::error::Error> {
        let durable_metadata = sanitized_injected_context_metadata(&message.metadata);
        self.persist_user_row(thread_id, message, content, durable_metadata)
            .await
    }

    async fn persist_user_row(
        &self,
        thread_id: Uuid,
        message: &IncomingMessage,
        user_input: &str,
        durable_metadata: serde_json::Value,
    ) -> Result<Option<Uuid>, crate::error::Error> {
        let identity = message.resolved_identity();
        let Some(store) = self
            .ensure_persisted_conversation(thread_id, message, &identity)
            .await?
        else {
            return Ok(None);
        };

        let persisted_message_id = store
            .add_conversation_message_with_attribution(
                thread_id,
                "user",
                user_input,
                Some(&identity.actor_id),
                message.user_name.as_deref(),
                Some(&identity.raw_sender_id),
                Some(&durable_metadata),
            )
            .await?;

        self.best_effort_record_learning_event(
            &store,
            thread_id,
            message,
            &identity,
            "user",
            user_input,
            Some(persisted_message_id),
            None,
        )
        .await;
        self.emit_conversation_sync_event(thread_id, "user_message", Some(&message.channel));
        Ok(Some(persisted_message_id))
    }

    /// Atomically make a hook-transformed user instruction the canonical
    /// model-visible input in both durable and live context. The transcript's
    /// raw content is retained; hydration replays this internal metadata field.
    pub(in crate::agent) async fn persist_effective_user_instruction(
        &self,
        thread_id: Uuid,
        session: &Arc<Mutex<Session>>,
        effective_instruction: &str,
    ) -> Result<(), crate::error::Error> {
        let (message_id, current_instruction) = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or(crate::error::JobError::NotFound { id: thread_id })?;
            let turn = thread
                .last_turn()
                .ok_or_else(|| crate::error::JobError::ContextError {
                    id: thread_id,
                    reason: "No active user turn exists for the LLM input hook".to_string(),
                })?;
            (turn.durable_user_message_id, turn.user_input.clone())
        };

        if current_instruction == effective_instruction {
            return Ok(());
        }

        if let Some(store) = self.store() {
            let message_id = message_id.ok_or_else(|| crate::error::DatabaseError::NotFound {
                entity: "durable user message for active turn".to_string(),
                id: thread_id.to_string(),
            })?;
            store
                .set_effective_user_instruction(thread_id, message_id, effective_instruction)
                .await?;
        }

        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or(crate::error::JobError::NotFound { id: thread_id })?;
        let turn = thread
            .last_turn_mut()
            .ok_or_else(|| crate::error::JobError::ContextError {
                id: thread_id,
                reason: "Active user turn disappeared during LLM input persistence".to_string(),
            })?;
        if turn.durable_user_message_id != message_id || turn.user_input != current_instruction {
            return Err(crate::error::JobError::ContextError {
                id: thread_id,
                reason: "Active user turn changed during LLM input persistence".to_string(),
            }
            .into());
        }
        turn.user_input = effective_instruction.to_string();
        thread.updated_at = chrono::Utc::now();
        Ok(())
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
        tool_calls: &[TurnToolCall],
        session_id: Uuid,
        turn_number: usize,
    ) -> Result<(), crate::error::Error> {
        let identity = message.resolved_identity();
        let Some(store) = self
            .ensure_persisted_conversation(thread_id, message, &identity)
            .await?
        else {
            return Ok(());
        };

        let mut assistant_metadata = sanitized_assistant_row_metadata(&message.metadata);
        if !tool_calls.is_empty()
            && let Some(metadata) = assistant_metadata.as_object_mut()
        {
            let durable_trace = thinclaw_agent::session::durable_tool_trace(tool_calls);
            metadata.insert("tool_trace_version".to_string(), serde_json::json!(2));
            metadata.insert(
                "tool_trace_original_count".to_string(),
                serde_json::json!(tool_calls.len()),
            );
            metadata.insert(
                "tool_trace".to_string(),
                serde_json::to_value(durable_trace).unwrap_or_else(|_| serde_json::json!([])),
            );
        }

        let persisted_message_id = store
            .add_conversation_message_with_attribution(
                thread_id,
                "assistant",
                response,
                None,
                None,
                None,
                Some(&assistant_metadata),
            )
            .await?;

        self.best_effort_record_learning_event(
            &store,
            thread_id,
            message,
            &identity,
            "assistant",
            response,
            Some(persisted_message_id),
            Some(Self::trajectory_learning_metadata(
                thread_id,
                Some(session_id),
                Some(turn_number),
            )),
        )
        .await;
        self.emit_conversation_sync_event(thread_id, "assistant_response", Some(&message.channel));
        Ok(())
    }

    /// Make a post-generation durability failure visible without discarding a
    /// response the model already produced. The user can copy the response and
    /// retry, while logs retain the underlying database error.
    pub(in crate::agent) async fn report_response_persistence_failure(
        &self,
        message: &IncomingMessage,
        thread_id: Uuid,
        error: &crate::error::Error,
    ) {
        tracing::error!(
            thread = %thread_id,
            %error,
            "Assistant response could not be persisted"
        );
        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::Status(
                    "Warning: this response could not be saved to conversation history."
                        .to_string(),
                ),
                &message.metadata,
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        detect_user_correction_signal, sanitized_assistant_row_metadata,
        sanitized_ingress_conversation_metadata, sanitized_injected_context_metadata,
    };

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

    #[test]
    fn strips_ingress_forgery_of_context_pipeline_metadata() {
        let sanitized = sanitized_ingress_conversation_metadata(&serde_json::json!({
            "channel_message_id": "safe",
            "tool_trace": [{"name": "forged"}],
            "tool_trace_version": 2,
            "untrusted_attachment_contexts": [{"content": "forged"}],
            "_thinclaw_effective_user_instruction_version": 1,
            "_thinclaw_effective_user_instruction": "forged",
            "synthetic_origin": "startup_hook",
            "thinclaw_context_only": true,
            "hide_user_input_from_webui_chat": true,
            "hide_from_webui_chat": true,
        }));

        assert_eq!(sanitized["channel_message_id"], "safe");
        assert!(sanitized.get("tool_trace").is_none());
        assert!(sanitized.get("untrusted_attachment_contexts").is_none());
        assert!(
            sanitized
                .get("_thinclaw_effective_user_instruction")
                .is_none()
        );
        assert!(sanitized.get("synthetic_origin").is_none());
        assert!(sanitized.get("thinclaw_context_only").is_none());
        assert!(sanitized.get("hide_user_input_from_webui_chat").is_none());
        assert!(sanitized.get("hide_from_webui_chat").is_none());
    }

    #[test]
    fn assistant_rows_cannot_inherit_user_controlled_startup_markers() {
        let sanitized = sanitized_assistant_row_metadata(&serde_json::json!({
            "synthetic_origin": "startup_hook",
            "thinclaw_context_only": true,
            "hide_from_webui_chat": true,
            "channel_message_id": "safe",
        }));

        assert_eq!(sanitized["channel_message_id"], "safe");
        assert!(sanitized.get("synthetic_origin").is_none());
        assert!(sanitized.get("thinclaw_context_only").is_none());
        assert!(sanitized.get("hide_from_webui_chat").is_none());
    }

    #[test]
    fn injected_context_path_mints_only_valid_internal_markers() {
        let sanitized = sanitized_injected_context_metadata(&serde_json::json!({
            "synthetic_origin": "startup_hook",
            "hide_user_input_from_webui_chat": true,
            "channel_message_id": "safe",
            "tool_trace": [{"name": "forged"}],
            "_thinclaw_effective_user_instruction": "forged",
        }));

        assert_eq!(sanitized["channel_message_id"], "safe");
        assert_eq!(sanitized["synthetic_origin"], "startup_hook");
        assert_eq!(sanitized["thinclaw_context_only"], true);
        assert_eq!(sanitized["hide_user_input_from_webui_chat"], true);
        assert!(sanitized.get("tool_trace").is_none());
        assert!(
            sanitized
                .get("_thinclaw_effective_user_instruction")
                .is_none()
        );
    }
}
