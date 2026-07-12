# ThinClaw — Robustness & Architecture Hardening Plan

> **Status:** COMPLETED (proposal v1, 2026-06-29). Most of Phases 0–2 shipped via the
> audit-driven remediation stack (13 workstreams, commits `4f88c43e`…`43460933` plus F-01..F-19
> follow-ups), merged to `main` at `bda7a61f`. This document is retained as a historical record and
> annotated with a **2026-07-11 status update**; it is no longer an execution checklist. Do not pick
> it up and start executing landed phases.
>
> **What landed:** the two wrong-direction crate edges are removed and CI-guarded (§1.6); all
> historical god-files are decomposed and a live 2,000-line CI guard prevents regression (§2.1);
> `/api/health` is a real readiness probe, the persistent rolling log sink exists, and all 10
> `ObserverEvent` variants emit (§0.4); the trusted-proxy CIDR fix (ipnet), the deleted
> `DangerousToolTracker`, and `StatusUpdate #[non_exhaustive]` are done (§0.2, §1.3); ROUTE_TABLE
> reached 100% coverage with a CI test (§2.2); desktop `cargo-deny` and pervasive `--locked` are in
> CI (§0.1); the Prometheus `/metrics` endpoint is live (§2.3).
>
> **Still open (do NOT mark done):** root dependency dedup (82 `cargo deny` duplicate diagnostics,
> improved from the 94 baseline but still above target; `deny.toml` still sets
> `multiple-versions = "warn"`), the `clippy::unwrap_used`
> panic-prevention lint (still `allow`), expanding coverage beyond `--lib` (CI now enforces the
> measured 38% project floor plus 70% changed-line coverage), a signed desktop release, finishing the `[workspace.dependencies]` migration (the
> table exists and 27 of 28 crates use it, but `tokio`/`uuid`/`reqwest`/`rand` are not hoisted), and
> detached channel-submission/scheduler cleanup waiters (A9), and the "largest file < 800 lines"
> stretch target. See the annotated §4 metrics table.
>
> **Original framing (2026-06-29):** Grounded in a 10-dimension code audit
> (crate architecture, error/panic safety, async/concurrency, testing/CI, security, dependencies,
> type/API extensibility, observability, build/packaging, and a hard-metrics inventory).
> Every claim below was backed by a `file:line` or a counted occurrence in the audit.

## 0. Honest framing

"Flawless" is asymptotic, not a checkbox. ThinClaw already has **strong foundations** — this plan
is the credible path to *extreme robustness*, not a rewrite. The audit found a healthy core with a
finite, addressable set of real risks. Two audit findings were **over-flagged and are corrected here**:

- The "11 `#[allow(clippy::await_holding_lock)]` = critical deadlock" claim is **false** — all 11 are
  in `#[tokio::test]` code with intentional, documented env-lock serialization (verified at
  `shell.rs:994`, `config/secrets.rs:126`). Production has **zero** std-locks held across `.await`.
- The "~3,120 production panic sites" raw count is **misleading** — ~90% of `unwrap`/`expect` live in
  inline test modules; the agent-loop, dispatcher, and gateway hot paths are **clean**. The real panic
  exposure is ~10 specific sites plus the *absence of a prevention lint* (§2).

## 1. Current posture — what to preserve (do not regress)

| Area | Strength (evidence) |
|---|---|
| Crate graph | **No dependency cycles** across 29 workspace members; root-import invariant holds (zero `use thinclaw::` in crates); 28 extracted crates with a thin 145-line root `lib.rs`; explicit port/adapter seam (`src/agent/root_ports.rs`, 12 `Root*Port` adapter structs, 10 bundled into `RootAgentRuntimePorts`). |
| Error model | Comprehensive `thiserror` hierarchy (14 typed enums in `thinclaw-types/src/error.rs`); `anyhow` correctly scoped to CLI/tunnel only; dispatcher & gateway hot paths free of production `unwrap`. |
| Async | `BackgroundTasksHandle` tracks + aborts long-lived tasks; `spawn_blocking` used for git/fs; cooperative cancellation via `TurnCancellationRegistry`; lock discipline mostly correct. |
| Security | 5-stage shell pipeline; AES-GCM secrets with AAD binding + OS keychain; constant-time bearer auth; WASM caps default-off + HTTPS allowlist; SSRF post-DNS pinning on the HTTP tool; leak-detector scrubs SSE logs; Ed25519 plugin/scanner signing. |
| Testing/CI | 3,747 `#[test]` + 813 `#[tokio::test]` fns in `src/`+`crates/`; 7-profile feature matrix; `cargo-deny` with **zero advisory ignores**; 4 fuzz targets; multi-OS smoke; Postgres+libSQL contract tests; MSRV pinned 1.92. |
| Dep hygiene | `cargo-deny` bans git/unknown registries, yanked, wildcards; documented in-tree libsql patch; edge-dependency footprint guard. |

