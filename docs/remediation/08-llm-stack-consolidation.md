# WS-08 — LLM Stack Consolidation

> **Status:** Not started · **Priority:** P2 · **Risk:** medium · **Effort:** L
> **Depends on:** none · **Blocks:** none (coordinates with WS-10 on shared files — see Scope)
> **Owns (symbols/files):**
> - `crates/thinclaw-llm/src/route_planner.rs` (RoutePlanner, RouteDecision, CascadePolicy, plan_cheap_split)
> - `crates/thinclaw-llm/src/smart_routing.rs` (SmartRoutingProvider — to be retired)
> - `crates/thinclaw-llm/src/provider_factory.rs` (the SmartRoutingProvider wiring block, lines ~947–997)
> - `crates/thinclaw-llm/src/rig_adapter.rs` streaming `Done` chunk finish-reason logic (the streaming fn at ~1490–1631)
> - `src/llm/runtime_manager.rs` `resolve_route` cascade-consumption logic (read-only edits to the RoutePlanner decision-consumption block ~580–656; **structural decomposition of this file is WS-10**)
> - `crates/thinclaw-tools/src/builtin/subagent.rs` `SpawnSubagentTool.executor` field (erase)
> - `src/llm/reasoning.rs` `Reasoning.safety` field (erase) — **coordinate with WS-10**, see Decision Points

## Vision & Goal

ThinClaw's headline LLM capability is intelligent, cost-aware multi-provider routing with cascade escalation, advisor-executor strategies, and accurate per-turn telemetry. Today that capability is split across two engines that drifted apart: a newer unified `RoutePlanner` (Phase 6b) that picks a provider chain, and a legacy `SmartRoutingProvider` decorator that independently re-classifies and cascades inside `complete()`. When CheapSplit is enabled, **both run, stacked**, so the planner's cost/quality decision is silently overridden by the decorator's own classifier, and the planner's computed cascade decision is thrown away. This workstream collapses the two into one authoritative engine, wires the cascade decision the planner already computes, and fixes the streaming telemetry that always reports `Stop`. The result is one routing brain whose decisions are actually executed and whose telemetry is trustworthy.

## Scope

**In scope:**
1. Resolve the dual routing engines (`SmartRoutingProvider` vs `RoutePlanner`): make `RoutePlanner` canonical, migrate the cheap/primary split + cascade behavior into the planner-driven runtime path, retire `SmartRoutingProvider`, document the decision.
2. Wire (or remove) the `CheapSplit` cascade decision computed at `route_planner.rs:565` and currently only logged at `route_planner.rs:1272`.
3. Fix native-streaming `finish_reason` so tool-call streams report `FinishReason::ToolUse` instead of always `Stop` (`rig_adapter.rs:1611`).
4. Erase `SpawnSubagentTool`'s unused `executor` field (`subagent.rs:60–68`) and `Reasoning`'s unused `safety` field (`reasoning.rs:493`) — leaky abstractions.

**Out of scope (and which WS owns it):**
- Structural decomposition of `src/llm/runtime_manager.rs` (~3100 LoC god-file) — **WS-10** (LLM runtime decomposition). WS-08 makes only localized behavior edits inside `resolve_route`; it must not move or split modules there.
- Advisor/executor consultation tool behavior (`crates/thinclaw-tools/src/builtin/advisor.rs`) — leave as-is; the planner already feeds it.
- Any change to `thinclaw-llm-core` provider traits or `FinishReason` enum shape (the enum already has the variants needed).
- The subagent *dispatch* path (`SubagentExecutor`, dispatcher interception) — WS-08 only deletes the dead field on the tool, not the spawn pipeline. Confirm with WS-10 before touching `reasoning.rs` (see Decision Points).

## Current State (verified)

