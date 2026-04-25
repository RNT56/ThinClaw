//! Phase 1 agent environment framework for evals and SFT collection.
//!
//! This module wraps ThinClaw's normal agent loop instead of creating a
//! separate simulator. Every step can therefore reuse canonical trajectory and
//! run-artifact logging while exposing a small environment API for research
//! campaigns and local benchmarks.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::{Json, Router, extract::State, routing::post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::{Agent, AgentRunArtifact, AgentRunArtifactLogger, AgentRunStatus};
use crate::channels::IncomingMessage;
use crate::llm::TokenCaptureSupport;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvState {
    pub episode_id: String,
    pub observations: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentAction {
    UserMessage { content: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub state: EnvState,
    pub response: Option<String>,
    pub reward: f64,
    pub done: bool,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryStep {
    pub action: AgentAction,
    pub response: Option<String>,
    pub reward: f64,
    pub done: bool,
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_capture: Option<TokenTrajectoryCapture>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenTrajectoryCapture {
    #[serde(default)]
    pub exact_tokens_supported: bool,
    #[serde(default)]
    pub logprobs_supported: bool,
    #[serde(default)]
    pub token_ids: Vec<u32>,
    #[serde(default)]
    pub tokens: Vec<String>,
    #[serde(default)]
    pub logprobs: Vec<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    pub env_name: String,
    pub episode_id: String,
    pub score: f64,
    pub steps: Vec<TrajectoryStep>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait AgentEnv: Send + Sync {
    fn name(&self) -> &str;
    async fn reset(&mut self) -> EnvState;
    async fn step(&mut self, action: AgentAction) -> anyhow::Result<StepResult>;
    fn score(&self) -> f64;
    fn is_terminal(&self) -> bool;
    async fn export_trajectory(&self) -> Trajectory;
}

pub struct AgentLoopEnv {
    name: String,
    agent: Arc<Agent>,
    episode_id: String,
    session_key: String,
    user_id: String,
    max_steps: usize,
    observations: Vec<String>,
    steps: Vec<TrajectoryStep>,
    terminal: bool,
}

impl AgentLoopEnv {
    pub fn new(agent: Arc<Agent>) -> Self {
        Self::named("agent_loop", agent)
    }

    pub fn named(name: impl Into<String>, agent: Arc<Agent>) -> Self {
        let episode_id = Uuid::new_v4().to_string();
        Self {
            name: name.into(),
            agent,
            session_key: format!("agent-env:{episode_id}"),
            episode_id,
            user_id: "agent-env".to_string(),
            max_steps: 8,
            observations: Vec::new(),
            steps: Vec::new(),
            terminal: false,
        }
    }

    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = user_id.into();
        self
    }

    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps.max(1);
        self
    }

    fn state(&self) -> EnvState {
        EnvState {
            episode_id: self.episode_id.clone(),
            observations: self.observations.clone(),
            metadata: serde_json::json!({
                "session_key": self.session_key,
                "steps": self.steps.len(),
                "max_steps": self.max_steps,
            }),
        }
    }
}

#[async_trait]
impl AgentEnv for AgentLoopEnv {
    fn name(&self) -> &str {
        &self.name
    }

    async fn reset(&mut self) -> EnvState {
        self.episode_id = Uuid::new_v4().to_string();
        self.session_key = format!("agent-env:{}", self.episode_id);
        self.observations.clear();
        self.steps.clear();
        self.terminal = false;
        self.state()
    }

    async fn step(&mut self, action: AgentAction) -> anyhow::Result<StepResult> {
        if self.terminal {
            return Ok(StepResult {
                state: self.state(),
                response: None,
                reward: 0.0,
                done: true,
                metadata: serde_json::json!({ "terminal": true }),
            });
        }

        let AgentAction::UserMessage { content } = &action;
        let message = IncomingMessage::new("agent_env", self.user_id.clone(), content.clone())
            .with_thread(self.session_key.clone())
            .with_metadata(serde_json::json!({
                "tool_profile": "restricted",
                "agent_env": true,
                "episode_id": self.episode_id,
            }));
        let response = self.agent.handle_message_external(&message).await?;
        let reward = heuristic_reward(response.as_deref());
        self.terminal = self.steps.len() + 1 >= self.max_steps;
        if let Some(ref text) = response {
            self.observations.push(text.clone());
        }

        let step = TrajectoryStep {
            action,
            response: response.clone(),
            reward,
            done: self.terminal,
            at: Utc::now(),
            token_capture: Some(agent_token_capture(&self.agent)),
            metadata: serde_json::json!({}),
        };
        self.steps.push(step);

        Ok(StepResult {
            state: self.state(),
            response,
            reward,
            done: self.terminal,
            metadata: serde_json::json!({}),
        })
    }

    fn score(&self) -> f64 {
        if self.steps.is_empty() {
            return 0.0;
        }
        self.steps.iter().map(|step| step.reward).sum::<f64>() / self.steps.len() as f64
    }

    fn is_terminal(&self) -> bool {
        self.terminal
    }

    async fn export_trajectory(&self) -> Trajectory {
        Trajectory {
            env_name: self.name.clone(),
            episode_id: self.episode_id.clone(),
            score: self.score(),
            steps: self.steps.clone(),
            metadata: serde_json::json!({
                "session_key": self.session_key,
                "phase": "eval_sft_phase_1",
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalBenchCase {
    pub name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub expected_stdout_contains: Vec<String>,
    #[serde(default)]
    pub expected_exit_code: Option<i32>,
    #[serde(default = "default_terminal_bench_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_terminal_bench_timeout_secs() -> u64 {
    30
}

pub struct TerminalBenchEnv {
    name: String,
    episode_id: String,
    cases: Vec<TerminalBenchCase>,
    cursor: usize,
    steps: Vec<TrajectoryStep>,
    observations: Vec<String>,
    terminal: bool,
}

impl TerminalBenchEnv {
    pub fn new(cases: Vec<TerminalBenchCase>) -> Self {
        Self {
            name: "terminal_bench".to_string(),
            episode_id: Uuid::new_v4().to_string(),
            cases,
            cursor: 0,
            steps: Vec::new(),
            observations: Vec::new(),
            terminal: false,
        }
    }

    fn state(&self) -> EnvState {
        EnvState {
            episode_id: self.episode_id.clone(),
            observations: self.observations.clone(),
            metadata: serde_json::json!({
                "benchmark": "terminal_bench",
                "case_index": self.cursor,
                "case_count": self.cases.len(),
            }),
        }
    }
}

#[async_trait]
impl AgentEnv for TerminalBenchEnv {
    fn name(&self) -> &str {
        &self.name
    }

    async fn reset(&mut self) -> EnvState {
        self.episode_id = Uuid::new_v4().to_string();
        self.cursor = 0;
        self.steps.clear();
        self.observations.clear();
        self.terminal = self.cases.is_empty();
        self.state()
    }

    async fn step(&mut self, action: AgentAction) -> anyhow::Result<StepResult> {
        if self.terminal {
            return Ok(StepResult {
                state: self.state(),
                response: None,
                reward: 0.0,
                done: true,
                metadata: serde_json::json!({ "terminal": true }),
            });
        }
        let Some(case) = self.cases.get(self.cursor).cloned() else {
            self.terminal = true;
            return Ok(StepResult {
                state: self.state(),
                response: None,
                reward: 0.0,
                done: true,
                metadata: serde_json::json!({ "terminal": true }),
            });
        };

        let output = tokio::time::timeout(Duration::from_secs(case.timeout_secs), async {
            let mut command = Command::new("sh");
            command.arg("-lc").arg(&case.command);
            if let Some(cwd) = &case.cwd {
                command.current_dir(cwd);
            }
            command.output().await
        })
        .await??;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);
        let stdout_ok = case
            .expected_stdout_contains
            .iter()
            .all(|needle| stdout.contains(needle));
        let exit_ok = case
            .expected_exit_code
            .map_or(output.status.success(), |expected| expected == exit_code);
        let reward = if stdout_ok && exit_ok { 1.0 } else { 0.0 };
        let response = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
        self.observations.push(response.clone());
        self.cursor += 1;
        self.terminal = self.cursor >= self.cases.len();
        let done = self.terminal;
        let metadata = serde_json::json!({
            "case": case.name,
            "exit_code": exit_code,
            "stdout_ok": stdout_ok,
            "exit_ok": exit_ok,
        });
        self.steps.push(TrajectoryStep {
            action,
            response: Some(response.clone()),
            reward,
            done,
            at: Utc::now(),
            token_capture: Some(unsupported_token_capture()),
            metadata: metadata.clone(),
        });

        Ok(StepResult {
            state: self.state(),
            response: Some(response),
            reward,
            done,
            metadata,
        })
    }

    fn score(&self) -> f64 {
        if self.steps.is_empty() {
            0.0
        } else {
            self.steps.iter().map(|step| step.reward).sum::<f64>() / self.steps.len() as f64
        }
    }

    fn is_terminal(&self) -> bool {
        self.terminal
    }

    async fn export_trajectory(&self) -> Trajectory {
        Trajectory {
            env_name: self.name.clone(),
            episode_id: self.episode_id.clone(),
            score: self.score(),
            steps: self.steps.clone(),
            metadata: serde_json::json!({
                "benchmark": "terminal_bench",
                "case_count": self.cases.len(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillBenchCase {
    pub name: String,
    pub skill_content: String,
    #[serde(default)]
    pub required_substrings: Vec<String>,
}

pub struct SkillBenchEnv {
    name: String,
    episode_id: String,
    cases: Vec<SkillBenchCase>,
    cursor: usize,
    steps: Vec<TrajectoryStep>,
    observations: Vec<String>,
    terminal: bool,
}

impl SkillBenchEnv {
    pub fn new(cases: Vec<SkillBenchCase>) -> Self {
        Self {
            name: "skill_bench".to_string(),
            episode_id: Uuid::new_v4().to_string(),
            cases,
            cursor: 0,
            steps: Vec::new(),
            observations: Vec::new(),
            terminal: false,
        }
    }

    fn state(&self) -> EnvState {
        EnvState {
            episode_id: self.episode_id.clone(),
            observations: self.observations.clone(),
            metadata: serde_json::json!({
                "benchmark": "skill_bench",
                "case_index": self.cursor,
                "case_count": self.cases.len(),
            }),
        }
    }
}

#[async_trait]
impl AgentEnv for SkillBenchEnv {
    fn name(&self) -> &str {
        &self.name
    }

    async fn reset(&mut self) -> EnvState {
        self.episode_id = Uuid::new_v4().to_string();
        self.cursor = 0;
        self.steps.clear();
        self.observations.clear();
        self.terminal = self.cases.is_empty();
        self.state()
    }

    async fn step(&mut self, action: AgentAction) -> anyhow::Result<StepResult> {
        if self.terminal {
            return Ok(StepResult {
                state: self.state(),
                response: None,
                reward: 0.0,
                done: true,
                metadata: serde_json::json!({ "terminal": true }),
            });
        }
        let Some(case) = self.cases.get(self.cursor).cloned() else {
            self.terminal = true;
            return Ok(StepResult {
                state: self.state(),
                response: None,
                reward: 0.0,
                done: true,
                metadata: serde_json::json!({ "terminal": true }),
            });
        };
        let has_heading = case.skill_content.contains("# ");
        let has_body = case.skill_content.lines().count() > 1;
        let required_ok = case
            .required_substrings
            .iter()
            .all(|needle| case.skill_content.contains(needle));
        let reward = if has_heading && has_body && required_ok {
            1.0
        } else {
            0.0
        };
        let response = if reward >= 1.0 {
            format!("skill bench '{}' passed", case.name)
        } else {
            format!("skill bench '{}' failed", case.name)
        };
        self.observations.push(response.clone());
        self.cursor += 1;
        self.terminal = self.cursor >= self.cases.len();
        let done = self.terminal;
        let metadata = serde_json::json!({
            "case": case.name,
            "has_heading": has_heading,
            "has_body": has_body,
            "required_ok": required_ok,
        });
        self.steps.push(TrajectoryStep {
            action,
            response: Some(response.clone()),
            reward,
            done,
            at: Utc::now(),
            token_capture: Some(unsupported_token_capture()),
            metadata: metadata.clone(),
        });

        Ok(StepResult {
            state: self.state(),
            response: Some(response),
            reward,
            done,
            metadata,
        })
    }

    fn score(&self) -> f64 {
        if self.steps.is_empty() {
            0.0
        } else {
            self.steps.iter().map(|step| step.reward).sum::<f64>() / self.steps.len() as f64
        }
    }

    fn is_terminal(&self) -> bool {
        self.terminal
    }

    async fn export_trajectory(&self) -> Trajectory {
        Trajectory {
            env_name: self.name.clone(),
            episode_id: self.episode_id.clone(),
            score: self.score(),
            steps: self.steps.clone(),
            metadata: serde_json::json!({
                "benchmark": "skill_bench",
                "case_count": self.cases.len(),
            }),
        }
    }
}

pub struct EnvRunner<E: AgentEnv> {
    env: E,
    artifact_logger: AgentRunArtifactLogger,
}

impl<E: AgentEnv> EnvRunner<E> {
    pub fn new(env: E) -> Self {
        Self {
            env,
            artifact_logger: AgentRunArtifactLogger::new(),
        }
    }

    pub fn with_artifact_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.artifact_logger = AgentRunArtifactLogger::with_root(root);
        self
    }

    pub async fn evaluate<F>(
        &mut self,
        n_episodes: usize,
        mut actions_for_episode: F,
    ) -> anyhow::Result<Vec<Trajectory>>
    where
        F: FnMut(usize) -> Vec<AgentAction>,
    {
        let mut trajectories = Vec::new();
        for episode in 0..n_episodes {
            self.env.reset().await;
            for action in actions_for_episode(episode) {
                let _ = self.env.step(action).await?;
                if self.env.is_terminal() {
                    break;
                }
            }
            let trajectory = self.env.export_trajectory().await;
            self.persist_trajectory_artifact(&trajectory).await?;
            trajectories.push(trajectory);
        }
        Ok(trajectories)
    }

    pub async fn collect_sft_jsonl(
        &mut self,
        n_episodes: usize,
        output: &Path,
        prompt_for_episode: impl FnMut(usize) -> String,
    ) -> anyhow::Result<()> {
        let mut prompt_for_episode = prompt_for_episode;
        let trajectories = self
            .evaluate(n_episodes, |idx| {
                vec![AgentAction::UserMessage {
                    content: prompt_for_episode(idx),
                }]
            })
            .await?;
        if let Some(parent) = output.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(output)
            .await?;
        use tokio::io::AsyncWriteExt;
        for trajectory in trajectories {
            if trajectory.score < 0.6 {
                continue;
            }
            for step in trajectory.steps {
                let AgentAction::UserMessage { content } = step.action;
                let Some(response) = step.response else {
                    continue;
                };
                let row = serde_json::json!({
                    "messages": [
                        { "role": "user", "content": content },
                        { "role": "assistant", "content": response }
                    ],
                    "metadata": {
                        "env": trajectory.env_name,
                        "episode_id": trajectory.episode_id,
                        "score": trajectory.score
                    }
                });
                file.write_all(serde_json::to_string(&row)?.as_bytes())
                    .await?;
                file.write_all(b"\n").await?;
            }
        }
        Ok(())
    }

    pub async fn serve_openai_compatible(self, addr: SocketAddr) -> anyhow::Result<()>
    where
        E: 'static,
    {
        let state = Arc::new(Mutex::new(self.env));
        let app = Router::new()
            .route("/v1/chat/completions", post(openai_chat_completions::<E>))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;
        Ok(())
    }

    async fn persist_trajectory_artifact(&self, trajectory: &Trajectory) -> anyhow::Result<()> {
        let status = if trajectory.score >= 0.6 {
            AgentRunStatus::Completed
        } else {
            AgentRunStatus::Failed
        };
        let mut artifact = AgentRunArtifact::new("agent_env", status, Utc::now(), Some(Utc::now()))
            .with_metadata(serde_json::to_value(trajectory)?);
        artifact.run_id = trajectory.episode_id.clone();
        self.artifact_logger.append_artifact(&artifact).await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiChatRequest {
    #[serde(default)]
    messages: Vec<OpenAiMessage>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

async fn openai_chat_completions<E: AgentEnv>(
    State(env): State<Arc<Mutex<E>>>,
    Json(request): Json<OpenAiChatRequest>,
) -> Json<serde_json::Value> {
    let prompt = request
        .messages
        .iter()
        .filter(|message| message.role == "user")
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut env = env.lock().await;
    let response = match env.step(AgentAction::UserMessage { content: prompt }).await {
        Ok(result) => result.response.unwrap_or_default(),
        Err(error) => format!("Environment error: {error}"),
    };
    Json(serde_json::json!({
        "id": format!("chatcmpl-{}", Uuid::new_v4().simple()),
        "object": "chat.completion",
        "created": Utc::now().timestamp(),
        "model": request.model.unwrap_or_else(|| "thinclaw-agent-env".to_string()),
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": response },
            "finish_reason": "stop"
        }]
    }))
}

fn heuristic_reward(response: Option<&str>) -> f64 {
    match response.map(str::trim) {
        Some("") | None => 0.0,
        Some(text) if text.to_ascii_lowercase().contains("error:") => 0.2,
        Some(_) => 1.0,
    }
}

fn unsupported_token_capture() -> TokenTrajectoryCapture {
    token_capture_from_support(TokenCaptureSupport::UNSUPPORTED, None, None)
}

fn token_capture_from_support(
    support: TokenCaptureSupport,
    provider: Option<String>,
    model: Option<String>,
) -> TokenTrajectoryCapture {
    TokenTrajectoryCapture {
        exact_tokens_supported: support.exact_tokens_supported,
        logprobs_supported: support.logprobs_supported,
        token_ids: Vec::new(),
        tokens: Vec::new(),
        logprobs: Vec::new(),
        provider,
        model,
    }
}

fn agent_token_capture(agent: &Agent) -> TokenTrajectoryCapture {
    let provider = agent.llm_provider_name();
    token_capture_from_support(
        agent.llm_token_capture_support(),
        Some(provider.clone()),
        Some(provider),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_reward_scores_empty_and_errors_low() {
        assert_eq!(heuristic_reward(None), 0.0);
        assert_eq!(heuristic_reward(Some("")), 0.0);
        assert!(heuristic_reward(Some("Error: failed")) < 0.5);
        assert_eq!(heuristic_reward(Some("All set")), 1.0);
    }

    #[test]
    fn trajectory_step_serializes_exact_token_capture_capabilities() {
        let step = TrajectoryStep {
            action: AgentAction::UserMessage {
                content: "hello".to_string(),
            },
            response: Some("world".to_string()),
            reward: 1.0,
            done: true,
            at: Utc::now(),
            token_capture: Some(TokenTrajectoryCapture {
                exact_tokens_supported: true,
                logprobs_supported: true,
                token_ids: vec![1, 2],
                tokens: vec!["hello".to_string(), "world".to_string()],
                logprobs: vec![-0.1, -0.2],
                provider: Some("test".to_string()),
                model: Some("model".to_string()),
            }),
            metadata: serde_json::json!({}),
        };

        let value = serde_json::to_value(step).expect("serialize step");
        assert_eq!(value["token_capture"]["exact_tokens_supported"], true);
        let first_logprob = value["token_capture"]["logprobs"][0]
            .as_f64()
            .expect("first logprob should serialize as a float");
        assert!((first_logprob - -0.1).abs() < 0.000_001);
    }

    #[test]
    fn token_capture_from_support_preserves_provider_capability_flags() {
        let capture = token_capture_from_support(
            TokenCaptureSupport {
                exact_tokens_supported: true,
                logprobs_supported: false,
            },
            Some("openai-compatible".to_string()),
            Some("gpt-test".to_string()),
        );

        assert!(capture.exact_tokens_supported);
        assert!(!capture.logprobs_supported);
        assert_eq!(capture.provider.as_deref(), Some("openai-compatible"));
        assert_eq!(capture.model.as_deref(), Some("gpt-test"));
        assert!(capture.token_ids.is_empty());
        assert!(capture.logprobs.is_empty());
    }

    #[tokio::test]
    async fn terminal_bench_env_runs_command_cases() {
        let mut env = TerminalBenchEnv::new(vec![TerminalBenchCase {
            name: "echo".to_string(),
            command: "printf bench-ok".to_string(),
            cwd: None,
            expected_stdout_contains: vec!["bench-ok".to_string()],
            expected_exit_code: Some(0),
            timeout_secs: 5,
        }]);

        let state = env.reset().await;
        assert_eq!(
            state.metadata["benchmark"],
            serde_json::json!("terminal_bench")
        );
        let result = env
            .step(AgentAction::UserMessage {
                content: "run".to_string(),
            })
            .await
            .expect("terminal bench step");
        assert!(result.done);
        assert_eq!(result.reward, 1.0);
        assert_eq!(env.score(), 1.0);
        let trajectory = env.export_trajectory().await;
        assert_eq!(
            trajectory.metadata["benchmark"],
            serde_json::json!("terminal_bench")
        );
        let capture = trajectory.steps[0]
            .token_capture
            .as_ref()
            .expect("token capture capability marker");
        assert!(!capture.exact_tokens_supported);
        assert!(!capture.logprobs_supported);
        assert!(capture.token_ids.is_empty());
        assert!(capture.logprobs.is_empty());
    }

    #[tokio::test]
    async fn skill_bench_env_scores_skill_content() {
        let mut env = SkillBenchEnv::new(vec![SkillBenchCase {
            name: "skill".to_string(),
            skill_content: "# Test Skill\nDo the thing.".to_string(),
            required_substrings: vec!["Do the thing".to_string()],
        }]);
        env.reset().await;
        let result = env
            .step(AgentAction::UserMessage {
                content: "check".to_string(),
            })
            .await
            .expect("skill bench step");
        assert!(result.done);
        assert_eq!(result.reward, 1.0);
        let trajectory = env.export_trajectory().await;
        assert_eq!(trajectory.env_name, "skill_bench");
        let capture = trajectory.steps[0]
            .token_capture
            .as_ref()
            .expect("token capture capability marker");
        assert!(!capture.exact_tokens_supported);
        assert!(!capture.logprobs_supported);
    }
}
