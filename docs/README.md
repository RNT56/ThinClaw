# ThinClaw Documentation

This directory is the canonical entry point for current ThinClaw docs.

Use this page to pick the right path for your role instead of relying on whichever top-level file you opened first.

## Start Here

- New to ThinClaw: go to [../README.md](../README.md)
- Running ThinClaw yourself: go to [DEPLOYMENT.md](DEPLOYMENT.md)
- Choosing or configuring providers: go to [LLM_PROVIDERS.md](LLM_PROVIDERS.md)
- Understanding channels: go to [CHANNEL_ARCHITECTURE.md](CHANNEL_ARCHITECTURE.md)
- Understanding extensions and tools: go to [EXTENSION_SYSTEM.md](EXTENSION_SYSTEM.md)
- Understanding security and trust: go to [SECURITY.md](SECURITY.md)

## By Audience

### Operators

- [DEPLOYMENT.md](DEPLOYMENT.md)
- Deployment note: `thinclaw` and `thinclaw run` are quiet by default; use `thinclaw --debug` or `thinclaw --debug run` for verbose startup logs
- [LLM_PROVIDERS.md](LLM_PROVIDERS.md)
- [EXTERNAL_DEPENDENCIES.md](EXTERNAL_DEPENDENCIES.md)
- [../channels-docs/README.md](../channels-docs/README.md)
- [../tools-docs/README.md](../tools-docs/README.md)

### Contributors And Maintainers

- [../CLAUDE.md](../CLAUDE.md)
- [../Agent_flow.md](../Agent_flow.md)
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
| Onboarding and setup behavior | [../src/setup/README.md](../src/setup/README.md) |
| Boot and runtime flow | [../Agent_flow.md](../Agent_flow.md) |
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

Historical migration-era material lives in [../rewrite-docs/README.md](../rewrite-docs/README.md). It is useful for history, not for current architecture or setup decisions.
