# Linq managed messaging channel

ThinClaw's native `linq` channel connects to the Linq Partner API v3 for
managed, headless iMessage, RCS, and SMS. It is distinct from the macOS-only
native iMessage channel and the self-hosted BlueBubbles bridge. Enabling it
does not require Apple code signing.

## Architecture decision

Linq is retained as an optional third deployment mode because it covers a real
gap rather than replacing either existing option:

| Mode | Operator requirement | Best fit |
| --- | --- | --- |
| Native iMessage | ThinClaw runs on a signed-in Mac with local `chat.db` access | Local, private, Apple-native deployment |
| BlueBubbles | Operator owns and maintains a Mac-hosted bridge | Self-hosted cross-platform deployment |
| Linq | Commercial Partner API account and managed number | Headless cloud/server deployment with no operator Mac |

The tradeoff is deliberate: Linq introduces a commercial third-party data
processor, so it remains disabled by default and cannot silently replace or
fall back from either self-hosted integration. The adapter is native because it
owns durable webhook deduplication, bounded media transfer, retry/idempotency,
and a long-running inbound stream; those lifecycle obligations are broader than
the current stateless packaged-channel contract.

## Security defaults

- Inbound requests require a valid Standard Webhooks HMAC-SHA256 signature.
- The webhook payload is pinned to version `2026-02-03`.
- `LINQ_ALLOW_FROM` is empty by default, which denies every inbound sender.
- Outbound delivery defaults to explicit `iMessage`; Linq may not silently
  fall back to RCS/SMS unless the operator selects `auto`.
- API keys and webhook signing secrets are read from the encrypted ThinClaw
  secret store. Environment variables are an explicit deployment fallback.
- Event IDs are recorded in an owner-private, crash-safe, bounded ledger for
  at-least-once delivery deduplication.
- Inbound media is accepted only from HTTPS URLs on `cdn.linqapp.com`.
  Linq's partner-specific presigned upload hosts are accepted only after
  public-address DNS validation and connection pinning. Both flows bypass
  proxies and enforce per-file, aggregate, count, MIME, redirect, and body-size
  limits.

## Configure

Store credentials without placing them in settings or shell history:

```bash
printf '%s' "$LINQ_API_KEY_VALUE" \
  | thinclaw secrets set linq_api_key --from-stdin --provider linq
printf '%s' "$LINQ_WEBHOOK_SECRET_VALUE" \
  | thinclaw secrets set linq_webhook_secret --from-stdin --provider linq
```

Then configure non-secret values:

```env
LINQ_ENABLED=true
LINQ_FROM_NUMBER=+12025550100
LINQ_ALLOW_FROM=+12025550101,user@example.com
LINQ_PREFERRED_SERVICE=imessage
LINQ_WEBHOOK_HOST=127.0.0.1 # numeric bind IP, not a DNS name
LINQ_WEBHOOK_PORT=8080
```

`LINQ_ALLOW_FROM=*` is an explicit allow-all setting. `LINQ_PREFERRED_SERVICE`
accepts `imessage`, `auto`, `rcs`, or `sms`. Only `auto` permits service
selection/fallback by Linq.

The environment fallbacks `LINQ_API_KEY` and `LINQ_WEBHOOK_SECRET` are useful
for container secret injection. A signing secret must use Linq's `whsec_`
format. The credential destination is fixed to Linq's official v3 API origin;
it is intentionally not configurable at runtime.

## Webhook subscription

Expose the gateway through the configured HTTPS tunnel, then create a Linq
subscription for `message.received` using this exact target path and version:

```text
https://your-agent.example/webhook/linq?version=2026-02-03
```

Linq returns the webhook signing secret only when the subscription is created;
store it immediately as `linq_webhook_secret`. The route is mounted on the
gateway (including its tunnel) and on the shared local webhook listener. If the
HTTP channel is also enabled, `LINQ_WEBHOOK_HOST`/`PORT` must match its listener
address so both route sets can share one socket.

## Operational behavior

Every outbound API request has a bounded retry budget. Retries reuse the same
message idempotency key. Replies preserve the Linq chat and source message IDs;
broadcasts create or reuse the Linq chat for the configured sending number.
Attachments use Linq's pre-upload flow and include all presigned required
headers while rejecting credential-bearing response headers.

Inbound actor endpoints use the normalized deliverable phone/email handle so
preferred-endpoint notifications can address the same person later. Linq's
opaque handle UUID remains provenance metadata, while direct/group threads use
the stable Linq chat UUID.

The channel fails startup when enabled but credentials, sender number, webhook
address, or protocol policy are invalid. Health checks query the v3
`phone_numbers` endpoint, and diagnostics expose only redacted origin and count
metadata—never keys, signing secrets, phone numbers, or allowlist entries.

## Credential-gated live smoke

The deterministic test suite covers signatures, replay/deduplication, canonical
identity/thread mapping, sender denial, retry idempotency, and explicit service
selection without contacting Linq. Before promoting a deployment, run this
credential-gated smoke against a dedicated test number:

1. Start the gateway and confirm `linq` is `running` in
   `thinclaw extensions channels status`.
2. Create a `message.received` subscription targeting the exact versioned URL
   above and store its returned signing secret.
3. Send a text and one supported attachment from an allowlisted test handle.
   Confirm one ThinClaw turn appears with the same sender endpoint and Linq chat
   across webhook redelivery.
4. Reply from ThinClaw and confirm Linq reports the expected service
   (`iMessage` by default), thread/reply target, and a single outbound message
   even when a transient API response is retried.
5. Repeat from an unlisted handle and with a tampered signature; neither may
   create a ThinClaw turn. Confirm no credential or full phone number appears
   in logs or diagnostics.
6. Delete the test attachment through Linq's attachment API if the account does
   not use the ephemeral attachment tier.

Do not run this smoke in general CI: it sends real messages and requires
provider credentials, a public callback, and a billable managed number.

Official references: [Linq quickstart](https://docs.linqapp.com/getting-started/quickstart/),
[sending messages](https://docs.linqapp.com/guides/messaging/sending-messages/),
[attachments](https://docs.linqapp.com/api/resources/attachments/methods/create/), and
[webhook verification/events](https://docs.linqapp.com/api/resources/webhook_events/).
