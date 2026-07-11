# WS-05 — Self-Repair, Extensions & Native-Plugin Pipeline

> **✅ STATUS: DONE. Landed in commit `daf8f440` (self-repair + observability + native-plugin pipeline), merged to `main` via the audit-hardening stack (`1fb29984`, HEAD `bda7a61f`).**
> This plan is complete; do not execute it. It is retained as an implementation record. Every task (T1–T10) shipped. The "Current State (verified)" section below describes the *pre-remediation* state and no longer reflects the code. Self-repair `with_builder` is invoked (`src/agent/agent_loop/mod.rs:672`), `create_observer` is wired through `AppBuilder` (`src/app.rs:1717`), `ExtensionKind::NativePlugin` exists (`src/extensions/mod.rs:59`), the health monitor and native activation path are wired (`src/extensions/ext_health_monitor.rs`, `src/extensions/manager/native.rs`), and the orphaned `RepairTask` was erased (zero occurrences in code).

> **Status:** Done (landed) · **Priority:** P1 · **Risk:** med · **Effort:** L
> **Depends on:** none · **Blocks:** none (coordinates with WS-10 on `extensions/manager.rs` decomposition; WS-11 now defers the `RepairTask` orphan entirely to this WS — see Decision Point 4)
> **Owns (symbols/files):** `src/agent/self_repair.rs` (the `with_builder` adapter wiring), the self-repair construction block at `src/agent/agent_loop.rs:604-699`, the orphaned `RepairTask` at `crates/thinclaw-agent/src/self_repair.rs:325` (erase-or-consolidate decision), `src/extensions/native.rs`, `src/extensions/ext_health_monitor.rs`, the `ExtensionKind::NativePlugin` addition in `src/extensions/mod.rs:43`, the native-plugin dispatch arms in `src/extensions/manager.rs` (install/auth/activate/list/remove), the native-load call site inside `ExtensionRegistry::register_plugin_manifest_contributions` (`src/extensions/registry.rs:104`), and the observability factory call-through (`src/observability/mod.rs::create_observer` is consumed, not owned, by `src/app.rs`).

## Vision & Goal

ThinClaw's pitch is a self-hosted agent that *heals itself* and *extends itself* under operator control. Three capabilities are fully built but unreachable: the automatic tool-rebuild path of self-repair (the agent rewrites and recompiles its own broken WASM tools), the signature-verified native dynamic-library plugin pipeline (~1500L of `dlopen`-grade extensibility for operators who want native performance), and the pluggable observability backend the wizard already asks operators to choose. This workstream connects each one end-to-end so the runtime delivers the autonomy and extensibility the docs promise, with the unsafe native boundary gated behind explicit, defensible safety controls.

## Scope

**In scope:**
- Inject `LlmSoftwareBuilder` + `ToolRegistry` into `DefaultSelfRepair` via the already-implemented `.with_builder(...)` adapter so `repair_broken_tool` returns real `Success`/`Retry` instead of always `ManualRequired`.
- WIRE the native dynamic-library plugin pipeline: add `ExtensionKind::NativePlugin`, dispatch arms in `ExtensionManager`, a startup manifest-scan + `register_plugin_manifest_contributions` call, native-runtime activation backed by `NativePluginRuntime::load` (`src/extensions/native.rs`), and panic-isolated invocation through a health-monitored boundary (`src/extensions/ext_health_monitor.rs`).
- Wire `create_observer` through `AppBuilder` so the operator's wizard/config choice produces an actual `Observer`.

**Out of scope (and which WS owns it):**
- God-file decomposition of `src/extensions/manager.rs` (3343L) → **WS-10** (overhaul/god-files). This WS adds the *narrowest possible* arms and a new `manager/native.rs`-style submodule; it does not restructure the existing file.
- Agent-loop restructuring at large → **WS-11** (agent loop / self-repair consolidation). *(Note: deciding the fate of the orphaned `RepairTask` at `crates/thinclaw-agent/src/self_repair.rs:325` is now **in scope here** — WS-11 defers that symbol entirely to this WS. See Decision Point 4 and T10.)*
- Sandboxing posture for `execute_code`/shell used *inside* the builder loop → owned by the safety/sandbox workstream; this WS reuses whatever the builder already enforces.

## Current State (verified)

> **Historical (pre-remediation) snapshot.** Everything below has since been resolved by the landed WS-05 work. Kept for context on what the workstream fixed. Paths cited here predate the WS-10 decompositions: `src/agent/agent_loop.rs` is now `src/agent/agent_loop/mod.rs`, and `src/extensions/manager.rs` is now the `src/extensions/manager/` directory (`core.rs`, `native.rs`, `lifecycle.rs`, `mcp.rs`, etc.).

