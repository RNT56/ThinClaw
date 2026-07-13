# ThinClaw Desktop — Overhaul Backlog (tracker-ready)

> **Status:** draft v1 · **Created:** 2026-06-27 · Companion to [`OVERHAUL_PLAN.md`](OVERHAUL_PLAN.md).

## Completion status (verified 2026-07-13)

Every row below is verified in the current implementation branch. ✅ = implemented
and locally verified; GitHub merge state remains authoritative until each PR lands.

| TDO | Item | State | Verified in code |
|---|---|---|---|
| TDO-001 | `RouteMode` enum + typed `BridgeError` + `gated()` helper | ✅ | `bridge.rs` (`RouteMode`:22, `BridgeError`:35) |
| TDO-002 | Bridge linter (`ROUTE_TABLE` + `all_gated_commands_are_classified`) | ✅ | `bridge.rs` (`ROUTE_TABLE`:116 + linter tests) |
| TDO-003 | Generated per-command remote route matrix + drift assertion | ✅ | `bridge.rs` (`render_route_matrix_section` + `committed_route_matrix_matches_the_registry`) |
| TDO-004 | Generated bindings and bindings-derived clients are the sole production frontend command transport | ✅ | `bindings.ts`, `command-client.ts`, `thinclaw.ts`, `production_frontend_has_one_command_calling_convention` |
| TDO-005 | Generated `UiEvent` union + one native event-bus listener with typed React fan-out | ✅ | `ui_types.rs`, `use-thinclaw-stream.ts`, `event-bus-migration.test.ts` |
| TDO-006 | Retired root `tauri_commands.rs`; service helpers live in `desktop_api` behind a deprecated compatibility alias | ✅ | `src/desktop_api.rs`, `src/lib.rs`, typed desktop command modules |
| TDO-010 | Typed `SharedServices` Tauri state + injectable React services context | ✅ | `backend/src/shared_services.rs`, `frontend/src/components/services-context.tsx`, `App.tsx` |
| TDO-011 | One keychain-backed `SecretStore` + live shared grants + functional custom-secret updates | ✅ | `secret_store.rs`, `secrets_adapter.rs`, `keys.rs`, `SecretsTab.tsx` |
| TDO-012 | One shared model/provider registry, discovery cache, local inventory, and key-readiness path | ✅ | `inference/model_discovery/mod.rs`, `inference/router.rs`, `model_manager.rs` |
| TDO-013 | Shared conversation store, surface isolation, and deterministic one-time legacy merge | ✅ | `history.rs`, `runtime_builder.rs`, `V30__conversation_surfaces.sql` |
| TDO-014 | One versioned canonical settings schema with typed Workbench and Agent views | ✅ | `config.rs`, `rpc_config.rs`, `ThinClawSystemControl.tsx` |
| TDO-015 | One versioned theme preference record and semantic token contract for both product surfaces | ✅ | `theme-provider.tsx`, `index.css`, `ThinClawView.tsx` |
| TDO-100 | Real per-thread compaction (`thinclaw_compact_session`) | ✅ | `rpc_extensions.rs` (drives core `ContextCompactor`) |
| TDO-102 | Self-repair lifecycle events → `UiEvent::AgentLifecycleEvent` + Event Inspector row | ✅ | `event_mapping.rs`, `agent_loop`, `ThinClawEventInspector.tsx` |
| TDO-103 | Checkpoints/rollback: `list`/`diff`/`restore` commands + Rollback panel | ✅ | `rpc_checkpoints.rs`:40/52/65 |
| TDO-104 | Undo/redo: `thinclaw_undo`/`_redo` commands + cockpit toolbar buttons | ✅ | `commands/sessions.rs`:105/147 |
| TDO-105 | Advisor consultation → `UiEvent::AgentLifecycleEvent` + Event Inspector row | ✅ | `event_mapping.rs`, `tool_execution.rs`, `ThinClawEventInspector.tsx` |
| TDO-106/114 | Trajectory viewer plus bounded SFT/DPO export command and explicit download controls | ✅ | `rpc_trajectory.rs`, `ThinClawTrajectory.tsx` |
| TDO-111/112 | Outcome evaluation and GPU operations return typed, actionable gateway gates in local mode | ✅ | `rpc_experiments_learning.rs`:394/631/654 |
| TDO-113 | Agent-eval commands + interactive Benchmarks panel; real-engine runtime smoke remains manual | ◐ | `rpc_experiments_learning.rs`, `experiments/BenchmarkPanel.tsx` |
| TDO-120 | Channel-config framework: native/WASM schemas, encrypted credential routing, validated local/remote submit commands, and Channel Config panel | ✅ | `rpc_channel_config.rs`, `handlers/channels.rs`, `wasm/wrapper/mod.rs` |
| TDO-121 | Signal, Discord, iMessage, and Nostr schemas preserve current non-secret values and resolve persisted settings on restart | ✅ | first-party channel adapters, `channel_config.rs` |
| TDO-122 | Long-tail schemas cover manifest-backed WASM channels, Apple Mail, BlueBubbles, and honest host-managed lifecycle adapters | ✅ | WASM loader/wrapper, native channel adapters |
| TDO-132 | Inline Memory Editor reads and saves the canonical memory document | ✅ | `MemoryEditor.tsx`, `commands/sessions.rs`:732/750 |
| TDO-140 | Repo-projects enroll→plan→merge-gate flow + readiness surface | ✅ | `rpc_repo_projects.rs`, `ThinClawRepoProjects.tsx`, `src/repo_projects` |
| TDO-143 | Local/remote session subscription activates live event routing | ✅ | `commands/sessions.rs`:675, `runtime_bridge.rs`:648 |
| Supplemental | Session search command + Session Search panel | ✅ | `rpc_session_search.rs`, `ThinClawSessionSearch.tsx` |

