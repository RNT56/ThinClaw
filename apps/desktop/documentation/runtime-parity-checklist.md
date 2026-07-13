# ThinClaw Desktop Runtime Parity Checklist

Last updated: 2026-07-13

This checklist maps ThinClaw root runtime surfaces to Desktop support status. Status values are:

- `wired`: available through local Desktop runtime wiring.
- `tested`: covered by automated contract/unit tests.
- `fixture-tested`: covered by deterministic local/remote fixture acceptance.
- `real-smoke-optional`: requires real credentials, sidecars, or remote hosts beyond fixture acceptance.
- `external-release-operator`: requires release-operator credentials or clean-machine packaging resources.
- `gated`: available only when config, compile features, and host permissions allow it.

| Surface | Desktop Status | Notes |
| --- | --- | --- |
| Agent dependencies | tested | Desktop uses the root `thinclaw` crate with the `desktop` feature. Agent deps share cost tracker, response cache, routing policy, SSE sender, subagent executor, model override, persistent `AgentRouter`, and persistent `AgentRegistry`. `runtime_ports` remains optional because root runtime has not made explicit ports mandatory. |
| Tools | fixture-tested | Shared AppBuilder tools are available: memory, files, search, TTS, ComfyUI, learning, MCP/extension tools, LLM model tools, advisor, MoA, vision, routines, skills, jobs, autonomy status, experiments, and subagents. Fixture acceptance must exercise job and send-message command paths. |
| Secrets | tested | Desktop uses one app-wide `SecretStore`; its runtime trait implementation preserves ThinClaw key names and grant checks. Legacy Scrappy/ThinClaw aliases migrate to canonical identifiers; new runtime writes use those identifiers. Contract tests cover denied read/access methods for ungranted keys. |
| Channels | wired | Tauri local channel is first-class. Slack and Telegram settings are preserved. Remote mode maps gateway `channel_setup` status into desktop channel status entries, including Gmail and Apple Mail availability. Gmail status, Apple Mail settings, and pairing APIs now route through ThinClaw DB/gateway APIs; remote Gmail OAuth returns an explicit gateway-host unavailable response instead of running local OAuth. |
| Routines | tested | Routine engine is started through ThinClaw background tasks and routine lifecycle SSE events forward to `thinclaw-event`. List/create/toggle/run/history/delete/clear stay backed by ThinClaw DB/runtime APIs in local and remote mode. Creation supports both full agent jobs and scheduled `SystemEvent` heartbeat messages; request compatibility, generated bindings, and the trigger-selector UI are covered by contract tests. |
| Cost / routing / cache | fixture-tested | Desktop exposes cost dashboard, response cache, routing policy, model discovery, `llm_select`, and LLM management surfaces. Remote mode proxies cost summary/export/reset, cache stats, provider routing config/status/simulation, and routing rule mutation through gateway APIs. The routing UI surfaces primary/cheap lanes, advisor readiness, runtime diagnostics, and route simulation. |
| Extensions | wired | Installed extension views, registry/direct install, ClawHub search/install, setup schema/secret submit, reconnect, remove, activate, validate setup, lifecycle audit, and manifest validation are surfaced. Remote mode proxies `/api/extensions` where the gateway exposes routes and returns explicit unavailable for hook/lifecycle/manifest-only local surfaces. |
| Skills | wired | Skill list/status, catalog search/install, direct URL/content install, remove, trust changes, single/all reload, inspect/audit, and publish dry-run/confirmed remote-write paths are wired. Remote mode proxies `/api/skills` routes with confirmation headers for mutating operations. |
| MCP | wired | Desktop exposes configured MCP servers, tools, resources/read, resource templates, prompts/get, OAuth discovery, log level, pending interactions, and approve/deny responses. Remote mode proxies `/api/mcp/*`. |
| Jobs | fixture-tested | Job list/detail/events/files/cancel/restart/prompt surfaces exist. Full execution is available when the job backend supports it; unsupported backends show visible reasons. |
| Autonomy | gated | Autonomy status/bootstrap/pause/resume/permissions/rollback/rollouts/checks/evidence surfaces exist. Execution remains disabled unless explicit reckless desktop config and host permission checks pass. Fixture acceptance covers allowed and denied states. |
| Experiments | fixture-tested | Projects, campaigns, runners, trials, targets, model usage, opportunities, GPU cloud validation, and test launch surfaces are present. Mutations are gated by ThinClaw API/config capability with visible reasons. |
| Learning | fixture-tested | Learning status/history/candidates/outcomes/proposals/reviews/rollbacks are visible. Mutation availability is tied to DB/runtime support and visible gate reasons. |
| Provider vault | fixture-tested | Provider settings and keychain fallback are present. Remote mode maps desktop cloud provider aliases to ThinClaw provider slugs, proxies provider model discovery, saves custom OpenAI-compatible keys through the provider vault endpoint, and updates remote primary/provider/model/pool selection through `/api/providers/config`. The provider UI surfaces key readiness, primary model selection, cheap-lane status, and advisor readiness. |
| Canvas / A2UI | wired | Canvas `StatusUpdate::CanvasAction` maps to `UiEvent::CanvasUpdate`; local AgentDeps include a `CanvasStore`. Remote gateway mode must preserve the same event schema and panel access semantics. |
| Lifecycle events | fixture-tested | `StatusUpdate::ContextCompactionStarted`/`AdvisorConsultationStarted`/`SelfRepairStarted`/`SelfRepairCompleted` map to `UiEvent::AgentLifecycleEvent` via `event_mapping.rs`; emitted from the dispatcher (compaction, advisor) and the self-repair loop. |
| Undo / redo | wired | `thinclaw_undo`/`thinclaw_redo` route `/undo`·`/redo` through the message pipeline (both modes); cockpit toolbar buttons added. |
| Session search | wired | `thinclaw_session_search` (FTS + optional cheap-model summarization) + Session Search panel. LocalOnly. |
| Checkpoints / rollback | wired | `list`/`diff`/`restore` commands + Rollback panel (filesystem shadow-git checkpoints). |
| Trajectory viewer and export | tested | `stats`/`records` commands plus the Trajectory panel. `thinclaw_trajectory_export` reuses the canonical CLI SFT/DPO validation over legacy and durable run-artifact records, bounds IPC to 5,000 examples, and downloads only after an explicit user action. |
| Profile evolution | tested | Learning Review displays the parsed local profile, confidence/evidence metadata, schedule, run history, and failures. The explicit run command upserts the reserved weekly routine from first-party profile material and starts it through the live routine engine; profile display is capped at 512 KiB and invalid JSON is reported without reflecting raw content. |
| Agent eval | wired | `thinclaw_experiments_list_envs` + `thinclaw_experiments_run_eval` (AgentLoopEnv, LocalOnly). Runtime smoke-test pending a running engine. |
| Channel config | wired | `Channel::config_schema()` + DTOs; read commands + `thinclaw_channel_config_submit` (LocalOnly; WASM live, native restart-required); Signal/Discord impls + Channel Config panel. |
| Tool policy | gated | `disabled_tools` deny-list enforced in the dispatcher tool-execution preflight. |

