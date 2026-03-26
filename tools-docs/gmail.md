# Gmail Tool

> Read, send, draft, reply, and manage email via the Gmail API.

## Authentication

Gmail uses **Google OAuth 2.0** with offline access. The OAuth flow opens a browser
for consent, then stores refresh + access tokens encrypted in the secret store.

### Setup Steps

1. **Run the OAuth flow**
   ```bash
   thinclaw auth gmail
   ```
   This opens a browser window for Google consent. Sign in with the Gmail account
   the agent should use and grant the requested permissions.

2. **Headless / Remote Setup**
   If running on a headless server (e.g. Mac Mini via SSH):
   ```bash
   # On your local machine:
   ssh -L 9876:127.0.0.1:9876 user@server

   # On the server:
   thinclaw auth gmail
   ```
   The OAuth callback URL (`http://127.0.0.1:9876/callback`) is forwarded to
   the server. The auth URL is printed to the terminal — copy it to your local browser.

3. **Verify**
   ```
   You: Check my recent emails
   ```

### OAuth Scopes Requested

| Scope | Purpose |
|-------|---------|
| `gmail.readonly` | Read emails and labels |
| `gmail.send` | Send emails |
| `pubsub` | Real-time notifications (for Gmail channel, optional) |

### Secret Name

`google_oauth_token` — shared with all Google tools (Calendar, Docs, Drive, Sheets, Slides).
Authenticating once with `thinclaw auth google` gives all Google tools access.

## Available Actions (6)

| Action | Description |
|--------|-------------|
| `list_messages` | List messages with optional query filter, label, and pagination |
| `get_message` | Get a single message by ID (full content) |
| `send_message` | Send an email (to, subject, body, optional cc/bcc) |
| `create_draft` | Create a draft email |
| `reply_to_message` | Reply to an existing message thread |
| `trash_message` | Move a message to trash |

## Rate Limits

- 60 requests/minute, 500 requests/hour (enforced by capabilities)
- Google API also enforces per-user quotas

## See Also

- [docs/GMAIL_SETUP.md](../docs/GMAIL_SETUP.md) — Detailed guide including Gmail Channel (real-time inbound)
