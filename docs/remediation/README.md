# ThinClaw Remediation Plan

> **Date:** 2026-06-23 · **Scope:** whole workspace (~543K LoC Rust, 25 crates + root facade + WASM channels/tools + Tauri desktop).
> This is the navigational hub for the remediation effort. Findings live in [`AUDIT-FINDINGS.md`](./AUDIT-FINDINGS.md); the work is split into 13 numbered workstreams (`WS-01` … `WS-13`) plus a global guardrails reference; execution sequencing lives in [`EXECUTION-PLAYBOOK.md`](./EXECUTION-PLAYBOOK.md).

## Overview & Vision

ThinClaw is already a **production-grade, fully-wired personal-agent platform** — the audit's headline conclusion. Its load-bearing systems (multi-session agent loop, routine/scheduler/heartbeat engine, tool registry + MCP runtime, twelve real WASM tool integrations, multi-provider LLM routing/failover/cache, both DB backends, the ~150-route web gateway, the autonomous experiments platform, and identity/soul/memory) are real and shipping.

What is **half-wired, aspirational, broken, or poorly-architected is concentrated and identifiable** — it clusters at the *frontier*: trust boundaries, the desktop app's newer subsystems, and the externally-packaged WASM channels. This plan exists to push that frontier to "done."

**"Done" means every audit finding is resolved with a clear disposition — and the disposition is biased toward *realizing the vision*, not amputating it:**

- **Built-but-inert capability is WIRED end-to-end.** Desktop cloud-sync ships (gated by storage mode, not a compile flag). The self-repair automatic-rebuild path gets its `with_builder` injection. The signature-verified native-plugin pipeline becomes reachable (default-off, signatures-required). The observability `create_observer` factory is threaded through `AppBuilder`. CheapSplit cascade execution, heartbeat target/verbosity, webhook-body pass-through, repo-project planning + concurrency + SSE consumers, and the WASM table/instance limits all get connected.
- **Trust-boundary holes are CLOSED.** Empty-token auth bypass, DNS-rebinding, OAuth state validation, store-backed proxy credential resolution, `execute_code`/filesystem containment, and the libSQL FTS5 MATCH divergence are all fixed, and the docs that overclaimed are re-grounded in code truth.
- **Genuinely drifted duplicate cruft is ERASED — explicitly and with sign-off.** ~7K lines of dead/drifted code (14 `src/safety/*` orphans, 3 unwired CLI modules, dead helpers, `self_message`), the dead `InferenceRouter` chat modality, two leaky-abstraction fields, the orphaned standalone heartbeat runner, the near-byte-identical `src/history/store` duplicate, and `qr_pairing`.
- **God-files are decomposed behavior-preservingly** (`thread_ops.rs` 3032L, `wrapper.rs` 5701L, the 5434L experiments file, and 8 more) one-file-per-PR with public paths preserved by re-export.
- **The CI gate is green and stays green** — `main` is currently red (`cargo deny`), and the remediation restores a hard gate (`fmt`; `clippy --all-targets --all-features -D warnings`; `cargo test`; `cargo deny`) plus a nightly `#[ignore]` matrix.

When this plan completes, no finding remains in an ambiguous "half-built" state: each is either a shipping feature or an honest, signed-off deletion.

## How to read this directory

| Doc | What it is | Read it when |
|---|---|---|
| [`AUDIT-FINDINGS.md`](./AUDIT-FINDINGS.md) | Source-of-truth findings record (subsystem status, confirmed bugs, risks). | You want the evidence behind a task. |
| [`BEST-PRACTICES-AND-PITFALLS.md`](./BEST-PRACTICES-AND-PITFALLS.md) | Global engineering guardrails (CC-A): façade hygiene, dependency-direction, feature-matrix, common pitfalls. | Before authoring or reviewing any change. |
| [`EXECUTION-PLAYBOOK.md`](./EXECUTION-PLAYBOOK.md) | The ULTRACODE execution model: DAG, 5-wave plan, shared-file conflict register, worktree strategy, `Workflow()` skeletons, rollback/resume. | Before scheduling/running any wave. |
| [`COVERAGE-CRITIC.md`](./COVERAGE-CRITIC.md) | Adversarial coverage review: DAG acyclicity, uncovered findings, cross-WS conflicts, label-drift notes. | To confirm nothing fell through the cracks. |
| [`01`](./01-security-and-ci-hardening.md) … [`13`](./13-test-and-ci-infrastructure.md) `WS-*.md` | One executable workstream each: ordered tasks with file:line anchors, the change, acceptance criteria, verification command, and decision points. | When executing a specific workstream. |

## Workstream Index

