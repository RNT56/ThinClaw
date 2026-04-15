# REPL / CLI Channel

> Interactive terminal interface — the default way to talk to the agent.

## Overview

The REPL (Read-Eval-Print Loop) channel provides a terminal-based chat interface.
When ThinClaw is built with the CLI surface available, running `thinclaw`
opens the interactive prompt by default.

Availability follows the build profile. Source builds that omit the CLI surface
may not include this interactive mode.

## Features

- Interactive prompt with line editing
- Shared slash commands for identity, memory, and workflows (`/help`, `/identity`, `/memory`, `/personality`, `/compress`, `/skills`, `/rollback`, etc.)
- Autocomplete for commands
- Multi-line input support
- Single-message mode: `thinclaw --message "What time is it?"`

## Usage

```bash
# Interactive mode
thinclaw

# Single message mode (exits after response)
thinclaw --message "Summarize the project README"

# If your build omits the CLI surface, use the Gateway or another channel
```

## Notes

- No authentication required (local terminal access is the auth boundary)
- When other channels are active, the REPL runs alongside them
- The boot screen is displayed when REPL mode is active
- Not available when running as a headless daemon or a build without the CLI surface
- Primary command vocabulary follows the shared agent-first docs: `/compress` is preferred over `/compact`, and `/personality` is preferred over `/vibe`

## Related Docs

- [../docs/IDENTITY_AND_PERSONALITY.md](../docs/IDENTITY_AND_PERSONALITY.md)
- [../docs/MEMORY_AND_GROWTH.md](../docs/MEMORY_AND_GROWTH.md)
- [../docs/SURFACES_AND_COMMANDS.md](../docs/SURFACES_AND_COMMANDS.md)
