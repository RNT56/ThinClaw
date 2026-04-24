//! Phase 1 agent environment framework for evals and SFT collection.
//!
//! This module wraps ThinClaw's normal agent loop instead of creating a
//! separate simulator. Every step can therefore reuse canonical trajectory and
//! run-artifact logging while exposing a small environment API for research
//! campaigns and local benchmarks.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use axum::{Json, Router, extract::State, routing::post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::{Agent, AgentRunArtifact, AgentRunArtifactLogger, AgentRunStatus};
use crate::channels::IncomingMessage;

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
    #[serde(default)]
    pub metadata: serde_json::Value,
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
}