**Self-repair tool rebuild — half-wired (adapter built, call site never invokes it):**
- `DefaultSelfRepair` (crate) exposes `with_builder(builder, tools)` at `crates/thinclaw-agent/src/self_repair.rs:128-136`; when `builder` is `None`, `repair_broken_tool` short-circuits to `RepairResult::ManualRequired` (`crates/thinclaw-agent/src/self_repair.rs:228-233`).
- The root adapter is *fully implemented*: `src/agent/self_repair.rs:54-64` `with_builder(Arc<dyn SoftwareBuilder>, Arc<ToolRegistry>)`, plus `RootToolRepairBuilder` (builds a `BuildRequirement` for the broken tool, `src/agent/self_repair.rs:156-194`) and `RootToolRegistryProbe` (`src/agent/self_repair.rs:196-205`). It carries `#[allow(dead_code)]` at `src/agent/self_repair.rs:53` because nothing calls it.
- The agent-loop construction at `src/agent/agent_loop.rs:605-614` calls `DefaultSelfRepair::new(...)` then only `.with_store(...)` — never `.with_builder(...)`. The loop handles `RepairResult::Success` for tools (`src/agent/agent_loop.rs:673-679`) but can never reach it.
- All inputs are already on the `Agent`: `self.deps.llm` (`Arc<dyn LlmProvider>`, agent_loop.rs:71), `self.deps.safety` (`Arc<SafetyLayer>`, :75), `self.deps.tools` (`Arc<ToolRegistry>`, :76), `self.deps.cost_tracker` (:108). `LlmSoftwareBuilder::new(config, llm, safety, tools)` exists at `src/tools/builder/core.rs:80-98` with `.with_cost_tracker(...)` at :101.
- **Orphan note (owned HERE — see Decision Point 4 + T10):** `RepairTask` at `crates/thinclaw-agent/src/self_repair.rs:325-388` is a near-duplicate of the inline loop body in `agent_loop.rs:617-699` and is never constructed in non-test code (AUDIT-FINDINGS §6 lists it under ERASE). WS-11 defers this symbol entirely to this WS.

**Native dynamic-library plugin pipeline — fully built, zero runtime callers (dead):**
- `src/extensions/native.rs` (483L) is complete and well-defended: `NativePluginRuntime::load` (marked `unsafe`, :71) gates on `allow_native_plugins`, runs `validate_plugin_manifest`, `verify_plugin_manifest_signature` when `require_plugin_signatures`, checks `abi == CAbiJsonV1`, resolves the artifact path with `..`/absolute rejection (`resolve_plugin_artifact_path`, :194), enforces `native_plugin_allowlist_dirs` (`ensure_native_path_allowed`, :214), and verifies SHA-256 (`verify_sha256`, :232) — all *before* `libloading::Library::new`. `invoke_json` (:144) is a length-bounded C-ABI JSON round-trip. Eight unit tests cover every gate (`src/extensions/native.rs:248-483`).
- `NativePluginRuntime` is re-exported (`src/extensions/mod.rs:33`) but instantiated nowhere outside its own tests (verified: grep for `NativePluginRuntime::load` / `invoke_json` returns only native.rs + unrelated `HostMediatedToolInvoker::invoke_json`).
- `ExtensionKind` (`src/extensions/mod.rs:43-50`) has only `McpServer | WasmTool | WasmChannel`. `ExtensionManager` dispatches by that enum in `install` (`src/extensions/manager.rs:575`), `authenticate` (:640), `activate` (:659), `list` (:702/753/794), `remove` (:884), and `install_from_entry` (:1208). No native arm anywhere.
- `register_plugin_manifest_contributions` (`src/extensions/registry.rs:104-243`) *does* register WASM tool/channel contributions, but for native plugins it only emits `native_plugins_available` / `native_plugins_skipped` name lists (`:115-124, :240-241`) and never loads them. It is called **only from tests** (`src/extensions/registry.rs:981, 1044`) — there is no startup manifest-scan path in `src/` that invokes it.
- `manifest.rs` native types are complete: `NATIVE_PLUGIN_ABI_VERSION` (:15), `PluginManifest.contributions.native_plugins` (:50), `NativePluginContribution` (:91), `NativePluginAbi` (:106), `PluginArtifactKind::NativeDylib` (:138), and `validate_plugin_manifest` rejects native contributions unless `allow_native_plugins` (:188-190).
- `ExtensionsSettings` already carries the gates (`crates/thinclaw-settings/src/extensions.rs:18-37`): `allow_native_plugins` (default `false`), `require_plugin_signatures` (default `true`), `trusted_manifest_keys`, `native_plugin_allowlist_dirs`.
- `src/extensions/ext_health_monitor.rs` (`ExtensionHealthMonitor`, 293L) is a complete health state machine but `new()` is called **only in its own tests** (:189+) — dead.
- The gateway DTO `ExtensionKindHint` (`crates/thinclaw-gateway/src/web/extensions.rs:46-59`) and string constants `EXTENSION_KIND_*` (:11-13) also lack a native variant, so a wired native kind needs a matching string + parse arm.

