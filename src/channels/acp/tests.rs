use super::*;
use thinclaw_channels::acp::session_modes;

fn assert_json_schema_valid(schema: &Value, instance: &Value) {
    let compiled = jsonschema::JSONSchema::compile(schema).expect("schema fixture compiles");
    if let Err(errors) = compiled.validate(instance) {
        panic!(
            "ACP message did not match schema fixture: {}",
            errors
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
}

#[test]
fn prompt_to_text_extracts_text_and_resources() {
    let prompt = json!([
        { "type": "text", "text": "Review this" },
        { "type": "resource", "resource": { "uri": "file:///tmp/a.rs", "text": "fn main() {}" } },
        { "type": "resourceLink", "uri": "file:///tmp/b.rs" }
    ]);
    let text = prompt_to_text_result(&prompt).expect("prompt text");
    assert!(text.contains("Review this"));
    assert!(text.contains("file:///tmp/a.rs"));
    assert!(text.contains("fn main()"));
    assert!(text.contains("file:///tmp/b.rs"));
}

#[test]
fn acp_content_blocks_round_trip_resource_shapes() {
    let resource = wire::ContentBlock::embedded_text_resource("file:///tmp/a.rs", "fn main() {}");
    let resource_json = serde_json::to_value(resource).expect("resource content json");
    assert_eq!(resource_json["type"], json!("resource"));
    assert_eq!(resource_json["resource"]["uri"], json!("file:///tmp/a.rs"));
    assert_eq!(resource_json["resource"]["text"], json!("fn main() {}"));
    assert_eq!(resource_json["resource"]["mimeType"], json!("text/plain"));

    let link = wire::ContentBlock::resource_link("file:///tmp/b.rs");
    let link_json = serde_json::to_value(link).expect("resource link json");
    assert_eq!(link_json["type"], json!("resource_link"));
    assert_eq!(link_json["uri"], json!("file:///tmp/b.rs"));

    let legacy_link: wire::ContentBlock =
        serde_json::from_value(json!({ "type": "resourceLink", "uri": "file:///tmp/c.rs" }))
            .expect("legacy resourceLink input");
    assert!(matches!(
        legacy_link,
        wire::ContentBlock::ResourceLink { .. }
    ));
}

#[test]
fn prompt_to_text_rejects_invalid_typed_content_blocks() {
    let err = prompt_to_text_result(&json!([{ "type": "resource_link" }]))
        .expect_err("resource links must include a uri");
    assert_eq!(err.code, -32602);
    assert!(err.message.contains("resource_link"));
}

#[test]
fn prompt_to_text_rejects_unadvertised_media() {
    let err = prompt_to_text_result(&json!([{ "type": "image", "data": "abc" }]))
        .expect_err("image prompts are not advertised");
    assert_eq!(err.code, -32602);
    assert!(err.message.contains("not advertised"));
}

#[tokio::test]
async fn initialize_advertises_protocol_one_and_stateful_capabilities() {
    let state = Arc::new(AcpConnectionState::default());
    let response = handle_initialize(
        &state,
        &json!({
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": { "readTextFile": true, "writeTextFile": true },
                "terminal": true
            },
            "clientInfo": { "name": "test-client", "version": "1.0.0" }
        }),
    )
    .await
    .expect("initialize");

    assert_eq!(response["protocolVersion"], json!(1));
    assert_eq!(response["agentInfo"]["name"], json!("thinclaw"));
    assert_eq!(response["agentCapabilities"]["loadSession"], json!(true));
    assert_eq!(response["_meta"]["toolProfile"], json!("acp"));
    assert_eq!(response["meta"], Value::Null);
    assert_eq!(
        response["agentCapabilities"]["sessionCapabilities"]["resume"],
        Value::Null,
        "session/resume is a compatibility handler, not an advertised ACP v1 capability"
    );
    assert_eq!(
        state.client_capabilities().await.fs.read_text_file,
        true,
        "client capabilities should be stored"
    );
}

#[tokio::test]
async fn session_new_requires_absolute_cwd() {
    let state = Arc::new(AcpConnectionState::default());
    let _ = handle_initialize(
        &state,
        &json!({ "protocolVersion": 1, "clientCapabilities": {} }),
    )
    .await
    .expect("initialize");
    let err = handle_new_session(
        None,
        None,
        &state,
        &json!({ "cwd": "relative/path", "mcpServers": [] }),
    )
    .await
    .expect_err("relative cwd should fail");
    assert_eq!(err.code, -32602);
}

#[tokio::test]
async fn session_list_returns_created_sessions() {
    let state = Arc::new(AcpConnectionState::default());
    let _ = handle_initialize(
        &state,
        &json!({ "protocolVersion": 1, "clientCapabilities": {} }),
    )
    .await
    .expect("initialize");
    let created = handle_new_session(
        None,
        None,
        &state,
        &json!({ "cwd": "/tmp", "mcpServers": [] }),
    )
    .await
    .expect("new session");
    let listed = handle_list_sessions(None, &state, &json!({ "cwd": "/tmp" }))
        .await
        .expect("list sessions");

    assert_eq!(listed["sessions"][0]["sessionId"], created["sessionId"]);
    assert_eq!(listed["nextCursor"], Value::Null);
}

#[tokio::test]
async fn session_set_config_option_is_known_but_rejects_unadvertised_options() {
    let state = Arc::new(AcpConnectionState::default());
    let _ = handle_initialize(
        &state,
        &json!({ "protocolVersion": 1, "clientCapabilities": {} }),
    )
    .await
    .expect("initialize");
    let created = handle_new_session(
        None,
        None,
        &state,
        &json!({ "cwd": "/tmp", "mcpServers": [] }),
    )
    .await
    .expect("new session");
    let err = handle_set_config_option(
        &state,
        &json!({
            "sessionId": created["sessionId"],
            "configId": "model",
            "value": "fast"
        }),
    )
    .await
    .expect_err("no config options are currently advertised");
    assert_eq!(err.code, -32602);
    assert_eq!(err.data.unwrap()["configOptions"], json!([]));

    let err = handle_set_config_option(
        &state,
        &json!({
            "sessionId": created["sessionId"],
            "configId": "approval",
            "value": { "mode": "ask" }
        }),
    )
    .await
    .expect_err("non-string config values should still parse before rejection");
    assert_eq!(err.code, -32602);
    assert_eq!(err.data.unwrap()["configOptions"], json!([]));
}

#[tokio::test]
async fn session_load_replays_in_process_transcript_in_order() {
    let state = Arc::new(AcpConnectionState::default());
    let _ = handle_initialize(
        &state,
        &json!({ "protocolVersion": 1, "clientCapabilities": {} }),
    )
    .await
    .expect("initialize");
    let session_id = Uuid::new_v4().to_string();
    let mut session =
        AcpSessionState::new(session_id.clone(), "/tmp/project".to_string(), Vec::new());
    session.append_transcript("user", "first prompt");
    session.append_transcript("assistant", "first answer");
    state.upsert_session(session).await;

    let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
    let loaded = handle_load_session(
        None,
        &writer_tx,
        &state,
        &json!({
            "sessionId": session_id,
            "cwd": "/tmp/project",
            "mcpServers": []
        }),
    )
    .await
    .expect("load session");

    assert_eq!(loaded["_meta"]["replayedMessages"], json!(2));
    let first = writer_rx.recv().await.expect("first replay");
    let second = writer_rx.recv().await.expect("second replay");
    assert_eq!(first["method"], json!("session/update"));
    assert_eq!(
        first["params"]["update"]["sessionUpdate"],
        json!("user_message_chunk")
    );
    assert_eq!(
        first["params"]["update"]["content"]["text"],
        json!("first prompt")
    );
    assert_eq!(
        second["params"]["update"]["sessionUpdate"],
        json!("agent_message_chunk")
    );
    assert_eq!(
        second["params"]["update"]["content"]["text"],
        json!("first answer")
    );
    let first_params: wire::SessionUpdateParams =
        serde_json::from_value(first["params"].clone()).expect("typed first replay");
    let second_params: wire::SessionUpdateParams =
        serde_json::from_value(second["params"].clone()).expect("typed second replay");
    assert!(matches!(
        first_params.update,
        wire::SessionUpdate::UserMessageChunk { .. }
    ));
    assert!(matches!(
        second_params.update,
        wire::SessionUpdate::AgentMessageChunk { .. }
    ));

    unregister_client_bridge(&session_id).await;
}

#[test]
fn session_metadata_carries_cwd_to_tools() {
    let metadata = acp_metadata_with_cwd("sess_test", "/tmp/project");

    assert_eq!(metadata["acp_session_id"], json!("sess_test"));
    assert_eq!(metadata["acp_cwd"], json!("/tmp/project"));
    assert_eq!(metadata["tool_base_dir"], json!("/tmp/project"));
    assert_eq!(metadata["tool_working_dir"], json!("/tmp/project"));
}

#[tokio::test]
async fn client_request_waiter_round_trips_result() {
    let state = Arc::new(AcpConnectionState::default());
    let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
    let request_state = Arc::clone(&state);

    let waiter = tokio::spawn(async move {
        request_state
            .send_client_request(
                &writer_tx,
                "fs/read_text_file",
                json!({ "path": "/tmp/a.rs" }),
                Duration::from_secs(1),
            )
            .await
            .expect("client response")
    });

    let outbound = writer_rx.recv().await.expect("outbound request");
    let request_id = json_rpc_id_key(&outbound["id"]);
    let tx = state
        .take_pending_client_request(&request_id)
        .await
        .expect("pending request");
    tx.send(AcpClientResponse::Result(json!({ "content": "ok" })))
        .expect("deliver response");

    assert_eq!(waiter.await.expect("join")["content"], json!("ok"));
}

#[tokio::test]
async fn client_request_waiter_times_out_and_cleans_state() {
    let state = Arc::new(AcpConnectionState::default());
    let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
    let err = state
        .send_client_request(
            &writer_tx,
            "fs/read_text_file",
            json!({ "path": "/tmp/a.rs" }),
            Duration::from_millis(5),
        )
        .await
        .expect_err("missing client response should time out");

    assert_eq!(err.code, -32000);
    assert!(err.message.contains("timed out"));
    let outbound = writer_rx.recv().await.expect("outbound request");
    let request_id = json_rpc_id_key(&outbound["id"]);
    assert!(
        state
            .take_pending_client_request(&request_id)
            .await
            .is_none(),
        "timeout should clear pending request waiter"
    );
}

#[tokio::test]
async fn active_prompt_waiter_rejects_second_turn_for_same_session() {
    let state = Arc::new(AcpConnectionState::default());
    let _first = state
        .start_prompt_waiter("sess_test")
        .await
        .expect("first prompt waiter");
    let err = state
        .start_prompt_waiter("sess_test")
        .await
        .expect_err("second active prompt waiter should fail");
    assert_eq!(err.code, -32000);
    assert!(err.message.contains("active prompt turn"));
}

async fn reply_to_next_client_request(
    state: &AcpSharedState,
    writer_rx: &mut mpsc::UnboundedReceiver<Value>,
    expected_method: &str,
    result: Value,
) -> Value {
    let outbound = writer_rx.recv().await.expect("outbound client request");
    assert_eq!(outbound["jsonrpc"], json!("2.0"));
    assert_eq!(outbound["method"], json!(expected_method));
    let request_id = json_rpc_id_key(&outbound["id"]);
    let tx = state
        .take_pending_client_request(&request_id)
        .await
        .expect("pending client request");
    tx.send(AcpClientResponse::Result(result))
        .expect("deliver client response");
    outbound
}

async fn reply_to_next_client_request_error(
    state: &AcpSharedState,
    writer_rx: &mut mpsc::UnboundedReceiver<Value>,
    expected_method: &str,
    error: JsonRpcErrorValue,
) -> Value {
    let outbound = writer_rx.recv().await.expect("outbound client request");
    assert_eq!(outbound["jsonrpc"], json!("2.0"));
    assert_eq!(outbound["method"], json!(expected_method));
    let request_id = json_rpc_id_key(&outbound["id"]);
    let tx = state
        .take_pending_client_request(&request_id)
        .await
        .expect("pending client request");
    tx.send(AcpClientResponse::Error(error))
        .expect("deliver client error");
    outbound
}

#[tokio::test]
async fn client_fs_bridge_correlates_read_and_write_requests() {
    let state = Arc::new(AcpConnectionState::default());
    let _ = handle_initialize(
        &state,
        &json!({
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": { "readTextFile": true, "writeTextFile": true }
            }
        }),
    )
    .await
    .expect("initialize");
    let session_id = Uuid::new_v4().to_string();
    let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
    register_client_bridge(&session_id, &writer_tx, &state).await;

    let read_session_id = session_id.clone();
    let read = tokio::spawn(async move {
        client_read_text_file(&read_session_id, "/tmp/a.rs", Some(2), Some(5))
            .await
            .expect("read file")
    });
    let read_request = reply_to_next_client_request(
        &state,
        &mut writer_rx,
        "fs/read_text_file",
        json!({ "content": "hello" }),
    )
    .await;
    let read_params: wire::ReadTextFileRequest =
        serde_json::from_value(read_request["params"].clone()).expect("read params");
    assert_eq!(read_params.session_id, session_id);
    assert_eq!(read_params.path, "/tmp/a.rs");
    assert_eq!(read_params.line, Some(2));
    assert_eq!(read_params.limit, Some(5));
    assert_eq!(read.await.expect("read join"), Some("hello".to_string()));

    let write_session_id = session_id.clone();
    let write = tokio::spawn(async move {
        client_write_text_file(&write_session_id, "/tmp/a.rs", "new text")
            .await
            .expect("write file")
    });
    let write_request =
        reply_to_next_client_request(&state, &mut writer_rx, "fs/write_text_file", json!({})).await;
    let write_params: wire::WriteTextFileRequest =
        serde_json::from_value(write_request["params"].clone()).expect("write params");
    assert_eq!(write_params.path, "/tmp/a.rs");
    assert_eq!(write_params.content, "new text");
    assert_eq!(write.await.expect("write join"), Some(()));

    unregister_client_bridge(&session_id).await;
}

