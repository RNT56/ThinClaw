# Slack Channel Package

> Slack is a packaged WASM channel in ThinClaw's channel runtime.

## What This Page Covers

This page is about the operator-facing Slack setup path. For the underlying architecture, use [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md).

## Runtime Shape

- channel type: WASM package
- transport shape: Slack Events API webhook path
- secret handling: credentials remain host-managed and are injected into the package runtime

## What Slack Is Not

Slack should not be documented as a native host channel in current ThinClaw docs. If another doc still frames it that way, prefer the architecture doc and the current registry-backed package model.

## Setup Notes

- create and install the Slack app
- configure the required Slack credentials for the channel package
- route Slack events into the ThinClaw channel runtime

## Behavior Highlights

- package-based Slack integration
- webhook/event-driven runtime shape
- host-managed formatting and response delivery

## Related Docs

- [README.md](README.md)
- [../docs/CHANNEL_ARCHITECTURE.md](../docs/CHANNEL_ARCHITECTURE.md)
- [../docs/DEPLOYMENT.md](../docs/DEPLOYMENT.md)
