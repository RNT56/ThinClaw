# Nostr Channel

> Decentralized, censorship-resistant messaging via NIP-04 encrypted DMs.

## Overview

The Nostr channel uses the Nostr protocol to receive and send encrypted direct messages
(NIP-04). The agent connects to one or more Nostr relays and listens for DMs.

## Prerequisites

1. **Generate a Nostr keypair** (or use an existing one)
   - Tools: [nostrtool](https://github.com/nickkatsios/nostrtool), or any Nostr client
   - You need the **private key** in hex or bech32 (`nsec`) format

2. **Identify relays** to connect to

## Configuration

```bash
# Required: Nostr private key (hex or nsec format)
NOSTR_PRIVATE_KEY=nsec1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx

# Optional: relay URLs (comma-separated)
# Defaults: wss://relay.damus.io, wss://nos.lol, wss://relay.nostr.band
NOSTR_RELAYS=wss://relay.damus.io,wss://nos.lol

# Optional: restrict to specific public keys (hex or npub)
# Empty = deny all, "*" = allow everyone
NOSTR_ALLOW_FROM=npub1xxxxxxxxxxxx,npub1yyyyyyyyyy
```

## Features

- NIP-04 encrypted DMs (end-to-end encryption)
- Multi-relay support with auto-reconnect
- Public key allow-list
- Decentralized operation — no central server
- Works behind firewalls (outbound-only WebSocket connections)

## Usage

To message the agent, send a NIP-04 DM to the agent's **public key** (npub)
from any Nostr client (Damus, Amethyst, Primal, etc.).

## Notes

- Empty `NOSTR_ALLOW_FROM` = **deny all** (secure by default)
- Set `NOSTR_ALLOW_FROM=*` to accept DMs from everyone
- The private key is securely handled via the `secrecy` crate (zeroized on drop)
- The agent uses `nostr-sdk` for relay connections