#[tokio::test]
async fn client_terminal_bridge_runs_create_wait_output_release_sequence() {
    let state = Arc::new(AcpConnectionState::default());
    let _ = handle_initialize(
        &state,
        &json!({
            "protocolVersion": 1,
            "clientCapabilities": { "terminal": true }
        }),
    )
    .await
    .expect("initialize");
    let session_id = Uuid::new_v4().to_string();
    let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
    register_client_bridge(&session_id, &writer_tx, &state).await;

    let mut env = HashMap::new();
    env.insert("A".to_string(), "B".to_string());
    let terminal_session_id = session_id.clone();
    let execution = tokio::spawn(async move {
        client_execute_terminal(
            &terminal_session_id,
            "echo ok",
            Some("/tmp"),
            Duration::from_secs(1),
            &env,
        )
        .await
        .expect("terminal execution")
    });

    let create_request = reply_to_next_client_request(
        &state,
        &mut writer_rx,
        "terminal/create",
        json!({ "terminalId": "term_1" }),
    )
    .await;
    let create_params: wire::TerminalCreateRequest =
        serde_json::from_value(create_request["params"].clone()).expect("create params");
    assert_eq!(create_params.session_id, session_id);
    assert_eq!(create_params.command, "sh");
    assert_eq!(create_params.args, vec!["-lc", "echo ok"]);
    assert_eq!(create_params.cwd.as_deref(), Some("/tmp"));
    assert_eq!(create_params.env[0].name, "A");

    let wait_request = reply_to_next_client_request(
        &state,
        &mut writer_rx,
        "terminal/wait_for_exit",
        json!({ "exitCode": 0 }),
    )
    .await;
    let wait_params: wire::TerminalIdRequest =
        serde_json::from_value(wait_request["params"].clone()).expect("wait params");
    assert_eq!(wait_params.terminal_id, "term_1");

    let output_request = reply_to_next_client_request(
        &state,
        &mut writer_rx,
        "terminal/output",
        json!({ "output": "ok\n", "truncated": false }),
    )
    .await;
    let output_params: wire::TerminalIdRequest =
        serde_json::from_value(output_request["params"].clone()).expect("output params");
    assert_eq!(output_params.terminal_id, "term_1");

    let release_request =
        reply_to_next_client_request(&state, &mut writer_rx, "terminal/release", json!({})).await;
    let release_params: wire::TerminalIdRequest =
        serde_json::from_value(release_request["params"].clone()).expect("release params");
    assert_eq!(release_params.terminal_id, "term_1");

    let execution = execution
        .await
        .expect("terminal join")
        .expect("terminal should use client bridge");
    assert_eq!(execution.terminal_id, "term_1");
    assert_eq!(execution.output, "ok\n");
    assert_eq!(execution.exit_code, Some(0));
    assert_eq!(execution.signal, None);
    assert!(!execution.truncated);

    unregister_client_bridge(&session_id).await;
}

