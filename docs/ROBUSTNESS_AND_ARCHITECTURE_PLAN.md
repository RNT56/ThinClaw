# ThinClaw — Robustness & Architecture Hardening Plan

> **Status:** proposal v1 · **Created:** 2026-06-29 · Grounded in a 10-dimension code audit
> (crate architecture, error/panic safety, async/concurrency, testing/CI, security, dependencies,
> type/API extensibility, observability, build/packaging, and a hard-metrics inventory).
> Every claim below is backed by a `file:line` or a counted occurrence in the audit.

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
| Crate graph | **No dependency cycles** across 27 members; root-import invariant holds (zero `use thinclaw::` in crates); 26 extracted crates with a thin 145-line root `lib.rs`; explicit port/adapter seam (`src/agent/root_ports.rs`, 11 `Root*Port` adapters). |
| Error model | Comprehensive `thiserror` hierarchy (14 typed enums in `thinclaw-types/src/error.rs`); `anyhow` correctly scoped to CLI/tunnel only; dispatcher & gateway hot paths free of production `unwrap`. |
| Async | `BackgroundTasksHandle` tracks + aborts long-lived tasks; `spawn_blocking` used for git/fs; cooperative cancellation via `TurnCancellationRegistry`; lock discipline mostly correct. |
| Security | 5-stage shell pipeline; AES-GCM secrets with AAD binding + OS keychain; constant-time bearer auth; WASM caps default-off + HTTPS allowlist; SSRF post-DNS pinning on the HTTP tool; leak-detector scrubs SSE logs; Ed25519 plugin/scanner signing. |
| Testing/CI | 4,186 test fns; 7-profile feature matrix; `cargo-deny` with **zero advisory ignores**; 4 fuzz targets; multi-OS smoke; Postgres+libSQL contract tests; MSRV pinned 1.92. |
| Dep hygiene | `cargo-deny` bans git/unknown registries, yanked, wildcards; documented in-tree libsql patch; edge-dependency footprint guard. |

These are genuine differentiators. The plan **adds guardrails to keep them true**, not to rebuild them.

## 2. The hardening roadmap (phased, prioritized)

Severity uses the audit's P0–P3. Each phase ends with a CI guardrail so fixes **stay** fixed (§3).

---

### Phase 0 — Critical correctness & supply chain (target: ~2 weeks)

These are *silent-wrongness* bugs: things that pass CI today but are incorrect, insecure, or
un-shippable.

**0.1 Supply-chain coverage gaps (P0)**
- `cargo-deny` runs against the **root workspace only** — the desktop workspace + 28 sub-workspace
  lockfiles (channels/tools) get **zero** advisory/license/yanked scanning (`ci.yml:66`). → Add
  `cargo deny check --manifest-path apps/desktop/backend/Cargo.toml` and a per-sub-workspace sweep.
- Root CI omits `--locked` on every `cargo check/test/build` (only the desktop backend uses it — and
  that gap already caused a real CI break). → Add `--locked` everywhere.
- `ort = "=2.0.0-rc.9"` with `download-binaries` fetches native ONNX libs from a third-party CDN at
  build time **without Cargo's hash verification** — a supply-chain injection point into every desktop
  release. → Vendor/pin+verify the artifact, or hash-check in the build script.
- `clawscan 1.0.0` drags `reqwest 0.11` (hyper 0.14) + `tokio-tungstenite 0.21` (CVE-era) into the
  desktop binary. → Replace/remove or upgrade.

**0.2 Security correctness gaps (P0)**
- **`DangerousToolTracker` is orphaned** — `is_disabled()` has zero call sites outside its own tests;
  enforcement is entirely in `ToolPolicyManager`. Calling `tracker.disable(tool)` gives a **false
  security guarantee**. → Wire it into `prepare_tool_call()` or delete it; add an enforcement test.
- **Trusted-proxy CIDR silently degrades to a single IP** (`gateway/web/auth.rs:64` —
  `"10.0.0.0/8".split('/').next()` → `10.0.0.0`). Operators trusting a subnet trust one host. → Use
  `ipnet` membership; warn on parse failure; test /8,/16,/24.
- **`/tmp` is exempted from path-escape detection** (`shell_security.rs:1311`) — the sandbox lets
  agent shell commands stage payloads in a world-readable shared dir. → Remove the exemption or gate
  it behind an opt-in setting (default off).

**0.3 Blocking-in-async (P0)**
- `std::net::TcpStream::connect_timeout` (1s) inside `async fn wait_for_ready`
  (`docker_chromium.rs:283`) steals a tokio worker per poll. → `tokio::time::timeout(tokio::net::…)`.
