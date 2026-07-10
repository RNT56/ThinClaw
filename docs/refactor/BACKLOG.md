# Refactor Backlog — executable tasks

Every task as a self-contained unit. Each has: **Priority** (P0–P3), **Effort**
(S≈≤1d, M≈2–4d, L≈1–2wk, XL≈3wk+), **Blocked-by** (do not start while open), **Files**, **Current
state**, **Steps/todos**, **Verify**, and **Guardrail** (the regression gate to ship with it).

Line numbers are 2026-06-29 audit hints — re-locate before editing (see `EXECUTION_PLAYBOOK.md`).

> **Status (2026-07-10):** the audit-hardening stack has landed on `main` (`bda7a61f`). Each task
> below carries a **[DONE]** / **[PARTIAL]** / **[OPEN]** marker reflecting the code as of that date.
> Do not execute a **[DONE]** task; the remaining work is the **[PARTIAL]** and **[OPEN]** items —
> chiefly dependency dedup (D1–D3, which has *regressed*), the `unwrap_used` lint, the coverage
> threshold, a signed release, B5's `Result<_, String>` migration, and the largest-file < 800 target.

---

## Phase 0 — DONE (reference)

Shipped + verified this cycle: **#126** security (deleted orphaned `DangerousToolTracker`; fixed
trusted-proxy CIDR via `ipnet`), **#127** async (blocking syscalls out of `async fn`s), **#128**
panics (`OnceLock` hot-path regexes/selectors; vision `unreachable!()`→`Err`), **#129** CI
`--locked` on primary gates, **#130** desktop dependency remediation (rand-unsound + 2 yanked,
fixed at source) + desktop advisory CI. Audit/plan: **#125**.

---

## Wave A — mostly landed (2026-07-10)

A1–A5 and A7 are **[DONE]**; A8's size guard is LIVE and every file is under 2,000 lines. A6, A9,
A10, A11 remain **[OPEN]/[PARTIAL]**.

### A1 · Crate boundary: Routine DTOs → `thinclaw-types` · P0 · L · **[DONE]**
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
**Landed:** Routine DTOs live in `crates/thinclaw-types/src/routine.rs`; `thinclaw-db` no longer
depends on `thinclaw-agent`; the guard is LIVE in `.github/workflows/ci.yml` "Check crate boundaries".

### A2 · Crate boundary: MCP + execution DTOs → `thinclaw-tools-core` · P0 · M · **[DONE]**
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
**Landed:** `thinclaw-gateway/Cargo.toml` depends on `thinclaw-tools-core`, not `thinclaw-tools`;
the boundary is asserted in the CI "Check crate boundaries" block.

### A3 · Observability: rolling file log sink · P0 · M · **[DONE]**
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
**Landed:** a rolling daily file sink attaches alongside the ring buffer at
`crates/thinclaw-gateway/src/web/log_layer.rs` (`tracing_appender::rolling::daily(dir, "thinclaw.log")`).

### A4 · Observability: real `/api/health` readiness · P1 · M · **[DONE]**
**Why:** `health_handler()` formerly returned a static `{status:"healthy"}` (now at
`src/channels/web/handlers/gateway.rs:37`) — liveness ≡ readiness; LBs route to broken instances.
**Steps:** add `State<…>` to the handler; DB ping with a 2s timeout + "≥1 LLM provider configured"
check; return 503 on failure, 200 + uptime/SSE-count otherwise; keep the URL. Thread the State into
the route registration. **Verify:** `cargo check`; gateway tests. **Guardrail:** a test asserting a
stubbed-down-DB state returns 503.
**Landed:** `health_handler` (`src/channels/web/handlers/gateway.rs:37`) now checks DB health under a
2s timeout, LLM-provider configuration, and an inbound channel, returning 503 when unhealthy; the
decision fn is `crates/thinclaw-gateway/src/web/status.rs`.