**Deferred / cross-lane (still open):** live-reload for startup-only native channel fields
remains future work; the eval runtime smoke-test needs a running engine. See
[`DEFERRED_FOLLOWUPS_PLAN.md`](DEFERRED_FOLLOWUPS_PLAN.md).

---

Drop these into your tracker as **Epics** (workstreams/phases) and **Issues**. IDs are
stable (`TDO-###`). Sizes: **S** ≈ ≤1d, **M** ≈ 2–4d, **L** ≈ 1–2wk, **XL** ≈ 3wk+.
"Phase" = which release gate it blocks (P1 parity / P2 stabilize / P3 UX / ∞ continuous).
Every issue inherits the global acceptance criteria below.

**Global acceptance criteria (apply to all):**
- New/changed command → registered in `setup/commands.rs` + wrapper + regenerated `bindings.ts`; contract test green.
- Behavior change → owning canonical doc + `FEATURE_PARITY.md` updated in the same PR.
- Storage/settings/command rename → versioned migration + migration test.
- `cargo fmt`, `cargo clippy --all --all-features`, `cargo test`, and frontend `vitest` pass.

---

## Epics

| Epic ID | Title | Type | Notes |
|---|---|---|---|
| TDO-EP1 | Bridge & Command-Surface Normalization | Workstream (WS-1) | Foundation — unblocks everything |
| TDO-EP2 | Shared-Services Unification | Workstream (WS-2) | Keep-both-but-unify |
| TDO-EP3 | Architecture Hygiene (god-file splits) | Workstream (WS-3) | On-touch + scheduled |
| TDO-EP4 | Test/QA & Observability | Workstream (WS-4) | CI gates |
| TDO-EP5 | Security & Secrets | Workstream (WS-5) | |
| TDO-EP6 | Packaging / Update / Platform | Workstream (WS-6) | macOS-first |
| TDO-EP7 | Phase 1 — Parity Closure | Milestone | Top priority |
| TDO-EP8 | Phase 2 — Stabilization & Upgrade | Milestone | |
| TDO-EP9 | Phase 3 — UX Refinement & 1.0 | Milestone | |

**Milestone exit gates:** P1 = zero stub / zero silent-unavailable, all panels wired or
honestly gated. P2 = clean-machine smoke + notarized auto-updating build + telemetry.
P3 = all 1.0 DoD gates.

---

## TDO-EP1 · Bridge & Command-Surface Normalization (do first)

