# Extension System

ThinClaw has three extension surfaces, and they do not share the same trust model:

- **WASM tools**: sandboxed tool modules loaded by ThinClaw
- **WASM channels**: sandboxed packaged channel modules loaded by ThinClaw
- **MCP servers**: operator-trusted external processes or remote services connected through the MCP client

This document is the canonical overview for those boundaries. For the public-facing security summary, see [SECURITY.md](SECURITY.md); for the deeper network model, see [../src/NETWORK_SECURITY.md](../src/NETWORK_SECURITY.md).

## The Three Extension Kinds

| Kind | Runtime Shape | Trust Boundary | Typical CLI Surface |
|---|---|---|---|
| WASM tool | Loaded inside ThinClaw's WASM runtime | Sandboxed, host-mediated | `thinclaw tool ...` |
| WASM channel | Loaded inside ThinClaw's WASM channel runtime | Sandboxed, host-mediated | registry / channel setup path |
| MCP server | External process or remote service | Operator-trusted, not sandboxed | `thinclaw mcp ...` |

## Do Not Blur These Flows

ThinClaw exposes several related but different operator paths:

- `thinclaw tool ...` manages WASM tools.
- `thinclaw mcp ...` manages MCP servers.
- `thinclaw registry ...` works with installable registry metadata for packaged artifacts.
- Conversational agent tools such as `tool_search` or `tool_install` are part of the runtime's agent-facing extension surface, not a replacement for the CLI reference.

## Trust Model

### WASM tools and channels

WASM components are the sandboxed extension path.

- The host runtime loads and mediates them.
- Capabilities are declared explicitly.
- Secret values are injected at the host boundary rather than exposed directly to guest code.

### MCP servers

MCP is not the sandboxed extension path.

- MCP servers run as external processes or remote services.
- They are configured and trusted by the operator.
- They can still be a great integration path, but they should be described as operator-trusted execution, not as isolated plugins.

## Installation And Auth Surface

Use the CLI that matches the extension kind:

| Need | Use |
|---|---|
| Install or inspect a WASM tool | `thinclaw tool ...` |
| Add, auth, test, or toggle an MCP server | `thinclaw mcp ...` |
| Work with registry-backed packaged artifacts | `thinclaw registry ...` |

Do not document these as interchangeable.

## MCP Operator Surfaces

The MCP surface is now split by task instead of a single flat command set:

- `thinclaw mcp server ...` for add, list, show, auth, test, remove, and toggle
- `thinclaw mcp resource ...` for listing and reading server resources
- `thinclaw mcp prompt ...` for listing prompts and fetching prompt payloads
- `thinclaw mcp root ...` for inspecting and changing roots grants
- `thinclaw mcp log ...` for inspecting and updating server log levels

The WebUI Extensions area also exposes a live MCP browser for server metadata, resources, prompts, OAuth discovery, and pending approval requests such as `sampling/createMessage` and `elicitation/create`.

Roots grants are treated as persisted server policy rather than a one-time startup snapshot. Long-lived MCP clients reload the configured grants when serving `roots/list`, so updated grants are visible to connected servers without requiring a full ThinClaw restart.

## Recommended Reading Order

- [Tool System](../src/tools/README.md)
- [Channel Architecture](CHANNEL_ARCHITECTURE.md)
- [Tool Guides](../tools-docs/README.md)
- [Deployment Guide](DEPLOYMENT.md)

## Summary

ThinClaw's extension story is one of deliberate separation:

- sandbox where sandbox makes sense
- native runtime where host integration matters
- MCP where ecosystem reach is worth an operator-trusted boundary

That separation is a feature, not a documentation inconvenience.
