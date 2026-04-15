# Gmail Setup Guide

ThinClaw treats Gmail as two related surfaces:

| Mode | Purpose | What it does | GCP Project required? |
|------|---------|--------------|----------------------|
| **Gmail Tool** | Send & read email | The agent can compose, reply, and search emails via the Gmail API | No |
| **Gmail Channel** | Receive email in real-time | Incoming emails are pushed to the agent via Google Pub/Sub | Yes |

You can use either or both. Most users want the **tool** for send/read and
optionally the **channel** for real-time inbound.

---

## Prerequisites

- A Google account for the agent (or your own account)
- ThinClaw installed with the `thinclaw` CLI available
- For the **channel**: a Google Cloud Platform project (free tier is sufficient)

---

## 1. Gmail Tool (Read & Send Emails)

The Gmail tool is a WASM extension that gives the agent the ability to:
- List and search emails
- Read email content
- Send new emails
- Reply to threads

### Setup

```bash
# Install the Gmail tool extension
thinclaw tool install gmail

# Authenticate with Google
thinclaw tool auth gmail
```

The `thinclaw tool auth gmail` command will:
1. Open your browser to Google's OAuth consent screen
2. Ask you to sign in with the agent's Google account
3. Request permission for `gmail.readonly`, `gmail.send`, and `pubsub` scopes
4. Store the OAuth token securely in ThinClaw's encrypted secrets store

**That's it.** The agent can now read and send emails.

### Headless / Remote Server Setup

If ThinClaw is running on a headless server (e.g., a Mac Mini via SSH), the browser won't open automatically. Instead:

```bash
# From your local machine, set up SSH port forwarding:
ssh -L 9876:127.0.0.1:9876 user@your-server

# On the server (via SSH), run:
thinclaw tool auth gmail

# The CLI will print:
#   Could not open browser: no display
#   Please open this URL manually:
#   https://accounts.google.com/o/oauth2/v2/auth?client_id=...
#
#   Waiting for authorization...

# Copy the URL and paste it into your LOCAL browser.
# Sign in → click "Allow"
# Google redirects to localhost:9876/callback
# The SSH tunnel forwards the callback to the server
# ThinClaw captures the token:
#   ✓ Google connected!
```

### Environment Overrides

| Variable | Default | Description |
|----------|---------|-------------|
| `GOOGLE_OAUTH_CLIENT_ID` | Built-in | Override the OAuth client ID |
| `GOOGLE_OAUTH_CLIENT_SECRET` | Built-in | Override the OAuth client secret |
| `THINCLAW_OAUTH_CALLBACK_URL` | `http://127.0.0.1:9876` | Custom callback URL for remote setups |
| `OAUTH_CALLBACK_HOST` | `127.0.0.1` | Listen address for the callback server |

ThinClaw ships with built-in Google "Desktop App" OAuth credentials (similar to `gcloud`, `rclone`). You do **not** need to register your own Google Cloud OAuth app unless you want to.

---

## 2. Gmail Channel (Real-Time Inbound Email)

The Gmail channel pushes incoming emails to the agent in real-time via Google Cloud Pub/Sub. The agent can then process and respond to emails automatically.

### Architecture

```
Incoming email → Gmail inbox
    → Google Pub/Sub notification (push)
        → ThinClaw polls the Pub/Sub subscription (every 10s)
            → Fetches full email via Gmail API
                → Passes to agent as IncomingMessage
                    → Agent processes and responds via Gmail API
```

### Prerequisites

You need a Google Cloud Platform project with:
1. **Gmail API** enabled
2. **Cloud Pub/Sub API** enabled
3. A **Pub/Sub topic** for Gmail notifications
4. A **Pub/Sub subscription** (pull-based) on that topic

### Step 1: Create a GCP Project

If you don't already have one:

