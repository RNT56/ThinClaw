# WS-06 — Repo-Project Supervisor Completion

> **✅ STATUS: DONE. Landed in commit `dd7b7cdb` (repo-project supervisor + routines/heartbeat), merged to `main` via the audit-hardening stack (`1fb29984`, HEAD `bda7a61f`).**
> This plan is complete; do not execute it. It is retained as an implementation record. All five gaps closed: the `RepoTaskPlanner` port + `SubagentRepoTaskPlanner` decompose `NeedsPlanning` projects (`src/repo_projects/planner.rs`, `subagent_planner.rs`, wired via `with_planner` behind the `REPO_PROJECTS_AUTOPLAN` gate); concurrency is enforced via `with_limits` (`src/repo_projects/supervisor.rs`); merge retry is bounded by `max_merge_attempts` (default 3, `src/repo_projects/pipeline.rs:61,77,522`) with per-SHA reset; `installation_id` is persisted at create/enroll and backfilled from webhooks (`match_and_backfill_repo`, `src/channels/web/handlers/github_webhook.rs:326`); and the WebUI SSE consumer is present. The "Current State (verified)" section below describes the *pre-remediation* state.

> **Status:** Done (landed) · **Priority:** P2 · **Risk:** medium · **Effort:** L
> **Depends on:** none · **Blocks:** none (coordinates with WS-12 doc inventory; coordinates the SSE pattern with the gateway but owns no gateway files)
> **Owns (symbols/files):**
> - `src/repo_projects/supervisor.rs` (`RepoSupervisorDecision::NeedsPlanning` handling, concurrency gating in `dispatch_next_task`, restart recovery)
> - `src/repo_projects/pipeline.rs` (`perform_merge`, the bounded merge-attempt counter)
> - `src/repo_projects/executor.rs` and a **new** `src/repo_projects/planner.rs` (autonomous planner)
> - `src/api/repo_projects.rs` repo-creation/enroll `installation_id` persistence (lines 305–306, 1000)
> - `src/channels/web/handlers/github_webhook.rs` (`find_project_id_for_repo` → persist webhook `installation_id`)
> - the **repo-projects SSE consumer block** added to `crates/thinclaw-gateway/src/web/static/app.js` (a new `eventSource.addEventListener` group + a `queueRepoProjectsRefresh` helper — new code, does not mutate existing gateway event handlers)
>
> WS-06 does **not** mutate: gateway routing/`server.rs`, `SseEvent` enum in `crates/thinclaw-gateway/src/web/types.rs` (already has the four repo variants — read-only here), the `Database` trait, or doc inventories (WS-12 owns `CRATE_OWNERSHIP.md`/`FEATURE_PARITY.md`/README inventory rows).

## Vision & Goal

The repo-project supervisor is ThinClaw's autonomous "ship code against real GitHub repos" loop: enroll a repo, decompose a goal into tasks, dispatch each task to a sandbox coding worker, drive PR → CI → review → guarded merge, and recover after a restart. It is recent (PRs #16–18), genuinely wired end-to-end, and roughly 78% complete. This workstream closes the five gaps that keep it from being trustworthy when left running unattended: it gives `Planning` projects an actual planner instead of a dead-end signal, makes the inert concurrency knobs real, bounds the merge-retry loop so a stuck PR cannot hammer GitHub forever, persists the GitHub installation id so App auth survives across repos, and lets the WebUI react live to the events the backend already emits.

## Scope

**In scope:**
- Decide and implement the fate of `RepoSupervisorDecision::NeedsPlanning` (build an autonomous planner subagent that decomposes a project goal into `Queued` tasks, or downgrade `NeedsPlanning` to an explicit "awaiting human plan" status surfaced through events/SSE).
- Enforce concurrency: make `ProjectPolicy.max_parallel_tasks` (and the config knobs `max_concurrent_tasks_per_project` / `max_concurrent_projects`) actually limit dispatch in `supervisor.rs::dispatch_next_task` and the project-selection loop.
- Add a bounded merge-attempt counter in `pipeline.rs::perform_merge` so a repeatedly-approved-but-unmerged PR escalates to a human instead of retrying every watchdog tick forever.
- Persist a per-repo `installation_id`: at repo creation/enroll when discoverable, and when a GitHub webhook arrives carrying one for a matched repo.
- Add a WebUI SSE consumer for the four repo-project events (`repo_project_updated`, `repo_task_updated`, `repo_project_event`, `repo_merge_gate_updated`) that already flow over `/api/chat/events`, debouncing a dashboard refresh when the Repo Projects tab is active.

