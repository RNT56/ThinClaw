# WS-13 — Test & CI Infrastructure

> **Status:** Landed (2026-06, commit `1ed26495`) · **Priority:** P1 · **Risk:** medium · **Effort:** L
> **Depends on:** WS-01 (owns the `--all-targets` clippy flag + the wasmtime-wasi/`deny.toml` fix that WS-13 only *verifies* is reflected across the matrix), WS-02 (owns the `schema_divergence` fail-not-skip change and the dual-backend `db_contract` punctuation assertion whose *gating* WS-13 wires)
> **Blocks:** none (this is the terminal verification/CI workstream; other WS's gates run through the jobs it adds/confirms)
> **Owns (symbols/files):**
> - `.github/workflows/ci.yml` — the **test-execution jobs** only: the new `nightly-ignored` job (this WS creates it), and the `db-contract-libsql` / `db-contract-postgres` / `schema-divergence` job **gating/required-status** (lines 640-723). NOTE: the `codestyle` clippy line (52) and `feature-matrix` clippy line (121) are **edited by WS-01**; WS-13 only *asserts* the flag landed in both — see Task T6.
> - `.github/workflows/nightly.yml` — **new** scheduled workflow file (this WS creates it), if we split nightly out of `ci.yml` (see Decision Point 1).
> - The root-cause tracking issue for the quarantined `autonomous_campaign_runs_planner_mutator_reviewer_and_docker_trial_end_to_end` race (this WS opens it; the *fix* to `src/api/experiments.rs` is owned by **WS-07 Experiments**, see dependency note).
> - `scripts/build-all.sh` — extending the WASM artifact build to cover the `tools-src/*` crates (or printing clearly that it does not); see Task T7.
> - The per-crate channel/tool **CI matrix** in `.github/workflows/ci.yml` — expanding it to build-check the 13 `channels-src` WASM shims on `wasm32-wasip2`; see Task T7.
> - NOTE: `tests/db_contract/support.rs`, `tests/schema_divergence.rs`, `tests/db_contract/conversations.rs` are **WS-02-owned**. WS-13 does **not** edit them — it only wires/asserts the CI jobs that run them.

> ---
>
> ## Completion banner
>
> **This workstream landed (commit `1ed26495`). Do not execute the CI-file tasks below as pending work.** The "Current State (verified)" section is a historical snapshot from before this WS; its "no schedule trigger" and "clippy escapes --all-targets" claims are now stale (corrected inline). Verified end-state in the working tree:
>
> - **T1**: `.github/workflows/nightly.yml` exists with `on: schedule: - cron: "0 6 * * *"` and the `nightly-docker-e2e`, `nightly-network-smoke`, and `nightly-heartbeat` jobs, selecting `#[ignore]`d tests by explicit target (not a blanket `-- --ignored`).
> - **T5**: the prerequisites matrix (this doc's table) is the canonical reference.
> - **T6**: both clippy invocations carry `--all-targets`, at `ci.yml:66` (`codestyle`) and `ci.yml:135` (`feature-matrix`).
> - **T7**: `scripts/build-all.sh` prints the explicit "Skipping tools-src/* WASM tool crates" notice (the doc's accepted alternative), and the per-crate CI matrix build-checks the `channels-src` WASM shims on `wasm32-wasip2` (`ci.yml:772+`).
> - **T3 / T4 (CI-file portion)**: `db-contract-libsql`, `db-contract-postgres`, and `schema-divergence` jobs exist and `schema-divergence` provisions `DATABASE_URL`. **Not verifiable from the working tree:** whether those jobs are set as required-for-merge branch-protection checks (a GitHub repo setting).
> - **T2**: opening the worktree/Docker-lifecycle-race tracking issue is a GitHub artifact and cannot be confirmed from the working tree; left unchecked here. The race fix itself is WS-07-owned.
>
> **Genuinely still open (do NOT mark done):**
> - Coverage gate: `cargo llvm-cov --all-features --lib` (`ci.yml:881`) still uses `--lib` and has **no `--fail-under`** threshold, so no coverage floor is enforced.
> - `cargo deny` does **not** scan the `channels-src/` and `tools-src/` sub-workspace lockfiles (`ci.yml` runs `cargo deny check` at the root and desktop only). These two gaps are outside WS-13's task list (T1-T7) and remain unaddressed.
>
> ---

## Vision & Goal

ThinClaw already has a broad, disciplined CI matrix (7 build profiles × 3 OSes, dual-backend DB contract jobs, ACP/host-runtime/deploy smokes, a desktop-companion job, and fuzzing). The gap is **coverage of the heavy real-world paths the audit found are never exercised in CI**: 14 first-party `#[ignore]`d tests (Docker E2Es, live desktop-autonomy smokes, a real-network WASM smoke, a heartbeat integration test, a WebUI provider diagnostic) plus one *quarantined* flaky autonomous-campaign E2E that masks a genuine worktree/Docker lifecycle race. This workstream makes those paths run on a schedule (so regressions surface before users hit them), turns the flaky-test quarantine into a tracked root-cause, and closes the two CI-wiring loopholes the audit named: silently-passing DB tests when no database is present, and clippy escaping `-D warnings` on test/example/bench code.

## Scope

**In scope:**
1. A new **nightly scheduled CI job** that runs the 13 runnable `#[ignore]`d tests via `--ignored`, with a documented Docker/Postgres/LLM-auth prerequisites matrix and graceful self-skip where a prerequisite (auth, desktop session) cannot be provisioned on a hosted runner.
2. **Open a tracking issue** for the quarantined `autonomous_campaign_..._end_to_end` worktree/Docker lifecycle race (`src/api/experiments.rs:5060`) and document the fix direction (the *fix itself* belongs to WS-07 Experiments — see dependency note).
3. **Confirm dual-backend `db_contract` gating** in CI: the `db-contract-libsql` and `db-contract-postgres` jobs (`ci.yml:640-689`) both run the WS-02 punctuation assertion and both stay required-for-merge so the parity assertion cannot be bypassed.
4. **Wire fail-not-skip in CI** for `schema_divergence` (and confirm the `db_contract` Postgres job always has `DATABASE_URL`): ensure the `schema-divergence` job (`ci.yml:691-723`) always provisions `DATABASE_URL` so WS-02's panic-on-missing-URL becomes a real gate, and that the job is required-for-merge. (The *assertion/panic* is WS-02; the *CI wiring/gating* is here.)
5. **Verify the `--all-targets` clippy gate** from WS-01 is reflected across **both** clippy invocations (`codestyle` line 52 and `feature-matrix` line 121) and therefore across all 7 profiles.

**Out of scope (and owning WS):**
- The `--all-targets` flag edit itself and the `await_holding_lock` fix it surfaces at `crates/thinclaw-config/src/secrets.rs:144` — **WS-01**. WS-13 only verifies the flag landed (T6).
- The `cargo deny` / wasmtime-wasi `36.0.10→36.0.11` bump and the stale `deny.toml:22-24` ignores — **WS-01**.
- The `schema_divergence` test body change (column types/nullability/indexes + the `expect()`-on-missing-`DATABASE_URL` panic) and the new `db_contract` punctuation assertion — **WS-02** (`tests/schema_divergence.rs`, `tests/db_contract/*`). WS-13 does not touch those files.
- The actual fix to the worktree-lifecycle race in `src/api/experiments.rs` — **WS-07 Experiments** (this WS opens the issue and scopes the direction; experiments code is WS-07-owned).

## Current State (verified)

**CI workflows (`.github/workflows/`):**
- `ci.yml` triggers on `workflow_dispatch`, `pull_request`, and `push` to `main` (lines 2-7). **RESOLVED (T1):** the nightly gap is closed. `.github/workflows/nightly.yml` now declares `on: schedule: - cron: "0 6 * * *"` and its jobs run `#[ignore]`d tests with `-- --ignored`. (Historical snapshot: at audit time there was no `schedule:` trigger and no `--ignored` run in CI.)
- `fuzz.yml` is the only other recurring job; it triggers on `push` to `main` (not cron) and uses a `strategy.matrix.target` fan-out (lines 11-18) — a clean **pattern to copy** for the nightly matrix.
- **Clippy `--all-targets`: RESOLVED (T6).** Both invocations now carry `--all-targets`: `codestyle` runs `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` (`ci.yml:66`) and `feature-matrix` runs `cargo clippy --locked --workspace --all-targets ${{ matrix.cargo-args }} -- -D warnings` (`ci.yml:135`), so `#[cfg(test)]`, `examples/`, and `benches/` code is now under `-D warnings` across all profiles. (Historical snapshot: at audit time neither invocation carried `--all-targets`, and the `await_holding_lock` in `secrets.rs` was hiding under test code.)
- **DB contract jobs already run on both backends:** `db-contract-libsql` (`ci.yml:640-654`, sets `DATABASE_BACKEND=libsql`, no external DB) and `db-contract-postgres` (`ci.yml:656-689`, spins a `pgvector/pgvector:pg17` service, sets `DATABASE_BACKEND=postgres` + `DATABASE_URL`, enables the `vector` extension, runs with `--test-threads=1`). The suite *body* selects the backend via `tests/db_contract/support.rs:22-40` (`contract_db_or_skip`). — **WIRED (both backends run); the missing piece is confirming the new WS-02 punctuation assertion runs in both and both jobs stay required.**
- **`schema-divergence` job** (`ci.yml:691-723`) spins Postgres, sets `DATABASE_URL` (line 709), enables pgvector, and runs `cargo test --test schema_divergence --no-default-features --features "postgres libsql" -- --nocapture --test-threads=1`. — **WIRED, but** the test *body* silently skips when `DATABASE_URL` is absent (`tests/schema_divergence.rs:35-38` `let Some(base_url) = ... else { eprintln!(...); return; }`) — WS-02's T4 flips that to a panic; WS-13 must ensure the job keeps `DATABASE_URL` set (it currently does) so the panic is a true gate.

**The 14 runnable `#[ignore]`d first-party tests** (verified count via `grep -rn "#[ignore" --include="*.rs"` excluding `patches/`):

| # | Test | Location | Prereqs |
|---|---|---|---|
| 1 | `repo_executor_dispatches_a_real_sandbox_container` | `tests/repo_project_docker_e2e.rs:160-161` | Docker + local `thinclaw-worker:latest` image |
| 2 | `interactive_worker_container_smoke_completes_after_done_prompt` | `tests/docker_sandbox_smoke.rs:251-252` | Docker + `thinclaw-worker:latest` |
| 3 | `claude_code_bridge_container_smoke_completes_one_shot_when_auth_available` | `tests/docker_sandbox_smoke.rs:305-306` | Docker + `thinclaw-worker:latest` + Claude auth (`ANTHROPIC_API_KEY` or OAuth; self-skips at lines 307-312 if absent) |
| 4 | `codex_code_bridge_container_smoke_completes_one_shot_when_auth_available` | `tests/docker_sandbox_smoke.rs:360-361` | Docker + `thinclaw-worker:latest` + Codex/OpenAI auth (`OPENAI_API_KEY` or `~/.codex`; self-skips at 365-373 if absent) |
| 5-10 | `*whole_machine_admin_live_desktop_smoke` ×3 OS, `*dedicated_user_live_desktop_smoke` ×3 OS | `tests/desktop_autonomy_live_smoke.rs:313/320/327/363/370/377` | `THINCLAW_LIVE_DESKTOP_SMOKE=1` (`live_smoke_enabled()` at line 13-15); dedicated-user variants also need `THINCLAW_LIVE_DEDICATED_USERNAME`; require a real privileged desktop session — **not provisionable on a vanilla hosted runner** (most paths self-skip or report a blocking reason) |
| 11 | `test_dedicated_runtime_real_http` | `crates/thinclaw-channels/src/wasm/wrapper.rs:5671-5672` | Outbound network (hits `https://api.telegram.org`) |
| 12 | `live_webui_provider_model_discovery_report` | `src/channels/web/server.rs:1931-1932` | Live provider credentials for the configured LLM providers (diagnostic) |
| 13 | `test_heartbeat_end_to_end` | `tests/heartbeat_integration.rs:18-19` | `#![cfg(feature = "postgres")]`; running database (`DATABASE_URL`) + LLM credentials; loads `.env` itself (line 22) |

Plus the **quarantined** (not a candidate for the always-on nightly until root-caused):

| # | Test | Location | State |
|---|---|---|---|
| 14 | `autonomous_campaign_runs_planner_mutator_reviewer_and_docker_trial_end_to_end` | `src/api/experiments.rs:5059-5061` | `#[ignore]`d in commit `64b9572f` (2026-06-14). Heavy autonomous Docker E2E (planner→mutator→reviewer→two real local-Docker trials over a git worktree). Fails intermittently under the main-only `--all-features --lib` coverage job with `Internal("No such file or directory (os error 2)")`. |

**Quarantined-race code reality (for T2 root-cause direction):**
- The campaign worktree is created/torn down in `prepare_campaign_worktree` (`src/api/experiments.rs:3290-3317`): it does `git worktree remove --force` → `git worktree prune` → `tokio::fs::remove_dir_all(worktree_path)` when the path already exists, then re-creates the parent. The three steps are **not atomic** and the function ignores the result of the `git` calls (`let _ = ...`).
- After each trial, `finalize_trial` (`src/api/experiments.rs:2886+`) calls `restore_campaign_worktree_after_trial` (invoked at line 3027); on restore failure it pauses the campaign (lines 3059-3063).
- Trial git ops run via `git_output(&worktree, ...)` / `git_output_raw` against the worktree path (e.g. `git_changed_files` at `:3329`, `push_experiment_branch` at `:3319`). The `os error 2` is one of these spawning a `git` against a worktree directory that a *concurrent* prepare/restore step has already removed — a **worktree-lifecycle race**, not pure CI flake. The autonomous path (`launch_next_trial_if_ready` at `:1020`) chains baseline→mutate→trial→restore over the *same* worktree, so a late git op from one phase can overlap teardown/re-prepare of the next. — **CONFIRMED latent correctness bug.**

## Decision Points

1. **Nightly: separate `nightly.yml` workflow vs a cron-triggered job inside `ci.yml`.**
   - Options: (a) **new `.github/workflows/nightly.yml`** with `on: schedule: [{cron: ...}]` + `workflow_dispatch`; (b) add a `schedule:` trigger to `ci.yml` and gate the heavy job with `if: github.event_name == 'schedule'`.
   - Trade-offs: (a) keeps the PR-path CI fast and uncluttered, gives the heavy E2Es their own clearly-labelled run history, and avoids accidentally running Docker E2Es on every PR; the cost is a small amount of duplicated checkout/toolchain boilerplate. (b) reuses caching but risks the `if:` guard drifting and makes the already-large `ci.yml` harder to read (it's at ~843 lines).
   - **Recommendation: (a) — a dedicated `nightly.yml`.** It mirrors the repo's existing split (`fuzz.yml` is already its own file) and matches the CLAUDE.md hygiene preference for focused, single-responsibility units. Trigger `schedule` (e.g. `cron: "0 6 * * *"`) + `workflow_dispatch` for on-demand runs.

2. **Live desktop-autonomy smokes (#5-10): include in nightly vs leave out.**
   - Options: (a) include them, relying on their built-in self-skip (`live_smoke_enabled()` returns false unless `THINCLAW_LIVE_DESKTOP_SMOKE=1`, so they no-op on a hosted runner); (b) exclude them from the hosted nightly entirely and document them as **operator-run-only** (they need a real privileged desktop session that GH hosted runners cannot give).
   - Trade-offs: (a) keeps a single `--ignored` invocation simple but the tests will *pass-by-skipping* on the hosted runner, giving false confidence; (b) is honest about what hosted CI can verify and pushes the real coverage to the `linux-desktop-autonomy-smoke` job (`ci.yml:370-411`) which already drives a real `dbus-run-session` desktop via `scripts/ci/linux_desktop_sidecar_smoke.sh`.
   - **Recommendation: (b) — exclude the `*_live_desktop_smoke` tests from the hosted nightly `--ignored` run and document them in the prerequisites matrix as operator/self-hosted-runner-only.** Run them by *name filter exclusion* so the nightly doesn't report green from no-op skips. The genuine desktop coverage already lives in `linux-desktop-autonomy-smoke`.

3. **Quarantined autonomous-campaign E2E: keep `#[ignore]` (do not add to always-on nightly) vs run it.**
   - Options: (a) leave it `#[ignore]`d and **out of the nightly** until WS-07 root-causes the race; (b) add it to the nightly `--ignored` set now.
   - Trade-offs: (b) would re-introduce intermittent red into the new nightly before the race is fixed, eroding trust in the job from day one. (a) keeps the nightly trustworthy and ties re-enabling to a tracked fix.
   - **Recommendation: (a).** Open the tracking issue (T2), and gate re-inclusion on WS-07's fix landing. The simpler `launch_campaign_baseline_runs_local_docker_trial_end_to_end` (`src/api/experiments.rs:4581`) already keeps the Docker-trial path under continuous `--all-features --lib` coverage, so the path is not dark.

4. **db_contract fail-vs-skip hard gate location: test body vs CI job.**
   - WS-02 Decision Point 4 already resolved that `contract_db_or_skip` (`tests/db_contract/support.rs:22`) **stays skip-on-missing-DB** so local `cargo test` doesn't break for developers without a local Postgres. The hard gate for the Postgres path therefore must live in the **CI job**.
   - **Recommendation:** WS-13 enforces the gate by (a) keeping `DATABASE_URL` set on `db-contract-postgres` (already true, `ci.yml:675`) and (b) ensuring `db-contract-postgres`, `db-contract-libsql`, and `schema-divergence` are all **required-for-merge** branch-protection checks. No test-body edit. This is the correct division: WS-02 owns the assertion, WS-13 owns the gate.

## Tasks

- [x] **T1: Add a nightly scheduled workflow that runs the runnable `#[ignore]`d tests.** DONE (`.github/workflows/nightly.yml`, cron `0 6 * * *`, jobs `nightly-docker-e2e`/`nightly-network-smoke`/`nightly-heartbeat`).
  - **Files:** new `.github/workflows/nightly.yml` (WS-13-owned). Do not edit `ci.yml` for this.
  - **Change:** create `nightly.yml` with `on: { schedule: [{ cron: "0 6 * * *" }], workflow_dispatch: {} }` and `permissions: { contents: read }`, mirroring the structure of `ci.yml` jobs with immutable action pins and the repository's Rust `1.94.0` toolchain. Use a `strategy.matrix` fan-out like `fuzz.yml:11-18`. Jobs:
    - `nightly-docker-e2e` (Ubuntu, Docker available on hosted runners): build the worker image the Docker tests need (`thinclaw-worker:latest`) the same way the deploy/host jobs build (see `ci.yml:447-452` for the binary-into-Dockerfile pattern; the worker image build steps must be confirmed against `scripts/build-all.sh` and any `Dockerfile.worker`), then run the Docker-only tests by name so auth-gated ones self-skip cleanly: `cargo test --features full --test docker_sandbox_smoke --test repo_project_docker_e2e -- --ignored --nocapture --test-threads=1`. Pass `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` from repo secrets if present (tests #3/#4 self-skip when absent — `docker_sandbox_smoke.rs:307-312,365-373`).
    - `nightly-network-smoke` (Ubuntu, network allowed): `cargo test -p thinclaw-channels --lib -- --ignored test_dedicated_runtime_real_http --nocapture` (test #11, `wrapper.rs:5671`).
    - `nightly-heartbeat` (Ubuntu + `pgvector/pgvector:pg17` service like `ci.yml:659-672`, with `DATABASE_URL` and LLM creds from secrets): run only when an LLM key secret is present (`if:` guard); `cargo test --features postgres --test heartbeat_integration -- --ignored --nocapture` (test #13). If no LLM secret, skip the job with a clear `echo`.
    - Explicitly **exclude** the `*_live_desktop_smoke` tests (Decision Point 2) and the quarantined experiments test (Decision Point 3) from the `--ignored` invocations by selecting specific test targets/names rather than a blanket `cargo test -- --ignored`.
  - **Acceptance:** `nightly.yml` parses (`gh workflow view nightly.yml` or `actionlint`); a `workflow_dispatch` run executes the Docker and network jobs; auth-gated tests self-skip without failing when secrets are absent; live-desktop and quarantined tests are not invoked.
  - **Effort:** L
  - **Verification:** `actionlint .github/workflows/nightly.yml` (or `npx @action-validator`); locally dry-run the test selection: `cargo test --features full --test docker_sandbox_smoke -- --ignored --list` to confirm the four ignored tests are discoverable; trigger via `gh workflow run nightly.yml` and inspect the run.

- [ ] **T2: Open the worktree/Docker-lifecycle-race tracking issue and document the fix direction.** Left unchecked: the GitHub issue is not verifiable from the working tree, and the fix itself is WS-07-owned.
  - **Files:** a GitHub issue (via `gh issue create`); reference it in this doc and the execution playbook. **No code edit** in WS-13 (the fix is WS-07-owned).
  - **Change:** open an issue titled e.g. *"Root-cause worktree/Docker lifecycle race in autonomous_campaign E2E (quarantined #[ignore], src/api/experiments.rs:5060)"*. Body must include: (a) the failure signature `Internal("No such file or directory (os error 2)")`; (b) the quarantine commit `64b9572f` and that re-runs pass with no code change; (c) the verified race surface — `prepare_campaign_worktree` (`src/api/experiments.rs:3290-3317`) does non-atomic `git worktree remove --force` → `git worktree prune` → `remove_dir_all` while ignoring the `git` call results (`let _ = ...`), and trial git ops (`git_changed_files :3329`, `push_experiment_branch :3319`, `git_output`) spawn against the worktree path that a concurrent prepare/restore step may have already removed; (d) the autonomous chain `launch_next_trial_if_ready :1020` → baseline → mutate → trial → `finalize_trial :2886` → `restore_campaign_worktree_after_trial :3027` over the **same** worktree, where a late git op can overlap the next phase's teardown. **Fix direction to propose (for WS-07):** serialize worktree mutation per campaign behind a per-campaign async lock so prepare/trial/restore cannot overlap; treat the `git worktree remove`/`prune` results instead of `let _ =` so a partial teardown is detected; and verify the worktree dir still exists immediately before each trial git op (or recreate via `prepare_campaign_worktree`) rather than assuming liveness. Label the issue for WS-07.
  - **Acceptance:** issue exists, links the exact file:line anchors above, names WS-07 as fix owner, and states the re-enable condition (drop `#[ignore]` + add to nightly once the lock lands).
  - **Effort:** S
  - **Verification:** `gh issue view <n>` shows the body with anchors; cross-referenced from WS-07's doc and `EXECUTION-PLAYBOOK.md`.

- [x] **T3: Confirm dual-backend `db_contract` gating runs the WS-02 punctuation assertion on both backends.** CI jobs `db-contract-libsql` and `db-contract-postgres` both run the `db_contract` target (`ci.yml:668`, `:684`). The branch-protection required-status setting is a GitHub repo setting, not verifiable from the tree.
  - **Files:** `.github/workflows/ci.yml` (jobs `db-contract-libsql` 640-654, `db-contract-postgres` 656-689) — **inspect/confirm only; edit solely if a job needs an explicit name filter, which it should not.**
  - **Change:** verify the new WS-02 test `conversation_search_tolerates_punctuation_contract` (added in WS-02 T2 to `tests/db_contract/conversations.rs`) is picked up by both jobs because each invokes the whole `db_contract` target per `DATABASE_BACKEND` (`cargo test --test db_contract ...`, `ci.yml:654` and `:689`). No new job is needed. Confirm both jobs are listed as **required status checks** in branch protection (via `gh api repos/:owner/:repo/branches/main/protection` or repo settings) so the parity assertion can't be merged around.
  - **Acceptance:** a CI run on a branch containing the WS-02 assertion shows the punctuation test executing in *both* `DB Contract (libSQL)` and `DB Contract (Postgres)` job logs; both jobs are required-for-merge.
  - **Effort:** S
  - **Verification:** `gh run view <id> --log | rg conversation_search_tolerates_punctuation` against both jobs; `gh api repos/RNT56/<repo>/branches/main/protection --jq '.required_status_checks.contexts'` includes `DB Contract (libSQL)` and `DB Contract (Postgres)`.

- [x] **T4: Wire fail-not-skip gating for `schema_divergence` and the Postgres `db_contract` job.** The `schema-divergence` job (`ci.yml:719`) provisions `DATABASE_URL`. The branch-protection required-status setting is not verifiable from the tree.
  - **Files:** `.github/workflows/ci.yml` (`schema-divergence` 691-723; `db-contract-postgres` 656-689) — confirm-and-gate; no test-body edits (those are WS-02).
  - **Change:** ensure the `schema-divergence` job always exports `DATABASE_URL` (currently `ci.yml:709`) so WS-02 T4's `expect("schema_divergence requires DATABASE_URL ...")` panic becomes a true gate (a misconfigured job that drops the URL now hard-fails instead of silently passing). Confirm `schema-divergence` and `db-contract-postgres` are **required status checks** in branch protection. If WS-02 lands before this, run the negative check: temporarily remove `DATABASE_URL` from the job in a throwaway branch and confirm the job *fails* (do not merge that branch).
  - **Acceptance:** `schema-divergence` keeps `DATABASE_URL` set and is required-for-merge; with WS-02 T4 merged, a build with the dual feature set and no `DATABASE_URL` fails (proven once on a throwaway branch); the green CI path still provisions Postgres and passes.
  - **Effort:** S
  - **Verification:** `rg "DATABASE_URL" .github/workflows/ci.yml` shows it set on `schema-divergence` (line ~709) and `db-contract-postgres` (line ~675); branch-protection contexts include `Schema Divergence` and `DB Contract (Postgres)`; throwaway-branch negative run shows a hard failure.

- [x] **T5: Document the Docker / Postgres / LLM-auth prerequisites matrix for the `--ignored` suite.** DONE (the matrix table in this doc is the canonical reference).
  - **Files:** this doc (the table above is the canonical matrix); cross-link from `EXECUTION-PLAYBOOK.md` and, if a contributor-facing testing doc exists, reference it. Do **not** create a new stray `*.md` summary.
  - **Change:** keep the "14 runnable `#[ignore]`d tests" table current as the prerequisites matrix: per test, the prereq (Docker + `thinclaw-worker:latest`; Postgres + `DATABASE_URL`; Claude/Codex auth secrets; outbound network; `THINCLAW_LIVE_DESKTOP_SMOKE=1` + privileged desktop session). Note which are run by the hosted nightly (Docker, network, heartbeat) vs operator/self-hosted only (the six `*_live_desktop_smoke`) vs blocked until root-cause (the quarantined campaign E2E).
  - **Acceptance:** every `#[ignore]`d first-party test has a row with verified file:line and prereqs; the doc states which runner tier runs each.
  - **Effort:** S
  - **Verification:** `grep -rn "#[ignore" --include="*.rs" tests src crates | grep -v patches` enumerates exactly the rows in the matrix (14 first-party).

- [x] **T6: Verify the WS-01 `--all-targets` clippy gate is reflected across the CI matrix.** DONE: both clippy steps carry `--all-targets` (`ci.yml:66` and `:135`).
  - **Files:** `.github/workflows/ci.yml` (clippy at line 52 `codestyle`, line 121 `feature-matrix`) — **read-only assertion**; WS-01 makes the edit.
  - **Change:** after WS-01 lands, confirm **both** clippy invocations carry `--all-targets`: `cargo clippy --workspace --all-targets -- -D warnings` (codestyle) and `cargo clippy --workspace --all-targets ${{ matrix.cargo-args }} -- -D warnings` (feature-matrix, applying to all 7 profiles: light, edge, full, all-features, desktop, minimal-libsql, minimal-postgres). If only one of the two was updated, that is the exact "fix landed in one of N copies" trap (see Common Pitfalls) — file it back to WS-01 as incomplete; do not edit the line in WS-13.
  - **Acceptance:** `rg "all-targets" .github/workflows/ci.yml` matches **both** clippy steps; a green codestyle + feature-matrix run after WS-01 proves test/example/bench code is now under `-D warnings`.
  - **Effort:** S
  - **Verification:** `rg -n "cargo clippy.*all-targets.*-D warnings" .github/workflows/ci.yml` returns two hits (lines ~52 and ~121); CI codestyle job green.

- [x] **T7: Build the `tools-src/*` crates in `build-all.sh` and CI-build-check the 13 `channels-src` WASM shims.** (Hand-off from WS-03 T6: "build-all.sh never builds tools-src + expand the channel-crates CI matrix to the 13 WASM shims" — no other WS task covers it.) DONE via the accepted alternative: `build-all.sh` prints an explicit "Skipping tools-src/* WASM tool crates" notice (`scripts/build-all.sh:97`), and the per-crate CI matrix build-checks the `channels-src` WASM shims on `wasm32-wasip2` (`ci.yml:772+`).
  - **Files:** `scripts/build-all.sh` (WS-13-owned for this extension); the per-crate channel/tool CI matrix in `.github/workflows/ci.yml`.
  - **Change:** (a) extend `scripts/build-all.sh` to build the `tools-src/*` crates as part of the WASM artifact build — or, if a `tools-src/*` crate cannot yet be built as a packaged artifact, have the script **print a clear, explicit notice** that it does not build them (so the gap is visible rather than silent). (b) Expand the per-crate channel/tool CI matrix in `ci.yml` to include the 13 `channels-src` WASM shims — `dingtalk`, `feishu_lark`, `google_chat`, `line`, `matrix`, `mattermost`, `ms_teams`, `qq`, `twilio_sms`, `twitch`, `wecom`, `weixin`, `shared_webhook_channel` — each with a `cargo build --target wasm32-wasip2` check so a shim that stops compiling to WASM fails CI.
  - **Acceptance:** `build-all.sh` either builds the `tools-src/*` crates or emits an explicit "does not build tools-src" notice; the CI matrix lists all 13 `channels-src` shims, each running `cargo build --target wasm32-wasip2`, and a deliberately-broken shim fails its matrix leg.
  - **Effort:** M
  - **Verification:** `./scripts/build-all.sh` shows the `tools-src/*` build (or the explicit notice); `rg -n "wasm32-wasip2" .github/workflows/ci.yml` covers all 13 shims; a throwaway break in one shim turns that matrix leg red.

## Best Practices (workstream-specific)

- **Copy the repo's existing workflow idioms.** Every job in `ci.yml` uses actions pinned to immutable commits and the repository's Rust `1.94.0` toolchain. The `fuzz.yml` file shows the standalone-workflow + `strategy.matrix` pattern. For a Postgres service, copy the `services.postgres` block verbatim from `db-contract-postgres` (`ci.yml:659-672`) including the health-check `options` and the `Enable pgvector extension` step (`:686-687`). For Docker-image builds in CI, follow the binary-into-Dockerfile pattern at `ci.yml:447-452`.
- **Self-skip, don't pre-gate, for missing auth.** The Docker bridge tests already return early with an `eprintln!` when auth is absent (`docker_sandbox_smoke.rs:307-312, 365-373`); the nightly should *invoke* them and let them self-skip, not condition the whole job on a secret. Reserve the `if:` job-level guard for the heartbeat job (which needs both a DB service and an LLM key) and for live-desktop exclusion.
- **`--test-threads=1` for DB-backed and Docker tests.** The existing `db-contract-postgres` (`ci.yml:689`), `schema-divergence` (`:723`), and `host_runtime_smoke` (`:237`) jobs all serialize. The `db_contract` harness uses a process-global serial lock (`tests/db_contract/support.rs:7` `CONTRACT_DB_TEST_LOCK`); Docker tests reserve ports per-test but share the daemon. Keep `--test-threads=1` for the nightly Docker job.
- **Keep job ownership clean.** Per CLAUDE.md and the WS-02 doc's explicit hand-off, the *assertions* live in WS-02's test files and the *flag* lives in WS-01; WS-13 only owns the *execution jobs* and *gating*. Never edit a WS-02 test body or the WS-01 clippy line from this workstream — assert and hand back.

## Common Pitfalls

- **The "fix landed in one of N copies" trap — verbatim from the audit.** The audit's headline test-infra finding (`§9`) is that clippy omits `--all-targets` in **both** `ci.yml:52` and `ci.yml:121`, and the cross-channel section (`§5`) documents the same class of bug landing the `split_message` fix "in only one of four copies." When verifying T6, check **both** clippy invocations and **all 7 profiles** — a partial WS-01 edit that updates only `codestyle` would leave the entire `feature-matrix` (and thus most profiles) still escaping `-D warnings`.
- **Green-by-skipping.** A blanket `cargo test -- --ignored` would make the six `*_live_desktop_smoke` tests *pass by no-op* on a hosted runner (they self-skip via `live_smoke_enabled()` at `desktop_autonomy_live_smoke.rs:13`), giving a falsely-green nightly. Select test targets/names explicitly (Decision Point 2).
- **Re-enabling the flaky test too early.** Adding the quarantined `autonomous_campaign_..._end_to_end` to the nightly before WS-07's worktree-lock fix lands will reintroduce intermittent red and burn trust in the new job (Decision Point 3). Gate re-inclusion on the tracking issue closing.
- **Editing WS-02 test bodies to "fix" the skip.** The temptation is to make `contract_db_or_skip` (`support.rs:22`) panic too — WS-02 Decision Point 4 explicitly rejects that because it breaks local `cargo test` for developers without a local Postgres. The db_contract hard gate is the **CI job** (required status check), not the test body.
- **Assuming the worker image just exists.** The Docker E2Es require a local `thinclaw-worker:latest`. The nightly must *build* it (confirm the build path against `scripts/build-all.sh` / any `Dockerfile.worker`) or the four Docker tests fail on `docker run` rather than testing anything.
- **Treating the campaign flake as pure CI noise.** The quarantine commit message frames it as a timing race "to be root-caused when reproducible," but the code at `experiments.rs:3290-3317` shows a genuine non-atomic teardown that ignores `git` results — it is a latent correctness bug. The issue (T2) must say so, not file it as flake.

## Multi-Worker Execution Plan (ultracode)

- **Worker decomposition:**
  - **Worker A (nightly + matrix):** T1 (create `nightly.yml`) and T5 (prereqs matrix doc). Largest task; owns the new workflow file end-to-end.
  - **Worker B (gating + verification):** T3 (db_contract dual-backend gating), T4 (schema_divergence fail-not-skip gating), T6 (verify `--all-targets`), and T7 (`build-all.sh` `tools-src/*` + the 13 `channels-src` WASM shim CI matrix). T3/T4/T6 are read/confirm + branch-protection edits; T7 adds the only real `ci.yml`/`scripts/build-all.sh` matrix edits.
  - **Worker C (tracking issue):** T2 (open the root-cause issue). Independent, no file mutation; can run immediately in parallel.
- **Isolation:** Worker A creates a brand-new file (`nightly.yml`) — no contention. Worker B's `ci.yml` touches (if any) are confined to confirming/keeping `DATABASE_URL` and branch-protection (mostly GitHub-settings/`gh api`, not file edits). Worker C touches no repo files. **A git worktree per worker is advisable** so A's new-file run and B's `ci.yml` inspection don't interleave, but contention risk is low because A and B edit disjoint workflow files. Worker C needs no worktree.
- **Workflow shape (implement → verify → review → fix):**
  1. **implement** (fan-out A‖B‖C): A writes `nightly.yml` + matrix doc; B confirms/edits gating + branch protection; C opens the issue.
  2. **verify** (per worker): A runs `actionlint` + `gh workflow run nightly.yml` (manual dispatch) and inspects the run; B confirms required-status contexts via `gh api` and runs the throwaway-branch negative check for T4; C confirms `gh issue view`.
  3. **review** (`/code-review` on any `ci.yml`/`nightly.yml` diff; `/ship` for the formatting/lint gate on any Rust touched — note WS-13 touches no Rust, so `cargo fmt`/`clippy` should be a no-op confirmation).
  4. **fix:** if T6 finds a partial `--all-targets` edit, hand back to WS-01; if T3/T4 find a job not required-for-merge, set the branch-protection context.
- **Verification gate (exact commands):**
  - `actionlint .github/workflows/nightly.yml .github/workflows/ci.yml` (workflow lint).
  - `cargo test --features full --test docker_sandbox_smoke --test repo_project_docker_e2e -- --ignored --list` (confirm the Docker `--ignored` tests are discoverable). **Prereq:** Docker daemon + `thinclaw-worker:latest` image built.
  - `cargo test -p thinclaw-channels --lib -- --ignored test_dedicated_runtime_real_http --nocapture` (network smoke). **Prereq:** outbound network.
  - `DATABASE_BACKEND=postgres DATABASE_URL=postgres://thinclaw:thinclaw@localhost:5432/thinclaw_test cargo test --test db_contract --no-default-features --features postgres -- --nocapture --test-threads=1` (confirm dual-backend job content). **Prereq:** local `pgvector/pgvector:pg17` Postgres with `migrations/V*.sql` applied (see CLAUDE.md local-dev Postgres note).
  - `cargo test --test schema_divergence --no-default-features --features "postgres libsql"` with `DATABASE_URL` **unset** → expect FAIL (negative gate check for T4, after WS-02 lands).
  - `rg -n "cargo clippy.*all-targets.*-D warnings" .github/workflows/ci.yml` → two hits (T6).
  - `/ship` (fmt + clippy + tests) — expected no-op on the Rust side since WS-13 changes only CI/issue artifacts.
  - **DB/Docker prerequisites:** a fresh `pgvector/pgvector:pg17` container suffices for `db_contract`/`schema_divergence`; the nightly Docker job additionally needs the Docker daemon and a built `thinclaw-worker:latest` image.

## Definition of Done

- [x] `.github/workflows/nightly.yml` exists, runs on a `schedule` + `workflow_dispatch`, and defines the Docker, network, and heartbeat jobs (heartbeat self-skips if no LLM secret).
- [x] The nightly runs the runnable `#[ignore]`d tests by explicit target selection; auth-gated Docker bridge tests self-skip when secrets are absent without failing the job.
- [x] The `*_live_desktop_smoke` tests are documented as operator/self-hosted-only and are **not** invoked by the hosted nightly (no green-by-skip).
- [ ] The quarantined `autonomous_campaign_..._end_to_end` is **not** in the nightly (verified). The GitHub tracking issue is not verifiable from the working tree; the fix is WS-07-owned.
- [ ] `schema-divergence` provisions `DATABASE_URL` (verified). Whether `db-contract-libsql`, `db-contract-postgres`, and `schema-divergence` are set as required-for-merge branch-protection checks is a GitHub repo setting, not verifiable from the tree.
- [x] `schema-divergence` always provisions `DATABASE_URL` (`ci.yml:719` job), so WS-02's fail-not-skip panic is a real gate.
- [x] Both clippy invocations (`ci.yml:66` and `:135`) carry `--all-targets`, so all profiles run under `-D warnings` on test/example/bench code (T6).
- [x] `scripts/build-all.sh` explicitly prints that it does not build the `tools-src/*` crates (`scripts/build-all.sh:97`), and the per-crate CI matrix build-checks the `channels-src` WASM shims on `wasm32-wasip2` (`ci.yml:772+`) (T7).
- [x] Decision Points 1-4 resolved as recommended; the prerequisites matrix (this doc) is the canonical reference.
- [x] No WS-02 test-body or WS-01 clippy-line edits attributed to WS-13.
- [ ] **Still open (outside T1-T7):** the coverage job (`ci.yml:881`) still runs `cargo llvm-cov --all-features --lib` with **no `--fail-under`** floor, and `cargo deny` does not scan the `channels-src/`/`tools-src/` sub-workspace lockfiles.
