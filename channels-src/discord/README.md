# Discord Interactions Channel for ThinClaw

Packaged WASM channel for Discord interactions in ThinClaw. This is the webhook/slash-command path, not the native Discord Gateway runtime.

## Features

- **Slash Commands** - Process Discord slash commands
- **Button Interactions** - Handle button clicks
- **Thread Support** - Respond in threads
- **DM Support** - Handle direct messages

## Install and Setup

1. Install the packaged channel from the ThinClaw registry:

   ```bash
   thinclaw registry install discord
   ```

2. Create a Discord Application at <https://discord.com/developers/applications>
3. Create a Bot and get the token
4. Set the Interactions URL to point at your ThinClaw host
5. Copy the Application ID and Public Key
6. Store the secrets ThinClaw expects:

   ```bash
   thinclaw secret set discord_bot_token YOUR_BOT_TOKEN
   ```

   **Note:** The `discord_bot_token` secret is the only value read directly by this
   Discord channel WASM component. The `discord_app_id` and `discord_public_key`
   secrets are used by the ThinClaw host (for example, to verify Discord
   interaction signatures and manage slash command registration) and are not
   accessed from the WASM module itself.

## Discord Configuration

### Register Slash Commands

```bash
curl -X POST \
  -H "Authorization: Bot YOUR_BOT_TOKEN" \
  -H "Content-Type: application/json" \
  https://discord.com/api/v10/applications/YOUR_APP_ID/commands \
  -d '{
    "name": "ask",
    "description": "Ask the AI agent",
    "options": [{
      "name": "question",
      "description": "Your question",
      "type": 3,
      "required": true
    }]
  }'
```

### Set Interactions Endpoint

In your Discord app settings, set:

- Interactions Endpoint URL: `https://your-thinclaw.com/webhook/discord`

ThinClaw registers the packaged Discord interactions channel on `/webhook/discord`. If you want normal message-stream handling instead of webhook interactions, use the native Discord Gateway path documented in [../../channels-docs/discord.md](../../channels-docs/discord.md).

## Usage Examples

### Slash Command

User types: `/ask question: What is the weather?`

The agent receives:

```text
User: @username
Content: /ask question: What is the weather?
```

### Button Click

When a user clicks a button in a message, the agent receives:

```text
User: @username
Content: [Button clicked] Original message content
```

## Error Handling

If an internal error occurs (e.g., metadata serialization failure), the tool attempts to send an ephemeral message to the user:

```text
❌ Internal Error: Failed to process command metadata.
```

Check the host logs for detailed error information.

## Advanced Usage

### Embeds

To send embeds, include an `embeds` array in the `metadata_json` field of the agent's response. The structure should match the Discord API `embed` object.

## Troubleshooting

### "Invalid Signature"

- Check that `discord_public_key` is set correctly in ThinClaw secrets.
- This validation happens on the host before reaching the WASM.

### "401 Unauthorized"

- Check that `discord_bot_token` is set correctly in ThinClaw secrets.
- Ensure the bot is added to the server.

### "Interaction Failed"

- The interaction might have timed out (Discord requires a response within 3 seconds).
- The `interactions_endpoint_url` might be unreachable.

## Building

```bash
cd channels-src/discord
cargo build --target wasm32-wasip2 --release
```

## License

MIT/Apache-2.0
