# ThinClaw Desktop — Overhaul Backlog (tracker-ready)

> **Status:** draft v1 · **Created:** 2026-06-27 · Companion to [`OVERHAUL_PLAN.md`](OVERHAUL_PLAN.md).

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
| TDO-003 | Generate `remote-gateway-route-matrix.md` from code; assert in test | M | ∞ | TDO-001 | `remote-gateway-route-matrix.md` |
| TDO-004 | Single calling convention: make generated `bindings.ts` (`commands.*`) the source of truth; reduce `lib/thinclaw.ts` to re-exports + types | L | ∞ | TDO-002 | `lib/thinclaw.ts`, `lib/bindings.ts` |
| TDO-005 | Typed `UiEvent` discriminated union + one React event-bus hook; replace scattered `listen('thinclaw-event')` | M | ∞ | TDO-004 | `ui_types.rs`, `hooks/use-thinclaw-stream.ts` |
| TDO-006 | Retire/shrink root `src/tauri_commands.rs` facade | M | ∞ | TDO-004 | `src/tauri_commands.rs` |

**TDO-001 acceptance:** each command declares its mode behavior; local-mode `unavailable`
responses carry a machine-readable `reason`; UI can render the reason. No command returns
a bare error string for a gated state.

---

## TDO-EP2 · Shared-Services Unification

| ID | Title | Size | Phase | Depends | Files |
|---|---|---|---|---|---|
| TDO-010 | `SharedServices` Rust seam + React `services` context (adapter, no behavior change) | L | ∞ | TDO-001 | new module, `App.tsx` |
| TDO-011 | Unify secrets: one keychain-backed service feeding Workbench + Cockpit; single `SecretsTab` | L | ∞ | TDO-010 | `secret_store.rs`, `KeychainSecretsAdapter`, `SecretsTab.tsx` |
| TDO-012 | Unify models/providers: one registry + provider-key vault; `sync_local_llm` canonical bridge | L | ∞ | TDO-010 | `model_manager.rs`, `inference/router.rs`, provider catalog |
| TDO-013 | Unify history: shared conversation store with `surface` discriminator (+ SQLite merge migration) | L | ∞ | TDO-010 | `history.rs`, ThinClaw session store |
| TDO-014 | Unify settings: one schema, two views (Workbench `config.rs` + `thinclaw_config_*`) | M | ∞ | TDO-010 | `config.rs`, `rpc_config.rs` |
| TDO-015 | Unify theming tokens (feeds design system) | M | P3 | TDO-010 | `theme-provider.tsx` |

**TDO-011 acceptance:** one code path stores/reads secrets; grant-denial contract test
still green; legacy aliases migrated; duplicate store deleted.

---

## TDO-EP3 · Architecture Hygiene (god-file splits)

| ID | Title | Size | Phase | Depends | Files |
|---|---|---|---|---|---|
| TDO-020 | Split `lib/thinclaw.ts` (2,534) → `lib/api/{domain}.ts` | L | ∞ | TDO-004 | `lib/thinclaw.ts` |
| TDO-021 | Split `runtime_builder.rs` (1,441) → inference/sandbox/bg-tasks/channels/deps modules | L | ∞ | — | `thinclaw/runtime_builder.rs` |
| TDO-022 | Split `ThinClawRepoProjects.tsx` (992) into sub-panels + hooks | M | P1 | — | component |
| TDO-023 | Split `ThinClawHooks.tsx` (962) | M | ∞ | — | component |
| TDO-024 | Split `ThinClawAutomations.tsx` (884) | M | P1 | — | component |
| TDO-025 | Split `SubAgentPanel.tsx` (792), `ThinClawChannels.tsx` (719), `ThinClawSkills.tsx` (670) | M | ∞ | — | components |

**Split acceptance:** public import paths preserved via `pub use` / barrel re-exports;
characterization test added before the split; no behavior change.

---

## TDO-EP4 · Test/QA & Observability

