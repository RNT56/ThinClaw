# IronClaw Feature Pipelines

> For per-pipeline agent audit. Each pipeline section lists all related source files.
> **Last audited**: 2026-03-19 — verified against live codebase.

---

## 1 · LLM Reasoning & Provider Pipeline
Multi-provider LLM routing, failover, smart routing, cost tracking, caching.

| Layer | Files |
|-------|-------|
| Core | [llm/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/mod.rs), [llm/provider.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/provider.rs), [llm/reasoning.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/reasoning.rs) |
| Adapters | [llm/rig_adapter.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/rig_adapter.rs), [llm/gemini.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/gemini.rs) (Gemini via OpenAI-compat gateway), [llm/bedrock.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/bedrock.rs) (AWS Bedrock via proxy), [llm/llama_cpp.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/llama_cpp.rs) (llama.cpp server via OpenAI-compat) |
| Routing | [llm/smart_routing.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/smart_routing.rs), [llm/routing_policy.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/routing_policy.rs), [llm/discovery.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/discovery.rs), [llm/provider_presets.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/provider_presets.rs) |
| Reliability | [llm/failover.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/failover.rs), [llm/circuit_breaker.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/circuit_breaker.rs), [llm/retry.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/retry.rs) |
| Cost & cache | [llm/cost_tracker.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/cost_tracker.rs), [llm/costs.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/costs.rs), [llm/response_cache.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/response_cache.rs), [llm/response_cache_ext.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/response_cache_ext.rs) |
| Hooks & misc | [llm/llm_hooks.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/llm_hooks.rs), [llm/llms_txt.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/llms_txt.rs), [llm/embeddings.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/embeddings.rs), [llm/extended_context.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/llm/extended_context.rs) |
| Frontend | [OpenClawRouting.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawRouting.tsx), [OpenClawBrain.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawBrain.tsx), [OpenClawCostDashboard.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawCostDashboard.tsx), [OpenClawCacheStats.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawCacheStats.tsx), [CloudBrainConfigModal.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/CloudBrainConfigModal.tsx), [OpenClawDashboard.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawDashboard.tsx) |

---

## 2 · Agent Orchestration Pipeline
Agent loop, dispatcher, session management, subagents, thread ops.

| Layer | Files |
|-------|-------|
| Core loop | [agent/agent_loop.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/agent_loop.rs), [agent/dispatcher.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/dispatcher.rs), [agent/worker.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/worker.rs) |
| Sessions | [agent/session.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/session.rs), [agent/session_manager.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/session_manager.rs), [agent/global_session.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/global_session.rs) |
| Threads | [agent/thread_ops.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/thread_ops.rs), [agent/thread_inheritance.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/thread_inheritance.rs) |
| Subagents | [agent/subagent_executor.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/subagent_executor.rs), [agent/submission.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/submission.rs) |
| Routing | [agent/agent_router.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/agent_router.rs) (workspace-level: which *agent* handles a message), [agent/router.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/router.rs) (message-level: `/command` → `MessageIntent`) |
| Context | [agent/compaction.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/compaction.rs), [agent/context_monitor.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/context_monitor.rs), [agent/cost_guard.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/cost_guard.rs) |
| Misc | [agent/task.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/task.rs), [agent/undo.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/undo.rs), [agent/commands.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/commands.rs), [agent/presence.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/presence.rs), [agent/runtime_behavior.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/runtime_behavior.rs), [agent/self_repair.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/self_repair.rs) |
| Frontend | [LiveAgentStatus.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/LiveAgentStatus.tsx), [SubAgentPanel.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/SubAgentPanel.tsx), [OpenClawPresence.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawPresence.tsx), [OpenClawChatView.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawChatView.tsx), [OpenClawSidebar.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawSidebar.tsx) |

---

## 3 · Scheduler & Routines Pipeline
Cron scheduling, routines, heartbeats, job monitoring.

| Layer | Files |
|-------|-------|
| Core | [agent/scheduler.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/scheduler.rs), [agent/routine.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/routine.rs), [agent/routine_engine.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/routine_engine.rs), [agent/routine_audit.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/routine_audit.rs) |
| Scheduling | [agent/cron_stagger.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/cron_stagger.rs), [agent/heartbeat.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/heartbeat.rs), [agent/job_monitor.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/job_monitor.rs), [agent/management_api.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/agent/management_api.rs) |
| Frontend | [OpenClawAutomations.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawAutomations.tsx), [AutomationCard.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/AutomationCard.tsx), [OpenClawRoutineAudit.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawRoutineAudit.tsx), [ApprovalCard.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/ApprovalCard.tsx) |

