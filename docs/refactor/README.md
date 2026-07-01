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

Line numbers in the backlog are from the 2026-06-29 audit and **drift** — always re-locate the
symbol (`grep`/`rg`) before editing; treat the file + the pattern as authoritative, the line as a hint.

## Status snapshot (2026-06-29)

- **Phase 0 — COMPLETE** (shipped this cycle, each verified): security correctness (#126), async
  blocking-in-async (#127), panic resilience (#128), CI `--locked` guardrail (#129), desktop
  dependency remediation + advisory CI (#130). Plan/audit: #125.
- **Phase 1 / Wave A — NOT STARTED** (clean, ready now): crate-boundary moves, observability
  bundle, god-file decompositions, async-lifecycle, security/panic long-tail.
- **Phase 1 / Wave B — BLOCKED on the in-flight queue** (#117/#118/#119 touch the same files):
  `StatusUpdate #[non_exhaustive]`, observer-event emission, WIT drift, command-surface migration.
- **Phase 2/3 — NOT STARTED**: metrics endpoint, LLM extraction, maturity long-tail.

See [`BACKLOG.md`](BACKLOG.md) for the per-task detail.
