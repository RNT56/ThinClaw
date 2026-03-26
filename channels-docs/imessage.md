# iMessage Channel

> Chat with the agent via iMessage on macOS.

## Overview

The iMessage channel reads incoming messages from Apple's `chat.db` SQLite database
and sends replies via AppleScript controlling Messages.app. macOS only.

## Prerequisites

| Requirement | Why |
|-------------|-----|
| **macOS** | Uses Messages.app and AppleScript |
| **Apple ID signed in** | Messages.app must be configured with iMessage |
| **Full Disk Access** | Process needs access to `~/Library/Messages/chat.db` |
| **Messages.app running** | Required for sending (auto-started if not running) |

## Configuration

```bash
# Enable the channel
IMESSAGE_ENABLED=true

# Restrict to specific phone numbers / email addresses (optional, empty = all)
IMESSAGE_ALLOW_FROM=+1234567890,user@icloud.com

# Polling interval in seconds (default: 3)
IMESSAGE_POLL_INTERVAL=3
```

## Auto-Start

If Messages.app isn't running when ThinClaw starts, it's automatically launched
via `osascript -e 'tell application "Messages" to launch'`.

## How It Works

- **Inbound**: Polls `~/Library/Messages/chat.db` every 3 seconds for new messages
- **Outbound**: Sends replies via AppleScript → Messages.app → iMessage servers

### Database Query

Joins `message`, `handle`, and `chat` tables:
- Extracts text, sender, chat identifier, attachment count, group detection
- Skips outgoing messages (`is_from_me = 1`)
- Deduplicates via ROWID tracking (HashSet, capped at 500)

## Features

- DM and group chat support
- Phone number and email sender detection
- Attachment count metadata
- Group chat detection (via `display_name` presence)
- Long message splitting (20,000 char chunks, prefers line boundaries)
- Text escaping for AppleScript safety
- Preflight diagnostics (`IMessageDiagnostic`)

## Diagnostics

Call `IMessageChannel::diagnose()` to check:
- `db_exists` — Whether chat.db exists
- `sqlite3_available` — Whether sqlite3 CLI is installed
- `osascript_available` — Whether AppleScript works
- `messages_running` — Whether Messages.app is running
- `total_messages` — Total count in chat.db

## Security Notes

- **Empty `IMESSAGE_ALLOW_FROM` = accept ALL messages.** Set this in production!
- Sender filtering supports phone numbers (`+1234567890`) and emails (`user@icloud.com`)
- The agent only reads chat.db — it cannot modify it
- Sending is via AppleScript (sandboxed by macOS)