#[tokio::test]
async fn client_terminal_wait_error_returns_error_without_output_or_kill() {
    let state = Arc::new(AcpConnectionState::default());
    let _ = handle_initialize(
        &state,
        &json!({
            "protocolVersion": 1,
            "clientCapabilities": { "terminal": true }
        }),
    )
    .await
    .expect("initialize");
    let session_id = Uuid::new_v4().to_string();
    let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
    register_client_bridge(&session_id, &writer_tx, &state).await;

    let terminal_session_id = session_id.clone();
    let execution = tokio::spawn(async move {
        let env = HashMap::new();
        client_execute_terminal(
            &terminal_session_id,
            "echo ok",
            Some("/tmp"),
            Duration::from_secs(1),
            &env,
        )
        .await
    });

    reply_to_next_client_request(
        &state,
        &mut writer_rx,
        "terminal/create",
        json!({ "terminalId": "term_1" }),
    )
    .await;
    reply_to_next_client_request_error(
        &state,
        &mut writer_rx,
        "terminal/wait_for_exit",
        JsonRpcErrorValue {
            code: -32010,
            message: "terminal failed".to_string(),
            data: Some(json!({ "terminalId": "term_1" })),
        },
    )
    .await;
    let release_request =
        reply_to_next_client_request(&state, &mut writer_rx, "terminal/release", json!({})).await;
    let release_params: wire::TerminalIdRequest =
        serde_json::from_value(release_request["params"].clone()).expect("release params");
    assert_eq!(release_params.terminal_id, "term_1");

    let err = execution
        .await
        .expect("terminal join")
        .expect_err("wait client error should fail terminal bridge");
    assert!(err.contains("terminal failed"));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), writer_rx.recv())
            .await
            .is_err(),
        "terminal wait client errors should not request output or kill"
    );

    unregister_client_bridge(&session_id).await;
}

