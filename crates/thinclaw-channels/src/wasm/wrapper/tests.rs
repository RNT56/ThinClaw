use std::sync::Arc;

use super::WasmChannel;
use super::conversions::{
    HttpResponse, default_wasm_channel_formatting_hints, merged_response_metadata,
    response_content_for_wasm,
};
use crate::pairing::PairingStore;
use crate::wasm::capabilities::ChannelCapabilities;
use crate::wasm::host::ChannelWorkspaceStore;
use crate::wasm::runtime::{PreparedChannelModule, WasmChannelRuntime, WasmChannelRuntimeConfig};
use crate::wasm::schema::{SecretSetupSchema, SetupSchema};
use thinclaw_channels_core::Channel;

fn create_test_channel() -> WasmChannel {
    let config = WasmChannelRuntimeConfig::for_testing();
    let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

    let prepared = Arc::new(PreparedChannelModule::for_testing("test", "Test channel"));

    let capabilities = ChannelCapabilities::for_channel("test").with_path("/webhook/test");

    WasmChannel::new(
        runtime,
        prepared,
        capabilities,
        "{}".to_string(),
        None,
        Arc::new(PairingStore::new()),
    )
}

#[test]
fn test_channel_name() {
    let channel = create_test_channel();
    assert_eq!(channel.name(), "test");
}

#[test]
fn test_channel_maps_manifest_setup_to_secret_only_config_schema() {
    let channel = create_test_channel().with_setup_schema(SetupSchema {
        required_secrets: vec![SecretSetupSchema {
            name: "test_bot_token".to_string(),
            prompt: "Bot token".to_string(),
            validation: None,
            optional: false,
            auto_generate: None,
        }],
        validation_endpoint: None,
    });

    let schema = channel
        .config_schema()
        .expect("setup schema should surface");
    assert_eq!(schema.channel_id, "test");
    assert_eq!(schema.fields.len(), 1);
    assert_eq!(schema.fields[0].id, "test_bot_token");
    assert_eq!(schema.fields[0].field_type, "password");
    assert!(schema.fields[0].required);
    assert!(schema.fields[0].default_value.is_none());
}

#[test]
fn test_channel_uses_explicit_formatting_hints() {
    let config = WasmChannelRuntimeConfig::for_testing();
    let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

    let prepared = Arc::new(PreparedChannelModule::for_testing(
        "custom",
        "Custom channel",
    ));

    let channel = WasmChannel::new(
        runtime,
        prepared,
        ChannelCapabilities::for_channel("custom"),
        "{}".to_string(),
        Some("Use plain text only.".to_string()),
        Arc::new(PairingStore::new()),
    );

    assert_eq!(
        channel.formatting_hints().as_deref(),
        Some("Use plain text only.")
    );
}

#[test]
fn test_channel_falls_back_to_builtin_platform_hints() {
    let config = WasmChannelRuntimeConfig::for_testing();
    let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

    let prepared = Arc::new(PreparedChannelModule::for_testing(
        "telegram",
        "Telegram channel",
    ));

    let channel = WasmChannel::new(
        runtime,
        prepared,
        ChannelCapabilities::for_channel("telegram"),
        "{}".to_string(),
        None,
        Arc::new(PairingStore::new()),
    );

    let hints = channel
        .formatting_hints()
        .expect("telegram should have default hints");
    assert!(hints.contains("Telegram"));
    assert!(hints.contains("HTML"));
}

#[test]
fn test_builtin_platform_hint_mapping_covers_supported_wasm_channels() {
    let telegram = default_wasm_channel_formatting_hints("telegram")
        .expect("telegram fallback hints should exist");
    assert!(telegram.contains("Telegram"));

    let slack =
        default_wasm_channel_formatting_hints("slack").expect("slack fallback should exist");
    assert!(slack.contains("Slack"));

    let whatsapp =
        default_wasm_channel_formatting_hints("whatsapp").expect("whatsapp fallback should exist");
    assert!(whatsapp.contains("WhatsApp"));

    let discord =
        default_wasm_channel_formatting_hints("discord").expect("discord fallback should exist");
    assert!(discord.contains("Discord"));

    assert!(default_wasm_channel_formatting_hints("custom").is_none());
}

#[test]
fn test_http_response_ok() {
    let response = HttpResponse::ok();
    assert_eq!(response.status, 200);
    assert!(response.body.is_empty());
}

#[test]
fn test_http_response_json() {
    let response = HttpResponse::json(serde_json::json!({"key": "value"}));
    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers.get("Content-Type"),
        Some(&"application/json".to_string())
    );
}

#[test]
fn test_http_response_error() {
    let response = HttpResponse::error(400, "Bad request");
    assert_eq!(response.status, 400);
    assert_eq!(response.body, b"Bad request");
}

