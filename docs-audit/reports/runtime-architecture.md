# Runtime Architecture Report

## Executive Summary

ThinClaw boots in a clear sequence: load env, decide whether onboarding is needed, resolve config, build core components through `AppBuilder`, seed workspace identity files, wire channels and gateways, then enter the main agent loop. The code is more coherent than the docs: the main drift is in step counts, channel taxonomy, and how much startup detail is duplicated across maintainer docs.

The right canonical architecture story should be code-backed and audience-neutral. `README.md` should stay high-level, `CLAUDE.md` should be a contributor guide with philosophy, and one canonical runtime overview should own the boot/runtime narrative instead of repeating it in several places.

## Actual Runtime Model

- `main.rs` first loads `./.env` and `~/.thinclaw/.env`, then checks `check_onboard_needed()` unless `--no-onboard` is set. Only after that does it resolve `Config::from_env_with_toml()` and initialize logging, so onboarding is gated by database presence plus `ONBOARD_COMPLETED`, not by a vague “first run” flag.
- `AppBuilder::build_all()` is the real core bootstrap path. It runs `init_database()`, `init_secrets()`, `init_llm()`, `init_tools()`, and `init_extensions()` in that order, then seeds workspace identity files and backfills embeddings.
- Database initialization is not just connection setup. It runs migrations, reloads config from the database with TOML overlay, normalizes provider settings from DB if needed, and clears stale sandbox/routine state before the agent starts.
- Secret initialization has two modes: pre-injected secrets store for Scrappy/Tauri, or a local master-key-backed secrets store for standalone runs. In both cases, it reinjects secrets into config and re-resolves config afterward.
- The runtime job model is split: `ContextManager` tracks job contexts and memories with a reserved overflow slot for system tasks, while `SessionManager` maps external channel threads to internal UUID-backed sessions and owns per-user workspace locks.
- The background runtime is routine-driven, not a loose set of ad hoc timers. `Agent::run()` creates a `RoutineEngine`, wires SSE/system-event/subagent/user-timezone inputs into it, spawns a cron ticker, and checks event triggers synchronously during message processing. Separate background tasks include channel health monitoring, memory hygiene, and zombie reaping.
- The web gateway is a first-class runtime surface, not just a side API. `GatewayState` carries the agent loop message sender, SSE manager, workspace, session manager, extension manager, tool registry, store, job manager, LLM/runtime state, cost tracking, routine engine, secrets store, and channel manager.
- Deployment shapes are explicit in code: standalone binary, managed tunnel, and embedded gateway/Tauri mode all coexist. `main.rs` wires the same core engine into different transport and host surfaces rather than branching into separate product modes.

## Current Doc Accuracy Assessment

- `Agent_flow.md` is the closest thing to a canonical internal runtime doc. Its boot layering and overall sequence are mostly right, but it is stale on onboarding step count and workspace seeding count, and it should not be the only place where the runtime story lives.
- `CLAUDE.md` gets the philosophy right, but it is stale as an implementation inventory. It still claims Telegram and Slack are native channels and says the setup wizard is 18 steps, both of which no longer match the current code/document set.
- `src/setup/README.md` should remain the wizard-specific spec, but the step list must be the only authoritative place for onboarding detail. `Agent_flow.md` should link to it instead of restating the wizard.
- `README.md` is too broad for the amount of runtime detail it currently carries. It should be a front page and gateway to canonical docs, not the place where startup sequencing, feature matrices, and architecture all compete for space.

## Contradictions And Drift

- `Agent_flow.md` says `seed_if_empty()` creates 7 core files, but the code seeds more than that, including `README`, `MEMORY`, `IDENTITY`, `SOUL`, `AGENTS`, `USER`, `TOOLS`, `BOOT`, `BOOTSTRAP`, and `HEARTBEAT`.
- `CLAUDE.md` still describes Telegram and Slack as native channels, while the runtime wiring now centers on native Discord, Gmail, Signal, Nostr, iMessage, plus WASM channel loading and hot reload.
- The onboarding story is inconsistent across docs: `CLAUDE.md` says 18 steps, `Agent_flow.md` says 9, and the setup docs should be the single source of truth for the actual wizard flow.
- `Agent_flow.md` under-describes the runtime background model. Heartbeat is now part of the routine engine / cron ticker path, not a separate standalone concept detached from routines.
- The config layering story should explicitly show the two-pass pattern: initial env/TOML resolution, then DB-backed resolution, then secret injection and re-resolution. That flow is visible in code but still too easy to miss in the docs.

