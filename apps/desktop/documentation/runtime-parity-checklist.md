# ThinClaw Desktop Runtime Parity Checklist

Last updated: 2026-05-15

This checklist maps ThinClaw root runtime surfaces to Desktop support status. Status values are:

- `wired`: available through local Desktop runtime wiring.
- `partial`: surfaced, with a named remaining gap in the notes.
- `gated`: available only when config, compile features, and host permissions allow it.
- `missing`: not yet wired.

| Surface | Desktop Status | Notes |
| --- | --- | --- |
| Agent dependencies | partial | Desktop uses the root `thinclaw` crate with the `desktop` feature. Agent deps now share cost tracker, response cache, routing policy, SSE sender, subagent executor, model override, persistent `AgentRouter`, and persistent `AgentRegistry`. Remaining named gap: `runtime_ports` stays `None` until ThinClaw makes explicit root ports mandatory. |
| Tools | partial | Shared AppBuilder tools are available: memory, files, search, TTS, ComfyUI, learning, MCP/extension tools, LLM model tools, advisor, MoA, vision, routines, skills, jobs, autonomy status, experiments, and subagents. Remaining named gap: job execution and send-message tools need final endpoint-by-endpoint desktop validation in manual smoke. |
| Secrets | partial | Desktop uses `KeychainSecretsAdapter` and preserves ThinClaw key names and grant checks. Legacy Scrappy key names remain read-only fallback. Contract tests cover denied read/access methods for ungranted keys. |
| Channels | wired | Tauri local channel is first-class. Slack and Telegram settings are preserved. Remote mode maps gateway `channel_setup` status into desktop channel status entries, including Gmail and Apple Mail availability. Gmail status, Apple Mail settings, and pairing APIs now route through ThinClaw DB/gateway APIs; remote Gmail OAuth returns an explicit gateway-host unavailable response instead of running local OAuth. |
| Routines | wired | Routine engine is started through ThinClaw background tasks and routine lifecycle SSE events forward to `openclaw-event`. List/create/toggle/run/history/delete/clear stay backed by ThinClaw DB/runtime APIs in local and remote mode. |
| Cost / routing / cache | partial | Desktop exposes cost dashboard, response cache, routing policy, model discovery, `llm_select`, and LLM management surfaces. Cost persistence and OpenRouter pricing sync now run in local desktop mode. Remote mode proxies cost summary/export/reset and maps provider routing config/status/simulation through gateway APIs. The routing UI surfaces primary/cheap lanes, advisor readiness, runtime diagnostics, and route simulation. Remaining named gaps: remote desktop-shaped routing rule mutations return explicit unavailable until translated to provider policy rules; response cache stats return explicit unavailable remotely because the gateway has no cache endpoint. |
| Extensions | wired | Installed extension views, registry/direct install, ClawHub search/install, setup schema/secret submit, reconnect, remove, activate, validate setup, lifecycle audit, and manifest validation are surfaced. Remote mode proxies `/api/extensions` where the gateway exposes routes and returns explicit unavailable for hook/lifecycle/manifest-only local surfaces. |
| Skills | wired | Skill list/status, catalog search/install, direct URL/content install, remove, trust changes, single/all reload, inspect/audit, and publish dry-run/confirmed remote-write paths are wired. Remote mode proxies `/api/skills` routes with confirmation headers for mutating operations. |
| MCP | wired | Desktop exposes configured MCP servers, tools, resources/read, resource templates, prompts/get, OAuth discovery, log level, pending interactions, and approve/deny responses. Remote mode proxies `/api/mcp/*`. |
| Jobs | gated | Job list/detail/events/files/cancel/restart/prompt surfaces exist. Full execution remains gated by backend support and host permissions; unsupported operations must show visible reasons. |
| Autonomy | gated | Autonomy status/bootstrap/pause/resume/permissions/rollback/rollouts/checks/evidence surfaces exist. Execution remains disabled unless explicit reckless desktop config and host permission checks pass. |
| Experiments | partial | Projects, campaigns, runners, trials, targets, model usage, opportunities, GPU cloud validation, and test launch surfaces are present. Remaining named gap: mutations stay gated by ThinClaw API availability and config, and final manual smoke must confirm every visible control reports availability correctly. |
| Learning | partial | Learning status/history/candidates/outcomes/proposals/reviews/rollbacks are visible. Remaining named gap: mutation availability must remain tied to DB/runtime support and final manual smoke. |
| Provider vault | partial | Provider settings and keychain fallback are present. Remote mode maps desktop cloud provider aliases to ThinClaw provider slugs, proxies provider model discovery, saves custom OpenAI-compatible keys through the provider vault endpoint, and updates remote primary provider/model selection through `/api/providers/config`. The provider UI surfaces key readiness, primary model selection, cheap-lane status, and advisor readiness. Remaining named gap: complete primary/cheap pool editing controls. |
| Canvas / A2UI | wired | Canvas `StatusUpdate::CanvasAction` maps to `UiEvent::CanvasUpdate`; local AgentDeps include a `CanvasStore`. Remote gateway mode must preserve the same event schema and panel access semantics. |

## Current Partial Entries

All `partial` entries above have named remaining gaps:

- Agent dependencies: root `runtime_ports` are not mandatory yet.
- Tools: final job/send-message endpoint validation belongs to P3 smoke.
- Cost/routing/cache: remote routing-rule translation and remote cache stats need gateway endpoints.
- Experiments/learning/provider vault: controls are present, but final availability-state smoke must confirm every mutation gate and complete pool editing.

## Contract Test Targets

- Every `StatusUpdate` variant maps to a `UiEvent` or is intentionally documented.
- Desktop `AgentDeps` includes the same required runtime handles as root `src/main.rs`.
- Remote proxy route matrix covers every desktop-exposed gateway API.
- Current matrix: `apps/desktop/documentation/remote-gateway-route-matrix.md`.
- `KeychainSecretsAdapter` denies ungranted keys for all read/access methods.
- Route simulation responses preserve ThinClaw planner target, fallback, rejection, score, and diagnostic fields.
- Generated frontend bindings must be regenerated from Rust and must pass sanitizer tests for Tauri `Channel<T>` and reserved argument names.
