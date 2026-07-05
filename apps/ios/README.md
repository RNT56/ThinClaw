# ThinClaw for iOS + watchOS

Native Swift surface for a self-hosted ThinClaw instance: iOS app, widgets,
Live Activity, and watchOS companion. Strictly a **gateway client** over the
documented network contract — no embedded runtime
(see `apps/desktop/documentation/runtime-boundaries.md`).

Canonical docs: [`docs/MOBILE_APP.md`](../../docs/MOBILE_APP.md) (contract +
milestones) and [`docs/MOBILE_SECURITY.md`](../../docs/MOBILE_SECURITY.md)
(security model — read before touching pairing, tokens, push, or transport).

## Status

This is the **R0 scaffold**. What is real today:

| Piece | Status |
|---|---|
| `ThinClawTransport` (SSE parser, event decoder, reconnect policy) | ✅ implemented + fixture-tested |
| `ThinClawCore` (domain models, chunk coalescer) | ✅ implemented + tested |
| `ThinClawSnapshotKit` (App Group snapshots, Live Activity attributes) | ✅ implemented + tested |
| `ThinClawAuth` (pairing payload, Keychain store) | ✅ implemented + tested |
| `ThinClawAPI` (endpoint/auth/error shell) | ✅ shell tested; generated client lands with M1 |
| `ThinClawPersistence` (in-memory store; GRDB at M1) | ✅ protocol + tests |
| `ThinClawDesign`, features, target shells, widgets, watch | 🚧 authored seeds — compiled via Tuist/xcodebuild, not yet exercised |
| Tuist manifests / CI `build-app` job | 🚧 authored; verify locally with `tuist generate` |

Milestones M1–M5 are defined in `docs/MOBILE_APP.md`.

## Toolchain

Requirements: Xcode 26+, [mise](https://mise.jdx.dev).

```bash
cd apps/ios
mise install          # pins tuist + swift-openapi-generator
tuist install
tuist generate        # writes ThinClaw.xcworkspace (gitignored)
```

Signing: `cp Config/Signing.example.xcconfig Config/Signing.local.xcconfig`
and set `DEVELOPMENT_TEAM`. Nothing secret is committed.

## Layout

```
App/ Widgets/ Watch/ WatchWidgets/   # thin target shells (composition only)
Packages/                            # all real code, local SPM packages
  ThinClawAPI        generated gateway client + auth/error shell
  ThinClawTransport  SSE parser, AgentEvent decoding, reconnect policy
  ThinClawCore       domain models, reducers
  ThinClawPersistence transcript cache + outbox (GRDB at M1)
  ThinClawAuth       pairing payload, Keychain, Bonjour browser
  ThinClawSnapshotKit App Group snapshots for widgets/watch + Live Activity attrs
  ThinClawDesign     Liquid Glass design system
  ThinClawWidgetKitShared  timeline providers + AppIntents (approve/deny/quick-ask)
  ThinClawWatchBridge WatchConnectivity relay (watch holds its own token)
  Features/*         one package per surface (root view + @Observable store)
Config/              xcconfigs (deployment targets, strict concurrency, signing)
scripts/             generate-api.sh, check-generated-drift.sh, record-fixtures.sh
```

Dependency rules: features depend on Core/Design (+Auth/Persistence where
justified); the widget extension never imports Transport or Persistence;
SnapshotKit stays Foundation-only.

## Testing

Pure-logic packages declare macOS and test on any Mac without a simulator:

```bash
for p in ThinClawTransport ThinClawCore ThinClawSnapshotKit ThinClawAuth ThinClawAPI ThinClawPersistence; do
  swift test --package-path Packages/$p
done
```

The SSE parser suite replays recorded gateway fixtures at adversarial
chunkings (byte-by-byte, mid-UTF-8 splits, CRLF variants) — extend fixtures
with `scripts/record-fixtures.sh` against a local gateway.

UI targets compile through the Tuist project (`xcodebuild test` with an iOS
26 simulator destination).

## API client generation

The gateway's OpenAPI snapshot is generated **from Rust** and committed at
`clients/openapi/thinclaw-gateway.openapi.json` (repo root; regenerate with
`cargo run --bin export-openapi -- generate`). Then:

```bash
scripts/generate-api.sh   # vendors the spec, runs swift-openapi-generator
```

Generated sources are committed; `scripts/check-generated-drift.sh` gates CI.
Never hand-edit generated code or the vendored spec.

## Security invariants (enforced by review)

- Device tokens (`tcd_…`) live in the shared Keychain group,
  `AfterFirstUnlockThisDeviceOnly`; never in UserDefaults, files, or logs.
- The connection layer refuses plain HTTP except loopback (debug) and the
  explicit `vpn-http` pairing mode; pinned-SPKI TLS is the default. ATS
  stays strict — never add `NSAllowsArbitraryLoads`.
- Push payloads are content-free; real content is fetched by the
  Notification Service Extension over the paired connection.
- High-risk approvals are never actionable from widgets, notifications, or
  the watch — they deep-link into the app behind Face ID.
