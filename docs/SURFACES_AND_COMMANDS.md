# Surfaces And Commands

This document defines the shared user-facing vocabulary ThinClaw exposes across its conversational interfaces (CLI, TUI, WebUI, and eligible messaging channels).

> [!NOTE]
> For terminal commands used to configure and run the agent (e.g., `thinclaw run`, `thinclaw config`), see the [CLI Reference](CLI_REFERENCE.md).

## Shared Slash Commands

The following commands can be typed directly into the agent's chat input:

- `/help` — Show the list of available commands and current context.
- `/status` — Show system health, active providers, and current configuration.
- `/context` — Show what the agent currently sees in its recent context window.
- `/model` — View or switch the active LLM provider for the current session.
- `/compress` (or `/compact`) — Manually compress the current session memory to save tokens.
- `/identity` — Show current operator/household identity context.
- `/memory` — Query or manage the persistent workspace memory.
- `/personality` (or `/vibe`) — Change the agent's behavior and instructions for the current session.
- `/skills` — List or manage the agent's active skills.
- `/heartbeat` — Trigger a proactive system check and background routine pass. Heartbeat routines honor a `target` (`none` suppresses delivery; `<channel>` routes the summary to that channel — light-context heartbeats broadcast it via the channel forwarder) and an `include_reasoning` knob.
- `/summarize` — Request a summary of the current session.
- `/suggest` — Ask the agent to suggest next steps based on the current context.
- `/rollback` — Filesystem-only checkpoint family (list/diff/restore shadow-git snapshots).
- `/rewind [n|list]` — Unified rewind: `/rewind list` is a dry run showing conversation rewind points and turn-tagged filesystem checkpoints; `/rewind <n>` restores **both** the conversation (to the start of turn `n`) and the working files (to that turn's checkpoint) in one step.
- `/plan [on|off]` — Toggle plan mode. While on, the agent investigates with read-only tools and proposes a numbered plan; every state-changing tool it calls pauses for operator approval before running. Survives restart.
- `/undo` — Undo the last turn in a thread (also exposed in ThinClaw Desktop as the `thinclaw_undo` command + a cockpit toolbar button).
- `/redo` — Redo a previously undone turn (desktop: `thinclaw_redo` command + toolbar button).

Local clients (CLI, TUI, WebUI) may add extra client-specific controls such as:
- `/skin <name>` — Change the interface visual theme (e.g., `/skin midnight`).
- `/think` — Force the agent to perform an explicit reasoning step.
- Shell escapes (e.g., `!ls`) if enabled by sandbox policy.

However, the core commands above form the baseline shared vocabulary across all surfaces.

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

## REPL Multi-line Input

The REPL channel supports multi-line input via two continuation mechanisms:

- **Backslash continuation** — End a line with `\` to request more input on the next line.
- **Fenced code blocks** — An odd number of triple-backtick (`\`\`\``) markers keeps the input open until the block is closed.

## Surface Expectations

- CLI and TUI should expose the same core command names.
- WebUI settings and copy should refer to `personality_pack`, `agent.name`, and shared skin vocabulary.
- Channels should inherit the same mental model even when the transport cannot expose every local-only command.

## Design Rule

Add new commands once in the shared catalog first, then mirror them into surface-specific help and UI affordances.
