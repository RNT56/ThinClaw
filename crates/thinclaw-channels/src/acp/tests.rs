use super::*;

#[test]
fn sanitizes_mcp_names_for_scoped_servers() {
    assert_eq!(sanitize_mcp_name("My Server/One"), "my-server-one");
    assert_eq!(sanitize_mcp_name("!!!"), "server");
    assert_eq!(sanitize_mcp_name("A.B:C"), "a-b-c");
}

#[test]
fn session_id_accepts_legacy_and_protocol_keys() {
    assert_eq!(
        acp_session_id(&json!({"acp_session_id": "local"})),
        Some("local")
    );
    assert_eq!(acp_session_id(&json!({"sessionId": "wire"})), Some("wire"));
}

#[test]
fn cwd_from_metadata_accepts_nested_and_legacy_absolute_paths() {
    assert_eq!(
        acp_cwd_from_metadata(&json!({"acp": {"cwd": "/workspace"}})),
        Some("/workspace")
    );
    assert_eq!(
        acp_cwd_from_metadata(&json!({"acp_cwd": "/tmp/project"})),
        Some("/tmp/project")
    );
    assert_eq!(
        acp_cwd_from_metadata(&json!({"acp": {"cwd": "relative"}})),
        None
    );
}

#[test]
fn validates_cwd_and_mcp_server_transports() {
    assert!(validate_cwd("/tmp").is_ok());
    assert_eq!(validate_cwd("").unwrap_err(), "cwd is required");
    assert_eq!(
        validate_cwd("relative").unwrap_err(),
        "cwd must be an absolute path"
    );

    assert!(validate_mcp_servers(&[json!({"type": "stdio"})]).is_ok());
    assert!(
        validate_mcp_servers(&[json!({"transport": "sse"})])
            .unwrap_err()
            .contains("not advertised")
    );
    assert!(
        validate_mcp_servers(&[json!({"type": "websocket"})])
            .unwrap_err()
            .contains("Unsupported")
    );
}

#[test]
fn json_rpc_error_helpers_format_and_classify_timeouts() {
    let timeout = json_rpc_error(-32000, "ACP client request 'terminal' timed out", None);
    assert!(is_client_request_timeout(&timeout));
    assert_eq!(
        format_json_rpc_error(json_rpc_error(-32602, "invalid", Some(json!({"x": 1})))),
        "invalid ({\"x\":1})"
    );
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
        permission_decision("selected", Some("allow-once")),
        (true, false, false)
    );
    assert_eq!(
        permission_decision("selected", Some("allow-always")),
        (true, true, false)
    );
    assert_eq!(permission_decision("cancelled", None), (false, false, true));
}

#[test]
fn permission_options_and_metadata_match_acp_tool_profile() {
    let options = permission_options();
    assert_eq!(options.len(), 3);
    assert_eq!(options[0]["optionId"], json!("allow-once"));

    let metadata = acp_metadata_with_cwd("sess_test", "user-1", "/tmp/project");
    assert_eq!(metadata["acp_session_id"], json!("sess_test"));
    assert_eq!(metadata["principal_id"], json!("user-1"));
    assert_eq!(metadata["tool_profile"], json!("acp"));
    assert_eq!(metadata["acp_cwd"], json!("/tmp/project"));
    assert_eq!(metadata["tool_base_dir"], json!("/tmp/project"));
    assert_eq!(metadata["tool_working_dir"], json!("/tmp/project"));
    assert_eq!(
        metadata["session_key"],
        json!(mint_session_key("acp", "session", "sess_test"))
    );
}

