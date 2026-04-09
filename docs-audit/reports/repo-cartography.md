# Repo Cartography Report

## Executive Summary

ThinClaw already has a strong documentation base, but it is split across too many voices and too many levels of specificity. The biggest risk is not missing information; it is contradictory information. The public README, maintainer guides, subsystem specs, channel/tool indexes, and historical rewrite docs overlap heavily, and several of them still describe the same feature through different architectural assumptions.

The cleanest direction is audience-first documentation: one public entry point, one canonical home per subsystem, and a strict archive boundary for migration-era material. ThinClaw-specific design ideas should be foregrounded, especially the hybrid native/WASM extension model, the security-by-architecture story, the proactive agent runtime, and the standalone-vs-embedded deployment split.

## Documentation Surface Map

| Cluster | Main Files | Audience | Purpose | Current Value | Likely Status |
|---|---|---|---|---|---|
| Public entrypoint | [README.md](/Users/vespian/coding/ThinClaw-main/README.md) | New users, evaluators, operators | Product pitch, quick start, feature overview, provider/install/deploy/security summary | Highest leverage, but overloaded | Partial |
| Maintainer guide | [CLAUDE.md](/Users/vespian/coding/ThinClaw-main/CLAUDE.md), [Agent_flow.md](/Users/vespian/coding/ThinClaw-main/Agent_flow.md) | Contributors, maintainers, agent authors | Internal workflow, boot/runtime model, project structure | High value, but stale in places and duplicative | Partial / Stale |
| Parity tracker | [FEATURE_PARITY.md](/Users/vespian/coding/ThinClaw-main/FEATURE_PARITY.md) | Engineers, release managers | Cross-check ThinClaw vs OpenClaw capability state | Useful coordination artifact, not a user doc | Accurate for tracking, not narrative |
| Canonical subsystem specs | [docs/CHANNEL_ARCHITECTURE.md](/Users/vespian/coding/ThinClaw-main/docs/CHANNEL_ARCHITECTURE.md), [docs/EXTENSION_SYSTEM.md](/Users/vespian/coding/ThinClaw-main/docs/EXTENSION_SYSTEM.md), [docs/LLM_PROVIDERS.md](/Users/vespian/coding/ThinClaw-main/docs/LLM_PROVIDERS.md), [docs/DEPLOYMENT.md](/Users/vespian/coding/ThinClaw-main/docs/DEPLOYMENT.md), [docs/EXTERNAL_DEPENDENCIES.md](/Users/vespian/coding/ThinClaw-main/docs/EXTERNAL_DEPENDENCIES.md), [docs/BUILDING_CHANNELS.md](/Users/vespian/coding/ThinClaw-main/docs/BUILDING_CHANNELS.md), [docs/BUILD_PROFILES.md](/Users/vespian/coding/ThinClaw-main/docs/BUILD_PROFILES.md), [docs/GMAIL_SETUP.md](/Users/vespian/coding/ThinClaw-main/docs/GMAIL_SETUP.md), [docs/TELEGRAM_SETUP.md](/Users/vespian/coding/ThinClaw-main/docs/TELEGRAM_SETUP.md) | Operators, contributors | Deep reference docs for architecture, deployment, provider setup, and subsystem behavior | Strongest docs cluster overall | Mostly Accurate, some Partial |
| Module specs | [src/setup/README.md](/Users/vespian/coding/ThinClaw-main/src/setup/README.md), [src/tools/README.md](/Users/vespian/coding/ThinClaw-main/src/tools/README.md), [src/workspace/README.md](/Users/vespian/coding/ThinClaw-main/src/workspace/README.md) | Maintainers, code contributors | Source-aligned specs for onboarding, tools, and memory/workspace behavior | Very useful, but should be kept tightly code-synced | Partial / Accurate |
| Channel guides | [channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md), `channels-docs/*.md` | Operators, integrators | Per-channel setup and messaging behavior | High-risk because transport descriptions diverge from current architecture docs | Stale / Contradicted |
| Tool guides | [tools-docs/README.md](/Users/vespian/coding/ThinClaw-main/tools-docs/README.md), `tools-docs/*.md` | Operators, integrators | Per-tool setup, auth, and permissions | Useful content, but command names and auth flow references are inconsistent | Stale / Contradicted |
| Contributor process | [CONTRIBUTING.md](/Users/vespian/coding/ThinClaw-main/CONTRIBUTING.md) | Contributors | Local QA, PR hygiene, feature-parity reminder | Small but important, currently contains a concrete stale command | Partial / Stale |
| Historical archive | [rewrite-docs/README.md](/Users/vespian/coding/ThinClaw-main/rewrite-docs/README.md), [rewrite-docs/REWRITE_TRACKER.md](/Users/vespian/coding/ThinClaw-main/rewrite-docs/REWRITE_TRACKER.md), `rewrite-docs/*.md` | Historical reference only | Migration-era OpenClaw/IronClaw rewrite material | Valuable only as history | Archive |

## Audience / Purpose Matrix