**Dual engines both live, and stacked in CheapSplit mode:**
- `RoutePlanner` is the canonical Phase-6b engine. Its module header (`crates/thinclaw-llm/src/route_planner.rs:1–11`) explicitly claims it "Replaces the dual-path routing logic (SmartRoutingProvider + RoutingPolicy)". It is constructed in the runtime at `src/llm/runtime_manager.rs:972` and invoked at `src/llm/runtime_manager.rs:606` (interactive) and `:1877` (a second `plan()` call). **Wired.**
- `SmartRoutingProvider` (legacy decorator) is still defined at `crates/thinclaw-llm/src/smart_routing.rs:55` and **still constructed** at `crates/thinclaw-llm/src/provider_factory.rs:984`, gated by `smart_routing_enabled && routing_mode == CheapSplit` (`provider_factory.rs:974–976`). It is exported at `crates/thinclaw-llm/src/lib.rs:28` and re-exported at `src/llm/mod.rs:73`. **Wired (and conflicting).**
- **The stacking bug:** `build_provider_chain` returns `snapshot.llm` already wrapped in `SmartRoutingProvider` when CheapSplit is on (`provider_factory.rs:981–994`). The runtime then *also* runs `RoutePlanner::plan()` over that same chain in `resolve_route` (`runtime_manager.rs:605–644`), and `provider_chain_for_targets` resolves the planner's "primary"/"cheap" target back into the wrapped chain. Net: planner picks a target, then the decorator's `classify()` (`smart_routing.rs:88–98`) independently re-routes inside `complete()` (`smart_routing.rs:156–213`). Two classifiers, one undocumented winner.

**Cascade computed-but-dropped:**
- `plan_cheap_split` computes `cascade = CascadePolicy::InspectAndEscalate` for Moderate complexity when `cascade_enabled` (`route_planner.rs:565–569`) and stores it on `RouteDecision.cascade` (`route_planner.rs:182`).
- The only consumer of `decision.cascade` is the structured log at `route_planner.rs:1272` (`cascade = ?decision.cascade`). Verified by grep: no runtime branch reads it. `ResolvedRoute` (`runtime_manager.rs:139–142`) carries only `provider` + `telemetry_key` — the cascade decision never crosses the runtime boundary.
- The *actual* cascade escalation behavior lives instead inside `SmartRoutingProvider` (`smart_routing.rs:178–211`, with `response_is_uncertain` at `:111–143` and `cascade_escalations` stat at `:28`). So cascade works today only via the legacy decorator, not via the canonical planner. **Half-wired / drifted: the planner decides, the decorator acts.**
- For contrast, the sibling field `RouteDecision.tool_phase_synthesis` is *also* not consumed off the decision — the dispatcher recomputes it independently at `src/agent/dispatcher/loop.rs:589` via `tool_phase_synthesis_enabled(...)`. So "decision field computed but acted on elsewhere" is an established (if unfortunate) pattern here.

**Streaming finish_reason always Stop:**
- Non-streaming path correctly derives finish reason: `rig_adapter.rs:612–616` sets `FinishReason::ToolUse` when `!tool_calls.is_empty()`, else `Stop`.
- Streaming path hard-codes `finish_reason: FinishReason::Stop` in the `Done` chunk (`rig_adapter.rs:1611`), even though the same `stream!` block yields `StreamChunk::ToolCall` (`rig_adapter.rs:1555`) and `StreamChunk::ToolCallDelta` (`rig_adapter.rs:1578`). No boolean tracks whether a tool call was seen. `FinishReason` enum has the needed variants (`crates/thinclaw-llm-core/src/provider.rs:287–293`). **Bug (telemetry/artifact correctness only — the tool loop keys off the emitted `ToolCall` chunks, not this field).**

**Leaky abstractions:**
- `SpawnSubagentTool.executor: Arc<dyn SubagentToolPort>` is `#[allow(dead_code)]` (`subagent.rs:60–63`). `execute()` (`subagent.rs:166–278`) emits a JSON `{"action":"spawn_subagent", ...}` packet that "the dispatcher intercepts and routes to the SubagentExecutor" (`subagent.rs:270–275`); it never calls `self.executor`. The `SubagentToolPort` trait only exposes `list_subagents`/`cancel_subagent` (`subagent.rs:52–54`), which *are* used by `ListSubagentsTool` (`subagent.rs:327`) and `CancelSubagentTool` (`subagent.rs:408`). So only the *spawn* tool's executor is dead. **Dead field.**
- `Reasoning.safety: Arc<SafetyLayer>` is `#[allow(dead_code)]` with comment "Will be used for sanitizing tool outputs" (`reasoning.rs:492–493`). The only read of `self.safety` is propagating it into a fork (`reasoning.rs:600`); actual sanitization uses the free function `sanitize_prompt_bound_content(...)` (`reasoning.rs:529`), not the layer. It is threaded through `Reasoning::new(llm, safety)` (`reasoning.rs:556`) and ~17 construction sites. **Dead field — but removal ripples through the public constructor signature; coordinate with WS-10.**

