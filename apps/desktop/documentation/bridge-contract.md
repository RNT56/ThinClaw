# ThinClaw Desktop Bridge Contract

This document is the final bridge contract for the Desktop bridge. Public
product labels use ThinClaw / ThinClaw Desktop, public Tauri command names use
the `thinclaw_*` prefix, and frontend events are emitted on `thinclaw-event`.
It covers the ThinClaw Agent Cockpit only. For the two-system Desktop split and
the non-agent Direct AI Workbench, read `runtime-boundaries.md` first.

Last updated: 2026-07-13

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

## Command Routing & Gating (TDO-001/002)

Dual-mode availability is expressed with typed primitives in
`apps/desktop/backend/src/thinclaw/bridge.rs` so the frontend can tell "gated, here's why"
from "failed":

- **`RouteMode`** — `LocalAndRemote` (works in both modes), `RemoteOnly` (needs a live gateway,
  e.g. sandbox job/GPU flows), `LocalOnly` (only meaningful embedded, e.g. sidecar control,
  channel-config submit, agent-loop eval).
- **`BridgeError`** — an internally-tagged enum (`kind`): `Unavailable { capability, reason,
  remediation, satisfied_by }` for a gated capability (the frontend renders a CTA), and
  `Runtime { message }` for a genuine error. `From<String>`/`From<&str>` let existing
  `?`/`map_err` sites migrate mechanically.
- **`gated(capability, reason, remediation, satisfied_by)`** — the helper that builds an
  `Unavailable`; it replaced the ad-hoc `local_unavailable`/`unavailable(...)` JSON helpers.
- **`ROUTE_TABLE`** — a `&[(&str, RouteMode)]` registry mapping command names to their mode.
  It is the bridge linter's ground truth: the test suite asserts every command that calls
  `gated()` is classified, and every `ROUTE_TABLE` command is registered in the binding surface.
  The committed per-command route matrix is generated from this table and guarded against drift.

When adding or gating a command, call `gated(...)` for the unavailable path and add the
command to `ROUTE_TABLE`, then run `cargo run --locked --example export_bindings` from
`apps/desktop/backend`. Do not edit the generated block in
[`remote-gateway-route-matrix.md`](remote-gateway-route-matrix.md) by hand.

## Event Contract

`UiEvent` is the single desktop event schema. Local and remote modes must converge on this shape before crossing the frontend boundary.

- Event bus: `thinclaw-event`
- Rust schema: `apps/desktop/backend/src/thinclaw/ui_types.rs`
- Generated TS type: `apps/desktop/frontend/src/lib/bindings.ts`
- Local conversion: `apps/desktop/backend/src/thinclaw/event_mapping.rs`
- Local transport: `apps/desktop/backend/src/thinclaw/tauri_channel.rs`
- Remote transport: `apps/desktop/backend/src/thinclaw/remote_proxy/`

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

### Frontend Calling Convention

`apps/desktop/frontend/src/lib/bindings.ts` is the only source of command names,
parameter lists, and generated result types. `lib/command-client.ts` derives a
runtime-guarded client from `commands`, unwraps the generated transport `Result`,
and preserves typed `BridgeError::Unavailable` detail in thrown errors.

`lib/thinclaw.ts` is a compatibility surface for existing component-friendly
names and richer legacy views over JSON-valued commands. It may delegate through
`compatibilityCommands`, but it must not import Tauri `invoke`, spell raw command
strings, or establish a second IPC path. Production frontend source is guarded
against raw `invoke` imports and calls; new code must use `commandClient` or a
purpose-built adapter that is itself derived from generated `commands`.

`src/desktop_api.rs` owns reusable backend service helpers. Tauri registration
and wire-shape adapters remain in `apps/desktop/backend/src/thinclaw/commands`;
the retired `src/tauri_commands.rs` name survives only as a deprecated Rust
re-export for downstream source compatibility.

All `UiEvent` consumers subscribe through `useThinClawEvents`. That module owns
the one native `thinclaw-event` listener and fans the generated discriminated
union out in process; panel-local listeners are rejected by a contract test.

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

Bindings are exported by the committed `apps/desktop/backend/examples/export_bindings.rs`
entry point. Run the official exporter instead of creating a temporary example:

```bash
cd apps/desktop/backend
cargo run --locked --example export_bindings
```

The backend contract suite regenerates the complete registry in memory and
requires its sanitized output to match the committed binding byte-for-byte. It
also proves that `Channel<T>` remains a real Tauri channel after Specta export,
that sanitization is idempotent, that command names are unique, and that no
generated command parameter is a reserved strict-mode TypeScript identifier.
Adding or changing any command therefore requires regenerating the bindings;
sampling a representative subset is not sufficient.

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

The grant-aware `SecretsStore` implementation on the shared `SecretStore` must deny ungranted access for `get`, `get_for_injection`, `exists`, `list`, and `is_accessible`.

Remote mode must never return raw provider secrets. It may expose save/delete/status capability only. Raw local injection commands must remain local-only and unavailable in remote mode.
