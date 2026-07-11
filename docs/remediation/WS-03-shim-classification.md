# WS-03 — Shim Channel Classification (Unit C, read-only audit)

> **Scope:** Read-only classification of the 13 thin-shim channels in `channels-src/`.
> No channel source was modified. This is the input artifact for WS-03 task **T5**
> (capabilities `production_status` + per-shim README) and the WS-12 doc-sync summary table.
> Verified against the working tree on 2026-06-23 (`main` @ `9e707985`).
>
> **Post-execution status (T5 landed):** every recommendation below has been applied. All 12
> `include!` shims now carry a `README.md`, and all 16 channel `*.capabilities.json` carry a
> `production_status` field. A sixth `WebhookSecretValidation::DiscordEd25519` variant was added, so
> `discord` is now production (not "after T2"). The "No README" entries in the tables below are the
> audit-time snapshot and no longer hold. The one cross-cutting gap that remains genuinely open is
> the CI-compile gate (the 12 shims are still not built in CI; `scripts/build-all.sh` still skips the
> `tools-src/*` crates).

## Method

Each candidate was assessed on four dimensions plus a single biggest-gap call:

- **Lines of real logic** — code unique to the channel. For the 12 `include!`-based shims
  this is **0** (each `src/lib.rs` is the identical 8-line wrapper: `wit_bindgen::generate!`,
  `include!("../../shared_webhook_channel/src/impl.rs")`, `export!(GenericWebhookChannel)`).
  Their behavior is entirely declared in `*.capabilities.json` `config`. The shared engine
  (`shared_webhook_channel/src/impl.rs`) is **731 LoC** and is the real implementation all
  12 shims run.
- **Capabilities manifest** — presence of `*.capabilities.json` with a `config` block, a
  `channel.webhook` block, `mapping.text`, a `response.url`, and `setup.required_secrets`.
- **Tests** — native unit tests in the crate. Verified: **only** `shared_webhook_channel`
  carries tests (4, in `impl.rs`), and those test the shared engine (`render_body`,
  `xml_body_to_json`, `event_payloads`), not any individual shim's config wiring. **No
  individual shim has tests.** The four custom-WASM channels (telegram/slack/discord/whatsapp)
  are out of this unit's classification (they are not shims).
- **Biggest gap** — the single highest-value missing piece.

**Host validation grounding.** `WebhookSecretValidation`
(`crates/thinclaw-channels/src/wasm/schema.rs`) and the router match
(`crates/thinclaw-channels/src/wasm/router.rs`) support **six** variants:
`Equals`, `HmacSha256Body`, `HmacSha256Base64Body`, `TwitchEventsubHmacSha256`,
`TwilioRequestSignature`, and `DiscordEd25519` (added by WS-03 T2, `schema.rs:556`, handled in the
router at `router.rs:700`). A shim is signature-grade only if its `secret_validation` maps to
one of the five cryptographic variants; `equals` is a plaintext shared-secret compare and is
only meaningful if the operator configures the platform to send that exact secret in the
configured header (most of these platforms do not).

## Summary Table

| Channel | Class | Real logic (own) | Manifest | Tests | `secret_validation` | Signature-grade auth? | Biggest gap |
|---|---|---|---|---|---|---|---|
| `shared_webhook_channel` | COMPLETE (engine) | 731 LoC | n/a (engine, no manifest) | Yes (4, engine-level) | n/a | n/a | Not a deployable channel itself; no per-shim wiring tests |
| `line` | COMPLETE | 0 (shared) | Yes (full) | No | `hmac_sha256_base64_body` | **Yes** | No README; no shim-config test |
| `twitch` | COMPLETE | 0 (shared) | Yes (full + challenge) | No | `twitch_eventsub_hmac_sha256` | **Yes** | No README; no shim-config test |
| `twilio_sms` | COMPLETE | 0 (shared) | Yes (full) | No | `twilio_request_signature` | **Yes** | No README; no shim-config test |
| `dingtalk` | PARTIAL | 0 (shared) | Yes (full) | No | `equals` | No | Real platform is HMAC-SHA256 over timestamp; `equals`/`X-Webhook-Secret` cannot validate native DingTalk requests |
| `feishu_lark` | PARTIAL | 0 (shared) | Yes (full + challenge) | No | `equals` | No | Real platform uses AES/verification-token + signature; `equals` against `verification_token` only |
| `wecom` | PARTIAL | 0 (shared) | Yes (full + GET challenge) | No | `equals` | No | Real platform uses `msg_signature` (SHA1 over token/timestamp/nonce); challenge echo is unsigned; `equals` cannot verify |
| `weixin` | PARTIAL | 0 (shared) | Yes (full + GET challenge) | No | `equals` | No | Real platform uses `signature` (SHA1 over token/timestamp/nonce); challenge echo is unsigned; `equals` cannot verify |
| `qq` | PARTIAL | 0 (shared) | Yes (full) | No | `equals` | No | Real platform uses Ed25519 over body; `equals` cannot validate. **Cheapest to fix** (can reuse the T2 Discord Ed25519 helper) |
| `google_chat` | PARTIAL | 0 (shared) | Yes (full) | No | `equals` | No | Real platform sends a Google-signed Bearer JWT (`Authorization`), not a shared `X-Webhook-Secret`; `equals` cannot verify |
| `ms_teams` | PARTIAL | 0 (shared) | Yes (full) | No | `equals` | No | Real Bot Framework sends a signed JWT bearer; `equals`/`X-Webhook-Secret` cannot verify |
| `matrix` | PARTIAL | 0 (shared) | Yes (full) | No | `equals` | No | Matrix has no standard inbound webhook signature; `equals` against a ThinClaw route secret is the realistic ceiling — but it must be documented as a proxy secret, not platform auth |
| `mattermost` | PARTIAL | 0 (shared) | Yes (full) | No | `equals` | No | Mattermost outgoing-webhook uses a per-webhook token (often in body `token`, not `X-Webhook-Secret`); `equals` works only if the operator routes the token to the configured header |

