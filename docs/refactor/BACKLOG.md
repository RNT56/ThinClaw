# Refactor Backlog — executable tasks

Every remaining task as a self-contained unit. Each has: **Priority** (P0–P3), **Effort**
(S≈≤1d, M≈2–4d, L≈1–2wk, XL≈3wk+), **Blocked-by** (do not start while open), **Files**, **Current
state**, **Steps/todos**, **Verify**, and **Guardrail** (the regression gate to ship with it).

Line numbers are 2026-06-29 audit hints — re-locate before editing (see `EXECUTION_PLAYBOOK.md`).

---

## Phase 0 — DONE (reference)

Shipped + verified this cycle: **#126** security (deleted orphaned `DangerousToolTracker`; fixed
trusted-proxy CIDR via `ipnet`), **#127** async (blocking syscalls out of `async fn`s), **#128**
panics (`OnceLock` hot-path regexes/selectors; vision `unreachable!()`→`Err`), **#129** CI
`--locked` on primary gates, **#130** desktop dependency remediation (rand-unsound + 2 yanked,
fixed at source) + desktop advisory CI. Audit/plan: **#125**.

---

## Wave A — clean, ready now (no in-flight conflict)

### A1 · Crate boundary: Routine DTOs → `thinclaw-types` · P0 · L · Blocked-by: —
**Why:** the single worst wrong-direction edge — `thinclaw-db` depends on `thinclaw-agent` only to
get Routine domain types, coupling persistence to the agent (LLM/workspace/media) layer.
**Files:** `crates/thinclaw-agent/src/routine.rs` (2,118 lines — DTOs *intermixed with* regex/
chrono-tz trigger logic), `crates/thinclaw-types/src/` (dest), `thinclaw-db/src/{lib,postgres,
postgres_store/mod}.rs` + `libsql/{mod,routines}.rs`, the two crates' `Cargo.toml`.
**Steps:**
1. This is a *split*, not a move: identify the pure DTO subset `thinclaw-db` imports (`Routine`,
   `RoutineRun`, `RoutineEvent`, `RoutineEventStatus`, `RoutineTriggerStatus`, `RunStatus`,
   `RoutineTrigger`, `RoutineTriggerDecision`, `RoutineEventEvaluation`, …). Confirm their only
   deps are `chrono`/`serde`/`uuid` + `thinclaw_types::{ToolProfile, error::RoutineError}` (both
   already in `thinclaw-types`).
2. Create `crates/thinclaw-types/src/routine.rs` with those types + their pure-data impls; export it.
3. Leave the trigger-evaluation logic (regex/`chrono_tz`) in `thinclaw-agent/src/routine.rs`; make
   it `pub use thinclaw_types::routine::*` for the moved types (path stability).
4. Update the 5 `thinclaw-db` files to import from `thinclaw_types`; remove `thinclaw-agent` from
   `thinclaw-db/Cargo.toml`.
**Verify:** full-workspace `cargo check` + `cargo check --manifest-path apps/desktop/backend/
Cargo.toml`; `cargo test -p thinclaw-db -p thinclaw-agent`; sync both lockfiles.
**Guardrail:** CI step asserting `rg 'thinclaw-agent' crates/thinclaw-db/Cargo.toml` is empty; add
to the codestyle "Check crate boundaries" block.

### A2 · Crate boundary: MCP + execution DTOs → `thinclaw-tools-core` · P0 · M · Blocked-by: —
**Why:** `thinclaw-gateway` (meant to be a light policy/DTO crate) pulls heavyweight
`thinclaw-tools` (→ wasmtime/chromiumoxide/nostr) just for pure DTOs.
**Files:** `crates/thinclaw-tools/src/{mcp/,execution.rs}` (source), `crates/thinclaw-tools-core/`
(dest), `thinclaw-gateway/src/web/{mcp,jobs}.rs` + its `Cargo.toml`.
**Steps:** move `McpTool/McpPrompt/McpResource/McpLoggingLevel/McpPendingInteraction/
GetPromptResult/McpPromptMessage/McpResourceContents/McpResourceTemplate` and
`ExecutionBackendKind/RuntimeDescriptor/local_job_runtime_descriptor/sandbox_job_runtime_descriptor`
to `tools-core` (they are pure data — verify no runtime deps); `pub use` from `thinclaw-tools` for
stability; repoint gateway imports to `tools-core`; drop `thinclaw-tools` from gateway `Cargo.toml`;
confirm gateway no longer transitively pulls wasmtime/browser.
**Verify:** workspace `cargo check`; `cargo tree -p thinclaw-gateway | rg -c 'wasmtime|chromiumoxide'`
== 0; sync lockfiles. **Guardrail:** CI grep that gateway's dep tree excludes wasmtime/chromiumoxide.