**Out of scope (and which WS owns it):**
- Doc inventory rows for the subsystem (`CRATE_OWNERSHIP.md` missing `thinclaw-repo-projects`, README/FEATURE_PARITY) — **WS-12 (doc inventory)**.
- The `SseEvent` enum definition and gateway route wiring — already complete in `crates/thinclaw-gateway` (read-only dependency here; do not add new variants).
- The quarantined flaky `repo_project_docker_e2e` worktree/Docker race — tracked separately under the experiments/CI workstream.
- Sandbox/secret-confinement hardening (`src/sandbox/proxy/*`) — security workstream.

## Current State (verified)

> **Historical (pre-remediation) snapshot.** The "Half-wired / stub", "Drifted / unbounded", and "Persistence gap" items below were all closed by the landed WS-06 work. Kept for context. Path anchors here predate the WS-10 decompositions: `src/agent/agent_loop.rs` is now `src/agent/agent_loop/mod.rs` and `src/api/repo_projects.rs` is now the `src/api/repo_projects/` directory (`mod.rs`); `src/repo_projects/planner.rs` now exists.

**Wired (working):**
- The bounded reconcile loop is real and spawned when `repo_projects.enabled` is true: `run_project_supervisor_loop` (`src/repo_projects/supervisor.rs:598`), wired from the agent loop at `src/agent/agent_loop.rs:967-1069` with executor, pipeline, SSE, and a 128-deep wake channel. Watchdog + webhook + manual wakes all route through `reconcile_once`.
- The GitHub pipeline drives `WaitingCi → WaitingReview → merge` with a guarded merge gate, CI classification, dedup of audit/SSE noise, and a bounded **CI-repair** counter (`pipeline.rs:277`, `max_ci_repair_attempts` default 3). Empty-CI grace counter exists (`pipeline.rs:233`).
- Restart recovery (`supervisor.rs:221-284`) re-syncs sandbox jobs and blocks orphaned `Running` tasks.
- The four repo SSE variants exist and are emitted: `SseEvent::RepoProjectUpdated/RepoTaskUpdated/RepoProjectEvent/RepoMergeGateUpdated` (`crates/thinclaw-gateway/src/web/types.rs:1437-1463`, label map `:1536-1540`), broadcast from `supervisor.rs:520`, `pipeline.rs:440/1211/1220`, and `github_webhook.rs:306`.
- `RepoProjectExecutor::dispatch_task`, `redispatch_repair_task`, `dispatch_review_task` are wired and persist worker-run records.

**Half-wired / stub:**
- `RepoSupervisorDecision::NeedsPlanning` (`supervisor.rs:38`) is **produced** at `supervisor.rs:151` (Draft/Planning with empty tasks) and `:205` (Draft fallthrough) but **only logged** — `reconcile_once` (`supervisor.rs:642-646`) prints `tracing::info!(?decision, ...)` and discards it. No subagent, no event, no SSE. The `repo_project_plan` tool (`src/tools/builtin/repo_projects.rs:140-173`) and API `plan_project` (`src/api/repo_projects.rs:375-389`) only flip the project to `Planning` ("for supervisor decomposition") — but the supervisor never decomposes. A project parked in `Planning` with no tasks sits forever unless a human calls `enqueue_task` (`api/repo_projects.rs:505`).
- Concurrency limits are **inert**. `RepoProjectsConfig.max_concurrent_projects` / `max_concurrent_tasks_per_project` (`crates/thinclaw-config/src/repo_projects.rs:14-21`, validated `>0` at `:87-101`) are never read anywhere in `src/repo_projects/` or the agent-loop wiring (grep confirms zero call sites). `ProjectPolicy.max_parallel_tasks` (`crates/thinclaw-repo-projects/src/lib.rs:209-210`, default 1) is only ever set as a literal in tests/merge_gate construction (`merge_gate.rs:216`) — never enforced. `dispatch_next_task` (`supervisor.rs:362-441`) dispatches exactly **one** `Queued`/`Ready` task per tick with no count of currently-`Running` tasks, so the next watchdog tick can dispatch another, and `max_parallel_tasks > 1` does nothing while `= 1` is only accidentally honored by the one-per-tick cadence.