#[test]
fn json_rpc_helpers_build_expected_protocol_shapes() {
    assert_eq!(
        session_update("sess", agent_message_chunk("hello")),
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "sess",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {
                        "type": "text",
                        "text": "hello"
                    }
                }
            }
        })
    );
    assert_eq!(
        user_message_chunk("hi"),
        json!({
            "sessionUpdate": "user_message_chunk",
            "content": {
                "type": "text",
                "text": "hi"
            }
        })
    );
    assert_eq!(
        tool_call_update("call-1", "completed", Some("ok")),
        json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call-1",
            "status": "completed",
            "content": [{
                "type": "content",
                "content": {
                    "type": "text",
                    "text": "ok"
                }
            }]
        })
    );
    assert_eq!(
        session_info(
            "sess",
            "/tmp/project",
            Some("Implement ACP"),
            "2026-04-24T12:00:00Z",
            "2026-04-24T12:01:00Z",
            "ask",
            3,
        ),
        json!({
            "sessionId": "sess",
            "cwd": "/tmp/project",
            "title": "Implement ACP",
            "createdAt": "2026-04-24T12:00:00Z",
            "updatedAt": "2026-04-24T12:01:00Z",
            "_meta": {
                "modeId": "ask",
                "messageCount": 3,
                "loadSessionScope": "active_process"
            }
        })
    );
    assert_eq!(
        session_info_update(
            Some("Implement ACP".to_string()),
            Some("2026-04-24T12:01:00Z".to_string()),
            Some(json!({"messageCount": 3})),
        ),
        json!({
            "sessionUpdate": "session_info_update",
            "title": "Implement ACP",
            "updatedAt": "2026-04-24T12:01:00Z",
            "_meta": {"messageCount": 3}
        })
    );
    assert_eq!(
        prompt_response("end_turn"),
        json!({
            "stopReason": "end_turn"
        })
    );
    assert_eq!(
        plan_update(vec![json!({"step":"compile"})]),
        json!({
            "sessionUpdate": "plan",
            "entries": [{"step":"compile"}]
        })
    );
    assert_eq!(
        usage_update(10, 15, Some(0.02), Some("model-x".to_string())),
        json!({
            "sessionUpdate": "usage_update",
            "usage": {
                "inputTokens": 10,
                "outputTokens": 15,
                "totalTokens": 25,
                "costUsd": 0.02,
                "model": "model-x"
            }
        })
    );
    assert_eq!(
        permission_tool_call_update(
            "call-1",
            "Approval needed: shell",
            "execute",
            json!({"command":"cargo test"}),
            "Run tests",
        ),
        json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call-1",
            "title": "Approval needed: shell",
            "kind": "execute",
            "status": "pending",
            "rawInput": {"command":"cargo test"},
            "_meta": {"description": "Run tests"}
        })
    );
    assert_eq!(
        client_request(json!(7), "fs/read_text_file", json!({"path":"README.md"})),
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "fs/read_text_file",
            "params": {"path":"README.md"}
        })
    );
    assert_eq!(
        success_response(Some(json!("req-1")), json!({"ok": true})),
        json!({
            "jsonrpc": "2.0",
            "id": "req-1",
            "result": {"ok": true}
        })
    );
    assert_eq!(
        error_response(None, -32600, "Invalid request", Some(json!({"field":"id"}))),
        json!({
            "jsonrpc": "2.0",
            "error": {
                "code": -32600,
                "message": "Invalid request",
                "data": {"field":"id"}
            }
        })
    );
    assert_eq!(json_rpc_id_key(&json!("abc")), "abc");
    assert_eq!(json_rpc_id_key(&json!(42)), "42");
}

#[test]
fn prompt_to_text_extracts_text_and_resources() {
    let prompt = json!([
        { "type": "text", "text": "Review this" },
        { "type": "resource", "resource": { "uri": "file:///tmp/a.rs", "text": "fn main() {}" } },
        { "type": "resourceLink", "uri": "file:///tmp/b.rs" }
    ]);
    let text = prompt_to_text(&prompt).expect("prompt text");
    assert!(text.contains("Review this"));
    assert!(text.contains("file:///tmp/a.rs"));
    assert!(text.contains("fn main()"));
    assert!(text.contains("file:///tmp/b.rs"));
}

#[test]
fn prompt_to_text_rejects_invalid_typed_content_blocks() {
    let err = prompt_to_text(&json!([{ "type": "resource_link" }]))
        .expect_err("resource links must include a uri");
    assert!(err.message.contains("resource_link"));
}

#[test]
fn prompt_to_text_rejects_unadvertised_media() {
    let err = prompt_to_text(&json!([{ "type": "image", "data": "abc" }]))
        .expect_err("image prompts are not advertised");
    assert!(err.message.contains("not advertised"));
}

#[test]
fn title_collapses_prompt_and_uses_default_for_blank() {
    assert_eq!(title_from_prompt("  hello\nworld  "), "hello world");
    assert_eq!(title_from_prompt(""), "ACP session");
    assert_eq!(title_from_prompt(&"a".repeat(90)).chars().count(), 80);
}

#[test]
fn maps_tool_kinds_for_acp_status() {
    assert_eq!(tool_kind("read_file"), "read");
    assert_eq!(tool_kind("apply_patch"), "edit");
    assert_eq!(tool_kind("shell"), "execute");
    assert_eq!(tool_kind("unknown"), "other");
}

#[test]
fn compat_transcript_fragments_parse_as_json_rpc() {
    for raw in [
        compat::INITIALIZE_REQUEST,
        compat::SESSION_NEW_REQUEST,
        compat::TEXT_PROMPT_REQUEST,
        compat::EMBEDDED_RESOURCE_PROMPT_REQUEST,
        compat::RESOURCE_LINK_PROMPT_REQUEST,
    ] {
        let message: wire::JsonRpcMessage =
            serde_json::from_str(raw).expect("compat fragment parses");
        assert_eq!(message.jsonrpc.as_deref(), Some("2.0"));
        assert!(message.method.is_some());
    }
}

