# Known Post-Alpha Items

Last updated: 2026-05-15

These are intentionally deferred items. Do not treat them as silent TODOs during alpha release readiness; keep unsupported behavior explicit in the UI and backend.

## Naming Cleanup

- Rename `openclaw_*` Tauri commands after alpha.
- Rename `openclaw-event` after alpha.
- Remove legacy Scrappy/OpenClaw compatibility identifiers after a migration window.
- Rename legacy config filenames only with a rollback plan.

## Gateway API Gaps

- Chat abort endpoint.
- Session reset, compact, and transcript export endpoints.
- Memory delete endpoint.
- Hook management endpoints.
- Recent log snapshot endpoint.
- Remote response cache stats endpoint.
- Native remote routing-rule mutation format for Desktop rule editor.

## Remaining Contract Tests

- Full generated-binding byte-for-byte drift test against a standalone Specta export.
- Future `StatusUpdate -> UiEvent` coverage as root ThinClaw adds new variants.
- Runtime-level concurrent session test with two live agent runs, not only mapper-level routing assertions.
- Remote unavailable-response matrix test that executes every command against a fixture gateway.
- Provider/routing translation tests for complete primary/cheap pool editing and advisor readiness.

## Product Hardening

- End-to-end screenshots for every Phase 2 surface.
- Stronger empty/error states for unavailable remote controls.
- More granular permission labels for autonomy and job execution.
- Better split between legacy FastAPI MCP sandbox settings and first-class ThinClaw MCP server management.
- Accessibility pass for dense management panels.

## Packaging

- Complete notarization/updater validation.
- Confirm macOS identity, entitlements, sidecars, and keychain access on a clean machine.
- Validate Windows/Linux gated behavior or hide unsupported platform-only controls.
- Decide whether to ship all engine profiles or a smaller alpha profile matrix.

## Documentation

- Move `openclaw` implementation references to historical notes after the rename.
- Add diagrams for local/remote event flow and provider-vault flows.
- Add a release runbook with screenshots once P3-W2 smoke is complete.
