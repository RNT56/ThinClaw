#![cfg(feature = "docker-sandbox")]

use std::collections::{HashMap, VecDeque};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::sync::Mutex;
use uuid::Uuid;

use thinclaw::error::LlmError;
use thinclaw::llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role,
    ToolCompletionRequest, ToolCompletionResponse,
};
use thinclaw::orchestrator::api::{OrchestratorApi, OrchestratorState, PendingPrompt};
use thinclaw::orchestrator::{ContainerJobConfig, ContainerJobManager, JobMode, TokenStore};
use thinclaw::platform::resolve_data_dir;
use thinclaw::sandbox_jobs::SandboxJobSpec;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(15);
const WORKER_JOB_TIMEOUT: Duration = Duration::from_secs(90);
const BRIDGE_JOB_TIMEOUT: Duration = Duration::from_secs(240);
const POLL_INTERVAL: Duration = Duration::from_millis(500);

type PromptQueue = Arc<Mutex<HashMap<Uuid, VecDeque<PendingPrompt>>>>;

struct SmokeHarness {
    job_manager: Arc<ContainerJobManager>,
    prompt_queue: PromptQueue,
    orchestrator: tokio::task::JoinHandle<()>,
}

fn reserve_local_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("read local addr")
        .port()
}

struct PromptAwareLlm;

impl PromptAwareLlm {
    fn response_for_messages(messages: &[ChatMessage]) -> String {
        let wrap_up_requested = messages.iter().rev().find_map(|message| {
            if message.role != Role::User {
                return None;
            }
            Some(message.content.to_ascii_lowercase())
        });

        if wrap_up_requested
            .as_deref()
            .is_some_and(|content| content.contains("wrap up"))
        {
            "I have completed the task.".to_string()
        } else {
            "The task is not complete yet.".to_string()
        }
    }
}

