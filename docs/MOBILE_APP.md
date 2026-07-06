# ThinClaw Mobile / iOS Surface

> **Status: design specification + phased implementation.** This doc is the
> canonical contract for the native Apple surface (iOS app, widgets, Live
> Activity, watchOS companion). Nothing described here should be assumed
> shipped unless the [implementation status matrix](#implementation-status)
> says so. Security decisions live in [`docs/MOBILE_SECURITY.md`](MOBILE_SECURITY.md);
> when the two disagree, the security doc wins.

## What it is

A first-party native Swift client for a self-hosted ThinClaw instance:

- **iOS app** (iOS 26+, SwiftUI, Liquid Glass): pairing/onboarding, sessions,
  streaming chat, tool approvals, notifications, jobs glance, settings.
- **Widgets**: agent status glance, pending approvals (interactive
  approve/deny), quick-ask, plus a **Live Activity** (Dynamic Island) for
  agent runs.
- **watchOS companion** (watchOS 26+): actionable approvals, dictated quick
  prompts, status complication.

The client lives at [`apps/ios/`](../apps/ios/README.md) (Tuist-managed
workspace, mirrors the `apps/desktop/` precedent).

## Architecture position

The iOS surface is a **gateway client over explicit network contracts** —
never an embedded runtime. This is mandated by
[`apps/desktop/documentation/runtime-boundaries.md`](../apps/desktop/documentation/runtime-boundaries.md)
and mirrors how ThinClaw Desktop's remote mode works:

- REST for actions (`POST /api/chat/send`, `/api/chat/approval`, …)
- SSE (`GET /api/chat/events`) as the primary event stream; the WebSocket
  endpoint remains available but the mobile client is SSE-primary
- Per-device scoped tokens for auth (see below), never the operator's shared
  gateway token

The app talks to the gateway over a **tailnet/VPN or LAN**; public exposure
of the gateway is never required.

## API contract: OpenAPI from the gateway

The gateway's mobile-relevant API surface is described by an OpenAPI document
generated **from the Rust source** (utoipa annotations behind the
`thinclaw-gateway` crate's `openapi` feature):

- Committed snapshot: `clients/openapi/thinclaw-gateway.openapi.json`
- Regenerate: `cargo run --bin export-openapi -- generate`
- CI drift gate: `cargo run --bin export-openapi -- check`
- Served live at `GET /api/openapi.json` (authenticated)
- Swift client: generated with Apple's `swift-openapi-generator` by
  `apps/ios/scripts/generate-api.sh`; generated code is **committed** (iOS CI
  never needs the Rust toolchain)

Rule: any PR that changes a mobile-contract endpoint or DTO must regenerate
the snapshot in the same PR (the CI check enforces it).

## Device identity and pairing (backend milestone B1)

Design summary — full protocol and rationale in
[`docs/MOBILE_SECURITY.md`](MOBILE_SECURITY.md):

- **Pairing** is operator-initiated: `POST /api/devices/pair/start`
  (authenticated) yields a QR payload
  `thinclaw://pair?d=<base64url(json)>` containing gateway URLs, the TLS
  SPKI fingerprint, a stable instance id, and a **one-time 32-byte pairing
  secret** (15-min TTL, single-use, rate-limited). A short human-typable code
  covers the no-camera flow. The device redeems it at the public
  `POST /api/devices/pair/complete`.
- **Tokens** are per-device, opaque (`tcd_` + 32 random bytes, base64url),
  stored server-side only as SHA-256 hashes, long-lived with instant
  revocation (tears down live SSE/WS), 90-day inactivity auto-revoke, and
  on-demand rotation. Header-only — device tokens are never accepted via
  query parameter.
- **Scopes** (v1): `chat`, `approvals`, `jobs:read`, `devices:self`.
  Settings, secrets, extensions, memory-write, logs, and admin surfaces are
  never grantable to device tokens.
- Registry: `crates/thinclaw-gateway/src/web/devices/` with fs4-locked JSON
  persistence under `~/.thinclaw/`, managed via `GET /api/devices`,
  `/{id}/rename|revoke|rotate`, `GET /api/devices/me`, the web UI devices
  card, and `thinclaw devices` CLI.
- **Companion devices (backend milestone M4).** An already-paired device can
  mint a reduced-scope *companion* (the watch) via
  `POST /api/devices/me/companions` (`devices:self` scope; the current device
  is the parent). A companion is a child `DeviceRecord` with
  `parent_device_id` set, its own independently-revocable `tcd_` token, and a
  **narrowed grant of `chat` + `approvals` only** — no `jobs:read`, and no
  `devices:self` (so a companion cannot enumerate/manage devices or mint
  sub-companions). The parent lists its companions with
  `GET /api/devices/me/companions` and revokes one with
  `DELETE /api/devices/me/companions/{id}`. Revoking a device **cascades**:
  every companion whose `parent_device_id` matches is revoked in the same
  write, its push/Live-Activity registrations cleared and any live SSE/WS
  stream torn down. Approvals from a **watchOS companion are enforced
  low-risk-only server-side**: the approve handler (`POST /api/chat/approval`)
  refuses a high-risk (or unknown-risk) approval from a watch companion with a
  generic `403`, using the gateway-side risk tier as the single source of
  truth — the phone/full-token principals are unaffected.

## Transport and connectivity

- The gateway grows an **optional rustls TLS listener**
  (`GATEWAY_TLS=off|auto|on`, self-signed cert, SPKI fingerprint delivered in
  the pairing QR — no trust-on-first-use window). The app pins that SPKI.
- Connection policy: pinned TLS everywhere by default; plain HTTP only to
  Tailscale address space when the operator explicitly paired in
  `vpn-http` mode; plain HTTP to LAN or public addresses is always refused.
  ATS stays strict (`NSAllowsLocalNetworking` only).
- **LAN discovery** (backend milestone B3): mDNS/Bonjour advertisement of
  `_thinclaw._tcp` (settings-gated, default off). Discovery is a locator
  only — a rediscovered endpoint must present the pinned SPKI and instance id
  before any credential is sent.

## Push architecture (backend milestone B2)

First-party device push is a **new notifier that reuses the APNs transport**
(`ApnsPusher`, extracted from the existing native-lifecycle APNs client). The
existing `apns` chat channel is unchanged; the two are distinct surfaces
(see [`docs/CHANNEL_ARCHITECTURE.md`](CHANNEL_ARCHITECTURE.md)).

- Registration is device-token-authenticated (`PUT /api/devices/me/push`,
  `PUT /api/devices/me/live-activity/{activity_id}`, push-to-start token),
  superseding the shared-secret webhook for first-party devices. A Live
  Activity registration carries the `thread_id` (agent runs) or `job_id`
  (jobs) it mirrors, so the notifier can route run-progress events to that
  activity's per-activity update token.
- **Payloads are content-free** (category + ids only); a Notification Service
  Extension fetches real content from the gateway and rewrites locally, so
  Apple's servers never see message content. Live Activity payloads carry a
  status enum + progress only.
- Event mapping: responses → collapsible alerts (or a Live Activity **end**
  when they close a tracked run); `approval_needed` → time-sensitive
  actionable alert; job results → alerts; run status/tool-started → throttled
  Live Activity **updates** while the run is tracked; background wake pushes
  under a per-device budget. When a run begins on a device that registered a
  push-to-start token but has no active activity for the thread, the notifier
  emits a one-shot Live Activity **push-to-start** so a killed app can spawn
  the activity.

## Apple workspace shape

See [`apps/ios/README.md`](../apps/ios/README.md) for the authoritative
layout, toolchain setup (mise + Tuist), and testing guide. Summary:

- Four targets (app, widgets, watch app, watch widgets); generated
  `.xcodeproj` is never committed.
- All real code lives in local SPM packages: `ThinClawAPI` (generated
  client), `ThinClawTransport` (SSE parser/stream), `ThinClawCore` (domain +
  reducers), `ThinClawPersistence`, `ThinClawAuth` (Keychain/pairing/
  Bonjour), `ThinClawSnapshotKit` (App Group snapshots for widgets/watch),
  `ThinClawLiveActivity` (agent-run Live Activity manager + pure `RunTracker`),
  `ThinClawDesign` (Liquid Glass design system), `ThinClawWidgetKitShared`,
  `ThinClawWatchBridge`, and `Features/*`.
- Watch connectivity is relay-first (WatchConnectivity through the phone —
  there is no Tailscale on watchOS) with direct-HTTP fallback; the watch
  holds its own reduced-scope token.

## Milestones

| # | Deliverable | Summary |
|---|---|---|
| R0 | Repo prep | Design docs, `apps/ios/` scaffold, OpenAPI baseline + committed spec, CI |
| B1 | Backend device identity | Devices module, pairing, TLS listener, scopes, approvals pull endpoint, CLI, web UI card |
| M1 | iOS pairing + chat | Onboarding, pinned TLS, streaming chat, history reconcile, offline cache |
| B2 | Backend push | ApnsPusher, device push registration, Live Activity emitter |
| M2 | iOS approvals + push | Risk-tiered approval cards, actionable notifications, NSE rewrite |
| M3 | Widgets + Live Activity | Snapshot pipeline, 4 widgets, Dynamic Island via push |
| B3 | Backend discovery | mDNS advertiser (settings-gated) |
| M4 | watchOS | Companion token backend (mint/list/revoke + cascade, watch low-risk-only enforcement), relay + approvals + dictation + complication |
| M5 | Polish + TestFlight | Device management UI, accessibility, archive pipeline |

## Implementation status

| Piece | Status |
|---|---|
| OpenAPI baseline (`openapi` feature, export-openapi, committed spec, CI check) | ✅ landed (R0) |
| `apps/ios/` scaffold (Tuist workspace, packages, SSE parser + tests, CI) | ✅ landed (R0); `tuist generate` verified locally and the whole `ThinClaw` app target builds for the iOS 26 simulator (`xcodebuild`) |
| Generated Swift client from the committed spec | ✅ landed (M1); `ThinClawAPI` REST client generated + committed, `swift test` passes |
| Device identity layer (pairing, tokens, scopes, TLS listener) | ✅ landed (B1) |
| Companion device tokens + watch low-risk-only approvals (backend) | ✅ landed (M4); `DeviceRecord.parent_device_id` (serde-default for legacy rows), `POST/GET /api/devices/me/companions` + `DELETE /api/devices/me/companions/{id}` (`devices:self` scope), companion grant = `chat`+`approvals` only, `DeviceStore::revoke_cascade` revokes all children with the parent (clearing push regs + tearing down streams), and server-side enforcement in `POST /api/chat/approval` refusing high/unknown-risk approvals from a watchOS companion (fail-closed, generic 403). `companion.created`/`companion.revoked` audit events; OpenAPI regenerated. Rust unit + `device_pairing_integration` coverage |
| Watch relay + companion provisioning (`ThinClawWatchBridge`, `App/Sources/WatchProvisioning`) | 🚧 authored (M4); the phone-side `WatchRelayHost` (`WCSessionDelegate`) mints the watch a reduced-scope companion via `POST /api/devices/me/companions` over the pinned client and delivers it (token + gateway URLs + SPKI pin + instance id) as `updateApplicationContext`; it answers watch RPCs by forwarding **the watch's own token opaquely** (never the phone's) to `POST /api/chat/approval` / `POST /api/chat/send` via the pure `WatchRelayResponder`, and pushes glanceable snapshots on significant changes. The watch-side `WatchGatewayRouter` selects relay→direct→queue (relay-first; direct is a pinned URLSession with the watch's own credential; else `transferUserInfo` queue + "pending sync") with per-route timeout fall-through inside the <5s approval budget. Re-provision when the watch reports a missing/stale credential; `DELETE` the companion on unpair (`WatchProvisioning` hook in `App/Sources`, activated while paired). Pure seams (envelope encode/decode, route selection, provisioning payload, and the watch-token-in-relayed-approval invariant) covered by `swift test` on macOS (39 tests); WCSession/watchOS whole-target compile is the Build stage's job |
| Watch client UI (`Watch/Sources`, `WatchWidgets/Sources`) | 🚧 authored (M4); glanceable `WatchRootView` (mirrored `AgentStatusSnapshot` phase + pending count + relay/direct/queued route badge), an approvals list offering Approve/Deny for **low-risk** entries only (high/unknown → "Approve on iPhone" hand-off, deny always allowed) with success/failure `WKInterfaceDevice` haptics and a round-trip spinner, and a dictated `AskView` (`TextField` dictation → `quickAsk`, sent/queued/failed receipt). All I/O is behind a `WatchGatewayProxy` seam (relay-first, the watch attaches its own reduced-scope token — D-K4); the default `MirroredSnapshotProxy` renders the watch App Group mirror and queues writes until the `ThinClawWatchBridge` relay proxy is wired. `StatusComplication` is a real WidgetKit complication (circular/corner/inline) reading the mirror, resilient to a missing snapshot ("open watch app"). Typechecks + swift-format-clean for watchOS 26; `ThinClawWatch` target link pending the bridge's `WatchRelayHost` watchOS cross-compile fix |
| `GET /api/chat/approvals` pull endpoint | ✅ landed (B1) |
| First-party push + Live Activity emitter | ✅ landed (B2); content-free policy + notifier + `PUT/DELETE /api/devices/me/push`, `/live-activity/{id}`, `/live-activity-start-token`. Credential-gated (off without APNs config); mock-tested only, real Apple/TestFlight delivery pending |
| Live Activity run routing (backend) | ✅ landed (M3); Live Activity registration now carries `thread_id`/`job_id`, and the notifier auto-tracks a run from that association: run-progress events (`tool_started`/`status`) emit throttled Live Activity **updates** to the per-activity token, `response` emits the **end**, and a run beginning on a device with a push-to-start token but no active activity emits a one-shot **push-to-start**. A Live Activity token 410 prunes only that activity (or only the start token), never the alert registration. Notifier-level + policy unit tests with a mock `PushSender`; real Apple delivery still pending |
| iOS Live Activity manager (client) | 🚧 authored (M3); new `ThinClawLiveActivity` SPM package. A pure, macOS-testable `RunTracker` reducer + `RunInputClassifier` turn the active thread's `AgentEvent`s into start/update/end actions with a monotonically increasing `revision` (start-once, end-on-completion, local-vs-push reconciliation). The `@MainActor` `LiveActivityManager` drives ActivityKit behind an `ActivityController` protocol: on a run start it `Activity.request(pushType: .token)` (guarded on `areActivitiesEnabled`), updates the activity **locally** on progress (lower latency than push; the widget drops a late push by `revision`), ends on completion with a `.after` dismissal, forwards per-activity `pushTokenUpdates` to `PUT /api/devices/me/live-activity/{activity_id}` (`kind: agent_run` + `thread_id`) and `pushToStartTokenUpdates` to `PUT /api/devices/me/live-activity-start-token`, and `DELETE`s on end — all over the pinned client. `AgentRunLiveActivity.swift` renders the lock-screen + Dynamic Island (compact/minimal/expanded) from the content-free state; the tool name shows only from local SSE, never a push. Wired into `AppDependencies`/`ChatTab` (observe the active thread while paired; torn down on unpair). 30 pure-logic tests pass under `swift test`; whole-app/ActivityKit `xcodebuild` verification pending (Build stage) |
| iOS App Group snapshot pipeline (client) | 🚧 authored (M3); `SnapshotPublisher` (in `ThinClawCore`, macOS-testable, no UIKit) projects live agent state into the three App Group snapshots — `AgentStatusSnapshot`, `PendingApprovalsSnapshot` (now carrying a fail-closed `RiskTier` so the widget gates inline approve), `JobsSnapshot` — via an injected `SnapshotSink`/`SnapshotClock`, debouncing bursts (250 ms) and suppressing no-op writes. All human-authored strings pass through a `SnapshotPrivacyPolicy` (truncate to a char limit; drop entirely when previews are "app only") so snapshots stay content-free (D-N / data-at-rest). Three triggers feed one `fetch → write → reload`: foreground (live approvals mirroring + one kick from `startSessionIfPaired`), silent push (`BackgroundRefresh.handleSilentPush` now fetches gateway status + `GET /api/chat/approvals` + jobs list over the pinned client, writes via `SnapshotStoreSink`, then `WidgetCenter.reloadAllTimelines`), and `BGAppRefresh` (`BackgroundRefresh.register` under `com.thinclaw.ios.refresh`, registered in `AppDelegate.application(_:didFinishLaunchingWithOptions:)`, re-armed on background). Pure mapping/debounce/privacy + publisher→store integration covered by `swift test`; BGTaskScheduler/UIKit compile is the Build stage's job |
| iOS push client — APNs registration, notification handling, NSE | 🚧 authored (M2); `AppDelegate` registers for remote notifications while paired and `PUT`s the hex APNs token (`environment` = development in DEBUG) over the pinned client, `DELETE`s on unpair. `PushCoordinator` registers the four categories (message, approval-low with inline Approve/Deny, approval-high Open-only, job), routes content-free pushes to `thinclaw://` deep links, POSTs low-risk approve/deny inline, and hands silent pushes to `BackgroundRefresh` (which now re-fetches snapshots then reloads widgets — see the snapshot-pipeline row). New `ThinClawNotificationService` app-extension target (`com.apple.usernotifications.service`, App Group + shared Keychain entitlements) rewrites approval title/body from `GET /api/chat/approvals` over the shared pinned connection, generic text on failure. `tuist generate` wires the target; whole-app/NSE `xcodebuild` verification pending (Build stage) |
| iOS client layers (transport, pairing, pinning, chat session) | ✅ landed (M1) as tested SPM packages — see the M1 caveat below |
| iOS app UX wiring — onboarding | ✅ landed (M1); `OnboardingStore` state machine + VisionKit QR scanner + manual path + app credential gating/unpair seam, store unit-tested on the iOS 26 simulator, whole app target compiles |
| iOS app UX wiring — chat + sessions | ✅ landed (M1); `ChatStore` folds live events through the pure `ChatTimelineReducer` (stream→final swap, tool rows, thread routing, out-of-order tolerance), optimistic send with an offline outbox, 429 composer cooldown, failure-row retry, history paging, and post-reconnect reconcile; `SessionsStore` is cache-first then refreshes via `threads()`; Sessions selection routes into the Chat tab. Pure logic (`ChatTimelineReducer`, `ComposerCooldown`, `SessionsListModel`) unit-tested on macOS; whole app target builds for the iOS 26 simulator |
| iOS transcript persistence (`ThinClawPersistence`) | ✅ landed (M1); GRDB 7.11.1 WAL `DatabasePool`, migration v1 (`threads`, `timeline_items` keyed `(thread_id,item_id)`, `outbox`), app-process-only, db dir `NSFileProtectionCompleteUntilFirstUserAuthentication` (iOS). `InMemoryTranscriptStore` kept; the `TranscriptStoring` contract is parameterized over both stores on macOS |
| mDNS advertisement | ✅ landed (B3); settings-gated `_thinclaw._tcp` advertiser behind the `mdns` cargo feature, default-off (`discovery.enabled` in settings or `MDNS_ENABLED`). Locator-only TXT (`version`, `api`, `name`, `fp` = base64url(sha256(instance-id))) — no tokens/secrets/paths; loopback binds skipped. Spawned from `GatewayChannel::start()`; responder in `src/channels/web/discovery.rs`. iOS-side consumption wired: `ThinClawAuth.BonjourBrowser` (`NWBrowser` + TXT parse) → `FeatureOnboarding.DiscoveryStore` → the onboarding "Discover on this network" affordance (locator-only — selecting a gateway only pre-fills the pairing URL; pairing still needs the QR secret and pinned-SPKI/instance-id verification). Unit-tested with a scripted browser; no live-LAN run yet |
| iOS app feature milestones | 🚧 M1 core wired + building (onboarding, chat, sessions, persistence); remaining M1 = live-gateway/real-device E2E + TestFlight. M2–M5 planned |

**M1 caveat (verified 2026-07):** the client *layers* are implemented and
unit-tested — `ThinClawTransport` (SSE parse/decode/reconnect,
`GatewaySession`/`GatewayStream` with per-thread routing, ~10 Hz coalescing,
and post-reconnect history reconcile), `ThinClawAuth` (pairing-payload parse,
`DeviceKeyPair` Secure-Enclave-preferred P-256 keygen with software fallback,
`SPKIEncoder`/`SPKIFingerprint`, `ConnectionPolicy` D-X2 matrix,
`PinnedSessionDelegate` SPKI pinning, `DeviceCredential` Keychain storage),
`ThinClawCore`, `ThinClawSnapshotKit`, `ThinClawPersistence`, and the generated
`ThinClawAPI` REST client. All seven SPM packages pass `swift test` on macOS
with **no simulator**. The **onboarding feature layer is now wired**:
`OnboardingStore` drives parse → confirm (with a D-X2 transport badge) → pair
(`LivePairingService` over the pinned session) → persist → done/pending/failed,
the camera scanner is a VisionKit `DataScannerViewController` behind a
permission gate with an always-present manual path, and the app swaps between
onboarding and the tab shell from the Keychain credential with an unpair seam
(`AppDependencies.unpair()`, best-effort self-revoke). `FeatureOnboarding`
compiles for the iOS 26 simulator, the whole `ThinClaw` app target builds, and
the store carries 27 simulator-run unit tests. The **chat + sessions feature
layer is now wired**: `FeatureChat.ChatStore` subscribes to the `GatewaySession`
per-thread event stream and connection state, folds events through the pure
`ChatTimelineReducer` (in `ThinClawCore`), sends optimistically with an offline
outbox that flushes in order on reconnect, applies a 429 composer cooldown,
offers failure-row retry, pages history on scroll-top, and reconciles the
transcript after a reconnect; `FeatureSessions.SessionsStore` hydrates from the
`ThinClawPersistence` cache then refreshes via `threads()`, and a Sessions row
tap routes into the Chat tab (`AppRouter.openThread`). `AppDependencies` builds
the real graph — Keychain credential → `GatewayEndpoint` + a single pinned
`URLSession` shared by the SSE byte-stream provider and the generated REST
transport → `GatewaySession` → the GRDB-backed transcript store — and
starts/stops the event stream on `scenePhase`. Pure logic (`ChatTimelineReducer`,
`ComposerCooldown`, `SessionsListModel`, the `TranscriptStoring` parity contract
over both stores, GRDB round-trip/migration) is unit-tested on macOS; the whole
`ThinClaw` app target builds for the iOS 26 simulator. What is *not* yet done:
`ChatStore`/`SessionsStore` async orchestration has no simulator UI tests
(coverage is at the pure-reducer level), and there is no real-device or
live-gateway end-to-end pairing/chat run. **Known API-spec gap:** the gateway's
`assistant_thread` is modeled in the committed OpenAPI snapshot as
`oneOf: [null, $ref]`, which swift-openapi-generator drops from the generated
`ThreadListResponse`, so `GatewaySession.threads()` cannot surface the pinned
assistant thread until that spec pattern is corrected and the client
regenerated.

## Doc obligations (same-PR rule)

When implementing later milestones, update in the same PR: B1 →
`src/NETWORK_SECURITY.md` (paired-device trust boundary, listener inventory)
and `docs/CRATE_OWNERSHIP.md`; B2 → `docs/CHANNEL_ARCHITECTURE.md`; B3 →
`docs/deploy/remote-access.md` and `docs/BUILD_PROFILES.md`; M5 →
`apps/desktop/documentation/runtime-boundaries.md`; every milestone →
`FEATURE_PARITY.md` §11 and this doc's status matrix.