### A3 · Observability: rolling file log sink · P0 · M · Blocked-by: —
**Why:** no persistent logs — only stderr + a 500-entry in-memory ring buffer; post-incident
analysis on a non-service deployment is impossible.
**Files:** `crates/thinclaw-gateway/src/web/log_layer.rs` (`init_tracing`, ~178), root `Cargo.toml`,
callers `src/main.rs` + `src/bin/thinclaw-acp.rs`.
**Steps:** add `tracing-appender`; give `init_tracing` a `logs_dir: PathBuf` param; add a
`rolling::daily(logs_dir, "thinclaw.log")` layer alongside the existing fmt + `WebLogLayer`. Handle
the non-blocking `WorkerGuard` lifetime — either use the blocking appender (no guard) or store the
guard for the process lifetime (return it / leak it intentionally). Update both callers to pass
`state_paths().logs_dir`. **Verify:** `cargo check`; lockfile sync. (Runtime "file actually writes"
is acceptable to defer.) **Guardrail:** none.

### A4 · Observability: real `/api/health` readiness · P1 · M · Blocked-by: —
**Why:** `health_handler()` returns a static `{status:"healthy"}` (`src/channels/web/handlers/
gateway.rs:21`) — liveness ≡ readiness; LBs route to broken instances.
**Steps:** add `State<…>` to the handler; DB ping with a 2s timeout + "≥1 LLM provider configured"
check; return 503 on failure, 200 + uptime/SSE-count otherwise; keep the URL. Thread the State into
the route registration. **Verify:** `cargo check`; gateway tests. **Guardrail:** a test asserting a
stubbed-down-DB state returns 503.

### A5 · God-file: `thinclaw-experiments/src/lib.rs` · P1 · M · Blocked-by: —
**Why:** 3,482 lines, **zero submodules** — the clearest god-file; every experiment change
recompiles it all.
**Steps:** decompose into `types.rs` / `policy.rs` (status/lifecycle) / `opportunities.rs` /
`cost.rs` (LLM attribution) / `campaign.rs` / `lease.rs`; `lib.rs` becomes a pure `pub use` façade —
**no public-path change.** **Verify:** `cargo check -p thinclaw-experiments` + its tests pass
untouched. **Guardrail:** contributes toward the size-guard (T10).

### A6 · God-file: `src/main.rs` `async_main` · P1 · M · Blocked-by: —
**Why:** `async_main` is ~1,934 lines (of a 2,384-line file) — inline wiring of 15+ channel types;
every channel/startup change conflicts here.
**Steps:** extract `bootstrap.rs` (AppBuilder), `surface_wiring.rs` (`register_messaging_channels`/
`register_gateway_channels`/`register_native_lifecycle_clients`), `signal_handling.rs`; `main` reads
as named phases, < 200 lines. Use the existing under-used `src/main_helpers.rs`. **Verify:**
`cargo check --features full`; host-smoke compiles. **Guardrail:** size-guard (T10).

### A7 · God-file: skill-tool twins · P2 · L · Blocked-by: —
**Why:** `crates/thinclaw-tools/src/builtin/skill.rs` (4,577) + `src/tools/builtin/skill_tools.rs`
(4,385) — the two largest files, parallel structures changed in lockstep.
**Steps:** one file per tool struct (`SkillInspect/Search/Install/Update/Publish/Remove/Trust/Tap`);
policy/scan helpers to `skill_policy.rs`/`scan.rs`; façade re-exports in both. **Verify:**
`cargo check -p thinclaw-tools --all-features` + skill tests. **Guardrail:** T10.

