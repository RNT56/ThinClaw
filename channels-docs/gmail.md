# Gmail Channel

> Real-time inbound email via Gmail API + Google Pub/Sub.

## Overview

ThinClaw treats Gmail as two related surfaces:

- the Gmail tool for reading and sending email
- the Gmail channel for real-time inbound delivery through Pub/Sub

This page documents the channel. The Gmail tool uses the same Google OAuth
token and should be configured once through the tool auth flow.

## Prerequisites

1. **Google Cloud Project** with Gmail API and Pub/Sub enabled
2. **Pub/Sub Topic** with Gmail push configured
3. **Shared Google OAuth token** from `thinclaw tool auth gmail`

### Setup Steps

1. Create a GCP project at [console.cloud.google.com](https://console.cloud.google.com)
2. Enable the Gmail API and Pub/Sub API
3. Create a Pub/Sub topic and subscription
4. Grant `gmail-api-push@system.gserviceaccount.com` Pub/Sub Publisher on your topic
5. Set up a Gmail watch: the agent does this automatically on startup
6. Run `thinclaw tool auth gmail` once; the channel reuses the shared token

## Configuration

```bash
# Required
GMAIL_ENABLED=true
GMAIL_PROJECT_ID=my-gcp-project
GMAIL_SUBSCRIPTION_ID=gmail-push-sub
GMAIL_TOPIC_ID=gmail-push-topic

# Optional: OAuth token shared with the Gmail tool
GMAIL_OAUTH_TOKEN=

# Optional: restrict to specific senders (empty = all)
GMAIL_ALLOWED_SENDERS=boss@company.com,team@company.com

# Optional: label filters (default: INBOX,UNREAD)
GMAIL_LABEL_FILTERS=INBOX,UNREAD

# Optional: max message body size (default: 10MB)
GMAIL_MAX_MESSAGE_SIZE=10485760
```

## Features

- Real-time email via Pub/Sub (near-instant delivery)
- Shared OAuth2 authentication with the Gmail tool
- Sender allow-list
- Label filtering
- Body size limits
- Unread detection

## Headless Auth

For headless servers (Mac Mini via SSH):
```bash
# On local machine:
ssh -L 9876:127.0.0.1:9876 user@server

# On server:
thinclaw tool auth gmail
```
Copy the auth URL to your local browser. The callback is forwarded to the server.

## Notes

- Requires a GCP project (Pub/Sub costs are negligible for email volumes)
- The OAuth token is shared with the Gmail tool and other Google tools (`google_oauth_token`)
- For simpler email on macOS, consider the [Apple Mail channel](apple-mail.md) instead
