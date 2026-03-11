# IronClaw → Scrappy Data Contract
**Everything IronClaw exposes to the Scrappy desktop UI**

Last updated: 2026-03-07

---

## Transport layer

IronClaw exposes data to Scrappy through **two parallel channels**:

| Channel | When | Used for |
|---|---|---|
| **Tauri commands** (`openclaw_*`) | On-demand polling by the UI | Dashboards, summaries, CRUD |
| **SSE events** (`openclaw-event`) | Real-time push from IronClaw | Live status updates, streaming |

---

## Tauri Commands (20 total)

### 💰 Cost Dashboard

#### `openclaw_cost_summary` → `CostSummary`
Real LLM spend data — **now populated by every agent LLM call** via `Reasoning::record_cost`.

| Field | Type | Description |
|---|---|---|
| `total_cost_usd` | `f64` | All-time spend |
| `daily` | `BTreeMap<String, f64>` | Per-day spend (key = `"2026-03-07"`) |
| `monthly` | `BTreeMap<String, f64>` | Per-month spend (key = `"2026-03"`) |
| `by_model` | `BTreeMap<String, f64>` | Spend per model name |
| `by_agent` | `BTreeMap<String, f64>` | Spend per agent ID |
| `alert_threshold_usd` | `Option<f64>` | Daily budget limit (from env `DAILY_BUDGET_USD`) |
| `alert_triggered` | `bool` | True if today's spend ≥ 90% of limit |

**Source:** `Arc<Mutex<CostTracker>>` shared from `AppComponents` → `AgentDeps` → `Reasoning`

---

#### `openclaw_cost_export_csv` → `String`
Raw CSV of every LLM call. Headers: `timestamp, agent_id, provider, model, input_tokens, output_tokens, cost_usd, request_id`

---

### 🔌 ClawHub Extension Catalog

#### `openclaw_clawhub_search(query)` → `Vec<CatalogEntry>`
Searches the pre-fetched ClawHub catalog (populated at startup via background HTTP fetch).

| Field | Type |
|---|---|
| `name` | `String` — package ID |
| `display_name` | `String` |
| `description` | `String` |
| `version` | `Option<String>` |
| `author` | `Option<String>` |
| `download_url` | `Option<String>` |
| `tags` | `Vec<String>` |

---

#### `openclaw_clawhub_install(plugin_id)` → `InstallResult`
Resolves the install path for a plugin. Scrappy performs the actual HTTP download.

| Field | Type |
|---|---|
| `plugin_name` | `String` |
| `version` | `String` |
| `install_path` | `String` |
| `success` | `bool` |
| `message` | `String` |

---

### 📋 Routine Audit Log

#### `openclaw_routine_audit_list(routine_key, limit?, outcome?)` → `Vec<RoutineRun>`
History of routine executions, filterable by outcome (`"ok"`, `"failed"`, `"attention"`).

| Field | Type |
|---|---|
| `id` | `Uuid` |
| `routine_id` | `Uuid` |
| `trigger_type` | `String` — `"cron"`, `"event"`, `"manual"`, `"webhook"` |
| `trigger_detail` | `Option<String>` |
| `started_at` | `DateTime<Utc>` |
| `completed_at` | `Option<DateTime<Utc>>` |
| `status` | `RunStatus` — `running/ok/attention/failed` |
| `result_summary` | `Option<String>` — LLM output snippet |
| `tokens_used` | `Option<i32>` |
| `job_id` | `Option<Uuid>` — links to full job if `FullJob` mode |

#### `openclaw_routine_create(RoutineCreateParams)` → `Routine`
Creates and persists a new routine as `~/.ironclaw/routines/{id}.json`.

Params: `name, description, user_id, trigger (Cron/Event/Webhook/Manual), action (Lightweight/FullJob), notify?`

---

### 💾 LLM Response Cache

#### `openclaw_cache_stats` → `CacheStats`

| Field | Type |
|---|---|
| `hits` | `u64` |
| `misses` | `u64` |
| `evictions` | `u64` |
| `size` | `usize` — current entry count |
| `hit_rate` | `f64` — `hits / (hits + misses)` |

---

### 🔌 Plugin Lifecycle Audit

#### `openclaw_plugin_lifecycle_list` → `Vec<SerializedLifecycleEvent>`
All plugin install/uninstall/enable/disable events captured by `AuditLogHook`.

| Field | Type |
|---|---|
| `event_type` | `String` — `"installed"`, `"uninstalled"`, `"enabled"`, `"disabled"`, `"error"` |
| `plugin_name` | `String` |
| `timestamp` | `String` (ISO 8601) |
| `details` | `Option<String>` — error message or version |

---

#### `openclaw_manifest_validate(PluginInfoRef)` → `ValidationResponse`
Validates a plugin manifest before install.

| Field | Type |
|---|---|
| `errors` | `Vec<String>` — blocking issues |
| `warnings` | `Vec<String>` — non-blocking notes |

---

### 🔄 LLM Routing Policy

#### `openclaw_routing_status` → `RoutingStatusResponse`

| Field | Type |
|---|---|
| `enabled` | `bool` |
| `default_provider` | `String` |
| `rule_count` | `usize` |
| `rules` | `Vec<RoutingRuleSummary>` |
| `latency_data` | `Vec<{ provider, avg_latency_ms }>` — real measured latencies |

#### `openclaw_routing_rules_list` → `Vec<RoutingRuleSummary>`
Each entry: `{ index, rule_type, description }`

#### `openclaw_routing_rules_add(rule, position?)` → `Vec<RoutingRuleSummary>`
Rule types: `LargeContext`, `VisionContent`, `Fallback`, `RoundRobin`, `ModelOverride`, `ChannelBased`

