# Loop Hardening Status

This status file captures the current implementation baseline for the
agentic and loop-adjacent execution surfaces. It is intentionally grounded in
the live checkout, not in older remediation notes.

## Baseline Verified Before This Slice

All focused loop gates passed before edits:

- `cargo test -p thinclaw-agent --lib`: 541 passed.
- `cargo test -p thinclaw --features desktop dispatcher --lib -- --test-threads=1`: 42 passed.
- `cargo test -p thinclaw --features desktop worker --lib -- --test-threads=1`: 58 passed.
- `cargo test -p thinclaw --features desktop routine_engine --lib -- --test-threads=1`: 12 passed.
- `cargo test -p thinclaw --features desktop repo_projects --lib -- --test-threads=1`: 54 passed.
- `cargo test -p thinclaw --features desktop outcomes --lib -- --test-threads=1`: 12 passed.

## Implemented In This Slice

- Added `thinclaw_agent::loop_control` with shared loop kinds, stop reasons,
  bounded iteration/retry/wall-time/idle budget helpers, and run summaries.
- Routed worker, dispatcher, and subagent iteration policy helpers through the
  shared stop-reason vocabulary without changing their existing boundaries.
- Made repo-project supervisor shutdown use its existing shutdown signal and
  drain for a bounded window before abort fallback.
- Added supervisor loop-stop summaries for graceful shutdown and wake-channel
  close paths.
- Added cooperative shutdown variants for routine cron and zombie-reaper
  loops, and wired the agent shutdown path to drain them before abort fallback.
- Added cooperative shutdown for the outcome service loop and wired it through
  the same bounded drain helper.
- Added cooperative shutdown for the self-repair loop, session pruning loop, and
  job-context pruning loop; agent shutdown now signals and drains them before
  abort fallback.
- Switched the routine notification forwarder from immediate abort to bounded
  drain after routine senders close.
- Added loop observability primitives to the root observer stack:
  `loop_starts`, `loop_stops`, `loop_iterations`, and `loop_retries` Prometheus
  families plus log/no-op support. Agent-owned background loops now emit
  structured start/stop metrics with stable loop-kind and stop-reason labels.
- Added a synchronous loop metric guard for hot paths with many early returns.
  Dispatcher, worker, and subagent loops now emit production loop start/stop
  metrics with structured terminal reasons for completion, interruption,
  cancellation, idle timeout, wall-time timeout, iteration-budget exhaustion,
  and fatal errors.
- Added deterministic lookup and authenticated replay for stored GitHub
  repo-project webhook deliveries. Replay re-matches the enrolled repo, emits a
  replay SSE event, and wakes the supervisor with the original delivery id.
- Extended GitHub webhook delivery persistence with optional raw payload and
  signature-header audit fields. New replay requests parse the stored raw
  payload when available, so replay follows the original delivery body instead
  of only reconstructing an envelope from derived metadata; older records still
  use the legacy metadata fallback.
- Added routine event dead-letter and replay primitives. Repeated dispatch
  failures are retried only up to the bounded attempt ceiling before the event is
  marked `dead_lettered`; `POST /api/routines/events/{id}/replay` resets a
  caller-owned failed/dead-lettered event to pending and drains the routine event
  queue when the engine is available.
- Exposed routine event `attempt_count` on the web activity response so
  operators can see retry-budget evidence next to failed and dead-lettered
  events.
- Added routine event queue fairness/backoff hardening. Pending event loads now
  prioritize fresh `attempt_count = 0` work before retries, then the root
  routine engine fairly interleaves each bounded batch by
  principal/actor/channel/conversation source so one noisy source cannot consume
  every slot in a batch.
- Added routine event and trigger queue loop metrics. Direct event processing,
  durable event queue drains, and durable trigger queue drains emit structured
  start/stop metrics with `completed`, `no_work`, `cancelled`, or `fatal_error`
  stop reasons, plus iteration counts and retry counts where applicable.
- Added durable outcome-candidate routing audit records. Successful outcome
  routes now stamp `outcome_candidate_route` into the candidate proposal and the
  evaluated contract metadata/evaluation details. Routing failures are recorded
  as terminal `quarantined` states requiring operator review instead of being
  debug-log-only failures after candidate insertion.
- Added durable self-repair attempt evidence. Tool repair attempts now write
  retry/success/manual-required JSON into the existing `tool_failures`
  `last_build_result` field, and a failed final attempt returns a terminal
  manual-required/quarantined state immediately instead of waiting for the next
  polling cycle.
