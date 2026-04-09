# Tools / Extensions / MCP Audit

## Executive Summary

ThinClaw’s extension system is real and fairly coherent in code, but the documentation currently blurs three different surfaces:

- the agent-facing extension tools (`tool_search`, `tool_install`, `tool_auth`, `tool_activate`, `tool_list`, `tool_remove`)
- the CLI surfaces (`thinclaw tool ...`, `thinclaw mcp ...`, `thinclaw registry ...`)
- the per-tool / per-channel setup docs

That blur creates the main drift. The runtime actually separates `WasmTool`, `McpServer`, and `WasmChannel`, while the docs often collapse them into “tools” or “extensions” without saying which layer they belong to.

The biggest concrete issues are:

- `src/tools/README.md` says `thinclaw tool install` handles both WASM tools and MCP servers, but the CLI code splits those responsibilities.
- `tools-docs/README.md` claims to cover every WASM tool, but `tools-src/brave-search/` is missing from the index.
- Several tool docs use nonexistent commands like `thinclaw auth gmail`, `thinclaw auth google`, and `thinclaw secret set ...`.
- `channels-docs/README.md` is still using an older transport story that conflicts with the current hybrid channel architecture.

## Actual Tool and Extension Model

ThinClaw has three extension kinds in code:

- `McpServer` for hosted or local MCP servers
- `WasmTool` for sandboxed tool modules
- `WasmChannel` for hot-activated channel modules

