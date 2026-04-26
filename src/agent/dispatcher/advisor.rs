use super::*;
impl Agent {
    pub(super) async fn execute_consult_advisor_call(
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

    pub(super) fn serialize_advisor_envelope(
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

    pub(super) async fn run_advisor_consultation(
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

    pub(super) fn parse_advisor_envelope(
        &self,
        content: &str,
    ) -> Option<crate::tools::builtin::advisor::AdvisorConsultationEnvelope> {
        serde_json::from_str(content).ok()
    }

    pub(super) fn apply_advisor_stop_directive(
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

    pub(super) fn build_auto_advisor_arguments(
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

    pub(super) fn next_auto_advisor_trigger(
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
    pub(super) async fn inject_auto_advisor_consultation(
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
}
