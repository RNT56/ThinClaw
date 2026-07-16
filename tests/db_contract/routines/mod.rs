use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hmac::{Hmac, KeyInit, Mac};
use rust_decimal::Decimal;
use sha2::Sha256;
use thinclaw::agent::routine::RunStatus;
use thinclaw::agent::routine_engine::RoutineEngine;
use thinclaw::channels::web::server::{GatewayState, start_server};
use thinclaw::channels::web::sse::SseManager;
use thinclaw::channels::web::ws::WsConnectionTracker;
use thinclaw::channels::{IncomingMessage, OutgoingResponse};
use thinclaw::config::RoutineConfig;
use thinclaw::context::JobContext;
use thinclaw::db::Database;
use thinclaw::error::LlmError;
use thinclaw::identity::{ConversationKind, ResolvedIdentity};
use thinclaw::llm::{
    CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};
use thinclaw::tools::ToolRegistry;
use thinclaw::workspace::Workspace;
use tokio::sync::mpsc;
use uuid::Uuid;

enum TestLlmMode {
    Reply(String),
    Fail(String),
}

struct TestLlm {
    mode: TestLlmMode,
}

impl TestLlm {
    fn reply(text: impl Into<String>) -> Self {
        let text = text.into();
        let content = if serde_json::from_str::<serde_json::Value>(&text)
            .is_ok_and(|value| value.is_object())
        {
            text
        } else {
            serde_json::json!({
                "status": "attention",
                "summary": text,
                "actions": [],
                "artifacts": []
            })
            .to_string()
        };
        Self {
            mode: TestLlmMode::Reply(content),
        }
    }

    fn fail(reason: impl Into<String>) -> Self {
        Self {
            mode: TestLlmMode::Fail(reason.into()),
        }
    }
}

#[async_trait]
impl LlmProvider for TestLlm {
    fn model_name(&self) -> &str {
        "routine-test-llm"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        match &self.mode {
            TestLlmMode::Reply(text) => Ok(CompletionResponse {
                content: text.clone(),
                provider_model: None,
                cost_usd: None,
                thinking_content: None,
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: FinishReason::Stop,
                token_capture: None,
            }),
            TestLlmMode::Fail(reason) => Err(LlmError::RequestFailed {
                provider: self.model_name().to_string(),
                reason: reason.clone(),
            }),
        }
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        match &self.mode {
            TestLlmMode::Reply(text) => Ok(ToolCompletionResponse {
                content: Some(text.clone()),
                provider_model: None,
                cost_usd: None,
                tool_calls: vec![],
                thinking_content: None,
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: FinishReason::Stop,
                token_capture: None,
            }),
            TestLlmMode::Fail(reason) => Err(LlmError::RequestFailed {
                provider: self.model_name().to_string(),
                reason: reason.clone(),
            }),
        }
    }
}

fn routine_test_context(user_id: &str, actor_id: &str) -> JobContext {
    JobContext::with_user_and_actor(
        user_id.to_string(),
        actor_id.to_string(),
        "routine contract test",
        "exercise the routine pipeline",
    )
}

fn owned_event_message(
    channel: &str,
    user_id: &str,
    actor_id: &str,
    content: &str,
) -> IncomingMessage {
    let scope_id = Uuid::new_v4();
    IncomingMessage::new(channel, user_id, content).with_identity(ResolvedIdentity {
        principal_id: user_id.to_string(),
        actor_id: actor_id.to_string(),
        conversation_scope_id: scope_id,
        conversation_kind: ConversationKind::Direct,
        raw_sender_id: actor_id.to_string(),
        stable_external_conversation_key: format!(
            "test://{channel}/{user_id}/{actor_id}/{scope_id}"
        ),
    })
}

fn build_routine_engine(
    db: Arc<dyn Database>,
    user_id: &str,
    llm: Arc<dyn LlmProvider>,
) -> (Arc<RoutineEngine>, mpsc::Receiver<OutgoingResponse>) {
    let workspace = Arc::new(Workspace::new_with_db(user_id, Arc::clone(&db)));
    let (notify_tx, notify_rx) = mpsc::channel(8);
    let engine = Arc::new(RoutineEngine::new(
        RoutineConfig::default(),
        db,
        llm,
        workspace,
        notify_tx,
        None,
    ));
    (engine, notify_rx)
}

fn build_registry(db: Arc<dyn Database>, engine: Arc<RoutineEngine>) -> Arc<ToolRegistry> {
    let registry = Arc::new(ToolRegistry::new());
    registry.register_routine_tools(db, engine);
    registry
}