Sorted P0 → P2. Effort: S/M/L/XL.

| ID | Title | Pri | Risk | Effort | Depends on | Doc |
|---|---|---|---|---|---|---|
| WS-01 | Security & CI Hardening | P0 | medium | L | — | [01](./01-security-and-ci-hardening.md) |
| WS-02 | Database Correctness & Backend Parity | P0 | medium | M | — | [02](./02-database-correctness-and-parity.md) |
| WS-03 | WASM Channels & Tools Repair + Shared SDK | P1 | medium | L | — | [03](./03-wasm-channels-tools-repair-and-sdk.md) |
| WS-04 | Desktop App Completion (cloud-sync, inference, dual stacks) | P1 | high | L | — | [04](./04-desktop-app-completion.md) |
| WS-05 | Self-Repair, Extensions & Native-Plugin Pipeline | P1 | medium | L | — | [05](./05-self-repair-extensions-native-plugins.md) |
| WS-12 | Docs & Drift Sync | P1 | low | M | — | [12](./12-docs-and-drift-sync.md) |
| WS-13 | Test & CI Infrastructure | P1 | medium | L | WS-01, WS-02 | [13](./13-test-and-ci-infrastructure.md) |
| WS-06 | Repo-Project Supervisor Completion | P2 | medium | L | — | [06](./06-repo-project-supervisor-completion.md) |
| WS-07 | Experiments / Research Platform Completion | P2 | medium | L | — | [07](./07-experiments-research-completion.md) |
| WS-08 | LLM Stack Consolidation | P2 | medium | L | — | [08](./08-llm-stack-consolidation.md) |
| WS-09 | Routines / Scheduler / Heartbeat Completion | P2 | low | M | — | [09](./09-routines-scheduler-heartbeat-completion.md) |
| WS-10 | Architecture Overhaul (god-files & crate migrations) | P2 | high | XL | WS-01…WS-09 | [10](./10-architecture-overhaul.md) |
| WS-11 | Dead-Code Sweep & Vision Decisions | P2 | low | L | WS-05, WS-10 | [11](./11-dead-code-sweep-and-vision-decisions.md) |
| — | Global Best Practices & Common Pitfalls (CC-A) | P1 | low | M | — | [guardrails](./BEST-PRACTICES-AND-PITFALLS.md) |

## Wave / Execution Summary

The DAG is **acyclic** (confirmed by the coverage critic). Each WS is one branch + one `implement → verify → adversarial-review → fix` Workflow, merged as small PRs only behind a green gate (`fmt`; `clippy --all-targets --all-features -D warnings`; per-crate/full `cargo test`; `cargo deny`; `/ship`; `/code-review high`). Five waves; see [`EXECUTION-PLAYBOOK.md`](./EXECUTION-PLAYBOOK.md) for the full DAG, worktree isolation, `Workflow()` skeletons, and rollback/resume detail.

| Wave | Contents | Why grouped |
|---|---|---|
| **Wave 0** | **WS-01** (serial T1→T2→T3 lead-in restoring the green CI gate + lockfile, then A/B/C/D fan-out) + **WS-02** + a **WS-12** inventory seed | Nothing merges until CI is green again; DB correctness is independent and P0. |
| **Wave 1** | **WS-03, WS-04, WS-05, WS-06, WS-09** in parallel | Independent behavior fixes; depend only on the Wave-0 green baseline. |
| **Wave 2** | **WS-07** (`api/experiments.rs`) + **WS-08** (`llm/runtime_manager.rs`) | WS-10 prerequisites that must land *additively* on eventual split-targets before any split. |
| **Wave 3** | **WS-10** god-file/crate-migration overhaul | Serialized last; one file per agent, public surface diffed. |
| **Wave 4** | **WS-11** dead-code sweep (after WS-05 + WS-10) | Deletions only safe once consumers/migrations have landed. |
| *Trailing* | **WS-12** doc-absorb pass per wave; **WS-13** test/CI infra after WS-01 + WS-02 | Doc sync follows the code; nightly/gating verifies the flags landed. |

**Shared-file serialization** (do NOT co-edit) — seven hot files require ordered edits rather than parallel ones: `Cargo.lock`, `.github/workflows/ci.yml`, `src/api/experiments.rs`, `src/llm/runtime_manager.rs`, `src/llm/reasoning.rs`, `src/agent/routine_engine.rs`, `src/agent/agent_loop.rs`, `src/extensions/manager.rs`. The full sequencing rules are in the playbook's §2.4 conflict register.

## Decision Register

Every `decision_point` across all workstreams, with the recommended disposition and the wave that must have operator sign-off *before* it runs. Full rationale lives in each WS doc and in the playbook's §7. Items that delete built code are marked **(deletes code → sign-off)**.

