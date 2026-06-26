# Follow-ups Ledger (non-deletion deferred work)

> Tracks work that was intentionally scoped-down during a wave because completing it required reaching outside the current change's safe boundary (a larger refactor, cross-crate plumbing, or another workstream's files). These are **not** deletions (see `DEFERRED-DELETIONS.md` for those) — they are remaining steps to fully close a fix.
>
> Each entry: what's done, what remains, and what it takes.

## Wave 0 — WS-01 (security)

### F-01 — Sandbox credential resolver: one runtime path still env-backed
- **Done:** `SandboxManager::with_credential_store` + `AppBuilder::build_sandbox_manager` wire the encrypted `SecretsStore` (owner `"default"`) into all three production sandbox-construction sites; the proxy now resolves credentials via the audited `get_for_injection` path. Env resolver retained as fallback.
- **Remains:** `src/api/experiments.rs` `experiment_execution_backend` (LocalDocker branch, ~:4144) builds a `DockerSandboxExecutionBackend` whose proxy still uses the env resolver — no `SecretsStore` is in scope in that free function.
- **Takes:** thread an `Arc<dyn SecretsStore + Send + Sync>` (+ owning user) through `experiment_execution_backend` and its callers, then call `NetworkProxyBuilder::from_config_with_store`. Coordinate with WS-07 (experiments) since it touches that file.

### F-02 — DNS-rebind pin: MCP HTTP client path still unpinned
- **Done:** the builtin HTTP tool, `extract_document`, the WASM HTTP host, and the WASM OAuth token-refresh client all now connect to the validated pinned IPs via `reqwest` `resolve_to_addrs`.
- **Remains:** the MCP Streamable-HTTP client (`crates/thinclaw-tools/src/mcp/client.rs`, long-lived `http_client` built ~:577, requests ~:770/813/858) and the MCP OAuth clients in `mcp/auth.rs` (~:366/402/489) are not pinned. `mcp/config.rs` validation is correctly left alone (no connection there).
- **Takes:** re-validate `self.server_url` with `validate_outbound_url_pinned` and thread pinned addrs into a per-request or rebuilt client at the send sites. Larger because the client is shared across many requests.

### F-03 — Shared constant-time state comparator (consistency, not a gap)
- **Done:** OAuth `state` is now generated + constant-time-validated end-to-end in both WASM-tool flow drivers; the MCP callback check was upgraded from plain `!=` to a local constant-time comparator.
- **Remains (optional):** `crates/thinclaw-tools/src/mcp/auth.rs` uses a local hand-rolled constant-time comparator because `subtle` isn't a dep of that crate, while `src/cli/oauth_defaults.rs` uses `subtle::ConstantTimeEq`.
- **Takes:** add `subtle` to `thinclaw-tools/Cargo.toml` and dedupe onto one shared helper. Low priority.