#[test]
fn acp_mcp_stdio_descriptor_becomes_scoped_config() {
    let config = acp_mcp_server_config(
        "12345678-1234-1234-1234-123456789abc",
        0,
        &json!({
            "type": "stdio",
            "name": "Local Tools",
            "command": "node",
            "args": ["server.js"],
            "env": { "A": "B" }
        }),
    )
    .expect("config");

    assert_eq!(config.name, "acp-12345678-1-local-tools");
    assert_eq!(config.command.as_deref(), Some("node"));
    assert_eq!(config.args, vec!["server.js"]);
    assert_eq!(config.env.get("A").map(String::as_str), Some("B"));
}

#[tokio::test]
async fn tool_call_ids_are_unique_and_correlated() {
    let state = Arc::new(AcpConnectionState::default());
    state
        .upsert_session(AcpSessionState::new(
            "sess_test".to_string(),
            "/tmp".to_string(),
            Vec::new(),
        ))
        .await;

    let first = status_to_acp_messages(
        &state,
        "sess_test",
        StatusUpdate::ToolStarted {
            name: "shell".to_string(),
            parameters: None,
        },
    )
    .await;
    let second = status_to_acp_messages(
        &state,
        "sess_test",
        StatusUpdate::ToolStarted {
            name: "shell".to_string(),
            parameters: None,
        },
    )
    .await;
    let first_id = first[0]["params"]["update"]["toolCallId"]
        .as_str()
        .unwrap()
        .to_string();
    let second_id = second[0]["params"]["update"]["toolCallId"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(first_id, second_id);

    let completion = status_to_acp_messages(
        &state,
        "sess_test",
        StatusUpdate::ToolCompleted {
            name: "shell".to_string(),
            success: true,
            result_preview: Some("ok".to_string()),
        },
    )
    .await;
    assert_eq!(
        completion[0]["params"]["update"]["toolCallId"],
        json!(first_id)
    );
}

#[tokio::test]
async fn approval_needed_emits_permission_request() {
    let state = Arc::new(AcpConnectionState::default());
    state
        .upsert_session(AcpSessionState::new(
            "sess_test".to_string(),
            "/tmp".to_string(),
            Vec::new(),
        ))
        .await;
    let messages = status_to_acp_messages(
        &state,
        "sess_test",
        StatusUpdate::ApprovalNeeded {
            request_id: Uuid::new_v4().to_string(),
            tool_name: "shell".to_string(),
            description: "run command".to_string(),
            parameters: json!({ "command": "cargo test" }),
        },
    )
    .await;

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["method"], json!("session/update"));
    assert_eq!(messages[1]["method"], json!("session/request_permission"));
    assert_eq!(
        messages[1]["params"]["options"][0]["optionId"],
        json!("allow-once")
    );
}

