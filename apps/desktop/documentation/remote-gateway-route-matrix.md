# Remote Gateway Route Matrix

P3-W4 checkpoint for ThinClaw Desktop remote mode. Desktop IPC names remain
`openclaw_*`; remote calls go through `RemoteGatewayProxy` to the root ThinClaw
HTTP gateway. Unsupported operations must return an `unavailable:` error with a
concrete reason.

Last updated: 2026-05-15

| Surface | Desktop command/proxy coverage | Remote endpoint | Status |
| --- | --- | --- | --- |
| Chat send | `openclaw_send_message` | `POST /api/chat/send` | wired |
| Chat abort | `openclaw_abort_chat` | none | unavailable: no gateway abort endpoint |
| Approvals | `openclaw_resolve_approval` | `POST /api/chat/approval` | wired |
| Sessions list/history/delete | `openclaw_get_sessions`, `openclaw_get_history`, `openclaw_delete_session` | `GET /api/chat/threads`, `GET /api/chat/history`, `DELETE /api/chat/thread/{id}` | wired |
| Session reset/export/compact | `openclaw_reset_session`, `openclaw_export_session`, `openclaw_compact_session` | none | unavailable: no clear/export/compact endpoint |
| Memory read/write/list/search | memory/file commands | `/api/memory/read`, `/api/memory/write`, `/api/memory/tree`, `/api/memory/search` | wired |
| Memory delete | `openclaw_delete_file` | none | unavailable: no memory delete endpoint |
| Routines list/run/history/toggle/delete | routine commands | `/api/routines`, `/api/routines/{id}/trigger`, `/api/routines/{id}/runs`, `/api/routines/{id}/toggle`, `DELETE /api/routines/{id}` | wired |
| Routine create/clear-runs | routine create/clear commands | `POST /api/routines`, `DELETE /api/routines/runs` | wired |
| Skills list/status/search/install/remove/trust/reload/inspect/publish | skill commands | `GET /api/skills`, `POST /api/skills/search`, `POST /api/skills/install`, `DELETE /api/skills/{name}`, `PUT /api/skills/{name}/trust`, `POST /api/skills/{name}/reload`, `POST /api/skills/reload-all`, `POST /api/skills/{name}/inspect`, `POST /api/skills/{name}/publish` | wired |
| Skill toggle/repo clone | skill commands | none | unavailable: no enable toggle; arbitrary git clone is local-only |
| Extensions list/install/registry/activate/reconnect/validate/remove/setup/tools | extension/tool commands | `/api/extensions`, `/api/extensions/install`, `/api/extensions/registry`, `/api/extensions/{name}/activate`, `/api/extensions/{name}/reconnect`, `/api/extensions/{name}/validate`, `/api/extensions/{name}/remove`, `/api/extensions/{name}/setup`, `/api/extensions/tools` | wired |
| Hooks/lifecycle audit/manifest validation/cache stats | dashboard/extension commands | none | unavailable or local-only with explicit reason |
| MCP servers/tools/resources/templates/prompts/OAuth/log-level/interactions | MCP desktop commands | `/api/mcp/servers`, `/api/mcp/servers/{name}`, `/api/mcp/servers/{name}/tools`, `/api/mcp/servers/{name}/resources`, `/api/mcp/servers/{name}/resources/read`, `/api/mcp/servers/{name}/resource-templates`, `/api/mcp/servers/{name}/prompts`, `/api/mcp/servers/{name}/prompts/{prompt_name}`, `/api/mcp/servers/{name}/oauth`, `/api/mcp/servers/{name}/log-level`, `/api/mcp/interactions`, `/api/mcp/interactions/{interaction_id}/respond` | wired |
| Settings | config get/set/patch | `/api/settings`, `/api/settings/{key}` | wired |
| Gateway status/presence | status/diagnostics/presence commands | `/api/health`, `/api/gateway/status` | wired |
| Logs | `openclaw_logs_tail` | live SSE only | unavailable: no recent-log snapshot endpoint |
| Costs | cost commands | `/api/costs/summary`, `/api/costs/export`, `/api/costs/reset` | wired |
| Routing/provider config | routing/cloud config/simulation commands | `/api/providers`, `/api/providers/config`, `/api/providers/{slug}/models`, `/api/providers/route/simulate` | wired for config/status/simulation; desktop-shaped rule mutations unavailable |
| Provider vault | key commands/proxy | `POST/DELETE /api/providers/{slug}/key` | wired for save/delete/status only; raw secret reads denied |
| Pairing/channels/Gmail | pairing/channel/Gmail commands | `/api/pairing/{channel}`, `/api/pairing/{channel}/approve`, `/api/gateway/status`, `/api/settings/*` | wired where gateway exposes status/config |
| Jobs | `openclaw_jobs_list`, `openclaw_jobs_summary`, `openclaw_job_detail`, `openclaw_job_cancel`, `openclaw_job_restart`, `openclaw_job_prompt`, `openclaw_job_events`, `openclaw_job_files_list`, `openclaw_job_file_read` | `/api/jobs/*` | wired; local direct jobs expose list/detail/events/cancel and explicit unavailable reasons for sandbox-only restart/prompt/files |
| Autonomy | `openclaw_autonomy_status`, `openclaw_autonomy_bootstrap`, `openclaw_autonomy_pause`, `openclaw_autonomy_resume`, `openclaw_autonomy_permissions`, `openclaw_autonomy_rollback`, `openclaw_autonomy_rollouts`, `openclaw_autonomy_checks`, `openclaw_autonomy_evidence` | `/api/autonomy/*` | wired for status/review surfaces; host-executing mutation remains gated by remote or local host policy |
| Experiments | experiment IPC wrappers and proxy helpers | `/api/experiments/*` | wired for status/review/action surfaces exposed by the gateway |
| Learning | learning IPC wrappers and proxy helpers | `/api/learning/*` | wired for status/history/candidates/review surfaces exposed by the gateway |

Known intentional gaps are gateway API gaps, not silent desktop no-ops. When the
root gateway adds endpoints for abort, reset, transcript export, memory delete,
hook management, or log snapshots, Desktop
should replace the matching `unavailable:` branch with the real route.

## Remote Mode Rules

- Mutating skill endpoints must send `X-Confirm-Action: true`.
- Remote provider-vault commands may save/delete/status keys, but must never read raw secret values.
- Local-only host actions, especially arbitrary git clone, Gmail OAuth launched on the desktop host, and autonomy execution, must report explicit unavailable reasons in remote mode.
- Remote SSE events must be normalized into `UiEvent` on `openclaw-event`; the frontend should not subscribe to a separate remote event schema.