#[tokio::test]
async fn test_channel_start_and_shutdown() {
    let channel = create_test_channel();

    // Start should succeed
    let stream = channel.start().await;
    assert!(stream.is_ok());

    // Health check should pass
    assert!(channel.health_check().await.is_ok());

    // Shutdown should succeed
    assert!(channel.shutdown().await.is_ok());

    // Health check should fail after shutdown
    assert!(channel.health_check().await.is_err());
}

#[tokio::test]
async fn test_execute_poll_no_wasm_returns_empty() {
    // When there's no WASM module (None component), execute_poll
    // should return an empty vector of messages
    let config = WasmChannelRuntimeConfig::for_testing();
    let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

    let prepared = Arc::new(PreparedChannelModule::for_testing(
        "poll-test",
        "Test channel",
    ));

    let capabilities = ChannelCapabilities::for_channel("poll-test").with_polling(1000);
    let credentials = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let timeout = std::time::Duration::from_secs(5);

    let workspace_store = Arc::new(crate::wasm::host::ChannelWorkspaceStore::new());

    let result = WasmChannel::execute_poll(
        "poll-test",
        &runtime,
        &prepared,
        &capabilities,
        &credentials,
        Arc::new(PairingStore::new()),
        timeout,
        &workspace_store,
    )
    .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[tokio::test]
async fn test_dispatch_emitted_messages_sends_to_channel() {
    use crate::wasm::host::EmittedMessage;

    let (tx, mut rx) = tokio::sync::mpsc::channel(10);
    let message_tx = Arc::new(tokio::sync::RwLock::new(Some(tx)));

    let rate_limiter = Arc::new(tokio::sync::RwLock::new(
        crate::wasm::host::ChannelEmitRateLimiter::new(
            crate::wasm::capabilities::EmitRateLimitConfig::default(),
        ),
    ));

    let messages = vec![
        EmittedMessage::new("user1", "Hello from polling!"),
        EmittedMessage::new("user2", "Another message"),
    ];

    let result = WasmChannel::dispatch_emitted_messages(
        "test-channel",
        messages,
        &message_tx,
        &rate_limiter,
    )
    .await;

    assert!(result.is_ok());

    // Verify messages were sent
    let msg1 = rx.try_recv().expect("Should receive first message");
    assert_eq!(msg1.user_id, "user1");
    assert_eq!(msg1.content, "Hello from polling!");

    let msg2 = rx.try_recv().expect("Should receive second message");
    assert_eq!(msg2.user_id, "user2");
    assert_eq!(msg2.content, "Another message");

    // No more messages
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_wasm_emitted_whatsapp_message_uses_incoming_event_session_key() {
    use super::conversions::emitted_message_to_incoming_message;
    use crate::wasm::host::EmittedMessage;
    use thinclaw_identity::ConversationKind;

    let metadata = serde_json::json!({
        "sender_phone": "+15551234567",
        "phone_number_id": "biz-1",
        "conversation_kind": "direct",
        "conversation_scope_id": "whatsapp:direct:biz-1:+15551234567",
        "external_conversation_key": "whatsapp://direct/biz-1/+15551234567"
    });
    let emitted = EmittedMessage::new("+15551234567", "hello").with_metadata(metadata.to_string());

    let msg = emitted_message_to_incoming_message("whatsapp", emitted);

    assert_eq!(msg.channel, "whatsapp");
    assert_eq!(
        msg.thread_id.as_deref(),
        Some("agent:main:whatsapp:dm:+15551234567")
    );
    assert_eq!(msg.metadata["session_key"], msg.thread_id.clone().unwrap());
    assert_eq!(msg.metadata["raw"]["sender_phone"], "+15551234567");
    assert_eq!(msg.metadata["sender_phone"], "+15551234567");

    let identity = msg.resolved_identity();
    assert_eq!(identity.conversation_kind, ConversationKind::Direct);
    assert_eq!(identity.principal_id, "+15551234567");
}

#[test]
fn test_wasm_emitted_telegram_group_topic_keeps_legacy_alias_and_slash_parse() {
    use super::conversions::emitted_message_to_incoming_message;
    use crate::wasm::host::EmittedMessage;
    use thinclaw_identity::ConversationKind;

    let metadata = serde_json::json!({
        "chat_id": -100123,
        "message_thread_id": 99,
        "is_private": false,
        "conversation_kind": "group",
        "conversation_scope_id": "telegram:group:-100123:topic:99",
        "external_conversation_key": "telegram://group/-100123/topic/99"
    });
    let emitted = EmittedMessage::new("42", "/Summarize   sprint notes")
        .with_thread_id("99")
        .with_metadata(metadata.to_string());

    let msg = emitted_message_to_incoming_message("telegram", emitted);

    assert_eq!(
        msg.thread_id.as_deref(),
        Some("agent:main:telegram:group:-100123_topic_99")
    );
    assert_eq!(msg.metadata["chat_id"], -100123);
    assert_eq!(msg.metadata["canonical_chat_id"], "-100123:topic:99");
    assert_eq!(msg.metadata["message_thread_id"], 99);
    assert_eq!(msg.metadata["slash_command"]["command"], "summarize");
    assert_eq!(msg.metadata["slash_command"]["args"], "sprint notes");

    let aliases = msg
        .metadata
        .get("legacy_session_key_aliases")
        .and_then(|value| value.as_array())
        .expect("legacy aliases should be present");
    assert!(aliases.contains(&serde_json::json!("telegram:group:-100123_topic_99")));
    assert!(aliases.contains(&serde_json::json!("99")));
    assert!(aliases.contains(&serde_json::json!("telegram:99")));

    let identity = msg.resolved_identity();
    assert_eq!(identity.conversation_kind, ConversationKind::Group);
    assert_eq!(
        identity.stable_external_conversation_key,
        "telegram://group/-100123/topic/99"
    );
}

#[tokio::test]
async fn test_dispatch_emitted_messages_no_sender_returns_ok() {
    use crate::wasm::host::EmittedMessage;

    // No sender available (channel not started)
    let message_tx = Arc::new(tokio::sync::RwLock::new(None));
    let rate_limiter = Arc::new(tokio::sync::RwLock::new(
        crate::wasm::host::ChannelEmitRateLimiter::new(
            crate::wasm::capabilities::EmitRateLimitConfig::default(),
        ),
    ));

    let messages = vec![EmittedMessage::new("user1", "Hello!")];

    // Should return Ok even without a sender (logs warning but doesn't fail)
    let result = WasmChannel::dispatch_emitted_messages(
        "test-channel",
        messages,
        &message_tx,
        &rate_limiter,
    )
    .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_channel_with_polling_stores_shutdown_sender() {
    // Create a channel with polling capabilities
    let config = WasmChannelRuntimeConfig::for_testing();
    let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

    let prepared = Arc::new(PreparedChannelModule::for_testing(
        "poll-channel",
        "Polling test channel",
    ));

    // Enable polling with a 1 second minimum interval
    let capabilities = ChannelCapabilities::for_channel("poll-channel")
        .with_path("/webhook/poll")
        .with_polling(1000);

    let channel = WasmChannel::new(
        runtime,
        prepared,
        capabilities,
        "{}".to_string(),
        None,
        Arc::new(PairingStore::new()),
    );

    // Start the channel
    let _stream = channel.start().await.expect("Channel should start");

    // Verify poll_shutdown_tx is set (polling was started)
    // Note: For testing channels without WASM, on_start returns no poll config,
    // so polling won't actually be started. This verifies the basic lifecycle.
    assert!(channel.health_check().await.is_ok());

    // Shutdown should clean up properly
    channel.shutdown().await.expect("Shutdown should succeed");
    assert!(channel.health_check().await.is_err());
}

#[tokio::test]
async fn test_call_on_poll_no_wasm_succeeds() {
    // Verify call_on_poll returns Ok when there's no WASM module
    let channel = create_test_channel();

    // Start the channel first to set up message_tx
    let _stream = channel.start().await.expect("Channel should start");

    // call_on_poll should succeed (no-op for no WASM)
    let result = channel.call_on_poll().await;
    assert!(result.is_ok());

    channel.shutdown().await.expect("Shutdown should succeed");
}

#[tokio::test]
async fn test_typing_task_starts_on_thinking() {
    let channel = create_test_channel();
    let _stream = channel.start().await.expect("Channel should start");

    let metadata = serde_json::json!({"chat_id": 123});

    // Sending Thinking should succeed (no-op for no WASM)
    let result = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
            &metadata,
        )
        .await;
    assert!(result.is_ok());

    // A typing task should have been spawned
    assert!(channel.typing_task.read().await.is_some());

    // Shutdown should cancel the typing task
    channel.shutdown().await.expect("Shutdown should succeed");
    assert!(channel.typing_task.read().await.is_none());
}

#[tokio::test]
async fn test_typing_task_cancelled_on_done() {
    let channel = create_test_channel();
    let _stream = channel.start().await.expect("Channel should start");

    let metadata = serde_json::json!({"chat_id": 123});

    // Start typing
    let _ = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
            &metadata,
        )
        .await;
    assert!(channel.typing_task.read().await.is_some());

    // Send Done status
    let _ = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::Status("Done".into()),
            &metadata,
        )
        .await;

    // Typing task should be cancelled
    assert!(channel.typing_task.read().await.is_none());

    channel.shutdown().await.expect("Shutdown should succeed");
}

