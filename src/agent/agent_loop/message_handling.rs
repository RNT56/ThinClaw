//! agent_loop: message-handling methods of `Agent`.

use super::*;

impl Agent {
    pub(crate) async fn handle_message(
        &self,
        message: &IncomingMessage,
    ) -> Result<Option<thinclaw_agent::submission::AgentResponsePayload>, Error> {
        self.observer()
            .record_event(&crate::observability::ObserverEvent::ChannelMessage {
                channel: message.channel.clone(),
                direction: "inbound".to_string(),
            });

        // Parse submission type first
        let mut submission = SubmissionParser::parse(&message.content);

        // Hook: BeforeInbound — allow hooks to modify or reject user input
        if submission.runs_inbound_hooks()
            && let Submission::UserInput { ref content } = submission
        {
            let event = crate::hooks::HookEvent::Inbound {
                user_id: message.user_id.clone(),
                channel: message.channel.clone(),
                content: content.clone(),
                thread_id: message.thread_id.clone(),
            };
            match self.hooks().run(&event).await {
                Err(crate::hooks::HookError::Rejected { reason }) => {
                    return Ok(Some(
                        thinclaw_agent::submission::AgentResponsePayload::text(
                            inbound_rejected_response(&reason),
                        ),
                    ));
                }
                Err(err) => {
                    return Ok(Some(
                        thinclaw_agent::submission::AgentResponsePayload::text(
                            inbound_blocked_response(&err.to_string()),
                        ),
                    ));
                }
                Ok(crate::hooks::HookOutcome::Continue {
                    modified: Some(new_content),
                }) => {
                    submission = Submission::UserInput {
                        content: new_content,
                    };
                }
                _ => {} // Continue, fail-open errors already logged in registry
            }
        }

        // Hydrate thread from DB if it's a historical thread not in memory
        if let Some(ref external_thread_id) = message.thread_id {
            self.maybe_hydrate_thread(message, external_thread_id).await;
        } else {
            self.maybe_hydrate_primary_direct_thread(message).await;
        }

        // Resolve session and thread
        let identity = message.resolved_identity();
        let (session, thread_id) = self
            .session_manager
            .resolve_thread_for_identity(&identity, &message.channel, message.thread_id.as_deref())
            .await;

        // Multi-agent routing: determine which agent workspace should handle this message.
        // Thread ownership is claimed on first interaction (first-responder wins).
        if let Some(decision) = self
            .agent_router
            .route(&message.channel, Some(thread_id), &message.content)
            .await
        {
            tracing::debug!(
                agent = %decision.agent_id,
                reason = %decision.reason,
                thread = %thread_id,
                "Routed message to agent workspace"
            );
            // Claim thread ownership if not already owned
            let claimed = self
                .agent_router
                .claim_thread(thread_id, &decision.agent_id)
                .await;
            if claimed {
                let _ = self
                    .session_manager
                    .set_thread_owner(thread_id, &decision.agent_id)
                    .await;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
            }
        }

        // Manual auth interception: only manual-token flows consume the next
        // user message as a credential. External OAuth flows remain in the
        // normal pipeline while the browser callback finishes separately.
        let pending_auth = {
            let sess = session.lock().await;
            sess.threads
                .get(&thread_id)
                .and_then(|t| t.pending_auth.clone())
        };

        if let Some(pending) = pending_auth
            && pending.auth_mode == crate::agent::session::PendingAuthMode::ManualToken
        {
            if submission.consumes_pending_manual_auth() {
                if let Submission::UserInput { content } = &submission {
                    return self
                        .process_auth_token(message, &pending, content, session, thread_id)
                        .await
                        .map(|result| {
                            result.map(thinclaw_agent::submission::AgentResponsePayload::text)
                        });
                }
            } else if submission.cancels_pending_manual_auth() {
                // Any non-user-input submission (interrupt, undo, etc.) cancels auth mode.
                let thread_snapshot = {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thread.pending_auth = None;
                        Some(thread.clone())
                    } else {
                        None
                    }
                };
                if let Some(thread_snapshot) = thread_snapshot {
                    let _ = thread_snapshot;
                    self.persist_thread_runtime_snapshot(message, &session, thread_id)
                        .await;
                }
                // Fall through to normal handling.
            }
        }

        tracing::debug!(
            "Received message from {} on {} ({} chars)",
            message.user_id,
            message.channel,
            message.content.len()
        );

