# Slack Tool

> Send messages, list channels, read history, add reactions, and look up user info.

## Authentication

Slack uses a **Bot Token** from a Slack App installed to your workspace.

### Setup Steps

1. **Create a Slack App**
   - Go to [api.slack.com/apps](https://api.slack.com/apps)
   - Click **"Create New App"** → **"From scratch"**
   - Name it (e.g. `ThinClaw Agent`) and select your workspace

2. **Configure Bot Token Scopes**
   - Navigate to **OAuth & Permissions**
   - Under **Bot Token Scopes**, add:

   | Scope | Purpose |
   |-------|---------|
   | `chat:write` | Send messages |
   | `channels:read` | List public channels |
   | `channels:history` | Read channel message history |
   | `groups:read` | Read private channels/groups the bot is in |
   | `groups:history` | Read private channel/group message history |
   | `reactions:write` | Add emoji reactions |
   | `users:read` | Look up user profiles |

3. **Install the App to your Workspace**
   - Go to **Install App** and click **"Install to Workspace"**
   - Approve the permissions
   - Copy the **Bot User OAuth Token** (starts with `xoxb-`)

4. **Authenticate the tool**
   ```bash
   thinclaw tool auth slack-tool
   ```

   Or provide `SLACK_BOT_TOKEN` through your deployment's env/secret flow if that is how you manage credentials.

5. **Invite the bot to channels**
   - In Slack, go to the channel and invite the app using its current display name, for example `/invite @Your Agent Name`
   - The bot can only read/write in channels it has been invited to

6. **Verify**
   ```
   You: List all Slack channels
   You: Send "Hello from the agent!" to #general
   ```

## Available Actions (5)

| Action | Description |
|--------|-------------|
| `send_message` | Send a message to a channel (by channel ID or name) |
| `list_channels` | List all public channels in the workspace |
| `get_channel_history` | Get recent messages from a channel |
| `post_reaction` | Add an emoji reaction to a message |
| `get_user_info` | Look up a user's profile by user ID |

## Rate Limits

- 50 requests/minute, 1000 requests/hour (enforced by capabilities)
- Slack also enforces per-method rate limits (typically 1 request/second for posting)