**Drifted / unbounded (bug):**
- Unbounded approved-merge retry. In `pipeline.rs::perform_merge` the two non-success arms — `Ok(response)` where GitHub accepted but did not merge (`pipeline.rs:532`) and `Err(error)` (`pipeline.rs:545`) — both record a `MergeDenied` event and return `Ok(PipelineOutcome::MergeGateRecorded { approved: true })`, leaving the task in `WaitingReview`. On the next watchdog tick `advance_waiting_review` re-evaluates the (still-approved) gate and calls `perform_merge` again. There is no per-task merge-attempt counter analogous to the CI-repair counter, so a structurally-unmergeable-but-gate-approved PR (e.g. protected branch, required status mismatch, repeated 405) is retried on every tick indefinitely, spamming GitHub and the event log.

**Persistence gap:**
- `installation_id` is never persisted on a repo row. `RepoProjectRepo.installation_id` is set to `None` at creation (`api/repo_projects.rs:306`) and enroll (`api/repo_projects.rs:1000`). The client provider falls back to the global `default_installation_id` (`github_provider.rs:128-130`), so multi-installation setups (App installed on several orgs) cannot pin the correct installation per repo. The webhook envelope **carries** `installation_id` (`github_webhook.rs:143`, `github.rs:1833-1850`) and `find_project_id_for_repo` (`github_webhook.rs:212-242`) already locates the matching repo row, but never writes the id back. The discovery client (`list_installation_repositories`, `github.rs:387`) is wired for the connector repo-picker but the resolved installation is not stored on enroll.

## Decision Points

1. **`NeedsPlanning`: build an autonomous planner subagent vs. downgrade to a human-facing status signal.**
   - *Option A — Build a planner.* Add `src/repo_projects/planner.rs` that, on a `NeedsPlanning` project, runs a one-shot planning agent (reuse `SubagentExecutor::spawn`, `src/agent/subagent_executor.rs:298`) to decompose `project.description` + enrolled-repo context into N task drafts, then persists each as a `Queued` `RepoProjectTask` via `db.upsert_repo_project_task` (the exact shape `enqueue_task` builds, `api/repo_projects.rs:539-572`), and transitions Draft→Planning→Active. Realizes the vision (truly autonomous), but pulls an LLM/subagent dependency into the supervisor store, which today is dependency-light; must be injected behind a port (a `RepoTaskPlanner` trait) so the supervisor crate-direction stays clean and tests can use a deterministic fake.
   - *Option B — Downgrade to a status signal.* Keep `NeedsPlanning` but make it actionable: when produced, transition the project to `AwaitingHuman` (reusing the existing `validate_project_state_transition`), append a `ProjectStateChanged` event ("project needs a plan; add tasks to proceed"), and emit `SseEvent::RepoProjectUpdated`. Cheap, honest, no LLM coupling — but leaves "autonomous" planning unrealized.
   - **Recommendation: Option A, built behind a `RepoTaskPlanner` port with Option B as the no-planner default.** The directive is to realize the vision; the `repo_project_plan` tool and `plan_project` API already promise "supervisor decomposition," so a planner closes a promise the UI/tool surface already makes. Inject `Option<Arc<dyn RepoTaskPlanner>>` into `DatabaseRepoSupervisorStore` (mirroring the existing `with_executor`/`with_pipeline`/`with_sse` builders, `supervisor.rs:82-97`). When no planner is wired (e.g. no LLM stack), fall back to Option B so a `Planning` project is never a silent dead end. This keeps the supervisor store testable and the LLM dependency optional, matching how the pipeline is optional today.

