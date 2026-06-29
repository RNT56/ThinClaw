# Execution Playbook

The mechanical procedure for every refactor PR. Follow it verbatim; it encodes the verification
gates and the conflict-avoidance learned executing Phase 0.

## 1. The loop (one task → one PR)

```
1. Sync + branch off main in an isolated worktree.
2. Make the change (re-locate symbols first — line numbers drift).
3. Run the verification gate for the change-type (§3).
4. Sync both lockfiles if a manifest changed (§4).
5. Update canonical docs + FEATURE_PARITY if behavior changed.
6. Commit (conventions §5) → push → open PR → arm auto-merge.
7. Drive the queue: update-branch when BEHIND, never merge red (§6).
8. Clean up the worktree + local branch.
```

## 2. Worktree & branch setup

Work in an isolated git worktree off the latest `main`, sharing one warm build target so verifies
are fast and collision-free:

```bash
WT=<scratch>/wt-<task>
git fetch origin -q
git worktree add "$WT" -b mt-<task> origin/main
export CARGO_TARGET_DIR=<scratch>/parity-target          # shared, warm across tasks
# frontend tasks only — fresh worktree needs node_modules:
( cd "$WT/apps/desktop" && npm ci )
```

- Branch naming: `mt-<short-task>` (e.g. `mt-routine-types`).
- To switch tasks in the same worktree: commit/push the current branch, then
  `git checkout -q origin/main -b mt-<next>` (re-fetch first). `node_modules` and the cargo target
  persist across checkouts.
- Always remove the worktree + delete the local branch when the PR is up:
  `git worktree remove --force "$WT"; git worktree prune; git branch -D mt-<task>`.

## 3. Verification gate by change-type

Pick the rows that apply. Shared target dir = `$CARGO_TARGET_DIR` above.

| You changed… | Run |
|---|---|
| **Any Rust (root crates / `src/`)** | `cargo fmt`; `cargo check`; `cargo clippy -p <crate> --all-features` (or workspace clippy if broad) |
| **Feature-gated Rust** (docker/browser/voice/wasm/channels) | also `cargo check --all-features` and/or `cargo check -p thinclaw-channels --all-features` |
| **A shared enum/trait matched across crates** (esp. `StatusUpdate`) | `cargo check` **and** `cargo check -p thinclaw-channels --all-features` — the 6-matcher ripple (PRINCIPLES §3.2) |
| **A `Cargo.toml`** (added/changed a dep) | `cargo check` root **and** `cd apps/desktop/backend && cargo check`; stage **both** `Cargo.lock`s |
| **A Tauri command / `UiEvent` / DTO** | `cargo run --example export_bindings` (in `apps/desktop/backend`); confirm the binding appears; then `cd apps/desktop/frontend && npx tsc --noEmit` |
| **Bridge / `ROUTE_TABLE` / gating** | `cargo test --lib bridge::` (the linter tests) |
| **Desktop frontend (`.tsx`/`.ts`)** | `npx tsc --noEmit` + `npx vitest run` (in `apps/desktop/frontend`) |
| **Any file** (always) | `node scripts/check-naming-cleanliness.mjs` (in `apps/desktop`) |
| **A crate boundary move** | full-workspace `cargo check` (the desktop is a *separate* workspace — also `cargo check --manifest-path apps/desktop/backend/Cargo.toml`) |

> The desktop CI gate is `cargo check` (not clippy). A pre-existing desktop clippy lint
> (`invisible character` in `rig_lib`) is **not yours and not a gate** — don't chase it.

## 4. Lockfile discipline (the #1 self-inflicted break)

Any `Cargo.toml` change updates two lockfiles. After the change:

```bash
cargo check                                  # updates root Cargo.lock
( cd apps/desktop/backend && cargo check )   # updates apps/desktop/backend/Cargo.lock
git add Cargo.lock apps/desktop/backend/Cargo.lock
```

CI runs `cargo check --locked`; an un-synced lockfile fails it. A dep already present transitively
(e.g. `ipnet`, `serde`) still changes the lock when you declare it directly — sync anyway.

## 5. Commit & PR conventions

- Commit subject: conventional (`fix(security): …`, `feat(channels): …`, `refactor(agent): …`,
  `ci: …`, `docs: …`). Body explains *why* + the verification run.
- **Commit trailer (required):** `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`
- **PR body trailer (required):** `🤖 Generated with [Claude Code](https://claude.com/claude-code)`
- PR body: what + why, the verification evidence, and **explicitly call out anything deferred** and
  why. Link the relevant backlog task and `ROBUSTNESS_AND_ARCHITECTURE_PLAN.md`.
- Arm auto-merge: `gh pr merge --repo <repo> --auto --merge <#>`.
- Never bypass branch protection; never admin-merge; never merge with a failing *relevant* check.

## 6. Queue & conflict management (strict-mode branch protection)

- Strict mode = branches must be up-to-date before merging; the queue drains one PR at a time.
- When a PR shows `BEHIND` with `fail=0`: `gh api -X PUT repos/<repo>/pulls/<#>/update-branch`.
  When it shows a failing check: **investigate, don't update-branch** (the failure is real or flaky;
  fix it).
- **Avoid same-file concurrency.** Do not open a PR that edits a file an in-flight PR also edits.
  The hotspots: the 6 `StatusUpdate` matchers (events PRs), `bridge.rs`/`ROUTE_TABLE`
  (command/gating PRs), both `Cargo.lock`s. The backlog's **Blocked-by** field encodes this.
- Keep the open-PR count low enough to review. If the queue is deep, prefer finishing/landing over
  opening more — especially for same-file work.

## 7. Refactor-specific recipes

**Cross-crate type move (e.g. Routine → `thinclaw-types`):**
1. Identify the *pure* subset to move (no logic, no heavy deps). If the source file mixes DTOs with
   logic (regex/tz/etc.), **split first**: extract DTOs to a new module, leave logic behind.
2. Add the new module to the destination crate; ensure its only deps already exist there.
3. In the source crate, replace the moved items with `pub use <dest>::*` (path stability).
4. Update downstream importers to the new path; remove the now-unneeded crate dep from their
   `Cargo.toml`.
5. Full-workspace `cargo check` + the desktop check; sync both lockfiles.
6. Add/extend the CI structural guard (e.g. `rg 'thinclaw-agent' crates/thinclaw-db/Cargo.toml`
   must be empty).

**God-file decomposition (same crate, zero behavior change):**
1. Read the file; group items by cohesive concern (types / policy / one-tool-per-file / …).
2. Create submodule files; move each group; fix visibility to `pub(crate)`/`pub(super)` as needed.
3. Make the old file a façade: `mod x; pub use x::*;` — **public paths must not change**.
4. `cargo check` + clippy + the crate's tests (behavior is identical, so tests must still pass
   untouched). Keep tests next to the module they validate.

**Adding a CI guard:** verify it **passes on current main locally first** (e.g. run the grep / the
test). A guard that fails immediately (size-guard before the splits; `multiple-versions=deny`
before the dedup) must wait for its prerequisite task — note that in **Blocked-by**.

## 8. Known gotchas (environment + tooling)

- macOS bash is **3.2** — no `declare -A` associative arrays; use `case` functions.
- Prefer the dedicated file tools over `cat`/`sed`/`echo`. `cd` inside a compound shell command can
  trigger a permission prompt — use absolute paths or subshells.
- `Date.now()`/`Math.random()`/argless `new Date()` are unavailable inside Workflow scripts.
- Re-audit after each wave: re-run the `robustness-architecture-audit` workflow and diff the metrics
  to confirm targets and catch new regressions.
