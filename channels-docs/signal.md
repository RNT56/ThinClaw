# Signal Channel

> End-to-end encrypted messaging via Signal, using signal-cli daemon.

## Overview

The Signal channel connects to a running `signal-cli daemon` via its HTTP API.
Messages are received via SSE (Server-Sent Events) and sent via JSON-RPC.
Supports both DMs and group messages with fine-grained access control.

## Prerequisites

1. **Install signal-cli**: [github.com/AsamK/signal-cli](https://github.com/AsamK/signal-cli)
2. **Register or link a phone number**:
   ```bash
   signal-cli -u +1234567890 register
   signal-cli -u +1234567890 verify 123456
   ```
3. **Start the daemon**:
   ```bash
   signal-cli -u +1234567890 daemon --http 127.0.0.1:8080
   ```

## Configuration

```bash
# Required
SIGNAL_HTTP_URL=http://127.0.0.1:8080
SIGNAL_ACCOUNT=+1234567890

# DM access control (default: account number only)
SIGNAL_ALLOW_FROM=+1234567890,+0987654321

# DM policy: "open", "allowlist", or "pairing" (default: "pairing")
SIGNAL_DM_POLICY=pairing

# Group access control (default: empty = deny all groups)
SIGNAL_ALLOW_FROM_GROUPS=*
# or specific group IDs: SIGNAL_ALLOW_FROM_GROUPS=abc123,def456

# Group policy: "disabled", "allowlist", or "open" (default: "allowlist")
SIGNAL_GROUP_POLICY=allowlist

# Group sender allow list (empty = inherits from SIGNAL_ALLOW_FROM)
SIGNAL_GROUP_ALLOW_FROM=+1234567890

# Skip attachment-only and story messages
SIGNAL_IGNORE_ATTACHMENTS=false
SIGNAL_IGNORE_STORIES=true
```

## DM Policies

| Policy | Behavior |
|--------|----------|
| `open` | Accept DMs from everyone |
| `allowlist` | Only accept from `SIGNAL_ALLOW_FROM` list |
| `pairing` | Allowlist + send pairing code to unknown senders. Approve via `thinclaw pairing approve signal <code>` |

## Group Policies

| Policy | Behavior |
|--------|----------|
| `disabled` | Reject all group messages |
| `allowlist` | Check both group ID (`SIGNAL_ALLOW_FROM_GROUPS`) and sender |
| `open` | Accept from allowed groups, don't filter by sender |

## Features

- End-to-end encryption (via Signal protocol)
- DM and group message support
- Generated media replies are written to temporary files and sent through
  signal-cli JSON-RPC `attachments`, then cleaned up after dispatch
- Pairing system for unknown senders
- Blocklist support (`thinclaw pairing block signal <sender>`)
- Typing indicators (via JSON-RPC)
- Debug mode toggle (`/debug`)
- Deterministic thread IDs for conversation persistence
- Auto-reconnect on SSE disconnect
- URL credential redaction in logs

## Security Notes

- Empty `SIGNAL_ALLOW_FROM` = **deny all** (secure by default)
- Blocklist takes precedence over allowlist
- Credentials in `SIGNAL_HTTP_URL` are redacted in logs
- UUID-based senders supported (privacy-mode Signal users)