**Observability backend — config + wizard collect the choice, factory never called (dead):**
- `create_observer(&ObservabilityConfig)` (`src/observability/mod.rs:42-47`) returns `LogObserver` for `"log"`, else `NoopObserver`. Called **only in tests** (`src/observability/mod.rs:60-101`).
- `Config.observability` is populated from `OBSERVABILITY_BACKEND` env / DB setting (`src/config/mod.rs:296-300`); the wizard's Step 18 `step_observability` writes `settings.observability_backend` (`src/setup/wizard/presentation.rs:169-184`), persisted at `src/setup/wizard/persistence.rs:172` and echoed in the summary (`src/setup/wizard/summary.rs:272-273`).
- No `Box<dyn Observer>` / `Arc<dyn Observer>` is constructed or stored anywhere in `src/app.rs`, `AgentDeps`, or the runtime (verified grep). The `Observer` trait (`src/observability/traits.rs:12`) has zero production call sites.

## Decision Points

1. **Native dynamic-library plugin pipeline — WIRE vs ERASE.** *(operator directive: realize the vision)*
   - **WIRE (recommended):** add `ExtensionKind::NativePlugin`, dispatch arms, a startup manifest-scan that calls `register_plugin_manifest_contributions`, and runtime activation via `NativePluginRuntime::load`, all behind the default-off `allow_native_plugins` gate. Trade-off: introduces a reachable `unsafe` `dlopen` path; mitigated because every safety gate (signature, ABI version, allowlist, SHA-256) is already implemented and tested in `native.rs`, and the feature stays disabled by default. Cost: ~1 new submodule + ~6 match arms + a scan-on-startup hook.
   - **ERASE alternative:** delete `src/extensions/native.rs`, `ext_health_monitor.rs`, the native branches of `register_plugin_manifest_contributions`, the `native_plugins` manifest fields, and the four `ExtensionsSettings` native gates (~1800L total). Trade-off: removes a genuinely complete, security-reviewed capability and the only non-WASM extension story; contradicts the realize-the-vision directive. Choose only if the operator explicitly declines native plugins.
   - **Recommendation: WIRE**, default-off, signature-required-by-default, with the explicit safety section in T4–T7. This is the highest-value item in the workstream and the code is essentially production-ready below the dispatch layer.

2. **Observability `create_observer` — WIRE vs remove config/wizard surface.**
   - **WIRE (recommended):** construct the observer in `AppBuilder` from `config.observability` and store it on the runtime/deps so events flow. Low risk, small surface, and it honors a choice the operator is *already asked to make* in onboarding.
   - **Remove:** strip Step 18, `observability_backend` from settings/persistence/summary, and `Config.observability`. Trade-off: deletes a working factory and a deliberate onboarding choice for no real saving; the factory is ~5L.
   - **Recommendation: WIRE.** Removing a surface the wizard already exposes is user-hostile and the wiring is trivial.

3. **Self-repair builder injection — there is no real WIRE-vs-ERASE here; it is pure wiring.** The adapter, ports, and result-handling arms all exist; only the constructor call is missing. **Recommendation: WIRE.** The orphaned `RepairTask` consolidation/deletion is a *separate* decision resolved in Decision Point 4 and T10 below.

4. **Orphaned `RepairTask` — CONSOLIDATE vs ERASE.** *(deletes-code → sign-off required)* WS-11 previously deferred this to WS-05, and WS-05 previously declined to touch it, leaving the symbol at risk of falling through with no owner. It is owned **here** now. The standalone `RepairTask` (`crates/thinclaw-agent/src/self_repair.rs:325-388`) is a near-duplicate of the inline loop body in `agent_loop.rs:617-699`. Since T1 wires the *canonical* inline self-repair loop in `src/agent` (via `DefaultSelfRepair::with_builder`), `RepairTask` is a drifted duplicate with no production caller.
   - **ERASE (recommended):** delete `RepairTask` (and any test-only constructors) as a drifted duplicate of the now-canonical inline loop. Matches AUDIT-FINDINGS §6 (lists it under ERASE). Trade-off: removes a code path; mitigated because it has zero non-test callers and the inline loop is the wired authority after T1.
   - **CONSOLIDATE alternative:** if reading shows `RepairTask` is the *better-factored* home (e.g. it cleanly encapsulates the loop the inline body open-codes), move the inline `agent_loop.rs:617-699` logic onto `RepairTask` and call it from the loop, rather than deleting it.
   - **Recommendation: ERASE** unless reading `RepairTask` shows it is the cleaner home, in which case consolidate. Either path is a **deletes-code → sign-off** decision (removing the inline body or removing `RepairTask`). See T10.

