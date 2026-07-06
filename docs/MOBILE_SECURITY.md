# Mobile Security Model

> **Status: canonical security spec; all phased milestones (B1→B3, M1→M5) have
> landed in code.** Canonical security spec for the ThinClaw iOS/watchOS
> surface. Every decision below is now implemented — the gateway-side controls
> are Rust-tested, and the client-side controls are `swift test`-covered with the
> whole app building for the iOS 26 simulator — with the honest exception that
> nothing has been exercised against a real device or Apple's live push
> infrastructure (see [`docs/MOBILE_APP.md`](MOBILE_APP.md) → Remaining work).
> `src/NETWORK_SECURITY.md` remains the authority for what the gateway
> enforces *today*; it gained the "Paired device client" trust boundary with
> milestone B1.

## Grounding constraints (verified against the codebase)

- Gateway auth today is a single shared bearer token compared with
  `subtle::ConstantTimeEq` (`crates/thinclaw-gateway/src/web/auth.rs`), with a
  `?token=` query fallback for browser SSE. `GatewayRequestIdentity` is
  already threaded through requests — the natural attach point for device
  identity.
- `crates/thinclaw-channels/src/pairing.rs` provides reusable machinery:
  15-min pending TTL, max 3 pending, 10-failures/5-min approval lockout,
  fs4 file-locked JSON stores. Its 8-char code (~40 bits) is *not* strong
  enough as the sole pairing credential for a network endpoint.
- `src/orchestrator/auth.rs` `TokenStore` is the scoped-token precedent
  (32-byte tokens, constant-time validation, revocation).
- The legacy APNs channel (`native_lifecycle_clients.rs`) puts message
  content into `aps.alert` — a documented disclosure to Apple that the
  first-party mobile path must not inherit.
- `NETWORK_SECURITY.md` finding **F-2** (no application-layer TLS) is
  partially closed for this surface by the TLS listener below.

## Threat model