| Decision | WS | Options | Recommendation | Sign-off before |
|---|---|---|---|---|
| HTTPS credential injection (Finding #7) | WS-01 | ERASE 3 dead HTTPS default mappings vs BUILD out-of-band delivery | ERASE the dead HTTPS defaults; keep `with_credential_resolver` + HTTP forward path alive | Wave 0 |
| `execute_code` approval when backend is LocalHost (§8) | WS-01 | Force `ApprovalRequirement::Always` vs feature-gate bare-host exec | `Always` (capability stays, bare-host runs need approval) | Wave 0 |
| Filesystem `base_dir==None` (§9) | WS-01 | cwd-containment vs hard `NotAuthorized` | Fail-closed via `current_dir()` containment | Wave 0 |
| WASM table/instance limits (§11) | WS-01 | WIRE counters/enforcement vs delete reserved counters | WIRE (built-but-disconnected) | Wave 0 |
| libSQL FTS5 sanitizer home | WS-02 | New `thinclaw-db/.../libsql/fts.rs` module vs copy-paste into `conversations.rs` | Extract once into a domain-named module, re-exported via façade | Wave 0 |
| Transcript search keyword expansion | WS-02 | Adopt `expand_query_keywords` vs quote-each-token only | NO — quote-only shared sanitizer; workspace keeps its keyword-OR on top | Wave 0 |
| `schema_divergence` on missing `DATABASE_URL` | WS-02 | FAIL (panic) vs skip | FAIL — cfg-gated, only the CI job runs it | Wave 0 |
| `db_contract` fail-vs-skip | WS-02 | Flip to panic vs leave skip-on-missing-DB | Leave skip; gate via WS-13 CI job (don't break local dev) | Wave 0 |
| CI dual-backend gating ownership | WS-02 | WS-02 edits ci.yml vs WS-13 owns gating | WS-02 owns assertions only; WS-13 owns execution/gating | Wave 0 |
| Shared SDK packaging | WS-03 | New crate vs shared `include!` `.rs` module | Option B (`include!` mirroring `shared_webhook_channel`) | Wave 1 |
| Discord Ed25519 verification | WS-03 | WIRE vs feature-gate vs erase-flag | WIRE (all infra exists; forgeable otherwise) | Wave 1 |
| Shim signature validation | WS-03 | Tighten now vs document-as-equals | Classify + document now; tighten opportunistically | Wave 1 |
| Cloud sync | WS-04 | WIRE end-to-end vs feature-gate | BUILD (runtime `StorageMode::Cloud` guard, not compile feature) | Wave 1 |
| InferenceRouter chat modality | WS-04 | WIRE `chat.rs` onto router vs ERASE | **ERASE** — drifted duplicate cruft, zero non-router callers **(deletes code → sign-off)** | Wave 1 |
| Dual agent stacks (System A/B) | WS-04 | Consolidate vs document | DOCUMENT — intentional per `runtime-boundaries.md`; add two-MCP/two-provider addendum | Wave 1 |
| Native dynamic-library plugin pipeline | WS-05 | WIRE (default-off, signatures-required) vs ERASE ~1800L | WIRE (all safety gates already implemented + tested) **(unsafe/dlopen → sign-off)** | Wave 1 |
| Observability `create_observer` | WS-05 | WIRE through `AppBuilder` vs remove config/wizard surface | WIRE (wizard already collects the choice) | Wave 1 |
| Self-repair builder injection | WS-05 | WIRE `with_builder` at `agent_loop.rs:605` | WIRE (adapter implemented, only the call is missing) | Wave 1 |
| Orphaned `RepairTask` (`thinclaw-agent/self_repair.rs:325`) | WS-05 | WIRE vs ERASE | **MUST decide** here (coverage critic flags risk of falling through WS-05/WS-11) | Wave 1 |
| NeedsPlanning | WS-06 | Build autonomous planner subagent vs downgrade to human signal | Build behind a `RepoTaskPlanner` port; `AwaitingHuman` fallback when no planner | Wave 1 |
| Concurrency precedence | WS-06 | Per-project policy vs global env ceiling | Per-project authoritative, clamped by config ceiling | Wave 1 |
| Merge-attempt bound | WS-06 | counter→Block vs counter→AwaitingHuman | Bounded counter → `AwaitingHuman` (default max 3), both non-success arms | Wave 1 |
| `installation_id` persistence | WS-06 | Enroll-time vs webhook-time vs both | Both, webhook-time first | Wave 1 |
| Heartbeat target (none/chat/channel) | WS-09 | WIRE vs ignore | WIRE onto delivery path | Wave 1 |
| Heartbeat `include_reasoning` | WS-09 | WIRE vs ignore | WIRE through `heartbeat_job_metadata` | Wave 1 |
| `dedup_window` | WS-09 | WIRE (default) vs ERASE | **WIRE** — half-finished capability; ERASE only if sizing forces, then remove all 5 sites atomically **(operator sign-off point)** | Wave 1 |
| Webhook body pass-through | WS-09 | WIRE vs drop | WIRE via `RoutineRun.trigger_detail` | Wave 1 |
| Orphaned standalone heartbeat runner | WS-09 | ERASE vs keep | **ERASE** — zero callers, superseded by routine engine **(deletes code → sign-off)** | Wave 1 |
| Per-event error isolation | WS-09 | break-on-first vs continue + diagnostics | BUILD continue + accumulated diagnostics | Wave 1 |
| Durable artifact storage backend | WS-07 | Host-side `ArtifactStore` port vs opendal/S3 | Option A (BUILD now behind a port; B slots in later) | Wave 2 |
| Reaper home | WS-07 | Dedicated reaper loop vs fold into reconcile | Option A (dedicated daily loop beside the controller) | Wave 2 |
| RunPod credit≈USD | WS-07 | Gate vs surface | SURFACE into `cost_summary` + runner details + docs; no gate | Wave 2 |
| Error-taxonomy scope (`api/experiments.rs`) | WS-07 | Fix all 106 `Internal` maps vs only mis-classifications | Fix only unambiguous mis-classifications; defer flattening to WS-10 | Wave 2 |
| Canonical routing engine | WS-08 | RoutePlanner vs SmartRoutingProvider decorator | WIRE RoutePlanner canonical; retire the decorator **(deletes code → sign-off)** | Wave 2 |
| CheapSplit cascade (computed-but-dropped) | WS-08 | WIRE through `ResolvedRoute` vs erase | WIRE inspect-and-escalate; reuse `response_is_uncertain` before deleting it | Wave 2 |
| Dead leaky-abstraction fields | WS-08 | ERASE `SpawnSubagentTool.executor` + `Reasoning.safety` | ERASE both; coordinate `reasoning.rs` edit sequencing with WS-10 **(deletes code → sign-off)** | Wave 2 |
| `history/store` consolidation | WS-10 | Thin-facade re-export vs delete-and-redirect | DELETE the root duplicate (crate is ahead); keep paths via `pub use` **(deletes code → sign-off)** | Wave 3 |
| `media` types ownership | WS-10 | Where `MediaExtractor`/`MediaPipeline` traits live | Move into `thinclaw-media`; leave `MediaContent`/`MediaType` in `thinclaw-types` | Wave 3 |
| `wrapper.rs` Telegram extraction | WS-10 | `WasmChannelTransport` trait vs inline submodule | The trait (generic host separated from Telegram branches) | Wave 3 |
| `experiments.rs` taxonomy ownership | WS-10 | WS-07 maps vs WS-10 maps | WS-07 defines mapping; WS-10 carries edits during the split | Wave 3 |
| Desktop decompositions sequencing | WS-10 | In-scope now vs defer | Keep in WS-10 but sequence LAST, after WS-04 consolidates | Wave 3 |
| `src/safety/*` (14 orphans) | WS-11 | WIRE vs ERASE | **ERASE** — drifted duplicates of live crate; won't compile **(deletes code → sign-off)** | Wave 4 |
| `src/cli/{nodes,subagent_spawn,session_export}.rs` | WS-11 | WIRE vs ERASE | **ERASE** — covered by live surfaces; wiring is out-of-scope command design **(deletes code → sign-off)** | Wave 4 |
| `self_message` | WS-11 | WIRE vs ERASE | **ERASE** — zero callers; doc-vs-behavior lie is itself a hazard **(low-confidence → sign-off)** | Wave 4 |
| `voice_wake` + `voice` feature + `cpal` | WS-11 | WIRE vs ERASE | **WIRE** behind existing `voice` feature; do NOT auto-enable in any profile | Wave 4 |
| `tailscale` `TailscaleDiscovery` | WS-11 | WIRE vs ERASE | **WIRE** into deployment auto-discovery under `tunnel` feature | Wave 4 |
| `qr_pairing` | WS-11 | WIRE vs ERASE | **ERASE** — parallel never-connected fallback; non-constant-time compare + hand-rolled base64 **(deletes code → sign-off)** | Wave 4 |
| FEATURE_PARITY §20 tool list | WS-12 | Regenerate vs drop count vs delete section | Drop only the `(80 max)` count + dated line; keep tables; name registry authoritative | Wave 0 seed |
| Stale "Scrappy" doc-comments | WS-12 | Rename vs delete | RENAME → "ThinClaw Desktop"; spare all legacy/migration identifiers | per-wave |
| Discord README fix timing | WS-12 | Fold into WS-03 PR vs WS-12 follow-up | Fold into WS-03's PR (same-PR rule); WS-12 picks up if WS-03 ships without it | Wave 1 |

## Coverage & Open Reconciliations

The coverage critic confirms the **DAG is acyclic** and every depended-on WS precedes its dependent (no back-edge). Coverage is *near-complete* but the operator must reconcile the following before execution:

**Previously-uncovered findings — now assigned (resolved):**

- **`image_gen.rs:700` divide-by-zero progress label** — RESOLVED. Assigned to **WS-04** (sole owner of the excluded `apps/desktop/backend` package).
- **`build-all.sh` never builds `tools-src` + `channel-crates` CI matrix** (13 WASM shims omitted) — RESOLVED. Assigned to **WS-13** (build-all.sh + channel-crates CI matrix added to its Owns/tasks).
- **`JobToolHostPort` half stubbed `Unavailable`** — RESOLVED. Assigned to **WS-10**.
- **`wasm-runtime` absent from the desktop profile** — RESOLVED. Desktop-profile gap assigned to **WS-04**.

**Conflicts to confirm sequencing on:**

- `discord/README.md` → WS-03 owns (land in same PR as code); WS-12 T9 is verify-only/conditional.
- `src/cli/session_export.rs` + `src/voice_wake.rs` → WS-11 owns file fate; WS-12 T8 must sequence *after* WS-11 and drop `session_export.rs` once erased.
- `src/extensions/manager.rs` → order WS-05 (arms) → WS-10 (decompose) → WS-11 (delete helper).
- `src/llm/reasoning.rs` `Reasoning.safety` → WS-08 owns semantic removal; WS-10 absorbs.
- **`RepairTask` (`thinclaw-agent/self_repair.rs:325`)** → RESOLVED in **WS-05**, which now records a concrete decision: **ERASE (deletes code → sign-off)**.

**Sign-off-gated deletions** the operator must approve: WS-04 DP-2 (InferenceRouter chat), WS-09 DP-3 (`dedup_window`) + DP-5 (standalone heartbeat runner), WS-11 DPs 3–6 (`self_message`, `voice_wake`, `tailscale`, `qr_pairing`), WS-08 DP-1/DP-3, WS-10 history/store delete. `self_message` and `conversation_metadata_with_handoff` are low-confidence erase calls — confirm before deleting.

**Label-drift (cosmetic, work is covered) — RESOLVED:** WS-02's three misrouted findings (sandbox→WS-01 not WS-03; desktop cloud-sync→WS-04 not WS-12; history/store→WS-10 not WS-09) and WS-08 T7's nonexistent "WS-14" (actually WS-11) cross-references are now corrected so executors aren't misled.

Full detail: [`COVERAGE-CRITIC.md`](./COVERAGE-CRITIC.md).

## Status Tracker

- [ ] **WS-01** Security & CI Hardening (P0) — 14 tasks
- [ ] **WS-02** Database Correctness & Backend Parity (P0) — 5 tasks
- [ ] **WS-03** WASM Channels & Tools Repair + Shared SDK (P1) — 6 tasks
- [ ] **WS-04** Desktop App Completion (P1) — 12 tasks
- [ ] **WS-05** Self-Repair, Extensions & Native-Plugin Pipeline (P1) — 10 tasks
- [ ] **WS-12** Docs & Drift Sync (P1) — 10 tasks
- [ ] **WS-13** Test & CI Infrastructure (P1) — 7 tasks
- [ ] **WS-06** Repo-Project Supervisor Completion (P2) — 8 tasks
- [ ] **WS-07** Experiments / Research Platform Completion (P2) — 7 tasks
- [ ] **WS-08** LLM Stack Consolidation (P2) — 8 tasks
- [ ] **WS-09** Routines / Scheduler / Heartbeat Completion (P2) — 6 tasks
- [ ] **WS-10** Architecture Overhaul (P2) — 12 tasks
- [ ] **WS-11** Dead-Code Sweep & Vision Decisions (P2) — 10 tasks
- [ ] **CC-A** Global Best Practices & Common Pitfalls (reference) — guardrails doc complete

**Wave gate:** [ ] Wave 0 green · [ ] Wave 1 merged · [ ] Wave 2 merged · [ ] Wave 3 merged · [ ] Wave 4 merged · [ ] `main` green under `--all-targets` + `cargo deny`