## Tasks

- [x] **T1: Inject `LlmSoftwareBuilder` into self-repair at construction**
  - **Files:** `src/agent/agent_loop.rs:605-614`; `src/agent/self_repair.rs:53` (remove the `#[allow(dead_code)]` on `with_builder`).
  - **Change:** After `repair = repair.with_store(...)` (agent_loop.rs:612), build a software builder from deps and inject it. Reuse the exact constructor pattern from `src/tools/registry.rs:1068-1072`:
    ```rust
    let builder = {
        use crate::tools::builder::{BuilderConfig, LlmSoftwareBuilder};
        let mut b = LlmSoftwareBuilder::new(
            BuilderConfig::default(),
            self.deps.llm.clone(),
            self.deps.safety.clone(),
            self.deps.tools.clone(),
        );
        if let Some(tracker) = self.deps.cost_tracker.clone() {
            b = b.with_cost_tracker(tracker);
        }
        Arc::new(b) as Arc<dyn crate::tools::SoftwareBuilder>
    };
    repair = repair.with_builder(builder, self.deps.tools.clone());
    ```
    Then drop the now-unnecessary `#[allow(dead_code)]` at `src/agent/self_repair.rs:53`.
  - **Acceptance:** `repair_broken_tool` can return `Success`/`Retry`; the `RepairResult::Success` arm at `agent_loop.rs:673` becomes reachable. `with_builder` no longer needs `#[allow(dead_code)]`. Clippy reports no `dead_code` for the removed allow.
  - **Effort:** S
  - **Verification:** `cargo clippy -p thinclaw --all-targets -- -D warnings`; add a unit test in `src/agent/self_repair.rs` `tests` that constructs `DefaultSelfRepair` with a stub `SoftwareBuilder` (returning a successful `BuildResult`) and asserts `repair_broken_tool` yields `RepairResult::Success`. `cargo test -p thinclaw self_repair`.

- [x] **T2: Guard rebuild cost/loop and document the new autonomy**
  - **Files:** `src/agent/agent_loop.rs:668-697` (broken-tool repair branch); `docs/EXTENSION_SYSTEM.md` or `src/tools/README.md` (self-repair note).
  - **Change:** Confirm the repair branch already increments attempts (it does, via `repair_broken_tool` → `store.increment_repair_attempts`, crate `self_repair.rs:256`) and that `max_repair_attempts` (config) caps the loop — no infinite rebuild. Add a one-paragraph doc note that automatic tool rebuild is now active and bounded by `max_repair_attempts` + `repair_check_interval`. If `cost_tracker` is absent, the rebuild still runs but is unmetered — note this.
  - **Acceptance:** No new unbounded loop; doc reflects that broken WASM tools are auto-rebuilt. `FEATURE_PARITY.md` self-repair row updated if one exists.
  - **Effort:** S
  - **Verification:** Re-read `crate self_repair.rs:241-247` (attempt cap) to confirm; `cargo test -p thinclaw-agent self_repair`.

- [x] **T3: Add `ExtensionKind::NativePlugin` and its serialization/DTO siblings**
  - **Files:** `src/extensions/mod.rs:43-60` (enum + `Display`); `crates/thinclaw-gateway/src/web/extensions.rs:11-59` (`EXTENSION_KIND_NATIVE_PLUGIN` const, `ExtensionKindHint::NativePlugin`, `parse_extension_kind_hint` arm); `src/tools/builtin/extension_tools.rs:13-17` (`tool_kind_to_root_kind` if a `ToolExtensionKind::NativePlugin` is added — otherwise leave the tool-facing enum unchanged and surface native only via the manager/gateway).
  - **Change:** Add `NativePlugin` to `ExtensionKind` with `#[serde(rename = "native_plugin")]` semantics matching the existing `snake_case` derive; add `Display` arm `=> write!(f, "native_plugin")`; add the gateway string const + hint variant + parse arm. Keep the agent-facing `ToolExtensionKind` (in `thinclaw-tools`) unchanged unless native install via the LLM tool is desired — default to **not** exposing native install to the model (operator-only).
  - **Acceptance:** Workspace compiles with the new variant; all existing `match kind` sites get an arm (see T5). Round-trip `serde` test: `ExtensionKind::NativePlugin` ⇄ `"native_plugin"`.
  - **Status:** Core `ExtensionKind::NativePlugin` variant, `"native_plugin"` `Display`/serde, and the round-trip test landed (`src/extensions/mod.rs`). The gateway DTO sibling (`EXTENSION_KIND_NATIVE_PLUGIN` const, `ExtensionKindHint::NativePlugin` variant, `parse_extension_kind_hint` arm in `crates/thinclaw-gateway/src/web/extensions.rs`) was **not** implemented — that enum still lists only `McpServer`/`WasmTool`/`WasmChannel`. Left outstanding because native install is operator-only and not surfaced through the gateway hint path.
  - **Effort:** S
  - **Verification:** `cargo build -p thinclaw -p thinclaw-gateway`; add a `serde_json` round-trip test next to `src/extensions/mod.rs` tests.

