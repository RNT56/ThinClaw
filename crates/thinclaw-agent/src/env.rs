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
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Mutex;
use uuid::Uuid;

use thinclaw_channels_core::IncomingMessage;
use thinclaw_llm_core::{ProviderTokenCapture, TokenCaptureSupport};

use crate::run_artifact::{AgentRunArtifact, AgentRunArtifactLogger, AgentRunStatus};

#[async_trait]
pub trait AgentEnvAgent: Send + Sync {
    async fn handle_env_message(&self, message: &IncomingMessage)
    -> anyhow::Result<Option<String>>;

    async fn latest_token_capture_for_env_message(
        &self,
        message: &IncomingMessage,
    ) -> Option<ProviderTokenCapture>;

    fn env_llm_token_capture_support(&self) -> TokenCaptureSupport;

    fn env_llm_provider_name(&self) -> String;
}

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

pub fn average_trajectory_score(trajectories: &[Trajectory]) -> f64 {
    if trajectories.is_empty() {
        0.0
    } else {
        trajectories
            .iter()
            .map(|trajectory| trajectory.score)
            .sum::<f64>()
            / trajectories.len() as f64
    }
}

pub fn trajectory_summary(trajectories: &[Trajectory]) -> serde_json::Value {
    let mut env_names = trajectories
        .iter()
        .map(|trajectory| trajectory.env_name.clone())
        .collect::<Vec<_>>();
    env_names.sort();
    env_names.dedup();

    let mut exact_tokens_supported = false;
    let mut logprobs_supported = false;
    let mut captured_token_ids = 0usize;
    let mut captured_logprobs = 0usize;
    let mut token_capture_steps = 0usize;
    let mut step_count = 0usize;

    for trajectory in trajectories {
        step_count += trajectory.steps.len();
        for step in &trajectory.steps {
            if let Some(capture) = step.token_capture.as_ref() {
                token_capture_steps += 1;
                exact_tokens_supported |= capture.exact_tokens_supported;
                logprobs_supported |= capture.logprobs_supported;
                captured_token_ids += capture.token_ids.len();
                captured_logprobs += capture.logprobs.len();
            }
        }
    }

    serde_json::json!({
        "env_names": env_names,
        "episode_count": trajectories.len(),
        "step_count": step_count,
        "score": average_trajectory_score(trajectories),
        "exact_tokens_supported": exact_tokens_supported,
        "logprobs_supported": logprobs_supported,
        "token_capture_steps": token_capture_steps,
        "captured_token_ids": captured_token_ids,
        "captured_logprobs": captured_logprobs,
    })
}

pub fn render_trajectory_log(trajectories: &[Trajectory]) -> String {
    let mut log = String::new();
    for trajectory in trajectories {
        log.push_str(&format!(
            "== {} {} score {:.3} ==\n",
            trajectory.env_name, trajectory.episode_id, trajectory.score
        ));
        for step in &trajectory.steps {
            log.push_str(&format!(
                "reward={:.3} done={}\n{}\n",
                step.reward,
                step.done,
                step.response.as_deref().unwrap_or_default()
            ));
        }
    }
    log
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

#[derive(Clone)]
pub struct AgentLoopEnv {
    name: String,
    agent: Arc<dyn AgentEnvAgent>,
    episode_id: String,
    session_key: String,
    user_id: String,
    max_steps: usize,
    observations: Vec<String>,
    steps: Vec<TrajectoryStep>,
    terminal: bool,
}

impl AgentLoopEnv {
    pub fn new(agent: Arc<dyn AgentEnvAgent>) -> Self {
        Self::named("agent_loop", agent)
    }

    pub fn named(name: impl Into<String>, agent: Arc<dyn AgentEnvAgent>) -> Self {
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
        let response = self.agent.handle_env_message(&message).await?;
        let token_capture = self
            .agent
            .latest_token_capture_for_env_message(&message)
            .await
            .map(token_capture_from_provider_capture)
            .unwrap_or_else(|| agent_token_capture(&self.agent));
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
            token_capture: Some(token_capture),
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
    /// Scripted fallback command, used only when the agent's action carries
    /// no usable command text (see `TerminalBenchEnv::step`). Verified
    /// episodes execute the agent's own action instead of this command.
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, alias = "expectedStdoutContains")]
    pub expected_stdout_contains: Vec<String>,
    #[serde(default, alias = "expectedExitCode")]
    pub expected_exit_code: Option<i32>,
    #[serde(default = "default_terminal_bench_timeout_secs", alias = "timeoutSecs")]
    pub timeout_secs: u64,
}

