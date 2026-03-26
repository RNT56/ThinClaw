# Discord Channel

> Chat with the agent via Discord Gateway WebSocket.

## Overview

The Discord channel connects to the Discord Gateway API using a WebSocket.
The bot receives messages from channels/DMs and responds via the Discord REST API.

## Prerequisites

1. **Create a Discord Application** at [discord.com/developers/applications](https://discord.com/developers/applications)
2. **Add a Bot**: Bot → Add Bot
3. **Copy the Bot Token**
4. **Enable Intents**: Bot → Privileged Gateway Intents:
   - ✅ Message Content Intent
   - ✅ Server Members Intent (optional, for user info)
5. **Invite the bot** to your server with appropriate permissions:
   - Send Messages
   - Read Messages/View Channels
   - Read Message History

## Configuration

```bash
# Required
DISCORD_BOT_TOKEN=your-discord-bot-token-here

# Optional: restrict to a specific server (guild)
DISCORD_GUILD_ID=1234567890123456789

# Optional: restrict to specific channel IDs (empty = allow all)
DISCORD_ALLOW_FROM=1234567890,9876543210

# Optional: stream mode for progressive responses
# Options: "full" (default), "edit", "typing"
DISCORD_STREAM_MODE=full
```

## Features

- Discord Gateway WebSocket connection
- DM and server channel support
- Optional guild restriction
- Progressive streaming responses (edit-in-place)
- Channel allow-list
- Auto-reconnect on disconnect

## Stream Modes

| Mode | Behavior |
|------|----------|
| `full` | Send complete response when done |
| `edit` | Edit message in-place as chunks arrive |
| `typing` | Send typing indicator during processing |

## Notes

- Message Content Intent must be enabled for the bot to read message content
- If `DISCORD_ALLOW_FROM` is empty, the bot responds in all channels it can see
- The bot ignores its own messages