        // Process based on submission type
        let result = match submission {
            Submission::UserInput { content } => {
                self.process_user_input(message, session, thread_id, &content)
                    .await
            }
            Submission::SystemCommand { command, args } => {
                self.handle_system_command(message, thread_id, &command, &args)
                    .await
            }
            Submission::Undo => self.process_undo(session, thread_id).await,
            Submission::Redo => self.process_redo(session, thread_id).await,
            Submission::Interrupt => self.process_interrupt(message, session, thread_id).await,
            Submission::Compact => self.process_compact(session, thread_id).await,
            Submission::Clear => self.process_clear(session, thread_id).await,
            Submission::NewThread => self.process_new_thread(message).await,
            Submission::Heartbeat => self.process_heartbeat().await,
            Submission::Summarize => self.process_summarize(session, thread_id).await,
            Submission::Suggest => self.process_suggest(session, thread_id).await,
            Submission::Quit => return Ok(None),
            Submission::Restart => {
                // Notify the user that the agent is restarting, then trigger
                // orderly shutdown. `main` decides whether to hand off to a
                // service manager restart or relaunch the foreground process.
                self.deps
                    .restart_requested
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                let target_channel = self.config.notify_channel.as_deref().unwrap_or("web");
                let restart_msg = OutgoingResponse::text(RESTART_NOTICE_TEXT);
                // Best-effort: send to preferred channel + web
                let _ = self
                    .channels
                    .broadcast(target_channel, &message.user_id, restart_msg.clone())
                    .await;
                if target_channel != "web" {
                    let _ = self
                        .channels
                        .broadcast("web", &message.user_id, restart_msg)
                        .await;
                }
                tracing::info!("Restart requested — performing orderly shutdown");
                return Ok(None);
            }
            Submission::SwitchThread { thread_id: target } => {
                self.process_switch_thread(message, target).await
            }
            Submission::Resume { checkpoint_id } => {
                self.process_resume(session, thread_id, checkpoint_id).await
            }
            Submission::ExecApproval {
                request_id,
                approved,
                always,
            } => {
                self.process_approval(
                    message,
                    session,
                    thread_id,
                    Some(request_id),
                    approved,
                    always,
                )
                .await
            }
            Submission::ApprovalResponse { approved, always } => {
                self.process_approval(message, session, thread_id, None, approved, always)
                    .await
            }
        };

