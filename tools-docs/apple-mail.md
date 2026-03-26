# Apple Mail Channel

> Read and reply to emails natively via macOS Mail.app — no API keys, no OAuth, just Apple ID.

## Overview

The Apple Mail channel monitors the local Mail.app database for incoming emails
and sends replies via AppleScript. This is a **macOS-only** channel that works
entirely locally — no cloud APIs, no tokens.

**How it works:**
- **Inbound**: Polls Mail.app's Envelope Index SQLite database (`~/Library/Mail/V10/MailData/Envelope Index`)
- **Outbound**: Sends replies via `osascript` (AppleScript) controlling Mail.app
- **Auto-start**: If Mail.app isn't running, ThinClaw launches it automatically

## Prerequisites

| Requirement | Why |
|-------------|-----|
| **macOS** | Uses Mail.app and AppleScript — macOS only |
| **Apple ID signed in** | Mail.app must be configured with an active email account (iCloud Mail, or any IMAP account) |
| **Full Disk Access** | The process running ThinClaw needs Full Disk Access to read `~/Library/Mail/` |

### Granting Full Disk Access

1. Open **System Settings → Privacy & Security → Full Disk Access**
2. Click **"+"** and add:
   - `Terminal.app` (if running interactively)
   - Or the ThinClaw binary (`/usr/local/bin/thinclaw`)
   - Or `iTerm2.app` if that's your terminal
3. Restart the terminal/process

## Configuration

### Environment Variables

```bash
# Required: Enable the channel
APPLE_MAIL_ENABLED=true

# Optional: Restrict to specific senders (comma-separated)
APPLE_MAIL_ALLOW_FROM=boss@company.com,team@company.com

# Optional: Polling interval in seconds (default: 10)
APPLE_MAIL_POLL_INTERVAL=10

# Optional: Only process unread messages (default: true)
APPLE_MAIL_UNREAD_ONLY=true

# Optional: Mark messages as read after processing (default: true)
APPLE_MAIL_MARK_AS_READ=true
```

### TOML Configuration

```toml
[channels.apple_mail]
enabled = true
allow_from = ["boss@company.com", "team@company.com"]
poll_interval_secs = 10
unread_only = true
mark_as_read = true
```

## How It Works

### Reading Emails

The channel polls Mail.app's SQLite database every N seconds (default: 10):

```
~/Library/Mail/V10/MailData/Envelope Index
```

This database contains all email metadata (subject, sender, date, read status,
message snippets). ThinClaw queries for new unread messages since the last check.

### Sending Replies

Replies are sent via AppleScript:

```applescript
tell application "Mail"
    set newMessage to make new outgoing message with properties {subject:"Re: ...", content:"...", visible:false}
    tell newMessage
        make new to recipient at end of to recipients with properties {address:"sender@example.com"}
    end tell
    send newMessage
end tell
```

### Auto-Start

When the Apple Mail channel is enabled, ThinClaw automatically:
1. Checks if Mail.app is running (`pgrep -x Mail`)
2. If not, launches it via `osascript -e 'tell application "Mail" to launch'`
3. Waits 3 seconds for initialization
4. Same behavior for Messages.app when the iMessage channel is enabled

## Security Notes

- **No cloud API**: All data stays on-device. No tokens or API keys needed.
- **Allow-list**: Always set `APPLE_MAIL_ALLOW_FROM` in production to prevent the agent from responding to spam.
- **Full Disk Access**: Required, but only for reading the mail database. The agent cannot modify the database — it only reads.
- **AppleScript**: Used for sending only. Mail.app handles all the actual SMTP transport.

## Comparison with Gmail Channel

| Feature | Apple Mail Channel | Gmail Channel |
|---------|-------------------|---------------|
| **Auth** | None (uses Apple ID already signed in) | Google OAuth 2.0 |
| **Cloud** | Fully local | GCP Pub/Sub required |
| **Real-time** | Polling (10s default) | Pub/Sub push (near-instant) |
| **Works on Linux** | ❌ macOS only | ✅ Any platform |
| **Setup complexity** | One env var | GCP project + Pub/Sub + OAuth |
