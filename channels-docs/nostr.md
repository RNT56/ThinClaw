# Nostr Channel

> Owner-only encrypted DM control plus separate Nostr social actions.

## Overview

ThinClaw uses one Nostr identity for two separate jobs:

- The **channel** accepts executable commands only from one explicit owner pubkey over encrypted DMs.
- The **`nostr_actions` tool** lets the agent post notes, reply, react, repost, quote, DM, inspect profiles/events, and optionally read non-owner DMs as untrusted external content.

Public mentions and non-owner DMs never enter the command pipeline.

## Configuration

```bash
# Required for any Nostr use: private key in hex or bech32 (nsec) format
NOSTR_PRIVATE_KEY=nsec1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx

# Optional legacy alias that still works
NOSTR_SECRET_KEY=nsec1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx

# Enable or disable the Nostr runtime explicitly
NOSTR_ENABLED=true

# Optional: relay URLs (comma-separated)
# Defaults: wss://relay.damus.io, wss://nos.lol, wss://relay.nostr.band
NOSTR_RELAYS=wss://relay.damus.io,wss://nos.lol

# Owner pubkey allowed to control the agent over encrypted DMs
NOSTR_OWNER_PUBKEY=npub1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx

# Optional: allow nostr_actions.get_dm_inbox to read non-owner DMs
NOSTR_SOCIAL_DM_ENABLED=false
```

## Behavior

- Inbound control accepts encrypted DMs from the configured owner only.
- The runtime supports both legacy **NIP-04** DMs and newer **Gift Wrap / NIP-17-style** encrypted DMs.
- Outbound DMs remain available through `send_message(platform="nostr")`.
- Public posting and social interactions belong to `nostr_actions`, not the channel.
- Third-party Nostr content returned by tools is marked as untrusted external content.

## Legacy Compatibility

- `NOSTR_ALLOW_FROM` is deprecated for command authorization.
- If `NOSTR_OWNER_PUBKEY` is unset and `NOSTR_ALLOW_FROM` contains exactly one concrete pubkey, ThinClaw treats that entry as the owner and logs a deprecation warning.
- Otherwise, `NOSTR_ALLOW_FROM` no longer authorizes command senders.

## Notes

- If `NOSTR_ENABLED=false`, Nostr stays disabled even if a private key exists.
- The private key is handled with `secrecy` and should be stored in the secrets store when possible.
- The agent uses `nostr-sdk` for relay connections, encrypted DM support, and event publishing.