That model is explicit in [src/extensions/mod.rs](/Users/vespian/coding/ThinClaw-main/src/extensions/mod.rs#L37), and the agent-facing layer preserves that three-way split in [src/tools/builtin/extension_tools.rs](/Users/vespian/coding/ThinClaw-main/src/tools/builtin/extension_tools.rs#L1).

The important runtime distinction is:

- `tool_search`, `tool_install`, `tool_auth`, `tool_activate`, `tool_list`, `tool_remove` are conversational agent tools that can handle all three kinds
- `thinclaw tool ...` is the CLI for WASM tools only
- `thinclaw mcp ...` is the CLI for MCP servers
- `thinclaw registry ...` is the registry catalog for installing tool and channel manifests, not MCP servers

That split is visible in [src/cli/mod.rs](/Users/vespian/coding/ThinClaw-main/src/cli/mod.rs#L7), [src/cli/tool.rs](/Users/vespian/coding/ThinClaw-main/src/cli/tool.rs#L27), [src/cli/mcp.rs](/Users/vespian/coding/ThinClaw-main/src/cli/mcp.rs#L21), and [src/cli/registry.rs](/Users/vespian/coding/ThinClaw-main/src/cli/registry.rs#L9).

Within the registry system, manifests are only for `tool` and `channel` kinds. MCP servers are configured separately through the MCP config path and startup session manager, not via registry manifests. That matters because the docs currently imply a more unified install story than the CLI actually provides.

## Doc Accuracy and Drift

`[docs/EXTENSION_SYSTEM.md](/Users/vespian/coding/ThinClaw-main/docs/EXTENSION_SYSTEM.md#L3)` is the strongest overview doc in this area, but it still needs a sharper boundary statement between:

- runtime extension kinds
- CLI surfaces
- registry/catalog installation
- conversational agent tools

Its current high-level description is good, but the reader can still walk away thinking `tool install` is the universal install command. It is not.

`[src/tools/README.md](/Users/vespian/coding/ThinClaw-main/src/tools/README.md#L105)` is accurate about WASM tool principles, but the line that says both WASM tools and MCP servers are first-class in `thinclaw tool install` is wrong. The real split is `thinclaw tool install` for WASM tools and `thinclaw mcp add` / `thinclaw mcp auth` for MCP servers.

`[tools-docs/README.md](/Users/vespian/coding/ThinClaw-main/tools-docs/README.md#L3)` is overstating coverage. It says the directory documents every WASM tool, but `tools-src/brave-search/` exists and is not listed. The same file also points users at `thinclaw auth ...`, which is not a CLI command.

`[tools-src/TOOLS.md](/Users/vespian/coding/ThinClaw-main/tools-src/TOOLS.md#L2)` reads like a brainstorming / roadmap scratchpad, not a current reference. It includes future or unsupported items such as Google Cloud, WhatsApp, Signal, and Uber. This should not be presented as active tool documentation.

`[channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md#L20)` conflicts with the current channel architecture story. It still presents Telegram, Slack, and Discord in a transport-centric way that does not match the hybrid native/WASM model described elsewhere in the repo.

## CLI / Auth / Install Mismatches

There is no top-level `thinclaw auth` command in the CLI. The actual commands are `thinclaw tool auth` and `thinclaw mcp auth`, as defined in [src/cli/mod.rs](/Users/vespian/coding/ThinClaw-main/src/cli/mod.rs#L7), [src/cli/tool.rs](/Users/vespian/coding/ThinClaw-main/src/cli/tool.rs#L90), and [src/cli/mcp.rs](/Users/vespian/coding/ThinClaw-main/src/cli/mcp.rs#L78).

That makes these docs stale:

- [tools-docs/gmail.md](/Users/vespian/coding/ThinClaw-main/tools-docs/gmail.md#L12)
- [tools-docs/google-calendar.md](/Users/vespian/coding/ThinClaw-main/tools-docs/google-calendar.md#L9)
- [tools-docs/google-docs.md](/Users/vespian/coding/ThinClaw-main/tools-docs/google-docs.md#L9)
- [tools-docs/google-drive.md](/Users/vespian/coding/ThinClaw-main/tools-docs/google-drive.md#L9)
- [tools-docs/google-sheets.md](/Users/vespian/coding/ThinClaw-main/tools-docs/google-sheets.md#L9)
- [tools-docs/google-slides.md](/Users/vespian/coding/ThinClaw-main/tools-docs/google-slides.md#L9)

Those docs should either point at `thinclaw tool auth <tool>` or, where appropriate, explain that a token is supplied through the setup wizard, secret store, or UI flow rather than a nonexistent top-level command.

Several docs also use `thinclaw secret set ...`, but there is no secret-set CLI in the current command surface. That appears in:

- [tools-docs/telegram.md](/Users/vespian/coding/ThinClaw-main/tools-docs/telegram.md#L19)
- [tools-docs/slack.md](/Users/vespian/coding/ThinClaw-main/tools-docs/slack.md#L33)
- [tools-docs/github.md](/Users/vespian/coding/ThinClaw-main/tools-docs/github.md#L33)
- [tools-src/brave-search/README.md](/Users/vespian/coding/ThinClaw-main/tools-src/brave-search/README.md#L18)

For the Google suite specifically, the registry shows shared auth on `google_oauth_token`, so the docs should say “authenticate once with a Google tool” rather than inventing a `thinclaw auth google` command. See [registry/tools/gmail.json](/Users/vespian/coding/ThinClaw-main/registry/tools/gmail.json) and [registry/tools/google-calendar.json](/Users/vespian/coding/ThinClaw-main/registry/tools/google-calendar.json).

For install flows, the docs need to distinguish three different actions:

- `thinclaw tool install` installs a WASM tool from source dir or `.wasm`
- `thinclaw registry install` installs registry-defined tool/channel bundles
- `thinclaw mcp add` registers an MCP server

That distinction is explicit in [src/cli/tool.rs](/Users/vespian/coding/ThinClaw-main/src/cli/tool.rs#L27), [src/cli/registry.rs](/Users/vespian/coding/ThinClaw-main/src/cli/registry.rs#L38), and [src/cli/mcp.rs](/Users/vespian/coding/ThinClaw-main/src/cli/mcp.rs#L23).

## Canonical Doc Recommendations

The cleanest structure is:

- keep [docs/EXTENSION_SYSTEM.md](/Users/vespian/coding/ThinClaw-main/docs/EXTENSION_SYSTEM.md#L1) as the canonical architecture overview for extensions
- keep [src/tools/README.md](/Users/vespian/coding/ThinClaw-main/src/tools/README.md#L1) as the developer-facing implementation guide
- keep [src/cli/mod.rs](/Users/vespian/coding/ThinClaw-main/src/cli/mod.rs#L1) and the CLI modules as the source of truth for command names
- make one user-facing “how to install/authenticate extensions” page that explains `tool`, `mcp`, and `registry` as separate flows
- make `tools-docs/` the per-tool user guide index only if it is rewritten to match the current CLI and runtime

I would also add one short canonical “auth and secrets” reference that explains:

- when to use `tool auth`
- when to use `mcp auth`
- when a token is stored in the secret store
- when setup happens in the wizard or UI instead of a CLI command

For channels, the current channel index should either be rewritten to align with the hybrid model in [docs/CHANNEL_ARCHITECTURE.md](/Users/vespian/coding/ThinClaw-main/docs/CHANNEL_ARCHITECTURE.md#L1) or merged into that canonical doc. Right now it reads like a parallel documentation branch.

## Rewrite / Merge / Archive Recommendations

- Rewrite `src/tools/README.md` so the WASM-vs-MCP guidance matches the CLI split.
- Rewrite `docs/EXTENSION_SYSTEM.md` to explicitly separate agent tools, CLI commands, registry installs, and MCP config.
- Rewrite `tools-docs/README.md` as a real current index, and add `brave-search` to it.
- Rewrite or merge the Google tool docs so they use `thinclaw tool auth <tool>` consistently.
- Rewrite `tools-docs/telegram.md` and `tools-docs/slack.md` so they stop referring to nonexistent secret-set commands and clearly identify whether they document a tool or a channel.
- Archive `tools-src/TOOLS.md` as scratch/brainstorming material, not current documentation.
- Merge or rewrite `channels-docs/README.md` into the canonical channel architecture docs so transport models do not drift again.

## Evidence Pointers

- [src/extensions/mod.rs](/Users/vespian/coding/ThinClaw-main/src/extensions/mod.rs#L37)
- [src/tools/builtin/extension_tools.rs](/Users/vespian/coding/ThinClaw-main/src/tools/builtin/extension_tools.rs#L1)
- [src/cli/mod.rs](/Users/vespian/coding/ThinClaw-main/src/cli/mod.rs#L7)
- [src/cli/tool.rs](/Users/vespian/coding/ThinClaw-main/src/cli/tool.rs#L27)
- [src/cli/mcp.rs](/Users/vespian/coding/ThinClaw-main/src/cli/mcp.rs#L21)
- [src/cli/registry.rs](/Users/vespian/coding/ThinClaw-main/src/cli/registry.rs#L9)
- [src/tools/README.md](/Users/vespian/coding/ThinClaw-main/src/tools/README.md#L105)
- [docs/EXTENSION_SYSTEM.md](/Users/vespian/coding/ThinClaw-main/docs/EXTENSION_SYSTEM.md#L3)
- [tools-docs/README.md](/Users/vespian/coding/ThinClaw-main/tools-docs/README.md#L3)
- [tools-docs/gmail.md](/Users/vespian/coding/ThinClaw-main/tools-docs/gmail.md#L12)
- [tools-docs/google-calendar.md](/Users/vespian/coding/ThinClaw-main/tools-docs/google-calendar.md#L9)
- [tools-docs/telegram.md](/Users/vespian/coding/ThinClaw-main/tools-docs/telegram.md#L19)
- [tools-src/brave-search/README.md](/Users/vespian/coding/ThinClaw-main/tools-src/brave-search/README.md#L1)
- [tools-src/TOOLS.md](/Users/vespian/coding/ThinClaw-main/tools-src/TOOLS.md#L2)
- [channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md#L20)
