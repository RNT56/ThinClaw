//! Thread lifecycle operations: undo/redo/resume, interrupt, compact, clear,
//! and new/switch thread.

use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::compaction::ContextCompactor;
use crate::agent::session::Session;
use crate::agent::submission::SubmissionResult;
use crate::channels::IncomingMessage;
use crate::error::Error;
use crate::identity::ResolvedIdentity;
use crate::tools::execution_backend::interactive_chat_runtime_descriptor;
use thinclaw_agent::thread_ops::{ThreadOperationMessage, UndoRedoAction, UndoRedoOutcome};

impl Agent {
    pub(in crate::agent) async fn process_undo(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut mgr = undo_mgr.lock().await;

        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        let outcome = thinclaw_agent::thread_ops::restore_thread_from_undo(thread, &mut mgr);
        match &outcome {
            UndoRedoOutcome::Restored { .. } => {
                let usage_percent = self
                    .effective_context_monitor()
                    .usage_percent(&thread.messages());
                // Row-count watermark for the thread as it stands *after*
                // the undo mutation above, so hydration truncates DB
                // history to match what /undo just restored in memory.
                let active_message_row_count = thread.messages().len() as i64;
                drop(sess);
                self.clear_thread_runtime_transients(thread_id).await;
                self.persist_active_watermark_and_undo_stack(
                    thread_id,
                    active_message_row_count,
                    &mgr,
                )
                .await;
                drop(mgr);
                self.record_context_pressure_state(thread_id, usage_percent)
                    .await;
            }
            UndoRedoOutcome::NothingAvailable | UndoRedoOutcome::Failed => {}
        }

        match thinclaw_agent::thread_ops::undo_redo_message(UndoRedoAction::Undo, &outcome) {
            ThreadOperationMessage::Ok(message) => Ok(SubmissionResult::ok_with_message(message)),
            ThreadOperationMessage::Error(message) => Ok(SubmissionResult::error(message)),
        }
    }

    pub(in crate::agent) async fn process_redo(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut mgr = undo_mgr.lock().await;

        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        let outcome = thinclaw_agent::thread_ops::restore_thread_from_redo(thread, &mut mgr);
        match &outcome {
            UndoRedoOutcome::Restored { .. } => {
                let usage_percent = self
                    .effective_context_monitor()
                    .usage_percent(&thread.messages());
                // Row-count watermark for the thread as it stands *after*
                // the redo mutation above, so hydration truncates DB
                // history to match what /redo just restored in memory.
                let active_message_row_count = thread.messages().len() as i64;
                drop(sess);
                self.clear_thread_runtime_transients(thread_id).await;
                self.persist_active_watermark_and_undo_stack(
                    thread_id,
                    active_message_row_count,
                    &mgr,
                )
                .await;
                drop(mgr);
                self.record_context_pressure_state(thread_id, usage_percent)
                    .await;
            }
            UndoRedoOutcome::NothingAvailable | UndoRedoOutcome::Failed => {}
        }

        match thinclaw_agent::thread_ops::undo_redo_message(UndoRedoAction::Redo, &outcome) {
            ThreadOperationMessage::Ok(message) => Ok(SubmissionResult::ok_with_message(message)),
            ThreadOperationMessage::Error(message) => Ok(SubmissionResult::error(message)),
        }
    }

    pub(in crate::agent) async fn process_interrupt(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        if thinclaw_agent::thread_ops::interrupt_thread(thread) {
            self.signal_turn_cancellation(thread_id).await;
            drop(sess);
            self.persist_thread_runtime_snapshot(message, &session, thread_id)
                .await;
            Ok(SubmissionResult::ok_with_message("Interrupted."))
        } else {
            Ok(SubmissionResult::ok_with_message("Nothing to interrupt."))
        }
    }

