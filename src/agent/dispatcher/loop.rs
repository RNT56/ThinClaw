use super::*;
use thinclaw_agent::loop_control::{LoopKind, LoopStopReason};

use crate::observability::LoopMetricGuard;

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
        let mut loop_metrics =
            LoopMetricGuard::start(Arc::clone(self.observer()), LoopKind::AgentDispatcher);
        match tokio::time::timeout(
            self.config.job_timeout,
            self.run_agentic_loop_inner(
                message,
                session,
                thread_id,
                initial_messages,
                &mut loop_metrics,
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                loop_metrics.stop_with(LoopStopReason::WallTimeBudgetExceeded);
                Err(crate::error::JobError::ContextError {
                    id: thread_id,
                    reason: format!(
                        "Agentic loop exceeded its {} second wall-time budget",
                        self.config.job_timeout.as_secs()
                    ),
                }
                .into())
            }
        }
    }

    async fn run_agentic_loop_inner(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        initial_messages: Vec<ChatMessage>,
        loop_metrics: &mut LoopMetricGuard,
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
            context_budget,
        } = self
            .prepare_prompt_context(message, session.clone(), thread_id)
            .await?;

        // Per-turn mutable state (context messages, stuck-loop tracking,
        // memory-flush/model-override flags, advisor state) bundled into a
        // single struct passed by `&mut` to phase helpers.
        let mut turn = TurnState {
            context_messages: initial_messages,
            generated_attachments: Vec::new(),
            last_call_signature: None,
            consecutive_same_calls: 0,
            memory_flush_fired: false,
            last_applied_model_override: None,
            advisor_state: AdvisorTurnState::default(),
        };

        // Plan mode: tell the model it is planning so it proposes concrete steps
        // rather than assuming execution. Enforcement is at the tool-approval
        // gate (dispatcher/tool_execution.rs); this only shapes behavior.
        let plan_mode = {
            let sess = session.lock().await;
            sess.threads
                .get(&thread_id)
                .map(|thread| thread.plan_mode)
                .unwrap_or(false)
        };
        if plan_mode {
            turn.context_messages.push(ChatMessage::immutable_policy(
                "plan_mode",
                "PLAN MODE is active. Investigate with read-only tools, then present a concise, \
                 numbered plan of the state-changing actions you intend to take (file edits, shell \
                 commands, sends) and why. Any state-changing tool you call will pause for the \
                 operator to approve before it runs, so propose deliberately rather than assuming \
                 execution. Plan mode ends when the operator runs `/plan off`.",
            ));
        }

        let tool_policies = crate::tools::policy::ToolPolicyManager::load_from_settings();

        // Create a JobContext for tool execution (chat doesn't have a real job)
        let mut job_ctx = JobContext::with_identity(
            identity.principal_id.clone(),
            identity.actor_id.clone(),
            "chat",
            "Interactive chat session",
        );
        let effective_timezone = if let Some(workspace) = self.workspace() {
            Some(
                workspace
                    .effective_timezone_for_identity(&identity)
                    .await
                    .to_string(),
            )
        } else {
            None
        };
        job_ctx.metadata = message.metadata.clone();
        if !job_ctx.metadata.is_object() {
            job_ctx.metadata = serde_json::json!({});
        }
        if let Some(metadata) = job_ctx.metadata.as_object_mut() {
            // Routed-agent authority is derived from the persisted thread
            // owner below. Never retain transport-supplied workspace IDs or
            // allowlists when the thread has no such route.
            for key in [
                "agent_id",
                "agent_workspace_id",
                "allowed_tools",
                "allowed_skills",
                "tool_profile",
            ] {
                metadata.remove(key);
            }
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
                "stable_external_conversation_key".to_string(),
                serde_json::json!(identity.stable_external_conversation_key.clone()),
            );
            metadata.insert(
                "principal_id".to_string(),
                serde_json::json!(identity.principal_id.clone()),
            );
            metadata.insert(
                "actor_id".to_string(),
                serde_json::json!(identity.actor_id.clone()),
            );
            if let Some(timezone) = effective_timezone {
                metadata.insert("user_timezone".to_string(), serde_json::json!(timezone));
            }
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
        let iteration_policy = IterationLimitPolicy::new(max_tool_iterations);
        let mut iteration = 0;
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

        loop {
            iteration += 1;
            loop_metrics.set_iterations(iteration);
            // Hard ceiling one past the forced-text iteration (should never be reached
            // since the iteration policy forces a text response, but kept as a safety net).
            let iteration_decision = iteration_policy.decision_for(iteration);
            if let Some(reason) = iteration_decision.abort_reason {
                loop_metrics.stop_with(LoopStopReason::IterationBudgetExceeded);
                return Err(crate::error::LlmError::InvalidResponse {
                    provider: "agent".to_string(),
                    reason,
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
                let current_reason = current_override
                    .as_ref()
                    .and_then(|mo| mo.reason.as_deref());
                match decide_model_override_activation(
                    current_spec.as_deref(),
                    current_reason,
                    turn.last_applied_model_override.as_deref(),
                    crate::tools::builtin::llm_tools::is_runtime_supported_provider_slug,
                ) {
                    ModelOverrideActivationDecision::Unchanged => {}
                    ModelOverrideActivationDecision::Activate {
                        model_spec, reason, ..
                    } => {
                        tracing::info!(
                            to = %model_spec,
                            reason = reason.unwrap_or("agent decision"),
                            "Agent-driven model switch via llm_select"
                        );
                        reasoning.swap_llm(
                            crate::tools::builtin::llm_tools::wrap_model_spec_override(
                                original_llm.clone(),
                                model_spec.to_string(),
                            ),
                        );
                        turn.last_applied_model_override = current_spec;
                    }
                    ModelOverrideActivationDecision::Unsupported { model_spec, .. } => {
                        tracing::warn!(
                            model = %model_spec,
                            "Failed to apply agent model override because the provider slug is unsupported"
                        );
                        override_lock.clear(&model_override_scope_key).await;
                        reasoning.swap_llm(original_llm.clone());
                        turn.context_messages.push(ChatMessage::trusted_prompt(
                            "unsupported_model_override",
                            unsupported_model_override_note(model_spec),
                        ));
                        turn.last_applied_model_override = None;
                    }
                    ModelOverrideActivationDecision::Reset => {
                        tracing::info!("Agent model override reset — restoring primary");
                        reasoning.swap_llm(original_llm.clone());
                        turn.last_applied_model_override = None;
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
                    let partial_output = turn
                        .context_messages
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
                        loop_metrics.stop_with(LoopStopReason::Interrupted);
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
                    loop_metrics.stop_with(LoopStopReason::Interrupted);
                    return Ok(AgenticLoopResult::Response(
                        thinclaw_agent::submission::AgentResponsePayload::text(partial),
                    )
                    .with_generated_attachments(&turn.generated_attachments));
                }
            }

            // Budget admission is centralized in UsageTrackingProvider, which
            // wraps every completion/streaming entry point. Keeping a second
            // check here would reserve two hourly slots for one provider call.

            // Context monitor for this iteration, derived from the active
            // model's context window. Reused below for both the memory-flush
            // trigger and the hard context cap so token estimation only
            // happens once per iteration.
            let active_model_name = reasoning.current_llm().active_model_name();
            let context_monitor = self.context_monitor_for_model(&active_model_name);
            let estimated_context_tokens = context_monitor.estimate_tokens(&turn.context_messages);
            let context_token_limit = context_monitor.limit();
            let history_token_budget = context_budget.history_token_limit(context_token_limit);

            // ── Pre-compaction memory flush ──────────────────────────────
            // When the conversation crosses 80% of the model's token budget,
            // fire a silent agentic turn to prompt the agent to write any
            // durable memories BEFORE old messages get dropped by the cap.
            // This matches openclaw's `memoryFlush` pre-compaction ping.
            // The user never sees the response; NO_REPLY means nothing to save.
            {
                if memory_flush_due(
                    estimated_context_tokens,
                    history_token_budget,
                    turn.memory_flush_fired,
                ) {
                    turn.memory_flush_fired = true;
                    tracing::info!(
                        estimated_tokens = estimated_context_tokens,
                        token_limit = context_token_limit,
                        messages = turn.context_messages.len(),
                        "Pre-compaction memory flush triggered"
                    );

                    // Build a minimal context for the flush turn (system + flush prompt).
                    let today = chrono::Utc::now().format("%Y-%m-%d");
                    let flush_system = ChatMessage::immutable_policy(
                        "memory_flush",
                        "Session nearing memory compaction. Store durable memories now.",
                    );
                    let flush_user = ChatMessage::user(format!(
                        "Write any lasting notes to daily/{today}.md via memory_write \
                         (target: \"daily_log\"). If nothing important to save, reply with only: NO_REPLY"
                    ));

                    let mut flush_msgs = turn.context_messages.clone();
                    flush_msgs.push(flush_system);
                    flush_msgs.push(flush_user);

                    // Only memory tools are executable in the flush turn (see
                    // the allowlist below) — advertise exactly those, honoring
                    // the routed agent's tool restrictions, instead of leaking
                    // every registered tool definition into the flush prompt.
                    let allowed_flush_tools = MEMORY_FLUSH_ALLOWED_TOOLS;
                    let flush_tool_defs = self
                        .tools()
                        .tool_definitions_for_capabilities(
                            routed_allowed_tools.as_deref(),
                            routed_allowed_skills.as_deref(),
                            None,
                        )
                        .await
                        .into_iter()
                        .filter(|tool| allowed_flush_tools.contains(&tool.name.as_str()))
                        .collect::<Vec<_>>();
                    let flush_ctx = ReasoningContext::new()
                        .with_messages(flush_msgs)
                        .with_tools(flush_tool_defs);

                    let flush_result = tokio::select! {
                        biased;
                        _ = self.wait_for_turn_cancellation(thread_id) => {
                            loop_metrics.stop_with(LoopStopReason::Interrupted);
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
                                    // Only memory tools may run in the flush context
                                    // to prevent side effects (allowlist declared above).
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

            // Inject a nudge message when approaching the iteration limit so the
            // LLM is aware it should produce a final answer on the next turn.
            if iteration_decision.inject_nudge {
                turn.context_messages.push(ChatMessage::immutable_policy(
                    "iteration_limit_nudge",
                    ITERATION_LIMIT_NUDGE_PROMPT,
                ));
            }

            let force_text = iteration_decision.force_text;

            // ── Hard chat history cap ───────────────────────────────────
            // Enforce a token budget derived from the active model's context
            // window (falling back to max_context_messages as a secondary
            // bound) to prevent OOM/HTTP 400s on very long conversations or
            // conversations padded by a few huge tool results. System
            // messages are always kept; oldest non-system messages are
            // dropped first until the estimated token count is back under
            // the trim target.
            let max_ctx = self.config.max_context_messages;
            match context_cap_decision_with_history_budget(
                estimated_context_tokens,
                context_token_limit,
                history_token_budget,
                turn.context_messages.len(),
                max_ctx,
            ) {
                ContextCapDecision::TrimToBudget { target_tokens } => {
                    let (systems, rest): (Vec<ChatMessage>, Vec<ChatMessage>) = turn
                        .context_messages
                        .drain(..)
                        .partition(|m| m.role == crate::llm::Role::System);

                    let system_tokens = context_monitor.estimate_tokens(&systems);
                    // Secondary bound: even once under the token target, also
                    // keep total message count within max_ctx so pathological
                    // conversations with many small messages (where token
                    // estimation stays low) still get capped.
                    let max_rest = max_ctx.saturating_sub(systems.len()).max(1);
                    let mut kept_rest: std::collections::VecDeque<ChatMessage> =
                        rest.into_iter().collect();
                    let mut rest_tokens: usize = kept_rest
                        .iter()
                        .map(|m| context_monitor.estimate_tokens(std::slice::from_ref(m)))
                        .sum();
                    let mut dropped = 0usize;
                    // Drop oldest non-system messages first until the estimated
                    // total is at or below the trim target and the count is
                    // within the secondary bound (leave at least one message
                    // so the conversation isn't emptied entirely).
                    while kept_rest.len() > 1
                        && (system_tokens + rest_tokens > target_tokens
                            || kept_rest.len() > max_rest)
                    {
                        if let Some(oldest) = kept_rest.pop_front() {
                            rest_tokens = rest_tokens.saturating_sub(
                                context_monitor.estimate_tokens(std::slice::from_ref(&oldest)),
                            );
                            dropped += 1;
                        } else {
                            break;
                        }
                    }
                    tracing::info!(
                        total = systems.len() + kept_rest.len() + dropped,
                        dropped,
                        tokens_before = estimated_context_tokens,
                        tokens_after = system_tokens + rest_tokens,
                        target_tokens,
                        max_context_messages = max_ctx,
                        "Chat history cap applied"
                    );
                    turn.context_messages = systems;
                    turn.context_messages.extend(kept_rest);

                    // Re-sanitize immediately after cap truncation: the cap may have
                    // dropped an assistant(tool_calls) message while keeping its
                    // downstream Tool result messages, creating orphaned tool roles.
                    // Running sanitize_tool_messages here converts them to user messages
                    // before the LLM call, preventing HTTP 400 errors.
                    crate::llm::sanitize_tool_messages(&mut turn.context_messages);

                    // Bug 9 fix (revised): reset the flush flag AFTER the hard cap
                    // actually drops messages so a new compaction cycle can trigger
                    // a fresh flush. Previously the reset fired in the same scope
                    // as the flush trigger (before truncation), which allowed a
                    // double-flush when messages jumped from the 80% threshold
                    // past max_ctx in a single iteration.
                    if dropped > 0 {
                        turn.memory_flush_fired = false;
                    }
                }
                ContextCapDecision::WithinBudget => {}
            }
            // ── Tool-result pruning ─────────────────────────────────────
            // Strip old tool results from context before the LLM call.
            // Matches openclaw's pre-call trimming: only the most recent
            // TOOL_RESULT_KEEP_TURNS turns' tool results are kept.
            // This does NOT modify JSONL/DB history — only the in-memory slice
            // sent to the LLM, preventing token burn over long sessions.
            {
                if let Some(prune_before_idx) =
                    tool_result_prune_boundary(&turn.context_messages, TOOL_RESULT_KEEP_TURNS)
                {
                    let pruned: usize = turn.context_messages[..prune_before_idx]
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
                        for msg in turn.context_messages[..prune_before_idx].iter_mut() {
                            if msg.role == crate::llm::Role::Tool {
                                msg.content =
                                    "[tool result pruned — see session history]".to_string();
                            }
                        }
                    }
                }
            }

            let bounded_history_tokens = context_monitor.estimate_tokens(&turn.context_messages);
            if history_token_budget == 0 || bounded_history_tokens > history_token_budget {
                loop_metrics.stop_with(LoopStopReason::IterationBudgetExceeded);
                return Err(crate::error::LlmError::ContextLengthExceeded {
                    used: bounded_history_tokens,
                    limit: history_token_budget,
                }
                .into());
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
                    .advisor_config_for_messages(&turn.context_messages)
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
                .and_then(|runtime| runtime.advisor_config_for_messages(&turn.context_messages))
                .is_some();
            if advisor_ready_for_turn
                && let Some((trigger, checkpoint, blocked_signature)) = self
                    .next_auto_advisor_trigger(
                        runtime_status.as_ref(),
                        &turn.context_messages,
                        &turn.advisor_state,
                        turn.consecutive_same_calls,
                        turn.last_call_signature,
                    )
            {
                self.inject_auto_advisor_consultation(
                    trigger,
                    checkpoint,
                    blocked_signature,
                    &mut turn.advisor_state,
                    &mut turn.context_messages,
                    &session,
                    thread_id,
                    message,
                    advisor_call_budget.as_ref(),
                    &mut turn.last_call_signature,
                    &mut turn.consecutive_same_calls,
                )
                .await?;
                continue;
            }
            let use_tool_phase_synthesis = tool_phase_synthesis_enabled(
                runtime_status.as_ref(),
                self.deps.cheap_llm.is_some(),
                force_text,
                !tool_defs.is_empty(),
                turn.last_applied_model_override.is_some(),
            );
            let hold_complex_final_pass = advisor_ready_for_turn
                && should_hold_complex_final_pass(
                    runtime_status.as_ref(),
                    &turn.context_messages,
                    &turn.advisor_state,
                );
            let tool_phase_primary_thinking_enabled = runtime_status
                .as_ref()
                .map(|status| status.tool_phase_primary_thinking_enabled)
                .unwrap_or(true);

            let phase_one_model_name = reasoning.current_llm().active_model_name();
            let phase_one_turn = self
                .execute_llm_turn(
                    &mut reasoning,
                    &mut turn.context_messages,
                    tool_defs,
                    thread_id,
                    &session,
                    message,
                    &persistent_draft,
                    &original_llm,
                    &mut turn.last_applied_model_override,
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
                                    loop_metrics.stop_with(LoopStopReason::Completed);
                                    return Ok(AgenticLoopResult::Response(
                                        thinclaw_agent::submission::AgentResponsePayload::text(
                                            text,
                                        ),
                                    )
                                    .with_generated_attachments(&turn.generated_attachments));
                                };

                                let cheap_model_name = cheap_llm.active_model_name();
                                let mut synthesis_reasoning =
                                    reasoning.fork_with_llm(cheap_llm.clone());
                                let mut synthesis_messages = turn.context_messages.clone();
                                synthesis_messages.push(ChatMessage::immutable_policy(
                                    "tool_phase_synthesis",
                                    TOOL_PHASE_SYNTHESIS_PROMPT,
                                ));

                                let synthesis_turn = self
                                    .execute_llm_turn(
                                        &mut synthesis_reasoning,
                                        &mut synthesis_messages,
                                        Vec::new(),
                                        thread_id,
                                        &session,
                                        message,
                                        &persistent_draft,
                                        &cheap_llm,
                                        &mut turn.last_applied_model_override,
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
                                        loop_metrics.stop_with(LoopStopReason::Completed);
                                        return Ok(self
                                            .agentic_result_from_text(
                                                synthesis_streamed_text,
                                                synthesized,
                                            )
                                            .with_generated_attachments(
                                                &turn.generated_attachments,
                                            ));
                                    }
                                    RespondResult::Text(text) => {
                                        tracing::warn!(
                                            finish_reason = ?synthesis_finish_reason,
                                            text_len = text.len(),
                                            "Tool-phase synthesis produced non-final text"
                                        );
                                        loop_metrics.stop_with(LoopStopReason::Completed);
                                        return Ok(AgenticLoopResult::Response(
                                            thinclaw_agent::submission::AgentResponsePayload::text(
                                                finalization_failure_response(
                                                    FinalizationFailureKind::ToolPhase,
                                                ),
                                            ),
                                        )
                                        .with_generated_attachments(&turn.generated_attachments));
                                    }
                                    RespondResult::ToolCalls { .. } => {
                                        tracing::warn!(
                                            "Tool-phase synthesis unexpectedly returned tool calls"
                                        );
                                        loop_metrics.stop_with(LoopStopReason::Completed);
                                        return Ok(AgenticLoopResult::Response(
                                            thinclaw_agent::submission::AgentResponsePayload::text(
                                                finalization_failure_response(
                                                    FinalizationFailureKind::ToolPhase,
                                                ),
                                            ),
                                        )
                                        .with_generated_attachments(&turn.generated_attachments));
                                    }
                                }
                            }
                            ToolPhaseTextOutcome::PrimaryFinalText => {
                                loop_metrics.stop_with(LoopStopReason::Completed);
                                return Ok(self
                                    .agentic_result_from_text(phase_one_streamed_text, text)
                                    .with_generated_attachments(&turn.generated_attachments));
                            }
                            ToolPhaseTextOutcome::PrimaryNeedsFinalization => {
                                let result = self
                                    .finalize_primary_text_only(
                                        &mut reasoning,
                                        &mut turn.context_messages,
                                        &prompt_context_documents,
                                        thread_id,
                                        &session,
                                        message,
                                        &persistent_draft,
                                        &original_llm,
                                        &mut turn.last_applied_model_override,
                                        finalization_failure_response(
                                            FinalizationFailureKind::ToolPhase,
                                        ),
                                    )
                                    .await?;
                                loop_metrics.stop_with(LoopStopReason::Completed);
                                return Ok(
                                    result.with_generated_attachments(&turn.generated_attachments)
                                );
                            }
                        }
                    }

                    if hold_complex_final_pass {
                        let checkpoint = turn
                            .advisor_state
                            .checkpoint_for(AdvisorAutoTrigger::ComplexFinalPass, "final_answer");
                        self.inject_auto_advisor_consultation(
                            AdvisorAutoTrigger::ComplexFinalPass,
                            checkpoint,
                            turn.last_call_signature,
                            &mut turn.advisor_state,
                            &mut turn.context_messages,
                            &session,
                            thread_id,
                            message,
                            advisor_call_budget.as_ref(),
                            &mut turn.last_call_signature,
                            &mut turn.consecutive_same_calls,
                        )
                        .await?;
                        continue;
                    }

                    loop_metrics.stop_with(LoopStopReason::Completed);
                    return Ok(self
                        .agentic_result_from_text(phase_one_streamed_text, text)
                        .with_generated_attachments(&turn.generated_attachments));
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
                    let signature_update = update_stuck_loop_signature(
                        turn.last_call_signature,
                        turn.consecutive_same_calls,
                        sig,
                    );
                    turn.last_call_signature = signature_update.last_call_signature;
                    turn.consecutive_same_calls = signature_update.consecutive_same_calls;

                    if turn.advisor_state.blocked_tool_signatures.contains(&sig) {
                        turn.context_messages
                            .push(ChatMessage::assistant_with_tool_calls(
                                content,
                                tool_calls.clone(),
                            ));
                        for tc in &tool_calls {
                            let blocked_message = serde_json::json!({
                                "status": "error",
                                "code": "advisor_stop_blocked",
                                "message": ADVISOR_BLOCKED_TOOL_RESULT_MESSAGE
                            })
                            .to_string();
                            turn.context_messages.push(ChatMessage::tool_result(
                                &tc.id,
                                &tc.name,
                                blocked_message,
                            ));
                        }
                        turn.context_messages.push(ChatMessage::immutable_policy(
                            "advisor_blocked",
                            ADVISOR_BLOCKED_SYSTEM_PROMPT,
                        ));
                        turn.last_call_signature = None;
                        turn.consecutive_same_calls = 0;
                        continue;
                    }

                    match stuck_loop_decision(turn.consecutive_same_calls) {
                        StuckLoopDecision::ForceText => {
                            tracing::warn!(
                                iteration,
                                consecutive = turn.consecutive_same_calls,
                                tool = %tool_calls.first().map(|t| t.name.as_str()).unwrap_or("?"),
                                "Stuck loop detected — forcing text-only response"
                            );
                            // Give the LLM one last chance with a strong nudge and no tools
                            turn.context_messages.push(ChatMessage::immutable_policy(
                                "stuck_loop_finalization",
                                STUCK_LOOP_FINALIZATION_PROMPT,
                            ));

                            let result = self
                                .finalize_primary_text_only(
                                    &mut reasoning,
                                    &mut turn.context_messages,
                                    &prompt_context_documents,
                                    thread_id,
                                    &session,
                                    message,
                                    &persistent_draft,
                                    &original_llm,
                                    &mut turn.last_applied_model_override,
                                    finalization_failure_response(
                                        FinalizationFailureKind::StuckLoop,
                                    ),
                                )
                                .await?;
                            loop_metrics.stop_with(LoopStopReason::Completed);
                            return Ok(
                                result.with_generated_attachments(&turn.generated_attachments)
                            );
                        }
                        StuckLoopDecision::Warn => {
                            tracing::info!(
                                iteration,
                                consecutive = turn.consecutive_same_calls,
                                tool = %tool_calls.first().map(|t| t.name.as_str()).unwrap_or("?"),
                                "Possible stuck loop detected — injecting nudge"
                            );
                            turn.context_messages.push(ChatMessage::trusted_prompt(
                                "stuck_loop_nudge",
                                STUCK_LOOP_NUDGE_PROMPT,
                            ));
                        }
                        StuckLoopDecision::Continue => {}
                    }

                    let blocked_signature = turn.last_call_signature;
                    if let Some(result) = self
                        .execute_tool_calls_phase(
                            content,
                            tool_calls,
                            &mut turn,
                            &session,
                            thread_id,
                            message,
                            &job_ctx,
                            advisor_call_budget.as_ref(),
                            &identity,
                            routed_agent_workspace_id,
                            routed_allowed_tools.as_deref(),
                            routed_allowed_skills.as_deref(),
                            blocked_signature,
                        )
                        .await?
                    {
                        loop_metrics.stop_with(LoopStopReason::Completed);
                        return Ok(result.with_generated_attachments(&turn.generated_attachments));
                    }
                }
            }
        }
    }
}
