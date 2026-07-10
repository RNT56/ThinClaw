# ThinClaw Desktop — Overhaul, Upgrade & Refinement Plan

> **Status:** Living roadmap (draft v1) · **Created:** 2026-06-27 · **Owner:** _TBD_
> **Scope:** End-to-end overhaul of ThinClaw Desktop (`apps/desktop/`).
> **Companion:** executable backlog in [`OVERHAUL_BACKLOG.md`](OVERHAUL_BACKLOG.md).
>
> **Progress (verified 2026-07-10):** the first parity-closure batch is **merged to `main`**:
> the dual-mode bridge contract (`RouteMode`/`BridgeError`/`ROUTE_TABLE` in
> `apps/desktop/backend/src/thinclaw/bridge.rs`), real per-thread compaction
> (`thinclaw_compact_session` in `rpc_extensions.rs`), checkpoints/rollback, session search,
> trajectory viewer, undo/redo, agent eval, lifecycle events
> (compaction/advisor/self-repair → `UiEvent::AgentLifecycleEvent`), and the channel-config
> framework. The §5a "invisible agent internals" gaps and the §5c channel-config gap listed
> below as landed are done; the rest of the roadmap (shared-services unification, packaging,
> the remaining §5 breadth items) stays open. See the completion-status table in
> [`OVERHAUL_BACKLOG.md`](OVERHAUL_BACKLOG.md) and
> [`DEFERRED_FOLLOWUPS_PLAN.md`](DEFERRED_FOLLOWUPS_PLAN.md) for per-item state.

This is the maintainer-facing roadmap for taking ThinClaw Desktop from its current
**experimental** state to a coherent, parity-complete, production-ready 1.0. It is
grounded in the verified bridge/command surface, not documentation claims. Keep it
aligned with code; when this plan and the code disagree, the code wins and the plan
gets corrected.

---

## 0. Operating constraints (locked decisions)

| Decision | Choice | Consequence |
|---|---|---|
| North star / sequencing | **Full ThinClaw parity first** | Phase order: Parity → Stabilization/Upgrade → UX refinement |
| Two-system architecture | **Keep both, unify shell + shared services** | Direct AI Workbench and ThinClaw Agent Cockpit stay distinct modes, but stop duplicating infra |
| Platform scope | **macOS-first, then expand** | macOS is the 1.0 reference platform; Win/Linux stay green (compile + core smoke), parity later |
| Architectural disruption | **Breaking changes OK pre-1.0, with migrations** | Aggressive cleanup allowed (god-file splits, command-surface normalization, storage migrations) behind versioned migrations + tests |

---

## 1. Architecture of the plan

Three sequential **release phases** gated by shippable builds, plus six **continuous
workstreams** that are foundations rather than milestones.

```
PHASES (sequential gates)          WORKSTREAMS (continuous foundations)
─────────────────────────          ────────────────────────────────────
P1  Parity Closure        ┐        WS-1  Bridge & Command-Surface Normalization
P2  Stabilization & Upgrade├───────▶WS-2  Shared-Services Unification
P3  UX Refinement & 1.0    ┘        WS-3  Architecture Hygiene (god-file splits)
                                    WS-4  Test/QA & Observability
                                    WS-5  Security & Secrets
                                    WS-6  Packaging / Update / Platform
```

**Phase-gate rule:** every phase ends in a **releasable, notarized macOS build** that
passes the contract suite and a dated entry in
[`manual-smoke-checklist.md`](manual-smoke-checklist.md).

---

## 2. Objectives, principles, definition of done

### Objectives
1. Make the cockpit a *complete* control surface for the ThinClaw runtime (close every gap).
2. Stop duplication between the two AI systems by unifying shared services, keeping them distinct user-facing modes.
3. Upgrade the substrate (models, engines, Tauri APIs, deps) and refine into a coherent, polished 1.0.

### Engineering principles
- **Code-grounded contracts.** Each capability = registered command in `apps/desktop/backend/src/setup/commands.rs` + a `lib/thinclaw.ts` (or generated `bindings.ts`) wrapper + a generated binding, verified by the contract test at `setup/commands.rs` (`generated_bindings_cover_phase_two_desktop_surfaces`). **No UI ships against a mock.**
- **No new god-files** (repo `CLAUDE.md`): split `lib/thinclaw.ts`, `runtime_builder.rs`, `src/tauri_commands.rs`, and the oversized `ThinClaw*` panel components as they're touched. Preserve public paths with `pub use` re-exports.
- **Migrations mandatory** for any storage/settings/command rename — ship the migration + a test in the same PR.
- **Feature-flag risky work** (engine swaps, autonomy, new runtime commands) so each phase build is always shippable.
- **Parity = "wired AND honest."** A command that returns `unavailable` in local mode is acceptable only if the UI shows the reason; silent stubs are bugs.
- **Same-PR docs rule** (repo `CLAUDE.md`): behavior changes update the owning canonical doc and `FEATURE_PARITY.md` in the same PR.