2. **Concurrency: enforce the per-project `ProjectPolicy.max_parallel_tasks` (persisted) vs. the global config `max_concurrent_tasks_per_project` (env).** There are two overlapping knobs. The policy field is per-project and persisted on the `RepoProject`; the config field is process-global.
   - **Recommendation: enforce `ProjectPolicy.max_parallel_tasks` as the effective per-project cap, clamped by the config `max_concurrent_tasks_per_project` ceiling.** In `dispatch_next_task`, count tasks already in `Running` (and arguably `WaitingCi`/`WaitingReview` if they hold a sandbox slot — choose `Running` only, since CI/review are GitHub-bound, not sandbox-bound) and dispatch up to `min(policy.max_parallel_tasks, config.max_concurrent_tasks_per_project) - running_count` tasks per tick. Enforce `max_concurrent_projects` in the project-selection loop (`supervisor.rs:107-131`) by limiting how many `Active`/`Planning` projects advance dispatch per reconcile. This makes the persisted, per-project knob authoritative (operators expect a project's own policy to win) while the env ceiling caps total host load. Requires threading the resolved `RepoProjectsConfig` (or just the two usizes) into `DatabaseRepoSupervisorStore` — add a `with_limits(...)` builder.

3. **Merge-attempt bound: counter-then-block vs. counter-then-AwaitingHuman.** Both keep the loop bounded. `AwaitingHuman` is more accurate (the PR is gate-approved; a human must finish), and matches the existing CI-exhaustion behavior which uses `AwaitingHuman` (`pipeline.rs:287`).
   - **Recommendation: bounded counter → `AwaitingHuman`.** Add a `merge_attempts` task-metadata counter incremented in the two non-success `perform_merge` arms; once it reaches a new `PipelineConfig.max_merge_attempts` (default 3), call `block_task` with a clear reason and return `PipelineOutcome::AwaitingHuman`. Reset the counter when the head SHA changes (a new push is a fresh merge target), mirroring the `reset_empty_ci_polls`/per-SHA signature pattern already in the file.

4. **`installation_id` persistence: at enroll-time discovery vs. webhook-time backfill vs. both.**
   - **Recommendation: both, webhook-time first (cheap, high-value).** Webhook backfill is a 5-line change in an existing matched-repo path and immediately fixes multi-installation auth. Enroll-time discovery (matching the connector repo against `list_installation_repositories` results, which include the installation id) is a nice-to-have that removes the "wait for first webhook" gap; do it as a follow-up sub-task, not a blocker.

## Tasks

- [x] **T1: Bound the approved-merge retry loop in the GitHub pipeline.**
  - **Files:** `src/repo_projects/pipeline.rs` (the `PipelineConfig` struct `:52-74`; `perform_merge` `:481-559`; helper region near `reset_empty_ci_polls` `:248`).
  - **Change:** Add `pub max_merge_attempts: u32` to `PipelineConfig` (default 3 in its `Default`). In `perform_merge`, before attempting the merge, read `metadata_u64(&task.metadata, "merge_attempts")`; if it `>= max_merge_attempts`, call `self.block_task(repo, task, &format!("Merge gate approved but #{number} failed to merge after {attempts} attempt(s); human intervention required."))` and return `Ok(PipelineOutcome::AwaitingHuman { reason: ... })`. In both non-success arms (`Ok(response)` non-merged `:532` and `Err(error)` `:545`), increment and persist `merge_attempts` (`merge_metadata` + `persist_task`) before returning. On a successful merge, and whenever the head SHA changes during `advance_waiting_review` (reset alongside the existing per-SHA logic), zero `merge_attempts`. Wire `max_merge_attempts` from an env override in the agent-loop `PipelineConfig` construction (`src/agent/agent_loop.rs:1009-1031`, e.g. `REPO_PROJECTS_MAX_MERGE_ATTEMPTS`), defaulting to 3.
  - **Acceptance:** A unit test in `pipeline_tests.rs` (`#[cfg(all(test, feature = "libsql"))]`, mirror the existing fake-GitHub + libSQL harness) drives a PR whose merge call repeatedly returns non-merged; after `max_merge_attempts` ticks the task ends in `Blocked` with a `MergeDenied`/blocked event and the outcome is `AwaitingHuman`, and no further merge API calls are issued. A SHA change resets the counter.
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw --features libsql repo_projects::pipeline` (or the workspace root crate that owns `src/repo_projects`); `cargo clippy --all-targets -- -D warnings`.

- [x] **T2: Persist webhook-carried `installation_id` onto the matched repo row.**
  - **Files:** `src/channels/web/handlers/github_webhook.rs` (`find_project_id_for_repo` `:212-242`, called from the handler `:120-126`).
  - **Change:** Extend the repo-matching loop so that when a repo matches and `envelope.installation_id` is `Some` and differs from `repo.installation_id`, set `repo.installation_id = Some(id)`, bump `updated_at`, and `store.upsert_repo_project_repo(&repo)`. Convert `i64 → u64` defensively (`u64::try_from`). Return the matched `project_id` as before. Keep this best-effort: log on upsert error, do not fail the webhook (the deduper has already accepted it).
  - **Acceptance:** Unit/integration test (libSQL) seeds a project with one repo (`installation_id: None`), feeds a signed `pull_request` webhook envelope carrying an installation id, and asserts the repo row now has `installation_id == Some(id)`. Re-delivery does not double-write (idempotent via the existing dedup path).
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw --features libsql github_webhook` ; `cargo clippy --all-targets -- -D warnings`.

- [x] **T3: Persist `installation_id` at enroll-time when discoverable.**
  - **Files:** `src/api/repo_projects.rs` — repo construction in `create_project` (`:300-315`, `installation_id: None` at `:306`), `enroll_repo` (`:994-1014`, `:1000`), and the discovery helper `list_connectable_repos_with_provider` (`:1105-1147`).
  - **Change:** When the active credential mode is GitHub App and a default installation id is configured/discoverable, set the new repo's `installation_id` to that resolved id instead of `None`. Where feasible, reuse the connector discovery result (`list_installation_repositories` already returns the installation's repos) to map the enrolled `owner/repo` to its installation id; otherwise fall back to the configured `github_app.installation_id`. Leave `None` for the personal-access-token path.
  - **Acceptance:** Creating/enrolling a repo under a configured GitHub App stores a non-`None` `installation_id`; PAT mode still stores `None`. Existing `create_project`/`enroll_repo` tests updated; add a test asserting the App-mode path.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw repo_projects` ; `cargo clippy --all-targets -- -D warnings`.

- [x] **T4: Enforce per-project task concurrency in `dispatch_next_task`.**
  - **Files:** `src/repo_projects/supervisor.rs` (`DatabaseRepoSupervisorStore` builders `:72-98`; project loop `:133-213`; `dispatch_next_task` `:362-441`); wiring in `src/agent/agent_loop.rs:967-1069`.
  - **Change:** Add a `with_limits(max_concurrent_projects: usize, max_concurrent_tasks_per_project: usize)` builder storing both on the store (default to the `RepoProjectsConfig` values, passed from the agent loop). In `dispatch_next_task`, compute `running = tasks.iter().filter(|t| t.state == RepoProjectTaskState::Running).count()` and an effective cap `cap = (project.policy.max_parallel_tasks as usize).min(self.max_concurrent_tasks_per_project).max(1)`; dispatch tasks while `running < cap` and a `Queued`/`Ready` task exists (loop, not single-shot), pushing one `DispatchTask` decision per dispatch. In the project-selection loop, stop advancing dispatch for additional projects once `max_concurrent_projects` projects have dispatched this reconcile. Thread `repo_projects_config` (already resolved at `agent_loop.rs:972`) into `with_limits`.
  - **Acceptance:** Unit test with a fake store/executor: a project with `max_parallel_tasks = 2` and 5 `Queued` tasks dispatches exactly 2 (and no more while 2 are `Running`); with `= 1` it dispatches 1. A second test asserts `max_concurrent_projects` caps cross-project dispatch.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw repo_projects::supervisor` ; `cargo clippy --all-targets -- -D warnings`.

