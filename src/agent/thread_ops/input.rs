//! Interactive user-input turn driver.

use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;

use crate::agent::Agent;
use crate::agent::compaction::ContextCompactor;
use crate::agent::dispatcher::AgenticLoopResult;
use crate::agent::learning::{ImprovementClass, LearningEvent, RiskTier};
use crate::agent::session::{Session, ThreadState};
use crate::agent::submission::SubmissionResult;
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::error::Error;
use thinclaw_agent::session::{TurnContextEvidence, bounded_turn_context_evidence};
use thinclaw_agent::thread_ops::ThreadInputAdmission;
use uuid::Uuid;

const MAX_ATTACHMENT_EVIDENCE_CHARS: usize = 64 * 1024;

impl Agent {
    pub(in crate::agent) async fn process_user_input(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        content: &str,
    ) -> Result<SubmissionResult, Error> {
        // ── Media attachment handling ────────────────────────────────
        // Images/audio/video → multimodal: attached to ChatMessage for vision/audio LLMs
        // PDFs/documents/unknown → bounded, typed untrusted evidence. Extracted
        // text must never be concatenated with the user's instruction: doing so
        // silently upgrades document prompt injection to user authority.
        let (multimodal_attachments, text_extract_attachments): (Vec<_>, Vec<_>) =
            message.attachments.iter().cloned().partition(|a| {
                matches!(
                    a.media_type,
                    crate::media::MediaType::Image
                        | crate::media::MediaType::Audio
                        | crate::media::MediaType::Video
                )
            });

        let attachment_evidence = if !text_extract_attachments.is_empty() {
            let pipeline = crate::media::MediaPipeline::new();
            let mut evidence = Vec::new();
            let mut remaining_chars = MAX_ATTACHMENT_EVIDENCE_CHARS;
            for (idx, attachment) in text_extract_attachments.iter().enumerate() {
                if remaining_chars == 0 {
                    tracing::warn!(
                        max_chars = MAX_ATTACHMENT_EVIDENCE_CHARS,
                        skipped_attachments = text_extract_attachments.len().saturating_sub(idx),
                        "Attachment evidence budget exhausted"
                    );
                    break;
                }
                match pipeline.extract(attachment) {
                    Ok(extracted) => {
                        let content = extracted.chars().take(remaining_chars).collect::<String>();
                        let content_chars = content.chars().count();
                        remaining_chars = remaining_chars.saturating_sub(content_chars);
                        if content.trim().is_empty() {
                            continue;
                        }
                        let source = attachment
                            .filename
                            .clone()
                            .or_else(|| attachment.source_url.clone())
                            .unwrap_or_else(|| {
                                format!("attachment {} ({})", idx + 1, attachment.mime_type)
                            });
                        evidence.push(TurnContextEvidence {
                            segment_id: format!("attachment_evidence_{}", idx + 1),
                            source,
                            content,
                        });
                        tracing::debug!(
                            attachment = idx,
                            media_type = %attachment.media_type,
                            size = attachment.size(),
                            extracted_chars = content_chars,
                            "Extracted text from media attachment"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            attachment = idx,
                            media_type = %attachment.media_type,
                            error = %e,
                            "Failed to extract text from media attachment"
                        );
                    }
                }
            }

            bounded_turn_context_evidence(&evidence)
        } else {
            Vec::new()
        };

        if !multimodal_attachments.is_empty() {
            tracing::info!(
                attachment_count = multimodal_attachments.len(),
                total_bytes = multimodal_attachments.iter().map(|a| a.size()).sum::<usize>(),
                types = ?multimodal_attachments.iter().map(|a| a.media_type.to_string()).collect::<Vec<_>>(),
                "Routing media attachments to multimodal LLM processing"
            );
        }

        // First check thread state without holding lock during I/O
        let thread_state = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.state
        };

        match thinclaw_agent::thread_ops::thread_state_input_admission(thread_state) {
            ThreadInputAdmission::Accept => {}
            ThreadInputAdmission::Reject(message) => return Ok(SubmissionResult::error(message)),
        }

        // Safety validation for user input
        let validation = self.safety().validate_input(content);
        if !validation.is_valid {
            let details = validation
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Ok(SubmissionResult::error(format!(
                "Input rejected by safety validation: {}",
                details
            )));
        }

