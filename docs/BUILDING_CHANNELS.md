# Building WASM Channels

This guide covers how to build WASM channel modules for ThinClaw.

## Overview

Channels are WASM components that handle communication with external messaging platforms (Telegram, WhatsApp, Slack, etc.). They run in a sandboxed environment and communicate with the host via the WIT (WebAssembly Interface Types) interface.

## Directory Structure

```
channels/                    # Or channels-src/
└── my-channel/
    ├── Cargo.toml
    ├── src/
    │   └── lib.rs
    └── my-channel.capabilities.json
```

After building, deploy to:
```
~/.thinclaw/channels/
├── my-channel.wasm
└── my-channel.capabilities.json
```

## Cargo.toml Template

```toml
[package]
name = "my-channel"
version = "0.1.0"
edition = "2021"
description = "My messaging platform channel for ThinClaw"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[profile.release]
opt-level = "s"
lto = true
strip = true
codegen-units = 1
```

## Channel Implementation

### Required Imports

```rust
// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",  // Adjust path as needed
});

use serde::{Deserialize, Serialize};

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig,
};
use near::agent::channel_host::{self, EmittedMessage};
```

### Implementing the Guest Trait

```rust
struct MyChannel;

impl Guest for MyChannel {
    /// Called once when the channel starts.
    /// Returns configuration for webhooks and polling.
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        // Parse config from capabilities file
        let config: MyConfig = serde_json::from_str(&config_json)
            .unwrap_or_default();

        Ok(ChannelConfig {
            display_name: "My Channel".to_string(),
            http_endpoints: vec![
                HttpEndpointConfig {
                    path: "/webhook/my-channel".to_string(),
                    methods: vec!["POST".to_string()],
                    require_secret: true,  // Validate webhook secret
                },
            ],
            poll: None,  // Or Some(PollConfig { interval_ms, enabled })
        })
    }

    /// Handle incoming HTTP requests (webhooks).
    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Parse webhook payload
        // Emit messages to agent
        // Return response to webhook caller
    }

    /// Called periodically if polling is enabled.
    fn on_poll() {
        // Fetch new messages from API
        // Emit any new messages
    }

    /// Send a response back to the messaging platform.
    fn on_respond(response: AgentResponse) -> Result<(), String> {
        // Parse metadata to get routing info
        // Convert Markdown to platform-native format (see Message Formatting below)
        // Call platform API to send message
    }

    /// Called when channel is shutting down.
    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Channel shutting down");
    }
}

// Export the channel implementation
export!(MyChannel);
```

## Critical Pattern: Metadata Flow

**The most important pattern**: Store routing info in message metadata so responses can be delivered.

```rust
// When receiving a message, store routing info:
#[derive(Debug, Serialize, Deserialize)]
struct MyMessageMetadata {
    chat_id: String,           // Where to send response
    sender_id: String,         // Who sent it (becomes recipient)
    original_message_id: String,
}

// In on_http_request or on_poll:
let metadata = MyMessageMetadata {
    chat_id: message.chat.id.clone(),
    sender_id: message.from.clone(),  // CRITICAL: Store sender!
    original_message_id: message.id.clone(),
};

channel_host::emit_message(&EmittedMessage {
    user_id: message.from.clone(),
    user_name: Some(name),
    content: text,
    thread_id: None,
    metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
});

// In on_respond, use the ORIGINAL message's metadata:
fn on_respond(response: AgentResponse) -> Result<(), String> {
    let metadata: MyMessageMetadata = serde_json::from_str(&response.metadata_json)?;

    // sender_id becomes the recipient!
    send_message(metadata.chat_id, metadata.sender_id, response.content);
}
```

The host now merges `OutgoingResponse.metadata` into the original inbound metadata
before calling `on_respond()`. Response metadata wins on key conflicts.

When the host needs to tunnel outbound attachments into a WASM channel, it also
serializes `OutgoingResponse.attachments` into `response.metadata_json` under
`response_attachments` using this envelope:

```json
[
  {
    "mime_type": "image/png",
    "filename": "reply.png",
    "data": "<base64>"
  }
]
```

## Credential Injection

**Never hardcode credentials!** Use placeholders that the host replaces:

### URL Placeholders (Telegram-style)

```rust
// The host replaces {TELEGRAM_BOT_TOKEN} with the actual token
let url = "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendMessage";
channel_host::http_request("POST", url, &headers_json, Some(&body));
```

### Header Placeholders (WhatsApp-style)

```rust
// The host replaces {WHATSAPP_ACCESS_TOKEN} in headers too
let headers = serde_json::json!({
    "Content-Type": "application/json",
    "Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}"
});
channel_host::http_request("POST", &url, &headers.to_string(), Some(&body));
```

The placeholder format is `{SECRET_NAME}` where `SECRET_NAME` matches the credential name in uppercase with underscores (e.g., `whatsapp_access_token` → `{WHATSAPP_ACCESS_TOKEN}`).

## Capabilities File

Create `my-channel.capabilities.json`:

