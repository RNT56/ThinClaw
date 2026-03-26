# HTTP Webhook Channel

> Simple HTTP POST endpoint for receiving messages from external systems.

## Overview

The HTTP channel exposes a webhook endpoint that accepts messages via POST requests.
Designed for integrating with automation systems, CI/CD pipelines, or custom clients.

## Configuration

```bash
# Set the port to enable (no separate ENABLED flag — presence of port enables it)
HTTP_PORT=8080
HTTP_HOST=0.0.0.0

# Optional webhook secret for HMAC verification
HTTP_WEBHOOK_SECRET=my-webhook-secret

# User ID for messages from this channel
HTTP_USER_ID=http
```

## Usage

Send a message to the agent:
```bash
curl -X POST http://localhost:8080/webhook \
  -H "Content-Type: application/json" \
  -d '{"content": "Run the test suite"}'
```

## Notes

- Shares the unified webhook server with WASM channel webhook routes
- Default bind address is `0.0.0.0` — restrict with `HTTP_HOST=127.0.0.1` if needed
- Responses are delivered asynchronously (the agent processes and responds via the channel)
