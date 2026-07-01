//! lib: sessions.

use super::*;

pub(crate) fn conversation_kind(is_private: bool) -> &'static str {
    if is_private {
        "direct"
    } else {
        "group"
    }
}

pub(crate) fn conversation_scope_id(
    chat_id: i64,
    message_thread_id: Option<i64>,
    is_private: bool,
) -> String {
    if is_private {
        format!("telegram:direct:{chat_id}")
    } else if let Some(thread_id) = message_thread_id {
        format!("telegram:group:{chat_id}:topic:{thread_id}")
    } else {
        format!("telegram:group:{chat_id}")
    }
}

pub(crate) fn normalized_message_thread_id(message_thread_id: Option<i64>) -> Option<i64> {
    message_thread_id.filter(|thread_id| *thread_id > 0)
}

pub(crate) fn managed_private_topic_kind_for_thread_id(
    state: &ManagedPrivateTopicState,
    thread_id: i64,
) -> Option<ManagedPrivateTopicKind> {
    if state.onboarding_thread_id == Some(thread_id) {
        Some(ManagedPrivateTopicKind::Onboarding)
    } else if state.general_thread_id == Some(thread_id) {
        Some(ManagedPrivateTopicKind::General)
    } else {
        None
    }
}

pub(crate) fn managed_private_topic_kind_for_incoming(
    chat_id: i64,
    message_thread_id: Option<i64>,
    is_private: bool,
) -> Option<ManagedPrivateTopicKind> {
    if !is_private {
        return None;
    }

    let thread_id = message_thread_id?;
    let registry = read_managed_private_topic_registry();
    let state = registry.chats.get(&chat_id.to_string())?;
    managed_private_topic_kind_for_thread_id(state, thread_id)
}

pub(crate) fn incoming_session_thread_id_for_kind(
    message_thread_id: Option<i64>,
    managed_private_kind: Option<ManagedPrivateTopicKind>,
) -> Option<String> {
    if managed_private_kind.is_some() {
        None
    } else {
        message_thread_id.map(|thread_id| thread_id.to_string())
    }
}

pub(crate) fn incoming_session_thread_id(
    chat_id: i64,
    message_thread_id: Option<i64>,
    is_private: bool,
) -> Option<String> {
    incoming_session_thread_id_for_kind(
        message_thread_id,
        managed_private_topic_kind_for_incoming(chat_id, message_thread_id, is_private),
    )
}

pub(crate) fn external_conversation_key(
    chat_id: i64,
    message_thread_id: Option<i64>,
    is_private: bool,
) -> String {
    if is_private {
        format!("telegram://direct/{chat_id}")
    } else if let Some(thread_id) = message_thread_id {
        format!("telegram://group/{chat_id}/topic/{thread_id}")
    } else {
        format!("telegram://group/{chat_id}")
    }
}

