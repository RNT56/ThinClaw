# ThinClaw for iOS + watchOS

Native Swift surface for a self-hosted ThinClaw instance: iOS app, widgets,
Live Activity, and watchOS companion. Strictly a **gateway client** over the
documented network contract — no embedded runtime
(see `apps/desktop/documentation/runtime-boundaries.md`).

Canonical docs: [`docs/MOBILE_APP.md`](../../docs/MOBILE_APP.md) (contract +
milestones) and [`docs/MOBILE_SECURITY.md`](../../docs/MOBILE_SECURITY.md)
(security model — read before touching pairing, tokens, push, or transport).

## Status

The **M1 client is implemented**, the **M2 approvals + push surfaces are
authored**, the **M3 widgets + Live Activity + snapshot pipeline are authored**,
and the **M4 watchOS companion is landed backend-side + authored client-side**:
the SPM packages, the onboarding flow, the chat + sessions surfaces, the
risk-tiered approvals surface, the home-screen widgets, the agent-run Live
Activity manager, the App Group snapshot pipeline, and the watch surface (relay
bridge, companion provisioning, wrist approvals/dictation, status complication)
are all wired, and the whole `ThinClaw` app target (plus its widget, watch, and
Notification Service Extension embeds) **builds for the iOS 26 simulator**. The
M4 companion-token backend (mint/list/revoke + cascade + watch low-risk-only
enforcement) is **landed and Rust-tested** in the gateway. All pure-logic
packages pass `swift test` on macOS with no simulator; the logic behind the
feature stores, the `RunTracker`/`LiveActivityManager`, the `SnapshotPublisher`,
and the watch relay/route seams is unit-tested. What remains is real-device /
live-gateway exercise, a whole-app/NSE/WidgetKit/ActivityKit + watchOS-target
`xcodebuild` verification, live APNs delivery, and the jobs/settings surfaces
(M3+). See the caveats below the table.

| Piece | Status |
|---|---|
| `ThinClawTransport` (SSE parser, event decoder, reconnect, `GatewaySession`/`GatewayStream`, reconcile) | ✅ implemented + fixture-tested (89 tests) |
| `ThinClawCore` (domain models, chunk coalescer, `ChatTimelineReducer`, `ComposerCooldown`, `SessionsListModel`, `ReconcileResult`, `SnapshotPublisher`/`SnapshotPrivacyPolicy`/`SnapshotStoreSink`) | ✅ implemented + tested (82 tests) |
| `ThinClawSnapshotKit` (App Group snapshots, Live Activity attributes) | ✅ implemented + tested (12 tests) |
| `ThinClawLiveActivity` (`RunTracker` reducer + `RunInputClassifier`, `LiveActivityManager` over `ActivityController`/`LiveActivityRegistrar`) | 🚧 authored (M3); pure logic tested on macOS (30 tests); ActivityKit compile pending Build stage |
| `ThinClawAuth` (pairing parse, Secure-Enclave keypair, SPKI pinning, connection policy, Keychain) | ✅ implemented + tested (23 tests) |
| `ThinClawAPI` (generated REST client + auth/error shell) | ✅ generated + tested (13 tests) |
| `ThinClawPersistence` (GRDB WAL store + in-memory store, parameterized `TranscriptStoring` parity) | ✅ implemented + tested (11 tests) |
| `FeatureOnboarding` (pairing state machine, QR scanner, app wiring) | ✅ implemented; store unit-tested on the iOS simulator (27 tests) |
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
| `FeatureJobs` / `FeatureSettings` | 🚧 placeholder screens (M3+); compiled into the app build, no stores yet |
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