These are genuine differentiators. The plan **adds guardrails to keep them true**, not to rebuild them.

## 2. The hardening roadmap (phased, prioritized)

Severity uses the audit's P0–P3. Each phase ends with a CI guardrail so fixes **stay** fixed (§3).

---

### Phase 0 — Critical correctness & supply chain (target: ~2 weeks)

These are *silent-wrongness* bugs: things that pass CI today but are incorrect, insecure, or
un-shippable.

**0.1 Supply-chain coverage gaps (P0)** *(partially landed, 2026-07-10)*
- **[LANDED]** Desktop `cargo-deny` coverage: the desktop backend is now scanned by two invocations
  in CI (`cargo deny check licenses bans sources` at `ci.yml:210` and `cargo deny check advisories`
  at `ci.yml:213`). **[STILL OPEN]** The `channels-src/` and `tools-src/` sub-workspace lockfiles
  are still not swept by `cargo-deny`.
- **[LANDED]** `--locked` is now used pervasively across clippy/check/test/build and tool installs
  (e.g. `ci.yml:66`, `:74`, `:131`).
- `ort = "=2.0.0-rc.9"` with `download-binaries` fetches native ONNX libs from a third-party CDN at
  build time **without Cargo's hash verification** — a supply-chain injection point into every desktop
  release. → Vendor/pin+verify the artifact, or hash-check in the build script.
- `clawscan 1.0.0` drags `reqwest 0.11` (hyper 0.14) + `tokio-tungstenite 0.21` (CVE-era) into the
  desktop binary. → Replace/remove or upgrade.

**0.2 Security correctness gaps (P0)** *(the two flagged items below landed; `/tmp` exemption unverified)*
- **[RESOLVED BY DELETION]** The orphaned `DangerousToolTracker` (false security guarantee) was
  removed in the Wave-4 dead-code purge (`4f26f5f4`). `DangerousToolTracker` no longer exists in
  `src/` or `crates/`; enforcement lives in `ToolPolicyManager`.
- **[LANDED]** Trusted-proxy CIDR now uses `ipnet` membership. `trusted_proxy_ips` is a
  `Vec<IpNet>` and `parse_trusted_proxy_entry` parses a CIDR network
  (`crates/thinclaw-gateway/src/web/auth.rs`), so `10.0.0.0/8` trusts the whole subnet rather than a
  single host.
- **`/tmp` is exempted from path-escape detection** (`shell_security.rs:1311`) — the sandbox lets
  agent shell commands stage payloads in a world-readable shared dir. → Remove the exemption or gate
  it behind an opt-in setting (default off).

**0.3 Blocking-in-async (P0)**
- `std::net::TcpStream::connect_timeout` (1s) inside `async fn wait_for_ready`
  (`docker_chromium.rs:283`) steals a tokio worker per poll. → `tokio::time::timeout(tokio::net::…)`.
- Blocking `docker build`/`docker image inspect` (`setup/wizard/sandbox.rs:212-246`) and
  `brew install`/cloudflared (`setup/channels/tunnel.rs`) run synchronously in `async fn`s — a
  multi-minute build freezes the runtime/UI. → `tokio::process::Command` or `spawn_blocking`.

**0.4 Observability black-holes (P0)** *(all three landed, 2026-07-10)*
- **[LANDED]** Persistent log sink: `tracing-appender` is a dependency and a rolling daily file sink
  (`thinclaw.log`) writing to `state_paths().logs_dir` is wired for both the service and the desktop
  backend (`crates/thinclaw-gateway/src/web/log_layer.rs`).
- **[LANDED]** All 10 `ObserverEvent` variants now emit from production paths (the previously-dead
  `LlmRequest`, `ChannelMessage`, `HeartbeatTick`, `AgentEnd`, `Error` included), e.g. `HeartbeatTick`
  at `src/agent/commands.rs:319`, `LlmRequest` at `src/agent/dispatcher/llm_turn.rs:368`, `AgentEnd`
  and `Error` at `src/agent/agent_loop/mod.rs`. Zero dead variants.
- **[LANDED]** `/api/health` is a real readiness probe: it checks DB reachability (via
  `store.health_check()` under a 2s timeout), a configured LLM provider, and a wired inbound channel,
  returning 200 only when all three hold and 503 otherwise. Decision fn:
  `crates/thinclaw-gateway/src/web/status.rs`.