| ID | Title | Size | Phase | Depends | Files |
|---|---|---|---|---|---|
| TDO-001 | Introduce `RouteBehavior` enum (`LocalAndRemote`/`RemoteOnly(reason)`/`LocalOnly(reason)`) on every command | L | ∞ | — | `thinclaw/commands/*`, `runtime_bridge.rs` |
| TDO-002 | Bridge linter CI test: fail if a command lacks {binding, wrapper, route-behavior, reason-on-gate} | M | ∞ | TDO-001 | extend `setup/commands.rs` test |
| TDO-003 ✅ | Generate `remote-gateway-route-matrix.md` from code; assert in test | M | ∞ | TDO-001 | `remote-gateway-route-matrix.md` |
| TDO-004 ✅ | Single calling convention: make generated `bindings.ts` (`commands.*`) the source of truth; reduce `lib/thinclaw.ts` to re-exports + types | L | ∞ | TDO-002 | `lib/thinclaw.ts`, `lib/bindings.ts` |
| TDO-005 ✅ | Typed `UiEvent` discriminated union + one React event-bus hook; replace scattered `listen('thinclaw-event')` | M | ∞ | TDO-004 | `ui_types.rs`, `hooks/use-thinclaw-stream.ts` |
| TDO-006 ✅ | Retire/shrink root `src/tauri_commands.rs` facade | M | ∞ | TDO-004 | `src/desktop_api.rs`, `src/lib.rs` |

**TDO-001 acceptance:** each command declares its mode behavior; local-mode `unavailable`
responses carry a machine-readable `reason`; UI can render the reason. No command returns
a bare error string for a gated state.

---

## TDO-EP2 · Shared-Services Unification

| ID | Title | Size | Phase | Depends | Files |
|---|---|---|---|---|---|
| TDO-010 ✅ | `SharedServices` Rust seam + React `services` context (adapter, no behavior change) | L | ∞ | TDO-001 | `backend/src/shared_services.rs`, `components/services-context.tsx`, `App.tsx` |
| TDO-011 ✅ | Unify secrets: one keychain-backed service feeding Workbench + Cockpit; single `SecretsTab` | L | ∞ | TDO-010 | `secret_store.rs`, `secrets_adapter.rs`, `SecretsTab.tsx` |
| TDO-012 ✅ | Unify models/providers: one registry + provider-key vault; `sync_local_llm` canonical bridge | L | ∞ | TDO-010 | `model_manager.rs`, `inference/router.rs`, provider catalog |
| TDO-013 ✅ | Unify history: shared conversation store with `surface` discriminator (+ SQLite merge migration) | L | ∞ | TDO-010 | `history.rs`, ThinClaw session store |
| TDO-014 ✅ | Unify settings: one schema, two views (Workbench `config.rs` + `thinclaw_config_*`) | M | ∞ | TDO-010 | `config.rs`, `rpc_config.rs` |
| TDO-015 ✅ | Unify theming tokens (feeds design system) | M | P3 | TDO-010 | `theme-provider.tsx` |

**TDO-011 acceptance:** one code path stores/reads secrets; grant-denial contract test
still green; legacy aliases migrated; duplicate store deleted.

**TDO-013 acceptance:** Direct and embedded-agent conversations share one local
database handle; every Direct operation is surface-scoped; the legacy SQLite merge is
deterministic, idempotent, and preserves Direct metadata and attachments.

**TDO-014 acceptance:** `ConfigManager` is the sole local settings service; the
database-backed Workbench config wins after a one-time JSON merge; Agent settings share
the same table without exposing reserved Desktop rows; both UI views save a typed envelope.

**TDO-015 acceptance:** one validated, versioned preference record replaces four legacy
localStorage keys; one token application path serves both product surfaces and windows;
semantic surface/content tokens plus compatibility aliases make Cockpit panels honor every
app palette without changing status-color meaning.

---

## TDO-EP3 · Architecture Hygiene (god-file splits)