- Blocking `docker build`/`docker image inspect` (`setup/wizard/sandbox.rs:212-246`) and
  `brew install`/cloudflared (`setup/channels/tunnel.rs`) run synchronously in `async fn`s — a
  multi-minute build freezes the runtime/UI. → `tokio::process::Command` or `spawn_blocking`.

**0.4 Observability black-holes (P0)**
- **No persistent log sink** — `tracing-appender` is absent; logs go only to stderr + a 500-entry
  in-memory ring buffer. Post-incident analysis on a non-service deployment is **impossible**. →
  Add a rolling file sink to `state_paths().logs_dir`.
- **5 of 10 `ObserverEvent` variants are dead** (`LlmRequest`, `ChannelMessage`, `HeartbeatTick`,
  `AgentEnd`, `Error` never emitted) and the observer defaults to `none` in every wizard profile. →
  Emit the missing 5; default to `log` (zero-cost when filtered).
- **`/api/health` is static** (`{status:"healthy"}` unconditionally) — supervisors route to broken
  instances. → Real readiness probe (DB ping, provider configured, return 503 on failure).

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
- Annotate `StatusUpdate` `#[non_exhaustive]` and add wildcard arms to the 2 hard-exhaustive matchers
  (`gateway/web/status.rs`, `tui.rs`) so a new variant no longer force-edits 5 sites across 3 crates.