- [x] **T5: Define the `RepoTaskPlanner` port and the no-planner status fallback (Decision 1).**
  - **Files:** new `src/repo_projects/planner.rs` (declare in `src/repo_projects/mod.rs:7-15`); `src/repo_projects/supervisor.rs` (`NeedsPlanning` handling at `:149-154`/`:205-207`, new `with_planner` builder, `reconcile_once` `:637-647`).
  - **Change:** Define `#[async_trait] pub trait RepoTaskPlanner: Send + Sync { async fn plan(&self, project: &RepoProject, repos: &[RepoProjectRepo]) -> Result<Vec<PlannedTask>, String>; }` plus a `PlannedTask { title, body, repo_id }` DTO, in `planner.rs`. Add `with_planner(Option<Arc<dyn RepoTaskPlanner>>)` to `DatabaseRepoSupervisorStore`. Replace the bare `NeedsPlanning` push with: if a planner is present, call it, persist each `PlannedTask` as a `Queued` `RepoProjectTask` (build identically to `enqueue_task`, `api/repo_projects.rs:539-572` — extract a shared `build_queued_task(project, repo, title, body)` helper used by both the API and the planner to avoid drift), transition the project to `Active`, append `TaskCreated` events, emit SSE; if no planner, transition the project to `AwaitingHuman`, append a `ProjectStateChanged` event ("plan required — add tasks to proceed"), emit `RepoProjectUpdated`. Keep `NeedsPlanning` as the returned decision for logging.
  - **Acceptance:** With a fake planner returning 3 tasks, a `Planning`+empty project ends `Active` with 3 `Queued` tasks and 3 `TaskCreated` events. With no planner, the project ends `AwaitingHuman` with a status event + SSE. No duplicate tasks on a second reconcile (idempotent: only plan when tasks are still empty).
  - **Effort:** L
  - **Verification:** `cargo test -p thinclaw repo_projects::supervisor repo_projects::planner` ; `cargo clippy --all-targets -- -D warnings`.