- [x] **T4: Native-plugin runtime registry + activation submodule (panic isolation, ABI/version checks)**
  - **Files:** new `src/extensions/manager/native.rs` (or a focused `src/extensions/native_activation.rs` if avoiding a `manager/` dir to not collide with WS-10) — **do not** grow `manager.rs`; `src/extensions/manager.rs:134-218` (add an `RwLock<HashMap<String, Arc<NativePluginRuntime>>>` field `native_plugins` to `ExtensionManager` + init in `new`).
  - **Change:** Implement `activate_native_plugin(&self, name)` that: looks up the manifest + contribution + plugin root, then calls `unsafe { NativePluginRuntime::load(manifest, contribution, plugin_root, &settings) }` (the safety gates run inside `load`). Wrap each `invoke_json` call site in `std::panic::catch_unwind` (the C-ABI boundary can abort/panic) and treat a caught panic as a failed health check, never a host crash. Store the loaded runtime in the new `native_plugins` map keyed by contribution id. ABI version is enforced inside `load` (`native.rs:91` checks `abi`, and the manifest validator checks `abi_version` at `manifest.rs:284`) — assert the call path passes `abi_version` through.
  - **Acceptance:** A signed, allowlisted, hash-matching native manifest activates and `invoke_json` round-trips; an unsigned manifest with `require_plugin_signatures=true` fails activation *before* `libloading`. A plugin that panics in `invoke_json` marks the plugin unhealthy and is contained.
  - **Effort:** L
  - **Verification:** Extend the `native.rs` C-ABI smoke test (`src/extensions/native.rs:412-482`) into a manager-level test that goes through `activate_native_plugin`. `cargo test -p thinclaw native_plugin`. Requires a C compiler (`cc`) on the test host — skip gracefully if absent, mirroring the existing smoke test.

- [x] **T5: Wire native dispatch arms in `ExtensionManager`**
  - **Files:** `src/extensions/manager.rs` install (`:575`), authenticate (`:640`), activate (`:659`), list (`:702/753/794`), remove (`:884`), install_from_entry (`:1208`), and the `kind_label` helpers (`:1224, :1764`).
  - **Change:** Add `ExtensionKind::NativePlugin => ...` arms. `activate` delegates to `activate_native_plugin` (T4). `authenticate` returns `auth_result(name, NativePlugin, "none", "no_auth_required")` (native plugins authenticate via signature, not OAuth/token). `install` for native is operator-side file placement into an allowlisted dir — return a clear "native plugins are installed by placing a signed manifest in an allowlisted directory" message rather than a download path (do **not** add a network download for native binaries). `list`/`remove`/`kind_label` get descriptive arms. Keep each arm minimal — this is dispatch, not new behavior (defer structural cleanup to WS-10).
  - **Acceptance:** Every `match kind` in `manager.rs` is exhaustive again; no `_ =>` catch-alls hide native. `tool activate` / gateway activate of a native plugin reaches `activate_native_plugin`.
  - **Effort:** M
  - **Verification:** `cargo build -p thinclaw`; `cargo clippy -p thinclaw --all-targets -- -D warnings` (catches non-exhaustive matches as errors only if `#[non_exhaustive]`; otherwise compiler errors on missing arms confirm coverage).

- [x] **T6: Startup manifest scan → `register_plugin_manifest_contributions` (the missing entrypoint)**
  - **Files:** `src/app.rs` (extension-manager assembly) calling a new `ExtensionManager::scan_and_register_plugin_manifests(&self, plugins_dir, &settings)`; the call delegates to `ExtensionRegistry::register_plugin_manifest_contributions` (`src/extensions/registry.rs:104`); extend that function so, when `settings.allow_native_plugins` and a native contribution is present, it eagerly invokes the T4 activation (or records it for lazy activation) rather than only listing `native_plugins_available`.
  - **Change:** Add a directory scan over the configured plugin dir(s) (reuse `wasm_tools_dir`/`wasm_channels_dir` siblings or a new `native_plugin_allowlist_dirs` walk), parse each `PluginManifest` (JSON), and call `register_plugin_manifest_contributions`. Gate native activation strictly behind `allow_native_plugins`. This is the single missing link that makes the entire pipeline reachable.
  - **Acceptance:** With `allow_native_plugins=false` (default), native contributions are skipped (existing `native_plugins_skipped` behavior) and nothing loads. With it enabled + a valid signed manifest in an allowlisted dir, the plugin is registered and activatable.
  - **Effort:** M
  - **Verification:** Integration test placing a manifest fixture in a temp dir, constructing an `ExtensionManager` with native settings on/off, asserting registration vs skip. `cargo test -p thinclaw plugin_manifest`.

