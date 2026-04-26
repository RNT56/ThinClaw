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
        let identity = message.resolved_identity();

        // Detect group chat from channel metadata (needed before loading system prompt)
        let is_group_chat = message
            .metadata
            .get("chat_type")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t == "group" || t == "channel" || t == "supergroup");

        let routed_agent =
            if let Some(owner_id) = self.agent_router.get_thread_owner(thread_id).await {
                self.agent_router.get_agent(&owner_id).await
            } else {
                None
            };
        let routed_agent_workspace_id = routed_agent.as_ref().and_then(|agent| agent.workspace_id);
        let routed_allowed_tools = routed_agent
            .as_ref()
            .and_then(|agent| agent.allowed_tools.as_deref());
        let routed_allowed_skills = routed_agent
            .as_ref()
            .and_then(|agent| agent.allowed_skills.as_deref());
        let effective_workspace = if let (Some(base_workspace), Some(workspace_id)) =
            (self.workspace(), routed_agent_workspace_id)
        {
            Some(Arc::new(
                base_workspace.as_ref().clone().with_agent(workspace_id),
            ))
        } else {
            self.workspace().map(Arc::clone)
        };
        let prompt_settings = if let Some(store) = self.store().map(Arc::clone) {
            match store.get_all_settings(&identity.principal_id).await {
                Ok(map) => crate::settings::Settings::from_db_map(&map).prompt,
                Err(_) => crate::settings::PromptSettings::default(),
            }
        } else {
            crate::settings::PromptSettings::default()
        };
        let existing_runtime = if let Some(store) = self.store().map(Arc::clone) {
            match crate::agent::load_thread_runtime(&store, thread_id).await {
                Ok(runtime) => runtime,
                Err(err) => {
                    tracing::debug!(
                        thread = %thread_id,
                        error = %err,
                        "Failed to load thread runtime before prompt assembly"
                    );
                    None
                }
            }
        } else {
            None
        };

        // Load workspace system prompt (identity files: AGENTS.md, SOUL.md, etc.)
        // In group chats, MEMORY.md is excluded to prevent leaking personal context.
        let mut workspace_prompt = if prompt_settings.session_freeze_enabled {
            existing_runtime
                .as_ref()
                .and_then(|runtime| runtime.frozen_workspace_prompt.clone())
        } else {
            None
        };
        if workspace_prompt.is_none() {
            workspace_prompt = if let Some(ws) = effective_workspace.as_ref() {
                match ws
                    .system_prompt_for_identity(
                        Some(&identity),
                        &message.channel,
                        self.deps.safety.redact_pii_in_prompts(),
                    )
                    .await
                {
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
        }
        if let Some(agent_prompt) = routed_agent
            .as_ref()
            .and_then(|agent| agent.system_prompt.as_ref())
        {
            workspace_prompt = Some(match workspace_prompt.take() {
                Some(prompt) if !prompt.is_empty() => {
                    format!("{}\n\n## Agent Override\n\n{}", prompt, agent_prompt)
                }
                _ => agent_prompt.clone(),
            });
        }
        let workspace_prompt = workspace_prompt
            .map(|prompt| {
                let sanitized =
                    sanitize_project_context(&prompt, prompt_settings.project_context_max_tokens);
                if sanitized.was_truncated {
                    tracing::info!(
                        thread = %thread_id,
                        "Workspace prompt context was truncated to fit prompt.project_context_max_tokens"
                    );
                }
                for pattern in &sanitized.warning_patterns {
                    tracing::warn!(
                        thread = %thread_id,
                        pattern = %pattern,
                        "Suspicious project context content detected during prompt assembly"
                    );
                }
                sanitized.content
            })
            .filter(|prompt| !prompt.trim().is_empty());

        // Select and prepare active skills (if skills system is enabled)
        let active_skills = self
            .select_active_skills(&message.content, routed_allowed_skills)
            .await;

        // Collect the full skill directory (all loaded skills, not just matched ones).
        // This powers the always-on ## Skills section so the agent always knows
        // what skills are installed, even when none keyword-matched this message.
        let all_skills = self.collect_all_skills(routed_allowed_skills).await;

        // Build skill context block.
        //
        // Structure:
        //   ## Skills
        //
        //   [### Active Skills — only when prefilter matched something]
        //   - **name** (vX, trust): description
        //   ...
        //   Use `skill_read` to load full instructions.
        //
        //   [### Available Skills — always present when any skills are loaded]
        //   name, name, name   ← compact directory
        //   If a task might benefit from a listed skill, use `skill_read` to check it.
        let skill_index_context = if !all_skills.is_empty() {
            let mut parts: Vec<String> = vec!["### Available Skills".to_string()];
            for (name, desc) in &all_skills {
                parts.push(format!("- **{}**: {}", name, desc));
            }
            parts.push(
                "\nUse `skill_read` with a skill name to inspect full instructions before relying on a skill.".to_string(),
            );
            Some(parts.join("\n"))
        } else {
            None
        };

        let active_skill_context = if !active_skills.is_empty() {
            let mut parts: Vec<String> = vec!["### Active Skills".to_string()];
            for skill in &active_skills {
                tracing::info!(
                    skill_name = skill.name(),
                    skill_version = skill.version(),
                    trust = %skill.trust,
                    "Skill activated"
                );
                parts.push(format!(
                    "- **{}** (v{}, {}): {}",
                    skill.name(),
                    skill.version(),
                    skill.trust,
                    skill.manifest.description,
                ));
            }
            parts.push(
                "\nUse `skill_read` with the skill name to load full instructions before using a skill.".to_string(),
            );
            Some(parts.join("\n"))
        } else {
            None
        };

        // Request-time provider routing now happens inside the runtime LLM wrapper,
        // which sees the full canonical provider config and live-reload state.
        let routed_llm: Arc<dyn crate::llm::LlmProvider> = if let Some(model_spec) =
            routed_agent.as_ref().and_then(|agent| agent.model.as_ref())
        {
            crate::tools::builtin::llm_tools::wrap_model_spec_override(
                self.llm().clone(),
                model_spec.clone(),
            )
        } else {
            self.llm().clone()
        };

        let active_channel_names = self.channels.channel_names().await;
        let active_channel_hint = self.channels.formatting_hints_for(&message.channel).await;
        let active_personality_overlay = {
            let session_guard = session.lock().await;
            session_guard
                .active_personality
                .as_ref()
                .map(personality::format_overlay)
        };
        let linked_recall_block = if matches!(
            identity.conversation_kind,
            crate::identity::ConversationKind::Direct
        ) && let Some(store) = self.store().map(Arc::clone)
            && let Ok(mut conversations) = store
                .list_actor_conversations_for_recall(
                    &identity.principal_id,
                    &identity.actor_id,
                    false,
                    8,
                )
                .await
        {
            conversations.retain(|summary| summary.id != thread_id);
            crate::history::LinkedConversationRecall::new(
                identity.principal_id.clone(),
                identity.actor_id.clone(),
                false,
                conversations,
            )
            .compact_block()
        } else {
            None
        };
        let (provider_context, provider_tool_extensions, provider_system_prompt) =
            if let Some(store) = self.store().map(Arc::clone) {
                let orchestrator = crate::agent::learning::LearningOrchestrator::new(
                    store,
                    self.workspace().cloned(),
                    self.skill_registry().cloned(),
                );
                let frozen_block = if prompt_settings.session_freeze_enabled {
                    existing_runtime
                        .as_ref()
                        .and_then(|runtime| runtime.frozen_provider_system_prompt.clone())
                } else {
                    None
                };
                let provider_system_prompt = if let Some(block) = frozen_block {
                    Some(block)
                } else {
                    orchestrator
                        .provider_system_prompt_block(&identity.principal_id)
                        .await
                };
                (
                    orchestrator
                        .prefetch_provider_context(&identity.principal_id, &message.content, 6)
                        .await,
                    orchestrator
                        .provider_tool_extensions(&identity.principal_id)
                        .await,
                    provider_system_prompt,
                )
            } else {
                (None, Vec::new(), None)
            };
        let post_compaction_fragment = if let Some(store) = self.store().map(Arc::clone)
            && let Ok(Some(runtime)) = crate::agent::load_thread_runtime(&store, thread_id).await
        {
            runtime.post_compaction_context
        } else {
            None
        };
        let runtime_capability_hint = {
            let has_execute_code = self.tools().has("execute_code").await;
            let has_shell = self.tools().has("shell").await;
            let has_process = self.tools().has("process").await;
            let has_create_job = self.tools().has("create_job").await;
            let mut caps = Vec::new();
            if has_execute_code {
                let host_local_network =
                    crate::tools::execution_backend::host_local_network_deny_support().as_str();
                caps.push(format!(
                    "execute_code(host-local no-network={host_local_network})"
                ));
            }
            if has_shell {
                let host_local_network =
                    crate::tools::execution_backend::host_local_network_deny_support().as_str();
                caps.push(format!("shell(host-local no-network={host_local_network})"));
            }
            if has_process {
                caps.push("process(long-running host process)".to_string());
            }
            if has_create_job {
                caps.push("create_job(persistent sandbox job runtimes)".to_string());
            }
            if caps.is_empty() {
                None
            } else {
                Some(format!(
                    "Runtime capability hints: available execution surfaces include {}. Use them based on policy, approvals, and current tool availability.",
                    caps.join(", ")
                ))
            }
        };
        let prompt_assembly = PromptAssemblyV2::new()
            .push_stable(
                "workspace_prompt",
                workspace_prompt.clone().unwrap_or_default(),
            )
            .push_stable(
                "provider_system_prompt",
                provider_system_prompt.clone().unwrap_or_default(),
            )
            .push_stable(
                "skills_index",
                skill_index_context
                    .map(|ctx| format!("## Skills\n{ctx}"))
                    .unwrap_or_default(),
            )
            .push_ephemeral("transcript_guidance", "Channel transcript guidance: when the user asks about prior Telegram, WebUI, or other channel conversations, use session_search to inspect transcript history. Do not use communication/action tools like telegram_actions to read transcript history or infer account login state; those tools perform live platform actions only.")
            .push_ephemeral(
                "provider_recall",
                provider_context
                    .as_ref()
                    .map(|ctx| format!("## External Memory Recall\n{}", ctx.rendered_context))
                    .unwrap_or_default(),
            )
            .push_ephemeral(
                "linked_recall",
                linked_recall_block
                    .as_ref()
                    .map(|block| format!("## Linked Recall\n{block}"))
                    .unwrap_or_default(),
            )
            .push_ephemeral(
                "channel_formatting_hints",
                active_channel_hint
                    .as_ref()
                    .map(|hints| {
                        format!(
                            "## Platform Formatting ({})\n{}",
                            message.channel,
                            hints
                        )
                    })
                    .unwrap_or_default(),
            )
            .push_ephemeral(
                "personality_overlay",
                active_personality_overlay
                    .as_ref()
                    .map(|overlay| format!("## Temporary Personality\n\n{overlay}"))
                    .unwrap_or_default(),
            )
            .push_ephemeral(
                "runtime_capabilities",
                runtime_capability_hint.unwrap_or_default(),
            )
            .push_ephemeral(
                "active_skills",
                active_skill_context
                    .map(|ctx| format!("## Skill Expansion\n{ctx}"))
                    .unwrap_or_default(),
            )
            .push_ephemeral(
                "post_compaction_fragment",
                post_compaction_fragment.unwrap_or_default(),
            )
            .with_provider_context_refs(
                provider_context
                    .as_ref()
                    .map(|ctx| ctx.context_refs.clone())
                    .unwrap_or_default(),
            )
            .build();
        if let Some(store) = self.store().map(Arc::clone) {
            let stable_hash = prompt_assembly.stable_hash.clone();
            let ephemeral_hash = prompt_assembly.ephemeral_hash.clone();
            let segment_order = prompt_assembly.segment_order.clone();
            let provider_context_refs = prompt_assembly.provider_context_refs.clone();
            let prior_stable_hash = existing_runtime
                .as_ref()
                .and_then(|runtime| runtime.prompt_snapshot_hash.clone());
            let frozen_workspace_prompt = workspace_prompt.clone();
            let frozen_provider_system_prompt = provider_system_prompt.clone();
            let _ = crate::agent::mutate_thread_runtime(&store, thread_id, |runtime| {
                if prompt_settings.session_freeze_enabled {
                    runtime.frozen_workspace_prompt = runtime
                        .frozen_workspace_prompt
                        .clone()
                        .or(frozen_workspace_prompt.clone());
                    runtime.frozen_provider_system_prompt = runtime
                        .frozen_provider_system_prompt
                        .clone()
                        .or(frozen_provider_system_prompt.clone());
                }
                runtime.prompt_snapshot_hash = Some(stable_hash.clone());
                runtime.ephemeral_overlay_hash = Some(ephemeral_hash.clone());
                runtime.prompt_segment_order = segment_order.clone();
                runtime.provider_context_refs = provider_context_refs.clone();
            })
            .await;
            if let Some(previous) = prior_stable_hash
                && previous != stable_hash
            {
                tracing::info!(
                    thread = %thread_id,
                    previous_stable_hash = %previous,
                    new_stable_hash = %stable_hash,
                    "Stable prompt hash changed; cache-bust event recorded"
                );
            }
        }

        // Capture the routed model name for cost tracking, thinking config, etc.
        // When smart routing selects the cheap model, this differs from
        // self.llm().active_model_name() (which always returns the primary).
        let routed_model_name = routed_llm.active_model_name();

        let mut reasoning = Reasoning::new(routed_llm, self.safety().clone())
            .with_channel(message.channel.clone())
            .with_model_name(routed_model_name.clone())
            .with_cheap_model_name(
                self.deps
                    .cheap_llm
                    .as_ref()
                    .map(|llm| llm.active_model_name()),
            )
            .with_model_guidance_enabled(self.config.model_guidance_enabled)
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

        if !prompt_assembly.stable_snapshot.trim().is_empty() {
            reasoning = reasoning.with_system_prompt(prompt_assembly.stable_snapshot.clone());
        }
        let prompt_context_documents = prompt_assembly.ephemeral_documents.clone();

        // Build context with messages that we'll mutate during the loop
        let mut context_messages = initial_messages;

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

            // Interrupts are cooperative at iteration boundaries. We check
            // before each new LLM/tool step and after responses return, but we
            // intentionally do not try to cancel an in-flight provider call.
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
                        .with_tools(
                            self.tools()
                                .tool_definitions_for_capabilities(None, None, None)
                                .await,
                        );

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
                    routed_allowed_tools,
                    routed_allowed_skills,
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
                                    return Ok(AgenticLoopResult::Response(text));
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
                                        return Ok(self.agentic_result_from_text(
                                            synthesis_streamed_text,
                                            synthesized,
                                        ));
                                    }
                                    RespondResult::Text(text) => {
                                        tracing::warn!(
                                            finish_reason = ?synthesis_finish_reason,
                                            text_len = text.len(),
                                            "Tool-phase synthesis produced non-final text"
                                        );
                                        return Ok(AgenticLoopResult::Response(
                                            TOOL_PHASE_FINALIZATION_FAILURE_RESPONSE.to_string(),
                                        ));
                                    }
                                    RespondResult::ToolCalls { .. } => {
                                        tracing::warn!(
                                            "Tool-phase synthesis unexpectedly returned tool calls"
                                        );
                                        return Ok(AgenticLoopResult::Response(
                                            TOOL_PHASE_FINALIZATION_FAILURE_RESPONSE.to_string(),
                                        ));
                                    }
                                }
                            }
                            ToolPhaseTextOutcome::PrimaryFinalText => {
                                return Ok(
                                    self.agentic_result_from_text(phase_one_streamed_text, text)
                                );
                            }
                            ToolPhaseTextOutcome::PrimaryNeedsFinalization => {
                                return self
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
                                    .await;
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

                    return Ok(self.agentic_result_from_text(phase_one_streamed_text, text));
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

                        return self
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
                            .await;
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
                                        !sess.is_tool_auto_approved_for_channel(
                                            &message.channel,
                                            &tc.name,
                                        )
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

                    let mut parallel_safe = runnable.len() > 1;
                    if parallel_safe {
                        for (_, tc) in &runnable {
                            if tc.name == crate::tools::builtin::advisor::ADVISOR_TOOL_NAME {
                                parallel_safe = false;
                                break;
                            }
                            match self.tools().tool_descriptor(&tc.name).await {
                                Some(descriptor) if descriptor.metadata.parallel_safe => {}
                                _ => {
                                    parallel_safe = false;
                                    break;
                                }
                            }
                        }
                    }

                    if !parallel_safe {
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

                            let result =
                                if tc.name == crate::tools::builtin::advisor::ADVISOR_TOOL_NAME {
                                    self.execute_consult_advisor_call(
                                        tc,
                                        &context_messages,
                                        advisor_call_budget.as_ref(),
                                    )
                                    .await
                                } else {
                                    self.execute_chat_tool(&tc.name, &tc.arguments, &job_ctx)
                                        .await
                                };

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
                            if tc.name == crate::tools::builtin::advisor::ADVISOR_TOOL_NAME {
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
                                    .execute_consult_advisor_call(
                                        tc,
                                        &context_messages,
                                        advisor_call_budget.as_ref(),
                                    )
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
                                continue;
                            }

                            let pf_idx = *pf_idx;
                            let tools = self.tools().clone();
                            let safety = self.safety().clone();
                            let channels = self.channels.clone();
                            let job_ctx = job_ctx.clone();
                            let tc = tc.clone();
                            let channel = message.channel.clone();
                            let metadata = message.metadata.clone();
                            let main_tool_profile = self.config.main_tool_profile;

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
                                    ToolExecutionLane::Chat,
                                    main_tool_profile,
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
                                if tc.name != crate::tools::builtin::advisor::ADVISOR_TOOL_NAME {
                                    advisor_state.real_tool_result_count += 1;
                                    advisor_state.last_failure = Some(AdvisorFailureContext {
                                        tool_name: tc.name.clone(),
                                        message: error_msg.clone(),
                                        signature: Some(tool_call_signature(std::slice::from_ref(
                                            &tc,
                                        ))),
                                        checkpoint: advisor_state.real_tool_result_count,
                                    });
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
                                                        && let Ok(mut request) = serde_json::from_value::<
                                                            crate::agent::subagent_executor::SubagentSpawnRequest,
                                                        >(
                                                            req_val.clone()
                                                        ) {
                                                            request.principal_id.get_or_insert_with(|| identity.principal_id.clone());
                                                            request.actor_id.get_or_insert_with(|| identity.actor_id.clone());
                                                            if request.agent_workspace_id.is_none() {
                                                                request.agent_workspace_id = routed_agent_workspace_id;
                                                            }
                                                            request.normalize_strict(
                                                                routed_allowed_tools,
                                                                routed_allowed_skills,
                                                                self.config.subagent_tool_profile,
                                                            );
                                                            let pending_resume_request = if request.wait {
                                                                None
                                                            } else {
                                                                Some(request.clone())
                                                            };
                                                            let mut spawn_metadata = message.metadata.clone();
                                                            if !spawn_metadata.is_object() {
                                                                spawn_metadata = serde_json::json!({});
                                                            }
                                                            if let Some(metadata) = spawn_metadata.as_object_mut() {
                                                                metadata.insert("thread_id".to_string(), serde_json::json!(thread_id.to_string()));
                                                                metadata.insert("principal_id".to_string(), serde_json::json!(identity.principal_id.clone()));
                                                                metadata.insert("actor_id".to_string(), serde_json::json!(identity.actor_id.clone()));
                                                                metadata.insert("conversation_kind".to_string(), serde_json::json!(identity.conversation_kind.as_str()));
                                                            }
                                                            let exec_result = executor
                                                                .spawn(
                                                                    request,
                                                                    &message.channel,
                                                                    &spawn_metadata,
                                                                    &message.user_id,
                                                                    Some(&identity),
                                                                    Some(&thread_id.to_string()),
                                                                )
                                                                .await;

                                                            tool_result = match exec_result {
                                                                Ok(result) => {
                                                                    if let (Some(store), Some(request)) =
                                                                        (self.store(), pending_resume_request)
                                                                    {
                                                                        let _ = crate::agent::mutate_thread_runtime(
                                                                            store,
                                                                            thread_id,
                                                                            |runtime| {
                                                                                runtime.active_subagents.push(
                                                                                    crate::agent::PersistedSubagentState {
                                                                                        agent_id: result.agent_id,
                                                                                        name: request.name.clone(),
                                                                                        request,
                                                                                        channel_name: message.channel.clone(),
                                                                                        channel_metadata: spawn_metadata.clone(),
                                                                                        parent_user_id: message.user_id.clone(),
                                                                                        parent_identity: Some(identity.clone()),
                                                                                        parent_thread_id: thread_id.to_string(),
                                                                                        reinject_result: true,
                                                                                    },
                                                                                );
                                                                            },
                                                                        )
                                                                        .await;
                                                                    }
                                                                    Ok(
                                                                        serde_json::to_string(&result)
                                                                            .unwrap_or_default(),
                                                                    )
                                                                }
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
                                    && parsed.get("a2a_request").and_then(|v| v.as_bool())
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
                                    let target_model =
                                        parsed.get("target_model").and_then(|v| v.as_str());
                                    let target_allowed_tools = parsed
                                        .get("target_allowed_tools")
                                        .and_then(|v| serde_json::from_value(v.clone()).ok());
                                    let target_allowed_skills = parsed
                                        .get("target_allowed_skills")
                                        .and_then(|v| serde_json::from_value(v.clone()).ok());
                                    let timeout_secs = parsed
                                        .get("timeout_secs")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(120);
                                    let target_workspace_id = parsed
                                        .get("target_workspace_id")
                                        .and_then(|v| v.as_str())
                                        .and_then(|v| Uuid::parse_str(v).ok());
                                    let target_tool_profile = parsed
                                        .get("target_tool_profile")
                                        .and_then(|v| v.as_str())
                                        .map(|value| {
                                            value
                                                .parse::<crate::tools::ToolProfile>()
                                                .unwrap_or(self.config.subagent_tool_profile)
                                        });
                                    let parent_identity = message.resolved_identity();

                                    if let Some(executor) = self.subagent_executor.as_ref() {
                                        let mut request =
                                            crate::agent::subagent_executor::SubagentSpawnRequest {
                                                name: format!("a2a:{}", target_id),
                                                task: a2a_message.to_string(),
                                                system_prompt: Some(system_prompt.to_string()),
                                                task_packet: None,
                                                memory_mode: None,
                                                tool_mode: None,
                                                skill_mode: None,
                                                tool_profile: target_tool_profile,
                                                allowed_tools: target_allowed_tools,
                                                allowed_skills: target_allowed_skills,
                                                principal_id: Some(
                                                    parent_identity.principal_id.clone(),
                                                ),
                                                actor_id: Some(parent_identity.actor_id.clone()),
                                                agent_workspace_id: target_workspace_id,
                                                model: target_model.map(String::from),
                                                wait: true,
                                                timeout_secs: Some(timeout_secs),
                                            };
                                        request.normalize_strict(
                                            routed_allowed_tools,
                                            routed_allowed_skills,
                                            self.config.subagent_tool_profile,
                                        );
                                        let mut spawn_metadata = message.metadata.clone();
                                        if !spawn_metadata.is_object() {
                                            spawn_metadata = serde_json::json!({});
                                        }
                                        if let Some(metadata) = spawn_metadata.as_object_mut() {
                                            metadata.insert(
                                                "thread_id".to_string(),
                                                serde_json::json!(thread_id.to_string()),
                                            );
                                            metadata.insert(
                                                "principal_id".to_string(),
                                                serde_json::json!(
                                                    parent_identity.principal_id.clone()
                                                ),
                                            );
                                            metadata.insert(
                                                "actor_id".to_string(),
                                                serde_json::json!(parent_identity.actor_id.clone()),
                                            );
                                            metadata.insert(
                                                "conversation_kind".to_string(),
                                                serde_json::json!(
                                                    parent_identity.conversation_kind.as_str()
                                                ),
                                            );
                                        }

                                        let exec_result = executor
                                            .spawn(
                                                request,
                                                &message.channel,
                                                &spawn_metadata,
                                                &message.user_id,
                                                Some(&parent_identity),
                                                Some(&thread_id.to_string()),
                                            )
                                            .await;

                                        tool_result = match exec_result {
                                            Ok(result) => Ok(serde_json::json!({
                                                "a2a_response": true,
                                                "from_agent": target_id,
                                                "from_display_name": target_name,
                                                "response": result.response,
                                                "success": result.success,
                                                "iterations": result.iterations,
                                                "duration_ms": result.duration_ms,
                                            })
                                            .to_string()),
                                            Err(e) => Ok(serde_json::json!({
                                                "a2a_response": true,
                                                "from_agent": target_id,
                                                "error": e.to_string(),
                                                "success": false,
                                            })
                                            .to_string()),
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
                                    && let Some(auth_request) =
                                        check_auth_required(&tc.name, &tool_result)
                                {
                                    let auth_data = parse_auth_result(&tool_result);
                                    {
                                        let mut sess = session.lock().await;
                                        if let Some(thread) = sess.threads.get_mut(&thread_id) {
                                            thread.enter_auth_mode(
                                                auth_request.extension_name.clone(),
                                                auth_request.auth_mode,
                                            );
                                        }
                                    }
                                    let _ = self
                                        .channels
                                        .send_status(
                                            &message.channel,
                                            StatusUpdate::AuthRequired {
                                                extension_name: auth_request.extension_name,
                                                instructions: Some(auth_request.instructions.clone()),
                                                auth_url: auth_data.auth_url,
                                                setup_url: auth_data.setup_url,
                                                auth_mode: auth_data.auth_mode.unwrap_or_else(|| {
                                                    match auth_request.auth_mode {
                                                        crate::agent::session::PendingAuthMode::ManualToken => "manual_token".to_string(),
                                                        crate::agent::session::PendingAuthMode::ExternalOAuth => "oauth".to_string(),
                                                    }
                                                }),
                                                auth_status: auth_data
                                                    .auth_status
                                                    .unwrap_or(auth_request.auth_status),
                                                shared_auth_provider: auth_data.shared_auth_provider,
                                                missing_scopes: auth_data.missing_scopes,
                                                thread_id: Some(thread_id.to_string()),
                                            },
                                            &message.metadata,
                                        )
                                        .await;
                                    deferred_auth = Some(auth_request.instructions);
                                }

                                let advisor_stop_after_result = if tc.name
                                    == crate::tools::builtin::advisor::ADVISOR_TOOL_NAME
                                {
                                    tool_result
                                        .as_ref()
                                        .ok()
                                        .and_then(|output| self.parse_advisor_envelope(output))
                                        .and_then(|envelope| envelope.advisor_decision)
                                } else {
                                    advisor_state.real_tool_result_count += 1;
                                    match &tool_result {
                                        Ok(output) => {
                                            if output.contains("\"success\":false")
                                                || output.contains("\"status\":\"error\"")
                                            {
                                                advisor_state.last_failure =
                                                    Some(AdvisorFailureContext {
                                                        tool_name: tc.name.clone(),
                                                        message: truncate_preview(output, 240),
                                                        signature: Some(tool_call_signature(
                                                            std::slice::from_ref(&tc),
                                                        )),
                                                        checkpoint: advisor_state
                                                            .real_tool_result_count,
                                                    });
                                            } else {
                                                advisor_state.last_failure = None;
                                            }
                                        }
                                        Err(error) => {
                                            advisor_state.last_failure =
                                                Some(AdvisorFailureContext {
                                                    tool_name: tc.name.clone(),
                                                    message: error.to_string(),
                                                    signature: Some(tool_call_signature(
                                                        std::slice::from_ref(&tc),
                                                    )),
                                                    checkpoint: advisor_state
                                                        .real_tool_result_count,
                                                });
                                        }
                                    }
                                    None
                                };

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
                                if let Some(decision) = advisor_stop_after_result.as_ref() {
                                    self.apply_advisor_stop_directive(
                                        decision,
                                        last_call_signature,
                                        &mut advisor_state,
                                        &mut context_messages,
                                        &mut last_call_signature,
                                        &mut consecutive_same_calls,
                                    );
                                }
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
}