### Definition of Done — 1.0 release gates
- Every cockpit panel maps to a real, non-stub command (or a clearly-labeled gated state).
- Zero `ui-stub-not-wired` rows. (The manual-compaction stub is already fixed — see §3.)
- Contract suite green; bindings regenerated from Rust; sanitizer tests pass.
- macOS notarized DMG built in CI; auto-update channel live; crash/error telemetry wired.
- Manual smoke checklist passes on a clean machine.

---

## 3. Baseline debts (starting point)

Verified from the bridge surface and per-feature audit. This table captures the
**starting-point** debts; items resolved by the first parity batch (see the §Progress
banner and [`OVERHAUL_BACKLOG.md`](OVERHAUL_BACKLOG.md)) have been struck from it. The
runtime is dual-mode: embedded `inner` vs `RemoteGatewayProxy` in `runtime_bridge.rs`.

| Class | Items |
|---|---|
| **Remote-only in local mode** | `learning_evaluate_outcomes`, `experiments_gpu_validate`/`gpu_launch_test`, `job_restart`/`job_prompt` |
| **Headless internals (no UI/telemetry)** | advisor auto-consult, pre-compaction flush, context-pressure, config watcher, observability metrics (self-repair, checkpoints/rollback, undo, and trajectory now have commands + UI) |
| **CLI-only (no command)** | SFT/DPO trajectory export (`src/cli/trajectory.rs`), tunnel, Claude-Code/Codex bridge job modes (the eval framework now has `thinclaw_experiments_run_eval`) |
| **Narrow coverage** | many channels still lack config UI (framework shipped, long tail pending); only cron routines creatable (not event); no `/personality`, profile-evolution, or external-memory UI |
| **Partial flows** | Repo-projects, Fleet, Cloud-Brain config, inline `MemoryEditor`, `subscribe_session` |
| **Duplication** | secrets, models, history, settings, theming exist twice (Workbench vs Cockpit) |
| **God-files** | `lib/thinclaw.ts`, `runtime_builder.rs`, `tauri_commands.rs`, and several `ThinClaw*` panel components |

---

## 4. Continuous workstreams

### WS-1 — Bridge & Command-Surface Normalization *(foundation; do first)*
The bridge is three artifacts that can drift: the `#[tauri::command]` surface, the
hand-written `lib/thinclaw.ts`, and generated `bindings.ts`.

- Adopt **one** calling convention: make generated `bindings.ts` (`commands.*`) the single source; reduce `lib/thinclaw.ts` to thin re-exports + types.
- Extend the existing domain split under `apps/desktop/backend/src/thinclaw/commands/`; retire/shrink the root `src/tauri_commands.rs` facade.
- Codify the dual-mode contract with a `RouteBehavior` enum per command: `LocalAndRemote | RemoteOnly(reason) | LocalOnly(reason)`. **Generate** `remote-gateway-route-matrix.md` from code and assert it in a test. Kills the silent-`unavailable` class.
- Generate one typed `UiEvent` discriminated union consumed by a single React event-bus hook (replace scattered `listen('thinclaw-event')`).
- **Deliverable:** a **bridge linter** CI test that fails if any command lacks {binding + wrapper + route-behavior + reason-on-gate}.

### WS-2 — Shared-Services Unification *(the "keep both, unify" decision)*

| Service | Today (duplicated) | Target (unified) |
|---|---|---|
| Secrets | `backend/src/secret_store.rs` (Workbench) + `KeychainSecretsAdapter` (Cockpit) | One keychain-backed secret service; one `SecretsTab` |
| Models / providers | `model_manager.rs`, `inference/router.rs` (Workbench) + ThinClaw provider catalog (Cockpit) | One model registry + one provider-key vault; `thinclaw_sync_local_llm` is the canonical bridge |
| History | `backend/src/history.rs` SQLite (Workbench) + ThinClaw session store (Cockpit) | Shared conversation store with a `surface` discriminator |
| Settings / config | `backend/src/config.rs` UserConfig (Workbench) + `thinclaw_config_*` (Cockpit) | One settings schema, two views |
| Theming | `theme-provider.tsx` + per-mode styles | One design-system token set (feeds WS / Phase 3) |