async fn execute_routine_tool(
    registry: &Arc<ToolRegistry>,
    tool_name: &str,
    params: serde_json::Value,
    ctx: &JobContext,
) -> serde_json::Value {
    let tool = registry
        .get(tool_name)
        .await
        .unwrap_or_else(|| panic!("tool should be registered: {tool_name}"));
    tool.execute(params, ctx)
        .await
        .unwrap_or_else(|err| panic!("tool execution should succeed for {tool_name}: {err}"))
        .result
}

async fn execute_routine_tool_error(
    registry: &Arc<ToolRegistry>,
    tool_name: &str,
    params: serde_json::Value,
    ctx: &JobContext,
) -> String {
    let tool = registry
        .get(tool_name)
        .await
        .unwrap_or_else(|| panic!("tool should be registered: {tool_name}"));
    tool.execute(params, ctx)
        .await
        .expect_err("tool execution should fail")
        .to_string()
}

fn parse_uuid(value: &serde_json::Value, field: &str) -> Uuid {
    Uuid::parse_str(
        value
            .get(field)
            .and_then(|entry| entry.as_str())
            .unwrap_or_else(|| panic!("missing string field: {field}")),
    )
    .unwrap_or_else(|err| panic!("field {field} should be a UUID: {err}"))
}

async fn wait_for_terminal_run(
    db: &Arc<dyn Database>,
    routine_id: Uuid,
) -> thinclaw::agent::routine::RoutineRun {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let runs = db
            .list_routine_runs(routine_id, 10)
            .await
            .expect("list_routine_runs should succeed");
        if let Some(run) = runs.into_iter().next()
            && run.status != RunStatus::Running
        {
            return run;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for routine run {routine_id} to finish"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_unseen_terminal_run(
    db: &Arc<dyn Database>,
    routine_id: Uuid,
    seen_run_ids: &HashSet<Uuid>,
) -> thinclaw::agent::routine::RoutineRun {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let runs = db
            .list_routine_runs(routine_id, 20)
            .await
            .expect("list_routine_runs should succeed");

        for run in runs {
            if seen_run_ids.contains(&run.id) || run.status == RunStatus::Running {
                continue;
            }
            return run;
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for routine {routine_id} to emit a new terminal run"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_notification(rx: &mut mpsc::Receiver<OutgoingResponse>) -> OutgoingResponse {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("notification should arrive before timeout")
        .expect("notification channel should stay open")
}

fn webhook_signature(secret: &str, body: &[u8]) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac secret should be accepted");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

async fn start_routine_gateway_server(
    db: Arc<dyn Database>,
    user_id: &str,
    actor_id: &str,
    engine: Arc<RoutineEngine>,
) -> SocketAddr {
    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: SseManager::new(),
        workspace: None,
        session_manager: None,
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: Some(db),
        job_manager: None,
        prompt_queue: None,
        context_manager: None,
        scheduler: tokio::sync::RwLock::new(None),
        user_id: user_id.to_string(),
        actor_id: actor_id.to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None,
        llm_runtime: None,
        skill_registry: None,
        skill_catalog: None,
        skill_remote_hub: None,
        skill_quarantine: None,
        chat_rate_limiter: thinclaw::channels::web::rate_limiter::RateLimiter::new(30, 60),
        pair_complete_rate_limiter: thinclaw::channels::web::rate_limiter::RateLimiter::new(
            10, 300,
        ),
        device_registry: std::sync::Arc::new(
            thinclaw_gateway::web::devices::DeviceRegistry::load(
                thinclaw_gateway::web::devices::DeviceStore::with_base_dir(std::env::temp_dir()),
            )
            .await
            .expect("load device store for test"),
        ),
        pending_approvals: std::sync::Arc::new(
            thinclaw::channels::web::server::PendingApprovalsStore::in_memory(),
        ),
        registry_entries: Vec::new(),
        cost_guard: None,
        cost_tracker: None,
        metrics_registry: None,
        response_cache: None,
        startup_time: std::time::Instant::now(),
        restart_requested: std::sync::atomic::AtomicBool::new(false),
        routine_engine: Arc::new(std::sync::RwLock::new(Some(engine))),
        repo_project_supervisor: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        secrets_store: None,
        channel_manager: None,
        hooks: None,
    });

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    start_server(
        addr,
        state,
        "routine-test-token".to_string(),
        vec![],
        vec![],
    )
    .await
    .expect("routine webhook server should start")
}

mod crud_runtime;
mod pipeline_events;
mod validation_priority;
