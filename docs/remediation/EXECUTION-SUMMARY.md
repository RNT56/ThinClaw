# ThinClaw Remediation — Execution Summary

> **Date:** 2026-06-25 · **Branch:** `remediation/execution`
> What was actually executed against the [audit findings](./AUDIT-FINDINGS.md) and [13-workstream plan](./README.md). Every commit was verified green before landing (`cargo fmt` · `clippy --all-targets -D warnings` · tests · `cargo deny`), with the desktop package verified on its own build.

## Outcome

**The entire 13-workstream remediation is complete.** Every confirmed bug fixed, every security hole closed, every "half-wired/aspirational" vision feature wired end-to-end, ~14K lines of dead/duplicate code removed (operator-approved, evidence-reviewed), the dual persistence layer consolidated to one, and the WS-10 architecture overhaul finished: **10 god-files decomposed** (`wasm/wrapper.rs`, `api/experiments.rs`, `agent/thread_ops.rs`, `llm/runtime_manager.rs`, `extensions/manager.rs`, `workspace_core.rs`, the 3 onboarding files, and the 3 desktop files) plus the `thinclaw-media` crate migration finished. Every commit verified green before landing.

## Commits

| Commit | Scope |
|---|---|
| `976fad29` | Remediation plan: audit findings + 13 workstreams + execution playbook + dossier |
| `4f88c43e` | **WS-01** Security & CI + **WS-02** DB correctness |
| `29188003` | **WS-01** wiring: store-backed credential resolver, DNS-rebind pin, OAuth state |
| `dd7b7cdb` | **WS-06** Repo-project supervisor + **WS-09** Routines/heartbeat |
| `d1c447c8` | **WS-03** WASM channels: split_message panic + Discord Ed25519 |
| `daf8f440` | **WS-05** Self-repair + observability + native-plugin pipeline |
| `41091179` | **WS-04** Desktop: cloud-sync built + S3 metadata + orchestration |
| `c5c27e56` | **WS-07** Experiments + **WS-08** LLM routing consolidation |
| `1ed26495` | **WS-12** Docs drift sync + **WS-13** Test/CI infrastructure |
| `4f26f5f4` | **Wave 4** Erase ~8K lines of verified-dead code (10 dossier items) |
| `43460933` | **WS-10** history/store consolidation + **WS-11** wire voice_wake |
| `d0328176` | **WS-10** decompose `wasm/wrapper.rs` (5701L → 6 modules) |
| `8222b581` | **WS-10** decompose `api/experiments.rs` (5919L → 9 modules) |
| `8dae446f` | **WS-10** decompose `agent/thread_ops.rs` (3031L → 7 modules) |
| `694c6a9f` | **WS-10** decompose `llm/runtime_manager.rs` (3210L → 11 modules) |
| `48986ba0` | **WS-10** decompose `extensions/manager.rs` (3583L → 8 modules) |
| `3e706a39` | **WS-10** decompose `workspace_core.rs` (2248L → 11 modules) |
| `c1404519` | **WS-10** decompose onboarding (wizard + channels) |
| `cc839765` | **WS-10** decompose desktop (rpc_dashboard + remote_proxy + sidecar) |
| `71adfa30` | **WS-10** finish `thinclaw-media` crate migration |

## Headline fixes (security & correctness)

- **Empty `gateway_auth_token` auth bypass** — closed at both config + constructor layers.
- **RUSTSEC-2026-0182** (wasmtime-wasi) — bumped; `cargo deny` green; CI un-redded.
- **OAuth `state`** — generated + constant-time-validated end-to-end (login-CSRF closed).
- **Sandbox credential confinement** — store-backed resolver wired into the live proxy.
- **DNS-rebinding TOCTOU** — HTTP/extract/WASM clients pin the validated IP.
- **libSQL FTS5 punctuation crash** — shared sanitizer + dual-backend regression test.
- **Discord Ed25519 webhook verification** — implemented (was claimed-but-absent).
- **`split_message` multibyte-UTF-8 panic** — fixed across telegram/slack/discord.
- **CI clippy** hardened to `--all-targets` + the pre-existing lints it exposed cleared.

