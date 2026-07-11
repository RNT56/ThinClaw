# WS-09 — Routines / Scheduler / Heartbeat Completion

> **✅ STATUS: DONE. Landed in commit `dd7b7cdb` (repo-project supervisor + routines/heartbeat), merged to `main` via the audit-hardening stack (`1fb29984`, HEAD `bda7a61f`).**
> This plan is complete; do not execute it. It is retained as an implementation record. All six tasks (T1–T6) shipped: `spawn_heartbeat` and the self-looping `HeartbeatRunner::run` are erased from both trees (`check_heartbeat` kept for `/heartbeat`); `process_claimed_event` accumulates `dispatch_errors` and continues instead of breaking (`src/agent/routine_engine.rs:1096,1116,1137`); heartbeat `target`/`include_reasoning` are honored via `heartbeat_job_metadata` (`include_reasoning`/`suppress_output`/`notify_channel` keys, `crates/thinclaw-agent/src/routine_engine.rs:1148-1152`); `dedup_window` is enforced by content-hash window dedup (`routine_engine.rs:967-1000`); and the validated webhook body is threaded through `fire_manual_with_payload`. DP-3 resolved as WIRE. The "Current State (verified)" section below describes the *pre-remediation* state.

> **Status:** Done (landed) · **Priority:** P2 · **Risk:** low · **Effort:** M
> **Depends on:** none · **Blocks:** WS-10 (heavy decomposition of `src/agent/routine_engine.rs`) — coordinate so WS-10 splits *after* the small behavioral edits here land, to avoid churn.
> **Owns (symbols/files):**
> - `src/agent/heartbeat.rs` (the root compatibility wrapper — `HeartbeatRunner::run`, `spawn_heartbeat`)
> - `crates/thinclaw-agent/src/heartbeat.rs` `HeartbeatRunner::run` + `spawn_heartbeat` (lines 132-191, 384-400)
> - `execute_heartbeat` in `src/agent/routine_engine.rs:1656` (the `target` / `include_reasoning` parameters and their consumption)
> - `RoutineEngine::fire_manual` / `spawn_fire` in `src/agent/routine_engine.rs:326,960` (the webhook-body payload plumbing)
> - `process_claimed_event` dispatch loop in `src/agent/routine_engine.rs:898-916` (per-event error isolation)
> - `RoutineGuardrails.dedup_window` in `crates/thinclaw-agent/src/routine.rs:809` (the enforce-or-erase decision and any new content-dedup store method)
> - `webhook_routine_trigger_handler` in `src/channels/web/handlers/routines.rs:533` (the body forwarding call site)
>
> Note: WS-08 (LLM) owns nothing here; WS-10 owns the *structural* decomposition of the 2536-line engine. This WS only touches the named symbols above.

## Vision & Goal

The routine/scheduler/heartbeat engine is the **proactive runtime** — the thing that makes ThinClaw a personal *agent* rather than a request/response chatbot. It is already fully wired and durable (cron + interval scheduling, event matching, webhook triggers, system-event injection, catch-up policy, leases, zombie reaping). This workstream closes the last gaps between what the config surface *promises operators* and what the engine actually *does*: heartbeat output routing (`target`), reasoning verbosity (`include_reasoning`), content dedup (`dedup_window`), webhook payload pass-through, and an orphaned legacy heartbeat loop. Each is a small, contained fix that makes a documented knob real (or honestly removes it), tightening operator trust in the proactive surface.

## Scope

**In scope:**
1. Honor the heartbeat `target` ("chat" | "none" | channel name) and `include_reasoning` flags in `execute_heartbeat`, or convert them to honest no-ops with a documented rationale.
2. Enforce `RoutineGuardrails.dedup_window` (content-hash dedup over a time window) **or** delete it cleanly across the type, both DB backends, and the config/profile writers.
3. Pass the validated webhook body into the triggered routine instead of dropping it.
4. Dispose of the orphaned standalone heartbeat runner (`HeartbeatRunner::run` self-loop + `spawn_heartbeat`), now that the routine engine owns heartbeat scheduling.
5. Make `process_claimed_event` dispatch isolate per-event spawn errors instead of `break`-ing the whole batch on the first failure.

