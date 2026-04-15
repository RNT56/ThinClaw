# WhatsApp Channel Package

> WhatsApp is a packaged WASM channel in ThinClaw's channel runtime.

## What This Page Covers

This page is about the operator-facing WhatsApp setup path. For the architecture behind the native/WASM split, use [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md).

## Runtime Shape

- channel type: WASM package
- transport shape: webhook-driven Cloud API delivery
- secret handling: access tokens and webhook secrets stay host-managed and are injected into the package runtime

## What WhatsApp Is Not

WhatsApp is not a native Rust channel in the current ThinClaw architecture. If another doc describes it that way, prefer the canonical architecture doc and the registry-backed package model.

## Setup Notes

- configure the WhatsApp Cloud API app and webhook route
- provide the required access token and verification secret through the host setup flow
- verify that inbound webhook callbacks reach the ThinClaw gateway/channel runtime

## Behavior Highlights

- package-based deployment under the WASM channel runtime
- webhook/event-driven runtime shape
- host-managed formatting, routing, and response delivery

## Related Docs

- [README.md](README.md)
- [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md)
- [../docs/DEPLOYMENT.md](../docs/DEPLOYMENT.md)
- [../docs/SURFACES_AND_COMMANDS.md](../docs/SURFACES_AND_COMMANDS.md)