### A5 · God-file: `thinclaw-experiments/src/lib.rs` · P1 · M · **[DONE]**
**Why:** was 3,482 lines with **zero submodules** — the clearest god-file; every experiment change
recompiled it all.
**Steps:** decompose into `types.rs` / `policy.rs` (status/lifecycle) / `opportunities.rs` /
`cost.rs` (LLM attribution) / `campaign.rs` / `lease.rs`; `lib.rs` becomes a pure `pub use` façade —
**no public-path change.** **Verify:** `cargo check -p thinclaw-experiments` + its tests pass
untouched. **Guardrail:** contributes toward the size-guard (T10).
**Landed:** `crates/thinclaw-experiments/src/lib.rs` is now a 22-line façade over `types.rs` /
`policy.rs` / `cost.rs` / `opportunities.rs` / `messages.rs` / `support.rs`.

### A6 · God-file: `async_main` · P1 · M · **[PARTIAL]**
**Why:** inline wiring of 15+ channel types; every channel/startup change conflicts here.
**Current state:** `async_main` was moved out of `src/main.rs` into its own module
`src/async_main.rs`; `src/main.rs` is now 356 lines and `src/bootstrap.rs` (623) + `src/main_helpers.rs`
(634) were extracted. **But `src/async_main.rs` is still ~1,928 lines** — the phase extraction is
incomplete. Under the 2,000-line guard, but not yet the intended "named phases" shape.
**Remaining:** finish extracting `surface_wiring` / `signal_handling` so `async_main` reads as named
phases. **Verify:** `cargo check --features full`; host-smoke compiles. **Guardrail:** size-guard (T10, live).

### A7 · God-file: skill-tool twins · P2 · L · **[DONE]**
**Why:** the former `skill.rs` (4,577) + `skill_tools.rs` (4,385) were the two largest files,
parallel structures changed in lockstep.
**Steps:** one file per tool struct; policy/scan helpers split out; façade re-exports in both.
**Verify:** `cargo check -p thinclaw-tools --all-features` + skill tests.
**Landed:** both are now directory modules — `crates/thinclaw-tools/src/builtin/skill/` and
`src/tools/builtin/skill_tools/` (one file per tool: `inspect`, `install`, `list`, `check`, `audit`,
etc.).

### A8 · God-files: the rest · P2/P3 · L (each S–M) · **[PARTIAL]**
Decompose, façade-preserving, each its own PR. Progress (all now under the 2,000-line guard):
`thinclaw-channels/src/signal.rs` → `signal/` directory ✅; `thinclaw-gateway/src/web/providers.rs`
→ `providers/` directory ✅; `src/channels/web/server.rs` now 1,672 lines ✅;
`src/agent/routine_engine.rs` now 1,580 lines ✅. **Still large (mod file under the guard but not at
< 800):** `src/channels/acp.rs` (~1,928, decomposed into `acp/` submodules but the mod file remains
big) and `src/llm/reasoning.rs` (~1,938, `reasoning/` submodules, mod file still big). `src/tui/mod.rs`
is 1,160. **Verify per file:** crate `cargo check` + tests untouched. **Guardrail:** the size-guard
is LIVE at 2,000 and passes; a stricter threshold is future work.

### A9 · Async lifecycle hardening · P1 · M · **[OPEN]**
**Files:** `src/agent/channel_submission.rs:35` (fire-and-forget spawn — already logs `Err`, but
panics are swallowed + no shutdown tracking), `src/main.rs` (untracked experiment/SIGHUP/SSE
spawns), `src/tools/builtin/learning_tools.rs:606` (skill-registry `reload()` under the write lock),
`src/agent/scheduler.rs` (cleanup spawns not aborted at stop).
**Steps:** add a `ShutdownSet`/`JoinSet` on the Agent (and register the channel-submission + main.rs
spawns; `abort_all` in shutdown); refactor skill reload to load-then-swap (build a fresh registry
off-lock via `discover_all`, then `*write().await = fresh` — touches `src/skills/registry.rs`
internals); register scheduler cleanup spawns for abort. **Verify:** `cargo check`; agent/dispatcher
tests. **Guardrail:** none (consider an `await_holding_lock` re-check).

