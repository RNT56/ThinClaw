use super::*;
impl Agent {
    /// Run the agentic loop: call LLM, execute tools, repeat until text response.
    ///
    /// Returns `AgenticLoopResult::Response` on completion, or
    /// `AgenticLoopResult::NeedApproval` if a tool requires user approval.
    ///
    pub(crate) async fn run_agentic_loop(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        initial_messages: Vec<ChatMessage>,
    ) -> Result<AgenticLoopResult, Error> {
        let PreparedPromptContext {
            identity,
            routed_agent,
            routed_agent_workspace_id,
            routed_allowed_tools,
            routed_allowed_skills,
            active_skills,
            provider_tool_extensions,
            mut reasoning,
            prompt_context_documents,
        } = self
            .prepare_prompt_context(message, session.clone(), thread_id)
            .await;

        // Build context with messages that we'll mutate during the loop
        let mut context_messages = initial_messages;
        let mut generated_attachments = Vec::new();

        let tool_policies = crate::tools::policy::ToolPolicyManager::load_from_settings();

        // Create a JobContext for tool execution (chat doesn't have a real job)
        let mut job_ctx = JobContext::with_identity(
            identity.principal_id.clone(),
            identity.actor_id.clone(),
            "chat",
            "Interactive chat session",
        );
        job_ctx.metadata = message.metadata.clone();
        if !job_ctx.metadata.is_object() {
            job_ctx.metadata = serde_json::json!({});
        }
        if let Some(metadata) = job_ctx.metadata.as_object_mut() {
            metadata.insert(
                "channel".to_string(),
                serde_json::json!(message.channel.clone()),
            );
            metadata.insert(
                "thread_id".to_string(),
                serde_json::json!(thread_id.to_string()),
            );
            metadata.insert(
                "conversation_kind".to_string(),
                serde_json::json!(identity.conversation_kind.as_str()),
            );
            metadata.insert(
                "conversation_scope_id".to_string(),
                serde_json::json!(identity.conversation_scope_id.to_string()),
            );
            metadata.insert(
                "principal_id".to_string(),
                serde_json::json!(identity.principal_id.clone()),
            );
            metadata.insert(
                "actor_id".to_string(),
                serde_json::json!(identity.actor_id.clone()),
            );
            if let Some(agent) = routed_agent.as_ref() {
                metadata.insert(
                    "agent_id".to_string(),
                    serde_json::json!(agent.agent_id.clone()),
                );
                if let Some(workspace_id) = agent.workspace_id {
                    metadata.insert(
                        "agent_workspace_id".to_string(),
                        serde_json::json!(workspace_id.to_string()),
                    );
                }
                if let Some(allowed_tools) = agent.allowed_tools.as_ref() {
                    metadata.insert(
                        "allowed_tools".to_string(),
                        serde_json::json!(allowed_tools),
                    );
                }
                if let Some(allowed_skills) = agent.allowed_skills.as_ref() {
                    metadata.insert(
                        "allowed_skills".to_string(),
                        serde_json::json!(allowed_skills),
                    );
                }
                if let Some(tool_profile) = agent.tool_profile {
                    metadata.insert(
                        "tool_profile".to_string(),
                        serde_json::json!(tool_profile.as_str()),
                    );
                }
            }
        }
        let _sandbox_child_guard = self
            .deps
            .sandbox_children
            .as_ref()
            .map(|registry| registry.guard(job_ctx.job_id));
        let model_override_scope_key =
            crate::tools::builtin::llm_tools::model_override_scope_key_from_metadata(
                &job_ctx.metadata,
                Some(identity.principal_id.as_str()),
                Some(identity.actor_id.as_str()),
            );

        let max_tool_iterations = self.config.max_tool_iterations;
        // Force a text-only response on the last iteration to guarantee termination
        // instead of hard-erroring. The penultimate iteration also gets a nudge
        // message so the LLM knows it should wrap up.
        let force_text_at = max_tool_iterations;
        let nudge_at = max_tool_iterations.saturating_sub(1);
        let mut iteration = 0;

        // Stuck loop detection: track consecutive identical tool calls.
        // If the LLM calls the same tool with the same arguments repeatedly,
        // it's stuck. After STUCK_WARN_THRESHOLD consecutive identical calls we
        // inject a system nudge; after STUCK_FORCE_THRESHOLD we force text-only.
        const STUCK_WARN_THRESHOLD: u32 = 3;
        const STUCK_FORCE_THRESHOLD: u32 = 5;
        let mut last_call_signature: Option<u64> = None;
        let mut consecutive_same_calls: u32 = 0;
        // Track whether we've already fired the pre-compaction memory flush this cycle.
        // Reset to false each time the hard history cap fires (a new compaction cycle begins).
        let mut memory_flush_fired = false;
        // Track the last applied model override to avoid recreating the provider each iteration.
        let mut last_applied_model_override: Option<String> = None;
        // Store the original LLM so we can restore it when the override is reset.
        let original_llm = reasoning.current_llm();

        // ── Persistent draft state for streaming ────────────────────
        // Lives outside the loop so a streamed message survives across
        // tool-call iterations. This prevents creating a new Telegram
        // message on each loop pass and ensures the ✦ indicator is
        // properly cleaned up when tool calls interrupt streaming.
        let persistent_draft: Arc<tokio::sync::Mutex<Option<crate::channels::DraftReplyState>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let advisor_call_budget = Arc::new(crate::tools::builtin::advisor::AdvisorCallBudget::new(
            self.deps
                .llm_runtime
                .as_ref()
                .map(|runtime| runtime.status().advisor_max_calls)
                .unwrap_or(3),
        ));
        let mut advisor_state = AdvisorTurnState::default();

        loop {
            iteration += 1;
            // Hard ceiling one past the forced-text iteration (should never be reached
            // since force_text_at guarantees a text response, but kept as a safety net).
            if iteration > max_tool_iterations + 1 {
                return Err(crate::error::LlmError::InvalidResponse {
                    provider: "agent".to_string(),
                    reason: format!("Exceeded maximum tool iterations ({max_tool_iterations})"),
                }
                .into());
            }

            // ── Agent-driven model override (llm_select tool) ────────────
            // If the agent called `llm_select` on a previous iteration, the
            // shared model_override will be Some. Wrap the current routed LLM
            // so subsequent calls carry a per-request model override that the
            // live runtime can resolve across catalog and non-catalog backends.
            if let Some(ref override_lock) = self.deps.model_override {
                let current_override = override_lock.get(&model_override_scope_key).await;
                let current_spec = current_override.as_ref().map(|mo| mo.model_spec.clone());
                if current_spec != last_applied_model_override {
                    if let Some(ref mo) = current_override {
                        let new_model = &mo.model_spec;
                        let provider_slug = new_model
                            .split_once('/')
                            .map(|(provider, _)| provider)
                            .unwrap_or("");
                        if crate::tools::builtin::llm_tools::is_runtime_supported_provider_slug(
                            provider_slug,
                        ) {
                            tracing::info!(
                                to = %new_model,
                                reason = mo.reason.as_deref().unwrap_or("agent decision"),
                                "Agent-driven model switch via llm_select"
                            );
                            reasoning.swap_llm(
                                crate::tools::builtin::llm_tools::wrap_model_spec_override(
                                    original_llm.clone(),
                                    new_model.clone(),
                                ),
                            );
                            last_applied_model_override = current_spec;
                        } else {
                            tracing::warn!(
                                model = %new_model,
                                "Failed to apply agent model override because the provider slug is unsupported"
                            );
                            override_lock.clear(&model_override_scope_key).await;
                            reasoning.swap_llm(original_llm.clone());
                            context_messages.push(ChatMessage::system(format!(
                                "Runtime note: requested model override '{}' could not be activated and was cleared because the provider slug is unsupported.",
                                new_model
                            )));
                            last_applied_model_override = None;
                        }
                    } else {
                        // Override was reset — swap back to the original provider.
                        tracing::info!("Agent model override reset — restoring primary");
                        reasoning.swap_llm(original_llm.clone());
                        last_applied_model_override = None;
                    }
                }
            }

            // Interrupts are checked at iteration boundaries and the active
            // provider/tool awaits also subscribe to the per-turn cancellation
            // signal. This block preserves partial progress when cancellation
            // is observed between steps.
            {
                let sess = session.lock().await;
                if let Some(thread) = sess.threads.get(&thread_id)
                    && thread.state == ThreadState::Interrupted
                {
                    // Extract the last assistant or tool result content from
                    // context_messages so the user sees partial progress.
                    let partial_output = context_messages
                        .iter()
                        .rev()
                        .filter_map(|m| match m.role {
                            crate::llm::Role::Assistant if !m.content.is_empty() => {
                                Some(m.content.clone())
                            }
                            crate::llm::Role::Tool if !m.content.is_empty() => {
                                let tool_name = m.name.as_deref().unwrap_or("tool");
                                let safe_end = crate::util::floor_char_boundary(&m.content, 500);
                                Some(format!("[{}: {}]", tool_name, &m.content[..safe_end]))
                            }
                            _ => None,
                        })
                        .take(3) // At most last 3 tool/assistant results
                        .collect::<Vec<_>>();

                    if partial_output.is_empty() {
                        return Err(crate::error::JobError::ContextError {
                            id: thread_id,
                            reason: "Interrupted".to_string(),
                        }
                        .into());
                    }

                    // Return the partial output as a response
                    let mut parts = partial_output;
                    parts.reverse(); // chronological order
                    let partial = format!(
                        "[Interrupted after {} iteration(s)]\n\n{}",
                        iteration - 1,
                        parts.join("\n\n")
                    );
                    return Ok(AgenticLoopResult::Response(
                        thinclaw_agent::submission::AgentResponsePayload::text(partial),
                    )
                    .with_generated_attachments(&generated_attachments));
                }
            }

            // Enforce cost guardrails before the LLM call
            if let Err(limit) = self.cost_guard().check_allowed().await {
                return Err(crate::error::LlmError::InvalidResponse {
                    provider: "agent".to_string(),
                    reason: limit.to_string(),
                }
                .into());
            }

            // ── Pre-compaction memory flush ──────────────────────────────
            // When the conversation crosses 80% of the hard history cap,
            // fire a silent agentic turn to prompt the agent to write any
            // durable memories BEFORE old messages get dropped by the cap.
            // This matches openclaw's `memoryFlush` pre-compaction ping.
            // The user never sees the response; NO_REPLY means nothing to save.
            {
                let max_ctx = self.config.max_context_messages;
                let flush_threshold = (max_ctx as f32 * 0.80) as usize;
                if !memory_flush_fired && context_messages.len() >= flush_threshold {
                    memory_flush_fired = true;
                    tracing::info!(
                        messages = context_messages.len(),
                        threshold = flush_threshold,
                        "Pre-compaction memory flush triggered"
                    );

                    // Build a minimal context for the flush turn (system + flush prompt).
                    let today = chrono::Utc::now().format("%Y-%m-%d");
                    let flush_system = ChatMessage::system(
                        "Session nearing memory compaction. Store durable memories now.",
                    );
                    let flush_user = ChatMessage::user(format!(
                        "Write any lasting notes to daily/{today}.md via memory_write \
                         (target: \"daily_log\"). If nothing important to save, reply with only: NO_REPLY"
                    ));

                    let mut flush_msgs = context_messages.clone();
                    flush_msgs.push(flush_system);
                    flush_msgs.push(flush_user);

                    let flush_ctx = ReasoningContext::new()
                        .with_messages(flush_msgs)
                        .with_tools(
                            self.tools()
                                .tool_definitions_for_capabilities(None, None, None)
                                .await,
                        );

                    let flush_result = tokio::select! {
                        biased;
                        _ = self.wait_for_turn_cancellation(thread_id) => {
                            return Err(Self::turn_interrupted_error(thread_id));
                        }
                        result = reasoning.respond_with_tools(&flush_ctx) => result
                    };

                    match flush_result {
                        Ok(flush_out) => {
                            match flush_out.result {
                                crate::llm::RespondResult::Text(t) => {
                                    let reply_text = t.trim().to_uppercase();
                                    if reply_text.starts_with("NO_REPLY") || reply_text.is_empty() {
                                        tracing::debug!(
                                            "Memory flush: agent replied NO_REPLY, nothing to save"
                                        );
                                    } else {
                                        tracing::debug!(
                                            chars = reply_text.len(),
                                            "Memory flush: agent responded with text (no tool calls)"
                                        );
                                    }
                                }
                                crate::llm::RespondResult::ToolCalls { tool_calls, .. } => {
                                    // Agent wants to write memories — actually execute the tool calls!
                                    // Only allow memory_write and memory_read tools in the flush context
                                    // to prevent side effects.
                                    let allowed_flush_tools =
                                        ["memory_write", "memory_read", "memory_tree"];
                                    for tc in &tool_calls {
                                        if !allowed_flush_tools.contains(&tc.name.as_str()) {
                                            tracing::debug!(
                                                tool = %tc.name,
                                                "Memory flush: skipping non-memory tool call"
                                            );
                                            continue;
                                        }
                                        match self
                                            .execute_chat_tool(&tc.name, &tc.arguments, &job_ctx)
                                            .await
                                        {
                                            Ok(output) => {
                                                tracing::info!(
                                                    tool = %tc.name,
                                                    output_len = output.content.len(),
                                                    "Memory flush: executed {} successfully",
                                                    tc.name
                                                );
                                            }
                                            Err(e) => {
                                                tracing::warn!(
                                                    tool = %tc.name,
                                                    error = %e,
                                                    "Memory flush: tool execution failed (non-fatal)"
                                                );
                                            }
                                        }
                                    }
                                    tracing::info!(
                                        tool_count = tool_calls.len(),
                                        "Memory flush: executed memory tool calls"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            // Non-fatal: log and continue. The main loop is unaffected.
                            tracing::warn!(error = %e, "Memory flush turn failed (non-fatal)");
                        }
                    }
                }
            }

            // ── Canvas action drain ────────────────────────────────────
            // Drain any pending user interactions from canvas panels
            // (button clicks, form submissions) and inject them as
            // context messages so the LLM can respond to UI actions.
            if let Some(ref store) = self.deps.canvas_store {
                let actions = store.drain_actions().await;
                for action in actions {
                    let values_json =
                        serde_json::to_string(&action.values).unwrap_or_else(|_| "{}".to_string());
                    let msg = format!(
                        "[Canvas Interaction] The user interacted with canvas panel \"{}\": \
                         action=\"{}\", values={}",
                        action.panel_id, action.action, values_json
                    );
                    tracing::info!(
                        panel_id = %action.panel_id,
                        action = %action.action,
                        "Injecting canvas action into context"
                    );
                    context_messages.push(ChatMessage::system(&msg));
                }
            }

            // Inject a nudge message when approaching the iteration limit so the
            // LLM is aware it should produce a final answer on the next turn.
            if iteration == nudge_at {
                context_messages.push(ChatMessage::system(
                    "You are approaching the tool call limit. \
                     Provide your best final answer on the next response \
                     using the information you have gathered so far. \
                     Do not call any more tools.",
                ));
            }

            let force_text = iteration >= force_text_at;

            // ── Hard chat history cap ───────────────────────────────────
            // Enforce max_context_messages to prevent OOM on very long
            // conversations. System messages are always kept; oldest
            // non-system messages are dropped first.
            let max_ctx = self.config.max_context_messages;
            if context_messages.len() > max_ctx {
                let (systems, rest): (Vec<ChatMessage>, Vec<ChatMessage>) = context_messages
                    .drain(..)
                    .partition(|m| m.role == crate::llm::Role::System);
                let keep_count = max_ctx.saturating_sub(systems.len());
                let skip = rest.len().saturating_sub(keep_count);
                tracing::info!(
                    total = systems.len() + rest.len(),
                    dropped = skip,
                    "Chat history cap applied (max_context_messages={})",
                    max_ctx
                );
                context_messages = systems;
                context_messages.extend(rest.into_iter().skip(skip));

                // Re-sanitize immediately after cap truncation: the cap may have
                // dropped an assistant(tool_calls) message while keeping its
                // downstream Tool result messages, creating orphaned tool roles.
                // Running sanitize_tool_messages here converts them to user messages
                // before the LLM call, preventing HTTP 400 errors.
                crate::llm::sanitize_tool_messages(&mut context_messages);

                // Bug 9 fix (revised): reset the flush flag AFTER the hard cap
                // actually drops messages so a new compaction cycle can trigger
                // a fresh flush. Previously the reset fired in the same scope
                // as the flush trigger (before truncation), which allowed a
                // double-flush when messages jumped from the 80% threshold
                // past max_ctx in a single iteration.
                if skip > 0 {
                    memory_flush_fired = false;
                }
            }
            // ── Tool-result pruning ─────────────────────────────────────
            // Strip old tool results from context before the LLM call.
            // Matches openclaw's pre-call trimming: only the most recent
            // TOOL_RESULT_KEEP_TURNS turns' tool results are kept.
            // This does NOT modify JSONL/DB history — only the in-memory slice
            // sent to the LLM, preventing token burn over long sessions.
            const TOOL_RESULT_KEEP_TURNS: usize = 3;
            {
                // Count distinct "assistant turn boundaries" (assistant messages
                // mark the start of a new reasoning turn).
                let mut turns_from_end = 0usize;
                let mut prune_before_idx = 0usize;
                for (i, msg) in context_messages.iter().enumerate().rev() {
                    if msg.role == crate::llm::Role::Assistant {
                        turns_from_end += 1;
                        if turns_from_end > TOOL_RESULT_KEEP_TURNS {
                            prune_before_idx = i + 1;
                            break;
                        }
                    }
                }
                if prune_before_idx > 0 {
                    let pruned: usize = context_messages[..prune_before_idx]
                        .iter()
                        .filter(|m| m.role == crate::llm::Role::Tool)
                        .count();
                    if pruned > 0 {
                        tracing::debug!(
                            pruned_tool_results = pruned,
                            "Pruning old tool results from context (keeping last {} turns)",
                            TOOL_RESULT_KEEP_TURNS
                        );
                        // Replace tool results in the old turns with a compact stub
                        for msg in context_messages[..prune_before_idx].iter_mut() {
                            if msg.role == crate::llm::Role::Tool {
                                msg.content =
                                    "[tool result pruned — see session history]".to_string();
                            }
                        }
                    }
                }
            }

            // Refresh tool definitions each iteration so newly built tools become visible
            let tool_defs = self
                .tools()
                .tool_definitions_for_capabilities(
                    routed_allowed_tools.as_deref(),
                    routed_allowed_skills.as_deref(),
                    Some(&provider_tool_extensions),
                )
                .await;
            let tool_defs =
                tool_policies.filter_tool_definitions_for_metadata(tool_defs, &job_ctx.metadata);

            // Apply trust-based tool attenuation if skills are active.
            let tool_defs = if !active_skills.is_empty() {
                let result = crate::skills::attenuate_tools(&tool_defs, &active_skills);
                tracing::info!(
                    min_trust = %result.min_trust,
                    tools_available = result.tools.len(),
                    tools_removed = result.removed_tools.len(),
                    removed = ?result.removed_tools,
                    explanation = %result.explanation,
                    "Tool attenuation applied"
                );
                result.tools
            } else {
                tool_defs
            };
            let tool_defs = self
                .tools()
                .filter_tool_definitions_for_execution_profile(
                    tool_defs,
                    ToolExecutionLane::Chat,
                    job_ctx
                        .metadata
                        .get("tool_profile")
                        .and_then(|value| value.as_str())
                        .and_then(|value| value.parse::<crate::tools::ToolProfile>().ok())
                        .unwrap_or(self.config.main_tool_profile),
                    &job_ctx.metadata,
                )
                .await;
            let tool_defs = if let Some(runtime) = self.deps.llm_runtime.as_ref() {
                if runtime
                    .advisor_config_for_messages(&context_messages)
                    .is_none()
                {
                    tool_defs
                        .into_iter()
                        .filter(|tool| {
                            tool.name != crate::tools::builtin::advisor::ADVISOR_TOOL_NAME
                        })
                        .collect()
                } else {
                    tool_defs
                }
            } else {
                tool_defs
            };

            if force_text {
                tracing::info!(
                    iteration,
                    "Forcing text-only response (iteration limit reached)"
                );
            }

            let runtime_status = self
                .deps
                .llm_runtime
                .as_ref()
                .map(|runtime| runtime.status());
            let advisor_ready_for_turn = self
                .deps
                .llm_runtime
                .as_ref()
                .and_then(|runtime| runtime.advisor_config_for_messages(&context_messages))
                .is_some();
            if advisor_ready_for_turn
                && let Some((trigger, checkpoint, blocked_signature)) = self
                    .next_auto_advisor_trigger(
                        runtime_status.as_ref(),
                        &context_messages,
                        &advisor_state,
                        consecutive_same_calls,
                        last_call_signature,
                    )
            {
                self.inject_auto_advisor_consultation(
                    trigger,
                    checkpoint,
                    blocked_signature,
                    &mut advisor_state,
                    &mut context_messages,
                    &session,
                    thread_id,
                    message,
                    advisor_call_budget.as_ref(),
                    &mut last_call_signature,
                    &mut consecutive_same_calls,
                )
                .await?;
                continue;
            }
            let use_tool_phase_synthesis = tool_phase_synthesis_enabled(
                runtime_status.as_ref(),
                self.deps.cheap_llm.is_some(),
                force_text,
                !tool_defs.is_empty(),
                last_applied_model_override.is_some(),
            );
            let hold_complex_final_pass = advisor_ready_for_turn
                && should_hold_complex_final_pass(
                    runtime_status.as_ref(),
                    &context_messages,
                    &advisor_state,
                );
            let tool_phase_primary_thinking_enabled = runtime_status
                .as_ref()
                .map(|status| status.tool_phase_primary_thinking_enabled)
                .unwrap_or(true);

            let phase_one_model_name = reasoning.current_llm().active_model_name();
            let phase_one_turn = self
                .execute_llm_turn(
                    &mut reasoning,
                    &mut context_messages,
                    tool_defs,
                    thread_id,
                    message,
                    &persistent_draft,
                    &original_llm,
                    &mut last_applied_model_override,
                    LlmTurnOptions {
                        force_text,
                        thinking: if !use_tool_phase_synthesis
                            || tool_phase_primary_thinking_enabled
                        {
                            self.thinking_config_for_model(&phase_one_model_name)
                        } else {
                            crate::llm::ThinkingConfig::Disabled
                        },
                        context_documents: prompt_context_documents.clone(),
                        stream_to_user: !use_tool_phase_synthesis && !hold_complex_final_pass,
                        emit_progress_status: !use_tool_phase_synthesis,
                        emit_thinking_status: !use_tool_phase_synthesis,
                        planning_mode: use_tool_phase_synthesis,
                        max_output_tokens: use_tool_phase_synthesis
                            .then_some(TOOL_PHASE_PLANNING_MAX_TOKENS),
                    },
                )
                .await?;

            let phase_one_finish_reason = phase_one_turn.output.finish_reason;
            let phase_one_streamed_text = phase_one_turn.streamed_text;

            match phase_one_turn.output.result {
                RespondResult::Text(text) => {
                    if use_tool_phase_synthesis {
                        match classify_tool_phase_text(&text, phase_one_finish_reason) {
                            ToolPhaseTextOutcome::NoToolsSignal => {
                                let Some(cheap_llm) = self.deps.cheap_llm.clone() else {
                                    tracing::warn!(
                                        "Tool-phase synthesis was enabled without a cheap_llm handle; returning primary response"
                                    );
                                    return Ok(AgenticLoopResult::Response(
                                        thinclaw_agent::submission::AgentResponsePayload::text(
                                            text,
                                        ),
                                    )
                                    .with_generated_attachments(&generated_attachments));
                                };

                                let cheap_model_name = cheap_llm.active_model_name();
                                let mut synthesis_reasoning =
                                    reasoning.fork_with_llm(cheap_llm.clone());
                                let mut synthesis_messages = context_messages.clone();
                                synthesis_messages
                                    .push(ChatMessage::system(TOOL_PHASE_SYNTHESIS_PROMPT));

                                let synthesis_turn = self
                                    .execute_llm_turn(
                                        &mut synthesis_reasoning,
                                        &mut synthesis_messages,
                                        Vec::new(),
                                        thread_id,
                                        message,
                                        &persistent_draft,
                                        &cheap_llm,
                                        &mut last_applied_model_override,
                                        LlmTurnOptions {
                                            force_text: true,
                                            thinking: self
                                                .thinking_config_for_model(&cheap_model_name),
                                            context_documents: prompt_context_documents.clone(),
                                            stream_to_user: true,
                                            emit_progress_status: true,
                                            emit_thinking_status: true,
                                            planning_mode: false,
                                            max_output_tokens: None,
                                        },
                                    )
                                    .await?;
                                let synthesis_streamed_text = synthesis_turn.streamed_text;
                                let synthesis_finish_reason = synthesis_turn.output.finish_reason;

                                match synthesis_turn.output.result {
                                    RespondResult::Text(synthesized)
                                        if synthesis_finish_reason
                                            == crate::llm::FinishReason::Stop =>
                                    {
                                        return Ok(self
                                            .agentic_result_from_text(
                                                synthesis_streamed_text,
                                                synthesized,
                                            )
                                            .with_generated_attachments(&generated_attachments));
                                    }
                                    RespondResult::Text(text) => {
                                        tracing::warn!(
                                            finish_reason = ?synthesis_finish_reason,
                                            text_len = text.len(),
                                            "Tool-phase synthesis produced non-final text"
                                        );
                                        return Ok(AgenticLoopResult::Response(
                                            thinclaw_agent::submission::AgentResponsePayload::text(
                                                TOOL_PHASE_FINALIZATION_FAILURE_RESPONSE,
                                            ),
                                        )
                                        .with_generated_attachments(&generated_attachments));
                                    }
                                    RespondResult::ToolCalls { .. } => {
                                        tracing::warn!(
                                            "Tool-phase synthesis unexpectedly returned tool calls"
                                        );
                                        return Ok(AgenticLoopResult::Response(
                                            thinclaw_agent::submission::AgentResponsePayload::text(
                                                TOOL_PHASE_FINALIZATION_FAILURE_RESPONSE,
                                            ),
                                        )
                                        .with_generated_attachments(&generated_attachments));
                                    }
                                }
                            }
                            ToolPhaseTextOutcome::PrimaryFinalText => {
                                return Ok(self
                                    .agentic_result_from_text(phase_one_streamed_text, text)
                                    .with_generated_attachments(&generated_attachments));
                            }
                            ToolPhaseTextOutcome::PrimaryNeedsFinalization => {
                                return Ok(self
                                    .finalize_primary_text_only(
                                        &mut reasoning,
                                        &mut context_messages,
                                        &prompt_context_documents,
                                        thread_id,
                                        message,
                                        &persistent_draft,
                                        &original_llm,
                                        &mut last_applied_model_override,
                                        TOOL_PHASE_FINALIZATION_FAILURE_RESPONSE,
                                    )
                                    .await?
                                    .with_generated_attachments(&generated_attachments));
                            }
                        }
                    }

                    if hold_complex_final_pass {
                        let checkpoint = advisor_state
                            .checkpoint_for(AdvisorAutoTrigger::ComplexFinalPass, "final_answer");
                        self.inject_auto_advisor_consultation(
                            AdvisorAutoTrigger::ComplexFinalPass,
                            checkpoint,
                            last_call_signature,
                            &mut advisor_state,
                            &mut context_messages,
                            &session,
                            thread_id,
                            message,
                            advisor_call_budget.as_ref(),
                            &mut last_call_signature,
                            &mut consecutive_same_calls,
                        )
                        .await?;
                        continue;
                    }

                    return Ok(self
                        .agentic_result_from_text(phase_one_streamed_text, text)
                        .with_generated_attachments(&generated_attachments));
                }
                RespondResult::ToolCalls {
                    tool_calls,
                    content,
                } => {
                    let sig = tool_call_signature(&tool_calls);
                    // ── Stuck loop detection ──────────────────────────────────
                    // Compute a signature from the tool call names + arguments.
                    // If the same set of calls repeats consecutively, the LLM is
                    // likely stuck in a loop.
                    if last_call_signature == Some(sig) {
                        consecutive_same_calls += 1;
                    } else {
                        consecutive_same_calls = 1;
                        last_call_signature = Some(sig);
                    }

                    if advisor_state.blocked_tool_signatures.contains(&sig) {
                        context_messages.push(ChatMessage::assistant_with_tool_calls(
                            content,
                            tool_calls.clone(),
                        ));
                        for tc in &tool_calls {
                            let blocked_message = serde_json::json!({
                                "status": "error",
                                "code": "advisor_stop_blocked",
                                "message": "Blocked by advisor STOP guidance for this turn. Follow the revised plan, ask a narrow clarification, or return a bounded limitation instead of retrying the same tool-call pattern."
                            })
                            .to_string();
                            context_messages.push(ChatMessage::tool_result(
                                &tc.id,
                                &tc.name,
                                blocked_message,
                            ));
                        }
                        context_messages.push(ChatMessage::system(
                            "Advisor STOP guidance is still active for the blocked tool-call pattern. Choose a different approach.",
                        ));
                        last_call_signature = None;
                        consecutive_same_calls = 0;
                        continue;
                    }

                    if consecutive_same_calls >= STUCK_FORCE_THRESHOLD {
                        tracing::warn!(
                            iteration,
                            consecutive = consecutive_same_calls,
                            tool = %tool_calls.first().map(|t| t.name.as_str()).unwrap_or("?"),
                            "Stuck loop detected — forcing text-only response"
                        );
                        // Give the LLM one last chance with a strong nudge and no tools
                        context_messages.push(ChatMessage::system(STUCK_LOOP_FINALIZATION_PROMPT));

                        return Ok(self
                            .finalize_primary_text_only(
                                &mut reasoning,
                                &mut context_messages,
                                &prompt_context_documents,
                                thread_id,
                                message,
                                &persistent_draft,
                                &original_llm,
                                &mut last_applied_model_override,
                                STUCK_LOOP_FINALIZATION_FAILURE_RESPONSE,
                            )
                            .await?
                            .with_generated_attachments(&generated_attachments));
                    }

                    if consecutive_same_calls == STUCK_WARN_THRESHOLD {
                        tracing::info!(
                            iteration,
                            consecutive = consecutive_same_calls,
                            tool = %tool_calls.first().map(|t| t.name.as_str()).unwrap_or("?"),
                            "Possible stuck loop detected — injecting nudge"
                        );
                        context_messages.push(ChatMessage::system(
                            "You appear to be calling the same tool repeatedly. \
                             Try a different approach, use different parameters, or \
                             provide your answer based on what you already know.",
                        ));
                    }

                    let blocked_signature = last_call_signature;
                    if let Some(result) = self
                        .execute_tool_calls_phase(
                            content,
                            tool_calls,
                            &mut context_messages,
                            &session,
                            thread_id,
                            message,
                            &job_ctx,
                            advisor_call_budget.as_ref(),
                            &mut advisor_state,
                            &identity,
                            routed_agent_workspace_id,
                            routed_allowed_tools.as_deref(),
                            routed_allowed_skills.as_deref(),
                            blocked_signature,
                            &mut last_call_signature,
                            &mut consecutive_same_calls,
                            &mut generated_attachments,
                        )
                        .await?
                    {
                        return Ok(result.with_generated_attachments(&generated_attachments));
                    }
                }
            }
        }
    }
}
