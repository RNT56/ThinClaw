# Source of Truth Matrix

| Topic | Canonical Doc | Supporting Evidence | Secondary Docs To Update | Notes |
|---|---|---|---|---|
| Public product definition | `README.md` | `src/app.rs`, audit reports | `CLAUDE.md`, overview docs | Keep short and route outward |
| Onboarding wizard behavior | `src/setup/README.md` + `src/setup/wizard/mod.rs` | wizard code | `README.md`, `docs/DEPLOYMENT.md`, `Agent_flow.md`, `CLAUDE.md` | Code wins when docs drift |
| Boot/runtime flow | `Agent_flow.md` | `src/main.rs`, `src/app.rs`, `src/agent/` | `CLAUDE.md`, `README.md` | Should not restate full wizard spec |
| Deployment modes and remote access | `docs/DEPLOYMENT.md` | `src/service.rs`, `src/config/channels.rs`, `src/tunnel/` | `README.md`, channel/operator docs | Separate defaults from remote overrides |
| LLM provider setup | `docs/LLM_PROVIDERS.md` | `src/config/llm.rs`, `src/llm/` | `README.md` | Reference doc, not front-page matrix |
| Channel architecture | `docs/CHANNEL_ARCHITECTURE.md` | `src/channels/`, `registry/channels/` | `channels-docs/README.md`, `CLAUDE.md`, README | Architecture authority |
| Per-channel setup | `channels-docs/*.md` | code + registry manifests | `channels-docs/README.md` | Pages must declare native vs WASM explicitly |
| Extension architecture and trust model | `docs/EXTENSION_SYSTEM.md` | `src/extensions/`, `src/tools/mcp/`, `src/tools/wasm/` | `README.md`, `src/tools/README.md`, tool docs | Must distinguish WASM vs MCP |
| Tool implementation guidance | `src/tools/README.md` | `src/tools/`, CLI modules | `docs/EXTENSION_SYSTEM.md` | Maintainer-facing |
| Per-tool operator setup | `tools-docs/*.md` | `src/cli/tool.rs`, registry manifests | `tools-docs/README.md` | Use current CLI/auth vocabulary |
| Memory/workspace behavior | `src/workspace/README.md` | `src/workspace/` | `README.md` | Internal spec |
| Security / network model | `src/NETWORK_SECURITY.md` | `src/safety/`, `src/sandbox/`, `src/secrets/` | `README.md`, `docs/EXTENSION_SYSTEM.md`, overview security doc | Add a shorter public-facing security overview |
| Contributor workflow | `CONTRIBUTING.md` | Rust toolchain + repo workflow | `CLAUDE.md` | Keep concise and current |
| Feature parity tracking | `FEATURE_PARITY.md` | engineering review | none | Internal coordination doc |
| Migration history | `rewrite-docs/README.md` | archive materials | none | Keep fenced off from current docs |