### A10 · Security long-tail · P1/P2 · M · **[OPEN]**
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

### A11 · Panic long-tail · P2 · S · **[PARTIAL]**
**Landed — every named production site is resolved.** `src/main.rs` no longer parses a `SocketAddr`
and contains no `expect(` at all (the file is now a 356-line entrypoint). `src/pairing/store.rs`
(`parent().expect()` ×5) was removed with that orphaned file in PR #197.
`apps/desktop/backend/src/system.rs` now uses `get_current_pid().ok()` and degrades to 0 app memory
instead of panicking. The only remaining `SocketAddr` `.unwrap()`s are in `#[cfg(test)]` code
(`src/channels/web/discovery.rs:223,238`).
**Still open:** the systemic guardrail. `clippy::unwrap_used` is still `"allow"` (`Cargo.toml:466`)
because turning it on collides with `-D warnings`; that must be decoupled first. The
`thinclaw-tools/src/builtin/shell.rs` external-scanner site overlaps with A10c.
**Verify:** `cargo check` (+ `--all-features` for desktop). **Guardrail:** see D-note on
`unwrap_used` (B-blocked by `-D warnings`).

---

## Wave B — partially landed (2026-07-10; no longer queue-blocked)

The #117/#118/#119 queue has drained. B1, B2, B3, B4 are **[DONE]**; B5 and B6 remain **[OPEN]**.

### B1 · `StatusUpdate` `#[non_exhaustive]` + wildcard arms · P1 · S · **[DONE]**
**Why:** ends the 6-matcher ripple tax (PRINCIPLES §3.2). **Steps:** add `#[non_exhaustive]` to the
enum (`thinclaw-channels-core/src/channel.rs`); add a `_` fallback arm to the matchers lacking one
(`thinclaw-gateway/.../status.rs` → `SseEvent::Status`; `tui.rs` → `TuiUpdate::Status`; `repl.rs` →
no-op; `acp.rs` → join the `=> None`; `wasm/.../conversions.rs` → `StatusType::Status`;
desktop `event_mapping.rs` → `None`). Construction is unaffected (PRINCIPLES §3.3).
**Verify:** `cargo check` + `cargo check -p thinclaw-channels --all-features`. **Guardrail:** the
`_` arms mean future variants no longer force edits — self-guarding.
**Landed:** `#[non_exhaustive]` sits above `pub enum StatusUpdate` at
`crates/thinclaw-channels-core/src/channel.rs:231`.

