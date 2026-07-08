# Client SDK (`thinclaw-client`)

`thinclaw-client` is a typed async Rust client for driving a ThinClaw gateway —
the analog of an Agent SDK. It wraps the gateway's HTTP + SSE surface so an
external program can send chat messages, stream responses and tool-call events,
browse thread history, and resolve tool approvals.

The crate lives at [`crates/thinclaw-client`](../crates/thinclaw-client) and is
`publish = false` today (see Stability below).

## Auth

Bearer-token, matching the gateway's `Authorization: Bearer <token>` scheme.

```rust
use thinclaw_client::Client;

// Explicit:
let client = Client::new("http://127.0.0.1:8080", "your-gateway-token")?;
// Or from THINCLAW_GATEWAY_URL + THINCLAW_GATEWAY_TOKEN:
let client = Client::from_env()?;
```

The browser-only `?token=` query-param mode (for `EventSource`, which cannot set
headers) is intentionally not the Rust client's default — a native client always
sends the header.

## Operations

| Method | Route | Notes |
|---|---|---|
| `send_message(content, thread_id)` | `POST /api/chat/send` | Async-accepted; returns a `message_id`. The reply arrives over the event stream, correlated by `thread_id`. |
| `send_and_wait(content, thread_id, timeout)` | send + `events` | Convenience: blocks until the first matching `Response`/`Error`. |
| `events()` | `GET /api/chat/events` | SSE stream of `SseEvent`s. |
| `history(thread_id, limit, before)` | `GET /api/chat/history` | Paginated transcript. |
| `list_threads()` | `GET /api/chat/threads` | Thread summaries. |
| `new_thread()` | `POST /api/chat/thread/new` | Create a thread. |
| `resolve_approval(request_id, action, thread_id)` | `POST /api/chat/approval` | Approve/always/deny a pending tool call. |
| `abort(thread_id)` | `POST /api/chat/abort` | Cancel the in-flight turn. |

### OpenAI-compatible fast path

The gateway also exposes `/v1/chat/completions` and `/v1/models`. If you already
have an OpenAI-shaped Rust client, point it at `client.openai_compat_base_url()`
(`{base_url}/v1`) for zero new code — at the cost of ThinClaw-native features
(tool-call visibility, approvals, thread management), which are only available
through this client's native methods.

## Event catalog

`SseEvent` models a **curated subset** of the gateway's server-sent events:
`Response`, `Thinking`, `ReasoningContent`, `ToolStarted`, `ToolCompleted`,
`ToolResult`, `StreamChunk`, `Status`, `UsageUpdate`, `ApprovalNeeded`, `Error`,
and `Heartbeat`. Any other (or future) server event surfaces as
`SseEvent::Unknown { event_type, raw }` with its full JSON payload preserved, so
a newer server never breaks an older client. A contract test
(`tests/wire_contract.rs`) asserts the modeled variants stay compatible with the
real `thinclaw_gateway::web::types::SseEvent`.

## Example

See [`crates/thinclaw-client/examples/chat_loop.rs`](../crates/thinclaw-client/examples/chat_loop.rs):

```sh
export THINCLAW_GATEWAY_URL=http://127.0.0.1:8080
export THINCLAW_GATEWAY_TOKEN=your-gateway-token
cargo run -p thinclaw-client --example chat_loop -- "run the tests"
```

## Stability

The gateway routes this client binds are operator-facing infrastructure, **not
yet a versioned public API** — there is no `/v2` route namespace. Treat the crate
as pre-1.0 and expect breaking changes as the surface stabilizes. The `Unknown`
event fallback means additive server changes are non-breaking for consumers.
