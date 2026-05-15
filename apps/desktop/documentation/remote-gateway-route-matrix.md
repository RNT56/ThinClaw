# Remote Gateway Route Matrix

Absolute-completion checkpoint for ThinClaw Desktop remote mode. Desktop IPC names are
`thinclaw_*`; remote calls go through `RemoteGatewayProxy` to the root ThinClaw
HTTP gateway. Unsupported operations must return an `unavailable:` error with a
concrete reason.

Last updated: 2026-05-15

| Surface | Desktop command/proxy coverage | Remote endpoint | Status |
| --- | --- | --- | --- |
| Chat send | `thinclaw_send_message` | `POST /api/chat/send` | wired |
| Chat abort | `thinclaw_abort_chat` | `POST /api/chat/abort` | wired |
| Approvals | `thinclaw_resolve_approval` | `POST /api/chat/approval` | wired |
| Sessions list/history/delete | `thinclaw_get_sessions`, `thinclaw_get_history`, `thinclaw_delete_session` | `GET /api/chat/threads`, `GET /api/chat/history`, `DELETE /api/chat/thread/{id}` | wired |
| Session reset/export/compact | `thinclaw_reset_session`, `thinclaw_export_session`, `thinclaw_compact_session` | `POST /api/chat/thread/{id}/reset`, `GET /api/chat/thread/{id}/export`, `POST /api/chat/thread/{id}/compact` | wired |
| Memory read/write/list/search | memory/file commands | `/api/memory/read`, `/api/memory/write`, `/api/memory/tree`, `/api/memory/search` | wired |
| Memory delete | `thinclaw_delete_file` | `POST /api/memory/delete` | wired |
| Routines list/run/history/toggle/delete | routine commands | `/api/routines`, `/api/routines/{id}/trigger`, `/api/routines/{id}/runs`, `/api/routines/{id}/toggle`, `DELETE /api/routines/{id}` | wired |
| Routine create/clear-runs | routine create/clear commands | `POST /api/routines`, `DELETE /api/routines/runs` | wired |
| Skills list/status/search/install/remove/trust/reload/inspect/publish | skill commands | `GET /api/skills`, `POST /api/skills/search`, `POST /api/skills/install`, `DELETE /api/skills/{name}`, `PUT /api/skills/{name}/trust`, `POST /api/skills/{name}/reload`, `POST /api/skills/reload-all`, `POST /api/skills/{name}/inspect`, `POST /api/skills/{name}/publish` | wired |
| Skill toggle/repo clone | skill commands | none | unavailable: no enable toggle; arbitrary git clone is local-only |
| Extensions list/install/registry/activate/reconnect/validate/remove/setup/tools | extension/tool commands | `/api/extensions`, `/api/extensions/install`, `/api/extensions/registry`, `/api/extensions/{name}/activate`, `/api/extensions/{name}/reconnect`, `/api/extensions/{name}/validate`, `/api/extensions/{name}/remove`, `/api/extensions/{name}/setup`, `/api/extensions/tools` | wired |
| Hooks/lifecycle audit/manifest validation/cache stats | dashboard/extension commands | `GET /api/hooks`, `POST /api/hooks`, `DELETE /api/hooks/{name}`, `GET /api/cache/stats`, local-only manifest/lifecycle internals | hook routes and cache stats wired; local-only internals return explicit reason |
| MCP servers/tools/resources/templates/prompts/OAuth/log-level/interactions | MCP desktop commands | `/api/mcp/servers`, `/api/mcp/servers/{name}`, `/api/mcp/servers/{name}/tools`, `/api/mcp/servers/{name}/resources`, `/api/mcp/servers/{name}/resources/read`, `/api/mcp/servers/{name}/resource-templates`, `/api/mcp/servers/{name}/prompts`, `/api/mcp/servers/{name}/prompts/{prompt_name}`, `/api/mcp/servers/{name}/oauth`, `/api/mcp/servers/{name}/log-level`, `/api/mcp/interactions`, `/api/mcp/interactions/{interaction_id}/respond` | wired |
| Settings | config get/set/patch | `/api/settings`, `/api/settings/{key}` | wired |
| Gateway status/presence | status/diagnostics/presence commands | `/api/health`, `/api/gateway/status` | wired |
| Logs | `thinclaw_logs_tail` | `/api/logs/recent`, `/api/logs/events` | wired |
| Costs | cost commands | `/api/costs/summary`, `/api/costs/export`, `/api/costs/reset` | wired |
| Routing/provider config | routing/cloud config/simulation commands | `/api/providers`, `/api/providers/config`, `/api/providers/{slug}/models`, `/api/providers/route/simulate` | wired for config/status/simulation/rule mutation/pool updates |
| Provider vault | key commands/proxy | `POST/DELETE /api/providers/{slug}/key` | wired for save/delete/status only; raw secret reads denied |
| Pairing/channels/Gmail | pairing/channel/Gmail commands | `/api/pairing/{channel}`, `/api/pairing/{channel}/approve`, `/api/gateway/status`, `/api/settings/*` | wired where gateway exposes status/config |
| Jobs | `thinclaw_jobs_list`, `thinclaw_jobs_summary`, `thinclaw_job_detail`, `thinclaw_job_cancel`, `thinclaw_job_restart`, `thinclaw_job_prompt`, `thinclaw_job_events`, `thinclaw_job_files_list`, `thinclaw_job_file_read` | `/api/jobs/*` | wired; local direct jobs expose list/detail/events/cancel and explicit unavailable reasons for sandbox-only restart/prompt/files |
| Autonomy | `thinclaw_autonomy_status`, `thinclaw_autonomy_bootstrap`, `thinclaw_autonomy_pause`, `thinclaw_autonomy_resume`, `thinclaw_autonomy_permissions`, `thinclaw_autonomy_rollback`, `thinclaw_autonomy_rollouts`, `thinclaw_autonomy_checks`, `thinclaw_autonomy_evidence` | `/api/autonomy/*` | wired for status/review surfaces; host-executing mutation remains gated by remote or local host policy |
| Experiments | experiment IPC wrappers and proxy helpers | `/api/experiments/*` | wired for status/review/action surfaces exposed by the gateway |
| Learning | learning IPC wrappers and proxy helpers | `/api/learning/*` | wired for status/history/candidates/review surfaces exposed by the gateway |

Known intentional gaps are external host-policy gates, not silent desktop no-ops.
Fixture acceptance must execute every route family in this matrix before a
release candidate is marked route-complete.

## Remote Mode Rules

- Mutating skill endpoints must send `X-Confirm-Action: true`.
- Remote provider-vault commands may save/delete/status keys, but must never read raw secret values.
- Local-only host actions, especially arbitrary git clone, Gmail OAuth launched on the desktop host, and autonomy execution, must report explicit unavailable reasons in remote mode.
- Remote SSE events must be normalized into `UiEvent` on `thinclaw-event`; the frontend should not subscribe to a separate remote event schema.
