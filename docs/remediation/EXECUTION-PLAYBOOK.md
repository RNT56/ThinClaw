# ThinClaw Remediation — ULTRACODE Execution Playbook

> **Audience:** the operator driving the remediation via ultracode multi-agent Workflows.
> **Inputs:** `AUDIT-FINDINGS.md` (findings) + `WS-01..WS-13` (per-workstream plans).
> **This doc:** the *order of operations* — how to drive the 13 workstreams to merged, green code with maximum safe parallelism and minimum rebase churn.
>
> Repo root: `/Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop`. Default/main branch: `main`. Toolchain pinned to Rust 1.92.0.

---

## 1. Execution Model

Each workstream is one **base branch off `main`** (`ws-01-security`, `ws-02-db`, …) and is executed by **one ultracode Workflow** following the same loop:

```
implement  →  verify (gate)  →  adversarial code-review  →  fix  →  (loop until clean)  →  PR
```

Rules of the road:

- **Small, reviewable PRs.** Decompose each WS into the task list its WS doc already defines (`T1…Tn`). Prefer one PR per cohesive task cluster, not one mega-PR per WS. God-file decompositions (WS-10) are **one file per PR**, behavior-preserving.
- **The quality gate must be green before merge.** No PR merges with red `cargo fmt`/`clippy --all-targets --all-features -D warnings`/`cargo test`/`cargo deny`. The gate per WS is in §5.
- **The adversarial review is mandatory**, not optional. Run `/code-review high` (security-critical WS: `/code-review high` with `--comment`) and feed every finding back into a fix phase before declaring done. Security boundary fixes (WS-01) and `unsafe`/`dlopen` work (WS-05 native plugins) get the deepest review tier available.
- **Branch hygiene.** Never commit to `main`. Branch first; the operator merges. Commit messages end with the `Co-Authored-By` trailer; PR bodies end with the generated-with-Claude-Code line (per repo `CLAUDE.md`).
- **Ownership is law.** A WS only edits files in its own `Owns` list. Shared files (called out in §2) are sequenced, never co-edited. Cross-WS needs are recorded as dependencies, not smuggled edits.
- **Canonical-doc discipline.** When a WS changes behavior, it updates its *own* canonical doc in the same branch (per `CLAUDE.md` Common Update Triggers). Inventory/drift docs are WS-12's; WS-12 *trails* every wave.

---

## 2. Dependency DAG & Wave Plan

### 2.1 Declared edges (from each WS header)

