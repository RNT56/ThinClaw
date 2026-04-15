# Codebase Audit Results

Audit date: 2026-04-15

Source plan: [CODEBASE_AUDIT_PLAN.md](./CODEBASE_AUDIT_PLAN.md)

This report consolidates the multi-worker audit of the ThinClaw repo. It is a static code/docs audit. No full test suite was run as part of this pass.

## Audit Coverage

The audit executed these lanes:

1. Bootstrap, config, onboarding, and app assembly
2. CLI, service, REPL, TUI, and shared command vocabulary
3. Web gateway and business API
4. Native channels
5. WASM channels and packaged channel registry
6. Agent runtime core
7. LLM routing and provider runtime
8. Workspace, context, built-in tools, and tool policy
9. WASM tools, MCP, extensions, registry, skills, and hooks
10. Safety, secrets, pairing, platform, observability, identity, media, and document extraction
11. Experiments, orchestration, workers, history, estimation, and evaluation
12. Docs, parity, and repo hygiene

## Highest-Priority Findings

### P1 / High

- `src/safety/media_url.rs`
  IPv6 private-address filtering is incomplete. ULA (`fc00::/7`), unspecified (`::`), and multicast IPv6 addresses can slip past the private-IP guard, which looks like an SSRF bypass.

- `src/main.rs`, `src/main_helpers.rs`, `src/config/mod.rs`
  First-run detection happens before full config resolution and only checks a narrow env/path set. TOML or persisted config can still be misclassified as “Database not configured,” forcing unnecessary onboarding.

- `src/channels/wasm/channel_watcher.rs`
  Hot-reloaded WASM channels are added to `ChannelManager` but do not register webhook routes, so inbound `/webhook/<name>` delivery can 404 after a hot-add.

- `src/tui/mod.rs`
  Typed `/quit` and `/exit` are advertised in the TUI but do not actually exit.

- `src/tools/policy.rs` with runtime call sites in `src/agent/{dispatcher,worker,scheduler,thread_ops}.rs`
  `ToolPolicyManager` exists but is not consulted by real execution paths, so channel/group tool policy is effectively inert.

- `src/llm/runtime_manager.rs`
  Single-target runtime routes skip `RetryProvider`, so retries/cooldowns only apply when multiple providers resolve.

- `src/experiments/runner.rs`
  Failed remote trials can return early without posting terminal completion, leaving runs in claimed/running state until stale-lease reconciliation.

- `deploy/setup.sh`, `deploy/docker-compose.yml`
  Deploy docs/scripts reference `.env.template`, but the repo ships `.env.example` and `deploy/env.example`. Fresh deploys can fail immediately on missing bootstrap files.

- `src/channels/http.rs` vs `channels-docs/http.md`
  The HTTP channel requires `HTTP_WEBHOOK_SECRET` at startup, but the docs present the secret as optional and show an example without it.

- `src/channels/gmail.rs`
  Gmail Pub/Sub notifications parse `history_id` but do not use it to drive delta fetches, so a single notification can trigger a broad unread scan.

## Important Medium-Priority Findings

### Runtime / Config

- `src/main.rs`, `src/config/database.rs`, `src/app.rs`
  `--no-db` does not fully bypass DB requirements because config loading still requires DB-related settings before the flag takes effect.

- `src/bootstrap.rs`, `src/main_helpers.rs`
  Legacy `bootstrap.json` migration copies database config but drops onboarding completion state, which can reopen onboarding for upgraded installs.

### Local Surfaces / Commands

- `src/channels/repl.rs`, `src/agent/command_catalog.rs`
  REPL help is a hand-maintained subset and drifts from the shared catalog.

- `src/agent/command_catalog.rs`, `src/agent/submission.rs`
  TUI completion exposes bare `/thread`, but the parser only accepts `/thread new` or `/thread <uuid>`.

- `src/agent/commands.rs`
  Manual `/heartbeat` uses default heartbeat/hygiene config instead of the agent’s live config.

- `src/agent/session.rs`, `src/agent/thread_ops.rs`
  `/clear` and `/resume` do not fully clear pending approval/auth state.

### Gateway / API

- `src/channels/web/handlers/jobs.rs`, `src/db/mod.rs`, `src/channels/web/static/app.js`
  The UI renders `jobs.stuck`, but the server hardcodes it to `0` and the store never computes it.

- `src/channels/web/server.rs`, `src/channels/web/handlers/chat.rs`, `src/channels/web/static/app.js`
  The browser uses SSE, while websocket chat remains mounted, described, and localhost-restricted. This looks like a stale or partially retired transport path.

- `src/api/system.rs`
  `engine_running`, `setup_completed`, and `active_extensions` are partly fabricated placeholder values.

### Native / Packaged Channels

- `src/channels/imessage_wiring.rs`
  Appears to be duplicate/stale wiring relative to the startup path in `main.rs`.

