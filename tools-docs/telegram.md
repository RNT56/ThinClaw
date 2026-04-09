# Telegram Tool

> Full Telegram client using MTProto — login, manage chats, send/search/forward messages.

## Authentication

Telegram uses **MTProto API credentials** (not the Bot API). This gives the agent
a real Telegram user account, not a bot — it can join groups, see full chat history,
and act as a regular user.

### Setup Steps

1. **Create a Telegram API Application**
   - Go to [my.telegram.org/apps](https://my.telegram.org/apps)
   - Log in with your phone number
   - Create a new application
   - Note your **API ID** (numeric) and **API Hash** (hex string)

2. **Store the credentials**
   Store them in the tool workspace:
   - write your numeric API ID to `telegram/api_id`
   - write your API hash string to `telegram/api_hash`

3. **Authenticate via the tool**
   - Ask the agent to log in:
   ```
   You: Use the Telegram tool to login with phone number +1234567890
   ```
   - Telegram will send a code to the phone/app
   ```
   You: Submit auth code 12345
   ```
   - If 2FA is enabled:
   ```
   You: Submit 2FA password mypassword
   ```

4. **Session persistence**
   - Save the returned session JSON to `telegram/session.json`
   - Subsequent startups reuse the session without re-authentication

## Available Actions (11)

| Action | Description |
|--------|-------------|
| `login` | Start authentication with a phone number |
| `submit_auth_code` | Submit the SMS/app verification code |
| `submit_2fa_password` | Submit 2FA password if enabled |
| `get_me` | Get the authenticated user's profile |
| `get_contacts` | List all contacts |
| `get_chats` | List recent chats (groups, channels, DMs) |
| `get_messages` | Get messages from a specific chat |
| `send_message` | Send a text message to a chat |
| `forward_message` | Forward a message between chats |
| `delete_message` | Delete a message |
| `search_messages` | Search messages across chats |
| `get_updates` | Get recent updates/notifications |

## Important Notes

- This uses the **user API**, not the Bot API — the agent acts as a full Telegram user
- The agent needs its own phone number for the Telegram account
- Session files contain authentication state — keep the workspace secure
- Rate limits: 30 requests/minute (tool-enforced), Telegram also has flood-wait limits
