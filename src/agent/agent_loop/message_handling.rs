//! agent_loop: message-handling methods of `Agent`.

use super::*;

impl Agent {
    /// Resolve and freeze the canonical request identity exactly once, before
    /// hooks, session lookup, prompt assembly, or tool execution can observe
    /// the message. Explicit identities supplied by trusted local/gateway
    /// surfaces win. Native endpoints are bound only through approved, active
    /// actor-registry records; unlinked endpoints retain the collision-safe
    /// channel-namespaced fallback from `thinclaw-identity`.
    pub(super) async fn resolve_ingress_identity(
        &self,
        message: &mut IncomingMessage,
    ) -> Result<(), Error> {
        if message.identity.is_some() {
            return Ok(());
        }

        // Privilege metadata is meaningful only on surfaces that attach an
        // explicit, authenticated identity (gateway/Tauri/TUI). Native channel
        // payloads are untrusted and must not be able to self-assert admin
        // authority through adapter metadata.
        if let Some(metadata) = message.metadata.as_object_mut() {
            metadata.remove("principal_admin");
            metadata.remove("gateway_role");
            metadata.remove("principal_id");
            metadata.remove("actor_id");
            metadata.remove("agent_id");
            metadata.remove("agent_workspace_id");
            metadata.remove("allowed_tools");
            metadata.remove("allowed_skills");
            metadata.remove("tool_profile");
        }

        let fallback_principal =
            crate::identity::external_principal_id(&message.channel, &message.user_id);
        let fallback = crate::identity::ResolvedIdentity::from_message_with_actor(
            message,
            fallback_principal.clone(),
            fallback_principal,
        );
        let Some(store) = self.store() else {
            message.identity = Some(fallback);
            return Ok(());
        };

        match store
            .resolve_actor_for_endpoint(&message.channel, &message.user_id)
            .await
        {
            Ok(Some(actor)) => {
                let identity = crate::identity::ResolvedIdentity::from_message_with_actor(
                    message,
                    actor.principal_id.clone(),
                    actor.actor_id.to_string(),
                );

                if identity.conversation_kind == crate::identity::ConversationKind::Direct {
                    let endpoint = crate::identity::ActorEndpointRef::new(
                        message.channel.clone(),
                        message.user_id.clone(),
                    );
                    if let Err(error) = store
                        .set_actor_last_active_direct_endpoint(actor.actor_id, Some(&endpoint))
                        .await
                    {
                        tracing::warn!(
                            channel = %message.channel,
                            actor_id = %actor.actor_id,
                            error = %error,
                            "Failed to record the actor's last active direct endpoint"
                        );
                    }
                }

                message.identity = Some(identity);
            }
            Ok(None) => {
                message.identity = Some(fallback);
            }
            Err(error) => {
                tracing::error!(
                    channel = %message.channel,
                    error = %error,
                    "Actor endpoint lookup failed; rejecting ingress to avoid identity divergence"
                );
                return Err(error.into());
            }
        }
        Ok(())
    }