**Out of scope (and owner):**
- Decomposing the 2536-line `src/agent/routine_engine.rs` into focused submodules — **WS-10**.
- The `gateway_auth_token` empty-bearer bypass that also touches `src/channels/web/` — **WS-01 (security)**. This WS edits a *different* handler in `src/channels/web/handlers/routines.rs`; if WS-01 is mid-flight on that tree, sequence the webhook task after it.
- Repo-project supervisor concurrency/planner gaps (also durable-queue shaped) — **WS-07**.
- Experiment artifact retention — **WS-06**.

## Current State (verified)

> **Historical (pre-remediation) snapshot.** All five gaps below (heartbeat knobs, `dedup_window`, webhook body, orphaned runner, break-on-first-error) were closed by the landed WS-09 work. Kept for context. Some persistence anchors here are stale: the `src/history/store/` directory was deleted in the WS-10 history/store consolidation, so the `dedup_window` persistence set is smaller and now lives only under `crates/thinclaw-db`.

Anchors confirmed by reading the code on 2026-06-23.

**Heartbeat `target` / `include_reasoning` — half-wired (config plumbed, execution inert):**
- `HeartbeatConfig` carries both fields with defaults `target = "chat"`, `include_reasoning = false` (`crates/thinclaw-config/src/heartbeat.rs:24-27,49-50`), resolved from settings/env (`heartbeat.rs:77-78`).
- They flow into `RoutineAction::Heartbeat { target, include_reasoning, .. }` via `HeartbeatRoutineSpec::from_config` (`crates/thinclaw-agent/src/agent_loop.rs:76,79`) and the `upsert_heartbeat_routine` call (`src/agent/agent_loop.rs:2202-2203`), persist/round-trip through `RoutineAction::to_config_json` / `from_db` (`crates/thinclaw-agent/src/routine.rs:646-664,764-767`).
- They are **dropped on the floor at execution**: `execute_heartbeat(..., _include_reasoning: bool, ..., _target: &str, ...)` — both parameters are underscore-prefixed and never read (`src/agent/routine_engine.rs:1662,1665`). The light-context path dispatches a worker job (`dispatch_job_reserved_for_routine`, line 1811) whose metadata (`heartbeat_job_metadata`, `crates/thinclaw-agent/src/routine_engine.rs:976`) does **not** include target or reasoning. Notification routing is governed only by `NotifyConfig.channel` via `send_notification` (`src/agent/routine_engine.rs:1400-1407,1932`), which is a *coarser* knob than the documented `target` ("none" especially has no representation).

**`dedup_window` — drifted (persisted everywhere, enforced nowhere):**
- Declared on `RoutineGuardrails` (`crates/thinclaw-agent/src/routine.rs:809`), default `None` (line 817).
- **Written and read** in both DB backends and the profile writer: `src/history/store/routine_crud.rs:42,190` + `routine_rows.rs:39`; `crates/thinclaw-db/src/postgres_store/routine_crud.rs:42,190` + `routine_rows.rs:39`; `crates/thinclaw-db/src/libsql/routines.rs:54,254` + `libsql/mod.rs:499,520`; column in `crates/thinclaw-db/src/libsql_migrations.rs:1054`; set by `src/profile_evolution.rs:227` and compared in change-detection at `:324` and `crates/thinclaw-agent/src/agent_loop.rs:151`.
- **Never consulted in any dispatch decision.** Event dedup is by `idempotency_key` (external message id, `crates/thinclaw-agent/src/routine_engine.rs:355-375`) checked via `routine_run_exists_for_trigger_key` (`src/agent/routine_engine.rs:822-828`). `content_hash` is computed and stored on every `RoutineEvent` (`crates/thinclaw-agent/src/routine_engine.rs:414`) but no query keys off `content_hash` + a time window. So `dedup_window` is pure dead config that round-trips through three stores.

**Webhook body — dropped (validated, then discarded):**
- `webhook_routine_trigger_handler` reads `body: Bytes` (`src/channels/web/handlers/routines.rs:537`), size-checks it (line 539), verifies the HMAC signature against it (line 574-580), then calls `engine.fire_manual(routine_id)` (line 583) — which takes **no payload** (`src/agent/routine_engine.rs:326`). The body is never forwarded. The non-engine fallback branch (lines 599-624) also ignores the body, building the prompt purely from the routine action.