```json
{
  "type": "channel",
  "name": "my-channel",
  "description": "My messaging platform channel",
  "setup": {
    "required_secrets": [
      {
        "name": "my_channel_api_token",
        "prompt": "Enter your API token",
        "validation": "^[A-Za-z0-9_-]+$"
      },
      {
        "name": "my_channel_webhook_secret",
        "prompt": "Webhook secret (leave empty to auto-generate)",
        "optional": true,
        "auto_generate": { "length": 32 }
      }
    ],
    "validation_endpoint": "https://api.my-platform.com/verify?token={my_channel_api_token}"
  },
  "capabilities": {
    "http": {
      "allowlist": [
        { "host": "api.my-platform.com", "path_prefix": "/" }
      ],
      "rate_limit": {
        "requests_per_minute": 60,
        "requests_per_hour": 1000
      }
    },
    "secrets": {
      "allowed_names": ["my_channel_*"]
    },
    "channel": {
      "allowed_paths": ["/webhook/my-channel"],
      "allow_polling": false,
      "workspace_prefix": "channels/my-channel/",
      "emit_rate_limit": {
        "messages_per_minute": 100,
        "messages_per_hour": 5000
      },
      "webhook": {
        "secret_header": "X-Webhook-Secret",
        "secret_name": "my_channel_webhook_secret",
        "secret_validation": "equals",
        "verify_token_param": "hub.verify_token",
        "verify_token_secret_name": "my_channel_verify_token"
      }
    }
  },
  "config": {
    "custom_option": "value"
  }
}
```

### Webhook Validation Modes

- `secret_validation: "equals"` compares the configured secret directly to the
  provided header value. This is the default.
- `secret_validation: "hmac_sha256_body"` treats the configured secret as an
  HMAC key and validates the raw request body against the configured
  `secret_header` value. Use this for providers like WhatsApp or GitHub.
- `secret_validation: "hmac_sha256_base64_body"` validates providers that send
  base64-encoded HMAC signatures, such as LINE.
- `secret_validation: "twilio_request_signature"` validates Twilio callback
  signatures against the callback URL, form/query parameters, and auth token.
- `secret_validation: "twitch_eventsub_hmac_sha256"` validates Twitch EventSub
  signatures using the message id, timestamp, and raw body.
- `verify_token_param` enables GET/HEAD query-param verification for webhook
  setup handshakes.
- `verify_token_secret_name` lets GET/HEAD verification use a different secret
  from POST signature validation.

### Structured Response Bodies

Shared webhook packages can render response payloads as nested JSON, arrays, or
URL-encoded forms. Prefer structured JSON bodies for provider APIs instead of
flattening everything into string fields. This is required for providers such as
LINE (`messages: [...]`), DingTalk (`text: { content: ... }`), Teams activity
replies, and WeCom/Weixin callback responses.

Template strings inside nested values still support `{content}`, request
metadata placeholders, and explicit `template_values`. Secret placeholders
remain uppercase (`{LINE_CHANNEL_ACCESS_TOKEN}`, `{TWILIO_ACCOUNT_SID}`, etc.)
and must map to declared lowercase secret names in `allowed_names`.

## Building and Deploying

### Supply Chain Security: No Committed Binaries

**Do not commit compiled WASM binaries.** They are a supply chain risk — the binary in a PR may not match the source. ThinClaw builds channels from source:

- `cargo build` automatically builds `telegram.wasm` via `build.rs`
- The built binary is in `.gitignore` and is not committed
- CI should run `cargo build` (or `./scripts/build-all.sh`) to produce releases

**Prerequisites:**
```bash
rustup target add wasm32-wasip2
cargo install wasm-tools --locked   # optional; fallback copies raw WASM if unavailable
```

### Build All Channels

The easiest way to build everything (all WASM channels + main binary):

```bash
./scripts/build-all.sh
```

This script:
1. Discovers all channels in `channels-src/`
2. Uses each channel's `build.sh` if present, otherwise runs the generic build pipeline
3. Deploys `.wasm` + `.capabilities.json` to `~/.thinclaw/channels/`
4. Builds the main ThinClaw binary

For air-gapped builds with all WASM extensions embedded:
```bash
./scripts/build-all.sh --bundled
```

### Single Channel Build

To build a specific channel individually:

```bash
# Channels with a build.sh (Telegram, Slack):
./channels-src/telegram/build.sh

# Channels without a build.sh (WhatsApp, Discord):
cd channels-src/whatsapp
cargo build --release --target wasm32-wasip2
wasm-tools component new target/wasm32-wasip2/release/whatsapp_channel.wasm -o whatsapp.wasm 2>/dev/null \
  || cp target/wasm32-wasip2/release/whatsapp_channel.wasm whatsapp.wasm
wasm-tools strip whatsapp.wasm -o whatsapp.wasm

# Deploy to ~/.thinclaw/channels/
cp whatsapp.wasm whatsapp.capabilities.json ~/.thinclaw/channels/
```

**Note**: The main ThinClaw binary bundles `telegram.wasm` via `include_bytes!`. When modifying the Telegram channel source, run `./channels-src/telegram/build.sh` **before** building the main crate, so the updated WASM is included.

