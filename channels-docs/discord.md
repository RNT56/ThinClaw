# Discord Integration

Discord has two different documentation paths in ThinClaw:

- native Discord Gateway messaging
- packaged WASM Discord interactions

If you are reading about Discord in ThinClaw, make sure you know which path you mean.

## Native Discord Gateway

Use the native path when you want the persistent Gateway WebSocket runtime for normal message handling.

Characteristics:

- native Rust channel
- persistent Gateway connection
- REST API for outbound responses
- generated media delivery through Discord multipart file uploads
- appropriate when long-lived connection state matters

## Discord Interactions Package

Use the packaged path when you want webhook-style Discord interactions behavior.

Characteristics:

- WASM channel package
- webhook-driven interactions model
- generated media delivery through multipart webhook followups
- packaged and managed like other WASM channels

## Canonical Architecture Reference

Use [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md) for the authoritative explanation of why ThinClaw splits Discord this way.

## Operator Guidance

- choose the native Gateway path for persistent real-time messaging
- choose the packaged interactions path for slash-command/webhook flows
- do not assume a single Discord doc or a single Discord transport model covers both
