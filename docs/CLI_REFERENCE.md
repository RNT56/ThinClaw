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
  - `--profile remote`: Preselects the Remote / SSH Host lane for Mac Mini, VPS, and other SSH-managed hosts.
  - `--profile pi-os-lite-64`: Preselects the Raspberry Pi OS Lite headless remote lane. It writes runtime headless markers and keeps desktop autonomy off.
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
- `thinclaw doctor`: Probe external dependencies and validate the current configuration. On Linux, add `--profile server`, `--profile remote`, `--profile pi-os-lite-64`, `--profile desktop-linux`, `--profile desktop-gnome`, or `--profile all-features`.
- `thinclaw status`: Show system health and diagnostics. On Linux, the same `--profile` values summarize runtime readiness.
- `thinclaw service`: Manage the OS background service through launchd, systemd, or the Windows Service Control Manager.
  - `install`: Install ThinClaw as a system service.
  - `start`: Start the background service.
  - `stop`: Stop the background service.
  - `status`: Show service-manager status.
  - `uninstall`: Remove the installed service.
- `thinclaw completion --shell <SHELL>`: Generate shell completion scripts.

## Browser Automation

- `thinclaw browser check`: Check whether a supported browser binary is available.
- `thinclaw browser open <URL>`: Open a URL and extract page content.
  - `--format text|html|json`: Select output format.
  - `--wait <SECONDS>`: Wait before capture for JavaScript-heavy pages.
  - `--screenshot <PATH>`: Save a PNG screenshot.
- `thinclaw browser screenshot <URL>`: Capture a screenshot.
  - `--output <PATH>` or `-o <PATH>`: Output path.
  - `--width <PX>` / `--height <PX>`: Viewport size.
- `thinclaw browser links <URL>`: Extract links from a page.
  - `--external-only`: Show only external links.

Browser automation uses local Chrome-family browsers when available. For Docker
Chromium fallback and host prerequisites, see [EXTERNAL_DEPENDENCIES.md](EXTERNAL_DEPENDENCIES.md).

## Registry

- `thinclaw registry list`: List installable extensions.
  - `--kind tool|channel`: Filter by extension kind.
  - `--tag <TAG>`: Filter by tag.
  - `--verbose`: Show detailed registry entries.
- `thinclaw registry search <QUERY>`: Search by name, description, or keywords.
- `thinclaw registry info <NAME>`: Show details for an extension or bundle.
- `thinclaw registry install <NAME>`: Install an extension or bundle.
  - `--force`: Overwrite an existing install.
  - `--build`: Build from source instead of using a prebuilt artifact.
- `thinclaw registry install-defaults`: Install the recommended default bundle.
- `thinclaw registry remove <NAME>`: Remove an installed registry extension.

## Trajectory Archive

- `thinclaw trajectory stats`: Show archive statistics.
- `thinclaw trajectory export`: Export archived records.
  - `--format jsonl|json|sft|dpo`: Select export format.
  - `--output <PATH>` or `-o <PATH>`: Write to a file instead of stdout.

## Readiness Profiles

`thinclaw doctor` and `thinclaw status` accept the same Linux readiness profiles:

| Profile | Use |
|---|---|
| `server` | Generic Linux server, workstation, or laptop readiness |
| `remote` | Remote/headless gateway and service posture |
| `pi-os-lite-64` | Raspberry Pi OS Lite 64-bit readiness |
| `desktop-linux` | Linux desktop-autonomy readiness for supported X11 or Wayland desktop sessions |
| `desktop-gnome` | Compatibility alias for `desktop-linux` |
| `all-features` | Optional Linux feature and `--all-features` build readiness |

These profiles are Linux-specific probes. On non-Linux hosts, `doctor` and
`status` still report general runtime health where available.