- Added repo-project supervisor phase instrumentation. Restart recovery and
  each reconcile now emit structured phase start/stop traces with stable
  `repo_project_supervisor` loop labels, phase names, stop reasons, elapsed
  time, decision-class counts, and error counts.
- Added `scripts/audit-loop-inventory.sh`, a read-only inventory command for
  core loop files, spawn sites, interval/receiver loops, and loop-control use.
- Converted remaining runtime-owned maintenance loops from fire-and-forget to
  owned shutdown handles. Cost persistence, pricing sync, experiment
  controller, and experiment artifact reaper now receive shutdown signals and
  drain for a bounded window before abort fallback.
- Added cooperative shutdown for the pricing sync loop, including cancellation
  before DB-cache load, before initial network fetch, while sleeping between
  daily refreshes, and before scheduled refresh fetches.
- Retained hot-reload watcher objects through runtime shutdown instead of
  dropping their task handles after startup. WASM tool, skill, and WASM channel
  watchers now stop cooperatively and drain before bounded abort fallback.
- Converted config watcher, channel health monitor, WASM channel watcher, WASM
  tool watcher, and skill watcher stop paths from direct abort to
  signal-and-drain shutdown.
- Added cooperative shutdown for extension MCP background loops. The MCP
  health monitor and per-server roots-grant watchers now have shutdown senders,
  bounded drain helpers, and an agent shutdown call site.
- Converted external OAuth credential sync from drop-only abort to
  signal-first shutdown with an explicit async drain path retained by
  `async_main`.
- Converted the remaining `async_main` receiver forwarders to owned shutdown
  handles. Voice wake event forwarding, Docker job-event SSE forwarding, and
  subagent result injection forwarding now select on shutdown and drain through
  the same bounded runtime-task helper as maintenance loops.
- Converted the Unix SIGHUP hot-reload handler from process-lifetime
  fire-and-forget to an owned runtime task with cooperative shutdown.
- Promoted repo-project supervisor phase instrumentation into the observer
  metric surface. `LoopPhaseRun` now records loop kind, phase, stop reason,
  elapsed duration, decision count, and error count; Prometheus exports
  `loop_phase_runs`, `loop_phase_seconds`, `loop_phase_iterations`, and
  `loop_phase_retries` families with phase labels.
- Added deterministic repo-project supervisor loop regression coverage for
  wake-channel closure and explicit shutdown while preserving restart recovery
  execution before loop exit.
- Converted provider channel runtimes from detached/atomic-only loops to owned
  task handles with bounded drain paths: Apple Mail polling, iMessage polling,
  Discord gateway/reconnect/heartbeat handling, Gmail polling and token refresh,
  Signal SSE reconnect handling, BlueBubbles webhook listener, Nostr
  notification handling, and TUI forwarding/runtime tasks.
- Removed Discord's nested detached heartbeat task by folding heartbeat ticks
  into the owned gateway select loop, so a Discord channel has one retained
  gateway task and all backoff/reconnect sleeps are shutdown-interruptible.
- Added channel manager ownership for hot-added and restarted stream forwarders.
  Hot-remove, restart, and shutdown now drain those per-channel forwarding
  tasks instead of relying on dropped streams to end eventually.
- Split loop-adjacent helpers out of the two files that crossed the CI
  file-size guard after hardening: heartbeat routine helpers and repo-project
  config resolution moved out of `src/agent/agent_loop/mod.rs`, and runtime
  maintenance/watch shutdown helpers moved out of `src/async_main.rs`.

## Verified After This Slice

- `cargo test -p thinclaw-agent --lib --quiet`: 551 passed.
- `cargo test -p thinclaw --features desktop dispatcher --lib -- --test-threads=1`:
  42 passed.
- `cargo test -p thinclaw --features desktop worker --lib -- --test-threads=1`:
  58 passed.
- `cargo test -p thinclaw --features desktop routine_engine --lib -- --test-threads=1`:
  12 passed.
- `cargo test -p thinclaw --features desktop repo_projects --lib -- --test-threads=1`:
  59 passed.
- `cargo test -p thinclaw --features desktop outcomes --lib -- --test-threads=1`:
  12 passed.
- `cargo test -p thinclaw --features desktop --lib --quiet subagent_executor -- --test-threads=1`:
  15 passed.
- `cargo test -p thinclaw --features desktop,libsql github_webhook --lib -- --test-threads=1`:
  9 passed.