Impact assumes attacker goal = full agent control (the agent runs tools and
acts on the operator's behalf).

| # | Threat | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| T1 | Lost/stolen phone | M | H | ThisDeviceOnly keychain; remote revoke with immediate stream disconnect; biometric gates on high-risk actions; inactivity auto-revoke |
| T2 | Token exfiltration via device backup | L | H | All credentials `*ThisDeviceOnly` — never restorable to another device, never in iCloud Keychain |
| T3 | MITM on LAN | M | H | Pinned TLS (SPKI from QR); plain HTTP to LAN refused |
| T4 | MITM on tailnet | VL | H | WireGuard already encrypts; pinned TLS still preferred; `vpn-http` mode is explicit opt-in, badged |
| T5 | Photographed / malicious QR | L | H/M | One-time 32-byte secret, single-use, 15-min TTL; "device paired" broadcast to all clients with one-tap revoke; optional explicit-confirm mode; QR carries gateway identity so a spoofed QR gains nothing about the real gateway |
| T6 | Pairing brute force | VL | H | 256-bit secret; lockout (10 fails/5 min); max 3 outstanding |
| T7 | Request replay | VL | M | TLS/WireGuard transport; v2 upgrade: proof-of-possession signing |
| T8 | Compromised widget process | L | M | v1: widget shares the device token (bounded by scopes; high-risk approvals refused outside the app); upgrade: distinct widget sub-token |
| T9 | Lock-screen leakage | H | L–M | Content-free pushes + local rewrite; Live Activity shows status enums only; per-category preview controls |
| T10 | Lost watch | L | M | Watch holds its own reduced-scope companion token (`chat`+`approvals` only), independently revocable and cascade-revoked with its parent; watch approvals enforced low-risk-only server-side (D-K4, landed M4); wrist-detection lock; no transcript persistence |
| T11 | Evil-twin Bonjour gateway | L | H | Discovery is a locator only; endpoint must present pinned SPKI + instance id before any credential is sent |
| T12 | Stale devices never revoked | H | M | `last_seen_at` surfacing, 90-day inactivity auto-revoke, device list UI, audit log |
| T13 | Server-side token store theft | L | M | Only SHA-256 hashes at rest; pairing secrets single-use and TTL'd |
| T14 | Token leakage via logs | M | H | `tcd_` prefix registered in the `LeakDetector`; device tokens header-only (never `?token=`); audit events carry device_id, never token material |

## Decisions

Each decision lists the rejected alternative. IDs are referenced from code
review and later phases.

### Pairing

- **D-P1 — QR credential is a one-time 32-byte random secret.** The QR is
  machine-read, so there is no usability ceiling on entropy. *Rejected:*
  reusing the 8-char channel-pairing code (too weak without mandatory human
  approval). A short typable code remains only as the no-camera fallback,
  behind the same lockout.
- **D-P2 — v1 tokens are opaque bearer; the device submits a Secure-Enclave
  P-256 public key at pairing (stored, not yet enforced).** Signed-request
  (PoP) auth adds per-request signing and nonce tracking, composes poorly
  with SSE streams and the watch, and defends a channel the pinned
  transport already covers. Collecting the key now gives v2 a clean
  upgrade: flip `require_pop` per device without re-pairing. *Rejected:*
  full challenge-response in v1; also rejected: not collecting the key.
- **D-P3 — Auto-approve pairing initiated from an authenticated surface.**
  Possession of the one-time secret is proof the operator initiated pairing
  (the QR only renders on authenticated surfaces). Completion is broadcast
  to all connected clients with a revoke affordance.
  `device_pairing.require_confirm=true` opts into a pending→approve flow.
  *Rejected:* mandatory confirmation (punishes the operator-holding-both-
  devices majority case).
- **D-P4 — Reuse `PairingStore` mechanics (TTL, max-pending, lockout, file
  locking) in a new store** (`~/.thinclaw/device-pairing.json`), not the
  channel files — channel pairing authorizes chat senders; device pairing
  authorizes API clients.

QR payload (versioned; unknown fields ignored):

```
thinclaw://pair?d=<base64url(json)>
{
  "v": 1,
  "urls": ["https://100.x.y.z:3443", "https://host.local:3443"],
  "fp":  "<base64url sha256 of TLS leaf SPKI>",   // omitted only in vpn-http mode
  "iid": "<stable gateway instance id>",
  "name": "<human label>",
  "sec": "<base64url 32-byte one-time secret>",
  "exp": <unix expiry, created + 15 min>
}
```

### Tokens

- **D-T1 — Format:** opaque 32 random bytes, base64url, fixed prefix `tcd_`
  (LeakDetector-greppable, human-recognizable in incident response).
  *Rejected:* JWT/macaroons — revocation needs a store anyway; opaque wins.
- **D-T2 — SHA-256 hash-at-rest**, hash-keyed lookup, `ct_eq` final compare.
  *Rejected:* argon2 (memory-hard hashing defends low-entropy inputs;
  pointless at 256 bits), plaintext-at-rest.
- **D-T3 — Long-lived + revocation**, no forced expiry: instant server-side
  revoke (closes live SSE/WS), 90-day inactivity auto-revoke, on-demand
  rotation (`POST /api/devices/{id}/rotate`), optional `expires_at` honored
  if set. *Rejected:* access+refresh token pairs — both would sit in the
  same Keychain item; complexity without a distinct threat covered.
- **D-T4 — Scopes v1:** `chat` (send/abort/history/threads/events/ws),
  `approvals` (`/api/chat/approval` — separate from `chat` so watch/widget
  stay least-privilege), `jobs:read`, `devices:self`. Never grantable:
  settings, secrets/providers, extensions/skills, memory write, logs,
  restart, pairing admin. `credential_prompt` responses are excluded from
  v1 device scopes.
- **D-T5 — Audit log** (`~/.thinclaw/device-audit.jsonl`): pairing
  created/consumed/failed, device paired/approved/rotated/revoked/
  auto-revoked, auth failures, scope denials, push-token registrations.
  Token material never logged (device_id + last 4 chars only).

### Transport

- **D-X1 — The gateway grows an optional rustls TLS listener**
  (`GATEWAY_TLS=off|auto|on`, default `auto` = started on first pairing;
  port 3443; rcgen self-signed P-256 cert with SANs for tailscale/LAN
  IPs + `.local`; key 0600 under `~/.thinclaw/tls/`). The SPKI fingerprint
  travels in the QR, so the very first connection is verified — **no
  trust-on-first-use window**. *Rejected:* documentation-only
  ("bring your own reverse proxy") — fails the Mac-mini/Pi operator and
  leaves LAN pairing unprotected; requiring Tailscale certs — vendor
  coupling, does nothing for plain-LAN users.
- **D-X2 — App connection policy matrix** (enforced client-side; the app
  never falls back from pinned TLS to HTTP for a paired gateway):

| Endpoint class | Pinned TLS | Public-chain TLS | Plain HTTP |
|---|---|---|---|
| Tailscale space | ✔ default | ✔ | only if paired `vpn-http` (opt-in, badged) |
| LAN / `.local` | ✔ default | ✔ | ✘ refused |
| Loopback (dev) | ✔ | ✔ | debug builds only |
| Public | ✔ | ✔ | ✘ refused |

  ATS stays strict: `NSAllowsLocalNetworking` only; never
  `NSAllowsArbitraryLoads`. The ATS IP-literal exemption makes `vpn-http`
  *possible*; it is not a guarantee to architect around.
- **D-X3 — Bonjour is a locator, never an authenticator.** Rediscovered
  endpoints must present the pinned SPKI and pairing-time instance id
  before the token is used.

### On-device credentials

- **D-K1 — Device token access class:
  `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`.** Widgets and the
  Notification Service Extension must read it while the device is locked;
  `ThisDeviceOnly` removes it from all backup-restore paths. The escalation
  control moves to explicit biometric gating (D-K3) — that is the honest
  trade. *Rejected:* `WhenUnlocked*` (breaks every locked-screen refresh);
  non-ThisDeviceOnly classes.
- **D-K2 — Keychain/entitlement matrix.** Shared access group
  `$(AppIdentifierPrefix)com.thinclaw.shared`; App Group
  `group.com.thinclaw.shared` for non-secret state (gateway URL, instance
  id, pin, widget snapshots).

| Item | App | Widgets | NSE | Watch | Class |
|---|---|---|---|---|---|
| Device token `tcd_…` | RW | R | R | — | AfterFirstUnlockThisDeviceOnly |
| SPKI pin + instance id | RW | R | R | R (copy) | App Group file (integrity, not secrecy) |
| SE P-256 key (v2 PoP) | use | use | use | — | Secure Enclave; cannot reach the watch |
| Watch token | issue | — | — | RW | watch keychain, AfterFirstUnlockThisDeviceOnly |

- **D-K3 — Biometric policy.** No gate: reading chat, sending, viewing jobs,
  approving **low-risk** tools. Face ID required: **high-risk** tool
  approvals (refused entirely in widget/watch — deep-link into the app),
  device management, server settings, revealing connection details. The
  `risk` tier is computed gateway-side (single source of truth) and carried
  in approval events and push categories, never approximated client-side.
- **D-K4 — Watch: own reduced-scope token, provisioned over
  WatchConnectivity, phone-relay primary transport** (no Tailscale on
  watchOS). Even relayed, the watch attaches its own token so the gateway
  attributes and revokes it independently. Watch scopes: `chat`,
  `approvals` (low-risk only, enforced server-side by device class).
  *Rejected:* sharing the phone token — all-or-nothing revocation, copies
  the highest-value credential to the weakest-authenticated device.

  **Backend landed (M4).** The watch token is modeled as a *companion* device:
  a child `DeviceRecord` carrying `parent_device_id`, minted by an
  already-paired parent at `POST /api/devices/me/companions` (`devices:self`
  scope) with a deliberately narrowed grant of **`chat` + `approvals` only**
  (no `jobs:read`, no `devices:self` — so the companion cannot enumerate or
  manage devices, self-register push over HTTP, or mint sub-companions; the
  relay-first watch does its device management through the paired phone). The
  parent lists (`GET /api/devices/me/companions`) and revokes
  (`DELETE /api/devices/me/companions/{id}`) its own companions, and any
  revocation **cascades**: `DeviceStore::revoke_cascade` revokes every child
  with the parent in one locked write, clearing push/Live-Activity
  registrations, and the registry broadcasts each revoked id so live SSE/WS
  streams tear down synchronously. The **low-risk-only rule is enforced
  server-side** in the approve handler (`POST /api/chat/approval`): when the
  authenticated principal is a watchOS companion, a `High` (or unknown, fail-
  closed) risk tier — read from the gateway-side pending-approvals cache, the
  D-K3 single source of truth — is refused with a generic `403`; a `deny` is
  always allowed, and phone/full-token principals are never affected. Audit:
  `companion.created` / `companion.revoked`.

  **Watch client UI landed (M4).** `apps/ios/Watch/Sources` renders the
  wrist surface behind a `WatchGatewayProxy` seam (relay-first; the watch
  attaches its OWN reduced-scope token, never the phone's): a glanceable status
  (mirrored `AgentStatusSnapshot` + relay/direct/queued route badge), an
  approvals list that offers an **approve action for low-risk entries only** —
  high-risk (and, fail-closed, unknown-tier) entries show "Approve on iPhone"
  and hand off, matching the server-side refusal as defense in depth — with a
  round-trip spinner and success/failure `WKInterfaceDevice` haptics, and a
  dictated quick-ask with a sent/queued/failed receipt. Deny is always
  available at any tier. `apps/ios/WatchWidgets/Sources` renders a WidgetKit
  status complication from the watch App Group mirror. The watch surface is
  driven **live**: `RouterGatewayProxy` routes every write through a
  `WatchGatewayRouter` over the relay/direct/queue transports (forwarding the
  watch's own token). The read-only `MirroredSnapshotProxy` remains only as the
  fallback for build targets without WatchConnectivity.

  **Relay + companion provisioning wired live (M4 + follow-up).**
  `ThinClawWatchBridge` carries the bridge half; the
  `App/Sources/WatchProvisioning` hook activates the phone-side host while paired
  and the watch-side `WatchSessionDelegate` activates the watch's `WCSession`,
  persists the provisioned credential, mirrors snapshots into the watch App
  Group, and reloads the complication on a fresh mirror. The phone pushes
  snapshot mirrors on every significant change (`AppDependencies` →
  `WatchProvisioning.mirror`). The phone-side `WatchRelayHost`
  (`WCSessionDelegate`) mints the watch a companion
  (`POST /api/devices/me/companions`, pinned parent client) when the watch
  reports a missing/stale credential and delivers it as
  `updateApplicationContext` (token + gateway URLs + SPKI pin + instance id);
  the watch persists it in its **own** keychain (`WatchCompanionCredential`,
  distinct key). Relayed RPCs forward the **watch's own token opaquely** — the
  phone assembles a client whose bearer is the forwarded watch token, never its
  own, so the gateway attributes/revokes the watch independently (a unit test
  asserts the watch token, not the phone token, rides in a relayed approve). A
  missing token or a 401/403 from a forwarded call fails closed to
  `reprovisionRequired`. The watch-side `WatchGatewayRouter` selects
  relay→direct→queue (direct = pinned URLSession with the watch's credential;
  else `transferUserInfo` queue + "pending sync") with per-route timeout
  fall-through inside the <5s approval budget; the watch app supplies the live
  `WCSession.sendMessage` relay, pinned-URLSession direct, and `transferUserInfo`
  queue transports. On unpair the phone `DELETE`s the companion (the parent
  cascade also covers it). Pure seams are `swift test`-covered on macOS (39
  tests); the whole-target watchOS compile is a CI hard gate (the `watch-build`
  job), and a full phone↔watch round-trip still needs physically paired hardware
  (WatchConnectivity does not function end-to-end in the simulator).

### Push privacy

- **D-N1 — Content-free pushes + local rewrite.** APNs payloads carry only
  category + event ids (`mutable-content: 1`); a Notification Service
  Extension fetches real content from the gateway over the pinned
  connection and rewrites locally. If unreachable, the generic text stands.
  The legacy `apns` channel's content-in-alert behavior is a documented
  finding the mobile path must not reuse. *Rejected:* content-in-payload —
  ships transcript fragments through Apple, trivially avoidable.
- **D-N2 — Live Activity payloads:** status enum + progress + short job id
  only; no prompt text, no tool arguments. The Live Activity *registration*
  also associates the activity with the `thread_id`/`job_id` it mirrors so the
  gateway can route run events to the right per-activity update token; these
  are opaque ids, never content. Update/end pushes go only to the
  per-activity token and push-to-start only to the start token — a rejected
  Live Activity token prunes just that entry, never the alert registration.
- **D-N3 — Per-category controls:** previews always/when-unlocked/never;
  approvals can be set "app only"; interactive approve-from-notification
  offered only for low-risk categories.

### Data at rest & logging hygiene

- Transcript cache: `NSFileProtectionCompleteUntilFirstUserAuthentication`;
  optional "Enhanced protection" upgrades to `Complete` (documented cost:
  no locked-screen refresh). **Client wiring landed (M5).** The Settings toggle
  drives `GRDBTranscriptStore.applyFileProtection(enhanced:)`, which re-tags the
  cache directory + SQLite sidecars to `Complete`/`CompleteUntilFirstUserAuth`,
  and persists the choice under the shared
  `com.thinclaw.ios.settings.enhancedProtection` defaults key. This toggle gates
  the heavier data-at-rest file-protection class only; the app-switcher
  redaction overlay below is always on, independent of it.
- Never cached on device: secrets-store values, `credential_prompt`
  contents, the pairing secret (freed after pairing).
- Widget snapshots carry at most: thread title, truncated preview
  (respecting the preview setting), approval title + risk badge. Enforced
  client-side (M3) by `SnapshotPrivacyPolicy` in the `SnapshotPublisher`
  pipeline: every human-authored string is truncated to a character cap before
  it reaches the App Group container, and the "app only" preview setting drops
  titles/descriptions entirely, leaving only status enums, counts, ids, and the
  risk tier. Tool names and risk tiers are structural (not operator prose) and
  are always retained so a redacted widget can still label and gate a row.
- OSLog: tokens/URLs logged with `privacy: .private`; no body logging.
- Gateway: `tcd_` registered in the `LeakDetector` scrub patterns; device
  tokens rejected on the `?token=` query path.
- **App-switcher redaction overlay landed (M5).** `App/Sources/PrivacyOverlay`
  covers the window whenever `scenePhase` goes `.inactive`/`.background`, so the
  app-switcher snapshot never shows transcript content. It is **always on** — a
  cheap, unconditional privacy measure independent of the "Enhanced protection"
  toggle (which gates only the transcript file-protection class). Pure
  `PrivacyRedactionPolicy` is macOS-tested;
  the SwiftUI overlay compiles in the `build-app` hard gate.

### Gateway-side hardening (B1)

1. ✅ **Landed.** Pairing: admin-only `pair/start` (max 3 outstanding, 15-min
   TTL); public `pair/complete` protected by the 32-byte single-use secret,
   atomic consume under file lock, lockout, a dedicated rate limiter, and a
   body limit (landed at 4 KB, not the 1 KB originally sketched here), audit
   on every attempt.
2. 📋 **Not yet implemented.** Device auth failures: sliding-window counter on
   `tcd_`-prefixed 401s → tarpit + audit burst alert. Auth failures are
   audited per-attempt today; the dedicated failure-rate tarpit/burst-alert
   is not wired.
3. ✅ **Landed.** Constant-time comparisons throughout; **fixed in passing**
   the non-constant-time `==` in `native_lifecycle.rs`
   `header_secret_matches_required` (now `subtle::ConstantTimeEq`).
4. ✅ **Landed.** Scope middleware returns identical 403 bodies for "no
   scope" and "unknown route" under a device principal (no route-existence
   leakage).
5. ✅ **Landed.** `DeviceRegistry::revoke` persists the revocation and
   broadcasts the revoked `device_id` on a `tokio::broadcast` channel
   (`subscribe_revocations`). Both the SSE (`/api/chat/events`) and WS
   (`/api/chat/ws`) handlers now subscribe via `device_revocation_guard` and
   tear down a live device-token connection synchronously the moment its
   device is revoked (WS stops both forwarding and accepting frames). APNs
   registration deletion on revoke is also implemented: `DeviceStore::revoke`
   clears the device's APNs push registration, all per-activity Live Activity
   tokens, and the push-to-start token in the same locked write, and the store
   setters reject re-attaching any push token to a revoked device — so a
   revoked device's stale tokens can never be pushed to.
6. 📋 **Not yet wired.** `DeviceRegistry::sweep_inactive` implements the
   90-day-inactivity selection logic (unit-tested), but nothing schedules or
   calls it yet — there is no running daily auto-revoke sweep.

## Client & push implementation status (M1–M5 / B2)

Annotates the decisions above against what the iOS client (`apps/ios/`) and the
first-party push notifier actually implement today. All client milestones
(M1–M5) have landed in code; the caveats below are about real-device / live-Apple
exercise, not missing implementation. The security primitives are
unit-tested Swift package layers; the **onboarding/pairing and chat/sessions
features are now wired end to end in code** (`OnboardingStore` state machine +
VisionKit QR scanner + app credential gating; `ChatStore`/`SessionsStore` over
the live pinned `GatewaySession` + GRDB cache), the whole app target compiles for
the iOS 26 simulator, and the onboarding store is covered by simulator-run unit
tests. **No real-device or live-gateway pairing/chat run has been exercised**,
and the chat/sessions async orchestration has no simulator UI tests (coverage is
at the pure-reducer level). See the
[M1 caveat in `docs/MOBILE_APP.md`](MOBILE_APP.md#implementation-status).

Phase 1 — iOS client security primitives (`ThinClawAuth`,
`swift test`-covered on macOS, no simulator):

- ✅ **D-P1 QR payload parse/validate + pairing flow.** `PairingPayload.parse`
  decodes the `thinclaw://pair?d=…` link, rejects unknown versions and
  expired/at-expiry payloads, and exposes the gateway URLs, SPKI fingerprint,
  instance id, and one-time secret. `FeatureOnboarding` now composes this into
  the full flow: `OnboardingStore.handleScanned` parses and advances to a
  confirm step (gateway name/id + a D-X2 transport badge distinguishing
  pinned/public-chain TLS from a badged `vpn-http` warning), `LivePairingService`
  filters candidate URLs through `ConnectionPolicy`, submits the SE public key,
  and drives `POST /api/devices/pair/complete` over the pinned session. The
  camera scanner is a VisionKit `DataScannerViewController` behind device
  support + a camera-permission gate, with an always-available manual path
  (paste link, or gateway URL + short code) for the simulator. **Confirm-mode
  (`device_pairing.require_confirm`) is now surfaced**: a 202 response parks the
  store in a `pendingApproval` state instead of pairing. Every `PairingError`
  maps to an actionable, retryable failure message. *Not done:* no live-gateway
  or real-device run has exercised the round-trip yet.
- ✅ **D-P2 Secure-Enclave keypair.** `DeviceKeyPair.generate` creates a
  non-exportable P-256 key in the Secure Enclave
  (`kSecAttrTokenIDSecureEnclave`, `AfterFirstUnlockThisDeviceOnly`,
  `.privateKeyUsage`) and transparently falls back to a software CryptoKit key
  on the simulator, returning the public key as base64 SPKI for the pairing
  body. Proof-of-possession is still not enforced (v2), matching the decision.
- ✅ **D-X1/D-X2 SPKI pinning + connection policy.** `SPKIEncoder` rebuilds the
  leaf SPKI DER, `SPKIFingerprint` computes bare unpadded base64url SHA-256 and
  compares constant-time, and `PinnedSessionDelegate` enforces the pin over a
  live `SecTrust` (bypassing chain validation only for the pinned anchor).
  `ConnectionPolicy` implements the full D-X2 matrix purely (tailnet/loopback/
  LAN/public × pinned/public-chain/plaintext), refusing plaintext to LAN/public
  and never falling back from a pinned gateway to HTTP.
- ✅ **D-K1/D-K2 Keychain credential storage.** `DeviceCredential` persists the
  `tcd_` token via a `KeychainStoring` abstraction
  (`SecItemKeychainStore`) and `DeviceToken.redacted` keeps token material out
  of logs. *Not done:* the shared access group / App Group entitlement wiring is
  in the target shells and now builds through the Tuist workspace (the `build-app`
  / `watch-build` CI gates), but the entitlements are only enforced at runtime on a
  signed build — that still needs on-device verification.

Phase 2 — push privacy (B2, `thinclaw-gateway`/`thinclaw-channels`/root,
Rust-tested):

- ✅ **D-N1 content-free payload builder.** `push_policy` builds every alert as
  a generic `aps.alert` (`mutable-content: 1`) plus an id-only `tc` dict; tests
  assert no message text, tool name, or parameters ever serialize into the
  payload. The runtime notifier carries the payload verbatim and never logs it.
- ✅ **D-N2 Live Activity payloads.** The gateway-**pushed** content-state
  carries only `{phase, progress?, revision}` with a monotonic revision and a
  ≥15 s/activity throttle; the tool name never rides in a pushed state.
  Background wakes are bounded by a per-device 3/hour budget. On the client
  (M3), `ThinClawLiveActivity`'s `LiveActivityManager` drives the activity
  **locally** while foregrounded and may include the tool name in a *local*
  update only — that state never transits APNs, and a late gateway push is
  superseded by the higher-`revision` local update the widget already applied.
  The `LiveActivity` **registration** associates the activity with its
  `thread_id` (`kind: agent_run`), an opaque id, so the notifier can route run
  events to the per-activity update token; on run end the client `DELETE`s that
  registration. The device's push-to-start token is registered so a killed app
  can be spawned.
- ✅ **D-N3 per-category controls** and the Notification Service Extension
  rewrite (client-side, M2/M5, landed in code): authored in `apps/ios`. The app registers four
  categories — `THINCLAW_MESSAGE`, `THINCLAW_APPROVAL_LOW` (inline Approve/Deny),
  `THINCLAW_APPROVAL_HIGH` (Open-only → deep-link → in-app Face ID), and
  `THINCLAW_JOB` — so an inline approve action is offered for low-risk approvals
  only. The `ThinClawNotificationService` extension re-fetches approval content
  over the shared pinned connection (`GET /api/chat/approvals`, device token from
  the shared Keychain group) and rewrites the visible title/body locally,
  leaving the generic text when the gateway is unreachable. Content-free pushes
  carry only the `tc` id dict end to end. **Per-category preview toggles landed
  (M5).** `ThinClawCore.NotificationPreferences` models per-category modes
  (message/approval/job × always/when-unlocked/never, plus approvals-only
  "app only"); the in-app Settings surface persists them through
  `NotificationPreferencesStore` into the shared App Group defaults
  (`notif.preview.<category>`), and the `ThinClawNotificationService` extension
  reads the same keys before rewriting an approval — `never`/`app only` (and
  `when-unlocked` while the device is locked, probed fail-closed) leave the
  generic content-free text. Pure model + persistence round-trip are
  macOS-tested; the SwiftUI Settings screen compiles in the `build-app` hard gate
  (only the on-device/live-Apple behavior is still pending, as in Phase 1).
- ✅ **D-K3 gateway-side risk classifier (single source of truth).**
  `thinclaw_gateway::web::devices::approval_risk::classify` maps a tool name to
  `ApprovalRisk::{Low, High}` from an auditable substring allowlist:
  read-only/informational verbs (`read`/`search`/`list`/`get`/…) are `low`;
  side-effecting, egress, browser, filesystem-mutating, deploy/send, and **any
  unrecognised** tool default to `high` (least-privilege — over-gating costs one
  Face ID prompt, under-gating approves a destructive action from a lock screen).
  A high-risk substring wins even when a read-ish word co-occurs. The tier
  serialises snake_case onto the `approval_needed` SSE event and the
  `GET /api/chat/approvals` pending entries, and the push notifier derives the
  APNs category from it — the client and NSE never approximate risk locally
  (Rust-tested; carried through the OpenAPI snapshot to the generated client).
- ✅ **D-K3 client biometric gate (landed in code, M2/M5).** `ApprovalsStore` fires an
  injected `BiometricGating` (`LAContext` Face ID) before a **high-risk approve**
  and never before deny or a low-risk decision; the widget/watch omit the approve
  action for high-risk entirely (deep-link into the app). The M5 Settings surface
  (`SettingsStore`) reuses the same `BiometricGating` seam to gate the reveal of
  the gateway URL + pinned fingerprint (device-management/server-detail reveal),
  and re-hides them on backgrounding. Push registration is
  device-token-authenticated via the `devices:self` scope
  (`PUT/DELETE /api/devices/me/push`). The screen compiles in the `build-app` hard
  gate; the live-Apple / on-device caveat from D-N3 still applies.

## v1 simplifications (explicit, each with an upgrade path)

1. **Bearer, not key-bound, tokens** — transport carries the burden; upgrade
   = per-device PoP enforcement using the pairing-time SE key.
2. **Widget shares the app token** — bounded by scopes + high-risk refusal;
   upgrade = distinct widget sub-token.
3. **`AfterFirstUnlock` availability** — compensated by revocation, biometric
   gates, inactivity expiry.
4. **`vpn-http` mode exists** — opt-in, badged, excluded from LAN; kept for
   operators who refuse cert management.

## NETWORK_SECURITY.md additions (due with B1, same PR)

✅ Done — see `src/NETWORK_SECURITY.md`.

- Threat-model row: *Paired device client* — authenticated, least-privilege,
  per-device scoped tokens, QR pairing, pinned TLS or tailnet transport.
- Network surface inventory rows: gateway TLS listener (3443,
  `GATEWAY_TLS`), public `POST /api/devices/pair/complete`.
- Authentication mechanisms row: device token (constant-time: yes,
  hash-at-rest: yes, header-only).
- Findings: F-2 partially resolved (device clients); legacy APNs channel
  content disclosure documented; webhook header compare made constant-time.
- Review checklist: new routes must declare a device scope; device tokens
  excluded from query-param auth.
