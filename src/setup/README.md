# Setup / Onboarding Specification

This is the code-adjacent specification for ThinClaw onboarding. If setup behavior changes in `src/setup/`, update this file in the same change.

For the canonical identity vocabulary used by setup, also keep [../../docs/IDENTITY_AND_PERSONALITY.md](../../docs/IDENTITY_AND_PERSONALITY.md) aligned.

## Scope

This document owns:

- first-run onboarding entry points
- bootstrap sequencing relevant to onboarding
- the current wizard shape and persistence behavior
- setup-specific invariants and operator expectations

This document does not own deployment or broader trust architecture. Use `docs/DEPLOYMENT.md` for deployment modes and `docs/IDENTITY_AND_PERSONALITY.md` for the current agent identity model.

## Entry Points

Explicit onboarding:

```bash
thinclaw onboard [--skip-auth] [--channels-only] [--guide[=<topic>]] [--ui auto|cli|tui]
```

Full reset:

```bash
thinclaw reset [--yes]
```

Implicit onboarding:

```bash
thinclaw
```

When run without a configured environment, ThinClaw checks whether onboarding is needed and launches the wizard automatically.

## First-Run Detection

Onboarding is triggered when ThinClaw does not have enough bootstrap state to continue normally.

High-level behavior:

- `.env` files are loaded first
- ThinClaw checks for an existing database configuration
- `ONBOARD_COMPLETED=true` suppresses repeat onboarding
- `--no-onboard` bypasses the auto-launch path

The exact entry logic lives in `main.rs` and the bootstrap helpers.

## Config Layers That Matter During Setup

Setup spans more than one persistence layer.

1. process environment
2. `./.env`
3. `~/.thinclaw/.env`
4. optional TOML overlay
5. injected or encrypted secrets
6. database-backed settings

This matters because ThinClaw must establish database and secret-handling state before later runtime settings can be resolved fully.

## Wizard Modes

### Default full wizard

This is the normal onboarding path. `--ui auto` is the default and prefers the
full-screen onboarding shell when ThinClaw is running in a compatible
interactive terminal. Operators can force `--ui cli` or `--ui tui`.

When the full onboarding wizard starts (CLI or TUI) and neither `--channels-only`
nor `--guide` is supplied, the first choice is now:

- `Quick Setup` for the reduced day-one path
- `Advanced Setup` for the existing topic-guided deeper configuration flow

This keeps Quick Setup short instead of expanding it with extra advanced steps.

Both CLI and TUI presentations now use the same Humanist Cockpit language:

- readiness is framed as launch readiness, not pass/fail setup
- follow-up work is captured explicitly instead of being silently implied
- the TUI shell is a presentation layer only; it still runs the same shared step plan and validation logic as CLI
- the active CLI skin now drives palette treatment across setup prompts and the onboarding TUI shell, so runtime and onboarding no longer drift visually

### `--skip-auth`

Skips credential collection while still keeping provider, model, and routing
review in the flow. Use this when credentials are injected externally and the
operator still wants to confirm the rest of the AI stack.

### `--channels-only`

Runs only the channel configuration path. This is the supported way to revisit channel setup without re-running the rest of onboarding.

### `--guide [<topic>]`

Opens the guided Advanced Setup lane directly from CLI or TUI without showing
the Quick/Advanced selector first.

- `thinclaw onboard --guide` opens the guided topic menu
- `thinclaw onboard --guide ai` jumps directly to AI & Models
- Valid topics: `menu`, `ai`, `channels`, `agent`, `tools`, `automation`, `runtime`

### `reset`

Runs a destructive reset intended for recovery or clean-room re-onboarding. The command:

- clears ThinClaw-owned state from the configured database backend
- removes the local `~/.thinclaw/` runtime directory, including `.env`, tools, channels, skills, logs, cached media, and local libSQL files
- deletes ThinClaw-managed OS secure-store entries such as the master key and stored provider API keys

It does not uninstall the ThinClaw binary or remove launchd, systemd, or Windows service definitions. Operators should stop any running ThinClaw service before invoking the reset so state is not recreated mid-wipe.

### Profile Lanes

The Profile step currently offers five onboarding lanes:

- `Balanced` for the standard first-run path
- `Local & Private` for a local-first, lower-dependency setup
- `Builder & Coding` for stronger planning, routing, and tool-heavy work
- `Channel-First` for messaging reachability and notification routing
- `Custom / Advanced` for a neutral baseline with minimal profile-driven defaults

`Custom / Advanced` does not add a different step plan. It runs the same wizard,
but avoids the opinionated profile presets that the other lanes apply after the
database step.

