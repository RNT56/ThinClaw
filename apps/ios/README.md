# ThinClaw for iOS + watchOS

Native Swift surface for a self-hosted ThinClaw instance: iOS app, widgets,
Live Activity, and watchOS companion. Strictly a **gateway client** over the
documented network contract — no embedded runtime
(see `apps/desktop/documentation/runtime-boundaries.md`).

Canonical docs: [`docs/MOBILE_APP.md`](../../docs/MOBILE_APP.md) (contract +
milestones) and [`docs/MOBILE_SECURITY.md`](../../docs/MOBILE_SECURITY.md)
(security model — read before touching pairing, tokens, push, or transport).

## Status

The **M1 client is implemented** and the **M2 approvals + push surfaces are
authored**: the SPM packages, the onboarding flow, the chat + sessions surfaces,
and the risk-tiered approvals surface are all wired, and the whole `ThinClaw` app
target (plus its widget, watch, and Notification Service Extension embeds)
**builds for the iOS 26 simulator**. All pure-logic packages pass `swift test` on
macOS with no simulator; the logic behind the feature stores is unit-tested. What
remains is real-device / live-gateway exercise, a whole-app/NSE `xcodebuild`
verification, live APNs delivery, and the jobs/settings surfaces (M3+). See the
caveats below the table.

| Piece | Status |
|---|---|
| `ThinClawTransport` (SSE parser, event decoder, reconnect, `GatewaySession`/`GatewayStream`, reconcile) | ✅ implemented + fixture-tested (89 tests) |
| `ThinClawCore` (domain models, chunk coalescer, `ChatTimelineReducer`, `ComposerCooldown`, `SessionsListModel`, `ReconcileResult`) | ✅ implemented + tested (49 tests) |
| `ThinClawSnapshotKit` (App Group snapshots, Live Activity attributes) | ✅ implemented + tested (12 tests) |
| `ThinClawAuth` (pairing parse, Secure-Enclave keypair, SPKI pinning, connection policy, Keychain) | ✅ implemented + tested (23 tests) |
| `ThinClawAPI` (generated REST client + auth/error shell) | ✅ generated + tested (13 tests) |
| `ThinClawPersistence` (GRDB WAL store + in-memory store, parameterized `TranscriptStoring` parity) | ✅ implemented + tested (11 tests) |
| `FeatureOnboarding` (pairing state machine, QR scanner, app wiring) | ✅ implemented; store unit-tested on the iOS simulator (27 tests) |
| `FeatureChat` / `FeatureSessions` (`ChatStore`, `SessionsStore` over the live session + cache) | ✅ implemented; pure logic tested on macOS, async orchestration not yet UI-tested |
| `App` composition (`AppDependencies` real graph, `AppRouter` deep links, `PushCoordinator`, `AppDelegate`, scenePhase lifecycle) | ✅ wired; whole app target builds for the iOS 26 simulator |
| `FeatureApprovals` (risk-tiered cards over `ApprovalsStore`: cold-load + live fan-out + `POST /api/chat/approval`, Face-ID gate on high-risk approve) | ✅ implemented (M2); pure store logic tested on macOS |
| iOS push client (`AppDelegate` APNs register/`PUT`/`DELETE`, `PushCoordinator` risk-split categories, `ThinClawNotificationService` NSE content rewrite) | 🚧 authored (M2); whole-app/NSE `xcodebuild` + live APNs delivery pending |
| B3 discovery consumption (`BonjourBrowser` `NWBrowser` + TXT parse, `DiscoveryStore`, onboarding "Discover on this network") | ✅ wired (B3); locator-only, tested with a scripted browser; no live-LAN run |
| `FeatureJobs` / `FeatureSettings`, widgets, watch | 🚧 placeholder screens (M3+); compiled into the app build, no stores yet |
| Tuist manifests / CI `build-app` job | ✅ `tuist generate` succeeds locally and the app builds; CI `build-app` job unverified here |

**M1 caveat:** the onboarding **and** chat/sessions flows are wired end to end
in code. `OnboardingStore` is a real state machine (parse → confirm + D-X2
transport badge → pair via `PairingService` → persist → done/pending/failed);
the camera scanner uses VisionKit behind availability + a permission gate with
an always-present manual path (paste link, or gateway URL + short code); and the
app flips between onboarding and the tab shell from the Keychain credential with
an unpair seam (`AppDependencies.unpair()` best-effort self-revokes then erases).
`ChatStore.send`/`apply` are **implemented, not stubs**: `ChatStore` folds live
`GatewaySession` events through the pure `ChatTimelineReducer` (stream→final
swap, tool rows, thread routing), sends optimistically with an offline outbox,
applies a 429 composer cooldown, offers failure-row retry, pages history, and
reconciles after reconnect; `SessionsStore` hydrates from the `ThinClawPersistence`
cache then refreshes via `threads()`, and a Sessions tap routes into Chat.
Transcript persistence is the GRDB WAL `DatabasePool` store. The whole `ThinClaw`
app target builds for the iOS 26 simulator (verified with `xcodebuild`), and the
onboarding store carries 27 simulator-run tests.

**M2 caveat:** the approvals surface and push client are **authored, not yet
live-verified**. `ApprovalsStore` (UI-free) cold-loads `GET /api/chat/approvals`,
folds live `approval_needed` events, and posts `POST /api/chat/approval`
decisions; the `ApprovalCard` renders the gateway-computed `risk` tier and a
Face-ID gate (injected `BiometricGating`, `LAContext`) fires on high-risk
**approve** only (D-K3). `AppDelegate` registers for remote notifications while
paired and `PUT`s/`DELETE`s the APNs token over the pinned client;
`PushCoordinator` registers the risk-split categories (inline Approve/Deny for
`THINCLAW_APPROVAL_LOW`, Open-only deep-link for `THINCLAW_APPROVAL_HIGH`) and
routes content-free pushes to `thinclaw://` deep links; the
`ThinClawNotificationService` app-extension rewrites approval title/body from
`GET /api/chat/approvals` over the shared pinned connection, leaving generic text
on failure. Chat also surfaces inline `auth_required` (opens the OAuth URL, never
captures the token) and `credential_prompt` (handle-on-desktop per D-T4) cards.
All exercised by `swift test` on macOS. Still **not** done: no
real-device or live-gateway pairing/chat run has happened (treat E2E as
unverified), the `ChatStore`/`SessionsStore` async orchestration has no simulator
UI tests (coverage is at the pure-reducer level), a whole-app/NSE `xcodebuild`
verification and live APNs delivery are pending, per-category *preview* toggles
(D-N3) and the jobs/settings surfaces are unbuilt. **Known API-spec gap:** the gateway's `assistant_thread` is
modeled in the committed OpenAPI snapshot as `oneOf: [null, $ref]`, which
swift-openapi-generator drops from `ThreadListResponse`, so `GatewaySession.threads()`
cannot surface the pinned assistant thread until that spec pattern is corrected
and the client regenerated. No real-device run and TestFlight are still pending.

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
NotificationService/                 # NSE shell (content-free push rewrite, M2)
Packages/                            # all real code, local SPM packages
  ThinClawAPI        generated gateway client + auth/error shell
  ThinClawTransport  SSE parser, AgentEvent decoding, reconnect policy
  ThinClawCore       domain models, reducers, ApprovalsStore, AuthPrompt
  ThinClawPersistence transcript cache + outbox (GRDB at M1)
  ThinClawAuth       pairing payload, Keychain, Bonjour discovery (_thinclaw._tcp)
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
