# Setup / Onboarding Specification

This is the code-adjacent specification for ThinClaw onboarding. If setup behavior changes in `src/setup/`, update this file in the same change.

## Scope

This document owns:

- first-run onboarding entry points
- bootstrap sequencing relevant to onboarding
- the current wizard shape and persistence behavior
- setup-specific invariants and operator expectations

This document does not own the broader runtime walkthrough. Use `Agent_flow.md` for boot/runtime flow and `docs/DEPLOYMENT.md` for deployment modes.

## Entry Points

Explicit onboarding:

```bash
thinclaw onboard [--skip-auth] [--channels-only]
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

This is the normal onboarding path.

### `--skip-auth`

Skips the provider-auth step when the operator intends to reuse an existing auth path or inject credentials separately.

### `--channels-only`

Runs only the channel configuration path. This is the supported way to revisit channel setup without re-running the rest of onboarding.

## Current Wizard Shape

The current wizard in `src/setup/wizard/mod.rs` runs 20 steps in the full path:

1. Database Connection
2. Security
3. Inference Provider
4. Model Selection
5. Smart Routing
6. Fallback Providers
7. Embeddings (Semantic Search)
8. Agent Identity
9. Timezone
10. Channel Configuration
11. Extensions
12. Local Tools & Docker Sandbox
13. Claude Code Sandbox
14. Tool Approval Mode
15. Routines (Scheduled Tasks)
16. Skills
17. Background Tasks
18. Notification Preferences
19. Web UI
20. Observability

If you change this order or count in code, update this section immediately.

## What Setup Persists Where

Bootstrap and runtime settings do not all live in one place.

- bootstrap values such as database connection details live in `~/.thinclaw/.env`
- encrypted credentials and related secure material use the secrets path
- broader runtime settings are persisted in the database-backed settings store

The design goal is simple: values needed before the database exists must be available earlier than values that can safely live in the database.

## Setup Invariants

- ThinClaw must know which database backend to use before the full runtime can resolve settings from the database.
- Secret handling must be established before provider credentials can be reused safely from encrypted storage.
- The wizard saves incrementally so failed later steps do not force operators to repeat earlier successful setup.
- Channel setup must remain reachable through `--channels-only`.
- Setup docs must not claim a different step count than the code.

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
- Update `docs/DEPLOYMENT.md`, `README.md`, and `Agent_flow.md` when setup-facing behavior changes the public story.
- If behavior changes affect parity-tracked functionality, update `FEATURE_PARITY.md` in the same branch.