| ID | Title | Size | Phase | Depends | Files |
|---|---|---|---|---|---|
| TDO-020 ✅ | Split `lib/thinclaw.ts` → `lib/api/{domain}.ts` | L | ∞ | TDO-004 | `lib/thinclaw.ts`, `lib/api/*` |
| TDO-021 ✅ | Split `runtime_builder.rs` → environment/sandbox/bg-tasks/event-forwarder modules | L | ∞ | — | `thinclaw/runtime_builder.rs`, `thinclaw/runtime_builder/*` |
| TDO-022 ✅ | Split `ThinClawRepoProjects.tsx` into fixtures, sub-panels, hook, and utilities | M | P1 | — | `ThinClawRepoProjects.tsx`, `thinclaw/repo-projects/*` |
| TDO-023 ✅ | Split `ThinClawHooks.tsx` into catalog, cards, modal, and data hook | M | ∞ | — | `ThinClawHooks.tsx`, `thinclaw/hooks/*` |
| TDO-024 ✅ | Split `ThinClawAutomations.tsx` into job card, create modal, schedule helpers, and data hook | M | P1 | — | `ThinClawAutomations.tsx`, `thinclaw/automations/*` |
| TDO-025 ✅ | Split `SubAgentPanel.tsx`, `ThinClawChannels.tsx`, `ThinClawSkills.tsx` into focused rows/cards, catalogs, and data hooks | M | ∞ | — | components |

**Split acceptance:** public import paths preserved via `pub use` / barrel re-exports;
characterization test added before the split; no behavior change.

---

## TDO-EP4 · Test/QA & Observability

| ID | Title | Size | Phase | Depends | Files |
|---|---|---|---|---|---|
| TDO-030 ✅ | Executable fixture acceptance for local + remote modes (make parity tiers runnable) | L | ∞ | TDO-002 | `scripts/ci/desktop-fixture-acceptance.sh`, bridge/proxy fixtures |
| TDO-031 ✅ | Playwright/WebDriver E2E for top 10 flows | L | P2 | — | `e2e/`, `wdio.browser.conf.ts` |
| TDO-032 ✅ | Wire core `Observer` → desktop sink + crash reporter | M | P2 | — | `thinclaw/desktop_observer.rs`, typed event bus |
| TDO-033 ✅ | Surface internal events as `UiEvent`s (context compaction, self-repair, advisor) in local and remote modes | M | P1 | TDO-005 | `ui_types.rs`, `event_mapping.rs`, gateway SSE |
| TDO-034 ✅ | Expand contract/sanitizer tests (`Channel<T>`, reserved args, every command) | M | ∞ | TDO-002 | `setup/commands.rs` test |

---

## TDO-EP5 · Security & Secrets

| ID | Title | Size | Phase | Depends | Files |
|---|---|---|---|---|---|
| TDO-040 ✅ | Single encrypted secret path (AES-256-GCM ↔ Keychain), grant enforcement | M | ∞ | TDO-011 | `config/keychain.rs`, `secret_store.rs`, `secrets_adapter.rs` |
| TDO-041 ✅ | "Security" panel: surface sanitizer decisions, effective sandbox allowlist, and live tool approval metadata (read-only + reasons) | M | P2 | TDO-033 | `SecurityPosturePanel.tsx`, `rpc_security.rs`, `security-posture.md` |
| TDO-042 ✅ | Master-key rotation + recovery-key in Settings (reuse cloud recovery-key UI) | M | P2 | TDO-040 | `rpc_secret_recovery.rs`, `RecoveryKeyPanel.tsx` |
| TDO-043 ✅ | Threat-model: untrusted runtime output → React; remote-proxy auth | S | P2 | — | `threat-model.md`, bounded rendering/transport, authenticated health, redacted profile/deploy credentials |

---

## TDO-EP6 · Packaging / Update / Platform (macOS-first)

| ID | Title | Size | Phase | Depends | Files |
|---|---|---|---|---|---|
| TDO-050 | CI notarized DMG (hardened runtime + staple); updater signing key in CI | L | P2 | — | CI, `tauri.conf.json` |
| TDO-051 | Auto-update channel wired to `UpdateChecker.tsx` | M | P2 | TDO-050 | `UpdateChecker.tsx` |
| TDO-052 | Sidecar bundling + size budget + lazy download; clean-machine `setup:all` validation | L | P2 | — | `scripts/`, `desktop-sidecars/` |
| TDO-053 ✅ | Keep Win/Linux in CI build matrix (compile + core smoke) | M | ∞ | — | `.github/workflows/ci.yml` |

---

## TDO-EP7 · Phase 1 — Parity Closure

