# Truth Map

| Claim | Current Doc | Evidence | Status | Action | Target Doc |
|---|---|---|---|---|---|
| ThinClaw is a self-hosted personal agent runtime, not just a chat wrapper | `README.md` | `src/app.rs`, `src/agent/`, `src/channels/web/`, `Agent_flow.md` | partial | rewrite | `README.md` |
| Security is layered, but trust boundaries differ between native code, WASM, and MCP | `README.md`, `CLAUDE.md`, `docs/EXTENSION_SYSTEM.md` | `src/safety/`, `src/sandbox/`, `src/secrets/`, `src/tools/README.md`, `src/NETWORK_SECURITY.md` | contradicted | rewrite | `README.md`, `docs/EXTENSION_SYSTEM.md`, new security overview |
| The setup wizard is the canonical onboarding path | `README.md`, `docs/DEPLOYMENT.md`, `Agent_flow.md`, `src/setup/README.md`, `CLAUDE.md` | `src/setup/wizard/mod.rs` | contradicted | rewrite | `src/setup/README.md`, `docs/DEPLOYMENT.md`, `Agent_flow.md`, `CLAUDE.md` |
| The onboarding wizard currently has 20 steps | `src/setup/README.md`, `Agent_flow.md`, `CLAUDE.md`, `docs/DEPLOYMENT.md` | `src/setup/wizard/mod.rs` | contradicted | rewrite | `src/setup/README.md`, `Agent_flow.md`, `CLAUDE.md`, `docs/DEPLOYMENT.md` |
| Gateway default local port is `3000` | `README.md`, `docs/DEPLOYMENT.md` | `src/config/channels.rs` | contradicted | rewrite | `README.md`, `docs/DEPLOYMENT.md` |
| `18789` is a deployment-specific remote/gateway value, not the universal default | `README.md`, `docs/DEPLOYMENT.md` | `src/config/channels.rs`, deployment guidance | partial | rewrite | `docs/DEPLOYMENT.md` |
| Channel architecture is hybrid: native for persistent/local access, WASM for stateless integrations | `docs/CHANNEL_ARCHITECTURE.md`, `channels-docs/README.md`, `CLAUDE.md` | `src/channels/mod.rs`, `src/channels/wasm/` | contradicted | rewrite | `docs/CHANNEL_ARCHITECTURE.md`, `channels-docs/README.md`, `CLAUDE.md` |
| Telegram, Slack, Discord interactions, and WhatsApp are packaged WASM channels | `channels-docs/README.md`, `CLAUDE.md` | `registry/channels/*.json`, `docs/CHANNEL_ARCHITECTURE.md` | contradicted | rewrite | `channels-docs/README.md`, `CLAUDE.md` |
| Discord Gateway is native and Discord interactions are a separate WASM package | `channels-docs/discord.md`, `channels-docs/README.md` | `src/channels/discord.rs`, `registry/channels/discord.json` | partial | rewrite | `channels-docs/README.md`, `channels-docs/discord.md` |
| `thinclaw tool ...`, `thinclaw mcp ...`, and registry installs are distinct flows | `docs/EXTENSION_SYSTEM.md`, `src/tools/README.md`, `tools-docs/README.md` | `src/cli/tool.rs`, `src/cli/mcp.rs`, `src/cli/registry.rs` | contradicted | rewrite | `docs/EXTENSION_SYSTEM.md`, `src/tools/README.md`, `tools-docs/README.md` |
| There is no top-level `thinclaw auth ...` command | `tools-docs/*.md` | `src/cli/mod.rs` | contradicted | rewrite | `tools-docs/README.md`, tool docs |
| There is no current `thinclaw secret set ...` command | `tools-docs/*.md`, `tools-src/brave-search/README.md` | `src/cli/mod.rs` | contradicted | rewrite | tool docs, brave-search README |
| `thinclaw message send` and the gateway docs should use the same route surface | `docs/DEPLOYMENT.md`, operator docs | `src/cli/message.rs`, `src/channels/web/server.rs` | contradicted | rewrite or code-follow-up | operator docs + possible code issue |
| `README.md` should be a front door and routing layer, not the full operator manual | `README.md` | audit reports, doc sprawl | partial | rewrite | `README.md` |
| `docs/DEPLOYMENT.md` should own deployment modes and remote access, not the full onboarding spec | `docs/DEPLOYMENT.md`, `src/setup/README.md`, `Agent_flow.md` | audit reports | partial | rewrite | `docs/DEPLOYMENT.md` |
| `docs/CHANNEL_ARCHITECTURE.md` should own architecture and `channels-docs/README.md` should be an index | `docs/CHANNEL_ARCHITECTURE.md`, `channels-docs/README.md` | audit reports | partial | rewrite | both |
| `FEATURE_PARITY.md` is an engineering tracker, not public narrative | `FEATURE_PARITY.md`, top-level docs | audit reports | accurate | keep | `FEATURE_PARITY.md` |
| `rewrite-docs/` is historical archive only | `rewrite-docs/README.md` | archive labels | accurate | keep | `rewrite-docs/README.md` |