| Audience | Best Docs Today | What They Need | Main Gap |
|---|---|---|---|
| New users | [README.md](/Users/vespian/coding/ThinClaw-main/README.md) | What ThinClaw is, how to start, what it needs, what it runs on | Too much breadth, not enough concise pathing |
| Operators | [docs/DEPLOYMENT.md](/Users/vespian/coding/ThinClaw-main/docs/DEPLOYMENT.md), [docs/LLM_PROVIDERS.md](/Users/vespian/coding/ThinClaw-main/docs/LLM_PROVIDERS.md), [docs/EXTERNAL_DEPENDENCIES.md](/Users/vespian/coding/ThinClaw-main/docs/EXTERNAL_DEPENDENCIES.md), [channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md), [tools-docs/README.md](/Users/vespian/coding/ThinClaw-main/tools-docs/README.md) | How to run, configure, authenticate, and connect integrations | Too many competing setup surfaces |
| Contributors | [CLAUDE.md](/Users/vespian/coding/ThinClaw-main/CLAUDE.md), [src/setup/README.md](/Users/vespian/coding/ThinClaw-main/src/setup/README.md), [src/tools/README.md](/Users/vespian/coding/ThinClaw-main/src/tools/README.md), [src/workspace/README.md](/Users/vespian/coding/ThinClaw-main/src/workspace/README.md), [CONTRIBUTING.md](/Users/vespian/coding/ThinClaw-main/CONTRIBUTING.md) | Repo-specific conventions, internal architecture, code paths, local QA | Some docs read like notes, not specs |
| Maintainers | [Agent_flow.md](/Users/vespian/coding/ThinClaw-main/Agent_flow.md), [FEATURE_PARITY.md](/Users/vespian/coding/ThinClaw-main/FEATURE_PARITY.md), `src/*` README docs | Accurate runtime flow, parity state, and subsystem contracts | Need stricter canonical ownership |
| Historical readers | [rewrite-docs/README.md](/Users/vespian/coding/ThinClaw-main/rewrite-docs/README.md) | Migration context only | Archive needs stronger boundary enforcement |

## Major Overlap and Drift Areas

The biggest overlap is between [README.md](/Users/vespian/coding/ThinClaw-main/README.md), [CLAUDE.md](/Users/vespian/coding/ThinClaw-main/CLAUDE.md), and [Agent_flow.md](/Users/vespian/coding/ThinClaw-main/Agent_flow.md). All three describe setup, runtime shape, and automation behavior, but they do it at different depths and with different terminology. That makes them useful individually and confusing collectively.

[channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md) conflicts with [docs/CHANNEL_ARCHITECTURE.md](/Users/vespian/coding/ThinClaw-main/docs/CHANNEL_ARCHITECTURE.md) on channel transport models. The channel index still presents Telegram as long-polling, Slack as Socket Mode, and Discord as a single Gateway channel, while the architecture doc splits the system into native and WASM paths with different reasons for each choice.

[tools-docs/README.md](/Users/vespian/coding/ThinClaw-main/tools-docs/README.md) and the individual tool docs still point to `thinclaw auth ...` flows that are not the same thing as the CLI’s `thinclaw tool auth ...` and `thinclaw mcp ...` surfaces. That makes the tool docs valuable, but not yet trustworthy as a single entry point.

[CLAUDE.md](/Users/vespian/coding/ThinClaw-main/CLAUDE.md) also carries stale inventory language. It still lists `slack.rs` and `telegram.rs` as native channels even though the current architecture docs and code layout treat those areas differently. The setup flow is another drift point: [Agent_flow.md](/Users/vespian/coding/ThinClaw-main/Agent_flow.md) says the wizard is 9 steps, while [src/setup/README.md](/Users/vespian/coding/ThinClaw-main/src/setup/README.md) documents 18 steps.

[docs/EXTENSION_SYSTEM.md](/Users/vespian/coding/ThinClaw-main/docs/EXTENSION_SYSTEM.md) is broadly strong, but it mixes current and legacy storage language for MCP config. That is a typical example of a good doc that needs a sharper canonical rule, not a wholesale rewrite.

[CONTRIBUTING.md](/Users/vespian/coding/ThinClaw-main/CONTRIBUTING.md) is short, but its opening `npm run ci` instruction is stale for this Rust repo. This is the kind of small error that can quietly erode trust.

## Archive and Merge Candidates

[rewrite-docs/README.md](/Users/vespian/coding/ThinClaw-main/rewrite-docs/README.md) and [rewrite-docs/REWRITE_TRACKER.md](/Users/vespian/coding/ThinClaw-main/rewrite-docs/REWRITE_TRACKER.md) should remain strictly archived. The archive is already labeled correctly, so the next step is protecting that boundary with navigation and cross-links, not reviving it as current reference.

[channels-docs/](/Users/vespian/coding/ThinClaw-main/channels-docs) should be merged toward a canonical channel architecture doc plus narrow per-channel how-tos. Its index is useful, but it should not compete with [docs/CHANNEL_ARCHITECTURE.md](/Users/vespian/coding/ThinClaw-main/docs/CHANNEL_ARCHITECTURE.md) for architecture truth.

