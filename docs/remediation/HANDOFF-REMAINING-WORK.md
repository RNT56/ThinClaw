# ThinClaw — Remaining Work Handoff (CLOSED)

> **STATUS — CLOSED 2026-06-26. Do NOT execute anything in this document.**
> This was the execution-ready handoff for the `F-xx` residuals that trailed the
> 13-workstream remediation. Every lane below has since been resolved. The
> authoritative, current record is the resolution table in
> [`FOLLOWUPS.md`](./FOLLOWUPS.md) (2026-06-26 pass); this file is retained only as a
> historical record of what the handoff proposed and how each item actually closed.
>
> Only two residuals remain open, both tracked in `FOLLOWUPS.md`:
> **`F-13`** (opendal/S3 object-store `ArtifactStore` backend — deferred) and
> **`F-09`** (full shared-`include!` SDK extraction — partial; the byte-identical
> drift-guard test is in place). Everything else on this page is DONE.
>
> The instructions that follow in the earlier revision described work that no longer
> exists to do, and several of their premises were already false when written (see the
> "Corrections" column). They have been replaced by the closed ledger below.

---

## Resolution ledger

Each F-item, its outcome, and the code evidence. Where the old handoff asserted a
capability was missing, the correction records that it was in fact already present (or
landed in the follow-up pass).

