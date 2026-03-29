# Slack Channel

> Chat with the agent via Slack using Socket Mode (WebSocket).

## Overview

The Slack channel uses Socket Mode to connect via WebSocket — no public URL needed.
The bot receives events (messages, app mentions) and responds via the Slack Web API.

## Prerequisites

1. **Create a Slack App** at [api.slack.com/apps](https://api.slack.com/apps)
2. **Enable Socket Mode**: Settings → Socket Mode → Enable
3. **Generate an App-Level Token**: Basic Information → App-Level Tokens → Generate with `connections:write` scope
4. **Add Bot Token Scopes**: OAuth & Permissions → Bot Token Scopes:
   - `chat:write` — Send messages
   - `app_mentions:read` — Respond to @mentions
   - `channels:history` — Read channel messages
   - `im:history` — Read DM messages
   - `im:read` — Access DM channels
5. **Subscribe to Events**: Event Subscriptions → Subscribe to bot events:
   - `message.im` — DM messages
   - `app_mention` — @mentions in channels
6. **Install the app** to your workspace

## Configuration

```bash
# Required: Bot User OAuth Token (xoxb-...)
SLACK_BOT_TOKEN=xoxb-your-slack-bot-token-here

# Required: App-Level Token (xapp-...) for Socket Mode
SLACK_APP_TOKEN=xapp-your-slack-app-token-here

# Optional: restrict to specific channel/DM IDs (empty = allow all)
SLACK_ALLOW_FROM=C1234567890,D0987654321
```

## Features

- Socket Mode (WebSocket) — no public URL required
- Responds to DMs and @mentions
- Thread-aware replies (replies in the same thread)
- Auto-reconnect with exponential backoff
- Bot's own messages are ignored
- Message splitting for responses >4000 characters
- Rich formatting (Markdown auto-converted to Slack mrkdwn: bold, links, headings)
- Agent progress messages forwarded to Slack
- Health check via `auth.test`

## How It Works

```
Slack ←→ Socket Mode (WSS) ←→ ThinClaw
         ↑                        ↓
    Events API             Web API (chat.postMessage)
```

1. ThinClaw calls `apps.connections.open` with the App Token to get a WSS URL
2. Connects to the WebSocket
3. Receives event envelopes, ACKs within 3 seconds
4. Processes `message` and `app_mention` events
5. Responds via `chat.postMessage` in the same thread

## Notes

- The bot must be invited to channels: `/invite @YourBot` in Slack
- `SLACK_BOT_TOKEN` must be valid (verified via `auth.test` at startup)
- Slack has no typing indicator API for bots — progress messages are sent as regular messages with emoji prefixes (⚠️, ❓, 📋, 💬)