| WS | depends_on | blocks | hard shared-file coupling |
|----|-----------|--------|---------------------------|
| WS-01 Security/CI | — | **every WS** (CI must be green to merge; empty-token + wasmtime gate `main`) | `Cargo.lock` (wasmtime line), `ci.yml` clippy lines, `src/channels/web/{mod,server}.rs`, `src/sandbox/proxy/**` |
| WS-02 DB correctness | — | WS-13 (consumes its assertions) | `tests/db_contract/support.rs` shared w/ WS-13 |
| WS-03 WASM channels/SDK | — | — (coord WS-12 README, WS-13 build) | host `wasm/{schema,router}.rs` (additive only) |
| WS-04 Desktop | — | WS-10 (desktop god-file split coordinates) | `apps/desktop/backend/**` (workspace-excluded, isolated) |
| WS-05 Self-repair/extensions/native | — | — (coord WS-10 manager split, WS-11 RepairTask) | `src/extensions/{manager,mod,registry}.rs`, `src/agent/agent_loop.rs` (additive block) |
| WS-06 Repo-project supervisor | — | — (coord WS-12 inventory) | `crates/.../web/static/app.js` (additive consumer) |
| WS-07 Experiments | — | WS-10 (split inherits taxonomy), WS-13 (race anchor) | `src/api/experiments.rs` **shared w/ WS-10** (additive hunks only) |
| WS-08 LLM consolidation | — | — (coord WS-10 shared files) | `src/llm/runtime_manager.rs` **shared w/ WS-10** (localized edits), `src/llm/reasoning.rs` shared w/ WS-10 |
| WS-09 Routines/scheduler | — | WS-10 (engine split after) | `src/agent/routine_engine.rs` **shared w/ WS-10** (named-symbol edits), `src/channels/web/handlers/routines.rs` (diff handler from WS-01) |
| WS-10 Architecture overhaul | **WS-01,02,03,04,05,06,07,08,09** | — | owns the god-files everyone else additively touched |
| WS-11 Dead-code sweep | **WS-05** (RepairTask/native), **WS-10** (history dedup, safety orphan handoff) | — | `src/extensions/manager.rs` (deletes a helper WS-05/WS-10 touch) |
| WS-12 Docs/drift | — (structural); **trails WS-03** (Discord README), **WS-01** (deny.toml header) | — | doc-only; coordinates README wording with WS-03 |
| WS-13 Test/CI infra | **WS-01** (`--all-targets`), **WS-02** (schema/db assertions) | — (terminal) | `ci.yml` test-job section (distinct from WS-01's clippy lines) |

### 2.2 The DAG (text form)

```
                 ┌──────────── WS-01 (security + green CI) ────────────┐
                 │  gates ALL merges; owns --all-targets + lockfile    │
                 └───────────────────────┬─────────────────────────────┘
                                          │ (CI green baseline)
   WS-02 (DB) ───────────┐               │
                          ├──> WS-13 (test/CI infra) <── WS-01 (--all-targets)
   ────────── independent behavior fixes (parallel) ──────────
   WS-03  WS-04  WS-05  WS-06  WS-07  WS-08  WS-09
     │      │      │             │      │      │
     │      │      │ (RepairTask/native handoff)
     │      │      └────────────────────────────────> WS-11 (dead-code sweep)
     │      │                                              ▲
     └──────┴── all behavior fixes land ──> WS-10 (god-files) ──┘ (safety-orphan handoff)
                                              ▲
   WS-12 (docs) trails each wave, absorbs canonical-doc updates as WS land
```

Key invariants:
- **WS-01 first, alone-ish.** It restores the green gate (wasmtime bump, `deny.toml`, `--all-targets`, the `await_holding_lock` it surfaces) and closes the auth bypass. Until its lead-in (T1→T2→T3) merges, nobody's PR can be green under the new `--all-targets` clippy.
- **WS-10 and WS-11 LAST.** WS-10 explicitly depends on WS-01..09; WS-11 depends on WS-05 + WS-10. Running them early guarantees painful rebases because every behavior WS additively touches a god-file WS-10 will move and a symbol WS-11 may delete.
- **WS-12 trails**, never blocks. It absorbs each wave's canonical-doc fallout.

### 2.3 Waves (max safe parallelism)

> A "wave" is a set of WS that can run concurrently because they touch disjoint files **or** are isolated in worktrees. Serialization points are called out by the shared-file column above.

**Wave 0 — Establish a GREEN baseline (gate everything else).**
- **WS-01** lead-in **must merge first**: `T1` (wasmtime 36.0.11) → `T2` (`deny.toml`) → `T3` (`--all-targets` + `await_holding_lock`). These three are a *serial* lead-in inside WS-01. Once merged to `main`, every later branch rebases onto the new `Cargo.lock` + clippy config.
- In parallel with the WS-01 lead-in (disjoint files): **WS-02** (libSQL FTS5 + schema parity) and **WS-12 doc-persist seed** (the inventory/crate-table tasks that have no code dependency — repo-project rows, 4 missing crates, dated-count purge).
- After WS-01 lead-in merges, the rest of **WS-01** fans out (see §4.2 — its own A/B/C/D worktrees).

**Wave 1 — Independent behavior fixes (high parallelism).** All depend only on the Wave-0 green baseline.
- **WS-03** (WASM channels/SDK) — `channels-src/**`, `tools-src/**`, additive host `wasm/{schema,router}.rs`.
- **WS-04** (Desktop) — `apps/desktop/backend/**` (workspace-excluded package; zero overlap with root → fully parallel, no worktree needed beyond branch).
- **WS-05** (self-repair / native plugins / observability) — `src/extensions/**`, additive `agent_loop.rs` block.
- **WS-06** (repo-project supervisor) — `src/repo_projects/**`, additive `app.js`.
- **WS-09** (routines/scheduler/heartbeat) — named symbols in `routine_engine.rs`/`heartbeat.rs`.
  - *Serialize note:* WS-09 edits `src/channels/web/handlers/routines.rs`; WS-01 edits `src/channels/web/{mod,server}.rs` (different files) — no conflict, but if WS-01's web work is still in flight, WS-09's webhook task rebases after it (WS-09 §Out-of-scope says so).

**Wave 2 — Shared-file behavior fixes (serialize against each other on `src/api/experiments.rs`, `src/llm/runtime_manager.rs`).**
- **WS-07** (experiments) — *additive hunks* into `src/api/experiments.rs` (reaper + durable upload + taxonomy). Must land **before** WS-10 splits that file.
- **WS-08** (LLM consolidation) — localized edits in `src/llm/runtime_manager.rs` `resolve_route` + `route_planner.rs` + retire `SmartRoutingProvider`. Must land **before** WS-10 decomposes `runtime_manager.rs`. WS-08's `reasoning.rs` field-removal coordinates with WS-10 (DP-3).
  - WS-07 and WS-08 touch *different* files (experiments vs llm) → they can run in parallel with each other and with Wave 1 if runner capacity allows; they are grouped here only because both are WS-10 prerequisites that mutate eventual-split targets.

**Wave 3 — Architecture overhaul (serialized last, after all behavior WS merge).**
- **WS-10** (god-files + crate migrations). Depends on WS-01..09 all merged. One file per PR. The `experiments.rs`/`runtime_manager.rs`/`routine_engine.rs`/`agent_loop.rs`/`reasoning.rs`/`manager.rs` splits each rebase onto the additive edits Waves 1–2 already landed.

**Wave 4 — Dead-code sweep (after WS-05 + WS-10).**
- **WS-11**. Deletes the 14 `src/safety/*` orphans (WS-10 handed these off), the dead CLI modules, dead helpers, and resolves the voice_wake/tailscale/qr_pairing wire-vs-erase decisions. Depends on WS-05 (RepairTask/native fate decided) and WS-10 (history dedup + safety-orphan handoff done).

**Trailing throughout — WS-12 doc-sync + WS-13 test/CI infra.**
- **WS-12** runs a small doc-absorb pass at the end of each wave (e.g. after WS-03 lands Discord Ed25519, reword `discord/README.md`; after Wave 1/2, refresh `FEATURE_PARITY`/`CRATE_OWNERSHIP`). Its *structural* inventory tasks seed in Wave 0.
- **WS-13** runs after WS-01 (`--all-targets`) and WS-02 (assertions) merge — it *verifies* the flag landed across both clippy invocations, creates `nightly.yml`, wires the `#[ignore]` matrix, and opens the flaky-campaign tracking issue. It can run concurrently with Wave 2/3.

### 2.4 Explicit shared-file conflict register (DO NOT co-edit)

| File | WS that touch it | Sequencing rule |
|------|-----------------|-----------------|
| `Cargo.lock` (wasmtime line) | WS-01 owns; everyone rebases | WS-01 lead-in lands first; others rebase. |
| `.github/workflows/ci.yml` | WS-01 (clippy lines 52,121) · WS-13 (test jobs 640-723 + new `nightly.yml`) | Disjoint line ranges; WS-13 only *asserts* WS-01's flag landed. WS-01 merges first. |
| `src/api/experiments.rs` | WS-07 (additive reaper/upload) · WS-10 (decomposition) | WS-07 lands **first** (additive); WS-10 splits after. |
| `src/llm/runtime_manager.rs` | WS-08 (localized `resolve_route`) · WS-10 (decomposition) | WS-08 lands **first**; WS-10 splits after. |
| `src/llm/reasoning.rs` | WS-08 (`safety` field erase) · WS-10 (decomposition) | WS-08 owns semantic removal; hand as rider to WS-10 if WS-10 splits it same cycle (WS-08 DP-3). |
| `src/agent/routine_engine.rs` | WS-09 (named symbols) · WS-10 (decomposition) | WS-09 lands **first**; WS-10 splits after. |
| `src/agent/agent_loop.rs` | WS-05 (additive `with_builder` block) · WS-10 (decomposition) | WS-05 lands **first**; WS-10 splits after. |
| `src/extensions/manager.rs` | WS-05 (additive native arms) · WS-10 (decomposition) · WS-11 (deletes `install_bundled_channel_from_artifacts`) | WS-05 → WS-10 → WS-11, in that order. |
| `crates/.../web/static/app.js` | WS-06 (additive SSE consumer) | Single owner; additive only. |
| `tests/db_contract/support.rs` | WS-02 owns · WS-13 read-only | WS-13 does not edit; only gates the CI job. |
| `channels-src/discord/README.md` | WS-03 (code+reword) · WS-12 (drift) | WS-03 lands the code + reword; WS-12 leaves it alone. |
| `deny.toml` | WS-01 only (incl. header) | WS-12 must NOT touch (explicitly disclaimed). |

---

## 3. Worktree Isolation Strategy

**Use `EnterWorktree` (git worktree per agent under `.claude/worktrees/`) when:**
- Two+ agents in the **same wave** could touch overlapping files or the shared `Cargo.lock` concurrently (WS-01's A/B/C/D fan-out; any Wave-1 agents that brush a shared crate).
- A god-file decomposition wave (WS-10) where N agents each carve a different file but all share the same WS-10 base branch — each agent in its own worktree off the WS-10 branch.
- Adversarial review needs a clean tree to diff while implementation continues elsewhere.

**Plain parallel branches (no worktree) are fine when:**
- The WS is in a **workspace-excluded package with zero root overlap** — notably **WS-04** (`apps/desktop/backend` has its own `sqlx` 0.8 and is `exclude`d; no other WS edits it). One branch, no worktree.
- The WS owns a disjoint subtree no concurrent WS reads (**WS-06** `src/repo_projects/**`, **WS-03** `channels-src/**`/`tools-src/**` standalone Cargo workspaces).

**Mechanics:** an agent calls `EnterWorktree({name: "ws-05-native"})` to branch from `origin/main` (default `worktree.baseRef: fresh`) — or off the WS base branch if the operator pre-creates it and the agent enters by `path`. Build/test inside the worktree. On completion the wave coordinator opens the PR from the worktree branch; `ExitWorktree({action: "keep"})` preserves it for review, `{action: "remove"}` after merge. **Never run two agents writing the same `Cargo.lock` outside isolation** — the wasmtime bump (WS-01) and any incidental dependency add will collide.

---

## 4. Per-Wave Workflow Skeletons

> Dialect: ultracode `Workflow()` — `export const meta`, `phase()`, `agent({schema})`, `pipeline()`, `parallel()`. The adversarial-verify pattern is *implement → verify-gate → review → fix → reloop-until-clean*. Paths below are real.

### 4.1 (a) Single-WS implementation + adversarial review

```js
// ws-single.workflow.js  — drives one workstream end-to-end
export const meta = {
  name: "ws-single",
  params: { wsId: "WS-09", wsDoc: "docs/remediation/09-routines-scheduler-heartbeat-completion.md",
            crate: "thinclaw", testFilter: "routine heartbeat",
            base: "main", branch: "ws-09-routines" },
};

export default async function ({ params }) {
  const { wsId, wsDoc, crate, testFilter, base, branch } = params;

  // 1. IMPLEMENT — agent reads its WS doc + AUDIT-FINDINGS, executes the T-tasks.
  const impl = await phase("implement", () =>
    agent({
      prompt: `You are executing ${wsId}. Read ${wsDoc} and docs/remediation/AUDIT-FINDINGS.md.
Branch ${branch} off ${base}. Implement every task (T1..Tn) in the doc, honoring the Decision
Points' recommended options and the Owns/Out-of-scope boundaries. Mirror the already-correct
sibling patterns the doc cites. Do NOT edit files owned by other workstreams.`,
      schema: { filesChanged: "string[]", tasksDone: "string[]", decisionsTaken: "string[]" },
    })
  );

  // 2. VERIFY — the exact gate (fmt + clippy --all-targets + per-crate tests + deny).
  const gate = await phase("verify", () => runGate({ crate, testFilter }));

  // 3. ADVERSARIAL REVIEW — high tier; security-sensitive WS use --comment.
  const review = await phase("review", () =>
    agent({
      prompt: `Run /code-review high on the current diff for ${wsId}. Be adversarial: hunt for
boundary regressions, only-one-of-N-copies fixes, widened visibility, and broken acceptance
criteria from ${wsDoc}'s Definition of Done. Return findings as a structured list.`,
      schema: { findings: "{severity:string, file:string, line:number, issue:string}[]" },
    })
  );

  // 4. FIX-LOOP — reloop until gate green AND no high/critical review findings.
  let pass = gate.green && review.findings.filter(f => /high|critical/.test(f.severity)).length === 0;
  for (let i = 0; i < 3 && !pass; i++) {
    await phase(`fix-${i}`, () =>
      agent({ prompt: `Fix these review findings and any gate failures, then stop:
${JSON.stringify(review.findings)}\nGate: ${JSON.stringify(gate)}`,
              schema: { fixed: "string[]" } }));
    const g = await phase(`reverify-${i}`, () => runGate({ crate, testFilter }));
    const r = await phase(`rereview-${i}`, () =>
      agent({ prompt: `Re-run /code-review high on the current diff for ${wsId}.`,
              schema: { findings: "{severity:string, file:string, line:number, issue:string}[]" } }));
    pass = g.green && r.findings.filter(f => /high|critical/.test(f.severity)).length === 0;
  }

  if (!pass) return { status: "needs-operator", wsId, branch };
  return { status: "ready-to-PR", wsId, branch, impl };
}