#[tokio::test]
async fn connection_core_tracks_sessions_modes_permissions_and_tool_ids() {
    let core = AcpConnectionCore::default();
    assert!(core.ensure_initialized().await.is_err());

    let response = core
        .initialize(
            wire::InitializeRequest {
                protocol_version: 1,
                client_capabilities: wire::AcpClientCapabilities {
                    fs: wire::AcpFsCapabilities {
                        read_text_file: true,
                        write_text_file: false,
                    },
                    terminal: true,
                    _meta: None,
                },
                client_info: None,
            },
            1,
            "test",
        )
        .await;
    assert_eq!(response.protocol_version, 1);
    assert!(core.ensure_initialized().await.is_ok());
    assert!(core.client_can_read_text_file().await);
    assert!(!core.client_can_write_text_file().await);
    assert!(core.client_can_execute_terminal().await);

    let mut session = AcpSessionState::new("sess_test".to_string(), "/tmp".to_string(), Vec::new());
    session.append_transcript("user", "hello");
    core.upsert_session(session).await;
    assert_eq!(core.sessions_for_list(Some("/tmp")).await.len(), 1);

    core.set_mode("sess_test", "code").await.expect("set mode");
    assert_eq!(core.get_session("sess_test").await.unwrap().mode_id, "code");
    assert!(core.set_mode("sess_test", "unknown").await.is_err());

    let first = core.tool_call_started("sess_test", "shell").await;
    let second = core.tool_call_started("sess_test", "shell").await;
    assert_ne!(first, second);
    assert_eq!(
        core.tool_call_update_id("sess_test", "shell", false).await,
        first
    );
    assert_eq!(
        core.tool_call_update_id("sess_test", "shell", true).await,
        first
    );

    core.insert_pending_permission(PendingPermission {
        client_request_id: "1".to_string(),
        session_id: "sess_test".to_string(),
        approval_request_id: "approval".to_string(),
        tool_call_id: "call".to_string(),
    })
    .await;
    assert!(core.has_pending_permission("sess_test").await);
    assert!(core.take_pending_permission("1").await.is_some());
    assert!(!core.has_pending_permission("sess_test").await);
}

#[test]
fn request_parsers_preserve_method_specific_validation_errors() {
    let err = parse_session_new_params(&json!({"cwd":"relative","mcpServers":[]}))
        .expect_err("relative cwd rejected");
    assert_eq!(err.code, -32602);
    assert_eq!(err.message, "cwd must be an absolute path");

    let err = parse_session_resume_params(
        &json!({"sessionId":"s","cwd":"/tmp","mcpServers":[{"type":"sse"}]}),
    )
    .expect_err("unsupported mcp server rejected");
    assert_eq!(err.code, -32602);
    assert!(err.message.contains("not advertised"));

    let err = parse_session_set_mode_params(&json!({"sessionId": 1})).expect_err("invalid params");
    assert_eq!(err.code, -32602);
    assert!(err.message.contains("Invalid session/set_mode params"));
}

#[test]
fn transcript_replay_projects_only_user_and_assistant_messages() {
    let mut session = AcpSessionState::new("sess_test".to_string(), "/tmp".to_string(), Vec::new());
    session.append_transcript("user", "first");
    session.append_transcript("system", "ignore");
    session.append_transcript("assistant", "second");

    let updates = transcript_replay_updates(&session);
    assert_eq!(updates.len(), 2);
    assert_eq!(
        updates[0]["params"]["update"]["sessionUpdate"],
        json!("user_message_chunk")
    );
    assert_eq!(
        updates[1]["params"]["update"]["sessionUpdate"],
        json!("agent_message_chunk")
    );
}