pub(crate) fn truncate_status_message(input: &str, max_chars: usize) -> String {
    let mut iter = input.chars();
    let truncated: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

pub(crate) fn status_message_for_user(update: &StatusUpdate) -> Option<String> {
    let message = update.message.trim();
    if message.is_empty() {
        None
    } else {
        Some(truncate_status_message(message, TELEGRAM_STATUS_MAX_CHARS))
    }
}

pub(crate) fn get_updates_url(offset: i64, timeout_secs: u32) -> String {
    format!(
        "https://api.telegram.org/bot{{TELEGRAM_BOT_TOKEN}}/getUpdates?offset={}&timeout={}&allowed_updates=[\"message\",\"edited_message\"]",
        offset, timeout_secs
    )
}

pub(crate) fn classify_status_update(update: &StatusUpdate) -> Option<TelegramStatusAction> {
    match update.status {
        // Show a typing indicator while the agent is producing text.
        StatusType::Thinking | StatusType::StreamChunk | StatusType::LifecycleStart => {
            Some(TelegramStatusAction::Typing)
        }
        // Turn-ending / ephemeral bookkeeping events carry no visible notice.
        StatusType::Done
        | StatusType::Interrupted
        | StatusType::LifecycleEnd
        | StatusType::Usage
        | StatusType::Plan => None,
        // Telegram doesn't have a rich activity rail like the WebUI, so
        // surface concise visible notices for tool lifecycle events.
        StatusType::ToolStarted | StatusType::ToolCompleted | StatusType::ToolResult => {
            status_message_for_user(update).map(TelegramStatusAction::Notify)
        }
        // Sub-agent lifecycle now arrives with dedicated status types; the
        // structured payload still travels in `message`, so keep parsing it.
        StatusType::SubagentSpawned
        | StatusType::SubagentProgress
        | StatusType::SubagentCompleted => {
            if let Some(event) = parse_subagent_event(&update.message) {
                Some(TelegramStatusAction::Subagent(event))
            } else {
                status_message_for_user(update).map(TelegramStatusAction::Notify)
            }
        }
        StatusType::Status => {
            if let Some(event) = parse_subagent_event(&update.message) {
                return Some(TelegramStatusAction::Subagent(event));
            }
            let msg = update.message.trim();
            if msg.eq_ignore_ascii_case("Done")
                || msg.eq_ignore_ascii_case("Interrupted")
                || msg.eq_ignore_ascii_case("Awaiting approval")
                || msg.eq_ignore_ascii_case("Rejected")
            {
                None
            } else {
                status_message_for_user(update).map(TelegramStatusAction::Notify)
            }
        }
        StatusType::ApprovalNeeded
        | StatusType::JobStarted
        | StatusType::AuthRequired
        | StatusType::AuthCompleted
        | StatusType::CredentialPrompt
        | StatusType::Error
        | StatusType::CanvasAction
        | StatusType::AgentMessage
        | StatusType::ContextCompactionStarted
        | StatusType::AdvisorConsultationStarted
        | StatusType::SelfRepairStarted
        | StatusType::SelfRepairCompleted => {
            status_message_for_user(update).map(TelegramStatusAction::Notify)
        }
    }
}

pub(crate) fn parse_subagent_event(message: &str) -> Option<SubagentEvent> {
    let trimmed = message.trim();
    let closing = trimmed.find(']')?;
    let prefix = trimmed.get(..=closing)?;
    if !prefix.starts_with("[subagent:") {
        return None;
    }

    let remainder = trimmed
        .get(closing + 1..)
        .unwrap_or_default()
        .trim_start()
        .to_string();
    let prefix_body = prefix.trim_start_matches('[').trim_end_matches(']');
    let parts: Vec<&str> = prefix_body.split(':').collect();
    if parts.len() < 3 {
        return None;
    }

    match parts[1] {
        "spawned" => {
            let agent_id = parts[2].to_string();
            if remainder.starts_with('{') {
                let payload: serde_json::Value = serde_json::from_str(&remainder).ok()?;
                let name = payload.get("name")?.as_str()?.to_string();
                let task = payload.get("task")?.as_str()?.to_string();
                Some(SubagentEvent::Spawned {
                    agent_id,
                    name,
                    task,
                })
            } else {
                let (name, task) = remainder
                    .split_once(" — ")
                    .or_else(|| remainder.split_once(" - "))
                    .map(|(name, task)| (name.trim().to_string(), task.trim().to_string()))?;
                Some(SubagentEvent::Spawned {
                    agent_id,
                    name,
                    task,
                })
            }
        }
        "progress" => {
            if parts.len() < 4 {
                return None;
            }
            let agent_id = parts[2].to_string();
            let category = parts[3].to_string();
            let message = if remainder.starts_with('{') {
                let payload: serde_json::Value = serde_json::from_str(&remainder).ok()?;
                payload
                    .get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string()
            } else {
                remainder
            };
            if message.trim().is_empty() {
                None
            } else {
                Some(SubagentEvent::Progress {
                    agent_id,
                    category,
                    message,
                })
            }
        }
        "completed" | "failed" => {
            let agent_id = parts[2].to_string();
            let success = parts[1] == "completed";
            if remainder.starts_with('{') {
                let payload: serde_json::Value = serde_json::from_str(&remainder).ok()?;
                Some(SubagentEvent::Completed {
                    agent_id,
                    name: payload
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("subagent")
                        .to_string(),
                    success: payload
                        .get("success")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(success),
                    response: payload
                        .get("response")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string),
                    duration_ms: payload.get("duration_ms").and_then(|value| value.as_u64()),
                    iterations: payload
                        .get("iterations")
                        .and_then(|value| value.as_u64())
                        .map(|value| value as usize),
                })
            } else {
                let name = remainder
                    .split_once(" (")
                    .map(|(value, _)| value.trim().to_string())
                    .unwrap_or_else(|| remainder.clone());
                Some(SubagentEvent::Completed {
                    agent_id,
                    name,
                    success,
                    response: None,
                    duration_ms: None,
                    iterations: None,
                })
            }
        }
        _ => None,
    }
}

