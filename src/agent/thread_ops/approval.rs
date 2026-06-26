//! Tool-approval flow, deferred-tool replay, and auth interception/token
//! handling.

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::dispatcher::AgenticLoopResult;
use crate::agent::session::{PendingApproval, PendingAuthMode, Session};
use crate::agent::submission::SubmissionResult;
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::context::JobContext;
use crate::error::Error;
use crate::llm::ChatMessage;
use crate::tools::{ToolExecutionLane, ToolProfile, execution};
use thinclaw_agent::thread_ops::PendingApprovalAdmission;

use thinclaw_agent::dispatcher_helpers::{
    check_auth_required_json as check_auth_required_content,
    parse_auth_result_json as parse_auth_result_content,
};

impl Agent {
    /// Process an approval or rejection of a pending tool execution.
    pub(in crate::agent) async fn process_approval(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        request_id: Option<Uuid>,
        approved: bool,
        always: bool,
    ) -> Result<SubmissionResult, Error> {
        let pending = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            match thinclaw_agent::thread_ops::take_pending_approval_matching(thread, request_id) {
                PendingApprovalAdmission::Ready(pending) => pending,
                PendingApprovalAdmission::Missing => {
                    return Ok(SubmissionResult::error(
                        thinclaw_agent::thread_ops::pending_approval_missing_message(),
                    ));
                }
                PendingApprovalAdmission::RequestIdMismatch => {
                    drop(sess);
                    self.persist_thread_runtime_snapshot(message, &session, thread_id)
                        .await;
                    return Ok(SubmissionResult::error(
                        thinclaw_agent::thread_ops::pending_approval_request_mismatch_message(),
                    ));
                }
            }
        };

        if approved {
            // If always, add to auto-approved set
            if always {
                let mut sess = session.lock().await;
                sess.auto_approve_tool_for_channel(&message.channel, &pending.tool_name);
                tracing::info!(
                    "Auto-approved tool '{}' for session {}",
                    pending.tool_name,
                    sess.id
                );
            }

            // Reset thread state to processing
            let processing_snapshot = {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    thinclaw_agent::thread_ops::mark_pending_approval_approved(thread);
                    Some(thread.clone())
                } else {
                    None
                }
            };
            if let Some(thread_snapshot) = processing_snapshot {
                let _ = thread_snapshot;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
            }

            // Execute the approved tool and continue the loop
            let identity = message.resolved_identity();
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
                if let Some(owner) = self.agent_router.get_thread_owner(thread_id).await {
                    metadata.insert("agent_id".to_string(), serde_json::json!(owner.clone()));
                    if let Some(agent) = self.agent_router.get_agent(&owner).await {
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
            }

            let profile_override = job_ctx
                .metadata
                .get("tool_profile")
                .and_then(|value| value.as_str())
                .and_then(|value| value.parse::<ToolProfile>().ok());

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::ToolStarted {
                        name: pending.tool_name.clone(),
                        parameters: Some(pending.parameters.clone()),
                    },
                    &message.metadata,
                )
                .await;

