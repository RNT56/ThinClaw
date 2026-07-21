use super::*;

#[test]
fn test_split_message_is_unicode_safe() {
    // A run of multibyte chars longer than the limit must not panic when
    // the limit boundary falls inside a multibyte character.
    let text = "🙂".repeat(5000);
    let chunks = split_message(&text, TELEGRAM_MAX_MESSAGE_LENGTH);

    assert_eq!(chunks.len(), 2);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.chars().count())
            .sum::<usize>(),
        5000
    );
}

#[test]
fn test_transport_preference_aliases() {
    assert_eq!(
        TelegramTransportPreference::from_str("auto"),
        Some(TelegramTransportPreference::Auto)
    );
    assert_eq!(
        TelegramTransportPreference::from_str("webhook"),
        Some(TelegramTransportPreference::Auto)
    );
    assert_eq!(
        TelegramTransportPreference::from_str("poll"),
        Some(TelegramTransportPreference::Polling)
    );
    assert_eq!(
        TelegramTransportPreference::from_str("disabled"),
        Some(TelegramTransportPreference::Polling)
    );
    assert_eq!(TelegramTransportPreference::from_str("mystery"), None);
}

#[test]
fn test_clean_message_text() {
    // Without bot_username: strips any leading @mention
    assert_eq!(clean_message_text("/start hello", None), "hello");
    assert_eq!(clean_message_text("@bot hello world", None), "hello world");
    assert_eq!(clean_message_text("/start", None), "");
    assert_eq!(clean_message_text("@botname", None), "");
    assert_eq!(clean_message_text("just text", None), "just text");
    assert_eq!(clean_message_text("  spaced  ", None), "spaced");

    // With bot_username: only strips @MyBot, not @alice
    assert_eq!(clean_message_text("@MyBot hello", Some("MyBot")), "hello");
    assert_eq!(clean_message_text("@mybot hi", Some("MyBot")), "hi");
    assert_eq!(
        clean_message_text("@alice hello", Some("MyBot")),
        "@alice hello"
    );
    assert_eq!(clean_message_text("@MyBot", Some("MyBot")), "");
}

#[test]
fn test_clean_message_text_bare_commands() {
    // Bare commands return empty (the caller decides what to emit)
    assert_eq!(clean_message_text("/start", None), "");
    assert_eq!(clean_message_text("/interrupt", None), "");
    assert_eq!(clean_message_text("/stop", None), "");
    assert_eq!(clean_message_text("/help", None), "");
    assert_eq!(clean_message_text("/undo", None), "");
    assert_eq!(clean_message_text("/ping", None), "");

    // Commands with args: command prefix stripped, args returned
    assert_eq!(clean_message_text("/start hello", None), "hello");
    assert_eq!(clean_message_text("/help me please", None), "me please");
    assert_eq!(
        clean_message_text("/model claude-opus-4-6", None),
        "claude-opus-4-6"
    );
}

