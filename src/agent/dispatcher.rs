//! Tool dispatch logic for the agent.
//!
//! Extracted from `agent_loop.rs` to keep the core agentic tool execution
//! loop (LLM call -> tool calls -> repeat) in its own focused module.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::personality;
use crate::agent::prompt_assembly::PromptAssemblyV2;
use crate::agent::prompt_sanitation::sanitize_project_context;
use crate::agent::session::{PendingApproval, Session, ThreadState};
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::context::JobContext;
use crate::error::Error;
use crate::llm::{
    ChatMessage, Reasoning, ReasoningContext, RespondOutput, RespondResult, ToolDefinition,
    turn_analysis::TurnAwareness,
};
use crate::settings::AdvisorAutoEscalationMode;
use crate::tools::ToolExecutionLane;

// Helper functions extracted to dispatcher_helpers.rs
use super::dispatcher_helpers::compact_messages_for_retry;
// Re-export for external consumers (thread_ops.rs, etc.)
pub(crate) use super::dispatcher_helpers::{
    check_auth_required, execute_chat_tool_standalone, parse_auth_result, truncate_preview,
};

/// Result of the agentic loop execution.
pub(super) enum AgenticLoopResult {
    /// Completed with a response (needs to be sent to channel by caller).
    Response(String),
    /// Completed and response was already streamed to the channel via
    /// progressive edits (sendMessage + editMessageText).  Caller should
    /// NOT send it again — only persist and update thread state.
    Streamed(String),
    /// A tool requires approval before continuing.
    NeedApproval {
        /// The pending approval request to store.
        pending: PendingApproval,
    },
}

#[derive(Clone)]
struct LlmTurnOptions {
    force_text: bool,
    thinking: crate::llm::ThinkingConfig,
    context_documents: Vec<String>,
    stream_to_user: bool,
    emit_progress_status: bool,
    emit_thinking_status: bool,
    planning_mode: bool,
    max_output_tokens: Option<u32>,
}

struct LlmTurnResult {
    output: RespondOutput,
    streamed_text: bool,
}

const TOOL_PHASE_SYNTHESIS_PROMPT: &str = "Provide the final user-facing answer using the conversation and any tool results above. Do not call tools in this phase.";
const TOOL_PHASE_NO_TOOLS_SENTINEL: &str = "NO_TOOLS_NEEDED";
const TOOL_PHASE_PLANNING_PROMPT: &str = "Planner mode: decide which tools to call next. If tools are needed, call them directly. If no more tools are needed, do not draft the final answer here. Reply with only: NO_TOOLS_NEEDED";
const TOOL_PHASE_PLANNING_MAX_TOKENS: u32 = 512;
const STUCK_LOOP_FINALIZATION_PROMPT: &str = "STOP. You have called the same tool repeatedly without making progress. Do NOT call any more tools. Summarize what you have done so far and provide your best answer with the information you already have.";
const TOOL_PHASE_FINALIZATION_FAILURE_RESPONSE: &str =
    "I was unable to prepare the final answer cleanly. Please try again.";
const STUCK_LOOP_FINALIZATION_FAILURE_RESPONSE: &str =
    "I was unable to make further progress. Please try rephrasing your request.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolPhaseTextOutcome {
    NoToolsSignal,
    PrimaryFinalText,
    PrimaryNeedsFinalization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AdvisorAutoTrigger {
    ToolFailure,
    StuckLoop,
    VisionInput,
    LargeContext,
    ComplexFinalPass,
}

impl AdvisorAutoTrigger {
    fn as_str(self) -> &'static str {
        match self {
            Self::ToolFailure => "tool_failure",
            Self::StuckLoop => "stuck_loop",
            Self::VisionInput => "vision_input",
            Self::LargeContext => "large_context",
            Self::ComplexFinalPass => "complex_final_pass",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::ToolFailure => "a non-auth tool failed during the current turn",
            Self::StuckLoop => "the executor appears stuck in a repeated tool-call loop",
            Self::VisionInput => {
                "the request includes vision input and benefits from an early strategic check"
            }
            Self::LargeContext => {
                "the request carries a large context window and benefits from an early strategic check"
            }
            Self::ComplexFinalPass => {
                "this is a complex or planning-heavy turn and needs a final-pass advisor check before the answer is returned"
            }
        }
    }
}

#[derive(Debug, Clone)]
struct AdvisorFailureContext {
    tool_name: String,
    message: String,
    signature: Option<u64>,
    checkpoint: u32,
}

#[derive(Debug, Default)]
struct AdvisorTurnState {
    real_tool_result_count: u32,
    blocked_tool_signatures: HashSet<u64>,
    auto_consult_checkpoints: HashSet<String>,
    last_failure: Option<AdvisorFailureContext>,
}

impl AdvisorTurnState {
    fn checkpoint_for(&self, trigger: AdvisorAutoTrigger, detail: impl Into<String>) -> String {
        format!(
            "{}:{}:{}",
            trigger.as_str(),
            self.real_tool_result_count,
            detail.into()
        )
    }

    fn should_fire(&self, checkpoint: &str) -> bool {
        !self.auto_consult_checkpoints.contains(checkpoint)
    }

    fn mark_fired(&mut self, checkpoint: String) {
        self.auto_consult_checkpoints.insert(checkpoint);
    }
}

fn is_tool_phase_no_tools_signal(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed == TOOL_PHASE_NO_TOOLS_SENTINEL
        || trimmed.starts_with(TOOL_PHASE_NO_TOOLS_SENTINEL)
            && trimmed.len() <= TOOL_PHASE_NO_TOOLS_SENTINEL.len() + 4
            && trimmed[TOOL_PHASE_NO_TOOLS_SENTINEL.len()..]
                .chars()
                .all(|c| c.is_whitespace() || c.is_ascii_punctuation())
}

fn tool_phase_synthesis_enabled(
    runtime_status: Option<&crate::llm::runtime_manager::RuntimeStatus>,
    has_cheap_llm: bool,
    force_text: bool,
    has_available_tools: bool,
    override_active: bool,
) -> bool {
    let Some(runtime_status) = runtime_status else {
        return false;
    };

    !force_text
        && has_available_tools
        && has_cheap_llm
        && runtime_status.cheap_model.is_some()
        && !override_active
        && runtime_status.routing_enabled
        && matches!(
            runtime_status.routing_mode,
            crate::settings::RoutingMode::CheapSplit
                | crate::settings::RoutingMode::AdvisorExecutor
        )
        && runtime_status.tool_phase_synthesis_enabled
}

fn tool_call_signature(tool_calls: &[crate::llm::ToolCall]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for tool_call in tool_calls {
        tool_call.name.hash(&mut hasher);
        tool_call.arguments.to_string().hash(&mut hasher);
    }
    hasher.finish()
}

fn is_complex_or_planning_turn(messages: &[ChatMessage]) -> bool {
    TurnAwareness::from_messages(messages).is_complex_or_planning_turn()
}

fn should_hold_complex_final_pass(
    runtime_status: Option<&crate::llm::runtime_manager::RuntimeStatus>,
    context_messages: &[ChatMessage],
    advisor_state: &AdvisorTurnState,
) -> bool {
    let Some(status) = runtime_status else {
        return false;
    };
    if !status.advisor_ready
        || status.advisor_auto_escalation_mode != AdvisorAutoEscalationMode::RiskAndComplexFinal
    {
        return false;
    }
    if !is_complex_or_planning_turn(context_messages) {
        return false;
    }
    let checkpoint =
        advisor_state.checkpoint_for(AdvisorAutoTrigger::ComplexFinalPass, "final_answer");
    advisor_state.should_fire(&checkpoint)
}