#[test]
fn compat_transcript_fragments_are_valid_json_rpc() {
    for raw in [
        compat::INITIALIZE_REQUEST,
        compat::SESSION_NEW_REQUEST,
        compat::TEXT_PROMPT_REQUEST,
        compat::EMBEDDED_RESOURCE_PROMPT_REQUEST,
        compat::RESOURCE_LINK_PROMPT_REQUEST,
    ] {
        let message: JsonRpcMessage =
            serde_json::from_str(raw).expect("compat fixture should parse");
        assert_eq!(message.jsonrpc.as_deref(), Some("2.0"));
        assert!(message.method.is_some());
    }
}

#[test]
fn compat_prompt_transcripts_use_typed_content_blocks() {
    for raw in [
        compat::TEXT_PROMPT_REQUEST,
        compat::EMBEDDED_RESOURCE_PROMPT_REQUEST,
        compat::RESOURCE_LINK_PROMPT_REQUEST,
    ] {
        let message: JsonRpcMessage =
            serde_json::from_str(raw).expect("compat fixture should parse");
        let prompt = message.params.get("prompt").expect("prompt blocks");
        let text = prompt_to_text_result(prompt).expect("typed prompt content");
        assert!(!text.is_empty());
    }
}

#[test]
fn emitted_session_update_round_trips_through_wire_type_and_ndjson() {
    let message = session_update("sess_test", agent_message_chunk("hello"));
    let line = serde_json::to_string(&message).expect("serialize");
    assert!(
        !line.contains('\n'),
        "stdout messages must be single-line NDJSON"
    );

    let params: wire::SessionUpdateParams =
        serde_json::from_value(message["params"].clone()).expect("typed update params");
    assert_eq!(params.session_id, "sess_test");
    assert!(matches!(
        params.update,
        wire::SessionUpdate::AgentMessageChunk { .. }
    ));
}

#[test]
fn emitted_acp_messages_validate_against_schema_fixtures() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/acp/v1_public_message_schemas.json"
    ))
    .expect("ACP v1 schema fixtures parse");
    let schemas = fixtures["schemas"]
        .as_object()
        .expect("ACP v1 fixtures contain schemas");
    let schema = |name: &str| {
        let mut schema = schemas.get(name).expect("fixture schema exists").clone();
        if let Some(object) = schema.as_object_mut() {
            object.insert("$defs".to_string(), fixtures["$defs"].clone());
        }
        schema
    };

    assert_json_schema_valid(
        &schema("initializeResponse"),
        &success_response(Some(json!(0)), wire::to_value(initialize_response(1))),
    );
    assert_json_schema_valid(
        &schema("newSessionResponse"),
        &success_response(
            Some(json!("new")),
            json!({
                "sessionId": "sess_test",
                "modes": session_modes("ask"),
                "configOptions": session_config_options()
            }),
        ),
    );
    assert_json_schema_valid(
        &schema("listSessionsResponse"),
        &success_response(
            Some(json!("list")),
            json!({
                "sessions": [{
                    "sessionId": "sess_test",
                    "cwd": "/tmp",
                    "title": "Test",
                    "createdAt": "2026-04-25T00:00:00Z",
                    "updatedAt": "2026-04-25T00:00:00Z"
                }],
                "nextCursor": null
            }),
        ),
    );
    assert_json_schema_valid(
        &schema("loadSessionResponse"),
        &success_response(
            Some(json!("load")),
            json!({
                "modes": session_modes("ask"),
                "configOptions": session_config_options(),
                "_meta": { "replayedMessages": 0 }
            }),
        ),
    );
    assert_json_schema_valid(
        &schema("promptResponse"),
        &prompt_response(wire::StopReason::EndTurn),
    );
    assert_json_schema_valid(
        &schema("sessionUpdateNotification"),
        &session_update("sess_test", agent_message_chunk("hello")),
    );
    assert_json_schema_valid(
        &schema("requestPermission"),
        &client_request(
            json!("permission-1"),
            "session/request_permission",
            json!({
                "sessionId": "sess_test",
                "toolCall": { "toolCallId": "tool_1", "title": "shell", "kind": "execute" },
                "options": permission_options()
            }),
        ),
    );
}