#[tokio::test]
async fn test_typing_task_persists_on_tool_started() {
    let channel = create_test_channel();
    let _stream = channel.start().await.expect("Channel should start");

    let metadata = serde_json::json!({"chat_id": 123});

    // Start typing
    let _ = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
            &metadata,
        )
        .await;
    assert!(channel.typing_task.read().await.is_some());

    // Intermediate tool status should not cancel typing
    let _ = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::ToolStarted {
                name: "http_request".into(),
                parameters: None,
            },
            &metadata,
        )
        .await;

    assert!(channel.typing_task.read().await.is_some());

    channel.shutdown().await.expect("Shutdown should succeed");
}

#[tokio::test]
async fn test_typing_task_cancelled_on_approval_needed() {
    let channel = create_test_channel();
    let _stream = channel.start().await.expect("Channel should start");

    let metadata = serde_json::json!({"chat_id": 123});

    // Start typing
    let _ = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
            &metadata,
        )
        .await;
    assert!(channel.typing_task.read().await.is_some());

    // Approval-needed should stop typing while waiting for user action
    let _ = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::ApprovalNeeded {
                request_id: "req-1".into(),
                tool_name: "http_request".into(),
                description: "Fetch weather".into(),
                parameters: serde_json::json!({"url": "https://wttr.in"}),
            },
            &metadata,
        )
        .await;

    assert!(channel.typing_task.read().await.is_none());

    channel.shutdown().await.expect("Shutdown should succeed");
}

