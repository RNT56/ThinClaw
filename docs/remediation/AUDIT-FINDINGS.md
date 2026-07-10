# ThinClaw — State of the Project Audit (Findings)

> **STATUS: HISTORICAL SNAPSHOT (2026-06-23). All findings below are now RESOLVED.**
> This is a dated findings record, not a live bug list. Every confirmed bug and every
> open-work item captured here was addressed by the 13-workstream remediation stack, which
> merged into `main` (audit-hardening stack, `bda7a61f`/`1fb29984`). For the outcome and the
> remaining deliberately-deferred follow-ups, see `EXECUTION-SUMMARY.md` and `FOLLOWUPS.md`.
> Individual confirmed bugs below carry an inline **RESOLVED** annotation with the fixing commit
> and current file location. Do not treat this document as work still to be done.
>
> **Date:** 2026-06-23 · **Scope (at audit time):** whole workspace (~543K LoC Rust, 28 crates + root facade + WASM channels/tools + Tauri desktop).
> **Method:** 35-agent parallel audit (16 subsystem surveys + 6 cross-cutting hunts) followed by an adversarial verification pass over every high/critical finding (10 confirmed, 1 refuted, 0 left unverified). A hunt agent ran a real `cargo check`. All claims are grounded in `file:line` evidence; corrected severities reflect the verification pass.
> This file is the source-of-truth findings record. The remediation plan that resolved these findings lives in the sibling `WS-*.md` workstream docs and `EXECUTION-PLAYBOOK.md`; the index is `README.md`.

## 1. Executive Summary

ThinClaw is a **genuinely mature, production-grade personal-agent platform** — not a scaffold dressed up with aspirational docs. The load-bearing systems are real and wired end-to-end: the multi-session agent loop (LLM→tools→repeat with iteration caps, stuck-loop detection, compaction, advisor escalation), the durable routine/scheduler/heartbeat engine, the tool registry + MCP runtime, twelve real external WASM tool integrations, the multi-provider LLM stack with live routing/failover/caching, both database backends (Postgres + libSQL), the 215-route web gateway control plane (plus 9 orchestrator routes), an end-to-end autonomous experiments platform, and the identity/soul/memory system. The crate-vs-root split is largely disciplined ports/adapters work rather than copy-paste duplication.

What was **aspirational or half-wired** at audit time was concentrated and identifiable: the desktop cloud-sync subsystem (fully written, never spawned); the self-repair *automatic* tool-rebuild path (`with_builder` never injected); the native dynamic-library plugin pipeline (~1500 lines, signature-verified, zero runtime callers); the observability backend (`create_observer` never called); and several orphaned peripheral modules (voice_wake, tailscale discovery, qr_pairing). **All of these were subsequently resolved:** cloud-sync, self-repair `with_builder`, `create_observer`, the native-plugin `ExtensionKind::NativePlugin` pipeline, and voice_wake are now wired; tailscale discovery and qr_pairing were deleted. See the per-bug RESOLVED annotations and `EXECUTION-SUMMARY.md`.

### The 5 biggest risks