### Agent-loop internals
| ID | Title | Size | Depends | Files |
|---|---|---|---|---|
| TDO-100 ✅ | **Compaction** → drives the real core `ContextCompactor` (Summarize) | M | TDO-001 | `rpc_extensions.rs` (`thinclaw_compact_session`) |
| TDO-101 ✅ | Context-pressure `UiEvent` + header indicator | M | TDO-005 | `context_monitor`, `ui_types.rs`, `ContextPressureBadge.tsx` |
| TDO-102 ✅ | Self-repair status event + panel row | M | TDO-033 | `self_repair.rs` |
| TDO-103 ✅ | Checkpoints UI: `thinclaw_checkpoints_list`/`checkpoint_diff`/`checkpoint_restore` + Rollback panel | L | TDO-001 | `rpc_checkpoints.rs` |
| TDO-104 ✅ | Undo/redo commands + control | S | TDO-001 | `commands/sessions.rs` |
| TDO-105 ✅ | Advisor consultation `UiEvent` in Event Inspector | S | TDO-005 | `dispatcher/advisor.rs` |
| TDO-106 ✅ | Trajectory viewer: `thinclaw_trajectory_stats`/`thinclaw_trajectory_records` + UI | M | TDO-001 | `rpc_trajectory.rs` |

### Proactive / learning / experiments
| ID | Title | Size | Depends | Files |
|---|---|---|---|---|
| TDO-110 ✅ | Event-triggered routine creation (`Trigger::SystemEvent`) + UI | M | TDO-024 | `rpc_routines.rs`, `automations/CreateJobModal.tsx` |
| TDO-111 ✅ | `evaluate_outcomes`: typed remote-only gate with gateway remediation | M | TDO-001 | `rpc_experiments_learning.rs:394` |
| TDO-112 ✅ | GPU validate/launch: typed remote-only gates with gateway remediation | M | TDO-001 | `rpc_experiments_learning.rs:631-675` |
| TDO-113 | Eval framework commands and Benchmarks panel are wired; real-engine runtime smoke-test remains | L | TDO-001 | `rpc_experiments_learning.rs`, `experiments/BenchmarkPanel.tsx` |
| TDO-114 ✅ | `thinclaw_trajectory_export(format)` (SFT/DPO) + export button | M | TDO-106 | `src/cli/trajectory.rs`, `rpc_trajectory.rs`, `ThinClawTrajectory.tsx` |
| TDO-115 ✅ | Profile-evolution viewer + force-run | S | TDO-001 | `rpc_profile_evolution.rs`, `learning/ProfileEvolutionPanel.tsx` |

### Channels (largest item)
| ID | Title | Size | Depends | Files |
|---|---|---|---|---|
| TDO-120 ✅ | **Channel-config schema framework** (`thinclaw_channel_config_schema`/`_schemas`/`_submit` commands + generic renderer in the `ThinClawChannelConfig` panel) | XL | TDO-001 | `rpc_channel_config.rs`, channel manifests |
| TDO-121 ✅ | Signal, Discord, iMessage, and Nostr expose validated non-secret config schemas with current values; persisted settings resolve on restart | L | TDO-120 | channel adapters |
| TDO-122 ✅ | WASM manifests declare encrypted credential forms for Matrix, Teams, LINE, SMS, WeCom, Feishu, Twitch, and peers; Apple Mail/BlueBubbles expose current non-secret values; native Matrix/voice/APNs/browser-push show explicit host-managed instructions | L | TDO-120 | channel adapters |
| TDO-123 ✅ | Pairing parity covers every adapter that enforces DM codes (Telegram, Slack, Discord, WhatsApp, Signal); unsupported web-login stubs removed | S | TDO-120 | `ThinClawPairing.tsx`, `pairing/catalog.ts` |

### Identity / memory / personality
| ID | Title | Size | Depends | Files |
|---|---|---|---|---|
| TDO-130 ✅ | `/personality` (`/vibe`) overlay command + accessible session control in Agent Chat | S | TDO-001 | `commands.rs`, `chat/PersonalityControl.tsx` |
| TDO-131 ✅ | Secret-safe external-memory setup/disable commands + provider health/configuration panel | M | TDO-001 | `rpc_experiments_learning.rs`, `learning/ExternalMemoryPanel.tsx` |
| TDO-132 ✅ | Inline `MemoryEditor` wired to `get_memory`/`save_memory` | S | — | `MemoryEditor.tsx`, `commands/sessions.rs` |