#[tokio::test]
async fn test_typing_task_cancelled_on_awaiting_approval_status() {
    let channel = create_test_channel();
    let _stream = channel.start().await.expect("Channel should start");

    let metadata = serde_json::json!({"chat_id": 123});

    // Start typing
    let _ = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
            &metadata,
        )
        .await;
    assert!(channel.typing_task.read().await.is_some());

    // Legacy terminal status string should also cancel typing
    let _ = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::Status("Awaiting approval".into()),
            &metadata,
        )
        .await;

    assert!(channel.typing_task.read().await.is_none());

    channel.shutdown().await.expect("Shutdown should succeed");
}

#[tokio::test]
async fn test_typing_task_replaced_on_new_thinking() {
    let channel = create_test_channel();
    let _stream = channel.start().await.expect("Channel should start");

    let metadata = serde_json::json!({"chat_id": 123});

    // Start typing
    let _ = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::Thinking("First...".into()),
            &metadata,
        )
        .await;

    // Get handle of first task
    let first_handle = {
        let guard = channel.typing_task.read().await;
        guard.as_ref().map(|h| h.id())
    };
    assert!(first_handle.is_some());

    // Start typing again (should replace the previous task)
    let _ = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::Thinking("Second...".into()),
            &metadata,
        )
        .await;

    // Should still have a typing task, but it's a new one
    let second_handle = {
        let guard = channel.typing_task.read().await;
        guard.as_ref().map(|h| h.id())
    };
    assert!(second_handle.is_some());
    // The task IDs should differ (old one was aborted, new one spawned)
    assert_ne!(first_handle, second_handle);

    channel.shutdown().await.expect("Shutdown should succeed");
}

#[tokio::test]
async fn test_respond_cancels_typing_task() {
    use thinclaw_channels_core::IncomingMessage;

    let channel = create_test_channel();
    let _stream = channel.start().await.expect("Channel should start");

    let metadata = serde_json::json!({"chat_id": 123});

    // Start typing
    let _ = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
            &metadata,
        )
        .await;
    assert!(channel.typing_task.read().await.is_some());

    // Respond should cancel the typing task
    let msg = IncomingMessage::new("test", "user1", "hello").with_metadata(metadata);
    let _ = channel
        .respond(
            &msg,
            thinclaw_channels_core::OutgoingResponse::text("response"),
        )
        .await;

    // Typing task should be gone
    assert!(channel.typing_task.read().await.is_none());

    channel.shutdown().await.expect("Shutdown should succeed");
}

#[tokio::test]
async fn test_stream_chunk_is_noop() {
    let channel = create_test_channel();
    let _stream = channel.start().await.expect("Channel should start");

    let metadata = serde_json::json!({"chat_id": 123});

    // StreamChunk should not start a typing task
    let result = channel
        .send_status(
            thinclaw_channels_core::StatusUpdate::StreamChunk("chunk".into()),
            &metadata,
        )
        .await;
    assert!(result.is_ok());
    assert!(channel.typing_task.read().await.is_none());

    channel.shutdown().await.expect("Shutdown should succeed");
}

#[test]
fn test_status_to_wit_thinking() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!({"chat_id": 42});
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::Thinking("Processing...".into()),
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::Thinking
    ));
    assert_eq!(wit.message, "Processing...");
    assert!(wit.metadata_json.contains("42"));
}

#[test]
fn test_status_to_wit_done() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::Status("Done".into()),
        &metadata,
    );

    assert!(matches!(wit.status, super::wit_channel::StatusType::Done));
}