**0.5 Build/packaging shippability (P0)**
- **`bundled-wasm` uses `cargo build`, not the component pipeline** (`build.rs:125` vs the per-channel
  `cargo component build` + `wasm-tools component new`). Air-gapped installs may ship raw modules that
  **fail to load** in the component-model runtime — and CI never extracts+executes them. → Fix the
  build script + add a `--all-features` extraction-and-load smoke test.
- **Registry artifact URLs pinned to `v0.13.6` while the package is `v0.14.0`** — fresh installs fetch
  wrong-version WASM (checksums pass because they match the *old* artifacts). The checksum job builds
  the fix but never commits it. → Auto-PR updated manifests on release.
- **No signed Tauri release** — `tauri.conf.json` has `createUpdaterArtifacts:true` + an updater pubkey,
  but CI only does `tauri:build:cloud:unsigned`. macOS users hit Gatekeeper; the updater endpoint
  points at a `latest.json` that never exists. → Implement signing/notarization, or remove the
  updater config and document developer-only status.

---

### Phase 1 — Robustness hardening (target: ~4–6 weeks)

**1.1 Panic resilience (P1)** — the error *model* is excellent; close the specific holes + add a gate.
- Hoist per-call `Selector::parse().unwrap()` (`web_search.rs:29-33`) and `Regex::new().unwrap()`
  (`rig_lib/llama_provider.rs:49-67`) into `OnceLock` statics (correctness + perf).