---

## 4 · Channels Pipeline
Multi-platform messaging: Signal, Telegram, Discord, Slack, iMessage, Gmail, Nostr, HTTP, REPL, Web, WebSocket.

| Layer | Files |
|-------|-------|
| Manager | [channels/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/mod.rs), [channels/manager.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/manager.rs), [channels/channel.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/channel.rs), [channels/health_monitor.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/health_monitor.rs) |
| Adapters | [channels/signal.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/signal.rs), [channels/telegram.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/telegram.rs), [channels/discord.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/discord.rs), [channels/slack.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/slack.rs), [channels/imessage.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/imessage.rs), [channels/gmail.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/gmail.rs), [channels/nostr.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/nostr.rs), [channels/http.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/http.rs), [channels/repl.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/repl.rs) |
| Wiring | [channels/imessage_wiring.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/imessage_wiring.rs), [channels/gmail_wiring.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/gmail_wiring.rs), [channels/webhook_server.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/webhook_server.rs) |
| Web | [channels/web/](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/web) (SSE gateway, OpenAI-compat API, CORS, static assets) |
| Groups & misc | [channels/group_priming.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/group_priming.rs), [channels/self_message.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/self_message.rs), [channels/ack_reaction.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/ack_reaction.rs), [channels/reaction_machine.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/reaction_machine.rs), [channels/forward_download.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/forward_download.rs), [channels/tool_stream.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/tool_stream.rs), [channels/status_view.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/status_view.rs), [channels/canvas_gateway.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/canvas_gateway.rs) |
| Frontend | [OpenClawChannels.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawChannels.tsx), [OpenClawChannelStatus.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawChannelStatus.tsx), [SlackTab.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/settings/SlackTab.tsx), [TelegramTab.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/settings/TelegramTab.tsx) |

---

## 4b · WASM Channel Runtime Pipeline
WASM-based channel plugins — discovery, loading, sandboxed execution, HTTP routing, host ABI.

> **Note:** This is distinct from `tools/wasm/` (WASM tool sandbox). `channels/wasm/` provides a full plugin runtime for channels that are distributed as `.wasm` modules, with their own capabilities schema, rate limiting, and lifecycle management. Referenced by `main.rs`, `hooks/bootstrap.rs`, and `extensions/manager.rs`.

| Layer | Files |
|-------|-------|
| Module root | [channels/wasm/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/wasm/mod.rs) |
| Runtime | [channels/wasm/runtime.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/wasm/runtime.rs) — Wasmtime engine config, module compilation, `PreparedChannelModule` |
| Loader | [channels/wasm/loader.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/wasm/loader.rs) — `WasmChannelLoader`, `discover_channels()`, filesystem discovery |
| Router | [channels/wasm/router.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/wasm/router.rs) — `WasmChannelRouter`, HTTP endpoint dispatch to WASM channels |
| Wrapper | [channels/wasm/wrapper.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/wasm/wrapper.rs) — `WasmChannel` / `SharedWasmChannel`, poll loop, HTTP handler, lifecycle |
| Host ABI | [channels/wasm/host.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/wasm/host.rs) — Host functions, `ChannelEmitRateLimiter`, `ChannelWorkspaceStore`, `EmittedMessage` |
| Capabilities | [channels/wasm/capabilities.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/wasm/capabilities.rs) — `ChannelCapabilities`, `EmitRateLimitConfig`, security model |
| Schema | [channels/wasm/schema.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/wasm/schema.rs) — `ChannelCapabilitiesFile`, `ChannelConfig`, manifest parsing |
| Bundled | [channels/wasm/bundled.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/wasm/bundled.rs) — `install_bundled_channel()`, built-in channel catalog |
| Error | [channels/wasm/error.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/channels/wasm/error.rs) — `WasmChannelError` |

---

## 5 · Tools Pipeline
Builtin tools, MCP tools, WASM tools, tool builder, policies.

