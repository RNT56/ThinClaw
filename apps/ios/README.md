# ThinClaw for iOS + watchOS

Native Swift surface for a self-hosted ThinClaw instance: iOS app, widgets,
Live Activity, and watchOS companion. Strictly a **gateway client** over the
documented network contract — no embedded runtime
(see `apps/desktop/documentation/runtime-boundaries.md`).

Canonical docs: [`docs/MOBILE_APP.md`](../../docs/MOBILE_APP.md) (contract +
milestones) and [`docs/MOBILE_SECURITY.md`](../../docs/MOBILE_SECURITY.md)
(security model — read before touching pairing, tokens, push, or transport).

## Status

**All planned milestones have landed in code (R0→M5, B1→B3).** The SPM
packages, the onboarding flow, chat + sessions, the risk-tiered approvals
surface, the home-screen widgets, the agent-run Live Activity manager, the App
Group snapshot pipeline, the watch surface (relay bridge, companion
provisioning, wrist approvals/dictation, status complication), the read-only
jobs glance, in-app device management (self + companions only), per-category
notification preview controls, the app-switcher redaction overlay, accessibility
passes, and the credential-gated TestFlight archive pipeline are all wired, and
the whole `ThinClaw` app target (plus its widget, watch, and Notification Service
Extension embeds) **builds for the iOS 26 simulator**. The gateway-side backends
(device identity B1, push B2, discovery B3, companion tokens M4) are **landed and
Rust-tested**. All pure-logic packages pass `swift test` on macOS with no
simulator; the logic behind every feature store (`ChatStore`, `SessionsStore`,
`ApprovalsStore`, `JobsStore`, `SettingsStore`), the
`RunTracker`/`LiveActivityManager`, the `SnapshotPublisher`, and the watch
relay/route seams is unit-tested.

**What remains is bring-up, not new milestones:** no real-device or live-gateway
end-to-end run has happened; the ActivityKit/WidgetKit/`BGTaskScheduler` paths
and the **watchOS whole-target compile** (wiring the `ThinClawWatchBridge` relay
proxy into `WatchApp` in place of the default `MirroredSnapshotProxy`) are the
Build stage's job; live APNs / Live Activity delivery is untested against Apple;
and the TestFlight archive has never actually run because **the repo carries no
Apple team**. See the caveats below the table and
[`docs/MOBILE_APP.md`](../../docs/MOBILE_APP.md) → Remaining work.

