# Apple Mail Channel

> Read and reply to emails natively via macOS Mail.app — no API keys, no OAuth.

## Overview

The Apple Mail channel monitors Mail.app's local SQLite database for incoming emails
and sends replies via AppleScript. macOS only, fully local, no cloud APIs.

## Prerequisites

| Requirement | Why |
|-------------|-----|
| **macOS** | Uses Mail.app and AppleScript |
| **Apple ID signed in** | Mail.app must have an active email account |
| **Full Disk Access** | Process needs access to `~/Library/Mail/` |

### Granting Full Disk Access

System Settings → Privacy & Security → Full Disk Access → Add your terminal or ThinClaw binary.

## Configuration

```bash
APPLE_MAIL_ENABLED=true

# Restrict to specific senders (optional)
APPLE_MAIL_ALLOW_FROM=boss@company.com,team@company.com

# Polling interval (default: 10 seconds)
APPLE_MAIL_POLL_INTERVAL=10

# Only process unread messages (default: true)
APPLE_MAIL_UNREAD_ONLY=true

# Mark as read after processing (default: true)
APPLE_MAIL_MARK_AS_READ=true
```

## Auto-Start

If Mail.app isn't running when ThinClaw starts, it's automatically launched
via `osascript -e 'tell application "Mail" to launch'`.

## How It Works

- **Inbound**: Polls `~/Library/Mail/V*/MailData/Envelope Index` every N seconds
- **Outbound**: Sends replies via AppleScript → Mail.app → SMTP
- **Generated media**: Adds generated files as Mail.app attachments when the
  final agent response includes media

## Comparison with Gmail Channel

| Feature | Apple Mail | Gmail |
|---------|-----------|-------|
| **Auth** | None (Apple ID in Mail.app) | Google OAuth |
| **Cloud** | Fully local | GCP Pub/Sub |
| **Real-time** | Polling (10s default) | Pub/Sub push |
| **Platform** | macOS only | Any |
| **Setup** | One env var | GCP project + OAuth |