/// Tests for the content_to_emit logic in handle_message.
/// Since handle_message uses WASM host calls, test the extracted decision function.
#[test]
fn test_content_to_emit_logic() {
    // /start → welcome placeholder
    assert_eq!(
        content_to_emit_for_agent("/start", None),
        Some("[User started the bot]".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/Start", None),
        Some("[User started the bot]".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("  /start  ", None),
        Some("[User started the bot]".to_string())
    );

    // /start with args → pass args through
    assert_eq!(
        content_to_emit_for_agent("/start hello", None),
        Some("hello".to_string())
    );

    // Control commands → pass through raw so Submission::parse() can match
    assert_eq!(
        content_to_emit_for_agent("/interrupt", None),
        Some("/interrupt".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/stop", None),
        Some("/stop".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/help", None),
        Some("/help".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/undo", None),
        Some("/undo".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/redo", None),
        Some("/redo".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/ping", None),
        Some("/ping".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/tools", None),
        Some("/tools".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/compact", None),
        Some("/compact".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/clear", None),
        Some("/clear".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/version", None),
        Some("/version".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/approve", None),
        Some("/approve".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/always", None),
        Some("/always".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/deny", None),
        Some("/deny".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/yes", None),
        Some("/yes".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("/no", None),
        Some("/no".to_string())
    );

    // Commands with args → cleaned text (command stripped)
    assert_eq!(
        content_to_emit_for_agent("/help me please", None),
        Some("me please".to_string())
    );

    // Plain text → pass through
    assert_eq!(
        content_to_emit_for_agent("hello world", None),
        Some("hello world".to_string())
    );
    assert_eq!(
        content_to_emit_for_agent("just text", None),
        Some("just text".to_string())
    );

    // Empty / whitespace → skip (None)
    assert_eq!(content_to_emit_for_agent("", None), None);
    assert_eq!(content_to_emit_for_agent("   ", None), None);

    // Bare @mention without bot → skip
    assert_eq!(content_to_emit_for_agent("@botname", None), None);

    // With bot username configured: other mentions are preserved.
    assert_eq!(
        content_to_emit_for_agent("@alice hello", Some("MyBot")),
        Some("@alice hello".to_string())
    );
}

#[test]
fn test_config_with_owner_id() {
    let json = r#"{"owner_id": 123456789}"#;
    let config: TelegramConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.owner_id, Some(123456789));
}

#[test]
fn test_config_without_owner_id() {
    let json = r#"{}"#;
    let config: TelegramConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.owner_id, None);
}

#[test]
fn test_config_with_null_owner_id() {
    let json = r#"{"owner_id": null}"#;
    let config: TelegramConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.owner_id, None);
}

#[test]
fn test_config_full() {
    let json = r#"{
        "bot_username": "my_bot",
        "owner_id": 42,
        "respond_to_all_group_messages": true,
        "telegram_subagent_session_mode": "reply_chain"
    }"#;
    let config: TelegramConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.bot_username, Some("my_bot".to_string()));
    assert_eq!(config.owner_id, Some(42));
    assert!(config.respond_to_all_group_messages);
    assert_eq!(config.subagent_session_mode.as_deref(), Some("reply_chain"));
}

#[test]
fn test_extract_subagent_session_mode_from_nested_config() {
    let value = serde_json::json!({
        "channels": {
            "telegram_subagent_session_mode": "compact_off"
        }
    });

    assert_eq!(
        extract_subagent_session_mode_from_value(&value).as_deref(),
        Some("compact_off")
    );
}

#[test]
fn test_parse_telegram_metadata_with_nested_subagent_mode() {
    let raw = serde_json::json!({
        "chat_id": 123,
        "message_id": 456,
        "user_id": 789,
        "is_private": false,
        "channels": {
            "telegram_subagent_session_mode": "reply_chain"
        }
    })
    .to_string();

    let metadata = parse_telegram_metadata(&raw).unwrap();
    assert_eq!(
        metadata.subagent_session_mode.as_deref(),
        Some("reply_chain")
    );
}

#[test]
fn test_resolve_subagent_session_mode_prefers_metadata_override() {
    let metadata = TelegramMessageMetadata {
        chat_id: 1,
        message_id: 2,
        user_id: 3,
        is_private: true,
        message_thread_id: None,
        conversation_kind: None,
        conversation_scope_id: None,
        external_conversation_key: None,
        raw_sender_id: None,
        stable_sender_id: None,
        subagent_session_mode: Some("compact_off".to_string()),
    };

    assert_eq!(
        resolve_subagent_session_mode(&metadata),
        TelegramSubagentSessionMode::CompactOff
    );
}

#[test]
fn test_parse_update() {
    let json = r#"{
        "update_id": 123,
        "message": {
            "message_id": 456,
            "from": {
                "id": 789,
                "is_bot": false,
                "first_name": "John",
                "last_name": "Doe"
            },
            "chat": {
                "id": 789,
                "type": "private"
            },
            "text": "Hello bot"
        }
    }"#;

    let update: TelegramUpdate = serde_json::from_str(json).unwrap();
    assert_eq!(update.update_id, 123);

    let message = update.message.unwrap();
    assert_eq!(message.message_id, 456);
    assert_eq!(message.text.unwrap(), "Hello bot");

    let from = message.from.unwrap();
    assert_eq!(from.id, 789);
    assert_eq!(from.first_name, "John");
}

#[test]
fn test_parse_message_with_caption() {
    let json = r#"{
        "message_id": 1,
        "from": {"id": 1, "is_bot": false, "first_name": "A"},
        "chat": {"id": 1, "type": "private"},
        "caption": "What's in this image?"
    }"#;
    let msg: TelegramMessage = serde_json::from_str(json).unwrap();
    assert_eq!(msg.text, None);
    assert_eq!(msg.caption.as_deref(), Some("What's in this image?"));
}

#[test]
fn test_get_updates_url_includes_offset_and_timeout() {
    let url = get_updates_url(444_809_884, 30);
    assert!(url.contains("offset=444809884"));
    assert!(url.contains("timeout=30"));
    assert!(url.contains("allowed_updates=[\"message\",\"edited_message\"]"));
}

#[test]
fn test_normalized_conversation_metadata() {
    assert_eq!(conversation_kind(true), "direct");
    assert_eq!(conversation_kind(false), "group");
    assert_eq!(conversation_scope_id(42, None, true), "telegram:direct:42");
    assert_eq!(
        conversation_scope_id(42, Some(7), false),
        "telegram:group:42:topic:7"
    );
    assert_eq!(
        external_conversation_key(42, Some(7), false),
        "telegram://group/42/topic/7"
    );
}

#[test]
fn test_normalized_message_thread_id_preserves_private_topics() {
    assert_eq!(normalized_message_thread_id(Some(61419)), Some(61419));
    assert_eq!(normalized_message_thread_id(None), None);
    assert_eq!(normalized_message_thread_id(Some(7)), Some(7));
}

#[test]
fn test_private_topic_metadata_keeps_direct_scope_key() {
    assert_eq!(
        conversation_scope_id(42, Some(61419), true),
        "telegram:direct:42"
    );
    assert_eq!(
        external_conversation_key(42, Some(61419), true),
        "telegram://direct/42"
    );
}

#[test]
fn test_managed_private_topic_kind_aliases() {
    assert_eq!(
        ManagedPrivateTopicKind::from_response_thread_id(Some("bootstrap")),
        Some(ManagedPrivateTopicKind::Onboarding)
    );
    assert_eq!(
        ManagedPrivateTopicKind::from_response_thread_id(Some("onboarding")),
        Some(ManagedPrivateTopicKind::Onboarding)
    );
    assert_eq!(
        ManagedPrivateTopicKind::from_response_thread_id(Some("boot")),
        Some(ManagedPrivateTopicKind::General)
    );
    assert_eq!(
        ManagedPrivateTopicKind::from_response_thread_id(Some("general")),
        Some(ManagedPrivateTopicKind::General)
    );
    assert_eq!(
        ManagedPrivateTopicKind::from_response_thread_id(Some("61419")),
        None
    );
    assert_eq!(ManagedPrivateTopicKind::from_response_thread_id(None), None);
}

#[test]
fn test_managed_private_topic_kind_for_thread_id_matches_registry_entries() {
    let state = ManagedPrivateTopicState {
        onboarding_thread_id: Some(61419),
        general_thread_id: Some(7),
    };

    assert_eq!(
        managed_private_topic_kind_for_thread_id(&state, 61419),
        Some(ManagedPrivateTopicKind::Onboarding)
    );
    assert_eq!(
        managed_private_topic_kind_for_thread_id(&state, 7),
        Some(ManagedPrivateTopicKind::General)
    );
    assert_eq!(managed_private_topic_kind_for_thread_id(&state, 99), None);
}

#[test]
fn test_incoming_session_thread_id_for_kind_collapses_managed_private_topics() {
    assert_eq!(
        incoming_session_thread_id_for_kind(Some(61419), Some(ManagedPrivateTopicKind::Onboarding)),
        None
    );
    assert_eq!(
        incoming_session_thread_id_for_kind(Some(7), Some(ManagedPrivateTopicKind::General)),
        None
    );
    assert_eq!(
        incoming_session_thread_id_for_kind(Some(99), None),
        Some("99".to_string())
    );
    assert_eq!(incoming_session_thread_id_for_kind(None, None), None);
}

#[test]
fn test_tool_result_deleted_path_detects_bootstrap_delete() {
    let update = StatusUpdate {
        status: StatusType::ToolResult,
        message: "Tool result: memory_delete\n{\"status\":\"deleted\",\"path\":\"BOOTSTRAP.md\"}"
            .to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(
        tool_result_deleted_path(&update),
        Some("BOOTSTRAP.md".to_string())
    );
}

#[test]
fn test_tool_result_deleted_path_ignores_other_tools() {
    let update = StatusUpdate {
        status: StatusType::ToolResult,
        message: "Tool result: memory_read\n{\"status\":\"ok\",\"path\":\"BOOTSTRAP.md\"}"
            .to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(tool_result_deleted_path(&update), None);
}

#[test]
fn test_should_ensure_general_topic_after_status_only_for_private_bootstrap_delete() {
    let update = StatusUpdate {
        status: StatusType::ToolResult,
        message: "Tool result: memory_delete\n{\"status\":\"deleted\",\"path\":\"BOOTSTRAP.md\"}"
            .to_string(),
        metadata_json: "{}".to_string(),
    };
    let private_metadata = TelegramMessageMetadata {
        chat_id: 42,
        message_id: 7,
        user_id: 1,
        is_private: true,
        message_thread_id: Some(61419),
        conversation_kind: Some("direct".to_string()),
        conversation_scope_id: Some("telegram:direct:42".to_string()),
        external_conversation_key: Some("telegram://direct/42".to_string()),
        raw_sender_id: Some("telegram:user:1".to_string()),
        stable_sender_id: Some("telegram:user:1".to_string()),
        subagent_session_mode: None,
    };
    let mut group_metadata = private_metadata.clone();
    group_metadata.is_private = false;

    assert!(should_ensure_general_topic_after_status(
        &private_metadata,
        &update
    ));
    assert!(!should_ensure_general_topic_after_status(
        &group_metadata,
        &update
    ));
}

#[test]
fn test_should_ensure_general_topic_after_status_accepts_leading_slash_bootstrap_path() {
    let update = StatusUpdate {
        status: StatusType::ToolResult,
        message: "Tool result: memory_delete\n{\"status\":\"deleted\",\"path\":\"/BOOTSTRAP.md\"}"
            .to_string(),
        metadata_json: "{}".to_string(),
    };
    let metadata = TelegramMessageMetadata {
        chat_id: 42,
        message_id: 7,
        user_id: 1,
        is_private: true,
        message_thread_id: Some(61419),
        conversation_kind: Some("direct".to_string()),
        conversation_scope_id: Some("telegram:direct:42".to_string()),
        external_conversation_key: Some("telegram://direct/42".to_string()),
        raw_sender_id: Some("telegram:user:1".to_string()),
        stable_sender_id: Some("telegram:user:1".to_string()),
        subagent_session_mode: None,
    };

    assert!(should_ensure_general_topic_after_status(&metadata, &update));
}

#[test]
fn test_classify_status_update_thinking() {
    let update = StatusUpdate {
        status: StatusType::Thinking,
        message: "Thinking...".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(
        classify_status_update(&update),
        Some(TelegramStatusAction::Typing)
    );
}

#[test]
fn test_classify_status_update_approval_needed() {
    let update = StatusUpdate {
        status: StatusType::ApprovalNeeded,
        message: "Approval needed for tool 'http_request'".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(
        classify_status_update(&update),
        Some(TelegramStatusAction::Notify(
            "Approval needed for tool 'http_request'".to_string()
        ))
    );
}

#[test]
fn test_classify_status_update_done_ignored() {
    let update = StatusUpdate {
        status: StatusType::Done,
        message: "Done".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(classify_status_update(&update), None);
}

#[test]
fn test_classify_status_update_auth_required() {
    let update = StatusUpdate {
        status: StatusType::AuthRequired,
        message: "Authentication required for weather.".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(
        classify_status_update(&update),
        Some(TelegramStatusAction::Notify(
            "Authentication required for weather.".to_string()
        ))
    );
}

#[test]
fn test_classify_status_update_tool_started_notify() {
    let update = StatusUpdate {
        status: StatusType::ToolStarted,
        message: "Tool started: http_request".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(
        classify_status_update(&update),
        Some(TelegramStatusAction::Notify(
            "Tool started: http_request".to_string()
        ))
    );
}

#[test]
fn test_classify_status_update_tool_completed_notify() {
    let update = StatusUpdate {
        status: StatusType::ToolCompleted,
        message: "Tool completed: http_request (ok)".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(
        classify_status_update(&update),
        Some(TelegramStatusAction::Notify(
            "Tool completed: http_request (ok)".to_string()
        ))
    );
}

#[test]
fn test_classify_status_update_job_started_notify() {
    let update = StatusUpdate {
        status: StatusType::JobStarted,
        message: "Job started: Daily sync".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(
        classify_status_update(&update),
        Some(TelegramStatusAction::Notify(
            "Job started: Daily sync".to_string()
        ))
    );
}

#[test]
fn test_classify_status_update_auth_completed_notify() {
    let update = StatusUpdate {
        status: StatusType::AuthCompleted,
        message: "Authentication completed for weather.".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(
        classify_status_update(&update),
        Some(TelegramStatusAction::Notify(
            "Authentication completed for weather.".to_string()
        ))
    );
}

#[test]
fn test_classify_status_update_tool_result_notify() {
    let update = StatusUpdate {
        status: StatusType::ToolResult,
        message: "Tool result: http_request ...".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(
        classify_status_update(&update),
        Some(TelegramStatusAction::Notify(
            "Tool result: http_request ...".to_string()
        ))
    );
}

#[test]
fn test_classify_status_update_awaiting_approval_ignored() {
    let update = StatusUpdate {
        status: StatusType::Status,
        message: "Awaiting approval".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(classify_status_update(&update), None);
}

#[test]
fn test_classify_status_update_interrupted_ignored() {
    let update = StatusUpdate {
        status: StatusType::Interrupted,
        message: "Interrupted".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(classify_status_update(&update), None);
}

#[test]
fn test_classify_status_update_status_done_ignored_case_insensitive() {
    let update = StatusUpdate {
        status: StatusType::Status,
        message: "done".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(classify_status_update(&update), None);
}

#[test]
fn test_classify_status_update_status_interrupted_ignored() {
    let update = StatusUpdate {
        status: StatusType::Status,
        message: "interrupted".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(classify_status_update(&update), None);
}

#[test]
fn test_classify_status_update_status_rejected_ignored() {
    let update = StatusUpdate {
        status: StatusType::Status,
        message: "Rejected".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(classify_status_update(&update), None);
}

#[test]
fn test_classify_status_update_status_notify() {
    let update = StatusUpdate {
        status: StatusType::Status,
        message: "Context compaction started".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(
        classify_status_update(&update),
        Some(TelegramStatusAction::Notify(
            "Context compaction started".to_string()
        ))
    );
}

#[test]
fn test_classify_status_update_context_pressure_notify() {
    let update = StatusUpdate {
        status: StatusType::ContextPressure,
        message: "Context pressure: critical (97.0%)".to_string(),
        metadata_json: r#"{"level":"critical","usage_percent":97.0}"#.to_string(),
    };

    assert_eq!(
        classify_status_update(&update),
        Some(TelegramStatusAction::Notify(
            "Context pressure: critical (97.0%)".to_string()
        ))
    );
}

#[test]
fn test_parse_subagent_event_spawned_legacy() {
    assert_eq!(
        parse_subagent_event("[subagent:spawned:agent-1] Researcher - Check brave search"),
        Some(SubagentEvent::Spawned {
            agent_id: "agent-1".to_string(),
            name: "Researcher".to_string(),
            task: "Check brave search".to_string(),
        })
    );
}

#[test]
fn test_parse_subagent_event_progress_json() {
    assert_eq!(
        parse_subagent_event(
            r#"[subagent:progress:agent-1:tool] {"message":"Running brave-search"}"#
        ),
        Some(SubagentEvent::Progress {
            agent_id: "agent-1".to_string(),
            category: "tool".to_string(),
            message: "Running brave-search".to_string(),
        })
    );
}

#[test]
fn test_parse_subagent_event_completed_json() {
    assert_eq!(
        parse_subagent_event(
            r#"[subagent:completed:agent-1] {"name":"Researcher","success":true,"response":"Done","duration_ms":1850,"iterations":3}"#
        ),
        Some(SubagentEvent::Completed {
            agent_id: "agent-1".to_string(),
            name: "Researcher".to_string(),
            success: true,
            response: Some("Done".to_string()),
            duration_ms: Some(1850),
            iterations: Some(3),
        })
    );
}

#[test]
fn test_classify_status_update_subagent_event() {
    let update = StatusUpdate {
        status: StatusType::Status,
        message: r#"[subagent:progress:agent-1:tool] {"message":"Running brave-search"}"#
            .to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(
        classify_status_update(&update),
        Some(TelegramStatusAction::Subagent(SubagentEvent::Progress {
            agent_id: "agent-1".to_string(),
            category: "tool".to_string(),
            message: "Running brave-search".to_string(),
        }))
    );
}

#[test]
fn test_prune_orphaned_subagent_sessions_removes_stale_entries() {
    let now = 100_000u64;
    let mut sessions = std::collections::HashMap::new();
    sessions.insert(
        "stale-agent".to_string(),
        StoredSubagentSession {
            chat_id: 1,
            parent_message_id: 10,
            parent_thread_id: None,
            topic_thread_id: None,
            mode: "reply_chain".to_string(),
            last_touched_epoch_secs: now.saturating_sub(SUBAGENT_SESSION_TTL_SECS + 1),
        },
    );
    sessions.insert(
        "fresh-agent".to_string(),
        StoredSubagentSession {
            chat_id: 1,
            parent_message_id: 11,
            parent_thread_id: None,
            topic_thread_id: None,
            mode: "reply_chain".to_string(),
            last_touched_epoch_secs: now,
        },
    );

    let removed = prune_orphaned_subagent_sessions(&mut sessions, now, false);
    assert_eq!(removed, 1);
    assert!(!sessions.contains_key("stale-agent"));
    assert!(sessions.contains_key("fresh-agent"));
}

#[test]
fn test_prune_orphaned_subagent_sessions_enforces_store_cap() {
    let now = 20_000u64;
    let mut sessions = std::collections::HashMap::new();

    for idx in 0..(SUBAGENT_SESSION_STORE_CAP + 3) {
        sessions.insert(
            format!("agent-{idx}"),
            StoredSubagentSession {
                chat_id: 1,
                parent_message_id: idx as i64,
                parent_thread_id: None,
                topic_thread_id: None,
                mode: "reply_chain".to_string(),
                last_touched_epoch_secs: now
                    .saturating_sub((SUBAGENT_SESSION_STORE_CAP + 3 - idx) as u64),
            },
        );
    }

    let removed = prune_orphaned_subagent_sessions(&mut sessions, now, false);
    assert_eq!(removed, 3);
    assert_eq!(sessions.len(), SUBAGENT_SESSION_STORE_CAP);
}

#[test]
fn test_status_message_for_user_ignores_blank() {
    let update = StatusUpdate {
        status: StatusType::AuthRequired,
        message: "   ".to_string(),
        metadata_json: "{}".to_string(),
    };

    assert_eq!(status_message_for_user(&update), None);
}

#[test]
fn test_truncate_status_message_appends_ellipsis() {
    let input = "abcdefghijklmnopqrstuvwxyz";
    let output = truncate_status_message(input, 10);
    assert_eq!(output, "abcdefghij...");
}

#[test]
fn test_status_message_for_user_truncates_long_input() {
    let update = StatusUpdate {
        status: StatusType::AuthRequired,
        message: "x".repeat(700),
        metadata_json: "{}".to_string(),
    };

    let msg = status_message_for_user(&update).expect("expected message");
    assert!(msg.len() <= TELEGRAM_STATUS_MAX_CHARS + 3);
    assert!(msg.ends_with("..."));
}
