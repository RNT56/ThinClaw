# REPL / CLI Channel

> Interactive terminal interface — the default way to talk to the agent.

## Overview

The REPL (Read-Eval-Print Loop) channel provides a terminal-based chat interface.
It's enabled by default and is the primary channel when running `thinclaw` interactively.

## Configuration

```bash
# Enabled by default. To disable:
CLI_ENABLED=false
```

## Features

- Interactive prompt with line editing
- Slash commands (`/help`, `/status`, `/restart`, `/tools`, etc.)
- Autocomplete for commands
- Multi-line input support
- Single-message mode: `thinclaw --message "What time is it?"`

## Usage

```bash
# Interactive mode (default)
thinclaw

# Single message mode (exits after response)
thinclaw --message "Summarize the project README"

# Disable CLI in favor of other channels
CLI_ENABLED=false thinclaw
```

## Notes

- No authentication required (local terminal access is the auth boundary)
- When other channels are active, the REPL runs alongside them
- The boot screen is displayed when REPL mode is active
- Not available when running as a headless daemon — use the Gateway or other channels instead
