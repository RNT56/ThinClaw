# Mobile Security Model

> **Status: design specification.** Canonical security spec for the ThinClaw
> iOS/watchOS surface. Implementation is phased (see
> [`docs/MOBILE_APP.md`](MOBILE_APP.md) milestones); until a phase lands,
> the corresponding sections describe intent, not current behavior.
> `src/NETWORK_SECURITY.md` remains the authority for what the gateway
> enforces *today* and gains a "Paired device client" trust boundary when
> milestone B1 ships.

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
| T10 | Lost watch | L | M | Watch holds its own reduced-scope token, independently revocable; wrist-detection lock; no transcript persistence |
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

### Push privacy

- **D-N1 — Content-free pushes + local rewrite.** APNs payloads carry only
  category + event ids (`mutable-content: 1`); a Notification Service
  Extension fetches real content from the gateway over the pinned
  connection and rewrites locally. If unreachable, the generic text stands.
  The legacy `apns` channel's content-in-alert behavior is a documented
  finding the mobile path must not reuse. *Rejected:* content-in-payload —
  ships transcript fragments through Apple, trivially avoidable.
- **D-N2 — Live Activity payloads:** status enum + progress + short job id
  only; no prompt text, no tool arguments.
- **D-N3 — Per-category controls:** previews always/when-unlocked/never;
  approvals can be set "app only"; interactive approve-from-notification
  offered only for low-risk categories.

### Data at rest & logging hygiene

- Transcript cache: `NSFileProtectionCompleteUntilFirstUserAuthentication`;
  optional "Enhanced protection" upgrades to `Complete` (documented cost:
  no locked-screen refresh).
- Never cached on device: secrets-store values, `credential_prompt`
  contents, the pairing secret (freed after pairing).
- Widget snapshots carry at most: thread title, truncated preview
  (respecting the preview setting), approval title + risk badge.
- OSLog: tokens/URLs logged with `privacy: .private`; no body logging.
- Gateway: `tcd_` registered in the `LeakDetector` scrub patterns; device
  tokens rejected on the `?token=` query path; optional app-switcher
  redaction overlay on the client.

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
5. 🚧 **Partially landed.** `DeviceRegistry::revoke` persists the revocation
   and broadcasts the revoked `device_id` on a `tokio::broadcast` channel
   (`subscribe_revocations`) built for live SSE/WS handlers to subscribe to;
   no SSE/WS handler subscribes yet, so live connections are not yet torn
   down synchronously on revoke. APNs registration/companion-token deletion
   on revoke is not yet implemented.
6. 📋 **Not yet wired.** `DeviceRegistry::sweep_inactive` implements the
   90-day-inactivity selection logic (unit-tested), but nothing schedules or
   calls it yet — there is no running daily auto-revoke sweep.

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