impl TerminalBenchCase {
    /// A case is "verifiable" when it declares at least one expected-output
    /// check. Verifiable cases score strictly from those checks; cases with
    /// no checks at all cannot be verified and fall back to a heuristic
    /// grade of the agent's own output (see `TerminalBenchEnv::step`).
    fn is_verifiable(&self) -> bool {
        !self.expected_stdout_contains.is_empty() || self.expected_exit_code.is_some()
    }
}

fn default_terminal_bench_timeout_secs() -> u64 {
    30
}

#[derive(Clone)]
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

        // The agent's action carries the command it decided to run for this
        // case. Prefer that over the scripted `case.command` so the
        // benchmark measures the agent's own behavior. Only fall back to the
        // scripted command (for log continuity) when the agent produced no
        // usable command text; that fallback path is unverified and always
        // scores 0.0.
        let AgentAction::UserMessage { content } = &action;
        let agent_command = content.trim();
        let agent_action_usable = !agent_command.is_empty();
        let command_to_run: &str = if agent_action_usable {
            agent_command
        } else {
            case.command.as_str()
        };

        let output = tokio::time::timeout(Duration::from_secs(case.timeout_secs), async {
            let mut command = Command::new("sh");
            command.arg("-lc").arg(command_to_run);
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
        let verified = agent_action_usable && case.is_verifiable();
        let reward = if !agent_action_usable {
            // No usable agent action: the scripted fallback ran (for log
            // continuity) but nothing about the agent was measured.
            0.0
        } else if verified {
            if stdout_ok && exit_ok { 1.0 } else { 0.0 }
        } else {
            // The agent acted, but the case has no expected-output checks to
            // verify against; grade the agent's own output heuristically.
            heuristic_reward(Some(&stdout))
        };
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
            "agent_action_usable": agent_action_usable,
            "verified": verified,
            "command_source": if agent_action_usable { "agent_action" } else { "scripted_fallback" },
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
    /// Reference skill content shown to the agent as context for this case.
    /// This is no longer scored directly (see `SkillBenchEnv::step`) — it is
    /// prompt material, not the agent's output.
    #[serde(alias = "skillContent")]
    pub skill_content: String,
    /// Substrings the agent's action/answer must contain to be considered
    /// correct. This doubles as the case's verifier: a case with no required
    /// substrings is unverifiable and falls back to a heuristic grade of the
    /// agent's action (see `SkillBenchEnv::step`).
    #[serde(default, alias = "requiredSubstrings")]
    pub required_substrings: Vec<String>,
}

impl SkillBenchCase {
    /// A case is "verifiable" when it declares at least one required
    /// substring to check the agent's action against.
    fn is_verifiable(&self) -> bool {
        !self.required_substrings.is_empty()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "benchmark", rename_all = "snake_case")]
pub enum AgentEnvBenchmarkConfig {
    TerminalBench {
        #[serde(default)]
        cases: Vec<TerminalBenchCase>,
        /// When true, actions are produced by the live agent runtime instead
        /// of scripted per-case reference actions. Defaults to false so
        /// benchmark trials stay deterministic and offline unless a campaign
        /// explicitly opts in.
        #[serde(default)]
        live_agent: bool,
    },
    SkillBench {
        #[serde(default)]
        cases: Vec<SkillBenchCase>,
        /// See `TerminalBench::live_agent`.
        #[serde(default)]
        live_agent: bool,
    },
}

pub fn agent_env_benchmark_config(
    backend_config: &serde_json::Value,
) -> Result<Option<AgentEnvBenchmarkConfig>, String> {
    let source = backend_config
        .get("agent_env")
        .or_else(|| backend_config.get("benchmark_config"))
        .unwrap_or(backend_config);
    if source.get("benchmark").is_none() {
        return Ok(None);
    }
    serde_json::from_value(source.clone())
        .map(Some)
        .map_err(|err| format!("Invalid AgentEnv benchmark config: {err}"))
}

#[derive(Clone)]
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
        // Score the agent's own action/answer against the case's verifier,
        // rather than statically inspecting the reference `skill_content`.
        let AgentAction::UserMessage { content } = &action;
        let agent_answer = content.trim();
        let agent_action_usable = !agent_answer.is_empty();
        let verified = agent_action_usable && case.is_verifiable();
        let required_ok = case
            .required_substrings
            .iter()
            .all(|needle| agent_answer.contains(needle));
        let reward = if !agent_action_usable {
            // No usable agent action to grade at all.
            0.0
        } else if verified {
            if required_ok { 1.0 } else { 0.0 }
        } else {
            // The agent answered, but the case declares no verifier; grade
            // the agent's own answer heuristically instead of the static
            // reference `skill_content`.
            heuristic_reward(Some(agent_answer))
        };
        let response = if !agent_action_usable {
            format!("skill bench '{}' failed (no agent action)", case.name)
        } else if verified {
            if required_ok {
                format!("skill bench '{}' passed", case.name)
            } else {
                format!("skill bench '{}' failed", case.name)
            }
        } else {
            format!(
                "skill bench '{}' unverified (heuristic reward {reward:.2})",
                case.name
            )
        };
        self.observations.push(response.clone());
        self.cursor += 1;
        self.terminal = self.cursor >= self.cases.len();
        let done = self.terminal;
        let metadata = serde_json::json!({
            "case": case.name,
            "agent_action_usable": agent_action_usable,
            "verified": verified,
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

/// Trajectory score threshold used to gate SFT export and "completed" run
/// artifact status. Verifier rewards (see `TerminalBenchEnv::step` and
/// `SkillBenchEnv::step`) can score up to `1.0` and clear this gate;
/// unverified `heuristic_reward` fallback scores are capped at `0.55` —
/// strictly below this constant — so they can never clear the gate on
/// their own, which is what makes the gate meaningful again.
pub const SFT_QUALITY_GATE_SCORE: f64 = 0.6;

/// Default number of episodes run concurrently by [`EnvRunner::evaluate`]
/// when the caller does not choose a different bound via
/// [`EnvRunner::with_concurrency`].
pub const DEFAULT_EVAL_CONCURRENCY: usize = 4;

pub struct EnvRunner<E: AgentEnv> {
    env: E,
    artifact_logger: AgentRunArtifactLogger,
    concurrency: usize,
}

impl<E: AgentEnv> EnvRunner<E> {
    pub fn new(env: E) -> Self {
        Self {
            env,
            artifact_logger: AgentRunArtifactLogger::new(),
            concurrency: DEFAULT_EVAL_CONCURRENCY,
        }
    }

    pub fn with_artifact_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.artifact_logger = AgentRunArtifactLogger::with_root(root);
        self
    }

    /// Bound the number of episodes run concurrently by [`Self::evaluate`].
    /// A value of `0` is treated as `1` (fully sequential).
    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency.max(1);
        self
    }

    /// Run `n_episodes` episodes and return their trajectories in episode
    /// order.
    ///
    /// Independent episodes are run concurrently, bounded by
    /// [`Self::with_concurrency`] (default [`DEFAULT_EVAL_CONCURRENCY`]).
    /// Each episode runs against its own clone of the environment (reset
    /// fresh for that episode), so steps *within* an episode stay strictly
    /// sequential and never race with each other — only different episodes
    /// may run at the same time. Results are collected and then persisted
    /// (artifact logging) and returned in deterministic episode order
    /// regardless of which episode happens to finish first.
    pub async fn evaluate<F>(
        &mut self,
        n_episodes: usize,
        mut actions_for_episode: F,
    ) -> anyhow::Result<Vec<Trajectory>>
    where
        E: Clone,
        F: FnMut(usize) -> Vec<AgentAction>,
    {
        let episode_actions: Vec<(usize, Vec<AgentAction>)> = (0..n_episodes)
            .map(|episode| (episode, actions_for_episode(episode)))
            .collect();

        let concurrency = self.concurrency.max(1);
        let base_env = self.env.clone();
        let mut results: Vec<(usize, anyhow::Result<Trajectory>)> = stream::iter(episode_actions)
            .map(|(episode, actions)| {
                let mut episode_env = base_env.clone();
                async move {
                    let result: anyhow::Result<Trajectory> = async {
                        episode_env.reset().await;
                        for action in actions {
                            let _ = episode_env.step(action).await?;
                            if episode_env.is_terminal() {
                                break;
                            }
                        }
                        Ok(episode_env.export_trajectory().await)
                    }
                    .await;
                    (episode, result)
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;

        results.sort_by_key(|(episode, _)| *episode);

        let mut trajectories = Vec::with_capacity(results.len());
        for (_, result) in results {
            let trajectory = result?;
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
    ) -> anyhow::Result<()>
    where
        E: Clone,
    {
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
            if trajectory.score < SFT_QUALITY_GATE_SCORE {
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
        let status = if trajectory.score >= SFT_QUALITY_GATE_SCORE {
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

/// Minimum response length (in characters, after trimming) for a response to
/// be considered "substantive" rather than "trivial" by [`heuristic_reward`].
const HEURISTIC_SUBSTANTIVE_MIN_LEN: usize = 40;

/// Graded fallback reward used only when an environment has no verifier
/// available for a step (i.e. there is no scripted/expected answer to check
/// the agent's action against).
///
/// This is intentionally a coarse, cheap proxy and is capped at `0.55`,
/// strictly below [`SFT_QUALITY_GATE_SCORE`]: it can tell "the agent said
/// nothing useful" from "the agent said something", but it cannot confirm
/// correctness. Verifier rewards (see `TerminalBenchEnv::step` and
/// `SkillBenchEnv::step`) supersede this heuristic whenever a case supplies
/// expected-output checks, and only verified rewards can clear the SFT
/// quality gate used by [`EnvRunner::collect_sft_jsonl`] and
/// [`EnvRunner::persist_trajectory_artifact`].
///
/// Scoring bands:
/// - `0.0`: empty/missing response, or a response carrying an `error:` marker.
/// - `0.3`: non-empty but trivial (shorter than
///   [`HEURISTIC_SUBSTANTIVE_MIN_LEN`] characters after trimming).
/// - `0.55`: substantive response with no verifier to confirm correctness.
fn heuristic_reward(response: Option<&str>) -> f64 {
    match response.map(str::trim) {
        Some("") | None => 0.0,
        Some(text) if text.to_ascii_lowercase().contains("error:") => 0.0,
        Some(text) if text.chars().count() < HEURISTIC_SUBSTANTIVE_MIN_LEN => 0.3,
        // Strictly below SFT_QUALITY_GATE_SCORE: an unverified response must
        // never clear the export gate on heuristics alone.
        Some(_) => 0.55,
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

fn token_capture_from_provider_capture(capture: ProviderTokenCapture) -> TokenTrajectoryCapture {
    TokenTrajectoryCapture {
        exact_tokens_supported: capture.exact_tokens_supported,
        logprobs_supported: capture.logprobs_supported,
        token_ids: capture.token_ids,
        tokens: capture.tokens,
        logprobs: capture.logprobs,
        provider: capture.provider,
        model: capture.model,
    }
}

fn agent_token_capture(agent: &Arc<dyn AgentEnvAgent>) -> TokenTrajectoryCapture {
    let provider = agent.env_llm_provider_name();
    token_capture_from_support(
        agent.env_llm_token_capture_support(),
        Some(provider.clone()),
        Some(provider),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_reward_is_graded_and_capped_at_the_sft_quality_gate() {
        // Empty / missing / error-marker responses score 0.0.
        assert_eq!(heuristic_reward(None), 0.0);
        assert_eq!(heuristic_reward(Some("")), 0.0);
        assert_eq!(heuristic_reward(Some("   ")), 0.0);
        assert_eq!(heuristic_reward(Some("Error: failed")), 0.0);
        assert_eq!(heuristic_reward(Some("error: lowercase too")), 0.0);

        // Trivial (short) non-empty responses score 0.3.
        assert_eq!(heuristic_reward(Some("ok")), 0.3);
        assert_eq!(heuristic_reward(Some("done")), 0.3);

        // Substantive responses score 0.55 — strictly BELOW the SFT quality
        // gate, since the heuristic cannot confirm correctness. Only
        // verifier-backed rewards may clear the gate.
        let substantive =
            "Here is a longer, substantive response describing what happened in detail.";
        assert!(substantive.len() >= HEURISTIC_SUBSTANTIVE_MIN_LEN);
        let score = heuristic_reward(Some(substantive));
        assert_eq!(score, 0.55);
        assert!(score < SFT_QUALITY_GATE_SCORE);
    }

    #[test]
    fn verifier_reward_beats_heuristic_reward_for_the_same_kind_of_response() {
        // A verified pass (used by TerminalBenchEnv/SkillBenchEnv when a
        // case's expected-output checks are satisfied) scores 1.0, which is
        // strictly greater than anything heuristic_reward can produce on its
        // own, and clears the SFT quality gate.
        let verified_pass_reward = 1.0;
        let best_possible_heuristic = heuristic_reward(Some(
            "A long substantive unverified response with no way to confirm correctness.",
        ));
        assert!(verified_pass_reward > best_possible_heuristic);
        assert!(
            best_possible_heuristic < SFT_QUALITY_GATE_SCORE,
            "unverified heuristic scores must not clear the SFT gate"
        );
        assert!(verified_pass_reward >= SFT_QUALITY_GATE_SCORE);
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

    #[test]
    fn token_capture_from_provider_preserves_real_provider_arrays() {
        let capture = token_capture_from_provider_capture(ProviderTokenCapture {
            exact_tokens_supported: true,
            logprobs_supported: true,
            token_ids: vec![11, 12],
            tokens: vec!["real".to_string(), " data".to_string()],
            logprobs: vec![-0.7, -0.8],
            provider: Some("openai".to_string()),
            model: Some("gpt-test".to_string()),
        });

        assert!(capture.exact_tokens_supported);
        assert!(capture.logprobs_supported);
        assert_eq!(capture.token_ids, vec![11, 12]);
        assert_eq!(capture.tokens, vec!["real", " data"]);
        assert_eq!(capture.logprobs, vec![-0.7, -0.8]);
        assert_eq!(capture.provider.as_deref(), Some("openai"));
        assert_eq!(capture.model.as_deref(), Some("gpt-test"));
    }

    #[test]
    fn trajectory_summary_counts_scores_steps_and_token_capture() {
        let trajectory = Trajectory {
            env_name: "skill_bench".to_string(),
            episode_id: "episode-1".to_string(),
            score: 0.5,
            steps: vec![TrajectoryStep {
                action: AgentAction::UserMessage {
                    content: "check".to_string(),
                },
                response: Some("ok".to_string()),
                reward: 0.5,
                done: true,
                at: Utc::now(),
                token_capture: Some(TokenTrajectoryCapture {
                    exact_tokens_supported: true,
                    logprobs_supported: true,
                    token_ids: vec![1, 2],
                    tokens: vec!["o".to_string(), "k".to_string()],
                    logprobs: vec![-0.1, -0.2],
                    provider: Some("test".to_string()),
                    model: Some("model".to_string()),
                }),
                metadata: serde_json::json!({}),
            }],
            metadata: serde_json::json!({}),
        };

        let summary = trajectory_summary(std::slice::from_ref(&trajectory));
        assert_eq!(
            average_trajectory_score(std::slice::from_ref(&trajectory)),
            0.5
        );
        assert_eq!(summary["env_names"], serde_json::json!(["skill_bench"]));
        assert_eq!(summary["episode_count"], serde_json::json!(1));
        assert_eq!(summary["step_count"], serde_json::json!(1));
        assert_eq!(summary["exact_tokens_supported"], serde_json::json!(true));
        assert_eq!(summary["captured_token_ids"], serde_json::json!(2));
        let log = render_trajectory_log(&[trajectory]);
        assert!(log.contains("== skill_bench episode-1 score 0.500 =="));
        assert!(log.contains("reward=0.500 done=true"));
        assert!(log.contains("ok"));
    }

    #[test]
    fn benchmark_cases_accept_webui_camel_case_fields() {
        let terminal: TerminalBenchCase = serde_json::from_value(serde_json::json!({
            "name": "smoke",
            "command": "printf ok",
            "expectedStdoutContains": ["ok"],
            "expectedExitCode": 0,
            "timeoutSecs": 7
        }))
        .expect("terminal bench camelCase");
        assert_eq!(terminal.expected_stdout_contains, vec!["ok"]);
        assert_eq!(terminal.expected_exit_code, Some(0));
        assert_eq!(terminal.timeout_secs, 7);

        let skill: SkillBenchCase = serde_json::from_value(serde_json::json!({
            "name": "skill",
            "skillContent": "# Skill",
            "requiredSubstrings": ["Skill"]
        }))
        .expect("skill bench camelCase");
        assert_eq!(skill.skill_content, "# Skill");
        assert_eq!(skill.required_substrings, vec!["Skill"]);
    }

    #[test]
    fn agent_env_benchmark_config_parses_nested_backend_config() {
        let config = agent_env_benchmark_config(&serde_json::json!({
            "agent_env": {
                "benchmark": "terminal_bench",
                "cases": [{
                    "name": "smoke",
                    "command": "printf ok",
                    "expectedExitCode": 0
                }]
            }
        }))
        .expect("parse benchmark config")
        .expect("benchmark config present");

        match config {
            AgentEnvBenchmarkConfig::TerminalBench { cases, .. } => {
                assert_eq!(cases.len(), 1);
                assert_eq!(cases[0].name, "smoke");
                assert_eq!(cases[0].expected_exit_code, Some(0));
            }
            AgentEnvBenchmarkConfig::SkillBench { .. } => panic!("expected terminal bench"),
        }

        assert!(
            agent_env_benchmark_config(&serde_json::json!({ "other": true }))
                .expect("empty config should parse")
                .is_none()
        );
    }

    fn terminal_bench_case(name: &str, command: &str) -> TerminalBenchCase {
        TerminalBenchCase {
            name: name.to_string(),
            command: command.to_string(),
            cwd: None,
            expected_stdout_contains: vec!["bench-ok".to_string()],
            expected_exit_code: Some(0),
            timeout_secs: 5,
        }
    }

    fn skill_bench_case(name: &str) -> SkillBenchCase {
        SkillBenchCase {
            name: name.to_string(),
            skill_content: "# Test Skill\nDo the thing.".to_string(),
            required_substrings: vec!["Do the thing".to_string()],
        }
    }

    #[tokio::test]
    async fn terminal_bench_env_runs_command_cases() {
        // The scripted `command` field is a deliberately wrong fallback here
        // so this test only passes if the env actually executes the agent's
        // action rather than the scripted command.
        let mut env = TerminalBenchEnv::new(vec![terminal_bench_case(
            "echo",
            "printf scripted-fallback-should-not-run",
        )]);

        let state = env.reset().await;
        assert_eq!(
            state.metadata["benchmark"],
            serde_json::json!("terminal_bench")
        );
        let result = env
            .step(AgentAction::UserMessage {
                content: "printf bench-ok".to_string(),
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
        assert_eq!(
            trajectory.steps[0].metadata["command_source"],
            serde_json::json!("agent_action")
        );
        assert_eq!(
            trajectory.steps[0].metadata["verified"],
            serde_json::json!(true)
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
    async fn terminal_bench_env_scores_wrong_agent_action_as_verified_failure() {
        let mut env = TerminalBenchEnv::new(vec![terminal_bench_case(
            "echo",
            "printf scripted-fallback",
        )]);
        env.reset().await;
        let result = env
            .step(AgentAction::UserMessage {
                content: "printf totally-different-output".to_string(),
            })
            .await
            .expect("terminal bench step");
        assert_eq!(result.reward, 0.0);
        assert_eq!(
            result.metadata["command_source"],
            serde_json::json!("agent_action")
        );
        assert_eq!(result.metadata["verified"], serde_json::json!(true));
        assert_eq!(result.metadata["stdout_ok"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn terminal_bench_env_falls_back_to_scripted_command_and_scores_zero_when_agent_action_empty()
     {
        let mut env = TerminalBenchEnv::new(vec![terminal_bench_case("echo", "printf bench-ok")]);
        env.reset().await;
        let result = env
            .step(AgentAction::UserMessage {
                content: "   ".to_string(),
            })
            .await
            .expect("terminal bench step");
        // The scripted command still ran (so stdout_ok reflects that it
        // would have passed), but the step is unverified/agent-less and
        // therefore scores 0.0 regardless.
        assert_eq!(result.reward, 0.0);
        assert_eq!(
            result.metadata["command_source"],
            serde_json::json!("scripted_fallback")
        );
        assert_eq!(
            result.metadata["agent_action_usable"],
            serde_json::json!(false)
        );
        assert_eq!(result.metadata["stdout_ok"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn skill_bench_env_scores_agent_action_against_verifier() {
        let mut env = SkillBenchEnv::new(vec![skill_bench_case("skill")]);
        env.reset().await;
        let result = env
            .step(AgentAction::UserMessage {
                content: "Do the thing".to_string(),
            })
            .await
            .expect("skill bench step");
        assert!(result.done);
        assert_eq!(result.reward, 1.0);
        let trajectory = env.export_trajectory().await;
        assert_eq!(trajectory.env_name, "skill_bench");
        assert_eq!(
            trajectory.steps[0].metadata["verified"],
            serde_json::json!(true)
        );
        let capture = trajectory.steps[0]
            .token_capture
            .as_ref()
            .expect("token capture capability marker");
        assert!(!capture.exact_tokens_supported);
        assert!(!capture.logprobs_supported);
    }

    #[tokio::test]
    async fn skill_bench_env_scores_agent_action_missing_required_substring_as_failure() {
        let mut env = SkillBenchEnv::new(vec![skill_bench_case("skill")]);
        env.reset().await;
        let result = env
            .step(AgentAction::UserMessage {
                content: "Do something unrelated".to_string(),
            })
            .await
            .expect("skill bench step");
        assert_eq!(result.reward, 0.0);
        assert_eq!(result.metadata["verified"], serde_json::json!(true));
        assert_eq!(result.metadata["required_ok"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn skill_bench_env_falls_back_to_heuristic_when_case_has_no_verifier() {
        let mut env = SkillBenchEnv::new(vec![SkillBenchCase {
            name: "unverifiable".to_string(),
            skill_content: "# Reference\nSome context.".to_string(),
            required_substrings: Vec::new(),
        }]);
        env.reset().await;
        let result = env
            .step(AgentAction::UserMessage {
                content: "A long substantive answer with no verifier to check it against."
                    .to_string(),
            })
            .await
            .expect("skill bench step");
        assert_eq!(result.metadata["verified"], serde_json::json!(false));
        // Unverifiable case: heuristic fallback, strictly below the gate.
        assert_eq!(result.reward, 0.55);
    }

    /// Build a small fixture of skill-bench cases whose pass/fail outcome
    /// depends on the episode index, so concurrent vs. sequential runs can
    /// be compared on non-trivial, mixed results.
    fn concurrency_fixture_cases() -> Vec<SkillBenchCase> {
        vec![
            SkillBenchCase {
                name: "case-a".to_string(),
                skill_content: "# Reference A".to_string(),
                required_substrings: vec!["alpha".to_string()],
            },
            SkillBenchCase {
                name: "case-b".to_string(),
                skill_content: "# Reference B".to_string(),
                required_substrings: vec!["beta".to_string()],
            },
        ]
    }

    fn episode_action_for_concurrency_fixture(episode: usize) -> Vec<AgentAction> {
        // Even episodes answer correctly for both cases; odd episodes get
        // the second case wrong, producing a mixed set of scores.
        let content = if episode.is_multiple_of(2) {
            "alpha then beta".to_string()
        } else {
            "alpha only".to_string()
        };
        vec![
            AgentAction::UserMessage {
                content: content.clone(),
            },
            AgentAction::UserMessage { content },
        ]
    }

    #[tokio::test]
    async fn concurrent_evaluate_matches_sequential_evaluate_per_case() {
        let n_episodes = 6;

        let sequential_root = tempfile::tempdir().expect("sequential artifact tempdir");
        let mut sequential_runner = EnvRunner::new(SkillBenchEnv::new(concurrency_fixture_cases()))
            .with_artifact_root(sequential_root.path())
            .with_concurrency(1);
        let sequential = sequential_runner
            .evaluate(n_episodes, episode_action_for_concurrency_fixture)
            .await
            .expect("sequential evaluate");

        let concurrent_root = tempfile::tempdir().expect("concurrent artifact tempdir");
        let mut concurrent_runner = EnvRunner::new(SkillBenchEnv::new(concurrency_fixture_cases()))
            .with_artifact_root(concurrent_root.path())
            .with_concurrency(4);
        let concurrent = concurrent_runner
            .evaluate(n_episodes, episode_action_for_concurrency_fixture)
            .await
            .expect("concurrent evaluate");

        assert_eq!(sequential.len(), n_episodes);
        assert_eq!(concurrent.len(), n_episodes);

        for episode in 0..n_episodes {
            let seq = &sequential[episode];
            let con = &concurrent[episode];
            assert_eq!(seq.score, con.score, "episode {episode} score mismatch");
            assert_eq!(
                seq.steps.len(),
                con.steps.len(),
                "episode {episode} step count mismatch"
            );
            for (seq_step, con_step) in seq.steps.iter().zip(con.steps.iter()) {
                assert_eq!(seq_step.reward, con_step.reward);
                assert_eq!(seq_step.response, con_step.response);
                assert_eq!(seq_step.metadata, con_step.metadata);
            }
        }

        // Sanity: the fixture actually produces mixed (non-uniform) scores,
        // so this test would catch order- or race-dependent regressions
        // rather than trivially passing on an all-zero or all-one fixture.
        let scores: Vec<f64> = sequential.iter().map(|t| t.score).collect();
        assert!(scores.contains(&1.0));
        assert!(scores.iter().any(|&s| s < 1.0));
    }

    #[derive(Default)]
    struct FixtureAgent {
        messages: std::sync::Mutex<Vec<IncomingMessage>>,
    }

    #[async_trait]
    impl AgentEnvAgent for FixtureAgent {
        async fn handle_env_message(
            &self,
            message: &IncomingMessage,
        ) -> anyhow::Result<Option<String>> {
            self.messages
                .lock()
                .expect("fixture messages lock")
                .push(message.clone());
            Ok(Some(
                "The embedded evaluation agent completed the requested runtime smoke successfully."
                    .to_string(),
            ))
        }

        async fn latest_token_capture_for_env_message(
            &self,
            _message: &IncomingMessage,
        ) -> Option<ProviderTokenCapture> {
            Some(ProviderTokenCapture {
                exact_tokens_supported: true,
                logprobs_supported: true,
                token_ids: vec![101, 102],
                tokens: vec!["runtime".to_string(), " smoke".to_string()],
                logprobs: vec![-0.1, -0.2],
                provider: Some("fixture-provider".to_string()),
                model: Some("fixture-model".to_string()),
            })
        }

        fn env_llm_token_capture_support(&self) -> TokenCaptureSupport {
            TokenCaptureSupport {
                exact_tokens_supported: true,
                logprobs_supported: true,
            }
        }

        fn env_llm_provider_name(&self) -> String {
            "fixture-provider".to_string()
        }
    }

    #[tokio::test]
    async fn agent_loop_runner_smoke_uses_restricted_isolated_sessions_and_real_capture_path() {
        let agent = Arc::new(FixtureAgent::default());
        let artifact_root = tempfile::tempdir().expect("agent-loop artifact tempdir");
        let env = AgentLoopEnv::new(agent.clone()).with_max_steps(1);
        let mut runner = EnvRunner::new(env)
            .with_artifact_root(artifact_root.path())
            .with_concurrency(2);

        let trajectories = runner
            .evaluate(2, |_| {
                vec![AgentAction::UserMessage {
                    content: "Run the embedded agent evaluation smoke.".to_string(),
                }]
            })
            .await
            .expect("agent-loop evaluation");

        assert_eq!(trajectories.len(), 2);
        assert_ne!(trajectories[0].episode_id, trajectories[1].episode_id);
        for trajectory in &trajectories {
            assert_eq!(trajectory.env_name, "agent_loop");
            assert_eq!(trajectory.steps.len(), 1);
            assert_eq!(trajectory.score, 0.55);
            let capture = trajectory.steps[0]
                .token_capture
                .as_ref()
                .expect("provider token capture");
            assert!(capture.exact_tokens_supported);
            assert!(capture.logprobs_supported);
            assert_eq!(capture.token_ids, vec![101, 102]);
            assert_eq!(capture.provider.as_deref(), Some("fixture-provider"));
        }

        let messages = agent.messages.lock().expect("fixture messages lock");
        assert_eq!(messages.len(), 2);
        for message in messages.iter() {
            assert_eq!(message.channel, "agent_env");
            assert_eq!(message.user_id, "agent-env");
            assert!(
                message
                    .thread_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with("agent-env:"))
            );
            assert_eq!(message.metadata["tool_profile"], "restricted");
            assert_eq!(message.metadata["agent_env"], true);
        }
    }
}