1. Go to [console.cloud.google.com](https://console.cloud.google.com)
2. Create a new project (e.g., `thinclaw-agent`)
3. Note the **Project ID** (e.g., `thinclaw-agent-12345`)

### Step 2: Enable APIs

```bash
# Using gcloud CLI (or do this in the Cloud Console)
gcloud services enable gmail.googleapis.com --project=YOUR_PROJECT_ID
gcloud services enable pubsub.googleapis.com --project=YOUR_PROJECT_ID
```

### Step 3: Create Pub/Sub Topic and Subscription

```bash
# Create the topic
gcloud pubsub topics create gmail-notifications --project=YOUR_PROJECT_ID

# Grant Gmail permission to publish to this topic
gcloud pubsub topics add-iam-policy-binding gmail-notifications \
  --project=YOUR_PROJECT_ID \
  --member="serviceAccount:gmail-api-push@system.gserviceaccount.com" \
  --role="roles/pubsub.publisher"

# Create a pull subscription
gcloud pubsub subscriptions create gmail-agent-sub \
  --project=YOUR_PROJECT_ID \
  --topic=gmail-notifications
```

### Step 4: Configure ThinClaw

During the onboarding wizard (`thinclaw onboard`), select Gmail as a channel.
The wizard will prompt for:

| Setting | Value | Example |
|---------|-------|---------|
| GCP Project ID | Your project ID | `thinclaw-agent-12345` |
| Pub/Sub Subscription ID | Subscription name | `gmail-agent-sub` |
| Pub/Sub Topic ID | Topic name | `gmail-notifications` |
| Allowed Senders | Comma-separated emails (empty = all) | `boss@company.com,team@company.com` |

Or configure via environment variables:

```bash
GMAIL_ENABLED=true
GMAIL_PROJECT_ID=thinclaw-agent-12345
GMAIL_SUBSCRIPTION_ID=gmail-agent-sub
GMAIL_TOPIC_ID=gmail-notifications
GMAIL_ALLOWED_SENDERS=boss@company.com,team@company.com  # optional
```

### Step 5: Authenticate

```bash
thinclaw tool auth gmail
```

This uses the same OAuth flow described in the Tool section above. The token
is shared between the Gmail tool, the Gmail channel, and other Google tools.

### Step 6: Register Gmail Watch

After authentication, you need to tell Gmail to push notifications to your Pub/Sub topic.
This is done via the Gmail API's `watch` endpoint. ThinClaw handles this automatically
when the channel starts, but you can also do it manually:

```bash
# Using curl (replace YOUR_TOKEN with the OAuth access token):
curl -X POST \
  'https://gmail.googleapis.com/gmail/v1/users/me/watch' \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "topicName": "projects/YOUR_PROJECT_ID/topics/gmail-notifications",
    "labelIds": ["INBOX"]
  }'
```

> **Note:** Gmail watch registrations expire after 7 days. ThinClaw automatically
> re-registers the watch on startup.

---

## Security Considerations

### OAuth Token Storage

- OAuth tokens are encrypted with AES-256-GCM using ThinClaw's master key
- Tokens are stored in the database secrets table, not on disk in plaintext
- The master key is stored in the OS secure store (macOS Keychain, Linux Secret Service, or Windows Credential Manager)
- The agent never sees raw tokens — the WASM sandbox uses the host's HTTP proxy
  to inject credentials into API requests

### Sender Filtering

- Use `GMAIL_ALLOWED_SENDERS` to restrict which email addresses can trigger the agent
- An empty list means **all senders are allowed**
- Filtering is case-insensitive substring matching (e.g., `@company.com` matches all company emails)

### Message Size Limits

- Maximum message size: 10 MB (configurable via `max_message_size_bytes`)
- Messages exceeding this limit are skipped with a warning

---

## Troubleshooting

### "No OAuth token configured"

Run `thinclaw tool auth gmail` to authenticate. If the token has expired,
re-run the command to get a fresh token.

### "Pub/Sub pull returned 403"

The OAuth token doesn't have the `pubsub` scope. Re-run `thinclaw tool auth gmail`
to get a token with the correct scopes.

### "Gmail list returned 401"

The OAuth token has expired. ThinClaw should auto-refresh tokens,
but if it doesn't, re-run `thinclaw tool auth gmail`.

### Emails not arriving

1. Verify the Pub/Sub subscription exists: `gcloud pubsub subscriptions describe gmail-agent-sub`
2. Check that the Gmail watch is active (watches expire after 7 days)
3. Check ThinClaw logs: `RUST_LOG=thinclaw::channels::gmail=debug thinclaw run`
4. Verify the sender is in the allowed list (if configured)

### Headless OAuth fails

Make sure SSH port forwarding is set up **before** running `thinclaw tool auth gmail`:

```bash
ssh -L 9876:127.0.0.1:9876 user@server
```

The port `9876` is ThinClaw's fixed OAuth callback port. If it's already in use,
check for other processes: `lsof -i :9876`.