- `unreachable!()` in the vision tool `execute()` (`thinclaw-tools/builtin/vision.rs:167`) →
  `Err(ToolError::ExecutionFailed)`; replace 40× `SystemTime…UNIX_EPOCH.unwrap()` in desktop commands
  with one `unix_millis_now()` helper; (`pairing/store.rs` `parent().expect()` — resolved: orphaned file removed in PR #197);
  parse the `SocketAddr` at config-validation time, not via startup `expect` (`main.rs:1023`).
- **Add `clippy::unwrap_used`/`expect_used` as workspace `warn` for non-test code** with explicit
  `#[allow]`+comment at infallible sites. This is the systemic fix — review alone won't scale at 538k LOC.

**1.2 Async lifecycle (P1)**
- Track the fire-and-forget per-message spawn (`channel_submission.rs:35`) and the 6 untracked
  `main.rs` spawns (experiment loops, SIGHUP, SSE bridge) in a `JoinSet`/shutdown list — today panics
  are silently dropped and tasks outlive shutdown holding DB/secret refs.
- Load-then-swap the skill-registry reload (`learning_tools.rs:606`) so the write lock isn't held
  across full disk discovery.

**1.3 Protocol & enum extensibility (P1)** — directly fixes the maintenance tax you already hit.
- **[LANDED]** `StatusUpdate` is now `#[non_exhaustive]`
  (`crates/thinclaw-channels-core/src/channel.rs:231`) and the gateway status matcher collapses
  future variants via a wildcard arm (`crates/thinclaw-gateway/src/web/status.rs`), so a new variant
  no longer force-edits sites across the crates.
- Fix the **WIT `StatusType` drift** (11 variants vs `StatusUpdate`'s 21) — 10 variants collapse lossily
  to `Status`, so WASM channels can't see lifecycle/subagent/credential events. Extend the WIT enum +
  bump the interface version.
- Standardize `UiEvent::Connected.protocol` (local emits `2`, remote proxy emits `1`).

**1.4 Dependency hygiene (P1)** *(partially landed; dedup still open)*
- **[LANDED]** A `[workspace.dependencies]` table exists and hoists 9 shared deps (serde, serde_json,
  anyhow, thiserror, tracing, chrono, async-trait, futures, utoipa) at `Cargo.toml:26`.
  **[STILL OPEN]** The full per-crate `workspace = true` migration is incomplete (tokio/uuid/reqwest/
  rand are intentionally not yet hoisted).
- **[STILL OPEN]** Dependency dedup improved but remains above target: the root `Cargo.lock` now
  produces **82** `cargo deny` duplicate diagnostics (baseline 94), including 3 `rand` versions
  (`0.8.6`, `0.9.4`, `0.10.2`) and 2 `wit-bindgen` versions. `deny.toml` still has
  `[bans] multiple-versions = "warn"`
  (not `deny`), `deny.toml:38`.
- Add Renovate/Dependabot (many lockfiles; manual hygiene isn't sustainable).

**1.5 Test & CI coverage (P1)**
- Add a coverage **threshold gate** (`--fail-under`) and drop `--lib` so integration paths count.
- Add an **MCP lifecycle integration test** (handshake/list/call/reconnect) — 32 unit tests for the
  primary external-tool surface is thin.
- Root-cause the Windows-smoke flake (remove the retry mask); un-quarantine or decompose the
  autonomous-campaign Docker E2E.

**1.6 Crate boundaries — the two wrong-direction edges (P1)** *(both LANDED and CI-guarded)*
- **[LANDED]** `thinclaw-db` no longer depends on `thinclaw-agent`; the `Routine` DTOs live in
  `thinclaw_types::routine`.
- **[LANDED]** `thinclaw-gateway` depends on `thinclaw-tools-core`, not `thinclaw-tools` (no
  transitive wasmtime/chromiumoxide/nostr pull).
- Both edges are enforced by the "Check crate boundaries" step in `.github/workflows/ci.yml`, which
  fails the build if either edge reappears.

---

### Phase 2 — Architecture & extensibility (target: ~6–8 weeks, incremental)

**2.1 God-file decomposition (P1–P2): LANDED.** As of 2026-07-11 **zero** committed `.rs` files
exceed 2,000 lines anywhere in the repo, and a live CI guard (`scripts/ci/check-file-sizes.sh`,
`MAX_LINES=2000`, run at `.github/workflows/ci.yml`) prevents regression. The largest file is now
`crates/thinclaw-channels/src/gmail.rs` at 1,999 lines. Every god-file the audit named was decomposed
into a directory module:
- `skill.rs` (4,577) and `skill_tools.rs` (4,385) no longer exist at those paths/sizes.
- `thinclaw-experiments/src/lib.rs` (was 3,482, *zero submodules*) is now a thin façade split into
  types/policy/cost/opportunities/support/messages submodules.
- `src/main.rs` (was 2,384) is now ~356 lines.
- `channels/web/server.rs` (was 2,392) is ~1,672; `signal.rs`, `gateway/web/providers.rs`,
  `routine_engine.rs`, `llm/reasoning.rs`, and `acp.rs` are all decomposed and under 2,000.

*Remaining stretch work:* the guard threshold is 2,000, not the "< 800 lines" target in §4; ~45 files
sit in the 1,500–1,999 band and could be split further if desired.

**2.2 Command-surface consistency (P2)**
- **[STILL OPEN]** 313 of 342 Tauri commands with a return type still return `Result<_, String>` and
  should migrate to `Result<T, BridgeError>` (the `From<String>` impl makes this mechanical); retire
  `local_unavailable()`.
- **[LANDED]** `ROUTE_TABLE` (`apps/desktop/backend/src/thinclaw/bridge.rs:116`) reached 100%
  coverage: 346 entries for 346 `#[tauri::command]` fns, enforced by the CI test
  `all_registered_commands_are_classified` (`bridge.rs:764`), which fails the build on any unclassified
  command.
- Replace stringly-typed `UiEvent` status/phase fields (`ToolUpdate.status`, `RunStatus.status`, …)
  with specta-exported enums so the TS side gets exhaustive unions.

**2.3 Metrics & operability (P2)**
- **[LANDED]** A Prometheus `/metrics` endpoint is registered at `src/channels/web/server.rs:875`,
  backed by the shared registry (`src/observability/prometheus.rs`, 22 series); it returns 200
  text/plain when `OBSERVABILITY_BACKEND=prometheus`, else 503. External alerting is now possible.
- Surface per-provider LLM `route_health` in `/api/status` + `thinclaw status`; warn on degraded score.

**2.4 LLM subsystem extraction (P2/P3)**
- The LLM layer is partially extracted; `src/llm/reasoning.rs` (now ~1,938 lines) and
  `src/llm/runtime_manager/` (now a directory of 11 modules) remain root-only. Continue the
  port-based extraction so crates needing reasoning policy don't route through root.

---

### Phase 3 — Maturity & long-tail (ongoing)

- Miri CI job for `thinclaw-secrets` (crypto) + `thinclaw-safety` (sanitizer/leak-detector).
- Plugin manifest version check `!=` → range (`>= MIN_SUPPORTED`); settings rename registry
  (`serde(alias)` + DB key-migration) documented in CLAUDE.md.
- `SAFE_BINS` rename + split (curl/wget/docker are not "safe"); `ExternalScanner` default
  `FailOpen → FailClosed`; in-memory secrets-store audit trail; `?token=` query-param log-exposure warning.
- Frontend test coverage for the chat hook + Tauri bridge (53 Vitest cases cover only utilities today).

Landed guardrail: `check-msrv-sync.py` now runs in CI and enforces that package MSRV and the pinned
developer/CI toolchain remain Rust 1.92.

---

## 3. Cross-cutting CI guardrails — "make it stay fixed"

The single highest-leverage investment. At 538k LOC, discipline must be **automated**, not reviewed.
Add to CI (most are S-effort, P0/P1):

| Guardrail | Prevents | Status (2026-07-11) |
|---|---|---|
| `--locked` on all cargo invocations | lockfile drift (already bit us twice) | Landed |
| `cargo-deny` across **all** workspaces/sub-lockfiles | unscanned advisories in desktop/channel/tool deps | Partial: desktop scanned; `channels-src/`/`tools-src/` sub-lockfiles still unswept |
| `clippy::await_holding_lock = deny` + `unwrap_used/expect_used = warn` (non-test) | async deadlocks; new production panics | Open: `clippy::unwrap_used` still `allow` (`Cargo.toml:466`) |
| `deny.toml multiple-versions = deny` (+ documented skips) | duplicate-version creep (82 `cargo deny` diagnostics in the root lock today) | Open: still `warn` (`deny.toml:38`) |
| God-file size guard (fail if any `.rs` > N lines) | re-growing the god-files | Landed: `scripts/ci/check-file-sizes.sh`, `MAX_LINES=2000` |
| ROUTE_TABLE coverage test (every command classified) | unclassified dual-mode commands | Landed: `all_registered_commands_are_classified` |
| `wit-bindgen` single-version check | WASM interface skew (2 versions today) | Open: 2 versions (`0.51.0`, `0.57.1`) |
| Bundle-reference resolution test | broken registry bundles (`slack-tool` today) | Unverified |
| Coverage threshold (`--fail-under`, no `--lib`) | silent coverage erosion | Partial: 38% project + 70% changed-line gates are live; CI still uses `--lib` (`ci.yml:892`) |
| `[workspace.dependencies]` enforced | per-crate version drift | Partial: table exists (9 deps); per-crate migration incomplete |

## 4. Metrics — baseline & targets

| Metric | Baseline (2026-06-29) | Target | Now (2026-07-11) |
|---|---|---|---|
| Dependency cycles | 0 | 0 (guarded) | 0, CI-guarded |
| Wrong-direction crate edges | 2 | 0 | 0, CI-guarded |
| Files > 2,000 lines | 18 | < 5, then 0 (guarded) | 0, guard live |
| Largest file | 4,577 | < 800 | 1,999 (target not yet met) |
| Duplicate-versioned crates (root lock) | 94 | < 30 (deny-gated) | 82 (improved; not gated) |
| ROUTE_TABLE command coverage | 15/341 (4%) | 100% (gated) | 346/346 (100%), gated |
| Commands returning `Result<_, String>` | ~149 | 0 | 313/342 (not met) |
| Dead `ObserverEvent` variants | 5/10 | 0 | 0 |
| Persistent log sink | none | rolling file + ring buffer | rolling daily file + ring buffer |
| `cargo-deny` workspace coverage | root only | all | root + desktop (sub-workspaces still unswept) |
| Signed desktop release | no | yes | no (not met) |
| StatusUpdate `#[non_exhaustive]` | no | yes | yes |

## 5. Sequencing

```
Phase 0 (correctness + supply chain + shippability)  ── do first; small, high-value, mostly independent
   └─ many are S-effort CI/config changes that immediately raise the floor
Phase 1 (panic/async/protocol/deps/coverage/boundaries) ── builds on Phase 0 guardrails
Phase 2 (god-files, command surface, metrics)        ── larger, incremental, parallelizable per-file
Phase 3 (maturity long-tail)                          ── ongoing, opportunistic
```

**Recommended first PRs (all P0, mostly small, land the guardrails early)** *(historical; these were
executed as part of the remediation stack. Current state noted inline)*
1. CI: `--locked` everywhere (landed) + `cargo-deny` desktop workspace (landed; sub-workspaces still
   open) + `clippy::await_holding_lock` / `unwrap_used=warn` (`unwrap_used` still `allow`).
2. Security: `DangerousToolTracker` deleted; trusted-proxy CIDR fixed via `ipnet` (both landed);
   `/tmp` exemption unverified.
3. Async: `docker_chromium` blocking connect + `spawn_blocking` wrappers (not verified here).
4. Observability: rolling file log sink + real `/api/health` + all 10 observer events emitting (all
   landed).
5. Build: `bundled-wasm` component pipeline + extraction smoke test; auto-PR registry checksums (not
   verified here).

Each first PR pairs a fix with its guardrail so the class of bug cannot silently return.

---

*This plan is grounded in the 2026-06-29 audit; re-run the audit workflow after Phase 0/1 to verify
the metrics targets and surface any new regressions.*