**Orphaned standalone heartbeat runner — dead (superseded by the routine engine):**
- The agent loop documents the supersession: "The old HeartbeatRunner included both heartbeat checks AND memory hygiene. Heartbeat checks are now fully handled by the routine engine" (`src/agent/agent_loop.rs:725-727`).
- `spawn_heartbeat` has **zero callers** (root: `src/agent/heartbeat.rs:154`, re-exported `src/agent/mod.rs:81`; crate: `crates/thinclaw-agent/src/heartbeat.rs:385`). Verified: `grep spawn_heartbeat` outside the definitions/`pub use` returns nothing.
- The self-looping `HeartbeatRunner::run` (root `src/agent/heartbeat.rs:75` → `self.inner.run()`; crate `crates/thinclaw-agent/src/heartbeat.rs:132-191`) is only reachable *through* `spawn_heartbeat` — also dead.
- `HeartbeatRunner::check_heartbeat` is **WIRED** and must stay: it backs the interactive `/heartbeat` command (`src/agent/commands.rs:237-248`).
- Stale doc reference: `src/workspace/README.md:99-103` still shows `spawn_heartbeat` as the heartbeat entry point.

**Break-on-first-error in event dispatch — confirmed (low sev, self-correcting):**
- `process_claimed_event` evaluates all matched routines, then in the fire loop, on the *first* `spawn_fire` error it sets `has_deferred = true` and `break`s (`src/agent/routine_engine.rs:898-916`). The remaining `should_fire` plans for that event are skipped; the whole event is then *released* (not completed) (line 933-941) and retried later. The audit rates this low/self-correcting via idempotency (`routine_run_exists_for_trigger_key` prevents double-fire on retry), but a single flaky routine defers all sibling routines on the same message until the next drain.

## Decision Points

**DP-1 — Heartbeat `target` ("chat" | "none" | channel name): WIRE.**
- *Options:* (a) Wire it so "none" suppresses all delivery, "chat" delivers to the default surface, and a channel name overrides `NotifyConfig.channel` for delivery. (b) Delete the field and rely solely on `NotifyConfig.channel`.
- *Trade-off:* `target` is strictly more expressive than `NotifyConfig.channel` — "none" (run silently, log only) has no `NotifyConfig` representation today, and it is a documented operator knob (`docs/SURFACES_AND_COMMANDS.md`, settings). Deleting it removes capability the vision wants (quiet background heartbeats).
- **Recommendation: WIRE.** Map `target` onto the existing notification path: `"none"` → skip `send_notification` and skip the worker's outbound delivery; `"chat"` → current behavior; any other value → treat as a channel override (set on the dispatched job metadata + `NotifyConfig`). This is a behavioral edit inside `execute_heartbeat` plus `heartbeat_job_metadata`, no new subsystem.

**DP-2 — Heartbeat `include_reasoning`: WIRE (lightweight) — small.**
- *Options:* (a) Thread it into the worker/prompt so the heartbeat output retains the reasoning chain when true. (b) Delete it.
- *Trade-off:* It's a one-field verbosity toggle already surfaced in settings; deleting churns config + settings + DTOs across three crates. Wiring is cheap: pass it through `heartbeat_job_metadata` so the worker includes reasoning in its emitted summary (mirror how `full_job` threads `max_iterations`).
- **Recommendation: WIRE**, paired with DP-1 in the same edit since both live in `execute_heartbeat`/`heartbeat_job_metadata`.