### B2 · Emit the 5 dead `ObserverEvent` variants + default observer to `log` · P1 · M · **[DONE]**
**Why:** `LlmRequest/ChannelMessage/HeartbeatTick/AgentEnd/Error` are never emitted (5/10 dead);
observer defaults to `none` in every wizard profile. **Steps:** emit `LlmRequest` in
`dispatcher/llm_turn.rs`, `HeartbeatTick` in `heartbeat.rs`, `ChannelMessage` in
`ChannelManager.broadcast`, `AgentEnd` at agent-loop exit, `Error` at the existing `tracing::error!`
sites; default `ObservabilityConfig` + wizard profiles to `log`. (Touches `llm_turn`/`agent_loop` →
conflicts with #118.) **Verify:** `cargo check`. **Guardrail:** a test asserting all 10 variants
have a production emit site.
**Landed:** all 10 `ObserverEvent` variants now have production emit sites — AgentStart
(`src/app.rs:1721`), LlmRequest (`src/agent/dispatcher/llm_turn.rs:368`), LlmResponse (`:690`),
TurnComplete (`:698`), ToolCallStart/End (`src/agent/dispatcher/tool_execution.rs:289`/`:311`),
ChannelMessage (`src/agent/agent_loop/message_handling.rs:12`), HeartbeatTick
(`src/agent/commands.rs:319`), AgentEnd (`src/agent/agent_loop/mod.rs:1608`), Error (`:1933`). Zero
dead variants.

### B3 · WIT `StatusType` drift · P1 · M · **[DONE]**
**Why:** WIT `status-type` used to carry far fewer variants than `StatusUpdate`, so lifecycle,
subagent, credential-prompt, compaction, advisor and self-repair events collapsed lossily to the
generic `status` variant and WASM channels could not classify them. **Landed:** `wit/channel.wit`
`status-type` now enumerates every host `StatusUpdate` variant (27 WIT entries covering all 25 host
variants, plus `done` / `interrupted`); `crates/thinclaw-channels/src/wasm/wrapper/conversions.rs`
maps each one explicitly; and the WIT interface is versioned for additive host/artifact negotiation
(`CHANNEL_WIT_VERSION = "0.2.0"`, `crates/thinclaw-channels/src/wasm/wrapper/mod.rs:85`).
**Verify:** `cargo check -p thinclaw-channels --all-features`; rebuild a WASM channel.

### B4 · `ROUTE_TABLE` full coverage + CI guard · P2 · M · **[DONE]**
**Why:** was 15/341 commands classified (4%). **Steps:** classify all commands by module in batches in
`bridge.rs ROUTE_TABLE`; add a test asserting `specta_builder()` command count == classified-command
count (extend the existing bridge linter). **Verify:** `cargo test --lib bridge::`. **Guardrail:**
the coverage test itself.
**Landed:** `ROUTE_TABLE` (`apps/desktop/backend/src/thinclaw/bridge.rs:116`) classifies all 346
commands (100%), enforced by `all_registered_commands_are_classified` (`bridge.rs:764`).

### B5 · `Result<T,String>` → `Result<T,BridgeError>` migration · P2 · L · **[OPEN]** · Blocked-by: —
**Why:** 313 of 342 commands still return untyped string errors; the frontend can't render
gated-capability CTAs. (The original ~149 figure undercounted the surface.)
**Steps:** file-by-file, change the return type (the `From<String>` impl makes existing
`.map_err(|e| e.to_string())` compile as-is); retire `local_unavailable()` (`rpc_jobs_autonomy.rs`).
Regenerate bindings each file. **Verify:** `export_bindings` + `tsc`; bridge tests.

### B6 · Stringly-typed `UiEvent` status fields → specta enums · P2 · M · **[OPEN]** · Blocked-by: —
Replace free-form `status`/`phase`/`message_type` strings (`ui_types.rs`) with serde-tagged,
specta-exported enums (`ToolStatus`, `RunStatus`, `SubAgentStatus`, `MessageType`).
**Verify:** `export_bindings` + `tsc`.

---

## Dependency hygiene

> **Regression note (2026-07-10):** root-lock dedup has moved the *wrong* way — 100 duplicate-versioned
> crates now (baseline was 94), 3 `rand` versions (0.8.6 / 0.9.4 / 0.10.1), 2 `wit-bindgen` versions
> (0.51.0 / 0.57.1). D1–D3 are the priority open work.

### D1 · `[workspace.dependencies]` table · P1 · M · **[PARTIAL]**
**Landed:** `[workspace.dependencies]` exists in root `Cargo.toml:26` with 9 hoisted deps (serde,
serde_json, anyhow, thiserror, tracing, chrono, async-trait, futures, utoipa), and 27 of the 28
crates consume them via `{ workspace = true }` (122 dep lines).
**Still open:** `tokio`, `uuid`, `reqwest` (per-crate feature divergence) and `rand` are
deliberately not hoisted yet, per the rationale comment at `Cargo.toml:21-24`. Resolving the `rand`
split is D2. **Verify:** workspace `cargo check`; sync lockfiles. **Guardrail:** none direct.

### D2 · Collapse `rand 0.8 → 0.9` · P1 · M · **[OPEN]** · Blocked-by: D1 (do together)
Root lock still carries 3 simultaneous `rand` versions (0.8.6 / 0.9.4 / 0.10.1). Upgrade to one.
**Verify:** `cargo check`; sync lockfiles.

### D3 · `deny.toml multiple-versions = warn → deny` · P1 · S · **[OPEN]** · Blocked-by: D1/D2 (must dedup first)
Still `multiple-versions = "warn"` (`deny.toml:38`). Flip to `deny`; add documented,
tracking-issue-linked `skip` entries only for genuinely unavoidable duplicates. **Verify:**
`cargo deny check bans`. **Guardrail:** is the guardrail.

### D4 · Desktop `deny.toml` (full bans/licenses) · P2 · M · **[DONE]**
The desktop formerly failed the root `deny.toml`'s bans/licenses. **Landed:** a desktop-scoped
`apps/desktop/backend/deny.toml` exists, and the desktop CI step now runs the full
`cargo deny check licenses bans sources` (`ci.yml:210`) **plus** `cargo deny check advisories`
(`ci.yml:213`). Note the `channels-src/` and `tools-src/` sub-workspace lockfiles are still unscanned
(tracked separately as T-sub).

### D5 · Long-tail deps · P2 · L · **[OPEN]**
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
| T10 | **[DONE]** God-file size-guard CI (`scripts/ci/check-file-sizes.sh`, `MAX_LINES=2000`, `ci.yml:64`) | P2 | A5–A8 |
| T11 | `wit-bindgen` single-version check + bundle-reference resolution test — **[OPEN]**, still 2 versions (0.51.0, 0.57.1) | P1 | — |

**Note — `clippy::unwrap_used`:** still **NOT enabled** — it is set to `"allow"` (`Cargo.toml:466`).
The systemic panic-prevention lint remains **blocked** by the existing `-D warnings` clippy gate (it
would hard-error ~2,200 sites). Prereq: decouple that lint from `-D warnings` (a separate clippy
invocation that allows it as warn-only), *or* an `#[allow]` sweep first. Until then, panic-prevention
is per-site (A11) + review.

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

- **Metrics endpoint — [DONE]:** the Prometheus `/metrics` route is registered
  (`src/channels/web/server.rs:875`) and backed by the shared registry
  (`src/observability/prometheus.rs`, which registers 22 series). Still **[OPEN]:** surfacing
  per-provider `route_health` in `/api/status` + `thinclaw status`.
- **LLM extraction — [OPEN]:** continue porting `src/llm/reasoning.rs` (`reasoning/` mod file
  ~1,938 lines) + `runtime_manager` (now `src/llm/runtime_manager/`, 11 modules) behind
  ports so crates needing reasoning policy don't route through root.
- **Schema evolution:** plugin manifest version `!=` → range check (`extensions/manifest.rs:167`);
  settings key-rename registry (`serde(alias)` + DB key migration) documented in `CLAUDE.md`.
- **Protocol versioning:** standardize `UiEvent::Connected.protocol` (local emits 2, remote emits 1).

---

## Sequencing summary (remaining work, 2026-07-10)

Done: A1 A2 A3 A4 A5 A7 · B1 B2 B4 · D4 · T10 · metrics endpoint. Remaining:

```
Now (parallel-safe, pick any): A6 (finish async_main phases) · A8 (acp/reasoning < 800) ·
                               A9 A10 A11 · D1+D2 · T1 T2 T3 T9 T11 · P1 P2 P4 P5 · B3 B6
After dedup (D1/D2):           D3 (multiple-versions = deny)
Decouple -D warnings first:    clippy::unwrap_used
Then:                          B5 (Result<_,String> migration) · D5 · T4 T5 T6 T7 T8 ·
                               LLM extraction / schema evolution / protocol versioning
External/infra:               P3 (signed release — signing secrets)
```

Re-run the audit + update `METRICS_AND_GUARDRAILS.md` after each wave.