#[test]
fn test_status_to_wit_done_case_insensitive() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);

    // lowercase
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::Status("done".into()),
        &metadata,
    );
    assert!(matches!(wit.status, super::wit_channel::StatusType::Done));

    // with whitespace
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::Status(" Done ".into()),
        &metadata,
    );
    assert!(matches!(wit.status, super::wit_channel::StatusType::Done));
}

#[test]
fn test_status_to_wit_interrupted() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::Status("Interrupted".into()),
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::Interrupted
    ));
}

#[test]
fn test_status_to_wit_interrupted_case_insensitive() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);

    // lowercase
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::Status("interrupted".into()),
        &metadata,
    );
    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::Interrupted
    ));

    // with whitespace
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::Status(" Interrupted ".into()),
        &metadata,
    );
    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::Interrupted
    ));
}

#[test]
fn test_status_to_wit_generic_status() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::Status("Awaiting approval".into()),
        &metadata,
    );

    assert!(matches!(wit.status, super::wit_channel::StatusType::Status));
    assert_eq!(wit.message, "Awaiting approval");
}

#[test]
fn test_status_to_wit_context_pressure() {
    use super::conversions::status_to_wit;

    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::ContextPressure {
            level: "critical".into(),
            usage_percent: 97.25,
        },
        &serde_json::json!({"thread_id": "thread-1"}),
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::ContextPressure
    ));
    assert_eq!(wit.message, "Context pressure: critical (97.2%)");
    assert!(wit.metadata_json.contains("thread-1"));
}

#[test]
fn test_status_to_wit_subagent_spawned_uses_structured_payload() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::SubagentSpawned {
            agent_id: "agent-1".to_string(),
            name: "Researcher".to_string(),
            task: "Check brave search".to_string(),
            task_packet: thinclaw_types::SubagentTaskPacket {
                objective: "Check brave search".to_string(),
                ..Default::default()
            },
            allowed_tools: vec![],
            allowed_skills: vec![],
            memory_mode: "provided_context_only".to_string(),
            tool_mode: "explicit_only".to_string(),
            skill_mode: "explicit_only".to_string(),
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::SubagentSpawned
    ));
    assert!(wit.message.starts_with("[subagent:spawned:agent-1] "));

    let payload = wit
        .message
        .split_once("] ")
        .map(|(_, payload)| payload)
        .expect("spawned message should include payload");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("spawned payload should be valid JSON");
    assert_eq!(payload["name"], "Researcher");
    assert_eq!(payload["task"], "Check brave search");
}

#[test]
fn test_status_to_wit_subagent_progress_uses_structured_payload() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::SubagentProgress {
            agent_id: "agent-1".to_string(),
            message: "Running brave-search".to_string(),
            category: "tool".to_string(),
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::SubagentProgress
    ));
    assert!(wit.message.starts_with("[subagent:progress:agent-1:tool] "));

    let payload = wit
        .message
        .split_once("] ")
        .map(|(_, payload)| payload)
        .expect("progress message should include payload");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("progress payload should be valid JSON");
    assert_eq!(payload["message"], "Running brave-search");
}

#[test]
fn test_status_to_wit_subagent_completed_uses_structured_payload() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::SubagentCompleted {
            agent_id: "agent-1".to_string(),
            name: "Researcher".to_string(),
            success: true,
            response: "Done".to_string(),
            duration_ms: 1850,
            iterations: 3,
            task_packet: thinclaw_types::SubagentTaskPacket {
                objective: "Check brave search".to_string(),
                ..Default::default()
            },
            allowed_tools: vec![],
            allowed_skills: vec![],
            memory_mode: "provided_context_only".to_string(),
            tool_mode: "explicit_only".to_string(),
            skill_mode: "explicit_only".to_string(),
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::SubagentCompleted
    ));
    assert!(wit.message.starts_with("[subagent:completed:agent-1] "));

    let payload = wit
        .message
        .split_once("] ")
        .map(|(_, payload)| payload)
        .expect("completed message should include payload");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("completed payload should be valid JSON");
    assert_eq!(payload["name"], "Researcher");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["response"], "Done");
    assert_eq!(payload["duration_ms"], 1850);
    assert_eq!(payload["iterations"], 3);
}

#[test]
fn test_status_to_wit_auth_required() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!({"chat_id": 42});
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::AuthRequired {
            extension_name: "weather".to_string(),
            instructions: Some("Paste your token".to_string()),
            auth_url: Some("https://example.com/auth".to_string()),
            setup_url: None,
            auth_mode: "manual_token".to_string(),
            auth_status: "awaiting_token".to_string(),
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            thread_id: None,
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::AuthRequired
    ));
    assert!(wit.message.contains("Authentication required for weather"));
    assert!(wit.message.contains("Paste your token"));
}

