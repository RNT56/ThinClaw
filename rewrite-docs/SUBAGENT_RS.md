# Sub-Agent Orchestration: Multi-Agent Spawning

The sub-agent system allows the **main agent** to spawn child agents with different models, system prompts, or tool restrictions to handle subtasks. This is how OpenClaw supports complex workflows: the main agent delegates specialized work to purpose-built child agents.

---

## 1. What OpenClaw Does Today

OpenClaw's sub-agent system (~25 files in `src/agents/subagent-*`) provides:

- **Spawn Modes:** `"run"` (fire-and-forget: child runs, delivers result, and dies) or `"session"` (child stays alive in a thread for follow-up conversation).
- **Depth Limits:** Configurable max spawn depth (default: 3) to prevent infinite sub-agent recursion.
- **Model Override:** The parent can request a specific model for the child (e.g., "use Claude for code review but GPT-4o-mini for summarization").
- **Result Announcement:** When a child completes, its result is delivered back to the parent's session as a user message. The parent receives it as if a user sent it.
- **Registry:** All running sub-agents are tracked in a persistent `SubagentRunRecord` store. Survives gateway restarts.
- **Lifecycle Events:** `started`, `completed`, `timeout`, `error`, `steer-restart` (redirect child to a new task).
- **Orphan Detection:** If a child's parent session disappears (e.g., session deleted, server restart), the orphan is cleaned up.
- **Thread Binding:** `"session"` mode sub-agents are bound to a specific chat thread, so subsequent messages in that thread go to the child (not the parent).

### Sub-Agent Spawn Parameters

```rust
pub struct SpawnSubagentParams {
    /// The task description for the child agent
    pub task: String,
    /// Human-readable label (shown in UI)
    pub label: Option<String>,
    /// Which agent config to use (default: same as parent)
    pub agent_id: Option<String>,
    /// Model override for the child (e.g., "gpt-4o-mini")
    pub model: Option<String>,
    /// Thinking/reasoning mode override
    pub thinking: Option<String>,
    /// Hard timeout for the run
    pub run_timeout_seconds: Option<u64>,
    /// Whether to bind the child to a chat thread
    pub thread: bool,
    /// "run" (fire-and-forget) or "session" (persistent thread)
    pub mode: SpawnMode,
    /// What to do after completion: "delete" or "keep" the session
    pub cleanup: Cleanup,
    /// Whether the child should send a completion message to the parent
    pub expects_completion_message: bool,
}

pub enum SpawnMode {
    /// Child runs the task once, announces result, and is cleaned up
    Run,
    /// Child stays alive in a thread for interactive follow-up
    Session,
}
```

---

## 2. Rust Implementation

### SubagentRegistry

