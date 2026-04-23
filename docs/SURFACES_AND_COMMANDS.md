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
- `/heartbeat` — Trigger a proactive system check and background routine pass.
- `/summarize` — Request a summary of the current session.
- `/suggest` — Ask the agent to suggest next steps based on the current context.
- `/rollback` — Undo the last agent action or conversational turn.

Local clients (CLI, TUI, WebUI) may add extra client-specific controls such as:
- `/skin <name>` — Change the interface visual theme (e.g., `/skin midnight`).
- `/think` — Force the agent to perform an explicit reasoning step.
- Shell escapes (e.g., `!ls`) if enabled by sandbox policy.

However, the core commands above form the baseline shared vocabulary across all surfaces.

## Surface Expectations

- CLI and TUI should expose the same core command names.
- WebUI settings and copy should refer to `personality_pack`, `agent.name`, and shared skin vocabulary.
- Channels should inherit the same mental model even when the transport cannot expose every local-only command.

## Design Rule

Add new commands once in the shared catalog first, then mirror them into surface-specific help and UI affordances.