No channel in this set classifies as **STUB**: every one has a complete, functional
capabilities `config` driving the shared engine, so all can receive and respond given a
correctly configured platform. The distinction is **auth correctness**, not missing code.

## Per-Channel Notes

### `shared_webhook_channel` (the engine — not a deployable channel)
- The actual 731-LoC implementation (`shared_webhook_channel/src/impl.rs`) that the 12 shims
  `include!`. Implements `on_start`/`on_http_request`/`on_respond`/`on_status`, JSON/XML/form
  body parsing, JSONPath extraction, challenge handling, and template-driven response delivery.
- Has the only tests in the shim set (4, engine-level). It has **no `Cargo.toml` and no
  manifest** of its own — it is source-included, never built or deployed standalone.
- **Gap:** there is no test that loads a real shim's `*.capabilities.json` `config` and
  asserts the shim's mapping/response wiring round-trips. A `mapping.text` path typo in any
  shim ships silently (the shims are also not compiled in CI — `ci.yml:725-751` covers only
  telegram/slack/discord/whatsapp).

### `line` — COMPLETE / production
- `secret_validation: hmac_sha256_base64_body` over `X-Line-Signature` keyed on
  `line_channel_secret` — matches LINE's real spec (base64 HMAC-SHA256 of the raw body).
- `events_path: "events"` correctly fan-outs LINE's batched webhook. `reply_token` is mapped
  into metadata and the response uses the reply endpoint. This is a faithful LINE integration.
- **Gap:** no README; no test pinning the `config` mapping.

### `twitch` — COMPLETE / production
- `secret_validation: twitch_eventsub_hmac_sha256` over `Twitch-Eventsub-Message-Signature`
  — matches Twitch EventSub's HMAC over `message-id + timestamp + body`. Includes the
  `webhook_callback_verification_pending` challenge handler.
- **Gap:** no README; no test.

### `twilio_sms` — COMPLETE / production
- `secret_validation: twilio_request_signature` over `X-Twilio-Signature` keyed on
  `twilio_auth_token` — matches Twilio's URL-plus-sorted-params signature. `body_format: form`
  and the basic-auth response URL are correct for Twilio.
