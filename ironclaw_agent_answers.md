# Answers to IronClaw Agent Questions — Sprint 13 Planning

> **Compiled:** 2026-03-04 10:56 CET  
> **Context:** Scrappy Tauri v2 desktop app (React + Rust), answering from codebase audit

---

## 🔴 Blocking / API Design

### 1. Tauri Command Naming Convention

**Answer: Yes — keep `openclaw_*` prefix for everything.**

All 50+ existing commands follow this convention consistently. The Scrappy frontend uses `invoke('openclaw_*')` exclusively (see [frontend/src/lib/openclaw.ts](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/lib/openclaw.ts)). There is no REST gateway model on the Scrappy side — everything goes through Tauri IPC.

**Recommended names:**

| Module | Command |
|--------|---------|
| Cost tracker | `openclaw_cost_summary` / `openclaw_cost_export_csv` |
| Channel status | `openclaw_channel_status_list` |
| Agent management | **Already exists:** [openclaw_agents_list](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#831-859) (returns `Vec<AgentProfile>`) — add `openclaw_agents_set_default` |
| ClawHub | `openclaw_clawhub_search` / `openclaw_clawhub_install` |
| Routine audit | `openclaw_routine_audit_list` |
| Cache stats | `openclaw_cache_stats` |
| Session export | Add [format](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawChatView.tsx#724-728) param to existing [openclaw_export_session](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/sessions.rs#834-902) (see #7) |
| Plugin lifecycle | `openclaw_plugin_lifecycle_list` |
| Manifest validation | `openclaw_manifest_validate` |

> [!IMPORTANT]
> [openclaw_agents_list](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#831-859) already exists in `rpc.rs:834` — it returns config profiles + injects a `"local-core"` entry if IronClaw is initialized. If you add `set_default`, follow the same `State<'_, OpenClawManager> + State<'_, IronClawState>` signature pattern.

---

### 2. Real-Time vs. Poll for Channel Status

**Answer: Hybrid — SSE push for state changes, with initial poll on mount.**

Our existing pattern is:
- **Poll on mount** (initial data load) via Tauri command
- **SSE push** for live state changes via `openclaw-event`

Evidence from codebase:
- [OpenClawChannels.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawChannels.tsx) polls [openclaw_channels_list](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#216-302) on mount then listens to `openclaw-event` for WhatsApp QR/status updates
- [OpenClawChatView.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawChatView.tsx) subscripts to `openclaw-event` for real-time message streaming
- [OpenClawEventInspector.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawEventInspector.tsx) listens to `openclaw-event` for all events
- [OpenClawSidebar.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawSidebar.tsx) polls status every 5 seconds as a heartbeat fallback
- [OpenClawDashboard.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawDashboard.tsx) polls every 5 seconds

**Recommendation:**
1. Wire `ChannelStatusView` state changes into the existing `openclaw-event` SSE pipeline with `kind: "ChannelStatus"` — this gives real-time state transitions (Running → Degraded → Reconnecting etc.)
2. Add `openclaw_channel_status_list` as a poll command for initial mount + refresh button
3. Scrappy will subscribe to both: `openclaw-event` for live updates, polling fallback at 10s

The SSE pipeline is already built: [TauriChannel](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/ironclaw_channel.rs#45-54) emits via `AppHandle::emit("openclaw-event", ...)` — just emit a new event variant.

---

### 3. Agent Management — Sidebar Picker Design

**Answer: Option C — Both via a dedicated component, ultimately a dropdown + panel.**

The UX vision:
1. **Sidebar dropdown** (compact, always visible) — shows current agent name + colored dot for status. Click to switch active agent (like the existing model selector pattern in TUI).
2. **Full panel** (in settings or dedicated view) — shows all agents with individual controls (add, remove, pause/resume, set-default, status badges).

**For the Tauri command response shape:**

```json
{
  "agents": [
    {
      "id": "local-core",
      "name": "Local Core",
      "url": "embedded://ironclaw",
      "mode": "embedded",
      "is_default": true,
      "status": "running",        // running | paused | error | offline
      "session_count": 12,
      "last_active_at": "2026-03-04T10:00:00Z"
    }
  ]
}
```

The existing [openclaw_agents_list](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#831-859) returns `Vec<AgentProfile>` which has `id, name, url, token, mode, auto_connect`. You'd need to **extend** (not replace) with `is_default`, [status](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/ironclaw_channel.rs#153-185), `session_count`, `last_active_at` fields. If that changes the [AgentProfile](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/lib/openclaw.ts#69-77) struct, add the new fields as `Option<>` to maintain backwards compat.

---

### 4. Gmail OAuth Flow

**Answer: Scrappy already has a working OAuth 2.0 PKCE flow — use it.**

Scrappy has `cloud_oauth_start` / `cloud_oauth_complete` commands (in [backend/src/cloud/commands.rs](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/cloud/commands.rs), exposed via [bindings.ts](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/lib/bindings.ts)). The flow:

1. Frontend calls `cloudOauthStart("gmail")` → backend returns `{ auth_url, code_verifier }`
2. Frontend opens `auth_url` in system browser (via `tauri::shell::open`)
3. User authenticates → Google redirects to localhost callback
4. Frontend passes [(code, code_verifier)](file:///Users/mt/Programming/Schtack/ironclaw/ironclaw/src/channels/channel.rs#38-56) to `cloudOauthComplete("gmail", code, codeVerifier)`
5. Backend exchanges code for tokens + stores in Keychain via `KeychainSecretsAdapter`

**What IronClaw should provide:**
- Google OAuth client credentials (client_id, redirect_uri, scopes) via the same `oauth_defaults.rs` pattern
- A `gmail` variant in whatever provider enum `cloud_oauth_start` dispatches on
- **Do NOT build a separate `/auth/gmail` gateway endpoint** — Scrappy handles the browser flow natively

---

### 5. ClawHub — Direct vs. Proxied

**Answer: Proxy through IronClaw gateway.**

Reasons:
1. **Security** — `CLAWHUB_API_KEY` should never touch the frontend. Scrappy stores secrets in macOS Keychain via `KeychainSecretsAdapter`; the key should stay server-side.
2. **Consistency** — all other API calls go through Tauri IPC → IronClaw. No frontend makes direct HTTP calls to external services.
3. **Caching** — the `CatalogCache` with TTL is on the IronClaw side, so the gateway benefits from it.

**Recommended commands:**
- `openclaw_clawhub_search` — takes query + optional filters, returns catalog entries
- `openclaw_clawhub_install` — takes plugin ID, installs to `~/.ironclaw/tools/`

No need for a full REST `/api/clawhub/search` endpoint — Tauri IPC is sufficient since Scrappy is the only consumer.

---

## 🟡 UX / Design

### 6. LLM Cost Dashboard — Data Granularity

**Answer: Full data via one endpoint; the frontend will pick what to show.**

Expose a single `openclaw_cost_summary` that returns the full struct:

```json
{
  "total_cost_usd": 12.34,
  "daily": { "2026-03-04": 1.23, "2026-03-03": 2.45 },
  "monthly": { "2026-03": 8.90 },
  "by_model": { "claude-3.5-sonnet": 5.67, "gpt-4o": 3.21 },
  "by_agent": { "local-core": 10.12 },
  "alert_threshold_usd": 50.0,
  "alert_triggered": false
}
```

The frontend will initially show:
- **Summary card**: total spend today / this month
- **Per-model breakdown** (bar chart or table)
- **Alert badge** if threshold exceeded

We'll add `by_agent` breakdown and CSV export in a follow-up. Better to get the full data now and progressively enhance the UI.

---

### 7. Session Export Format Picker

**Answer: Add [format](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawChatView.tsx#724-728) parameter to existing [openclaw_export_session](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/sessions.rs#834-902) command.**

The current command signature is:
```rust
pub async fn openclaw_export_session(
    ironclaw: State<'_, IronClawState>,
    session_key: String,
) -> Result<SessionExportResponse, String>
```

**Change to:**
```rust
pub async fn openclaw_export_session(
    ironclaw: State<'_, IronClawState>,
    session_key: String,
    format: Option<String>,  // "md" | "json" | "csv" | "html" | "txt", default "md"
) -> Result<SessionExportResponse, String>
```

No need for a separate command — `Option<String>` defaults to `"md"` for backward compat.

**Output destination:** The frontend currently saves to clipboard. We'll add a dropdown: **Clipboard** (default) | **Save to file** (via Tauri `dialog::save`). Both options available from the same export button.

---

### 8. Routine Audit Log — Pagination

**Answer: Last N runs per routine, with optional outcome filter.**

For the automations panel:
- **Default:** Last 20 runs per routine (matches existing [openclaw_cron_history](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#201-211) which takes [limit](file:///Users/mt/Programming/Schtack/ironclaw/ironclaw/src/pairing/store.rs#319-332))
- **Filter:** By outcome (success/failure/all)
- **No full pagination needed** — the ring-buffer is bounded, and users care about recent runs

**Command shape:**
```rust
pub async fn openclaw_routine_audit_list(
    routine_key: String,
    limit: Option<u32>,           // default 20
    outcome: Option<String>,      // "success" | "failure" | null (= all)
) -> Result<Vec<RoutineAuditEntry>, String>
```

---

### 9. LLM Routing Rule Builder — Power-User vs. Mainstream

**Answer: Advanced settings, collapsed by default. Target Sprint 14 (not immediate).**

The 6 rule types (LargeContext, Vision, CostOptimized, LowestLatency, RoundRobin, Fallback) are power-user territory. The UX:

1. **Mainstream tier:** A "Smart Routing" toggle in provider settings. On = use IronClaw's default routing. Off = manual model selection only.
2. **Advanced tier:** Expandable panel below the toggle for custom rules. Each rule gets a row: condition → provider → priority. Collapsed by default.

**Priority:** This is Tier 4 #25 but not urgent. The 1-click "Smart Routing" toggle is Sprint 13 material; the full rule builder is Sprint 14. Ship the command interface now, UI later.

---

## 🟢 State / Progress

### 10. What Has Scrappy Already Started from Tier 4?

**Of the 12 new todos (#17–#28), here's the current state:**

| # | Feature | Status |
|---|---------|--------|
| 17 | Multi-agent picker | **Not started.** No `AgentSwitcher.tsx`. However, [openclaw_agents_list](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#831-859) already exists and returns `Vec<AgentProfile>`, so the backend hook is partially there. |
| 18 | LLM cost dashboard | **Not started.** No cost/billing views exist. |
| 19 | Channel status panel | **Partially done.** [OpenClawChannels.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawChannels.tsx) reads `enabled/disabled` + type + stream_mode from [openclaw_channels_list](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#216-302). It does NOT read uptime/state/counters — those would be new fields. |
| 20 | ClawHub browser | **Not started.** `OpenClawPlugins.tsx` exists for local plugin management but has no ClawHub integration. |
| 21 | Routine run history | **Not started.** [OpenClawAutomations.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawAutomations.tsx) has a [handleViewHistory()](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawAutomations.tsx#141-151) that calls [openclaw_cron_history(key, 10)](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#201-211) — but [openclaw_cron_history](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#201-211) currently returns `[]` (placeholder). Wire this to `RoutineAuditLog`. |
| 22 | Gmail channel card | **Not started.** [OpenClawChannels.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawChannels.tsx) doesn't mention Gmail. Adding it is trivial — the card grid is data-driven. |
| 23 | Extension health badges | **Not started.** No health indicators on channel/plugin cards. |
| 24 | Session export format picker | **Not started.** Currently markdown-only with clipboard copy. |
| 25 | LLM routing rule builder | **Not started.** |
| 26 | Plugin lifecycle log | **Not started.** `OpenClawPlugins.tsx` has no lifecycle tab. |
| 27 | Manifest validation | **Not started.** |
| 28 | Response cache stats | **Not started.** |

**Summary:** Only #19 and #21 have partial hooks. Everything else is greenfield on the Scrappy side.

---

### 11. Test Coverage for IronClaw Command Bindings

**There IS a test file:** [frontend/src/tests/lib/openclaw.test.ts](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/tests/lib/openclaw.test.ts) (209 lines, Vitest).

**Pattern:**
- Mocks `invoke` from `@tauri-apps/api/core`
- Each test calls the exported wrapper function
- Asserts that `mockInvoke` was called with the correct command name + payload

**Currently tested commands:**
- [openclaw_get_status](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/gateway.rs#15-252)
- [openclaw_start_gateway](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/gateway.rs#297-391) / [openclaw_stop_gateway](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/gateway.rs#392-413)
- `openclaw_save_gateway_settings`
- `openclaw_toggle_node_host` / `openclaw_toggle_local_inference`
- `openclaw_add_agent_profile` / `openclaw_remove_agent_profile`
- [openclaw_save_cloud_config](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#580-608)
- `openclaw_delete_session` / `openclaw_get_sessions`
- [openclaw_config_patch](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#385-404)
- `openclaw_test_connection`

**Not yet tested** (newer commands): [openclaw_channels_list](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#216-302), [openclaw_cron_lint](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#307-331), `openclaw_memory_search`, [openclaw_export_session](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/sessions.rs#834-902), and all Sprint 12 additions.

**Follow the same pattern:** mock `invoke`, assert command name + args, verify return type. New commands should get corresponding test stubs when the frontend wrapper is added.

---

### 12. Broken or Inconsistent IronClaw Commands

**Known issues from the Scrappy integration layer:**

1. **[openclaw_cron_history](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#201-211) is a stub** — Returns `[]` always. The comment in `rpc.rs:203` says "history not yet exposed through IronClaw API". If `RoutineAuditLog` is ready, this command needs to be wired to it. The frontend ([handleViewHistory()](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawAutomations.tsx#141-151) in [OpenClawAutomations.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawAutomations.tsx)) already calls it with [(key, limit)](file:///Users/mt/Programming/Schtack/ironclaw/ironclaw/src/channels/channel.rs#38-56) args but gets nothing back.

2. **[openclaw_agents_list](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#831-859) response shape** — Currently returns bare `Vec<AgentProfile>` (which only has `id, name, url, token, mode, auto_connect`). Missing: `is_default`, [status](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/ironclaw_channel.rs#153-185), `session_count`. The frontend sidebar ([OpenClawSidebar.tsx](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/components/openclaw/OpenClawSidebar.tsx)) calls this but can't show status badges without these fields. Consider extending [AgentProfile](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/lib/openclaw.ts#69-77) with `Option<>` fields.

3. **No response type mismatches detected** — All other commands return the expected shapes. The `specta` type generation keeps frontend/backend types in sync via [bindings.ts](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/frontend/src/lib/bindings.ts).

4. **[openclaw_channels_list](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/rpc.rs#216-302) doesn't need [IronClawState](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/ironclaw_bridge.rs#43-51)** — Originally I wrote it to use `ironclaw.agent().config()` which doesn't exist. It was rewritten to use [OpenClawManager](file:///Users/mt/Programming/Schtack/scrappy-cursor%20Kopie%203/backend/src/openclaw/commands/mod.rs#34-40) + env vars. If IronClaw can expose a `channels_status()` API on the Agent, that would be cleaner than reading env vars for Discord/Signal/Nostr.

---

## Summary: Time-Sensitive Answers

| # | Question | Answer |
|---|----------|--------|
| **1** | Naming | `openclaw_*` prefix. No REST gateway. |
| **2** | SSE vs poll | **Hybrid:** SSE push via `openclaw-event` + poll for initial mount. Emit `kind: "ChannelStatus"` events. |
| **4** | Gmail OAuth | Use existing `cloud_oauth_start` / `cloud_oauth_complete` PKCE flow. Add `"gmail"` provider variant. No separate gateway endpoint. |