| Item | Outcome | Evidence / correction |
|---|---|---|
| **F-01** — experiments LocalDocker credential resolver | ✅ Done | `experiment_execution_backend` chains `.with_credential_store(store, user_id)` via the registered `research_secrets_store()` (env fallback when unset) — `src/api/experiments/git.rs:133`. |
| **F-02** — MCP HTTP/OAuth DNS-rebind pin | ✅ Done | Streamable-HTTP client + OAuth clients pin DNS through `build_pinned_mcp_client` — `crates/thinclaw-tools/src/mcp/client.rs:464` (loopback/IP-literal hosts fall back to unpinned, no regression). |
| **F-03** — dedupe OAuth state compare onto `subtle` | ✅ Done. **Correction:** the old claim that "`subtle` is *not* a dep of `thinclaw-tools`" was false — `subtle = "2"` is present. | `crates/thinclaw-tools/Cargo.toml:34`; `oauth_state_matches` uses `ConstantTimeEq`. |
| **F-04** — shell-scanner health line in `thinclaw status` | ✅ Done | `run_status_command` prints a "Shell scanner" line (mode/reachable/fail-open/last_error) — `src/cli/status.rs:161-176`. |
| **F-05** — `search_files` base_dir | ✅ Accepted no-op | Consistent with `file.rs`; no action. |
| **F-06** — concrete `SubagentRepoTaskPlanner` + kill switch | ✅ Done. **Correction:** the old claim that "`REPO_PROJECTS_AUTOPLAN` does not exist yet (greenfield)" was false — the adapter and env gate both exist. | `SubagentRepoTaskPlanner` at `src/repo_projects/subagent_planner.rs:20`, injected opt-in behind `REPO_PROJECTS_AUTOPLAN` at `src/agent/agent_loop/mod.rs:1157`; `AwaitingHuman` fallback in place. |
| **F-07** — WebUI SSE consumer for repo-project events | ✅ Done | `app.js` listens for the 5 wire events (`repo_project_updated`, `repo_task_updated`, `repo_worker_run_updated`, `repo_project_event`, `repo_merge_gate_updated`) → debounced dashboard refresh — `crates/thinclaw-gateway/src/web/static/app.js:993`. |
| **F-08** — channel broadcast for light-context worker heartbeats | ✅ Done | `notify_tx: Option<mpsc::Sender<OutgoingResponse>>` threaded into `WorkerDeps` — `src/agent/worker.rs:84`; `target=<channel>` heartbeats broadcast via the agent-loop forwarder. |
| **F-09** — shared WASM channel/tool SDK | 🟡 **Partial (open)** | Byte-identical drift-guard test added for the github↔notion helpers; full shared-`include!` extraction remains. Tracked in `FOLLOWUPS.md`. |
| **F-10** — thin-shim `production_status` + real sink | ✅ Done. **Correction:** the old claim that "`production_status` exists in zero capabilities.json and is not a field on `ChannelCapabilitiesFile`" was false. | Typed field at `crates/thinclaw-channels/src/wasm/schema.rs:82`; present in all 16 `*.capabilities.json`; catalog test enforces it. |
| **F-11** — observability per-turn/per-tool events | ✅ Done. **Correction:** the old claim that "`AgentDeps` has no observer field / the agent loop never sees it" was false. | `pub observer: Arc<dyn crate::observability::Observer>` at `src/agent/agent_loop/mod.rs:134`, threaded through the loop; emits `ToolCallStart/End`, `LlmResponse`, `TurnComplete` (no-op safe). |
| **F-12** — native-plugin safety docs | ✅ Done (docs) | `EXTENSION_SYSTEM.md` panic-isolation + in-process caveat; `NETWORK_SECURITY.md` native-plugin trust boundary; `FEATURE_PARITY.md` row. Gateway exposure intentionally NOT added. |
| **F-13** — object-store `ArtifactStore` backend | ⏸️ **Deferred (open)** | Needs the heavy `opendal` dependency; deferred to avoid risking the `cargo deny` gate without a dedicated dependency/license review. Port + reaper unchanged. Tracked in `FOLLOWUPS.md`. |
| **F-14** — experiments error-taxonomy normalization | ✅ Done | Worktree/branch `Internal`→`InvalidInput` normalized (7 sites) plus a bounded audit of the remaining `Internal` sites (each genuinely wraps a downstream fault). `ApiError` has no `NotFound`/`Conflict`. |
| **F-15** — skill parameters from `save_skill` | ✅ Done | `save_skill(id, script, description, params)` overload populates `SkillManifest.parameters`; 3-arg form unchanged. |
| **F-16** — quarantine live-network desktop tests | ✅ Done | `#[ignore]` at `apps/desktop/backend/src/rig_lib/tools/web_search.rs:667`; nightly `--ignored` job runs it. |
| **F-17** — CI ALSA for the `full` profile | ✅ Resolved-by-reversal | `voice` is intentionally **not** in `full` (`Cargo.toml:335` full = light/acp/web-gateway/repl/tunnel/docker-sandbox/browser/nostr). CI installs `libasound2-dev` only for `all-features`. **Do NOT add ALSA to the `full` leg.** |
| **F-18** — STT capture-on-wake glue | ✅ Done. **Correction:** the old claim that "`capture_and_transcribe` does not exist / dispatch cannot be threaded" was false. | `pub async fn capture_and_transcribe` at `src/talk_mode.rs:699`; the voice-wake path transcribes and injects an `IncomingMessage` on the synthetic `voice` channel at `src/async_main.rs:1120-1123`. |
| **F-19** — voice config promotion | ✅ Done | `VoiceWakeConfig::from_env()` typed env overlay replaces `::default()` — `src/voice_wake.rs:79`. A true keyword model stays documented future work. |
| **Baseline** — DB-backend profile guard | ✅ Done | `compile_error!` DB-backend guard in `src/lib.rs` locks `edge`/libSQL as the minimum profile. |

## Also-done items from the original orientation

- **image_gen divide-by-zero** — fixed (`apps/desktop/backend/src/image_gen.rs` zero-guard).
- **Shim CI-compile gate** — all 12 shims build to `wasm32-wasip2` in CI.
- **Shim config-load round-trip test** — `tests/registry_channel_catalog.rs` parses every shim's `*.capabilities.json`.
- **`api/experiments` god-file split** — done (directory module `src/api/experiments/`, 9 modules).

## What is still open

Only the two residuals above: **F-13** (deferred) and **F-09** (partial). Both live in
[`FOLLOWUPS.md`](./FOLLOWUPS.md); when either is picked up it inherits the same-PR
canonical-doc rule from `CLAUDE.md`. Do not treat any other item on this page as
schedulable work — the numbered `WS-01`…`WS-13` docs remain frozen "Status: Not started"
by design, and [`README.md`](./README.md)'s Status Tracker is the live roll-up.