Approach: **strangler-fig with an adapter seam** — a `SharedServices` Rust module + a
React `services` context; migrate consumers one PR at a time; delete each duplicate
once both modes use the seam. Data-merging migrations modeled on `cloud/migration.rs`
+ `MigrationProgressDialog.tsx`.

### WS-3 — Architecture Hygiene (god-file decomposition)
Triggered on-touch, but schedule the worst offenders:
- `frontend/src/lib/thinclaw.ts` → `lib/api/{sessions,memory,routines,learning,experiments,mcp,…}.ts`.
- `backend/src/thinclaw/runtime_builder.rs` → provider/inference setup · sandbox/Docker orchestrator · background-task wiring · channel wiring · deps assembly.
- Oversized panel components: `ThinClawRepoProjects.tsx`, `ThinClawHooks.tsx`, `ThinClawAutomations.tsx`, `SubAgentPanel.tsx`, `ThinClawChannels.tsx`, `ThinClawSkills.tsx` → extract sub-panels + hooks.
- Retire/shrink `src/tauri_commands.rs`.

### WS-4 — Test/QA & Observability
- Contract tests per command (route-behavior matrix, bindings coverage, `Channel<T>`/reserved-arg sanitizer — extend the existing test).
- Executable **fixture acceptance** for local + remote modes (make the parity-checklist tiers runnable, not manual).
- Frontend: Vitest component tests (`frontend/src/tests/`) + Playwright/WebDriver E2E for the top 10 flows.
- Runtime telemetry: wire the core `Observer` (currently `NoopObserver`) to a desktop sink + crash reporter; surface context-pressure / self-repair / advisor as `UiEvent`s (also closes §5 parity gaps).

### WS-5 — Security & Secrets
- Single encrypted secret path (AES-256-GCM core store ↔ macOS Keychain); grant checks enforced (contract test covers denial).
- Surface (read-only, with reasons) core safety internals — sanitizer hits, sandbox network-allowlist, dangerous-tool tracker — in a "Security" panel.
- Wire master-key rotation + recovery-key into Settings (reuse the cloud-sync recovery-key UI).
- Threat-model the bridge (untrusted runtime output → React) and the remote-proxy auth.

### WS-6 — Packaging / Update / Platform (macOS-first)
- CI: notarized DMG, hardened runtime, stapling; Tauri updater signing key in CI secrets (currently release-operator manual).
- Auto-update channel wired to `UpdateChecker.tsx`.
- Sidecar bundling (Chromium, Piper, Whisper, sd.cpp/mflux, engines) validated by `npm run setup:all` on a clean machine; size budget + lazy download.
- Keep Windows/Linux in the build matrix (compile + core smoke); do not gate releases on them.

---

## 5. Phase 1 — Parity Closure (top priority)

Backlog grouped by parity domain. Sizes: S/M/L/XL. (Issue IDs in
[`OVERHAUL_BACKLOG.md`](OVERHAUL_BACKLOG.md).)

### 5a. Agent-loop internals → observable/controllable
| Gap | Approach | Key files | Size |
|---|---|---|---|
| ~~Manual compaction is a stub~~ **DONE** | Local path drives the core `ContextCompactor` (Summarize) over each thread and mutates live thread state | `rpc_extensions.rs` (`thinclaw_compact_session`) | M |
| No context-pressure signal | Add `ContextPressure` `UiEvent` + header indicator | `crates/thinclaw-agent/context_monitor`, `ui_types.rs` | M |
| ~~Self-repair invisible~~ **DONE** | `SelfRepairStarted`/`SelfRepairCompleted` → `AgentLifecycleEvent` | `src/agent/self_repair.rs`, `ui_types.rs` | M |
| ~~Checkpoints/`/rollback` no UI~~ **DONE** | `thinclaw_checkpoints_list`/`checkpoint_diff`/`checkpoint_restore` + Rollback panel | `rpc_checkpoints.rs` | L |
| ~~Undo manager no UI~~ **DONE** | `thinclaw_undo`/`thinclaw_redo` commands + control | `commands/sessions.rs` | S |
| ~~Advisor invisible~~ **DONE** | `AdvisorConsultationStarted` → `AgentLifecycleEvent` in Event Inspector | `src/agent/dispatcher/advisor.rs` | S |
| ~~Trajectory viewer~~ **DONE** | `thinclaw_trajectory_stats`/`thinclaw_trajectory_records` + viewer | `rpc_trajectory.rs` | M |

