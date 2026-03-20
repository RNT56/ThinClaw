> ⛔ **ARCHIVED** — This is a historical migration guide from the OpenClaw→IronClaw rewrite (early 2026). It does NOT reflect the current codebase. See [`../CLAUDE.md`](../CLAUDE.md) for current documentation.

---

# Cron & Scheduled Tasks: Proactive Agent Behaviors

The cron system is what makes the agent **proactive** — it can run scheduled tasks, send daily briefings, check for emails, and wake itself up without user prompts. OpenClaw's cron system (71 files) is far more than a simple timer.

---

## 1. What OpenClaw Does Today

OpenClaw's `CronService` manages arbitrary scheduled jobs with:

- **Schedule Types:** One-shot `at` (run once at a specific time), repeating `every` (interval in ms), or standard `cron` expressions with timezone support and stagger windows.
- **Session Isolation:** Each cron job can run in the `main` session (shared history) or an `isolated` session (fresh context per run).
- **Delivery Targets:** Cron results can be delivered to a specific chat channel (e.g., "run this daily and DM me on Telegram"), announced in the current thread, or sent to a webhook URL.
- **Persistence:** Jobs survive restarts. The `CronStoreFile` (JSON) is saved to disk and reloaded on boot.
- **Catchup Logic:** If the server was down during a scheduled run, it catches up missed jobs.
- **Consecutive Error Tracking:** Auto-disables jobs after a threshold of consecutive failures.
- **Wake Modes:** `"now"` (run immediately) vs `"next-heartbeat"` (defer to next heartbeat cycle).

### CronJob Data Model

```rust
pub struct CronJob {
    pub id: String,
    pub agent_id: Option<String>,
    pub session_key: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub delete_after_run: bool,             // One-shot behavior
    pub schedule: CronSchedule,
    pub session_target: SessionTarget,      // "main" | "isolated"
    pub wake_mode: WakeMode,                // "now" | "next-heartbeat"
    pub payload: CronPayload,
    pub delivery: Option<CronDelivery>,
    pub state: CronJobState,
}

pub enum CronSchedule {
    /// Run once at a specific ISO 8601 timestamp
    At { at: String },
    /// Run every N milliseconds (with optional anchor)
    Every { every_ms: u64, anchor_ms: Option<u64> },
    /// Standard cron expression with timezone and stagger
    Cron { expr: String, tz: Option<String>, stagger_ms: Option<u64> },
}

pub enum CronPayload {
    /// A system event text injected as context
    SystemEvent { text: String },
    /// A full agent turn (the LLM processes this message and responds)
    AgentTurn {
        message: String,
        model: Option<String>,
        thinking: Option<String>,
        timeout_seconds: Option<u64>,
        deliver: Option<bool>,
        channel: Option<String>,
        to: Option<String>,
    },
}

pub struct CronDelivery {
    pub mode: DeliveryMode,           // "none" | "announce" | "webhook"
    pub channel: Option<String>,      // Target channel (e.g., "telegram")
    pub to: Option<String>,           // Target user/chat ID
    pub best_effort: bool,            // Don't fail job if delivery fails
}

pub struct CronJobState {
    pub next_run_at_ms: Option<u64>,
    pub last_run_at_ms: Option<u64>,
    pub last_run_status: Option<RunStatus>,
    pub last_error: Option<String>,
    pub consecutive_errors: u32,
    pub last_delivery_status: Option<DeliveryStatus>,
}
```

---

## 2. Rust Implementation

### CronService Struct

