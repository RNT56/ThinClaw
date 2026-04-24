# CLI Reference

This document provides a comprehensive reference for all `thinclaw` terminal commands. 

> [!NOTE]
> For in-chat slash commands (like `/help` and `/personality`) used while talking to the agent, see [SURFACES_AND_COMMANDS.md](SURFACES_AND_COMMANDS.md).

## Global Flags

These flags can be appended to almost any command:
- `--debug`: Enable verbose terminal logs for troubleshooting.
- `--no-db`: Skip the database connection (primarily for testing).
- `--cli-only`: Run in interactive CLI mode only, disabling background channels.
- `--config <PATH>`: Specify a custom configuration file path.
- `--no-onboard`: Skip the first-run onboarding check.
- `--message <TEXT>` (or `-m`): Send a single message and immediately exit.

## Core Runtime & Setup

- `thinclaw run` (or simply `thinclaw`): Starts the agent in standard mode. Background routines, channels, and the web gateway will be launched based on your configuration.
- `thinclaw tui`: Launches the agent inside the full-screen terminal UI.
- `thinclaw onboard`: Launches the interactive onboarding wizard to configure the agent, set up the database, and link your first provider.
  - `--ui tui`: Forces the onboarding to run in full-screen mode.
  - `--profile remote`: Preselects the Remote / SSH Host lane for Raspberry Pi, Mac Mini, VPS, and other SSH-managed hosts.
  - Other profile values: `balanced`, `local-private`, `builder-coding`, `channel-first`, and `custom`.
- `thinclaw reset`: Completely wipes ThinClaw's local database and state so you can start fresh.
- `thinclaw update`: Checks for updates and performs a self-update.

## Configuration & Identity

- `thinclaw config`: Manage configuration settings.
  - `list`: Show all current configurations.
  - `get <KEY>`: Retrieve a specific configuration value.
  - `set <KEY> <VALUE>`: Update a configuration value.
- `thinclaw identity`: Manage household actors and linked endpoints.
- `thinclaw models`: Inspect available LLM models from your configured providers.
- `thinclaw secrets`: Manage encrypted local secrets without printing values.
  - `status`: Show OS secure-store and env-fallback posture.
  - `list [--user <ID>]`: List secret metadata, versions, and usage counts.
  - `set <NAME> [--value <VALUE>] [--provider <SLUG>] [--user <ID>]`: Store or replace one secret.
  - `delete <NAME> [--user <ID>]`: Delete one secret.
  - `rotate-master`: Generate a new OS secure-store master key, re-encrypt active v2 secrets, and advance the local key version.
- `thinclaw channels`: Manage messaging channel configurations (Telegram, Discord, etc.).
- `thinclaw gateway`: Manage the built-in web gateway settings.
  - `access [--show-token]`: Print the bind address, WebUI URL, token URL, SSH tunnel command, auth status, health status, and service-safe warnings.

## Extensions & Tools

- `thinclaw tool`: Manage local WASM tool extensions.
  - `install <PATH/URL>`: Install a new WASM tool.
  - `list`: List all installed WASM tools.
  - `remove <ID>`: Remove a WASM tool.
- `thinclaw registry`: Browse and install extensions from the ThinClaw registry.
- `thinclaw mcp`: Manage Model Context Protocol (MCP) servers.
  - `add`: Add a new MCP server.
  - `auth`: Configure authentication for an MCP server.
  - `list`: Show configured MCP servers.
  - `test`: Run a diagnostic test on an MCP server.
- `thinclaw browser`: Launch or debug the headless Chrome browser automation.

## Memory & Sessions

- `thinclaw memory`: Query and manage persistent workspace memory.
  - `search <QUERY>`: Search the vector database for memories.
  - `read <ID>`: Read a specific memory entry.
  - `write`: Manually inject a memory entry.
- `thinclaw sessions`: Manage active agent sessions.
  - `list`: View recent sessions.
  - `show <ID>`: View details of a specific session.
  - `prune`: Clean up old or inactive sessions.
- `thinclaw agents`: Manage agent workspaces.
  - `add`: Register a new agent workspace.
  - `list`: View available agents.
  - `remove`: Delete an agent workspace.
- `thinclaw trajectory`: Export or inspect archived agent trajectories for analysis.

## Background Work & Operations

- `thinclaw cron`: Manage scheduled background routines.
- `thinclaw experiments`: Manage research automation (campaigns, providers, targets).
- `thinclaw message`: Send a message to the agent directly from the CLI without starting the interactive prompt.
- `thinclaw pairing`: DM pairing logic to approve inbound requests from unknown senders on supported channels.
- `thinclaw logs`: Query, tail, and filter system logs.
- `thinclaw doctor`: Probe external dependencies and validate the current configuration. On Linux, add `--profile server`, `--profile remote`, `--profile pi-os-lite-64`, `--profile desktop-gnome`, or `--profile all-features`.
- `thinclaw status`: Show system health and diagnostics. On Linux, the same `--profile` values summarize runtime readiness.
- `thinclaw service`: Manage the OS background service through launchd, systemd, or the Windows Service Control Manager.
  - `install`: Install ThinClaw as a system service.
  - `start`: Start the background service.
  - `stop`: Stop the background service.
