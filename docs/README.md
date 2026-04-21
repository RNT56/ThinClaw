# ThinClaw Documentation

This directory is the canonical entry point for current ThinClaw docs.

Use this page to pick the right path for your role instead of relying on whichever top-level file you opened first.

## Start Here

- New to ThinClaw: go to [../README.md](../README.md)
- Running ThinClaw yourself: go to [DEPLOYMENT.md](DEPLOYMENT.md)
- Running reckless desktop autonomy: go to [DESKTOP_AUTONOMY.md](DESKTOP_AUTONOMY.md)
- Understanding identity, packs, and `/personality`: go to [IDENTITY_AND_PERSONALITY.md](IDENTITY_AND_PERSONALITY.md)
- Understanding memory, continuity, and `/compress`: go to [MEMORY_AND_GROWTH.md](MEMORY_AND_GROWTH.md)
- Understanding outcome-backed learning, Learning Ledger outcomes, and deferred review: go to [OUTCOME_BACKED_LEARNING.md](OUTCOME_BACKED_LEARNING.md)
- Understanding research workspaces, experiments, and GPU clouds: go to [RESEARCH_AND_EXPERIMENTS.md](RESEARCH_AND_EXPERIMENTS.md)
- Understanding the shared surface vocabulary: go to [SURFACES_AND_COMMANDS.md](SURFACES_AND_COMMANDS.md)
- Choosing or configuring providers: go to [LLM_PROVIDERS.md](LLM_PROVIDERS.md)
- Understanding channels: go to [CHANNEL_ARCHITECTURE.md](CHANNEL_ARCHITECTURE.md)
- Understanding extensions and tools: go to [EXTENSION_SYSTEM.md](EXTENSION_SYSTEM.md)
- Understanding security and trust: go to [SECURITY.md](SECURITY.md)

## By Audience

### Operators

- [DEPLOYMENT.md](DEPLOYMENT.md)
- [DESKTOP_AUTONOMY.md](DESKTOP_AUTONOMY.md)
- [IDENTITY_AND_PERSONALITY.md](IDENTITY_AND_PERSONALITY.md)
- [MEMORY_AND_GROWTH.md](MEMORY_AND_GROWTH.md)
- [OUTCOME_BACKED_LEARNING.md](OUTCOME_BACKED_LEARNING.md)
- [RESEARCH_AND_EXPERIMENTS.md](RESEARCH_AND_EXPERIMENTS.md)
- Deployment note: `thinclaw` and `thinclaw run` are quiet by default; use `thinclaw --debug` or `thinclaw --debug run` for verbose startup logs
- [LLM_PROVIDERS.md](LLM_PROVIDERS.md)
- [EXTERNAL_DEPENDENCIES.md](EXTERNAL_DEPENDENCIES.md)
- [../channels-docs/README.md](../channels-docs/README.md)
- [../tools-docs/README.md](../tools-docs/README.md)

### Contributors And Maintainers

- [../CLAUDE.md](../CLAUDE.md)
- [AUDIT_CLOSURE_WAVE7_WAVE8.md](AUDIT_CLOSURE_WAVE7_WAVE8.md)
- [IDENTITY_AND_PERSONALITY.md](IDENTITY_AND_PERSONALITY.md)
- [../src/setup/README.md](../src/setup/README.md)
- [../src/tools/README.md](../src/tools/README.md)
- [../src/workspace/README.md](../src/workspace/README.md)
- [../src/NETWORK_SECURITY.md](../src/NETWORK_SECURITY.md)

### Architecture

- [CHANNEL_ARCHITECTURE.md](CHANNEL_ARCHITECTURE.md)
- [EXTENSION_SYSTEM.md](EXTENSION_SYSTEM.md)
- [BUILD_PROFILES.md](BUILD_PROFILES.md)

## Canonical Ownership

| Topic | Canonical Doc |
|---|---|
| Public product entry point | [../README.md](../README.md) |
| Identity packs and session personality | [IDENTITY_AND_PERSONALITY.md](IDENTITY_AND_PERSONALITY.md) |
| Memory, continuity, and growth surfaces | [MEMORY_AND_GROWTH.md](MEMORY_AND_GROWTH.md) |
| Outcome-backed learning and Learning Ledger outcomes | [OUTCOME_BACKED_LEARNING.md](OUTCOME_BACKED_LEARNING.md) |
| Desktop autonomy profile, bootstrap, and rollback | [DESKTOP_AUTONOMY.md](DESKTOP_AUTONOMY.md) |
| Research, experiments, and remote runners | [RESEARCH_AND_EXPERIMENTS.md](RESEARCH_AND_EXPERIMENTS.md) |
| Shared cross-surface command vocabulary | [SURFACES_AND_COMMANDS.md](SURFACES_AND_COMMANDS.md) |
| Onboarding and setup behavior | [../src/setup/README.md](../src/setup/README.md) |
| Deployment and remote access | [DEPLOYMENT.md](DEPLOYMENT.md) |
| Channel architecture | [CHANNEL_ARCHITECTURE.md](CHANNEL_ARCHITECTURE.md) |
| Security and trust | [SECURITY.md](SECURITY.md) |
| Extension architecture | [EXTENSION_SYSTEM.md](EXTENSION_SYSTEM.md) |
| LLM provider configuration | [LLM_PROVIDERS.md](LLM_PROVIDERS.md) |
| Provider catalog (code) | [../src/config/provider_catalog.rs](../src/config/provider_catalog.rs) |
| Tool implementation guidance | [../src/tools/README.md](../src/tools/README.md) |
| Memory and workspace model | [../src/workspace/README.md](../src/workspace/README.md) |
| Deep network model | [../src/NETWORK_SECURITY.md](../src/NETWORK_SECURITY.md) |
| Parity + ThinClaw-first feature tracking | [../FEATURE_PARITY.md](../FEATURE_PARITY.md) |

## Archive Boundary

Historical migration-era notes may still appear elsewhere in the repository, but they are not part of the current docs tree. Treat anything outside this index and the linked canonicals above as archival context rather than a source of truth for current architecture or setup decisions.