- `cargo test -p thinclaw --features desktop db_contract::repo_projects --test db_contract -- --test-threads=1`:
  1 passed.
- `cargo test -p thinclaw-db --features libsql webhook_delivery_dedup_and_project_run_round_trip --lib`:
  1 passed.
- `cargo test -p thinclaw-agent event_attempt_policy_dead_letters_only_at_positive_ceiling --lib`:
  1 passed.
- `cargo test -p thinclaw --features desktop db_contract::routines::pipeline_events::routine_event_dead_letter_and_replay_round_trip --test db_contract -- --test-threads=1`:
  1 passed.
- `cargo test -p thinclaw --features desktop db_contract::routines::pipeline_events::pending_routine_events_prioritize_fresh_events_before_retries --test db_contract -- --test-threads=1`:
  1 passed.
- `cargo test -p thinclaw-gateway --lib`: 328 passed.
- `cargo test -p thinclaw test_spawn_pruner_with_shutdown_exits_cleanly --lib`:
  1 passed.
- `cargo test -p thinclaw --features desktop agent::agent_loop --lib -- --test-threads=1`:
  14 passed.
- `cargo test -p thinclaw --features desktop --lib --quiet observability -- --test-threads=1`:
  27 passed.
- `cargo test -p thinclaw --features desktop db_contract::tool_failures::tool_failure_threshold_and_repair_contract --test db_contract -- --test-threads=1`:
  1 passed.
- `cargo test -p thinclaw --features desktop db_contract::conversations::outcome_contract_claims_are_idempotent_under_parallel_workers --test db_contract -- --test-threads=1`:
  1 passed.
- `cargo test -p thinclaw-config watcher --lib`: 7 passed.
- `cargo test -p thinclaw-channels health_monitor --lib`: 5 passed.
- `cargo test -p thinclaw-channels channel_watcher --lib`: 0 matched, module
  compiled.
- `cargo test -p thinclaw-tools watcher --lib`: 0 matched, module compiled.
- `cargo test -p thinclaw stop_drains_running_watcher_promptly --lib`: 1 passed.
- `cargo test -p thinclaw credential_sync --lib`: 7 passed.
- `cargo test -p thinclaw pricing_sync --lib`: 3 passed.
- `cargo test -p thinclaw --features desktop --lib observability -- --test-threads=1`:
  27 passed.
- `cargo test -p thinclaw --features desktop --lib repo_projects::supervisor -- --test-threads=1`:
  6 passed.
- `cargo test -p thinclaw-channels apple_mail --lib`: 10 passed.
- `cargo test -p thinclaw-channels imessage --lib`: 20 passed.
- `cargo test -p thinclaw-channels discord --lib`: 7 passed.
- `cargo test -p thinclaw-channels gmail --lib`: 35 passed.
- `cargo test -p thinclaw-channels signal --lib`: 81 passed.
- `cargo test -p thinclaw-channels bluebubbles --lib`: 17 passed.
- `cargo test -p thinclaw-channels manager --lib`: 8 passed.
- `cargo test -p thinclaw-channels --lib`: 347 passed.
- `cargo test -p thinclaw --features desktop tui_channel --lib`: 0 matched,
  module compiled.
- `cargo test -p thinclaw --features desktop db_contract --test db_contract -- --test-threads=1`:
  54 passed.
- `cargo check --locked --workspace --all-targets --all-features`: passed.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`:
  passed.
- `bash scripts/audit-loop-inventory.sh`: passed.
- `scripts/ci/check-file-sizes.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

## Remaining Lanes

- Provider-specific channel runtimes that own channel-lifetime loops have been
  converted to explicit task-handle shutdown or already used owned handles.
  Remaining spawn inventory entries are test scaffolding, one-shot request
  handlers, protocol sub-tasks, blocking adapters, or existing owned task
  handles rather than untracked provider event loops.
- Deterministic local regression coverage now covers the core loop policies,
  repo-project restart/replay/shutdown paths, routine dead-letter/replay/fairness
  paths, outcome idempotent claim behavior, watcher drains, provider channel
  shutdown compilation, and channel-manager stream forwarder drain behavior.
- Heavier external-service stress coverage remains intentionally outside the
  default local gate: live GitHub/Docker/LLM, Postgres process-level restart
  stress, and provider API end-to-end runs should stay behind explicit
  integration or nightly jobs.
- The CI-blocking file-size decomposition is complete for files touched by this
  loop-hardening slice. Any broader module reshaping beyond the guard is now a
  mechanical architecture follow-up rather than a behavioral blocker.