### A8 · God-files: the rest · P2/P3 · L (each S–M) · Blocked-by: —
Decompose, façade-preserving, each its own PR, in this order:
`src/channels/web/server.rs` (2,392 → per-port modules under `web/ports/`); `thinclaw-channels/src/
signal.rs` (2,918 → `types/client/auth/channel`); `thinclaw-gateway/src/web/providers.rs` (2,827 →
`validation/routing/display/credentials`); `src/agent/routine_engine.rs` (2,809);
`src/channels/acp.rs` (3,150); `src/llm/reasoning.rs` (2,553); `src/tui/mod.rs` (1,160 → `state/
input/layout/event_loop`, mod.rs as façade). **Verify per file:** crate `cargo check` + tests
untouched. **Guardrail:** T10 flips to enforcing once all ≤ threshold.

### A9 · Async lifecycle hardening · P1 · M · Blocked-by: —
**Files:** `src/agent/channel_submission.rs:35` (fire-and-forget spawn — already logs `Err`, but
panics are swallowed + no shutdown tracking), `src/main.rs` (untracked experiment/SIGHUP/SSE
spawns), `src/tools/builtin/learning_tools.rs:606` (skill-registry `reload()` under the write lock),
`src/agent/scheduler.rs` (cleanup spawns not aborted at stop).
**Steps:** add a `ShutdownSet`/`JoinSet` on the Agent (and register the channel-submission + main.rs
spawns; `abort_all` in shutdown); refactor skill reload to load-then-swap (build a fresh registry
off-lock via `discover_all`, then `*write().await = fresh` — touches `src/skills/registry.rs`
internals); register scheduler cleanup spawns for abort. **Verify:** `cargo check`; agent/dispatcher
tests. **Guardrail:** none (consider an `await_holding_lock` re-check).

### A10 · Security long-tail · P1/P2 · M · Blocked-by: —
Independent fixes (can be 1–2 PRs): **(a)** `/tmp` path-escape exemption (`thinclaw-tools/.../
shell_security.rs:1312`) — gate behind an opt-in setting (default = no exemption) threaded into
`detect_path_escape`; note it also currently allows `/tmp/../` traversal. **(b)** `SAFE_BINS` →
rename `ALLOWED_EXECUTABLES` + split read-only vs network/mutation (curl/wget/docker still
smart-approval). **(c)** `ExternalCommandScanner` default `FailOpen → FailClosed` (+ startup warn).
**(d)** `InMemorySecretsStore::record_access_audit` — implement an in-memory audit (no-op stub
today). **(e)** `Policy::default()` shell-injection regex requires a leading `;` — add undecorated
patterns. **(f)** `?token=` query-param: operator startup warning (RFC 6750 log-exposure). **(g)**
`ToolPolicyManager::load_from_settings` per-call I/O on the hot path → short-TTL cache.
**Verify:** `cargo check --all-features`; safety/tools tests + new tests per fix.

### A11 · Panic long-tail · P2 · S · Blocked-by: —
Remaining specific sites (the systemic lint is B-blocked): `src/main.rs:1023` SocketAddr `expect` →
config-validation error; `src/pairing/store.rs` `parent().expect()` ×5 → typed error;
`thinclaw-tools/src/builtin/shell.rs` external-scanner overlaps with A10c; `apps/desktop/backend/src/
system.rs:50` `get_current_pid().unwrap()` → `unwrap_or`. **Verify:** `cargo check` (+ `--all-features`
for desktop). **Guardrail:** see D-note on `unwrap_used` (B-blocked by `-D warnings`).

---

## Wave B — blocked on the in-flight queue (#117/#118/#119 touch these files)

### B1 · `StatusUpdate` `#[non_exhaustive]` + wildcard arms · P1 · S · Blocked-by: #118, #121
**Why:** ends the 6-matcher ripple tax (PRINCIPLES §3.2). **Steps:** add `#[non_exhaustive]` to the
enum (`thinclaw-channels-core/src/channel.rs`); add a `_` fallback arm to the matchers lacking one
(`thinclaw-gateway/.../status.rs` → `SseEvent::Status`; `tui.rs` → `TuiUpdate::Status`; `repl.rs` →
no-op; `acp.rs` → join the `=> None`; `wasm/.../conversions.rs` → `StatusType::Status`;
desktop `event_mapping.rs` → `None`). Construction is unaffected (PRINCIPLES §3.3).
**Verify:** `cargo check` + `cargo check -p thinclaw-channels --all-features`. **Guardrail:** the
`_` arms mean future variants no longer force edits — self-guarding.

