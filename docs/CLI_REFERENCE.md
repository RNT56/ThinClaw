# CLI Reference

This document provides a comprehensive reference for all `thinclaw` terminal commands.

> [!NOTE]
> For in-chat slash commands (like `/help` and `/personality`) used while talking to the agent, see [SURFACES_AND_COMMANDS.md](SURFACES_AND_COMMANDS.md).

## Global Flags

These flags are valid across the command tree:
- `--debug`: Enable verbose terminal logs for troubleshooting.
- `--output-format human|json|jsonl`: Select the presentation contract.
- `--color auto|always|never`: Select terminal color behavior (`NO_COLOR` is honored).
- `--quiet` / `--verbose`: Reduce or expand human diagnostics.
- `--config <PATH>`: Specify a custom configuration file path.

Runtime-only flags belong to `run`, `tui`, and `ask`: `--channels
none|configured|<csv>`, `--skip-setup-check`, and the hidden diagnostic
`--no-db`. Use `thinclaw ask TEXT` for one local turn or `thinclaw send TEXT`
to inject through a running web runtime.

## Core Runtime & Setup

- `thinclaw run` (or simply `thinclaw`): Starts the agent in standard mode. Background routines, channels, and the web gateway will be launched based on your configuration.
- `thinclaw tui`: Launches the agent inside the full-screen terminal UI.
- `thinclaw setup`: Configures ThinClaw and exits when the setup flow completes.
  - `--mode quick|advanced`: Selects recommended or comprehensive setup.
  - `--run`: Explicitly continues into the local runtime after apply.
  - `--ui cli|tui|auto`: Selects only the setup renderer.
  - `--profile remote`: Preselects the Remote / SSH Host lane for Mac Mini, VPS, and other SSH-managed hosts.
  - `--profile pi-os-lite-64`: Preselects the Raspberry Pi OS Lite headless remote lane. It writes runtime headless markers and keeps desktop autonomy off.
  - Other profile values: `balanced`, `local-private`, `builder-coding`, `channel-first`, and `custom`.
- `thinclaw setup edit <TOPIC>`: Revisits one focused setup area.
- `thinclaw setup reset`: Reviews and resets selected ThinClaw-owned state.
- `thinclaw runtime update`: Checks for updates and performs a self-update.

## Configuration & Identity

- `thinclaw config`: Manage configuration settings.
  - `init`: Generate a default `config.toml` file (default `~/.thinclaw/config.toml`; `--out <PATH>` selects a path and `--force` overwrites).
  - `list`: Show all current configurations (`--filter <PREFIX>` narrows to a setting prefix).
  - `get <KEY>`: Retrieve a specific configuration value.
  - `set <KEY> <VALUE>`: Update a configuration value.
  - `reset <KEY>`: Reset a setting to its default value.
  - `path`: Show the settings storage info.
- `thinclaw access identities`: Manage household actors and linked endpoints.
- `thinclaw config models`: Inspect available LLM models from your configured providers.
- `thinclaw config secrets`: Manage encrypted local secrets without printing values.
  - `status`: Show OS secure-store and env-fallback posture.
  - `list [--user <ID>]`: List secret metadata, versions, and usage counts.
  - `set <NAME> [--from-stdin|--from-env <VAR>|--from-file <FILE>] [--provider <SLUG>] [--user <ID>]`: Store or replace one secret without placing its value in argv.
  - `delete <NAME> [--user <ID>] [--yes]`: Delete one secret, with confirmation required for noninteractive use.
  - `rotate-master`: Generate a new OS secure-store master key, re-encrypt active v2 secrets, and advance the local key version.
- `thinclaw extensions channels`: Manage messaging channel configurations (Telegram, Discord, etc.).
  - `list`: Show configured native and WASM channel surfaces.
  - `info <NAME> [--variant <DRIVER>]`: Show channel-specific setup details.
  - `check-config <NAME> [--variant <DRIVER>]`: Check static local configuration without connecting.
  - `probe [<NAME>|--all]`: Run bounded, side-effect-minimized live health probes.
    Native APNs and browser-push endpoint registrations persist under `$THINCLAW_HOME/native-endpoints/` by default; override with `APNS_ENDPOINT_REGISTRY_PATH` or `BROWSER_PUSH_ENDPOINT_REGISTRY_PATH` when the service account needs a custom writable location.