        let violations = self.safety().check_policy(content);
        if violations
            .iter()
            .any(|rule| rule.action == crate::safety::PolicyAction::Block)
        {
            return Ok(SubmissionResult::error("Input rejected by safety policy."));
        }

        // Handle explicit commands (starting with /) directly
        // Everything else goes through the normal agentic loop with tools
        let temp_message = IncomingMessage {
            content: content.to_string(),
            ..message.clone()
        };

        if let Some(intent) = self.router.route_command(&temp_message) {
            // Explicit command like /status, /job, /list - handle directly
            return self.handle_job_or_command(intent, message, thread_id).await;
        }

        // Reset the file checkpoint dedup bucket for this thread's new turn.
        // The turn number is tagged just below (once the session is locked) so
        // filesystem checkpoints align with the conversation undo checkpoint.
        crate::agent::checkpoint::new_turn(thread_id.to_string(), None);
        let resolved_identity = message.resolved_identity();

        // Natural language goes through the agentic loop
        // Job tools (create_job, list_jobs, etc.) are in the tool registry

        // Auto-compact if needed BEFORE adding new turn
        let auto_compaction_plan = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            let messages = thread.messages();
            let monitor = self.effective_context_monitor();
            monitor.suggest_compaction(&messages).map(|strategy| {
                (
                    thread.clone(),
                    thread.updated_at,
                    thread.persisted_message_count() as i64,
                    monitor.usage_percent(&messages),
                    strategy,
                )
            })
        };

        if let Some((
            mut compacted_thread,
            snapshot_updated_at,
            persisted_rows_before,
            pct,
            strategy,
        )) = auto_compaction_plan
        {
            tracing::info!("Context at {:.1}% capacity, auto-compacting", pct);

            if let Some(store) = self.store().map(Arc::clone) {
                let identity = &resolved_identity;
                let event = LearningEvent::new(
                    "thread_ops::pre_compaction_nudge",
                    ImprovementClass::Memory,
                    RiskTier::Low,
                    "Context nearing limit; compaction nudge emitted before turn",
                )
                .with_target("context_compaction")
                .with_metadata(json!({
                    "thread_id": thread_id.to_string(),
                    "channel": message.channel,
                    "usage_percent": pct,
                    "strategy": format!("{:?}", strategy),
                }))
                .into_persisted(
                    identity.principal_id.clone(),
                    Some(identity.actor_id.clone()),
                    Some(message.channel.clone()),
                    Some(thread_id.to_string()),
                    Some(thread_id),
                    None,
                    None,
                );

                if store.insert_learning_event(&event).await.is_ok()
                    && let Some(orchestrator) = self.learning_orchestrator()
                {
                    let _ = orchestrator
                        .handle_event("pre_compaction_memory_nudge", &event)
                        .await;
                }
            }

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status(format!("Context at {:.0}% capacity, compacting...", pct)),
                    &message.metadata,
                )
                .await;

            let mut compactor = ContextCompactor::new(self.llm().clone());
            if let Some(ref tracker) = self.deps.cost_tracker {
                compactor = compactor.with_cost_tracker(std::sync::Arc::clone(tracker));
            }
            let compaction_workspace = self
                .authorized_compaction_workspace(thread_id, &resolved_identity, &message.channel)
                .await;
            match tokio::time::timeout(
                self.config.job_timeout,
                compactor.compact(
                    &mut compacted_thread,
                    strategy,
                    compaction_workspace.as_ref(),
                ),
            )
            .await
            {
                Err(_) => tracing::warn!(
                    timeout_secs = self.config.job_timeout.as_secs(),
                    "Auto-compaction timed out"
                ),
                Ok(Err(e)) => tracing::warn!("Auto-compaction failed: {}", e),
                Ok(Ok(result)) => {
                    let persisted_rows_after = compacted_thread.persisted_message_count() as i64;
                    let base_fragment = self
                        .build_post_compaction_context_fragment(
                            Some(&message.content),
                            Some(&resolved_identity),
                        )
                        .await;
                    let fragment = merge_summary_into_fragment(result.summary, base_fragment);
                    let original_thread = {
                        let mut sess = session.lock().await;
                        let current = sess.threads.get_mut(&thread_id).ok_or_else(|| {
                            Error::from(crate::error::JobError::NotFound { id: thread_id })
                        })?;
                        if current.updated_at != snapshot_updated_at {
                            None
                        } else {
                            let original = current.clone();
                            *current = compacted_thread;
                            Some(original)
                        }
                    };
                    if let Some(original_thread) = original_thread {
                        if let Err(error) = self
                            .advance_active_history_window(
                                thread_id,
                                persisted_rows_before.saturating_sub(persisted_rows_after),
                                persisted_rows_after,
                                fragment,
                            )
                            .await
                        {
                            let mut sess = session.lock().await;
                            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                                thinclaw_agent::thread_ops::restore_thread_after_failed_persistence(
                                    thread,
                                    original_thread,
                                );
                            }
                            tracing::warn!(
                                thread = %thread_id,
                                %error,
                                "Auto-compaction durability commit failed; restored original context"
                            );
                        } else {
                            self.session_manager
                                .get_undo_manager(thread_id)
                                .await
                                .lock()
                                .await
                                .clear();
                        }
                    } else {
                        tracing::warn!(
                            thread = %thread_id,
                            "Discarding stale auto-compaction result"
                        );
                    }
                }
            }
        }

        // Create checkpoint before turn and start the in-memory turn.
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let (mut turn_messages, pre_turn_thread, pre_turn_undo) = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            let mut mgr = undo_mgr.lock().await;
            let pre_turn_thread = thread.clone();
            let pre_turn_undo = mgr.clone();
            // Tag filesystem checkpoints created during this turn with the same
            // turn number the undo checkpoint uses. Capture it before the
            // mutation, but only create the filesystem checkpoint after atomic
            // admission succeeds.
            let checkpoint_turn = thread.turn_number();
            match thinclaw_agent::thread_ops::try_start_user_turn(
                thread,
                &mut mgr,
                content,
                &message.metadata,
            ) {
                Ok(_) => {}
                Err(reason) => return Ok(SubmissionResult::error(reason)),
            }
            if let Some(turn) = thread.last_turn_mut() {
                turn.untrusted_contexts = attachment_evidence.clone();
            }
            crate::agent::checkpoint::new_turn(thread_id.to_string(), Some(checkpoint_turn));
            (thread.messages(), pre_turn_thread, pre_turn_undo)
        };
        let _turn_cancellation_guard = self.begin_turn_cancellation_guard(thread_id).await;

        // Put the current turn's evidence before its instruction at provider
        // transport time. The durable Turn representation keeps evidence
        // attached to the turn, while this ordering makes the user's actual
        // request the most recent user-role message seen by the model.
        if !attachment_evidence.is_empty()
            && let Some(user_index) = turn_messages
                .iter()
                .rposition(|message| message.is_user_instruction())
        {
            let trailing_evidence = turn_messages[user_index + 1..]
                .iter()
                .all(|message| message.untrusted_context_identity().is_some());
            if trailing_evidence {
                let mut evidence_messages = turn_messages.split_off(user_index + 1);
                if let Some(user_message) = turn_messages.pop() {
                    turn_messages.append(&mut evidence_messages);
                    turn_messages.push(user_message);
                }
            }
        }

        // Attach multimodal media to the last user message for LLM processing.
        // The rig adapter converts these to provider-native base64 content blocks.
        if !multimodal_attachments.is_empty()
            && let Some(last_user) = turn_messages.iter_mut().rev().find(|m| {
                m.role == crate::llm::Role::User && m.untrusted_context_identity().is_none()
            })
        {
            last_user.attachments = multimodal_attachments;
        }

        // Persist user message to DB immediately so it survives crashes.
        // Attachment evidence is passed as a typed internal value rather than
        // copied from ingress metadata, so a channel sender cannot forge it.
        let persisted_user_message_id = match self
            .persist_user_message(thread_id, message, content, &attachment_evidence)
            .await
        {
            Ok(message_id) => message_id,
            Err(error) => {
                // Admission is not complete until the durable user row exists. Put
                // both the thread and its undo/redo stacks back exactly as they were
                // before `try_start_user_turn`; otherwise a storage outage leaves an
                // unpersisted failed turn in future prompts and consumes an undo
                // checkpoint. Preserve an interrupt that raced the database call as
                // a visible thread state, but never retain the ephemeral turn.
                let mut sess = session.lock().await;
                let mut mgr = undo_mgr.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    thinclaw_agent::thread_ops::restore_thread_after_failed_persistence(
                        thread,
                        pre_turn_thread,
                    );
                }
                *mgr = pre_turn_undo;
                drop(mgr);
                drop(sess);
                crate::agent::checkpoint::new_turn(thread_id.to_string(), None);
                tracing::error!(
                    thread = %thread_id,
                    %error,
                    "User turn rejected because it could not be persisted"
                );
                return Ok(SubmissionResult::error(
                    "Your message could not be saved, so the turn was not started. Please retry.",
                ));
            }
        };
        if let Some(message_id) = persisted_user_message_id {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            let turn = thread.last_turn_mut().ok_or_else(|| {
                Error::from(crate::error::JobError::ContextError {
                    id: thread_id,
                    reason: "Persisted user row has no owning in-memory turn".to_string(),
                })
            })?;
            turn.durable_user_message_id = Some(message_id);
        }
        self.persist_thread_runtime_snapshot(message, &session, thread_id)
            .await;

        // ── Lifecycle: start ─────────────────────────────────────────
        // Emit immediately — before compaction or LLM call — so the
        // frontend can show a thinking indicator right away.
        let run_id = uuid::Uuid::new_v4().to_string();
        let _ = self
            .channels
            .send_status(
                &message.channel,
                crate::channels::StatusUpdate::LifecycleStart {
                    run_id: run_id.clone(),
                },
                &message.metadata,
            )
            .await;

        // Send thinking status
        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::Thinking("Processing...".into()),
                &message.metadata,
            )
            .await;

        // Run the agentic tool execution loop
        let result = self
            .run_agentic_loop(message, session.clone(), thread_id, turn_messages)
            .await;

        // Response transforms can execute hooks and must not hold the session
        // mutex. Once they finish, the state check and terminal mutation below
        // happen under one short critical section, so an interrupt either wins
        // before finalization or observes the completed turn afterward.
        let result = match result {
            Ok(AgenticLoopResult::Response(mut payload)) => {
                self.transform_response_payload(message, thread_id, &mut payload)
                    .await;
                Ok(AgenticLoopResult::Response(payload))
            }
            Ok(AgenticLoopResult::Streamed(mut payload)) => {
                self.transform_response_payload(message, thread_id, &mut payload)
                    .await;
                Ok(AgenticLoopResult::Streamed(payload))
            }
            other => other,
        };

        let mut sess = session.lock().await;
        let session_id = sess.id;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        if thread.state == ThreadState::Interrupted {
            drop(sess);
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status("Interrupted".into()),
                    &message.metadata,
                )
                .await;
            // Lifecycle end: interrupted
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    crate::channels::StatusUpdate::LifecycleEnd {
                        run_id: run_id.clone(),
                        phase: "interrupted".to_string(),
                    },
                    &message.metadata,
                )
                .await;
            return Ok(SubmissionResult::Interrupted);
        }

        // Complete, fail, or request approval
        let was_streamed = matches!(&result, Ok(AgenticLoopResult::Streamed(_)));
        match result {
            Ok(AgenticLoopResult::Response(payload)) | Ok(AgenticLoopResult::Streamed(payload)) => {
                let (turn_number, messages) =
                    thinclaw_agent::thread_ops::complete_thread_response(thread, &payload.content);
                let tool_calls = thread
                    .last_turn()
                    .map(|turn| turn.tool_calls.clone())
                    .unwrap_or_default();
                let usage_percent = self.effective_context_monitor().usage_percent(&messages);
                drop(sess);

                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Done".into()),
                        &message.metadata,
                    )
                    .await;

                // Persist assistant response (user message already persisted at turn start)
                if let Err(error) = self
                    .persist_assistant_response(
                        thread_id,
                        message,
                        &payload.content,
                        &tool_calls,
                        session_id,
                        turn_number,
                    )
                    .await
                {
                    self.report_response_persistence_failure(message, thread_id, &error)
                        .await;
                }
                self.sync_context_pressure_warning(message, thread_id, usage_percent)
                    .await;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;

                // Lifecycle end: response
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        crate::channels::StatusUpdate::LifecycleEnd {
                            run_id,
                            phase: "response".to_string(),
                        },
                        &message.metadata,
                    )
                    .await;

                if was_streamed {
                    Ok(SubmissionResult::Streamed(payload))
                } else {
                    Ok(SubmissionResult::Response { payload })
                }
            }
            Ok(AgenticLoopResult::NeedApproval { pending }) => {
                // Store pending approval in thread and update state
                let request_id = pending.request_id;
                let tool_name = pending.tool_name.clone();
                let description = pending.description.clone();
                let parameters = pending.parameters.clone();
                let messages = thinclaw_agent::thread_ops::await_thread_approval(thread, pending);
                let usage_percent = self.effective_context_monitor().usage_percent(&messages);
                drop(sess);

                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Awaiting approval".into()),
                        &message.metadata,
                    )
                    .await;
                self.sync_context_pressure_warning(message, thread_id, usage_percent)
                    .await;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
                Ok(SubmissionResult::NeedApproval {
                    request_id,
                    tool_name,
                    description,
                    parameters,
                })
            }
            Err(e) => {
                let messages = thinclaw_agent::thread_ops::fail_thread_turn(thread, &e.to_string());
                let usage_percent = self.effective_context_monitor().usage_percent(&messages);
                drop(sess);

                // User message already persisted at turn start; nothing else to save
                // Lifecycle end: error
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        crate::channels::StatusUpdate::LifecycleEnd {
                            run_id,
                            phase: "error".to_string(),
                        },
                        &message.metadata,
                    )
                    .await;
                self.sync_context_pressure_warning(message, thread_id, usage_percent)
                    .await;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
                Ok(SubmissionResult::error(e.to_string()))
            }
        }
    }

    pub(in crate::agent) async fn transform_response_payload(
        &self,
        message: &IncomingMessage,
        thread_id: Uuid,
        payload: &mut thinclaw_agent::submission::AgentResponsePayload,
    ) {
        if payload.response_transform_applied() {
            return;
        }
        let event = crate::hooks::HookEvent::ResponseTransform {
            user_id: message.user_id.clone(),
            thread_id: thread_id.to_string(),
            response: payload.content.clone(),
        };
        payload.content = match self.hooks().run(&event).await {
            Err(crate::hooks::HookError::Rejected { reason }) => {
                payload.attachments.clear();
                format!("[Response filtered: {}]", reason)
            }
            Err(err) => {
                payload.attachments.clear();
                format!("[Response blocked by hook policy: {}]", err)
            }
            Ok(crate::hooks::HookOutcome::Continue {
                modified: Some(new_response),
            }) => new_response,
            _ => payload.content.clone(),
        };
        payload.mark_response_transform_applied();
    }
}

