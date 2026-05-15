# ThinClaw Desktop Handoff

Last updated: 2026-05-15

This is the quick orientation document for future Desktop parity workers.

## Current Contract

- Public product name: ThinClaw Desktop.
- Stable alpha Tauri commands: `openclaw_*`.
- Stable alpha event bus: `openclaw-event`.
- Generated frontend bindings: `apps/desktop/frontend/src/lib/bindings.ts`.
- Local runtime bridge: `apps/desktop/backend/src/openclaw/ironclaw_bridge.rs`.
- Local event channel: `apps/desktop/backend/src/openclaw/ironclaw_channel.rs`.
- Event conversion: `apps/desktop/backend/src/openclaw/ironclaw_types.rs`.
- Remote gateway proxy: `apps/desktop/backend/src/openclaw/remote_proxy.rs`.
- IPC registry: `apps/desktop/backend/src/setup/commands.rs`.

## Source Of Truth Docs

Read these before changing Desktop bridge behavior:

1. `apps/desktop/documentation/bridge-contract.md`
2. `apps/desktop/documentation/runtime-parity-checklist.md`
3. `apps/desktop/documentation/remote-gateway-route-matrix.md`
4. `apps/desktop/documentation/secrets-policy.md`
5. `apps/desktop/documentation/env-requirements.md`
6. `apps/desktop/documentation/packaging-platform-readiness.md`
7. `apps/desktop/documentation/packaging-platform-readiness.md`
8. `apps/desktop/documentation/manual-smoke-checklist.md`
9. `apps/desktop/documentation/known-post-alpha.md`

## Where To Add Work

| Task | Primary files |
| --- | --- |
| Add/modify Tauri command | `apps/desktop/backend/src/openclaw/commands/*`, then register in `setup/commands.rs`. |
| Add/modify event schema | `ui_types.rs`, `ironclaw_types.rs`, frontend event consumers, regenerate bindings. |
| Add remote route | `remote_proxy.rs`, matching root `src/channels/web/handlers/*`, update route matrix. |
| Add provider/secret behavior | `openclaw/commands/keys.rs`, `ironclaw_secrets.rs`, `openclaw/config/keychain.rs`, secrets policy. |
| Add UI surface | `apps/desktop/frontend/src/components/openclaw/*` or `components/settings/*`, wrapper in `lib/openclaw.ts`. |
| Add root gateway endpoint | `src/channels/web/server.rs`, `src/channels/web/handlers/*`, shared `src/api/*` when possible. |

## Required Workflow

1. Preserve dirty worktree changes you did not make.
2. Use existing command names during alpha.
3. Make unsupported behavior explicit and typed.
4. Regenerate bindings from Rust after command/type changes.
5. Update documentation when adding/removing a Desktop-exposed route.
6. Run the relevant automated gate.

## Minimum Verification

For backend contract changes:

```bash
cd apps/desktop/backend && cargo check --locked
cd apps/desktop/backend && cargo test --locked --lib -- --skip web_search
```

For frontend/IPC changes:

```bash
cd apps/desktop && npm run lint:ts
cd apps/desktop && npm test
cd apps/desktop && npm run build
```

For release handoff:

```bash
cd apps/desktop/backend && cargo check --locked
cd apps/desktop/backend && cargo test --locked --lib -- --skip web_search
cd apps/desktop && npm run lint:ts
cd apps/desktop && npm test
cd apps/desktop && npm run build
cd /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop && cargo test --workspace
cd apps/desktop && npx tauri info
```

Then execute `manual-smoke-checklist.md`.

## High-Risk Areas

- Event routing: concurrent sessions must not receive each other's events.
- Secrets: remote mode must never leak raw secrets.
- Remote unsupported commands: no silent no-op, no fake success.
- Generated bindings: do not hand-edit.
- Autonomy/jobs: execution must remain gated by explicit config and host permissions.
- Legacy compatibility: Scrappy fallback reads are allowed; new writes are ThinClaw identifiers only.
