# ThinClaw Tool Guides

This directory contains operator-facing guides for ThinClaw's packaged tools.

Use these pages for tool-specific setup and permissions. For the architecture and trust model behind WASM tools, WASM channels, MCP servers, and registry installs, start with [../docs/EXTENSION_SYSTEM.md](../docs/EXTENSION_SYSTEM.md).

For the shared identity and command vocabulary that operator-facing tools should assume, also use:

- [../docs/IDENTITY_AND_PERSONALITY.md](../docs/IDENTITY_AND_PERSONALITY.md)
- [../docs/MEMORY_AND_GROWTH.md](../docs/MEMORY_AND_GROWTH.md)
- [../docs/SURFACES_AND_COMMANDS.md](../docs/SURFACES_AND_COMMANDS.md)

## Current Auth Vocabulary

Do not assume one generic auth flow for every tool.

- Use `thinclaw tool auth <tool>` when the tool exposes a CLI auth flow.
- Use the setup guide when a tool relies on manual token entry, workspace files, or deployment-specific secret handling.
- Use `thinclaw mcp ...` only for MCP servers, not for WASM tools.

## Tool Guides

| Tool | Auth Shape | Secret / Storage | Guide |
|---|---|---|---|
| GitHub | manual token / secret-entry flow | `github_token` | [github.md](github.md) |
| Notion | manual token / secret-entry flow | `notion_token` | [notion.md](notion.md) |
| Gmail | `thinclaw tool auth gmail` | `google_oauth_token` | [gmail.md](gmail.md) |
| Google Calendar | `thinclaw tool auth google-calendar` | `google_oauth_token` | [google-calendar.md](google-calendar.md) |
| Google Docs | `thinclaw tool auth google-docs` | `google_oauth_token` | [google-docs.md](google-docs.md) |
| Google Drive | `thinclaw tool auth google-drive` | `google_oauth_token` | [google-drive.md](google-drive.md) |
| Google Sheets | `thinclaw tool auth google-sheets` | `google_oauth_token` | [google-sheets.md](google-sheets.md) |
| Google Slides | `thinclaw tool auth google-slides` | `google_oauth_token` | [google-slides.md](google-slides.md) |
| Slack | `thinclaw tool auth slack-tool` or env-based auth | `slack_bot_token` | [slack.md](slack.md) |
| Telegram | workspace files + in-tool login flow | `telegram/api_id`, `telegram/api_hash`, `telegram/session.json` | [telegram.md](telegram.md) |
| Okta | `thinclaw tool auth okta` + workspace domain | `okta_oauth_token`, `okta/domain` | [okta.md](okta.md) |
| Brave Search | `thinclaw tool auth brave-search` | `brave_search_api_key` | [../tools-src/brave-search/README.md](../tools-src/brave-search/README.md) |
| ComfyUI media generation | built-in runtime tools + optional `comfy-cli` lifecycle | `comfy_cloud_api_key` for cloud mode | [comfyui.md](comfyui.md) |

## Notes

- All WASM tools run in the host-managed WASM runtime.
- Secret values are injected at the host boundary rather than exposed directly to WASM code.
- MCP servers are a separate extension path with a different trust model.
- Generated media from `image_generate` and `comfy_run_workflow` is eligible for
  automatic final-response attachment. The unified `send_message` tool can send
  generated files explicitly with an `attachments` array containing `file_path`,
  optional `filename`, and optional `mime_type`.

## Built-In Runtime Surfaces

Not every operator-facing capability is a packaged WASM tool. For first-party runtime surfaces that now sit alongside the tool catalog, use:

- [../docs/MEMORY_AND_GROWTH.md](../docs/MEMORY_AND_GROWTH.md) for memory, recall, learning, and prompt mutation flows
- [../docs/SURFACES_AND_COMMANDS.md](../docs/SURFACES_AND_COMMANDS.md) for the shared `/personality`, `/compress`, `/skills`, and continuity command vocabulary
- [../docs/IDENTITY_AND_PERSONALITY.md](../docs/IDENTITY_AND_PERSONALITY.md) for base identity packs, overlays, and cross-surface naming
- [comfyui.md](comfyui.md) for the built-in ComfyUI `image_generate` tool family