- `thinclaw runtime web`: Manage the built-in web gateway.
  - `start [--port <PORT>] [--host <HOST>] [--foreground]`: Start the web gateway (daemonizes unless `--foreground`).
  - `stop`: Stop a running gateway.
  - `reload [--port <PORT>] [--host <HOST>] [--foreground]`: Restart or refresh the managed gateway process.
  - `status`: Show gateway status.
  - `access [--reveal-token --yes]`: Print credential-free access guidance by default; explicit reveal is confirmation-gated and TTY-only.
- `thinclaw access devices`: Manage paired mobile devices (pair, list, rename, revoke). Requires a running gateway and a resolvable `GATEWAY_AUTH_TOKEN` (from the environment or settings); commands use the same authenticated gateway client as `thinclaw send`.
  - `pair [--name <NAME>]`: Start a pairing session, render a terminal QR code and human-readable code, then poll until the pairing completes or expires. Pairing is how the iOS/mobile app links to this gateway — see [MOBILE_APP.md](MOBILE_APP.md).
  - `list [--json]`: Show a table of paired devices (id, name, platform, scopes, last seen, revoked status).
  - `rename <ID> <NAME>`: Rename a paired device by id (or an unambiguous id prefix).
  - `revoke <ID>`: Revoke a paired device by id (or an unambiguous id prefix), disconnecting any live sessions.

## Extensions & Tools

- `thinclaw extensions tools`: Manage local WASM tool extensions.
  - `install <PATH/URL>`: Install a new WASM tool.
  - `list`: List all installed WASM tools.
  - `remove <ID>`: Remove a WASM tool.
  - `info <NAME_OR_PATH>`: Inspect an installed tool or WASM artifact.
  - `auth <NAME>`: Start the tool's supported local authentication flow.
- `thinclaw extensions registry`: Browse and install extensions from the ThinClaw registry.
- `thinclaw extensions skills`: Inspect, audit, install, update, publish, remove, reload, and assign trust to runtime skills; manage catalogs with `taps`.
- `thinclaw extensions mcp`: Manage Model Context Protocol (MCP) servers. Grouped into five subcommand families:
  - `thinclaw extensions mcp server`: Manage MCP server registration, activation, and auth.
    - `add <NAME> [URL] [--command <CMD>] [--args <A,B>] [--env <K=V>] [--secret-env <K=SOURCE_ID>] [--client-id ...] [--auth-url ...] [--token-url ...] [--scopes ...] [--description ...]`: Add a server (HTTP via URL, or stdio via `--command`). Secret-shaped values are rejected from generic `--env`; bind authorized sources with `--secret-env`.
    - `remove <NAME>`: Remove a server.
    - `list [--verbose]`: Show configured servers.
    - `show <NAME>`: Show a single server's configuration.
    - `auth <NAME> [--user <ID>]`: Authenticate with a server (OAuth flow).
    - `test <NAME> [--user <ID>]`: Test connection to a server.
    - `toggle <NAME> [--enable|--disable]`: Enable or disable a server.
  - `thinclaw extensions mcp resource`: Browse MCP resources from a server.
    - `list`, `read`, `templates`.
  - `thinclaw extensions mcp prompt`: Browse MCP prompts from a server.
    - `list`, `get`.
  - `thinclaw extensions mcp root`: Inspect and manage roots grants for a server.
    - `list`, `grant`, `revoke`.
  - `thinclaw extensions mcp log`: Inspect and change MCP logging levels.
    - `show`, `set`.