#[test]
fn session_info_update_uses_official_flat_fields() {
    let update = session_info_update(
        Some("Implement ACP".to_string()),
        Some("2026-04-24T12:00:00Z".to_string()),
        Some(json!({ "messageCount": 1 })),
    );

    assert_eq!(update["sessionUpdate"], json!("session_info_update"));
    assert_eq!(update["title"], json!("Implement ACP"));
    assert_eq!(update["updatedAt"], json!("2026-04-24T12:00:00Z"));
    assert_eq!(update["_meta"]["messageCount"], json!(1));
    assert_eq!(update["session"], Value::Null);

    let params = wire::SessionUpdateParams {
        session_id: "sess_test".to_string(),
        update: serde_json::from_value(update).expect("session_info_update wire shape"),
    };
    assert!(matches!(
        params.update,
        wire::SessionUpdate::SessionInfoUpdate { .. }
    ));
}

#[test]
fn json_rpc_responses_preserve_id_shapes_and_stay_ndjson() {
    let numeric = success_response(Some(json!(42)), json!({ "ok": true }));
    assert_eq!(numeric["jsonrpc"], json!("2.0"));
    assert_eq!(numeric["id"], json!(42));
    assert_eq!(numeric["result"]["ok"], json!(true));

    let string = error_response(
        Some(json!("client-req-1")),
        -32602,
        "Invalid params".to_string(),
        Some(json!({ "field": "cwd" })),
    );
    assert_eq!(string["id"], json!("client-req-1"));
    assert_eq!(string["error"]["code"], json!(-32602));
    assert_eq!(string["error"]["data"]["field"], json!("cwd"));

    for message in [numeric, string] {
        let line = serde_json::to_string(&message).expect("serialize response");
        assert!(!line.contains('\n'), "ACP stdout must remain NDJSON");
        let reparsed: Value = serde_json::from_str(&line).expect("response is valid JSON");
        assert_eq!(reparsed["jsonrpc"], json!("2.0"));
    }
}

#[test]
fn permission_request_shape_round_trips_through_wire_type() {
    let request = client_request(
        json!("perm-1"),
        "session/request_permission",
        json!({
            "sessionId": "sess_test",
            "toolCall": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call_1",
                "title": "Approval needed: shell",
                "kind": "execute",
                "status": "pending",
                "rawInput": { "command": "cargo test" }
            },
            "options": permission_options()
        }),
    );

    assert_eq!(request["jsonrpc"], json!("2.0"));
    assert_eq!(request["id"], json!("perm-1"));
    assert_eq!(request["method"], json!("session/request_permission"));
    let params: wire::RequestPermissionParams =
        serde_json::from_value(request["params"].clone()).expect("permission params");
    assert_eq!(params.session_id, "sess_test");
    assert_eq!(params.tool_call["toolCallId"], json!("call_1"));
    assert_eq!(params.options.len(), 3);
    assert!(
        params
            .options
            .iter()
            .any(|option| option.option_id == "allow-once")
    );
    assert!(
        params
            .options
            .iter()
            .any(|option| option.option_id == "allow-always")
    );
    assert!(
        params
            .options
            .iter()
            .any(|option| option.option_id == "reject-once")
    );
}

