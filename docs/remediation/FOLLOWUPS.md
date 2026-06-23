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

*Add follow-ups under the owning wave as they arise. Resolve or explicitly accept each before declaring a workstream done.*
