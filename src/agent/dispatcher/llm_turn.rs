use super::*;
impl Agent {
    pub(super) fn thinking_config_for_model(&self, model_name: &str) -> crate::llm::ThinkingConfig {
        let (enabled, budget) = self.config.resolve_thinking_for_model(model_name);
        if enabled {
            crate::llm::ThinkingConfig::Enabled {
                budget_tokens: budget,
            }
        } else {
            crate::llm::ThinkingConfig::Disabled
        }
    }

    pub(super) fn build_turn_context(
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

    pub(super) fn agentic_result_from_text(
        &self,
        streamed_text: bool,
        text: String,
    ) -> AgenticLoopResult {
        if streamed_text {
            AgenticLoopResult::Streamed(text)
        } else {
            AgenticLoopResult::Response(text)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn finalize_primary_text_only(
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
    pub(super) async fn execute_llm_turn(
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

        self.record_thread_token_capture(thread_id, output.token_capture.clone())
            .await;

        Ok(LlmTurnResult {
            output,
            streamed_text,
        })
    }
}
