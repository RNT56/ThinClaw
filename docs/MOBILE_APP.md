# ThinClaw Mobile / iOS Surface

> **Status: implemented across all planned milestones (R0→M5, B1→B3);
> real-device/TestFlight bring-up remains.** This doc is the canonical contract
> for the native Apple surface (iOS app, widgets, Live Activity, watchOS
> companion). Every milestone in the [status matrix](#implementation-status) has
> landed in code — the whole app (plus widget, watch, and Notification Service
> Extension embeds) builds for the iOS 26 / watchOS 26 simulators — but nothing
> here has been exercised against a real device, a live gateway, or Apple's push
> and TestFlight infrastructure; see the [honest remaining
> work](#remaining-work-honest) list. Security decisions live in
> [`docs/MOBILE_SECURITY.md`](MOBILE_SECURITY.md); when the two disagree, the
> security doc wins.

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
- **In-app device management (client milestone M5).** The Settings surface
  (`ThinClawCore.SettingsStore` behind a `DeviceManaging` seam, adapters in
  `FeatureSettings`) shows this device (`GET /api/devices/me` — name, platform,
  scopes, last seen, token prefix), lists its companions
  (`GET /api/devices/me/companions`) with a per-companion Revoke
  (`DELETE /api/devices/me/companions/{id}` — how the operator de-authorizes the
  watch from the phone), and Unpairs (`AppDependencies.unpair()` — self-revoke +
  credential erase + return to onboarding). **No device self-rename or
  self-rotate** is offered: `/api/devices/{id}/rename` and `/{id}/rotate` are
  admin-only (they reject a device token), so the phone has nothing to call over
  its own credential and those actions are deliberately absent from the client
  seam rather than wired to the admin path. Connection status comes from the live
  `GatewaySession` state + the Keychain credential (gateway name/instance id),
  **not** a `/api/gateway/status` call (not device-scoped); the gateway URL + pin
  reveal is Face-ID-gated (D-K3).

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

## TestFlight archive pipeline (milestone M5)

TestFlight builds are cut by a **fastlane-free** archive pipeline: raw
`xcodebuild archive` + `xcodebuild -exportArchive` authenticated with an App
Store Connect API key. There is **no fastlane, no `match`, and no committed
provisioning profile** — distribution signing is resolved at export time with
`-allowProvisioningUpdates`.

- CI: the `archive` job in `.github/workflows/ios.yml` runs **only on an
  `ios-v*` tag push** (`if: startsWith(github.ref, 'refs/tags/ios-v')`). It
  selects the newest Xcode, installs Tuist via mise, generates the workspace,
  then runs `apps/ios/scripts/archive.sh --upload`.
- Local: an operator can run `apps/ios/scripts/archive.sh` (add `--upload` to
  send to TestFlight) with their own Apple team. It mirrors the CI job.
- Export options live in `apps/ios/Config/ExportOptions.plist` (method
  `app-store`, `manageAppVersionAndBuildNumber`, `uploadSymbols`; `teamID` is a
  `$(DEVELOPMENT_TEAM)` placeholder substituted from a secret at export time).

**Credential-gated — the repo carries no Apple team.** Both the CI job and the
script are a no-op-with-message when the credentials below are absent, so tag
pushes never fail CI for contributors. To cut a build, push an `ios-v*` tag
(e.g. `git tag ios-v0.1.0 && git push origin ios-v0.1.0`) with these GitHub
Actions secrets configured:

| Secret | Meaning |
|---|---|
| `APPLE_DEVELOPMENT_TEAM` | Apple Developer team id (10 chars, e.g. `ABCDE12345`) |
| `APP_STORE_CONNECT_KEY_ID` | App Store Connect API key id |
| `APP_STORE_CONNECT_ISSUER_ID` | App Store Connect API key issuer id |
| `APP_STORE_CONNECT_KEY_P8` | The `.p8` private key, base64-encoded (`base64 -i AuthKey_XXXX.p8 \| tr -d '\n'`) |

The same four values map to environment variables (`DEVELOPMENT_TEAM`,
`APP_STORE_CONNECT_KEY_ID`, `APP_STORE_CONNECT_ISSUER_ID`,
`APP_STORE_CONNECT_KEY_P8`) for the local script; `DEVELOPMENT_TEAM` may instead
live in the gitignored `Config/Signing.local.xcconfig`. **Nothing secret is
committed** — the `.p8` is materialized into a temp dir at runtime and deleted
on exit.

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
| M5 | Polish + TestFlight | Read-only jobs glance (list/detail + polled event tail), in-app device management (self + companions only), per-category notification preview controls, app-switcher redaction, biometric-gated connection-detail reveal, Enhanced protection, accessibility, credential-gated archive pipeline |

**Program status:** all planned milestones (R0→M5, B1→B3) have landed in code.
The remaining work is bring-up against real hardware and Apple's services, not
new milestones — see [Remaining work](#remaining-work-honest).

## Implementation status

| Piece | Status |
|---|---|
| OpenAPI baseline (`openapi` feature, export-openapi, committed spec, CI check) | ✅ landed (R0) |
| `apps/ios/` scaffold (Tuist workspace, packages, SSE parser + tests, CI) | ✅ landed (R0); `tuist generate` verified locally and the whole `ThinClaw` app target builds for the iOS 26 simulator (`xcodebuild`) |
| Generated Swift client from the committed spec | ✅ landed (M1); `ThinClawAPI` REST client generated + committed, `swift test` passes |
| Device identity layer (pairing, tokens, scopes, TLS listener) | ✅ landed (B1) |
| Companion device tokens + watch low-risk-only approvals (backend) | ✅ landed (M4); `DeviceRecord.parent_device_id` (serde-default for legacy rows), `POST/GET /api/devices/me/companions` + `DELETE /api/devices/me/companions/{id}` (`devices:self` scope), companion grant = `chat`+`approvals` only, `DeviceStore::revoke_cascade` revokes all children with the parent (clearing push regs + tearing down streams), and server-side enforcement in `POST /api/chat/approval` refusing high/unknown-risk approvals from a watchOS companion (fail-closed, generic 403). `companion.created`/`companion.revoked` audit events; OpenAPI regenerated. Rust unit + `device_pairing_integration` coverage |
| Watch relay + companion provisioning (`ThinClawWatchBridge`, `App/Sources/WatchProvisioning`) | 🚧 authored (M4); the phone-side `WatchRelayHost` (`WCSessionDelegate`) mints the watch a reduced-scope companion via `POST /api/devices/me/companions` over the pinned client and delivers it (token + gateway URLs + SPKI pin + instance id) as `updateApplicationContext`; it answers watch RPCs by forwarding **the watch's own token opaquely** (never the phone's) to `POST /api/chat/approval` / `POST /api/chat/send` via the pure `WatchRelayResponder`, and pushes glanceable snapshots on significant changes. The watch-side `WatchGatewayRouter` selects relay→direct→queue (relay-first; direct is a pinned URLSession with the watch's own credential; else `transferUserInfo` queue + "pending sync") with per-route timeout fall-through inside the <5s approval budget. Re-provision when the watch reports a missing/stale credential; `DELETE` the companion on unpair (`WatchProvisioning` hook in `App/Sources`, activated while paired). Pure seams (envelope encode/decode, route selection, provisioning payload, and the watch-token-in-relayed-approval invariant) covered by `swift test` on macOS (39 tests); WCSession/watchOS whole-target compile is the Build stage's job |
| Watch client UI (`Watch/Sources`, `WatchWidgets/Sources`) | 🚧 authored (M4); glanceable `WatchRootView` (mirrored `AgentStatusSnapshot` phase + pending count + relay/direct/queued route badge), an approvals list offering Approve/Deny for **low-risk** entries only (high/unknown → "Approve on iPhone" hand-off, deny always allowed) with success/failure `WKInterfaceDevice` haptics and a round-trip spinner, and a dictated `AskView` (`TextField` dictation → `quickAsk`, sent/queued/failed receipt). All I/O is behind a `WatchGatewayProxy` seam (relay-first, the watch attaches its own reduced-scope token — D-K4); the **live** `RouterGatewayProxy` drives a `ThinClawWatchBridge` `WatchGatewayRouter` over real `WCSession.sendMessage` relay / pinned-URLSession direct / `transferUserInfo` queue transports, and `WatchSessionDelegate` activates the watch's `WCSession`, stores the provisioned companion credential in the watch keychain, mirrors snapshots into the watch App Group, and reloads the complication on a fresh mirror. The read-only `MirroredSnapshotProxy` remains only as the fallback for hosts without WatchConnectivity. `StatusComplication` is a real WidgetKit complication (circular/corner/inline) reading the mirror, resilient to a missing snapshot ("open watch app"). Pure seams swift-format-clean + `swift test`-green; whole-target watchOS compile is the Build stage's job and a full phone↔watch round-trip needs physically paired hardware |
| `GET /api/chat/approvals` pull endpoint | ✅ landed (B1) |
| First-party push + Live Activity emitter | ✅ landed (B2); content-free policy + notifier + `PUT/DELETE /api/devices/me/push`, `/live-activity/{id}`, `/live-activity-start-token`. Credential-gated (off without APNs config); mock-tested only, real Apple/TestFlight delivery pending |
| Live Activity run routing (backend) | ✅ landed (M3); Live Activity registration now carries `thread_id`/`job_id`, and the notifier auto-tracks a run from that association: run-progress events (`tool_started`/`status`) emit throttled Live Activity **updates** to the per-activity token, `response` emits the **end**, and a run beginning on a device with a push-to-start token but no active activity emits a one-shot **push-to-start**. A Live Activity token 410 prunes only that activity (or only the start token), never the alert registration. Notifier-level + policy unit tests with a mock `PushSender`; real Apple delivery still pending |
| iOS Live Activity manager (client) | 🚧 authored (M3); new `ThinClawLiveActivity` SPM package. A pure, macOS-testable `RunTracker` reducer + `RunInputClassifier` turn the active thread's `AgentEvent`s into start/update/end actions with a monotonically increasing `revision` (start-once, end-on-completion, local-vs-push reconciliation). The `@MainActor` `LiveActivityManager` drives ActivityKit behind an `ActivityController` protocol: on a run start it `Activity.request(pushType: .token)` (guarded on `areActivitiesEnabled`), updates the activity **locally** on progress (lower latency than push; the widget drops a late push by `revision`), ends on completion with a `.after` dismissal, forwards per-activity `pushTokenUpdates` to `PUT /api/devices/me/live-activity/{activity_id}` (`kind: agent_run` + `thread_id`) and `pushToStartTokenUpdates` to `PUT /api/devices/me/live-activity-start-token`, and `DELETE`s on end — all over the pinned client. `AgentRunLiveActivity.swift` renders the lock-screen + Dynamic Island (compact/minimal/expanded) from the content-free state; the tool name shows only from local SSE, never a push. Wired into `AppDependencies`/`ChatTab` (observe the active thread while paired; torn down on unpair). 30 pure-logic tests pass under `swift test`; whole-app/ActivityKit `xcodebuild` verification pending (Build stage) |
| iOS App Group snapshot pipeline (client) | 🚧 authored (M3); `SnapshotPublisher` (in `ThinClawCore`, macOS-testable, no UIKit) projects live agent state into the three App Group snapshots — `AgentStatusSnapshot`, `PendingApprovalsSnapshot` (now carrying a fail-closed `RiskTier` so the widget gates inline approve), `JobsSnapshot` — via an injected `SnapshotSink`/`SnapshotClock`, debouncing bursts (250 ms) and suppressing no-op writes. All human-authored strings pass through a `SnapshotPrivacyPolicy` (truncate to a char limit; drop entirely when previews are "app only") so snapshots stay content-free (D-N / data-at-rest). Three triggers feed one `fetch → write → reload`: foreground (live approvals mirroring + one kick from `startSessionIfPaired`), silent push (`BackgroundRefresh.handleSilentPush` now fetches gateway status + `GET /api/chat/approvals` + jobs list over the pinned client, writes via `SnapshotStoreSink`, then `WidgetCenter.reloadAllTimelines`), and `BGAppRefresh` (`BackgroundRefresh.register` under `com.thinclaw.ios.refresh`, registered in `AppDelegate.application(_:didFinishLaunchingWithOptions:)`, re-armed on background). Pure mapping/debounce/privacy + publisher→store integration covered by `swift test`; BGTaskScheduler/UIKit compile is the Build stage's job |
| iOS push client — APNs registration, notification handling, NSE | 🚧 authored (M2); `AppDelegate` registers for remote notifications while paired and `PUT`s the hex APNs token (`environment` = development in DEBUG) over the pinned client, `DELETE`s on unpair. `PushCoordinator` registers the four categories (message, approval-low with inline Approve/Deny, approval-high Open-only, job), routes content-free pushes to `thinclaw://` deep links, POSTs low-risk approve/deny inline, and hands silent pushes to `BackgroundRefresh` (which now re-fetches snapshots then reloads widgets — see the snapshot-pipeline row). New `ThinClawNotificationService` app-extension target (`com.apple.usernotifications.service`, App Group + shared Keychain entitlements) rewrites approval title/body from `GET /api/chat/approvals` over the shared pinned connection, generic text on failure. `tuist generate` wires the target; whole-app/NSE `xcodebuild` verification pending (Build stage) |
| iOS read-only jobs glance (client) | ✅ landed (M5); a UI-free `ThinClawCore.JobsStore` (macOS-tested) lists jobs + summary and loads a job's detail over the generated `ThinClawAPI` client (`GET /api/jobs`, `GET /api/jobs/{id}`, `jobs:read` scope), and tails a job's event log by **polling** `GET /api/jobs/{id}/events` — a JSON snapshot, **not** SSE (there is no per-job stream on the gateway) — folding new rows by a monotonic id cursor with geometric backoff and stopping on a terminal phase. `FeatureJobs` renders the list (pull-to-refresh, summary chips, empty state, an explicit "view only — can't cancel/restart from this device" footer) and detail (header, state transitions, live tail). Read-only by design: the phone token holds `jobs:read`, and job mutation routes are not device-scoped. `swift test`-covered on macOS; SwiftUI screen + `xcodebuild` are the Build stage's job |
| iOS Settings + device management (client) | ✅ landed (M5); `ThinClawCore.SettingsStore` (behind `DeviceManaging`/`Unpairing`/`ConnectionStateSource`/`TranscriptProtectionControlling` seams, adapters in `FeatureSettings`) shows this device (`GET /api/devices/me`), lists companions with per-companion Revoke (`DELETE /api/devices/me/companions/{id}`), and Unpairs (`AppDependencies.unpair()`). **No self-rename/rotate** (admin-only routes reject a device token — deliberately omitted). Per-category notification preview preferences (`NotificationPreferences` message/approval/job × always/when-unlocked/never + approvals-only "app only") persist to shared App Group defaults (`notif.preview.<category>`); the NSE reads the same keys before rewriting. Connection row = live `GatewaySession` state + Keychain gateway name/instance id (never `/api/gateway/status`); the URL/pin reveal is Face-ID-gated (D-K3). "Enhanced protection" drives `GRDBTranscriptStore.applyFileProtection(enhanced:)` + the shared overlay defaults key. `SettingsStore` (self/companion load + revoke + unpair with a mocked client, gated reveal, connection-state fold, enhanced-protection persist) and `NotificationPreferences` round-trip are `swift test`-covered on macOS (12 settings + 8 notification tests); the SwiftUI screen + `xcodebuild` are the Build stage's job |
| iOS client layers (transport, pairing, pinning, chat session) | ✅ landed (M1) as tested SPM packages — see the M1 caveat below |
| iOS app UX wiring — onboarding | ✅ landed (M1); `OnboardingStore` state machine + VisionKit QR scanner + manual path + app credential gating/unpair seam, store unit-tested on the iOS 26 simulator, whole app target compiles |
| iOS app UX wiring — chat + sessions | ✅ landed (M1); `ChatStore` folds live events through the pure `ChatTimelineReducer` (stream→final swap, tool rows, thread routing, out-of-order tolerance), optimistic send with an offline outbox, 429 composer cooldown, failure-row retry, history paging, and post-reconnect reconcile; `SessionsStore` is cache-first then refreshes via `threads()`; Sessions selection routes into the Chat tab. Pure logic (`ChatTimelineReducer`, `ComposerCooldown`, `SessionsListModel`) unit-tested on macOS; whole app target builds for the iOS 26 simulator |
| iOS transcript persistence (`ThinClawPersistence`) | ✅ landed (M1); GRDB 7.11.1 WAL `DatabasePool`, migration v1 (`threads`, `timeline_items` keyed `(thread_id,item_id)`, `outbox`), app-process-only, db dir `NSFileProtectionCompleteUntilFirstUserAuthentication` (iOS). `InMemoryTranscriptStore` kept; the `TranscriptStoring` contract is parameterized over both stores on macOS |
| mDNS advertisement | ✅ landed (B3); settings-gated `_thinclaw._tcp` advertiser behind the `mdns` cargo feature, default-off (`discovery.enabled` in settings or `MDNS_ENABLED`). Locator-only TXT (`version`, `api`, `name`, `fp` = base64url(sha256(instance-id))) — no tokens/secrets/paths; loopback binds skipped. Spawned from `GatewayChannel::start()`; responder in `src/channels/web/discovery.rs`. iOS-side consumption wired: `ThinClawAuth.BonjourBrowser` (`NWBrowser` + TXT parse) → `FeatureOnboarding.DiscoveryStore` → the onboarding "Discover on this network" affordance (locator-only — selecting a gateway only pre-fills the pairing URL; pairing still needs the QR secret and pinned-SPKI/instance-id verification). Unit-tested with a scripted browser; no live-LAN run yet |
| iOS TestFlight archive pipeline | ✅ landed (M5); fastlane-free `archive` job in `.github/workflows/ios.yml` (tag-triggered on `ios-v*`) + `apps/ios/scripts/archive.sh` + `apps/ios/Config/ExportOptions.plist`. `xcodebuild archive` → `-exportArchive` (method `app-store`) → `xcrun altool --upload-app`, authenticated with an App Store Connect API key (`-authenticationKeyPath`/`-allowProvisioningUpdates`), no `match`, no committed profiles. Credential-gated: a no-op-with-message when the `APPLE_DEVELOPMENT_TEAM` / `APP_STORE_CONNECT_KEY_ID` / `APP_STORE_CONNECT_ISSUER_ID` / `APP_STORE_CONNECT_KEY_P8` secrets are absent, so tag pushes never fail CI for contributors. YAML validated (actionlint); the real archive is unrun here (no Apple team). See the TestFlight section above |
| iOS accessibility + app-switcher redaction (client) | ✅ landed (M5); a pure `ThinClawCore.PrivacyRedactionPolicy` + `TimelineAccessibility` (VoiceOver labels for timeline items) are macOS-tested; `App/Sources/PrivacyOverlay` always covers the window on backgrounding/inactive `scenePhase`, and `FeatureChat`/`ThinClawDesign` honor VoiceOver + Reduce Motion. Pure logic `swift test`-covered; the SwiftUI overlay + `xcodebuild` are the Build stage's job |
| iOS app feature milestones | ✅ all planned milestones landed in code (M1 onboarding/chat/sessions/persistence; M2 approvals + push; M3 widgets + Live Activity + snapshot pipeline; M4 watch companion; M5 jobs glance + device management + notification controls + accessibility + archive pipeline). **Remaining is bring-up, not milestones:** real-device / live-gateway E2E, a live WatchConnectivity phone↔watch round-trip (the `WCSession` wiring itself now lands the `RouterGatewayProxy`/`WatchSessionDelegate` live, but the transport can only be exercised on paired hardware), whole-app/NSE/WidgetKit/ActivityKit + watchOS-target `xcodebuild`, live APNs/TestFlight delivery (needs an operator Apple team). See [Remaining work](#remaining-work-honest) |

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
live-gateway end-to-end pairing/chat run. The former `assistant_thread`
API-spec gap is now **fixed end to end**: `ThreadListResponse.assistant_thread`
is emitted as an optional plain `$ref` to `ThreadInfo` (via
`schema(nullable = false)` + `skip_serializing_if`) instead of the
generator-hostile `oneOf: [null, $ref]`, so swift-openapi-generator retains it as
`ThreadInfo?`. The Swift client has been regenerated, and
`GatewaySession.threadListing()` (with `threads()` layered over it) now surfaces
the pinned assistant thread; `AppDependencies.defaultThread()` prefers it as the
landing thread.

## Remaining work (honest)

All planned milestones have landed in code, but the surface has **never run
against real hardware or Apple's live services**. What is genuinely still open:

- **Real-device / live-gateway end-to-end.** No pairing, streaming chat,
  approval, jobs, or watch round-trip has been exercised against a running
  gateway on real hardware. Coverage today is `swift test` (pure logic + stores
  on macOS), plus the iOS-only Feature packages' XCTest targets run on a
  concrete iOS 26 simulator in CI (the `feature-tests` job / `scripts/
  feature-tests.sh`; currently `FeatureOnboarding`), plus a whole-app simulator
  `xcodebuild` **build** (the now-hard `build-app` gate). Still open: simulator
  UI tests for the other feature stores' async orchestration, and any E2E run.
- **Whole-target device builds.** The app + widget + NSE embeds build for the
  iOS 26 simulator; the ActivityKit/WidgetKit/`BGTaskScheduler` paths and the
  **watchOS whole-target compile** are the Build stage's job and are not yet
  verified against a device destination.
- **Live WatchConnectivity round-trip (tracked).** The live `WCSession` wiring is
  now in place on both sides — the watch's `RouterGatewayProxy` drives the bridge
  router over real relay/direct/queue transports, `WatchSessionDelegate` handles
  activation + provisioning + snapshot mirrors, and the phone pushes mirrors on
  significant changes. What remains is a full round-trip between a **physically
  paired iPhone + Apple Watch**: WatchConnectivity does not function end-to-end in
  the simulator, so provisioning, relay approve/deny/quick-ask, and mirror
  delivery can only be exercised on hardware.
- **Live APNs + Live Activity delivery.** The content-free push builder,
  notifier, and NSE rewrite are tested only against mocks/the local gateway. No
  real Apple push, Live Activity update/push-to-start, or NSE rewrite has been
  delivered.
- **TestFlight archive.** The fastlane-free pipeline is wired, credential-gated,
  and actionlint-validated, but **the repo carries no Apple team**, so the real
  `archive → export → upload` has never run. Cutting a TestFlight build requires
  an operator's own Apple Developer team and App Store Connect API key (see the
  [archive pipeline section](#testflight-archive-pipeline-milestone-m5)).

The `assistant_thread` API-spec gap is **resolved** (previously listed here as
open): the gateway emits `ThreadListResponse.assistant_thread` as an optional
plain `$ref` to `ThreadInfo` (`schema(nullable = false)` +
`skip_serializing_if`) instead of the `oneOf: [null, $ref]` that
swift-openapi-generator dropped, the Swift client is regenerated, and
`GatewaySession.threadListing()` surfaces the pinned assistant thread (preferred
by `AppDependencies.defaultThread()`).

## On-device watch relay verification (hardware only)

WatchConnectivity does not function end-to-end in the simulator, so the live
relay wiring can only be verified on a **physically paired iPhone + Apple Watch**
against a running gateway. Steps:

1. **Pair the phone.** Complete onboarding on the iPhone (QR pairing to the
   gateway) so a `DeviceCredential` lands in the shared Keychain.
2. **Provisioning.** Foreground the app with the watch paired. The phone's
   `WatchRelayHost` activates its `WCSession`, and on the watch reporting no
   credential it mints a companion (`POST /api/devices/me/companions`) and
   pushes it via `updateApplicationContext`. Confirm on the phone's Settings →
   companions list that an "Apple Watch" companion appears, and that the watch
   leaves the "Not paired yet" state.
3. **Mirror delivery.** Trigger a status change (start an agent run, or create a
   pending approval). Confirm the watch root view and the `StatusComplication`
   update to the new phase / pending count (the phone pushes on significant
   changes; the watch reloads the complication on a fresh mirror).
4. **Relay approve/deny.** With a **low-risk** pending approval, tap Approve on
   the wrist. Confirm the route badge reads "via iPhone" (relay), the spinner
   shows, a success haptic fires, and the gateway records the decision
   attributed to the **watch** companion device (not the phone). A high/unknown
   entry must show "Approve on iPhone" with no wrist approve; deny must work at
   any tier.
5. **Quick-ask.** Dictate a prompt in Ask; confirm a "Sent" receipt and that the
   gateway received a `chat/send` from the watch companion token.
6. **Queue path.** Put the phone out of range / background it and repeat an
   approve; confirm the badge reads "pending sync" and the action is queued
   (`transferUserInfo`), then delivered when the phone returns.
7. **Deprovision.** Unpair on the phone (Settings → sign out). Confirm the
   companion is `DELETE`d and the watch surface drops back to unprovisioned.

## Doc obligations (same-PR rule)

When implementing later milestones, update in the same PR: B1 →
`src/NETWORK_SECURITY.md` (paired-device trust boundary, listener inventory)
and `docs/CRATE_OWNERSHIP.md`; B2 → `docs/CHANNEL_ARCHITECTURE.md`; B3 →
`docs/deploy/remote-access.md` and `docs/BUILD_PROFILES.md`; M5 →
`apps/desktop/documentation/runtime-boundaries.md`; every milestone →
`FEATURE_PARITY.md` §11 and this doc's status matrix.
