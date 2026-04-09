# ThinClaw Agent Flow

This document explains how ThinClaw boots and runs after configuration is in place. It is the runtime-flow companion to `src/setup/README.md`.

## What This Doc Owns

- bootstrap and early startup flow
- `AppBuilder` initialization phases
- runtime wiring for sessions, channels, tools, and background systems
- workspace seeding and system-prompt inputs

This document does not own the full onboarding spec. Use `src/setup/README.md` for the wizard and setup invariants.

## High-Level Runtime Shape

ThinClaw has three major layers:

1. bootstrap and configuration loading
2. application construction through `AppBuilder`
3. the long-lived runtime around sessions, channels, tools, and background jobs

At a glance:

```text
.env / bootstrap -> AppBuilder -> session manager + tools + channels + gateway -> agent runtime
```

## Early Bootstrap

Before the main runtime is built, ThinClaw:

- loads `./.env`
- loads `~/.thinclaw/.env`
- resolves early config inputs
- decides whether onboarding is needed
- initializes tracing and shared runtime helpers

This is the stage where ThinClaw decides whether to continue into normal runtime startup or divert into the onboarding path.

## AppBuilder Phases

`AppBuilder` is the main initialization pipeline. The exact code lives in `src/app.rs`, but the phases are conceptually:

1. database initialization and settings access
2. secrets initialization
3. LLM stack initialization
4. tool and workspace setup
5. channels, gateway, extensions, and runtime surfaces

The important rule is that later phases depend on earlier trust and config layers already existing.

## Config Resolution In Practice

ThinClaw's effective runtime configuration is assembled from several layers:

- process environment
- local `.env`
- `~/.thinclaw/.env`
- optional TOML overlay
- encrypted or injected secrets
- database-backed settings

Do not assume a single flat “env wins” model is enough to describe runtime behavior in every deployment shape.

## Workspace Seeding

The workspace is more than storage. It also provides identity and long-term runtime context.

Typical seeded or well-known files include:

- `AGENTS.md`
- identity and value files
- memory and routine-related workspace paths

These files influence how ThinClaw builds prompts and persists context across sessions.

## Runtime Assembly

Once bootstrap and `AppBuilder` complete, the runtime wires together:

- session and conversation lifecycle management
- tool registry and execution policy
- channel ingress and response delivery
- web gateway and API surfaces
- workspace and memory systems
- background execution such as routines and heartbeat

This is why ThinClaw should be documented as a runtime, not a thin request-response wrapper.

## Operator Surfaces

ThinClaw exposes multiple ways to operate the same runtime:

- CLI commands
- web gateway
- interactive terminal flow
- messaging and webhook channels
- embedded Scrappy mode

The gateway is the control plane. It should be documented that way, not as just another channel alongside messaging integrations.

## Channel And Extension Wiring

Channel delivery is hybrid:

- native Rust channels are used where persistent connections or local access matter
- packaged WASM channels are used where stateless delivery and host-boundary credential handling matter

Extension flows are also split:

- built-in tools live in the core runtime
- WASM tools are sandboxed guest components
- MCP servers are external operator-trusted integrations

Those trust boundaries matter at boot time because they affect how capabilities are loaded and what guarantees ThinClaw can honestly claim.

## Session And Message Flow

At runtime, incoming work is normalized into the same core session machinery regardless of where it came from.

Broadly:

1. a message or event arrives
2. ThinClaw resolves or creates the appropriate session/thread context
3. the dispatcher builds the tool/prompt/runtime context
4. the agent loop performs reasoning, tool use, and response generation
5. the result is streamed or delivered back through the calling surface

This applies to gateway traffic, CLI-triggered operations, and channel-driven message flow.

## Background Systems

ThinClaw is not only interactive. The runtime also supports:

- routines and scheduled work
- heartbeat-style proactive execution
- maintenance or monitoring behavior
- extension and channel lifecycle handling

That proactive layer is part of the core operating model.

## Scrappy Embedding

When ThinClaw is embedded inside Scrappy, the same core runtime is reused. The main difference is how local capabilities, secrets, and operator UI state are provided to the backend.

The embedded path should be thought of as a different host environment for the same runtime, not as a second implementation.

## Documentation Rule

If runtime behavior changes:

- update this file when boot/runtime flow changed
- update `src/setup/README.md` when onboarding behavior changed
- update `docs/DEPLOYMENT.md` when deployment or remote-access guidance changed
- update `FEATURE_PARITY.md` when the change affects parity-tracked behavior