/// Combine a compaction summary with the rules/facts/skills fragment into the
/// single post-compaction context slot. Either part may be absent.
pub(super) fn merge_summary_into_fragment(
    summary: Option<String>,
    base_fragment: Option<String>,
) -> Option<String> {
    let summary_block = summary
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| format!("## Summary of Earlier Conversation\n\n{s}"));

    match (summary_block, base_fragment) {
        (Some(summary), Some(base)) => Some(format!("{summary}\n\n{base}")),
        (Some(summary), None) => Some(summary),
        (None, base) => base,
    }
}

#[cfg(test)]
mod tests {
    use super::merge_summary_into_fragment;

    #[test]
    fn merges_summary_ahead_of_fragment() {
        let merged = merge_summary_into_fragment(
            Some("did X and Y".to_string()),
            Some("## Rules\n- be nice".to_string()),
        )
        .expect("merged fragment");
        assert!(merged.contains("## Summary of Earlier Conversation"));
        assert!(merged.contains("did X and Y"));
        assert!(merged.contains("## Rules"));
        assert!(merged.find("Summary").unwrap() < merged.find("Rules").unwrap());
    }

    #[test]
    fn summary_only_and_fragment_only_and_empty() {
        assert!(
            merge_summary_into_fragment(Some("s".to_string()), None)
                .unwrap()
                .contains("## Summary of Earlier Conversation")
        );
        assert_eq!(
            merge_summary_into_fragment(None, Some("frag".to_string())).as_deref(),
            Some("frag")
        );
        assert_eq!(
            merge_summary_into_fragment(Some("   ".to_string()), None),
            None
        );
        assert_eq!(merge_summary_into_fragment(None, None), None);
    }
}
