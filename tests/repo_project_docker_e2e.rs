//! Real-Docker end-to-end test for the repo project executor.
//!
//! Unlike the unit/fake-GitHub tests, this dispatches an actual sandbox
//! container through `RepoProjectExecutor` + `ContainerJobManager` and verifies
//! the dispatch → container → worker-run → status-sync loop against a local bare
//! git repository (no GitHub required). It is ignored by default because it
//! needs Docker and a local `thinclaw-worker:latest` image
//! (`docker build -f Dockerfile.worker -t thinclaw-worker .`).
#![cfg(feature = "docker-sandbox")]

use std::collections::{HashMap, VecDeque};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::sync::Mutex;
use uuid::Uuid;

use thinclaw::db::Database;
use thinclaw::db::libsql::LibSqlBackend;
use thinclaw::error::LlmError;
use thinclaw::llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role,
    ToolCompletionRequest, ToolCompletionResponse,
};
use thinclaw::orchestrator::api::{OrchestratorApi, OrchestratorState, PendingPrompt};
use thinclaw::orchestrator::{ContainerJobConfig, ContainerJobManager, TokenStore};
use thinclaw::repo_projects::executor::{RepoProjectExecutor, RepoProjectExecutorConfig};
use thinclaw_repo_projects::{
    CodingBackend, GitHubAuthMode, MergeMethod, ProjectPolicy, RepoProject, RepoProjectRepo,
    RepoProjectState, RepoProjectTask, RepoProjectTaskState, RepoWorkerRunState,
};

const HEALTH_TIMEOUT: Duration = Duration::from_secs(15);
const JOB_TIMEOUT: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(500);

type PromptQueue = Arc<Mutex<HashMap<Uuid, VecDeque<PendingPrompt>>>>;

struct WrapUpLlm;

impl WrapUpLlm {
    fn reply(messages: &[ChatMessage]) -> String {
        let wrap_up = messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.to_ascii_lowercase())
            .is_some_and(|c| c.contains("wrap up"));
        if wrap_up {
            "I have completed the task.".to_string()
        } else {
            "The task is not complete yet.".to_string()
        }
    }
}

#[async_trait]
impl LlmProvider for WrapUpLlm {
    fn model_name(&self) -> &str {
        "wrap-up"
    }
    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: Self::reply(&request.messages),
            provider_model: Some("wrap-up".to_string()),
            cost_usd: Some(0.0),
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }
    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Ok(ToolCompletionResponse {
            content: Some(Self::reply(&request.messages)),
            provider_model: Some("wrap-up".to_string()),
            cost_usd: Some(0.0),
            tool_calls: Vec::new(),
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }
}

fn reserve_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("addr")
        .port()
}

async fn wait_for_health(port: u16) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    let url = format!("http://127.0.0.1:{port}/health");
    let deadline = Instant::now() + HEALTH_TIMEOUT;
    loop {
        if let Ok(r) = client.get(&url).send().await
            && r.status().is_success()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "orchestrator never became healthy"
        );
        tokio::time::sleep(POLL).await;
    }
}

fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