- [x] **T6: Implement the LLM-backed planner adapter and wire it into the agent loop.**
  - **Files:** new adapter (root-owned, where the LLM/subagent stack is assembled — alongside the supervisor wiring in `src/agent/agent_loop.rs:967-1069`, or a small `src/repo_projects/` adapter that takes an injected `Arc<SubagentExecutor>`); reuse `src/agent/subagent_executor.rs::spawn` (`:298`) and `src/repo_projects/prompts.rs` for prompt shaping.
  - **Change:** Implement `RepoTaskPlanner` by spawning a one-shot planning subagent (read-only tools / no merge authority) that is given the project goal + enrolled-repo summaries and returns a structured task list (title + body per repo). Parse its structured output into `Vec<PlannedTask>`. Wire `with_planner(Some(...))` into the supervisor store construction only when an LLM/subagent stack is available (guard like the existing `if let Some(secrets) = ...` pipeline guard, `agent_loop.rs:993`). Add an env opt-out (`REPO_PROJECTS_AUTOPLAN`, default on when the LLM stack is present) so operators can force the human-plan fallback.
  - **Acceptance:** With the LLM stack present and `REPO_PROJECTS_AUTOPLAN` unset, a freshly-created project that is started decomposes into ≥1 task without human intervention (covered by an integration test using a stub subagent that returns canned structured output, not a live model). With the opt-out set, the T5 `AwaitingHuman` fallback fires.
  - **Effort:** L
  - **Verification:** `cargo test -p thinclaw repo_projects` ; `cargo clippy --all-targets -- -D warnings` ; manual smoke via `RUST_LOG=thinclaw=debug cargo run` with `REPO_PROJECTS_ENABLED=true`.

- [x] **T7: Consume the repo-project SSE in the WebUI (coordinate with gateway pattern).**
  - **Files:** `crates/thinclaw-gateway/src/web/static/app.js` — add a new consumer block in `connectSSE()` near the existing experiments group (`:974-978`) and a `queueRepoProjectsRefresh()` helper modeled on `queueResearchRefresh()` (`:805-811`).
  - **Change:** Register `['repo_project_updated','repo_task_updated','repo_project_event','repo_merge_gate_updated'].forEach((evtType) => eventSource.addEventListener(evtType, () => queueRepoProjectsRefresh()))`. Implement `queueRepoProjectsRefresh()` with a debounce timer that calls `loadRepoProjectsDashboard()` only when `currentTab === 'repo-projects'` (mirroring the research refresh, and `app.js:2992` which already lazy-loads on tab switch). Do **not** add new `SseEvent` variants or touch backend routing — the four events already flow over `/api/chat/events` (`types.rs:1536-1540`).
  - **Acceptance:** With the Repo Projects tab open, a backend-emitted task/merge-gate/project event refreshes the dashboard within the debounce window without a manual click; with the tab inactive, no fetch is issued. No regression to existing SSE handlers (chat/jobs/experiments).
  - **Effort:** S
  - **Verification:** `./scripts/build-all.sh` is **not** required (static asset). Manual: `cargo run` with `REPO_PROJECTS_ENABLED=true`, open the Repo Projects tab, trigger a webhook/manual wake, observe live refresh. Confirm via the `add-sse-event` skill conventions (consumer side only).