## Current Wizard Shape

The current onboarding flow is phase-based and shared by both the CLI wizard
and the onboarding TUI shell, but it is not one single 26-step path anymore.

### Quick Setup

Quick Setup is the default reduced path. It currently includes:

1. Choose Your Cockpit Skin
2. Primary Model Provider
3. Advisor Model (Primary)
4. Agent Name & Personality
5. Primary Channel
6. Channel Verification
7. Autonomy Level
8. Worker Sandbox
9. Coding Workers
10. Finish

The quick path also applies the existing auto-configured runtime defaults and
quick notification defaults between those visible steps, which is why the user
experience is shorter than the full catalog of onboarding topics.

### Advanced Setup

Advanced Setup is the topic-guided lane. It opens the existing guided topic
menu and then builds a focused plan around one topic:

- AI & Models
- Channels & Notifications
- Agent & Experience
- Tools & Safety
- Automation & Skills
- Runtime & Diagnostics

Each topic-specific plan still ends with Finish.

### Channels-only

`--channels-only` runs only the Channel Configuration, Channel Verification, and
Finish parts of the plan.

### Phase Catalog

Across Quick Setup and guided Advanced Setup, the available operator-facing
phases remain:

- Skin & Profile
- Core Runtime
- AI Stack
- Identity & Presence
- Channels & Continuity
- Capabilities & Automation
- Experience & Operations
- Finish

If you change this order, branching, or phase shape in code, update this
section immediately.

## What Setup Persists Where

Bootstrap and runtime settings do not all live in one place.

- bootstrap values such as database connection details live in `~/.thinclaw/.env`
- encrypted credentials and related secure material use the secrets path
- broader runtime settings are persisted in the database-backed settings store

The design goal is simple: values needed before the database exists must be available earlier than values that can safely live in the database.

Telegram owner binding also seeds that owner into the Telegram pairing
allowlist so the first onboarding reply can continue immediately without a
second pairing round-trip. The onboarding path now waits for a fresh private
Telegram DM after the bot token is accepted and can fall back to manual numeric
owner ID entry if automatic capture does not complete.

## Operator Transparency Defaults

The setup/runtime defaults relevant to the new transparency surfaces are:

- `agent.subagent_transparency_level = "balanced"`
- `agent.main_tool_profile = "standard"`
- `agent.worker_tool_profile = "restricted"`
- `agent.subagent_tool_profile = "explicit_only"`
- `channels.telegram_subagent_session_mode = "temp_topic"`

These are runtime settings, not separate onboarding state. Setup may explain or expose them later, but they persist through the same database-backed settings path as the rest of operator preferences.

Profile behavior is intentionally asymmetric:

- `standard` keeps the main agent's full lane-eligible tool surface after policy and approval checks.
- `restricted` only grants safe read-only orchestrator tools implicitly; anything broader must be granted explicitly.
- `explicit_only` keeps delegated agents on coordination tools unless the caller grants additional tools.
- delegated workers and subagents inherit parent `allowed_tools` and `allowed_skills` ceilings unless a narrower request is supplied.

## Setup Invariants

- ThinClaw must know which database backend to use before the full runtime can resolve settings from the database.
- Secret handling must be established before provider credentials can be reused safely from encrypted storage.
- The wizard saves incrementally so failed later steps do not force operators to repeat earlier successful setup.
- Channel setup must remain reachable through `--channels-only`.
- The CLI wizard and onboarding TUI shell must use the same step plan and validation logic.
- The CLI wizard and onboarding TUI shell must keep the same readiness framing and follow-up semantics.
- Onboarding must clearly point to both local runtime entrypoints: `thinclaw` and `thinclaw tui`.
- Setup docs must not claim a different step or phase shape than the code.

## High-Value Setup Areas

### Database

The first step establishes the backend and makes later settings persistence possible.

### Security

The setup path decides how ThinClaw will obtain and protect secret material.

### Provider and model selection

This is where the operator establishes the default inference path and any related routing/fallback choices.

### Channels and extensions

These steps shape how the runtime will communicate and what additional capabilities it exposes.

### Sandbox, approvals, and observability

Later steps focus on trust boundaries, operator control, and day-two usability rather than just initial connectivity.

## Maintainer Rules

- Do not restate onboarding in multiple conflicting docs.
- Treat `src/setup/wizard/mod.rs` as the ultimate source of truth for step order and wizard branching.
- Update `docs/DEPLOYMENT.md`, `README.md`, and the identity/surface canonicals when setup-facing behavior changes the public story.
- If behavior changes affect parity-tracked functionality, update `FEATURE_PARITY.md` in the same branch.