    pub(in crate::agent) async fn process_compact(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let session_user_id = sess.user_id.clone();
        let session_id = sess.id;
        let principal_id = sess.principal_id.clone();
        let actor_id = sess.actor_id.clone();
        let conversation_scope_id = sess.conversation_scope_id;
        let conversation_kind = sess.conversation_kind;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        let messages = thread.messages();
        let monitor = self.effective_context_monitor();
        let usage = monitor.usage_percent(&messages);
        let strategy = monitor.suggest_compaction(&messages).unwrap_or(
            crate::agent::context_monitor::CompactionStrategy::Summarize { keep_recent: 5 },
        );

        let mut compactor = ContextCompactor::new(self.llm().clone());
        if let Some(ref tracker) = self.deps.cost_tracker {
            compactor = compactor.with_cost_tracker(std::sync::Arc::clone(tracker));
        }
        match compactor
            .compact(thread, strategy, self.workspace().map(|w| w.as_ref()))
            .await
        {
            Ok(result) => {
                let compaction_summary = result.summary.clone();
                let usage_after = monitor.usage_percent(&thread.messages());
                let session_extract_artifact = crate::agent::AgentRunArtifact::new(
                    "thread_compaction",
                    crate::agent::AgentRunStatus::Completed,
                    chrono::Utc::now(),
                    Some(chrono::Utc::now()),
                )
                .with_runtime_descriptor(Some(&crate::agent::run_artifact::run_runtime_descriptor(
                    &interactive_chat_runtime_descriptor(),
                )))
                .with_metadata(serde_json::json!({
                    "event": "thread_compaction",
                    "thread_id": thread_id,
                    "turn_count": thread.turns.len(),
                    "strategy": format!("{strategy:?}"),
                    "tokens_before": result.tokens_before,
                    "tokens_after": result.tokens_after,
                }));
                let mut msg = format!(
                    "Compacted: {} turns removed, {} → {} tokens (was {:.1}% full)",
                    result.turns_removed, result.tokens_before, result.tokens_after, usage
                );
                if result.summary_written {
                    msg.push_str(", summary saved to workspace");
                }
                drop(sess);
                if let Some(store) = self.store().map(Arc::clone) {
                    let mut artifact = session_extract_artifact.clone();
                    artifact.session_id = Some(session_id);
                    artifact.thread_id = Some(thread_id);
                    artifact.user_id = Some(session_user_id.clone());
                    artifact.actor_id = Some(actor_id.clone());
                    artifact.conversation_scope_id = Some(conversation_scope_id);
                    artifact.conversation_kind = Some(conversation_kind.as_str().to_string());
                    let manager = crate::agent::learning::MemoryProviderManager::new(store);
                    let extract_principal_id = principal_id.clone();
                    tokio::spawn(async move {
                        let harness = crate::agent::AgentRunHarness::new(None);
                        if let Err(err) = harness.append_artifact(&artifact).await {
                            tracing::debug!(error = %err, "Failed to append thread compaction artifact");
                        }
                        manager
                            .session_end_extract(&extract_principal_id, &artifact)
                            .await;
                    });
                }
                let last_user_query = messages
                    .iter()
                    .rev()
                    .find(|message| message.role == crate::llm::Role::User)
                    .map(|message| message.content.as_str());
                let compaction_identity = ResolvedIdentity {
                    principal_id: principal_id.clone(),
                    actor_id: actor_id.clone(),
                    conversation_scope_id,
                    conversation_kind,
                    raw_sender_id: actor_id.clone(),
                    stable_external_conversation_key: String::new(),
                };
                let base_fragment = self
                    .build_post_compaction_context_fragment(
                        last_user_query,
                        Some(&compaction_identity),
                    )
                    .await;
                let fragment =
                    super::input::merge_summary_into_fragment(compaction_summary, base_fragment);
                self.update_post_compaction_context(thread_id, fragment)
                    .await;
                self.record_context_pressure_state(thread_id, usage_after)
                    .await;
                Ok(SubmissionResult::ok_with_message(msg))
            }
            Err(e) => Ok(SubmissionResult::error(format!("Compaction failed: {}", e))),
        }
    }