#[test]
fn status_projection_round_trips_through_typed_session_update_variants() {
    let session_id = "sess_test";
    let cases = vec![
        (
            AcpStatusUpdate::Thinking {
                content: "thinking".to_string(),
            },
            "agent_thought_chunk",
        ),
        (
            AcpStatusUpdate::Status {
                tool_call_id: "status_1".to_string(),
                content: "running".to_string(),
            },
            "tool_call_update",
        ),
        (
            AcpStatusUpdate::Plan {
                entries: vec![json!({ "content": "Inspect files", "status": "pending" })],
            },
            "plan",
        ),
        (
            AcpStatusUpdate::Usage {
                input_tokens: 3,
                output_tokens: 5,
                cost_usd: Some(0.0001),
                model: Some("test-model".to_string()),
            },
            "usage_update",
        ),
        (
            AcpStatusUpdate::StreamChunk {
                content: "chunk".to_string(),
            },
            "agent_message_chunk",
        ),
        (
            AcpStatusUpdate::ToolStarted {
                tool_call_id: "tool_1".to_string(),
                name: "shell".to_string(),
                parameters: Some(json!({ "command": "true" })),
            },
            "tool_call",
        ),
        (
            AcpStatusUpdate::ToolResult {
                tool_call_id: "tool_1".to_string(),
                preview: "stdout".to_string(),
            },
            "tool_call_update",
        ),
        (
            AcpStatusUpdate::ToolCompleted {
                tool_call_id: "tool_1".to_string(),
                success: true,
                result_preview: Some("done".to_string()),
            },
            "tool_call_update",
        ),
        (
            AcpStatusUpdate::AgentMessage {
                content: "persistent".to_string(),
            },
            "agent_message_chunk",
        ),
        (
            AcpStatusUpdate::Error {
                message: "failed".to_string(),
                code: Some("llm".to_string()),
            },
            "agent_message_chunk",
        ),
        (
            AcpStatusUpdate::SubagentSpawned {
                agent_id: "sub_1".to_string(),
                name: "researcher".to_string(),
                task: "look".to_string(),
            },
            "tool_call",
        ),
        (
            AcpStatusUpdate::SubagentProgress {
                agent_id: "sub_1".to_string(),
                message: "working".to_string(),
                category: "thinking".to_string(),
            },
            "tool_call_update",
        ),
        (
            AcpStatusUpdate::SubagentCompleted {
                agent_id: "sub_1".to_string(),
                success: true,
                response: "done".to_string(),
                duration_ms: 12,
                iterations: 1,
            },
            "tool_call_update",
        ),
    ];

    for (status, expected_update) in cases {
        let messages = status_to_acp_messages(session_id, status);
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
}

#[test]
fn approval_status_projection_emits_permission_request() {
    let messages = status_to_acp_messages(
        "sess_test",
        AcpStatusUpdate::ApprovalNeeded {
            client_request_id: json!(7),
            tool_call_id: "tool_approval".to_string(),
            tool_name: "shell".to_string(),
            description: "approve shell".to_string(),
            parameters: json!({ "command": "true" }),
        },
    );

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["method"], json!("session/update"));
    assert_eq!(messages[1]["method"], json!("session/request_permission"));
    let update_params: wire::SessionUpdateParams =
        serde_json::from_value(messages[0]["params"].clone()).expect("approval update");
    assert!(matches!(
        update_params.update,
        wire::SessionUpdate::ToolCall { .. }
    ));
    let permission_params: wire::RequestPermissionParams =
        serde_json::from_value(messages[1]["params"].clone()).expect("approval request");
    assert_eq!(permission_params.session_id, "sess_test");
    assert_eq!(
        permission_params.tool_call["toolCallId"],
        json!("tool_approval")
    );
    assert_eq!(permission_params.options[0].option_id, "allow-once");
}

#[test]
fn descriptor_parser_accepts_stdio_and_rejects_invalid_shapes() {
    let descriptor = acp_mcp_server_descriptor(
        "12345678-0000-0000-0000-000000000000",
        1,
        &json!({
            "transport": "stdio",
            "name": "Local Tools",
            "command": "node",
            "args": ["server.js"],
            "env": {"A": "B"}
        }),
    )
    .expect("stdio descriptor");
    assert_eq!(descriptor.name, "acp-12345678-2-local-tools");
    assert_eq!(descriptor.command, "node");
    assert_eq!(descriptor.args, vec!["server.js"]);
    assert_eq!(descriptor.env.get("A").map(String::as_str), Some("B"));

    assert!(
        acp_mcp_server_descriptor("sess", 0, &json!({"transport":"sse","command":"node"}))
            .unwrap_err()
            .contains("Unsupported")
    );
    assert!(
        acp_mcp_server_descriptor("sess", 0, &json!({"command":""}))
            .unwrap_err()
            .contains("command is required")
    );
    assert!(
        acp_mcp_server_descriptor("sess", 0, &json!({"command":"node","args":[1]}))
            .unwrap_err()
            .contains("args must be strings")
    );
    assert!(
        acp_mcp_server_descriptor("sess", 0, &json!({"command":"node","env":{"A":1}}))
            .unwrap_err()
            .contains("env values must be strings")
    );
}

#[test]
fn json_rpc_line_helpers_preserve_ndjson_boundaries() {
    let message = success_response(Some(json!(7)), json!({"ok": true}));
    let bytes = serialize_json_rpc_line(&message).expect("serialize");
    assert_eq!(bytes.last(), Some(&b'\n'));
    let parsed = parse_json_rpc_line(std::str::from_utf8(&bytes[..bytes.len() - 1]).unwrap())
        .expect("parse line");
    assert_eq!(parsed.id, Some(json!(7)));

    let parse_error = parse_json_rpc_line("{").expect_err("invalid json");
    assert_eq!(parse_error["error"]["code"], json!(-32700));
}
