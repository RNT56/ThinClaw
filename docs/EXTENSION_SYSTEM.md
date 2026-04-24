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

## Skill Provenance vs Trust

Skills now expose two different concepts on purpose:

- `trust`: the hard authority ceiling (`installed` vs `trusted`)
- `source_tier`: ecosystem/display provenance (`builtin`, `official`, `trusted`, `community`, `unvetted`)

Only `trust` participates in tool attenuation and safety decisions. `source_tier` is informational and should not be used as an authorization signal.

## User Tool Fast Path

ThinClaw now has a lightweight operator-trusted tool drop-in path at `~/.thinclaw/user-tools/`.

- Each `*.toml` file in that directory is discovered at startup.
- `kind = "shell"` wraps a command template and exposes placeholder parameters such as `{input}` as a real agent tool.
- `kind = "wasm"` loads a local WASM tool file through the existing WASM runtime instead of inventing a parallel sandbox.
- `kind = "mcp_proxy"` creates a narrow alias over an already-registered tool, which is useful for pre-binding or simplifying MCP-backed workflows.

This fast path is intentionally separate from `~/.thinclaw/tools/`, which remains the WASM tool install directory.

Shell user tools inherit the same workspace/safety defaults as the local dev-tool registration path:

- sandboxed workspaces keep a filesystem boundary
- project mode keeps the working directory pinned
- unrestricted mode remains unrestricted

Example:

```toml
name = "cargo-check-quick"
description = "Run cargo check in the current workspace"
kind = "shell"
command = "cargo check --message-format short"
approval = "auto_approved"
```

## Agent-Facing Memory Setup

The agent-facing learning surface now includes:

- `external_memory_setup`
- `external_memory_status`
- `external_memory_recall`
- `external_memory_off`

These tools are operator-trusted settings/configuration flows layered on top of the existing external-memory provider runtime.
