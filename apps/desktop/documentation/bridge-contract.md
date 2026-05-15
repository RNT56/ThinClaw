# ThinClaw Desktop Bridge Contract

This document is the alpha compatibility contract for the Desktop bridge. Public product labels use ThinClaw / ThinClaw Desktop. Internal Tauri command names still use the `openclaw_*` prefix until the post-alpha rename.

## Runtime Modes

### Local Mode

Desktop runs ThinClaw in-process through `IronClawState` and `IronClawInner`.

- The frontend invokes stable Tauri commands.
- Chat messages are injected into ThinClaw through `TauriChannel`.
- ThinClaw emits `StatusUpdate` values through the channel.
- `StatusUpdate` values are converted into `UiEvent` and emitted as `openclaw-event`.

### Remote Mode

Desktop talks to a remote ThinClaw gateway through `RemoteGatewayProxy`.

- The frontend still invokes the same Tauri commands.
- Command handlers forward supported calls to the remote HTTP gateway.
- The remote SSE stream is subscribed by the proxy.
- Remote events must be re-emitted to the frontend as the same `openclaw-event` `UiEvent` schema used by local mode.
- Unsupported remote endpoints must return a typed unavailable response or a clear error reason. They must not silently no-op.

## Event Contract

`UiEvent` is the single desktop event schema. Local and remote modes must converge on this shape before crossing the frontend boundary.

- Event bus: `openclaw-event`
- Rust schema: `apps/desktop/backend/src/openclaw/ui_types.rs`
- Generated TS type: `apps/desktop/frontend/src/lib/bindings.ts`
- Local conversion: `apps/desktop/backend/src/openclaw/ironclaw_types.rs`
- Local transport: `apps/desktop/backend/src/openclaw/ironclaw_channel.rs`
- Remote transport: `apps/desktop/backend/src/openclaw/remote_proxy.rs`

Every current ThinClaw `StatusUpdate` variant must be either mapped to `UiEvent` or explicitly documented as intentionally ignored. As of this checkpoint, plan and usage updates are mapped rather than dropped.

## IPC Stability

The alpha frontend and existing automation scripts depend on the current Tauri command names.

- Keep `openclaw_*` command names stable for alpha.
- Keep `openclaw-event` stable for alpha.
- Add new capabilities through additive commands or additive `UiEvent` variants.
- Do not rename `thinclaw-desktop-tools` during alpha unless the migration is explicitly planned.

## Session Routing

Desktop event routing is metadata-first.

- Prefer ThinClaw `thread_id` or `session_key` metadata.
- Use `run_id` and `message_id` metadata when present.
- Fall back to the most recently activated session only when ThinClaw metadata is absent.
- Concurrent session tests should prove one active run cannot receive another run's events.

## Secrets And Identifiers

New writes use ThinClaw identifiers. Legacy Scrappy identifiers are read-only fallback inputs for app data, cloud, and keychain migration.

`KeychainSecretsAdapter` must deny ungranted access for `get`, `get_for_injection`, `exists`, `list`, and `is_accessible`.
