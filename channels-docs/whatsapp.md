# WhatsApp Channel Package

ThinClaw ships WhatsApp as a packaged WASM channel that talks to the Meta
WhatsApp Cloud API over webhook callbacks plus Graph API responses.

## What You Need

- A public HTTPS ThinClaw base URL that Meta can reach.
- A Meta app with the WhatsApp product enabled.
- A phone number connected to the WhatsApp Cloud API.
- These ThinClaw secrets:
  - `whatsapp_access_token`
  - `whatsapp_verify_token`
  - `whatsapp_app_secret`

## Secret Roles

- `whatsapp_access_token`
  - Used for Graph API calls to send messages, upload media, and download media.
- `whatsapp_verify_token`
  - Used only for Meta's initial `GET` webhook verification handshake.
- `whatsapp_app_secret`
  - Used only for `POST` webhook signature validation via `X-Hub-Signature-256`.

## Callback URL

Use this callback URL in the Meta app:

```text
https://<your-thinclaw-host>/webhook/whatsapp
```

ThinClaw expects:

- `GET /webhook/whatsapp`
  - Meta verification handshake.
  - Query param: `hub.verify_token`
  - ThinClaw replies with plain-text `hub.challenge`.
- `POST /webhook/whatsapp`
  - Signed inbound webhooks.
  - Header: `X-Hub-Signature-256`
  - Validation: HMAC-SHA256 over the raw request body using `whatsapp_app_secret`.

## Meta App Setup

1. Create or open a Meta app at [developers.facebook.com/apps](https://developers.facebook.com/apps/).
2. Add the WhatsApp product.
3. Open the webhook configuration for the WhatsApp product.
4. Set the callback URL to `https://<your-thinclaw-host>/webhook/whatsapp`.
5. Set the verify token to the exact ThinClaw `whatsapp_verify_token` value.
6. Subscribe at minimum to message webhooks.
7. Add the app secret to ThinClaw as `whatsapp_app_secret`.
8. Add the long-lived or system-user access token to ThinClaw as `whatsapp_access_token`.

## Expected Runtime Behavior

- Inbound support:
  - text
  - image/audio/video/document/sticker
  - location
  - contacts
  - interactive replies
  - reactions
- Status-only events are logged for diagnostics and are not dispatched to the agent.
- Unknown inbound message types are surfaced as safe fallback text instead of being dropped.
- Outbound support:
  - text replies
  - outbound media upload + send for image/audio/video/document/sticker
  - generated media auto-attachments through the host `response_attachments`
    bridge
  - reply threading when `reply_to_message` is enabled

## Pairing Flow

ThinClaw treats WhatsApp as a direct-message surface.

- `dm_policy: "open"`
  - All inbound DMs are accepted immediately.
- `dm_policy: "pairing"`
  - Unknown senders trigger a pairing request.
  - ThinClaw sends a pairing code back over WhatsApp.
  - An operator approves with `thinclaw pairing approve whatsapp <code>`.
- `owner_id`
  - Restricts the channel to one WhatsApp sender phone number.

## 24-Hour Window

This channel is intentionally reply-centric.

- Free-form outbound messages work inside Meta's customer-service window.
- If a send falls outside that window, ThinClaw returns a clear "template required"
  error instead of attempting template initiation automatically.
- Template/campaign/catalog/order workflows are out of scope for this channel package.

## Dev And Live Testing

Recommended validation sequence:

1. Save the three WhatsApp secrets in ThinClaw.
2. Verify the Meta webhook callback succeeds.
3. Send a real inbound text message from an allowed sender.
4. Confirm ThinClaw emits the inbound event to the agent.
5. Confirm a plain-text reply succeeds.
6. Confirm an outbound media reply succeeds.
7. Confirm invalid verify tokens fail.
8. Confirm invalid webhook signatures fail.

For live-mode rollout, repeat the same checks after switching the Meta app and
phone number from dev/test configuration to production/live configuration.

## Operational Notes

- The channel is webhook-only; polling is not used.
- Media-heavy callbacks use an elevated callback timeout in capabilities.
- `api_version` is stored in channel workspace state and reused for reply,
  upload, and media-download calls.
- Generic proactive broadcast is not supported for WhatsApp.
  Use explicit route metadata with:
  - `phone_number_id`
  - `recipient_phone`
  - optional `reply_to_message_id`

## Related Docs

- [../docs/BUILDING_CHANNELS.md](../docs/BUILDING_CHANNELS.md)
- [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md)
- [registry/channels/whatsapp.json](../registry/channels/whatsapp.json)
