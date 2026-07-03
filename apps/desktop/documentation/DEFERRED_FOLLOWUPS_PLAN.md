# Deferred Follow-ups — Completion Tracker

Tracks the follow-ups that were deferred while landing the first Lane-B parity batch
(foundations #116 undo, #117 eval, #118 events, #119 channel-config) and their
subsequent execution. **Updated 2026-06-29: items 1–5 are now implemented and in-flight
(PRs #120–#123); item 6 needs a running app; items 7–8 are additive/optional.**

State legend: ✅ shipped (PR) · 🟡 implemented, in-flight PR · ⏳ ready, not started ·
➖ intentionally skipped.

**Shared foundations (in place):** generic `UiEvent::AgentLifecycleEvent`
(`apps/desktop/backend/src/thinclaw/ui_types.rs`); `event_mapping.rs::status_to_ui_event`;
emit idiom `self.channels.send_status(&message.channel, StatusUpdate::X, &message.metadata)`;
the 5-matcher `StatusUpdate` ripple (tui/gateway-SSE/repl/acp/wasm); `Channel::config_schema()`
+ `ChannelManager::config_schema_for/config_schemas/update_channel_runtime_config`.

---

## 1. Undo/redo UI control — 🟡 **#120**
Toolbar buttons in `ThinClawChatView` (next to Stop Run, gated on `gatewayRunning`) calling
`thinclawCommands.thinclawUndo/Redo(effectiveSessionKey)` with toast feedback. Completes
TDO-104 end-to-end (commands #116 + UI #120).

## 2. Advisor-consultation lifecycle event — 🟡 **#121**
`StatusUpdate::AdvisorConsultationStarted` emitted at both `consult_advisor` sites in
`tool_execution.rs` (inline + parallel) → `AgentLifecycleEvent`.

## 3. Self-repair lifecycle event — 🟡 **#121**
`StatusUpdate::SelfRepairStarted` / `SelfRepairCompleted` emitted from the background repair
loop (`agent_loop.rs`) around `repair_stuck_job` / `repair_broken_tool`. **Decision D-1
applied:** broadcasts on the `web` target with synthetic `session_key=agent:main` (repair is
decoupled from active runs — no per-session context to thread).

## 4. Channel-config submit — 🟡 **#122**
`thinclaw_channel_config_submit(channel_id, values)` persists each field under
`channels.{id}_{field}` (`api::config::set_setting`) and forwards to the live channel via
`ChannelManager::update_channel_runtime_config`. **Decisions D-2/D-3 applied:** WASM channels
apply live; native channels persist + report `restart_required` (no live-reload refactor in
v1); gated **LocalOnly** + classified in `ROUTE_TABLE` (no remote path in v1).

## 5. Channel-config settings form — 🟡 **#123**
**Decision D-4 applied:** delivered as a **new Lane-B panel** `ThinClawChannelConfig` (a
`channel-config` cockpit page) rather than editing the Lane-A-owned `ThinClawChannels.tsx`.
Renders the schema (`thinclaw_channel_config_schemas`) as a dynamic form and submits via
item 4; surfaces the backend `restart_required` note as a toast.

## 6. Eval runtime smoke-test — ⏳ (needs running app)
Not a code change. With an embedded engine running, call
`thinclaw_experiments_run_eval("agent_loop", "<prompt>", 1, 4)` and confirm a scored
trajectory in a throwaway `agent-env:` session. **Cannot be executed in a headless dev
environment** — left as a manual QA step rather than faked.

## 7. More channels implement `config_schema` — ⏳ additive
Signal (#119) and Discord (#122) are done. Gmail/Nostr/BlueBubbles (native, mirror Signal)
and the WASM channels (Telegram/Slack, which support **live** submit) remain additive
follow-ups — each an isolated override, no ripple.

## 8. Typed `ConfigSchema` DTO in bindings — ➖ skipped
The read commands return `serde_json::Value` and the panel (item 5) renders dynamically from
that JSON, so a specta-typed mirror DTO was unnecessary. Revisit only if a consumer wants
typed props.

---

## Remaining work
- **#7** native + WASM `config_schema` long-tail (additive).
- **#6** eval smoke-test (manual, needs the app).
- Future: remote-mode channel-config submit (gateway route + `RemoteGatewayProxy` method);
  `Arc<RwLock>` live-reload for native channels so submit applies without a restart.