| ID | Title | Size | Phase | Depends | Files |
|---|---|---|---|---|---|
| TDO-030 | Executable fixture acceptance for local + remote modes (make parity tiers runnable) | L | ∞ | TDO-002 | new test harness |
| TDO-031 | Playwright/WebDriver E2E for top 10 flows | L | P2 | — | `frontend/` |
| TDO-032 | Wire core `Observer` → desktop sink + crash reporter | M | P2 | — | `src/agent` observer, backend |
| TDO-033 | Surface internal events as `UiEvent`s (context-pressure, self-repair, advisor) | M | P1 | TDO-005 | `ui_types.rs` |
| TDO-034 | Expand contract/sanitizer tests (`Channel<T>`, reserved args, every command) | M | ∞ | TDO-002 | `setup/commands.rs` test |

---

## TDO-EP5 · Security & Secrets

| ID | Title | Size | Phase | Depends | Files |
|---|---|---|---|---|---|
| TDO-040 | Single encrypted secret path (AES-256-GCM ↔ Keychain), grant enforcement | M | ∞ | TDO-011 | secret modules |
| TDO-041 | "Security" panel: surface sanitizer hits, sandbox allowlist, dangerous-tool tracker (read-only + reasons) | M | P2 | TDO-033 | new panel |
| TDO-042 | Master-key rotation + recovery-key in Settings (reuse cloud recovery-key UI) | M | P2 | TDO-040 | `RecoveryKeyPanel.tsx` |
| TDO-043 | Threat-model: untrusted runtime output → React; remote-proxy auth | S | P2 | — | doc + hardening |

---

## TDO-EP6 · Packaging / Update / Platform (macOS-first)

| ID | Title | Size | Phase | Depends | Files |
|---|---|---|---|---|---|
| TDO-050 | CI notarized DMG (hardened runtime + staple); updater signing key in CI | L | P2 | — | CI, `tauri.conf.json` |
| TDO-051 | Auto-update channel wired to `UpdateChecker.tsx` | M | P2 | TDO-050 | `UpdateChecker.tsx` |
| TDO-052 | Sidecar bundling + size budget + lazy download; clean-machine `setup:all` validation | L | P2 | — | `scripts/`, `desktop-sidecars/` |
| TDO-053 | Keep Win/Linux in CI build matrix (compile + core smoke) | M | ∞ | — | CI |

---

## TDO-EP7 · Phase 1 — Parity Closure

### Agent-loop internals
| ID | Title | Size | Depends | Files |
|---|---|---|---|---|
| TDO-100 | **Fix compaction stub** → call real core compaction (Summarize/Truncate/MoveToWorkspace) | M | TDO-001 | `rpc_extensions.rs:1523-1558`, `ThinClawConfig.tsx` |
| TDO-101 | Context-pressure `UiEvent` + header indicator | M | TDO-005 | `context_monitor`, `ui_types.rs` |
| TDO-102 | Self-repair status event + panel row | M | TDO-033 | `self_repair.rs` |
| TDO-103 | Checkpoints UI: `thinclaw_checkpoints_list/diff/restore` + Rollback panel | L | TDO-001 | `checkpoint.rs` |
| TDO-104 | Undo command + control | S | TDO-001 | `agent/undo.rs` |
| TDO-105 | Advisor consultation `UiEvent` in Event Inspector | S | TDO-005 | `dispatcher/advisor.rs` |
| TDO-106 | Trajectory viewer: `thinclaw_trajectory_list/get` + UI | M | TDO-001 | `trajectory.rs` |

### Proactive / learning / experiments
| ID | Title | Size | Depends | Files |
|---|---|---|---|---|
| TDO-110 | Event-triggered routine creation (`Trigger::SystemEvent`) + UI | M | TDO-024 | `rpc_routines.rs:326`, `ThinClawAutomations.tsx` |
| TDO-111 | `evaluate_outcomes`: embedded evaluator OR honest gate + CTA | M | TDO-001 | `rpc_experiments_learning.rs:394` |
| TDO-112 | GPU validate/launch: local path OR honest gate | M | TDO-001 | `rpc_experiments_learning.rs:625-661` |
| TDO-113 | Eval framework command `thinclaw_experiments_run_eval` + Benchmarks panel | L | TDO-001 | `crates/thinclaw-agent/src/env.rs` |
| TDO-114 | `thinclaw_trajectory_export(format)` (SFT/DPO) + export button | M | TDO-106 | `src/cli/trajectory.rs` |
| TDO-115 | Profile-evolution viewer + force-run | S | TDO-001 | `profile_evolution.rs` |