#### `openclaw_routing_rules_remove(index)` → `Vec<RoutingRuleSummary>`

#### `openclaw_routing_rules_reorder(from, to)` → `Vec<RoutingRuleSummary>`

---

### 📧 Gmail Channel

#### `openclaw_gmail_status` → `GmailStatusResponse`

| Field | Type |
|---|---|
| `enabled` | `bool` |
| `configured` | `bool` |
| `status` | `String` — human-readable state |
| `project_id` | `String` |
| `subscription_id` | `String` |
| `label_filters` | `Vec<String>` |
| `allowed_senders` | `Vec<String>` |
| `missing_fields` | `Vec<String>` — what's not yet configured |
| `oauth_configured` | `bool` |

#### `openclaw_gmail_oauth_start` → `GmailOAuthResult`
Runs the full PKCE OAuth flow (opens browser, waits for callback, exchanges code for tokens).

| Field | Type |
|---|---|
| `success` | `bool` |
| `access_token` | `Option<String>` |
| `refresh_token` | `Option<String>` |
| `expires_in` | `Option<u64>` |
| `scope` | `Option<String>` |
| `error` | `Option<String>` |

---

### 🎨 Canvas Panels (A2UI)

#### `openclaw_canvas_panels_list` → `Vec<{ panel_id, title }>`
All active panels the agent has emitted.

#### `openclaw_canvas_panel_get(panel_id)` → `Option<CanvasPanelData>`

| Field | Type |
|---|---|
| `panel_id` | `String` |
| `title` | `String` |
| `components` | `serde_json::Value` — full panel component tree |
| `metadata` | `Option<serde_json::Value>` |

#### `openclaw_canvas_panel_dismiss(panel_id)` → `bool`

---

### 📡 Channel Status

#### `openclaw_channel_status_list` → `Vec<ChannelStatusEntry>`
Live data from atomic counters in `ChannelManager` — real message counts, not estimates.

| Field | Type |
|---|---|
| `name` | `String` — channel name (e.g. `"telegram"`) |
| `channel_type` | `String` |
| `state` | `ChannelViewState` — `Running{uptime_secs}`, `Connecting`, `Reconnecting`, `Failed`, `Disabled`, `Draining` |
| `last_message_at` | `Option<String>` |
| `last_error` | `Option<String>` |
| `messages_received` | `u64` — atomic counter |
| `messages_sent` | `u64` — atomic counter |
| `errors` | `u32` — atomic counter |

---

## SSE Push Events (`openclaw-event`)

IronClaw emits these via `AppHandle::emit("openclaw-event", ...)` — Scrappy subscribes for real-time updates:

| `kind` field | Payload | Trigger |
|---|---|---|
| `"ChannelStatus"` | `{ channel, state, timestamp }` | Channel state changes |
| `"RoutineLifecycle"` | `{ routine_id, name, status, trigger_type, summary? }` | Routine fires start/complete |
| `"CostAlert"` | `{ daily_spend, threshold, utilization }` | Budget ≥ 90% consumed |
| `"ExtensionEvent"` | `{ plugin_name, event_type, details? }` | Plugin install/uninstall |

---

## AppComponents — What IronClaw Initialises

These are the shared objects that back all the commands above:

| Component | Type | Role |
|---|---|---|
| `llm` | `Arc<dyn LlmProvider>` | Primary chat LLM |
| `cheap_llm` | `Option<Arc<dyn LlmProvider>>` | Fast LLM for heartbeat/routing |
| `cost_tracker` | `Arc<Mutex<CostTracker>>` | Real-time cost accumulator |
| `cost_guard` | `Arc<CostGuard>` | Budget enforcement before each LLM call |
| `tools` | `Arc<ToolRegistry>` | All registered tools |
| `hooks` | `Arc<HookRegistry>` | Hook event bus |
| `audit_hook` | `Arc<AuditLogHook>` | Plugin lifecycle event log |
| `workspace` | `Option<Arc<Workspace>>` | AGENTS.md, SOUL.md, workspace files |
| `extension_manager` | `Option<Arc<ExtensionManager>>` | WASM extension lifecycle + catalog_cache |
| `context_manager` | `Arc<ContextManager>` | Per-job state and memory |
| `skill_registry` | `Option<Arc<RwLock<SkillRegistry>>>` | Loaded prompt skills |
| `db` | `Option<Arc<dyn Database>>` | Persistent store (libSQL or Postgres) |
| `log_broadcaster` | `Arc<LogBroadcaster>` | Streaming log lines to UI |
| `tool_bridge` | `Option<Arc<dyn ToolBridge>>` | Screen capture, camera, mic (OS grants) |
| `session_approvals` | `Arc<SessionApprovals>` | Per-session user grants |
| `mcp_session_manager` | `Arc<McpSessionManager>` | MCP tool server sessions |

---

## What's Still Wired Only Inside AppComponents (not yet a Tauri command)

These are fully populated but not yet exposed via a command — potential future additions:

| Data | Where | Notes |
|---|---|---|
| Hook registry contents | `hooks.list_with_details()` | Hooks installed page |
| Skill list | `skill_registry.read()` | Skills toggle page |
| Log stream | `log_broadcaster` | Already used by web gateway, not Tauri |
| MCP sessions | `mcp_session_manager` | Could expose server status |
| `CostGuard` live state | `cost_guard` | Hourly rate limit counters |
| `CachedResponseStore` | `CachedResponseStore` | Cache stats command already exists but wrapper not yet written to |