    pub(in crate::agent) async fn process_clear(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let session_user_id = sess.user_id.clone();
        let session_id = sess.id;
        let principal_id = sess.principal_id.clone();
        let actor_id = sess.actor_id.clone();
        let conversation_scope_id = sess.conversation_scope_id;
        let conversation_kind = sess.conversation_kind.as_str().to_string();
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
        thinclaw_agent::thread_ops::clear_thread(thread);
        let usage_percent = self
            .effective_context_monitor()
            .usage_percent(&thread.messages());
        // /clear empties the in-memory thread, so the watermark drops to 0:
        // hydration must not resurrect the cleared DB rows after a restart.
        let active_message_row_count = thread.messages().len() as i64;
        let mut session_extract_artifact = crate::agent::AgentRunArtifact::new(
            "thread_clear",
            crate::agent::AgentRunStatus::Completed,
            chrono::Utc::now(),
            Some(chrono::Utc::now()),
        )
        .with_runtime_descriptor(Some(&crate::agent::run_artifact::run_runtime_descriptor(
            &interactive_chat_runtime_descriptor(),
        )))
        .with_metadata(serde_json::json!({
            "event": "thread_clear",
            "thread_id": thread_id,
        }));
        session_extract_artifact.session_id = Some(session_id);
        session_extract_artifact.thread_id = Some(thread_id);
        session_extract_artifact.user_id = Some(session_user_id.clone());
        session_extract_artifact.actor_id = Some(actor_id);
        session_extract_artifact.conversation_scope_id = Some(conversation_scope_id);
        session_extract_artifact.conversation_kind = Some(conversation_kind);

        // Clear undo history too
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        {
            let mut mgr = undo_mgr.lock().await;
            mgr.clear();
            self.persist_active_watermark_and_undo_stack(thread_id, active_message_row_count, &mgr)
                .await;
        }
        drop(sess);
        if let Some(store) = self.store().map(Arc::clone) {
            let manager = crate::agent::learning::MemoryProviderManager::new(store);
            tokio::spawn(async move {
                let harness = crate::agent::AgentRunHarness::new(None);
                if let Err(err) = harness.append_artifact(&session_extract_artifact).await {
                    tracing::debug!(error = %err, "Failed to append thread clear artifact");
                }
                manager
                    .session_end_extract(&principal_id, &session_extract_artifact)
                    .await;
            });
        }
        self.clear_thread_runtime_transients(thread_id).await;
        self.record_context_pressure_state(thread_id, usage_percent)
            .await;

        Ok(SubmissionResult::ok_with_message("Thread cleared."))
    }

    pub(in crate::agent) async fn process_new_thread(
        &self,
        message: &IncomingMessage,
    ) -> Result<SubmissionResult, Error> {
        let identity = message.resolved_identity();
        let session = self
            .session_manager
            .get_or_create_session_for_identity(&identity)
            .await;
        let mut sess = session.lock().await;
        let thread = sess.create_thread();
        let thread_id = thread.id;
        Ok(SubmissionResult::ok_with_message(format!(
            "New thread: {}",
            thread_id
        )))
    }

    pub(in crate::agent) async fn process_switch_thread(
        &self,
        message: &IncomingMessage,
        target_thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let identity = message.resolved_identity();
        let session = self
            .session_manager
            .get_or_create_session_for_identity(&identity)
            .await;
        let mut sess = session.lock().await;

        if sess.switch_thread(target_thread_id) {
            Ok(SubmissionResult::ok_with_message(format!(
                "Switched to thread {}",
                target_thread_id
            )))
        } else {
            Ok(SubmissionResult::error("Thread not found."))
        }
    }

    pub(in crate::agent) async fn process_resume(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        checkpoint_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut mgr = undo_mgr.lock().await;

        let outcome = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            let description = thinclaw_agent::thread_ops::restore_thread_from_checkpoint(
                thread,
                &mut mgr,
                checkpoint_id,
            );
            // Row-count watermark for the thread as it stands *after* the
            // checkpoint restore above, so hydration truncates DB history to
            // match what /resume just restored in memory.
            description.map(|description| (description, thread.messages().len() as i64))
        };

        if let Some((description, active_message_row_count)) = outcome {
            self.clear_thread_runtime_transients(thread_id).await;
            self.persist_active_watermark_and_undo_stack(thread_id, active_message_row_count, &mgr)
                .await;
            drop(mgr);
            Ok(SubmissionResult::ok_with_message(format!(
                "Resumed from checkpoint: {}",
                description
            )))
        } else {
            Ok(SubmissionResult::error("Checkpoint not found."))
        }
    }
}