### Channels (largest item)
| ID | Title | Size | Depends | Files |
|---|---|---|---|---|
| TDO-120 | **Channel-config schema framework** (`channel_config_schema` command + generic renderer) | XL | TDO-001 | `ThinClawChannels.tsx`, channel manifests |
| TDO-121 | First channels on framework: Signal, Discord, iMessage, Nostr | L | TDO-120 | channel adapters |
| TDO-122 | Long-tail channel configs (Matrix, Teams, LINE, SMS, BlueBubbles, Apple Mail, …) | L | TDO-120 | channel adapters |
| TDO-123 | Pairing/web-login parity across paired channels | S | TDO-120 | `ThinClawPairing.tsx` |

### Identity / memory / personality
| ID | Title | Size | Depends | Files |
|---|---|---|---|---|
| TDO-130 | `/personality` (`/vibe`) overlay command + chat control | S | TDO-001 | identity/soul |
| TDO-131 | External-memory provider setup/status commands + panel | M | TDO-001 | `external_memory_*` |
| TDO-132 | Finish inline `MemoryEditor` wiring | S | — | `MemoryEditor.tsx` |

### Repo-projects / fleet / remote
| ID | Title | Size | Depends | Files |
|---|---|---|---|---|
| TDO-140 | Complete repo-projects enroll→plan→merge-gate + readiness gates | L | TDO-022 | `rpc_repo_projects.rs`, component |
| TDO-141 | Define fleet model (multi-agent A2A) → real status + broadcast | L | TDO-001 | `fleet.rs`, `FleetCommandCenter.tsx` |
| TDO-142 | Tunnel/Tailscale commands + Remote-access panel | M | TDO-001 | `src/tunnel/` |
| TDO-143 | Replace `subscribe_session` stub with real subscription | S | TDO-001 | `sessions.rs` |

---

## TDO-EP8 · Phase 2 — Stabilization & Upgrade

| ID | Title | Size | Files |
|---|---|---|---|
| TDO-200 | Error taxonomy + user-facing error surfaces (no raw `String` errors) | M | bridge + UI |
| TDO-201 | Bridge resilience: timeouts/retries/reconnect for `RemoteGatewayProxy` + failover UX | M | `runtime_bridge.rs` |
| TDO-202 | Performance budgets: cold start, event-stream throughput, history virtualization, sidecar memory | L | frontend + backend |
| TDO-203 | Model upgrade: default to latest Claude family in catalog + onboarding | M | provider catalog, onboarding |
| TDO-204 | Engine bump (llama.cpp/MLX/vLLM/Ollama) + GGUF/quant matrix validation | L | `engine/*`, sidecars |
| TDO-205 | Tauri v2 capabilities audit + npm/Cargo dep refresh + advisory sweep (fix-at-source) | M | `capabilities/default.json`, manifests |
| TDO-206 | RAG/inference upgrades: reranker refresh, embedding-dim auto-detect hardening | M | `rag.rs`, `reranker.rs`, `hf_hub.rs` |

---

## TDO-EP9 · Phase 3 — UX Refinement & 1.0

| ID | Title | Size | Files |
|---|---|---|---|
| TDO-300 | Design system: token set + shared component library | L | `lib/app-themes.ts`, components |
| TDO-301 | Mode seam: explicit Workbench↔Cockpit switch (state/identity/model) + shared command palette | M | `ModeNavigator.tsx`, `ChatLayout.tsx` |
| TDO-302 | Onboarding overhaul: single wizard for both systems | L | `OnboardingWizard.tsx`, setup wizard |
| TDO-303 | Accessibility pass (keyboard, focus, SR labels, contrast) | L | design system |
| TDO-304 | Frontend i18n wiring (core i18n → UI) | M | `i18n`, frontend |
| TDO-305 | Polish: empty/loading/error states, progress, micro-interactions, density | M | components |

---

## Suggested first sprint (de-risk + prove the loop)
`TDO-001` → `TDO-002` → `TDO-100` (compaction fix, smallest high-signal win) →
`TDO-120` spike (largest parity item) → `TDO-021`/`TDO-020` (split god-files on touch) →
`TDO-030` (CI fixture acceptance gate).