fn classify_tool_phase_text(
    text: &str,
    finish_reason: crate::llm::FinishReason,
) -> ToolPhaseTextOutcome {
    match finish_reason {
        crate::llm::FinishReason::Stop if is_tool_phase_no_tools_signal(text) => {
            ToolPhaseTextOutcome::NoToolsSignal
        }
        crate::llm::FinishReason::Stop => ToolPhaseTextOutcome::PrimaryFinalText,
        crate::llm::FinishReason::Length
        | crate::llm::FinishReason::Unknown
        | crate::llm::FinishReason::ContentFilter
        | crate::llm::FinishReason::ToolUse => ToolPhaseTextOutcome::PrimaryNeedsFinalization,
    }
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

    fn thinking_config_for_model(&self, model_name: &str) -> crate::llm::ThinkingConfig {
        let (enabled, budget) = self.config.resolve_thinking_for_model(model_name);
        if enabled {
            crate::llm::ThinkingConfig::Enabled {
                budget_tokens: budget,
            }
        } else {
            crate::llm::ThinkingConfig::Disabled
        }
    }

    fn build_turn_context(
        &self,
        context_messages: &[ChatMessage],
        available_tools: Vec<ToolDefinition>,
        thread_id: Uuid,
        options: &LlmTurnOptions,
    ) -> ReasoningContext {
        let mut messages = context_messages.to_vec();
        if options.planning_mode {
            messages.push(ChatMessage::system(TOOL_PHASE_PLANNING_PROMPT));
        }
        let mut context = ReasoningContext::new()
            .with_messages(messages)
            .with_context_documents(options.context_documents.clone())
            .with_tools(available_tools)
            .with_metadata({
                let mut metadata = std::collections::HashMap::new();
                metadata.insert("thread_id".to_string(), thread_id.to_string());
                metadata
            });
        context.force_text = options.force_text;
        context.thinking = options.thinking;
        context.max_output_tokens = options.max_output_tokens;
        context
    }

    fn agentic_result_from_text(&self, streamed_text: bool, text: String) -> AgenticLoopResult {
        if streamed_text {
            AgenticLoopResult::Streamed(text)
        } else {
            AgenticLoopResult::Response(text)
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn finalize_primary_text_only(
        &self,
        reasoning: &mut Reasoning,
        context_messages: &mut Vec<ChatMessage>,
        context_documents: &[String],
        thread_id: Uuid,
        message: &IncomingMessage,
        persistent_draft: &Arc<tokio::sync::Mutex<Option<crate::channels::DraftReplyState>>>,
        original_llm: &Arc<dyn crate::llm::LlmProvider>,
        last_applied_model_override: &mut Option<String>,
        fallback_response: &'static str,
    ) -> Result<AgenticLoopResult, Error> {
        let final_model_name = reasoning.current_llm().active_model_name();
        let final_turn = self
            .execute_llm_turn(
                reasoning,
                context_messages,
                Vec::new(),
                thread_id,
                message,
                persistent_draft,
                original_llm,
                last_applied_model_override,
                LlmTurnOptions {
                    force_text: true,
                    thinking: self.thinking_config_for_model(&final_model_name),
                    context_documents: context_documents.to_vec(),
                    stream_to_user: true,
                    emit_progress_status: true,
                    emit_thinking_status: true,
                    planning_mode: false,
                    max_output_tokens: None,
                },
            )
            .await?;

        let final_finish_reason = final_turn.output.finish_reason;
        let final_streamed_text = final_turn.streamed_text;

        match final_turn.output.result {
            RespondResult::Text(text) if final_finish_reason == crate::llm::FinishReason::Stop => {
                Ok(self.agentic_result_from_text(final_streamed_text, text))
            }
            RespondResult::Text(text) => {
                tracing::warn!(
                    finish_reason = ?final_finish_reason,
                    text_len = text.len(),
                    "Primary finalization produced non-final text; returning fallback response"
                );
                Ok(AgenticLoopResult::Response(fallback_response.to_string()))
            }
            RespondResult::ToolCalls { .. } => {
                tracing::warn!(
                    "Primary finalization unexpectedly returned tool calls; returning fallback response"
                );
                Ok(AgenticLoopResult::Response(fallback_response.to_string()))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_llm_turn(
        &self,
        reasoning: &mut Reasoning,
        context_messages: &mut Vec<ChatMessage>,
        available_tools: Vec<ToolDefinition>,
        thread_id: Uuid,
        message: &IncomingMessage,
        persistent_draft: &Arc<tokio::sync::Mutex<Option<crate::channels::DraftReplyState>>>,
        original_llm: &Arc<dyn crate::llm::LlmProvider>,
        last_applied_model_override: &mut Option<String>,
        options: LlmTurnOptions,
    ) -> Result<LlmTurnResult, Error> {
        let request_model_name = reasoning.current_llm().active_model_name();
        let identity = message.resolved_identity();
        let model_override_scope_key =
            crate::tools::builtin::llm_tools::model_override_scope_key_from_metadata(
                &message.metadata,
                Some(identity.principal_id.as_str()),
                Some(identity.actor_id.as_str()),
            );
        let mut context = self.build_turn_context(
            context_messages,
            available_tools.clone(),
            thread_id,
            &options,
        );

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
                model: request_model_name,
                system_message: system_msg,
                user_message: last_user_msg,
                message_count: context_messages.len(),
                user_id: message.user_id.clone(),
            };
            match self.hooks().run(&event).await {
                Ok(crate::hooks::HookOutcome::Continue { modified }) => {
                    if let Some(new_content) = modified {
                        if let Some(last) = context_messages
                            .iter_mut()
                            .rev()
                            .find(|m| m.role == crate::llm::Role::User)
                        {
                            last.content = new_content;
                        }
                        context = self.build_turn_context(
                            context_messages,
                            available_tools.clone(),
                            thread_id,
                            &options,
                        );
                    }
                }
                Ok(crate::hooks::HookOutcome::Reject { reason }) => {
                    tracing::info!(reason = %reason, "BeforeLlmInput hook rejected LLM call");
                    return Err(crate::error::Error::Hook(
                        crate::hooks::HookError::Rejected {
                            reason: format!("BeforeLlmInput hook rejected: {}", reason),
                        },
                    ));
                }
                Err(crate::hooks::HookError::Rejected { reason }) => {
                    tracing::info!(reason = %reason, "BeforeLlmInput hook rejected LLM call");
                    return Err(crate::error::Error::Hook(
                        crate::hooks::HookError::Rejected {
                            reason: format!("BeforeLlmInput hook rejected: {}", reason),
                        },
                    ));
                }
                Err(err) => {
                    tracing::warn!("BeforeLlmInput hook error (fail-open): {}", err);
                }
            }
        }

        let channel_stream_mode = if options.stream_to_user {
            self.channels.stream_mode(&message.channel).await
        } else {
            crate::channels::StreamMode::None
        };
        let native_streaming_available = reasoning.current_llm().supports_streaming_for_model(None);
        let use_streaming = options.stream_to_user
            && channel_stream_mode != crate::channels::StreamMode::None
            && native_streaming_available;
        if options.stream_to_user
            && channel_stream_mode != crate::channels::StreamMode::None
            && !native_streaming_available
        {
            tracing::debug!(
                channel = %message.channel,
                "Skipping progressive streaming because the selected provider is not native-streaming capable"
            );
        }

        if options.emit_progress_status {
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
        }

        let llm_start = std::time::Instant::now();
        let mut recovered_from_override_failure = false;
        let mut streamed_text = false;
        let output = loop {
            let attempt: Result<crate::llm::RespondOutput, crate::error::Error> = if use_streaming {
                let channels = Arc::clone(&self.channels);
                let channel_name = message.channel.clone();
                let mode = channel_stream_mode;

                let draft = {
                    let prev = persistent_draft.lock().await;
                    let mut new_draft = crate::channels::DraftReplyState::new(&channel_name);
                    if let Some(ref prev_draft) = *prev {
                        new_draft.message_id = prev_draft.message_id.clone();
                        new_draft.posted = prev_draft.posted;
                    }
                    Arc::new(tokio::sync::Mutex::new(new_draft))
                };

                let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel::<String>(64);
                let consumer_draft = Arc::clone(&draft);
                let consumer_channels = Arc::clone(&channels);
                let consumer_ch_name = message.channel.clone();
                let consumer_md = message.metadata.clone();
                let saw_event_chunk = Arc::new(AtomicBool::new(false));
                let consumer_saw_event_chunk = Arc::clone(&saw_event_chunk);

                let consumer_handle = tokio::spawn(async move {
                    while let Some(chunk) = chunk_rx.recv().await {
                        if mode == crate::channels::StreamMode::EventChunks {
                            consumer_saw_event_chunk.store(true, Ordering::Relaxed);
                            let _ = consumer_channels
                                .send_status(
                                    &consumer_ch_name,
                                    StatusUpdate::StreamChunk(chunk),
                                    &consumer_md,
                                )
                                .await;
                            continue;
                        }

                        let mut d = consumer_draft.lock().await;
                        let should_send = d.append(&chunk);
                        if should_send {
                            let display = match mode {
                                crate::channels::StreamMode::StatusLine => {
                                    let word_count = d.accumulated.split_whitespace().count();
                                    format!("✦ Generating... ({} words)", word_count)
                                }
                                _ => d.display_text(),
                            };

                            let mut send_draft =
                                crate::channels::DraftReplyState::new(&consumer_ch_name);
                            send_draft.accumulated = display;
                            send_draft.message_id = d.message_id.clone();
                            send_draft.posted = d.posted;

                            match consumer_channels
                                .send_draft(&consumer_ch_name, &send_draft, &consumer_md)
                                .await
                            {
                                Ok(msg_id) => d.mark_sent(msg_id),
                                Err(crate::error::ChannelError::MessageTooLong { .. }) => {
                                    tracing::info!(
                                        "Streaming overflow detected, will fall back to on_respond()"
                                    );
                                    d.overflow = true;
                                }
                                Err(e) => {
                                    tracing::debug!("Draft edit failed (non-fatal): {}", e);
                                }
                            }
                        }
                    }
                });

                let stream_result = reasoning
                    .respond_with_tools_streaming(&context, move |chunk: &str| {
                        let _ = chunk_tx.try_send(chunk.to_string());
                    })
                    .await;

                let _ = consumer_handle.await;

                if mode == crate::channels::StreamMode::EventChunks {
                    let marker = if stream_result.is_ok() {
                        "stream_complete"
                    } else {
                        "stream_error"
                    };
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::Status(marker.to_string()),
                            &message.metadata,
                        )
                        .await;
                }

                let was_streamed = {
                    let d = draft.lock().await;
                    if mode == crate::channels::StreamMode::EventChunks {
                        saw_event_chunk.load(Ordering::Relaxed)
                    } else if d.overflow {
                        if let Some(ref msg_id) = d.message_id {
                            tracing::info!(
                                msg_id = %msg_id,
                                "Deleting partial streaming message before fallback"
                            );
                            let _ = self
                                .channels
                                .delete_message(&message.channel, msg_id, &message.metadata)
                                .await;
                        }
                        false
                    } else if d.posted && !d.accumulated.is_empty() {
                        let mut final_draft =
                            crate::channels::DraftReplyState::new(&message.channel);
                        final_draft.accumulated = d.accumulated.clone();
                        final_draft.message_id = d.message_id.clone();
                        final_draft.posted = true;

                        let final_edit_ok = self
                            .channels
                            .send_draft(&message.channel, &final_draft, &message.metadata)
                            .await
                            .is_ok();

                        if !final_edit_ok {
                            tracing::warn!(
                                "Final streaming edit failed, falling back to on_respond()"
                            );
                            if let Some(ref msg_id) = d.message_id {
                                let _ = self
                                    .channels
                                    .delete_message(&message.channel, msg_id, &message.metadata)
                                    .await;
                            }
                        }
                        final_edit_ok
                    } else {
                        false
                    }
                };

                {
                    let d = draft.lock().await;
                    let mut persist = persistent_draft.lock().await;
                    *persist = Some(crate::channels::DraftReplyState {
                        message_id: d.message_id.clone(),
                        channel_id: d.channel_id.clone(),
                        accumulated: d.accumulated.clone(),
                        last_edit_at: d.last_edit_at,
                        posted: d.posted,
                        overflow: d.overflow,
                    });
                }

                match stream_result {
                    Ok(output) => {
                        streamed_text =
                            was_streamed && matches!(&output.result, RespondResult::Text(_));
                        Ok(output)
                    }
                    Err(e) => Err(e.into()),
                }
            } else {
                match reasoning.respond_with_tools(&context).await {
                    Ok(output) => Ok(output),
                    Err(crate::error::LlmError::ContextLengthExceeded { used, limit }) => {
                        tracing::warn!(
                            used,
                            limit,
                            "Context length exceeded, compacting messages and retrying"
                        );

                        *context_messages = compact_messages_for_retry(context_messages);

                        let retry_context = self.build_turn_context(
                            context_messages,
                            available_tools.clone(),
                            thread_id,
                            &options,
                        );

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
                            })
                    }
                    Err(e) => Err(e.into()),
                }
            };

            match attempt {
                Ok(output) => break output,
                Err(err) => {
                    if !recovered_from_override_failure
                        && let Some(ref override_lock) = self.deps.model_override
                        && let Some(failed_override) =
                            override_lock.get(&model_override_scope_key).await
                    {
                        override_lock.clear(&model_override_scope_key).await;
                        tracing::warn!(
                            model = %failed_override.model_spec,
                            error = %err,
                            "Runtime model override failed; resetting to previous provider and retrying once"
                        );
                        reasoning.swap_llm(original_llm.clone());
                        *last_applied_model_override = None;
                        context_messages.push(ChatMessage::system(format!(
                            "Runtime note: model override '{}' failed and has been reset to the previous working model. Do not retry this override in this conversation unless the user explicitly asks again. Error: {}",
                            failed_override.model_spec, err
                        )));
                        context = self.build_turn_context(
                            context_messages,
                            available_tools.clone(),
                            thread_id,
                            &options,
                        );
                        recovered_from_override_failure = true;
                        continue;
                    }
                    return Err(err);
                }
            }
        };

        let active_llm = reasoning.current_llm();
        let active_model_name = active_llm.active_model_name();
        let model_name = output
            .routed_model_name
            .clone()
            .unwrap_or_else(|| active_model_name.clone());

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
                Ok(crate::hooks::HookOutcome::Continue { .. }) => {}
                Ok(crate::hooks::HookOutcome::Reject { reason }) => {
                    tracing::info!(reason = %reason, "AfterLlmOutput hook rejected response");
                    let streamed_msg_id = if streamed_text {
                        persistent_draft
                            .lock()
                            .await
                            .as_ref()
                            .and_then(|draft| draft.message_id.clone())
                    } else {
                        None
                    };
                    if let Some(msg_id) = streamed_msg_id {
                        let _ = self
                            .channels
                            .delete_message(&message.channel, &msg_id, &message.metadata)
                            .await;
                    }
                    return Err(crate::error::Error::Hook(
                        crate::hooks::HookError::Rejected {
                            reason: format!("AfterLlmOutput hook rejected: {}", reason),
                        },
                    ));
                }
                Err(crate::hooks::HookError::Rejected { reason }) => {
                    tracing::info!(reason = %reason, "AfterLlmOutput hook rejected response");
                    let streamed_msg_id = if streamed_text {
                        persistent_draft
                            .lock()
                            .await
                            .as_ref()
                            .and_then(|draft| draft.message_id.clone())
                    } else {
                        None
                    };
                    if let Some(msg_id) = streamed_msg_id {
                        let _ = self
                            .channels
                            .delete_message(&message.channel, &msg_id, &message.metadata)
                            .await;
                    }
                    return Err(crate::error::Error::Hook(
                        crate::hooks::HookError::Rejected {
                            reason: format!("AfterLlmOutput hook rejected: {}", reason),
                        },
                    ));
                }
                Err(err) => {
                    tracing::warn!("AfterLlmOutput hook error (fail-open): {}", err);
                }
            }
        }

        // NOTE: Cost recording into CostTracker + CostGuard is handled
        // by the UsageTrackingProvider decorator that wraps the LLM.
        // We only need to check budget thresholds here for SSE alerts.
        tracing::debug!(
            "LLM call used {} input + {} output tokens",
            output.usage.input_tokens,
            output.usage.output_tokens,
        );
        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::Usage {
                    input_tokens: output.usage.input_tokens,
                    output_tokens: output.usage.output_tokens,
                    cost_usd: None,
                    model: output
                        .routed_model_name
                        .clone()
                        .or_else(|| Some(model_name.clone())),
                },
                &message.metadata,
            )
            .await;

        if let Some(ref policy_lock) = self.deps.routing_policy {
            let latency_ms = llm_start.elapsed().as_millis() as f64;
            if let Ok(mut policy) = policy_lock.write() {
                let latency_key = crate::llm::routing_policy::canonical_latency_key(&model_name);
                policy.record_latency(&latency_key, latency_ms);
            }
        }

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

        if options.emit_thinking_status
            && let Some(ref thinking_text) = output.thinking_content
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

        Ok(LlmTurnResult {
            output,
            streamed_text,
        })
    }

    async fn execute_consult_advisor_call(
        &self,
        tool_call: &crate::llm::ToolCall,
        context_messages: &[ChatMessage],
        advisor_call_budget: &crate::tools::builtin::advisor::AdvisorCallBudget,
    ) -> Result<String, Error> {
        let question = tool_call
            .arguments
            .get("question")
            .and_then(|value| value.as_str())
            .unwrap_or("(no question provided)");
        let context_summary = tool_call
            .arguments
            .get("context_summary")
            .and_then(|value| value.as_str());
        let envelope = self
            .run_advisor_consultation(
                question,
                context_summary,
                context_messages,
                advisor_call_budget,
                crate::tools::builtin::advisor::AdvisorConsultationMode::Manual,
                "manual consultation requested by the executor",
            )
            .await;
        Ok(self.serialize_advisor_envelope(&envelope))
    }

    fn serialize_advisor_envelope(
        &self,
        envelope: &crate::tools::builtin::advisor::AdvisorConsultationEnvelope,
    ) -> String {
        serde_json::to_string(envelope).unwrap_or_else(|error| {
            serde_json::json!({
                "status": "error",
                "mode": "manual",
                "reason": "failed to serialize advisor envelope",
                "code": "advisor_envelope_serialize_failed",
                "message": error.to_string(),
            })
            .to_string()
        })
    }

    async fn run_advisor_consultation(
        &self,
        question: &str,
        context_summary: Option<&str>,
        context_messages: &[ChatMessage],
        advisor_call_budget: &crate::tools::builtin::advisor::AdvisorCallBudget,
        mode: crate::tools::builtin::advisor::AdvisorConsultationMode,
        reason: &str,
    ) -> crate::tools::builtin::advisor::AdvisorConsultationEnvelope {
        let Some(runtime) = self.deps.llm_runtime.as_ref() else {
            return crate::tools::builtin::advisor::AdvisorConsultationEnvelope::error(
                mode,
                reason,
                "advisor_unavailable",
                "Advisor runtime is unavailable in this environment.",
            );
        };

        let Some(advisor_config) = runtime.advisor_config_for_messages(context_messages) else {
            let disabled_reason = runtime
                .status()
                .advisor_disabled_reason
                .unwrap_or_else(|| "Advisor lane is unavailable for this turn.".to_string());
            return crate::tools::builtin::advisor::AdvisorConsultationEnvelope::error(
                mode,
                reason,
                "advisor_unavailable",
                disabled_reason,
            );
        };

        if let Err(limit_message) = advisor_call_budget.try_consume() {
            tracing::warn!(
                advisor_target = %advisor_config.advisor_target,
                "Advisor call rejected: call budget exhausted"
            );
            return crate::tools::builtin::advisor::AdvisorConsultationEnvelope::error(
                mode,
                reason,
                "advisor_call_limit_reached",
                limit_message,
            );
        }

        let advisor_provider =
            match runtime.provider_handle_for_target(&advisor_config.advisor_target) {
                Ok(provider) => {
                    if let Some(tracker) = self.deps.cost_tracker.as_ref() {
                        Arc::new(crate::llm::usage_tracking::UsageTrackingProvider::new(
                            provider,
                            Arc::clone(tracker),
                            self.deps.store.clone(),
                            Some(Arc::clone(&self.deps.cost_guard)),
                        )) as Arc<dyn crate::llm::LlmProvider>
                    } else {
                        provider
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        advisor_target = %advisor_config.advisor_target,
                        error = %error,
                        "Failed to resolve advisor target"
                    );
                    return crate::tools::builtin::advisor::AdvisorConsultationEnvelope::error(
                        mode,
                        reason,
                        "advisor_unavailable",
                        format!(
                            "Advisor target '{}' could not be resolved: {}",
                            advisor_config.advisor_target, error
                        ),
                    );
                }
            };

        match crate::tools::builtin::advisor::execute_advisor_consultation(
            advisor_provider.as_ref(),
            &advisor_config,
            question,
            context_summary,
            context_messages,
        )
        .await
        {
            Ok(decision) => crate::tools::builtin::advisor::AdvisorConsultationEnvelope::ok(
                mode, reason, decision,
            ),
            Err(error) => {
                tracing::warn!(error = %error, "Advisor consultation failed");
                crate::tools::builtin::advisor::AdvisorConsultationEnvelope::error(
                    mode,
                    reason,
                    "advisor_consultation_failed",
                    format!(
                        "Advisor consultation failed: {}. Continue without advisor guidance.",
                        error
                    ),
                )
            }
        }
    }

    fn parse_advisor_envelope(
        &self,
        content: &str,
    ) -> Option<crate::tools::builtin::advisor::AdvisorConsultationEnvelope> {
        serde_json::from_str(content).ok()
    }

    fn apply_advisor_stop_directive(
        &self,
        decision: &crate::tools::builtin::advisor::AdvisorDecision,
        blocked_signature: Option<u64>,
        advisor_state: &mut AdvisorTurnState,
        context_messages: &mut Vec<ChatMessage>,
        last_call_signature: &mut Option<u64>,
        consecutive_same_calls: &mut u32,
    ) {
        if decision.recommendation != crate::tools::builtin::advisor::AdvisorRecommendation::Stop {
            return;
        }

        if let Some(signature) = blocked_signature {
            advisor_state.blocked_tool_signatures.insert(signature);
            if *last_call_signature == Some(signature) {
                *last_call_signature = None;
                *consecutive_same_calls = 0;
            }
        }

        advisor_state.last_failure = None;
        let stop_reason = decision
            .stop_reason
            .as_deref()
            .unwrap_or(decision.summary.as_str());
        let directive = if blocked_signature.is_some() {
            format!(
                "Advisor STOP directive: {} Do not repeat the blocked tool-call pattern in this turn. Follow the revised plan, ask a narrow clarification, or return a bounded limitation.",
                stop_reason
            )
        } else {
            format!(
                "Advisor STOP directive: {} Follow the revised plan, ask a narrow clarification, or return a bounded limitation instead of retrying the same approach.",
                stop_reason
            )
        };
        context_messages.push(ChatMessage::system(directive));
    }

    fn build_auto_advisor_arguments(
        &self,
        trigger: AdvisorAutoTrigger,
        context_messages: &[ChatMessage],
        advisor_state: &AdvisorTurnState,
    ) -> (String, Option<String>) {
        let awareness = TurnAwareness::from_messages(context_messages);
        let last_user = awareness
            .last_user_objective
            .as_deref()
            .unwrap_or("No user objective found.");
        let base_context = awareness.context_snapshot(Some(advisor_state.real_tool_result_count));
        match trigger {
            AdvisorAutoTrigger::ToolFailure => {
                if let Some(failure) = advisor_state.last_failure.as_ref() {
                    (
                        format!(
                            "A tool failed during execution. How should I recover without repeating the mistake? Failed tool: {}. Failure: {}",
                            failure.tool_name, failure.message
                        ),
                        Some(format!(
                            "User objective: {}. {}",
                            last_user, base_context
                        )),
                    )
                } else {
                    (
                        "A tool failed during execution. What is the safest recovery plan?"
                            .to_string(),
                        Some(format!("User objective: {}. {}", last_user, base_context)),
                    )
                }
            }
            AdvisorAutoTrigger::StuckLoop => (
                "I appear to be repeating the same tool calls without making progress. What should I do next, and should I stop retrying this path?"
                    .to_string(),
                Some(format!("User objective: {}. {}", last_user, base_context)),
            ),
            AdvisorAutoTrigger::VisionInput => (
                "This turn includes image input. What strategy should I follow before taking action?"
                    .to_string(),
                Some(format!("User objective: {}. {}", last_user, base_context)),
            ),
            AdvisorAutoTrigger::LargeContext => (
                "This turn has a large context window. What is the safest high-level plan before I continue?"
                    .to_string(),
                Some(format!("User objective: {}. {}", last_user, base_context)),
            ),
            AdvisorAutoTrigger::ComplexFinalPass => (
                "Before I return the final answer on this complex turn, what corrections or caveats should I incorporate?"
                    .to_string(),
                Some(format!(
                    "User objective: {}. {}",
                    last_user, base_context
                )),
            ),
        }
    }

    fn next_auto_advisor_trigger(
        &self,
        runtime_status: Option<&crate::llm::runtime_manager::RuntimeStatus>,
        context_messages: &[ChatMessage],
        advisor_state: &AdvisorTurnState,
        consecutive_same_calls: u32,
        last_call_signature: Option<u64>,
    ) -> Option<(AdvisorAutoTrigger, String, Option<u64>)> {
        let status = runtime_status?;
        if !status.advisor_ready
            || status.advisor_auto_escalation_mode == AdvisorAutoEscalationMode::ManualOnly
        {
            return None;
        }
        let awareness = TurnAwareness::from_messages(context_messages);

        if let Some(failure) = advisor_state.last_failure.as_ref() {
            let checkpoint = advisor_state.checkpoint_for(
                AdvisorAutoTrigger::ToolFailure,
                failure.checkpoint.to_string(),
            );
            if advisor_state.should_fire(&checkpoint) {
                return Some((
                    AdvisorAutoTrigger::ToolFailure,
                    checkpoint,
                    failure.signature,
                ));
            }
        }

        if consecutive_same_calls >= 3
            && let Some(signature) = last_call_signature
        {
            let checkpoint = advisor_state.checkpoint_for(
                AdvisorAutoTrigger::StuckLoop,
                format!("{}:{}", signature, consecutive_same_calls),
            );
            if advisor_state.should_fire(&checkpoint) {
                return Some((AdvisorAutoTrigger::StuckLoop, checkpoint, Some(signature)));
            }
        }

        let vision_checkpoint =
            advisor_state.checkpoint_for(AdvisorAutoTrigger::VisionInput, "vision");
        if awareness.has_vision && advisor_state.should_fire(&vision_checkpoint) {
            return Some((AdvisorAutoTrigger::VisionInput, vision_checkpoint, None));
        }

        let large_context_checkpoint =
            advisor_state.checkpoint_for(AdvisorAutoTrigger::LargeContext, "large_context");
        if awareness.estimated_tokens >= 12_000
            && advisor_state.should_fire(&large_context_checkpoint)
        {
            return Some((
                AdvisorAutoTrigger::LargeContext,
                large_context_checkpoint,
                None,
            ));
        }

        None
    }

    #[allow(clippy::too_many_arguments)]
    async fn inject_auto_advisor_consultation(
        &self,
        trigger: AdvisorAutoTrigger,
        checkpoint: String,
        blocked_signature: Option<u64>,
        advisor_state: &mut AdvisorTurnState,
        context_messages: &mut Vec<ChatMessage>,
        session: &Arc<Mutex<Session>>,
        thread_id: Uuid,
        message: &IncomingMessage,
        advisor_call_budget: &crate::tools::builtin::advisor::AdvisorCallBudget,
        last_call_signature: &mut Option<u64>,
        consecutive_same_calls: &mut u32,
    ) -> Result<(), Error> {
        let (question, context_summary) =
            self.build_auto_advisor_arguments(trigger, context_messages, advisor_state);
        let tool_call = crate::llm::ToolCall {
            id: format!(
                "auto_consult_advisor_{}_{}",
                trigger.as_str(),
                advisor_state.real_tool_result_count
            ),
            name: crate::tools::builtin::advisor::ADVISOR_TOOL_NAME.to_string(),
            arguments: serde_json::json!({
                "question": question,
                "context_summary": context_summary,
            }),
        };
        context_messages.push(ChatMessage::assistant_with_tool_calls(
            Some(format!("Auto consulting advisor: {}", trigger.reason())),
            vec![tool_call.clone()],
        ));
        {
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id)
                && let Some(turn) = thread.last_turn_mut()
            {
                turn.record_tool_call(&tool_call.name, tool_call.arguments.clone());
            }
        }

        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::ToolStarted {
                    name: tool_call.name.clone(),
                    parameters: Some(tool_call.arguments.clone()),
                },
                &message.metadata,
            )
            .await;

        let envelope = self
            .run_advisor_consultation(
                tool_call
                    .arguments
                    .get("question")
                    .and_then(|value| value.as_str())
                    .unwrap_or("(no question provided)"),
                tool_call
                    .arguments
                    .get("context_summary")
                    .and_then(|value| value.as_str()),
                context_messages,
                advisor_call_budget,
                crate::tools::builtin::advisor::AdvisorConsultationMode::Auto,
                trigger.reason(),
            )
            .await;
        let serialized = self.serialize_advisor_envelope(&envelope);

        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::ToolCompleted {
                    name: tool_call.name.clone(),
                    success: matches!(
                        envelope.status,
                        crate::tools::builtin::advisor::AdvisorEnvelopeStatus::Ok
                    ),
                    result_preview: Some(truncate_preview(&serialized, 500)),
                },
                &message.metadata,
            )
            .await;
        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::ToolResult {
                    name: tool_call.name.clone(),
                    preview: serialized.clone(),
                },
                &message.metadata,
            )
            .await;

        {
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id)
                && let Some(turn) = thread.last_turn_mut()
            {
                turn.record_tool_result(serde_json::json!(serialized.clone()));
            }
        }
        context_messages.push(ChatMessage::tool_result(
            &tool_call.id,
            &tool_call.name,
            serialized,
        ));
        advisor_state.mark_fired(checkpoint);
        if let Some(decision) = envelope.advisor_decision.as_ref() {
            self.apply_advisor_stop_directive(
                decision,
                blocked_signature,
                advisor_state,
                context_messages,
                last_call_signature,
                consecutive_same_calls,
            );
        }
        Ok(())
    }

    /// Execute a tool for chat (without full job context).
    pub(super) async fn execute_chat_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
        job_ctx: &JobContext,
    ) -> Result<String, Error> {
        execute_chat_tool_standalone(
            self.tools(),
            self.safety(),
            tool_name,
            params,
            job_ctx,
            ToolExecutionLane::Chat,
            self.config.main_tool_profile,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use rust_decimal::Decimal;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use super::{
        AdvisorAutoTrigger, AdvisorFailureContext, AdvisorTurnState,
        STUCK_LOOP_FINALIZATION_PROMPT, TOOL_PHASE_NO_TOOLS_SENTINEL,
        TOOL_PHASE_PLANNING_MAX_TOKENS, TOOL_PHASE_PLANNING_PROMPT, TOOL_PHASE_SYNTHESIS_PROMPT,
        classify_tool_phase_text, is_tool_phase_no_tools_signal, should_hold_complex_final_pass,
        tool_phase_synthesis_enabled,
    };
    use crate::agent::agent_loop::{Agent, AgentDeps};
    use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
    use crate::agent::session::Session;
    use crate::channels::{
        Channel, ChannelManager, DraftReplyState, IncomingMessage, MessageStream, OutgoingResponse,
        StatusUpdate, StreamMode,
    };
    use crate::config::{AgentConfig, Config, SafetyConfig, SkillsConfig};
    use crate::context::ContextManager;
    use crate::error::{ChannelError, LlmError};
    use crate::hooks::HookRegistry;
    use crate::llm::{
        ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider,
        StreamSupport, ThinkingConfig, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
    };
    use crate::safety::SafetyLayer;
    use crate::settings::{
        AdvisorAutoEscalationMode, ProviderModelSlots, ProvidersSettings, RoutingMode,
        SecretsMasterKeySource, Settings,
    };
    use crate::tools::{ApprovalRequirement, Tool, ToolOutput, ToolRegistry};

    #[derive(Debug, Clone)]
    struct CapturedRequest {
        messages: Vec<ChatMessage>,
        context_documents: Vec<String>,
        tool_names: Vec<String>,
        max_tokens: Option<u32>,
        thinking: ThinkingConfig,
    }

    #[derive(Debug, Clone)]
    enum ScriptedResult {
        Text(String),
        ToolCalls {
            content: Option<String>,
            tool_calls: Vec<ToolCall>,
        },
    }

    #[derive(Debug, Clone)]
    struct ScriptedResponse {
        result: ScriptedResult,
        finish_reason: FinishReason,
        thinking_content: Option<String>,
    }

    impl ScriptedResponse {
        fn text(text: impl Into<String>, finish_reason: FinishReason) -> Self {
            Self {
                result: ScriptedResult::Text(text.into()),
                finish_reason,
                thinking_content: None,
            }
        }

        fn text_with_thinking(
            text: impl Into<String>,
            finish_reason: FinishReason,
            thinking: impl Into<String>,
        ) -> Self {
            Self {
                result: ScriptedResult::Text(text.into()),
                finish_reason,
                thinking_content: Some(thinking.into()),
            }
        }

        fn tool_calls(tool_calls: Vec<ToolCall>, finish_reason: FinishReason) -> Self {
            Self {
                result: ScriptedResult::ToolCalls {
                    content: None,
                    tool_calls,
                },
                finish_reason,
                thinking_content: None,
            }
        }
    }

    struct ScriptedLlm {
        model_name: String,
        responses: Mutex<VecDeque<ScriptedResponse>>,
        requests: Mutex<Vec<CapturedRequest>>,
        stream_support: StreamSupport,
    }

    impl ScriptedLlm {
        fn new(model_name: impl Into<String>, responses: Vec<ScriptedResponse>) -> Self {
            Self::with_stream_support(model_name, responses, StreamSupport::Simulated)
        }

        fn with_stream_support(
            model_name: impl Into<String>,
            responses: Vec<ScriptedResponse>,
            stream_support: StreamSupport,
        ) -> Self {
            Self {
                model_name: model_name.into(),
                responses: Mutex::new(VecDeque::from(responses)),
                requests: Mutex::new(Vec::new()),
                stream_support,
            }
        }

        async fn requests(&self) -> Vec<CapturedRequest> {
            self.requests.lock().await.clone()
        }

        async fn response_count(&self) -> usize {
            self.requests.lock().await.len()
        }

        async fn pop_response(&self) -> ScriptedResponse {
            self.responses
                .lock()
                .await
                .pop_front()
                .expect("scripted llm ran out of queued responses")
        }

        async fn record_request(
            &self,
            messages: Vec<ChatMessage>,
            context_documents: Vec<String>,
            tool_names: Vec<String>,
            max_tokens: Option<u32>,
            thinking: ThinkingConfig,
        ) {
            self.requests.lock().await.push(CapturedRequest {
                messages,
                context_documents,
                tool_names,
                max_tokens,
                thinking,
            });
        }
    }

    #[async_trait]
    impl LlmProvider for ScriptedLlm {
        fn model_name(&self) -> &str {
            &self.model_name
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        fn stream_support(&self) -> StreamSupport {
            self.stream_support
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            self.record_request(
                request.messages,
                request.context_documents,
                Vec::new(),
                request.max_tokens,
                request.thinking,
            )
            .await;

            let response = self.pop_response().await;
            match response.result {
                ScriptedResult::Text(content) => Ok(CompletionResponse {
                    content,
                    provider_model: Some(self.model_name.clone()),
                    cost_usd: Some(0.0),
                    thinking_content: response.thinking_content,
                    input_tokens: 10,
                    output_tokens: 5,
                    finish_reason: response.finish_reason,
                }),
                ScriptedResult::ToolCalls { .. } => {
                    panic!("complete() received a tool-call scripted response");
                }
            }
        }

        async fn complete_with_tools(
            &self,
            request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            self.record_request(
                request.messages,
                request.context_documents,
                request.tools.iter().map(|tool| tool.name.clone()).collect(),
                request.max_tokens,
                request.thinking,
            )
            .await;

            let response = self.pop_response().await;
            match response.result {
                ScriptedResult::Text(content) => Ok(ToolCompletionResponse {
                    content: Some(content),
                    provider_model: Some(self.model_name.clone()),
                    cost_usd: Some(0.0),
                    tool_calls: Vec::new(),
                    thinking_content: response.thinking_content,
                    input_tokens: 10,
                    output_tokens: 5,
                    finish_reason: response.finish_reason,
                }),
                ScriptedResult::ToolCalls {
                    content,
                    tool_calls,
                } => Ok(ToolCompletionResponse {
                    content,
                    provider_model: Some(self.model_name.clone()),
                    cost_usd: Some(0.0),
                    tool_calls,
                    thinking_content: response.thinking_content,
                    input_tokens: 10,
                    output_tokens: 5,
                    finish_reason: response.finish_reason,
                }),
            }
        }
    }

    #[derive(Debug, Clone)]
    enum RecordedChannelEvent {
        Status(StatusUpdate),
        Draft(String),
        Deleted,
        Response,
    }

    #[derive(Clone)]
    struct RecordingChannel {
        name: String,
        stream_mode: StreamMode,
        formatting_hints: Option<String>,
        events: Arc<Mutex<Vec<RecordedChannelEvent>>>,
    }

    impl RecordingChannel {
        fn new(name: impl Into<String>, stream_mode: StreamMode) -> Self {
            Self {
                name: name.into(),
                stream_mode,
                formatting_hints: None,
                events: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_formatting_hints(mut self, hints: impl Into<String>) -> Self {
            self.formatting_hints = Some(hints.into());
            self
        }

        async fn events(&self) -> Vec<RecordedChannelEvent> {
            self.events.lock().await.clone()
        }
    }

    #[async_trait]
    impl Channel for RecordingChannel {
        fn name(&self) -> &str {
            &self.name
        }

        async fn start(&self) -> Result<MessageStream, ChannelError> {
            Ok(Box::pin(futures::stream::empty()))
        }

        async fn respond(
            &self,
            _msg: &IncomingMessage,
            _response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            self.events
                .lock()
                .await
                .push(RecordedChannelEvent::Response);
            Ok(())
        }

        async fn send_status(
            &self,
            status: StatusUpdate,
            _metadata: &serde_json::Value,
        ) -> Result<(), ChannelError> {
            self.events
                .lock()
                .await
                .push(RecordedChannelEvent::Status(status));
            Ok(())
        }

        async fn send_draft(
            &self,
            draft: &DraftReplyState,
            _metadata: &serde_json::Value,
        ) -> Result<Option<String>, ChannelError> {
            self.events
                .lock()
                .await
                .push(RecordedChannelEvent::Draft(draft.accumulated.clone()));
            Ok(Some("draft-id".to_string()))
        }

        async fn delete_message(
            &self,
            _message_id: &str,
            _metadata: &serde_json::Value,
        ) -> Result<(), ChannelError> {
            self.events.lock().await.push(RecordedChannelEvent::Deleted);
            Ok(())
        }

        fn stream_mode(&self) -> StreamMode {
            self.stream_mode
        }

        fn formatting_hints(&self) -> Option<String> {
            self.formatting_hints.clone()
        }

        async fn health_check(&self) -> Result<(), ChannelError> {
            Ok(())
        }
    }

    struct TestTool {
        name: String,
        approval: ApprovalRequirement,
        result: String,
    }

    impl TestTool {
        fn new(
            name: impl Into<String>,
            approval: ApprovalRequirement,
            result: impl Into<String>,
        ) -> Self {
            Self {
                name: name.into(),
                approval,
                result: result.into(),
            }
        }
    }

    #[async_trait]
    impl Tool for TestTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "Test tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            })
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &crate::context::JobContext,
        ) -> Result<ToolOutput, crate::tools::ToolError> {
            Ok(ToolOutput::text(
                self.result.clone(),
                Duration::from_millis(1),
            ))
        }

        fn requires_sanitization(&self) -> bool {
            false
        }

        fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
            self.approval
        }
    }

    fn runtime_status(
        routing_mode: RoutingMode,
        cheap_model: Option<&str>,
        enabled: bool,
    ) -> crate::llm::runtime_manager::RuntimeStatus {
        crate::llm::runtime_manager::RuntimeStatus {
            revision: 1,
            last_error: None,
            primary_model: "openai_compatible/primary-model".to_string(),
            cheap_model: cheap_model.map(str::to_string),
            routing_enabled: true,
            routing_mode,
            tool_phase_synthesis_enabled: enabled,
            tool_phase_primary_thinking_enabled: true,
            primary_provider: Some("openai_compatible".to_string()),
            fallback_chain: Vec::new(),
            advisor_max_calls: 4,
            advisor_auto_escalation_mode: AdvisorAutoEscalationMode::RiskAndComplexFinal,
            advisor_escalation_prompt: None,
            advisor_ready: routing_mode == RoutingMode::AdvisorExecutor,
            advisor_disabled_reason: None,
            executor_target: (routing_mode == RoutingMode::AdvisorExecutor)
                .then_some("cheap".to_string()),
            advisor_target: (routing_mode == RoutingMode::AdvisorExecutor)
                .then_some("primary".to_string()),
        }
    }

    async fn make_runtime_manager(
        tool_phase_synthesis_enabled: bool,
        tool_phase_primary_thinking_enabled: bool,
    ) -> Arc<crate::llm::runtime_manager::LlmRuntimeManager> {
        make_runtime_manager_for_mode(
            tool_phase_synthesis_enabled,
            tool_phase_primary_thinking_enabled,
            RoutingMode::CheapSplit,
            3,
        )
        .await
    }

    async fn make_runtime_manager_for_mode(
        tool_phase_synthesis_enabled: bool,
        tool_phase_primary_thinking_enabled: bool,
        routing_mode: RoutingMode,
        advisor_max_calls: u32,
    ) -> Arc<crate::llm::runtime_manager::LlmRuntimeManager> {
        let mut settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("http://localhost:12345/v1".to_string()),
            selected_model: Some("gpt-5.4".to_string()),
            ..Settings::default()
        };
        settings.secrets.master_key_source = SecretsMasterKeySource::None;
        let config = Config::from_test_settings(&settings)
            .await
            .expect("config should load");

        let mut providers = ProvidersSettings {
            enabled: vec!["openai_compatible".to_string()],
            primary: Some("openai_compatible".to_string()),
            primary_model: Some("gpt-5.4".to_string()),
            cheap_model: Some("openai_compatible/gpt-5.4-mini".to_string()),
            smart_routing_enabled: true,
            routing_mode,
            tool_phase_synthesis_enabled,
            tool_phase_primary_thinking_enabled,
            advisor_max_calls,
            advisor_auto_escalation_mode: AdvisorAutoEscalationMode::RiskAndComplexFinal,
            ..ProvidersSettings::default()
        };
        providers.provider_models.insert(
            "openai_compatible".to_string(),
            ProviderModelSlots {
                primary: Some("gpt-5.4".to_string()),
                cheap: Some("gpt-5.4-mini".to_string()),
            },
        );

        crate::llm::runtime_manager::LlmRuntimeManager::new(
            config,
            providers,
            None,
            None,
            "test-user",
            None,
        )
        .expect("runtime manager should build")
    }

    async fn make_test_agent(
        primary_llm: Arc<dyn LlmProvider>,
        cheap_llm: Option<Arc<dyn LlmProvider>>,
        tools: Arc<ToolRegistry>,
        llm_runtime: Option<Arc<crate::llm::runtime_manager::LlmRuntimeManager>>,
        stream_mode: StreamMode,
        thinking_enabled: bool,
        max_tool_iterations: usize,
    ) -> (Agent, RecordingChannel) {
        let recording_channel = RecordingChannel::new("test", stream_mode);
        make_test_agent_with_channel(
            primary_llm,
            cheap_llm,
            tools,
            llm_runtime,
            recording_channel,
            thinking_enabled,
            max_tool_iterations,
        )
        .await
    }

    async fn make_test_agent_with_channel(
        primary_llm: Arc<dyn LlmProvider>,
        cheap_llm: Option<Arc<dyn LlmProvider>>,
        tools: Arc<ToolRegistry>,
        llm_runtime: Option<Arc<crate::llm::runtime_manager::LlmRuntimeManager>>,
        recording_channel: RecordingChannel,
        thinking_enabled: bool,
        max_tool_iterations: usize,
    ) -> (Agent, RecordingChannel) {
        let channels = Arc::new(ChannelManager::new());
        channels.add(Box::new(recording_channel.clone())).await;

        let deps = AgentDeps {
            store: None,
            llm: primary_llm,
            cheap_llm,
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
                redact_pii_in_prompts: true,
                smart_approval_mode: "off".to_string(),
                external_scanner_mode: "off".to_string(),
                external_scanner_path: None,
            })),
            tools,
            workspace: None,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: SkillsConfig::default(),
            hooks: Arc::new(HookRegistry::new()),
            cost_guard: Arc::new(CostGuard::new(CostGuardConfig::default())),
            sse_sender: None,
            agent_router: None,
            agent_registry: None,
            canvas_store: None,
            subagent_executor: None,
            cost_tracker: None,
            response_cache: None,
            llm_runtime,
            routing_policy: None,
            model_override: None,
            restart_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sandbox_children: None,
        };

        let agent = Agent::new(
            AgentConfig {
                name: "test-agent".to_string(),
                max_parallel_jobs: 1,
                job_timeout: Duration::from_secs(60),
                stuck_threshold: Duration::from_secs(60),
                repair_check_interval: Duration::from_secs(30),
                max_repair_attempts: 1,
                use_planning: false,
                session_idle_timeout: Duration::from_secs(300),
                allow_local_tools: false,
                max_cost_per_day_cents: None,
                max_actions_per_hour: None,
                max_tool_iterations,
                max_context_messages: 200,
                thinking_enabled,
                thinking_budget_tokens: 128,
                auto_approve_tools: false,
                main_tool_profile: crate::tools::ToolProfile::Standard,
                worker_tool_profile: crate::tools::ToolProfile::Restricted,
                subagent_tool_profile: crate::tools::ToolProfile::ExplicitOnly,
                subagent_transparency_level: "balanced".to_string(),
                model_thinking_overrides: HashMap::new(),
                workspace_mode: "unrestricted".to_string(),
                workspace_root: None,
                notify_channel: None,
                model_guidance_enabled: true,
                cli_skin: "cockpit".to_string(),
                personality_pack: "balanced".to_string(),
                persona_seed: "default".to_string(),
                checkpoints_enabled: true,
                max_checkpoints: 50,
                browser_backend: "chromium".to_string(),
                cloud_browser_provider: None,
            },
            deps,
            channels,
            None,
            None,
            None,
            Some(Arc::new(ContextManager::new(1))),
            None,
        );

        (agent, recording_channel)
    }

    async fn make_session_and_thread() -> (Arc<Mutex<Session>>, Uuid) {
        let session = Arc::new(Mutex::new(Session::new("user-1")));
        let thread_id = {
            let mut guard = session.lock().await;
            let thread = guard.create_thread();
            thread.start_turn("test request");
            thread.id
        };
        (session, thread_id)
    }

    async fn register_tool(
        registry: &Arc<ToolRegistry>,
        name: &str,
        approval: ApprovalRequirement,
        result: &str,
    ) {
        registry
            .register(Arc::new(TestTool::new(name, approval, result)))
            .await;
    }

    fn count_prompt(messages: &[ChatMessage], prompt: &str) -> usize {
        messages.iter().filter(|msg| msg.content == prompt).count()
    }

    fn contains_prompt(messages: &[ChatMessage], prompt: &str) -> bool {
        count_prompt(messages, prompt) > 0
    }

    fn tool_call(name: &str) -> ToolCall {
        ToolCall {
            id: format!("call_{}", name),
            name: name.to_string(),
            arguments: serde_json::json!({ "query": "demo" }),
        }
    }

    #[test]
    fn tool_phase_requires_cheap_split_with_real_cheap_model() {
        let status = runtime_status(RoutingMode::CheapSplit, Some("openai/gpt-5.4-mini"), true);

        assert!(tool_phase_synthesis_enabled(
            Some(&status),
            true,
            false,
            true,
            false,
        ));
    }

    #[test]
    fn tool_phase_is_disabled_without_cheap_model() {
        let status = runtime_status(RoutingMode::CheapSplit, None, true);

        assert!(!tool_phase_synthesis_enabled(
            Some(&status),
            true,
            false,
            true,
            false,
        ));
    }

    #[test]
    fn tool_phase_is_disabled_outside_cheap_split() {
        let status = runtime_status(RoutingMode::Policy, Some("openai/gpt-5.4-mini"), true);

        assert!(!tool_phase_synthesis_enabled(
            Some(&status),
            true,
            false,
            true,
            false,
        ));
    }

    #[test]
    fn complex_final_pass_only_holds_for_ready_advisor_complex_turns() {
        let status = runtime_status(
            RoutingMode::AdvisorExecutor,
            Some("openai/gpt-5.4-mini"),
            false,
        );
        let advisor_state = AdvisorTurnState::default();
        let messages = vec![ChatMessage::user(
            "Please design an architecture and implementation analysis for this migration.",
        )];

        assert!(should_hold_complex_final_pass(
            Some(&status),
            &messages,
            &advisor_state
        ));
        assert!(!should_hold_complex_final_pass(
            Some(&runtime_status(
                RoutingMode::CheapSplit,
                Some("openai/gpt-5.4-mini"),
                false
            )),
            &messages,
            &advisor_state
        ));
    }

    #[test]
    fn complex_final_pass_uses_full_turn_context_not_only_last_user_message() {
        let status = runtime_status(
            RoutingMode::AdvisorExecutor,
            Some("openai/gpt-5.4-mini"),
            false,
        );
        let advisor_state = AdvisorTurnState::default();
        let messages = vec![
            ChatMessage::user(
                "Please design the migration architecture and review the implementation risks.",
            ),
            ChatMessage::assistant_with_tool_calls(
                Some(
                    "I should inspect the current implementation before finalizing the design."
                        .to_string(),
                ),
                vec![tool_call("read_file"), tool_call("search_code")],
            ),
            ChatMessage::tool_result(
                "call_1",
                "read_file",
                "{\"status\":\"error\",\"message\":\"config missing\"}",
            ),
            ChatMessage::user("Continue."),
        ];

        assert!(should_hold_complex_final_pass(
            Some(&status),
            &messages,
            &advisor_state
        ));
    }

    #[tokio::test]
    async fn auto_trigger_prefers_recorded_tool_failure() {
        let primary = Arc::new(ScriptedLlm::new(
            "primary-model",
            vec![ScriptedResponse::text("done", FinishReason::Stop)],
        ));
        let (agent, _) = make_test_agent(
            primary.clone(),
            Some(primary),
            Arc::new(ToolRegistry::new()),
            None,
            StreamMode::None,
            true,
            4,
        )
        .await;
        let status = runtime_status(
            RoutingMode::AdvisorExecutor,
            Some("openai/gpt-5.4-mini"),
            false,
        );
        let mut advisor_state = AdvisorTurnState::default();
        advisor_state.real_tool_result_count = 2;
        advisor_state.last_failure = Some(AdvisorFailureContext {
            tool_name: "shell".to_string(),
            message: "command failed".to_string(),
            signature: Some(42),
            checkpoint: 2,
        });

        let trigger = agent.next_auto_advisor_trigger(
            Some(&status),
            &[ChatMessage::user("Debug the deployment failure.")],
            &advisor_state,
            0,
            None,
        );

        assert!(matches!(
            trigger,
            Some((AdvisorAutoTrigger::ToolFailure, _, Some(42)))
        ));
    }

    #[test]
    fn tool_phase_signal_requires_explicit_sentinel() {
        assert!(is_tool_phase_no_tools_signal("NO_TOOLS_NEEDED"));
        assert!(is_tool_phase_no_tools_signal("NO_TOOLS_NEEDED."));
        assert!(!is_tool_phase_no_tools_signal("No tools needed."));
        assert!(!is_tool_phase_no_tools_signal(
            "Here is the final answer for the user."
        ));
    }

    #[test]
    fn tool_phase_text_classification_prefers_finish_reason() {
        assert_eq!(
            classify_tool_phase_text("NO_TOOLS_NEEDED", FinishReason::Stop),
            super::ToolPhaseTextOutcome::NoToolsSignal
        );
        assert_eq!(
            classify_tool_phase_text("Primary answer", FinishReason::Stop),
            super::ToolPhaseTextOutcome::PrimaryFinalText
        );
        assert_eq!(
            classify_tool_phase_text("Truncated answer", FinishReason::Length),
            super::ToolPhaseTextOutcome::PrimaryNeedsFinalization
        );
    }

    #[tokio::test]
    async fn tool_phase_runs_cheap_synthesis_only_after_explicit_no_tools_signal() {
        let primary = Arc::new(ScriptedLlm::new(
            "primary-model",
            vec![
                ScriptedResponse::tool_calls(vec![tool_call("test_tool")], FinishReason::ToolUse),
                ScriptedResponse::text_with_thinking(
                    TOOL_PHASE_NO_TOOLS_SENTINEL,
                    FinishReason::Stop,
                    "hidden planner thought",
                ),
            ],
        ));
        let cheap = Arc::new(ScriptedLlm::with_stream_support(
            "cheap-model",
            vec![ScriptedResponse::text_with_thinking(
                "Cheap final answer",
                FinishReason::Stop,
                "visible synthesis thought",
            )],
            StreamSupport::Native,
        ));
        let runtime = make_runtime_manager(true, true).await;
        let tools = Arc::new(ToolRegistry::new());
        register_tool(
            &tools,
            "test_tool",
            ApprovalRequirement::Never,
            "tool output",
        )
        .await;
        let (agent, channel) = make_test_agent(
            primary.clone(),
            Some(cheap.clone()),
            tools,
            Some(runtime),
            StreamMode::EditFirst,
            true,
            10,
        )
        .await;
        let (session, thread_id) = make_session_and_thread().await;
        let message = IncomingMessage::new("test", "user-1", "help");

        let result = agent
            .run_agentic_loop(
                &message,
                session,
                thread_id,
                vec![ChatMessage::user("help")],
            )
            .await
            .expect("agentic loop should succeed");

        match result {
            super::AgenticLoopResult::Streamed(text) => assert_eq!(text, "Cheap final answer"),
            other => panic!(
                "expected streamed result, got {:?}",
                std::mem::discriminant(&other)
            ),
        }

        assert_eq!(cheap.response_count().await, 1);

        let primary_requests = primary.requests().await;
        assert_eq!(primary_requests.len(), 2);
        assert_eq!(
            primary_requests
                .iter()
                .map(|req| req.max_tokens)
                .collect::<Vec<_>>(),
            vec![
                Some(TOOL_PHASE_PLANNING_MAX_TOKENS),
                Some(TOOL_PHASE_PLANNING_MAX_TOKENS)
            ]
        );
        assert!(
            primary_requests
                .iter()
                .all(|req| count_prompt(&req.messages, TOOL_PHASE_PLANNING_PROMPT) == 1)
        );

        let cheap_requests = cheap.requests().await;
        assert_eq!(cheap_requests.len(), 1);
        assert_eq!(cheap_requests[0].tool_names.len(), 0);
        assert_eq!(cheap_requests[0].max_tokens, Some(4096));
        assert!(contains_prompt(
            &cheap_requests[0].messages,
            TOOL_PHASE_SYNTHESIS_PROMPT
        ));
        assert!(!contains_prompt(
            &cheap_requests[0].messages,
            TOOL_PHASE_PLANNING_PROMPT
        ));

        let events = channel.events().await;
        assert!(events.iter().any(|event| matches!(
            event,
            RecordedChannelEvent::Draft(text) if text.contains("Cheap final answer")
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            RecordedChannelEvent::Draft(text) if text.contains(TOOL_PHASE_NO_TOOLS_SENTINEL)
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            RecordedChannelEvent::Status(StatusUpdate::Thinking(text))
                if text.contains("hidden planner thought")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            RecordedChannelEvent::Status(StatusUpdate::Thinking(text))
                if text.contains("visible synthesis thought")
        )));
    }

    #[tokio::test]
    async fn tool_phase_direct_primary_text_skips_cheap_follow_up() {
        let primary = Arc::new(ScriptedLlm::new(
            "primary-model",
            vec![ScriptedResponse::text(
                "Primary final answer",
                FinishReason::Stop,
            )],
        ));
        let cheap = Arc::new(ScriptedLlm::new("cheap-model", vec![]));
        let runtime = make_runtime_manager(true, true).await;
        let tools = Arc::new(ToolRegistry::new());
        register_tool(
            &tools,
            "test_tool",
            ApprovalRequirement::Never,
            "tool output",
        )
        .await;
        let (agent, channel) = make_test_agent(
            primary.clone(),
            Some(cheap.clone()),
            tools,
            Some(runtime),
            StreamMode::None,
            true,
            10,
        )
        .await;
        let (session, thread_id) = make_session_and_thread().await;
        let message = IncomingMessage::new("test", "user-1", "help");

        let result = agent
            .run_agentic_loop(
                &message,
                session,
                thread_id,
                vec![ChatMessage::user("help")],
            )
            .await
            .expect("agentic loop should succeed");

        match result {
            super::AgenticLoopResult::Response(text) => assert_eq!(text, "Primary final answer"),
            other => panic!(
                "expected response result, got {:?}",
                std::mem::discriminant(&other)
            ),
        }

        assert_eq!(cheap.response_count().await, 0);
        let primary_requests = primary.requests().await;
        assert_eq!(primary_requests.len(), 1);
        assert_eq!(
            primary_requests[0].max_tokens,
            Some(TOOL_PHASE_PLANNING_MAX_TOKENS)
        );
        assert!(contains_prompt(
            &primary_requests[0].messages,
            TOOL_PHASE_PLANNING_PROMPT
        ));
        assert!(
            channel
                .events()
                .await
                .iter()
                .all(|event| !matches!(event, RecordedChannelEvent::Draft(_)))
        );
    }

    #[tokio::test]
    async fn truncated_planner_text_runs_primary_finalization_without_cheap() {
        let primary = Arc::new(ScriptedLlm::new(
            "primary-model",
            vec![
                ScriptedResponse::text("Truncated planner answer", FinishReason::Length),
                ScriptedResponse::text("Primary finalized answer", FinishReason::Stop),
            ],
        ));
        let cheap = Arc::new(ScriptedLlm::new("cheap-model", vec![]));
        let runtime = make_runtime_manager(true, true).await;
        let tools = Arc::new(ToolRegistry::new());
        register_tool(
            &tools,
            "test_tool",
            ApprovalRequirement::Never,
            "tool output",
        )
        .await;
        let (agent, _) = make_test_agent(
            primary.clone(),
            Some(cheap.clone()),
            tools,
            Some(runtime),
            StreamMode::None,
            true,
            10,
        )
        .await;
        let (session, thread_id) = make_session_and_thread().await;
        let message = IncomingMessage::new("test", "user-1", "help");

        let result = agent
            .run_agentic_loop(
                &message,
                session,
                thread_id,
                vec![ChatMessage::user("help")],
            )
            .await
            .expect("agentic loop should succeed");

        match result {
            super::AgenticLoopResult::Response(text) => {
                assert_eq!(text, "Primary finalized answer")
            }
            other => panic!(
                "expected response result, got {:?}",
                std::mem::discriminant(&other)
            ),
        }

        assert_eq!(cheap.response_count().await, 0);
        let primary_requests = primary.requests().await;
        assert_eq!(primary_requests.len(), 2);
        assert_eq!(
            primary_requests[0].max_tokens,
            Some(TOOL_PHASE_PLANNING_MAX_TOKENS)
        );
        assert_eq!(primary_requests[1].max_tokens, Some(4096));
        assert!(!contains_prompt(
            &primary_requests[1].messages,
            TOOL_PHASE_PLANNING_PROMPT
        ));
        assert!(primary_requests[1].tool_names.is_empty());
    }

    #[tokio::test]
    async fn force_text_iteration_does_not_run_tool_phase_synthesis() {
        let primary = Arc::new(ScriptedLlm::new(
            "primary-model",
            vec![ScriptedResponse::text(
                "Forced final answer",
                FinishReason::Stop,
            )],
        ));
        let cheap = Arc::new(ScriptedLlm::new("cheap-model", vec![]));
        let runtime = make_runtime_manager(true, true).await;
        let tools = Arc::new(ToolRegistry::new());
        register_tool(
            &tools,
            "test_tool",
            ApprovalRequirement::Never,
            "tool output",
        )
        .await;
        let (agent, _) = make_test_agent(
            primary.clone(),
            Some(cheap.clone()),
            tools,
            Some(runtime),
            StreamMode::None,
            true,
            1,
        )
        .await;
        let (session, thread_id) = make_session_and_thread().await;
        let message = IncomingMessage::new("test", "user-1", "help");

        let result = agent
            .run_agentic_loop(
                &message,
                session,
                thread_id,
                vec![ChatMessage::user("help")],
            )
            .await
            .expect("agentic loop should succeed");

        match result {
            super::AgenticLoopResult::Response(text) => assert_eq!(text, "Forced final answer"),
            other => panic!(
                "expected response result, got {:?}",
                std::mem::discriminant(&other)
            ),
        }

        assert_eq!(cheap.response_count().await, 0);
        let primary_requests = primary.requests().await;
        assert_eq!(primary_requests.len(), 1);
        assert!(primary_requests[0].tool_names.is_empty());
        assert!(!contains_prompt(
            &primary_requests[0].messages,
            TOOL_PHASE_PLANNING_PROMPT
        ));
        assert!(!contains_prompt(
            &primary_requests[0].messages,
            TOOL_PHASE_SYNTHESIS_PROMPT
        ));
        assert_eq!(primary_requests[0].max_tokens, Some(4096));
    }

    #[tokio::test]
    async fn stuck_loop_recovery_uses_primary_finalization_only() {
        let mut responses = Vec::new();
        for _ in 0..5 {
            responses.push(ScriptedResponse::tool_calls(
                vec![tool_call("loop_tool")],
                FinishReason::ToolUse,
            ));
        }
        responses.push(ScriptedResponse::text(
            "Recovered on primary",
            FinishReason::Stop,
        ));

        let primary = Arc::new(ScriptedLlm::new("primary-model", responses));
        let cheap = Arc::new(ScriptedLlm::new("cheap-model", vec![]));
        let runtime = make_runtime_manager(true, true).await;
        let tools = Arc::new(ToolRegistry::new());
        register_tool(
            &tools,
            "loop_tool",
            ApprovalRequirement::Never,
            "loop result",
        )
        .await;
        let (agent, _) = make_test_agent(
            primary.clone(),
            Some(cheap.clone()),
            tools,
            Some(runtime),
            StreamMode::None,
            true,
            20,
        )
        .await;
        let (session, thread_id) = make_session_and_thread().await;
        let message = IncomingMessage::new("test", "user-1", "help");

        let result = agent
            .run_agentic_loop(
                &message,
                session,
                thread_id,
                vec![ChatMessage::user("help")],
            )
            .await
            .expect("agentic loop should succeed");

        match result {
            super::AgenticLoopResult::Response(text) => assert_eq!(text, "Recovered on primary"),
            other => panic!(
                "expected response result, got {:?}",
                std::mem::discriminant(&other)
            ),
        }

        assert_eq!(cheap.response_count().await, 0);
        let primary_requests = primary.requests().await;
        assert_eq!(primary_requests.len(), 6);
        let final_request = primary_requests.last().expect("final request should exist");
        assert!(contains_prompt(
            &final_request.messages,
            STUCK_LOOP_FINALIZATION_PROMPT
        ));
        assert!(final_request.tool_names.is_empty());
        assert!(!contains_prompt(
            &final_request.messages,
            TOOL_PHASE_SYNTHESIS_PROMPT
        ));
    }

    #[tokio::test]
    async fn planner_thinking_toggle_only_changes_hidden_primary_phase() {
        async fn run_case(
            primary_planning_thinking_enabled: bool,
        ) -> (Vec<CapturedRequest>, Vec<CapturedRequest>) {
            let primary = Arc::new(ScriptedLlm::new(
                "primary-model",
                vec![ScriptedResponse::text(
                    TOOL_PHASE_NO_TOOLS_SENTINEL,
                    FinishReason::Stop,
                )],
            ));
            let cheap = Arc::new(ScriptedLlm::new(
                "cheap-model",
                vec![ScriptedResponse::text("Cheap reply", FinishReason::Stop)],
            ));
            let runtime = make_runtime_manager(true, primary_planning_thinking_enabled).await;
            let tools = Arc::new(ToolRegistry::new());
            register_tool(
                &tools,
                "test_tool",
                ApprovalRequirement::Never,
                "tool output",
            )
            .await;
            let (agent, _) = make_test_agent(
                primary.clone(),
                Some(cheap.clone()),
                tools,
                Some(runtime),
                StreamMode::None,
                true,
                10,
            )
            .await;
            let (session, thread_id) = make_session_and_thread().await;
            let message = IncomingMessage::new("test", "user-1", "help");

            let _ = agent
                .run_agentic_loop(
                    &message,
                    session,
                    thread_id,
                    vec![ChatMessage::user("help")],
                )
                .await
                .expect("agentic loop should succeed");

            (primary.requests().await, cheap.requests().await)
        }

        let (primary_enabled, cheap_enabled) = run_case(true).await;
        let (primary_disabled, cheap_disabled) = run_case(false).await;

        assert!(matches!(
            primary_enabled[0].thinking,
            ThinkingConfig::Enabled { .. }
        ));
        assert!(matches!(
            primary_disabled[0].thinking,
            ThinkingConfig::Disabled
        ));
        assert!(matches!(
            cheap_enabled[0].thinking,
            ThinkingConfig::Enabled { .. }
        ));
        assert!(matches!(
            cheap_disabled[0].thinking,
            ThinkingConfig::Enabled { .. }
        ));
    }

    #[tokio::test]
    async fn advisor_interception_runs_in_parallel_path_and_enforces_budget() {
        let primary = Arc::new(ScriptedLlm::new(
            "primary-model",
            vec![
                ScriptedResponse::tool_calls(
                    vec![tool_call("consult_advisor"), tool_call("test_tool")],
                    FinishReason::ToolUse,
                ),
                ScriptedResponse::text("Final answer", FinishReason::Stop),
                ScriptedResponse::text("Final answer", FinishReason::Stop),
                ScriptedResponse::text("Final answer", FinishReason::Stop),
            ],
        ));
        let runtime =
            make_runtime_manager_for_mode(false, true, RoutingMode::AdvisorExecutor, 0).await;
        let tools = Arc::new(ToolRegistry::new());
        register_tool(
            &tools,
            "test_tool",
            ApprovalRequirement::Never,
            "tool output",
        )
        .await;
        let (agent, channel) = make_test_agent(
            primary.clone(),
            Some(primary.clone()),
            tools,
            Some(runtime),
            StreamMode::None,
            true,
            10,
        )
        .await;
        let (session, thread_id) = make_session_and_thread().await;
        let message = IncomingMessage::new("test", "user-1", "help");

        let result = agent
            .run_agentic_loop(
                &message,
                session,
                thread_id,
                vec![ChatMessage::user("help")],
            )
            .await
            .expect("agentic loop should succeed");

        match result {
            super::AgenticLoopResult::Response(text) => assert_eq!(text, "Final answer"),
            other => panic!(
                "expected response result, got {:?}",
                std::mem::discriminant(&other)
            ),
        }

        let events = channel.events().await;
        assert!(events.iter().any(|event| matches!(
            event,
            RecordedChannelEvent::Status(StatusUpdate::ToolCompleted { name, success, .. })
                if name == "consult_advisor" && *success
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            RecordedChannelEvent::Status(StatusUpdate::ToolResult { name, preview })
                if name == "consult_advisor" && preview.contains("advisor_call_limit_reached")
        )));
    }

    #[tokio::test]
    async fn pending_approval_context_does_not_persist_planning_prompt() {
        let primary = Arc::new(ScriptedLlm::new(
            "primary-model",
            vec![ScriptedResponse::tool_calls(
                vec![tool_call("approval_tool")],
                FinishReason::ToolUse,
            )],
        ));
        let cheap = Arc::new(ScriptedLlm::new("cheap-model", vec![]));
        let runtime = make_runtime_manager(true, true).await;
        let tools = Arc::new(ToolRegistry::new());
        register_tool(
            &tools,
            "approval_tool",
            ApprovalRequirement::Always,
            "approval tool output",
        )
        .await;
        let (agent, _) = make_test_agent(
            primary,
            Some(cheap),
            tools,
            Some(runtime),
            StreamMode::None,
            true,
            10,
        )
        .await;
        let (session, thread_id) = make_session_and_thread().await;
        let message = IncomingMessage::new("test", "user-1", "help");

        let result = agent
            .run_agentic_loop(
                &message,
                session,
                thread_id,
                vec![ChatMessage::user("help")],
            )
            .await
            .expect("agentic loop should succeed");

        match result {
            super::AgenticLoopResult::NeedApproval { pending } => {
                assert!(!contains_prompt(
                    &pending.context_messages,
                    TOOL_PHASE_PLANNING_PROMPT
                ));
                assert!(!contains_prompt(
                    &pending.context_messages,
                    TOOL_PHASE_SYNTHESIS_PROMPT
                ));
            }
            other => panic!(
                "expected approval result, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[tokio::test]
    async fn run_agentic_loop_uses_channel_formatting_hints_from_channel_manager() {
        let primary = Arc::new(ScriptedLlm::new(
            "primary-model",
            vec![ScriptedResponse::text(
                "Plain text response",
                FinishReason::Stop,
            )],
        ));
        let tools = Arc::new(ToolRegistry::new());
        let recording_channel = RecordingChannel::new("test", StreamMode::None)
            .with_formatting_hints("- Test channel prefers plain text only.");
        let (agent, _) = make_test_agent_with_channel(
            primary.clone(),
            None,
            tools,
            None,
            recording_channel,
            false,
            10,
        )
        .await;
        let (session, thread_id) = make_session_and_thread().await;
        let message = IncomingMessage::new("test", "user-1", "hello");

        let result = agent
            .run_agentic_loop(
                &message,
                session,
                thread_id,
                vec![ChatMessage::user("hello")],
            )
            .await
            .expect("agentic loop should succeed");

        match result {
            super::AgenticLoopResult::Response(text) => assert_eq!(text, "Plain text response"),
            other => panic!(
                "expected text response, got {:?}",
                std::mem::discriminant(&other)
            ),
        }

        let requests = primary.requests().await;
        assert!(requests.iter().any(|req| {
            req.context_documents
                .iter()
                .any(|doc| doc.contains("Test channel prefers plain text only."))
        }));
    }
}