// Shared gate runner — the §5 commands for this WS.
async function runGate({ crate, testFilter }) {
  return agent({
    prompt: `Run, in order, and report pass/fail + output tail of each:
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  cargo test -p ${crate} ${testFilter}
  cargo deny check
Set green=true only if ALL pass.`,
    schema: { green: "boolean", failing: "string[]", tails: "string[]" },
  });
}
```

### 4.2 (b) Wave-level fan-out — several independent WS in parallel, per-WS worktree

```js
// wave1.workflow.js  — runs Wave 1 behavior fixes concurrently, each isolated.
export const meta = { name: "wave1-behavior-fixes", params: { base: "main" } };

const WAVE1 = [
  { wsId: "WS-03", wsDoc: "docs/remediation/03-wasm-channels-tools-repair-and-sdk.md",
    worktree: "ws-03-wasm",  crate: "thinclaw-channels", testFilter: "split_message webhook" },
  { wsId: "WS-04", wsDoc: "docs/remediation/04-desktop-app-completion.md",
    worktree: null,          // workspace-excluded package; plain branch, no worktree
    crate: "thinclaw-desktop-backend", testFilter: "cloud sync inference",
    cwd: "apps/desktop/backend" },
  { wsId: "WS-05", wsDoc: "docs/remediation/05-self-repair-extensions-native-plugins.md",
    worktree: "ws-05-native", crate: "thinclaw", testFilter: "self_repair extensions native" },
  { wsId: "WS-06", wsDoc: "docs/remediation/06-repo-project-supervisor-completion.md",
    worktree: "ws-06-repo",  crate: "thinclaw", testFilter: "repo_project supervisor pipeline" },
  { wsId: "WS-09", wsDoc: "docs/remediation/09-routines-scheduler-heartbeat-completion.md",
    worktree: "ws-09-routines", crate: "thinclaw", testFilter: "routine heartbeat" },
];

