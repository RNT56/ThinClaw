//! Tool dispatch logic for the agent.
//!
//! Extracted from `agent_loop.rs` to keep the core agentic tool execution
//! loop (LLM call -> tool calls -> repeat) in its own focused module.

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::session::{PendingApproval, Session, ThreadState};
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::context::JobContext;
use crate::error::Error;
use crate::llm::{ChatMessage, Reasoning, ReasoningContext, RespondResult};

// Helper functions extracted to dispatcher_helpers.rs
use super::dispatcher_helpers::compact_messages_for_retry;
// Re-export for external consumers (thread_ops.rs, etc.)
pub(crate) use super::dispatcher_helpers::{
    check_auth_required, execute_chat_tool_standalone, parse_auth_result, truncate_preview,
};

/// Result of the agentic loop execution.
pub(super) enum AgenticLoopResult {
    /// Completed with a response.
    Response(String),
    /// A tool requires approval before continuing.
    NeedApproval {
        /// The pending approval request to store.
        pending: PendingApproval,
    },
}

impl Agent {
    /// Run the agentic loop: call LLM, execute tools, repeat until text response.
    ///
    /// Returns `AgenticLoopResult::Response` on completion, or
    /// `AgenticLoopResult::NeedApproval` if a tool requires user approval.
    ///
    pub(super) async fn run_agentic_loop(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        initial_messages: Vec<ChatMessage>,
    ) -> Result<AgenticLoopResult, Error> {
        // Detect group chat from channel metadata (needed before loading system prompt)
        let is_group_chat = message
            .metadata
            .get("chat_type")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t == "group" || t == "channel" || t == "supergroup");

