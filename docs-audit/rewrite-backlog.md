# Rewrite Backlog

## P0 Canonicals

| File | Goal | Status |
|---|---|---|
| `README.md` | front door, quick start, docs routing, thinner public story | completed |
| `docs/README.md` | docs hub and canonical routing | completed |
| `docs/DEPLOYMENT.md` | deployment modes, defaults, remote access, source-build truth | completed |
| `docs/CHANNEL_ARCHITECTURE.md` | native vs WASM channel authority | completed |
| `docs/SECURITY.md` | short public security/trust overview | completed |
| `docs/EXTENSION_SYSTEM.md` | WASM vs MCP trust model and CLI boundary | completed |
| `channels-docs/README.md` | operator-facing channel index | completed |
| `tools-docs/README.md` | operator-facing tool index and current auth vocabulary | completed |
| `src/setup/README.md` | authoritative onboarding/setup spec | completed |
| `Agent_flow.md` | runtime/boot flow only | completed |

## P1 Important

| File Group | Goal | Status |
|---|---|---|
| `CLAUDE.md` | maintainer map aligned to current architecture | completed |
| `CONTRIBUTING.md` | contributor workflow and doc-update rules | completed |
| `channels-docs/*.md` | transport/runtime wording aligned to current model | high-risk and routing-critical pages completed |
| `tools-docs/*.md` | stale auth/secret command cleanup | major auth-sensitive pages completed |
| `docs/GMAIL_SETUP.md` | align shared Gmail tool/channel auth story | completed |
| `tools-src/brave-search/README.md` | align install/auth wording to current registry/tool flow | completed |

## P2 Cleanup

| File Group | Goal | Status |
|---|---|---|
| `docs/EXTERNAL_DEPENDENCIES.md` | optional-feature wording and defaults cleanup | completed |
| `docs/LLM_PROVIDERS.md` | verify public-facing routing only, keep reference-heavy detail here | pending |
| `FEATURE_PARITY.md` references | confirm behavior-facing wording after docs stabilization | pending |
| archive and scratch docs | keep out of current reference flow | ongoing |

## Remaining Hard Blocker

| Area | Issue | Owner |
|---|---|---|
| Gateway / CLI message route | `src/cli/message.rs` uses `/api/chat` while gateway-facing docs and route tables use `/api/chat/send` | code follow-up or explicit docs decision |