## Canonical Architecture Topics

- Boot and config layering.
- Onboarding trigger and first-run behavior.
- AppBuilder phase boundaries and ownership.
- Workspace seeding and system-prompt identity files.
- Session/thread/job model and concurrency limits.
- Routines, heartbeat, and other background work.
- Web gateway and operator-facing deployment shapes.
- Hot-reloaded channels and extensions, but only at the architecture level.

## Rewrite Recommendations

- Create one canonical runtime architecture overview from code, and make other docs link to it instead of duplicating the path.
- Keep `Agent_flow.md` as the internal flow reference, but trim duplicate wizard and startup detail after the canonical overview exists.
- Demote `CLAUDE.md` to a contributor guide plus philosophy; remove outdated runtime inventory claims from it.
- Keep `README.md` concise and user-facing, with runtime specifics delegated to the canonical architecture doc.
- Remove brittle counts and obsolete feature names from the canonical docs unless they are generated or clearly labeled as approximate.

## Evidence Pointers

- Boot, onboarding, config load, and `AppBuilder` entry: [src/main.rs](/Users/vespian/coding/ThinClaw-main/src/main.rs#L212), [src/main.rs](/Users/vespian/coding/ThinClaw-main/src/main.rs#L228), [src/main.rs](/Users/vespian/coding/ThinClaw-main/src/main.rs#L256)
- Onboarding trigger: [src/main_helpers.rs](/Users/vespian/coding/ThinClaw-main/src/main_helpers.rs#L378)
- DB, secrets, LLM, tools, extensions, and workspace seeding phases: [src/app.rs](/Users/vespian/coding/ThinClaw-main/src/app.rs#L183), [src/app.rs](/Users/vespian/coding/ThinClaw-main/src/app.rs#L322), [src/app.rs](/Users/vespian/coding/ThinClaw-main/src/app.rs#L424), [src/app.rs](/Users/vespian/coding/ThinClaw-main/src/app.rs#L432), [src/app.rs](/Users/vespian/coding/ThinClaw-main/src/app.rs#L550), [src/app.rs](/Users/vespian/coding/ThinClaw-main/src/app.rs#L892), [src/app.rs](/Users/vespian/coding/ThinClaw-main/src/app.rs#L1006)
- Workspace seeding implementation: [src/workspace/workspace_core.rs](/Users/vespian/coding/ThinClaw-main/src/workspace/workspace_core.rs#L879)
- Session and thread ownership model: [src/agent/session_manager.rs](/Users/vespian/coding/ThinClaw-main/src/agent/session_manager.rs#L27)
- Job/context capacity model: [src/context/manager.rs](/Users/vespian/coding/ThinClaw-main/src/context/manager.rs#L11)
- Background routines and cron ticker: [src/agent/agent_loop.rs](/Users/vespian/coding/ThinClaw-main/src/agent/agent_loop.rs#L556), [src/agent/agent_loop.rs](/Users/vespian/coding/ThinClaw-main/src/agent/agent_loop.rs#L929), [src/agent/heartbeat.rs](/Users/vespian/coding/ThinClaw-main/src/agent/heartbeat.rs#L434)
- Gateway runtime surface: [src/channels/web/server.rs](/Users/vespian/coding/ThinClaw-main/src/channels/web/server.rs#L124)
- Doc drift examples: [Agent_flow.md](/Users/vespian/coding/ThinClaw-main/Agent_flow.md#L127), [CLAUDE.md](/Users/vespian/coding/ThinClaw-main/CLAUDE.md#L14), [CLAUDE.md](/Users/vespian/coding/ThinClaw-main/CLAUDE.md#L26)
