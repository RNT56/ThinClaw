# Contradictions Log

| Topic | Docs In Conflict | Code / Evidence | Impact | Recommended Resolution |
|---|---|---|---|---|
| Onboarding step count | `src/setup/README.md`, `Agent_flow.md`, `CLAUDE.md`, `docs/DEPLOYMENT.md` | `src/setup/wizard/mod.rs` | High | Make wizard code canonical and update every setup-facing doc |
| Gateway default port | `README.md`, `docs/DEPLOYMENT.md` | `src/config/channels.rs` | High | Standardize on `3000` as default and document remote overrides separately |
| Channel taxonomy | `CLAUDE.md`, `channels-docs/README.md`, channel guides | `src/channels/mod.rs`, `registry/channels/*.json`, `docs/CHANNEL_ARCHITECTURE.md` | High | Make architecture doc canonical and rewrite channel index/pages accordingly |
| Tool vs MCP vs registry install/auth flows | `docs/EXTENSION_SYSTEM.md`, `src/tools/README.md`, `tools-docs/*.md` | `src/cli/tool.rs`, `src/cli/mcp.rs`, `src/cli/registry.rs` | High | Separate command surfaces and trust boundaries explicitly |
| `thinclaw auth` docs | `tools-docs/README.md`, Google/Gmail tool docs | `src/cli/mod.rs` | High | Replace with `thinclaw tool auth ...` or correct setup paths |
| `thinclaw secret set` docs | `tools-docs/telegram.md`, `tools-docs/slack.md`, `tools-docs/github.md`, `tools-src/brave-search/README.md` | `src/cli/mod.rs` | Medium | Remove nonexistent CLI references and document actual auth storage path |
| Operator message route | `docs/DEPLOYMENT.md` vs CLI behavior | `src/cli/message.rs`, `src/channels/web/server.rs` | High | Decide route truth in code/docs and align both |
| Tunnel defaults | `docs/EXTERNAL_DEPENDENCIES.md` | `Cargo.toml` feature defaults | Medium | Mark tunnel features as optional, not default-on |
| Security guarantees | `README.md`, `CLAUDE.md`, `docs/EXTENSION_SYSTEM.md` | `src/tools/README.md`, `src/NETWORK_SECURITY.md` | High | Rewrite trust-boundary language to distinguish WASM, native, and MCP |