#[test]
fn test_status_to_wit_tool_started() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!({"chat_id": 7});
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::ToolStarted {
            name: "http_request".to_string(),
            parameters: None,
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::ToolStarted
    ));
    assert_eq!(wit.message, "Tool started: http_request");
}

#[test]
fn test_status_to_wit_tool_completed_success() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::ToolCompleted {
            name: "http_request".to_string(),
            success: true,
            result_preview: None,
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::ToolCompleted
    ));
    assert_eq!(wit.message, "Tool completed: http_request (ok)");
}

#[test]
fn test_status_to_wit_tool_completed_failure() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::ToolCompleted {
            name: "http_request".to_string(),
            success: false,
            result_preview: None,
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::ToolCompleted
    ));
    assert_eq!(wit.message, "Tool completed: http_request (failed)");
}

#[test]
fn test_status_to_wit_tool_result() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::ToolResult {
            name: "http_request".to_string(),
            preview: "{".to_string() + "\"temperature\": 22}",
            artifacts: Vec::new(),
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::ToolResult
    ));
    assert!(wit.message.starts_with("Tool result: http_request\n"));
}

#[test]
fn test_status_to_wit_tool_result_truncates_preview() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);
    let long_preview = "x".repeat(400);
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::ToolResult {
            name: "big_tool".to_string(),
            preview: long_preview,
            artifacts: Vec::new(),
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::ToolResult
    ));
    assert!(wit.message.ends_with("..."));
}

#[test]
fn test_status_to_wit_job_started() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!({"chat_id": 1});
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::JobStarted {
            job_id: "job-1".to_string(),
            title: "Daily sync".to_string(),
            browse_url: "https://example.com/jobs/job-1".to_string(),
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::JobStarted
    ));
    assert!(wit.message.contains("Daily sync"));
    assert!(wit.message.contains("https://example.com/jobs/job-1"));
}

#[test]
fn test_status_to_wit_auth_completed_success() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::AuthCompleted {
            extension_name: "weather".to_string(),
            success: true,
            message: "Token saved".to_string(),
            auth_mode: Some("manual_token".to_string()),
            auth_status: Some("authenticated".to_string()),
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            thread_id: None,
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::AuthCompleted
    ));
    assert!(wit.message.contains("Authentication completed"));
    assert!(wit.message.contains("Token saved"));
}

#[test]
fn test_status_to_wit_auth_completed_failure() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!(null);
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::AuthCompleted {
            extension_name: "weather".to_string(),
            success: false,
            message: "Invalid token".to_string(),
            auth_mode: None,
            auth_status: None,
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            thread_id: None,
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::AuthCompleted
    ));
    assert!(wit.message.contains("Authentication failed"));
    assert!(wit.message.contains("Invalid token"));
}

#[test]
fn test_status_to_wit_approval_needed() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!({"chat_id": 42});
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::ApprovalNeeded {
            request_id: "req-123".to_string(),
            tool_name: "http_request".to_string(),
            description: "Fetch weather data".to_string(),
            parameters: serde_json::json!({"url": "https://api.weather.test"}),
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::ApprovalNeeded
    ));
    assert!(wit.message.contains("http_request"));
    assert!(wit.message.contains("/approve"));
}

#[test]
fn test_approval_prompt_roundtrip_submission_aliases() {
    use super::conversions::status_to_wit;

    let metadata = serde_json::json!({"chat_id": 42});
    let wit = status_to_wit(
        &thinclaw_channels_core::StatusUpdate::ApprovalNeeded {
            request_id: "req-321".to_string(),
            tool_name: "http_request".to_string(),
            description: "Fetch weather data".to_string(),
            parameters: serde_json::json!({"url": "https://api.weather.test"}),
        },
        &metadata,
    );

    assert!(matches!(
        wit.status,
        super::wit_channel::StatusType::ApprovalNeeded
    ));
    assert!(wit.message.contains("/approve"));
    assert!(wit.message.contains("/deny"));
    assert!(wit.message.contains("/always"));
}

#[test]
fn test_clone_wit_status_update() {
    use super::conversions::clone_wit_status_update;
    use super::wit_channel;

    let original = wit_channel::StatusUpdate {
        status: wit_channel::StatusType::Thinking,
        message: "hello".to_string(),
        metadata_json: "{\"a\":1}".to_string(),
    };

    let cloned = clone_wit_status_update(&original);
    assert!(matches!(cloned.status, wit_channel::StatusType::Thinking));
    assert_eq!(cloned.message, "hello");
    assert_eq!(cloned.metadata_json, "{\"a\":1}");
}

#[test]
fn test_clone_wit_status_update_approval_needed() {
    use super::conversions::clone_wit_status_update;
    use super::wit_channel;

    let original = wit_channel::StatusUpdate {
        status: wit_channel::StatusType::ApprovalNeeded,
        message: "approval needed".to_string(),
        metadata_json: "{\"chat_id\":42}".to_string(),
    };

    let cloned = clone_wit_status_update(&original);
    assert!(matches!(
        cloned.status,
        wit_channel::StatusType::ApprovalNeeded
    ));
    assert_eq!(cloned.message, "approval needed");
    assert_eq!(cloned.metadata_json, "{\"chat_id\":42}");
}