- **Gap:** no README; no test. (Minor: the response URL embeds credentials as userinfo
  `https://{SID}:{TOKEN}@...`; the WASM egress allowlist rejects userinfo by design —
  flag for WS-03 T2-adjacent verification, but out of this read-only unit's lane.)

### `dingtalk` — PARTIAL / beta
- Complete config (text/markdown mapping, robot-send response, access-token query-param
  credential). **Auth gap:** DingTalk outgoing robot callbacks are signed with HMAC-SHA256
  over `timestamp\nsecret` and delivered as `timestamp` + `sign` query params, **not** a
  plaintext `X-Webhook-Secret`. `equals` cannot validate a genuine DingTalk request.

### `feishu_lark` — PARTIAL / beta
- Complete config plus a `url_verification` JSON challenge. **Auth gap:** Feishu event
  callbacks use a verification token and (in encrypt mode) AES; the signature is
  `X-Lark-Signature` over timestamp+nonce+body, not `X-Webhook-Secret`. `equals` against
  `verification_token` is weaker than the platform's real signature scheme.

### `wecom` — PARTIAL / beta
- Complete config with a GET challenge (`echostr`). **Auth gap:** WeCom/Enterprise-WeChat
  uses `msg_signature` (SHA1 over token + timestamp + nonce + encrypt) and AES message
  encryption; the GET challenge `echostr` is itself signature-gated on the real platform.
  The shim's GET challenge echoes `echostr` **without verifying `msg_signature`**, and
  inbound auth is `equals` only.

### `weixin` — PARTIAL / beta
- Same shape as `wecom` (GET `echostr` challenge, `equals`). **Auth gap:** WeChat Official
  Account uses `signature` = SHA1(sort(token, timestamp, nonce)); the shim echoes `echostr`
  without verifying it and uses `equals` for inbound messages.

### `qq` — PARTIAL / beta (cheapest to upgrade)
- Complete config. **Auth gap:** QQ bot webhooks use **Ed25519** signatures over the body.
  `equals` cannot validate. This is the one `equals` shim that could be promoted to
  production cheaply by reusing WS-03 T2's `verify_discord_ed25519_signature` helper
  generalized to an `Ed25519TimestampBody`/`Ed25519Body` variant (flagged as the T5 stretch).

### `google_chat` — PARTIAL / beta
- Complete config (space/thread mapping, bearer response). **Auth gap:** Google Chat app
  events arrive with a Google-signed Bearer JWT in `Authorization` that the receiver must
  verify against Google's public certs; the shim instead expects a shared `X-Webhook-Secret`
  with `equals`. No platform-native verification.

### `ms_teams` — PARTIAL / beta
- Complete config (Bot Framework activity mapping, conversation-activities response).
  **Auth gap:** Teams/Bot-Framework sends a signed JWT bearer that must be validated against
  the Bot Connector's OpenID metadata; `equals`/`X-Webhook-Secret` cannot perform this.

### `matrix` — PARTIAL / beta
- Complete config (room/event mapping, `m.room.message` PUT response). **Auth nuance:**
  Matrix has no single standard inbound webhook signature; the `matrix_webhook_secret` is a
  ThinClaw **route/proxy secret** an operator places in front. `equals` is a reasonable
  ceiling here, but the README/capabilities must say "route secret," not "platform auth," to
  stay honest.

### `mattermost` — PARTIAL / beta
- Complete config (post mapping, `/api/v4/posts` response). **Auth gap:** Mattermost
  outgoing webhooks carry a per-webhook `token` (commonly in the request body/form field
  `token`), not a header. `equals` against `X-Webhook-Secret` only works if the operator
  manually routes the token into that header. Document the constraint.

## Recommendation Per Channel

| Channel | Recommendation |
|---|---|
| `shared_webhook_channel` | **LEAVE** as the shared engine. **FINISH (test):** add a config-load round-trip test (load each shim's `config`, assert mapping/response parse) — owned by WS-03 T4/T5 + the WS-13 CI-compile gate, not this read-only unit. |
| `line` | **FINISH (docs only):** mark `production_status: production` and add a README. Code/auth already production-grade. |
| `twitch` | **FINISH (docs only):** mark `production_status: production` + README. |
| `twilio_sms` | **FINISH (docs only):** mark `production_status: production` + README. (Verify the userinfo-in-URL egress separately.) |
| `qq` | **FINISH (code, stretch):** add an Ed25519 host variant reusing the T2 Discord helper, then mark `production`. Until then: **MARK NON-PRODUCTION** (`beta`, "inbound auth = shared-secret `equals` only; platform uses Ed25519"). |
| `dingtalk` | **MARK NON-PRODUCTION** (`beta`) + README caveat: "inbound auth is shared-secret `equals` only; DingTalk-native HMAC-SHA256 timestamp signature is not verified." |
| `feishu_lark` | **MARK NON-PRODUCTION** (`beta`) + caveat: verification-token `equals` only; no Lark signature/AES verification. |
| `wecom` | **MARK NON-PRODUCTION** (`beta`) + caveat: `equals` only; `msg_signature`/AES and challenge-signature verification not implemented. |
| `weixin` | **MARK NON-PRODUCTION** (`beta`) + caveat: `equals` only; WeChat `signature` SHA1 verification not implemented. |
| `google_chat` | **MARK NON-PRODUCTION** (`beta`) + caveat: `equals` only; Google Bearer-JWT verification not implemented. |
| `ms_teams` | **MARK NON-PRODUCTION** (`beta`) + caveat: `equals` only; Bot Framework JWT verification not implemented. |
| `matrix` | **MARK NON-PRODUCTION** (`beta`) + README clarifying `matrix_webhook_secret` is a ThinClaw route/proxy secret (`equals`), not Matrix platform auth. |
| `mattermost` | **MARK NON-PRODUCTION** (`beta`) + caveat: per-webhook token via `equals` only if operator routes it into `X-Webhook-Secret`; native body-`token` placement not auto-handled. |

### Rollup for T5 / WS-12 table
- **Production (4):** `line`, `twitch`, `twilio_sms`, and `discord` (WS-03 T2 landed; `discord` now
  verifies Ed25519 host-side via `WebhookSecretValidation::DiscordEd25519`).
- **Beta / "inbound auth = shared-secret `equals` only" (9):** `dingtalk`, `feishu_lark`,
  `google_chat`, `matrix`, `mattermost`, `ms_teams`, `qq`, `wecom`, `weixin`.
- **Stretch promotion:** `qq` to production via reused Ed25519 helper (not yet taken; still `beta`).
- **Cross-cutting status:** READMEs **landed**: all 12 shims now carry a `README.md` and all channel
  `*.capabilities.json` carry `production_status` (T5). Still open (WS-13-owned): the config-wiring
  round-trip test and the CI-compile gate. `scripts/build-all.sh` still skips the `tools-src/*`
  crates and the channel matrix compiles only the four custom-WASM channels.