| Layer | Files |
|-------|-------|
| Framework | [tools/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/mod.rs), [tools/tool.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/tool.rs), [tools/registry.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/registry.rs), [tools/policy.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/policy.rs), [tools/rate_limiter.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/rate_limiter.rs), [tools/intent_display.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/intent_display.rs), [tools/browser_args.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/browser_args.rs) |
| Builtin | [builtin/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/mod.rs), [builtin/file.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/file.rs), [builtin/shell.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/shell.rs), [builtin/http.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/http.rs), [builtin/browser.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/browser.rs), [builtin/memory.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/memory.rs), [builtin/job.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/job.rs), [builtin/routine.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/routine.rs), [builtin/canvas.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/canvas.rs), [builtin/subagent.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/subagent.rs), [builtin/tts.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/tts.rs), [builtin/screen_capture.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/screen_capture.rs), [builtin/camera_capture.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/camera_capture.rs), [builtin/discord_actions.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/discord_actions.rs), [builtin/slack_actions.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/slack_actions.rs), [builtin/telegram_actions.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/telegram_actions.rs), [builtin/agent_control.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/agent_control.rs), [builtin/location.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/location.rs), [builtin/time.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/time.rs), [builtin/json.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/json.rs), [builtin/echo.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/echo.rs), [builtin/skill_tools.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/skill_tools.rs), [builtin/extension_tools.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/extension_tools.rs), [builtin/device_info.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/device_info.rs), [builtin/html_converter.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builtin/html_converter.rs) |
| MCP | [mcp/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/mcp/mod.rs), [mcp/client.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/mcp/client.rs), [mcp/config.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/mcp/config.rs), [mcp/auth.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/mcp/auth.rs), [mcp/protocol.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/mcp/protocol.rs), [mcp/session.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/mcp/session.rs) |
| WASM | [wasm/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/mod.rs), [wasm/wrapper.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/wrapper.rs), [wasm/loader.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/loader.rs), [wasm/host.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/host.rs), [wasm/runtime.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/runtime.rs), [wasm/storage.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/storage.rs), [wasm/capabilities.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/capabilities.rs), [wasm/capabilities_schema.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/capabilities_schema.rs), [wasm/allowlist.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/allowlist.rs), [wasm/credential_injector.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/credential_injector.rs), [wasm/error.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/error.rs), [wasm/limits.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/limits.rs), [wasm/rate_limiter.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/wasm/rate_limiter.rs) |
| Builder | [builder/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builder/mod.rs), [builder/core.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builder/core.rs), [builder/templates.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builder/templates.rs), [builder/validation.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builder/validation.rs), [builder/testing.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tools/builder/testing.rs) |
| Frontend | [OpenClawToolPolicies.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawToolPolicies.tsx) |

---

## 6 · Skills Pipeline
Skill catalog, parser, selector, gating, attenuation, registry.

| Files |
|-------|
| [skills/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/skills/mod.rs), [skills/catalog.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/skills/catalog.rs), [skills/registry.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/skills/registry.rs), [skills/selector.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/skills/selector.rs), [skills/parser.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/skills/parser.rs), [skills/gating.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/skills/gating.rs), [skills/attenuation.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/skills/attenuation.rs) |
| Frontend: [OpenClawSkills.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawSkills.tsx) |

---

## 7 · Workspace & RAG Pipeline
Document ingestion, chunking, embeddings, vector search, citations.

| Files |
|-------|
| [workspace/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/workspace/mod.rs), [workspace/search.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/workspace/search.rs), [workspace/chunker.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/workspace/chunker.rs), [workspace/embeddings.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/workspace/embeddings.rs), [workspace/document.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/workspace/document.rs), [workspace/repository.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/workspace/repository.rs), [workspace/citations.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/workspace/citations.rs), [workspace/hygiene.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/workspace/hygiene.rs), [workspace/lancedb.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/workspace/lancedb.rs), [workspace/sqlite_vec.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/workspace/sqlite_vec.rs), [workspace/qmd.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/workspace/qmd.rs) |
| Frontend: [OpenClawMemory.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawMemory.tsx), [MemoryEditor.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/MemoryEditor.tsx) |

---

## 8 · Media Pipeline
Audio, image, video, PDF, sticker, TTS processing and caching.

