# ThinClaw Security And Trust

ThinClaw's safety model is layered, but not every surface has the same trust boundary.

This page is the short public-facing overview. For deeper network and runtime detail, use [../src/NETWORK_SECURITY.md](../src/NETWORK_SECURITY.md).

## The Core Idea

ThinClaw tries to keep control in the host runtime and make trust boundaries explicit.

That means:

- sandboxing where sandboxing makes sense
- host-managed secret handling
- policy and allowlist controls around tools and network access
- explicit operator trust decisions for external integrations

## Runtime Trust Boundaries

| Surface | Trust Model |
|---|---|
| Native runtime code | trusted host runtime |
| WASM tools | sandboxed and capability-scoped |
| WASM channels | package-based and host-managed |
| MCP servers | operator-trusted external processes or services |
| External providers and APIs | explicit data egress paths when configured |

Do not treat all integrations as if they had the same isolation guarantees.

## What ThinClaw Does

- Keeps secret injection at the host boundary rather than exposing raw values to WASM guests
- Uses policy and validation layers around dangerous tools and external content
- Supports network controls and allowlists
- Separates sandboxed extension paths from operator-trusted external paths
- Makes the gateway, channels, tools, and extension surfaces part of the security model

## What ThinClaw Does Not Claim

ThinClaw does not claim that:

- all configured integrations are sandboxed
- all data always stays local once you configure external providers or remote services
- MCP servers have the same trust profile as WASM tools

Those distinctions are part of the product design and should stay visible in the docs.

## Deep References

- [../src/NETWORK_SECURITY.md](../src/NETWORK_SECURITY.md)
- [EXTENSION_SYSTEM.md](EXTENSION_SYSTEM.md)
- [CHANNEL_ARCHITECTURE.md](CHANNEL_ARCHITECTURE.md)
- [../src/tools/README.md](../src/tools/README.md)
- [../src/setup/README.md](../src/setup/README.md)