### 5b. Proactive / learning / experiments
| Gap | Approach | Key files | Size |
|---|---|---|---|
| Event-triggered routines uncreatable | Extend `routine_create` to wire `Trigger::SystemEvent`; UI trigger-type selector | `rpc_routines.rs:326`, `ThinClawAutomations.tsx` | M |
| `evaluate_outcomes` remote-only | Embedded evaluator OR honest gate w/ "needs gateway" CTA | `rpc_experiments_learning.rs:394` | M |
| GPU validate/launch remote-only | Local credential path OR honest gate | `rpc_experiments_learning.rs:625-661` | M |
| ~~Eval framework CLI-only~~ **DONE** | `thinclaw_experiments_list_envs` + `thinclaw_experiments_run_eval` exposed (runtime smoke-test remains a manual QA step) | `rpc_experiments_learning.rs` | L |
| SFT/DPO export CLI-only | `thinclaw_trajectory_export(format)` + export button | `src/cli/trajectory.rs` | M |
| Profile-evolution no panel | Dedicated viewer + force-run | `src/profile_evolution.rs` | S |

### 5c. Channels (breadth) — largest item
| Gap | Approach | Key files | Size |
|---|---|---|---|
| Many channels lack config UI (framework **DONE**, long tail pending) | **Schema-driven channel-config framework**: each native/WASM channel declares a config schema; UI renders generically (mirrors MCP/extension setup-schema). Framework + `thinclaw_channel_config_schema`/`_schemas`/`_submit` + Signal/Discord shipped; iMessage/Nostr and the long tail remain | `rpc_channel_config.rs`, `ThinClawChannelConfig` panel, channel manifests | **XL** |
| Pairing/web-login parity | Reuse pairing UI for all paired channels | `ThinClawPairing.tsx` | S |

### 5d. Identity / memory / personality
| Gap | Approach | Key files | Size |
|---|---|---|---|
| No `/personality` (`/vibe`) overlay | `thinclaw_personality_set/clear` + chat control | identity/soul crates | S |
| External-memory providers no UI | setup/status commands + panel (Mem0/Letta/Zep/…) | `external_memory_*` tools | M |
| Inline MemoryEditor partial | Finish wiring to `get_memory/save_memory` | `MemoryEditor.tsx` | S |

### 5e. Repo-projects / fleet / remote (finish partials)
| Gap | Approach | Key files | Size |
|---|---|---|---|
| Repo-projects partial | Complete enroll→plan→merge-gate; surface readiness gates | `ThinClawRepoProjects.tsx` (split first), `rpc_repo_projects.rs` | L |
| Fleet partial | Define fleet model (multi-agent A2A) → real status + broadcast | `thinclaw/fleet.rs`, `thinclaw/fleet/FleetCommandCenter.tsx` | L |
| Tunnel/Tailscale no UI | `thinclaw_tunnel_*` + Remote-access panel | `src/tunnel/` | M |
| `subscribe_session` stub | Real subscription semantics | `thinclaw/commands/sessions.rs` | S |

**Phase 1 exit gate:** parity matrix shows zero stub / zero silent-unavailable; every
panel wired or honestly gated; contract suite green.

---

## 6. Phase 2 — Stabilization & Upgrade

**Stabilize**
- Error taxonomy + user-facing error surfaces (no raw `String` errors in the UI).
- Bridge resilience: timeouts, retries, reconnect for `RemoteGatewayProxy`; dual-mode failover UX.
- Performance budgets: cold start; `UiEvent` stream throughput (30 variants); large-history virtualization; sidecar memory ceilings.
- Crash reporting + structured logs surfaced in the Doctor panel.

**Upgrade**
- **Models:** default to the latest Claude family (Opus/Sonnet/Haiku 4.x, Fable 5) in provider catalog + onboarding; verify pricing/caching via the `claude-api` reference.
- **Engines:** bump llama.cpp/MLX/vLLM/Ollama sidecars; validate GGUF/quant matrix; MLX-first on Apple Silicon.
- **Tauri/deps:** v2 capabilities audit (`backend/capabilities/default.json`); npm + Cargo refresh; advisory sweep — fix at source, no `deny`-ignore; no heavy deps for off-by-default features without sign-off.
- **RAG/inference:** reranker model refresh; embedding-dimension auto-detect hardening.