- [x] **T8: Regression + restart-recovery coverage for concurrency and merge bounds.**
  - **Files:** `src/repo_projects/pipeline_tests.rs`, `src/repo_projects/supervisor.rs` `#[cfg(test)]` module (`:688-721`).
  - **Change:** Add tests proving: (a) concurrency cap honored across consecutive reconciles (not just one tick); (b) merge-attempt counter survives `recover()` (metadata is persisted, so a restart mid-loop does not reset the bound); (c) planner idempotency under repeated `NeedsPlanning`.
  - **Acceptance:** All new tests green; no flakiness across 3 local runs.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw --features libsql repo_projects` run 3×.

## Best Practices (workstream-specific)

- **Bound every retry with a per-SHA-resettable metadata counter.** Copy the CI-repair pattern exactly: `metadata_u64(&task.metadata, "...")` read, compare to a `PipelineConfig` cap, `block_task` + `AwaitingHuman` on exhaustion, reset on a new head SHA (`pipeline.rs:225-320`, `reset_empty_ci_polls` `:248`). The new merge counter must follow this shape so all the supervisor's loops are uniformly bounded.
- **Inject capability behind a builder + optional `Arc<dyn Trait>`,** never as a hard field. The store already does this for executor/pipeline/SSE (`supervisor.rs:82-97`). The planner and limits must follow the same `with_*` style so the supervisor crate-direction stays clean (no LLM dependency leaks into the data path) and tests can supply fakes — exactly how `pipeline_tests.rs` uses `FixedTokenGitHubClientProvider` (`github_provider.rs:230`).
- **Persist task changes through `upsert_repo_project_task` and append a `RepoProjectEvent` + emit SSE in lockstep.** Every state mutation in `pipeline.rs`/`supervisor.rs` already does persist → `record_event` → `emit_sse`. New code (planner task creation, merge-bound block) must keep this triple so the freshly-added WebUI consumer (T7) actually reflects the change.
- **Reuse the task-construction shape from `enqueue_task` (`api/repo_projects.rs:539-572`).** Extract a shared `build_queued_task` helper rather than hand-rolling a second `RepoProjectTask` literal in the planner — a divergent literal is exactly the kind of copy-paste drift the audit flagged elsewhere.
- **WebUI SSE consumers debounce and gate on the active tab.** The research/experiments block (`app.js:805-811`, `:974-978`) is the canonical model; copy it verbatim for repo-projects rather than inventing a new refresh strategy.

## Common Pitfalls

- **Fixing the merge loop only for the `Err` arm.** There are **two** unbounded return paths in `perform_merge` (the `Ok(response)` non-merged arm at `pipeline.rs:532` AND the `Err(error)` arm at `:545`) — both currently return `MergeGateRecorded { approved: true }` and re-trigger next tick. The audit anchor names `:532`; missing `:545` leaves the loop half-bounded. Increment/check the counter in both.
- **Letting `recover()` silently reset bounds.** Counters live in `task.metadata`, which survives restart — but only if you persist before returning. If you increment in memory and return without `persist_task`, a watchdog restart resets the count and the bound never bites.
- **Enforcing concurrency one-tick-at-a-time only.** The current one-dispatch-per-tick cadence *looks* like a `max_parallel_tasks = 1` limit but is not one: it is purely incidental and breaks the moment you loop dispatch or wakes arrive faster than tasks reach `Running`. Count actual `Running` tasks; don't rely on cadence.
- **Two concurrency knobs drifting apart.** `ProjectPolicy.max_parallel_tasks` (persisted, per-project) and `RepoProjectsConfig.max_concurrent_tasks_per_project` (env, global) both exist. Decide the precedence once (Decision 2: policy clamped by config ceiling) and apply it in a single place; do not enforce one in dispatch and the other somewhere else.
- **Planner double-dispatch.** `NeedsPlanning` fires on every reconcile while tasks are empty. If the planner is slow/async and you don't guard on "tasks still empty AND not already planning-in-flight," a burst of wakes spawns duplicate planning subagents and duplicate tasks. Guard idempotently and transition state before the planner returns where possible.
- **Adding a new `SseEvent` variant for T7.** The four repo events already exist in `crates/thinclaw-gateway/src/web/types.rs:1437-1463` — adding a variant would be a gateway-owned change and create a duplicate. T7 is **consumer-only**.
- **Touching `installation_id` only at creation (`api:306`) and forgetting enroll (`api:1000`) and the webhook backfill.** All three are independent `None` sites; the audit anchor is `:305`/`:306` but a complete fix covers enroll and the live webhook path.

## Multi-Worker Execution Plan (ultracode)

- **Worker decomposition:**
  - **Track A (independent, parallelizable):** T1 (merge bound) and T2 (webhook installation_id). Both are small, file-local, and touch non-overlapping files (`pipeline.rs` vs `github_webhook.rs`).
  - **Track B (independent):** T7 (WebUI SSE consumer) — pure `app.js`, no Rust overlap.
  - **Track C (sequential):** T4 (concurrency enforcement) → T8 concurrency tests; then T5 (planner port + fallback) → T6 (LLM planner adapter) → T3 (enroll-time installation_id, which can fold into the App-mode discovery work) → remaining T8 tests. T5/T6 share `supervisor.rs`, `planner.rs`, and the agent-loop wiring, so they must be sequential. T4 and T5 both edit `supervisor.rs`/`agent_loop.rs` — run them on the **same** worktree/branch sequentially (T4 first) to avoid conflict.
  - Recommended split: 3 parallel subagents for {A: T1, A: T2, B: T7} on isolated worktrees, then a single sequential worker for Track C on its own worktree, finishing with the shared T8 test pass.
- **Isolation:** Yes — use git worktree isolation. Tracks A/B touch disjoint files and can each run in their own worktree and merge cleanly. Track C must be a single worktree because T4/T5/T6 mutate the same `supervisor.rs` + `agent_loop.rs`; do not fan T4–T6 into parallel worktrees.
- **Workflow shape:**
  1. **Implement (fan-out 4):** worktree-1 T1, worktree-2 T2, worktree-3 T7, worktree-4 T4→T5→T6→T3 (sequential within the worktree).
  2. **Verify (per worktree):** run the per-task `cargo test` + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check` gate before merge.
  3. **Review (`/code-review` at high effort):** focus on the two merge-loop arms, counter persistence/reset, concurrency-count correctness, and planner idempotency/duplicate-task risk.
  4. **Integrate:** merge A/B worktrees first (low conflict), then Track C; run the full T8 regression pass on the integrated branch.
  5. **Fix:** address review findings, re-run the gate.