**DP-3 — `dedup_window`: ERASE (default) vs WIRE.**
- *Options:* (a) ERASE — remove the field, the three-backend persistence, the migration column (or leave the column unused), and the profile/agent_loop writers. (b) WIRE — add a store method `routine_event_recent_content_match(routine_id, content_hash, since)` and have `process_claimed_event` skip-as-`SkippedDuplicate` when a matching content hash fired inside the window.
- *Trade-off:* The vision values dedup (it's why `content_hash` is already computed and stored, and `SkippedDuplicate`/`RoutineEventDecision::SkippedDuplicate` decision variants already exist — `crates/thinclaw-agent/src/routine.rs:978`). But the *current* idempotency-key dedup already covers the common case (same message id). `dedup_window` adds value only for *semantically duplicate distinct messages* (e.g. two different "deploy prod" messages within 24h). That is a real but niche capability, and wiring it touches the hot event path + adds a store method + tests on both backends.
- **Recommendation: WIRE (it is genuine half-finished capability, not drifted cruft — the hash is already stored and the decision variant already exists), but gate the effort:** implement it as a *single* additive store method behind the existing `Database` trait, reuse the `SkippedDuplicate` decision, and only run the extra query when `routine.guardrails.dedup_window.is_some()` (so the default-`None` hot path is unchanged). If WS sizing forces a cut, ERASE is the acceptable fallback — but then erase *all five* persistence/writer sites in one commit (see Common Pitfalls). Default plan below assumes WIRE.

**DP-4 — Webhook body pass-through: WIRE.**
- *Options:* (a) Add a payload-bearing fire path (`fire_manual_with_payload`) that injects the body into the routine's effective prompt / system-event message. (b) Keep dropping it.
- **Recommendation: WIRE.** The body is already received, size-limited, and HMAC-verified; dropping it makes signed webhook payloads useless. Thread `Option<String>` (UTF-8 body, capped) through `spawn_fire` → `RoutineRun.trigger_detail` (already exists) and into prompt assembly for `Lightweight`/`FullJob`/`SystemEvent`/`Heartbeat`.

**DP-5 — Orphaned standalone heartbeat runner: ERASE.**
- *Options:* (a) ERASE `spawn_heartbeat` + `HeartbeatRunner::run` (both root wrapper and crate), keeping `check_heartbeat`. (b) Keep as "future API."
- **Recommendation: ERASE.** Zero callers, explicitly superseded by the routine engine per the in-code comment at `agent_loop.rs:725`. Keeping it is exactly the "default forever / aspirational API" the project rules warn against. `check_heartbeat` stays (wired to `/heartbeat`).

**DP-6 — Per-event error isolation: BUILD (fix in place).**
- Not a wire-vs-erase; the fix is to `continue` instead of `break` and accumulate errors. Recommendation: build it.

## Tasks

Ordered. T1–T2 are independent and parallelizable; T3 depends on nothing but touches the engine; T4/T5/T6 are independent. Land small commits.

- [x] **T1: Erase the orphaned standalone heartbeat runner**
  - **Files:** `src/agent/heartbeat.rs` (remove `pub async fn run` line 74-77 and `pub fn spawn_heartbeat` lines 153-174); `src/agent/mod.rs:81` (drop `spawn_heartbeat` from the re-export, keep `HeartbeatConfig, HeartbeatResult, HeartbeatRunner`); `crates/thinclaw-agent/src/heartbeat.rs` (remove `pub async fn run` lines 132-191 and `pub fn spawn_heartbeat` lines 384-400, and prune now-unused fields like `consecutive_failures`/`config.max_failures`/`config.interval` *only if* nothing else reads them — verify with grep first); `src/workspace/README.md:99-103` (replace the stale `spawn_heartbeat` example with a pointer to the routine-engine heartbeat / `upsert_heartbeat_routine`).
  - **Change:** Delete the dead self-loop and spawner. Keep `HeartbeatRunner::new`, builder methods, and `check_heartbeat` (still used by `/heartbeat`, `src/agent/commands.rs:248`). If `run`'s removal orphans the crate-side `send_notification`/hygiene-tick helpers, remove those too; if they're shared with `check_heartbeat`, leave them.
  - **Acceptance:** `grep -rn "spawn_heartbeat\|HeartbeatRunner::run\|\.run().await" src crates | grep -i heartbeat` returns only intentional hits (none). `/heartbeat` command still compiles and runs. No `dead_code` warnings reintroduced.
  - **Effort:** S
  - **Verification:** `cargo build -p thinclaw-agent && cargo build` ; `cargo clippy -p thinclaw-agent --all-targets -- -D warnings`.

- [x] **T2: Per-event error isolation in `process_claimed_event`**
  - **Files:** `src/agent/routine_engine.rs:898-916`.
  - **Change:** Replace the `break` on `spawn_fire` error with `continue`, collecting all errors (e.g. `let mut dispatch_errors: Vec<String> = Vec::new();`). Keep setting `has_deferred = true` so the event is released for retry, but let every other `should_fire` plan still attempt its spawn (each is independently idempotent via `routine_run_exists_for_trigger_key`). Update the `diagnostics` JSON to carry `dispatch_errors` (array) instead of the single `dispatch_error` (line 930) — or keep a joined string if you want to avoid touching the diagnostics schema consumers; grep `dispatch_error` first.
  - **Acceptance:** A failing routine no longer prevents sibling routines on the same event from firing in the same drain. Add a unit test in the existing `#[cfg(test)] mod tests` of `routine_engine.rs` (there are already engine tests around line 2156+) that wires two `should_fire` plans where the first spawn errors and asserts the second still fires (or, if the spawn path is hard to fake, assert the loop accumulates both errors). Reuse the existing test scaffolding (`notify_tx`, mock store) at `src/agent/routine_engine.rs:2156-2480`.
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw routine` (root package) and the new test; `cargo clippy --all-targets -- -D warnings`.

- [x] **T3: Honor heartbeat `target` and `include_reasoning`**
  - **Files:** `src/agent/routine_engine.rs` `execute_heartbeat` (1656-1853) — rename `_include_reasoning`/`_target` to real params; `crates/thinclaw-agent/src/routine_engine.rs` `heartbeat_job_metadata` (976-983) — extend signature to accept `target: &str` and `include_reasoning: bool` and emit them in the metadata JSON (mirror the keys already there).
  - **Change:**
    - `target == "none"`: after a successful run, skip outbound delivery — return a status that does *not* notify. For the light-context worker path, set a metadata flag (`"suppress_output": true`) the worker honors, OR (simpler, no worker change) return `RunStatus::Ok` with a summary but ensure `send_notification` is bypassed for this routine (since heartbeat uses `NotifyConfig` indirectly). Confirm the exact notify seam: heartbeat light-context returns `RunStatus::Running` (line 1848) so completion/notify is handled by the worker; for `target="none"` set the worker metadata to suppress its emitted user message.
    - `target == "chat"`: current behavior (default surface).
    - `target == <channel>`: thread the channel into `heartbeat_job_metadata` (`"notify_channel": target`) so the worker's outbound delivery and any `RoutineLifecycle` SSE route to that channel, overriding `NotifyConfig.channel`.
    - `include_reasoning`: pass into `heartbeat_job_metadata` (`"include_reasoning": true`) so the worker includes the reasoning chain in its emitted summary (mirror how the worker reads `max_iterations` from heartbeat metadata).
  - **Acceptance:** Setting `HEARTBEAT_TARGET=none` (or settings `heartbeat.target="none"`) produces no chat/channel output (verifiable via the routine run summary + absence of an outbound message); a channel-name target routes output there; `include_reasoning=true` causes the heartbeat summary to retain reasoning. Add/extend a unit test asserting `heartbeat_job_metadata(routine, iters, target, include_reasoning)` carries the new keys.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw-agent heartbeat` ; `cargo test -p thinclaw heartbeat` ; manual: set the env vars, run with `RUST_LOG=thinclaw=debug cargo run`, observe a heartbeat tick. Confirm the worker side actually consumes the new metadata keys — grep the worker job execution for how it reads `heartbeat`/`max_iterations` metadata before assuming the key name; if the worker does not yet read `include_reasoning`/`notify_channel`, add the consumption there (that worker code is root-owned in `src/agent/` — confirm it is not WS-10-reserved before editing; if it is, raise a dependency note).

- [x] **T4: Pass the webhook body into the triggered routine**
  - **Files:** `src/agent/routine_engine.rs` — add `pub async fn fire_manual_with_payload(&self, routine_id: Uuid, payload: Option<String>)` alongside `fire_manual` (326) that forwards `payload` into `spawn_fire` as `trigger_detail` (the existing `Option<String>` slot, line 964); have `fire_manual` delegate with `None`. In `execute_routine` / the per-action executors, inject `run.trigger_detail` (when present) into the effective prompt: for `Lightweight`/`FullJob` append a clearly delimited "Webhook payload:" block; for `SystemEvent`/`Heartbeat` include it in the injected message. `src/channels/web/handlers/routines.rs:583` — call `fire_manual_with_payload(routine_id, body_as_utf8_capped)` instead of `fire_manual`.
  - **Change:** Decode `body: Bytes` to UTF-8 lossily, cap to a sane length (reuse/define a constant near the existing `routine_webhook_body_too_large` policy in `crates/thinclaw-gateway/src/web/routines.rs:563`). Do **not** thread raw bytes into the prompt unbounded.
  - **Acceptance:** A signed webhook POST with a JSON/text body results in a routine run whose prompt/context contains the payload (visible in the run trajectory/summary). `fire_manual` callers (manual tool, CLI) are unaffected (pass `None`). Webhook size limit still enforced.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw routine` ; integration: existing webhook handler tests in `crates/thinclaw-gateway/src/web/routines.rs:660+` — add a case asserting the payload reaches the fire path. `cargo clippy --all-targets -- -D warnings`.

- [x] **T5 (DP-3 = WIRE): Enforce `dedup_window` via content-hash window dedup**
  - **Files:** `crates/thinclaw-db/src/lib.rs` (extend the `Database` trait with `routine_event_recent_content_match(routine_id: Uuid, content_hash: &str, since: DateTime<Utc>) -> Result<bool, DatabaseError>`); implement in `crates/thinclaw-db/src/postgres.rs` + `postgres_store/routine_events.rs` and `crates/thinclaw-db/src/libsql/routines.rs` (query `routine_events` / `routine_event_evaluations` for a `fired` decision with matching `content_hash` and `created_at >= since`); call site in `src/agent/routine_engine.rs:818-840` inside the `Matched` arm — when `routine.guardrails.dedup_window.is_some()`, compute `since = now - window`, query, and on hit set decision `RoutineEventDecision::SkippedDuplicate` (variant already exists, `crates/thinclaw-agent/src/routine.rs:978`) and `should_fire = false`.
  - **Change:** Only issue the extra query when the window is set (keep the default `None` hot path identical). Reuse `event.content_hash` (already populated, `crates/thinclaw-agent/src/routine_engine.rs:414`).
  - **Acceptance:** Two distinct messages with identical content arriving within `dedup_window` cause only the first to fire; the second records `SkippedDuplicate`. Outside the window, both fire. Add db_contract coverage (`tests/db_contract/routines.rs` already exercises routine store methods) for the new method on both backends.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw-db` ; `cargo test --test db_contract routines` against both Postgres (Docker `pgvector/pgvector:pg17` + migrations applied per CLAUDE.md) and libSQL; `cargo clippy --all-targets -- -D warnings`. **If sizing forces ERASE instead:** remove the field from `crates/thinclaw-agent/src/routine.rs:809,817` and *all* persistence sites listed in Current State, plus the migration column, in one atomic commit; acceptance becomes "no `dedup_window` references remain and both backends round-trip routines without it."
  - **Decision gate:** This task is the one operator sign-off point (see decision_points). Default = WIRE.

- [x] **T6: Docs + FEATURE_PARITY sync**
  - **Files:** `docs/SURFACES_AND_COMMANDS.md` (heartbeat `target`/`include_reasoning` now honored), `src/setup/README.md` only if onboarding surfaces these knobs, `FEATURE_PARITY.md` (flip routine/heartbeat completeness items), `docs/MEMORY_AND_GROWTH.md` if heartbeat behavior is described there. Confirm `src/workspace/README.md` was fixed in T1.
  - **Change:** Update behavior descriptions for the wired knobs; note `dedup_window` is now enforced (or removed). Do not restate code internals.
  - **Acceptance:** No doc still claims `target`/`include_reasoning`/`dedup_window` are inert or shows `spawn_heartbeat` as the entry point.
  - **Effort:** S
  - **Verification:** `grep -rn "spawn_heartbeat\|dedup_window\|include_reasoning\|heartbeat.*target" docs src/workspace/README.md src/setup/README.md` reviewed by hand.

## Best Practices (workstream-specific)

- **Edit the crate-owned policy, re-export through the root facade.** Pure helpers (metadata builders, filter/decision logic, schedule math) live in `crates/thinclaw-agent/src/routine_engine.rs` and `routine.rs`; the root `src/agent/routine_engine.rs` owns side-effecting execution (spawning workers, DB writes, SSE). Keep new *pure* logic (e.g. the `target` → delivery mapping decision) in the crate and call it from the root — mirror how `heartbeat_job_metadata`, `evaluate_routine_event_filters`, and `build_routine_notification` already split (`crates/thinclaw-agent/src/routine_engine.rs:976,484,792`).
- **Use the existing decision enums, don't invent strings.** `RoutineEventDecision` / `RoutineTriggerDecision` already have `SkippedDuplicate`, `DeferredConcurrency`, etc. (`crates/thinclaw-agent/src/routine.rs:970-982,1112-1120`). Reuse them for T5 rather than ad-hoc reasons.
- **Round-trip any new persisted field through both backends + a test.** Every routine field already has Postgres + libSQL CRUD and a `to/from_db` round-trip test (`crates/thinclaw-agent/src/routine.rs` test module). Follow that pattern for T5's store method (db_contract).
- **Thread optional context through existing slots.** `RoutineRun.trigger_detail: Option<String>` already exists and is persisted — reuse it for the webhook payload (T4) instead of adding a new column.
- **Keep the default hot path allocation-free.** Guard new queries (T5) behind `is_some()` so the overwhelmingly common `dedup_window = None` event flow does not gain a DB round-trip.
- **Preserve idempotency invariants.** The whole engine relies on `routine_run_exists_for_trigger_key` to make retries safe; T2's `continue`-on-error is only safe *because* of that — do not weaken it.

## Common Pitfalls

- **Partial-erase drift (the audit's signature failure mode).** `dedup_window` lives in **five** persistence/writer sites across three crates (`src/history/store/`, `crates/thinclaw-db/src/postgres_store/`, `crates/thinclaw-db/src/libsql/`, `src/profile_evolution.rs`, `crates/thinclaw-agent/src/agent_loop.rs`). The audit explicitly flagged a fix landing "in only one of N copies" (e.g. `split_message`). If you ERASE, remove *all* of them atomically; if you WIRE, the `content_hash` column already exists in both migrations — do not add a redundant one. Grep `dedup_window` and `content_hash` before and after.
- **Two `routine_engine.rs` files.** `src/agent/routine_engine.rs` (root, side effects) and `crates/thinclaw-agent/src/routine_engine.rs` (crate, pure helpers) are different files. The orphaned *standalone runner* is neither — it's in `heartbeat.rs` (also two copies). Do not confuse them; T1 touches `heartbeat.rs`, not `routine_engine.rs`.
- **`check_heartbeat` is live — don't delete it with `run`.** They share `HeartbeatRunner`; `/heartbeat` (`src/agent/commands.rs:248`) calls `check_heartbeat`. Removing the struct or `new`/builders breaks the command.
- **`target` is finer-grained than `NotifyConfig.channel`.** Don't "wire" `target` by simply copying it into `NotifyConfig.channel` — that loses the `"none"` semantics (silent run). Map all three cases explicitly.
- **Worker-side consumption gap.** Light-context heartbeats run as worker jobs; setting metadata keys (`include_reasoning`, `notify_channel`) only helps if the worker reads them. Verify the worker job execution path consumes the new keys before claiming T3 done — otherwise the knob is still inert, just relocated.
- **Webhook payload as injection vector.** The body is operator-trusted (HMAC-signed) but still untrusted *content*. Cap length, decode lossily, and inject into the prompt inside a clearly delimited block; never let it masquerade as system instructions.
- **Diagnostics schema consumers.** T2 changes `dispatch_error` (singular) in the event diagnostics JSON (`src/agent/routine_engine.rs:930`). Grep for readers (SSE/UI/`api/routines.rs`) before renaming to plural.

## Multi-Worker Execution Plan (ultracode)

- **Worker decomposition:**
  - **Worker A (cleanup):** T1 (erase runner) + T2 (error isolation) + T6 docs for those. Small, fast, no DB.
  - **Worker B (heartbeat knobs):** T3 (`target`/`include_reasoning`). Touches root `execute_heartbeat`, crate `heartbeat_job_metadata`, and the worker consumption path.
  - **Worker C (webhook payload):** T4. Touches root engine `fire_manual*` + the gateway handler.
  - **Worker D (dedup, gated):** T5. Touches both DB backends + the event hot path + db_contract. Start only after the DP-3 decision is confirmed (default WIRE).
  - Sequence the shared-file contention: Workers A, B, C, D **all** touch `src/agent/routine_engine.rs`. Run **B → C → D sequentially on the engine file** (or use worktrees + a serialized merge), while A's engine edit (T2) is tiny and can lead. Do **not** parallel-mutate `src/agent/routine_engine.rs` without isolation.
- **Isolation:** Yes — use git worktrees. Multiple tasks edit `src/agent/routine_engine.rs` and the two `heartbeat.rs` files. Give each worker its own worktree/branch and merge serially (cleanup → heartbeat → webhook → dedup) to keep the engine file conflict-free. T5's DB-backend edits are isolated enough to run in parallel with B/C as long as the engine call-site edit is merged last.
- **Workflow shape:** implement → verify → review → fix, fanned out per worker:
  1. **Implement** (fan-out A/B/C/D in worktrees).
  2. **Verify** each branch independently with its task's verification commands.
  3. **Merge serially** in dependency order; re-run the full gate after each merge.
  4. **Review** (`/code-review` high) the combined diff, focusing on the engine hot path (T5 guard), webhook injection (T4), and the worker metadata consumption (T3).
  5. **Fix** review findings; re-gate.
- **Verification gate (run on the merged branch):**
  - `cargo fmt --all`
  - `cargo clippy --all --benches --tests --examples --all-features -- -D warnings` (note: CI omits `--all-targets`; run it locally per AUDIT-FINDINGS §9 to catch test/bench warnings)
  - `cargo test -p thinclaw-agent`
  - `cargo test -p thinclaw-db`
  - `cargo test -p thinclaw` (root) — routine/heartbeat tests
  - `cargo test --test db_contract routines` — **requires** Docker `pgvector/pgvector:pg17` + repo `migrations/V*.sql` applied to `thinclaw_test` for the Postgres path, and libSQL for the other (per CLAUDE.md local-dev notes); libSQL path runs without Docker.
  - `/ship` (the repo quality-gate skill) then `/code-review high` on the diff.
  - **DB/Docker prerequisite (T5 only):** if Docker is unhealthy, follow the CLAUDE.md recovery note (check `df -h /System/Volumes/Data`, clear `target*`, restart Docker) before running db_contract.

## Definition of Done

- [x] DP-3 (`dedup_window` WIRE vs ERASE) explicitly resolved with operator sign-off; the chosen path fully implemented across all five persistence/writer sites with no partial drift.
- [x] Heartbeat `target` ("none"/"chat"/channel) and `include_reasoning` are honored end-to-end (config → action → execution → worker output), verified by a manual heartbeat tick and unit tests on `heartbeat_job_metadata`.
- [x] Signed webhook bodies reach the triggered routine's prompt/context (capped, delimited); `fire_manual` non-webhook callers unaffected.
- [x] Orphaned `spawn_heartbeat` + `HeartbeatRunner::run` removed (both root and crate); `check_heartbeat` / `/heartbeat` still work; `src/workspace/README.md` no longer references `spawn_heartbeat`.
- [x] `process_claimed_event` isolates per-event spawn errors (`continue`, accumulated diagnostics) with a regression test; idempotency invariant preserved.
- [x] Full verification gate green (fmt, clippy `-D warnings` with `--all-targets`, all named test targets incl. db_contract on both backends).
- [x] Canonical docs updated in-branch (`docs/SURFACES_AND_COMMANDS.md`, `FEATURE_PARITY.md`, `src/workspace/README.md`, and setup docs if surfaced); no doc still calls these knobs inert.
- [x] No new god-file growth introduced (new pure logic lives in the `thinclaw-agent` crate, re-exported through the root facade) — coordinated with WS-10 so its decomposition lands after these edits.
