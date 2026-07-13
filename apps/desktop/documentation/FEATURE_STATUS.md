# ThinClaw Desktop — Feature Status & Verified Gaps

> **Snapshot:** re-verified 2026-07-12 (originally 2026-06-28) · point-in-time,
> code-grounded. This is intentionally *thinly scoped* (per the repo doc rules): it does
> not re-inventory every feature. For the roadmap and parity ledger use the canonical docs
> below; this file only records **verified open gaps** that are easy to lose track of.
> Several earlier gaps (tool-policy deny-list, per-channel stream-mode, Gmail label filter,
> the sidecar/dashboard status rows, the web-search probes) have since been closed and were
> removed from the table.

## Orientation

ThinClaw Desktop is a Tauri v2 app that intentionally contains two AI systems plus
shared infrastructure:

- **Direct AI Workbench** — non-autonomous local/cloud chat, RAG, voice, and image
  generation (`backend/src/chat.rs`, `rig_lib/`, `inference/`, `engine/`).
- **ThinClaw Agent Cockpit** — embeds the autonomous ThinClaw runtime in-process, or
  proxies to a remote gateway (dual-mode: embedded `inner` vs `RemoteGatewayProxy`).
- **Shared infra** — secrets, sidecars, settings, onboarding, cloud sync.

The app is **experimental / pre-1.0**. Read [`runtime-boundaries.md`](runtime-boundaries.md)
before changing either system.

### Canonical status docs (authoritative)

| Topic | Doc |
|---|---|
| Overhaul roadmap & phases | [`OVERHAUL_PLAN.md`](OVERHAUL_PLAN.md) |
| Executable backlog (TDO-###) | [`OVERHAUL_BACKLOG.md`](OVERHAUL_BACKLOG.md) |
| Runtime parity tiers | [`runtime-parity-checklist.md`](runtime-parity-checklist.md) |
| Two-system boundaries | [`runtime-boundaries.md`](runtime-boundaries.md) |
| Local/remote command behavior | [`remote-gateway-route-matrix.md`](remote-gateway-route-matrix.md) |
| Cross-surface parity ledger | [`../../../FEATURE_PARITY.md`](../../../FEATURE_PARITY.md) |

## Verified open gaps

No correctness gap remains open in this narrow point-in-time list. Product breadth,
packaging, shared-service unification, and native release qualification remain tracked
in the canonical roadmap rather than being duplicated here.

## Recently closed

| Area | Resolution | Evidence |
|---|---|---|
| Skills | The unsupported toggle was removed; the panel is explicitly read-only. | `ThinClawSkills.tsx`, `rpc_skills.rs` |
| Channels | The unreachable WhatsApp QR modal was removed; unsupported login is honestly gated. | `ThinClawChannels.tsx`, `rpc_config.rs` |
| Voice | Read Aloud decodes encoded audio through `decodeAudioData`, with PCM fallback only for raw responses. | `MessageBubble.tsx` |
| Cloud sync | `AppNapGuard` now owns a real `NSProcessInfo` activity token and ends it exactly once on drop. | `cloud/app_nap.rs` |
| Desktop startup | The default `npm run dev` now launches the Tauri runtime; renderer-only development is explicit as `dev:web`. | `package.json`, `tauri.conf.json` |
| Frontend performance | App/mode/control-surface lazy loading keeps every production JS chunk under the enforced 500 KiB budget. | `App.tsx`, `ChatLayout.tsx`, `ThinClawView.tsx`, `check_frontend_bundle.mjs` |
| Runtime performance budgets | Backend/renderer readiness is measured; Agent token deltas are frame-coalesced with strict ordering; both chat histories are virtualized; Desktop and descendant-sidecar memory are split and compared with the configured ceiling. | `performance-budgets.md`, `thinclaw-event-buffer.ts`, `system.rs` |
| Claude catalog | Fable 5, Opus 4.8, Sonnet 5, and Haiku 4.5 metadata/defaults are source-verified and shared by runtime setup, Desktop onboarding, discovery fallback, Bedrock, and cost tracking. | `registry/models.json`, `registry/providers.json`, `docs/LLM_PROVIDERS.md` |
| Local engine matrix | llama.cpp/uv assets are checksum and version verified, MLX/vLLM bootstraps use exact pins and versioned markers, Ollama reports its external version, and bounded GGUF metadata/quant validation fails before sidecar spawn. | `engine-compatibility.md`, `engine/mod.rs`, `gguf.rs`, setup scripts |
| Frontend E2E | WebdriverIO Tauri browser mode exercises onboarding and deterministic IPC in CI without a native binary; the runner resolves the exact installed Chrome driver on macOS. | `wdio.browser.conf.ts`, `e2e/onboarding.e2e.ts`, `scripts/run_browser_e2e.mjs` |
| Prompt authority | PromptStack and Prompt System V2 now compile through one typed, budgeted authority graph per real provider request; untyped system text is demoted to untrusted evidence. | `src/llm/reasoning.rs`, `crates/thinclaw-llm-core/src/prompt_stack.rs`, `docs/PROMPT_SYSTEM.md` |
| Routine isolation | Desktop-autonomy state is retained by `AppComponents` and explicitly injected into the desktop agent and routine engine, so independent app builds cannot alter routine emergency-stop behavior through process-global state. | `src/app.rs`, `src/agent/routine_engine.rs`, `backend/src/thinclaw/runtime_builder.rs` |
| Build hygiene | Bundled WASM extension builds now use the parent Cargo `OUT_DIR`; all-feature builds no longer create nested `target/` trees inside channel or tool sources. | `build.rs`, `tests/repo_hygiene.rs` |

## Notes

- `update_remote_model_catalog` / `get_remote_model_catalog` remain a generated,
  local-only backend contract with no current panel consumer. They are not presented as
  a user feature; remove them only with the normal specta binding regeneration flow, or
  wire them when the shared model-registry roadmap reaches that migration.