### F-04 — CLI status surface for shell scanner
- **Done:** `ShellTool::scanner_status()` exposes external-scanner health/fail-open state in the tool's `execute` output.
- **Remains:** wire a "Shell scanner" line into `src/cli/status.rs` `run_status_command` so an operator sees scanner mode/reachability in `thinclaw status`.
- **Takes:** small CLI edit (out of the tool crate's lane).

### F-05 — `search_files` base_dir consistency
- **Note:** `crates/thinclaw-tools/src/builtin/search_files.rs` has its own `validate_path` duplicate. Now consistent with `file.rs` again (both treat `None` base as unrestricted). If a future change tightens containment, consolidate both onto the shared `file.rs::validate_path` and have search pass an explicit base. No action needed now.

---

## Wave 1 — WS-06 (repo-project supervisor)

### F-06 — LLM-backed RepoTaskPlanner adapter
- **Done:** the `RepoTaskPlanner` port + `PlannedTask` DTO + `with_planner` wiring + the no-planner `AwaitingHuman` fallback are shipped; `NeedsPlanning` now always acts (no silent stall). Wiring point at `agent_loop.rs` passes `with_planner(None)`.
- **Remains:** the concrete adapter that spawns a one-shot planning subagent (`SubagentExecutor::spawn` + planner prompts) and a `REPO_PROJECTS_AUTOPLAN` opt-in.
- **Takes:** a focused pass pulling the LLM/subagent stack into the supervisor wiring + an integration test with a stub subagent.

### F-07 — WebUI SSE consumer for repo-project events
- **Done:** the backend emits `RepoTaskUpdated`/`ProjectStateChanged`/`TaskCreated` in lockstep with state changes.
- **Remains:** the frontend consumer in `crates/thinclaw-gateway/src/web/static/app.js` to render live supervisor progress.
- **Takes:** small frontend addition; independent of backend.

## Wave 1 — WS-09 (routines/heartbeat)

### F-08 — Full channel-broadcast routing for worker heartbeats
- **Done:** heartbeat `target` (none/chat/channel) + `include_reasoning` are honored — `target=none` suppresses the SSE delivery (output still recorded), `target=<channel>` tags the SSE summary, `include_reasoning` is enforced via the prompt + metadata.
- **Remains:** true channel broadcast for light-context worker heartbeats requires adding a `notify_tx` to `WorkerDeps` (the worker has no broadcast seam today; only the agent-loop forwarder reads `notify_channel`).
- **Takes:** thread a notification sender into `WorkerDeps` (cross-cutting in the worker runtime).

---

## Wave 1 — WS-03 (WASM channels)

### F-09 — Shared channel/tool SDK extraction (kills copy-paste drift)
- **Done:** the `split_message` UTF-8 fix is now correct in all four channels (telegram/slack/discord/whatsapp); Discord Ed25519 verification is implemented end-to-end.
- **Remains:** `split_message`/`byte_index_for_char_limit`/`json_response`/`conversation_scope_id`/`external_conversation_key` (channels) and `url_encode_path`/`validate_input_length` (tools) are still duplicated byte-for-byte across the `channels-src/*` and `tools-src/*` crates — the exact pattern that caused the panic fix to originally land in only one copy.
- **Takes:** extract a shared `include!`-style source module (mirroring `channels-src/shared_webhook_channel`) consumed by each standalone crate. Deferred to the **Wave 3 architecture pass** (it's a dedup/maintainability refactor, not a correctness fix). Decision register WS-03 "shared SDK packaging" = Option B (`include!`).

### F-10 — Thin-shim channel dispositions
- **Done:** all 13 thin-shim channels classified — see [WS-03-shim-classification.md](./WS-03-shim-classification.md).
- **Remains:** for each shim, execute the recommendation (finish the gap, or mark non-production in its capabilities manifest + README).
- **Takes:** per-channel work sized in the classification doc; schedule alongside Wave 3 or as opportunistic follow-ups.

---

## Wave 1 — WS-05 (self-repair / extensions / native plugins)

### F-11 — Observability event emission beyond startup
- **Done:** `create_observer` is wired through `AppBuilder`, the configured observer is constructed, stored on `AppComponents`, and emits a startup `AgentStart` event (the wizard/`OBSERVABILITY_BACKEND` choice now has effect).
- **Remains:** route the stored observer into per-turn/per-tool event sites (`ToolCallStart/End`, `LlmRequest/Response`, `TurnComplete`) so events flow beyond startup. The plan scoped these emission sites as optional.
- **Takes:** thread `AppComponents.observer` into the agent loop's event points.

### F-12 — Native-plugin gateway exposure + safety docs
- **Done:** native plugins are reachable via `ExtensionKind::NativePlugin` (operator-only), default-off, signature-gated (all gates run before dlopen), `catch_unwind`-isolated, with a startup allowlist scan. Security model documented in code comments.
- **Remains:** (a) if native install/activate should be drivable from the web gateway, add `ExtensionKindHint::NativePlugin` to `crates/thinclaw-gateway/src/web/extensions.rs` + the `src/api/extensions.rs` mapping arm (deliberately NOT exposed today — operator-only); (b) mirror the in-code safety model into `docs/EXTENSION_SYSTEM.md` + `src/NETWORK_SECURITY.md` (signature/ABI/allowlist/SHA-256/panic-isolation/default-off + the in-process non-sandboxed caveat) and a `FEATURE_PARITY.md` row — WS-12 doc lane.
- **Note:** native plugins run **in-process with full host privilege** (not WASM-sandboxed). This is the documented trade-off of native extensibility; the signature gate + default-off + operator-allowlist are the controls.

---

## Wave 2 — WS-07 (experiments)

### F-13 — Durable artifact storage: object-store backend
- **Done:** `ArtifactStore` port + `LocalArtifactStore` (host-disk) impl; remote-runner artifacts upload through it (content_base64, 16 MiB inline cap) and record `fetchable:true`; a daily retention reaper prunes expired artifacts; RunPod credit≈USD surfaced in `cost_summary`.
- **Remains:** an opendal/S3 `ArtifactStore` impl for cloud durability (the port is shaped to accept it). Over-cap artifacts still fall back to the pod-local breadcrumb.
- **Takes:** implement the object-store backend behind the existing port. Decision register WS-07 DP-1 Option B.

### F-14 — Experiments error-taxonomy + god-file split
- **Remains:** the ~106 `map_err(ApiError::Internal)` flattening and the Internal-vs-InvalidInput inconsistency for "campaign missing worktree/branch" (lines ~1823/3181/… vs ~1915/2333) plus the structural split of `src/api/experiments.rs` (5400+L). Left additive/rebase-friendly.
- **Takes:** WS-10 architecture pass (the Wave 2 work deliberately stayed additive for this).

## Wave 1 — WS-04 (desktop)

### F-15 — Skill parameters from `save_skill`
- **Done:** `sandbox_factory` `tools_used` is best-effort populated.
- **Remains:** `SkillManifest.parameters` is left empty because the Rhai `save_skill(id, script, description)` builtin has no parameter input.
- **Takes:** define a skill-parameter convention + extend the builtin signature (out of scope this pass).

## Wave 2 / cross — test infra (WS-13)

### F-16 — Quarantine live-network desktop tests
- **Observed:** `apps/desktop/backend` `rig_lib::tools::web_search::tests::test_ddg_search_with_scraping` does a live DuckDuckGo search+scrape and fails offline / under anti-bot variance (pre-existing, untouched by WS-04).
- **Takes:** `#[ignore]` it (and audit sibling live tests) so the desktop test suite is deterministic offline; run live tests in a dedicated nightly job. WS-13.

---

## Wave 4 — WS-11 voice_wake (now WIRED)

### F-17 — CI ALSA for the `full` profile
- `voice` is now in the `full` feature, so the Linux `full` build/check jobs need `libasound2-dev` + `pkg-config` (cpal). Update `.github/workflows/ci.yml` (the "Install Linux all-features packages" step ~:112) to also run for `matrix.name == 'full'`. macOS builds fine (CoreAudio). Out of the implementation lane (CI yaml).

### F-18 — STT capture-on-wake glue
- `VoiceWakeRuntime` is constructed + started under `#[cfg(feature="voice")]` + `THINCLAW_VOICE_WAKE`, and `WakeWordDetected` reaches a dispatch seam in `src/app.rs` that currently logs. Remaining: on wake, capture+transcribe the follow-up utterance (reuse `talk_mode.rs`/STT) and route the transcript into the agent dispatcher — needs a dispatcher/session handle threaded into the spawn block.

### F-19 — voice config promotion + true keyword model
- Gating is via `THINCLAW_VOICE_WAKE` env (no typed config). Optionally add a `VoiceConfig` to `thinclaw-config`. Also: a true "hey thinclaw" phrase (vs the default `EnergyDetector` speech-VAD) needs a shipped `sherpa-onnx-keyword-spotter` binary + ONNX model + keywords.txt, then switch `WakeBackend` to `SherpaOnnx`.

> **Done this wave:** `history/store` consolidation (WIRE) — root `src/history/store` + `analytics.rs` + `experiments.rs` deleted, all 6 callers redirected onto `thinclaw-db` `PgBackend`, `src/history/mod.rs` now a thin DTO façade. One persistence implementation remains.

---

*Add follow-ups under the owning wave as they arise. Resolve or explicitly accept each before declaring a workstream done.*