export default async function () {
  // Each WS runs the single-WS pipeline (4.1) inside its own worktree (or plain branch for WS-04).
  const results = await parallel(WAVE1.map((ws) => async () => {
    if (ws.worktree) await EnterWorktree({ name: ws.worktree });   // isolate shared Cargo.lock + crates
    const out = await pipeline("ws-single", {
      wsId: ws.wsId, wsDoc: ws.wsDoc, crate: ws.crate,
      testFilter: ws.testFilter, base: meta.params.base,
      branch: ws.worktree ?? `ws-04-desktop`,
    });
    if (ws.worktree) await ExitWorktree({ action: "keep" });       // keep for review; remove after merge
    return out;
  }));

  // Wave gate: every WS must be ready-to-PR before the operator merges the wave.
  const blocked = results.filter((r) => r.status !== "ready-to-PR");
  return { wave: "1", ready: results.filter(r => r.status === "ready-to-PR").map(r => r.wsId),
           blocked: blocked.map(r => ({ wsId: r.wsId, status: r.status })) };
}
```

> WS-04 runs with `cwd: "apps/desktop/backend"` and **no** worktree (excluded package, its own `sqlx` 0.8). WS-03 builds `channels-src/*`/`tools-src/*` as standalone `wasm32-wasip2` workspaces — its gate uses the per-crate `cargo build --target wasm32-wasip2` plus `./scripts/build-all.sh`, not the root workspace clippy.

### 4.3 (c) God-file decomposition (WS-10) — one file per agent, behavior-preserving, re-export-checked

```js
// ws10-decompose.workflow.js  — Wave 3, one file per agent off the WS-10 base branch.
export const meta = { name: "ws10-god-file-decompose", params: { base: "ws-10-overhaul" } };

// Each target: the god-file, its decomposition recipe from WS-10, and the public paths to preserve.
const TARGETS = [
  { file: "crates/thinclaw-channels/src/wasm/wrapper.rs", lines: 5701,
    recipe: "extract Telegram transport behind a trait; mod.rs stays a façade",
    publicPaths: ["thinclaw_channels::wasm::*"], crate: "thinclaw-channels" },
  { file: "src/api/experiments.rs", lines: 5434,
    recipe: "split handlers per domain (CRUD/reconcile/trial/planner-mutator-reviewer/lease/cost/git); coordinate error.rs with WS-07 (already merged)",
    publicPaths: ["crate::api::experiments::*"], crate: "thinclaw" },
  { file: "src/agent/thread_ops.rs", lines: 3032,
    recipe: "extract process_approval (~850L) into a focused submodule",
    publicPaths: ["crate::agent::thread_ops::*"], crate: "thinclaw" },
  { file: "src/llm/runtime_manager.rs", lines: 3096,
    recipe: "split after WS-08's resolve_route edits landed; types/core/route-consumption/persistence",
    publicPaths: ["crate::llm::runtime_manager::*"], crate: "thinclaw" },
  { file: "src/extensions/manager.rs", lines: 3343,
    recipe: "split after WS-05 native arms landed; carve manager/native.rs etc.",
    publicPaths: ["crate::extensions::manager::*"], crate: "thinclaw" },
  // … src/agent/routine_engine.rs, agent_loop.rs, workspace_core.rs, setup/wizard/*, desktop rpc_dashboard/remote_proxy/sidecar
];

export default async function () {
  // Strictly serialize per file (each is its own PR), but isolate each in a worktree off the WS-10 branch.
  const out = [];
  for (const t of TARGETS) {
    await EnterWorktree({ name: `ws10-${t.file.split("/").pop().replace(".rs","")}` });

    const before = await phase("snapshot", () =>
      agent({ prompt: `Record the full public surface of ${t.file}: every pub/pub(crate) item and
every external import path. This is the behavior-preservation contract.`,
              schema: { publicItems: "string[]", externalCallers: "string[]" } }));

    await phase("decompose", () =>
      agent({ prompt: `Decompose ${t.file} (${t.lines}L) per WS-10: ${t.recipe}.
RULES (CLAUDE.md architecture hygiene): mod.rs becomes a façade (declare submodules + pub use
re-exports only); one domain concept per new file; visibility pub(super)/pub(in crate::...) —
do NOT widen APIs to compile; no util/common/misc buckets. Preserve every public path in
${JSON.stringify(t.publicPaths)} via pub use. This must be BEHAVIOR-PRESERVING — move code, do
not change it.`,
              schema: { newFiles: "string[]", reexports: "string[]" } }));

    const gate = await phase("verify", () =>
      agent({ prompt: `Run:
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  cargo build -p ${t.crate}
  cargo test -p ${t.crate}
green only if all pass — a compile failure means a public path broke.`,
              schema: { green: "boolean", failing: "string[]" } }));

    const review = await phase("review", () =>
      agent({ prompt: `Adversarial /code-review high: confirm this is a PURE MOVE. Diff the public
surface against the snapshot ${JSON.stringify(before.publicItems)} — flag ANY signature change,
visibility widening, or dropped re-export. Flag any behavioral change disguised as a move.`,
              schema: { behaviorChanged: "boolean", surfaceDrift: "string[]" } }));

    out.push({ file: t.file, green: gate.green, pureMove: !review.behaviorChanged, drift: review.surfaceDrift });
    await ExitWorktree({ action: "keep" }); // one PR per file
  }
  return { ws: "WS-10", files: out };
}
```

---

## 5. Verification Gates (per WS)

**Base gate (every WS, every PR):**
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings   # the WS-01 invariant
cargo deny check                                                        # green after WS-01 T1/T2
```
Plus `/ship` to run the full fmt+clippy+test gate, and `/code-review high` (security-critical: `--comment`).

| WS | Targeted test command(s) | Postgres? | Docker? | Notes |
|----|--------------------------|-----------|---------|-------|
| WS-01 | `cargo test -p thinclaw-config`, `-p thinclaw-tools`, `-p thinclaw-tools-core`, root `-p thinclaw web/sandbox/cli/oauth`; `cargo deny check`; `cargo check --workspace --no-default-features --features edge` | No | No (full sandbox E2E stays `#[ignore]`) | T13 real-WASM check wants `./scripts/build-all.sh` artifacts present. |
| WS-02 | `cargo test --test schema_divergence --no-default-features --features "postgres libsql" -- --test-threads=1`; `cargo test --test db_contract` (both `DATABASE_BACKEND=libsql` and `=postgres`) | **Yes** for postgres path + schema_divergence | No | Needs `pgvector/pgvector:pg17` + migrations applied (mirror CI). |
| WS-03 | per-crate `cargo build --target wasm32-wasip2` in each `channels-src/*`, `tools-src/*`; `./scripts/build-all.sh`; host `cargo test -p thinclaw-channels wasm` | No | No | Standalone Cargo workspaces — root clippy does not cover them; add wasm32 target. |
| WS-04 | `cd apps/desktop/backend && cargo test` (own `sqlx` 0.8); `cargo build` | No | No | Workspace-excluded; gate runs inside the package. |
| WS-05 | `cargo test -p thinclaw extensions self_repair`; native-plugin tests; `cargo clippy --all-targets` (the `dlopen`/`unsafe` boundary) | No | No | `/code-review high` mandatory on the `unsafe` native-load path. |
| WS-06 | `cargo test -p thinclaw repo_project supervisor pipeline planner` | Optional | E2E only | The repo Docker E2E stays `#[ignore]` (WS-13 nightly). |
| WS-07 | `cargo test -p thinclaw experiments`, `-p thinclaw-experiments` | No (unit) | E2E only | Campaign Docker E2E stays quarantined (WS-13 issue). |
| WS-08 | `cargo test -p thinclaw-llm route_planner smart_routing rig_adapter`; `cargo test -p thinclaw llm` | No | No | Verify CheapSplit cascade fires once (not stacked). |
| WS-09 | `cargo test -p thinclaw routine heartbeat`; `-p thinclaw-agent routine heartbeat` | Both backends for dedup store method | No | dedup_window store method tested on Postgres + libSQL. |
| WS-10 | per-target `cargo build -p <crate> && cargo test -p <crate>` (compile failure = broken public path) | As per moved crate | No | Behavior-preserving; surface-diff review per file. |
| WS-11 | `cargo build --workspace`; `cargo clippy --all-targets`; `cargo check --features voice` (if voice WIRED) | No | No | Deleting orphans must not break the workspace; confirm no live caller. |
| WS-12 | doc-only: read-through against code; `cargo doc` optional | No | No | No behavior tests. |
| WS-13 | `cargo test --test schema_divergence …`; assert `--all-targets` in `ci.yml:52,121`; validate `nightly.yml` YAML | **Yes** (gates schema/db jobs) | Nightly Docker E2Es (self-skip on hosted) | Owns the *jobs*, not the assertions. |

**DB/Docker prerequisites (set up once, mirror CI):**
- Postgres-backed WS (WS-02, WS-13, optionally WS-06/WS-09 dedup tests): start `pgvector/pgvector:pg17`, create `thinclaw_test`, enable `vector`, **apply `migrations/V*.sql`** (broader integration tests need this — per `CLAUDE.md` dev note), set `DATABASE_URL`, run with `--test-threads=1`.
- Docker-backed E2Es (WS-06 repo executor, WS-07 campaign, sandbox smokes): need a local `thinclaw-worker:latest` image. These stay `#[ignore]` for the main gate; **WS-13's `nightly.yml`** runs them. If Docker stalls, check host disk (`df -h /System/Volumes/Data`) per the `CLAUDE.md` note before assuming a product bug.

---

## 6. Rollback & Resume

- **Resume a crashed/aborted Workflow:** re-invoke with `resumeFromRunId: "<runId>"` so completed phases (implement/verify) are not re-run; the loop picks up at the first incomplete phase. Keep each phase idempotent (the implement agent works on a branch; re-running re-reads the diff).
- **Worktree cleanup:** finished + merged → `ExitWorktree({ action: "remove" })`. Abandoned with uncommitted work → `ExitWorktree({ action: "remove", discard_changes: true })` only after confirming the work is dead; otherwise `{ action: "keep" }` and revisit. List stray worktrees with `git worktree list`; prune with `git worktree prune`.
- **Branch abandonment:** a WS that goes sideways → leave its branch unmerged, `git branch -D ws-XX-…` after capturing any salvage into a follow-up note. Because waves are file-disjoint, abandoning one WS branch does not block its wave-mates.
- **Bisect a regression:** because PRs are small and per-task, `git bisect start <bad> <last-known-good>` over the merge commits isolates the offending WS/task fast. Each WS doc's Definition of Done lists the negative tests — re-run the relevant one at the bisect midpoint. For a clippy/deny regression, `git bisect run cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- **Wave rollback:** if a merged wave breaks `main`, revert the wave's merge commits (they are contiguous) rather than cherry-pick-reverting tasks; re-open the WS branches from the revert point.

---

## 7. Decision Register Gate (operator sign-off BEFORE the owning wave runs)

> Each WS doc states a *recommended* option. The operator must confirm or override **before that WS's wave starts**, because several are wire-vs-erase calls that change scope/effort. Recommendations below are the WS docs' defaults.

**Wave 0 (WS-01) — sign off before security work:**
- **WS-01 DP-1 — HTTPS credential injection:** erase the 3 dead HTTPS default mappings (`api.openai.com`/`anthropic`/`near.ai`) vs build OOB delivery. *Rec: ERASE dead defaults, keep `with_credential_resolver` + HTTP path (Option A).*
- **WS-01 DP-2 — `execute_code` bare-host:** force `Always` approval on `LocalHost` vs feature-gate. *Rec: force `Always` on `LocalHost`/`RemoteRunnerAdapter`, keep `UnlessAutoApproved` for `DockerSandbox`.*
- **WS-01 DP-3 — filesystem `base_dir==None`:** hard error vs contain-to-cwd. *Rec: fail-closed to cwd containment.*
- **WS-01 DP-4 — WASM table/instance limits:** enforce vs delete reserved counters. *Rec: WIRE (enforce).*

**Wave 0 (WS-02) — sign off before DB work:**
- **WS-02 DP-1 — libSQL sanitizer location:** inline copy vs extract to shared `libsql::fts`. *Rec: extract (Option b).*
- **WS-02 DP-2 — adopt `expand_query_keywords` for transcript search?** *Rec: NO — quote-each-token only (avoid new ranking divergence).*

**Wave 1 — sign off before the parallel behavior fan-out:**
- **WS-04 DP-1 — cloud-sync: WIRE end-to-end vs FEATURE-GATE.** *Rec: WIRE (build the vision: activate FileStore cloud mode + upload worker + SyncEngine + read-path download + startup restore).* **HIGH-IMPACT — biggest effort/risk call in the plan; confirm before WS-04 starts.**
- **WS-04 — InferenceRouter chat path: wire vs remove the dead chat modality.** *Rec per doc: resolve the wire-or-remove (chat path is dead; `direct_chat_stream` bypasses the router).* Operator must pick wire vs erase.
- **WS-05 — native dynamic-library plugin pipeline: WIRE vs ERASE (~1500L, `dlopen`/`unsafe`).** *Rec: WIRE behind the existing `allow_native_plugins`/signature gates.* **SECURITY-SENSITIVE — confirm appetite for shipping an `unsafe` native-load path before WS-05 starts; demands the deepest review tier.**
- **WS-05 — self-repair `with_builder`: WIRE.** *Rec: WIRE (inject `LlmSoftwareBuilder` + `ToolRegistry`).* Low controversy; confirm.
- **WS-05 — observability `create_observer`: WIRE vs remove.** *Rec: WIRE through `AppBuilder`.*
- **WS-06 DP-1 — `NeedsPlanning`: build autonomous planner subagent vs downgrade to human status.** *Rec: build planner behind a `RepoTaskPlanner` port, Option-B fallback when no LLM wired.*
- **WS-06 DP-2/3/4 — concurrency knob precedence; merge-attempt bound→AwaitingHuman; `installation_id` persistence.** *Rec: per-project policy clamped by config ceiling; bounded counter→AwaitingHuman (default 3); webhook-time backfill first.* Confirm defaults.
- **WS-09 DP-3 — `dedup_window`: WIRE vs ERASE.** *Rec: WIRE (hash + decision variant already exist); ERASE-all-five-sites is the acceptable fallback if sizing forces a cut.* Confirm which.
- **WS-09 DP-1/2/4/5 — heartbeat `target`/`include_reasoning` WIRE; webhook body WIRE; standalone heartbeat runner ERASE.** *Rec: as stated.* Confirm the ERASE of `spawn_heartbeat`/`HeartbeatRunner::run`.

**Wave 2 — sign off before LLM/experiments shared-file work:**
- **WS-08 DP-1 — which routing engine survives: RoutePlanner canonical, retire `SmartRoutingProvider`.** *Rec: RoutePlanner canonical (Option A).* **ARCHITECTURE-DEFINING — confirm before deleting the decorator.**
- **WS-08 DP-2 — CheapSplit cascade: WIRE vs erase.** *Rec: WIRE the planner's computed `decision.cascade`.*
- **WS-08 DP-3 — erase `SpawnSubagentTool.executor` (yes) and `Reasoning.safety` (coordinate timing with WS-10).** Confirm the `reasoning.rs` rider sequencing.
- **WS-07 DP-1 — durable artifact storage backend: host-side copy (Option A) vs opendal S3 (Option B), do NOT gate.** *Rec: Option A now behind an `ArtifactStore` port.*
- **WS-07 DP-2/3/4 — reaper as dedicated loop; surface (not gate) RunPod credit≈USD; fix only unambiguous error-taxonomy mis-classifications.** *Rec: as stated.*

**Wave 3 (WS-10) — sign off before the overhaul:**
- **WS-10 — confirm all of WS-01..09 merged** (hard dependency) and the per-file decomposition recipes. No wire-vs-erase, but the operator gates the *go* on a green `main`.

**Wave 4 (WS-11) — sign off before deletions:**
- **WS-11 DP — `src/safety/*` 14 orphans: ERASE.** *Rec: ERASE (drifted duplicates of live `thinclaw-safety`, won't compile).*
- **WS-11 — `src/cli/{nodes,subagent_spawn,session_export}.rs`: ERASE vs WIRE.** *Rec: ERASE (abandoned; live surfaces exist). Escalate to a dedicated WS if operator wants them as real commands.*
- **WS-11 — `voice_wake` / `tailscale` (discovery) / `qr_pairing`: WIRE vs ERASE.** *These are the operator's "realize the vision" calls.* Defaults lean ERASE (no profile enables `voice`; discovery + qr_pairing are parallel never-connected mechanisms with security defects), but the directive prefers WIRE where viable — **operator decides each.** If `qr_pairing` is WIRED, the non-constant-time compare + hand-rolled base64 must be fixed first.
- **WS-11 — `self_message` module: ERASE vs WIRE the anti-loop guarantee.** *Rec: ERASE unless the anti-loop guarantee is wanted (it's currently unenforced/dead).*

---

## 8. Kickoff — first commands to run next turn (start Wave 0)

**Step 0 — operator sign-off** on the Wave-0 decision register (WS-01 DP-1..4, WS-02 DP-1..2 in §7).

**Step 1 — establish the green baseline (WS-01 lead-in, serial).** Branch and run the three lead-in tasks:
```bash
git switch -c ws-01-security main
cargo update -p wasmtime-wasi --precise 36.0.11   # T1 (also bump wasmtime to 36.0.11 if resolver requires lockstep)
cargo deny check                                   # confirm RUSTSEC-2026-0182 cleared
# T2: edit deny.toml (header → ci.yml codestyle job; drop stale RUSTSEC-2026-0098/0099/0104 ignores)
# T3: ci.yml:52 and :121 → add --all-targets; fix the await_holding_lock in thinclaw-config/src/secrets.rs
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

**Step 2 — launch Wave 0 Workflows in parallel** (WS-01 fan-out after lead-in merges + WS-02 + WS-12 seed):
```js
// Run the WS-01 internal A/B/C/D fan-out (see WS-01 §Multi-Worker plan) once T1-T3 merge:
await pipeline("ws-single", { wsId: "WS-01", wsDoc: "docs/remediation/01-security-and-ci-hardening.md",
  crate: "thinclaw-config", testFilter: "secrets gateway", base: "main", branch: "ws-01-security" });

// Concurrently (disjoint files): WS-02 DB correctness + WS-12 inventory seed.
await parallel([
  () => pipeline("ws-single", { wsId: "WS-02", wsDoc: "docs/remediation/02-database-correctness-and-parity.md",
        crate: "thinclaw-db", testFilter: "fts conversations schema", base: "main", branch: "ws-02-db" }),
  () => pipeline("ws-single", { wsId: "WS-12", wsDoc: "docs/remediation/12-docs-and-drift-sync.md",
        crate: "thinclaw", testFilter: "", base: "main", branch: "ws-12-docs-seed" }),
]);
```

**Step 3 — provision the Postgres prereq for WS-02** (mirror CI) before its verify phase:
```bash
docker run -d --name tc-pg -e POSTGRES_PASSWORD=pg -p 5432:5432 pgvector/pgvector:pg17
# create thinclaw_test, enable vector, apply migrations/V*.sql, export DATABASE_URL, then run with --test-threads=1
```

**Step 4 — merge Wave 0 to `main`** (WS-01 lead-in first, then WS-01 fan-out + WS-02 + WS-12 seed), confirm `main` is green under the new `--all-targets` + `cargo deny`, then launch **Wave 1** via `wave1.workflow.js` (§4.2).

---

*Generated 2026-06-23 from `AUDIT-FINDINGS.md` + `WS-01..WS-13`. Index: `README.md`. The waves, conflict register, and decision gate are the load-bearing parts — run WS-01 first, WS-10/WS-11 last, and let WS-12 trail.*