#[test]
fn client_bridge_payloads_match_typed_wire_requests() {
    let read = wire::to_value(wire::ReadTextFileRequest {
        session_id: "sess_test".to_string(),
        path: "/tmp/a.rs".to_string(),
        line: Some(10),
        limit: Some(20),
    });
    let read: wire::ReadTextFileRequest =
        serde_json::from_value(read).expect("read_text_file params");
    assert_eq!(read.session_id, "sess_test");
    assert_eq!(read.path, "/tmp/a.rs");
    assert_eq!(read.line, Some(10));
    assert_eq!(read.limit, Some(20));

    let write = wire::to_value(wire::WriteTextFileRequest {
        session_id: "sess_test".to_string(),
        path: "/tmp/a.rs".to_string(),
        content: "fn main() {}\n".to_string(),
    });
    let write: wire::WriteTextFileRequest =
        serde_json::from_value(write).expect("write_text_file params");
    assert_eq!(write.content, "fn main() {}\n");

    let terminal = wire::to_value(wire::TerminalCreateRequest {
        session_id: "sess_test".to_string(),
        command: "sh".to_string(),
        args: vec!["-lc".to_string(), "echo ok".to_string()],
        cwd: Some("/tmp".to_string()),
        env: vec![wire::TerminalEnvVar {
            name: "A".to_string(),
            value: "B".to_string(),
        }],
        output_byte_limit: ACP_TERMINAL_OUTPUT_LIMIT,
    });
    let terminal: wire::TerminalCreateRequest =
        serde_json::from_value(terminal).expect("terminal/create params");
    assert_eq!(terminal.command, "sh");
    assert_eq!(terminal.args, vec!["-lc", "echo ok"]);
    assert_eq!(terminal.cwd.as_deref(), Some("/tmp"));
    assert_eq!(terminal.env[0].name, "A");
    assert_eq!(terminal.output_byte_limit, ACP_TERMINAL_OUTPUT_LIMIT);

    let terminal_id = wire::to_value(wire::TerminalIdRequest {
        session_id: "sess_test".to_string(),
        terminal_id: "term_1".to_string(),
    });
    let terminal_id: wire::TerminalIdRequest =
        serde_json::from_value(terminal_id).expect("terminal id params");
    assert_eq!(terminal_id.terminal_id, "term_1");
}

#[tokio::test]
async fn status_updates_round_trip_through_typed_session_update_variants() {
    let state = Arc::new(AcpConnectionState::default());
    let session_id = "sess_test";
    state
        .upsert_session(AcpSessionState::new(
            session_id.to_string(),
            "/tmp".to_string(),
            Vec::new(),
        ))
        .await;

    let cases = vec![
        (
            StatusUpdate::Thinking("thinking".to_string()),
            "agent_thought_chunk",
        ),
        (
            StatusUpdate::Status("running".to_string()),
            "tool_call_update",
        ),
        (
            StatusUpdate::Plan {
                entries: vec![json!({ "content": "Inspect files", "status": "pending" })],
            },
            "plan",
        ),
        (
            StatusUpdate::Usage {
                input_tokens: 3,
                output_tokens: 5,
                cost_usd: Some(0.0001),
                model: Some("test-model".to_string()),
            },
            "usage_update",
        ),
        (
            StatusUpdate::StreamChunk("chunk".to_string()),
            "agent_message_chunk",
        ),
        (
            StatusUpdate::ToolStarted {
                name: "shell".to_string(),
                parameters: Some(json!({ "command": "true" })),
            },
            "tool_call",
        ),
        (
            StatusUpdate::ToolResult {
                name: "shell".to_string(),
                preview: "stdout".to_string(),
                artifacts: Vec::new(),
            },
            "tool_call_update",
        ),
        (
            StatusUpdate::ToolCompleted {
                name: "shell".to_string(),
                success: true,
                result_preview: Some("done".to_string()),
            },
            "tool_call_update",
        ),
        (
            StatusUpdate::AgentMessage {
                content: "persistent".to_string(),
                message_type: "info".to_string(),
            },
            "agent_message_chunk",
        ),
        (
            StatusUpdate::Error {
                message: "failed".to_string(),
                code: Some("llm".to_string()),
            },
            "agent_message_chunk",
        ),
        (
            StatusUpdate::SubagentSpawned {
                agent_id: "sub_1".to_string(),
                name: "researcher".to_string(),
                task: "look".to_string(),
                task_packet: crate::agent::subagent_executor::SubagentTaskPacket::default(),
                allowed_tools: Vec::new(),
                allowed_skills: Vec::new(),
                memory_mode: "provided_context_only".to_string(),
                tool_mode: "explicit_only".to_string(),
                skill_mode: "explicit_only".to_string(),
            },
            "tool_call",
        ),
        (
            StatusUpdate::SubagentProgress {
                agent_id: "sub_1".to_string(),
                message: "working".to_string(),
                category: "thinking".to_string(),
            },
            "tool_call_update",
        ),
        (
            StatusUpdate::SubagentCompleted {
                agent_id: "sub_1".to_string(),
                name: "researcher".to_string(),
                success: true,
                response: "done".to_string(),
                duration_ms: 12,
                iterations: 1,
                task_packet: crate::agent::subagent_executor::SubagentTaskPacket::default(),
                allowed_tools: Vec::new(),
                allowed_skills: Vec::new(),
                memory_mode: "provided_context_only".to_string(),
                tool_mode: "explicit_only".to_string(),
                skill_mode: "explicit_only".to_string(),
            },
            "tool_call_update",
        ),
    ];

    for (status, expected_update) in cases {
        let messages = status_to_acp_messages(&state, session_id, status).await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["method"], json!("session/update"));
        assert_eq!(
            messages[0]["params"]["update"]["sessionUpdate"],
            json!(expected_update)
        );
        let params: wire::SessionUpdateParams =
            serde_json::from_value(messages[0]["params"].clone())
                .expect("status update should match typed wire shape");
        assert_eq!(params.session_id, session_id);
    }

    let approval = status_to_acp_messages(
        &state,
        session_id,
        StatusUpdate::ApprovalNeeded {
            request_id: Uuid::new_v4().to_string(),
            tool_name: "shell".to_string(),
            description: "approve shell".to_string(),
            parameters: json!({ "command": "true" }),
        },
    )
    .await;
    assert_eq!(approval.len(), 2);
    let update_params: wire::SessionUpdateParams =
        serde_json::from_value(approval[0]["params"].clone()).expect("approval update");
    assert!(matches!(
        update_params.update,
        wire::SessionUpdate::ToolCall { .. }
    ));
    let permission_params: wire::RequestPermissionParams =
        serde_json::from_value(approval[1]["params"].clone()).expect("approval request");
    assert_eq!(permission_params.session_id, session_id);
}

