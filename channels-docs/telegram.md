# Telegram Channel

> Chat with the agent via Telegram Bot API.

## Overview

The Telegram channel connects to the Telegram Bot API using long polling.
Create a bot via @BotFather, set the token, and the agent accepts messages from Telegram.

## Prerequisites

1. **Create a Telegram bot**: Message [@BotFather](https://t.me/BotFather) on Telegram
2. **Copy the bot token** (format: `123456789:ABCDEfghijklMNOpqrstUVWxyz`)

## Configuration

```bash
# Required
TELEGRAM_BOT_TOKEN=123456789:ABCDEfghijklMNOpqrstUVWxyz

# Optional: restrict to a single owner (numeric user ID)
TELEGRAM_OWNER_ID=123456789

# Optional: restrict to specific user IDs (comma-separated)
TELEGRAM_ALLOW_FROM=123456789,987654321

# Optional: stream mode for progressive responses
# Options: "full" (default), "edit", "typing"
TELEGRAM_STREAM_MODE=full
```

### Finding your Telegram User ID

1. Message [@userinfobot](https://t.me/userinfobot) on Telegram
2. It replies with your numeric user ID

## Features

- Long polling (no webhook server needed)
- Progressive streaming responses (edit-in-place)
- Owner-only mode via `TELEGRAM_OWNER_ID`
- Allow-list for multi-user access
- Rate-limited message editing for streaming
- Rich HTML formatting (Markdown auto-converted to Telegram HTML: bold, italic, code, links, headings)
- Auto-reconnect on disconnect

## Stream Modes

| Mode | Behavior |
|------|----------|
| `full` | Send complete response when done |
| `edit` | Edit message in-place as chunks arrive |
| `typing` | Send typing indicator during processing |

## Notes

- If `TELEGRAM_OWNER_ID` is set, it takes precedence (only that user can interact)
- If both `TELEGRAM_OWNER_ID` and `TELEGRAM_ALLOW_FROM` are empty, all Telegram users can message the bot
- The bot must be started by the user first (`/start` in the bot chat)
