# Surfaces And Commands

This document defines the shared user-facing vocabulary ThinClaw should expose across CLI, TUI, WebUI, and eligible messaging surfaces.

## Shared Commands

- `/help`
- `/status`
- `/context`
- `/model`
- `/compress` (`/compact` alias)
- `/identity`
- `/memory`
- `/personality` (`/vibe` alias)
- `/skills`
- `/heartbeat`
- `/summarize`
- `/suggest`
- `/rollback`

Local clients may add extra controls such as `/skin`, `/think`, or shell escapes, but the commands above are the baseline shared vocabulary.

## Surface Expectations

- CLI and TUI should expose the same core command names.
- WebUI settings and copy should refer to `personality_pack`, `agent.name`, and shared skin vocabulary.
- Channels should inherit the same mental model even when the transport cannot expose every local-only command.

## Design Rule

Add new commands once in the shared catalog first, then mirror them into surface-specific help and UI affordances.