### Repo-projects / fleet / remote
| ID | Title | Size | Depends | Files |
|---|---|---|---|---|
| TDO-140 ✅ | Repo-projects enroll→plan→merge-gate + readiness gates | L | TDO-022 | `rpc_repo_projects.rs`, component |
| TDO-141 ✅ | Authenticated multi-agent fleet status, real remote task routing, and one-delivery-per-node broadcast receipts | L | TDO-001 | `fleet.rs`, `rpc_orchestration.rs`, `fleet/FleetCommandCenter.tsx` |
| TDO-142 ✅ | Authenticated loopback gateway + typed Tailscale Serve/Funnel commands + Remote Access panel | M | TDO-001 | `runtime_builder.rs`, `remote_access.rs`, `ThinClawRemoteAccess.tsx`, `src/tunnel/` |
| TDO-143 ✅ | Real local/remote session subscription semantics | S | TDO-001 | `sessions.rs`, `runtime_bridge.rs` |

---

## TDO-EP8 · Phase 2 — Stabilization & Upgrade

| ID | Title | Size | Files |
|---|---|---|---|
| TDO-200 | Error taxonomy + user-facing error surfaces (no raw `String` errors) — ✅ complete | M | bridge + UI |
| TDO-201 ✅ | Bridge resilience: bounded idempotent retries, typed transport/HTTP failures, shutdown-safe SSE reconnect, and visible failover/recovery UX | M | `remote_proxy/`, `ThinClawChatView.tsx` |
| TDO-202 ✅ | Observable performance budgets: backend/renderer readiness, frame-batched event stream, two-surface history virtualization, frontend chunks, and app/sidecar memory ceiling status | L | `performance-budgets.md`, frontend + backend |
| TDO-203 ✅ | Current Claude family: Fable 5, Opus 4.8 default, Sonnet 5 balanced slot, Haiku 4.5 fast slot across catalog, discovery fallback, onboarding, Bedrock, and cost metadata | M | provider catalog, onboarding |
| TDO-204 ✅ | Reproducible engine matrix: verified llama.cpp/uv assets, exact MLX/vLLM pins with versioned upgrades, Ollama version reporting, and fail-closed bounded GGUF/quant validation | L | `engine-compatibility.md`, `engine/*`, sidecars |
| TDO-205 ✅ | Window-isolated Tauri v2 capabilities + npm/Cargo refresh + enabled-graph advisory sweep with no RustSec ignores | M | `security-and-dependencies.md`, `capabilities/*.json`, manifests |
| TDO-206 ✅ | RAG/inference upgrades: verified reranker artifacts, live embedding-dim authority, current provider defaults, and safe index migration | M | `rag-inference-compatibility.md`, `rag.rs`, `reranker.rs`, `hf_hub.rs` |

---

## TDO-EP9 · Phase 3 — UX Refinement & 1.0

| ID | Title | Size | Files |
|---|---|---|---|
| TDO-300 ✅ | Design system: stable layout/motion/density tokens plus typed Button, Surface, Progress, and AsyncState primitives shared by both surfaces | L | `lib/app-themes.ts`, `components/ui/`, `index.css` |
| TDO-301 ✅ | Mode seam: persistent labeled Workbench/Cockpit/Imagine switch, runtime status, direct keyboard shortcuts, and searchable shared command palette | M | `ModeNavigator.tsx`, `CommandPalette.tsx`, `ChatLayout.tsx` |
| TDO-302 | Onboarding overhaul: single wizard for both systems | L | `OnboardingWizard.tsx`, setup wizard |
| TDO-303 | Accessibility pass (keyboard, focus, SR labels, contrast) | L | design system |
| TDO-304 | Frontend i18n wiring (core i18n → UI) | M | `i18n`, frontend |
| TDO-305 | Polish: empty/loading/error states, progress, micro-interactions, density | M | components |

---

## Suggested first sprint (de-risk + prove the loop)

This sprint has largely executed: `TDO-001` → `TDO-002` → `TDO-100` (compaction) →
`TDO-120` (channel-config framework) all landed. Remaining from the original ordering:
`TDO-021`/`TDO-020` (split god-files on touch) → `TDO-030` (CI fixture acceptance gate).
