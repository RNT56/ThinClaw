use super::*;

pub(super) struct PreparedPromptContext {
    pub(super) identity: crate::identity::ResolvedIdentity,
    pub(super) routed_agent: Option<crate::agent::agent_router::AgentWorkspace>,
    pub(super) routed_agent_workspace_id: Option<Uuid>,
    pub(super) routed_allowed_tools: Option<Vec<String>>,
    pub(super) routed_allowed_skills: Option<Vec<String>>,
    pub(super) active_skills: Vec<crate::skills::LoadedSkill>,
    pub(super) provider_tool_extensions: Vec<String>,
    pub(super) reasoning: Reasoning,
    pub(super) prompt_context_documents: Vec<String>,
}

impl Agent {
    pub(super) async fn prepare_prompt_context(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> PreparedPromptContext {
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
            .and_then(|agent| agent.allowed_tools.clone());
        let routed_allowed_skills = routed_agent
            .as_ref()
            .and_then(|agent| agent.allowed_skills.clone());
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
                let sanitized = sanitize_project_context_for_channel(
                    &prompt,
                    prompt_settings.project_context_max_tokens,
                    Some(&message.channel),
                    self.deps.safety.redact_pii_in_prompts(),
                );
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
            .select_active_skills(&message.content, routed_allowed_skills.as_deref())
            .await;

        // Collect the full skill directory (all loaded skills, not just matched ones).
        // This powers the always-on ## Skills section so the agent always knows
        // what skills are installed, even when none keyword-matched this message.
        let all_skills = self
            .collect_all_skills(routed_allowed_skills.as_deref())
            .await;

        let skill_index_context = render_available_skill_index(
            &all_skills
                .iter()
                .map(|(name, description)| thinclaw_agent::ports::SkillSummary {
                    name: name.clone(),
                    version: String::new(),
                    description: description.clone(),
                    trust: String::new(),
                    path: None,
                })
                .collect::<Vec<_>>(),
        );

        for skill in &active_skills {
            tracing::info!(
                skill_name = skill.name(),
                skill_version = skill.version(),
                trust = %skill.trust,
                "Skill activated"
            );
        }
        let active_skill_context = render_active_skill_block(
            &active_skills
                .iter()
                .map(crate::agent::skill_context_store::skill_summary)
                .collect::<Vec<_>>(),
        );

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
            .compact_block_for_channel(Some(&message.channel))
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
        let sanitize_prompt_segment = |segment: &str, content: String| {
            let sanitized = sanitize_project_context_for_channel(
                &content,
                prompt_settings.project_context_max_tokens,
                Some(&message.channel),
                self.deps.safety.redact_pii_in_prompts(),
            );
            if sanitized.was_truncated {
                tracing::info!(
                    thread = %thread_id,
                    segment,
                    "Prompt context segment was truncated to fit prompt.project_context_max_tokens"
                );
            }
            for pattern in &sanitized.warning_patterns {
                tracing::warn!(
                    thread = %thread_id,
                    segment,
                    pattern = %pattern,
                    "Suspicious prompt context content detected during prompt assembly"
                );
            }
            sanitized.content
        };
        let provider_system_prompt = provider_system_prompt
            .map(|prompt| sanitize_prompt_segment("provider_system_prompt", prompt));
        let skill_index_context = skill_index_context
            .map(|ctx| sanitize_prompt_segment("skills_index", render_skill_index_context(&ctx)));
        let active_skill_context = active_skill_context
            .map(|ctx| sanitize_prompt_segment("active_skills", render_active_skill_context(&ctx)));
        let provider_recall_context = provider_context.as_ref().map(|ctx| {
            sanitize_prompt_segment(
                "provider_recall",
                format!("## External Memory Recall\n{}", ctx.rendered_context),
            )
        });
        let linked_recall_context = linked_recall_block.as_ref().map(|block| {
            sanitize_prompt_segment("linked_recall", format!("## Linked Recall\n{block}"))
        });
        let channel_formatting_context = active_channel_hint.as_ref().map(|hints| {
            sanitize_prompt_segment(
                "channel_formatting_hints",
                format!("## Platform Formatting ({})\n{}", message.channel, hints),
            )
        });
        let personality_overlay_context = active_personality_overlay.as_ref().map(|overlay| {
            sanitize_prompt_segment(
                "personality_overlay",
                format!("## Temporary Personality\n\n{overlay}"),
            )
        });
        let post_compaction_fragment = post_compaction_fragment
            .map(|fragment| sanitize_prompt_segment("post_compaction_fragment", fragment));
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
        let prompt_assembly = assemble_dispatcher_prompt_materials(&DispatcherPromptMaterials {
            workspace_prompt: workspace_prompt.clone(),
            provider_system_prompt: provider_system_prompt.clone(),
            skill_index_context,
            provider_recall_context,
            linked_recall_context,
            channel_formatting_context,
            personality_overlay_context,
            runtime_capability_hint,
            active_skill_context,
            post_compaction_fragment,
            provider_context_refs: provider_context
                .as_ref()
                .map(|ctx| ctx.context_refs.clone())
                .unwrap_or_default(),
        });
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

        PreparedPromptContext {
            identity,
            routed_agent,
            routed_agent_workspace_id,
            routed_allowed_tools,
            routed_allowed_skills,
            active_skills,
            provider_tool_extensions,
            reasoning,
            prompt_context_documents,
        }
    }
}