- Fix the **WIT `StatusType` drift** (11 variants vs `StatusUpdate`'s 21) — 10 variants collapse lossily
  to `Status`, so WASM channels can't see lifecycle/subagent/credential events. Extend the WIT enum +
  bump the interface version.
- Standardize `UiEvent::Connected.protocol` (local emits `2`, remote proxy emits `1`).

**1.4 Dependency hygiene (P1)**
- Add a `[workspace.dependencies]` table; migrate the 22 crates to `workspace = true` (kills silent
  drift — e.g. `rand` is declared 6× independently). Collapse `rand 0.8→0.9` (3 simultaneous versions
  today). Flip `deny.toml` `multiple-versions = warn → deny` with documented skips.
- Add Renovate/Dependabot (35 lockfiles, ~1k packages each — manual hygiene isn't sustainable).

**1.5 Test & CI coverage (P1)**
- Add a coverage **threshold gate** (`--fail-under`) and drop `--lib` so integration paths count.
- Add an **MCP lifecycle integration test** (handshake/list/call/reconnect) — 32 unit tests for the
  primary external-tool surface is thin.
- Root-cause the Windows-smoke flake (remove the retry mask); un-quarantine or decompose the
  autonomous-campaign Docker E2E.

**1.6 Crate boundaries — the two wrong-direction edges (P1)**
- Move `Routine*` domain types `thinclaw-agent → thinclaw-types`; drop `thinclaw-db`'s dependency on
  `thinclaw-agent` (a persistence crate must not depend on the agent layer).
- Move MCP + execution DTOs `thinclaw-tools → thinclaw-tools-core`; drop `thinclaw-gateway`'s heavy
  `thinclaw-tools` dependency (which transitively pulls wasmtime/chromiumoxide/nostr).

---

### Phase 2 — Architecture & extensibility (target: ~6–8 weeks, incremental)

**2.1 God-file decomposition (P1–P2)** — 18 files exceed 2,000 lines; the project bans god-files.
Highest value first (effort in parens):
- `skill.rs` (4,577) + `skill_tools.rs` (4,385) twin files → per-tool + policy/scan submodules (L).
- `thinclaw-experiments/src/lib.rs` (3,482, *zero submodules* in an extracted crate) → types/policy/
  cost/campaign/lease (M).
- `src/main.rs` (2,384; `async_main` ≈1,934 lines) → `bootstrap.rs`/`surface_wiring.rs`/`signal_handling.rs`,
  main < 200 lines (M).
- Then: `channels/web/server.rs` (2,392) → per-port modules; `signal.rs` (2,918); `gateway/web/providers.rs`
  (2,827); `routine_engine.rs` (2,809); `llm/reasoning.rs` (2,553); `acp.rs` (3,150); `tui/mod.rs` (1,160).

**2.2 Command-surface consistency (P2)**
- Migrate the ~149 `Result<T, String>` Tauri commands to `Result<T, BridgeError>` (the `From<String>`
  impl makes this mechanical); retire `local_unavailable()`.
- Expand `ROUTE_TABLE` from 15/341 to full coverage + a CI test asserting every registered command is
  classified (the bridge linter already proves this pattern works).
- Replace stringly-typed `UiEvent` status/phase fields (`ToolUpdate.status`, `RunStatus.status`, …)
  with specta-exported enums so the TS side gets exhaustive unions.

**2.3 Metrics & operability (P2)**
- Feature-gated Prometheus `/metrics` endpoint backed by the existing `ObserverMetric` variants
  (latency p99, token burn, queue depth) → external alerting becomes possible.
- Surface per-provider LLM `route_health` in `/api/status` + `thinclaw status`; warn on degraded score.

**2.4 LLM subsystem extraction (P2/P3)**
- The LLM layer is partially extracted; `reasoning.rs` (2,553) + `runtime_manager` remain root-only.
  Continue the port-based extraction so crates needing reasoning policy don't route through root.

---

### Phase 3 — Maturity & long-tail (ongoing)

- Miri CI job for `thinclaw-secrets` (crypto) + `thinclaw-safety` (sanitizer/leak-detector).
- Plugin manifest version check `!=` → range (`>= MIN_SUPPORTED`); settings rename registry
  (`serde(alias)` + DB key-migration) documented in CLAUDE.md.
- `SAFE_BINS` rename + split (curl/wget/docker are not "safe"); `ExternalScanner` default
  `FailOpen → FailClosed`; in-memory secrets-store audit trail; `?token=` query-param log-exposure warning.
- Frontend test coverage for the chat hook + Tauri bridge (53 Vitest cases cover only utilities today).
- Dedicated MSRV-verification CI job (the stable pin coincidentally equals MSRV today).

---

## 3. Cross-cutting CI guardrails — "make it stay fixed"

The single highest-leverage investment. At 538k LOC, discipline must be **automated**, not reviewed.
Add to CI (most are S-effort, P0/P1):

| Guardrail | Prevents |
|---|---|
| `--locked` on all cargo invocations | lockfile drift (already bit us twice) |
| `cargo-deny` across **all** workspaces/sub-lockfiles | unscanned advisories in desktop/channel/tool deps |
| `clippy::await_holding_lock = deny` + `unwrap_used/expect_used = warn` (non-test) | async deadlocks; new production panics |
| `deny.toml multiple-versions = deny` (+ documented skips) | duplicate-version creep (94 today) |
| God-file size guard (fail if any `.rs` > N lines) | re-growing the 18 god-files |
| ROUTE_TABLE coverage test (every command classified) | unclassified dual-mode commands |
| `wit-bindgen` single-version check | WASM interface skew (2 versions today) |
| Bundle-reference resolution test | broken registry bundles (`slack-tool` today) |
| Coverage threshold (`--fail-under`, no `--lib`) | silent coverage erosion |
| `[workspace.dependencies]` enforced | per-crate version drift |

## 4. Metrics — baseline & targets

| Metric | Baseline | Target |
|---|---|---|
| Dependency cycles | 0 | 0 (guarded) |
| Wrong-direction crate edges | 2 | 0 |
| Files > 2,000 lines | 18 | < 5, then 0 (guarded) |
| Largest file | 4,577 | < 800 |
| Duplicate-versioned crates (root lock) | 94 | < 30 (deny-gated) |
| ROUTE_TABLE command coverage | 15/341 (4%) | 100% (gated) |
| Commands returning `Result<_, String>` | ~149 | 0 |
| Dead `ObserverEvent` variants | 5/10 | 0 |
| Persistent log sink | none | rolling file + ring buffer |
| `cargo-deny` workspace coverage | 1/2+28 | all |
| Signed desktop release | no | yes |
| StatusUpdate `#[non_exhaustive]` | no | yes |

## 5. Sequencing

```
Phase 0 (correctness + supply chain + shippability)  ── do first; small, high-value, mostly independent
   └─ many are S-effort CI/config changes that immediately raise the floor
Phase 1 (panic/async/protocol/deps/coverage/boundaries) ── builds on Phase 0 guardrails
Phase 2 (god-files, command surface, metrics)        ── larger, incremental, parallelizable per-file
Phase 3 (maturity long-tail)                          ── ongoing, opportunistic
```

**Recommended first PRs (all P0, mostly small, land the guardrails early):**
1. CI: `--locked` everywhere + `cargo-deny` all workspaces + `clippy::await_holding_lock=deny` + `unwrap_used=warn`.
2. Security: wire/delete `DangerousToolTracker`; fix trusted-proxy CIDR; remove `/tmp` exemption.
3. Async: fix the `docker_chromium` blocking connect + the `spawn_blocking` wrappers.
4. Observability: rolling file log sink + real `/api/health` + emit the 5 dead observer events.
5. Build: fix `bundled-wasm` component pipeline + extraction smoke test; auto-PR registry checksums.

Each first PR pairs a fix with its guardrail so the class of bug cannot silently return.

---

*This plan is grounded in the 2026-06-29 audit; re-run the audit workflow after Phase 0/1 to verify
the metrics targets and surface any new regressions.*
