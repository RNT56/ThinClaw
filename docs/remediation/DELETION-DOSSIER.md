# ThinClaw — Deletion & WIRE Dossier
> One honest entry per candidate so the operator can decide keep / wire / erase. Date 2026-06-25.

> **✅ RESOLVED (decisions applied).** This dossier's recommendations have been executed. The **10
> ERASE** candidates were deleted in the Wave 4 deletion batch (commit `4f26f5f4`, 2026-06-25): the
> `src/safety/*` orphans, InferenceRouter chat modality, `SmartRoutingProvider`, `RepairTask`, the
> heartbeat runner (`spawn_heartbeat`/`HeartbeatRunner::run`), the dead `Reasoning.safety` /
> `SpawnSubagentTool.executor` fields, the CLI stubs (`nodes`/`subagent_spawn`/`session_export`),
> `self_message`, `qr_pairing`, `tailscale`, and the misc helper group + HTTPS cred mappings. The
> **1 WIRE** candidate (`src/history/store/`) was consolidated onto `thinclaw-db` (commit `43460933`).
> The **1 DECIDE** candidate (`voice_wake`) was **WIRED, not erased** (operator chose "build the wake
> word"): `VoiceWakeRuntime::new` is now constructed in `AppBuilder` (`src/app.rs`) and its events are
> consumed in `src/async_main.rs`, behind the opt-in `voice` feature. The per-candidate "current state
> / compiles-unused / zero-callers" snapshots below are therefore **historical**: they describe the
> pre-deletion tree of 2026-06-25, not the current code. See `DEFERRED-DELETIONS.md` and
> `EXECUTION-SUMMARY.md`.

## Summary table

| Candidate | ~Lines | Current state | Alternative exists? | Recommendation | Confidence | One-line |
|---|---|---|---|---|---|---|
| `src/safety/*` orphan siblings (14 .rs) | 4931 | uncompiled-orphan | Yes — `thinclaw-safety` crate (live, canonical) | **ERASE** | high | 14 uncompiled pre-extraction copies of the live crate; mod.rs is a pure re-export façade; 5 drifted copies are the older/inferior side. |
| InferenceRouter chat modality (D-06) | ~350 | built-but-unreachable | Yes — `chat.rs::resolve_provider` (live Tauri path) | **ERASE** | high | Router chat backend is write-only dead state (zero inference callers, `LocalChatBackend` never built). |
| `SmartRoutingProvider` decorator (D-04) | ~290 | dead-zero-callers | Yes — `RoutePlanner` + `cascade.rs` (live hot path) | **ERASE** | high | Original cost-routing decorator superseded by RoutePlanner in Wave 2; zero `::new` callers. |
| `RepairTask` (D-03) | ~65 | dead-zero-callers | Yes — inline loop in `agent_loop.rs` (superset) | **ERASE** | high | Zero-caller log-only duplicate of the channel-aware inline self-repair loop. |
| Heartbeat runner (`run`/`spawn_heartbeat`) | ~250 | dead-zero-callers | Yes — routine-engine `execute_heartbeat` (superset) | **ERASE** | high | Abandoned fork-era heartbeat loop with zero callers; routine engine is a strict superset. |
| Dead fields: `Reasoning.safety` + `SpawnSubagentTool.executor` (D-05) | ~40 (+28-caller ripple) | compiles-unused | Yes — `SafetyLayer::sanitize_tool_output` + dispatcher interception | **ERASE** | high | Both fields vestigial-from-birth (never read in any commit); sanitization & spawning live elsewhere. |
| Dead CLI stubs: `nodes.rs`, `subagent_spawn.rs`, `session_export.rs` | ~1014 | compiles-unused | Yes (2 of 3) — `sessions.rs` export, `spawn_subagent` tool | **ERASE** | high | Three Sprint-era CLI stubs declared `pub mod` but never given a Command variant or dispatch arm. |
| `self_message` anti-loop module | ~250 | compiles-unused | Yes — per-transport inline filters + owner allowlists | **ERASE** | high | Dead centralized anti-loop module; real self-message protection is live & redundant per transport. |
| `qr_pairing` (`src/qr_pairing.rs`) | ~329 | compiles-unused | Yes — `PairingStore` (live, persistent) | **ERASE** | high | Never-wired fork-era QR-pairing scaffold (zero callers, unbuilt TLS premise, 2 security defects). |
| `src/tailscale.rs` `TailscaleDiscovery` | ~230 | dead-zero-callers | Live need met by unrelated `TailscaleTunnel`/deploy | **ERASE** | high | Orphaned IronClaw-era thin-client tailnet-discovery module (zero callers). |
| Misc dead helpers + 3 HTTPS cred mappings (group) | ~250 | drifted-duplicate | Yes — worker credentials endpoint, `subscribe_raw`, `build_all_wasm_extensions` | **ERASE** | high | Group ERASE: HTTPS cred mappings are no-ops; 3 `#[allow(dead_code)]` helpers + test-only `subscribe()`. |
| `src/history/store/` (root Postgres Store) | ~? (18 files) | drifted-duplicate | Yes — `thinclaw-db/postgres_store` (canonical, live via PgBackend) | **WIRE** | high | Stalled crate-extraction duplicate; still compiled & called by 6 concrete sites (3 CLI, 3 setup wizard). |
| `voice_wake` module + `voice` feature + `cpal` dep | ~749 | compiles-unused | Yes — frontend `use-voice-wake.ts` on the only mic surface | **DECIDE** | high | Headless wake-word scaffold w/ real cpal/Sherpa plumbing but zero callers and no enabling build profile. |

## Recommendation tally

- **ERASE: 10** candidates
- **WIRE: 1** candidate (`src/history/store/`)
- **DECIDE: 1** candidate (`voice_wake`)
- (No KEEP-recommended candidates.)

**Lines removed if all ERASE accepted:** roughly **8,000+ lines** of dead/unreachable code. The single largest win is the `src/safety/*` orphan siblings at **4,931 lines**, followed by the CLI stubs (~1,014), `voice_wake` (~749 — counted under DECIDE, not ERASE), `qr_pairing` (~329), the inference chat subtree (~350), `smart_routing` (~290), `self_message` (~250), `tailscale` (~230), the misc helper group (~250), `RepairTask`/heartbeat/dead-fields rounding out the rest. Erasing the ten ERASE items alone clears well over 7,000 lines with zero runtime impact.

### Highest-stakes decisions

1. **`self_message` safety (anti-loop) — ERASE is safe, but verify the claim.** This is the one deletion that *sounds* like it removes a safety guarantee. It does not: the module is never constructed in production (zero readers of `SELF_MESSAGE_BYPASS` / `BOT_USER_IDS`), and the actual self-loop protection is live and redundant in every transport (`discord.rs:583`, `imessage.rs:623`, `bluebubbles.rs:1067` `is_from_me`, plus owner-allowlist ingress on Signal/Nostr/Gmail). Deleting it removes no live safety behavior — but because it is *named* like a safety control, the operator should confirm the per-transport filters before signing off, then erase.

2. **`src/history/store/` — the only WIRE worth doing, and it pays off.** This is a stalled crate-extraction: the canonical `thinclaw-db/postgres_store` is already live via `PgBackend`/`Arc<dyn Database>` *and* it has coverage the root copy lacks (`experiments.rs` ~1402 lines, `repo_projects.rs`). The root copy is still compiled (`src/lib.rs:70`) and still held as a concrete type by 6 sites (3 CLI subcommands, 3 setup-wizard sites). A blind delete breaks the build. Medium-effort consolidation (redirect 6 callers onto `PgBackend`/`dyn Database`, port the 2 analytics methods, keep a thin `pub use` façade) collapses two persistence implementations into one and gives root the experiments/repo-projects coverage for free. This is the highest-value structural cleanup in the dossier.

3. **`voice_wake` — DECIDE, and the call hinges on roadmap, not code.** ~749 lines of genuinely non-trivial IP (cpal capture thread + a real Sherpa-ONNX subprocess integration), but zero callers, no build profile enables the `voice` feature, and the desktop product already ships its own independent frontend wake path (`use-voice-wake.ts`). If a headless "hey thinclaw" is on the roadmap, WIRE (medium effort, but needs a shipped ONNX model + binary and an agent-dispatch glue layer that does not exist). Otherwise ERASE and preserve the Sherpa scaffold in git history.

## Per-candidate detail

### src/safety/* orphan sibling files (14 .rs files, ~4931 lines)

**What it is.** 14 Rust source files sitting in `src/safety/` alongside `mod.rs` — byte-for-byte (or near) copies of the safety subsystem now owned by `crates/thinclaw-safety`: prompt-injection sanitizer, leak detector, PII redactor, OSV vuln check, skill-path containment, validator, policy, dangerous-tools list, auth profiles, device pairing, elevated-mode, key rotation, media-url checks, credential detection. `src/safety/mod.rs` is a pure 4-line compatibility façade that declares **no** submodules and re-exports `thinclaw_safety::*` plus `SmartApprover`/`ApprovalDecision`/`SmartApprovalMode` from `crate::tools::builtin`.

**Why it existed.** These were the original in-tree safety modules. Commit `e352a40d` "Refactor modules and extract crates" extracted the subsystem into `crates/thinclaw-safety` and rewrote `src/safety/mod.rs` into a glob-re-export façade, but did not delete the superseded sibling files. Proof of pre-extraction lineage: the orphans still use root-only import paths (`osv_check.rs` imports `crate::config::helpers::lock_env`; `skill_path.rs` uses `crate::platform::state_paths()`; `sanitizer.rs` imports `crate::safety::{...}`; `leak_detector.rs` uses `crate::safety::leak_detector::` test paths) that would not compile inside the standalone crate.

**Current state (uncompiled-orphan).** Locations:
- `src/safety/mod.rs:1-4` (façade), `src/lib.rs:86` (`pub mod safety;` → points only at mod.rs)
- `src/safety/{auth_profiles, credential_detect, dangerous_tools, device_pairing, elevated, key_rotation, leak_detector, media_url, osv_check, pii_redactor, policy, sanitizer, skill_path, validator}.rs`
- `crates/thinclaw-safety/src/lib.rs:1-16` (canonical module declarations)

Because `mod.rs` never declares `mod auth_profiles;` etc., none of the 14 files are in any compilation unit.

**Is there an alternative?** Yes — `crates/thinclaw-safety/src/*.rs` (live, compiled, canonical), re-exported into `crate::safety` via `mod.rs` `pub use thinclaw_safety::*`. All ~30 production/test consumers (`SafetyLayer` in `src/llm/reasoning.rs`, `src/agent/outcomes.rs`, `src/agent/subagent_executor.rs`, `SkillPathConfig` at `src/skills/registry.rs:423`, etc.) resolve through the façade to the crate, never to the sibling files.

**Could we fully implement it instead? (WIRE)** Not feasible and no value. Re-declaring them as modules would (a) collide with the glob re-export of identical symbols and (b) fail to compile because they use root-only paths. No unique live logic exists in the orphans — every drift makes the orphan the regressed side: the `pii_redactor.rs` orphan **lacks** the crate's `has_user_id_label_prefix` feature (which keeps labeled user/actor/sender IDs from being redacted) and uses the old `keep(value)` closure instead of `keep(value, range)`.

**Why it's safe to delete.** (1) `mod.rs` declares zero submodules — `rg '^\s*(pub )?mod (auth_profiles|...|validator)\b' src/` returns no hits in `src/safety`. (2) Every consumer reference resolves through `pub use thinclaw_safety::*` to the crate. (3) No `include!`/path-attribute/`build.rs` pulls them in. (4) diff confirms 9 files byte-identical to the crate and 5 drifted with the orphan as the older/inferior side. Deleting all 14 (keeping `mod.rs`) removes 4931 lines and cannot affect the build. The `#[cfg(test)]` modules in the orphans are never compiled; equivalent tests live and run in the crate.

**Recommendation: ERASE.** Delete all 14 sibling files, keep `mod.rs`. Pure dead weight that is also strictly inferior to the live crate.

### InferenceRouter chat modality (D-06)

**What it is.** A per-modality chat backend slot on `InferenceRouter` plus a full `ChatBackend` abstraction (trait + `LocalChatBackend` + `CloudChatBackend`) wrapping `UnifiedProvider` for streaming/non-streaming chat completion. `reconfigure()` builds a `CloudChatBackend` from `config.chat_backend` into `router.chat` (None for local). The trait exposes `stream_chat`/`complete`/`count_tokens`.

**Why it existed.** An earlier "unify all 5 AI modalities behind one router" design: `InferenceRouter` was meant to own the active chat backend the same way it owns embedding/TTS/STT/diffusion (all of which **are** live). Chat got a parallel, simpler implementation (`chat.rs::resolve_provider` feeding the Orchestrator/RigManager pipeline with tools, multimodal, RAG, web search, MCP sandbox, image-history filtering). The router chat path could not keep up, so the real chat command (`direct_chat_stream`) was wired to `resolve_provider` and the router chat object was left constructed-but-unread.

**Current state (built-but-unreachable).** Locations: `apps/desktop/backend/src/inference/router.rs:45` (chat field), `:87-89` (accessor), `:121-123` (setter), `:148`/`:164-168`/`:279-318` (Chat arms / reconfigure block); `inference/chat/mod.rs:55-77` (trait), `chat/local.rs:15-158`, `chat/cloud.rs:15-194`; live replacement `backend/src/chat.rs:48-151` (`resolve_provider`). **Keep:** `inference/mod.rs:69-75` (`Modality::Chat` enum) and `router.rs:196-220` (`available_backends_for(Chat)`).

**Is there an alternative?** Yes — `chat.rs::resolve_provider` (reads `config.chat_backend`, builds `UnifiedProvider` directly) feeding the Orchestrator/RigManager pipeline in `direct_chat_stream`/`direct_chat_completion`. This is the canonical registered Tauri chat path.

**Could we fully implement it instead? (WIRE)** Feasible but large effort, low value. Routing `direct_chat_stream` through `router.chat_backend()` would mean the router `ChatBackend` trait (plain streaming completion only — no tool-calling, Orchestrator, MCP sandbox, RAG/file-search, web-search permissions, multimodal image embedding, or image-history filtering) either re-implements the entire Orchestrator pipeline or guts existing chat features. It adds no user-facing capability — only relocates the call. `LocalChatBackend` is also never constructed, so the local path would need new wiring to `SidecarManager`/`EngineManager`.

**Why it's safe to delete.** `rg` across the repo: `.chat_backend()` → 0 callers; `.set_chat_backend(` → 0 callers; `LocalChatBackend` → only its own definition; `.stream_chat(` / `ChatRequest` / `ChatBackend` trait → 0 consumers outside `inference/chat/` and `router.rs`; no test references. The chat field is written in `reconfigure()` and read in exactly one place: `active_backends()` at `router.rs:167` (`b.info()`), which feeds the settings-UI "active" badge.

**Risk.** Deleting does not affect chat inference. The only breakage is cosmetic: the settings UI "active chat backend" badge would go blank unless the Chat arm of `active_backends()` is changed to synthesize a `BackendInfo` from `config.chat_backend`. `Modality::Chat`, `available_backends_for(Chat)`, `config.chat_backend`, and the Chat arm of `direct_inference_update_backend` are **live** and must stay.

**Recommendation: ERASE** the router chat field + `inference/chat/` subtree; keep `Modality::Chat`/`available_backends_for`/`config.chat_backend`, and re-derive the settings active-badge from config. Write-only dead state, no inference path.

### SmartRoutingProvider routing decorator (D-04)

**What it is.** An `LlmProvider` decorator wrapping a `primary` and `cheap` provider, overriding `complete()` to classify the last user message (via `classify_message`) into Simple/Moderate/Complex and route accordingly. For Moderate turns with cascade enabled it runs cheap, inspects the answer with a private `response_is_uncertain` heuristic (empty / <10 chars / hard-refusal phrases), and escalates to primary if uncertain. Tool calls and streaming always go to primary. Keeps atomic routing counters via `stats() -> SmartRoutingSnapshot`.

**Why it existed.** The original cost-routing engine — a chain decorator built alongside `RetryProvider`/`CachedProvider`/`CircuitBreakerProvider`, inspired by RelayPlane cost reduction. It predates the unified `RoutePlanner`. Wave 2 made `RoutePlanner` the single hot-path router; the classification and cascade logic moved into `route_planner.rs` + `runtime_manager.rs`, and the decorator was deliberately removed from the provider chain in `provider_factory.rs` to stop double-routing (comment at `:988` documents the retirement). The cascade heuristic was lifted verbatim into `cascade.rs`. The struct was left behind.

**Current state (dead-zero-callers).** Locations: `crates/thinclaw-llm/src/smart_routing.rs:55` (struct), `:62-144`, `:147-288`, `:24-49` (Stats/Snapshot); re-exports `lib.rs:29`, `src/llm/mod.rs:74`. Canonical: `cascade.rs:37` (`response_is_uncertain`), `runtime_manager.rs:827` (live caller), `route_planner.rs:531-572` (cheap/primary + cascade routing), `provider_factory.rs:978-990` (retirement comment).

**Is there an alternative?** Yes — `RoutePlanner` (`route_planner.rs:531-572`, deriving Simple/Moderate/Complex + `CascadePolicy::InspectAndEscalate`) plus `cascade::response_is_uncertain` (`cascade.rs:37`), driven from `runtime_manager.rs:827`. The canonical wired hot path.

**Could we fully implement it instead? (WIRE)** Not feasible, no value. Re-wiring means re-inserting the decorator into the provider chain — exactly what Wave 2 removed to stop double-routing. It adds nothing the planner lacks (Simple/Moderate/Complex routing, inspect-and-escalate cascade, tools/streaming to primary, richer scoring/telemetry). The only unique piece — the in-process `SmartRoutingSnapshot` counters — has zero consumers and is subsumed by the planner's `record_route_outcome` telemetry.

**Why it's safe to delete.** `rg 'SmartRoutingProvider::new'` → zero hits; never constructed in production or tests. Remaining references are the definition, two pub re-exports, and prose comments. `SmartRoutingSnapshot` is only produced inside the module. `response_is_uncertain` is a verbatim duplicate of the live `cascade.rs:37` copy. No `#[test]`/`#[cfg(test)]` blocks; no test file references it. The provider chain no longer wraps it.

**Risk.** Deleting the struct + impl + Stats/Snapshot + duplicate heuristic: nothing breaks. If the whole file is removed, the re-exports of `SmartRoutingConfig`/`TaskComplexity`/`classify_message` at `lib.rs` and `src/llm/mod.rs:74` must be repointed at `thinclaw_llm_core` (where they canonically live and where `RoutePlanner` already imports them) — a one-line façade change.

**Recommendation: ERASE.** Original cost-routing decorator superseded by RoutePlanner in Wave 2; zero callers, heuristic already duplicated into `cascade.rs`. Repoint the config/complexity re-exports at `thinclaw_llm_core`.

### RepairTask (D-03)

**What it is.** A background-task wrapper holding `Arc<dyn SelfRepair>` + a `check_interval`, whose `run()` loops forever: sleep, `detect_stuck_jobs` → `repair_stuck_job`, `detect_broken_tools` → `repair_broken_tool`/`dismiss_broken_tool`, logging each `RepairResult` via tracing. Pure orchestration shell over the `SelfRepair` trait; no repair logic of its own.

**Why it existed.** Introduced in commit `dd4a32de` (Wave 1C / WS-05 self-repair + crate-extraction) in the **same commit** that created `DefaultSelfRepair` — a convenience runner shipped alongside the extracted subsystem. The root `agent_loop.rs` instead inlined its own richer loop (adding channel notifications) and never adopted `RepairTask`. An extraction artifact that stalled at zero adoption.

**Current state (dead-zero-callers).** Locations: `crates/thinclaw-agent/src/self_repair.rs:324-388` (struct + `run()`); re-exports `src/agent/self_repair.rs:15`, `src/agent/mod.rs:99`. Canonical: `src/agent/agent_loop.rs:604-716` (inline self-repair loop).

**Is there an alternative?** Yes — the inline loop spawned at `agent_loop.rs:634-716`, built on `DefaultSelfRepair`. It is a strict superset of `RepairTask::run()`: same `SelfRepair` trait calls **plus** user-facing notification broadcasts over the channel manager.

**Could we fully implement it instead? (WIRE)** Small effort, no value. Adopting `RepairTask` would replace ~80 lines of inline loop but lose its distinguishing capability: broadcasting `Self-Repair: ...` notifications to channels on success/failure/manual-required. To preserve parity you would thread `Arc<ChannelManager>` + notification policy into `RepairTask`, turning it into a heavier abstraction for a single caller. Lateral move at best, feature regression at worst.

**Why it's safe to delete.** `rg "RepairTask"` returns only the definition (`:325`, `:330`) and two pub-use re-exports. No `::new`, no `.run()` call in any production code, binary, or test. The live path constructs `DefaultSelfRepair` directly and spawns its own loop.

**Risk.** None. Delete the struct+impl and drop `RepairTask` from the two pub-use lists. `SelfRepair`, `DefaultSelfRepair`, `RepairResult`, and all ports remain.

**Recommendation: ERASE.** Zero-caller, log-only duplicate of the channel-aware inline loop; born unused in the same commit as `DefaultSelfRepair`.

### Orphaned standalone heartbeat runner

**What it is.** A self-contained periodic heartbeat loop. `HeartbeatRunner::run()` ticks a tokio interval, runs memory hygiene + `check_heartbeat()` each cycle, tracks `consecutive_failures` against `config.max_failures` to self-disable, and pushes findings to an mpsc channel via `send_notification`. `spawn_heartbeat()` builds a runner and `tokio::spawn`s `run()`. Both exist twice (extracted crate + thin root wrapper). Sibling helpers in the same module (`check_heartbeat`, `new`, builders, `build_daily_context`, `is_effectively_empty`, `strip_html_comments`, `cap_daily_log`) are **live**.

**Why it existed.** The original heartbeat scheduler from the upstream fork lineage (Scrappy/OpenClaw → ironclaw → ThinClaw; first appears in `13dcdb0c`). The routine engine later absorbed all scheduling (`8574374f`), and `agent_loop.rs:742-744` documents the migration: "Heartbeat checks are now fully handled by the routine engine." The crate split duplicated the runner into a root wrapper, but the loop was never re-wired to a call site.

**Current state (dead-zero-callers).** Locations: `crates/thinclaw-agent/src/heartbeat.rs:133` (`run`), `:385` (`spawn_heartbeat`), `:284` (`send_notification`), `:97` (`consecutive_failures`), `:16`/`:21` (`interval`/`max_failures`); `src/agent/heartbeat.rs:75`/`:154` (wrappers); `src/agent/mod.rs:81` (re-export). Live replacement: `agent_loop.rs:854`/`:2223` (`upsert_heartbeat_routine`), `routine_engine.rs:1755` (`execute_heartbeat`).

**Is there an alternative?** Yes — `routine_engine.rs:1755 execute_heartbeat`, dispatched via the live routine registered by `agent_loop.rs:2223 upsert_heartbeat_routine` (called at `:854`).

**Could we fully implement it instead? (WIRE)** Not feasible, no value — actively regressing. Re-wiring `spawn_heartbeat` at startup would create a **second** heartbeat scheduler racing the routine-engine one (double LLM calls on the same `HEARTBEAT.md`). The routine-engine path is a strict superset: injects the prompt into the main session with full context, supports `HeartbeatTarget` routing/output suppression, post-completion critique scoring (feedback loop), reserved overflow job slot so heartbeats never starve, and persists run records/failures in the DB. The standalone runner is a blind fire-and-forget loop with an in-memory failure counter and a self-disable behavior the operator cannot observe or reset.

**Why it's safe to delete.** `rg spawn_heartbeat` → only two definitions + the `mod.rs:81` re-export + docs; zero call sites. `HeartbeatRunner` is constructed only at `src/agent/commands.rs:237` (the `/heartbeat` command) and `tests/heartbeat_integration.rs:92`, and **both call only `check_heartbeat()`**, never `run()`. The run()-only fields are confirmed dead: `config.interval` read only at `:141`/`:144` (inside run); `config.max_failures`/`consecutive_failures` read only at `:181` (inside run); `send_notification` called only at `:172` (inside run). All other matches in the repo belong to unrelated subsystems.

**Risk.** None to runtime. Remove `run()`/`spawn_heartbeat` plus the now-orphaned `interval`/`max_failures`/`consecutive_failures`/`send_notification` (and run-only response-channel plumbing) in the same commit to avoid dead_code warnings. Keep `check_heartbeat`, `new()`, builders, and shared helpers (live via `/heartbeat` and `routine_engine::execute_heartbeat`). Drop `spawn_heartbeat` from `mod.rs:81` and fix the stale `src/workspace/README.md:99` example.

**Recommendation: ERASE.** Abandoned fork-era loop with zero callers; routine engine is a strict superset.

### D-05: Reasoning.safety + SpawnSubagentTool.executor

**What it is.** Two unused struct fields that each force a constructor argument across many call sites without ever being read.
(1) `Reasoning.safety: Arc<SafetyLayer>` — all 28 `Reasoning::new(llm, safety)` callers pass a real `SafetyLayer`, but the field is only stored (`reasoning.rs:559`) and cloned in `fork_with_llm` (`:600`); no method ever calls `self.safety.<anything>()`. Reasoning's actual prompt-fragment sanitization uses the free function `sanitize_prompt_bound_content` (`:529`).
(2) `SpawnSubagentTool.executor: Arc<dyn SubagentToolPort>` — stored in `new()` (`subagent.rs:67`) but never read; `SpawnSubagentTool::execute` (`:166-278`) only emits a JSON `{action:"spawn_subagent", request:...}` envelope that the dispatcher intercepts and routes through its **own** `self.subagent_executor` (`tool_execution.rs:604`). The sibling tools `ListSubagentsTool`/`CancelSubagentTool` **do** use their executor — only the Spawn variant's field is dead.

**Why it existed.** Both vestigial-from-birth. `git log -S "self.safety." -- src/llm/reasoning.rs` returns **zero** commits: no version ever called a method on the field — the `:492` comment "Will be used for sanitizing tool outputs" describes intended-but-never-built behavior. Traces to `3c8bf93a` "feat: ironclaw agent engine integration". For the subagent tool, `SpawnSubagentTool` got the same `new(executor)` shape for API symmetry with its siblings, but execution was always delegated to the dispatcher interception (intentional — `tool_execution.rs:594-597`). The `#[allow(dead_code)]` on both fields acknowledges the deadness.

**Current state (compiles-unused).** Locations: `src/llm/reasoning.rs:493`/`:492`/`:556-575`/`:600`; `crates/thinclaw-tools/src/builtin/subagent.rs:60-69`/`:61`; `src/main.rs:1702` (only production constructor); `src/agent/dispatcher/tool_execution.rs:593-617` (the real path).

**Is there an alternative?** Yes. Tool-output sanitization → `SafetyLayer::sanitize_tool_output`, called at the execution/dispatch layer (`src/tools/execution.rs:276`, `agent/thread_ops.rs:2192` & `:2501`, `dispatcher/tool_execution.rs:973`, `agent/worker.rs:976`). Reasoning is not on that path. Subagent spawning → dispatcher interception at `tool_execution.rs:598-617` using its own `self.subagent_executor`.

**Could we fully implement it instead? (WIRE)** Feasible (medium) but low value. Wiring `Reasoning.safety` (e.g. `validate_input` on inbound content, or `redact_pii_in_prompts` on assembled prompts) would **duplicate** protection already upstream (fragments via `sanitize_prompt_bound_content`; PII via `dispatcher/prompt_context.rs:92`; tool outputs at the execution layer) — a second overlapping enforcement point with unclear ownership and real risk of double-redaction. `SpawnSubagentTool.executor`: nothing to wire — the architecture deliberately routes spawning through the dispatcher. Removal, not wiring.

**Why it's safe to delete.** `grep self.safety src/llm/reasoning.rs` → only field-def, constructor store, fork clone; no method call; `git log -S` → zero readers ever. Removal drops the `safety` param from `Reasoning::new` — a mechanical ripple across 28 callers, each already holding the `SafetyLayer` in scope. `grep self.executor` in subagent.rs → reads only in List/Cancel, never in Spawn; `execute()` ignores it; one production caller to adjust (`main.rs:1702`). The `SubagentToolPort` trait/impl stays (used by siblings).

**Risk.** Near-zero — compile-time-only ripples. A missed caller fails to compile rather than misbehaving silently. Coordinate with WS-10's `reasoning.rs` decomposition so the constructor-signature change lands in one pass.

**Recommendation: ERASE.** Both fields vestigial-from-birth (never read in any commit per `git -S`); sanitization and spawning already live in the canonical execution/dispatch layer.

### Dead CLI scaffolding: nodes.rs, subagent_spawn.rs, session_export.rs

**What it is.** Three modules declared `pub mod` in `src/cli/mod.rs` but never given a clap `Command` enum variant or a `run_*` dispatch arm. None expose a `Command` subcommand type or async entrypoint — each is a pure in-memory data structure with doc-comments describing subcommands that do not exist. `nodes.rs`: `Node`/`NodeStore` + `format_node_list/detail` (doc claims `nodes list/show/remove/clear`). `subagent_spawn.rs`: `parse_spawn_command`, `SpawnRequest`/`SpawnResult`, `SubagentTracker`. `session_export.rs`: `ExportFormat`, `SessionExporter`, `ExportRecord`, `SessionExportResponse`. Each carries in-file `#[test]` blocks (10/8/12 tests) exercising its own API — the only reason the dead-code lint stays quiet.

**Why it existed.** Fork/sprint lineage. `nodes.rs` and `subagent_spawn.rs` first appeared in `461ebe1d` "Sprint 10"; `session_export.rs` in `db2b008c` "Sprint 12". Sprint-era feature stubs from the pre-rebrand history (Scrappy/OpenClaw fork), written API-first with unit tests but never wired into clap dispatch. The capabilities were later implemented properly elsewhere.

**Current state (compiles-unused).** Locations: the three files (`nodes.rs:1-265`, `subagent_spawn.rs:1-374`, `session_export.rs:1-375`); `src/cli/mod.rs:38`/`:41`/`:44` (`pub mod` lines); `src/cli/mod.rs:143-371` (Command enum — no matching variant); `src/main.rs:64-360` (dispatch — no matching arm).

**Is there an alternative?** Mostly. `session_export.rs` → `src/cli/sessions.rs SessionCommand::Export` (`:42-43`, supports json + markdown transcript with output path) and `trajectory.rs TrajectoryCommand::Export`. `subagent_spawn.rs` → live `spawn_subagent` agent tool (`crates/thinclaw-tools/src/builtin/subagent.rs:60`) + runtime `SubagentExecutor` (`src/agent/subagent_executor.rs`). `nodes.rs` → **no** live CLI equivalent (device/node management is not a shipped CLI surface); the module is a non-functional stub with no backing store or transport.

**Could we fully implement it instead? (WIRE)** Not wire-ready, low value. None define a clap `Command` type or `run_*` entrypoint, so "wiring" means writing the subcommands from scratch. `session_export` and `subagent_spawn` would duplicate already-shipped surfaces (negative value). `nodes.rs` would require designing a real device/node registry (persistent store, discovery/transport, online/offline tracking) — the in-memory HashMap stub provides almost none of it. Green-field feature, not a disconnected one.

**Why it's safe to delete.** `rg` for every public symbol returns zero hits outside each file: `NodeStore|format_node_list|format_node_detail` → none; `parse_spawn_command|SubagentTracker` → none; `SessionExporter|SessionExportResponse|ExportFormat` → none. Only external references are the three `pub mod` lines. The Command enum and dispatch match contain no matching arm. The unrelated `SubagentSpawn*` grep hits are runtime types in agent/tools/channels crates, none importing `src/cli::subagent_spawn`.

**Risk.** None to production. Deleting the three files + three `pub mod` lines removes ~1014 LOC and ~30 unit tests that only test the dead types. `cargo build`/`test` stay green.

**Recommendation: ERASE** all three. Sprint-era stubs declared `pub mod` but never dispatched; session-export and subagent-spawn already shipped elsewhere; nodes is a non-functional stub.

### self_message anti-loop module (SelfMessageConfig + TrustedMetadata)

**What it is.** A self-contained anti-loop safety module. `SelfMessageConfig` holds a set of bot user IDs (from `BOT_USER_ID`/`BOT_USER_IDS` env or `register_bot_id`) + an `enabled` flag (`SELF_MESSAGE_BYPASS`), and exposes `is_self_message(&IncomingMessage)` and `filter_messages(Vec<IncomingMessage>)` to drop messages whose `user_id` matches a known bot ID — intended to stop the agent from replying to its own echoed sends. Also defines `TrustedMetadata` (sender/channel/thread/is_self/is_group/timestamp DTO with `from_message` + `to_system_context`). 11 unit tests cover the logic. Correct in isolation; never invoked.

**Why it existed.** Added in `04234432` "feat: Sprint 8 — core quality, LLM providers, channel hardening" as part of a channel-hardening pass (rebrand of the Scrappy/OpenClaw fork). Written as a generic, env-configurable cross-channel loop guard but never wired into the channel manager or any transport. Each native transport grew its own inline self/echo filter, so this centralized module became a stranded abstraction.

**Current state (compiles-unused).** Locations: `crates/thinclaw-channels/src/self_message.rs:1-250`; `crates/thinclaw-channels/src/lib.rs:28`; `src/channels/self_message.rs:1`; `src/channels/mod.rs:68`/`:99`.

**Is there an alternative?** Yes — live, redundant per-transport inline filters: Discord `discord.rs:583-587` (`msg.author.bot` + `author.id == bot_user_id`); iMessage `imessage.rs:623` (`if msg.is_from_me { continue }` + dedup ring buffer `:628-634`); BlueBubbles `bluebubbles.rs:1053-1069` (`is_from_me`); and owner-allowlist ingress structurally excluding the bot for Signal (`signal.rs:244`), Nostr (`nostr.rs:1-3` single owner pubkey), and Gmail. None reference `self_message.rs`.

**Could we fully implement it instead? (WIRE)** Feasible (medium) but low value. Would require threading a `SelfMessageConfig` through `ChannelManager` (populated with each channel's bot ID at connect time — no transport currently reports its bot ID to the manager) and applying `filter_messages` in `manager.rs:452` before dispatch. It adds **no** protection the system lacks and is strictly weaker than several existing filters: matches only on `user_id` equality, so it would miss Discord's general `author.bot` rule, iMessage tapback suppression, and the dedup ring buffer. Net value is DRY consolidation, not safety.

**Why it's safe to delete.** Zero production consumers. `rg 'SelfMessageConfig::|from_env'` and `rg 'SELF_MESSAGE_BYPASS|BOT_USER_IDS|"BOT_USER_ID"'` (excluding the module) return nothing — never constructed, env vars never read. Only non-test references are two façade re-exports. `is_self_message`/`filter_messages`/`TrustedMetadata::from_message` are called only from the module's own tests. The actual anti-self-loop guarantee is enforced — redundantly — by the live inline filters and owner-allowlist ingress.

**Risk.** None to runtime safety: the agent cannot loop via this module today (it is inert) and remains protected by live transport filters after deletion. Only mechanical risk is removing the two façade re-exports; `TrustedMetadata` is also dead and goes with it.

**Recommendation: ERASE.** Dead centralized anti-loop module — never constructed in production; real protection is live and redundant per transport plus owner allowlists, so erasing carries no safety loss. (Verify the per-transport filters before signing off, since the name implies a safety control.)

### qr_pairing (src/qr_pairing.rs)

**What it is.** A self-contained Rust module for QR-code device pairing as a Tailscale-less fallback. Defines `PairingInfo` (host/port/protocol/cert_fingerprint/one-time `pairing_token`/version) with a custom `thinclaw://pair?...` URL scheme (`to_url`/`from_url`), `generate_pairing_token` (32 random bytes → hand-rolled URL-safe base64), `render_qr_terminal` (real scannable QR via the `qrcode` crate, Unicode half-blocks), and `PairingSession` (in-memory one-time token validation + terminal QR display). The docstring claims the orchestrator generates a self-signed TLS cert via `rcgen`, pins its SHA-256 fingerprint into the QR, and the Tauri client scans-to-pair with cert pinning.

**Why it existed.** Fork-lineage scaffolding. Added 2026-03-01 in `b9fa57bd` ("feat(pairing): add QR code pairing for non-Tailscale setups") by the original fork author under the prior brand — the URL scheme was literally `ironclaw://pair`, and the commit message boasts protocol-completeness. Speculative remote-mode scaffolding written to tick a checklist, never wired to a server. Survived the IronClaw→ThinClaw rebrand (`727453ee`) as a mechanical rename. The TLS-pinning premise was never built: there is no `rcgen` usage anywhere and no producer of `cert_fingerprint` outside this file, so `PairingSession::new` requires a fingerprint nothing generates.

**Current state (compiles-unused).** Locations: `src/qr_pairing.rs:1-329`; `src/lib.rs:83` (`pub mod qr_pairing;`); security defects at `:219` (non-constant-time token compare `if self.info.pairing_token == token`), `:94` (hand-rolled base64), `:88` (`generate_pairing_token` via per-byte `rand::random`). Live alternative `thinclaw_channels::pairing::PairingStore` (re-exported via `src/pairing/mod.rs`; the orphaned root copy `src/pairing/store.rs` was removed in PR #197).

**Is there an alternative?** Yes — `thinclaw_channels::pairing::PairingStore` (re-exported via `src/pairing/mod.rs`), the live persistent device/channel pairing mechanism, wired at `src/setup/channels.rs`, `src/cli/pairing.rs`, `src/channels/wasm/mod.rs`, and `tests/wasm_channel_integration.rs`. Provides pending-request TTL, max-pending caps, approve rate-limiting, allow/block lists, and on-disk persistence — none of which `qr_pairing.rs` offers.

**Could we fully implement it instead? (WIRE)** Feasible but large effort, low value — effectively building a new feature. `qr_pairing.rs` is only the encode/display half; the consuming half does not exist. Wiring the documented TLS-pinned flow requires: (1) actually generating a self-signed cert (add `rcgen`, compute SHA-256) in the server — nothing produces `cert_fingerprint` today; (2) a server endpoint presenting the pinned cert and validating the token; (3) Tauri client side to scan `thinclaw://pair`, pin, and connect (the current frontend `qrCode` state is an unrelated generic `api.qrserver.com` image); (4) two mandatory security fixes — replace the `==` token compare (`:219`) with `subtle::ConstantTimeEq::ct_eq` (pattern at `crates/thinclaw-gateway/src/web/auth.rs` and `src/orchestrator/auth.rs`) and drop hand-rolled base64 (`:94`) for `base64 URL_SAFE_NO_PAD`; (5) back one-time-use state with `PairingStore` to survive restarts. Only the QR rendering (~65 lines) is genuinely reusable.

**Why it's safe to delete.** `rg 'PairingSession|PairingInfo|render_qr_terminal|generate_pairing_token|qr_pairing::' -g '!src/qr_pairing.rs'` → zero hits. The only reference is `pub mod qr_pairing;` at `src/lib.rs:83`. The `validate_token` name collides with an unrelated OAuth helper in `src/cli/tool.rs`. The desktop frontend `qrCode` state (`ThinClawChannels.tsx:197`) is separate TS using a third-party image API and references neither `thinclaw://pair`, `cert_fingerprint`, nor `pairing_token`. The live `PairingStore` is independently wired and unaffected.

**Risk.** Nothing breaks. Deleting the file and the `lib.rs:83` line leaves the build green and `PairingStore` untouched. The only loss is ~65 lines of reusable QR-terminal rendering, recoverable from `b9fa57bd` if a real feature is later built.

**Recommendation: ERASE** the file and `src/lib.rs:83`. Never-wired fork-era scaffold (zero callers, unbuilt TLS premise, two security defects), fully superseded by the live `PairingStore`.

### src/tailscale.rs — TailscaleDiscovery

**What it is.** An async client for the Tailscale local daemon API (`http://localhost:41112/localapi/v0/status`). Parses tailnet status/peers and exposes: `discover_orchestrators()` (online peers tagged "thinclaw" or whose hostname contains "thinclaw"/"molty"), `find_orchestrator_url()` (returns `http://<ip>:3000`), `local_ip()`, `is_available()`, `extract_identity()` (local tailnet user/IPs for passwordless gateway auth), and `is_trusted_peer(remote_ip)`. The doc states the purpose: let a Tauri thin client auto-find a headless orchestrator on the tailnet, optionally with implicit gateway auth by tailnet IP.

**Why it existed.** Fork/abandoned-feature lineage. Added in the IronClaw era (`d0c3206c` "feat: add screen/camera/location tools, auto-update, tailscale"), supporting a thin-client architecture where a Tauri app connected over a tailnet to a **separate** headless orchestrator and discovered it automatically. That split design was abandoned: `apps/desktop/backend` now runs the orchestrator in-process (Tauri commands + sidecar + local Docker sandbox), so there is no remote orchestrator to discover. Its sibling `src/qr_pairing.rs` is also orphaned, confirming the whole thin-client layer died together. The module survived only as a mechanical rebrand carry-through (`616d5d7a`, `d40b0a25`, `c786fe4f`).

**Current state (dead-zero-callers).** Locations: `src/tailscale.rs:17`/`:62`/`:77`/`:143`/`:195`/`:206`/`:228`; `src/lib.rs:96` (`pub mod tailscale`).

**Is there an alternative?** The **live** Tailscale capability is a different concern: `src/tunnel/tailscale.rs` (`TailscaleTunnel` — real `tailscale serve`/`funnel` ingress, used via `src/tunnel/mod.rs:124`), plus setup-wizard tunnel config (`src/setup/channels.rs`), doctor checks (`src/cli/doctor.rs:100`), and remote-deploy `--tailscale <key>` (`apps/desktop/backend/src/thinclaw/deploy.rs:111`). These cover remote-access ingress; none consume `TailscaleDiscovery`. The discovery/identity/trusted-peer use cases themselves have no surviving consumer.

**Could we fully implement it instead? (WIRE)** Feasible but large, low value. Wiring requires first re-introducing a consumer that no longer exists: a thin-client/remote-orchestrator connection path in `apps/desktop/backend` (the orchestrator runs in-process — nothing to auto-discover), or a gateway auth layer trusting tailnet IPs. Concretely: a desktop bootstrap calling `find_orchestrator_url()` feeding a nonexistent remote-orchestrator client + connection UI; a tagging convention so orchestrators advertise `tag:thinclaw`; and gateway middleware mapping source IP to tailnet identity for token bypass. Value is low because the current architecture (single-process desktop + tunnel-based remote access) already covers remote reach; the discovery layer presupposes a multi-node topology the product no longer ships.

**Why it's safe to delete.** `rg` for all the type/function names excluding `src/tailscale.rs` → zero matches; `crate::tailscale`/`::tailscale::` → zero matches. The only reference is `pub mod tailscale;` at `src/lib.rs:96`. All other "tailscale" hits belong to the unrelated live `src/tunnel/tailscale.rs` and tunnel/deploy code. Internal `#[cfg(test)]` tests are the sole exercisers.

**Risk.** None to functionality. Deletion requires removing the `pub mod tailscale;` line (it is public API surface; no in-repo consumer imports `thinclaw::tailscale`). Delete `src/tailscale.rs` and the `lib.rs` line together; consider deleting the equally-orphaned `src/qr_pairing.rs` in the same cleanup (both belong to the dead thin-client layer).

**Recommendation: ERASE.** Orphaned IronClaw-era thin-client tailnet-discovery (zero callers); the live remote-access need is met by the unrelated `TailscaleTunnel` + deploy path. The WIRE option presupposes a multi-node topology the product no longer ships.

### Misc dead helpers + dead HTTPS credential mappings (group)

**What it is.** A grab-bag of dead-or-no-op code:
- **(a)** The 3 entries in sandbox `default_credential_mappings()` — `OPENAI_API_KEY`/`api.openai.com`, `ANTHROPIC_API_KEY`/`api.anthropic.com`, `NEARAI_API_KEY`/`api.near.ai`. The **function** is live (called by `NetworkProxyBuilder::new`/`from_config` at `src/sandbox/proxy/mod.rs:64,78`), but these entries only produce a `NetworkDecision::AllowWithCredentials`, which the proxy honors **only** on the plaintext `http://` forward path. All 3 are HTTPS hosts reached via `CONNECT` (`handle_connect`, `src/sandbox/proxy/http.rs:305`), which tunnels opaquely and discards the credential portion (`http.rs:289` treats Allow and AllowWithCredentials identically). No credential is ever injected.
- **(b)** `build_telegram_channel` (`build.rs:43`, `#[allow(dead_code)]`) — a ~90L build-script helper.
- **(c)** `sse.rs subscribe()` (`crates/thinclaw-gateway/src/web/sse.rs:88`) — a pre-formatted SSE stream constructor.
- **(d)** `install_bundled_channel_from_artifacts` (`src/extensions/manager.rs:1761`, `#[allow(dead_code)]`), `redact_gateway_url` (`src/boot_screen.rs:243`, `#[allow(dead_code)]`), `_secret_cli_access_context` (`src/cli/secrets.rs:279`, `#[allow(dead_code)]`).

**Why it existed.** Mixed lineage. The HTTPS cred mappings are a stale assumption from when credential injection was imagined to cover HTTPS too (the `http.rs` module doc and `handle_connect` explicitly state HTTPS credentials must instead come out-of-band via the orchestrator's `/worker/{id}/credentials` endpoint). `build_telegram_channel` is a leftover default-build path superseded by `build_all_wasm_extensions`. `sse.rs subscribe()` is a superseded API; production migrated to `subscribe_raw()`. `install_bundled_channel_from_artifacts` is annotated "Reserved: ... currently done via SetupWizard". `redact_gateway_url` and `_secret_cli_access_context` are orphaned helpers whose call sites were removed or never added.

**Current state (drifted-duplicate).** See locations above; live alternatives below.

**Is there an alternative?** Yes. (a) HTTPS auth → orchestrator credentials endpoint `GET /worker/{id}/credentials` (env-var delivery, `http.rs:9-12,301-304`); the 3 hosts also remain on `default_allowlist()` (`crates/thinclaw-types/src/sandbox.rs:141-143`), so access is unaffected. (b) `build_all_wasm_extensions` (`build.rs:139`). (c) `subscribe_raw` (`sse.rs:61`) — used by `src/channels/web/ws.rs:90`, `handlers/chat.rs:313/834/936`, `mod.rs:658`. (d) `install_bundled_channel_from_artifacts` → `SetupWizard`; the other two have no consumer.

**Could we fully implement it instead? (WIRE)** Not feasible, no value. (a) Wiring the HTTPS mappings would require a MITM TLS proxy or moving injection into the request body — explicitly rejected by the design; the working alternative already exists, so the mappings would still be no-ops. (b) `build_telegram_channel` just re-adds a redundant single-channel build path. (c) Routing through `subscribe()` instead of `subscribe_raw()` centralizes formatting, but production deliberately formats per-channel — no gain. (d) `install_bundled_channel_from_artifacts` only re-wraps `channels::wasm::install_bundled_channel` already used by SetupWizard; the other two have no consumer needing them.

**Why it's safe to delete.** (a) The 3 hosts are independently in `default_allowlist()` (`sandbox.rs:141-143`), and `http.rs:289` + `handle_connect` prove `AllowWithCredentials` is never acted on for HTTPS/CONNECT — removing the entries changes no runtime behavior. **Keep the function** `default_credential_mappings()` (live callers at `proxy/mod.rs:64,78`); only the 3 vec entries are dead. (b)(d) `rg` returns only the definition lines (all `#[allow(dead_code)]`) — zero callers. (c) `subscribe()` is referenced only once, in test `test_subscribe_raw_rejects_over_limit` (`sse.rs:254`) as an `.is_none()` assertion; no production caller.

**Risk.** Minimal. (a) Leave `default_credential_mappings()` empty (or only future `http://` hosts) — do **not** delete the function. (c) One test assertion (`sse.rs:254`) must be removed/retargeted to `subscribe_raw` or the build breaks. Otherwise nothing references these symbols.

**Recommendation: ERASE.** (a) Delete the 3 HTTPS cred-map entries, keep the function (injection is http-only; HTTPS uses CONNECT + the worker credentials endpoint; hosts already on the allowlist); (b) delete `build_telegram_channel`; (d) delete `install_bundled_channel_from_artifacts`/`redact_gateway_url`/`_secret_cli_access_context`; (c) delete `sse.rs subscribe()` and fix the one test assertion at `sse.rs:254`.

### src/history/store/ (root PostgreSQL Store)

**What it is.** A near-duplicate PostgreSQL persistence layer. `src/history/store/` contains a full `Store` struct (own deadpool `Pool`, `new`/`from_pool`, plus conversation/job/routine/sandbox/settings/learning/outcome query impls across 18 files). `crates/thinclaw-db/src/postgres_store/` is the same code (13 of the common files byte-identical) **plus** two files the root lacks: `experiments.rs` (1402 lines) and `repo_projects.rs`. Both re-export shared DTOs from the `thinclaw-history` crate; the divergence in the non-identical files (core.rs, types.rs, mod.rs, conversation_queries.rs, conversations.rs, routine_rows.rs, sandbox_jobs.rs) is almost entirely import-path rewrites (root `crate::agent::routine`/`crate::config::DatabaseConfig`/`crate::context` vs crate `thinclaw_agent::routine`/`thinclaw_types`).

**Why it existed.** Both copies were created in the same commit `e352a40d` "Refactor modules and extract crates" — the ports/adapters crate-extraction effort CLAUDE.md describes as still in progress. `postgres_store` was lifted into `thinclaw-db`, rewired onto the extracted crates, and grew the canonical surface (experiments, repo_projects, the `dyn Database` trait impl via `PgBackend`). The root `src/history/store` copy was left in place so root-package call sites holding a concrete `Store`/`Pool` (CLI subcommands and the setup wizard) would keep compiling. A stalled migration: the runtime moved to `thinclaw-db`, but six concrete-type call sites were never redirected.

**Current state (drifted-duplicate).** Locations: root `src/history/store/core.rs:33`, `store/mod.rs:1-65`, `src/lib.rs:70` (compiled), `src/history/mod.rs:12-23` (re-exports root `store::Store`), `src/history/analytics.rs:8-11` (root-only `impl Store` adding JobStats/ToolStats). Canonical `crates/thinclaw-db/src/postgres_store/core.rs:0`, `postgres_store/mod.rs:1-60`, `thinclaw-db/src/lib.rs:46`, `postgres.rs:14`/`:75,85` (`PgBackend` wraps `Store`), wired live at `src/app.rs:311-324` (`Arc<dyn Database>`). Live root-copy callers: `src/cli/mcp.rs:1256`, `src/cli/secrets.rs:264`, `src/cli/tool.rs:587`, `src/setup/wizard/mod.rs:1669`, `src/setup/wizard/persistence.rs:20`, `:215`.

**Is there an alternative?** Yes — `crates/thinclaw-db/src/postgres_store/` (wrapped by `PgBackend`, exposed as `Arc<dyn Database>`), the canonical, more-complete implementation used by the live `AppBuilder` runtime.

**Could we fully implement it instead? (WIRE)** Feasible, medium effort, **high value**. Consolidate onto `thinclaw-db`, then delete `src/history/store`. `thinclaw-db` is already a root dependency (`Cargo.toml:48`). Redirect the 6 concrete callers off `crate::history::Store`: (a) the setup wizard (3 sites) uses `from_pool` + `get_all_settings`/`set_all_settings` — all on the `SettingsStore`/`Database` trait, so route via the existing `Arc<dyn Database>` or `thinclaw_db PgBackend`; (b) CLI mcp/secrets/tool (3 sites) need `Store::new` + `run_migrations` + `pool()` to construct `PostgresSecretsStore` — covered by `PgBackend::new`/`run_migrations`/`pool()`, all already public (`postgres.rs:85,93,102`). The only root-only logic is the analytics `impl Store` (JobStats/ToolStats) in `src/history/analytics.rs`, consumed only inside `src/history` — port those two methods to `thinclaw-db` (or drop if unused). Keep a thin `pub use` façade at `src/history/mod.rs` so `crate::history::...` import paths survive. Net result: one persistence implementation, plus root automatically gains the experiments/repo_projects coverage it currently lacks.

**Why it's NOT safe to blind-delete.** `src/history/store` is **not** dead. `src/lib.rs:70` compiles it, and grep finds 6 live non-test callers of the concrete `crate::history::Store` type: `src/cli/{mcp.rs:1256,secrets.rs:264,tool.rs:587}` call `Store::new(&config.database)` then `run_migrations()`+`pool()`; `src/setup/wizard/{mod.rs:1669,persistence.rs:20,persistence.rs:215}` call `Store::from_pool(pool)` then `get/set_all_settings`. `src/history/analytics.rs` adds an `impl Store`. The replacement exists and the runtime already uses it (`app.rs:311`), but these 6 sites bypass `dyn Database` and hold the concrete root type.

**Risk.** Deleting before redirecting callers **breaks the build**: CLI secrets/mcp/tool subcommands and the setup-wizard settings persistence/reconnect paths lose their `Store` type. Safe only after the medium-effort consolidation redirects those 6 sites and ports the analytics methods.

**Recommendation: WIRE.** Stalled crate-extraction duplicate: `thinclaw-db/postgres_store` is canonical (live via `PgBackend`/`dyn Database`, with extra experiments+repo_projects), but `src/history/store` is still compiled and called by 6 concrete sites (3 CLI, 3 setup wizard). Redirect those onto `thinclaw-db`, port the analytics methods, keep a `pub use` façade, then delete. The highest-value structural cleanup in this dossier.

### voice_wake module + the `voice` cargo feature + optional `cpal` dependency

> **OUTCOME: WIRED, not erased.** The operator chose "build the wake word", so the ERASE reasoning in
> this entry did **not** apply. `VoiceWakeRuntime::new` is now a live production constructor in
> `AppBuilder` (`src/app.rs:1681`, stored on the runtime as `src/app.rs:119` `voice_wake:
> Option<VoiceWakeRuntime>`) and its `VoiceWakeEvent`s are consumed by the `voice_wake_forwarder` task
> in `src/async_main.rs` (`:1094` takes the runtime, `:1111` handles `WakeWordDetected`, `:1141`
> handles `Error`), routing wake-word utterances into the dispatcher. It remains behind the opt-in
> `voice` feature (still not in any default profile). The "zero callers / compiles-unused / safe to
> delete / Recommendation: DECIDE" text below is the pre-decision 2026-06-25 snapshot and no longer
> describes the code.

**What it is.** A headless/remote-mode wake-word detection runtime. `VoiceWakeRuntime` spawns a background tokio task plus a dedicated OS thread for cpal audio capture (`cpal::Stream` is `!Send`), computes RMS energy frames, and emits `VoiceWakeEvent` values over an mpsc channel with a watch-based status flag. Two backends: `EnergyDetector` (fully implemented RMS voice-activity detection — detects that *someone is speaking*, **not** the phrase "hey molty") and `SherpaOnnx` (a real but unproven integration that shells out to an external `sherpa-onnx-keyword-spotter` binary over stdin/stdout for offline keyword spotting, falling back to energy when the binary/model/keywords.txt are absent). Without the `voice` feature the detection loop is a no-op that sleeps. Real plumbing, but the only word-level detection path depends on an external binary + ONNX model the repo does not ship.

**Why it existed.** Duplicate-of-frontend and stalled headless scaffold.

**Current state (compiles-unused).** Locations: `src/voice_wake.rs:1-749`; `src/lib.rs:109` (`pub mod voice_wake;`); `Cargo.toml:223` (cpal optional dep), `:337-339` (voice feature def), `:296-305` (full profile — does **not** list voice); `crates/thinclaw-config/Cargo.toml:33` (`voice = []` no-op); sibling wired `src/talk_mode.rs:1-60`, `src/app.rs:1014` (`TalkModeTool` registered; no VoiceWake equivalent); frontend `apps/desktop/frontend/src/hooks/use-voice-wake.ts:1-90`, `components/voice/VoiceWakeOverlay.tsx`.

**Is there an alternative?** Yes. For the desktop product: `use-voice-wake.ts` — a self-contained browser Web Audio API (AnalyserNode) RMS energy VAD that triggers `onWake` and feeds `VoiceWakeOverlay.tsx` + `ChatProvider.tsx`. This is the live, shipping voice-wake path and it does **not** call the Rust module. For audio-to-text input generally: `src/talk_mode.rs` (`TalkModeTool`) is the wired sibling (`src/app.rs:1014`). The Rust `voice_wake.rs` has no live consumer — but its *purpose* (energy-level wake) is already covered by the frontend on the only surface with a mic UI.

**Could we fully implement it instead? (WIRE)** Feasible, medium effort, low value. (1) Add `voice` to a build profile (extend `full` at `Cargo.toml:296` or document a headless profile; `libasound2-dev` required on Linux). (2) In headless/gateway startup, construct `VoiceWakeRuntime::new(config)`, call `take_events()`, spawn a consumer, and `start()` it — zero call sites exist today. (3) Wire `WakeWordDetected` into the agent listening/dispatch path (likely chained into talk_mode/STT to capture the follow-up utterance) — this glue does not exist. (4) For a true "hey thinclaw" wake word (not just any-speech VAD), ship or fetch the Sherpa-ONNX binary + zipformer model + keywords.txt and verify the subprocess output-parsing contract (currently unverified guesswork matching on the literal `keyword_detected` substring). The `EnergyDetector` alone cannot distinguish a wake phrase from any noise. Capability if fully wired: hands-free "hey thinclaw" activation in **headless** deployments only — a niche surface, since the desktop app (the mic-owning product) already has its own frontend wake path.

**Why it's safe to delete.** `rg` for `VoiceWakeRuntime|VoiceWakeConfig|VoiceWakeEvent` across the Rust workspace returns zero hits outside `src/voice_wake.rs` (only external reference is `pub mod voice_wake;` at `src/lib.rs:109`). `VoiceWakeRuntime::new` has no production callers even under `--features voice`. The `voice` feature is absent from every build profile including `full` (`Cargo.toml:296-305`), and `thinclaw-config/voice` is a no-op empty feature. The desktop product's voice wake is an independent TS implementation that never touches this code.

**Risk.** Effectively nothing. Removing `src/voice_wake.rs`, the `pub mod` line, the `voice` feature stanza, the optional cpal dep, and the no-op `thinclaw-config/voice` feature breaks no production path and no shipped capability. The only loss is the (untested-against-real-models) Sherpa-ONNX subprocess scaffold — the only genuinely non-trivial IP here; preserve it in git history / a branch if a headless wake word is on the roadmap. `BUILD_PROFILES.md` references to `voice` would need a docs sweep.

**Recommendation: DECIDE.** Headless wake-word scaffold with real cpal/Sherpa plumbing but zero callers, no enabling build profile, and a live frontend reimplementation on the only mic-owning surface. **ERASE unless a headless "hey thinclaw" is genuinely on the roadmap**, in which case WIRE is medium-effort/low-value (and needs a shipped ONNX model + binary plus agent-dispatch glue that does not exist).