pub(crate) fn extract_subagent_session_mode_from_value(
    value: &serde_json::Value,
) -> Option<String> {
    value
        .get("telegram_subagent_session_mode")
        .and_then(|mode| mode.as_str())
        .or_else(|| {
            value.get("channels").and_then(|channels| {
                channels
                    .get("telegram_subagent_session_mode")
                    .and_then(|mode| mode.as_str())
            })
        })
        .map(|mode| mode.trim().to_string())
        .filter(|mode| !mode.is_empty())
}

pub(crate) fn extract_subagent_session_mode_from_json(raw: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| extract_subagent_session_mode_from_value(&value))
}

pub(crate) fn parse_telegram_metadata(
    raw: &str,
) -> Result<TelegramMessageMetadata, serde_json::Error> {
    let mut metadata: TelegramMessageMetadata = serde_json::from_str(raw)?;
    if metadata.subagent_session_mode.is_none() {
        metadata.subagent_session_mode = extract_subagent_session_mode_from_json(raw);
    }
    Ok(metadata)
}

pub(crate) fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn maybe_close_orphaned_topic(session: &StoredSubagentSession) {
    if TelegramSubagentSessionMode::from_str(&session.mode)
        != Some(TelegramSubagentSessionMode::TempTopic)
    {
        return;
    }

    if let Some(topic_thread_id) = session.topic_thread_id {
        if let Err(error) = close_forum_topic(session.chat_id, topic_thread_id) {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!(
                    "Failed to close stale temp forum topic for orphaned subagent session: {}",
                    error
                ),
            );
        }
    }
}

pub(crate) fn prune_orphaned_subagent_sessions(
    sessions: &mut std::collections::HashMap<String, StoredSubagentSession>,
    now: u64,
    close_topics: bool,
) -> usize {
    let mut removed = 0usize;

    let stale_ids: Vec<String> = sessions
        .iter()
        .filter_map(|(agent_id, session)| {
            let age_secs = now.saturating_sub(session.last_touched_epoch_secs);
            if age_secs > SUBAGENT_SESSION_TTL_SECS {
                Some(agent_id.clone())
            } else {
                None
            }
        })
        .collect();

    for agent_id in stale_ids {
        if let Some(session) = sessions.remove(&agent_id) {
            removed += 1;
            if close_topics {
                maybe_close_orphaned_topic(&session);
            }
        }
    }

    if sessions.len() > SUBAGENT_SESSION_STORE_CAP {
        let overflow = sessions.len() - SUBAGENT_SESSION_STORE_CAP;
        let mut by_oldest_touch: Vec<(String, u64)> = sessions
            .iter()
            .map(|(agent_id, session)| (agent_id.clone(), session.last_touched_epoch_secs))
            .collect();
        by_oldest_touch.sort_by_key(|(_, touched)| *touched);

        for (agent_id, _) in by_oldest_touch.into_iter().take(overflow) {
            if let Some(session) = sessions.remove(&agent_id) {
                removed += 1;
                if close_topics {
                    maybe_close_orphaned_topic(&session);
                }
            }
        }
    }

    removed
}

