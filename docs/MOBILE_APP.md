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
  superseding the shared-secret webhook for first-party devices.
- **Payloads are content-free** (category + ids only); a Notification Service
  Extension fetches real content from the gateway and rewrites locally, so
  Apple's servers never see message content. Live Activity payloads carry a
  status enum + progress only.
- Event mapping: responses → collapsible alerts; `approval_needed` →
  time-sensitive actionable alert; job results → alerts; run status →
  throttled Live Activity updates; background wake pushes under a per-device
  budget.

## Apple workspace shape

See [`apps/ios/README.md`](../apps/ios/README.md) for the authoritative
layout, toolchain setup (mise + Tuist), and testing guide. Summary:

- Four targets (app, widgets, watch app, watch widgets); generated
  `.xcodeproj` is never committed.
- All real code lives in local SPM packages: `ThinClawAPI` (generated
  client), `ThinClawTransport` (SSE parser/stream), `ThinClawCore` (domain +
  reducers), `ThinClawPersistence`, `ThinClawAuth` (Keychain/pairing/
  Bonjour), `ThinClawSnapshotKit` (App Group snapshots for widgets/watch),
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
| M4 | watchOS | Relay + approvals + dictation + complication |
| M5 | Polish + TestFlight | Device management UI, accessibility, archive pipeline |

## Implementation status

| Piece | Status |
|---|---|
| OpenAPI baseline (`openapi` feature, export-openapi, committed spec, CI check) | ✅ landed (R0) |
| `apps/ios/` scaffold (Tuist workspace, packages, SSE parser + tests, CI) | ✅ landed (R0); Tuist manifests authored, first `tuist generate` verification pending |
| Generated Swift client from the committed spec | 📋 lands with M1 (`apps/ios/scripts/generate-api.sh`) |
| Device identity layer (pairing, tokens, scopes, TLS listener) | ✅ landed (B1) |
| `GET /api/chat/approvals` pull endpoint | ✅ landed (B1) |
| First-party push + Live Activity emitter | 📋 planned (B2) |
| mDNS advertisement | 📋 planned (B3) |
| iOS app feature milestones | 📋 planned (M1–M5) |

## Doc obligations (same-PR rule)

When implementing later milestones, update in the same PR: B1 →
`src/NETWORK_SECURITY.md` (paired-device trust boundary, listener inventory)
and `docs/CRATE_OWNERSHIP.md`; B2 → `docs/CHANNEL_ARCHITECTURE.md`; B3 →
`docs/deploy/remote-access.md` and `docs/BUILD_PROFILES.md`; M5 →
`apps/desktop/documentation/runtime-boundaries.md`; every milestone →
`FEATURE_PARITY.md` §11 and this doc's status matrix.