### B2 · Emit the 5 dead `ObserverEvent` variants + default observer to `log` · P1 · M · Blocked-by: #118
**Why:** `LlmRequest/ChannelMessage/HeartbeatTick/AgentEnd/Error` are never emitted (5/10 dead);
observer defaults to `none` in every wizard profile. **Steps:** emit `LlmRequest` in
`dispatcher/llm_turn.rs`, `HeartbeatTick` in `heartbeat.rs`, `ChannelMessage` in
`ChannelManager.broadcast`, `AgentEnd` at agent-loop exit, `Error` at the existing `tracing::error!`
sites; default `ObservabilityConfig` + wizard profiles to `log`. (Touches `llm_turn`/`agent_loop` →
conflicts with #118.) **Verify:** `cargo check`. **Guardrail:** a test asserting all 10 variants
have a production emit site.

### B3 · WIT `StatusType` drift · P1 · M · Blocked-by: #118
**Why:** WIT `status-type` has 11 variants vs `StatusUpdate`'s 21 → 10 collapse lossily to `Status`,
so WASM channels can't see lifecycle/subagent/credential events. **Steps:** extend
`wit/channel.wit` `status-type` (or add a structured payload) with the missing types; update
`thinclaw-channels/src/wasm/wrapper/conversions.rs`; bump the WIT interface version for host/artifact
negotiation. **Verify:** `cargo check -p thinclaw-channels --all-features`; rebuild a WASM channel.

### B4 · `ROUTE_TABLE` full coverage + CI guard · P2 · M · Blocked-by: #117, #119
**Why:** 15/341 commands classified (4%). **Steps:** classify all commands by module in batches in
`bridge.rs ROUTE_TABLE`; add a test asserting `specta_builder()` command count == classified-command
count (extend the existing bridge linter). **Verify:** `cargo test --lib bridge::`. **Guardrail:**
the coverage test itself.

### B5 · `Result<T,String>` → `Result<T,BridgeError>` migration · P2 · L · Blocked-by: #117, #119
**Why:** ~149 commands return untyped string errors; the frontend can't render gated-capability CTAs.
**Steps:** file-by-file, change the return type (the `From<String>` impl makes existing
`.map_err(|e| e.to_string())` compile as-is); retire `local_unavailable()` (`rpc_jobs_autonomy.rs`).
Regenerate bindings each file. **Verify:** `export_bindings` + `tsc`; bridge tests.

### B6 · Stringly-typed `UiEvent` status fields → specta enums · P2 · M · Blocked-by: #118 (event_mapping)
Replace free-form `status`/`phase`/`message_type` strings (`ui_types.rs`) with serde-tagged,
specta-exported enums (`ToolStatus`, `RunStatus`, `SubAgentStatus`, `MessageType`).
**Verify:** `export_bindings` + `tsc`.

---

## Dependency hygiene

### D1 · `[workspace.dependencies]` table · P1 · M · Blocked-by: —
Add `[workspace.dependencies]` to root `Cargo.toml`; migrate the ~22 crates to `{ workspace = true }`
for shared deps (serde/tokio/uuid/chrono/tracing/thiserror/anyhow/reqwest/rand/…). Kills silent
per-crate drift. **Verify:** workspace `cargo check`; sync lockfiles. **Guardrail:** none direct.

### D2 · Collapse `rand 0.8 → 0.9` · P1 · M · Blocked-by: D1 (do together)
6 crates pin `rand 0.8` → 3 simultaneous versions. Upgrade to one. **Verify:** `cargo check`;
sync lockfiles.

### D3 · `deny.toml multiple-versions = warn → deny` · P1 · S · Blocked-by: D1/D2 (must dedup first)
Flip to `deny`; add documented, tracking-issue-linked `skip` entries only for genuinely unavoidable
duplicates. **Verify:** `cargo deny check bans`. **Guardrail:** is the guardrail.

### D4 · Desktop `deny.toml` (full bans/licenses) · P2 · M · Blocked-by: —
The desktop fails the root `deny.toml`'s bans/licenses (path-dep "wildcards", `thinclaw-desktop-tools`
unlicensed). Add a desktop-scoped `deny.toml` (license allowlist; add the missing `license` field);
then upgrade the desktop CI step from `check advisories` to full `check`.

