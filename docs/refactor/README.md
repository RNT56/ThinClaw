# ThinClaw Repo Refactor — Execution Documentation

This directory is the **complete, execute-ready playbook** for taking ThinClaw from its
current state to *extreme robustness and flawless architecture*. It is self-contained: an
engineer or agent should be able to execute the entire refactor end-to-end from these docs
without re-deriving anything.

## The documents

| Doc | What it is |
|---|---|
| [`README.md`](README.md) | This file — goals, status, how to use, index. |
| [`PRINCIPLES.md`](PRINCIPLES.md) | Best practices: engineering principles + this repo's own architecture rules + the lessons learned executing Phase 0. **Read before touching code.** |
| [`EXECUTION_PLAYBOOK.md`](EXECUTION_PLAYBOOK.md) | The *how*: worktree/branch/PR workflow, the verification gate per change-type, lockfile discipline, conflict/queue management, and the known gotchas. **The procedure for every refactor PR.** |
| [`BACKLOG.md`](BACKLOG.md) | The *what*: every remaining task as a self-contained executable unit — current state (file:line), steps/todos, verification, done-criteria, dependencies, conflict notes. **The work list.** |
| [`METRICS_AND_GUARDRAILS.md`](METRICS_AND_GUARDRAILS.md) | The baseline→target metrics dashboard and the full CI-guardrail catalog (each with feasibility/prerequisites). **How we know we're done and how we keep it fixed.** |

Strategic context lives in the companion roadmap
[`../ROBUSTNESS_AND_ARCHITECTURE_PLAN.md`](../ROBUSTNESS_AND_ARCHITECTURE_PLAN.md), which was
produced from a 10-dimension code audit. These `refactor/` docs are the *execution-level*
expansion of that plan.

## Goal — what "done" looks like

A codebase where:
- **No silent-wrongness.** Anything that compiles is correct: no false security guarantees, no
  blocking-in-async, no panics on hot paths, no lockfile drift, no unscanned dependencies.
- **The architecture is enforced, not aspirational.** No dependency cycles, no wrong-direction
  edges, no god-files — and CI guards each invariant so it cannot regress.
- **Extending the system is cheap.** Adding a `StatusUpdate` variant, a command, or a channel
  touches one place, not five; the protocol/enum/schema surfaces evolve safely.
- **Incidents are diagnosable.** Persistent logs, real health/readiness, metrics, and complete
  lifecycle observability.
- **The supply chain is clean.** Every workspace + lockfile is advisory-scanned; no duplicate or
  EOL crates; deps are pinned and inherited from one source of truth.

Measured by the dashboard in [`METRICS_AND_GUARDRAILS.md`](METRICS_AND_GUARDRAILS.md)
(e.g. god-files 18→0, wrong-direction crate edges 2→0, ROUTE_TABLE coverage 4%→100%).

## How to use this for end-to-end execution

1. Read [`PRINCIPLES.md`](PRINCIPLES.md) and [`EXECUTION_PLAYBOOK.md`](EXECUTION_PLAYBOOK.md) once.
2. Pick the next unblocked task from [`BACKLOG.md`](BACKLOG.md) (respect the **Sequencing** order
   and the **Blocked-by** field — do not start a Wave-B item while its in-flight blocker is open).
3. Execute it following the playbook (worktree → change → the task's Verify steps → PR → auto-merge).
4. One task = one PR. Keep PRs focused and reviewable. Ship the fix **with its guardrail** when the
   task has one.
5. After each wave, re-run the audit workflow and update the metrics dashboard.

Line numbers in the backlog originate from the 2026-06-29 audit and **drift** — always re-locate the
symbol (`grep`/`rg`) before editing; treat the file + the pattern as authoritative, the line as a hint.

## Status snapshot (2026-07-11)

The audit-hardening stack has landed on `main` (merges `1fb29984` / `bda7a61f`). Most of Phase 1
Wave A and the unblocked Wave B items shipped; the Phase 2 metrics endpoint shipped too. What
remains is dependency dedup, a handful of lint/coverage/release guardrails, and the long-tail
typed-error and file-size targets.

- **Phase 0 — COMPLETE** (verified): security correctness (#126), blocking-in-async (#127), panic
  resilience (#128), CI `--locked` guardrail (#129), desktop dependency remediation + advisory CI
  (#130). Plan/audit: #125.
- **Phase 1 / Wave A — MOSTLY LANDED**: crate-boundary moves (A1 routine DTOs → `thinclaw-types`,
  A2 MCP/execution DTOs → `thinclaw-tools-core`, both CI-guarded), the observability bundle (A3
  rolling daily file sink, A4 real `/api/health` readiness), and the god-file decompositions (A5
  experiments `lib.rs` now a 22-line façade, A7 skill-tool twins split into directories, A8 signal /
  providers / server / routine-engine now under the 2,000-line guard) are done. The god-file size
  guard is LIVE in CI. **Still open in Wave A:** A6 (`async_main` extracted to `src/async_main.rs`
  but still ~1,928 lines), the `acp` / `reasoning` mod files (~1,900 lines, under the guard but not
  yet at the < 800 target), and the security long-tail (A10). A9 (async lifecycle) is **partial**:
  long-running loops are owned and drained, while channel-submission and scheduler cleanup waiters
  remain detached one-shot tasks. A11 (panic long-tail) is **partial**: every named production
  panic site is resolved, but the systemic
  `clippy::unwrap_used` lint is still `"allow"`.
- **Phase 1 / Wave B — PARTIALLY LANDED** (no longer blocked): B1 (`StatusUpdate` is now
  `#[non_exhaustive]`), B2 (all 10 `ObserverEvent` variants have production emit sites, zero dead
  variants), B3 (WIT `status-type` now covers every host `StatusUpdate` variant, conversions map
  each explicitly, and the interface is versioned at `CHANNEL_WIT_VERSION = "0.2.0"`), and B4
  (`ROUTE_TABLE` classifies all 346 commands, 100%, test-enforced) are done.
  **Still open:** B5 (313/342 commands still return `Result<_, String>`), B6 (stringly-typed
  `UiEvent` status fields).
- **Phase 2/3 — metrics endpoint SHIPPED**: the Prometheus `/metrics` route is registered and
  backed by the shared registry (`src/channels/web/server.rs:875`). **Still open:** LLM extraction
  (reasoning/runtime_manager behind ports) and the maturity long-tail.

**What actually remains** (do not treat these as done): dependency dedup (D2/D3; the root lock has
82 `cargo deny` duplicate diagnostics, 3 `rand` versions, 2 `wit-bindgen` versions, and
`deny.toml` still has `multiple-versions = "warn"`), finishing D1 (the `[workspace.dependencies]`
table exists and 27/28 crates use it, but `tokio`/`uuid`/`reqwest`/`rand` are not hoisted), the
`clippy::unwrap_used` panic-prevention lint (still `"allow"`), expanding coverage beyond the
library target (a measured 38% project floor and 70% changed-line gate are now live), a signed
desktop release (P3), sub-workspace `cargo-deny` scanning of
`channels-src/` + `tools-src/`, the `Result<_, String>` → `BridgeError` command migration (B5),
the remaining A9 one-shot task ownership, and the largest-file < 800 target (A8: largest is now
1,999 lines, not 4,577).

See [`BACKLOG.md`](BACKLOG.md) for the per-task detail.