            let tool_result = match execution::prepare_tool_call(execution::ToolPrepareRequest {
                tools: self.tools(),
                safety: self.safety(),
                job_ctx: &job_ctx,
                tool_name: &pending.tool_name,
                params: &pending.parameters,
                lane: ToolExecutionLane::DeferredChat,
                default_profile: self.config.main_tool_profile,
                profile_override,
                approval_mode: execution::ToolApprovalMode::Bypass,
                hooks: None,
            })
            .await
            {
                Ok(execution::ToolPrepareOutcome::Ready(prepared)) => {
                    execution::execute_tool_call(&prepared, self.safety(), &job_ctx).await
                }
                Ok(execution::ToolPrepareOutcome::NeedsApproval(_)) => {
                    Err(crate::error::ToolError::AuthRequired {
                        name: pending.tool_name.clone(),
                    }
                    .into())
                }
                Err(err) => Err(err),
            };

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::ToolCompleted {
                        name: pending.tool_name.clone(),
                        success: tool_result.is_ok(),
                        result_preview: tool_result.as_ref().ok().map(|output| {
                            crate::agent::dispatcher::truncate_preview(
                                &output.sanitized_content,
                                500,
                            )
                        }),
                    },
                    &message.metadata,
                )
                .await;

            if let Ok(ref output) = tool_result
                && !output.sanitized_content.is_empty()
            {
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::ToolResult {
                            name: pending.tool_name.clone(),
                            preview: output.sanitized_content.clone(),
                            artifacts: output.artifacts.clone(),
                        },
                        &message.metadata,
                    )
                    .await;
            }

            // Build context including the tool result
            let mut context_messages = pending.context_messages;
            let deferred_tool_calls = pending.deferred_tool_calls;

            // Sanitize the restored snapshot before appending new results.
            // The snapshot was captured at approval time; if the hard history
            // cap had fired in that same iteration and orphaned any Tool
            // messages, those orphans would be frozen into the snapshot.
            // Sanitizing here ensures the context is clean before we append
            // the approved tool result and resume the agentic loop.
            crate::llm::sanitize_tool_messages(&mut context_messages);

            // Record result in thread
            {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id)
                    && let Some(turn) = thread.last_turn_mut()
                {
                    match &tool_result {
                        Ok(output) => {
                            turn.record_tool_result(serde_json::json!(output.sanitized_content));
                        }
                        Err(e) => {
                            turn.record_tool_error(e.to_string());
                        }
                    }
                }
            }

            // If tool auth returned an auth-required state, enter auth mode when needed and
            // return instructions directly (skip agentic loop continuation).
            if let Some(auth_request) = check_auth_required_content(
                &pending.tool_name,
                tool_result
                    .as_ref()
                    .ok()
                    .map(|output| output.sanitized_content.as_str()),
            ) {
                self.handle_auth_intercept(
                    &session,
                    thread_id,
                    message,
                    tool_result
                        .as_ref()
                        .ok()
                        .map(|output| output.sanitized_content.as_str()),
                    auth_request.extension_name,
                    auth_request.instructions.clone(),
                    auth_request.auth_mode,
                )
                .await;
                return Ok(SubmissionResult::response(auth_request.instructions));
            }

            // Add tool result to context
            let result_content = match tool_result {
                Ok(output) => {
                    let sanitized = self
                        .safety()
                        .sanitize_tool_output(&pending.tool_name, &output.sanitized_content);
                    self.safety().wrap_for_llm(
                        &pending.tool_name,
                        &sanitized.content,
                        sanitized.was_modified,
                    )
                }
                Err(e) => format!("Error: {}", e),
            };

            context_messages.push(ChatMessage::tool_result(
                &pending.tool_call_id,
                &pending.tool_name,
                result_content,
            ));

            // Replay deferred tool calls from the same assistant message so
            // every tool_use ID gets a matching tool_result before the next
            // LLM call.
            if !deferred_tool_calls.is_empty() {
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Thinking(format!(
                            "Executing {} deferred tool(s)...",
                            deferred_tool_calls.len()
                        )),
                        &message.metadata,
                    )
                    .await;
            }

            // === Phase 1: Preflight (sequential) ===
            // Walk deferred tools through the shared preparation pipeline so
            // hooks, approval checks, validation, and rate limits stay
            // aligned with the live dispatcher path.
            let mut preflight_tool_calls: Vec<crate::llm::ToolCall> = Vec::new();
            let mut immediate_results: Vec<(usize, Result<execution::ToolExecutionOutput, Error>)> =
                Vec::new();
            let mut runnable: Vec<(usize, crate::llm::ToolCall, execution::PreparedToolCall)> =
                Vec::new();
            let mut approval_needed: Option<(
                usize,
                crate::llm::ToolCall,
                execution::PendingToolApproval,
            )> = None;

            for (idx, original_tc) in deferred_tool_calls.iter().enumerate() {
                let session_auto_approved = {
                    let sess = session.lock().await;
                    sess.is_tool_auto_approved_for_channel(&message.channel, &original_tc.name)
                };

                match execution::prepare_tool_call(execution::ToolPrepareRequest {
                    tools: self.tools(),
                    safety: self.safety(),
                    job_ctx: &job_ctx,
                    tool_name: &original_tc.name,
                    params: &original_tc.arguments,
                    lane: ToolExecutionLane::DeferredChat,
                    default_profile: self.config.main_tool_profile,
                    profile_override,
                    approval_mode: execution::ToolApprovalMode::Interactive {
                        auto_approve_tools: self.config.auto_approve_tools,
                        session_auto_approved,
                    },
                    hooks: Some(execution::ToolHookConfig {
                        registry: self.hooks().as_ref(),
                        user_id: &message.user_id,
                        context: "chat",
                    }),
                })
                .await
                {
                    Ok(execution::ToolPrepareOutcome::Ready(prepared)) => {
                        let mut tc = original_tc.clone();
                        tc.arguments = prepared.params.clone();
                        let preflight_idx = preflight_tool_calls.len();
                        preflight_tool_calls.push(tc.clone());
                        runnable.push((preflight_idx, tc, prepared));
                    }
                    Ok(execution::ToolPrepareOutcome::NeedsApproval(pending_approval)) => {
                        let mut tc = original_tc.clone();
                        tc.arguments = pending_approval.params.clone();
                        approval_needed = Some((idx, tc, pending_approval));
                        break;
                    }
                    Err(err) => {
                        let preflight_idx = preflight_tool_calls.len();
                        preflight_tool_calls.push(original_tc.clone());
                        immediate_results.push((preflight_idx, Err(err)));
                    }
                }
            }

            // === Phase 2: Parallel execution ===
            let mut exec_results: Vec<Option<Result<execution::ToolExecutionOutput, Error>>> =
                (0..preflight_tool_calls.len()).map(|_| None).collect();
            for (idx, result) in immediate_results {
                exec_results[idx] = Some(result);
            }

            let parallel_safe = runnable.len() > 1
                && runnable
                    .iter()
                    .all(|(_, _, prepared)| prepared.descriptor.metadata.parallel_safe);

            if !parallel_safe {
                for (pf_idx, tc, prepared) in runnable {
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
                        execution::execute_tool_call(&prepared, self.safety(), &job_ctx).await;

                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::ToolCompleted {
                                name: tc.name.clone(),
                                success: result.is_ok(),
                                result_preview: result.as_ref().ok().map(|output| {
                                    crate::agent::dispatcher::truncate_preview(
                                        &output.sanitized_content,
                                        500,
                                    )
                                }),
                            },
                            &message.metadata,
                        )
                        .await;

                    exec_results[pf_idx] = Some(result);
                }
            } else {
                let mut join_set = JoinSet::new();
                let runnable_slots = runnable
                    .iter()
                    .map(|(pf_idx, tc, _)| (*pf_idx, tc.clone()))
                    .collect::<Vec<_>>();
                let runnable_count = runnable.len();

                for (spawn_idx, (pf_idx, tc, prepared)) in runnable.into_iter().enumerate() {
                    let safety = self.safety().clone();
                    let channels = self.channels.clone();
                    let job_ctx = job_ctx.clone();
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

                        let result =
                            execution::execute_tool_call(&prepared, &safety, &job_ctx).await;

                        let _ = channels
                            .send_status(
                                &channel,
                                StatusUpdate::ToolCompleted {
                                    name: tc.name.clone(),
                                    success: result.is_ok(),
                                    result_preview: result.as_ref().ok().map(|output| {
                                        crate::agent::dispatcher::truncate_preview(
                                            &output.sanitized_content,
                                            500,
                                        )
                                    }),
                                },
                                &metadata,
                            )
                            .await;

                        (spawn_idx, pf_idx, result)
                    });
                }

                let mut ordered: Vec<
                    Option<(usize, Result<execution::ToolExecutionOutput, Error>)>,
                > = (0..runnable_count).map(|_| None).collect();
                while let Some(join_result) = join_set.join_next().await {
                    match join_result {
                        Ok((spawn_idx, pf_idx, result)) => {
                            ordered[spawn_idx] = Some((pf_idx, result));
                        }
                        Err(e) => {
                            if e.is_panic() {
                                tracing::error!("Deferred tool execution task panicked: {}", e);
                            } else {
                                tracing::error!("Deferred tool execution task cancelled: {}", e);
                            }
                        }
                    }
                }

                for (idx, opt) in ordered.into_iter().enumerate() {
                    let (pf_idx, result) = opt.unwrap_or_else(|| {
                        let (pf_idx, tc) = &runnable_slots[idx];
                        let err: Error = crate::error::ToolError::ExecutionFailed {
                            name: tc.name.clone(),
                            reason: "Task failed during execution".to_string(),
                        }
                        .into();
                        (*pf_idx, Err(err))
                    });
                    exec_results[pf_idx] = Some(result);
                }
            }

            // === Phase 3: Post-flight (sequential, in original order) ===
            // Process all results before any conditional return so every
            // tool result is recorded in the session audit trail.
            let mut deferred_auth: Option<String> = None;

            for (tc, deferred_result) in preflight_tool_calls
                .into_iter()
                .zip(exec_results.into_iter())
                .map(|(tc, result)| {
                    let result = result.unwrap_or_else(|| {
                        Err(crate::error::ToolError::ExecutionFailed {
                            name: tc.name.clone(),
                            reason: "Deferred tool result missing after execution".to_string(),
                        }
                        .into())
                    });
                    (tc, result)
                })
            {
                if let Ok(ref output) = deferred_result
                    && !output.sanitized_content.is_empty()
                {
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::ToolResult {
                                name: tc.name.clone(),
                                preview: output.sanitized_content.clone(),
                                artifacts: output.artifacts.clone(),
                            },
                            &message.metadata,
                        )
                        .await;
                }

                // Record in thread
                {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id)
                        && let Some(turn) = thread.last_turn_mut()
                    {
                        match &deferred_result {
                            Ok(output) => {
                                turn.record_tool_result(serde_json::json!(output.sanitized_content))
                            }
                            Err(e) => turn.record_tool_error(e.to_string()),
                        }
                    }
                }

                // Auth detection — defer return until all results are recorded
                if deferred_auth.is_none()
                    && let Some(auth_request) = check_auth_required_content(
                        &tc.name,
                        deferred_result
                            .as_ref()
                            .ok()
                            .map(|output| output.sanitized_content.as_str()),
                    )
                {
                    self.handle_auth_intercept(
                        &session,
                        thread_id,
                        message,
                        deferred_result
                            .as_ref()
                            .ok()
                            .map(|output| output.sanitized_content.as_str()),
                        auth_request.extension_name,
                        auth_request.instructions.clone(),
                        auth_request.auth_mode,
                    )
                    .await;
                    deferred_auth = Some(auth_request.instructions);
                }

                let deferred_content = match deferred_result {
                    Ok(output) => {
                        let sanitized = self
                            .safety()
                            .sanitize_tool_output(&tc.name, &output.sanitized_content);
                        self.safety().wrap_for_llm(
                            &tc.name,
                            &sanitized.content,
                            sanitized.was_modified,
                        )
                    }
                    Err(e) => format!("Error: {}", e),
                };

                context_messages.push(ChatMessage::tool_result(&tc.id, &tc.name, deferred_content));
            }

            // Return auth response after all results are recorded
            if let Some(instructions) = deferred_auth {
                return Ok(SubmissionResult::response(instructions));
            }

            // Handle approval if a tool needed it
            if let Some((approval_idx, tc, pending_approval)) = approval_needed {
                let new_pending = PendingApproval {
                    request_id: Uuid::new_v4(),
                    tool_name: tc.name.clone(),
                    parameters: pending_approval.params.clone(),
                    description: pending_approval.descriptor.description.clone(),
                    tool_call_id: tc.id.clone(),
                    context_messages: context_messages.clone(),
                    deferred_tool_calls: deferred_tool_calls[approval_idx + 1..].to_vec(),
                };

                let request_id = new_pending.request_id;
                let tool_name = new_pending.tool_name.clone();
                let description = new_pending.description.clone();
                let parameters = new_pending.parameters.clone();

                {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thinclaw_agent::thread_ops::await_thread_approval(thread, new_pending);
                    }
                }
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;

                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Awaiting approval".into()),
                        &message.metadata,
                    )
                    .await;

                return Ok(SubmissionResult::NeedApproval {
                    request_id,
                    tool_name,
                    description,
                    parameters,
                });
            }

            // Continue the agentic loop (a tool was already executed this turn)
            let result = self
                .run_agentic_loop(message, session.clone(), thread_id, context_messages)
                .await;

            // Handle the result
            let mut sess = session.lock().await;
            let session_id = sess.id;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            let was_streamed = matches!(&result, Ok(AgenticLoopResult::Streamed(_)));
            match result {
                Ok(AgenticLoopResult::Response(payload))
                | Ok(AgenticLoopResult::Streamed(payload)) => {
                    let (turn_number, messages) =
                        thinclaw_agent::thread_ops::complete_thread_response(
                            thread,
                            &payload.content,
                        );
                    let usage_percent = self.context_monitor.usage_percent(&messages);
                    // User message already persisted at turn start; save assistant response
                    self.persist_assistant_response(
                        thread_id,
                        message,
                        &payload.content,
                        session_id,
                        turn_number,
                    )
                    .await;
                    drop(sess);
                    self.sync_context_pressure_warning(message, thread_id, usage_percent)
                        .await;
                    self.persist_thread_runtime_snapshot(message, &session, thread_id)
                        .await;
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::Status("Done".into()),
                            &message.metadata,
                        )
                        .await;
                    if was_streamed {
                        Ok(SubmissionResult::Streamed(payload))
                    } else {
                        Ok(SubmissionResult::Response { payload })
                    }
                }
                Ok(AgenticLoopResult::NeedApproval {
                    pending: new_pending,
                }) => {
                    let request_id = new_pending.request_id;
                    let tool_name = new_pending.tool_name.clone();
                    let description = new_pending.description.clone();
                    let parameters = new_pending.parameters.clone();
                    let messages =
                        thinclaw_agent::thread_ops::await_thread_approval(thread, new_pending);
                    let usage_percent = self.context_monitor.usage_percent(&messages);
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::Status("Awaiting approval".into()),
                            &message.metadata,
                        )
                        .await;
                    drop(sess);
                    self.sync_context_pressure_warning(message, thread_id, usage_percent)
                        .await;
                    self.persist_thread_runtime_snapshot(message, &session, thread_id)
                        .await;
                    Ok(SubmissionResult::NeedApproval {
                        request_id,
                        tool_name,
                        description,
                        parameters,
                    })
                }
                Err(e) => {
                    let messages =
                        thinclaw_agent::thread_ops::fail_thread_turn(thread, &e.to_string());
                    let usage_percent = self.context_monitor.usage_percent(&messages);
                    // User message already persisted at turn start
                    drop(sess);
                    self.sync_context_pressure_warning(message, thread_id, usage_percent)
                        .await;
                    self.persist_thread_runtime_snapshot(message, &session, thread_id)
                        .await;
                    Ok(SubmissionResult::error(e.to_string()))
                }
            }
        } else {
            // Rejected - complete the turn with a rejection message and persist
            let rejection = format!(
                "Tool '{}' was rejected. The agent will not execute this tool.\n\n\
                 You can continue the conversation or try a different approach.",
                pending.tool_name
            );
            {
                let mut sess = session.lock().await;
                let session_id = sess.id;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    let (turn_number, messages) =
                        thinclaw_agent::thread_ops::reject_pending_approval(thread, &rejection);
                    let usage_percent = self.context_monitor.usage_percent(&messages);
                    // User message already persisted at turn start; save rejection response
                    self.persist_assistant_response(
                        thread_id,
                        message,
                        &rejection,
                        session_id,
                        turn_number,
                    )
                    .await;
                    drop(sess);
                    self.sync_context_pressure_warning(message, thread_id, usage_percent)
                        .await;
                    self.persist_thread_runtime_snapshot(message, &session, thread_id)
                        .await;
                }
            }

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status("Rejected".into()),
                    &message.metadata,
                )
                .await;

            Ok(SubmissionResult::response(rejection))
        }
    }

    /// Handle an auth-required result from a tool execution.
    ///
    /// Enters auth mode on the thread, completes + persists the turn,
    /// and sends the AuthRequired status to the channel.
    /// Returns the instructions string for the caller to wrap in a response.
    async fn handle_auth_intercept(
        &self,
        session: &Arc<Mutex<Session>>,
        thread_id: Uuid,
        message: &IncomingMessage,
        tool_result_content: Option<&str>,
        ext_name: String,
        instructions: String,
        auth_mode: PendingAuthMode,
    ) {
        let auth_data = parse_auth_result_content(tool_result_content);
        let thread_snapshot = {
            let mut sess = session.lock().await;
            let session_id = sess.id;
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                let (turn_number, _) =
                    thinclaw_agent::thread_ops::enter_auth_mode_and_complete_turn(
                        thread,
                        ext_name.clone(),
                        auth_mode,
                        &instructions,
                    );
                // User message already persisted at turn start; save auth instructions
                self.persist_assistant_response(
                    thread_id,
                    message,
                    &instructions,
                    session_id,
                    turn_number,
                )
                .await;
                Some(thread.clone())
            } else {
                None
            }
        };
        if let Some(thread_snapshot) = thread_snapshot {
            let _ = thread_snapshot;
            self.persist_thread_runtime_snapshot(message, session, thread_id)
                .await;
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
                    auth_mode: thinclaw_agent::thread_ops::auth_required_status_mode(
                        auth_data.auth_mode,
                        auth_mode,
                    ),
                    auth_status: thinclaw_agent::thread_ops::auth_required_status(
                        auth_data.auth_status,
                    ),
                    shared_auth_provider: auth_data.shared_auth_provider,
                    missing_scopes: auth_data.missing_scopes,
                    thread_id: Some(thread_id.to_string()),
                },
                &message.metadata,
            )
            .await;
    }

    /// Handle an auth token submitted while the thread is in auth mode.
    ///
    /// The token goes directly to the extension manager's credential store,
    /// completely bypassing logging, turn creation, history, and compaction.
    pub(in crate::agent) async fn process_auth_token(
        &self,
        message: &IncomingMessage,
        pending: &crate::agent::session::PendingAuth,
        token: &str,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<Option<String>, Error> {
        let token = token.trim();

        // Clear auth mode regardless of outcome
        let cleared_snapshot = {
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                thinclaw_agent::thread_ops::clear_pending_auth(thread);
                Some(thread.clone())
            } else {
                None
            }
        };
        if let Some(thread_snapshot) = cleared_snapshot {
            let _ = thread_snapshot;
            self.persist_thread_runtime_snapshot(message, &session, thread_id)
                .await;
        }

        let ext_mgr = match self.deps.extension_manager.as_ref() {
            Some(mgr) => mgr,
            None => return Ok(Some("Extension manager not available.".to_string())),
        };

        match ext_mgr.auth(&pending.extension_name, Some(token)).await {
            Ok(result) if result.status == "authenticated" => {
                tracing::info!(
                    "Extension '{}' authenticated via auth mode",
                    pending.extension_name
                );

                // Auto-activate so tools are available immediately after auth
                match ext_mgr.activate(&pending.extension_name).await {
                    Ok(activate_result) => {
                        let msg = thinclaw_agent::thread_ops::auth_activation_success_message(
                            &pending.extension_name,
                            &activate_result.tools_loaded,
                        );
                        let _ = self
                            .channels
                            .send_status(
                                &message.channel,
                                StatusUpdate::AuthCompleted {
                                    extension_name: pending.extension_name.clone(),
                                    success: true,
                                    message: msg.clone(),
                                    auth_mode: Some("manual_token".to_string()),
                                    auth_status: Some("authenticated".to_string()),
                                    shared_auth_provider: result.shared_auth_provider.clone(),
                                    missing_scopes: result.missing_scopes.clone(),
                                    thread_id: Some(thread_id.to_string()),
                                },
                                &message.metadata,
                            )
                            .await;
                        Ok(Some(msg))
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Extension '{}' authenticated but activation failed: {}",
                            pending.extension_name,
                            e
                        );
                        let msg = thinclaw_agent::thread_ops::auth_activation_failed_message(
                            &pending.extension_name,
                            &e.to_string(),
                        );
                        let _ = self
                            .channels
                            .send_status(
                                &message.channel,
                                StatusUpdate::AuthCompleted {
                                    extension_name: pending.extension_name.clone(),
                                    success: true,
                                    message: msg.clone(),
                                    auth_mode: Some("manual_token".to_string()),
                                    auth_status: Some("authenticated".to_string()),
                                    shared_auth_provider: result.shared_auth_provider.clone(),
                                    missing_scopes: result.missing_scopes.clone(),
                                    thread_id: Some(thread_id.to_string()),
                                },
                                &message.metadata,
                            )
                            .await;
                        Ok(Some(msg))
                    }
                }
            }
            Ok(result) => {
                // Invalid token, re-enter auth mode
                {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thinclaw_agent::thread_ops::reenter_pending_auth(thread, pending);
                    }
                }
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
                let msg =
                    thinclaw_agent::thread_ops::invalid_auth_token_message(result.instructions);
                // Re-emit AuthRequired so web UI re-shows the card
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::AuthRequired {
                            extension_name: pending.extension_name.clone(),
                            instructions: Some(msg.clone()),
                            auth_url: result.auth_url,
                            setup_url: result.setup_url,
                            auth_mode: result.auth_mode.clone(),
                            auth_status: result.auth_status.clone(),
                            shared_auth_provider: result.shared_auth_provider.clone(),
                            missing_scopes: result.missing_scopes.clone(),
                            thread_id: Some(thread_id.to_string()),
                        },
                        &message.metadata,
                    )
                    .await;
                Ok(Some(msg))
            }
            Err(e) => {
                let msg = thinclaw_agent::thread_ops::auth_failed_message(
                    &pending.extension_name,
                    &e.to_string(),
                );
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::AuthCompleted {
                            extension_name: pending.extension_name.clone(),
                            success: false,
                            message: msg.clone(),
                            auth_mode: None,
                            auth_status: None,
                            shared_auth_provider: None,
                            missing_scopes: Vec::new(),
                            thread_id: Some(thread_id.to_string()),
                        },
                        &message.metadata,
                    )
                    .await;
                Ok(Some(msg))
            }
        }
    }
}