## External Release Operator Entries

- Real provider credentials and remote gateway tokens are optional after fixture acceptance.
- Apple signing identity, notary credentials, updater private key, and clean-machine notarization remain release-operator responsibilities.

## Contract Test Targets

The deterministic local/remote gate is executable from the repository root:

```bash
scripts/ci/desktop-fixture-acceptance.sh
```

CI runs this harness in the Desktop Companion job. It requires no provider
credentials: the local fixture checks representative bridge availability and
typed gating, while the loopback gateway fixture exercises authenticated chat,
session, jobs, autonomy, learning, experiments, MCP, skill, provider, cost, and
cache routes, including mutation-confirmation headers.

- Every `StatusUpdate` variant maps to a `UiEvent` or is intentionally documented.
- Desktop `AgentDeps` includes the same required runtime handles as root `src/main.rs`.
- Remote proxy route matrix covers every desktop-exposed gateway API.
- Current matrix: `apps/desktop/documentation/remote-gateway-route-matrix.md`.
- The shared `SecretStore` runtime view denies ungranted keys for all read/access methods.
- Route simulation responses preserve ThinClaw planner target, fallback, rejection, score, and diagnostic fields.
- Generated frontend bindings must be regenerated from Rust and must pass sanitizer tests for Tauri `Channel<T>` and reserved argument names.