pub(crate) fn read_subagent_sessions() -> std::collections::HashMap<String, StoredSubagentSession> {
    let mut sessions = channel_host::workspace_read(SUBAGENT_SESSIONS_PATH)
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let now = now_epoch_secs();
    let removed = prune_orphaned_subagent_sessions(&mut sessions, now, true);
    if removed > 0 {
        write_subagent_sessions(&sessions);
        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "Cleaned up {} stale subagent session entries from Telegram state",
                removed
            ),
        );
    }
    sessions
}

pub(crate) fn write_subagent_sessions(
    sessions: &std::collections::HashMap<String, StoredSubagentSession>,
) {
    if let Ok(serialized) = serde_json::to_string(sessions) {
        if let Err(error) = channel_host::workspace_write(SUBAGENT_SESSIONS_PATH, &serialized) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to persist subagent session state: {}", error),
            );
        }
    }
}

pub(crate) fn run_subagent_session_gc(force: bool) {
    let now = now_epoch_secs();

    if !force {
        let last_run = channel_host::workspace_read(SUBAGENT_GC_LAST_RUN_PATH)
            .and_then(|raw| raw.parse::<u64>().ok());
        if let Some(last_run) = last_run {
            if now.saturating_sub(last_run) < SUBAGENT_GC_INTERVAL_SECS {
                return;
            }
        }
    }

    let _ = read_subagent_sessions();
    let _ = channel_host::workspace_write(SUBAGENT_GC_LAST_RUN_PATH, &now.to_string());
}

pub(crate) fn resolve_subagent_session_mode(
    metadata: &TelegramMessageMetadata,
) -> TelegramSubagentSessionMode {
    metadata
        .subagent_session_mode
        .as_deref()
        .and_then(TelegramSubagentSessionMode::from_str)
        .or_else(|| {
            channel_host::workspace_read(SUBAGENT_SESSION_MODE_PATH)
                .as_deref()
                .and_then(TelegramSubagentSessionMode::from_str)
        })
        .unwrap_or_default()
}

pub(crate) fn truncate_topic_name(name: &str, task: &str) -> String {
    let base = if task.trim().is_empty() {
        name.trim().to_string()
    } else {
        format!("{}: {}", name.trim(), task.trim())
    };
    let limit = 64usize;
    if base.chars().count() <= limit {
        return base;
    }
    let mut out = String::new();
    for ch in base.chars().take(limit.saturating_sub(3)) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

pub(crate) fn create_forum_topic(chat_id: i64, name: &str) -> Result<i64, String> {
    let payload = serde_json::json!({
        "chat_id": chat_id,
        "name": name,
    });
    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|e| format!("Failed to serialize payload: {}", e))?;
    let headers = serde_json::json!({ "Content-Type": "application/json" });
    let response = channel_host::http_request(
        "POST",
        "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/createForumTopic",
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    )
    .map_err(|e| format!("HTTP request failed: {}", e))?;

    if response.status != 200 {
        let body_str = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Telegram API returned status {}: {}",
            response.status, body_str
        ));
    }

    let api_response: TelegramApiResponse<ForumTopic> = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse response: {}", e))?;
    if !api_response.ok {
        return Err(api_response
            .description
            .unwrap_or_else(|| "unknown topic creation error".to_string()));
    }

    api_response
        .result
        .map(|result| result.message_thread_id)
        .ok_or_else(|| "Telegram did not return a topic thread id".to_string())
}

pub(crate) fn close_forum_topic(chat_id: i64, message_thread_id: i64) -> Result<(), String> {
    let payload = serde_json::json!({
        "chat_id": chat_id,
        "message_thread_id": message_thread_id,
    });
    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|e| format!("Failed to serialize payload: {}", e))?;
    let headers = serde_json::json!({ "Content-Type": "application/json" });
    let response = channel_host::http_request(
        "POST",
        "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/closeForumTopic",
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    )
    .map_err(|e| format!("HTTP request failed: {}", e))?;

    if response.status != 200 {
        let body_str = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Telegram API returned status {}: {}",
            response.status, body_str
        ));
    }

    let api_response: TelegramApiResponse<bool> = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse response: {}", e))?;
    if api_response.ok {
        Ok(())
    } else {
        Err(api_response
            .description
            .unwrap_or_else(|| "unknown close topic error".to_string()))
    }
}