- `src/channels/manager.rs`, `src/channels/status_view.rs`
  Channel status entries do not populate useful telemetry like `last_message_at` or `last_error`, and `channel_type` derivation is lossy for names like `apple_mail`.

- `channels-src/discord/README.md`
  Discord packaged-channel docs still describe an older build/config story.

### Tools / Context / Workspace

- `src/tools/toolset.rs`
  The `communication` toolset lists singular tool names (`telegram_action`, etc.) while the actual tools are plural (`telegram_actions`, etc.).

- `src/tools/toolset.rs`
  Toolset resolution uses a `visited` set that treats valid shared-inclusion DAGs as cycles.

- `src/context/post_compaction.rs`, `src/context/read_audit.rs`
  Post-compaction reinjection and read-audit helpers appear exported but not wired into the live runtime.

### Extensions / Registry

- `registry/tools/slack.json`, `tools-docs/*`, `tools-src/slack/README.md`
  Slack WASM tool naming drifts between `slack-tool` in the registry and `slack` in docs/readmes, which matters because runtime install/auth uses exact stems.

- `src/extensions/{plugin_manifest,plugin_interfaces,plugin_routes}.rs`
  Looks like a parallel plugin subsystem that is exported but not part of the real extension runtime.

### Safety / Storage / Media

- `src/pairing/store.rs`
  `add_allow_from()` truncates the allow-list file before reading/rebuilding it, which can drop prior approvals.

- `src/document_extraction/extractors.rs`
  XLSX extraction only reads `sharedStrings.xml`, so inline-string and worksheet-only text paths are incomplete.

- `src/observability/mod.rs`
  Only `noop` and `log` are selectable, even though fan-out infrastructure exists.

- `src/platform/mod.rs`
  Capability flags are largely optimistic/hard-coded rather than detected.

### Research / History / Learning

- `src/api/experiments.rs`, `src/history/store.rs`
  Trial/campaign finalization does not feed learning/history APIs that already exist, and finalization writes are split across calls.

- `src/experiments/adapters.rs`, `src/api/experiments.rs`
  Some remote runners validate even when they are only manual/bootstrap-ready, but campaigns treat them as launch-ready.

## Stale / Drift Themes

- Compatibility aliases are still widespread:
  `/vibe`, `/compact`, `persona_seed`, legacy provider aliases, legacy Bedrock proxy fallback.

- Several areas show “defined but not truly live” behavior:
  `ToolPolicyManager`, post-compaction injection, read audit, websocket browser chat path, plugin subsystem modules, parts of channel status telemetry.

- Multiple docs drift from runtime behavior:
  HTTP webhook auth, Gmail delivery model, Discord packaged-channel docs, release/build profile docs, deploy bootstrap filenames, parity status rows.

- Some generated or environment-specific artifacts look misplaced in the repo:
  `channels-src/telegram/target/`,
  deploy service files hardcoded to `thinclaw-prod` / `us-central1`.

## Remediation Order

### Wave 1: correctness and security

- Fix IPv6 SSRF filtering in `src/safety/media_url.rs`
- Fix WASM channel hot-reload route registration
- Fix TUI `/quit` and `/exit`
- Fix `ToolPolicyManager` enforcement in shared tool dispatch
- Fix single-target retry behavior in `src/llm/runtime_manager.rs`
- Fix failed experiment-run completion in `src/experiments/runner.rs`
- Fix first-run detection ordering and `--no-db`

### Wave 2: operator-facing contract fixes

- Align HTTP webhook docs with runtime secret requirements
- Fix Gmail event-to-fetch wiring or rewrite docs to match current behavior
- Fix deploy `.env.template` vs `env.example`
- Fix placeholder/fabricated status values in `src/api/system.rs`
- Fix `jobs.stuck` summary or remove the UI card
- Fix Slack tool stem naming across registry/docs/runtime

### Wave 3: cleanup and structure

- Remove or wire duplicate/stale modules like `imessage_wiring.rs` and plugin-system leftovers
- Wire or prune post-compaction/read-audit helpers
- Unify REPL/TUI/shared command help and alias documentation
- Reconcile docs/parity ledger with the actual runtime
- Add regression tests around toolset naming/resolution, pairing allow-list persistence, and experiments finalization

## Tests That Should Be Added First

- IPv6 SSRF regression tests for ULA/unspecified/multicast
- TUI `/quit` and `/exit` integration tests
- WASM channel hot-add/hot-remove webhook registration tests
- Startup tests for onboarding detection from TOML/persisted config
- `--no-db` startup regression test
- Tool policy enforcement tests in real dispatch paths
- Single-target LLM retry behavior tests
- Experiment-run failure completion / stale-lease tests
- Pairing allow-list persistence tests
- Toolset registry parity and DAG-resolution tests

## Notes

- This audit was static. Findings should be verified with focused repro tests before code changes are batched together.
- Several “stale/drift” findings may be intentional compatibility surfaces, but they still deserve explicit ownership and documentation.