#[test]
fn prompt_response_uses_typed_stop_reason_values() {
    assert_eq!(
        prompt_response(wire::StopReason::Cancelled)["stopReason"],
        json!("cancelled")
    );
    assert_eq!(
        prompt_response(wire::StopReason::MaxTokens)["stopReason"],
        json!("max_tokens")
    );
    assert_eq!(
        wire::StopReason::from_error_text("provider finish_reason: length"),
        Some(wire::StopReason::MaxTokens)
    );
    assert_eq!(
        wire::StopReason::from_error_text("model returned content_filter"),
        Some(wire::StopReason::Refusal)
    );
}

#[test]
fn all_emitted_wire_update_variants_round_trip() {
    let updates = vec![
        wire::SessionUpdate::UserMessageChunk {
            content: wire::ContentBlock::text("user"),
        },
        wire::SessionUpdate::AgentMessageChunk {
            content: wire::ContentBlock::text("agent"),
        },
        wire::SessionUpdate::AgentThoughtChunk {
            content: wire::ContentBlock::text("thought"),
        },
        wire::SessionUpdate::ToolCall {
            tool_call_id: "call_1".to_string(),
            title: "shell".to_string(),
            kind: "execute".to_string(),
            status: "pending".to_string(),
            raw_input: json!({ "command": "true" }),
            meta: Some(json!({ "approvalNeeded": false })),
        },
        wire::SessionUpdate::ToolCallUpdate {
            tool_call_id: "call_1".to_string(),
            status: "completed".to_string(),
            content: Some(vec![wire::ToolContentBlock::text("ok")]),
            meta: None,
        },
        wire::SessionUpdate::CurrentModeUpdate {
            current_mode_id: "ask".to_string(),
        },
        wire::SessionUpdate::ConfigOptionUpdate {
            config_options: json!([]),
        },
        wire::SessionUpdate::SessionInfoUpdate {
            title: Some("ACP".to_string()),
            updated_at: Some("2026-04-24T12:00:00Z".to_string()),
            meta: Some(json!({ "messageCount": 1 })),
        },
        wire::SessionUpdate::Plan {
            entries: vec![json!({ "content": "Run tests", "status": "pending" })],
        },
        wire::SessionUpdate::UsageUpdate {
            usage: json!({ "inputTokens": 1, "outputTokens": 2, "totalTokens": 3 }),
        },
    ];

    for update in updates {
        let message = wire::JsonRpcNotification {
            jsonrpc: "2.0",
            method: "session/update",
            params: wire::SessionUpdateParams {
                session_id: "sess_test".to_string(),
                update,
            },
        };
        let line = serde_json::to_string(&message).expect("serialize update");
        assert!(!line.contains('\n'));
        let value: Value = serde_json::from_str(&line).expect("valid JSON");
        let params: wire::SessionUpdateParams =
            serde_json::from_value(value["params"].clone()).expect("wire update params");
        assert_eq!(params.session_id, "sess_test");
    }
}

#[test]
fn permission_outcome_accepts_editor_response_variants() {
    let nested = permission_outcome_from_result(
        &json!({ "outcome": { "outcome": "selected", "optionId": "allow-once" } }),
    );
    assert_eq!(nested.outcome, "selected");
    assert_eq!(nested.option_id.as_deref(), Some("allow-once"));

    let direct =
        permission_outcome_from_result(&json!({ "outcome": "selected", "optionId": "reject" }));
    assert_eq!(direct.outcome, "selected");
    assert_eq!(direct.option_id.as_deref(), Some("reject"));

    assert_eq!(
        permission_decision_from_outcome(&direct),
        (false, false, false)
    );
    assert_eq!(
        permission_decision_from_outcome(&wire::PermissionOutcome {
            outcome: "selected".to_string(),
            option_id: Some("allow-once".to_string()),
        }),
        (true, false, false)
    );
    assert_eq!(
        permission_decision_from_outcome(&wire::PermissionOutcome {
            outcome: "selected".to_string(),
            option_id: Some("allow-always".to_string()),
        }),
        (true, true, false)
    );
    assert_eq!(
        permission_decision_from_outcome(&wire::PermissionOutcome {
            outcome: "cancelled".to_string(),
            option_id: None,
        }),
        (false, false, true)
    );
}