| Files |
|-------|
| [media/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/media/mod.rs), [media/audio.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/media/audio.rs), [media/image.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/media/image.rs), [media/video.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/media/video.rs), [media/pdf.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/media/pdf.rs), [media/sticker.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/media/sticker.rs), [media/tts.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/media/tts.rs), [media/tts_streaming.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/media/tts_streaming.rs), [media/cache.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/media/cache.rs), [media/limits.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/media/limits.rs), [media/media_cache_config.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/media/media_cache_config.rs), [media/types.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/media/types.rs) |

---

## 9 · Safety & Security Pipeline
Credential detection, leak detection, sanitization, auth, key rotation, tool policies.

| Files |
|-------|
| [safety/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/mod.rs), [safety/credential_detect.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/credential_detect.rs), [safety/leak_detector.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/leak_detector.rs), [safety/sanitizer.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/sanitizer.rs), [safety/validator.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/validator.rs), [safety/policy.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/policy.rs), [safety/dangerous_tools.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/dangerous_tools.rs), [safety/auth_profiles.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/auth_profiles.rs), [safety/elevated.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/elevated.rs), [safety/key_rotation.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/key_rotation.rs), [safety/device_pairing.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/device_pairing.rs), [safety/media_url.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/media_url.rs), [safety/skill_path.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/safety/skill_path.rs) |
| Docs | [NETWORK_SECURITY.md](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/NETWORK_SECURITY.md) |

---

## 10 · Sandbox Pipeline
Docker/Podman container management, proxy, chromium.

| Files |
|-------|
| [sandbox/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/sandbox/mod.rs), [sandbox/manager.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/sandbox/manager.rs), [sandbox/container.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/sandbox/container.rs), [sandbox/config.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/sandbox/config.rs), [sandbox/detect.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/sandbox/detect.rs), [sandbox/error.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/sandbox/error.rs), [sandbox/podman.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/sandbox/podman.rs), [sandbox/docker_init.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/sandbox/docker_init.rs), [sandbox/docker_chromium.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/sandbox/docker_chromium.rs), [sandbox/proxy/](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/sandbox/proxy) |

---

## 11 · Extensions & Plugins Pipeline
Plugin manager, registry, lifecycle, manifest, ClawHub.

| Files |
|-------|
| [extensions/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/extensions/mod.rs), [extensions/manager.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/extensions/manager.rs), [extensions/registry.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/extensions/registry.rs), [extensions/clawhub.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/extensions/clawhub.rs), [extensions/discovery.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/extensions/discovery.rs), [extensions/lifecycle_hooks.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/extensions/lifecycle_hooks.rs), [extensions/manifest_validator.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/extensions/manifest_validator.rs), [extensions/plugin_manifest.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/extensions/plugin_manifest.rs), [extensions/plugin_interfaces.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/extensions/plugin_interfaces.rs), [extensions/plugin_routes.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/extensions/plugin_routes.rs), [extensions/ext_health_monitor.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/extensions/ext_health_monitor.rs) |
| Frontend: [OpenClawPlugins.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawPlugins.tsx) |

---

## 12 · Hooks Pipeline
Pre/post-processing hooks, bundled hooks, bootstrap hooks.

| Files |
|-------|
| [hooks/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/hooks/mod.rs), [hooks/hook.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/hooks/hook.rs), [hooks/bundled.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/hooks/bundled.rs), [hooks/registry.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/hooks/registry.rs), [hooks/bootstrap.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/hooks/bootstrap.rs) |
| Frontend: [OpenClawHooks.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawHooks.tsx) |

---

## 13 · Context & History Pipeline
Context management, memory, post-compaction, read audits.

| Files |
|-------|
| [context/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/context/mod.rs), [context/manager.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/context/manager.rs), [context/memory.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/context/memory.rs), [context/state.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/context/state.rs), [context/post_compaction.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/context/post_compaction.rs), [context/read_audit.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/context/read_audit.rs) |
| [history/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/history/mod.rs), [history/store.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/history/store.rs), [history/analytics.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/history/analytics.rs) |

---

## 14 · Secrets & Pairing Pipeline
Secret store, crypto, keychain, device pairing, QR pairing.

