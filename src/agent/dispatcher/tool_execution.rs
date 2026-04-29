use super::*;
impl Agent {
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

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_tool_calls_phase(
        &self,
        content: Option<String>,
        tool_calls: Vec<crate::llm::ToolCall>,
        context_messages: &mut Vec<ChatMessage>,
        session: &Arc<Mutex<Session>>,
        thread_id: Uuid,
        message: &IncomingMessage,
        job_ctx: &JobContext,
        advisor_call_budget: &crate::tools::builtin::advisor::AdvisorCallBudget,
        advisor_state: &mut AdvisorTurnState,
        identity: &crate::identity::ResolvedIdentity,
        routed_agent_workspace_id: Option<Uuid>,
        routed_allowed_tools: Option<&[String]>,
        routed_allowed_skills: Option<&[String]>,
        blocked_signature: Option<u64>,
        last_call_signature: &mut Option<u64>,
        consecutive_same_calls: &mut u32,
    ) -> Result<Option<AgenticLoopResult>, Error> {
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
                StatusUpdate::Thinking(format!("Executing {} tool(s)...", tool_calls.len())),
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
                            !sess.is_tool_auto_approved_for_channel(&message.channel, &tc.name)
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

                let result = tokio::select! {
                    biased;
                    _ = self.wait_for_turn_cancellation(thread_id) => {
                        Err(Self::turn_interrupted_error(thread_id))
                    }
                    result = async {
                        if tc.name == crate::tools::builtin::advisor::ADVISOR_TOOL_NAME {
                            self.execute_consult_advisor_call(tc, &context_messages, advisor_call_budget)
                                .await
                        } else {
                            self.execute_chat_tool(&tc.name, &tc.arguments, &job_ctx)
                                .await
                        }
                    } => result
                };

                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::ToolCompleted {
                            name: tc.name.clone(),
                            success: result.is_ok(),
                            result_preview: result.as_ref().ok().map(|s| truncate_preview(s, 500)),
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
                        .execute_consult_advisor_call(tc, &context_messages, advisor_call_budget)
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

            loop {
                let join_result = tokio::select! {
                    biased;
                    _ = self.wait_for_turn_cancellation(thread_id) => {
                        join_set.abort_all();
                        while join_set.join_next().await.is_some() {}
                        return Err(Self::turn_interrupted_error(thread_id));
                    }
                    join_result = join_set.join_next() => join_result
                };

                let Some(join_result) = join_result else {
                    break;
                };
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
                                .or_else(|| panic_payload.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "<non-string panic payload>".to_string());
                            tracing::error!(
                                panic = %panic_msg,
                                "Chat tool execution task panicked"
                            );
                        } else {
                            tracing::error!("Chat tool execution task cancelled: {}", e);
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
                    exec_results[*pf_idx] = Some(Err(crate::error::ToolError::ExecutionFailed {
                        name: tc.name.clone(),
                        reason: "Task panicked or was cancelled during execution".to_string(),
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
                            signature: Some(tool_call_signature(std::slice::from_ref(&tc))),
                            checkpoint: advisor_state.real_tool_result_count,
                        });
                    }
                    context_messages.push(ChatMessage::tool_result(&tc.id, &tc.name, error_msg));
                }
                PreflightOutcome::Runnable => {
                    // Retrieve the execution result for this slot
                    let mut tool_result = exec_results[pf_idx].take().unwrap_or_else(|| {
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
                        && let Ok(action) =
                            serde_json::from_str::<crate::tools::builtin::CanvasAction>(output)
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
                                            serde_json::to_value(components).unwrap_or_default(),
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
                                            serde_json::to_value(components).unwrap_or_default(),
                                            None,
                                        )
                                        .await;
                                }
                                crate::tools::builtin::CanvasAction::Dismiss { panel_id } => {
                                    store.dismiss(panel_id).await;
                                }
                                crate::tools::builtin::CanvasAction::Notify { .. } => {
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
                        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(output)
                        && let Some(msg) = parsed.get("message").and_then(|v| v.as_str())
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
                        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(output)
                        && parsed.get("action").and_then(|v| v.as_str()) == Some("spawn_subagent")
                    {
                        if let Some(executor) = self.subagent_executor.as_ref() {
                            if let Some(req_val) = parsed.get("request")
                                && let Ok(mut request) =
                                    serde_json::from_value::<
                                        crate::agent::subagent_executor::SubagentSpawnRequest,
                                    >(req_val.clone())
                            {
                                request
                                    .principal_id
                                    .get_or_insert_with(|| identity.principal_id.clone());
                                request
                                    .actor_id
                                    .get_or_insert_with(|| identity.actor_id.clone());
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
                                    metadata.insert(
                                        "thread_id".to_string(),
                                        serde_json::json!(thread_id.to_string()),
                                    );
                                    metadata.insert(
                                        "principal_id".to_string(),
                                        serde_json::json!(identity.principal_id.clone()),
                                    );
                                    metadata.insert(
                                        "actor_id".to_string(),
                                        serde_json::json!(identity.actor_id.clone()),
                                    );
                                    metadata.insert(
                                        "conversation_kind".to_string(),
                                        serde_json::json!(identity.conversation_kind.as_str()),
                                    );
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
                                                            channel_metadata: spawn_metadata
                                                                .clone(),
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
                                        Ok(serde_json::to_string(&result).unwrap_or_default())
                                    }
                                    Err(e) => Ok(serde_json::json!({
                                        "error": e.to_string(),
                                        "success": false,
                                    })
                                    .to_string()),
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
                        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(output)
                        && parsed.get("a2a_request").and_then(|v| v.as_bool()) == Some(true)
                    {
                        let target_id = parsed
                            .get("target_agent_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let target_name = parsed
                            .get("target_display_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or(target_id);
                        let a2a_message =
                            parsed.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        let system_prompt = parsed
                            .get("target_system_prompt")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let target_model = parsed.get("target_model").and_then(|v| v.as_str());
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
                                    principal_id: Some(parent_identity.principal_id.clone()),
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
                                    serde_json::json!(parent_identity.principal_id.clone()),
                                );
                                metadata.insert(
                                    "actor_id".to_string(),
                                    serde_json::json!(parent_identity.actor_id.clone()),
                                );
                                metadata.insert(
                                    "conversation_kind".to_string(),
                                    serde_json::json!(parent_identity.conversation_kind.as_str()),
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
                        && let Some(auth_request) = check_auth_required(&tc.name, &tool_result)
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
                                    advisor_state.last_failure = Some(AdvisorFailureContext {
                                        tool_name: tc.name.clone(),
                                        message: truncate_preview(output, 240),
                                        signature: Some(tool_call_signature(std::slice::from_ref(
                                            &tc,
                                        ))),
                                        checkpoint: advisor_state.real_tool_result_count,
                                    });
                                } else {
                                    advisor_state.last_failure = None;
                                }
                            }
                            Err(error) => {
                                advisor_state.last_failure = Some(AdvisorFailureContext {
                                    tool_name: tc.name.clone(),
                                    message: error.to_string(),
                                    signature: Some(tool_call_signature(std::slice::from_ref(&tc))),
                                    checkpoint: advisor_state.real_tool_result_count,
                                });
                            }
                        }
                        None
                    };

                    // Sanitize and add tool result to context
                    let result_content = match tool_result {
                        Ok(output) => {
                            let sanitized = self.safety().sanitize_tool_output(&tc.name, &output);
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
                            blocked_signature,
                            advisor_state,
                            context_messages,
                            last_call_signature,
                            consecutive_same_calls,
                        );
                    }
                }
            }
        }

        // Return auth response after all results are recorded
        if let Some(instructions) = deferred_auth {
            return Ok(Some(AgenticLoopResult::Response(instructions)));
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

            return Ok(Some(AgenticLoopResult::NeedApproval { pending }));
        }

        Ok(None)
    }
}