- `thinclaw dev browser`: Launch or debug the headless Chrome browser automation.
- `thinclaw media comfy`: Manage and use ComfyUI media generation.
  - `health`: Check the configured ComfyUI server and object-info availability.
  - `hardware-check`: Print local hardware suitability information for local generation.
  - `setup --gpu cpu|nvidia|amd|m-series`: Install ComfyUI through `comfy-cli`.
  - `launch`: Launch local ComfyUI through `comfy-cli`.
  - `stop`: Stop local ComfyUI through `comfy-cli`.
  - `list-workflows`: List bundled workflow names.
  - `check-deps <WORKFLOW>`: Report missing models/custom nodes for a bundled or approved API-format workflow.
  - `generate <PROMPT>`: Generate media through the configured ComfyUI server.
    Common flags: `--workflow <NAME_OR_PATH>`, `--aspect-ratio square|wide|portrait`, `--negative-prompt <TEXT>`, `--seed <N>`, `--width <PX>`, `--height <PX>`, `--steps <N>`, `--cfg <N>`, `--model <NAME>`, `--input-image <PATH>`, `--mask-image <PATH>`, `--no-wait`.

ComfyUI generation is disabled until `comfyui.enabled = true` or
`COMFYUI_ENABLED=true` is set. See [COMFYUI_MEDIA_GENERATION.md](COMFYUI_MEDIA_GENERATION.md)
for configuration, local/cloud mode, and workflow security details.

## Memory & Sessions

- `thinclaw data memory`: Query and manage persistent workspace memory.
  - `search <QUERY>`: Search the vector database for memories.
  - `read <ID>`: Read a specific memory entry.
  - `write`: Manually inject a memory entry.
  - `tree`: Show the workspace directory tree.
  - `status`: Show workspace status (document count, index health).
- `thinclaw data conversations`: Manage active agent sessions.
  - `list`: View recent sessions.
  - `show <ID>`: View details of a specific session.
  - `prune`: Clean up old or inactive sessions.
  - `export`: Export a session transcript.
  - `search <QUERY>`: Search durable conversation messages.
  - `delete <ID>`: Preview or confirm deletion of one conversation.
- `thinclaw agents`: Manage agent workspaces.
  - `add`: Register a new agent workspace.
  - `list`: View available agents.
  - `remove`: Delete an agent workspace.
- `thinclaw data trajectories`: Export or inspect archived agent trajectories for analysis.
- `thinclaw data learning`: Inspect durable learning history, outcomes, feedback, proposals, rollback records, and external-memory provider status.

## Background Work & Operations

