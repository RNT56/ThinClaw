//! Thread lifecycle operations: undo/redo/resume, interrupt, compact, clear,
//! and new/switch thread.

use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::checkpoint;
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
        self.process_undo_or_redo(session, thread_id, UndoRedoAction::Undo)
            .await
    }

    pub(in crate::agent) async fn process_redo(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        self.process_undo_or_redo(session, thread_id, UndoRedoAction::Redo)
            .await
    }

    /// Shared /undo and /redo driver — the two commands differ only in
    /// which restore function runs and which action labels the result.
    async fn process_undo_or_redo(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        action: UndoRedoAction,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        // Lock ordering: session BEFORE undo manager, matching the hot turn
        // path in thread_ops/input.rs. The reverse order here would create an
        // AB-BA deadlock with a concurrent chat turn on the same thread (the
        // Tauri desktop surface dispatches commands and messages without
        // per-thread serialization).
        let mut sess = session.lock().await;
        let mut mgr = undo_mgr.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        let outcome = match action {
            UndoRedoAction::Undo => {
                thinclaw_agent::thread_ops::restore_thread_from_undo(thread, &mut mgr)
            }
            UndoRedoAction::Redo => {
                thinclaw_agent::thread_ops::restore_thread_from_redo(thread, &mut mgr)
            }
        };
        match &outcome {
            UndoRedoOutcome::Restored { .. } => {
                let usage_percent = self
                    .effective_context_monitor()
                    .usage_percent(&thread.messages());
                // Row-count watermark for the thread as it stands *after*
                // the mutation above, so hydration truncates DB history to
                // match what was just restored in memory.
                let active_message_row_count = thread.persisted_message_count() as i64;
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

        match thinclaw_agent::thread_ops::undo_redo_message(action, &outcome) {
            ThreadOperationMessage::Ok(message) => Ok(SubmissionResult::ok_with_message(message)),
            ThreadOperationMessage::Error(message) => Ok(SubmissionResult::error(message)),
        }
    }

    /// `/rewind` — unified conversation + filesystem rewind to an earlier turn.
    ///
    /// With no args or `list`, this is a **dry run**: it prints the available
    /// rewind targets (conversation checkpoints and turn-tagged filesystem
    /// checkpoints) and mutates nothing. With a turn number, it restores the
    /// conversation to the start of that turn (via the undo manager) and, when
    /// filesystem checkpoints are enabled, restores files to the matching
    /// turn-tagged checkpoint.
    pub(in crate::agent) async fn process_rewind(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        args: &[String],
    ) -> Result<SubmissionResult, Error> {
        let sub = args.first().map(|s| s.trim()).unwrap_or("");

        // List / dry-run: no mutation.
        if sub.is_empty() || sub.eq_ignore_ascii_case("list") {
            return Ok(SubmissionResult::response(
                self.rewind_list_text(thread_id).await,
            ));
        }

        let Some(target_turn) = sub.parse::<usize>().ok() else {
            return Ok(SubmissionResult::error(
                "Usage: /rewind <turn-number>  |  /rewind list",
            ));
        };

        // ── Conversation restore (precise, via the undo manager) ──
        // Lock ordering: session before undo manager (see process_undo_or_redo).
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut sess = session.lock().await;
        let mut mgr = undo_mgr.lock().await;
        let restored = {
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            match thinclaw_agent::thread_ops::restore_thread_to_turn(thread, &mut mgr, target_turn)
            {
                Some(info) => {
                    let usage_percent = self
                        .effective_context_monitor()
                        .usage_percent(&thread.messages());
                    let active_message_row_count = thread.persisted_message_count() as i64;
                    Some((info, usage_percent, active_message_row_count))
                }
                None => None,
            }
        };
        // Release the session lock before the async persistence tail.
        drop(sess);
        let restored = match restored {
            Some((info, usage_percent, active_message_row_count)) => {
                self.clear_thread_runtime_transients(thread_id).await;
                self.persist_active_watermark_and_undo_stack(
                    thread_id,
                    active_message_row_count,
                    &mgr,
                )
                .await;
                self.record_context_pressure_state(thread_id, usage_percent)
                    .await;
                Some(info)
            }
            None => None,
        };
        drop(mgr);

        let Some((turn, _description)) = restored else {
            return Ok(SubmissionResult::error(format!(
                "No conversation checkpoint for turn {target_turn}. Run `/rewind list` to see \
                 available rewind points."
            )));
        };

        // ── Filesystem restore (best-effort, turn-tagged) ──
        let file_note = self.rewind_files_to_turn(thread_id, turn).await;

        Ok(SubmissionResult::ok_with_message(format!(
            "Rewound the conversation to the start of turn {turn}.{file_note}"
        )))
    }

    /// Render the `/rewind list` dry-run report.
    async fn rewind_list_text(&self, thread_id: Uuid) -> String {
        let mut out = String::from("Rewind targets\n\n");

        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let turns = undo_mgr.lock().await.checkpoint_turns();
        if turns.is_empty() {
            out.push_str("Conversation: no rewind points yet.\n");
        } else {
            out.push_str("Conversation (turn — snapshot):\n");
            for (turn, description) in &turns {
                out.push_str(&format!("  {turn} — {description}\n"));
            }
        }

        out.push('\n');
        if !self.config.checkpoints_enabled {
            out.push_str("Filesystem checkpoints: disabled in settings.\n");
        } else {
            let fallback_root = self
                .config
                .workspace_root
                .clone()
                .or_else(|| std::env::current_dir().ok());
            match checkpoint::resolve_thread_root(&thread_id.to_string(), fallback_root.as_deref())
            {
                Some(project_root) => match checkpoint::list_checkpoints(&project_root).await {
                    Ok(entries) if !entries.is_empty() => {
                        out.push_str("Filesystem (turn — commit — summary):\n");
                        for entry in entries.iter().take(15) {
                            let turn = entry
                                .turn
                                .map(|t| t.to_string())
                                .unwrap_or_else(|| "—".to_string());
                            let short = &entry.commit_hash[..entry.commit_hash.len().min(8)];
                            out.push_str(&format!("  {turn} — {short} — {}\n", entry.summary));
                        }
                    }
                    _ => out.push_str("Filesystem checkpoints: none yet.\n"),
                },
                None => out.push_str("Filesystem checkpoints: project root unresolved.\n"),
            }
        }

        out.push_str("\nRun `/rewind <turn>` to restore both conversation and files to that turn.");
        out
    }

    /// Restore files to the newest turn-tagged checkpoint at or before `turn`.
    /// Returns a human-readable note to append to the command reply.
    async fn rewind_files_to_turn(&self, thread_id: Uuid, turn: usize) -> String {
        if !self.config.checkpoints_enabled {
            return String::new();
        }
        let fallback_root = self
            .config
            .workspace_root
            .clone()
            .or_else(|| std::env::current_dir().ok());
        let Some(project_root) =
            checkpoint::resolve_thread_root(&thread_id.to_string(), fallback_root.as_deref())
        else {
            return String::new();
        };
        let entries = match checkpoint::list_checkpoints(&project_root).await {
            Ok(entries) => entries,
            Err(_) => return String::new(),
        };
        // Entries are newest-first; the first with a turn tag <= target is the
        // closest file state at or before that turn.
        let Some(entry) = entries.iter().find(|e| e.turn.is_some_and(|t| t <= turn)) else {
            return " No matching filesystem checkpoint for that turn (files unchanged)."
                .to_string();
        };
        let short = &entry.commit_hash[..entry.commit_hash.len().min(8)];
        match checkpoint::restore_with_scope(
            &thread_id.to_string(),
            &project_root,
            &entry.commit_hash,
            None,
        )
        .await
        {
            Ok(()) => format!(
                " Restored files to checkpoint {short} (turn {}).",
                entry.turn.map(|t| t.to_string()).unwrap_or_default()
            ),
            Err(e) => format!(" (file restore failed: {e})"),
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
        let active_message_row_count = thread.persisted_message_count() as i64;
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
        // Lock ordering: session before undo manager (see process_undo_or_redo).
        let mut sess = session.lock().await;
        let mut mgr = undo_mgr.lock().await;

        let outcome = {
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
            description.map(|description| (description, thread.persisted_message_count() as i64))
        };
        // Release the session lock before the async persistence tail.
        drop(sess);

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
