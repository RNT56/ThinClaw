# Deferred Follow-ups — Completion Tracker

Tracks the follow-ups that were deferred while landing the first Lane-B parity batch
(foundations #116 undo, #117 eval, #118 events, #119 channel-config) and their
subsequent execution. **Verified 2026-07-13: items 1–5 and 7 are implemented and locally
verified; item 6 needs a running app; item 8 remains intentionally skipped.**

State legend: ✅ shipped (merged to `main`) · ⏳ ready, not started ·
➖ intentionally skipped.

**Shared foundations (in place):** generic `UiEvent::AgentLifecycleEvent`
(`apps/desktop/backend/src/thinclaw/ui_types.rs`); `event_mapping.rs::status_to_ui_event`;
emit idiom `self.channels.send_status(&message.channel, StatusUpdate::X, &message.metadata)`;
the 5-matcher `StatusUpdate` ripple (tui/gateway-SSE/repl/acp/wasm); `Channel::config_schema()`
+ `ChannelManager::config_schema_for/config_schemas/update_channel_runtime_config`.

---

## 1. Undo/redo UI control — ✅ **#120**
Toolbar buttons in `ThinClawChatView` (next to Stop Run, gated on `gatewayRunning`) calling
`thinclawCommands.thinclawUndo/Redo(effectiveSessionKey)` with toast feedback. Completes
TDO-104 end-to-end (commands #116 + UI #120).

## 2. Advisor-consultation lifecycle event — ✅ **#121**
`StatusUpdate::AdvisorConsultationStarted` emitted at both `consult_advisor` sites in
`tool_execution.rs` (inline + parallel) → `AgentLifecycleEvent`.

## 3. Self-repair lifecycle event — ✅ **#121**
`StatusUpdate::SelfRepairStarted` / `SelfRepairCompleted` emitted from the background repair
loop (`agent_loop/mod.rs`) around `repair_stuck_job` / `repair_broken_tool`. **Decision D-1
applied:** broadcasts on the `web` target with synthetic `session_key=agent:main` (repair is
decoupled from active runs — no per-session context to thread).

## 4. Channel-config submit — ✅ **#122**
`thinclaw_channel_config_submit(channel_id, values)` persists each field under
`channels.{id}_{field}` (`api::config::set_setting`) and forwards to the live channel via
`ChannelManager::update_channel_runtime_config`. Non-secret fields persist as settings;
manifest credentials route only to encrypted secret storage. The same validation and submit
contract is available locally and through the authenticated remote gateway. Startup-only fields
still require channel restart/reactivation.

## 5. Channel-config settings form — ✅ **#123**
**Decision D-4 applied:** delivered as a **new Lane-B panel** `ThinClawChannelConfig` (a
`channel-config` cockpit page) rather than editing the Lane-A-owned `ThinClawChannels.tsx`.
Renders the schema (`thinclaw_channel_config_schemas`) as a dynamic form and submits via
item 4; surfaces the backend `restart_required` note as a toast.

## 6. Eval runtime smoke-test — ⏳ (needs running app)
Not a code change. With an embedded engine running, use the Experiments Benchmarks panel
to call `thinclaw_experiments_run_eval("agent_loop", "<prompt>", 1, 4)` and confirm a
scored trajectory in a throwaway `agent-env:` session. **Cannot be executed in a headless
dev environment** — left as a manual QA step rather than faked.

## 7. More channels implement `config_schema` — ✅
Signal, Discord, iMessage, Nostr, Apple Mail, and BlueBubbles expose non-secret native
schemas. Installed WASM channels map `setup.required_secrets` from their capabilities manifests
to encrypted-store-only password fields. Native Matrix, voice-call, APNs, and browser-push
surface exact host-managed setup instructions rather than presenting non-functional forms.

## 8. Typed `ConfigSchema` DTO in bindings — ➖ skipped
The read commands return `serde_json::Value` and the panel (item 5) renders dynamically from
that JSON, so a specta-typed mirror DTO was unnecessary. Revisit only if a consumer wants
typed props.

---

## Remaining work
- **#6** eval smoke-test (manual, needs the app).
- Future: `Arc<RwLock>` live-reload for startup-only native channel fields so submit applies
  without a restart.
