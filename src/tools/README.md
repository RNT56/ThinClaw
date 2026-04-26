# Tool System

This document is the maintainer-facing guide to ThinClaw's tool system.

## Core Split

ThinClaw has three related but distinct extension surfaces:

- **Built-in tools** implemented in Rust inside the core runtime
- **WASM tools** loaded in the host-managed WASM runtime
- **MCP servers** connected as external tool providers

Do not document or implement those as if they were the same thing.

## Built-In Tools

Add a built-in Rust tool when the capability belongs inside the core runtime.

1. Create `src/tools/builtin/my_tool.rs`
2. Implement the `Tool` trait
3. Export it from `src/tools/builtin/mod.rs`
4. Register it in the tool registry
5. Add tests

## WASM Tools

WASM tools are the preferred extension path when you want:

- sandboxed execution
- explicit capabilities
- host-boundary secret injection
- package-style deployment

Typical workflow:

1. Create a crate in `tools-src/<name>/`
2. Implement the `wit/tool.wit` interface
3. Add a `<name>.capabilities.json`
4. Build the component
5. Install it with `thinclaw tool install ...`

## MCP Servers

MCP servers are a different path.

- They are not installed with `thinclaw tool install`.
- They are managed with `thinclaw mcp ...`, especially:
- `thinclaw mcp server ...` for server lifecycle and auth
- `thinclaw mcp resource ...` for live resource inspection
- `thinclaw mcp prompt ...` for prompt browsing and retrieval
- `thinclaw mcp root ...` for roots grants
- `thinclaw mcp log ...` for MCP logging level control
- They are operator-trusted external processes or remote services, not sandboxed guest code.

ThinClaw's MCP client now supports live approval flows for server-initiated `sampling/createMessage` and `elicitation/create` requests across the CLI/runtime/WebUI surfaces, and long-lived clients reload persisted roots grants when answering `roots/list`.

Use MCP when ecosystem leverage is more important than the stronger isolation ThinClaw provides for WASM tools.

## Auth Guidance

- `thinclaw tool auth <tool>` is the CLI surface for WASM tool authentication.
- `thinclaw mcp auth <server>` is the CLI surface for MCP authentication.
- Tool docs should not invent a generic top-level `thinclaw auth ...` command.

## Documentation Rules

- Keep service-specific auth and endpoint details in tool docs or capabilities, not in the core runtime docs.
- Keep the trust-boundary distinction obvious: WASM is sandboxed, MCP is operator-trusted.
- When updating command names or auth flows, update the relevant tool docs in the same change.

## Related Docs

- [Extension System](../../docs/EXTENSION_SYSTEM.md)
- [Tool Guides](../../tools-docs/README.md)
