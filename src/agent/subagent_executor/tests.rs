use super::*;
use crate::config::SafetyConfig;
use crate::testing::StubLlm;

#[test]
fn test_subagent_config_defaults() {
    let config = SubagentConfig::default();
    assert_eq!(config.max_concurrent, 5);
    assert_eq!(config.default_timeout_secs, 300);
    assert!(!config.allow_nested);
    assert_eq!(config.max_tool_iterations, 30);
}

#[test]
fn test_subagent_status_equality() {
    assert_eq!(SubagentStatus::Running, SubagentStatus::Running);
    assert_ne!(SubagentStatus::Running, SubagentStatus::Completed);
    assert_eq!(
        SubagentStatus::Failed("err".to_string()),
        SubagentStatus::Failed("err".to_string())
    );
}

#[test]
fn test_subagent_result_serialization() {
    let result = SubagentResult {
        agent_id: Uuid::new_v4(),
        name: "researcher".to_string(),
        response: "Found 3 papers".to_string(),
        iterations: 5,
        duration_ms: 3200,
        success: true,
        error: None,
    };
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("researcher"));
    assert!(json.contains("Found 3 papers"));

    let deserialized: SubagentResult = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.name, "researcher");
    assert_eq!(deserialized.iterations, 5);
}

#[test]
fn test_spawn_request_with_defaults() {
    let req = SubagentSpawnRequest {
        name: "test".to_string(),
        task: "do something".to_string(),
        system_prompt: None,
        model: None,
        task_packet: None,
        memory_mode: None,
        tool_mode: None,
        skill_mode: None,
        tool_profile: None,
        allowed_tools: None,
        allowed_skills: None,
        principal_id: None,
        actor_id: None,
        agent_workspace_id: None,
        timeout_secs: None,
        wait: true,
    };
    assert_eq!(req.name, "test");
    assert!(req.wait);
    assert!(req.allowed_tools.is_none());
}

#[test]
fn test_spawn_request_serialization() {
    let request = SubagentSpawnRequest {
        name: "researcher".to_string(),
        task: "Find papers about AI".to_string(),
        system_prompt: None,
        model: None,
        task_packet: None,
        memory_mode: None,
        tool_mode: None,
        skill_mode: None,
        tool_profile: None,
        allowed_tools: Some(vec!["http".to_string(), "read_file".to_string()]),
        allowed_skills: None,
        principal_id: None,
        actor_id: None,
        agent_workspace_id: None,
        timeout_secs: Some(120),
        wait: true,
    };
    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("researcher"));
    assert!(json.contains("Find papers about AI"));

    let deserialized: SubagentSpawnRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.name, "researcher");
    assert_eq!(deserialized.allowed_tools.unwrap().len(), 2);
}

#[test]
fn test_spawn_request_defaults() {
    let json = r#"{"name":"test","task":"do work"}"#;
    let request: SubagentSpawnRequest = serde_json::from_str(json).unwrap();
    assert_eq!(request.name, "test");
    assert!(request.system_prompt.is_none());
    assert!(request.model.is_none());
    assert!(request.task_packet.is_none());
    assert!(request.allowed_tools.is_none());
    assert!(request.timeout_secs.is_none());
    assert!(!request.wait);
}

#[test]
fn normalize_strict_inherits_parent_tool_and_skill_ceilings() {
    let mut request = SubagentSpawnRequest {
        name: "researcher".to_string(),
        task: "Inspect the repo".to_string(),
        system_prompt: None,
        model: None,
        task_packet: None,
        memory_mode: None,
        tool_mode: None,
        skill_mode: None,
        tool_profile: None,
        allowed_tools: None,
        allowed_skills: None,
        principal_id: None,
        actor_id: None,
        agent_workspace_id: None,
        timeout_secs: None,
        wait: false,
    };

    request.normalize_strict(
        Some(&["time".to_string(), "json".to_string()]),
        Some(&["github".to_string(), "openai-docs".to_string()]),
        ToolProfile::Restricted,
    );

    assert_eq!(
        request.allowed_tools,
        Some(vec!["json".to_string(), "time".to_string()])
    );
    assert_eq!(
        request.allowed_skills,
        Some(vec!["github".to_string(), "openai-docs".to_string()])
    );
    assert_eq!(request.tool_profile, Some(ToolProfile::Restricted));
}