#[async_trait]
impl LlmProvider for PromptAwareLlm {
    fn model_name(&self) -> &str {
        "prompt-aware-smoke"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: Self::response_for_messages(&request.messages),
            provider_model: Some("prompt-aware-smoke".to_string()),
            cost_usd: Some(0.0),
            thinking_content: None,
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Ok(ToolCompletionResponse {
            content: Some(Self::response_for_messages(&request.messages)),
            provider_model: Some("prompt-aware-smoke".to_string()),
            cost_usd: Some(0.0),
            tool_calls: Vec::new(),
            thinking_content: None,
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }
}

async fn wait_for_orchestrator_health(port: u16) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("build health client");
    let url = format!("http://127.0.0.1:{port}/health");
    let deadline = Instant::now() + HEALTH_TIMEOUT;

    loop {
        if let Ok(response) = client.get(&url).send().await
            && response.status().is_success()
        {
            return;
        }

        assert!(
            Instant::now() < deadline,
            "orchestrator health endpoint did not become ready within {:?}",
            HEALTH_TIMEOUT
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn wait_for_completion(
    job_manager: &ContainerJobManager,
    job_id: Uuid,
    timeout: Duration,
) -> thinclaw::orchestrator::CompletionResult {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(handle) = job_manager.get_handle(job_id).await
            && let Some(result) = handle.completion_result.clone()
        {
            return result;
        }

        assert!(
            Instant::now() < deadline,
            "sandbox job {job_id} did not complete within {:?}",
            timeout
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn spawn_harness(config: ContainerJobConfig) -> SmokeHarness {
    let port = config.orchestrator_port;
    let token_store = TokenStore::new();
    let prompt_queue: PromptQueue = Arc::new(Mutex::new(HashMap::new()));
    let job_manager = Arc::new(ContainerJobManager::new(config, token_store.clone()));

    let orchestrator = tokio::spawn({
        let prompt_queue = Arc::clone(&prompt_queue);
        let job_manager = Arc::clone(&job_manager);
        async move {
            let _ = OrchestratorApi::start(
                OrchestratorState {
                    llm: Arc::new(PromptAwareLlm),
                    job_manager,
                    token_store,
                    job_event_tx: None,
                    prompt_queue,
                    store: None,
                    secrets_store: None,
                },
                port,
            )
            .await;
        }
    });

    wait_for_orchestrator_health(port).await;

    SmokeHarness {
        job_manager,
        prompt_queue,
        orchestrator,
    }
}

impl SmokeHarness {
    async fn shutdown(self, job_id: Uuid) {
        self.job_manager.cleanup_job(job_id).await;
        self.orchestrator.abort();
        let _ = self.orchestrator.await;
    }
}

fn create_workspace_dir(prefix: &str) -> tempfile::TempDir {
    let projects_dir = resolve_data_dir("projects");
    std::fs::create_dir_all(&projects_dir).expect("create projects dir");
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(&projects_dir)
        .expect("create sandbox workspace dir")
}

fn sandbox_job_spec(
    title: &str,
    description: &str,
    workspace_dir: &Path,
    mode: JobMode,
    interactive: bool,
) -> SandboxJobSpec {
    let mut spec = SandboxJobSpec::new(
        title,
        description,
        "smoke-user",
        "smoke-actor",
        Some(workspace_dir.to_string_lossy().to_string()),
        mode,
    );
    spec.interactive = interactive;
    spec.idle_timeout_secs = 120;
    spec
}

fn host_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn codex_auth_home_dir() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| host_home_dir().map(|home| home.join(".codex")))
        .filter(|path| path.join("auth.json").is_file())
}

fn claude_auth() -> Option<(Option<String>, Option<String>)> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let oauth_token = thinclaw::config::ClaudeCodeConfig::extract_oauth_token()
        .filter(|value| !value.trim().is_empty());
    if api_key.is_some() || oauth_token.is_some() {
        Some((api_key, oauth_token))
    } else {
        None
    }
}

#[tokio::test]
#[ignore = "requires Docker plus a local thinclaw-worker:latest image"]
async fn interactive_worker_container_smoke_completes_after_done_prompt() {
    let harness = spawn_harness(ContainerJobConfig {
        orchestrator_port: reserve_local_port(),
        ..ContainerJobConfig::default()
    })
    .await;

    let job_id = Uuid::new_v4();
    let workspace_dir = create_workspace_dir("docker-worker-smoke-");
    let spec = sandbox_job_spec(
        "Docker smoke job",
        "Work until you receive a wrap up instruction, then finish cleanly.",
        workspace_dir.path(),
        JobMode::Worker,
        true,
    );

    let create_result = harness
        .job_manager
        .create_job(job_id, spec, Vec::new())
        .await;
    assert!(
        create_result.is_ok(),
        "failed to create Docker sandbox job: {create_result:?}"
    );

    tokio::time::sleep(Duration::from_secs(2)).await;

    {
        let mut queue = harness.prompt_queue.lock().await;
        queue.entry(job_id).or_default().push_back(PendingPrompt {
            content: Some("Please wrap up now.".to_string()),
            done: true,
        });
    }

    let result =
        wait_for_completion(harness.job_manager.as_ref(), job_id, WORKER_JOB_TIMEOUT).await;
    assert!(
        result.success,
        "interactive worker smoke job should succeed, got: {result:?}"
    );
    assert_eq!(result.status, "completed");
    assert_eq!(
        result.message.as_deref(),
        Some("I have completed the task."),
        "worker should finish after processing the done prompt"
    );

    harness.shutdown(job_id).await;
}

#[tokio::test]
#[ignore = "requires Docker plus a local thinclaw-worker:latest image and Claude auth"]
async fn claude_code_bridge_container_smoke_completes_one_shot_when_auth_available() {
    let Some((claude_code_api_key, claude_code_oauth_token)) = claude_auth() else {
        eprintln!(
            "skipping Claude bridge smoke: no ANTHROPIC_API_KEY or Claude OAuth token available"
        );
        return;
    };

    let harness = spawn_harness(ContainerJobConfig {
        orchestrator_port: reserve_local_port(),
        claude_code_api_key,
        claude_code_oauth_token,
        claude_code_enabled: true,
        claude_code_max_turns: 4,
        ..ContainerJobConfig::default()
    })
    .await;

    let job_id = Uuid::new_v4();
    let workspace_dir = create_workspace_dir("docker-claude-smoke-");
    let spec = sandbox_job_spec(
        "Claude bridge smoke job",
        "Reply with exactly 'Bridge smoke OK' and then stop. Do not ask follow-up questions.",
        workspace_dir.path(),
        JobMode::ClaudeCode,
        false,
    );

    let create_result = harness
        .job_manager
        .create_job(job_id, spec, Vec::new())
        .await;
    assert!(
        create_result.is_ok(),
        "failed to create Claude bridge sandbox job: {create_result:?}"
    );

    let result =
        wait_for_completion(harness.job_manager.as_ref(), job_id, BRIDGE_JOB_TIMEOUT).await;
    assert!(
        result.success,
        "Claude bridge smoke job should succeed, got: {result:?}"
    );
    assert_eq!(result.status, "completed");
    assert_eq!(
        result.message.as_deref(),
        Some("Claude Code session completed"),
        "Claude bridge should self-terminate after the first successful one-shot session"
    );

    harness.shutdown(job_id).await;
}

#[tokio::test]
#[ignore = "requires Docker plus a local thinclaw-worker:latest image and Codex auth"]
async fn codex_code_bridge_container_smoke_completes_one_shot_when_auth_available() {
    let codex_code_api_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let Some(codex_code_home_dir) = codex_auth_home_dir().or_else(|| {
        if codex_code_api_key.is_some() {
            host_home_dir().map(|home| home.join(".codex"))
        } else {
            None
        }
    }) else {
        eprintln!("skipping Codex bridge smoke: no OPENAI_API_KEY or ~/.codex/auth.json available");
        return;
    };

    let harness = spawn_harness(ContainerJobConfig {
        orchestrator_port: reserve_local_port(),
        codex_code_api_key,
        codex_code_enabled: true,
        codex_code_home_dir,
        ..ContainerJobConfig::default()
    })
    .await;

    let job_id = Uuid::new_v4();
    let workspace_dir = create_workspace_dir("docker-codex-smoke-");
    let spec = sandbox_job_spec(
        "Codex bridge smoke job",
        "Reply with exactly 'Bridge smoke OK' and then stop. Do not ask follow-up questions.",
        workspace_dir.path(),
        JobMode::CodexCode,
        false,
    );

    let create_result = harness
        .job_manager
        .create_job(job_id, spec, Vec::new())
        .await;
    assert!(
        create_result.is_ok(),
        "failed to create Codex bridge sandbox job: {create_result:?}"
    );

    let result =
        wait_for_completion(harness.job_manager.as_ref(), job_id, BRIDGE_JOB_TIMEOUT).await;
    assert!(
        result.success,
        "Codex bridge smoke job should succeed, got: {result:?}"
    );
    assert_eq!(result.status, "completed");
    assert_eq!(
        result.message.as_deref(),
        Some("Codex session completed"),
        "Codex bridge should self-terminate after the first successful one-shot session"
    );

    harness.shutdown(job_id).await;
}
