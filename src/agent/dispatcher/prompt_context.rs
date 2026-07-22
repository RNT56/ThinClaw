use super::*;

fn frozen_prompt_contract_version(
    explicit_version: Option<&str>,
    has_pre_v2_snapshot: bool,
) -> Option<&str> {
    explicit_version.or_else(|| has_pre_v2_snapshot.then_some("legacy"))
}

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
    pub(super) context_budget: PreparedContextBudget,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PreparedContextBudget {
    prompt_tokens: usize,
    tool_schema_tokens: usize,
    output_reserve_tokens: usize,
    safety_margin_percent: u8,
}

impl PreparedContextBudget {
    pub(super) fn history_token_limit(self, context_window_tokens: usize) -> usize {
        let safety_margin =
            context_window_tokens.saturating_mul(self.safety_margin_percent as usize) / 100;
        context_window_tokens
            .saturating_sub(self.prompt_tokens)
            .saturating_sub(self.tool_schema_tokens)
            .saturating_sub(self.output_reserve_tokens)
            .saturating_sub(safety_margin)
    }
}

impl Agent {
    pub(super) async fn prepare_prompt_context(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<PreparedPromptContext, Error> {
        let identity = message.resolved_identity();

        // Identity is canonicalized once at ingress. Never re-derive group
        // privacy policy from transport-specific metadata here.
        let is_group_chat = identity.conversation_kind == crate::identity::ConversationKind::Group;

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
        let effective_workspace = self.workspace().map(|base_workspace| {
            Arc::new(base_workspace.scoped_clone(
                identity.principal_id.clone(),
                routed_agent_workspace_id.or(base_workspace.agent_id()),
            ))
        });
        if let Some(workspace) = effective_workspace.as_ref()
            && let Err(error) = workspace.seed_if_empty(None, None).await
        {
            tracing::warn!(
                principal_id = %identity.principal_id,
                error = %error,
                "Failed to seed the principal workspace before prompt assembly"
            );
        }
        if identity.conversation_kind == crate::identity::ConversationKind::Direct
            && identity.actor_id == identity.principal_id
            && let Some(workspace) = effective_workspace.as_ref()
        {
            match workspace
                .migrate_legacy_owner_knowledge(&identity.actor_id)
                .await
            {
                Ok(migrated) if migrated > 0 => tracing::info!(
                    principal_id = %identity.principal_id,
                    actor_id = %identity.actor_id,
                    migrated,
                    "Migrated legacy root knowledge into the canonical actor namespace"
                ),
                Err(error) => tracing::warn!(
                    principal_id = %identity.principal_id,
                    actor_id = %identity.actor_id,
                    error = %error,
                    "Could not migrate legacy root knowledge; legacy prompt fallback remains available"
                ),
                _ => {}
            }
        }
        // Independent store reads on the hot path of every message — fetch
        // them concurrently instead of paying their summed latency.
        let store_handle = self.store().map(Arc::clone);
        let (mut prompt_settings, existing_runtime) = tokio::join!(
            async {
                if let Some(store) = store_handle.as_ref() {
                    match store.get_all_settings(&identity.principal_id).await {
                        Ok(map) => crate::settings::Settings::from_db_map(&map).prompt,
                        Err(_) => crate::settings::PromptSettings::default(),
                    }
                } else {
                    crate::settings::PromptSettings::default()
                }
            },
            async {
                if let Some(store) = store_handle.as_ref() {
                    match crate::agent::load_thread_runtime(store, thread_id).await {
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
                }
            },
        );
        prompt_settings.normalize_runtime_bounds();

        // Load workspace system prompt (identity files: AGENTS.md, SOUL.md, etc.)
        // In group chats, MEMORY.md is excluded to prevent leaking personal context.
        let frozen_workspace_prompt = if prompt_settings.session_freeze_enabled {
            existing_runtime
                .as_ref()
                .and_then(|runtime| runtime.frozen_workspace_prompt.clone())
        } else {
            None
        };
        // Workspace/provider knowledge is mutable state, not a rollout
        // contract. Read it every turn so memory edits, policy changes, and day
        // rollovers become visible immediately. The persisted workspace block
        // is only a last-known-good fallback for a transient storage failure.
        let (mut workspace_prompt, workspace_loaded_live) =
            if let Some(ws) = effective_workspace.as_ref() {
                match ws
                    .system_prompt_for_identity(
                        Some(&identity),
                        &message.channel,
                        self.deps.safety.redact_pii_in_prompts(),
                    )
                    .await
                {
                    Ok(prompt) if !prompt.is_empty() => (Some(prompt), true),
                    Ok(_) => (None, true),
                    Err(e) => {
                        tracing::warn!(
                            thread = %thread_id,
                            error = %e,
                            "Could not refresh workspace system prompt; using last-known-good block"
                        );
                        (frozen_workspace_prompt, false)
                    }
                }
            } else {
                (frozen_workspace_prompt, false)
            };
        if let Some(agent_prompt) = routed_agent
            .as_ref()
            .and_then(|agent| agent.system_prompt.as_ref())
            // A fallback snapshot already contains the routed override from
            // the successful turn that produced it. Do not append it again.
            && (workspace_loaded_live || workspace_prompt.is_none())
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
                match sanitized.response() {
                    crate::agent::prompt_sanitation::InjectionResponse::DropSegment => {
                        for warning in &sanitized.warnings {
                            tracing::warn!(
                                thread = %thread_id,
                                pattern = %warning.pattern,
                                severity = ?warning.severity,
                                "Suspected prompt-injection content dropped from workspace prompt"
                            );
                        }
                        "[segment removed: suspected prompt-injection content]".to_string()
                    }
                    crate::agent::prompt_sanitation::InjectionResponse::WarnUser => {
                        for warning in &sanitized.warnings {
                            tracing::warn!(
                                thread = %thread_id,
                                pattern = %warning.pattern,
                                severity = ?warning.severity,
                                "Suspicious project context content detected during prompt assembly"
                            );
                        }
                        sanitized.content
                    }
                    crate::agent::prompt_sanitation::InjectionResponse::LogOnly => {
                        for warning in &sanitized.warnings {
                            tracing::info!(
                                thread = %thread_id,
                                pattern = %warning.pattern,
                                severity = ?warning.severity,
                                "Low-severity project context content flagged during prompt assembly"
                            );
                        }
                        sanitized.content
                    }
                }
            })
            .filter(|prompt| !prompt.trim().is_empty());

        // Skill selection and the full skill directory are independent —
        // fetch both concurrently. (The directory powers the always-on
        // ## Skills section so the agent knows what is installed even when
        // nothing keyword-matched this message.)
        let (active_skills, all_skills) = tokio::join!(
            self.select_active_skills(&message.content, routed_allowed_skills.as_deref()),
            self.collect_all_skills(routed_allowed_skills.as_deref()),
        );

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

        let active_personality_overlay = {
            let session_guard = session.lock().await;
            session_guard
                .active_personality
                .as_ref()
                .map(personality::format_overlay)
        };
        // Channel metadata, linked recall, and the three learning-provider
        // fetches touch disjoint resources — run them all concurrently. The
        // provider calls each carry a settings load plus a health probe, so
        // overlapping them is the largest latency win in prompt preparation.
        let linked_recall_fut = async {
            if !matches!(
                identity.conversation_kind,
                crate::identity::ConversationKind::Direct
            ) {
                return None;
            }
            let store = self.store().map(Arc::clone)?;
            let mut conversations = store
                .list_actor_conversations_for_recall(
                    &identity.principal_id,
                    &identity.actor_id,
                    false,
                    8,
                )
                .await
                .ok()?;
            conversations.retain(|summary| summary.id != thread_id);
            crate::history::LinkedConversationRecall::new(
                identity.principal_id.clone(),
                identity.actor_id.clone(),
                false,
                conversations,
            )
            .compact_block_for_channel(Some(&message.channel))
        };
        let provider_fut = async {
            // Reuse the agent's shared orchestrator (and its warmed-up
            // MemoryProviderManager readiness cache + pooled HTTP client)
            // instead of constructing a fresh one on every prompt assembly —
            // this closure alone issues up to 3 provider calls per message.
            let Some(orchestrator) = self.learning_orchestrator() else {
                return (None, Vec::new(), None);
            };
            let provider_access = identity.access_context(message.channel.clone());
            let (provider_context, provider_tool_extensions, provider_system_prompt) = tokio::join!(
                orchestrator.prefetch_provider_context(&provider_access, &message.content, 6),
                orchestrator.provider_tool_extensions(&provider_access),
                orchestrator.provider_system_prompt_block(&provider_access),
            );
            (
                provider_context,
                provider_tool_extensions,
                provider_system_prompt,
            )
        };
        let (
            active_channel_names,
            active_channel_hint,
            linked_recall_block,
            (provider_context, provider_tool_extensions, provider_system_prompt),
        ) = tokio::join!(
            self.channels.channel_names(),
            self.channels.formatting_hints_for(&message.channel),
            linked_recall_fut,
            provider_fut,
        );
        // Reuse the runtime loaded at the top of this function instead of a
        // second identical DB read (it cannot change mid-preparation).
        let post_compaction_fragment = existing_runtime
            .as_ref()
            .and_then(|runtime| runtime.post_compaction_context.clone());
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
            match sanitized.response() {
                crate::agent::prompt_sanitation::InjectionResponse::DropSegment => {
                    for warning in &sanitized.warnings {
                        tracing::warn!(
                            thread = %thread_id,
                            segment,
                            pattern = %warning.pattern,
                            severity = ?warning.severity,
                            "Suspected prompt-injection content dropped from prompt segment"
                        );
                    }
                    "[segment removed: suspected prompt-injection content]".to_string()
                }
                crate::agent::prompt_sanitation::InjectionResponse::WarnUser => {
                    for warning in &sanitized.warnings {
                        tracing::warn!(
                            thread = %thread_id,
                            segment,
                            pattern = %warning.pattern,
                            severity = ?warning.severity,
                            "Suspicious prompt context content detected during prompt assembly"
                        );
                    }
                    sanitized.content
                }
                crate::agent::prompt_sanitation::InjectionResponse::LogOnly => {
                    for warning in &sanitized.warnings {
                        tracing::info!(
                            thread = %thread_id,
                            segment,
                            pattern = %warning.pattern,
                            severity = ?warning.severity,
                            "Low-severity prompt context content flagged during prompt assembly"
                        );
                    }
                    sanitized.content
                }
            }
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
        // format_overlay already emits the "## Temporary Personality" header.
        let personality_overlay_context = active_personality_overlay
            .as_ref()
            .map(|overlay| sanitize_prompt_segment("personality_overlay", overlay.clone()));
        let post_compaction_fragment = post_compaction_fragment
            .map(|fragment| sanitize_prompt_segment("post_compaction_fragment", fragment));
        let workspace_evidence_context = if let Some(workspace) = effective_workspace.as_ref() {
            match workspace
                .untrusted_context_for_identity(
                    &identity,
                    &message.channel,
                    self.deps.safety.redact_pii_in_prompts(),
                )
                .await
            {
                Ok(Some(context)) if !context.trim().is_empty() => {
                    Some(sanitize_prompt_segment("workspace_evidence", context))
                }
                Ok(_) => None,
                Err(error) => {
                    tracing::warn!(
                        thread = %thread_id,
                        %error,
                        "Failed to load actor/group workspace evidence"
                    );
                    None
                }
            }
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
        let history_tokens = {
            let session = session.lock().await;
            session
                .threads
                .get(&thread_id)
                .map(|thread| {
                    thread
                        .messages()
                        .iter()
                        .map(crate::llm::ChatMessage::estimated_chars)
                        .sum::<usize>()
                        .div_ceil(4)
                })
                .unwrap_or_default()
        };
        let tool_schema_tokens = self
            .tools()
            .tool_definitions()
            .await
            .iter()
            .map(|tool| {
                (tool.name.chars().count()
                    + tool.description.chars().count()
                    + tool.parameters.to_string().chars().count())
                .div_ceil(4)
            })
            .sum();
        let routed_context_window_tokens = self
            .context_monitor_for_model(&routed_llm.active_model_name())
            .limit();
        prompt_settings.normalize_for_context_window(routed_context_window_tokens);
        let prompt_budget = thinclaw_llm_core::PromptBudget {
            context_window_tokens: routed_context_window_tokens,
            tool_schema_tokens,
            history_tokens,
            output_reserve_tokens: prompt_settings.output_reserve_tokens,
            safety_margin_percent: prompt_settings.safety_margin_percent,
            prompt_cap_tokens: Some(prompt_settings.max_total_tokens),
        };
        let prompt_materials = DispatcherPromptMaterials {
            workspace_prompt: workspace_prompt.clone(),
            workspace_evidence_context: workspace_evidence_context.clone(),
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
        };
        let prompt_source_segments =
            dispatcher_prompt_assembly(&prompt_materials).into_prompt_segments();
        let prompt_assembly = match assemble_dispatcher_prompt_materials_with_budget(
            &prompt_materials,
            prompt_budget,
        ) {
            Ok(assembly) => assembly,
            Err(error) => {
                tracing::error!(thread = %thread_id, %error, "Prompt V2 compilation failed; using a minimal bounded fallback");
                // Do not carry the oversized segment that caused the primary
                // compile to fail into the fallback. The immutable transcript
                // policy remains, while all mutable material is omitted for
                // this turn. A second failure is returned through the agent
                // error path rather than panicking the process.
                let fallback = DispatcherPromptMaterials::default();
                assemble_dispatcher_prompt_materials_with_budget(
                    &fallback,
                    thinclaw_llm_core::PromptBudget::default(),
                )
                .map_err(|fallback_error| crate::error::JobError::ContextError {
                    id: thread_id,
                    reason: format!(
                        "prompt compilation failed ({error}); minimal fallback also failed ({fallback_error})"
                    ),
                })?
            }
        };
        let context_budget = PreparedContextBudget {
            prompt_tokens: prompt_assembly.estimated_tokens,
            tool_schema_tokens,
            output_reserve_tokens: prompt_settings.output_reserve_tokens,
            safety_margin_percent: prompt_settings.safety_margin_percent,
        };
        let frozen_contract_version = existing_runtime.as_ref().and_then(|runtime| {
            frozen_prompt_contract_version(
                runtime.prompt_contract_version.as_deref(),
                runtime.prompt_snapshot_hash.is_some()
                    || runtime.frozen_workspace_prompt.is_some()
                    || runtime.frozen_provider_system_prompt.is_some(),
            )
        });
        let effective_rollout_mode = prompt_settings.rollout_mode.effective_for_session(
            prompt_settings.session_freeze_enabled,
            frozen_contract_version,
        );
        if matches!(
            effective_rollout_mode,
            crate::settings::PromptRolloutMode::Shadow
        ) {
            tracing::info!(
                thread = %thread_id,
                contract_version = %prompt_assembly.contract_version,
                manifest_digest = %prompt_assembly.manifest_digest,
                estimated_tokens = prompt_assembly.estimated_tokens,
                segment_count = prompt_assembly.manifest.len(),
                "Prompt V2 shadow compilation completed"
            );
        }
        if let Some(store) = self.store().map(Arc::clone) {
            let exact_v2_telemetry_is_deferred = matches!(
                effective_rollout_mode,
                crate::settings::PromptRolloutMode::V2
            );
            let stable_hash = prompt_assembly.stable_hash.clone();
            let ephemeral_hash = prompt_assembly.ephemeral_hash.clone();
            let segment_order = prompt_assembly.segment_order.clone();
            let provider_context_refs = prompt_assembly.provider_context_refs.clone();
            let active_contract_version = match effective_rollout_mode {
                crate::settings::PromptRolloutMode::V2 => prompt_assembly.contract_version.clone(),
                crate::settings::PromptRolloutMode::Legacy
                | crate::settings::PromptRolloutMode::Shadow => "legacy".to_string(),
            };
            let manifest_digest = prompt_assembly.manifest_digest.clone();
            let prior_stable_hash = existing_runtime
                .as_ref()
                .and_then(|runtime| runtime.prompt_snapshot_hash.clone());
            let frozen_workspace_prompt = workspace_prompt.clone();
            let frozen_provider_system_prompt = provider_system_prompt.clone();
            let _ = crate::agent::mutate_thread_runtime(&store, thread_id, |runtime| {
                if prompt_settings.session_freeze_enabled {
                    runtime.frozen_workspace_prompt = frozen_workspace_prompt.clone();
                    runtime.frozen_provider_system_prompt = frozen_provider_system_prompt.clone();
                } else {
                    runtime.frozen_workspace_prompt = None;
                    runtime.frozen_provider_system_prompt = None;
                }
                runtime.prompt_contract_version = Some(active_contract_version.clone());
                if !exact_v2_telemetry_is_deferred {
                    runtime.prompt_snapshot_hash = Some(stable_hash.clone());
                    runtime.ephemeral_overlay_hash = Some(ephemeral_hash.clone());
                    runtime.prompt_manifest_digest = Some(manifest_digest.clone());
                    runtime.prompt_segment_order = segment_order.clone();
                }
                runtime.provider_context_refs = provider_context_refs.clone();
            })
            .await;
            if !exact_v2_telemetry_is_deferred
                && let Some(previous) = prior_stable_hash
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

        let mut reasoning = Reasoning::new(routed_llm)
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

        let prompt_context_documents = match effective_rollout_mode {
            crate::settings::PromptRolloutMode::V2 => {
                reasoning = reasoning.with_prompt_contract(prompt_source_segments, prompt_budget);
                Vec::new()
            }
            crate::settings::PromptRolloutMode::Legacy
            | crate::settings::PromptRolloutMode::Shadow => {
                let system_prompt = prompt_assembly.stable_snapshot.clone();
                if !system_prompt.trim().is_empty() {
                    reasoning = reasoning.with_system_prompt(system_prompt);
                }
                prompt_assembly.legacy_ephemeral_documents.clone()
            }
        };

        Ok(PreparedPromptContext {
            identity,
            routed_agent,
            routed_agent_workspace_id,
            routed_allowed_tools,
            routed_allowed_skills,
            active_skills,
            provider_tool_extensions,
            reasoning,
            prompt_context_documents,
            context_budget,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{PreparedContextBudget, frozen_prompt_contract_version};

    #[test]
    fn pre_v2_frozen_session_stays_legacy_until_session_boundary() {
        assert_eq!(frozen_prompt_contract_version(None, true), Some("legacy"));
        assert_eq!(frozen_prompt_contract_version(None, false), None);
        assert_eq!(frozen_prompt_contract_version(Some("v2"), true), Some("v2"));
    }

    #[test]
    fn history_budget_reserves_prompt_tools_output_and_margin() {
        let budget = PreparedContextBudget {
            prompt_tokens: 1_000,
            tool_schema_tokens: 2_000,
            output_reserve_tokens: 4_000,
            safety_margin_percent: 10,
        };

        assert_eq!(budget.history_token_limit(32_000), 21_800);
        assert_eq!(budget.history_token_limit(4_000), 0);
    }
}