    pub(crate) async fn handle_message(
        &self,
        message: &IncomingMessage,
        parsed: Option<Submission>,
    ) -> Result<Option<thinclaw_agent::submission::AgentResponsePayload>, Error> {
        let mut canonical_message = message.clone();
        self.observer()
            .record_event(&crate::observability::ObserverEvent::ChannelMessage {
                channel: message.channel.clone(),
                direction: "inbound".to_string(),
            });

        // Parse submission type first (reusing the dispatch loop's parse
        // when it already did the work for control-command routing).
        let mut submission = parsed.unwrap_or_else(|| SubmissionParser::parse(&message.content));

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
                    let payload = thinclaw_agent::submission::AgentResponsePayload::text(
                        inbound_rejected_response(&reason),
                    );
                    return Ok(self.apply_before_outbound_hook(message, payload).await);
                }
                Err(err) => {
                    let payload = thinclaw_agent::submission::AgentResponsePayload::text(
                        inbound_blocked_response(&err.to_string()),
                    );
                    return Ok(self.apply_before_outbound_hook(message, payload).await);
                }
                Ok(crate::hooks::HookOutcome::Continue {
                    modified: Some(new_content),
                }) => {
                    canonical_message.content = new_content.clone();
                    submission = Submission::UserInput {
                        content: new_content,
                    };
                }
                _ => {} // Continue, fail-open errors already logged in registry
            }
        }

        let message = &canonical_message;

        let identity = message.resolved_identity();

        // Resolve session and thread
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
            && pending.accepts_identity(&identity)
        {
            if submission.consumes_pending_manual_auth() {
                if let Submission::UserInput { content } = &submission {
                    let response = self
                        .process_auth_token(message, &pending, content, session, thread_id)
                        .await?;
                    let Some(content) = response else {
                        return Ok(None);
                    };
                    let mut payload =
                        thinclaw_agent::submission::AgentResponsePayload::text(content);
                    self.transform_response_payload(message, thread_id, &mut payload)
                        .await;
                    return Ok(self.apply_before_outbound_hook(message, payload).await);
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
            Submission::Compact => self.process_compact(message, session, thread_id).await,
            Submission::Clear => self.process_clear(message, session, thread_id).await,
            Submission::NewThread => self.process_new_thread(message, thread_id).await,
            Submission::Heartbeat => self.process_heartbeat(message).await,
            Submission::Summarize => self.process_summarize(message, session, thread_id).await,
            Submission::Suggest => self.process_suggest(message, session, thread_id).await,
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
                self.process_switch_thread(message, thread_id, target).await
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
                // Bare conversational words ("ok", "yes", "no", "cancel")
                // parse as approval responses, but outside an approval prompt
                // they are normal replies — e.g. answering a yes/no question
                // the agent just asked. Explicit slash commands (/approve,
                // /deny, ...) keep approval semantics either way.
                let bare_word = !message.content.trim_start().starts_with('/');
                let awaiting_approval = {
                    let sess = session.lock().await;
                    sess.threads.get(&thread_id).is_some_and(|thread| {
                        thread.state == crate::agent::session::ThreadState::AwaitingApproval
                    })
                };
                if bare_word && !awaiting_approval {
                    self.process_user_input(message, session, thread_id, message.content.trim())
                        .await
                } else {
                    self.process_approval(message, session, thread_id, None, approved, always)
                        .await
                }
            }
        };

        // Convert SubmissionResult to a response payload or root-side status effect.
        match plan_submission_response(result?, crate::llm::is_silent_reply) {
            SubmissionResponsePlan::Respond(mut payload) => {
                if payload.is_empty() {
                    return Ok(Some(payload));
                }
                self.transform_response_payload(message, thread_id, &mut payload)
                    .await;
                Ok(self.apply_before_outbound_hook(message, payload).await)
            }
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

    async fn apply_before_outbound_hook(
        &self,
        message: &IncomingMessage,
        mut payload: thinclaw_agent::submission::AgentResponsePayload,
    ) -> Option<thinclaw_agent::submission::AgentResponsePayload> {
        let event = crate::hooks::HookEvent::Outbound {
            user_id: message.user_id.clone(),
            channel: message.channel.clone(),
            content: payload.content.clone(),
            thread_id: message.thread_id.clone(),
        };
        match self.hooks().run(&event).await {
            Err(err) => {
                tracing::warn!(error = %err, "BeforeOutbound hook blocked response");
                None
            }
            Ok(crate::hooks::HookOutcome::Continue {
                modified: Some(new_content),
            }) => {
                payload.content = new_content;
                Some(payload)
            }
            _ => Some(payload),
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
    ) -> Result<Option<thinclaw_agent::submission::AgentResponsePayload>, Error> {
        self.handle_message_payload_external_parsed(message, None)
            .await
    }

    /// Compatibility adapter for protocols that can only represent text.
    /// New response-capable callers should use [`Self::handle_message_external`]
    /// so generated media attachments are not discarded.
    pub async fn handle_message_text_external(
        &self,
        message: &IncomingMessage,
    ) -> Result<Option<String>, Error> {
        Ok(self
            .handle_message_external(message)
            .await?
            .map(|payload| payload.content))
    }

    /// Like [`Self::handle_message_external`], but accepts an
    /// already-parsed submission so the standalone dispatch loop (which
    /// parses once for control-command routing) doesn't parse every message
    /// twice.
    pub(crate) async fn handle_message_payload_external_parsed(
        &self,
        message: &IncomingMessage,
        parsed: Option<Submission>,
    ) -> Result<Option<thinclaw_agent::submission::AgentResponsePayload>, Error> {
        let run_driver = AgentRunDriver::new();
        let mut canonical_message = message.clone();
        self.resolve_ingress_identity(&mut canonical_message)
            .await?;
        let message = &canonical_message;
        let identity = message.resolved_identity();
        let submission = parsed.unwrap_or_else(|| SubmissionParser::parse(&message.content));
        let is_interrupt = matches!(&submission, Submission::Interrupt);
        let is_approval_response = matches!(
            &submission,
            Submission::ExecApproval { .. } | Submission::ApprovalResponse { .. }
        );
        // Hold admission through the exact post-turn snapshot. Otherwise a
        // queued turn can complete after handle_message returns but before
        // trajectory capture, causing the first caller to record the second
        // caller's turn (and the second turn to be recorded twice).
        let execution_lock = if is_interrupt {
            None
        } else {
            Some(
                self.session_manager
                    .execution_lock_for_identity(&identity)
                    .await,
            )
        };
        let _execution_guard = match execution_lock.as_ref() {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        self.maybe_hydrate_ingress_thread(message).await?;
        let (session, thread_id) = self
            .session_manager
            .resolve_thread_for_identity(&identity, &message.channel, message.thread_id.as_deref())
            .await;
        let trajectory_turn_index = {
            let sess = session.lock().await;
            sess.threads.get(&thread_id).and_then(|thread| {
                if is_approval_response
                    && thread.state == crate::agent::session::ThreadState::AwaitingApproval
                {
                    thread.turns.len().checked_sub(1)
                } else {
                    Some(thread.turns.len())
                }
            })
        };

        let result = self.handle_message(message, Some(submission)).await;

        self.record_trajectory_turn(
            message,
            &run_driver,
            session,
            thread_id,
            trajectory_turn_index,
        )
        .await;

        result
    }

    /// Record the completed turn for trajectory/learning purposes.
    ///
    /// Snapshots the session under the lock, then runs the heavy tail
    /// (artifact JSONL append, provider memory sync, generated-skill review)
    /// in a detached task — none of it needs to block the user's response.
    pub(crate) async fn record_trajectory_turn(
        &self,
        message: &IncomingMessage,
        run_driver: &AgentRunDriver,
        session: Arc<tokio::sync::Mutex<crate::agent::session::Session>>,
        thread_id: Uuid,
        turn_index: Option<usize>,
    ) {
        let Some(turn_index) = turn_index else {
            return;
        };
        let (session_snapshot, thread_snapshot) = {
            let sess = session.lock().await;
            let thread = match sess.threads.get(&thread_id) {
                Some(thread) => thread.clone(),
                None => return,
            };
            (sess.clone(), thread)
        };

        let Some(turn_snapshot) = thread_snapshot.turns.get(turn_index).cloned() else {
            return;
        };
        // Approval chains can return multiple times while this same turn is
        // still live. Persist only its eventual terminal form so trajectory
        // learning sees the real tool outcome instead of an early
        // "awaiting approval" snapshot.
        if turn_snapshot.state == crate::agent::session::TurnState::Processing {
            return;
        }

        let run_driver = run_driver.clone();
        let harness_store = self.store().map(Arc::clone);
        let harness_memory_manager = self
            .learning_orchestrator()
            .map(|orchestrator| orchestrator.memory_provider_manager());
        let agent_name = self.config.name.clone();
        let model_name = self.llm().active_model_name();
        let message = message.clone();
        // Clone the shared orchestrator handle (cheap: a few `Arc` clones) so
        // the detached task reuses the same `MemoryProviderManager` — and
        // therefore the same readiness cache and pooled HTTP client — instead
        // of constructing a fresh one.
        let orchestrator = self.learning_orchestrator().cloned();

        self.spawn_tail_task(async move {
            let harness = crate::agent::AgentRunHarness::with_driver_and_memory_manager(
                run_driver,
                harness_store,
                harness_memory_manager,
            );
            match harness
                .record_chat_turn(
                    &agent_name,
                    &model_name,
                    &session_snapshot,
                    thread_id,
                    &message,
                    &turn_snapshot,
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

            if let Some(orchestrator) = orchestrator
                && let Err(err) = orchestrator
                    .review_completed_turn_for_generated_skill(
                        &session_snapshot,
                        thread_id,
                        &message,
                        &turn_snapshot,
                    )
                    .await
            {
                tracing::debug!(
                    thread = %thread_id,
                    error = %err,
                    "Generated skill reviewer skipped turn"
                );
            }
        })
        .await;
    }

    /// Inject a message into session history without triggering a turn.
    ///
    /// Used for boot sequences, date context injection, silent memory updates,
    /// and any case where the caller wants `deliver=false` semantics.
    /// The message is appended to live state and persisted when a store exists;
    /// no LLM call is made.
    pub async fn inject_context(&self, message: &IncomingMessage) -> Result<(), Error> {
        let mut canonical_message = message.clone();
        self.resolve_ingress_identity(&mut canonical_message)
            .await?;
        canonical_message.metadata = crate::agent::thread_ops::sanitized_injected_context_metadata(
            &canonical_message.metadata,
        );
        let message = &canonical_message;
        let identity = message.resolved_identity();
        let execution_lock = self
            .session_manager
            .execution_lock_for_identity(&identity)
            .await;
        let _execution_guard = execution_lock.lock().await;

        self.maybe_hydrate_ingress_thread(message).await?;
        let (session, thread_id) = self
            .session_manager
            .resolve_thread_for_identity(&identity, &message.channel, message.thread_id.as_deref())
            .await;
        let pre_injection_thread = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            let before = thread.clone();
            thread.inject_context(
                message.content.clone(),
                thinclaw_agent::session::message_hides_user_input_in_main_chat(&message.metadata),
            );
            sess.touch_last_active();
            before
        };
        let persisted_message_id = match self
            .persist_injected_context_message(thread_id, message, &message.content)
            .await
        {
            Ok(message_id) => message_id,
            Err(error) => {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    *thread = pre_injection_thread;
                }
                return Err(error);
            }
        };
        if let Some(message_id) = persisted_message_id {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            let turn = thread.last_turn_mut().ok_or_else(|| {
                Error::from(crate::error::JobError::ContextError {
                    id: thread_id,
                    reason: "Persisted context row has no owning in-memory turn".to_string(),
                })
            })?;
            turn.durable_user_message_id = Some(message_id);
        }
        self.persist_thread_runtime_snapshot(message, &session, thread_id)
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
        let scope_id = identity.conversation_scope_id;
        let (session, thread_id) = self
            .session_manager
            .lookup_thread_for_identity(&identity, channel, Some(session_key))
            .await
            .ok_or_else(|| {
                Error::from(crate::error::JobError::ContextError {
                    id: scope_id,
                    reason: format!("No active conversation for session key '{session_key}'"),
                })
            })?;
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
            conversation_scope_id: crate::identity::direct_scope_id("local_user", "local_user"),
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
