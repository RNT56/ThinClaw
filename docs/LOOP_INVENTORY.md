# Core Agentic Loop Inventory

This inventory describes the live core agentic and loop-adjacent execution
surfaces. It deliberately does not classify unrelated UI request handlers as
agent loops. Run `bash scripts/audit-loop-inventory.sh` for the current source
and spawn-site inventory.

| Loop | Production entrypoint | Bound and fairness policy | Shutdown ownership | Durable recovery / quarantine | Terminal telemetry |
| --- | --- | --- | --- | --- | --- |
| Conversation worker pool | `Agent::run` in `src/agent/agent_loop/mod.rs` | Per-conversation ordering, bounded queues, global turn semaphore, idle timeout | Shared `JoinSet`; graceful drain then abort-and-join | Conversation state is persisted by the normal thread path | `conversation_worker` start/stop reason |
| Dispatcher | `Dispatcher::run_agentic_loop` in `src/agent/dispatcher/loop.rs` | Hard iteration cap and whole-job wall-time budget | Turn cancellation drops provider/tool futures | Thread checkpoints and normal conversation persistence | Guarded start/stop reason, iterations, errors |
| Worker | `Worker::run` in `src/agent/worker.rs` | One iteration budget covers planned and direct actions; inactivity and hard wall timers | Owned scheduler task; cancellation and channel closure terminate execution | Job context and scheduler recovery | Guarded start/stop reason, iterations, retries |
| Subagent | `SubagentExecutor` in `src/agent/subagent_executor/mod.rs` | Iteration, idle, and execution bounds; status delivery has its own short timeout; learning and routine finalization run concurrently inside a separate finalization bound | Registry-owned join handles with cancellation | Ledger/status writes are bounded and replay-safe | Structured subagent start/stop events and metrics |
| Routine cron | `spawn_cron_ticker_with_shutdown` in `src/agent/routine_engine.rs` | Bounded trigger batches and missed-tick skipping | Agent-owned shutdown sender and join handle | Durable trigger queue with retry visibility | `routine_cron` stop reason |
| Routine trigger queue | `drain_pending_trigger_queue` in `src/agent/routine_engine.rs` | Fresh-before-retry ordering, four batches per drain, exponential retry backoff, global/project capacity checks; PostgreSQL claims use row locking with `SKIP LOCKED` | Runs inside the owned routine engine task | Exclusive lease, `next_attempt_at`, bounded retries, terminal `failed` state with merged diagnostics | Phase loop metrics with accurate iterations/retries and stop reason |
| Routine event queue | `drain_pending_event_queue` in `src/agent/routine_engine.rs` | Fresh-before-retry ordering and round-robin source interleave; four batches per drain; dispatch errors enter the bounded failure policy | Runs inside owned cron/direct-dispatch paths | Idempotency key, lease, dead letter, authenticated replay | Queue start/stop metrics, fatal-error reporting, and durable diagnostics |
| Routine notifications | Forwarder created in `Agent::start_background_tasks` | Bounded channel; closes after routine senders | Agent-owned handle drains after engine shutdown | Notification result remains in routine run/event state | `routine_notification_forwarder` stop reason |
| Routine zombie reaper | `spawn_zombie_reaper_with_shutdown` | Fixed cadence; only stale active runs are handled | Agent-owned sender and join handle | Persisted run state is authoritative | `routine_zombie_reaper` stop reason |
| Outcome evaluator | `spawn_outcome_service_with_shutdown` in `src/agent/outcomes.rs` | Per-user ownership guard, due-work cap, contract wall timeout, bounded retry backoff | Shutdown races scheduler planning and each user evaluation | Atomic DB lease, stable effect IDs, stale-lease replay, retry quarantine | `outcome_service` start/stop plus contract diagnostics |
| Self repair | Task created in `Agent::start_background_tasks` | Poll cadence and bounded job/tool repair attempts | Agent-owned sender and join handle with abort fallback | Real scheduler resubmission; abandoned jobs and tool quarantine require new evidence/reopen | `self_repair` stop reason and operator status events |
| Repo-project supervisor | `run_project_supervisor_loop` in `src/repo_projects/supervisor.rs` | Nonblocking coalesced wakes, project isolation/deadline, concurrency ceilings | Recovery and reconcile futures are shutdown-cancellable | Restart recovery, durable project events, webhook replay, terminal blocked/error state | Loop and recovery/reconcile phase metrics |
| GitHub transport | `GitHubApiClient` in `src/repo_projects/github.rs` | Connect/request timeouts; bounded jittered retry for idempotent methods; rate-limit delay and shared circuit breaker | Request futures cancel with supervisor reconciliation | Live state is reduced through the same pipeline after webhook replay | Retry/circuit traces plus supervisor phase outcome |
| Sandbox job monitor | `spawn_job_monitor` in `src/agent/job_monitor.rs` | One filtered broadcast receiver per fire-and-forget job; each injection is time-bounded and a backpressured progress update is dropped so the terminal result can still be observed | Single task exits on terminal result, broadcast close, or injection-channel close | Sandbox job/event persistence remains authoritative | Start, lag, completion, and close traces |
| Maintenance loops | Agent pruning, hygiene, pricing, watcher, and runtime maintenance owners | Fixed cadence and bounded work batches where applicable | Explicit shutdown senders/handles or abort-and-join for interval-only tasks | Each service uses its own persisted source of truth | Stable maintenance loop stop labels where agent-owned |

## Regression Gates

- Core policy and behavior: `cargo test -p thinclaw-agent --lib`.
- Desktop loop suite: `cargo test -p thinclaw --features desktop dispatcher worker routine_engine repo_projects outcomes --lib -- --test-threads=1`.
- Persistence contracts: `cargo test -p thinclaw --features desktop db_contract --test db_contract -- --test-threads=1`.
- GitHub fake-server scenarios cover merge flow, retry, rate limiting, timeout,
  circuit opening, replay, and restart recovery.
- `benches/loop_control.rs` records budget-accounting and routine fairness
  throughput; the routine fairness unit suite includes a generous debug-build
  timing smoke.
- `scripts/ci/check-file-sizes.sh`, route/status/WIT contract tests, workspace
  check/clippy, and `cargo deny check` enforce architecture and build drift.

## Deliberate External Gates

No concurrent system can be truthfully guaranteed flawless. Live GitHub,
Docker, provider, and LLM behavior depends on credentials, service availability,
rate limits, and host resources. ThinClaw bounds and surfaces those failures;
real-service stress and Docker scenarios remain explicit or nightly integration
gates instead of making the default local suite nondeterministic.
