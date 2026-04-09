# Setup / Config / Deployment Report

## Executive Summary

ThinClaw’s setup and deployment story should be split into three canonical layers: bootstrap config, persisted settings, and runtime deployment. Right now the docs mix those layers together, which creates real drift around onboarding step counts, gateway defaults, and config precedence.

The strongest source of truth is the code itself: `src/bootstrap.rs`, `src/config/`, `src/setup/wizard/mod.rs`, and `src/service.rs`. The best outcome is a minimal canonical story that explains how ThinClaw starts, what must exist before the database, what gets persisted after onboarding, and how it runs locally, headless, or as a service.

## Actual Setup And Deployment Model

ThinClaw bootstraps from environment files first, then resolves config overlays, then loads database-backed settings once the DB exists. The actual precedence is more nuanced than “env > database > default”: `optional_env()` resolves bridge-injected UI config first, then injected secrets, then process env, while startup loads `./.env` before `~/.thinclaw/.env` and only later reconciles DB settings after the database is available ([`src/config/helpers.rs`](../../../src/config/helpers.rs), [`src/bootstrap.rs`](../../../src/bootstrap.rs), [`src/config/mod.rs`](../../../src/config/mod.rs)).

The onboarding wizard is a 20-step flow in code, with a special `--channels-only` path and a `--skip-auth` path for provider reuse ([`src/setup/wizard/mod.rs`](../../../src/setup/wizard/mod.rs)). It persists bootstrap vars to `~/.thinclaw/.env` and the rest into the database, which is the right abstraction for a tool that needs database selection before everything else ([`src/setup/README.md`](../../../src/setup/README.md), [`src/bootstrap.rs`](../../../src/bootstrap.rs)).

Deployment is a split between standalone/headless service mode and embedded Scrappy mode. The built-in service manager generates launchd on macOS and systemd user units on Linux, and both start `thinclaw run --no-onboard` ([`src/service.rs`](../../../src/service.rs)). The web gateway defaults to port `3000` in code, while remote-access guidance in the deployment docs uses `0.0.0.0` as the bind address ([`src/config/channels.rs`](../../../src/config/channels.rs), [`docs/DEPLOYMENT.md`](../../../docs/DEPLOYMENT.md)).

## Current Doc Accuracy Assessment

| Doc | Assessment | Notes |
|---|---|---|
| `README.md` | Partial | Good positioning, but it mixes onboarding, deployment, and product marketing, and it is inconsistent with deployment docs on gateway defaults and channel setup. |
| `src/setup/README.md` | Partial | Best setup spec, but the step count and flow are stale relative to `src/setup/wizard/mod.rs`. |
| `docs/DEPLOYMENT.md` | Partial | Broad and useful, but it conflicts with code on gateway defaults and with the wizard on step count and required setup paths. |
| `docs/LLM_PROVIDERS.md` | Mostly accurate | Good provider reference; should stay reference-first, not be folded into the public story. |
| `docs/BUILD_PROFILES.md` | Mostly accurate | Feature/profile mapping is broadly aligned with `Cargo.toml`, but it should be treated as reference material, not a user onboarding path. |
| `docs/EXTERNAL_DEPENDENCIES.md` | Partial | The tunnel section is too strong and currently treats tunnel as default-on, which conflicts with the actual default feature set. |

## Contradictions And Drift

- `src/setup/README.md` says the wizard has 18 steps, while `src/setup/wizard/mod.rs` says 20 steps and implements 20 steps. `docs/DEPLOYMENT.md` still presents a 9-step wizard, and `README.md` also uses a shorter flow. This is the biggest user-facing inconsistency in the setup story.
- `README.md` and `docs/DEPLOYMENT.md` disagree on gateway defaults. The docs bounce between `3000` and `18789`, while code defaults `GATEWAY_PORT` to `3000` and `docs/DEPLOYMENT.md` later calls `3000` the default for gateway config.
- `docs/EXTERNAL_DEPENDENCIES.md` says the `tunnel` feature is enabled by default, but `Cargo.toml` defaults to `light`, and `tunnel` is only part of `full` unless explicitly enabled.
- `src/config/mod.rs` and `src/config/helpers.rs` use a richer overlay model than the docs describe: bridge UI config > injected secrets > process env, with `.env` files loaded in a specific order. The current docs flatten that into “env first” and miss the Scrappy bridge layer entirely.
- `README.md` duplicates setup/deployment guidance that belongs in canonical how-to docs, which makes it easier for stale port or step-count claims to spread.

## Canonical Setup / Config / Deployment Topics

1. Bootstrap and config layering:
   - `./.env`
   - `~/.thinclaw/.env`
   - database-backed settings
   - bridge-injected UI config
   - injected secrets overlay
2. First-run onboarding:
   - database selection
   - secret/master-key setup
   - provider setup
   - model selection
   - channel setup
   - persisted incremental saves
3. Deployment modes:
   - standalone headless agent
   - launched as a macOS/Linux service
   - embedded Scrappy runtime
4. Remote access:
   - gateway on `3000` by default
   - `0.0.0.0` for remote binding
   - Tailscale / tunnel options as explicit add-ons
5. External dependencies:
   - tunnel providers
   - Docker / Podman
   - browser automation
   - Signal CLI
   - Claude Code
   - local inference engines
6. Build profiles:
   - `light`
   - `full`
   - `desktop`
   - `bundled-wasm`

## Rewrite Recommendations

- Make `README.md` short and opinionated: what ThinClaw is, how to start it, what makes it distinct, and where the canonical deep docs live.
- Keep `src/setup/README.md` as the authoritative onboarding spec, but update it to match the actual 20-step flow and the real `--skip-auth` / `--channels-only` behavior.
- Rewrite `docs/DEPLOYMENT.md` into a deployment reference that assumes the canonical setup spec exists elsewhere, and stop duplicating wizard details there.
- Update `docs/EXTERNAL_DEPENDENCIES.md` so tunnel features are described as optional, not default-on.
- Add one explicit config-precedence section that documents the real resolver order, including the Scrappy bridge overlay and secrets overlay.
- Remove brittle claims from public-facing docs unless they are generated or guaranteed to stay in sync.

## Evidence Pointers

- [`src/setup/wizard/mod.rs`](../../../src/setup/wizard/mod.rs)
- [`src/setup/README.md`](../../../src/setup/README.md)
- [`src/bootstrap.rs`](../../../src/bootstrap.rs)
- [`src/config/mod.rs`](../../../src/config/mod.rs)
- [`src/config/helpers.rs`](../../../src/config/helpers.rs)
- [`src/config/channels.rs`](../../../src/config/channels.rs)
- [`src/service.rs`](../../../src/service.rs)
- [`docs/DEPLOYMENT.md`](../../../docs/DEPLOYMENT.md)
- [`docs/LLM_PROVIDERS.md`](../../../docs/LLM_PROVIDERS.md)
- [`docs/BUILD_PROFILES.md`](../../../docs/BUILD_PROFILES.md)
- [`docs/EXTERNAL_DEPENDENCIES.md`](../../../docs/EXTERNAL_DEPENDENCIES.md)
- [`README.md`](../../../README.md)
