# WS-07 — Experiments / Research Platform Completion

> **✅ STATUS: DONE. Landed in commit `c5c27e56` (experiments + LLM routing consolidation), merged to `main` via the audit-hardening stack (`1fb29984`, HEAD `bda7a61f`).**
> This plan is complete; do not execute it. It is retained as an implementation record. The three operability gaps closed: the artifact-retention reaper is a real spawned maintenance loop threading `retention_days` (`src/api/experiments/controller.rs`, spawned from `src/async_main/runtime_maintenance.rs`); the durable `ArtifactStore` port + `LocalArtifactStore` exist (`src/experiments/artifact_store.rs`, host-local, no desktop-package dependency) and are used on the lease ingest path; and the RunPod credit≈USD cost basis is surfaced on the headline `cost_summary` (`src/api/experiments/execution.rs`). **One item stays deferred:** DP-1 Option B (the optional `opendal`/S3 object-store artifact backend) was intentionally NOT built (see DP-1 below). The "Current State (verified)" section describes the *pre-remediation* state; the `src/api/experiments.rs` monolith it references has since been decomposed into `src/api/experiments/` (`campaign.rs`, `controller.rs`, `crud.rs`, `execution.rs`, `git.rs`, `leases.rs`, `mod.rs`, `subagents.rs`, `tests.rs`, `types.rs`).