```rust
use tokio_cron_scheduler::{Job, JobScheduler};
use tokio::sync::mpsc;

pub struct CronService {
    scheduler: JobScheduler,
    store: CronStore,
    event_tx: mpsc::Sender<CronEvent>,
}

impl CronService {
    pub async fn new(store_path: PathBuf) -> Result<Self> {
        let scheduler = JobScheduler::new().await?;
        let store = CronStore::load(store_path)?;
        let (event_tx, _) = mpsc::channel(64);
        Ok(Self { scheduler, store, event_tx })
    }

    pub async fn start(&mut self) -> Result<()> {
        // Load all enabled jobs from disk
        for job in self.store.jobs.iter().filter(|j| j.enabled) {
            self.schedule_job(job).await?;
        }
        self.scheduler.start().await?;

        // Run catchup for any jobs that were missed while offline
        self.catchup_missed_jobs().await?;
        Ok(())
    }

    async fn catchup_missed_jobs(&mut self) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis() as u64;
        for job in self.store.jobs.iter_mut() {
            if let Some(next_at) = job.state.next_run_at_ms {
                if next_at < now && job.enabled {
                    tracing::info!(
                        job_id = %job.id, name = %job.name,
                        "Catching up missed cron job"
                    );
                    self.execute_job(&job.id).await?;
                }
            }
        }
        Ok(())
    }

    async fn execute_job(&mut self, job_id: &str) -> Result<CronRunOutcome> {
        let job = self.store.get_mut(job_id)?;
        job.state.running_at_ms = Some(now_ms());

        let outcome = match &job.payload {
            CronPayload::SystemEvent { text } => {
                // Inject text as a system prompt addition for the next heartbeat
                CronRunOutcome { status: RunStatus::Ok, summary: Some(text.clone()), .. }
            },
            CronPayload::AgentTurn { message, model, timeout_seconds, .. } => {
                // Run a full agent session with this message
                let session = if job.session_target == SessionTarget::Isolated {
                    AgentSession::new_isolated(model.as_deref())
                } else {
                    AgentSession::main()
                };
                let result = session.chat(message).await;
                // ... handle result, update state
                result.into()
            },
        };

        // Deliver result to target channel if configured
        if let Some(delivery) = &job.delivery {
            self.deliver_result(&outcome, delivery).await?;
        }

        // Update state
        job.state.last_run_at_ms = Some(now_ms());
        job.state.last_run_status = Some(outcome.status.clone());
        if outcome.status == RunStatus::Error {
            job.state.consecutive_errors += 1;
        } else {
            job.state.consecutive_errors = 0;
        }

        // Auto-disable after too many failures
        if job.state.consecutive_errors >= 5 {
            job.enabled = false;
            tracing::warn!(job_id, "Auto-disabled cron job after 5 consecutive failures");
        }

        // Delete one-shot jobs
        if job.delete_after_run {
            self.store.remove(job_id);
        }

        self.store.persist()?;
        Ok(outcome)
    }
}
```

---

## 3. The Heartbeat (Special Cron Behavior)

The heartbeat from `INTERNAL_SYSTEMS_RS.md` is implemented **as a built-in cron job** with `wake_mode: "next-heartbeat"` and a 15-minute interval. It is the only cron job that runs in the `main` session by default, allowing it to process accumulated context and memories.

```toml
# Built-in heartbeat job (auto-created, user can adjust interval)
[cron.heartbeat]
enabled = true
schedule = { kind = "every", every_ms = 900000 }  # 15 minutes
session_target = "main"
wake_mode = "next-heartbeat"
payload = { kind = "systemEvent", text = "Heartbeat: review recent context and take proactive actions." }
```

---

## 4. Stagger / Thundering Herd Prevention

When multiple cron jobs share the same schedule (e.g., `0 9 * * *` daily at 9 AM), they should not all fire simultaneously. The `stagger_ms` field adds a deterministic random delay per job (seeded by `job.id`) to spread the load:

```rust
fn compute_stagger(job_id: &str, stagger_ms: u64) -> u64 {
    let hash = crc32fast::hash(job_id.as_bytes());
    (hash as u64) % stagger_ms
}
```

---

## 5. User Management via Slash Commands

Users can manage cron jobs via chat commands (see `CHAT_COMMANDS_RS.md`):

- **`/cron list`** — show all jobs with status, next run time, last result
- **`/cron add "Daily Briefing" every 24h deliver telegram`** — create a new job
- **`/cron run <id>`** — force-run a job immediately
- **`/cron disable <id>`** — pause a job
- **`/cron remove <id>`** — delete a job

---

## 6. Agent-Created Reminders

The LLM itself can create cron jobs via a `set_reminder` tool:

```json
{
  "tool": "set_reminder",
  "params": {
    "name": "Check PR status",
    "schedule": { "kind": "at", "at": "2026-02-28T10:00:00Z" },
    "message": "Check the status of PR #42 on GitHub and report back.",
    "deliver": true,
    "channel": "telegram",
    "deleteAfterRun": true
  }
}
```

This creates a one-shot `CronJob` that runs once, delivers the result to Telegram, and deletes itself.

---

## 7. Persistence Format

```toml
# ~/.thinclaw/cron.json
{
  "version": 1,
  "jobs": [
    {
      "id": "heartbeat-main",
      "name": "Heartbeat",
      "enabled": true,
      "schedule": { "kind": "every", "everyMs": 900000 },
      ...
    }
  ]
}
```

The cron store is a simple JSON file. On every mutation (add, update, remove, state change), it is atomically rewritten to disk.
