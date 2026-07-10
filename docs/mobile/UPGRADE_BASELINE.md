# ThinClaw Apple Production Upgrade Baseline

Status: hardening and live-service certification in progress.

## Source baseline

- Branch: `feat/audit-hardening-main`
- Baseline commit and `origin/main`: `bda7a61f492040b275fae2fb7f49509c381e2fcd`
- First hardened TestFlight version: `0.2.0`
- Minimum deployment: iOS 18.0 and watchOS 11.0
- Progressive enhancement: iOS/watchOS 26 Liquid Glass behind availability checks
- OpenAPI SHA-256 at regenerated upgrade head: `2f434fa9c56ba942ed764934959b76a30891daf26f546b79a87a3f98774206dd`

The pre-upgrade code did not contain committed reference screenshots. The new
XCUITest target retains onboarding screenshots in its `.xcresult`; core-state
reference screenshots remain a release gate until deterministic paired fixtures
cover them.

## Local verification evidence (2026-07-10)

- Tuist `ThinClaw` scheme: 10/10 app, notification-extension, routing, and UI
  tests passed on iPhone 17 Pro / iOS 26.5; the two UI paths retain screenshots.
- iOS-only packages: direct simulator test targets for FeatureApprovals,
  FeatureChat, FeatureJobs, FeatureOnboarding (36/36), FeatureSessions,
  FeatureSettings, and typed AppRoute (4/4) all passed.
- ThinClawCore: 131/131 passed; persistence, snapshot, transport, API, auth,
  Live Activity, and Watch Bridge suites also pass in their package lanes.
- Gateway: format and library checks pass; six durable-approval tests, both QR
  rendering/wiring tests, and all 15 pairing/device/companion integration tests
  pass. Every root integration target compiles with the durable registry.
- Release compilation: generic iOS and generic watchOS builds passed under
  Swift 6 strict concurrency with first-party warnings-as-errors. The only
  emitted warning is Xcode's AppIntents metadata skip for a target without an
  AppIntents dependency.
- Archive infrastructure: the final source produced an unsigned `0.2.0`
  archive with local build number `20260710` at
  `build/ThinClaw-unsigned.xcarchive`; signed export/upload is external.
- Contract: Rust OpenAPI drift check passes; both snapshots share SHA-256
  `2f434fa9c56ba942ed764934959b76a30891daf26f546b79a87a3f98774206dd`, and
  Swift client regeneration is deterministic.
- This host has iOS/watchOS 26.2 and 26.5 simulators, but no iOS 18/watchOS 11
  simulator runtime. Minimum-OS execution therefore remains a designated
  release-runner gate; deployment manifests and generic-device targets are 18/11.

## Architecture decisions

- [ADR-001: supported Apple platforms](ADR-001-supported-platforms.md)
- [ADR-002: gateway-scoped local state](ADR-002-gateway-scoped-state.md)
- [ADR-003: durable authoritative approvals](ADR-003-authoritative-approvals.md)

## Upgrade exit checklist

- [x] Source branch synchronized with `origin/main` without touching unrelated work.
- [x] iOS 18+/watchOS 11+ manifests and iOS 26 availability fallbacks.
- [x] Application-lifetime composition coordinator and stable feature stores.
- [x] Relaunch-safe pairing, server-authoritative identity, typed routes, replacement confirmation.
- [x] Gateway-hashed transcript namespaces, scoped snapshots, deterministic local cleanup.
- [x] Durable gateway approvals and authoritative client replacement semantics.
- [x] Stable idempotent app/widget outboxes and encrypted widget queue.
- [x] Background refresh/NSE completion ownership and last-known-good snapshots.
- [x] App icon, launch assets, privacy manifest, semantic styling, iPad sidebar.
- [x] Strict CI graph, generic device builds, app unit/UI targets, release archive validation.
- [ ] Live gateway acceptance matrix on a signed physical iPhone and paired Apple Watch.
- [ ] APNs sandbox/production and Live Activity token lifecycle evidence.
- [ ] TestFlight 0.2.0 upload, upgrade install, migration verification, and 48-hour soak.

The final three gates require Apple Developer credentials, APNs configuration,
a physical iPhone, a paired Apple Watch, and elapsed soak time. They cannot be
truthfully closed by simulator evidence.