> **Status:** Done (landed; opendal object-store backend deferred; see DP-1) · **Priority:** P2 · **Risk:** med · **Effort:** L
> **Depends on:** none · **Blocks:** WS-10 (god-file split inherits this WS's error-taxonomy groundwork), WS-13 (flaky-E2E root-cause shares the worktree-teardown anchor)
> **Owns (symbols/files):**
> - `src/experiments/runner.rs` (remote runner job loop, artifact upload)
> - `src/experiments/adapters.rs` (provider launch/revoke/cost; RunPod credit handling)
> - `crates/thinclaw-experiments/src/lib.rs` — `runner_cost_breakdown`, `provider_hourly_rate_usd`, `estimated_provider_runtime_cost_usd`, `RunnerCostBreakdown`, `ProviderCostEstimate`, `ExperimentRunnerArtifactUpload`, `ExperimentArtifactRef` (cost/artifact policy only; lifecycle DTOs shared with WS-11)
> - `crates/thinclaw-config/src/experiments.rs` — `ExperimentsConfig`
> - `src/api/experiments.rs` — the artifact-retention reaper loop and `lease_artifact` durable-upload path (this WS adds the reaper + upload wiring; the broad god-file *split* is WS-10's, and the controller-reconcile churn is shared — coordinate edits)
>
> **NOTE on shared ownership of `src/api/experiments.rs`:** This 5434-line file is the WS-10 split target. WS-07 only *adds* a self-contained reaper function + small error-taxonomy edits and threads config into the controller spawn. Do those as additive, easily-rebased hunks. Do not begin the structural decomposition here — that is WS-10's, sequenced after this WS lands.

## Vision & Goal

The experiments platform is a genuine, end-to-end autonomous-research engine (planner → mutator → reviewer → local-Docker / remote-GPU trials, cost attribution, promotion to draft PR) that ships *default-off* and is ~88% complete. This workstream closes the three operability gaps that make it untrustworthy to leave running unattended: it makes the artifact-retention knob actually reclaim disk and stale references, makes remote-runner artifacts survive pod teardown so the result trail is durable, and makes the RunPod credit≈USD cost assumption explicit at the surfaces an operator reads when deciding whether a campaign is within budget. Realizing these turns "wired but inert" knobs into a platform an operator can safely trust with a budget and a GPU key.

## Scope

**In scope:**
1. Enforce `default_artifact_retention_days` with a reaper that prunes `experiment_artifact_refs` (and best-effort the underlying local files) past the retention window. Currently the config field exists end-to-end (settings → env → web UI) but **nothing reads it at runtime**.
2. Durable remote-runner artifact upload: the remote runner currently posts `ExperimentRunnerArtifactUpload { fetchable: false, uri_or_local_path: <pod-local path> }`. After pod teardown that is a dead reference. Upload to durable storage and record `fetchable: true` with the durable URI.
3. Surface/flag the RunPod credit≈USD assumption at the cost surfaces (`runner_cost_breakdown` details + the campaign-level `cost_summary` in `src/api/experiments.rs`). The assumption is captured today *only* in `provider_job_metadata` (`normalization: "assumed_1_credit_equals_1_usd"`), which is not on the headline cost surface an operator reads.
4. Small, low-risk error-taxonomy fixes in `src/api/experiments.rs` (the 106 `map_err(|e| ApiError::Internal(...))` flattenings) — only the clear-cut mis-classifications (e.g. not-found/validation collapsed to `Internal`). Broad rewrite is **out**.
5. Document the worktree-teardown race anchor for WS-13 (this WS owns `prepare_campaign_worktree` only insofar as it does *not* fix the race here).

**Out of scope (and which WS owns it):**
- The structural split of `src/api/experiments.rs` (CRUD / reconcile controller / trial execution / planner-mutator-reviewer subagents / lease lifecycle / cost / git) → **WS-10** (god-file overhauls). This doc *describes* the target split (see Decision Points) so WS-10 inherits it, but defers the heavy refactor.
- Root-cause + de-quarantine of `autonomous_campaign_runs_planner_mutator_reviewer_and_docker_trial_end_to_end` (`src/api/experiments.rs:5060`) → **WS-13** (flaky-test / CI). This WS only annotates the suspected mechanism at `prepare_campaign_worktree` (`src/api/experiments.rs:3290`).
- Repo-project supervisor completeness (NeedsPlanning, concurrency limits, merge-retry) → **WS (repo-project supervisor)**, distinct subsystem.
- Desktop cloud-sync wiring (`apps/desktop/backend/src/cloud/*`) → desktop WS. WS-07 *reuses the object-store pattern* but must not depend on the desktop package (dependency direction; see Decision Points).

## Current State (verified)

> **Historical (pre-remediation) snapshot.** The "Half-wired (the WS-07 gaps)" items below were all closed by the landed WS-07 work. Kept for context. This section predates the WS-10 decomposition: `src/api/experiments.rs` no longer exists as a monolith; it is now the `src/api/experiments/` directory (the reaper lives in `controller.rs`, lease/artifact ingest in `leases.rs`, cost surfacing in `execution.rs`), so the `experiments.rs:NNNN` anchors below no longer resolve.

**Wired / production:**
- `ExperimentsConfig` is fully threaded: defined `crates/thinclaw-config/src/experiments.rs:9-16`, default `default_artifact_retention_days: 30` (`:23`), env override `EXPERIMENTS_ARTIFACT_RETENTION_DAYS` (`:39-42`), re-exported `src/config/experiments.rs:3`, resolved into `Config` at `src/config/mod.rs:294`, field at `src/config/mod.rs:146`.
- The controller reconcile loop exists and is spawned only when experiments are enabled: `start_experiment_controller_loop` (`src/api/experiments.rs:762`) on a `DEFAULT_EXPERIMENT_CONTROLLER_TICK_SECS = 30` interval (`:83`); spawned at `src/main.rs:1820-1827` (gated by `config.experiments.enabled`). It is spawned with **only `Arc<dyn Database>`** — no config is threaded in.
- Remote runner job loop is real and complete: `run_remote_runner` (`src/experiments/runner.rs:19`) clones the repo, runs prepare/run, extracts metrics, posts status/event/artifact/complete back to the gateway lease endpoints.
- Artifact ingest is wired: `lease_artifact` (`src/api/experiments.rs:2690`) verifies the lease token, appends an `ExperimentArtifactRef`, and persists via `replace_experiment_artifacts`. DB methods exist on both backends (`crates/thinclaw-db/src/lib.rs:933-946`, postgres `crates/thinclaw-db/src/postgres_store/experiments.rs:684`, libSQL schema `crates/thinclaw-db/src/libsql_migrations.rs:424`).
- Cost attribution is real and tested: `runner_cost_breakdown` (`crates/thinclaw-experiments/src/lib.rs:2539`), `estimated_provider_runtime_cost_usd` (`:2616`), `provider_hourly_rate_usd` (`:2645`). The RunPod branch (`:2650-2666`) already records `native_currency: "runpod_credits"` and `normalization: "assumed_1_credit_equals_1_usd"` in the estimate. Test `runpod_cost_is_normalized_from_credits` (`:3401`) asserts these fields.

**Half-wired (the WS-07 gaps):**
- `default_artifact_retention_days` is a **no-op at runtime.** The only runtime reference is the web settings form label (`src/channels/web/static/app.js:10788`). No reaper, no read in the controller loop, nothing prunes `experiment_artifact_refs`. (Confirmed via grep across `src/` and `crates/`.)
- Remote-runner artifacts are written with `fetchable: false` and a **pod-local path** in every post: `run_log` (`src/experiments/runner.rs:154-165`), `summary_json` (`:172-187`), and the failure log (`:244-258`). All set `fetchable: false`. After pod teardown the `uri_or_local_path` is a dead reference. The completion manifest also stores pod-local paths (`log_preview_path`, `checkout_dir`, `summary_json_path`, `:197-202`).
- The credit≈USD assumption is **not surfaced on the headline cost surface.** `record_runner_completion`-style finalization (`src/api/experiments.rs:2917-2948`) puts `runner_cost.details` into `artifact_manifest_json.cost_breakdown.runner` and the campaign `cost_summary` (`:2941-2947`) carries only `total_usd`/`llm_usd`/`runner_usd` — no `native_currency`/`normalization`/`estimated` flag. The assumption only reaches `provider_job_metadata` via the overlay merge at `:2935-2937`.

**Error-taxonomy debt (low-risk subset for this WS):**
- 106 of 139 `map_err` calls in `src/api/experiments.rs` are `map_err(|e| ApiError::Internal(e.to_string()))` — DB errors, validation, not-found and serialization all flatten to `Internal`. `ApiError` (`src/api/error.rs:10-42`) has the right variants (`InvalidInput`, `SessionNotFound`, `Unavailable`, `FeatureDisabled`, `Serialization`, `UuidParse`, `Internal`) and a clean machine-code map (`error_code`, `:46-57`). Many of the 106 are genuinely internal DB failures and are *correctly* `Internal`; only a minority are mis-classified.

**Dead-reference / drift (none to ERASE here):** No genuinely drifted duplicate experiments code found. The cost/artifact policy lives cleanly in `thinclaw-experiments`; the API file is large but coherent. This WS is build-the-vision, not erase.

**WS-13 coordination anchor:** the quarantined E2E (`src/api/experiments.rs:5060`, ignored `:5060`, added in commit `64b9572f`) fails with `Internal("No such file or directory (os error 2)")` from a worktree git op spawning after the worktree path vanished mid-trial. The teardown path is `prepare_campaign_worktree` (`src/api/experiments.rs:3290-3317`): it does `worktree remove --force` → `worktree prune` → `remove_dir_all`, and trial completion restores the worktree to a clean committed state. The race is between trial-completion cleanup and the next reconcile preparing the worktree.

## Decision Points

**DP-1 — Durable artifact storage: which backend? (BUILD, do not gate.)**
The finding says "reuse desktop cloud providers / object store pattern." But `apps/desktop/backend/src/cloud/provider.rs:178` (`trait CloudProvider`, `async fn put(&self, key, data)`) lives in the **desktop app package**, which the root crate and `thinclaw-experiments` must not depend on (dependency direction — `docs/CRATE_OWNERSHIP.md`).
- **Option A (recommended): host-side durable copy in `lease_artifact`.** The runner posts the artifact bytes (or the gateway pulls from a runner-served path before completion); the *gateway host* writes them under a durable, operator-controlled root (e.g. `<workspace>/experiments/artifacts/<trial_id>/<artifact_id>`), then records `fetchable: true` + the durable path. This keeps all storage logic on the gateway side where the encrypted secrets and config already live, needs no new dependency, and works for the default (local) deployment. RunPod/Vast pods can `curl` the artifact bytes to the lease `/artifact` endpoint as a multipart/base64 body instead of a path. **This is the realize-the-vision path with the least coupling.**
- **Option B: optional S3-compatible object store via `opendal`.** `s3.rs` already uses `opendal::Operator` (`apps/desktop/backend/src/cloud/providers/s3.rs:7-8`). Add an `opendal`-backed durable store *in the root or a small `thinclaw-experiments`-adjacent crate* (NOT importing the desktop crate) behind an `ExperimentsConfig` opt-in (`durable_artifact_store_url`). Heavier; defer to a follow-up.
- **Recommendation: ship Option A now** (host-side durable copy + `fetchable: true`), structured behind a small `ArtifactStore` port so Option B can slot in later without touching call sites. Do **not** feature-gate the capability off — durability is core to trusting unattended runs.
- **Outcome (landed):** Option A shipped: the `ArtifactStore` trait + host-local `LocalArtifactStore` live in `src/experiments/artifact_store.rs`, and the module comment notes that an `opendal`/S3 object-store backend "can slot in behind this same port later." **Option B (the `opendal` object-store backend) is deliberately DEFERRED** and remains open: it pulls a heavy dependency and needs a `cargo-deny` review before adoption, so it was not built. The `ArtifactStore` port makes adding it later a non-breaking change.

**DP-2 — Reaper home: controller loop vs dedicated task. (BUILD.)**
- **Option A (recommended): a dedicated reaper loop** mirroring `spawn_pricing_sync` (`src/llm/pricing_sync.rs:234`) and the existing controller loop. Spawn it next to the controller in `src/main.rs:1820-1827`, also gated on `config.experiments.enabled`, threading `config.experiments.default_artifact_retention_days`. A daily-ish interval (not the 30s controller tick) is correct for retention.
- **Option B: fold the prune into `reconcile_experiments_once`.** Rejected — it muddies the reconcile controller (already a WS-10 split candidate) and runs 2× too often.
- **Recommendation: Option A.** Add `start_experiment_artifact_reaper_loop(store, retention_days)` as a self-contained function so WS-10's later split has a clean unit to relocate.

**DP-3 — RunPod credit≈USD: gate or just surface? (SURFACE, do not gate.)**
The assumption is already computed and stored in metadata. The fix is to *propagate the flag to the headline surface*, not to block on it. **Recommendation: surface** `estimated`, `native_currency`, and `normalization` into the campaign `cost_summary` and runner cost `details`, and add a one-line note in `docs/RESEARCH_AND_EXPERIMENTS.md`. No config gate — gating would make the platform refuse to estimate cost it can reasonably estimate.

**DP-4 — Error-taxonomy scope. (FIX small subset only.)**
**Recommendation:** fix only unambiguous mis-classifications (not-found → `SessionNotFound`/a not-found message, input validation → `InvalidInput`, UUID/serde already have `#[from]`). Leave DB-failure `Internal` mappings as-is. The wholesale flattening cleanup belongs to WS-10's split where call sites get reorganized anyway.

## Tasks

- [x] **T1: Add the artifact-retention reaper loop**
  - **Files:** `src/api/experiments.rs` (add `start_experiment_artifact_reaper_loop` + helper, near the controller loop ~`:762`); `src/main.rs` (spawn next to `:1820-1827`).
  - **Change:** New `pub async fn start_experiment_artifact_reaper_loop(store: Arc<dyn Database>, retention_days: u32)`. On a daily interval (constant `DEFAULT_ARTIFACT_REAPER_TICK_SECS = 86_400`, tick-first like the controller), call a `reap_expired_artifacts_once(&store, retention_days)` that: lists campaigns → trials (`list_experiment_trials`) → artifacts (`list_experiment_artifacts`); for each artifact older than `now - retention_days` (`ExperimentArtifactRef.created_at`), best-effort delete the local file when `!fetchable` *and* the path is under the experiments artifact root, then re-persist the surviving set via `replace_experiment_artifacts`. Treat `retention_days == 0` as "disabled" (skip). In `main.rs`, when `config.experiments.enabled`, `tokio::spawn(start_experiment_artifact_reaper_loop(Arc::clone(&db), config.experiments.default_artifact_retention_days))` with a `tracing::info!` mirroring the controller log.
  - **Acceptance:** Unit test `reap_expired_artifacts_removes_only_expired` using `crate::testing::test_db()` (pattern: existing `#[tokio::test]` tests ~`:4581`+) seeds two artifacts (one `created_at` 40d ago, one fresh) with a 30d window and asserts only the stale one is pruned from `list_experiment_artifacts`. A `retention_days = 0` case prunes nothing.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw --lib experiments::tests::reap` (or the crate the API file compiles in — `src/api/experiments.rs` is in the root `thinclaw` package); `cargo clippy --all-targets -- -D warnings`.

- [x] **T2: Define a durable `ArtifactStore` port + host-local implementation**
  - **Files:** new `src/experiments/artifact_store.rs` (declare in `src/experiments/mod.rs` façade with `pub mod artifact_store;` + a narrow `pub use`).
  - **Change:** `pub trait ArtifactStore: Send + Sync { async fn put(&self, trial_id: Uuid, artifact_id: Uuid, kind: &str, bytes: &[u8]) -> anyhow::Result<String /* durable uri/path */>; }` plus `LocalArtifactStore { root: PathBuf }` writing under `<root>/<trial_id>/<artifact_id>` and returning the absolute path. Mirror the shape of `apps/desktop/backend/src/cloud/provider.rs:178` (`trait CloudProvider::put`) but own it in this crate so there is **no** dependency on the desktop package. Keep it minimal (no list/delete — the reaper in T1 handles deletion by path).
  - **Acceptance:** `LocalArtifactStore::put` round-trips bytes to disk and the returned path exists; unit test in the new module.
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw --lib experiments::artifact_store`; `cargo clippy --all-targets -- -D warnings`.

- [x] **T3: Make remote-runner artifacts durable (close the dead-reference gap)**
  - **Files:** `src/experiments/runner.rs` (post artifact bytes, not just paths); `src/api/experiments.rs` `lease_artifact` (`:2690-2725`); `crates/thinclaw-experiments/src/lib.rs` `ExperimentRunnerArtifactUpload` (`:727-733`).
  - **Change:** Extend `ExperimentRunnerArtifactUpload` with an optional inline payload (`#[serde(default, skip_serializing_if = "Option::is_none")] pub content_base64: Option<String>`). In `runner.rs`, for `run_log`/`summary_json`/failure-log uploads (`:154-258`), read the file and attach base64 content; keep `uri_or_local_path` as the pod-local breadcrumb but stop relying on it. In `lease_artifact`, when `content_base64` is present, decode and call `ArtifactStore::put` (host-local from T2, rooted under the campaign workspace e.g. `<workspace>/.thinclaw/experiments/artifacts`), then store the **durable** path with `fetchable: true`; when absent, preserve today's behavior (`fetchable` as posted). Update the completion manifest paths (`:197-202`) note to reference durable artifacts where available.
  - **Acceptance:** Extend an existing lease E2E (the non-flaky `launch_campaign_baseline_runs_local_docker_trial_end_to_end` family, `:4581`) or add a focused unit test asserting that an upload with `content_base64` produces an `ExperimentArtifactRef { fetchable: true, .. }` whose path exists on disk. Existing remote-runner tests stay green.
  - **Effort:** L
  - **Verification:** `cargo test -p thinclaw --lib experiments`; `cargo test -p thinclaw-experiments`; `cargo clippy --all-targets -- -D warnings`. (Feature matrix: touches default/desktop/full where experiments compile; experiments code is not in `edge`/`light` — confirm with `cargo check --no-default-features --features edge` still green since these paths gate on `config.experiments.enabled` but compile unconditionally in the root crate.)

- [x] **T4: Surface the RunPod credit≈USD assumption on the headline cost surface**
  - **Files:** `src/api/experiments.rs` finalization block (`:2925-2948`); optionally `crates/thinclaw-experiments/src/lib.rs` `RunnerCostBreakdown`/`runner_cost_breakdown` (`:2486-2595`).
  - **Change:** Propagate `estimated`, `native_currency`, and `normalization` from `runner_cost.details` (and `provider_metadata_overlay`) into the campaign-level `cost_summary` JSON (`:2941-2947`) — e.g. add `"runner_cost_basis": { "estimated": <bool>, "native_currency": <str|null>, "normalization": <str|null> }`. The data already exists in `runner_cost.details` (`crates/thinclaw-experiments/src/lib.rs:2564-2572`); no recomputation needed. Ensure the `cost_breakdown.runner` block already carries it (it does — just lift the key fields up to `cost_summary`).
  - **Acceptance:** A finalization unit test asserts that after a RunPod trial completes, the campaign `metadata.cost_summary` contains `normalization: "assumed_1_credit_equals_1_usd"` (or `estimated`/`native_currency`). Existing `runpod_cost_is_normalized_from_credits` (`:3401`) stays green.
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw-experiments`; `cargo test -p thinclaw --lib experiments`; `cargo clippy --all-targets -- -D warnings`.

- [x] **T5: Low-risk error-taxonomy fixes (clear-cut subset only)**
  - **Files:** `src/api/experiments.rs` (the not-found / validation `map_err(|e| ApiError::Internal(...))` sites only).
  - **Change:** Re-classify the unambiguous cases: lease/trial/campaign "not found" lookups that currently flatten to `Internal` → the existing not-found path / message helpers already imported (`experiment_lease_not_found_message`, `experiment_campaign_not_found_message`, `:38-46`); caller-input validation → `ApiError::InvalidInput`. Leave genuine DB-failure mappings as `Internal`. Do **not** touch the reconcile-controller internals beyond these point fixes (WS-10 owns the restructure).
  - **Acceptance:** No behavioral regression in existing experiments tests; `error_code()` for the touched paths now returns the precise code (spot-checked in a test or by reading the changed sites). Net `map_err(... Internal ...)` count drops only for the re-classified sites; do not mass-rewrite.
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw --lib experiments`; `cargo clippy --all-targets -- -D warnings`.

- [x] **T6: Annotate the worktree-teardown race for WS-13 (no fix here)**
  - **Files:** `src/api/experiments.rs` `prepare_campaign_worktree` (`:3290-3317`) — comment only.
  - **Change:** Add a `// WS-13:` comment documenting the suspected race (cleanup `remove_dir_all` vs next reconcile's `worktree remove`/`prune`/`create_dir_all`) and the observed `Internal("No such file or directory (os error 2)")`. Do not change behavior — root-cause + de-quarantine is WS-13.
  - **Acceptance:** Comment present; no logic change; quarantined test still `#[ignore]`.
  - **Effort:** S
  - **Verification:** `cargo build -p thinclaw`; `cargo fmt --check`.

- [x] **T7: Update docs**
  - **Files:** `docs/RESEARCH_AND_EXPERIMENTS.md` (retention reaper, durable remote artifacts, the credit≈USD cost basis note); check `FEATURE_PARITY.md` for a coordinated status note per CLAUDE.md "Common Update Triggers" (experiments/runner change).
  - **Change:** Document that `default_artifact_retention_days` is now enforced by a reaper; that remote-runner artifacts are uploaded to durable host storage (`fetchable: true`); and that RunPod cost is normalized from credits under a `1 credit ≈ 1 USD` assumption now surfaced in `cost_summary`. Keep it thin per the documentation rules.
  - **Acceptance:** Docs reflect shipped behavior; no stale "no-op" implication remains.
  - **Effort:** S
  - **Verification:** Manual read; ensure no brittle counts added.

## Best Practices (workstream-specific)

- **Mirror the existing background-loop pattern.** The reaper should look like `start_experiment_controller_loop` (`src/api/experiments.rs:762-779`) and `spawn_pricing_sync` (`src/llm/pricing_sync.rs:234`): tick-first `interval`, match-and-`tracing::warn!` on error, never panic the task, gate the spawn on `config.experiments.enabled` in `src/main.rs:1820-1827`.
- **Keep cost/artifact *policy* in `thinclaw-experiments`, I/O in the root.** Cost math (`runner_cost_breakdown`, `provider_hourly_rate_usd`) and DTOs stay in the crate; disk/network/store side effects stay in `src/experiments/*` and `src/api/experiments.rs` (`docs/CRATE_OWNERSHIP.md` direction). Do not import the desktop `apps/desktop/backend` cloud package.
- **Re-use the lease verification + `replace_experiment_artifacts` round-trip** already in `lease_artifact` (`:2698-2716`) — list, mutate, replace. There is no single-artifact delete method, so the reaper prunes by re-persisting the surviving set, same as ingest appends.
- **Add new submodules as façade-declared modules.** `src/experiments/mod.rs` is a façade (`pub mod adapters; pub mod runner; pub use thinclaw_experiments::*;`). Add `pub mod artifact_store;` there; do not bloat `runner.rs` with storage concerns.
- **Keep additive hunks rebase-friendly.** Because WS-10 will split `src/api/experiments.rs`, prefer new top-level functions over edits threaded deep into existing ones, so the split can relocate them wholesale.

## Common Pitfalls

- **Cross-channel-style "fix one copy of N" trap.** The audit's recurring failure mode (split_message fixed in 1 of 4 WASM copies, AUDIT-FINDINGS §5) applies here to the *two* DB backends: any artifact-deletion or schema assumption must hold for **both** Postgres (`crates/thinclaw-db/src/postgres_store/experiments.rs:684`) and libSQL (`crates/thinclaw-db/src/libsql_migrations.rs:424`). The reaper goes through the `Database` trait (`crates/thinclaw-db/src/lib.rs:933-946`) so this is handled — do not reach past the trait into one backend.
- **`fetchable: false` is silent.** The current dead-reference bug produces no error — the artifact row persists, the path just doesn't exist post-teardown. Tests must assert the path *exists*, not merely that a row was written.
- **Reaper deleting files outside the artifact root.** Local artifact paths are arbitrary today (pod `run.log` paths). Only `remove_dir_all`/`remove_file` paths confirmed under the experiments artifact root; never delete based on an unvalidated `uri_or_local_path` from a runner.
- **Over-reaching the error-taxonomy fix.** Re-classifying a DB failure as `InvalidInput` would mislabel a real internal error. Touch only not-found/validation that is unambiguous.
- **Editing the reconcile controller body.** That is the WS-10 split target; deep edits here will collide. Keep WS-07 changes additive and at the edges.
- **Don't gate cost estimation off.** The credit≈USD assumption should be *flagged*, not used as a reason to refuse estimation (DP-3).

## Multi-Worker Execution Plan (ultracode)

- **Worker decomposition:**
  - *Sequential spine:* T2 (`ArtifactStore` port) → T3 (durable upload uses it). T1 (reaper) is independent and can start in parallel with T2/T3 (it only needs the `Database` trait, already stable).
  - *Parallel fan-out (independent files / additive):* Worker-A = T1 reaper (+ `main.rs` spawn). Worker-B = T2+T3 durable artifacts (runner + `lease_artifact` + DTO). Worker-C = T4 cost-surface + T5 error-taxonomy + T6 comment (all small, same file `src/api/experiments.rs` — keep on one worker to avoid intra-file conflicts). Worker-D = T7 docs (after A/B/C land).
  - T4/T5/T6 all touch `src/api/experiments.rs`; assign them to **one** worker (Worker-C) to serialize intra-file edits. T1's `main.rs` edit and reaper function are in distinct regions from C's edits — low conflict.
- **Isolation:** Worker-A (reaper, edits `main.rs` + new region of `experiments.rs`) and Worker-B (runner.rs + DTO crate + `lease_artifact`) and Worker-C (cost/error/comment in `experiments.rs`) all touch `src/api/experiments.rs` in different regions → use **git worktree isolation per worker** and a sequenced merge (A → C → B, or land C first since its edits are smallest), then rebase. Worker-D (docs) needs no isolation, runs last.
- **Workflow shape (implement → verify → review → fix):**
  1. *Implement* fan-out: A, B, C in parallel worktrees.
  2. *Verify gate* per worker (commands below) before merge.
  3. *Integrate:* merge C → A → B sequentially, re-running the gate after each merge (intra-file rebase risk).
  4. *Review:* `/code-review` on the combined diff (focus: reaper path-safety, base64 size limits, cost-surface JSON shape, error-code correctness).
  5. *Fix:* address review findings; re-run gate.
  6. *Docs:* Worker-D lands T7 on the integrated branch.
- **Verification gate (exact):**
  - `cargo fmt --all`
  - `cargo clippy --all --benches --tests --examples --all-features -- -D warnings` (per CLAUDE.md; note CI currently omits `--all-targets` — run it locally to catch test/bench warnings)
  - `cargo test -p thinclaw-experiments`
  - `cargo test -p thinclaw --lib experiments` (the API + runner tests; `src/api/experiments.rs` compiles in the root `thinclaw` package)
  - `cargo check --no-default-features --features edge` (confirm the build-profile matrix stays green; experiments paths compile but are inert under `edge`)
  - `/ship` for the full Rust quality gate before PR.
  - **DB/Docker prerequisites:** the reaper and durable-artifact unit tests use `crate::testing::test_db()` (libSQL/in-memory by default — no Docker needed). The *existing* heavy Docker E2E (`:5060`) stays `#[ignore]` and is **not** part of this gate (it is WS-13). For Postgres-backed parity (optional), apply `migrations/V*.sql` into `thinclaw_test` and run with `DATABASE_URL` set, per CLAUDE.md local-dev notes; if Docker is flaky, check `df -h /System/Volumes/Data` first.

## Definition of Done

- [x] Reaper enforces `default_artifact_retention_days`: stale `experiment_artifact_refs` are pruned on schedule, with a passing unit test (incl. `retention_days = 0` = disabled) and spawn gated on `config.experiments.enabled`.
- [x] Remote-runner `run_log`/`summary_json`/failure-log artifacts land in durable host storage with `fetchable: true` and an on-disk path; no `fetchable: false` pod-local-only references remain on the success path. Test asserts the durable path exists.
- [x] `ArtifactStore` port + `LocalArtifactStore` live in `src/experiments/artifact_store.rs`, declared via the `src/experiments/mod.rs` façade, with **no** dependency on the desktop cloud package.
- [x] RunPod credit≈USD assumption (`estimated`/`native_currency`/`normalization`) is visible in the campaign `cost_summary` and runner cost `details`; test asserts it.
- [x] Clear-cut error-taxonomy mis-classifications in `src/api/experiments.rs` corrected to precise `ApiError` variants; DB-failure `Internal` mappings left intact; no mass rewrite (that is WS-10).
- [x] `prepare_campaign_worktree` carries a `// WS-13:` race-mechanism annotation; the quarantined E2E remains `#[ignore]` (de-quarantine owned by WS-13).
- [x] `docs/RESEARCH_AND_EXPERIMENTS.md` updated (reaper, durable artifacts, cost basis); `FEATURE_PARITY.md` checked for a coordinated status note.
- [x] All four decision points resolved as recorded (DP-1 Option A, DP-2 Option A, DP-3 surface-not-gate, DP-4 small subset).
- [x] Verification gate green: `cargo fmt`, `cargo clippy --all ... -D warnings`, `cargo test -p thinclaw-experiments`, `cargo test -p thinclaw --lib experiments`, `cargo check --no-default-features --features edge`, `/ship`.