### D5 · Long-tail deps · P2 · L · Blocked-by: —
`rig-core` single version (desktop has 0.7 + 0.30); eliminate EOL `rustls 0.21` (via newer
`aws-smithy-http-client`); `ort` `download-binaries` → vendored/hash-verified; replace/remove
`clawscan` (drags `reqwest 0.11`); add Renovate/Dependabot.

---

## Testing & CI

| ID | Task | Pri | Blocked-by |
|---|---|---|---|
| T1 | Coverage threshold gate (`--fail-under`) + drop `--lib` so integration tests count | P1 | — |
| T2 | MCP lifecycle integration test (handshake/list/call/reconnect) | P1 | — |
| T3 | Root-cause the Windows-smoke flake; remove the retry mask | P2 | — |
| T4 | Expand DB contract tests to ≥10/domain (FTS, pgvector, pagination, joins) | P2 | — |
| T5 | Miri CI job for `thinclaw-secrets` (crypto) + `thinclaw-safety` (sanitizer/leak) | P2 | — |
| T6 | Contract test for `SafetyLayer`/`SecretsStore` injection in `AppBuilder` | P2 | — |
| T7 | Frontend tests for the chat hook (`use-chat.ts`) + Tauri bridge | P3 | — |
| T8 | Dedicated MSRV-verification CI job (currently stable pin == MSRV by coincidence) | P3 | — |
| T9 | Extend `--locked` to the remaining CI jobs (host-smoke/acp/release/db-contract) | P1 | — |
| T10 | God-file size-guard CI (fail if any `.rs` > N lines) | P2 | A5–A8 |
| T11 | `wit-bindgen` single-version check + bundle-reference resolution test | P1 | — |

**Note — `clippy::unwrap_used`:** the systemic panic-prevention lint is **blocked** by the existing
`-D warnings` clippy gate (it would hard-error ~2,200 sites). Prereq: decouple that lint from
`-D warnings` (a separate clippy invocation that allows it as warn-only), *or* an `#[allow]` sweep
first. Until then, panic-prevention is per-site (A11) + review.

---

## Build & packaging

| ID | Task | Pri | Blocked-by |
|---|---|---|---|
| P1 | `bundled-wasm` `build.rs` uses `cargo build` not the component pipeline → add `wasm-tools component new` + an `--all-features` extraction-and-load smoke test | P0 | — |
| P2 | Registry artifact URLs pinned `v0.13.6` on `v0.14.0` → automate post-release checksum/version PR | P0 | — |
| P3 | Signed Tauri release pipeline (macOS notarization + Windows Authenticode + updater `latest.json`) — **infra, needs signing secrets** | P1 | external |
| P4 | Scope `cargo-dist` so musl targets don't build `full` (bollard/chromiumoxide) | P2 | — |
| P5 | Fix `registry/_bundles.json` `tools/slack-tool` → `tools/slack`; add a bundle-resolution test (= T11) | P1 | — |

---

## Phase 2 / 3 — maturity

- **Metrics endpoint:** feature-gated Prometheus `/metrics` backed by `ObserverMetric` (latency p99,
  token burn, queue depth); surface per-provider `route_health` in `/api/status` + `thinclaw status`.
- **LLM extraction:** continue porting `src/llm/reasoning.rs` (2,553) + `runtime_manager` behind
  ports so crates needing reasoning policy don't route through root.
- **Schema evolution:** plugin manifest version `!=` → range check (`extensions/manifest.rs:167`);
  settings key-rename registry (`serde(alias)` + DB key migration) documented in `CLAUDE.md`.
- **Protocol versioning:** standardize `UiEvent::Connected.protocol` (local emits 2, remote emits 1).

---

## Sequencing summary

```
Now (parallel-safe, pick any): A1 A2 A3 A4 A5 A6 A9 A10 A11 · D1+D2+D3 · D4 · T1 T2 T3 T9 T11 · P1 P2 P4 P5
After A5–A8 land:              T10 (size-guard)
After #117/#118/#119 merge:    B1 → B2 B3 B6 · B4 B5
After dedup (D1/D2):           D3
Then:                          A7 A8 (god-file long-tail) · D5 · T4 T5 T6 T7 T8 · Phase 2/3
External/infra:               P3 (signing secrets)
```

Re-run the audit + update `METRICS_AND_GUARDRAILS.md` after each wave.