- [x] **T7: Wire `ExtensionHealthMonitor` to the native (and optionally WASM) activation path**
  - **Files:** `src/extensions/ext_health_monitor.rs`; the native activation submodule from T4.
  - **Change:** Instantiate one `ExtensionHealthMonitor` (default config) owned by `ExtensionManager`; `register(name)` on activation; `record_success`/`record_failure` around each `invoke_json` (including caught panics → failure). Expose `summary()` for the existing extension status surface. This removes the dead-code status of the monitor and gives the unsafe native boundary observability.
  - **Acceptance:** Activating a native plugin registers it as `Unknown`; a successful invoke → `Healthy` after `recovery_threshold`; repeated failures/panics → `Unhealthy`. Monitor is no longer constructed only in tests.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw ext_health` plus the T4 panic-isolation test asserting an unhealthy transition.

- [x] **T8: Write the native-plugin SAFETY section in docs**
  - **Files:** `docs/EXTENSION_SYSTEM.md`; `src/NETWORK_SECURITY.md` (trust-boundary note); `FEATURE_PARITY.md` (native-plugin row).
  - **Change:** Document the unsafe boundary explicitly: (a) signature verification (`require_plugin_signatures` default true, `trusted_manifest_keys`) runs before any `dlopen`; (b) ABI versioning (`NATIVE_PLUGIN_ABI_VERSION`, `CAbiJsonV1`) gates load; (c) allowlist + SHA-256 pin the artifact; (d) panic isolation via `catch_unwind` at every call; (e) default-off (`allow_native_plugins=false`); (f) native plugins run **in-process with full host privileges** — operators must only trust signed, audited binaries. State that native plugins are NOT sandboxed like WASM tools (this is the deliberate trade for native performance) and align with CLAUDE.md's "MCP servers are operator-trusted" framing.
  - **Acceptance:** Docs state every gate and the in-process-privilege caveat; no doc claims native plugins are sandboxed.
  - **Effort:** S
  - **Verification:** Doc review against `src/extensions/native.rs` gates; cross-check `CRATE_OWNERSHIP.md`/CLAUDE.md repo-shape notes still accurate (extensions stay root-owned).

- [x] **T9: Wire `create_observer` through `AppBuilder`**
  - **Files:** `src/app.rs` (`AppBuilder` build flow, near where `Config` is consumed); a storage slot for `Arc<dyn Observer>` (new field on the runtime/app or `AgentDeps` if events should flow into the agent loop).
  - **Change:** Call `let observer = crate::observability::create_observer(&config.observability);` during build, store it as `Arc<dyn Observer>` (change `create_observer` return type to `Arc<dyn Observer>` *only if* a shared owner is needed — otherwise keep `Box` and wrap). Hold it on the app/runtime so it is at minimum constructed and reachable; if event emission sites are out of scope, at least record one startup `ObserverEvent` so the `"log"` backend is observably active.
  - **Acceptance:** With `OBSERVABILITY_BACKEND=log`, startup emits at least one structured event via `LogObserver`; with `none`, zero overhead. `create_observer` has a production caller; the wizard choice has an effect.
  - **Effort:** S
  - **Verification:** `RUST_LOG=thinclaw=debug OBSERVABILITY_BACKEND=log cargo run` shows the log-observer event; `cargo test -p thinclaw observability`. Confirm `cargo build --no-default-features --features edge` still green (observer must not pull desktop-only deps).

- [x] **T10: Resolve the orphaned `RepairTask` (ERASE-or-consolidate) — deletes-code → sign-off**
  - **Files:** `crates/thinclaw-agent/src/self_repair.rs:325-388` (`RepairTask` + any test-only constructors); cross-check the inline loop body at `src/agent/agent_loop.rs:617-699`.
  - **Change:** Implements Decision Point 4 after T1 lands (T1 makes the inline loop the wired authority). First **read** `RepairTask` to confirm it is a drifted duplicate with no non-test caller. **ERASE (recommended):** delete `RepairTask` and its test-only constructors; ensure no `pub use` re-export dangles. **CONSOLIDATE (only if reading shows `RepairTask` is the better-factored home):** move the inline `agent_loop.rs:617-699` logic onto `RepairTask` and call it from the loop. Do not leave both the inline loop and `RepairTask` present. Because this removes a code path either way, it requires **sign-off** before deletion.
  - **Acceptance:** Exactly one self-repair loop home remains; no `#[allow(dead_code)]`/orphan-symbol warning for `RepairTask`; grep confirms no surviving non-test references to a deleted symbol. AUDIT-FINDINGS §6 `RepairTask` item is resolved.
  - **Effort:** S
  - **Verification:** `cargo clippy -p thinclaw-agent --all-targets -- -D warnings` (no dead_code); `cargo test -p thinclaw-agent self_repair`; grep `RepairTask` across the workspace returns only intended references (none if erased). Depends on T1.