## Vision features wired (were built-but-inert)

- **Desktop cloud-sync** — real end-to-end (FileStore cloud mode + upload worker + SyncEngine + read-path download + startup restore), replacing a cosmetic flag.
- **Native dynamic-library plugin pipeline** — made reachable (operator-approved), strictly default-off and signature-verified-before-`dlopen`, `catch_unwind`-isolated.
- **Self-repair automatic tool rebuild** — `with_builder` injected (was always `ManualRequired`).
- **Observability** — `create_observer` threaded through `AppBuilder`.
- **voice_wake** — headless wake word wired behind the `voice` feature (EnergyDetector default; Sherpa-ONNX keyword model is a documented upgrade).
- **Repo-project supervisor** — autonomous planner port, concurrency limits, bounded merge-retry, installation-id persistence.
- **Routines/heartbeat** — target/verbosity/dedup-window/webhook-body all honoured.
- **Experiments** — artifact-retention reaper + durable `ArtifactStore` + cost-basis surfacing.
- **LLM routing** — RoutePlanner canonical, CheapSplit cascade wired, finish_reason fixed.

## Dead code erased (Wave 4, ~8K lines, operator sign-off + per-item dossier)

`src/safety/*` orphans · dead CLI stubs · InferenceRouter chat modality · `SmartRoutingProvider` decorator · `self_message` · `qr_pairing` · `tailscale` discovery · `RepairTask` · standalone heartbeat runner · leaky `Reasoning.safety`/`SpawnSubagentTool.executor` fields · dead HTTPS cred mappings + misc helpers. Each had a verified live replacement — see [DELETION-DOSSIER.md](./DELETION-DOSSIER.md).

## Structural

- **history/store consolidation** — deleted the stalled-extraction duplicate; one Postgres store (`thinclaw-db`) remains; 6 callers redirected; root `history` is now a thin façade.
- **`wasm/wrapper.rs` (5701L)** — decomposed into 6 focused modules; Telegram logic behind a `WasmChannelTransport` trait.

## WS-10 architecture overhaul (complete)

All god-file decompositions landed as behaviour-preserving directory-module splits with public paths preserved (façade `mod.rs` + cohesive submodules + `pub use`), each verified green:

- `wasm/wrapper.rs` 5701L → 6 modules (Telegram logic behind a `WasmChannelTransport` trait)
- `api/experiments.rs` 5919L → 9 modules · `agent/thread_ops.rs` 3031L → 7 · `llm/runtime_manager.rs` 3210L → 11
- `extensions/manager.rs` 3583L → 8 · `workspace_core.rs` 2248L → 11
- onboarding: `wizard/mod.rs` (→ flow/profile/verification/readiness/reconnect), `wizard/llm.rs` (→ providers/models/steps), `setup/channels.rs` (→ per-channel modules)
- desktop: `rpc_dashboard.rs` → 9, `remote_proxy.rs` → 10, `sidecar.rs` → 5
- `thinclaw-media` migration finished (extractors moved into the crate; `AudioExtractor`+`MediaPipeline` kept root-local to respect dependency direction)

The biggest remaining file (`channels-src/telegram/src/lib.rs`, a packaged WASM channel) is out of the main workspace and tracked separately; within the workspace, no single-file god-file over ~2.4K lines remains.

## Deferred (tracked, non-blocking)

See [FOLLOWUPS.md](./FOLLOWUPS.md) (F-01…F-19): e.g. the experiments-backend credential path, the MCP-client DNS pin, the shared channel/tool SDK, observability per-event emission, voice CI ALSA + STT-capture-on-wake glue. None is a regression; each is a residual to fully close its area.
