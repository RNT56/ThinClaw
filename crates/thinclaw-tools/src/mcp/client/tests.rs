use super::*;

#[test]
fn test_mcp_request_list_tools() {
    let req = McpRequest::list_tools(1, None);
    assert_eq!(req.method, "tools/list");
    assert_eq!(req.id, 1);
}

#[test]
fn test_mcp_request_call_tool() {
    let req = McpRequest::call_tool(2, "test", serde_json::json!({"key": "value"}));
    assert_eq!(req.method, "tools/call");
    assert!(req.params.is_some());
}

#[test]
fn test_extract_server_name() {
    assert_eq!(
        extract_server_name("https://mcp.notion.com/v1"),
        "mcp_notion_com"
    );
    assert_eq!(extract_server_name("http://localhost:8080"), "localhost");
    assert_eq!(extract_server_name("invalid"), "unknown");
}

#[test]
fn test_registered_tool_prefix() {
    assert_eq!(
        McpClient::registered_tool_prefix("GitHub Copilot"),
        "mcp__github_20copilot__"
    );
}

#[test]
fn test_simple_client_creation() {
    let client = McpClient::new("http://localhost:8080");
    assert_eq!(client.server_url(), "http://localhost:8080");
    assert!(client.session_manager.is_none());
    assert!(client.secrets.is_none());
}

#[tokio::test]
async fn pending_interaction_registration_and_cancellation_are_atomic() {
    let runtime = McpRuntimeState::new("atomic-test", None, None);
    let request = McpRequest::new(42, "sampling/createMessage", None);
    let (interaction, receiver) = runtime
        .build_pending_interaction(&request, McpInteractionKind::Sampling)
        .await
        .expect("interaction should register");

    assert_eq!(runtime.list_pending_interactions().await.len(), 1);
    runtime
        .cancel_pending_server_request(Some(42), "transport closed".to_string())
        .await;
    assert!(runtime.list_pending_interactions().await.is_empty());
    assert!(matches!(
        receiver.await,
        Ok(PendingInteractionResolution::Denied(reason)) if reason == "transport closed"
    ));
    assert!(
        runtime
            .resolve_pending_interaction(
                &interaction.id,
                PendingInteractionResolution::Denied("late".to_string()),
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn duplicate_pending_request_ids_are_rejected_without_overwriting_state() {
    let runtime = McpRuntimeState::new("duplicate-test", None, None);
    let first = McpRequest::new(7, "sampling/createMessage", None);
    let duplicate = McpRequest::new(7, "elicitation/create", None);
    let (interaction, _receiver) = runtime
        .build_pending_interaction(&first, McpInteractionKind::Sampling)
        .await
        .expect("first interaction should register");

    assert!(
        runtime
            .build_pending_interaction(&duplicate, McpInteractionKind::Elicitation)
            .await
            .is_err()
    );
    let pending = runtime.list_pending_interactions().await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, interaction.id);
}

#[tokio::test]
async fn sse_framing_preserves_bounded_json_rpc_messages() {
    let client = McpClient::new("http://localhost:8080");
    let mut data = Vec::new();
    let mut events = 0;
    assert!(
        client
            .process_sse_line(
                br#"data: {"jsonrpc":"2.0","id":9,"result":null}"#,
                &mut data,
                &mut events,
            )
            .await
            .unwrap()
            .is_none()
    );
    let response = client
        .process_sse_line(b"", &mut data, &mut events)
        .await
        .unwrap()
        .expect("blank line should complete the event");
    assert_eq!(response.id, 9);
    assert_eq!(response.result, Some(serde_json::Value::Null));
    assert_eq!(events, 1);
    assert!(data.is_empty());
}

#[test]
fn http_payload_and_sse_line_limits_are_enforced() {
    let request = McpRequest::new(
        1,
        "tools/call",
        Some(serde_json::Value::String(
            "x".repeat(MAX_MCP_HTTP_MESSAGE_BYTES + 1),
        )),
    );
    assert!(matches!(
        serialize_http_payload(&request),
        Err(ToolError::InvalidParameters(_))
    ));

    let mut line = Vec::new();
    assert!(
        append_sse_line_fragment(
            &mut line,
            &vec![0; MAX_MCP_HTTP_MESSAGE_BYTES + b"data:".len() + 2],
        )
        .is_err()
    );
}

#[test]
fn catalog_pagination_rejects_cursor_cycles() {
    let mut pagination = CatalogPagination::default();
    pagination.begin_page("test").unwrap();
    pagination
        .accept_items(0, &[serde_json::json!({"name": "one"})], "test")
        .unwrap();
    assert_eq!(
        pagination
            .accept_cursor(Some("cursor-1".to_string()), "test")
            .unwrap()
            .as_deref(),
        Some("cursor-1")
    );
    pagination.begin_page("test").unwrap();
    assert!(
        pagination
            .accept_cursor(Some("cursor-1".to_string()), "test")
            .is_err()
    );
}

#[tokio::test]
async fn health_check_ok_for_live_stdio_process() {
    // `cat` reads stdin and stays alive, so the stdio transport's reader
    // loop keeps running and health_check reports healthy.
    let config = crate::mcp::config::McpServerConfig::new_stdio("health-live", "cat", vec![]);
    let Ok(client) = McpClient::new_stdio(&config) else {
        // No `cat` on this platform — skip rather than fail spuriously.
        return;
    };
    assert!(client.health_check().await.is_ok());
}

#[tokio::test]
async fn health_check_errs_for_exited_stdio_process() {
    // A command that exits immediately: the reader loop observes EOF and
    // flips `is_running()` to false, which health_check reports as unhealthy.
    let config = crate::mcp::config::McpServerConfig::new_stdio(
        "health-dead",
        "sh",
        vec!["-c".to_string(), "exit 0".to_string()],
    );
    let Ok(client) = McpClient::new_stdio(&config) else {
        return;
    };
    // Give the spawned reader task a moment to observe the child's EOF.
    for _ in 0..50 {
        if client.health_check().await.is_err() {
            return; // observed unhealthy as expected
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("stdio health_check never reported the exited process as unhealthy");
}