## Best Practices (workstream-specific)

- **Ports/adapters discipline:** self-repair already follows it perfectly — the crate (`thinclaw-agent`) owns the trait + `with_builder` port; the root (`src/agent/self_repair.rs`) owns the `RootToolRepairBuilder`/`RootToolRegistryProbe` adapters. Copy this exact shape for any new native-plugin trait you extract; do not put root concretes in the crate.
- **Construct builders via the canonical pattern:** `src/tools/registry.rs:1068-1072` is the reference `LlmSoftwareBuilder` assembly (arg order `config, llm, safety, tools`, then `.with_cost_tracker`). Reuse it verbatim in T1 — do not hand-roll a divergent constructor.
- **Exhaustive `match kind` over catch-alls:** the existing manager dispatch (`manager.rs:575/640/659`) uses explicit arms per kind. Add explicit `NativePlugin` arms so the compiler enforces coverage; never collapse to `_ =>`.
- **Keep the unsafe surface tiny and gated:** all validation already lives *inside* `NativePluginRuntime::load` before `libloading::Library::new` (native.rs:71-131). Do not duplicate or weaken those checks in the manager — call `load` and trust its gates; add only `catch_unwind` around `invoke_json`.
- **Don't grow god-files:** `manager.rs` is 3343L and is WS-10's overhaul target. Put native activation in a new focused submodule and have `manager.rs` delegate (façade pattern per CLAUDE.md). Avoid a `misc`/`util` bucket — name it `native` after the domain.
- **Build-profile awareness:** observability and extensions live in the root package; verify the wiring compiles under `edge` (`--no-default-features --features edge`) since CI checks it. `libloading` is a native-only dep — confirm `native.rs` activation does not break `edge`/`light` profiles (gate behind a feature if `libloading` is not already in those profiles).

## Common Pitfalls

- **Editing only the crate or only the root, not both halves of a port.** Self-repair is a two-file dance (`crates/thinclaw-agent/src/self_repair.rs` trait/port + `src/agent/self_repair.rs` adapter). T1's change is in the *root* call site (`agent_loop.rs`) and the root adapter — the crate already exposes everything. Do not reimplement `with_builder` in the crate.
- **Resolving `RepairTask` before T1 lands.** It is owned **here** now (T10), not WS-11. But do T10 *after* T1, because T1 establishes the inline loop as the wired authority that makes `RepairTask` the duplicate; resolving it first risks deleting the wrong home. It is a deletes-code → sign-off decision — do not erase without sign-off.
- **Adding a native enum variant but missing a `match` arm.** `ExtensionKind` is matched in *at least* 8 places in `manager.rs` plus the gateway DTO. The audit's recurring failure mode (split_message fix landing in only one of four copies) applies here — grep every `match.*kind` and `ExtensionKind::` site before declaring T5 done.
- **Wiring observability behind a desktop-only dependency.** `create_observer` must work in `edge`. The audit flagged the desktop profile omitting `wasm-runtime`; don't introduce an analogous profile gap.
- **Letting the native rebuild/repair loop run unbounded.** Self-repair already caps via `max_repair_attempts` (crate self_repair.rs:241); a native plugin that panics every invoke must transition to `Unhealthy` and stop being re-invoked, not retry forever.
- **Assuming `register_plugin_manifest_contributions` is wired because it has callers.** Its only non-test callers are tests (registry.rs:981/1044). T6 (the startup scan) is the load-bearing wiring — without it, T3–T7 are still unreachable.
- **Forgetting the `dlopen` privilege caveat in docs.** Native plugins are NOT WASM-sandboxed. Shipping them without the explicit T8 safety section would be the exact "weaker-than-documented" trap the audit called out for the sandbox proxy.

## Multi-Worker Execution Plan (ultracode)

