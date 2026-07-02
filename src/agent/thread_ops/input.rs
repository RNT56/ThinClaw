//! Interactive user-input turn driver.

use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;

use crate::agent::Agent;
use crate::agent::compaction::ContextCompactor;
use crate::agent::dispatcher::AgenticLoopResult;
use crate::agent::learning::{ImprovementClass, LearningEvent, LearningOrchestrator, RiskTier};
use crate::agent::session::{Session, ThreadState};
use crate::agent::submission::SubmissionResult;
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::error::Error;
use thinclaw_agent::thread_ops::ThreadInputAdmission;
use uuid::Uuid;

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
        // PDFs/documents/unknown → text extraction: prepended to the user content
        let (multimodal_attachments, text_extract_attachments): (Vec<_>, Vec<_>) =
            message.attachments.iter().cloned().partition(|a| {
                matches!(
                    a.media_type,
                    crate::media::MediaType::Image
                        | crate::media::MediaType::Audio
                        | crate::media::MediaType::Video
                )
            });

        let content = if !text_extract_attachments.is_empty() {
            let pipeline = crate::media::MediaPipeline::new();
            let mut media_context = String::new();
            for (idx, attachment) in text_extract_attachments.iter().enumerate() {
                match pipeline.extract(attachment) {
                    Ok(extracted) => {
                        if !media_context.is_empty() {
                            media_context.push_str("\n\n");
                        }
                        media_context.push_str(&extracted);
                        tracing::debug!(
                            attachment = idx,
                            media_type = %attachment.media_type,
                            size = attachment.size(),
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

            if media_context.is_empty() {
                content.to_string()
            } else {
                format!("{}\n\n{}", media_context, content)
            }
        } else {
            content.to_string()
        };
        let content = content.as_str();

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
        crate::agent::checkpoint::new_turn(thread_id.to_string());
        let resolved_identity = message.resolved_identity();

        // Natural language goes through the agentic loop
        // Job tools (create_job, list_jobs, etc.) are in the tool registry

        // Auto-compact if needed BEFORE adding new turn
        let mut auto_compaction_fragment: Option<Option<String>> = None;
        {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            let messages = thread.messages();
            let monitor = self.effective_context_monitor();
            if let Some(strategy) = monitor.suggest_compaction(&messages) {
                let pct = monitor.usage_percent(&messages);
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

                    if store.insert_learning_event(&event).await.is_ok() {
                        let orchestrator = LearningOrchestrator::new(
                            store,
                            self.workspace().cloned(),
                            self.skill_registry().cloned(),
                        );
                        let _ = orchestrator
                            .handle_event("pre_compaction_memory_nudge", &event)
                            .await;
                    }
                }

                // Notify the user that compaction is happening
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status(format!(
                            "Context at {:.0}% capacity, compacting...",
                            pct
                        )),
                        &message.metadata,
                    )
                    .await;

                let mut compactor = ContextCompactor::new(self.llm().clone());
                if let Some(ref tracker) = self.deps.cost_tracker {
                    compactor = compactor.with_cost_tracker(std::sync::Arc::clone(tracker));
                }
                match compactor
                    .compact(thread, strategy, self.workspace().map(|w| w.as_ref()))
                    .await
                {
                    Err(e) => {
                        tracing::warn!("Auto-compaction failed: {}", e);
                    }
                    Ok(result) => {
                        // Fold the generated summary into the post-compaction
                        // fragment so the model keeps the gist of the dropped
                        // turns. The fragment is persisted into thread runtime
                        // and rehydrated, so the summary also survives restart.
                        let base_fragment = self
                            .build_post_compaction_context_fragment(
                                Some(&message.content),
                                Some(&resolved_identity),
                            )
                            .await;
                        auto_compaction_fragment =
                            Some(merge_summary_into_fragment(result.summary, base_fragment));
                    }
                }
            }
        }
        if let Some(fragment) = auto_compaction_fragment {
            self.update_post_compaction_context(thread_id, fragment)
                .await;
        }

        // Create checkpoint before turn and start the in-memory turn.
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut turn_messages = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            let mut mgr = undo_mgr.lock().await;
            thinclaw_agent::thread_ops::start_user_turn(
                thread,
                &mut mgr,
                content,
                &message.metadata,
            )
        };
        self.begin_turn_cancellation(thread_id).await;

        // Attach multimodal media to the last user message for LLM processing.
        // The rig adapter converts these to provider-native base64 content blocks.
        if !multimodal_attachments.is_empty()
            && let Some(last_user) = turn_messages
                .iter_mut()
                .rev()
                .find(|m| m.role == crate::llm::Role::User)
        {
            last_user.attachments = multimodal_attachments;
        }

        // Persist user message to DB immediately so it survives crashes
        self.persist_user_message(thread_id, message, content).await;
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

        // Re-acquire lock and check if interrupted
        let mut sess = session.lock().await;
        let session_id = sess.id;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        if thread.state == ThreadState::Interrupted {
            self.finish_turn_cancellation(thread_id).await;
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
            Ok(AgenticLoopResult::Response(mut payload))
            | Ok(AgenticLoopResult::Streamed(mut payload)) => {
                // Hook: TransformResponse — allow hooks to modify or reject the final response
                let response = {
                    let event = crate::hooks::HookEvent::ResponseTransform {
                        user_id: message.user_id.clone(),
                        thread_id: thread_id.to_string(),
                        response: payload.content.clone(),
                    };
                    match self.hooks().run(&event).await {
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
                        _ => payload.content.clone(), // fail-open: use original
                    }
                };
                payload.content = response;

                let (turn_number, messages) =
                    thinclaw_agent::thread_ops::complete_thread_response(thread, &payload.content);
                let usage_percent = self.context_monitor.usage_percent(&messages);
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Done".into()),
                        &message.metadata,
                    )
                    .await;

                // Persist assistant response (user message already persisted at turn start)
                self.persist_assistant_response(
                    thread_id,
                    message,
                    &payload.content,
                    session_id,
                    turn_number,
                )
                .await;
                drop(sess);
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
                    self.finish_turn_cancellation(thread_id).await;
                    Ok(SubmissionResult::Streamed(payload))
                } else {
                    self.finish_turn_cancellation(thread_id).await;
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
                let usage_percent = self.context_monitor.usage_percent(&messages);
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Awaiting approval".into()),
                        &message.metadata,
                    )
                    .await;
                drop(sess);
                self.sync_context_pressure_warning(message, thread_id, usage_percent)
                    .await;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
                self.finish_turn_cancellation(thread_id).await;
                Ok(SubmissionResult::NeedApproval {
                    request_id,
                    tool_name,
                    description,
                    parameters,
                })
            }
            Err(e) => {
                let messages = thinclaw_agent::thread_ops::fail_thread_turn(thread, &e.to_string());
                let usage_percent = self.context_monitor.usage_percent(&messages);
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
                drop(sess);
                self.sync_context_pressure_warning(message, thread_id, usage_percent)
                    .await;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
                self.finish_turn_cancellation(thread_id).await;
                Ok(SubmissionResult::error(e.to_string()))
            }
        }
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