1. **Empty gateway auth token silently disables all `/api` auth** (verified). `gateway_auth_token: ""` → `Some("")`, which `GatewayChannel::new` does not replace with a random token; the constant-time compare then accepts an empty `Bearer` — full auth bypass on the operator control plane. The parallel `GatewayAccess` path filters empties correctly, proving the invariant is violated here.
2. **CI is red on `main`.** `cargo deny check` fails on RUSTSEC-2026-0182 (wasmtime-wasi 36.0.10, in the default `wasm-runtime` path). One-line fix, but it gates the codestyle job.
3. **Sandbox secret-confinement is weaker than documented** (verified). The proxy's credential resolver reads process env, never the AES-256-GCM SecretsStore; injection only fires for plaintext HTTP while every shipped default mapping is HTTPS — so the headline guarantee never fires for defaults.
4. **Cross-backend correctness divergence on the desktop-default DB** (verified). libSQL transcript search feeds raw user input to FTS5 `MATCH`, so a query with a colon/quote/hyphen throws where Postgres tolerates it. The fix pattern already exists next door (memory search).
5. **Shipped WASM channels have a broken auth path and a reachable panic** (verified). Discord webhook signature verification is declared but implemented nowhere; `split_message` panics on multibyte UTF-8 at the size boundary in telegram/slack/discord (whatsapp was fixed, the fix wasn't ported).

None break the core agent runtime. They cluster in **trust boundaries, the desktop app's newer subsystems, and the externally-packaged WASM channels** — the project's frontier, not its core.

## 2. Subsystem Status

| Subsystem | Status | Wired? | ~Complete | One-line assessment |
|---|---|---|---|---|
| Agent runtime & loop | Production | Yes | 88% | Mature, coherent hot path; weakened only by dead self-repair wiring and god-files (`thread_ops.rs` 3032L). |
| Routines / scheduler / heartbeat | Production | Yes | 85% | The real proactive runtime, fully wired; inert config knobs and an orphaned standalone runner. |
| Tools core & registry | Production | Yes | 88% | Disciplined crate/root split, no hot-path stubs; structured `JobToolHostPort` half is stubbed `Unavailable`. |
| WASM tools (external) | Production | Yes | 90% | 12 genuine integrations, zero stubs; gaps are build ergonomics + desktop profile omitting `wasm-runtime`. |
| Channels (native + core) | Production | Yes | 85% | Mature, well-tested; `wrapper.rs` is a 5701L god-file and `self_message` anti-loop is dead. |
| WASM channels (external) | **Mixed** | Yes | 78% | 4 strong custom + 13 thin shims; Discord auth broken, split_message panic in 3 channels. |
| LLM stack & routing | Production | Yes | 85% | Real multi-provider routing/failover/cache; dual routing engines coexist, CheapSplit cascade computed-but-dropped. |
| Database & migrations | Production | Yes | 88% | Both backends shipping; FTS5 divergence bug, parity test only checks column names. |
| Web gateway & HTTP API | Production | Yes | 88% | Solid auth + 215 routes (`src/channels/web/server.rs`) plus 9 orchestrator routes; two API god-files, repo-project SSE emitted but never consumed by UI. |
| Experiments / research | Production | Yes (default off) | 88% | Genuine end-to-end platform; 5434L god-file, unenforced artifact retention, RunPod credit≈USD assumption. |
| Repo-project supervisor | **Partial** | Yes (default off) | 78% | Recent, wired; NeedsPlanning never acted on, concurrency limits inert, unbounded merge-retry loop. |
| Desktop app (Tauri) | **Mixed** | Yes | 82% | Large, mostly real; cloud-sync never spawned, InferenceRouter local + chat backends dead. |
| Setup / onboarding / CLI | **Mixed** | Yes | 85% | Broad and real; env-bootstrap drift, 3 dead CLI modules, onboarding god-files. |
| Safety / sandbox / secrets | **Mixed** | Yes | 72% | Strong live stack; proxy credential confinement gaps + ~4931L of dead drifted `src/safety/*`. |
| Extensions / skills / registry | **Mixed** | Yes | 82% | MCP/WASM/skills production-wired; native-plugin pipeline fully dead (~1500L), god-files. |
| Identity / soul / personality / memory | Production | Yes | 82% | Built and wired, no critical bugs; dual profile renderers drift, "Change Contract" never rendered into prompt. |
| Desktop-autonomy / orchestrator / worker / media | **Mixed** | Yes | 70% | Core pieces wired; voice_wake / tailscale / qr_pairing / observability orphaned. |

## 3. Confirmed Bugs & Correctness Issues

Severities are the corrected post-verification values. **Every bug in this table is now RESOLVED**; the Resolution column records the fixing commit and the current source location.

| # | Issue | File (at audit time) | Sev | Resolution |
|---|---|---|---|---|
| 1 | Empty `gateway_auth_token` → `Some("")` → empty `Bearer` authenticates → full `/api` auth bypass | `crates/thinclaw-config/src/channel_config.rs:183` | **High** | **RESOLVED** (WS-01, `4f88c43e`). The `auth_token` builder now trims then `.filter(\|token\| !token.trim().is_empty())` at `crates/thinclaw-config/src/channel_config.rs:200`, mirroring the `GatewayAccess` empty-token filter at `src/platform/gateway_access.rs:29`. |
| 2 | `cargo deny` fails on RUSTSEC-2026-0182 (wasmtime-wasi 36.0.10) — CI red | `Cargo.toml:160` | **Med** | **RESOLVED**. `wasmtime-wasi` is now `36.0.12` in `Cargo.lock`; root `deny.toml` carries `[advisories] ignore = []` (no stale ignores). `cargo deny check` is green. |
| 3 | libSQL transcript search throws on `:`/`"`/`-`; Postgres tolerates | `crates/thinclaw-db/src/libsql/conversations.rs:846` | **High** | **RESOLVED** (WS-02, `4f88c43e`). Transcript search now sanitizes via the shared `super::fts::sanitize_fts5_match(query)` before `MATCH` at `crates/thinclaw-db/src/libsql/conversations/mod.rs:574` (the file was decomposed into a directory module). |
| 4 | Discord WASM declares signature verification, implements none | `channels-src/discord/src/lib.rs:148` | **High** | **RESOLVED** (WS-03, `d1c447c8`). Ed25519 webhook verification is implemented host-side (`verify_discord_ed25519_signature`) at `crates/thinclaw-channels/src/wasm/router.rs:316`, dispatched via `WebhookSecretValidation::DiscordEd25519` (`schema.rs:556`, `router.rs:700`). |
| 5 | `split_message` panics on multibyte UTF-8 boundary (telegram/slack/discord) | `channels-src/telegram/src/lib.rs:2189`, `channels-src/slack/src/lib.rs:1058`, `channels-src/discord/src/lib.rs:750` | **High** | **RESOLVED** (WS-03, `d1c447c8`). Chunking is now char-aware via `char_indices()` (`channels-src/slack/src/lib.rs:1055`, `channels-src/discord/src/lib.rs:778`); `split_message` is a shared helper. |
| 6 | Sandbox proxy credential resolver reads process env, never encrypted SecretsStore | `src/sandbox/proxy/mod.rs:72` | **Med** | **RESOLVED** (WS-01 wiring, `29188003`). A store-backed `StoreCredentialResolver` reads the encrypted `SecretsStore` (`src/sandbox/proxy/http.rs`). |
| 7 | Sandbox credential injection inert for HTTPS; all default mappings are HTTPS | `src/sandbox/proxy/http.rs:351` | **Med** | **RESOLVED** (WS-01 wiring, `29188003`), together with #6. |
| 8 | Desktop `migrate_to_cloud` flips a flag but spawns no sync — writes stay local | `apps/desktop/backend/src/cloud/mod.rs:445` | **Med** | **RESOLVED** (WS-04, `41091179`). `migrate_to_cloud` (now `mod.rs:418`) runs `migration::run_to_cloud`, and `start_live_sync` (`apps/desktop/backend/src/cloud/live_sync.rs`) starts when `is_cloud_mode()` is true. |

**Lower-severity confirmed:** native-streaming `finish_reason` always `Stop` even with tool calls (telemetry only) `crates/thinclaw-llm/src/rig_adapter.rs:1611`; image_gen progress divide-by-zero → garbage % label (display-only) `apps/desktop/backend/src/image_gen.rs:700`; routine event dispatch break-on-first-error defers the whole batch (self-correcting via idempotency) `src/agent/routine_engine.rs:898`; desktop child-session registry never cleaned (slow leak, lying doc comments) `apps/desktop/backend/src/thinclaw/commands/rpc_orchestration.rs:103`.

**Refuted — do not act on:** "`thinclaw mcp`/`memory` fail to find `DATABASE_URL`" — `Config::from_env` loads dotenv itself (`src/config/mod.rs:217-218`); connection works.

**Provably-safe (audited, no action):** all 22 first-party `unreachable!()` sites are sound; the lone `todo!()` is in vendored `patches/libsql` on a remote-hrana path local-file usage never hits; all four `len()`-subtraction sites are underflow-guarded. **Unverified high/critical findings: 0.**

## 4. Open Work (condensed)

> **All items in this section have been actioned by the 13-workstream remediation stack (merged to
> `main`, `bda7a61f`/`1fb29984`).** The list is retained as the historical work breakdown. A small
> set of deliberately-deferred follow-ups (e.g. `F-13` opendal object-store backend) and known
> not-yet-met targets are tracked live in `FOLLOWUPS.md` and `HANDOFF-REMAINING-WORK.md`, not here.

### P0 — security, red CI, data-correctness
- Bump wasmtime-wasi 36.0.10→36.0.11 (RUSTSEC-2026-0182); remove 3 stale `advisory-not-detected` ignores in `deny.toml:22-24`.
- Close empty-auth-token bypass (`channel_config.rs:183` + `src/channels/web/mod.rs:73`).
- Sanitize libSQL FTS5 MATCH input (`conversations.rs:846`, reuse `workspace.rs:677-693`).
- Fix sandbox credential confinement (`proxy/mod.rs:72`, `proxy/http.rs:351`).

### P1 — correctness & broken shipped features
- `split_message` UTF-8 fix (telegram/slack/discord).
- Discord Ed25519 interaction verification.
- Wire desktop cloud-sync end-to-end OR feature-gate.
- Wire or document self-repair `with_builder` (`src/agent/agent_loop.rs:605`).
- Apply security layers to `extra_public_routes` (`src/channels/web/server.rs:1462`).
- Resolve CLI env-bootstrap drift (`src/main.rs:150,154`) — cosmetic.
- Strengthen `schema_divergence` beyond column names; fail-not-skip on missing `DATABASE_URL`.

### P2 — completeness & operability
- Repo-project supervisor: planner for `NeedsPlanning` (`supervisor.rs:150`); enforce concurrency limits; bounded merge-retry (`pipeline.rs:532`); persist `installation_id` (`api/repo_projects.rs:305`).
- Experiments: enforce `default_artifact_retention_days`; durable remote artifacts; flag RunPod credit≈USD.
- Routines: honor heartbeat `target`/`include_reasoning`; enforce/delete `dedup_window`; pass webhook body to routine.
- LLM: wire/remove CheapSplit cascade (`route_planner.rs:565`); fix native-streaming `finish_reason` (`rig_adapter.rs:1611`).
- Observability: wire `create_observer` or remove; voice_wake (`voice` feature enabled by no profile).
- Desktop: fix S3 `last_modified=0` (`cloud/providers/s3.rs:142`); wire child-session registry cleanup.
- Build ergonomics: wasm target in tool-crate CI; `build-all.sh` build `tools-src`; `wasm-runtime` in desktop profile or document.

## 5. Overhaul Candidates

> **RESOLVED (WS-10, decomposition wave).** Every god-file listed below was decomposed into a
> directory module; **no committed `.rs` file now exceeds 2,000 lines** anywhere in the repo, and a
> CI guard (`scripts/ci/check-file-sizes.sh`, `MAX_LINES=2000`) enforces this. The largest surviving
> façades are `crates/thinclaw-secrets/src/store.rs` (1,974L), `src/agent/agent_loop/mod.rs`
> (1,968L), and `crates/thinclaw-agent/src/session.rs` (1,958L). The half-finished migrations were
> also completed: `src/history/store/` was deleted (consolidated onto `thinclaw-db`) and the 14
> orphaned `src/safety/*.rs` files were removed (live code is `crates/thinclaw-safety/src/*`). The
> list below is the audit-time inventory and cites paths/line-counts that no longer exist.

**God-files (worst first):** `crates/thinclaw-channels/src/wasm/wrapper.rs` (5701L); `src/api/experiments.rs` (5434L, 106 flattening `map_err`); `crates/thinclaw-tools/src/builtin/skill.rs` (4577L) + `src/tools/builtin/skill_tools.rs` (4381L) (clean split, but each a god-file); `src/agent/thread_ops.rs` (3032L, ~850L `process_approval`); `src/llm/runtime_manager.rs` (~3100L); `src/extensions/manager.rs` (3343L); `src/agent/routine_engine.rs`; `src/agent/agent_loop.rs`; `crates/thinclaw-workspace/src/workspace_core.rs`; desktop `rpc_dashboard.rs`/`remote_proxy.rs`/`sidecar.rs`; onboarding `wizard/mod.rs`/`wizard/llm.rs`/`setup/channels.rs`.

**Half-finished crate migrations (highest structural cost):** `src/history/store/` vs `crates/thinclaw-db/src/postgres_store/` (near-byte-for-byte duplication, dead code maintained twice); `src/safety/*.rs` (facade + 14 uncompiled orphans, 5 drifted); `src/media` (half-extracted from `crates/thinclaw-media`).

**Leaky abstractions:** dual LLM routing engines (`SmartRoutingProvider` vs `RoutePlanner`); dual desktop agent stacks (embedded Gateway + rig_lib Orchestrator) with two MCP clients/provider builders; `SpawnSubagentTool` unused `executor`; `Reasoning` unused `SafetyLayer`.

**Cross-channel copy-paste drift:** `json_response`/`split_message`/`conversation_scope_id`/`external_conversation_key`/`url_encode_path`/`validate_input_length` duplicated across WASM crates — already caused the split_message fix to land in only one of four copies.

## 6. Dead / Orphaned Code

**ERASE:** `src/safety/*.rs` (14 files ~4931L); `src/voice_wake.rs` (749L) + `voice` feature + cpal (unless WIRED); `src/tailscale.rs` (331L); `src/qr_pairing.rs` (329L, non-constant-time compare); `crates/thinclaw-agent/src/self_repair.rs:325` `RepairTask`; `crates/thinclaw-channels/src/self_message.rs`; `build.rs:42` `build_telegram_channel`; `crates/thinclaw-gateway/src/web/sse.rs:88` `subscribe()`; `src/cli/{nodes,subagent_spawn,session_export}.rs` (~1014L); confirmed-dead helpers (`install_bundled_channel_from_artifacts`, `redact_gateway_url`, `_secret_cli_access_context`).

**WIRE (built, valuable, disconnected):** self-repair tool rebuild (`with_builder`); native dynamic-library plugin pipeline (`src/extensions/native.rs`, `register_plugin_manifest_contributions`, `ext_health_monitor.rs` — needs `ExtensionKind::NativePlugin` variant + dispatch; `dlopen` unsafe, careful review); observability `create_observer`; WASM table/instance resource limits (`crates/thinclaw-tools/src/wasm/limits.rs:71`).

## 7. Doc vs Code Drift

Repo-project supervisor missing from inventory docs; `CRATE_OWNERSHIP.md` lists 22 of 26 crates (missing `thinclaw-identity`, `thinclaw-soul`, `thinclaw-repo-projects`, `thinclaw-runtime-contracts`); `FEATURE_PARITY.md` §20 hardcoded dated counts; `provider_catalog.rs:4` "20+ providers" vs 16 in registry; 42 "Scrappy" upstream-fork comment refs; `src/workspace/README.md:99` stale `spawn_heartbeat`; Discord WASM `README.md:106` claims nonexistent verification; `deny.toml:3` header points at non-existent `code_style.yml`.

## 8. Security & Safety

Trust-boundary engineering is genuinely strong: constant-time bearer/HMAC/webhook/lease compares, AES-256-GCM + HKDF + AAD-bound secret storage with redacted `Debug` and OS-keychain master key, a unicode/ANSI/homograph-aware shell command scanner, a well-tested SSRF guard, WASM HTTP allowlist with path-traversal hardening, fuel+epoch+memory limits, hardened Docker containers (cap_drop ALL, no-new-privileges, readonly rootfs, non-root). No secret-value logging found.

Confirmed concerns: empty-token bypass (#1), wasmtime advisory (#2), the two sandbox proxy gaps (#6, #7). Posture context: "local operator fully trusted," so #6/#7 are weaker-than-documented confinement, not remote holes.

Lower-priority: `execute_code` runs on the bare host by default with weaker safeguards than the shell tool and `requires_approval: UnlessAutoApproved` (`crates/thinclaw-tools/src/builtin/execute_code.rs:914`); DNS-rebinding protection claimed but not enforced (TOCTOU in `validate_outbound_url`); OAuth `state` generated but never validated (`src/cli/oauth_defaults.rs:347`); filesystem tool no containment when `base_dir` is `None`; external shell scanner defaults fail-open.

## 9. Build / Test / CI Health

Builds clean (`cargo check --no-default-features --features edge` green, zero warnings). Toolchain pinned to Rust 1.92.0. CI matrix broad (7 profiles, 3 OSes, Postgres + libSQL `db_contract`, ACP/host-runtime/deploy smokes, desktop-companion).

Two real problems, **both now RESOLVED:** `cargo deny check` FAILED on main (RUSTSEC-2026-0182), now fixed (wasmtime-wasi now `36.0.12`, `deny.toml [advisories] ignore = []`, gate green); CI clippy omitted `--all-targets`, now fixed: `ci.yml:66` now runs `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` (the feature-matrix leg at `ci.yml:135` also uses `--all-targets`), and the `await_holding_lock` in `crates/thinclaw-config/src/secrets.rs` is now an explicit `#[allow]` with a justifying comment.

15 `#[ignore]` tests (Docker E2Es, live smokes), incl. a quarantined known-flaky `autonomous_campaign_..._end_to_end` (`src/api/experiments.rs:5060`, commit `64b9572f`) masking an unfixed worktree/Docker race. None run in CI.

---

*Generated from the multi-agent audit on 2026-06-23. Remediation plan: see `README.md`, `WS-*.md`, and `EXECUTION-PLAYBOOK.md` in this directory.*