| Piece | Status |
|---|---|
| `ThinClawTransport` (SSE parser, event decoder, reconnect, `GatewaySession`/`GatewayStream`, reconcile) | ✅ implemented + fixture-tested (89 tests) |
| `ThinClawCore` (domain models, chunk coalescer, `ChatTimelineReducer`, `ComposerCooldown`, `SessionsListModel`, `ReconcileResult`, `SnapshotPublisher`/`SnapshotPrivacyPolicy`/`SnapshotStoreSink`) | ✅ implemented + tested (82 tests) |
| `ThinClawSnapshotKit` (App Group snapshots, Live Activity attributes) | ✅ implemented + tested (12 tests) |
| `ThinClawLiveActivity` (`RunTracker` reducer + `RunInputClassifier`, `LiveActivityManager` over `ActivityController`/`LiveActivityRegistrar`) | 🚧 authored (M3); pure logic tested on macOS (30 tests); ActivityKit compile pending Build stage |
| `ThinClawAuth` (pairing parse, Secure-Enclave keypair, SPKI pinning, connection policy, Keychain) | ✅ implemented + tested (23 tests) |
| `ThinClawAPI` (generated REST client + auth/error shell) | ✅ generated + tested (13 tests) |
| `ThinClawPersistence` (GRDB WAL store + in-memory store, parameterized `TranscriptStoring` parity) | ✅ implemented + tested (11 tests) |
| `FeatureOnboarding` (pairing state machine, QR scanner, app wiring) | ✅ implemented; store unit-tested on the iOS simulator (32 tests), run in CI by the `feature-tests` job (`scripts/feature-tests.sh`) |
| `FeatureChat` / `FeatureSessions` (`ChatStore`, `SessionsStore` over the live session + cache) | ✅ implemented; pure logic tested on macOS, async orchestration not yet UI-tested |
| `App` composition (`AppDependencies` real graph, `AppRouter` deep links, `PushCoordinator`, `AppDelegate`, scenePhase lifecycle) | ✅ wired; whole app target builds for the iOS 26 simulator |
| `FeatureApprovals` (risk-tiered cards over `ApprovalsStore`: cold-load + live fan-out + `POST /api/chat/approval`, Face-ID gate on high-risk approve) | ✅ implemented (M2); pure store logic tested on macOS |
| iOS push client (`AppDelegate` APNs register/`PUT`/`DELETE`, `PushCoordinator` risk-split categories, `ThinClawNotificationService` NSE content rewrite) | 🚧 authored (M2); whole-app/NSE `xcodebuild` + live APNs delivery pending |
| B3 discovery consumption (`BonjourBrowser` `NWBrowser` + TXT parse, `DiscoveryStore`, onboarding "Discover on this network") | ✅ wired (B3); locator-only, tested with a scripted browser; no live-LAN run |
| Widgets (`AgentStatusWidget`, `PendingApprovalsWidget`, `QuickAskWidget`, `AgentRunLiveActivity`) | 🚧 authored (M3); read App Group snapshots via `WidgetSnapshotAccess`, inline Approve/Deny gated to low-risk rows (high/unknown → Deny + deep link), Dynamic Island renders the content-free run state; WidgetKit compile pending Build stage |
| Companion device tokens + watch low-risk-only approvals (gateway) | ✅ landed (M4); `POST/GET /api/devices/me/companions` + `DELETE /api/devices/me/companions/{id}` (`devices:self` scope), companion grant = `chat`+`approvals` only, `DeviceStore::revoke_cascade` (parent→children), and `POST /api/chat/approval` server-side refusal of high/unknown-risk approvals from a watchOS companion (fail-closed 403). Rust unit + `device_pairing_integration` coverage; OpenAPI regenerated |
| `ThinClawWatchBridge` (phone-side `WatchRelayHost` + companion provisioning) | 🚧 authored (M4); `WCSessionDelegate` mints the watch a companion over the pinned parent client, delivers it (token + URLs + SPKI pin + instance id) via `updateApplicationContext`, forwards relayed approve/quick-ask with the **watch's own token opaquely** (never the phone's), fails closed to `reprovisionRequired` on 401/403; `DELETE`s the companion on unpair. Pure seams tested on macOS (39 tests); WCSession/watchOS whole-target compile is Build stage |
| Watch UI (`Watch/Sources`: `WatchRootView`, `ApprovalsListView`, `AskView`) | 🚧 authored (M4); glanceable status (mirrored `AgentStatusSnapshot` + pending count + relay/direct/queued route badge), approvals list offering Approve/Deny for **low-risk** entries only (high/unknown → "Approve on iPhone" hand-off, deny always allowed) with `WKInterfaceDevice` haptics + round-trip spinner, and a dictated quick-ask; all I/O behind a `WatchGatewayProxy` seam (relay-first, watch's own reduced-scope token). Default `MirroredSnapshotProxy` renders the App Group mirror and queues writes until the bridge relay proxy is wired. Typechecks + swift-format-clean for watchOS 26 |
| Watch complication (`WatchWidgets/Sources/StatusComplication`) | 🚧 authored (M4); real WidgetKit complication (circular/corner/inline) reading the watch App Group mirror, resilient to a missing snapshot ("open watch app"); WidgetKit compile pending Build stage |
| `FeatureJobs` (read-only jobs glance over `ThinClawCore.JobsStore`) | ✅ implemented (M5); lists jobs + summary and loads detail over the generated client (`GET /api/jobs`, `/api/jobs/{id}`, `jobs:read`), tails events by **polling** `GET /api/jobs/{id}/events` (JSON snapshot, not SSE) with an id-cursor fold + backoff; UI has pull-to-refresh, summary chips, empty state, a "view only — can't cancel/restart from this device" footer, and a detail tail. `JobsStore` macOS-tested; SwiftUI screen at Build stage |
| `FeatureSettings` (device management + notification prefs over `ThinClawCore.SettingsStore`) | ✅ implemented (M5); shows this device (`GET /api/devices/me`), lists companions with per-companion Revoke (`DELETE /api/devices/me/companions/{id}`), Unpairs (`AppDependencies.unpair()`) — **no self-rename/rotate** (admin-only routes reject a device token). Per-category notification preview prefs persist to shared App Group defaults the NSE reads; connection row from live `GatewaySession` state + Keychain identity (never `/api/gateway/status`); URL/pin reveal Face-ID-gated (D-K3); "Enhanced protection" drives `GRDBTranscriptStore.applyFileProtection(enhanced:)` (file-protection class only; the app-switcher overlay is always on). Stores macOS-tested; SwiftUI screen at Build stage |
| Accessibility + app-switcher redaction (`PrivacyRedactionPolicy`, `TimelineAccessibility`, `App/Sources/PrivacyOverlay`) | ✅ implemented (M5); pure redaction/VoiceOver logic macOS-tested, overlay always covers the window on background/inactive `scenePhase`, `FeatureChat`/`ThinClawDesign` honor VoiceOver + Reduce Motion; overlay verified at Build stage |
| TestFlight archive pipeline (`scripts/archive.sh`, `Config/ExportOptions.plist`, CI `archive` job) | ✅ implemented (M5); fastlane-free `xcodebuild archive` → `-exportArchive` → `xcrun altool`, App Store Connect API key auth, tag-triggered (`ios-v*`), credential-gated no-op-with-message when secrets absent. actionlint-validated; real archive unrun (no Apple team in repo) |
| Tuist manifests / CI `build-app` job | ✅ `scripts/tuist-generate.sh` + `xcodebuild build` verified locally from a fresh checkout (App/Resources deleted → generate exit 1→0 → **BUILD SUCCEEDED**); CI `build-app` is now a hard gate (was best-effort), fixing the graph-construction failure. Unverified only on the GitHub runner image itself |

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
(D-N3) were the M2-stage gaps; the jobs and settings surfaces landed later in M5.
The former `assistant_thread` API-spec gap is now **resolved**: the gateway emits
`ThreadListResponse.assistant_thread` as an optional plain `$ref` to `ThreadInfo`
(`schema(nullable = false)` + `skip_serializing_if`) instead of the
generator-hostile `oneOf: [null, $ref]`, the Swift client is regenerated, and
`GatewaySession.threadListing()` surfaces the pinned assistant thread (preferred
as the landing thread by `AppDependencies.defaultThread()`). No real-device run
and TestFlight are still pending.

**M3 caveat:** the widgets, the agent-run Live Activity manager, and the App
Group snapshot pipeline are **authored, not yet WidgetKit/ActivityKit
build-verified**. The four widgets read the App Group snapshots through
`WidgetSnapshotAccess` and degrade to placeholders on any read failure;
`PendingApprovalsWidget` offers inline Approve/Deny only on low-risk rows (the
`ApproveToolIntent` refuses high/unknown-risk, so a lock screen can never approve
a high-risk action — D-K3), and `AgentRunLiveActivity` renders the content-free
run state (lock screen + Dynamic Island compact/minimal/expanded). The snapshot
pipeline lives in `ThinClawCore`: `SnapshotPublisher` projects live agent state
into the three snapshots through an injected `SnapshotSink`/`SnapshotClock`,
debounces bursts, suppresses no-op writes, and runs every human-authored string
through `SnapshotPrivacyPolicy` (truncate to a char cap; drop titles/descriptions
entirely under the "app only" preview setting) so App Group snapshots stay
content-free (D-N / data-at-rest). Three triggers feed one fetch→write→reload:
foreground (live approvals mirroring + one kick from `startSessionIfPaired`),
silent push (`BackgroundRefresh.handleSilentPush` fetches gateway status +
`GET /api/chat/approvals` + jobs over the pinned client, writes via
`SnapshotStoreSink`, then `WidgetCenter.reloadAllTimelines`), and `BGAppRefresh`
(`com.thinclaw.ios.refresh`, registered at launch in `AppDelegate`, re-armed on
background). `ThinClawLiveActivity`'s `@MainActor` `LiveActivityManager` drives
ActivityKit behind an `ActivityController` protocol, updates the activity
**locally** on progress (a late gateway push is dropped by `revision`), forwards
per-activity + push-to-start tokens to the gateway over the pinned client, and
`DELETE`s on run end. Pure `RunTracker`/`RunInputClassifier`/`LiveActivityManager`
logic (30 tests) and the `SnapshotPublisher` mapping/debounce/privacy +
publisher→store integration pass `swift test` on macOS; the WidgetKit/ActivityKit
and `BGTaskScheduler` compile is the Build stage's job.

**M4 caveat:** the watchOS **companion-token backend is landed and Rust-tested**;
the **watch client (relay + UI + complication) is authored, not yet
watchOS-build-verified**. Backend: an already-paired parent mints a reduced-scope
companion at `POST /api/devices/me/companions` (`devices:self` scope; grant is
`chat`+`approvals` only — no `jobs:read`, no `devices:self`), lists via `GET`
and revokes via `DELETE /api/devices/me/companions/{id}`; revoking any device
**cascades** to its companions (`DeviceStore::revoke_cascade` — one locked write,
push regs cleared, live SSE/WS streams torn down). Watch approvals are enforced
**low-risk-only server-side** in `POST /api/chat/approval` (a watchOS-companion
principal is refused a high/unknown-risk approve with a generic 403, using the
D-K3 gateway-side risk tier; deny always allowed; phone tokens unaffected).
Client: the phone-side `WatchRelayHost` (`WCSessionDelegate`) provisions the
companion and delivers it over `updateApplicationContext`; the watch persists it
in its **own** keychain (`WatchCompanionCredential`) and, whether relaying or
going direct, attaches its **own** token — the relay forwards it opaquely, so the
gateway attributes/revokes the watch independently (unit-tested). The watch-side
`WatchGatewayRouter` selects relay→direct→queue with per-route timeout
fall-through inside the <5s approval budget. `Watch/Sources` renders the wrist
surface (glanceable status, low-risk-only approvals with hand-off, dictated
quick-ask) behind a `WatchGatewayProxy` seam; `WatchWidgets/Sources` renders the
status complication from the App Group mirror. There is **no transcript
persistence on the watch**. The pure seams pass `swift test` on macOS (39 tests);
the WCSession + watchOS whole-target compile (and wiring the bridge relay proxy
into `WatchApp` in place of the default `MirroredSnapshotProxy`) is the Build
stage's job. No real-device or live-gateway watch run has happened.

**M5 caveat:** the polish milestone is **landed in code, not real-device /
TestFlight-verified**. The read-only jobs glance, in-app device management (self +
companions only — the phone token can't manage arbitrary devices, and self-rename/
rotate are deliberately absent because those routes are admin-only), per-category
notification preview controls, the app-switcher redaction overlay, the
biometric-gated connection-detail reveal, "Enhanced protection", and accessibility
passes are all wired; the pure logic and stores (`JobsStore`, `SettingsStore`,
`NotificationPreferences`, `PrivacyRedactionPolicy`, `TimelineAccessibility`) are
`swift test`-covered on macOS. The fastlane-free archive pipeline is wired and
actionlint-validated but **has never actually run** — the repo carries no Apple
team, so cutting a TestFlight build requires an operator's own Apple Developer
team + App Store Connect API key. The SwiftUI screens, the overlay, and the whole
archive are Build-stage / operator work.

Milestones M1–M5 are defined in `docs/MOBILE_APP.md`; the honest cross-program
remaining-work list lives there under **Remaining work**.

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
  ThinClawLiveActivity RunTracker reducer + LiveActivityManager (agent-run activity)
  ThinClawDesign     Liquid Glass design system
  ThinClawWidgetKitShared  timeline providers + AppIntents (approve/deny/quick-ask)
  ThinClawWatchBridge WatchConnectivity relay + companion provisioning (watch holds its own token)
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
for p in ThinClawTransport ThinClawCore ThinClawSnapshotKit ThinClawLiveActivity ThinClawAuth ThinClawAPI ThinClawPersistence; do
  swift test --package-path Packages/$p
done
```

The SSE parser suite replays recorded gateway fixtures at adversarial
chunkings (byte-by-byte, mid-UTF-8 splits, CRLF variants) — extend fixtures
with `scripts/record-fixtures.sh` against a local gateway.

The Feature packages (`Packages/Features/*`) are `.iOS(.v26)`-only, so
`swift test` (which builds for the host Mac) cannot run them. Their XCTest
targets run on a concrete iOS 26 simulator via `xcodebuild test`:

```bash
scripts/feature-tests.sh   # auto-discovers feature packages that have tests
```

The script picks (and boots) an available iOS 26 simulator, then runs
`xcodebuild test -scheme <Package>` per package (no Tuist workspace needed —
SPM emits a scheme per package). If the machine has no iOS 26 runtime it
soft-skips with a warning and exits 0; set `FEATURE_TESTS_REQUIRE_SIM=1` to
make a missing simulator a hard failure. To run one package by hand:

```bash
cd Packages/Features/FeatureOnboarding
xcodebuild test -scheme FeatureOnboarding \
  -destination 'platform=iOS Simulator,name=iPhone 17,OS=latest'
```

Whole-app UI compiles through the Tuist workspace (`xcodebuild test`/`build`
with an iOS 26 simulator destination).

## CI

`.github/workflows/ios.yml` runs on `apps/ios/**`, `clients/openapi/**`, and
its own path. Every job first selects the newest installed Xcode (the packages
need Swift 6.2 / Xcode 26; the runner default is older). The hard gates are:

- **swift test** matrix — the macOS-testable packages (Transport, Core,
  SnapshotKit, Auth, API, Persistence).
- **feature package tests** — the `.iOS(.v26)`-only Feature packages on a
  concrete iOS 26 simulator via `scripts/feature-tests.sh` (soft-skips if the
  runner has no iOS 26 runtime; see Testing).
- **generated client drift** — `scripts/check-generated-drift.sh`.
- **swift-format lint**.
- **tuist build** — `scripts/tuist-generate.sh` + `xcodebuild build` for the
  iOS simulator. This was previously best-effort (`continue-on-error`); the
  failure was a reproducible, CI-only bug — `Project.swift` declares
  `resources: ["App/Resources/**"]`, the repo commits no files there, and git
  does not track empty directories, so on a fresh checkout the directory is
  absent and `tuist generate` errors during graph construction. The generator
  script now creates the declared-but-empty resource directory first, so this
  is a hard gate (verified locally by deleting `App/Resources`: generate goes
  from exit 1 to exit 0, then the app builds).

The tag-triggered **TestFlight archive** job (see below) reuses the same
`scripts/tuist-generate.sh` so it inherits the fix.

## API client generation

The gateway's OpenAPI snapshot is generated **from Rust** and committed at
`clients/openapi/thinclaw-gateway.openapi.json` (repo root; regenerate with
`cargo run --bin export-openapi -- generate`). Then:

```bash
scripts/generate-api.sh   # vendors the spec, runs swift-openapi-generator
```

Generated sources are committed; `scripts/check-generated-drift.sh` gates CI.
Never hand-edit generated code or the vendored spec.

## TestFlight (fastlane-free archive)

TestFlight builds use raw `xcodebuild archive` + `-exportArchive` +
`xcrun altool`, authenticated by an App Store Connect API key. No fastlane, no
`match`, no committed provisioning profiles — signing is resolved with
`-allowProvisioningUpdates` at export time.

```bash
# Local, with your own Apple team:
export DEVELOPMENT_TEAM=ABCDE12345
export APP_STORE_CONNECT_KEY_ID=XXXXXXXXXX
export APP_STORE_CONNECT_ISSUER_ID=00000000-0000-0000-0000-000000000000
export APP_STORE_CONNECT_KEY_P8="$(base64 -i AuthKey_XXXXXXXXXX.p8 | tr -d '\n')"
scripts/archive.sh --upload      # omit --upload to just produce the .ipa
```

CI cuts a build when you push an `ios-v*` tag (the `archive` job in
`.github/workflows/ios.yml`), reading the same four values from GitHub secrets
(`APPLE_DEVELOPMENT_TEAM`, `APP_STORE_CONNECT_KEY_ID`,
`APP_STORE_CONNECT_ISSUER_ID`, `APP_STORE_CONNECT_KEY_P8`). The pipeline is
**credential-gated**: with no team configured it is a no-op-with-message, so it
never fails CI for contributors. Export options live in
`Config/ExportOptions.plist` (`teamID` is a placeholder substituted at export
time). Nothing secret is committed. See `docs/MOBILE_APP.md` for the full
secret table.

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
