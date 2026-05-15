# ThinClaw Desktop Bridge Contract

This document is the final bridge contract for the Desktop bridge. Public
product labels use ThinClaw / ThinClaw Desktop, public Tauri command names use
the `thinclaw_*` prefix, and frontend events are emitted on `thinclaw-event`.
It covers the ThinClaw Agent Cockpit only. For the two-system Desktop split and
the non-agent Direct AI Workbench, read `runtime-boundaries.md` first.

Last updated: 2026-05-15

Related docs:

- Runtime boundaries: `apps/desktop/documentation/runtime-boundaries.md`
- Runtime parity: `apps/desktop/documentation/runtime-parity-checklist.md`
- Remote gateway matrix: `apps/desktop/documentation/remote-gateway-route-matrix.md`
- Environment requirements: `apps/desktop/documentation/env-requirements.md`
- Packaging readiness: `apps/desktop/documentation/packaging-platform-readiness.md`
- Platform readiness: `apps/desktop/documentation/packaging-platform-readiness.md`
- Secrets policy: `apps/desktop/documentation/secrets-policy.md`
- Manual smoke checklist: `apps/desktop/documentation/manual-smoke-checklist.md`
- External release prerequisites: `apps/desktop/documentation/external-release-prerequisites.md`

## Runtime Modes

### Local Mode

Desktop runs ThinClaw in-process through `ThinClawRuntimeState` and `ThinClawRuntimeInner`.

- The frontend invokes stable Tauri commands.
- Chat messages are injected into ThinClaw through `TauriChannel`.
- ThinClaw emits `StatusUpdate` values through the channel.
- `StatusUpdate` values are converted into `UiEvent` and emitted as `thinclaw-event`.

### Remote Mode

Desktop talks to a remote ThinClaw gateway through `RemoteGatewayProxy`.

- The frontend still invokes the same Tauri commands.
- Command handlers forward supported calls to the remote HTTP gateway.
- The remote SSE stream is subscribed by the proxy.
- Remote events must be re-emitted to the frontend as the same `thinclaw-event` `UiEvent` schema used by local mode.
- Unsupported remote endpoints must return a typed unavailable response or a clear error reason. They must not silently no-op.

## Event Contract

`UiEvent` is the single desktop event schema. Local and remote modes must converge on this shape before crossing the frontend boundary.

- Event bus: `thinclaw-event`
- Rust schema: `apps/desktop/backend/src/thinclaw/ui_types.rs`
- Generated TS type: `apps/desktop/frontend/src/lib/bindings.ts`
- Local conversion: `apps/desktop/backend/src/thinclaw/event_mapping.rs`
- Local transport: `apps/desktop/backend/src/thinclaw/tauri_channel.rs`
- Remote transport: `apps/desktop/backend/src/thinclaw/remote_proxy.rs`

Every current ThinClaw `StatusUpdate` variant must be either mapped to `UiEvent` or explicitly documented as intentionally ignored. As of this checkpoint, Desktop maps chat, plan, usage, cost, lifecycle, approval, auth, canvas, job, subagent, agent-message, and routine events. Unknown remote gateway SSE events are forwarded as `UiEvent::GatewayEvent` instead of being silently dropped.

### Routing Rules

Event routing is metadata-first and must stay deterministic:

1. Use `thread_id` when present.
2. Use `session_key` when present and `thread_id` is absent.
3. Use `run_id` as secondary metadata for run-local state and UI disambiguation.
4. Use the explicit local command session for local token/chat deltas where ThinClaw emitted no metadata.
5. Use `agent:main` for truly unscoped backend events.
6. Never route remote SSE events by "latest visible session" when the gateway event carries thread/session metadata.

Concurrent-session regressions should be treated as contract breaks, not UI bugs.

## IPC Stability

The frontend and existing automation scripts depend on the current Tauri command names.

- Keep `thinclaw_*` command names stable.
- Keep `thinclaw-event` stable.
- Add new capabilities through additive commands or additive `UiEvent` variants.
- Do not rename `thinclaw-desktop-tools` unless the migration is explicitly planned.
- Regenerate `apps/desktop/frontend/src/lib/bindings.ts` from Rust after command/type changes. Do not hand-edit generated bindings.

### Command Surface Groups

The command registry lives in `apps/desktop/backend/src/setup/commands.rs`.

| Surface | Local mode behavior | Remote mode behavior |
| --- | --- | --- |
| Chat/sessions/approvals | Uses in-process ThinClaw runtime and `TauriChannel`. | Proxies chat/session/approval HTTP routes and forwards gateway SSE. |
| Memory/files | Uses ThinClaw memory/workspace APIs. | Proxies gateway memory read/write/list/search/delete routes and returns explicit unavailable errors for host-only operations. |
| Providers/routing/vault | Uses local keychain, provider config, route simulation, model discovery. | Uses provider gateway endpoints; raw secret reads remain denied. |
| Skills/extensions/MCP | Uses root skill registry, extension manager, MCP API. | Uses `/api/skills`, `/api/extensions`, and `/api/mcp` gateway routes. |
| Jobs/autonomy/experiments/learning | Uses root APIs when DB/runtime/config allow them. | Status/review routes are proxied; host-executing mutation stays gated or unavailable with concrete reasons. |
| Channels/routines/pairing | Uses ThinClaw DB/runtime APIs and forwards routine lifecycle events. | Proxies gateway routes and status/config APIs where available. |

## Binding Generation

Bindings are exported by the debug Tauri startup path in `apps/desktop/backend/src/lib.rs`. For standalone regeneration without launching the app, create a temporary backend example that calls `tauri_app_lib::setup::commands::specta_builder().export(...)`, run it, and delete the example before committing. This is the same flow used during the P2-W4 checkpoint.

After regeneration, run:

```bash
cd apps/desktop/backend && cargo check --locked
cd apps/desktop && npm run lint:ts
```

## Session Routing

Desktop event routing is metadata-first.

- Prefer ThinClaw `thread_id` or `session_key` metadata.
- Use `run_id` and `message_id` metadata when present.
- Use the command-provided local session for local chat deltas without metadata.
- Fall back to `agent:main` for truly unscoped events.
- Concurrent session tests should prove one active run cannot receive another run's events.

## Secrets And Identifiers

New writes use ThinClaw identifiers. Legacy Scrappy identifiers are read-only fallback inputs for app data, cloud, and keychain migration.

`KeychainSecretsAdapter` must deny ungranted access for `get`, `get_for_injection`, `exists`, `list`, and `is_accessible`.

Remote mode must never return raw provider secrets. It may expose save/delete/status capability only. Raw local injection commands must remain local-only and unavailable in remote mode.