- `thinclaw automation routines`: Manage scheduled background routines.
- `thinclaw labs experiments`: Manage research automation (campaigns, providers, targets).
- `thinclaw automation projects`: Manage the GitHub repository project supervisor (default off until enabled in settings). See [Repo Project Supervisor](#repo-project-supervisor).
- `thinclaw data backup`: Export or restore an encrypted whole-agent backup. See [Backup & Restore](#backup--restore).
- `thinclaw send <TEXT>`: Inject a message through the authenticated running web runtime.
- `thinclaw access senders`: Approve or reject durable unknown-sender requests by stable request ID.
- `thinclaw runtime logs`: Query, tail, and filter system logs.
- `thinclaw doctor --readiness-profile <PROFILE>`: Run the complete bounded diagnostic report. Required failures exit `3`.
- `thinclaw status [--readiness-profile <PROFILE>] [--live]`: Show the fast static capability snapshot, or opt into bounded live probes.
- `thinclaw status tools [NAME] [--all] [--match <GLOB>]`: Inspect independently filtered compiled, configured, registered, dependency, exposure, approval, and health facts. Shared status flags parse on either side of `tools`.
- `thinclaw runtime service`: Manage the OS background service through launchd, systemd, or the Windows Service Control Manager.
  - `install`: Install ThinClaw as a system service.
  - `start`: Start the background service.
  - `stop`: Stop the background service.
  - `status`: Show service-manager status.
  - `uninstall`: Remove the installed service.
- `thinclaw completion --shell <SHELL>`: Generate shell completion scripts.

## Browser Automation

- `thinclaw dev browser check`: Check whether a supported browser binary is available.
- `thinclaw dev browser open <URL>`: Open a URL and extract page content.
  - `--format text|html|json`: Select output format.
  - `--wait <SECONDS>`: Wait before capture for JavaScript-heavy pages.
  - `--screenshot <PATH>`: Save a PNG screenshot.
- `thinclaw dev browser screenshot <URL>`: Capture a screenshot.
  - `--out <PATH>` or `-o <PATH>`: Output path.
  - `--width <PX>` / `--height <PX>`: Viewport size.
- `thinclaw dev browser links <URL>`: Extract links from a page.
  - `--external-only`: Show only external links.

Browser automation uses local Chrome-family browsers when available. For Docker
Chromium fallback and host prerequisites, see [EXTERNAL_DEPENDENCIES.md](EXTERNAL_DEPENDENCIES.md).

## Registry

- `thinclaw extensions registry list`: List installable extensions.
  - `--kind tool|channel`: Filter by extension kind.
  - `--tag <TAG>`: Filter by tag.
  - `--verbose`: Show detailed registry entries.
- `thinclaw extensions registry search <QUERY>`: Search by name, description, or keywords.
- `thinclaw extensions registry info <NAME>`: Show details for an extension or bundle.
- `thinclaw extensions registry install <NAME>`: Install an extension or bundle.
  - `--force`: Overwrite an existing install.
  - `--build`: Build from source instead of using a prebuilt artifact.
- `thinclaw extensions registry install-defaults`: Install the recommended default bundle.
- `thinclaw extensions registry remove <NAME>`: Remove an installed registry extension.
- `thinclaw extensions registry check <NAME|BUNDLE>`: Validate registry manifests and capabilities without installing. This checks that setup secrets declared by channel/tool capabilities are also represented in registry auth metadata.

## Repo Project Supervisor

The `thinclaw automation projects` commands manage the GitHub repository project
supervisor. The supervisor is default off and stays inactive until enabled in
settings (`thinclaw automation projects setup --enable`). Commands talk directly to the
database and secrets store, the same layer the desktop commands and gateway
handlers use.

- `thinclaw automation projects list`: List all repository projects.
- `thinclaw automation projects show <PROJECT_ID>`: Show one project's full status (backlog, workers, PRs, merge gates).
- `thinclaw automation projects status`: Show supervisor setup readiness (feature flag, credentials, policy).
- `thinclaw automation projects setup`: Enable and configure the supervisor (writes settings).
  - `--enable` / `--disable`: Enable or disable the supervisor.
  - `--app-id <ID>`: GitHub App id.
  - `--installation-id <ID>`: GitHub App installation id.
  - `--private-key-secret <NAME>`: Name of the secret holding the GitHub App PEM private key.
  - `--webhook-secret-secret <NAME>`: Name of the secret holding the GitHub webhook secret.
  - `--app-slug <SLUG>`: Public GitHub App slug (used to build the install URL).
  - `--default-coding-backend <BACKEND>`: Default coding backend for new tasks.
  - `--default-write-mode <MODE>`: Default write mode for new projects (`read_only_clone`, `fork_pr`, `maintainer_branch_pr`, `maintainer_auto_merge`).
  - `--auto-merge <BOOL>`: Whether to auto-merge passing PRs.
  - `--watchdog-interval-secs <SECS>`: Reconcile/watchdog interval in seconds.
- `thinclaw automation projects set-credential <SLOT>`: Create and bind a purpose-scoped GitHub credential source through masked local input.
  - `--from-stdin`, `--from-env <VAR>`, or `--from-file <FILE>`: Select secure ingress; when omitted, a TTY prompts without echo. Credential values never belong in argv.
- `thinclaw automation projects create`: Create a project and enroll its first repository.
  - `--name <NAME>`: Project name.
  - `--repo-url <URL>`: Repository URL.
  - `--default-branch <BRANCH>`: Default branch.
  - `--description <TEXT>`: Project description.
  - `--write-mode <MODE>`: Write mode for the project (`read_only_clone`, `fork_pr`, `maintainer_branch_pr`, `maintainer_auto_merge`).
  - `--fork-owner <OWNER>`: Fork owner (for `fork_pr` mode).
  - `--fork-repo <REPO>`: Fork repository name (for `fork_pr` mode).
- `thinclaw automation projects enroll <PROJECT_ID>`: Enroll an additional repository into a project.
  - `--repo-url <URL>`: Repository URL.
  - `--default-branch <BRANCH>`: Default branch.
  - `--fork-owner <OWNER>`: Fork owner (for `fork_pr` mode).
  - `--fork-repo <REPO>`: Fork repository name (for `fork_pr` mode).
- `thinclaw automation projects repos`: List the GitHub repositories the connected credential can act on, marking which are already enrolled.
- `thinclaw automation projects connect [REPOS...]`: Bring repositories under supervision (a project is created for each). Pass one or more `owner/repo`, or `--all` for every accessible repo.
  - `--all`: Connect every repository the credential can access.
  - `--write-mode <MODE>`: Write mode for the new projects (`read_only_clone`, `fork_pr`, `maintainer_branch_pr`, `maintainer_auto_merge`).
  - `--fork-owner <OWNER>`: Fork owner for enrolled repositories (for `fork_pr` mode).
  - `--fork-repo <REPO>`: Fork repository name (for `fork_pr` mode).
- `thinclaw automation projects start <PROJECT_ID>`: Start a project.
- `thinclaw automation projects pause <PROJECT_ID>`: Pause a project.
- `thinclaw automation projects resume <PROJECT_ID>`: Resume a paused project.
- `thinclaw automation projects cancel <PROJECT_ID>`: Cancel a project.
- `thinclaw automation projects events <PROJECT_ID>`: List recent project events.
  - `--limit <N>`: Maximum number of events to show (default 20).

## Backup & Restore

`thinclaw data backup` produces and restores a single **encrypted bundle** of the
whole agent: the ThinClaw home directory (config, `SOUL.md`, skills, channels,
personality) as a file tree, plus a database payload. The bundle is portable —
it decrypts with the passphrase alone on any machine — and sealed with
scrypt + XChaCha20-Poly1305.

The passphrase comes from a private regular file (`--passphrase-file`),
`THINCLAW_BACKUP_PASSPHRASE`, or a masked TTY prompt. Volatile and secret
paths — `logs/`, `.env`, pid files, capture dirs (`screenshots/`, `camera/`,
`audio/`), and the live database file — are excluded from the file tree.
**Secrets are not exported**; they live in the OS keychain / secrets store and
must be re-provisioned after a restore.

- `thinclaw data backup export`: Write an encrypted bundle.
  - `--out <PATH>` or `-o <PATH>`: Bundle path (default `./thinclaw-backup-<timestamp>.tclaw`, written `0600`).
  - `--passphrase-file <PATH>`: Read from an absolute, owner-only private regular file.
  - `--no-database`: Config + workspace files only (skip the database section).
  - `--require-database`: Fail rather than producing a partial bundle when database export is unavailable.
  - The database section is a WAL-checkpointed libSQL snapshot, or a
    `pg_dump --format=custom` archive for Postgres. If neither is available
    (e.g. `pg_dump` not installed) the bundle is still written without it.
- `thinclaw data backup inspect <BUNDLE>`: Print a bundle's manifest (producer, timestamp, sections) without changing anything.
- `thinclaw data backup import <BUNDLE>`: Restore config + workspace files into the ThinClaw home.
  - Without `--yes` it is a dry run that only prints the manifest.
  - `--yes`: Confirm overwriting config + workspace files.
  - `--restore-database`: Also restore the database. For libSQL this overwrites
    the local database file (**ThinClaw must be stopped**); for Postgres the
    exact `pg_restore` command is printed rather than run, and the dump is
    written next to the bundle as `<bundle>.database-dump`.

## Trajectory Archive

- `thinclaw data trajectories stats`: Show archive statistics.
- `thinclaw data trajectories export`: Export archived records.
  - `--format jsonl|json|sft|dpo`: Select export format.
  - `--out <PATH>` or `-o <PATH>`: Write to a file instead of stdout.

## Readiness Profiles

`thinclaw doctor` and `thinclaw status` accept the same platform-neutral readiness profiles through `--readiness-profile`:

| Profile | Use |
|---|---|
| `server` | Generic server, workstation, or laptop readiness |
| `remote` | Remote/headless gateway and service posture |
| `pi-os-lite-64` | Raspberry Pi OS Lite 64-bit readiness |
| `desktop` | Current-platform desktop and autonomy prerequisites |
| `all-features` | Optional feature and all-capability build readiness |

The hidden compatibility values `desktop-linux` and `desktop-gnome` map to
`desktop` during the migration window. Unsupported Pi profiles remain parseable
and report a typed required platform failure.