**Build profile / feature notes:**
- `thinclaw-llm` has **no feature flags** (verified `crates/thinclaw-llm/Cargo.toml`) — all of `route_planner.rs`, `smart_routing.rs`, `provider_factory.rs`, `rig_adapter.rs` compile in every profile (edge/light/desktop/full). No `#[cfg]` gating to thread; a deletion in `smart_routing.rs` affects all profiles uniformly.
- Routing settings live in `crates/thinclaw-settings/src/providers.rs` (`smart_routing_enabled`, `routing_mode`, `smart_routing_cascade:257`, `tool_phase_synthesis_enabled`) and `crates/thinclaw-config/src/llm.rs` (`smart_routing_cascade:236`, env `SMART_ROUTING_CASCADE:522`). The `SMART_ROUTING_CASCADE` env is documented in `docs/LLM_PROVIDERS.md:525`.

## Decision Points

**DP-1 — Which routing engine is canonical?**
- **Option A (recommended): RoutePlanner is canonical; retire SmartRoutingProvider.** The planner is the explicitly-designed unified engine (its own header says it replaces the decorator), it is already invoked on the hot path, and it produces full telemetry/cost/quality scoring the decorator lacks. The decorator duplicates classification (`classify_message`) the planner already calls (`route_planner.rs:18`), and stacking the two is a latent correctness bug. **Realize the vision: finish the cutover the header promised.**
- Option B: Keep SmartRoutingProvider as the CheapSplit executor, demote RoutePlanner to PrimaryOnly/AdvisorExecutor/Policy modes. Rejected — it abandons the planner's scoring/telemetry for CheapSplit and contradicts the documented architecture direction.
- **Recommendation: A.** Migrate cascade into the planner-driven path (DP-2), then delete `SmartRoutingProvider` and its wiring.

