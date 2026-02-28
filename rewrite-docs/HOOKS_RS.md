# Lifecycle Hooks & External Triggers

Hooks are the **reactive** counterpart to cron's **proactive** scheduling. While cron runs on a schedule, hooks fire in response to **external events** — a new email arrives, a webhook POSTs data, the agent boots, or a message is sent.

---

## 1. What OpenClaw Does Today

OpenClaw's hook system (`src/hooks/`, 43 files) provides:

### Internal Hooks (`internal-hooks.ts`)

An event bus with typed events and handlers:

| Event Type | Action | Fires When |
|---|---|---|
| `agent` | `bootstrap` | Agent session starts (system prompt loaded) |
| `gateway` | `startup` | Gateway server starts |
| `message` | `received` | An inbound message is received from any channel |
| `message` | `sent` | An outbound message is sent to any channel |
| `command` | `new` | A slash command is processed |

Handlers are registered with `registerInternalHook("message:received", async (event) => { ... })`. Multiple handlers per event key are supported. Errors in one handler don't prevent others from running.

### Gmail Hook (`gmail.ts`, `gmail-ops.ts`, `gmail-watcher.ts`)

A full Gmail integration that watches an inbox for new emails:
- Connects via Google Pub/Sub push notifications
- Authenticates with OAuth2 (`gog` helper binary)
- Receives new email notifications via webhook
- Extracts email body (up to 20KB), formats it for the LLM
- Delivers email content to the agent as a system event
- Supports Tailscale serve/funnel for webhook endpoint exposure

### Webhook Hooks (`hooks.ts`, `loader.ts`)

Generic HTTP webhook endpoints that trigger agent actions when external services POST data (e.g., GitHub events, Stripe webhooks).

---

## 2. Rust Hook Architecture

### Hook Registry

```rust
use std::collections::HashMap;
use tokio::sync::broadcast;

pub type HookEventType = String; // e.g., "message", "agent", "gateway"
pub type HookAction = String;    // e.g., "received", "bootstrap", "startup"

#[derive(Clone, Debug)]
pub struct HookEvent {
    pub event_type: HookEventType,
    pub action: HookAction,
    pub session_key: Option<String>,
    pub context: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

pub struct HookRegistry {
    tx: broadcast::Sender<HookEvent>,
}

impl HookRegistry {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    /// Fire an event to all subscribers
    pub fn trigger(&self, event: HookEvent) {
        let _ = self.tx.send(event);
    }

    /// Subscribe to events (filtered by event_type and/or action)
    pub fn subscribe(&self) -> broadcast::Receiver<HookEvent> {
        self.tx.subscribe()
    }
}
```

### Built-In Hook Points

The Orchestrator fires hooks at these locations:

```rust
// On agent session start
hooks.trigger(HookEvent {
    event_type: "agent".into(),
    action: "bootstrap".into(),
    context: json!({ "agent_id": agent_id, "session_key": session_key }),
    ..
});

// On inbound message from any channel
hooks.trigger(HookEvent {
    event_type: "message".into(),
    action: "received".into(),
    context: json!({
        "from": sender,
        "content": text,
        "channel": "telegram",
        "conversation_id": chat_id,
    }),
    ..
});

// On outbound message sent
hooks.trigger(HookEvent {
    event_type: "message".into(),
    action: "sent".into(),
    context: json!({
        "to": recipient,
        "content": response_text,
        "channel": "discord",
        "success": true,
    }),
    ..
});
```

---

## 3. Gmail Integration in Rust

Gmail integration is a **hook preset** — a pre-built handler that connects to Gmail via Google Cloud Pub/Sub.

### Architecture

```
Gmail Inbox → Google Pub/Sub → Push Notification (HTTPS POST)
    → ThinClaw webhook endpoint (/hooks/gmail)
    → GmailHookHandler processes email
    → Injects email content as HookEvent
    → Agent receives and responds
```

### Config

```toml
[hooks]
token = "your-webhook-auth-token"  # stored in Keychain via secret_ref

[hooks.gmail]
account = "user@gmail.com"
topic = "projects/my-gcp-project/topics/gmail-watch"
subscription = "gmail-watch-push"
push_token = "your-pubsub-token"   # stored in Keychain via secret_ref
label = "INBOX"
include_body = true
max_bytes = 20000

[hooks.gmail.tailscale]
mode = "serve"                     # Expose webhook via Tailscale Serve
path = "/gmail-pubsub"
```

### Rust Handler

```rust
use axum::{Router, extract::Json, http::StatusCode};

pub struct GmailHookHandler {
    hook_registry: Arc<HookRegistry>,
    config: GmailHookConfig,
}

impl GmailHookHandler {
    pub fn router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/hooks/gmail", axum::routing::post(Self::handle_push))
            .with_state(self)
    }

    async fn handle_push(
        state: axum::extract::State<Arc<Self>>,
        Json(notification): Json<PubSubNotification>,
    ) -> StatusCode {
        // Validate push token
        if notification.subscription != state.config.subscription {
            return StatusCode::FORBIDDEN;
        }

        // Decode email from Pub/Sub message
        let email_data = base64::decode(&notification.message.data)?;
        let email = parse_gmail_notification(&email_data, state.config.max_bytes);

        // Fire hook event
        state.hook_registry.trigger(HookEvent {
            event_type: "email".into(),
            action: "received".into(),
            context: json!({
                "from": email.from,
                "subject": email.subject,
                "body": email.body_truncated,
                "date": email.date,
            }),
            ..Default::default()
        });

        StatusCode::OK
    }
}
```

---

## 4. Generic Webhook Hooks

Users can define custom webhook endpoints in config:

```toml
[[hooks.webhooks]]
name = "github-events"
path = "/hooks/github"
token = "github-webhook-secret"    # Validated via HMAC-SHA256
agent_prompt = "A GitHub event occurred: {payload}"
```

When a POST arrives at `/hooks/github`, the Orchestrator:
1. Validates the signature (HMAC-SHA256 with the token)
2. Extracts the JSON payload
3. Templates the `agent_prompt` with the payload
4. Fires a `HookEvent` which triggers an agent session

---

## 5. Hook → Cron Relationship

Hooks and cron are two sides of the same coin:

| | Cron | Hooks |
|---|---|---|
| **Trigger** | Time-based (schedule) | Event-based (external stimulus) |
| **Session** | Can use main or isolated | Always isolated (event context) |
| **Delivery** | Configurable (channel, webhook) | Typically replies in-channel |
| **Config** | `config.toml [cron.*]` | `config.toml [hooks.*]` |
| **Persistence** | `cron.json` store file | Stateless (events are transient) |

Both funnel into the same agent execution pipeline — the only difference is what starts the execution.

---

## 6. Crate Dependencies

```toml
[dependencies]
axum = "0.7"         # Webhook HTTP server
base64 = "0.22"      # Pub/Sub message decoding
hmac = "0.12"        # Webhook signature validation
sha2 = "0.10"        # HMAC-SHA256
```