**Phase 2 exit gate:** clean-machine smoke passes; notarized auto-updating build; telemetry live.

---

## 7. Phase 3 — UX Refinement & 1.0

- **Design system:** one token set (color/spacing/type/motion) + shared component library; reconcile both modes' visual language behind `ModeNavigator`/`ChatLayout` so the Workbench↔Cockpit seam is intentional.
- **Mode seam:** make switching obvious (state, identity, model context); shared spotlight + command palette across both.
- **Onboarding overhaul:** single wizard configuring both systems (engine, keys+grants, identity bootstrap, first channel); de-dupe `OnboardingWizard` + setup wizard.
- **Accessibility:** keyboard nav, focus management, screen-reader labels, contrast — done once in the design system.
- **i18n:** wire core i18n into the frontend (currently core-only).
- **Polish:** empty/loading/error states, real-time progress (generalize the Imagine pattern), micro-interactions, density options.

**1.0 exit gate:** all DoD release gates (§2) met.

---

## 8. Cross-cutting strategy
- **Migrations:** versioned runner for settings schema, SQLite history merge, secret-store consolidation, command renames (keep deprecated aliases one minor version). Template: `cloud/migration.rs` + `MigrationProgressDialog.tsx`.
- **Feature flags:** typed registry (Rust + TS) gating each risky workstream so phase builds stay shippable.
- **Docs:** behavior changes update owning canonical doc same-PR; generate route-matrix from code (WS-1).
- **Telemetry & privacy:** opt-in, local-first; on-device or self-hosted only — privacy posture is a product selling point.

---

## 9. Testing & QA matrix
| Layer | Tooling | Gate |
|---|---|---|
| Command contracts | Rust tests + bridge linter | Every command: binding+wrapper+route-behavior |
| Dual-mode behavior | Fixture acceptance (local+remote) | Per route-matrix row |
| Frontend units | Vitest (`frontend/src/tests/`) | Components + lib |
| E2E flows | Playwright/WebDriver | Top 10 flows green |
| Clean-machine smoke | Manual checklist + dated report | Each phase gate |
| Security | Secret-grant denial, sanitizer, SSRF | CI |
| Packaging | Notarization/staple, updater signature | Release |

---

## 10. Risks & mitigations
| Risk | Mitigation |
|---|---|
| Parity-first churns surface before hardening | Lock the bridge contract (WS-1) before mass command additions; bridge linter prevents drift |
| Channel-config framework (XL) balloons | Ship framework + 4 channels first; long tail is incremental |
| Shared-services migration corrupts data | Versioned migrations + dry-run + recovery-key/backups; one service at a time |
| God-file splits regress | Split behind `pub use` re-exports; characterization tests before refactor |
| Remote-only features confuse users | `RouteBehavior` reason strings + explicit "needs gateway" CTAs |
| macOS-first leaves Win/Linux rotting | Keep them in CI build matrix (compile + core smoke) |
| Model/engine upgrades break flows | Feature-flag + provider-catalog versioning + fixture tests |

---

## 11. Kickoff sequence (first concrete moves)
1. WS-1 bridge linter + `RouteMode` enum — make the contract enforceable first. (`RouteMode`/`BridgeError`/`ROUTE_TABLE` in `bridge.rs` and the linter test have landed.)
2. Generate route-matrix & `UiEvent` union from code; start the `lib/thinclaw.ts` split.
3. ~~Fix the compaction stub~~ **done**; `thinclaw_compact_session` now drives the core `ContextCompactor`.
4. Channel-config schema framework spike — de-risk the largest parity item early. (Framework + Signal/Discord landed; long tail pending.)
5. Split `runtime_builder.rs` + `lib/thinclaw.ts` as their first consumers are touched.
6. Stand up fixture acceptance in CI so every subsequent PR is gated.

---

## 12. Related docs
- [`runtime-parity-checklist.md`](runtime-parity-checklist.md) — runtime surface status tiers
- [`bridge-contract.md`](bridge-contract.md) — Tauri command/event/binding contract
- [`remote-gateway-route-matrix.md`](remote-gateway-route-matrix.md) — local/remote behavior (to be code-generated, WS-1)
- [`runtime-boundaries.md`](runtime-boundaries.md) — two-system boundaries
- [`OVERHAUL_BACKLOG.md`](OVERHAUL_BACKLOG.md) — executable epic/issue backlog
- root [`FEATURE_PARITY.md`](../../../FEATURE_PARITY.md) — parity ledger