| Files |
|-------|
| [secrets/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/secrets/mod.rs), [secrets/store.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/secrets/store.rs), [secrets/crypto.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/secrets/crypto.rs), [secrets/keychain.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/secrets/keychain.rs), [secrets/types.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/secrets/types.rs) |
| [pairing/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/pairing/mod.rs), [pairing/store.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/pairing/store.rs) |
| [qr_pairing.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/qr_pairing.rs) *(root-level, not inside `pairing/`)* |
| Frontend: [OpenClawPairing.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawPairing.tsx) |

---

## 15 · Tunnel & Networking Pipeline
Tunneling (Cloudflare, ngrok, Tailscale, custom), hardware bridge.

| Files |
|-------|
| [tunnel/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tunnel/mod.rs), [tunnel/cloudflare.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tunnel/cloudflare.rs), [tunnel/ngrok.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tunnel/ngrok.rs), [tunnel/tailscale.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tunnel/tailscale.rs), [tunnel/custom.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tunnel/custom.rs), [tunnel/none.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tunnel/none.rs) |
| [hardware_bridge.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/hardware_bridge.rs), [tailscale.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tailscale.rs) |

---

## 16 · Setup & Config Pipeline
Setup wizard, config, settings, bootstrap, database, service, CLI.

| Files |
|-------|
| [setup/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/setup/mod.rs), [setup/wizard.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/setup/wizard.rs), [setup/channels.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/setup/channels.rs), [setup/prompts.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/setup/prompts.rs) |
| [config/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/config/mod.rs) + 24 sub-config files |
| [settings.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/settings.rs), [bootstrap.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/bootstrap.rs), [service.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/service.rs) |
| [db/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/db/mod.rs), [db/postgres.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/db/postgres.rs), [db/libsql/](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/db/libsql), [db/libsql_migrations.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/db/libsql_migrations.rs) |
| [cli/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/cli/mod.rs) + 25 CLI subcommands |
| Frontend: [OpenClawConfig.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawConfig.tsx), [OpenClawSystemControl.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawSystemControl.tsx), [OpenClawDoctor.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawDoctor.tsx), [BootstrapModal.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/BootstrapModal.tsx), [RemoteDeployWizard.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/RemoteDeployWizard.tsx) |

---

## 17 · Estimation & Evaluation Pipeline
Cost, time, value estimation; success metrics.

| Files |
|-------|
| [estimation/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/estimation/mod.rs), [estimation/cost.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/estimation/cost.rs), [estimation/time.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/estimation/time.rs), [estimation/value.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/estimation/value.rs), [estimation/learner.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/estimation/learner.rs) |
| [evaluation/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/evaluation/mod.rs), [evaluation/success.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/evaluation/success.rs), [evaluation/metrics.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/evaluation/metrics.rs) |

---

## 18 · Observability Pipeline
Logging, tracing, multi-output.

| Files |
|-------|
| [observability/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/observability/mod.rs), [observability/log.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/observability/log.rs), [observability/multi.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/observability/multi.rs), [observability/traits.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/observability/traits.rs), [observability/noop.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/observability/noop.rs) |
| [tracing_fmt.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tracing_fmt.rs) |
| Frontend: [OpenClawEventInspector.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawEventInspector.tsx) |

---

## 19 · Worker & API Pipeline
Worker runtime, Claude bridge, public API server (chat, sessions, memory, config, extensions, skills, routines, system status).

| Layer | Files |
|-------|-------|
| Worker | [worker/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/worker/mod.rs), [worker/runtime.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/worker/runtime.rs), [worker/claude_bridge.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/worker/claude_bridge.rs), [worker/api.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/worker/api.rs), [worker/proxy_llm.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/worker/proxy_llm.rs) |
| API | [api/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/api/mod.rs), [api/chat.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/api/chat.rs), [api/sessions.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/api/sessions.rs), [api/memory.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/api/memory.rs), [api/config.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/api/config.rs), [api/extensions.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/api/extensions.rs), [api/skills.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/api/skills.rs), [api/routines.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/api/routines.rs), [api/system.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/api/system.rs), [api/error.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/api/error.rs) |

---

## 20 · Registry & Canvas Pipeline
Skill/tool registry, catalog, installer; Canvas window system.