```rust
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot};

pub struct SubagentRunRecord {
    pub run_id: String,
    pub child_session_key: String,
    pub requester_session_key: String,
    pub agent_id: String,
    pub label: Option<String>,
    pub model: Option<String>,
    pub mode: SpawnMode,
    pub started_at: u64,
    pub timeout_at: Option<u64>,
    pub status: RunStatus,
    pub outcome: Option<SubagentRunOutcome>,
    /// Channel/thread where the result should be delivered
    pub delivery_target: DeliveryTarget,
}

pub enum RunStatus {
    Running,
    Completed,
    TimedOut,
    Error,
    Cancelled,
}

pub struct SubagentRegistry {
    runs: HashMap<String, SubagentRunRecord>,
    /// Persistence path for run records
    store_path: PathBuf,
    /// Sweeper interval for timeout detection
    sweep_interval: tokio::time::Interval,
}

impl SubagentRegistry {
    /// Spawn a new child agent
    pub async fn spawn(
        &mut self,
        params: SpawnSubagentParams,
        parent_ctx: &AgentContext,
        agent_factory: &AgentFactory,
    ) -> Result<SpawnResult> {
        // Check depth limit
        let current_depth = self.depth_of(parent_ctx.session_key());
        if current_depth >= self.max_depth {
            return Err(SubagentError::DepthLimitExceeded {
                current: current_depth,
                max: self.max_depth,
            });
        }

        let run_id = uuid::Uuid::new_v4().to_string();
        let child_session_key = format!(
            "{}:subagent:{}",
            parent_ctx.session_key(),
            params.label.as_deref().unwrap_or(&run_id[..8])
        );

        // Create child agent session with optional model override
        let child_session = agent_factory.create_session(CreateSessionOpts {
            session_key: child_session_key.clone(),
            model: params.model.clone(),
            system_prompt_prefix: Some(format!(
                "You are a sub-agent. Your task: {}\n\
                 Report your results clearly when done.",
                params.task
            )),
            timeout: params.run_timeout_seconds.map(Duration::from_secs),
        }).await?;

        // Register the run
        let record = SubagentRunRecord {
            run_id: run_id.clone(),
            child_session_key: child_session_key.clone(),
            requester_session_key: parent_ctx.session_key().to_string(),
            mode: params.mode,
            started_at: now_ms(),
            timeout_at: params.run_timeout_seconds.map(|s| now_ms() + s * 1000),
            status: RunStatus::Running,
            ..
        };
        self.runs.insert(run_id.clone(), record);
        self.persist()?;

        // Spawn the task in a background tokio task
        let registry = self.clone_handle();
        tokio::spawn(async move {
            let result = child_session.chat(&params.task).await;
            registry.complete_run(&run_id, result).await;
        });

        Ok(SpawnResult {
            status: "accepted",
            child_session_key,
            run_id,
            mode: params.mode,
        })
    }

    /// Called when a child completes its task
    async fn complete_run(&mut self, run_id: &str, result: Result<String>) {
        let record = self.runs.get_mut(run_id).unwrap();
        record.status = RunStatus::Completed;

        // Announce result back to the parent session
        let announcement = match result {
            Ok(text) => format!(
                "Sub-agent '{}' completed:\n{}",
                record.label.as_deref().unwrap_or("unnamed"),
                text
            ),
            Err(e) => format!("Sub-agent failed: {}", e),
        };

        // Deliver as a user message to the parent's session
        self.announce_to_parent(record, &announcement).await;

        // Cleanup
        if record.mode == SpawnMode::Run {
            self.cleanup_session(&record.child_session_key).await;
            self.runs.remove(run_id);
        }

        self.persist().unwrap();
    }

    /// Periodic sweep for timed-out runs
    async fn sweep(&mut self) {
        let now = now_ms();
        let timed_out: Vec<String> = self.runs.iter()
            .filter(|(_, r)| {
                r.status == RunStatus::Running
                    && r.timeout_at.map_or(false, |t| now > t)
            })
            .map(|(id, _)| id.clone())
            .collect();

        for run_id in timed_out {
            self.complete_run(&run_id, Err(SubagentError::Timeout.into())).await;
        }
    }

    /// Calculate the nesting depth of a session
    fn depth_of(&self, session_key: &str) -> usize {
        self.runs.values()
            .filter(|r| r.child_session_key == session_key)
            .map(|r| 1 + self.depth_of(&r.requester_session_key))
            .max()
            .unwrap_or(0)
    }
}
```

---

## 3. The `spawn_subagent` Tool

The LLM triggers sub-agent creation via a tool call:

```json
{
  "tool": "spawn_subagent",
  "params": {
    "task": "Research the latest pricing for AWS S3 and summarize",
    "label": "s3-research",
    "model": "gpt-4o-mini",
    "mode": "run",
    "run_timeout_seconds": 120,
    "cleanup": "delete"
  }
}
```

The Orchestrator's tool handler calls `SubagentRegistry::spawn()`.

---

## 4. Thread-Bound Sub-Agents (Session Mode)

In `"session"` mode, the child agent is bound to a specific chat thread:

```
User (in main thread): "Create a code review agent for PR #42"
  → Main Agent spawns sub-agent in "session" mode
  → Sub-agent opens a new thread: "PR #42 Review"

User (in PR #42 thread): "What about the error handling?"
  → This message goes to the SUB-AGENT (not the main agent)
  → The thread binding routes it

User (in main thread): "What's the weather?"
  → This message goes to the MAIN AGENT (normal routing)
```

Thread binding is managed via a `thread_id → session_key` mapping.

---

## 5. Security & Limits

| Rule | Value |
|---|---|
| Max spawn depth | 3 (configurable) |
| Default timeout | Uses agent-level timeout (typically 120s) |
| Max concurrent sub-agents | 10 (configurable) |
| Tool restrictions | Sub-agents inherit parent's tool policy unless overridden |
| Model override | Only models the user has configured API keys for |
| Sub-agent spawning sub-agents | Allowed within depth limit |

---

## 6. Config

```toml
[agent.subagents]
max_depth = 3
max_concurrent = 10
default_timeout_seconds = 120
# Whether sub-agents can spawn their own sub-agents
allow_nested = true
```