pub(crate) fn send_subagent_compact_notice(session: &StoredSubagentSession, text: &str) -> bool {
    if let Err(error) = send_message(session.chat_id, text, None, None, session.parent_thread_id) {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Failed to send compact subagent notice: {}", error),
        );
        false
    } else {
        true
    }
}

pub(crate) fn send_subagent_reply_or_compact(
    session: &StoredSubagentSession,
    text: &str,
) -> TelegramSubagentSessionMode {
    if send_message(
        session.chat_id,
        text,
        Some(session.parent_message_id),
        None,
        session.parent_thread_id,
    )
    .is_ok()
    {
        return TelegramSubagentSessionMode::ReplyChain;
    }

    let _ = send_subagent_compact_notice(session, text);
    TelegramSubagentSessionMode::CompactOff
}

pub(crate) fn render_subagent_spawn_notice(name: &str, task: &str) -> String {
    format!(
        "{} is working on: {}",
        name,
        truncate_status_message(task, 220)
    )
}

pub(crate) fn render_subagent_progress_notice(category: &str, message: &str) -> String {
    let label = match category {
        "tool" => "Tool",
        "question" => "Question",
        "warning" => "Warning",
        _ => "Progress",
    };
    format!("{label}: {}", truncate_status_message(message, 280))
}

pub(crate) fn render_subagent_completion_notice(
    name: &str,
    success: bool,
    response: Option<&str>,
    duration_ms: Option<u64>,
    iterations: Option<usize>,
) -> String {
    let mut lines = vec![format!(
        "{} {}",
        if success {
            "Completed"
        } else {
            "Finished with issues"
        },
        name
    )];
    let mut meta = Vec::new();
    if let Some(duration_ms) = duration_ms {
        meta.push(format!("{:.1}s", duration_ms as f64 / 1000.0));
    }
    if let Some(iterations) = iterations {
        meta.push(format!("{iterations} iterations"));
    }
    if !meta.is_empty() {
        lines.push(meta.join(" · "));
    }
    if let Some(response) = response.map(str::trim).filter(|value| !value.is_empty()) {
        lines.push(truncate_status_message(response, 500));
    }
    lines.join("\n")
}