#[test]
fn normalize_strict_intersects_requested_tools_with_parent_ceiling() {
    let mut request = SubagentSpawnRequest {
        name: "researcher".to_string(),
        task: "Inspect the repo".to_string(),
        system_prompt: None,
        model: None,
        task_packet: None,
        memory_mode: None,
        tool_mode: None,
        skill_mode: None,
        tool_profile: None,
        allowed_tools: Some(vec!["json".to_string(), "shell".to_string()]),
        allowed_skills: Some(vec!["github".to_string(), "skill-creator".to_string()]),
        principal_id: None,
        actor_id: None,
        agent_workspace_id: None,
        timeout_secs: None,
        wait: false,
    };

    request.normalize_strict(
        Some(&["time".to_string(), "json".to_string()]),
        Some(&["github".to_string(), "openai-docs".to_string()]),
        ToolProfile::Restricted,
    );

    assert_eq!(request.allowed_tools, Some(vec!["json".to_string()]));
    assert_eq!(request.allowed_skills, Some(vec!["github".to_string()]));
}

#[test]
fn extract_subagent_message_prefers_message_and_falls_back_to_content() {
    let from_message = serde_json::json!({
        "message": "Checking the docs",
        "content": "older field"
    });
    let from_content = serde_json::json!({
        "content": "Legacy progress payload"
    });

    assert_eq!(
        extract_subagent_message(&from_message).as_deref(),
        Some("Checking the docs")
    );
    assert_eq!(
        extract_subagent_message(&from_content).as_deref(),
        Some("Legacy progress payload")
    );
}

#[test]
fn with_subagent_thread_metadata_inserts_thread_id_for_non_object_metadata() {
    let metadata = serde_json::json!("legacy");
    let merged = with_subagent_thread_metadata(&metadata, "thread-123", "web");

    assert_eq!(merged["thread_id"], serde_json::json!("thread-123"));
}

#[test]
fn with_subagent_thread_metadata_overrides_existing_thread_id() {
    let metadata = serde_json::json!({
        "thread_id": "stale-thread",
        "channel": "web"
    });
    let merged = with_subagent_thread_metadata(&metadata, "thread-fresh", "web");

    assert_eq!(merged["thread_id"], serde_json::json!("thread-fresh"));
    assert_eq!(merged["channel"], serde_json::json!("web"));
}

#[test]
fn normalize_subagent_progress_category_maps_known_message_types() {
    assert_eq!(
        normalize_subagent_progress_category("progress"),
        "milestone"
    );
    assert_eq!(
        normalize_subagent_progress_category("interim_result"),
        "finding"
    );
    assert_eq!(normalize_subagent_progress_category("question"), "question");
    assert_eq!(normalize_subagent_progress_category("warning"), "warning");
    assert_eq!(normalize_subagent_progress_category("tool"), "activity");
    assert_eq!(normalize_subagent_progress_category("other"), "update");
}

#[test]
fn subagent_tool_activity_message_uses_argument_hints() {
    let path_message = subagent_tool_activity_message(
        "read_file",
        &serde_json::json!({ "path": "/tmp/demo.txt" }),
    );
    let query_message = subagent_tool_activity_message(
        "web_search",
        &serde_json::json!({ "query": "Rust async channels" }),
    );

    assert_eq!(path_message, "Running read file on /tmp/demo.txt");
    assert_eq!(query_message, "Running web search for Rust async channels");
}

