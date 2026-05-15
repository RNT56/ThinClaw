# ThinClaw Desktop Runtime Parity Checklist

This checklist maps ThinClaw root runtime surfaces to Desktop support status. Status values are:

- `wired`: available through local Desktop runtime wiring.
- `partial`: surfaced, but some root behavior or remote parity remains.
- `gated`: available only when config, compile features, and host permissions allow it.
- `missing`: not yet wired.

| Surface | Desktop Status | Notes |
| --- | --- | --- |
| Agent dependencies | partial | Desktop uses the root `thinclaw` crate with the `desktop` feature. Agent deps now share cost tracker, response cache, routing policy, SSE sender, subagent executor, model override, persistent `AgentRouter`, and persistent `AgentRegistry`. `runtime_ports` stays `None` until ThinClaw makes explicit root ports mandatory. |
| Tools | partial | Shared AppBuilder tools are available: memory, files, search, TTS, ComfyUI, learning, MCP/extension tools, LLM model tools, advisor, MoA, vision, routines, skills, and subagents. Agent-management tools are now registered. Job and send-message tools still need endpoint-by-endpoint desktop validation. |
| Secrets | partial | Desktop uses `KeychainSecretsAdapter` and preserves ThinClaw key names and grant checks. Legacy Scrappy key names remain read-only fallback. Contract tests still need to cover all denied methods. |
| Channels | partial | Tauri local channel is first-class. Slack and Telegram settings are preserved. Remote mode maps gateway `channel_setup` status into desktop channel status entries. Gmail OAuth/status, Apple Mail settings, and pairing APIs need full command/proxy parity checks. |
| Routines | partial | Routine engine is started through ThinClaw background tasks and routine lifecycle SSE events forward to `openclaw-event`. Create/toggle/history/clear should stay backed by ThinClaw DB APIs, not frontend memory. |
| Cost / routing / cache | partial | Desktop exposes cost dashboard, response cache, routing policy, model discovery, `llm_select`, and LLM management surfaces. Cost persistence and OpenRouter pricing sync now run in local desktop mode. Remote mode now proxies cost summary/export/reset and maps provider routing config/status through the ThinClaw gateway APIs. Remote desktop-shaped routing rule mutations return an explicit unavailable error until they can be translated to ThinClaw provider policy rules. Response cache stats return an explicit unavailable error in remote mode because the gateway has no cache endpoint yet. Route simulation and policy mutation still need continued frontend coverage. |
| Extensions | partial | ClawHub, extension list/install, lifecycle audit, manifest validation, and MCP extension tools are surfaced where ThinClaw APIs exist. Remote parity needs typed unavailable responses where gateway endpoints are absent. |
| Jobs | gated | Job surfaces exist in ThinClaw and some desktop UI. Full execution should only be enabled when backend features and host permissions are present. |
| Autonomy | gated | Desktop autonomy tools remain disabled by default. Register only when ThinClaw config explicitly enables reckless desktop autonomy and host permission checks pass. |
| Experiments | partial | Root experiment controller exists. Desktop should start with read/status/review surfaces, with mutations gated by explicit config. |
| Learning | partial | Learning tools are registered when the DB exists. Desktop UI should expose status/review before enabling mutation controls broadly. |
| Provider vault | partial | Provider settings and keychain fallback are present. Remote mode now maps desktop cloud provider aliases to ThinClaw provider slugs, proxies provider model discovery, saves custom OpenAI-compatible keys through the provider vault endpoint, and updates remote primary provider/model selection through `/api/providers/config`. Desktop still needs complete key command coverage for every provider, primary/cheap pool editing, and advisor readiness surfacing. |
| Canvas / A2UI | wired | Canvas `StatusUpdate::CanvasAction` maps to `UiEvent::CanvasUpdate`; local AgentDeps include a `CanvasStore`. Remote gateway mode must preserve the same event schema and panel access semantics. |

## Contract Test Targets

- Every `StatusUpdate` variant maps to a `UiEvent` or is intentionally documented.
- Desktop `AgentDeps` includes the same required runtime handles as root `src/main.rs`.
- Remote proxy route matrix covers every desktop-exposed gateway API.
- `KeychainSecretsAdapter` denies ungranted keys for all read/access methods.
- Generated frontend bindings must be regenerated from Rust and must not contain hand-edited drift.