**DP-2 — Wire vs erase the CheapSplit cascade decision (`route_planner.rs:565`).**
- **Option A (recommended): WIRE it.** The cascade is genuine, valuable capability already half-built (the decorator proves the behavior is wanted and `SMART_ROUTING_CASCADE` is a documented operator knob). Surface `decision.cascade` through `ResolvedRoute`/the runtime so the planner-driven path performs the inspect-and-escalate that today only the decorator does, reusing the decorator's `response_is_uncertain` heuristic before deleting it.
- Option B: Remove the `cascade` field entirely and rely on advisor-executor escalation. Rejected — it deletes a documented, shipping feature and an operator setting; that is the opposite of realizing the vision.
- **Recommendation: A.** This is the operator-directed default: build the wiring rather than delete the capability. (If WS-10's runtime decomposition lands first and exposes a clean escalation hook, prefer wiring through that — note the dependency, do not block on it.)

**DP-3 — Erase `SpawnSubagentTool.executor` and `Reasoning.safety`.**
- `SpawnSubagentTool.executor`: **ERASE** — genuinely dead; the spawn goes through the dispatcher action packet, not the port. Low ripple (constructor is internal-ish, one call site at `src/main.rs:1702`).
- `Reasoning.safety`: **ERASE the field, but coordinate with WS-10.** It is dead, but it sits in a constructor signature touched by ~17 sites and `reasoning.rs` is a WS-10 decomposition target. **Recommendation:** WS-08 owns the *semantic* removal (the field is dead), but sequence it so it does not collide with a WS-10 split of `reasoning.rs`. If WS-10 is splitting `reasoning.rs` in the same cycle, hand them the one-line field removal as a rider; otherwise WS-08 does it directly. Decision needed at execution time based on WS-10 scheduling.

## Tasks

- [ ] **T1: Surface the cascade decision through the runtime route resolution.**
  - **Files:** `src/llm/runtime_manager.rs` (`ResolvedRoute` struct ~139–142; `resolve_route` RoutePlanner block ~605–644).
  - **Change:** Add a `cascade: CascadePolicy` field to `ResolvedRoute` (import the type from `crate::llm::route_planner`). In the RoutePlanner block, set `cascade: decision.cascade` on the returned `ResolvedRoute`; default `CascadePolicy::None` on all other return sites (override path ~552, kill-switch path ~574, fallback ~652). Do **not** restructure the function — additive field only (WS-10 owns decomposition).
  - **Acceptance:** `ResolvedRoute` carries the planner's cascade decision; all existing callers still compile; no behavior change yet (cascade not acted on until T2).
  - **Effort:** S
  - **Verification:** `cargo build -p thinclaw && cargo clippy -p thinclaw --all-targets -- -D warnings`

- [ ] **T2: Execute inspect-and-escalate in the planner-driven completion path.**
  - **Files:** `src/llm/runtime_manager.rs` (the `RuntimeLlmProvider::complete` path that uses `resolve_route` — locate via the `complete`/`complete_with_tools` impls; the non-streaming `complete` is the cascade target since the decorator only cascaded on `complete()`, not `complete_with_tools` per `smart_routing.rs:215–227`). Reuse logic from `crates/thinclaw-llm/src/smart_routing.rs:111–143` (`response_is_uncertain`).
  - **Change:** When `resolved.cascade == CascadePolicy::InspectAndEscalate` and the resolved target is the cheap lane: run the cheap completion, inspect with a moved/shared `response_is_uncertain`, and on uncertainty re-issue against the primary chain (resolve "primary" via `provider_chain_for_targets`). Move `response_is_uncertain` into a shared location in `crates/thinclaw-llm` (a small `cascade.rs` module or pub(crate) fn on the planner) — do **not** leave it stranded in `smart_routing.rs` since that file is deleted in T4. Emit a tracing event mirroring `smart_routing.rs:189–193` and record an escalation counter if one is available; otherwise log only.
  - **Acceptance:** With `routing_mode = CheapSplit`, `smart_routing_cascade = true`, and a Moderate-complexity turn whose cheap response trips `response_is_uncertain`, the runtime re-issues against primary. A unit test asserts escalation fires for an uncertain cheap response and does not fire for a confident one.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw-llm cascade` and `cargo test -p thinclaw runtime` (add a focused test); `cargo clippy -p thinclaw-llm --all-targets -- -D warnings`

- [ ] **T3: Make the planner own the cheap/primary classification (stop double-routing).**
  - **Files:** `crates/thinclaw-llm/src/provider_factory.rs` (smart-routing wiring ~947–997).
  - **Change:** Stop wrapping the chain in `SmartRoutingProvider`. Keep building `cheap_llm` (still returned as the second tuple element and used widely — see `src/app.rs:483`, `runtime_manager.rs:970`), but the `if smart_routing_enabled { Arc::new(SmartRoutingProvider::new(...)) }` block (`:981–994`) becomes just `llm` (no decorator). The classification + cascade now live exclusively in the planner path (T1/T2). Update the doc comment at `:854–862` (drop step 4 "SmartRoutingProvider").
  - **Acceptance:** `build_provider_chain` no longer constructs `SmartRoutingProvider`; `cheap_llm` is still returned; CheapSplit routing now flows solely through `RoutePlanner` → cheap/primary target → `provider_chain_for_targets`. No stacked re-classification.
  - **Effort:** M
  - **Verification:** `cargo build -p thinclaw-llm`; grep shows zero `SmartRoutingProvider::new` callers remain in non-test code: `rg -n "SmartRoutingProvider::new" --type rust src/ crates/`

- [ ] **T4: Delete `SmartRoutingProvider` and its exports.**
  - **Files:** `crates/thinclaw-llm/src/smart_routing.rs` (struct + impl `:55–...`, stats `:24–49`); `crates/thinclaw-llm/src/lib.rs:28` (`pub use smart_routing::SmartRoutingProvider`); `src/llm/mod.rs:73` (re-export). Keep `SmartRoutingConfig`/`TaskComplexity`/`classify_message` (owned by `thinclaw-llm-core::smart_routing` and re-exported — the planner depends on them via `route_planner.rs:18`); only the *provider decorator* and its stats/snapshot types are removed.
  - **Change:** Remove `SmartRoutingProvider`, `SmartRoutingStats`, `SmartRoutingSnapshot`, and the now-orphaned `response_is_uncertain` (moved in T2). Fix the two `pub use` lines to drop the decorator while preserving the still-used config/classifier re-exports. Update `provider_factory.rs:20` import.
  - **Acceptance:** `SmartRoutingProvider` no longer exists; `SmartRoutingConfig`/`TaskComplexity`/`classify_message` still resolve for the planner; workspace compiles.
  - **Effort:** S
  - **Verification:** `rg -n "SmartRoutingProvider|SmartRoutingSnapshot|SmartRoutingStats" --type rust` returns no definitions/uses; `cargo build --workspace`

- [ ] **T5: Fix streaming `finish_reason` to map real reasons.**
  - **Files:** `crates/thinclaw-llm/src/rig_adapter.rs` (streaming fn ~1490–1631).
  - **Change:** Add `let mut saw_tool_call = false;` before the `stream!` block (near `:1524`). Set `saw_tool_call = true;` in the `StreamedAssistantContent::ToolCall` arm (`:1545`) and the `ToolCallDelta` arm (`:1561`). In the `Final` arm `Done` chunk (`:1606–1613`), replace `finish_reason: FinishReason::Stop` with `finish_reason: if saw_tool_call { FinishReason::ToolUse } else { FinishReason::Stop }` — mirroring the non-streaming derivation at `:612–616`. (If the upstream `resp` exposes a provider-native finish reason, prefer mapping that and fall back to the `saw_tool_call` heuristic; otherwise the heuristic is sufficient and matches the existing non-streaming behavior.)
  - **Acceptance:** A streaming completion that emits tool-call chunks ends with `FinishReason::ToolUse`; a plain text stream ends with `Stop`. Add/extend a streaming test alongside `rig_adapter.rs:1960`/`:1972` style assertions.
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw-llm rig_adapter` (or the streaming-specific test); `cargo clippy -p thinclaw-llm --all-targets -- -D warnings`

- [ ] **T6: Erase `SpawnSubagentTool.executor` (dead field).**
  - **Files:** `crates/thinclaw-tools/src/builtin/subagent.rs:60–68`; call site `src/main.rs:1702`.
  - **Change:** Remove the `executor` field, drop the `#[allow(dead_code)]`, and make `SpawnSubagentTool::new()` take no args (or keep a unit struct). Update `src/main.rs:1702` to `SpawnSubagentTool::new()` and drop the now-unused `Arc::clone(&subagent_port)` only if it has no other consumer at that site (verify `subagent_port` is still used by List/Cancel tools nearby before deleting the clone). Leave `SubagentToolPort`, `ListSubagentsTool`, `CancelSubagentTool` untouched (their executor is live).
  - **Acceptance:** `SpawnSubagentTool` has no `executor`; `execute()` unchanged (still emits the action packet); `subagent_port` still wired to List/Cancel tools.
  - **Effort:** S
  - **Verification:** `cargo build -p thinclaw-tools && cargo build -p thinclaw`; `rg -n "SpawnSubagentTool" --type rust` shows the new signature everywhere.

- [ ] **T7: Erase `Reasoning.safety` (dead field) — coordinate with WS-10 (DP-3).**
  - **Files:** `src/llm/reasoning.rs` (field `:492–493`, constructor `:556–575`, fork propagation `:600`, ~17 `SafetyLayer::new` construction sites in tests `:2207+`); import `:21`.
  - **Change:** Remove the `safety` field and its `Arc<SafetyLayer>` constructor parameter; drop `:600` propagation; update `Reasoning::new` to `new(llm: Arc<dyn LlmProvider>)`; update every caller (production + the ~17 test constructions). Drop the `SafetyLayer` import at `:21` if `sanitize_prompt_bound_content` is imported separately (it is). **Do not** touch `crates/thinclaw-safety` (that is separate WS-11 dead-`src/safety` cleanup territory) — only remove the unused dependency edge from `Reasoning`.
  - **Acceptance:** `Reasoning` no longer references `SafetyLayer`; sanitization via `sanitize_prompt_bound_content` is unchanged; all callers updated.
  - **Effort:** M (signature ripple across ~17 sites)
  - **Verification:** `cargo build -p thinclaw && cargo test -p thinclaw reasoning`; `rg -n "self\.safety|Reasoning::new" --type rust src/llm/reasoning.rs`
  - **Sequencing note:** If WS-10 is decomposing `reasoning.rs` this cycle, fold this one-line field removal into their split to avoid a merge collision; otherwise WS-08 executes it directly.

- [ ] **T8: Document the routing-engine decision and update operator docs.**
  - **Files:** `docs/LLM_PROVIDERS.md` (routing section; `SMART_ROUTING_CASCADE` table row `:525`); the `route_planner.rs:1–11` header (drop the "Replaces … (SmartRoutingProvider…)" past-tense if the decorator is now actually gone — make it present-tense "is the single routing engine"). Check `FEATURE_PARITY.md` for any SmartRoutingProvider/routing-engine claim and update if present.
  - **Change:** State that `RoutePlanner` is the single routing engine, that cascade is now executed by the planner-driven runtime path, and that `SMART_ROUTING_CASCADE` still controls inspect-and-escalate. Remove any doc text implying a separate `SmartRoutingProvider` decorator.
  - **Acceptance:** Docs match code; no reference to a live `SmartRoutingProvider` decorator remains; `SMART_ROUTING_CASCADE` semantics documented against the new path.
  - **Effort:** S
  - **Verification:** `rg -ni "SmartRoutingProvider" docs/ FEATURE_PARITY.md` returns no stale "decorator" claims; manual read of the routing section.

## Best Practices (workstream-specific)

- **Follow the existing decorator-vs-planner separation correctly.** Decorators (`RetryProvider`, `CachedProvider`, `CircuitBreakerProvider`) wrap *reliability/caching* concerns that are orthogonal to routing. `SmartRoutingProvider` was a *routing* concern masquerading as a decorator — that is exactly the anti-pattern being removed. Keep routing decisions in `RoutePlanner`; keep cross-cutting reliability in decorators.
- **Mirror the non-streaming finish-reason derivation** (`rig_adapter.rs:612–616`) for the streaming fix — same boolean-on-tool-calls logic, so the two paths agree.
- **Preserve public paths during the `smart_routing.rs` cut** using `pub use` for the still-needed `SmartRoutingConfig`/`TaskComplexity`/`classify_message` (these are owned by `thinclaw-llm-core` per the import at `route_planner.rs:18`). Per CLAUDE.md, do not widen visibility just to make the split compile — `response_is_uncertain` should move at `pub(crate)`.
- **Additive, not structural, edits to `runtime_manager.rs`.** That god-file is WS-10's decomposition target; WS-08 adds one field to `ResolvedRoute` and one escalation branch. Leave the boundaries obvious so WS-10 can extract cleanly.
- **Reuse `provider_chain_for_targets`** (`runtime_manager.rs`, called at `:626`) to resolve the "primary" escalation target in T2 — do not hand-roll provider lookup.
- Cite the wired sibling `tool_phase_synthesis` (`dispatcher/loop.rs:589`) only as a *contrast*, not a pattern to copy — recomputing decisions outside the planner is the drift we are reducing, not extending.

## Common Pitfalls

- **Deleting `SmartRoutingConfig`/`TaskComplexity`/`classify_message` along with the decorator.** These are owned by `thinclaw-llm-core::smart_routing` and consumed by `RoutePlanner` (`route_planner.rs:18`). Only the *decorator* and its stats types die. Verify with grep before deleting.
- **Removing the cascade decorator without wiring the planner path first.** Doing T3/T4 before T1/T2 silently drops cascade escalation entirely — a functional regression on a documented operator setting (`SMART_ROUTING_CASCADE`). Honor task order.
- **Editing only one of the two `plan()` call sites.** The runtime calls `plan()` at both `runtime_manager.rs:606` and `:1877`; cascade consumption must be considered for whichever path performs `complete()`. The audit's recurring failure mode is fixes landing in one of N copies (e.g. the `split_message` UTF-8 fix landing in 1 of 4 WASM channels) — check both.
- **Touching `crates/thinclaw-safety` while removing `Reasoning.safety`.** The dead `SafetyLayer` *usage* in `Reasoning` is WS-08; the broader dead `src/safety/*` cleanup is a different workstream. Remove only the unused edge.
- **Treating the finish_reason fix as behavior-affecting for the tool loop.** It is **telemetry/artifact-only**: the agent loop reacts to emitted `StreamChunk::ToolCall` chunks, not the `Done.finish_reason`. Do not "fix" the loop to read this field.
- **Structurally refactoring `runtime_manager.rs`.** That is WS-10. A WS-08 PR that moves modules there will collide.

## Multi-Worker Execution Plan (ultracode)

- **Worker decomposition:**
  - **Sequential spine (one worker):** T1 → T2 → T3 → T4 (the routing-engine consolidation must be ordered; each step depends on the prior). T8 (docs) follows T4.
  - **Parallel-safe (separate workers/worktrees):** T5 (streaming finish_reason — isolated to `rig_adapter.rs`) and T6 (`SpawnSubagentTool` — isolated to `subagent.rs` + `main.rs:1702`) have no overlap with the spine and can run concurrently.
  - **Gated/coordinated:** T7 (`Reasoning.safety`) touches a WS-10 file; schedule it after confirming WS-10's `reasoning.rs` status (DP-3). Run it solo to avoid clobbering a concurrent decomposition.
- **Isolation:** Use **git worktree isolation** for the three lanes (spine T1–T4/T8, T5, T6) since they mutate disjoint files in parallel; merge spine first, then T5/T6, then T7 last. Within the spine, a single worktree is fine (strictly sequential).
- **Workflow shape:**
  1. **Implement (fan-out 3):** spine worker (T1→T4, then T8); rig_adapter worker (T5); subagent worker (T6).
  2. **Verify (per worker):** crate-scoped build + clippy + tests (commands below) before merge.
  3. **Integrate:** merge spine → main branch, rebase T5/T6 lanes, re-verify workspace build.
  4. **Coordinate gate:** resolve DP-3 with WS-10, then implement T7 on a clean tree.
  5. **Review:** `/code-review` on the combined diff focusing on the routing cutover (T2/T3) for regressions.
  6. **Fix:** address review findings; re-run the full gate.
- **Verification gate (exact):**
  - `cargo fmt`
  - `cargo clippy --all --benches --tests --examples --all-features` (matches CLAUDE.md) — and per-crate `cargo clippy -p thinclaw-llm --all-targets -- -D warnings`, `cargo clippy -p thinclaw-tools --all-targets -- -D warnings`
  - `cargo test -p thinclaw-llm` (cascade + rig_adapter tests), `cargo test -p thinclaw-tools`, `cargo test -p thinclaw` (runtime/reasoning tests)
  - `/ship` for the full Rust quality gate; `/code-review` (medium) on the final diff.
  - **DB/Docker prerequisites:** none for WS-08 — the routing/streaming/subagent paths are not Postgres/libSQL-backed and need no migrations or containers. (Standard caveat from CLAUDE.md applies only if a chosen test pulls in DB-backed integration fixtures, which these do not.)

## Definition of Done

- [ ] `RoutePlanner` is the sole routing engine; `SmartRoutingProvider` (decorator + stats + snapshot) is deleted and exports removed (T3, T4).
- [ ] CheapSplit cascade decided by the planner is actually executed in the runtime completion path (T1, T2) — escalation fires on uncertain cheap responses and is covered by a unit test.
- [ ] No stacked double-classification remains (`build_provider_chain` no longer wraps `SmartRoutingProvider`; grep clean).
- [ ] Streaming `finish_reason` reports `ToolUse` when tool-call chunks were emitted, `Stop` otherwise, with test coverage (T5).
- [ ] `SpawnSubagentTool.executor` and `Reasoning.safety` dead fields removed; all call sites updated (T6, T7).
- [ ] DP-1, DP-2, DP-3 resolved and recorded; routing-engine decision documented in `docs/LLM_PROVIDERS.md` and the `route_planner.rs` header; `FEATURE_PARITY.md` updated if it referenced the old engine (T8).
- [ ] `cargo fmt` clean; `cargo clippy --all --benches --tests --examples --all-features` clean (`-D warnings`); `cargo test -p thinclaw-llm -p thinclaw-tools -p thinclaw` green.
- [ ] No structural edits leaked into `src/llm/runtime_manager.rs` beyond the additive `ResolvedRoute.cascade` field and escalation branch (WS-10 ownership respected).