pub(crate) fn handle_subagent_status(metadata: &TelegramMessageMetadata, event: SubagentEvent) {
    let mut sessions = read_subagent_sessions();

    match event {
        SubagentEvent::Spawned {
            agent_id,
            name,
            task,
        } => {
            let now = now_epoch_secs();
            let requested_mode = resolve_subagent_session_mode(metadata);
            let mut session = StoredSubagentSession {
                chat_id: metadata.chat_id,
                parent_message_id: metadata.message_id,
                parent_thread_id: metadata.message_thread_id,
                topic_thread_id: None,
                mode: requested_mode.as_str().to_string(),
                last_touched_epoch_secs: now,
            };

            let kickoff = render_subagent_spawn_notice(&name, &task);
            match requested_mode {
                TelegramSubagentSessionMode::TempTopic => {
                    let topic_name = truncate_topic_name(&name, &task);
                    match create_forum_topic(metadata.chat_id, &topic_name) {
                        Ok(topic_thread_id) => {
                            session.topic_thread_id = Some(topic_thread_id);
                            if let Err(error) = send_message(
                                metadata.chat_id,
                                &kickoff,
                                None,
                                None,
                                Some(topic_thread_id),
                            ) {
                                channel_host::log(
                                    channel_host::LogLevel::Warn,
                                    &format!(
                                        "Failed to send subagent kickoff to temp topic: {}",
                                        error
                                    ),
                                );
                            }
                        }
                        Err(error) => {
                            channel_host::log(
                                channel_host::LogLevel::Warn,
                                &format!(
                                    "Failed to create temp topic for subagent '{}': {}. Falling back to reply chain.",
                                    agent_id, error
                                ),
                            );
                            let fallback_mode = send_subagent_reply_or_compact(&session, &kickoff);
                            session.mode = fallback_mode.as_str().to_string();
                        }
                    }
                }
                TelegramSubagentSessionMode::ReplyChain => {
                    let fallback_mode = send_subagent_reply_or_compact(&session, &kickoff);
                    session.mode = fallback_mode.as_str().to_string();
                }
                TelegramSubagentSessionMode::CompactOff => {
                    let _ = send_subagent_compact_notice(&session, &kickoff);
                    session.mode = TelegramSubagentSessionMode::CompactOff.as_str().to_string();
                }
            }

            sessions.insert(agent_id, session);
            write_subagent_sessions(&sessions);
        }
        SubagentEvent::Progress {
            agent_id,
            category,
            message,
        } => {
            let Some(session) = sessions.get_mut(&agent_id) else {
                return;
            };
            session.last_touched_epoch_secs = now_epoch_secs();
            let notice = render_subagent_progress_notice(&category, &message);
            let mode = TelegramSubagentSessionMode::from_str(&session.mode).unwrap_or_default();
            match mode {
                TelegramSubagentSessionMode::TempTopic => {
                    if let Some(topic_thread_id) = session.topic_thread_id {
                        if let Err(error) = send_message(
                            session.chat_id,
                            &notice,
                            None,
                            None,
                            Some(topic_thread_id),
                        ) {
                            channel_host::log(
                                channel_host::LogLevel::Warn,
                                &format!(
                                    "Failed to send subagent progress to topic, falling back: {}",
                                    error
                                ),
                            );
                            let fallback_mode = send_subagent_reply_or_compact(session, &notice);
                            session.mode = fallback_mode.as_str().to_string();
                            session.topic_thread_id = None;
                        }
                    } else {
                        let fallback_mode = send_subagent_reply_or_compact(session, &notice);
                        session.mode = fallback_mode.as_str().to_string();
                    }
                }
                TelegramSubagentSessionMode::ReplyChain => {
                    let fallback_mode = send_subagent_reply_or_compact(session, &notice);
                    session.mode = fallback_mode.as_str().to_string();
                }
                TelegramSubagentSessionMode::CompactOff => {
                    let _ = send_subagent_compact_notice(session, &notice);
                }
            }
            write_subagent_sessions(&sessions);
        }
        SubagentEvent::Completed {
            agent_id,
            name,
            success,
            response,
            duration_ms,
            iterations,
        } => {
            let Some(session) = sessions.remove(&agent_id) else {
                return;
            };
            let notice = render_subagent_completion_notice(
                &name,
                success,
                response.as_deref(),
                duration_ms,
                iterations,
            );
            let mode = TelegramSubagentSessionMode::from_str(&session.mode).unwrap_or_default();
            match mode {
                TelegramSubagentSessionMode::TempTopic => {
                    if let Some(topic_thread_id) = session.topic_thread_id {
                        if let Err(error) = send_message(
                            session.chat_id,
                            &notice,
                            None,
                            None,
                            Some(topic_thread_id),
                        ) {
                            channel_host::log(
                                channel_host::LogLevel::Warn,
                                &format!(
                                    "Failed to send subagent completion to topic, falling back: {}",
                                    error
                                ),
                            );
                            let _ = send_subagent_reply_or_compact(&session, &notice);
                        }
                        if let Err(error) = close_forum_topic(session.chat_id, topic_thread_id) {
                            channel_host::log(
                                channel_host::LogLevel::Debug,
                                &format!("Failed to close temp forum topic: {}", error),
                            );
                        }
                    } else {
                        let _ = send_subagent_reply_or_compact(&session, &notice);
                    }
                }
                TelegramSubagentSessionMode::ReplyChain => {
                    let _ = send_subagent_reply_or_compact(&session, &notice);
                }
                TelegramSubagentSessionMode::CompactOff => {
                    let _ = send_subagent_compact_notice(&session, &notice);
                }
            }
            write_subagent_sessions(&sessions);
        }
    }
}