#[tokio::test]
async fn subagent_heartbeat_emits_progress_and_stops_on_cancel() {
    use crate::channels::Channel;
    use futures::stream;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CaptureChannel {
        progress_count: Arc<AtomicUsize>,
        tx: tokio::sync::mpsc::UnboundedSender<StatusUpdate>,
    }

    #[async_trait::async_trait]
    impl Channel for CaptureChannel {
        fn name(&self) -> &str {
            "capture"
        }

        async fn start(
            &self,
        ) -> Result<crate::channels::MessageStream, crate::error::ChannelError> {
            Ok(Box::pin(stream::empty()))
        }

        async fn respond(
            &self,
            _msg: &crate::channels::IncomingMessage,
            _response: crate::channels::OutgoingResponse,
        ) -> Result<(), crate::error::ChannelError> {
            Ok(())
        }

        async fn send_status(
            &self,
            status: StatusUpdate,
            _metadata: &serde_json::Value,
        ) -> Result<(), crate::error::ChannelError> {
            if matches!(status, StatusUpdate::SubagentProgress { .. }) {
                self.progress_count.fetch_add(1, Ordering::SeqCst);
            }
            let _ = self.tx.send(status);
            Ok(())
        }

        async fn health_check(&self) -> Result<(), crate::error::ChannelError> {
            Ok(())
        }
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let progress_count = Arc::new(AtomicUsize::new(0));
    let channel = CaptureChannel {
        progress_count: Arc::clone(&progress_count),
        tx,
    };

    let channels = Arc::new(ChannelManager::new());
    channels.add(Box::new(channel)).await;

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (activity_tx, activity_rx) = watch::channel(
        Instant::now()
            .checked_sub(SUBAGENT_HEARTBEAT_INTERVAL)
            .unwrap_or_else(Instant::now),
    );

    let heartbeat = SubagentHeartbeat::spawn(
        Arc::clone(&channels),
        "capture".to_string(),
        serde_json::json!({"thread_id": "thread-1"}),
        "agent-1".to_string(),
        "researcher".to_string(),
        activity_tx.clone(),
        activity_rx,
        cancel_tx.clone(),
        cancel_rx,
    );

    let first = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("heartbeat should emit")
        .expect("status channel should remain open");
    assert!(matches!(first, StatusUpdate::SubagentProgress { .. }));
    assert_eq!(progress_count.load(Ordering::SeqCst), 1);

    touch_subagent_activity(&activity_tx);
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(
        rx.try_recv().is_err(),
        "activity reset should suppress immediate re-heartbeat"
    );

    let _ = cancel_tx.send(true);
    drop(heartbeat);

    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(
        rx.try_recv().is_err(),
        "heartbeat task should stop after cancellation"
    );
}

#[tokio::test]
async fn completed_subagent_is_marked_completed_and_not_running() {
    let llm = Arc::new(StubLlm::new("done"));
    let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
        max_output_length: 100_000,
        injection_check_enabled: false,
        redact_pii_in_prompts: true,
        smart_approval_mode: "off".to_string(),
        external_scanner_mode: "off".to_string(),
        external_scanner_path: None,
        external_scanner_require_verified: false,
    }));
    let tools = Arc::new(ToolRegistry::new());
    let channels = Arc::new(ChannelManager::new());
    let (executor, mut result_rx) =
        SubagentExecutor::new(llm, safety, tools, channels, SubagentConfig::default());

    let spawned = executor
        .spawn(
            SubagentSpawnRequest {
                name: "test".to_string(),
                task: "say done".to_string(),
                system_prompt: None,
                model: None,
                task_packet: None,
                memory_mode: None,
                tool_mode: None,
                skill_mode: None,
                tool_profile: None,
                allowed_tools: None,
                allowed_skills: None,
                principal_id: None,
                actor_id: None,
                agent_workspace_id: None,
                timeout_secs: Some(5),
                wait: false,
            },
            "web",
            &serde_json::json!({ "thread_id": "agent:main" }),
            "default",
            None,
            Some("agent:main"),
        )
        .await
        .expect("subagent should spawn");

    let completed = tokio::time::timeout(Duration::from_secs(2), result_rx.recv())
        .await
        .expect("result should arrive")
        .expect("channel should stay open");
    assert_eq!(completed.result.agent_id, spawned.agent_id);
    assert!(completed.result.success);

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if executor.running_count().await == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("running count should drop after completion");

    let info = executor
        .list()
        .await
        .into_iter()
        .find(|entry| entry.id == spawned.agent_id)
        .expect("spawned agent should stay listed");
    assert_eq!(info.status, SubagentStatus::Completed);
}