- **Verification gate (exact):**
  - `cargo fmt --all -- --check`
  - `cargo clippy --all --benches --tests --examples --all-features -- -D warnings`  *(matches CLAUDE.md; note CI omits `--all-targets`, so run it locally)*
  - `cargo test -p thinclaw --features libsql repo_projects` (supervisor + pipeline + planner + webhook)
  - `cargo test -p thinclaw-config repo_projects` (config knobs unchanged/extended)
  - `/ship` for the full Rust quality gate before opening the PR; `/code-review high` on the diff.
  - **DB/Docker prerequisites:** the libSQL pipeline/supervisor tests run in-process (no Docker). Postgres-backed `db_contract::repo_projects` needs a `pgvector/pgvector:pg17` container with `migrations/V*.sql` applied to `thinclaw_test` (per CLAUDE.md local-dev notes). The `repo_project_docker_e2e` end-to-end test is `#[ignore]`/quarantined — do **not** gate this WS on it.

## Definition of Done

- [x] `RepoSupervisorDecision::NeedsPlanning` is acted on: with a planner wired, `Planning`/`Draft`-empty projects decompose into `Queued` tasks and go `Active`; with no planner, they transition to `AwaitingHuman` with a status event + SSE. No project can silently stall in `Planning`.
- [x] `ProjectPolicy.max_parallel_tasks` (clamped by `max_concurrent_tasks_per_project`) and `max_concurrent_projects` actually limit dispatch, proven by multi-tick tests.
- [x] `perform_merge` is bounded by `max_merge_attempts` in **both** non-success arms, escalates to `AwaitingHuman`/`Blocked` on exhaustion, resets on head-SHA change, and the bound survives `recover()`.
- [x] `installation_id` is persisted on the repo row from the webhook path, and at enroll/create when in GitHub App mode (PAT mode stays `None`).
- [x] The WebUI live-refreshes the Repo Projects dashboard from the four existing SSE events when the tab is active, debounced, with no new `SseEvent` variant.
- [x] `cargo fmt`, `clippy --all-targets -D warnings`, and the repo-projects test suites are green; `/ship` passes.
- [x] Decision Points 1–4 are resolved in the merged code (planner-with-fallback, policy-clamped-by-config, counter→AwaitingHuman, installation_id both paths).
- [x] Behavior-change docs handed to **WS-12** (inventory rows for the subsystem are WS-12's; this WS only flags that the planner/concurrency/merge-bound behavior is new so WS-12 can update `FEATURE_PARITY.md` and add the missing `thinclaw-repo-projects` row to `CRATE_OWNERSHIP.md`).