## Host Functions Available

The channel host provides these functions:

```rust
// Logging
channel_host::log(LogLevel::Info, "Message");

// Time
let now = channel_host::now_millis();

// Workspace (scoped to channel namespace)
let data = channel_host::workspace_read("state/offset");
channel_host::workspace_write("state/offset", "12345")?;

// HTTP requests (credentials auto-injected)
let response = channel_host::http_request("POST", &url, &headers, Some(&body))?;

// Emit message to agent
channel_host::emit_message(&EmittedMessage { ... });
```

## Common Patterns

### Webhook Secret Validation

The host validates webhook secrets automatically. Check `req.secret_validated`:

```rust
fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
    if !req.secret_validated {
        channel_host::log(LogLevel::Warn, "Invalid webhook secret");
        // Host should have already rejected, but defense in depth
    }
    // ...
}
```

### Polling with Offset Tracking

For platforms that require polling (not webhook-based):

```rust
const OFFSET_PATH: &str = "state/last_offset";

fn on_poll() {
    // Read last offset
    let offset = channel_host::workspace_read(OFFSET_PATH)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);

    // Fetch updates since offset
    let updates = fetch_updates(offset);

    // Process and track new offset
    let mut new_offset = offset;
    for update in updates {
        if update.id >= new_offset {
            new_offset = update.id + 1;
        }
        emit_message(update);
    }

    // Save new offset
    if new_offset != offset {
        let _ = channel_host::workspace_write(OFFSET_PATH, &new_offset.to_string());
    }
}
```

### Status Message Filtering

Skip status updates to prevent loops:

```rust
// Skip status updates (delivered, read, etc.)
if !payload.statuses.is_empty() && payload.messages.is_empty() {
    return;  // Only status updates, no actual messages
}
```

### Bot Message Filtering

Skip bot messages to prevent infinite loops:

```rust
if sender.is_bot {
    return;  // Don't respond to bots
}
```

## Message Formatting

LLMs output standard Markdown, but each messaging platform has its own formatting syntax. Channels should convert Markdown to platform-native format in `on_respond` **before** splitting into chunks.

### Built-in Converters

| Channel | Function | Conversions |
|---------|----------|-------------|
| **Telegram** | `markdown_to_telegram_html()` | MD → HTML, `parse_mode=HTML` |
| **Slack** | `markdown_to_slack_mrkdwn()` | `**bold**` → `*bold*`, `[text](url)` → `<url\|text>`, `# Heading` → `*Heading*` |
| **WhatsApp** | `markdown_to_whatsapp()` | `**bold**` → `*bold*`, `[text](url)` → `text (url)`, `# Heading` → `*Heading*` |
| **Discord** | Pass-through | Discord natively supports Markdown |

### Implementation Pattern

```rust
fn on_respond(response: AgentResponse) -> Result<(), String> {
    let metadata: MyMetadata = serde_json::from_str(&response.metadata_json)?;

    // 1. Convert Markdown to platform format
    let formatted = markdown_to_my_platform(&response.content);

    // 2. Split into platform-sized chunks (AFTER formatting)
    let chunks = split_message(&formatted, MAX_MESSAGE_LENGTH);

    // 3. Send each chunk
    for chunk in &chunks {
        send_message(&metadata, chunk)?;
    }
    Ok(())
}
```

### Key Rules

- **Convert before splitting** — Platform-specific char limits apply to the _formatted_ output, not raw Markdown
- **Preserve code blocks** — Never convert formatting inside ``` fences or backtick spans
- **Preserve inline code** — Content inside backticks passes through unchanged
- **Graceful degradation** — If a platform doesn't support a feature (e.g., WhatsApp headings), convert to the nearest equivalent (bold text)

## Testing

Add tests in the same file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_webhook() {
        let json = r#"{ ... }"#;
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.messages.len(), 1);
    }

    #[test]
    fn test_metadata_roundtrip() {
        let meta = MyMessageMetadata { ... };
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: MyMessageMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(meta.chat_id, parsed.chat_id);
    }

    #[test]
    fn test_formatting_conversion() {
        assert_eq!(markdown_to_my_platform("**bold**"), "*bold*");
    }
}
```

Run tests with:
```bash
cargo test
```

## Troubleshooting

### "byte index N is not a char boundary"

Never slice strings by byte index! Use character-aware truncation:

```rust
// BAD: panics on multi-byte UTF-8 (emoji, etc.)
let preview = &content[..50];

// GOOD: safe truncation
let preview: String = content.chars().take(50).collect();
```

### Credential placeholders not replaced

1. Check the secret name matches (lowercase with underscores)
2. Verify the secret is in `allowed_names` in capabilities
3. Check logs for "unresolved placeholders" warnings

### Messages not routing to responses

Ensure `on_respond` uses the ORIGINAL message's metadata, not response metadata:
```rust
// response.metadata_json comes from the ORIGINAL emit_message call
let metadata: MyMetadata = serde_json::from_str(&response.metadata_json)?;
```