- **Worker decomposition:**
  - **Worker A (sequential, small, low-risk):** T1 + T2 (self-repair injection), T10 (resolve `RepairTask`, *after* T1), and T9 (observability). These touch `agent_loop.rs`, `self_repair.rs` (root + crate), `app.rs`, `observability/mod.rs` — disjoint from the extension files. Can land first and independently. T10 is a deletes-code → sign-off step gated behind T1.
  - **Worker B (the native pipeline, mostly sequential):** T3 → T4 → T5 → T6 → T7, in order, because each depends on the previous (enum before dispatch, runtime before activation, activation before scan, scan before health wiring). T8 (docs) can run in parallel with B once T4's gates are confirmed.
  - Workers A and B are file-disjoint and can run as **parallel fan-out**; B is internally sequential.
- **Isolation:** Use **git worktree isolation** — Worker A and Worker B mutate different files, but both eventually rebuild the root package; run each in its own worktree to avoid stepping on `Cargo.lock`/incremental artifacts, then merge A first (smaller, lower risk), then B.
- **Workflow shape:**
  1. **Implement** — fan out A and B into two worktrees. Within B, a single worker executes T3→T7 sequentially; spawn a third worker for T8 docs after T4.
  2. **Verify** — each worker runs the per-task verification gate below in its worktree.
  3. **Review** — `/code-review` on each worktree diff; pay special attention to the `unsafe`/`catch_unwind` boundary in T4 and exhaustive-match coverage in T5.
  4. **Fix** — address review findings in-worktree, re-run the gate.
  5. **Integrate** — merge A → main, then B → main, re-running the full gate after each merge to catch enum-variant fallout across crates.
- **Verification gate (exact commands):**
  - `cargo fmt --all -- --check`
  - `cargo clippy -p thinclaw -p thinclaw-agent -p thinclaw-gateway --all-targets --all-features -- -D warnings` (note: the audit found CI omits `--all-targets`; include it here)
  - `cargo test -p thinclaw-agent self_repair`
  - `cargo test -p thinclaw native_plugin plugin_manifest ext_health observability self_repair`
  - `cargo build --no-default-features --features edge` (profile guard for observability + `libloading` gating)
  - `cargo build` (default/desktop profile)
  - `/ship` (full Rust quality gate) before each merge; `/code-review` high effort on the T4–T7 native diff.
  - **DB/Docker prerequisites:** none for these tasks (no migrations, no Postgres). The native C-ABI smoke test needs `cc` on PATH (skips gracefully if absent). No Docker required.

## Definition of Done

- [x] T1: `DefaultSelfRepair::with_builder` is invoked at `agent_loop.rs:605`; `#[allow(dead_code)]` removed from `src/agent/self_repair.rs:53`; a test proves `repair_broken_tool` returns `Success` with a builder injected.
- [x] T2: broken-tool auto-rebuild is bounded by `max_repair_attempts`; self-repair docs updated.
- [x] T10: orphaned `RepairTask` resolved (erased as drifted duplicate, or consolidated as the loop home) with sign-off; exactly one self-repair loop home remains; AUDIT-FINDINGS §6 item closed.
- [x] T3: core `ExtensionKind::NativePlugin` variant (`src/extensions/mod.rs`, 4th variant) with `"native_plugin"` serialization and a serde round-trip test exists. NOTE: the gateway DTO sibling (`ExtensionKindHint::NativePlugin` + `EXTENSION_KIND_NATIVE_PLUGIN` const + `parse_extension_kind_hint` arm in `crates/thinclaw-gateway/src/web/extensions.rs`) was **not** added — that file still exposes only `McpServer`/`WasmTool`/`WasmChannel`. Native plugins are operator-side (not model/gateway-exposed by default), so the gateway hint sibling remains outstanding.
- [x] T4–T7: native plugins can be scanned, registered (gated by `allow_native_plugins`), activated via `NativePluginRuntime::load`, invoked with panic isolation, and health-tracked; `ExtensionHealthMonitor` and `NativePluginRuntime` have production callers; all `match kind` sites exhaustive.
- [x] T8: native-plugin SAFETY section documents signature/ABI/allowlist/SHA-256/panic-isolation/default-off and the in-process-privilege (non-sandboxed) caveat.
- [x] T9: `create_observer` has a production caller in `AppBuilder`; `OBSERVABILITY_BACKEND=log` produces observable events; `edge` build stays green.
- [x] Decision points 1 (WIRE native), 2 (WIRE observability), and 4 (`RepairTask` ERASE-or-consolidate, signed off) resolved and reflected in code + docs.
- [x] Full verification gate green (fmt, clippy `--all-targets -D warnings`, targeted tests, `edge` + default builds); `/ship` passes.
- [x] `docs/EXTENSION_SYSTEM.md`, `src/NETWORK_SECURITY.md`, `FEATURE_PARITY.md` updated; no overlap with WS-10 (`manager.rs` decomposition) or WS-11 (broader agent-loop restructure) — dependencies noted, not actioned. The `RepairTask` symbol is resolved here (T10), not deferred to WS-11.