        // Load workspace system prompt (identity files: AGENTS.md, SOUL.md, etc.)
        // In group chats, MEMORY.md is excluded to prevent leaking personal context.
        let system_prompt = if let Some(ws) = self.workspace() {
            match ws.system_prompt_for_context(is_group_chat).await {
                Ok(prompt) if !prompt.is_empty() => Some(prompt),
                Ok(_) => None,
                Err(e) => {
                    tracing::debug!("Could not load workspace system prompt: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Select and prepare active skills (if skills system is enabled)
        let active_skills = self.select_active_skills(&message.content).await;

        // Build skill context block — announce skills compactly (Phase 3: lazy loading).
        // Instead of injecting full SKILL.md content, list names + descriptions.
        // The agent uses `skill_read` to load full instructions on demand.
        let skill_context = if !active_skills.is_empty() {
            let mut context_parts = Vec::new();
            for skill in &active_skills {
                tracing::info!(
                    skill_name = skill.name(),
                    skill_version = skill.version(),
                    trust = %skill.trust,
                    "Skill activated"
                );

                context_parts.push(format!(
                    "- **{}** (v{}, {}): {}",
                    skill.name(),
                    skill.version(),
                    skill.trust,
                    skill.manifest.description,
                ));
            }
            context_parts.push(
                "\nUse `skill_read` with the skill name to load full instructions before using a skill.".to_string()
            );
            Some(context_parts.join("\n"))
        } else {
            None
        };

        // ── Smart routing: select LLM provider BEFORE Reasoning is constructed ──
        // Bug 3 fix: previously the selected provider was computed but the result
        // was never used — Reasoning always received self.llm(). We now pick the
        // provider first so Reasoning is initialized with the correct backend.
        let routed_llm: Arc<dyn crate::llm::LlmProvider> =
            if let Some(ref policy_lock) = self.deps.routing_policy {
                let policy = policy_lock.read().await;
                if policy.is_enabled() && policy.rule_count() > 0 {
                    let est_tokens: u32 = initial_messages
                        .iter()
                        .map(|m| (m.estimated_chars() / 4) as u32)
                        .sum();
                    let ctx = crate::llm::routing_policy::RoutingContext {
                        estimated_input_tokens: est_tokens,
                        has_vision: message
                            .attachments
                            .iter()
                            .any(|a| a.mime_type.starts_with("image/")),
                        has_tools: self.deps.tools.count() > 0,
                        requires_streaming: false,
                        budget_usd: None,
                    };
                    let selected = policy.select_provider(&ctx);
                    let current = self.llm().active_model_name();
                    if selected != current {
                        let cheap = self.cheap_llm();
                        if cheap.active_model_name() == selected {
                            tracing::info!(
                                current_provider = %current,
                                selected_provider = %selected,
                                est_tokens = est_tokens,
                                "Smart routing: switching to cheap model"
                            );
                            Arc::clone(cheap)
                        } else {
                            // Selected model name doesn't match cheap_llm — graceful fallback
                            tracing::info!(
                                selected_provider = %selected,
                                "Smart routing: selected provider not available, using primary"
                            );
                            self.llm().clone()
                        }
                    } else {
                        self.llm().clone()
                    }
                } else {
                    self.llm().clone()
                }
            } else {
                self.llm().clone()
            };

        let active_channel_names = self.channels.channel_names().await;

        let mut reasoning = Reasoning::new(routed_llm, self.safety().clone())
            .with_channel(message.channel.clone())
            .with_model_name(self.llm().active_model_name())
            .with_group_chat(is_group_chat)
            .with_active_channels(active_channel_names)
            .with_workspace_mode(
                &self.config.workspace_mode,
                self.config
                    .workspace_root
                    .as_ref()
                    .map(|p| p.display().to_string()),
            );
        if let Some(ref tracker) = self.deps.cost_tracker {
            reasoning = reasoning.with_cost_tracker(Arc::clone(tracker));
        }
        if let Some(ref cache) = self.deps.response_cache {
            reasoning = reasoning.with_response_cache(Arc::clone(cache));
        }

        if let Some(prompt) = system_prompt {
            reasoning = reasoning.with_system_prompt(prompt);
        }
        if let Some(ctx) = skill_context {
            reasoning = reasoning.with_skill_context(ctx);
        }

        // Build context with messages that we'll mutate during the loop
        let mut context_messages = initial_messages;

        // Create a JobContext for tool execution (chat doesn't have a real job)
        let job_ctx = JobContext::with_user(&message.user_id, "chat", "Interactive chat session");

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
            // shared model_override will be Some. Create a new provider from
            // the catalog and swap it into Reasoning so subsequent LLM calls
            // use the agent's chosen model. We track the last applied spec to
            // avoid recreating the provider on every iteration.
            if let Some(ref override_lock) = self.deps.model_override {
                let current_override = override_lock.read().await.clone();
                let current_spec = current_override.as_ref().map(|mo| mo.model_spec.clone());
                if current_spec != last_applied_model_override {
                    if let Some(ref mo) = current_override {
                        let new_model = &mo.model_spec;
                        if let Some((provider_slug, model_name)) = new_model.split_once('/') {
                            match crate::llm::provider_factory::create_provider_for_catalog_entry(
                                provider_slug,
                                model_name,
                            ) {
                                Ok(new_provider) => {
                                    tracing::info!(
                                        to = %new_model,
                                        reason = mo.reason.as_deref().unwrap_or("agent decision"),
                                        "Agent-driven model switch via llm_select"
                                    );
                                    reasoning.swap_llm(new_provider);
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        model = %new_model,
                                        error = %e,
                                        "Failed to create provider for agent model override, keeping current"
                                    );
                                }
                            }
                        }
                    } else {
                        // Override was reset — swap back to the original provider.
                        tracing::info!("Agent model override reset — restoring primary");
                        reasoning.swap_llm(original_llm.clone());
                    }
                    last_applied_model_override = current_spec;
                }
            }

            // Check if interrupted — preserve partial output
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
                    return Ok(AgenticLoopResult::Response(partial));
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
                        .with_tools(self.tools().tool_definitions().await);

                    match reasoning.respond_with_tools(&flush_ctx).await {
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
                                                    output_len = output.len(),
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

                // Bug 9 fix: reset the flush flag ONLY when the hard cap fires AND
                // messages have actually been dropped (a new compaction cycle begins).
                // Previously the reset fired in the same iteration as the flush when
                // messages jumped from flush_threshold to > max_ctx in one step,
                // allowing a second flush immediately on the next iteration.
                //
                // We check this BEFORE the flush guard so that if context_messages
                // crossed the cap AND the threshold in this very iteration, we correctly
                // reset the flag for the NEW compaction window rather than incorrectly
                // re-arming it after firing and resetting in the same pass.
                if context_messages.len() > max_ctx {
                    // Only reset if the flush hasn't fired in THIS iteration yet.
                    // The flag was already set to true above if we just fired, so this
                    // check doesn't accidentally prevent the reset on the next cycle.
                    memory_flush_fired = false;
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
            let tool_defs = self.tools().tool_definitions().await;

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

            // Call LLM with current context; force_text drops tools to guarantee a
            // text response on the final iteration.
            let thinking = {
                let model_name = self.llm().active_model_name();
                let (enabled, budget) = self.config.resolve_thinking_for_model(&model_name);
                if enabled {
                    crate::llm::ThinkingConfig::Enabled {
                        budget_tokens: budget,
                    }
                } else {
                    crate::llm::ThinkingConfig::Disabled
                }
            };
            let mut context = ReasoningContext::new()
                .with_messages(context_messages.clone())
                .with_tools(tool_defs)
                .with_metadata({
                    let mut m = std::collections::HashMap::new();
                    m.insert("thread_id".to_string(), thread_id.to_string());
                    m
                });
            context.force_text = force_text;
            context.thinking = thinking;

            if force_text {
                tracing::info!(
                    iteration,
                    "Forcing text-only response (iteration limit reached)"
                );
            }

            // ── Fire BeforeLlmInput hook ───────────────────────────────
            {
                let last_user_msg = context_messages
                    .iter()
                    .rev()
                    .find(|m| m.role == crate::llm::Role::User)
                    .map(|m| m.content.clone())
                    .unwrap_or_default();
                let system_msg = context_messages
                    .iter()
                    .find(|m| m.role == crate::llm::Role::System)
                    .map(|m| m.content.clone());
                let event = crate::hooks::HookEvent::LlmInput {
                    model: self.llm().active_model_name(),
                    system_message: system_msg,
                    user_message: last_user_msg,
                    message_count: context_messages.len(),
                    user_id: message.user_id.clone(),
                };
                match self.hooks().run(&event).await {
                    Ok(crate::hooks::HookOutcome::Continue { modified }) => {
                        if let Some(new_content) = modified {
                            // Replace the last user message with the modified content
                            if let Some(last) = context_messages
                                .iter_mut()
                                .rev()
                                .find(|m| m.role == crate::llm::Role::User)
                            {
                                last.content = new_content;
                            }
                            // Rebuild context with modified messages
                            context = context.with_messages(context_messages.clone());
                        }
                    }
                    Ok(crate::hooks::HookOutcome::Reject { reason }) => {
                        tracing::info!(reason = %reason, "BeforeLlmInput hook rejected LLM call");
                        return Err(crate::error::Error::from(
                            crate::error::ChannelError::StartupFailed {
                                name: "hook".into(),
                                reason: format!("BeforeLlmInput hook rejected: {}", reason),
                            },
                        ));
                    }
                    Err(crate::hooks::HookError::Rejected { reason }) => {
                        tracing::info!(reason = %reason, "BeforeLlmInput hook rejected LLM call");
                        return Err(crate::error::Error::from(
                            crate::error::ChannelError::StartupFailed {
                                name: "hook".into(),
                                reason: format!("BeforeLlmInput hook rejected: {}", reason),
                            },
                        ));
                    }
                    Err(err) => {
                        tracing::warn!("BeforeLlmInput hook error (fail-open): {}", err);
                    }
                }
            }

            // ── Choose streaming vs non-streaming LLM call ─────────────
            let channel_stream_mode = self.channels.stream_mode(&message.channel).await;
            let use_streaming = channel_stream_mode != crate::channels::StreamMode::None;

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Thinking(if use_streaming {
                        "Streaming response...".into()
                    } else {
                        "Calling LLM...".into()
                    }),
                    &message.metadata,
                )
                .await;

            let llm_start = std::time::Instant::now();
            let output = if use_streaming {
                // Streaming path: forward text chunks to channel as draft edits
                let channels = Arc::clone(&self.channels);
                let channel_name = message.channel.clone();
                let metadata = message.metadata.clone();
                let mode = channel_stream_mode;

                // Draft state tracks the in-progress message
                let draft = Arc::new(tokio::sync::Mutex::new(
                    crate::channels::DraftReplyState::new(&channel_name),
                ));
                let draft_for_stream = Arc::clone(&draft);

                let stream_result = reasoning
                    .respond_with_tools_streaming(&context, move |chunk: &str| {
                        // Fire-and-forget draft update (we're in a sync FnMut callback)
                        let channels = Arc::clone(&channels);
                        let ch_name = channel_name.clone();
                        let md = metadata.clone();
                        let draft_ref = Arc::clone(&draft_for_stream);
                        let chunk_owned = chunk.to_string();

                        tokio::spawn(async move {
                            let mut d = draft_ref.lock().await;
                            let should_send = d.append(&chunk_owned);
                            if should_send {
                                let display = match mode {
                                    crate::channels::StreamMode::StatusLine => {
                                        let word_count = d.accumulated.split_whitespace().count();
                                        format!("✦ Generating... ({} words)", word_count)
                                    }
                                    _ => d.display_text(),
                                };

                                // Create a temporary draft with display text for sending
                                let mut send_draft =
                                    crate::channels::DraftReplyState::new(&ch_name);
                                send_draft.accumulated = display;
                                send_draft.message_id = d.message_id.clone();
                                send_draft.posted = d.posted;

                                match channels.send_draft(&ch_name, &send_draft, &md).await {
                                    Ok(msg_id) => d.mark_sent(msg_id),
                                    Err(e) => {
                                        tracing::debug!("Draft edit failed (non-fatal): {}", e);
                                    }
                                }
                            }
                        });
                    })
                    .await;

                // Send final draft with complete text (remove typing indicator)
                {
                    let d = draft.lock().await;
                    if d.posted && !d.accumulated.is_empty() {
                        let _ = self
                            .channels
                            .send_draft(&message.channel, &d, &message.metadata)
                            .await;
                    }
                }

                match stream_result {
                    Ok(output) => output,
                    Err(e) => return Err(e.into()),
                }
            } else {
                // Non-streaming path (original)
                match reasoning.respond_with_tools(&context).await {
                    Ok(output) => output,
                    Err(crate::error::LlmError::ContextLengthExceeded { used, limit }) => {
                        tracing::warn!(
                            used,
                            limit,
                            iteration,
                            "Context length exceeded, compacting messages and retrying"
                        );

                        // Compact: keep system messages + last user message + current turn
                        context_messages = compact_messages_for_retry(&context_messages);

                        // Rebuild context with compacted messages
                        let mut retry_context = ReasoningContext::new()
                            .with_messages(context_messages.clone())
                            .with_tools(if force_text {
                                Vec::new()
                            } else {
                                context.available_tools.clone()
                            })
                            .with_metadata(context.metadata.clone());
                        retry_context.force_text = force_text;

                        reasoning
                            .respond_with_tools(&retry_context)
                            .await
                            .map_err(|retry_err| {
                                tracing::error!(
                                    original_used = used,
                                    original_limit = limit,
                                    retry_error = %retry_err,
                                    "Retry after auto-compaction also failed"
                                );
                                crate::error::Error::from(retry_err)
                            })?
                    }
                    Err(e) => return Err(e.into()),
                }
            };

            // Record cost and track token usage
            let model_name = self.llm().active_model_name();

            // ── Fire AfterLlmOutput hook ──────────────────────────────
            {
                let output_text = match &output.result {
                    crate::llm::RespondResult::Text(t) => t.clone(),
                    crate::llm::RespondResult::ToolCalls { content, .. } => {
                        content.clone().unwrap_or_default()
                    }
                };
                let event = crate::hooks::HookEvent::LlmOutput {
                    model: model_name.clone(),
                    content: output_text,
                    input_tokens: output.usage.input_tokens,
                    output_tokens: output.usage.output_tokens,
                    user_id: message.user_id.clone(),
                };
                match self.hooks().run(&event).await {
                    Ok(crate::hooks::HookOutcome::Continue { .. }) => {
                        // AfterLlmOutput modifications are informational —
                        // the output struct is already committed.
                    }
                    Ok(crate::hooks::HookOutcome::Reject { reason }) => {
                        tracing::info!(reason = %reason, "AfterLlmOutput hook rejected response");
                        return Err(crate::error::Error::from(
                            crate::error::ChannelError::StartupFailed {
                                name: "hook".into(),
                                reason: format!("AfterLlmOutput hook rejected: {}", reason),
                            },
                        ));
                    }
                    Err(crate::hooks::HookError::Rejected { reason }) => {
                        tracing::info!(reason = %reason, "AfterLlmOutput hook rejected response");
                        return Err(crate::error::Error::from(
                            crate::error::ChannelError::StartupFailed {
                                name: "hook".into(),
                                reason: format!("AfterLlmOutput hook rejected: {}", reason),
                            },
                        ));
                    }
                    Err(err) => {
                        tracing::warn!("AfterLlmOutput hook error (fail-open): {}", err);
                    }
                }
            }
            let call_cost = self
                .cost_guard()
                .record_llm_call(
                    &model_name,
                    output.usage.input_tokens,
                    output.usage.output_tokens,
                    Some(self.llm().cost_per_token()),
                )
                .await;
            tracing::debug!(
                "LLM call used {} input + {} output tokens (${:.6})",
                output.usage.input_tokens,
                output.usage.output_tokens,
                call_cost,
            );

            // Record latency for smart routing (LowestLatency rule)
            if let Some(ref policy_lock) = self.deps.routing_policy {
                let latency_ms = llm_start.elapsed().as_millis() as f64;
                policy_lock
                    .write()
                    .await
                    .record_latency(&model_name, latency_ms);
            }

            // Emit cost alert SSE event when approaching/exceeding budget
            if let Some(ref sse_tx) = self.deps.sse_sender
                && let Some(limit_cents) = self.config.max_cost_per_day_cents
            {
                use rust_decimal::prelude::ToPrimitive;
                let daily_spend = self.cost_guard().daily_spend().await;
                let spent_usd = daily_spend.to_f64().unwrap_or(0.0);
                let limit_usd = limit_cents as f64 / 100.0;
                let pct = if limit_usd > 0.0 {
                    spent_usd / limit_usd * 100.0
                } else {
                    0.0
                };
                if pct >= 100.0 {
                    let _ = sse_tx.send(crate::channels::web::types::SseEvent::CostAlert {
                        alert_type: "exceeded".to_string(),
                        current_cost_usd: spent_usd,
                        limit_usd,
                        message: Some(format!(
                            "Daily budget exceeded: ${:.2} of ${:.2}",
                            spent_usd, limit_usd,
                        )),
                    });
                } else if pct >= 80.0 {
                    let _ = sse_tx.send(crate::channels::web::types::SseEvent::CostAlert {
                        alert_type: "warning".to_string(),
                        current_cost_usd: spent_usd,
                        limit_usd,
                        message: Some(format!(
                            "Approaching daily budget: ${:.2} of ${:.2} ({:.0}%)",
                            spent_usd, limit_usd, pct,
                        )),
                    });
                }
            }

            // Emit extended thinking content if present
            if let Some(ref thinking_text) = output.thinking_content
                && !thinking_text.is_empty()
            {
                tracing::debug!(
                    thinking_len = thinking_text.len(),
                    "LLM returned extended thinking content"
                );
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Thinking(format!("[Reasoning]\n{}", thinking_text)),
                        &message.metadata,
                    )
                    .await;
            }

            match output.result {
                RespondResult::Text(text) => {
                    return Ok(AgenticLoopResult::Response(text));
                }
                RespondResult::ToolCalls {
                    tool_calls,
                    content,
                } => {
                    // ── Stuck loop detection ──────────────────────────────────
                    // Compute a signature from the tool call names + arguments.
                    // If the same set of calls repeats consecutively, the LLM is
                    // likely stuck in a loop.
                    {
                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        for tc in &tool_calls {
                            tc.name.hash(&mut hasher);
                            tc.arguments.to_string().hash(&mut hasher);
                        }
                        let sig = hasher.finish();

                        if last_call_signature == Some(sig) {
                            consecutive_same_calls += 1;
                        } else {
                            consecutive_same_calls = 1;
                            last_call_signature = Some(sig);
                        }
                    }

                    if consecutive_same_calls >= STUCK_FORCE_THRESHOLD {
                        tracing::warn!(
                            iteration,
                            consecutive = consecutive_same_calls,
                            tool = %tool_calls.first().map(|t| t.name.as_str()).unwrap_or("?"),
                            "Stuck loop detected — forcing text-only response"
                        );
                        // Give the LLM one last chance with a strong nudge and no tools
                        context_messages.push(ChatMessage::system(
                            "STOP. You have called the same tool repeatedly without making progress. \
                             Do NOT call any more tools. Summarize what you have done so far and \
                             provide your best answer with the information you already have.",
                        ));
                        let mut final_context = ReasoningContext::new()
                            .with_messages(context_messages.clone())
                            .with_tools(Vec::new()); // No tools available
                        final_context.force_text = true;
                        let final_output = reasoning.respond_with_tools(&final_context).await?;
                        if let RespondResult::Text(text) = final_output.result {
                            return Ok(AgenticLoopResult::Response(text));
                        }
                        // If it still somehow returns tool_calls (shouldn't happen with
                        // empty tools + force_text), return an error message.
                        return Ok(AgenticLoopResult::Response(
                            "I was unable to make further progress. Please try rephrasing your request.".to_string()
                        ));
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

                    // Add the assistant message with tool_calls to context.
                    // OpenAI protocol requires this before tool-result messages.
                    context_messages.push(ChatMessage::assistant_with_tool_calls(
                        content,
                        tool_calls.clone(),
                    ));

                    // Execute tools and add results to context
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::Thinking(format!(
                                "Executing {} tool(s)...",
                                tool_calls.len()
                            )),
                            &message.metadata,
                        )
                        .await;

                    // Record tool calls in the thread
                    {
                        let mut sess = session.lock().await;
                        if let Some(thread) = sess.threads.get_mut(&thread_id)
                            && let Some(turn) = thread.last_turn_mut()
                        {
                            for tc in &tool_calls {
                                turn.record_tool_call(&tc.name, tc.arguments.clone());
                            }
                        }
                    }

                    // === Phase 1: Preflight (sequential) ===
                    // Walk tool_calls checking approval and hooks. Classify
                    // each tool as Rejected (by hook) or Runnable. Stop at the
                    // first tool that needs approval.
                    //
                    // Outcomes are indexed by original tool_calls position so
                    // Phase 3 can emit results in the correct order.
                    enum PreflightOutcome {
                        /// Hook rejected/blocked this tool; contains the error message.
                        Rejected(String),
                        /// Tool passed preflight and will be executed.
                        Runnable,
                    }
                    let mut preflight: Vec<(crate::llm::ToolCall, PreflightOutcome)> = Vec::new();
                    let mut runnable: Vec<(usize, crate::llm::ToolCall)> = Vec::new();
                    let mut approval_needed: Option<(
                        usize,
                        crate::llm::ToolCall,
                        Arc<dyn crate::tools::Tool>,
                    )> = None;

                    for (idx, original_tc) in tool_calls.iter().enumerate() {
                        let mut tc = original_tc.clone();

                        // Hook: BeforeToolCall (runs before approval so hooks can
                        // modify parameters — approval is checked on final params)
                        let event = crate::hooks::HookEvent::ToolCall {
                            tool_name: tc.name.clone(),
                            parameters: tc.arguments.clone(),
                            user_id: message.user_id.clone(),
                            context: "chat".to_string(),
                        };
                        match self.hooks().run(&event).await {
                            Err(crate::hooks::HookError::Rejected { reason }) => {
                                preflight.push((
                                    tc,
                                    PreflightOutcome::Rejected(format!(
                                        "Tool call rejected by hook: {}",
                                        reason
                                    )),
                                ));
                                continue; // skip to next tool (not infinite: using for loop)
                            }
                            Err(err) => {
                                preflight.push((
                                    tc,
                                    PreflightOutcome::Rejected(format!(
                                        "Tool call blocked by hook policy: {}",
                                        err
                                    )),
                                ));
                                continue;
                            }
                            Ok(crate::hooks::HookOutcome::Continue {
                                modified: Some(new_params),
                            }) => match serde_json::from_str(&new_params) {
                                Ok(parsed) => tc.arguments = parsed,
                                Err(e) => {
                                    tracing::warn!(
                                        tool = %tc.name,
                                        "Hook returned non-JSON modification for ToolCall, ignoring: {}",
                                        e
                                    );
                                }
                            },
                            _ => {}
                        }

                        // Check if tool requires approval on the final (post-hook)
                        // parameters. When auto_approve_tools is set, auto-approve
                        // everything EXCEPT ApprovalRequirement::Always (destructive
                        // commands from NEVER_AUTO_APPROVE_PATTERNS like rm -rf,
                        // DROP DATABASE, etc.) which always require human approval.
                        if let Some(tool) = self.tools().get(&tc.name).await {
                            use crate::tools::ApprovalRequirement;
                            let approval = tool.requires_approval(&tc.arguments);
                            let needs_approval = if self.config.auto_approve_tools {
                                // Auto-approve mode: only block Always-approval
                                // tools (destructive shell commands, hardware access).
                                matches!(approval, ApprovalRequirement::Always)
                            } else {
                                // Normal mode: full approval check.
                                match approval {
                                    ApprovalRequirement::Never => false,
                                    ApprovalRequirement::UnlessAutoApproved => {
                                        let sess = session.lock().await;
                                        !sess.is_tool_auto_approved(&tc.name)
                                    }
                                    ApprovalRequirement::Always => true,
                                }
                            };

                            if needs_approval {
                                approval_needed = Some((idx, tc, tool));
                                break; // remaining tools are deferred
                            }
                        }

                        let preflight_idx = preflight.len();
                        preflight.push((tc.clone(), PreflightOutcome::Runnable));
                        runnable.push((preflight_idx, tc));
                    }

                    // === Phase 2: Parallel execution ===
                    // Execute runnable tools and slot results back by preflight
                    // index so Phase 3 can iterate in original order.
                    let mut exec_results: Vec<Option<Result<String, Error>>> =
                        (0..preflight.len()).map(|_| None).collect();

                    if runnable.len() <= 1 {
                        // Single tool (or none): execute inline
                        for (pf_idx, tc) in &runnable {
                            let _ = self
                                .channels
                                .send_status(
                                    &message.channel,
                                    StatusUpdate::ToolStarted {
                                        name: tc.name.clone(),
                                        parameters: Some(tc.arguments.clone()),
                                    },
                                    &message.metadata,
                                )
                                .await;

                            let result = self
                                .execute_chat_tool(&tc.name, &tc.arguments, &job_ctx)
                                .await;

                            let _ = self
                                .channels
                                .send_status(
                                    &message.channel,
                                    StatusUpdate::ToolCompleted {
                                        name: tc.name.clone(),
                                        success: result.is_ok(),
                                        result_preview: result
                                            .as_ref()
                                            .ok()
                                            .map(|s| truncate_preview(s, 500)),
                                    },
                                    &message.metadata,
                                )
                                .await;

                            exec_results[*pf_idx] = Some(result);
                        }
                    } else {
                        // Multiple tools: execute in parallel via JoinSet
                        let mut join_set = JoinSet::new();

                        for (pf_idx, tc) in &runnable {
                            let pf_idx = *pf_idx;
                            let tools = self.tools().clone();
                            let safety = self.safety().clone();
                            let channels = self.channels.clone();
                            let job_ctx = job_ctx.clone();
                            let tc = tc.clone();
                            let channel = message.channel.clone();
                            let metadata = message.metadata.clone();

                            join_set.spawn(async move {
                                let _ = channels
                                    .send_status(
                                        &channel,
                                        StatusUpdate::ToolStarted {
                                            name: tc.name.clone(),
                                            parameters: Some(tc.arguments.clone()),
                                        },
                                        &metadata,
                                    )
                                    .await;

                                let result = execute_chat_tool_standalone(
                                    &tools,
                                    &safety,
                                    &tc.name,
                                    &tc.arguments,
                                    &job_ctx,
                                )
                                .await;

                                let _ = channels
                                    .send_status(
                                        &channel,
                                        StatusUpdate::ToolCompleted {
                                            name: tc.name.clone(),
                                            success: result.is_ok(),
                                            result_preview: result
                                                .as_ref()
                                                .ok()
                                                .map(|s| truncate_preview(s, 500)),
                                        },
                                        &metadata,
                                    )
                                    .await;

                                (pf_idx, result)
                            });
                        }

                        while let Some(join_result) = join_set.join_next().await {
                            match join_result {
                                Ok((pf_idx, result)) => {
                                    exec_results[pf_idx] = Some(result);
                                }
                                Err(e) => {
                                    // Bug 13 fix: capture panic info for debugging.
                                    // The JoinError::into_panic() payload is forwarded
                                    // into the error message so it appears in the LLM
                                    // context and in logs, instead of being silently dropped.
                                    if e.is_panic() {
                                        let panic_payload = e.into_panic();
                                        let panic_msg = panic_payload
                                            .downcast_ref::<&str>()
                                            .map(|s| s.to_string())
                                            .or_else(|| {
                                                panic_payload.downcast_ref::<String>().cloned()
                                            })
                                            .unwrap_or_else(|| {
                                                "<non-string panic payload>".to_string()
                                            });
                                        tracing::error!(
                                            panic = %panic_msg,
                                            "Chat tool execution task panicked"
                                        );
                                    } else {
                                        tracing::error!(
                                            "Chat tool execution task cancelled: {}",
                                            e
                                        );
                                    }
                                }
                            }
                        }

                        // Fill panicked/cancelled slots with descriptive error results
                        for (pf_idx, tc) in runnable.iter() {
                            if exec_results[*pf_idx].is_none() {
                                tracing::error!(
                                    tool = %tc.name,
                                    "Filling failed task slot with error"
                                );
                                exec_results[*pf_idx] =
                                    Some(Err(crate::error::ToolError::ExecutionFailed {
                                        name: tc.name.clone(),
                                        reason: "Task panicked or was cancelled during execution"
                                            .to_string(),
                                    }
                                    .into()));
                            }
                        }
                    }

                    // === Phase 3: Post-flight (sequential, in original order) ===
                    // Process all results — both hook rejections and execution
                    // results — in the original tool_calls order. Auth intercept
                    // is deferred until after every result is recorded.
                    let mut deferred_auth: Option<String> = None;

                    for (pf_idx, (tc, outcome)) in preflight.into_iter().enumerate() {
                        match outcome {
                            PreflightOutcome::Rejected(error_msg) => {
                                // Record hook rejection in thread
                                {
                                    let mut sess = session.lock().await;
                                    if let Some(thread) = sess.threads.get_mut(&thread_id)
                                        && let Some(turn) = thread.last_turn_mut()
                                    {
                                        turn.record_tool_error(error_msg.clone());
                                    }
                                }
                                context_messages
                                    .push(ChatMessage::tool_result(&tc.id, &tc.name, error_msg));
                            }
                            PreflightOutcome::Runnable => {
                                // Retrieve the execution result for this slot
                                let mut tool_result =
                                    exec_results[pf_idx].take().unwrap_or_else(|| {
                                        Err(crate::error::ToolError::ExecutionFailed {
                                            name: tc.name.clone(),
                                            reason: "No result available".to_string(),
                                        }
                                        .into())
                                    });

                                // Send ToolResult preview
                                if let Ok(ref output) = tool_result
                                    && !output.is_empty()
                                {
                                    let _ = self
                                        .channels
                                        .send_status(
                                            &message.channel,
                                            StatusUpdate::ToolResult {
                                                name: tc.name.clone(),
                                                preview: output.clone(),
                                            },
                                            &message.metadata,
                                        )
                                        .await;
                                }

                                // ── Canvas tool interception ────────────────────
                                // If the tool is `canvas` and succeeded, parse the
                                // result as a CanvasAction, emit it as a status
                                // update, and persist it in the CanvasStore.
                                if tc.name == "canvas"
                                    && let Ok(ref output) = tool_result
                                    && let Ok(action) = serde_json::from_str::<
                                        crate::tools::builtin::CanvasAction,
                                    >(output)
                                {
                                    // Emit the action to the channel for
                                    // real-time rendering in the frontend.
                                    let _ = self
                                        .channels
                                        .send_status(
                                            &message.channel,
                                            StatusUpdate::CanvasAction(action.clone()),
                                            &message.metadata,
                                        )
                                        .await;

                                    // Persist in the CanvasStore for HTTP
                                    // access at /canvas/.
                                    if let Some(ref store) = self.deps.canvas_store {
                                        match &action {
                                            crate::tools::builtin::CanvasAction::Show {
                                                panel_id,
                                                title,
                                                components,
                                                ..
                                            } => {
                                                store
                                                    .upsert(
                                                        panel_id.clone(),
                                                        title.clone(),
                                                        serde_json::to_value(components)
                                                            .unwrap_or_default(),
                                                        None,
                                                    )
                                                    .await;
                                            }
                                            crate::tools::builtin::CanvasAction::Update {
                                                panel_id,
                                                components,
                                            } => {
                                                // Update: keep existing title
                                                let existing_title = store
                                                    .get(panel_id)
                                                    .await
                                                    .map(|p| p.title)
                                                    .unwrap_or_else(|| panel_id.clone());
                                                store
                                                    .upsert(
                                                        panel_id.clone(),
                                                        existing_title,
                                                        serde_json::to_value(components)
                                                            .unwrap_or_default(),
                                                        None,
                                                    )
                                                    .await;
                                            }
                                            crate::tools::builtin::CanvasAction::Dismiss {
                                                panel_id,
                                            } => {
                                                store.dismiss(panel_id).await;
                                            }
                                            crate::tools::builtin::CanvasAction::Notify {
                                                ..
                                            } => {
                                                // Notifications are transient;
                                                // no store persistence needed.
                                            }
                                        }
                                    }
                                }

                                // ── emit_user_message interception ───────────────
                                // When the agent calls emit_user_message, forward
                                // the message to the user's channel as a visible
                                // status update. The loop continues normally.
                                if tc.name == "emit_user_message"
                                    && let Ok(ref output) = tool_result
                                    && let Ok(parsed) =
                                        serde_json::from_str::<serde_json::Value>(output)
                                    && let Some(msg) =
                                        parsed.get("message").and_then(|v| v.as_str())
                                {
                                    let msg_type = parsed
                                        .get("message_type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("progress")
                                        .to_string();
                                    let _ = self
                                        .channels
                                        .send_status(
                                            &message.channel,
                                            StatusUpdate::AgentMessage {
                                                content: msg.to_string(),
                                                message_type: msg_type,
                                            },
                                            &message.metadata,
                                        )
                                        .await;
                                }

                                // ── spawn_subagent interception ───────────────────
                                // The spawn_subagent tool outputs a JSON request.
                                // We intercept it here to execute the actual spawning
                                // via the SubagentExecutor and replace the tool result
                                // with the sub-agent's output.
                                if tc.name == "spawn_subagent"
                                    && let Ok(ref output) = tool_result
                                    && let Ok(parsed) =
                                        serde_json::from_str::<serde_json::Value>(output)
                                    && parsed.get("action").and_then(|v| v.as_str())
                                        == Some("spawn_subagent")
                                {
                                    if let Some(executor) = self.subagent_executor.as_ref() {
                                        if let Some(req_val) = parsed.get("request")
                                                        && let Ok(request) = serde_json::from_value::<
                                                            crate::agent::subagent_executor::SubagentSpawnRequest,
                                                        >(
                                                            req_val.clone()
                                                        ) {
                                                            let exec_result = executor
                                                                .spawn(
                                                                    request,
                                                                    &message.channel,
                                                                    &message.metadata,
                                                                )
                                                                .await;

                                                            tool_result = match exec_result {
                                                                Ok(result) => Ok(
                                                                    serde_json::to_string(&result)
                                                                        .unwrap_or_default(),
                                                                ),
                                                                Err(e) => Ok(
                                                                    serde_json::json!({
                                                                        "error": e.to_string(),
                                                                        "success": false,
                                                                    })
                                                                    .to_string(),
                                                                ),
                                                            };
                                                        }
                                    } else {
                                        tool_result = Ok(serde_json::json!({
                                            "error": "Sub-agent system not initialized",
                                            "success": false,
                                        })
                                        .to_string());
                                    }
                                }

                                // ── message_agent interception ────────────────────
                                // The message_agent tool outputs a structured A2A
                                // request. We intercept it here to run the target
                                // agent's task via SubagentExecutor with the target's
                                // system prompt and model.
                                if tc.name == "message_agent"
                                    && let Ok(ref output) = tool_result
                                    && let Ok(parsed) =
                                        serde_json::from_str::<serde_json::Value>(output)
                                    && parsed
                                        .get("a2a_request")
                                        .and_then(|v| v.as_bool())
                                        == Some(true)
                                {
                                    let target_id = parsed
                                        .get("target_agent_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown");
                                    let target_name = parsed
                                        .get("target_display_name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(target_id);
                                    let a2a_message = parsed
                                        .get("message")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let system_prompt = parsed
                                        .get("target_system_prompt")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let target_model = parsed
                                        .get("target_model")
                                        .and_then(|v| v.as_str());
                                    let timeout_secs = parsed
                                        .get("timeout_secs")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(120);

                                    if let Some(executor) = self.subagent_executor.as_ref() {
                                        let request =
                                            crate::agent::subagent_executor::SubagentSpawnRequest {
                                                name: format!("a2a:{}", target_id),
                                                task: a2a_message.to_string(),
                                                system_prompt: Some(system_prompt.to_string()),
                                                allowed_tools: None,
                                                model: target_model.map(String::from),
                                                wait: true,
                                                timeout_secs: Some(timeout_secs),
                                            };

                                        let exec_result = executor
                                            .spawn(
                                                request,
                                                &message.channel,
                                                &message.metadata,
                                            )
                                            .await;

                                        tool_result = match exec_result {
                                            Ok(result) => Ok(
                                                serde_json::json!({
                                                    "a2a_response": true,
                                                    "from_agent": target_id,
                                                    "from_display_name": target_name,
                                                    "response": result.response,
                                                    "success": result.success,
                                                    "iterations": result.iterations,
                                                    "duration_ms": result.duration_ms,
                                                })
                                                .to_string(),
                                            ),
                                            Err(e) => Ok(
                                                serde_json::json!({
                                                    "a2a_response": true,
                                                    "from_agent": target_id,
                                                    "error": e.to_string(),
                                                    "success": false,
                                                })
                                                .to_string(),
                                            ),
                                        };
                                    } else {
                                        tool_result = Ok(serde_json::json!({
                                            "error": "Sub-agent system not initialized — cannot route A2A message",
                                            "a2a_response": true,
                                        })
                                        .to_string());
                                    }

                                    tracing::info!(
                                        target_agent = %target_id,
                                        "Dispatched A2A message via SubagentExecutor"
                                    );
                                }

                                // Record result in thread
                                {
                                    let mut sess = session.lock().await;
                                    if let Some(thread) = sess.threads.get_mut(&thread_id)
                                        && let Some(turn) = thread.last_turn_mut()
                                    {
                                        match &tool_result {
                                            Ok(output) => {
                                                turn.record_tool_result(serde_json::json!(output));
                                            }
                                            Err(e) => {
                                                turn.record_tool_error(e.to_string());
                                            }
                                        }
                                    }
                                }

                                // Check for auth awaiting — defer the return
                                // until all results are recorded.
                                if deferred_auth.is_none()
                                    && let Some((ext_name, instructions)) =
                                        check_auth_required(&tc.name, &tool_result)
                                {
                                    let auth_data = parse_auth_result(&tool_result);
                                    {
                                        let mut sess = session.lock().await;
                                        if let Some(thread) = sess.threads.get_mut(&thread_id) {
                                            thread.enter_auth_mode(ext_name.clone());
                                        }
                                    }
                                    let _ = self
                                        .channels
                                        .send_status(
                                            &message.channel,
                                            StatusUpdate::AuthRequired {
                                                extension_name: ext_name,
                                                instructions: Some(instructions.clone()),
                                                auth_url: auth_data.auth_url,
                                                setup_url: auth_data.setup_url,
                                            },
                                            &message.metadata,
                                        )
                                        .await;
                                    deferred_auth = Some(instructions);
                                }

                                // Sanitize and add tool result to context
                                let result_content = match tool_result {
                                    Ok(output) => {
                                        let sanitized =
                                            self.safety().sanitize_tool_output(&tc.name, &output);
                                        self.safety().wrap_for_llm(
                                            &tc.name,
                                            &sanitized.content,
                                            sanitized.was_modified,
                                        )
                                    }
                                    Err(e) => format!("Error: {}", e),
                                };

                                context_messages.push(ChatMessage::tool_result(
                                    &tc.id,
                                    &tc.name,
                                    result_content,
                                ));
                            }
                        }
                    }

                    // Return auth response after all results are recorded
                    if let Some(instructions) = deferred_auth {
                        return Ok(AgenticLoopResult::Response(instructions));
                    }

                    // Handle approval if a tool needed it
                    if let Some((approval_idx, tc, tool)) = approval_needed {
                        let pending = PendingApproval {
                            request_id: Uuid::new_v4(),
                            tool_name: tc.name.clone(),
                            parameters: tc.arguments.clone(),
                            description: tool.description().to_string(),
                            tool_call_id: tc.id.clone(),
                            context_messages: context_messages.clone(),
                            deferred_tool_calls: tool_calls[approval_idx + 1..].to_vec(),
                        };

                        return Ok(AgenticLoopResult::NeedApproval { pending });
                    }
                }
            }
        }
    }

    /// Execute a tool for chat (without full job context).
    pub(super) async fn execute_chat_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
        job_ctx: &JobContext,
    ) -> Result<String, Error> {
        execute_chat_tool_standalone(self.tools(), self.safety(), tool_name, params, job_ctx).await
    }
}