| Files |
|-------|
| [registry/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/registry/mod.rs), [registry/catalog.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/registry/catalog.rs), [registry/installer.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/registry/installer.rs), [registry/manifest.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/registry/manifest.rs), [registry/artifacts.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/registry/artifacts.rs), [registry/embedded.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/registry/embedded.rs) |
| Frontend Canvas: [CanvasWindow.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/canvas/CanvasWindow.tsx), [CanvasProvider.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/canvas/CanvasProvider.tsx), [CanvasToolbar.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/canvas/CanvasToolbar.tsx), [CanvasPanelRenderer.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/canvas/CanvasPanelRenderer.tsx) |
| Frontend Fleet *(see also Pipeline 2)*: [FleetCommandCenter.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/fleet/FleetCommandCenter.tsx), [FleetGraph.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/fleet/FleetGraph.tsx), [AgentNode.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/fleet/AgentNode.tsx), [FleetTerminal.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/fleet/FleetTerminal.tsx) |

---

## 21 · App Core Pipeline
Main entry, app state, talk mode, voice wake, TUI, i18n, testing.

| Files |
|-------|
| [main.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/main.rs), [app.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/app.rs), [lib.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/lib.rs), [boot_screen.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/boot_screen.rs), [talk_mode.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/talk_mode.rs), [voice_wake.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/voice_wake.rs), [tui/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tui/mod.rs), [i18n.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/i18n.rs), [error.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/error.rs), [util.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/util.rs), [testing.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/testing.rs), [update_checker.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/update_checker.rs), [tauri_commands.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/tauri_commands.rs) |

---

## 22 · Orchestrator Pipeline
Container orchestration for sandboxed worker jobs. Internal **HTTP/JSON** API (powered by axum, **not** gRPC) for worker-to-orchestrator communication, per-job bearer token authentication, Docker container lifecycle management, credential grants.

| Layer | Files |
|-------|-------|
| Core | [orchestrator/mod.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/orchestrator/mod.rs) — module root; defines sub-modules and shared types |
| Internal API | [orchestrator/api.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/orchestrator/api.rs) — **HTTP/JSON** API (default port 50051, chosen to avoid conflicts with web gateway :3000; not gRPC) for job status, LLM completions, and event reporting. Workers communicate with the orchestrator through this endpoint. |
| Authentication | [orchestrator/auth.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/orchestrator/auth.rs) — per-job bearer token generation/validation/revocation; manages credential grants so workers can only access secrets explicitly granted to their job |
| Job Management | [orchestrator/job_manager.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/ironclaw/src/orchestrator/job_manager.rs) — Docker container lifecycle (create, start, stop, cleanup); supports Worker and ClaudeCode job modes; manages job state, tokens, and credential grants |

---

## Bug Log

> **Last updated:** 2026-03-19
> Canonical list of all known open, deferred, and recently fixed bugs across pipelines.
> Organized by pipeline. Bugs are numbered sequentially across audit rounds.

### Legend

| Tag | Meaning |
|-----|---------|
| ✅ | Fixed and verified |
| ⏳ | Open — deferred for complexity or low impact |
| 🆕 | Newly discovered in latest audit pass |

---

### Pipeline 1 — LLM Reasoning & Provider

| # | Sev | Status | File | Summary |
|---|-----|--------|------|---------|
| 7 | 🟠 | ✅ | `smart_routing.rs` | Uncertainty detection used fragile string matching — replaced with length heuristic |
| 12 | 🟡 | ✅ | `response_cache.rs` | Cache key generation was correct (system prompt implicitly in messages) |
| 18 | 🟡 | ✅ | `reasoning.rs` | Cache lookup used `read()` instead of `write()` for LRU `get()` |
| 20 | 🔴 | ✅ | `cost_tracker.rs` | CSV injection — unescaped user fields in `export_csv()`. Now RFC 4180 escaped |
| 21 | 🟠 | ✅ | `cost_tracker.rs` | `_this_month` unused — monthly budget alerts never fired. Now wired up |
| 22 | 🟠 | ✅ | `costs.rs` | No Gemini pricing — Gemini users billed at 33× actual cost. Added all Gemini models |
| 27 | 🟡 | ✅ | `heartbeat.rs` | `max_tokens = ctx / 2` wasted context on large-context models — capped at 2048 |
| 30 | 🟡 | ✅ | `compaction.rs` | Compaction LLM calls not tracked — now logs input/output tokens |
| 32 | ℹ️ | ✅ | `cost_tracker.rs` | Unnecessary JSON clone in `from_json()` — added comment, clone unavoidable with serde API |
| 33 | ℹ️ | ✅ | `llm/mod.rs` | Provider chain ordering verified correct (Cache → CB → SmartRouting → Retry → Failover) |
| — | 🟡 | ✅ | `gemini.rs` | Pre-existing test failures (model default bumped to `gemini-3.1-flash` but tests expected `gemini-2.0-flash`) |

