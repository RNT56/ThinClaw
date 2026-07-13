# ThinClaw Desktop — Overhaul, Upgrade & Refinement Plan

> **Status:** Living roadmap (draft v1) · **Created:** 2026-06-27 · **Owner:** _TBD_
> **Scope:** End-to-end overhaul of ThinClaw Desktop (`apps/desktop/`).
> **Companion:** executable backlog in [`OVERHAUL_BACKLOG.md`](OVERHAUL_BACKLOG.md).
>
> **Progress (verified 2026-07-13):** the first parity-closure batch is **merged to `main`**:
> the dual-mode bridge contract (`RouteMode`/`BridgeError`/`ROUTE_TABLE` in
> `apps/desktop/backend/src/thinclaw/bridge.rs`), real per-thread compaction
> (`thinclaw_compact_session` in `rpc_extensions.rs`), checkpoints/rollback, session search,
> trajectory viewer, undo/redo, agent eval, lifecycle events
> (compaction/advisor/self-repair → `UiEvent::AgentLifecycleEvent`), and the channel-config
> framework. The §5a "invisible agent internals" gaps and the §5c channel-config gap listed
> below as landed are done. Shared services now cover secrets, models/providers,
> local conversation history (including the one-time legacy SQLite merge), one versioned
> settings schema with separate Workbench/Agent views, and one shared theme-token system. The rest
> of the roadmap (packaging,
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
- **No new god-files** (repo `CLAUDE.md`): split `lib/thinclaw.ts`, `runtime_builder.rs`, `src/desktop_api.rs`, and the oversized `ThinClaw*` panel components as they're touched. Preserve public paths with `pub use` re-exports.
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
| **Remote-only in local mode** | `learning_evaluate_outcomes` and GPU operations are honestly gated with gateway remediation; `job_restart`/`job_prompt` remain remote-only |
| **Headless internals (no UI/telemetry)** | advisor auto-consult, pre-compaction flush, context-pressure, config watcher, observability metrics (self-repair, checkpoints/rollback, undo, and trajectory now have commands + UI) |
| **CLI-only (no command)** | tunnel and Claude-Code/Codex bridge job modes (the eval framework and SFT/DPO trajectory export now have Desktop commands) |
| **Narrow coverage** | many channels still lack config UI (framework shipped, long tail pending); no `/personality`, profile-evolution, or external-memory UI |
| **Partial flows** | Fleet and Cloud-Brain config |
| **Duplication** | Shared-service duplication is closed: secrets, models/providers, local conversation history, settings storage/schema, and theming are unified |
| **God-files** | `lib/thinclaw.ts`, `runtime_builder.rs`, `desktop_api.rs`, and several `ThinClaw*` panel components (the root Tauri facade is retired) |

---

## 4. Continuous workstreams

### WS-1 — Bridge & Command-Surface Normalization *(foundation; do first)*
The bridge is three artifacts that can drift: the `#[tauri::command]` surface, the
hand-written `lib/thinclaw.ts`, and generated `bindings.ts`.

- Adopt **one** calling convention: make generated `bindings.ts` (`commands.*`) the single source; reduce `lib/thinclaw.ts` to thin re-exports + types.
- Extend the existing domain split under `apps/desktop/backend/src/thinclaw/commands/`; the root `src/tauri_commands.rs` facade is retired in favor of `src/desktop_api.rs` plus a deprecated alias.
- Codify the dual-mode contract with a `RouteBehavior` enum per command: `LocalAndRemote | RemoteOnly(reason) | LocalOnly(reason)`. **Generate** `remote-gateway-route-matrix.md` from code and assert it in a test. Kills the silent-`unavailable` class.
- Generate one typed `UiEvent` discriminated union consumed by a single React event-bus hook (replace scattered `listen('thinclaw-event')`).
- **Deliverable:** a **bridge linter** CI test that fails if any command lacks {binding + wrapper + route-behavior + reason-on-gate}.

### WS-2 — Shared-Services Unification *(the "keep both, unify" decision)*

| Service | Today (duplicated) | Target (unified) |
|---|---|---|
| Secrets | One app-wide `SecretStore`; its grant-aware `SecretsStore` implementation feeds the Cockpit while host methods feed Workbench | Unified; one keychain cache, live shared grants, one `SecretsTab` |
| Models / providers | `model_manager.rs`, `inference/router.rs` (Workbench) + ThinClaw provider catalog (Cockpit) | One model registry + one provider-key vault; `thinclaw_sync_local_llm` is the canonical bridge |
| History | In the default local profile, one `SharedHistoryStore` opens `thinclaw-runtime.db`; Direct commands use a SQLx adapter and the embedded agent receives the same runtime handle. In the PostgreSQL profile, Direct stays local and the agent uses PostgreSQL | Unified locally; `direct_workbench` and `agent_cockpit` rows are isolated by `surface`, with deterministic legacy merge |
| Settings / config | In the default local profile, one versioned `ConfigManager` envelope in the shared runtime database serves typed Workbench and key/value Agent views. In the PostgreSQL profile, Workbench remains recovery-file-backed and the agent uses its remote store | Unified locally; `user_config.json` is a recovery mirror after deterministic first attach |
| Theming | One versioned `thinclaw-ui-theme` preference record and one semantic token application path | Unified across Workbench, Cockpit, Spotlight, system mode changes, and all five palettes; feeds WS / Phase 3 |

Approach: **strangler-fig with an adapter seam** — a `SharedServices` Rust module + a
React `services` context; migrate consumers one PR at a time; delete each duplicate
once both modes use the seam. Data-merging migrations modeled on `cloud/migration.rs`
+ `MigrationProgressDialog.tsx`.

The adapter seam is implemented in `backend/src/shared_services.rs` and
`frontend/src/components/services-context.tsx`. It delegates to the existing
managed singletons and generated transport. TDO-011 through TDO-015 have migrated
secrets, models/providers, history, settings, and theming without changing product-mode
ownership.

### WS-3 — Architecture Hygiene (god-file decomposition)
Triggered on-touch, but schedule the worst offenders:
- ✅ `frontend/src/lib/thinclaw.ts` is now a stable 13-line barrel over focused
  `lib/api/{core,gateway,integrations,operations,repo-projects}.ts` modules; the
  existing component import path remains compatible.
- ✅ `backend/src/thinclaw/runtime_builder.rs` is now the 706-line assembly
  coordinator; environment/provider resolution, sandbox/Docker orchestration,
  background-task ownership, and event forwarding live in focused child modules.
- Oversized panel components: ✅ `ThinClawRepoProjects.tsx` now delegates to a
  focused data hook plus fixture, utility, and panel modules. ✅ `ThinClawHooks.tsx`
  now composes a tested template catalog, cards, custom editor, and data hook.
  ✅ `ThinClawAutomations.tsx` now composes a job card, create modal, tested
  schedule helpers, and runtime data hook. ✅ `SubAgentPanel.tsx`,
  `ThinClawChannels.tsx`, and `ThinClawSkills.tsx` now preserve their public
  entry points while delegating child-session rendering, channel/stream catalogs,
  skill cards, and channel/skill data ownership to focused modules; each public
  panel is below the god-file threshold.
- ✅ Retired `src/tauri_commands.rs`; reusable helpers now live in `src/desktop_api.rs` and registration stays in typed Desktop command modules.

### WS-4 — Test/QA & Observability
- Contract tests per command (route-behavior matrix, bindings coverage, `Channel<T>`/reserved-arg sanitizer — extend the existing test).
- ✅ Executable **fixture acceptance** for local + remote modes now runs representative bridge/gating checks and an authenticated loopback gateway across chat, jobs, autonomy, learning, experiments, MCP, skills, providers, costs, and cache surfaces in Desktop CI.
- Frontend: Vitest component tests (`frontend/src/tests/`) plus ✅ browser-mode
  WebDriver coverage for the top 10 user journeys: onboarding navigation and
  appearance, Chat, Dashboard, Channels, Automations, Jobs, Models, Secrets,
  and Appearance. The same suite separately checks deterministic Tauri IPC.
- ✅ Runtime telemetry: Desktop now decorates the operator-selected core
  `Observer` backend with an always-on, metadata-only typed event sink. Redacted
  observer errors and process panics are persisted locally as private `0600`
  reports with a 20-file retention cap; nothing is uploaded.
- ✅ Internal lifecycle telemetry: context compaction, advisor consultation,
  and self-repair start/completion preserve phase, label, and detail as typed
  `AgentLifecycleEvent`s in both embedded and remote modes. The standalone
  client consumes the same structured gateway SSE contract. The earlier
  context-pressure header indicator remains the separate TDO-101 scope.

### WS-5 — Security & Secrets
- ✅ Single encrypted secret path: the app-wide store serializes all local
  credentials into one authenticated `SecretsCrypto` AES-256-GCM envelope in
  macOS Keychain, with a separate random master-key item, transactional legacy
  migration, fail-closed reads, a rotation seam, and live grant enforcement on
  every runtime operation. Contract tests cover ciphertext, tamper rejection,
  key-version rotation, canonical aliases, and grant denial.
- ✅ Read-only Security panel now reports metadata-only sanitizer/policy decisions,
  the effective local sandbox and network allowlist, and live tool descriptor /
  approval metadata with human-readable reasons. It explicitly reports local,
  stopped, and remote evidence availability. The never-wired dangerous-tool
  tracker remains retired and is not presented as an enforcement control.
- ✅ Secrets Settings reuses the recovery-key panel for explicit, one-minute
  key reveal, checksummed recovery import, and exact-confirmation master-key
  rotation. Rotation persists the replacement core key, re-encrypts and
  verifies the complete Desktop envelope, and rolls both stores back on
  failure. The panel honestly gates persistence to macOS Keychain.
- ✅ The bridge threat model now treats runtime/SSE/Markdown/error content as
  untrusted, bounds transport and rendering, validates session IDs and external
  links, requires authenticated remote health, blocks public plaintext bearer
  transport, redacts profile credentials from disk/IPC/logs, and hardens SSH
  deployment input, host-key, stdin-secret, and gateway-port behavior.

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
| ~~Event-triggered routines uncreatable~~ **DONE** | Local and remote `routine_create` wire `Trigger::SystemEvent`; the creation modal has an accessible trigger-type selector | `rpc_routines.rs`, `automations/CreateJobModal.tsx` | M |
| ~~`evaluate_outcomes` failed opaquely in local mode~~ **DONE** | Typed remote-only gate explains that a gateway is required | `rpc_experiments_learning.rs:394` | M |
| ~~GPU validate/launch failed opaquely in local mode~~ **DONE** | Typed remote-only gates explain the gateway credential boundary | `rpc_experiments_learning.rs:631-675` | M |
| Eval framework partially exposed | Commands are wired; add the Benchmarks panel and runtime smoke-test | `rpc_experiments_learning.rs`, frontend | L |
| ~~SFT/DPO export CLI-only~~ **DONE** | CLI and Desktop share the canonical validated exporter; Desktop adds a bounded local command and explicit SFT/DPO download controls | `src/cli/trajectory.rs`, `rpc_trajectory.rs`, `ThinClawTrajectory.tsx` | M |
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
| ~~Inline MemoryEditor partial~~ **DONE** | Reads and saves the canonical memory document through registered commands | `MemoryEditor.tsx`, `commands/sessions.rs` | S |

### 5e. Repo-projects / fleet / remote (finish partials)
| Gap | Approach | Key files | Size |
|---|---|---|---|
| ~~Repo-projects partial~~ **DONE** | Enroll→plan→merge-gate flow and readiness gates are wired end to end | `ThinClawRepoProjects.tsx`, `rpc_repo_projects.rs`, `src/repo_projects` | L |
| Fleet partial | Define fleet model (multi-agent A2A) → real status + broadcast | `thinclaw/fleet.rs`, `thinclaw/fleet/FleetCommandCenter.tsx` | L |
| Tunnel/Tailscale no UI | `thinclaw_tunnel_*` + Remote-access panel | `src/tunnel/` | M |
| ~~`subscribe_session` stub~~ **DONE** | Activates local/remote live-event routing with real subscription semantics | `thinclaw/commands/sessions.rs`, `runtime_bridge.rs` | S |

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
| E2E flows | WebdriverIO browser mode | ✅ Top 10 flows + IPC contract green |
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