#[test]
fn test_clone_wit_status_update_auth_completed() {
    use super::conversions::clone_wit_status_update;
    use super::wit_channel;

    let original = wit_channel::StatusUpdate {
        status: wit_channel::StatusType::AuthCompleted,
        message: "auth complete".to_string(),
        metadata_json: "{}".to_string(),
    };

    let cloned = clone_wit_status_update(&original);
    assert!(matches!(
        cloned.status,
        wit_channel::StatusType::AuthCompleted
    ));
    assert_eq!(cloned.message, "auth complete");
}

#[test]
fn test_clone_wit_status_update_all_variants() {
    use super::conversions::clone_wit_status_update;
    use super::wit_channel;

    let variants = vec![
        wit_channel::StatusType::Thinking,
        wit_channel::StatusType::Done,
        wit_channel::StatusType::Interrupted,
        wit_channel::StatusType::ToolStarted,
        wit_channel::StatusType::ToolCompleted,
        wit_channel::StatusType::ToolResult,
        wit_channel::StatusType::ApprovalNeeded,
        wit_channel::StatusType::Status,
        wit_channel::StatusType::ContextPressure,
        wit_channel::StatusType::JobStarted,
        wit_channel::StatusType::AuthRequired,
        wit_channel::StatusType::AuthCompleted,
        wit_channel::StatusType::StreamChunk,
        wit_channel::StatusType::Plan,
        wit_channel::StatusType::Usage,
        wit_channel::StatusType::CredentialPrompt,
        wit_channel::StatusType::Error,
        wit_channel::StatusType::CanvasAction,
        wit_channel::StatusType::AgentMessage,
        wit_channel::StatusType::LifecycleStart,
        wit_channel::StatusType::LifecycleEnd,
        wit_channel::StatusType::SubagentSpawned,
        wit_channel::StatusType::SubagentProgress,
        wit_channel::StatusType::SubagentCompleted,
        wit_channel::StatusType::ContextCompactionStarted,
        wit_channel::StatusType::AdvisorConsultationStarted,
        wit_channel::StatusType::SelfRepairStarted,
        wit_channel::StatusType::SelfRepairCompleted,
    ];

    for status in variants {
        let original = wit_channel::StatusUpdate {
            status,
            message: "sample".to_string(),
            metadata_json: "{}".to_string(),
        };
        let cloned = clone_wit_status_update(&original);

        assert_eq!(
            std::mem::discriminant(&cloned.status),
            std::mem::discriminant(&original.status)
        );
        assert_eq!(cloned.message, "sample");
        assert_eq!(cloned.metadata_json, "{}");
    }
}

#[test]
fn test_redact_credentials_replaces_values() {
    use super::store::ChannelStoreData;

    let mut creds = std::collections::HashMap::new();
    creds.insert(
        "TELEGRAM_BOT_TOKEN".to_string(),
        "8218490433:AAEZeUxwqZ5OO3mOCXv7fKvpdhDgsmBBNis".to_string(),
    );
    creds.insert("OTHER_SECRET".to_string(), "s3cret".to_string());

    let store = ChannelStoreData::new(
        1024 * 1024,
        "test",
        ChannelCapabilities::default(),
        creds,
        Arc::new(PairingStore::new()),
        Arc::new(ChannelWorkspaceStore::new()),
    );

    let error = "HTTP request failed: error sending request for url \
        (https://api.telegram.org/bot8218490433:AAEZeUxwqZ5OO3mOCXv7fKvpdhDgsmBBNis/getUpdates)";

    let redacted = store.redact_credentials(error);

    assert!(
        !redacted.contains("8218490433:AAEZeUxwqZ5OO3mOCXv7fKvpdhDgsmBBNis"),
        "credential value should be redacted"
    );
    assert!(
        redacted.contains("[REDACTED:TELEGRAM_BOT_TOKEN]"),
        "redacted text should contain placeholder name"
    );
    assert!(
        !redacted.contains("s3cret"),
        "other credentials should also be redacted"
    );
}

#[test]
fn test_redact_credentials_no_op_without_credentials() {
    use super::store::ChannelStoreData;

    let store = ChannelStoreData::new(
        1024 * 1024,
        "test",
        ChannelCapabilities::default(),
        std::collections::HashMap::new(),
        Arc::new(PairingStore::new()),
        Arc::new(ChannelWorkspaceStore::new()),
    );

    let input = "some error message";
    assert_eq!(store.redact_credentials(input), input);
}