### Pipeline 2 — Agent Orchestration

| # | Sev | Status | File | Summary |
|---|-----|--------|------|---------|
| 3 | 🔴 | ✅ | `dispatcher.rs` | Smart routing not applied — LLM provider selected after Reasoning constructed |
| 8 | 🟠 | ✅ | `agent_loop.rs` | Hygiene interval used non-existent `interval_secs` — fixed to `cadence_hours * 3600` |
| 9 | 🟠 | ✅ | `agent_loop.rs` | Memory flush reset logic caused double-flushes |
| 10 | 🟠 | ✅ | `session_manager.rs` | TOCTOU in `prune_stale_sessions` — merged into single-pass collection |
| 13 | 🟠 | ✅ | `dispatcher.rs` | Parallel tool task panic info discarded — now captured from `JoinError` |
| 19 | 🔴 | ✅ | `worker.rs` | Same panic info loss in worker pipeline — fixed like Bug 13 |
| 24 | 🟠 | ✅ | `api/chat.rs` | Spawned task panic now caught via `catch_unwind` — error surfaced to UI |
| 25 | 🟠 | ✅ | `session_manager.rs` | TOCTOU eliminated — single read pass collects user IDs + thread IDs together |
| 26 | 🟡 | ✅ | `worker.rs` | Nudge interval increased from 3/5 to 8/10 to reduce noise |
| 28 | 🟡 | ✅ | `cost_guard.rs` | `to_cents()` used `to_string().parse()` — replaced with `Decimal::try_into()` |
| 29 | 🟡 | ✅ | `router.rs` | `/job` with no args created empty job — now returns `Unknown` |
| 31 | 🟡 | ✅ | `config/mod.rs` | `OBSERVABILITY_BACKEND` bypassed config overlay — now uses `optional_env()` |

### Pipeline 3 — Scheduler & Routines

| # | Sev | Status | File | Summary |
|---|-----|--------|------|---------|
| 34 | 🟠 | ✅ | `scheduler.rs` | Routine/reserved schedule paths now use oneshot-driven cleanup like `schedule()` |
| 35 | 🟡 | ✅ | `scheduler.rs` | Subtask cleanup loop now capped with 10-minute timeout to prevent infinite polling |
| 36 | 🟡 | ✅ | `scheduler.rs` | Routine paths now use `done_tx`/`done_rx` oneshot for event-driven cleanup |

### Pipeline 2b — Subagent Executor

| # | Sev | Status | File | Summary |
|---|-----|--------|------|---------|
| 37 | 🟠 | ✅ | `subagent_executor.rs` | Concurrency check + insert now under single write lock — eliminates TOCTOU |
| 38 | 🟡 | ✅ | `subagent_executor.rs` | Handle tracking entry pre-inserted before `tokio::spawn`, JoinHandle added after |
| 39 | 🟡 | ✅ | `subagent_executor.rs` | `force_text` now triggers at `max_iterations - 2` giving the model a full iteration to respond |

### Backend — Rig Agent

| # | Sev | Status | File | Summary |
|---|-----|--------|------|---------|
| 40 | 🟡 | ✅ | `backend/rig_lib/agent.rs` | `AGENT_NAME` now checks `IRONCLAW_AGENT_NAME` first for config overlay consistency |
| 41 | ℹ️ | ✅ | `backend/rig_lib/agent.rs` | Hardcoded `"gpt-3.5-turbo"` replaced with named `LOCAL_SENTINEL_MODEL` constant |

---

### Summary Statistics

| Category | Count |
|----------|-------|
| **Total bugs found** | 41 |
| **Fixed ✅** | 41 |
| **Open / Deferred ⏳** | 0 |
| **Critical (🔴)** | 4 (all fixed) |
| **Medium (🟠)** | 15 (all fixed) |
| **Low (🟡)** | 17 (all fixed) |
| **Info (ℹ️)** | 5 (all addressed) |

