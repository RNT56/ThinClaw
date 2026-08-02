# Surfaces And Commands

This document defines the shared user-facing vocabulary ThinClaw exposes across its conversational interfaces (CLI, TUI, WebUI, and eligible messaging channels).

> [!NOTE]
> For terminal commands used to configure and run the agent (e.g., `thinclaw run`, `thinclaw config`), see the [CLI Reference](CLI_REFERENCE.md).

## Shared Slash Commands

The executable registry in `thinclaw-types` is the source of truth for routing,
help, and completion. Common commands include `/help`, `/status`, `/context`,
`/model`, `/tools [NAME|--all]`, `/rollback`, `/rewind`, `/plan`, `/undo`,
`/redo`, `/compress` (`/compact`), `/clear`, `/interrupt` (`/stop`), `/new`
(`/reset`), `/thread`, `/resume`, `/identity`, `/personality` (`/vibe`),
`/memory`, `/heartbeat`, `/summarize` (`/summary`), `/suggest`, `/skills`,
`/restart`, `/job`, and `/quit` (`/exit`, `/shutdown`).

`/job` is the canonical job family. The legacy `/create`, `/list`, `/jobs`,
`/cancel`, `/status ID`, and `/help ID` spellings remain hidden compatibility
routes; bare `/status` and `/help` retain their local meanings.

REPL and TUI handle `/help`, `/status`, `/tools`, `/debug`, `/skin`, `/cls`,
and quitting locally when declared by the registry. TUI additionally owns
`/back`, `/top`, and `/bottom`; `/back` closes a view and never deletes a
transcript entry. Agent-message routes are declared separately, so local-only
commands cannot be submitted as remote system commands.

Unimplemented reasoning-view toggles and raw `!command` shell escapes are not
part of the command registry. They return a concise safety explanation and
never become chat input or launch a process. The registered agent tool
`agent_think` is a separate policy-controlled runtime capability.

## TUI Input Controls

The full-screen TUI uses `ratatui-textarea` for multi-line input:

| Key | Action |
|-----|--------|
| `Enter` | Submit (single-line) or insert newline (multi-line content) |
| `Alt+Enter` / `Shift+Enter` | Insert a newline (multi-line continuation) |
| `Ctrl+Enter` | Force-submit regardless of content |
| `Up` / `Down` | Browse input history (single-line) or move cursor (multi-line) |
| `Tab` | Autocomplete slash commands |
| `Ctrl+C` | Abort active stream, or double-tap to exit |
| `Ctrl+L` | Clear the chat area |
| `PageUp` / `PageDown` | Scroll the chat history |
| `Ctrl+B` | Close the active detail view; does nothing to transcript content when none is open |

The TUI hydrates the latest 100 messages from the runtime-selected durable
direct conversation. Scrolling upward pages older messages in stable 100-message
windows (maximum request size 500), deduplicated by durable message ID. Input
history is stored separately in an owner-private bounded local file.

## REPL Multi-line Input

The REPL channel supports multi-line input via two continuation mechanisms:

- **Backslash continuation** — End a line with `\` to request more input on the next line.
- **Fenced code blocks** — An odd number of triple-backtick (`\`\`\``) markers keeps the input open until the block is closed.

## Surface Expectations

- REPL and TUI help/completion are generated from the same executable registry.
- `/tools` and `/status` consume a complete sealed capability revision and
  replace it only with a newer whole revision.
- WebUI settings and copy should refer to `personality_pack`, `agent.name`, and shared skin vocabulary.
- Channels should inherit the same mental model even when the transport cannot expose every local-only command.

## Design Rule

Add new commands once in the shared catalog first, then mirror them into surface-specific help and UI affordances.