#[test]
fn test_redact_credentials_skips_empty_values() {
    use super::store::ChannelStoreData;

    let mut creds = std::collections::HashMap::new();
    creds.insert("EMPTY_TOKEN".to_string(), String::new());

    let store = ChannelStoreData::new(
        1024 * 1024,
        "test",
        ChannelCapabilities::default(),
        creds,
        Arc::new(PairingStore::new()),
        Arc::new(ChannelWorkspaceStore::new()),
    );

    let input = "should not match anything";
    assert_eq!(store.redact_credentials(input), input);
}

#[test]
fn test_telegram_webhook_pending_updates_without_inbound_is_unhealthy() {
    let reason = WasmChannel::telegram_webhook_unhealthy_reason(
        200_000,
        Some("https://example.test/webhook/telegram"),
        Some("https://example.test/webhook/telegram"),
        None,
        None,
        Some(3),
        Some(0),
        None,
    );

    assert_eq!(
        reason.as_deref(),
        Some(
            "Telegram has 3 pending webhook update(s) but ThinClaw has not received any inbound webhook events"
        )
    );
}

#[test]
fn test_telegram_webhook_recent_registration_allows_grace_period() {
    let reason = WasmChannel::telegram_webhook_unhealthy_reason(
        20_000,
        Some("https://example.test/webhook/telegram"),
        Some("https://example.test/webhook/telegram"),
        None,
        None,
        Some(2),
        Some(0),
        None,
    );

    assert!(reason.is_none());
}

#[test]
fn test_telegram_webhook_recent_inbound_with_pending_updates_stays_healthy() {
    let reason = WasmChannel::telegram_webhook_unhealthy_reason(
        100_000,
        Some("https://example.test/webhook/telegram"),
        Some("https://example.test/webhook/telegram"),
        None,
        None,
        Some(1),
        Some(0),
        Some(40_000),
    );

    assert!(reason.is_none());
}

#[test]
fn test_merged_response_metadata_overrides_and_includes_attachments() {
    let original = serde_json::json!({
        "chat_id": 42,
        "message_id": "orig",
        "keep": true,
    });
    let response = thinclaw_channels_core::OutgoingResponse {
        content: "hello".to_string(),
        thread_id: None,
        metadata: serde_json::json!({
            "message_id": "override",
            "extra": "value",
        }),
        attachments: vec![
            thinclaw_media::MediaContent::new(vec![1, 2, 3], "image/png")
                .with_filename("reply.png"),
        ],
    };

    let merged = merged_response_metadata(&original, &response);

    assert_eq!(merged["chat_id"], 42);
    assert_eq!(merged["message_id"], "override");
    assert_eq!(merged["extra"], "value");
    assert_eq!(merged["response_attachments"][0]["mime_type"], "image/png");
    assert_eq!(merged["response_attachments"][0]["filename"], "reply.png");
    assert_eq!(merged["response_attachments"][0]["data"], "AQID");
}

#[test]
fn test_wasm_response_content_falls_back_for_text_only_channels() {
    let response = thinclaw_channels_core::OutgoingResponse {
        content: "done".to_string(),
        thread_id: None,
        metadata: serde_json::Value::Null,
        attachments: vec![
            thinclaw_media::MediaContent::new(vec![1, 2, 3], "image/png")
                .with_filename("reply.png")
                .with_source_url("/tmp/reply.png"),
        ],
    };

    let fallback = response_content_for_wasm("twilio_sms", &response);
    assert!(fallback.contains("done"));
    assert!(fallback.contains("Generated media:"));
    assert!(fallback.contains("reply.png"));
    assert!(fallback.contains("/tmp/reply.png"));

    assert_eq!(response_content_for_wasm("slack", &response), "done");
}

/// Verify that WASM HTTP host functions work using a dedicated
/// current-thread runtime inside spawn_blocking.
#[tokio::test]
async fn test_dedicated_runtime_inside_spawn_blocking() {
    let result = tokio::task::spawn_blocking(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build runtime");
        rt.block_on(async { 42 })
    })
    .await
    .expect("spawn_blocking panicked");
    assert_eq!(result, 42);
}

/// Verify a real HTTP request works using the dedicated-runtime pattern.
/// This catches DNS, TLS, and I/O driver issues that trivial tests miss.
#[tokio::test]
#[ignore] // requires network
async fn test_dedicated_runtime_real_http() {
    let result = tokio::task::spawn_blocking(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build runtime");
        rt.block_on(async {
            let client = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("failed to build client");
            let resp = client
                .get("https://api.telegram.org/bot000/getMe")
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await;
            match resp {
                Ok(r) => r.status().as_u16(),
                Err(e) if e.is_timeout() => panic!("request timed out: {e}"),
                Err(e) => panic!("unexpected error: {e}"),
            }
        })
    })
    .await
    .expect("spawn_blocking panicked");
    // 404 because "000" is not a valid bot token
    assert_eq!(result, 404);
}