[tools-docs/](/Users/vespian/coding/ThinClaw-main/tools-docs) should be merged toward a canonical extensions/tools overview plus per-tool setup docs that all use the same auth vocabulary. The current split is too easy to misread.

The module READMEs are worth keeping, but they should be treated as internal specs, not public docs. [src/setup/README.md](/Users/vespian/coding/ThinClaw-main/src/setup/README.md), [src/tools/README.md](/Users/vespian/coding/ThinClaw-main/src/tools/README.md), and [src/workspace/README.md](/Users/vespian/coding/ThinClaw-main/src/workspace/README.md) are best kept as the code-adjacent contract layer.

## Recommended Information Architecture

1. Public layer: `README.md` as the ThinClaw introduction, quick start, and link hub.
2. Overview layer: a small set of canonical overview docs for architecture, security, runtime, and deployment.
3. How-to layer: install, provider setup, channels, tools, remote access, and platform-specific setup.
4. Reference layer: CLI, config/env, API, channel matrix, tool matrix, and operator-facing behavior.
5. Internal layer: module READMEs, parity tracking, contributor guidance, and implementation notes.
6. Archive layer: rewrite history and migration comparisons, clearly fenced off from current reference.

That structure fits ThinClaw better than the current “everything in a lot of places” model. It also makes the product story clearer: ThinClaw is a self-hosted agent runtime with strong security boundaries, hybrid extensibility, and multiple deployment modes, not just a generic AI assistant repository.

## Evidence Pointers

- [README.md](/Users/vespian/coding/ThinClaw-main/README.md#L8) and [README.md](/Users/vespian/coding/ThinClaw-main/README.md#L34) show the broad public-facing pitch and the amount of detail already embedded in the top-level entrypoint.
- [CLAUDE.md](/Users/vespian/coding/ThinClaw-main/CLAUDE.md#L14), [CLAUDE.md](/Users/vespian/coding/ThinClaw-main/CLAUDE.md#L26), and [CLAUDE.md](/Users/vespian/coding/ThinClaw-main/CLAUDE.md#L135) show stale channel inventory and an 18-step onboarding claim.
- [Agent_flow.md](/Users/vespian/coding/ThinClaw-main/Agent_flow.md#L127) and [src/setup/README.md](/Users/vespian/coding/ThinClaw-main/src/setup/README.md#L51) disagree on wizard step count and are both trying to describe the same workflow.
- [docs/CHANNEL_ARCHITECTURE.md](/Users/vespian/coding/ThinClaw-main/docs/CHANNEL_ARCHITECTURE.md#L3), [docs/CHANNEL_ARCHITECTURE.md](/Users/vespian/coding/ThinClaw-main/docs/CHANNEL_ARCHITECTURE.md#L11), and [docs/CHANNEL_ARCHITECTURE.md](/Users/vespian/coding/ThinClaw-main/docs/CHANNEL_ARCHITECTURE.md#L15) define the hybrid native/WASM channel model that should anchor all channel docs.
- [channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md#L25), [channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md#L26), and [channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md#L27) still present Telegram, Slack, and Discord through older transport assumptions.
- [tools-docs/README.md](/Users/vespian/coding/ThinClaw-main/tools-docs/README.md#L18), [tools-docs/README.md](/Users/vespian/coding/ThinClaw-main/tools-docs/README.md#L19), and [tools-docs/README.md](/Users/vespian/coding/ThinClaw-main/tools-docs/README.md#L25) use auth language that does not consistently match the current CLI surface.
- [docs/EXTENSION_SYSTEM.md](/Users/vespian/coding/ThinClaw-main/docs/EXTENSION_SYSTEM.md#L132) and [docs/EXTENSION_SYSTEM.md](/Users/vespian/coding/ThinClaw-main/docs/EXTENSION_SYSTEM.md#L192) show the mixed MCP storage/startup narrative.
- [CONTRIBUTING.md](/Users/vespian/coding/ThinClaw-main/CONTRIBUTING.md#L7) and [CONTRIBUTING.md](/Users/vespian/coding/ThinClaw-main/CONTRIBUTING.md#L47) show the stale `npm run ci` instruction and the parity-update rule.
- [rewrite-docs/README.md](/Users/vespian/coding/ThinClaw-main/rewrite-docs/README.md#L1) and [rewrite-docs/REWRITE_TRACKER.md](/Users/vespian/coding/ThinClaw-main/rewrite-docs/REWRITE_TRACKER.md#L1) clearly mark the migration docs as archived history.
- [src/workspace/README.md](/Users/vespian/coding/ThinClaw-main/src/workspace/README.md#L9), [src/workspace/README.md](/Users/vespian/coding/ThinClaw-main/src/workspace/README.md#L68), and [src/workspace/README.md](/Users/vespian/coding/ThinClaw-main/src/workspace/README.md#L87) are good examples of internal module specs that are already close to the right pattern.