/// Build a bare "origin" repo with a seeded `main` branch and return its path.
fn seed_bare_repo(root: &Path) -> std::path::PathBuf {
    let bare = root.join("remote.git");
    let seed = root.join("seed");
    git(
        root,
        &["init", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    git(
        root,
        &["clone", bare.to_str().unwrap(), seed.to_str().unwrap()],
    );
    git(&seed, &["config", "user.email", "t@example.com"]);
    git(&seed, &["config", "user.name", "Test"]);
    std::fs::write(seed.join("README.md"), b"base\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-m", "base"]);
    git(&seed, &["push", "-u", "origin", "main"]);
    bare
}

#[tokio::test]
#[ignore = "requires Docker plus a local thinclaw-worker:latest image"]
async fn repo_executor_dispatches_real_container_and_syncs_task() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let bare = seed_bare_repo(tmp.path());

    // Orchestrator + container job manager.
    let port = reserve_port();
    let token_store = TokenStore::new();
    let prompt_queue: PromptQueue = Arc::new(Mutex::new(HashMap::new()));
    let job_manager = Arc::new(ContainerJobManager::new(
        ContainerJobConfig {
            orchestrator_port: port,
            ..ContainerJobConfig::default()
        },
        token_store.clone(),
    ));
    let orchestrator = tokio::spawn({
        let prompt_queue = Arc::clone(&prompt_queue);
        let job_manager = Arc::clone(&job_manager);
        async move {
            let _ = OrchestratorApi::start(
                OrchestratorState {
                    llm: Arc::new(WrapUpLlm),
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
    wait_for_health(port).await;

    // DB + seeded project/repo/task pointing at the bare repo as its remote.
    let db_path = tmp.path().join("repo-projects.db");
    let backend = LibSqlBackend::new_local(&db_path).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let db: Arc<dyn Database> = Arc::new(backend);

    let project_id = Uuid::new_v4();
    let repo_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();
    let now = chrono::Utc::now();

    let mut project = RepoProject {
        id: project_id,
        slug: "docker".to_string(),
        name: "Docker E2E".to_string(),
        state: RepoProjectState::Active,
        policy: ProjectPolicy {
            merge_method: MergeMethod::Squash,
            default_coding_backend: CodingBackend::Worker,
            github_auth_mode: GitHubAuthMode::UserToken,
            ..ProjectPolicy::default()
        },
        description: None,
        current_run_id: None,
        created_at: now,
        updated_at: now,
        started_at: Some(now),
        completed_at: None,
    };
    let repo = RepoProjectRepo {
        id: repo_id,
        project_id,
        owner: "owner".to_string(),
        repo: "repo".to_string(),
        github_repo_id: None,
        installation_id: None,
        default_branch: "main".to_string(),
        base_branch: Some("main".to_string()),
        enrolled: true,
        local_path: None,
        auth_mode: GitHubAuthMode::UserToken,
        metadata: serde_json::json!({ "clone_url": bare.to_string_lossy() }),
        created_at: now,
        updated_at: now,
    };
    let mut task = RepoProjectTask {
        id: task_id,
        project_id,
        repo_id,
        title: "Docker dispatch".to_string(),
        body: Some("Work until you receive a wrap up instruction, then finish.".to_string()),
        state: RepoProjectTaskState::Queued,
        coding_backend: CodingBackend::Worker,
        base_branch: "main".to_string(),
        branch_name: "thinclaw/docker/aaaaaaaaaaaa".to_string(),
        head_sha: None,
        pull_request_number: None,
        pull_request_url: None,
        github_issue_number: None,
        assigned_worker_id: None,
        priority: 0,
        labels: vec![],
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
        queued_at: Some(now),
        started_at: None,
        completed_at: None,
    };
    db.create_repo_project(&project).await.unwrap();
    db.upsert_repo_project_repo(&repo).await.unwrap();
    db.upsert_repo_project_task(&task).await.unwrap();

    let executor = RepoProjectExecutor::new(
        Arc::clone(&db),
        Some(Arc::clone(&job_manager)),
        RepoProjectExecutorConfig {
            workspace_base_dir: tmp.path().join("workspace"),
            ..RepoProjectExecutorConfig::default()
        },
    );

    // Dispatch into a real container.
    let result = executor
        .dispatch_task(&mut project, &repo, &mut task)
        .await
        .expect("dispatch should succeed")
        .expect("dispatch should produce a worker run");

    assert_eq!(task.state, RepoProjectTaskState::Running);
    assert!(
        job_manager.get_handle(result.job_id).await.is_some(),
        "a real container must be created for the dispatched job"
    );

    // Nudge the worker to wrap up, then wait for the worker run + task to sync.
    tokio::time::sleep(Duration::from_secs(2)).await;
    prompt_queue
        .lock()
        .await
        .entry(result.job_id)
        .or_default()
        .push_back(PendingPrompt {
            content: Some("Please wrap up now.".to_string()),
            done: true,
        });

    let deadline = Instant::now() + JOB_TIMEOUT;
    loop {
        executor.sync_worker_runs(project_id).await.unwrap();
        let runs = db.list_repo_worker_runs(project_id).await.unwrap();
        if matches!(
            runs.first().map(|r| r.state),
            Some(RepoWorkerRunState::Succeeded | RepoWorkerRunState::Failed)
        ) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "worker run did not reach a terminal state in time"
        );
        tokio::time::sleep(POLL).await;
    }

    let synced = db.get_repo_project_task(task_id).await.unwrap().unwrap();
    assert_ne!(
        synced.state,
        RepoProjectTaskState::Running,
        "the task should leave Running once its sandbox job is terminal"
    );

    job_manager.cleanup_job(result.job_id).await;
    orchestrator.abort();
    let _ = orchestrator.await;
}