        // Convert SubmissionResult to a response payload or root-side status effect.
        match plan_submission_response(result?, crate::llm::is_silent_reply) {
            SubmissionResponsePlan::Respond(payload) => Ok(Some(payload)),
            SubmissionResponsePlan::Suppress => {
                tracing::debug!("Suppressing silent or empty submission response");
                Ok(None)
            }
            SubmissionResponsePlan::SendApprovalStatusAndSuppress(approval) => {
                // Each channel renders the approval prompt via send_status.
                // Web gateway shows an inline card, REPL prints a formatted prompt, etc.
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::ApprovalNeeded {
                            request_id: approval.request_id.to_string(),
                            tool_name: approval.tool_name,
                            description: approval.description,
                            parameters: approval.parameters,
                        },
                        &message.metadata,
                    )
                    .await;

                // Empty string signals the caller to skip respond() (no duplicate text)
                Ok(Some(
                    thinclaw_agent::submission::AgentResponsePayload::text(""),
                ))
            }
        }
    }

    // ─── Public API for external callers (Tauri, API module) ─────────

    /// Process a message from an external caller (Tauri command, API endpoint).
    ///
    /// This is the public entry point for `handle_message()`, which remains
    /// `pub(super)` for internal use. Delegates directly — same hooks,
    /// safety checks, and session resolution as the internal path.
    pub async fn handle_message_external(
        &self,
        message: &IncomingMessage,
    ) -> Result<Option<String>, Error> {
        Ok(self
            .handle_message_payload_external(message)
            .await?
            .map(|payload| payload.content))
    }

    pub(crate) async fn handle_message_payload_external(
        &self,
        message: &IncomingMessage,
    ) -> Result<Option<thinclaw_agent::submission::AgentResponsePayload>, Error> {
        let run_driver = AgentRunDriver::new();
        if let Some(ref external_thread_id) = message.thread_id {
            self.maybe_hydrate_thread(message, external_thread_id).await;
        } else {
            self.maybe_hydrate_primary_direct_thread(message).await;
        }
        let identity = message.resolved_identity();
        let (session, thread_id) = self
            .session_manager
            .resolve_thread_for_identity(&identity, &message.channel, message.thread_id.as_deref())
            .await;
        let starting_turn_count = {
            let sess = session.lock().await;
            sess.threads
                .get(&thread_id)
                .map(|thread| thread.turns.len())
                .unwrap_or(0)
        };

        let result = self.handle_message(message).await;

        self.record_trajectory_turn(
            message,
            &run_driver,
            session,
            thread_id,
            starting_turn_count,
        )
        .await;

        result
    }

    pub(crate) async fn record_trajectory_turn(
        &self,
        message: &IncomingMessage,
        run_driver: &AgentRunDriver,
        session: Arc<tokio::sync::Mutex<crate::agent::session::Session>>,
        thread_id: Uuid,
        starting_turn_count: usize,
    ) {
        let (session_snapshot, thread_snapshot) = {
            let sess = session.lock().await;
            let thread = match sess.threads.get(&thread_id) {
                Some(thread) => thread.clone(),
                None => return,
            };
            (sess.clone(), thread)
        };

        if thread_snapshot.turns.len() <= starting_turn_count {
            return;
        }

        let Some(turn) = thread_snapshot.turns.last() else {
            return;
        };

        let harness = crate::agent::AgentRunHarness::with_driver(
            run_driver.clone(),
            self.store().map(Arc::clone),
        );
        match harness
            .record_chat_turn(
                &self.config.name,
                &self.llm().active_model_name(),
                &session_snapshot,
                thread_id,
                message,
                turn,
            )
            .await
        {
            Ok(_artifact) => {}
            Err(err) => {
                tracing::debug!(
                    thread = %thread_id,
                    error = %err,
                    "Canonical run artifact logging failed"
                );
            }
        }

        if let Some(store) = self.store().map(Arc::clone) {
            let orchestrator = crate::agent::learning::LearningOrchestrator::new(
                store,
                self.workspace().cloned(),
                self.skill_registry().cloned(),
            );
            if let Err(err) = orchestrator
                .review_completed_turn_for_generated_skill(
                    &session_snapshot,
                    thread_id,
                    message,
                    turn,
                )
                .await
            {
                tracing::debug!(
                    thread = %thread_id,
                    error = %err,
                    "Generated skill reviewer skipped turn"
                );
            }
        }
    }

    /// Inject a message into session history without triggering a turn.
    ///
    /// Used for boot sequences, date context injection, silent memory updates,
    /// and any case where the caller wants `deliver=false` semantics.
    /// The message is persisted to the DB but no LLM call is made.
    pub async fn inject_context(&self, message: &IncomingMessage) -> Result<(), Error> {
        if let Some(ref external_thread_id) = message.thread_id {
            self.maybe_hydrate_thread(message, external_thread_id).await;
        } else {
            self.maybe_hydrate_primary_direct_thread(message).await;
        }
        let identity = message.resolved_identity();
        let (_, thread_id) = self
            .session_manager
            .resolve_thread_for_identity(&identity, &message.channel, message.thread_id.as_deref())
            .await;
        self.persist_user_message(thread_id, message, &message.content)
            .await;
        Ok(())
    }

    /// Cancel a running turn directly — bypasses the full message pipeline.
    ///
    /// Faster than routing `/interrupt` through `handle_message_external()`
    /// because it skips hook chains, submission parsing, and hydration.
    /// Directly locks the session and sets the thread's cancellation flag.
    pub async fn cancel_turn_for_identity(
        &self,
        channel: &str,
        session_key: &str,
        identity: crate::identity::ResolvedIdentity,
        metadata: serde_json::Value,
    ) -> Result<(), Error> {
        let (session, thread_id) = self
            .session_manager
            .resolve_thread_for_identity(&identity, channel, Some(session_key))
            .await;
        let message = crate::channels::IncomingMessage::new(
            channel,
            identity.raw_sender_id.clone(),
            "/interrupt",
        )
        .with_thread(session_key)
        .with_metadata(metadata)
        .with_identity(identity);
        self.process_interrupt(&message, session, thread_id).await?;
        Ok(())
    }

    pub async fn cancel_turn(&self, session_key: &str) -> Result<(), Error> {
        let identity = crate::identity::ResolvedIdentity {
            principal_id: "local_user".to_string(),
            actor_id: "local_user".to_string(),
            conversation_scope_id: crate::identity::scope_id_from_key(&format!(
                "tauri:direct:{session_key}"
            )),
            conversation_kind: crate::identity::ConversationKind::Direct,
            raw_sender_id: "local_user".to_string(),
            stable_external_conversation_key: format!("tauri:direct:{session_key}"),
        };
        self.cancel_turn_for_identity(
            "tauri",
            session_key,
            identity,
            serde_json::json!({"thread_id": session_key}),
        )
        .await
    }
}
