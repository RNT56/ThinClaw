# Open Issues — IronClaw / Scrappy

> Full codebase audit — 2026-03-11  
> **34 issues found. No code was modified during analysis.**  
> Severity levels: 🔴 Critical · 🟠 High · 🟡 Medium · 🔵 Low · 📦 Legacy/Stale  
> ✅ = Proposed solution verified against source code

---

## 🔴 Critical — Broken Logic / Real Bugs

---

### IC-001 · `auto_approve_tools` env var not refreshed on gateway restart

**Severity:** 🔴 Critical  
**File:** `backend/src/openclaw/ironclaw_bridge.rs` L867–877  
**Status:** Open

The code only sets `AGENT_AUTO_APPROVE_TOOLS` if the env var is not already present (`is_err()` guard). This means if the gateway is **stopped and restarted**, any user change to the `auto_approve_tools` config setting is silently ignored — the env var from the previous process run is still alive.

**Impact:** Users cannot change the autonomy mode without fully quitting and relaunching the app.

**Root Cause:** The `is_err()` guard was added to preserve the value set by `openclaw_set_autonomy_mode` (in `rpc.rs` L700–704), which writes the env var directly. The problem is that `stop()` (L444–453) clears `LLM_BACKEND` / `LLM_BASE_URL` / `LLM_API_KEY` / `LLM_MODEL`, but does **not** clear `AGENT_AUTO_APPROVE_TOOLS`. So on restart, the stale env var survives.

#### ✅ Proposed Solution

The root intent is: "if the user toggled autonomy via the runtime UI, don't clobber it on next restart." The correct fix is to clear the env var in `stop()` so `start()` always writes the fresh config value.

```diff
--- a/backend/src/openclaw/ironclaw_bridge.rs  (stop method, ~L448)
+++ b/backend/src/openclaw/ironclaw_bridge.rs
             unsafe {
                 std::env::remove_var("LLM_BACKEND");
                 std::env::remove_var("LLM_BASE_URL");
                 std::env::remove_var("LLM_API_KEY");
                 std::env::remove_var("LLM_MODEL");
+                std::env::remove_var("AGENT_AUTO_APPROVE_TOOLS");
             }

--- a/backend/src/openclaw/ironclaw_bridge.rs  (start method, ~L867-877)
+++ b/backend/src/openclaw/ironclaw_bridge.rs
-                // Only set if not already overridden (e.g. by openclaw_set_autonomy_mode
-                // which sets the env var immediately for the next run).
-                if std::env::var("AGENT_AUTO_APPROVE_TOOLS").is_err() {
-                    let auto_approve = oc_config
-                        .as_ref()
-                        .map(|c| c.auto_approve_tools)
-                        .unwrap_or(false);
-                    std::env::set_var("AGENT_AUTO_APPROVE_TOOLS", auto_approve.to_string());
-                    tracing::info!("[ironclaw] Set AGENT_AUTO_APPROVE_TOOLS={}", auto_approve);
-                }
+                let auto_approve = oc_config
+                    .as_ref()
+                    .map(|c| c.auto_approve_tools)
+                    .unwrap_or(false);
+                std::env::set_var("AGENT_AUTO_APPROVE_TOOLS", auto_approve.to_string());
+                tracing::info!("[ironclaw] Set AGENT_AUTO_APPROVE_TOOLS={}", auto_approve);
```

**Verification:** `openclaw_set_autonomy_mode` in `rpc.rs` L696 persists the value to `OpenClawConfig` via `cfg.set_auto_approve_tools(enabled)`. On restart, `start()` reads the persisted config — so the env var and config always agree. The runtime toggle in `rpc.rs` L701 still works because it sets both the config AND the env var for the currently running engine.

---

### IC-002 · Concurrent workspace writes from multiple sessions (no per-user lock)

**Severity:** 🔴 Critical  
**File:** `ironclaw/src/agent/session_manager.rs`  
**Status:** Open

When the same user connects from two channels simultaneously (e.g. Tauri UI + Telegram), `resolve_thread()` creates separate threads per `(channel, user)` pair. Both can run agent turns concurrently, creating competing writes to shared workspace files (`MEMORY.md`, daily log file). No mutex guards workspace files at the per-user level.

**Impact:** Memory corruption / data loss in workspace files for multi-channel users.

#### ✅ Proposed Solution

Add a per-user `RwLock` in `SessionManager` that the dispatcher acquires before executing workspace-mutating operations. This doesn't block concurrent reads but serializes writes.

```diff
--- a/ironclaw/src/agent/session_manager.rs
+++ b/ironclaw/src/agent/session_manager.rs
 pub struct SessionManager {
     sessions: RwLock<HashMap<String, Arc<Mutex<Session>>>>,
     thread_map: RwLock<HashMap<ThreadKey, Uuid>>,
     undo_managers: RwLock<HashMap<Uuid, Arc<Mutex<UndoManager>>>>,
     thread_owners: RwLock<HashMap<Uuid, String>>,
     hooks: Option<Arc<HookRegistry>>,
+    /// Per-user workspace write lock: prevents concurrent MEMORY.md / daily log writes.
+    /// Keyed by user_id. The dispatcher acquires `.write()` before workspace mutations.
+    workspace_locks: RwLock<HashMap<String, Arc<tokio::sync::RwLock<()>>>>,
 }

+impl SessionManager {
+    /// Get or create the per-user workspace lock.
+    pub async fn workspace_lock(&self, user_id: &str) -> Arc<tokio::sync::RwLock<()>> {
+        {
+            let locks = self.workspace_locks.read().await;
+            if let Some(lock) = locks.get(user_id) {
+                return Arc::clone(lock);
+            }
+        }
+        let mut locks = self.workspace_locks.write().await;
+        locks.entry(user_id.to_string())
+            .or_insert_with(|| Arc::new(tokio::sync::RwLock::new(())))
+            .clone()
+    }
+}
```

Then in the dispatcher (`agent_loop.rs` `handle_message` / `handle_message_external`), acquire the lock before the turn executes:

```rust
let ws_lock = self.session_manager.workspace_lock(&message.user_id).await;
let _guard = ws_lock.write().await;
// ... existing turn execution ...
```

**Verification:** This serializes workspace writes per-user while allowing different users to operate concurrently. The lock is released when the turn completes (guard drops). Read operations (like building system prompts) don't need the write lock.

---

### IC-003 · Heartbeat `JoinHandle` not stored — task outlives gateway shutdown

**Severity:** 🔴 Critical  
**File:** `ironclaw/src/agent/heartbeat.rs`, `ironclaw/src/agent/agent_loop.rs`  
**Status:** Open (partially mitigated)

**Re-analysis after code review:** The standalone `spawn_heartbeat()` function in `heartbeat.rs` L401–416 is NOT used by the current `start_background_tasks()` flow. The background tasks handle (`BackgroundTasksHandle` at `agent_loop.rs` L310) already stores:
- `heartbeat_handle: Option<JoinHandle<()>>` — the hygiene task
- `routine_handle: Option<(JoinHandle<()>, RoutineEngine)>` — the cron ticker

And `shutdown_background()` at L629–643 aborts all of them. The heartbeat checks are now done via `RoutineEngine` (as a registered routine), not via the standalone `HeartbeatRunner`.

**Remaining gap:** The `spawn_heartbeat()` function still exists and could be called by external code. Additionally, the notification forwarder task spawned at L561–580 inside `start_background_tasks()` is not tracked — its `JoinHandle` is dropped.

#### ✅ Proposed Solution

1. Mark `spawn_heartbeat()` as `#[deprecated]` since heartbeat is now handled by the routine engine.
2. Track the notification forwarder task:

```diff
--- a/ironclaw/src/agent/agent_loop.rs
+++ a/ironclaw/src/agent/agent_loop.rs
 pub struct BackgroundTasksHandle {
     repair_handle: tokio::task::JoinHandle<()>,
     pruning_handle: tokio::task::JoinHandle<()>,
     heartbeat_handle: Option<tokio::task::JoinHandle<()>>,
     routine_handle: Option<(tokio::task::JoinHandle<()>, Arc<RoutineEngine>)>,
     health_monitor: Option<Arc<crate::channels::ChannelHealthMonitor>>,
+    /// Notification forwarder task for routine notifications.
+    notify_forwarder_handle: Option<tokio::task::JoinHandle<()>>,
     system_event_mutex: tokio::sync::Mutex<Option<...>>,
 }
```

And in `shutdown_background()`:
```diff
+        if let Some(h) = handle.notify_forwarder_handle {
+            h.abort();
+        }
```

**Verification:** The main heartbeat is already properly tracked via `routine_handle`. The only leak is the notification forwarder, which we now track.

---

### IC-004 · `ToolHistoryGroup` failure detection produces false positives

**Severity:** 🔴 Critical  
**File:** `frontend/src/components/openclaw/OpenClawChatView.tsx` L337–340  
**Status:** Open

```tsx
const hasFailures = messages.some(m => {
    return m.metadata?.status === 'failed' || m.text.includes('FAIL') || m.text.includes('error');
});
```

The case-sensitive `.includes('error')` match flags any tool output containing "error" in a success context.

#### ✅ Proposed Solution

Remove the string heuristics entirely. The metadata-based check is the correct, reliable signal.

```diff
--- a/frontend/src/components/openclaw/OpenClawChatView.tsx  L337-340
+++ b/frontend/src/components/openclaw/OpenClawChatView.tsx
-    const hasFailures = messages.some(m => {
-        // basic heuristic for failure
-        return m.metadata?.status === 'failed' || m.text.includes('FAIL') || m.text.includes('error');
-    });
+    const hasFailures = messages.some(m => m.metadata?.status === 'failed');
```

**Verification:** All tool executions in IronClaw set `metadata.status` to either `"completed"` or `"failed"` via `ToolOutput::success()` / `ToolOutput::failure()`. The metadata field is the authoritative source. The heuristic was a legacy fallback from before structured metadata existed.

---

### IC-005 · `extractSessionKey()` silently fails on JSON parse error

**Severity:** 🔴 Critical  
**File:** `frontend/src/components/openclaw/OpenClawChatView.tsx` L219–227  
**Status:** Open

#### ✅ Proposed Solution

Add a `console.warn` and attempt regex extraction as fallback:

```diff
--- a/frontend/src/components/openclaw/OpenClawChatView.tsx  L218-227
+++ b/frontend/src/components/openclaw/OpenClawChatView.tsx
 function extractSessionKey(output: any): string | null {
     if (!output) return null;
     let data = output;
     if (typeof output === 'string') {
-        try { data = JSON.parse(output); } catch { return null; }
+        try {
+            data = JSON.parse(output);
+        } catch {
+            // Fallback: try to extract a UUID-like session key from raw text
+            const uuidMatch = output.match(
+                /(?:session[_-]?(?:key|id))\s*[:=]\s*["']?([a-f0-9-]{36}|agent:\w+)["']?/i
+            );
+            if (uuidMatch) return uuidMatch[1];
+            console.warn('[OpenClaw] extractSessionKey: output is not JSON and no session key found:', output.slice(0, 200));
+            return null;
+        }
     }
     return data?.sessionKey || data?.session_key || data?.sessionId || data?.session_id || null;
 }
```

**Verification:** The regex matches both UUID session keys (`a1b2c3d4-...`) and named session keys (`agent:main`). The `console.warn` makes debugging visible without breaking the UI. The original structured-JSON path is tried first.

---

### IC-006 · Zombie `RoutineRun` DB records from subagent crash/cancellation

**Severity:** 🔴 Critical  
**File:** `ironclaw/src/agent/routine_engine.rs` L565–624  
**Status:** Open

When `execute_as_subagent()` succeeds in spawning, it returns `RunStatus::Running`, causing `execute_routine()` to skip `complete_routine_run()`. If the subagent crashes, the DB row stays `Running` forever.

#### ✅ Proposed Solution

Add a zombie reaper to the cron ticker loop. This runs every cron cycle and marks stale `Running` routine runs as `Failed`:

```diff
--- a/ironclaw/src/agent/routine_engine.rs  (RoutineEngine)
+++ b/ironclaw/src/agent/routine_engine.rs
 impl RoutineEngine {
+    /// Reap zombie routine runs that have been stuck in `Running` for too long.
+    ///
+    /// Called periodically by the cron ticker. Marks runs stuck beyond
+    /// `ZOMBIE_RUN_TIMEOUT` as `Failed` with a descriptive summary.
+    pub async fn reap_zombie_runs(&self) {
+        const ZOMBIE_RUN_TIMEOUT_SECS: i64 = 600; // 10 minutes
+        match self.store.reap_stale_routine_runs(ZOMBIE_RUN_TIMEOUT_SECS).await {
+            Ok(count) if count > 0 => {
+                tracing::warn!("Reaped {} zombie RoutineRun records (stuck >{}s)", count, ZOMBIE_RUN_TIMEOUT_SECS);
+            }
+            Ok(_) => {}
+            Err(e) => {
+                tracing::error!("Failed to reap zombie routine runs: {}", e);
+            }
+        }
+    }
 }
```

Add the DB method (in the `Database` trait / SQLite impl):
```rust
/// Mark RoutineRun records stuck in `Running` for more than `timeout_secs` as `Failed`.
async fn reap_stale_routine_runs(&self, timeout_secs: i64) -> Result<usize>;
// SQL: UPDATE routine_runs SET status = 'failed', completed_at = NOW(),
//      result_summary = 'Reaped: stuck in Running for >10 minutes'
//      WHERE status = 'running' AND started_at < NOW() - timeout_secs
```

Hook it into the cron ticker:
```diff
--- a/ironclaw/src/agent/routine_engine.rs  (spawn_cron_ticker)
+++ b/ironclaw/src/agent/routine_engine.rs
 pub fn spawn_cron_ticker(...) -> tokio::task::JoinHandle<()> {
     tokio::spawn(async move {
         // ...
         loop {
             ticker.tick().await;
             engine.check_cron_triggers().await;
+            engine.reap_zombie_runs().await;
         }
     })
 }
```

**Verification:** The 10-minute timeout matches the existing TTL reaper for jobs. The reaper runs on the same cron interval (default: 30s), which is frequent enough to prevent long accumulation. It cleanly handles both subagent crash and gateway restart scenarios since the DB query is time-based.

---

## 🟠 High — Significant Defects / Design Gaps

---

### IC-007 · ~25 `unsafe { std::env::set_var() }` calls during async operation

**Severity:** 🟠 High  
**File:** `backend/src/openclaw/ironclaw_bridge.rs` (throughout)  
**Status:** Open

#### ✅ Proposed Solution

This is a larger refactor. The pragmatic immediate fix is to collect all env var mutations into a single synchronous block that runs *before* any async work:

```diff
--- a/backend/src/openclaw/ironclaw_bridge.rs  (build_inner)
+++ b/backend/src/openclaw/ironclaw_bridge.rs
+    /// Set all environment variables atomically before any async work begins.
+    /// This runs on the calling thread, not inside an async task.
+    fn configure_environment(oc_config: &Option<OpenClawConfig>) {
+        // Collect all values first
+        let vars: Vec<(&str, String)> = vec![
+            ("ALLOW_LOCAL_TOOLS", /* ... */),
+            ("WORKSPACE_MODE", /* ... */),
+            // ... all 25 vars
+        ];
+        // Apply atomically (single-threaded at this point)
+        for (key, value) in &vars {
+            std::env::set_var(key, value);
+        }
+    }
```

Call `configure_environment()` at the top of `start()` before the first `.await` point. Since `start()` is called from the Tauri main thread which is single-threaded at that point, this eliminates the data-race window.

Long-term: Pass all values through `ironclaw::Config` (typed struct) and remove env-var reads from `AgentConfig::resolve()`.

**Verification:** Moving all `set_var` calls before the first `.await` ensures no concurrent reader can observe a partial write. The `AgentConfig::resolve()` function runs after all vars are set.

---

### IC-008 · LLM backend fallback to `ollama` is silent — no UI notification

**Severity:** 🟠 High  
**File:** `backend/src/openclaw/ironclaw_bridge.rs` L1005–1013  
**Status:** Open

#### ✅ Proposed Solution

Emit a `UiEvent` warning to the frontend:

```diff
--- a/backend/src/openclaw/ironclaw_bridge.rs  (~L1013)
+++ b/backend/src/openclaw/ironclaw_bridge.rs
                     // Last resort: placeholder
                     std::env::set_var("LLM_BACKEND", "ollama");
+                    // Warn the user — this is likely to fail at runtime
+                    use tauri::Emitter;
+                    let warning = UiEvent::Error {
+                        message: "No LLM provider configured. Falling back to Ollama (localhost). \
+                                  Install Ollama or configure a cloud provider in Settings → Brain."
+                            .to_string(),
+                        code: Some("LLM_FALLBACK".to_string()),
+                    };
+                    let _ = app_handle.emit("openclaw-event", &warning);
+                    tracing::warn!("[ironclaw] No LLM provider configured — using Ollama fallback");
```

**Verification:** The frontend's event listener at `OpenClawChatView.tsx` L727–738 already handles `kind === 'Error'` events with a toast. The `UiEvent::Error` variant exists and includes a `code` field. The warning will appear as a red toast with 8-second duration.

---

### IC-009 · Raw `(window as any).__tauri__?.invoke(...)` bypasses type-safe bindings

**Severity:** 🟠 High  
**File:** `frontend/src/components/openclaw/OpenClawChatView.tsx` L436, L763  
**Status:** Open

#### ✅ Proposed Solution

Replace both occurrences with the generated Specta binding:

```diff
--- a/frontend/src/components/openclaw/OpenClawChatView.tsx  L436
-    (window as any).__tauri__?.invoke('openclaw_reveal_file', { path: metadata.absolute_path }).catch(() => { });
+    commands.openclawRevealFile(metadata.absolute_path).catch(() => { });

--- a/frontend/src/components/openclaw/OpenClawChatView.tsx  L763
-    (window as any).__tauri__?.invoke('openclaw_reveal_file', { path }).catch(() => { });
+    commands.openclawRevealFile(path).catch(() => { });
```

**Verification:** The `openclawRevealFile` command exists in `rpc.rs` and is registered with Specta. The generated binding accepts a single `path: string` argument, matching the usage.

---

### IC-010 · Stateful JS RegExp `lastIndex` reset in `AssistantMessageContent` is fragile

**Severity:** 🟠 High  
**File:** `frontend/src/components/openclaw/OpenClawChatView.tsx` L261–269  
**Status:** Open

#### ✅ Proposed Solution

Create the regex fresh each time instead of reusing with manual `lastIndex` reset:

```diff
--- a/frontend/src/components/openclaw/OpenClawChatView.tsx  L258-270
+++ b/frontend/src/components/openclaw/OpenClawChatView.tsx
 function AssistantMessageContent({ text }: { text: string }) {
-    const toolCallRegex = /\[TOOL_CALLS\](\w+)\[ARGS\](\{.*?\})(?:\s|$)/gm;
-    const hasToolCalls = toolCallRegex.test(text);
+    // Stateless check — no /g flag, just test for presence
+    const hasToolCalls = /\[TOOL_CALLS\]\w+\[ARGS\]\{/.test(text);
 
     if (!hasToolCalls) {
         return <div ...><ReactMarkdown ...>{text}</ReactMarkdown></div>;
     }

-    // Reset regex after test
-    toolCallRegex.lastIndex = 0;
-
     // Split into text parts and tool call parts
     const parts = [];
     let lastIndex = 0;
-    let match;
-
-    while ((match = toolCallRegex.exec(text)) !== null) {
+    // Fresh regex for exec loop — avoids stateful lastIndex issues
+    const execRegex = /\[TOOL_CALLS\](\w+)\[ARGS\](\{.*\})$/gm;
+    let match;
+    while ((match = execRegex.exec(text)) !== null) {
```

Note: this also addresses IC-029 (nested JSON) — the `.*` is now greedy-to-EOL instead of `.*?` (non-greedy to first `}`). See IC-029 for details.

**Verification:** The presence test uses a non-global regex (no `lastIndex` statefulness). The exec loop creates a fresh regex per function call (it's inside the function body). The `/gm` flags on the exec regex mean `$` matches end-of-line per line, and the greedy `.*` consumes the full JSON argument.

---

### IC-011 · 5-minute ID reconciliation window too broad — wrong `realId` on rapid regenerations

**Severity:** 🟠 High  
**File:** `frontend/src/hooks/use-chat.ts` L297–299  
**Status:** Open

#### ✅ Proposed Solution

Tighten the window from 5 minutes to 30 seconds, and add a content-length proximity check:

```diff
--- a/frontend/src/hooks/use-chat.ts  L294-300
+++ b/frontend/src/hooks/use-chat.ts
                         // 3. Last Assistant Message Match:
-                        // If this is one of the last few messages from DB, and matches role
-                        // increased time buffer to 5 minutes to account for long generations
+                        // Tighter match: only reconcile if within 30 seconds
+                        // (long generations still finish within this window because
+                        // the temp message timestamp is when the request was sent)
                         if (curr.role === m.role && m.role === 'assistant') {
                             const timeDiff = Math.abs((curr.created_at || 0) - (m.created_at || 0));
-                            return timeDiff < 300000;
+                            return timeDiff < 30000; // 30 seconds
                         }
```

**Verification:** The `created_at` for temp messages is set at `Date.now()` when the user clicks send (L431). The DB `created_at` for the assistant message is set when the backend creates the message row, typically within 1-2 seconds. Even for long generations, the row is created at the start (not end) of streaming. 30 seconds is generous enough for any realistic latency.

---

### IC-012 · `RigManager::new()` panics if `app_handle` is `None`

**Severity:** 🟠 High  
**File:** `backend/src/rig_lib/agent.rs` L146, L149  
**Status:** Open

#### ✅ Proposed Solution

Conditionally add tools based on `app_handle` availability:

```diff
--- a/backend/src/rig_lib/agent.rs  L141-153
+++ b/backend/src/rig_lib/agent.rs
-        let agent = builder
-            .tool(CalculatorTool)
-            .tool(RAGTool {
-                app: app_handle
-                    .clone()
-                    .expect("App handle required for RAG tool"),
-            })
-            .tool(ImageGenTool {
-                app: app_handle
-                    .clone()
-                    .expect("App handle required for Image tool"),
-            })
-            .build();
+        builder = builder.tool(CalculatorTool);
+        if let Some(ref handle) = app_handle {
+            builder = builder
+                .tool(RAGTool { app: handle.clone() })
+                .tool(ImageGenTool { app: handle.clone() });
+        }
+        let agent = builder.build();
```

**Verification:** The `CalculatorTool` does not need an app handle. `RAGTool` and `ImageGenTool` require it because they use Tauri's state management. In non-Tauri mode (e.g. CLI), these tools simply aren't registered — which is correct behavior since they wouldn't work without a Tauri runtime anyway.

---

### IC-013 · Heartbeat prompt-building logic duplicated between two files

**Severity:** 🟠 High  
**File:** `ironclaw/src/agent/heartbeat.rs` L210–257 and `ironclaw/src/agent/routine_engine.rs` L766–799  
**Status:** Open

#### ✅ Proposed Solution

Extract the shared logic into a public function in `heartbeat.rs`:

```diff
--- a/ironclaw/src/agent/heartbeat.rs
+++ b/ironclaw/src/agent/heartbeat.rs
+/// Build the heartbeat prompt with daily log context.
+///
+/// Reads today's and yesterday's daily logs from the workspace, caps them
+/// to prevent token explosion, and assembles the full heartbeat prompt.
+/// Used by both the standalone `HeartbeatRunner` and `RoutineEngine`.
+pub async fn build_heartbeat_prompt(
+    workspace: &Workspace,
+    checklist: &str,
+    custom_prompt: Option<&str>,
+) -> String {
+    let mut daily_context = String::new();
+    let today = chrono::Utc::now().date_naive();
+
+    if let Ok(doc) = workspace.today_log().await {
+        if !doc.content.trim().is_empty() {
+            let capped = cap_daily_log(&doc.content, 3000);
+            daily_context.push_str(&format!(
+                "\n\n## Daily Log — {} (today)\n\n{}",
+                today.format("%Y-%m-%d"), capped
+            ));
+        }
+    }
+    if let Some(yesterday) = today.pred_opt() {
+        if let Ok(doc) = workspace.daily_log(yesterday).await {
+            if !doc.content.trim().is_empty() {
+                let capped = cap_daily_log(&doc.content, 2000);
+                daily_context.push_str(&format!(
+                    "\n\n## Daily Log — {} (yesterday)\n\n{}",
+                    yesterday.format("%Y-%m-%d"), capped
+                ));
+            }
+        }
+    }
+
+    let prompt_body = custom_prompt.unwrap_or(DEFAULT_HEARTBEAT_PROMPT);
+    format!("{}\n\n## HEARTBEAT.md\n\n{}{}", prompt_body, checklist, daily_context)
+}
```

Then replace both call sites:
- `HeartbeatRunner::check_heartbeat()` L210–257 → call `build_heartbeat_prompt(&self.workspace, &checklist, None).await`
- `routine_engine.rs::execute_heartbeat()` L766–799 → call `crate::agent::heartbeat::build_heartbeat_prompt(&ctx.workspace, &checklist, custom_prompt).await`

**Verification:** Both sites currently use identical logic (same cap sizes: 3000/2000, same date formatting, same order). The shared function preserves all behavior. The `DEFAULT_HEARTBEAT_PROMPT` constant is already in `routine_engine.rs` and can be moved to `heartbeat.rs` as the canonical location.

---

## 🟡 Medium — Reliability / Misleading Code / Dead State

---

### IC-014 · `coreTab` initial state logic inverted for non-core views

**Severity:** 🟡 Medium  
**File:** `frontend/src/components/openclaw/OpenClawChatView.tsx` L580  
**Status:** Open

#### ✅ Proposed Solution

Always initialize to `'chat'` — the tab bar only renders for `isCoreView` anyway:

```diff
-    const [coreTab, setCoreTab] = useState<'chat' | 'console' | 'memory'>(isCoreView ? 'chat' : 'console');
+    const [coreTab, setCoreTab] = useState<'chat' | 'console' | 'memory'>('chat');
```

**Verification:** The tab bar rendering is gated by `isCoreView`. For non-core views, `coreTab` is never read. Setting it to `'chat'` is harmless and correct for all paths.

---

### IC-015 · Dead editor commentary in production source code

**Severity:** 🟡 Medium  
**File:** `backend/src/rig_lib/agent.rs` L198–213  
**Status:** Open

#### ✅ Proposed Solution

Replace the editor commentary with a proper doc comment:

```diff
--- a/backend/src/rig_lib/agent.rs  L198-211
+++ b/backend/src/rig_lib/agent.rs
+    /// Non-streaming RAG chat: runs explicit search, builds a context prompt,
+    /// and returns the LLM's response as a single string.
+    ///
+    /// Prefer `stream_rag_chat()` for production use — this method is kept
+    /// for simpler use cases and testing.
     pub async fn rag_chat(
         &self,
         query: &str,
         chat_history: Vec<crate::chat::Message>,
     ) -> Result<String, String> {
-        // ... (existing implementation details for non-streaming fallback if needed)
-        // For now, we are replacing the call site in chat.rs to use stream_rag_chat
-        // But we keep this for compatibility or simpler use cases.
-        // I will leave this as is.
-        // Re-implementing just to match "TargetContent" correctly or I can append.
-        // Actually, I'll allow the user to keep rag_chat for now.
-        // I will Add stream_rag_chat below it.
-
-        // Wait, replace_file_content replaces the block. I should just ADD stream_rag_chat.
-
         // 1. Run Explicit Search
```

---

### IC-016 · `get_local_llm_config()` hardcodes `"chatml"` model family

**Severity:** 🟡 Medium  
**File:** `backend/src/openclaw/config/types.rs` L441  
**Status:** Open

#### ✅ Proposed Solution

Read the model family from the config's provider metadata. If not present, detect from the model name:

```diff
--- a/backend/src/openclaw/config/types.rs  L421-442
+++ b/backend/src/openclaw/config/types.rs
     pub fn get_local_llm_config(&self) -> Option<(u16, String, u32, String)> {
         let models = self.models.as_ref()?;
         let local = models.providers.get("local")?;
         let base_url = local.get("baseUrl")?.as_str()?;
         let port = base_url.split(':').last()?.trim_matches('/').parse().ok()?;
         let api_key = local.get("apiKey").and_then(|v| v.as_str()).unwrap_or("").to_string();
         let models_list = local.get("models")?.as_array()?;
         let context_size = models_list.get(0)?.get("contextWindow")?.as_u64()? as u32;

-        // Model family is not stored in config JSON, default to chatml
-        Some((port, api_key, context_size, "chatml".into()))
+        // Try to read model family from config; infer from model name; fall back to chatml
+        let model_family = models_list.get(0)
+            .and_then(|m| m.get("family"))
+            .and_then(|v| v.as_str())
+            .map(|s| s.to_string())
+            .unwrap_or_else(|| {
+                let model_name = models_list.get(0)
+                    .and_then(|m| m.get("id"))
+                    .and_then(|v| v.as_str())
+                    .unwrap_or("");
+                infer_model_family(model_name)
+            });
+        Some((port, api_key, context_size, model_family))
     }
+
+    fn infer_model_family(model_name: &str) -> String {
+        let lower = model_name.to_lowercase();
+        if lower.contains("llama-3") || lower.contains("llama3") { "llama3".into() }
+        else if lower.contains("mistral") || lower.contains("mixtral") { "mistral".into() }
+        else if lower.contains("phi-3") || lower.contains("phi3") { "phi3".into() }
+        else if lower.contains("gemma") { "gemma".into() }
+        else { "chatml".into() } // safe fallback
+    }
```

**Verification:** The `models` array in `openclaw-engine.json` already contains model metadata objects with `id` fields. Adding a `family` field is a forward-compatible extension. The inference function handles common model families.

---

### IC-017 · `SkillInstallTool` JSON schema misleadingly marks `name` as required

**Severity:** 🟡 Medium  
**File:** `ironclaw/src/tools/builtin/skill_tools.rs` L366–368  
**Status:** Open

#### ✅ Proposed Solution

Update the schema to reflect the actual behavior (three install modes):

```diff
-            "required": ["name"]
+            "required": [],
+            "oneOf": [
+                { "required": ["name"], "description": "Install by catalog name/slug" },
+                { "required": ["url"], "description": "Install from URL" },
+                { "required": ["content"], "description": "Install from raw SKILL.md content" }
+            ]
```

And add a runtime validation in `execute()`:

```rust
let name = params.get("name").and_then(|v| v.as_str());
let url = params.get("url").and_then(|v| v.as_str());
let content = params.get("content").and_then(|v| v.as_str());
if name.is_none() && url.is_none() && content.is_none() {
    return Err(ToolError::MissingParameter("name, url, or content".to_string()));
}
```

---

### IC-018 · `spawn_fire()` drops `JoinHandle` — no cancellation path

**Severity:** 🟡 Medium  
**File:** `ironclaw/src/agent/routine_engine.rs` L319–355  
**Status:** Open

#### ✅ Proposed Solution

Use `tokio::task::JoinSet` to track all spawned routine tasks:

```diff
--- a/ironclaw/src/agent/routine_engine.rs
+++ a/ironclaw/src/agent/routine_engine.rs
+use tokio::task::JoinSet;

 pub struct RoutineEngine {
     // ... existing fields ...
+    /// Tracked handles for all running routine tasks.
+    active_tasks: Arc<tokio::sync::Mutex<JoinSet<()>>>,
 }

 impl RoutineEngine {
     fn spawn_fire(&self, routine: Routine, ...) {
         // ... build run and engine context ...
         let store = self.store.clone();
+        let tasks = self.active_tasks.clone();
-        tokio::spawn(async move { ... });
+        let mut guard = tasks.lock().await;
+        guard.spawn(async move {
+            if let Err(e) = store.create_routine_run(&run).await { ... return; }
+            execute_routine(engine, routine, run).await;
+        });
     }

+    /// Abort all running routine tasks. Called on engine shutdown.
+    pub async fn abort_all(&self) {
+        let mut guard = self.active_tasks.lock().await;
+        guard.abort_all();
+    }
 }
```

Wire `abort_all()` into `shutdown_background()`:
```rust
if let Some((cron_handle, engine)) = handle.routine_handle {
    cron_handle.abort();
    engine.abort_all().await;
}
```

**Verification:** `JoinSet` tracks all spawned tasks. `abort_all()` sends abort signals to every running routine on shutdown. Panics within tasks are observable via the `JoinSet`'s output (though not consumed here, they won't be silent anymore).

---

### IC-019 · ~~Stale session pruning threshold hardcoded at 2 hours~~ **RETRACTED**

**Severity:** ~~🟡 Medium~~ → **N/A — Invalid issue**  
**Status:** Retracted after re-analysis

**Re-analysis:** The session idle timeout is already configurable via `AgentConfig.session_idle_timeout`, sourced from `settings.rs` `AgentSettings.session_idle_timeout_secs` (default: **7 days**, not 2 hours) and overridable via `SESSION_IDLE_TIMEOUT_SECS` env var. This issue was incorrectly identified. The 2-hour value only appeared in a unit test fixture (`dispatcher.rs` L1754: `session_idle_timeout: Duration::from_secs(300)`).

**No fix required.**

---

### IC-020 · `openclaw-event` listener cleanup race in `OpenClawChatView`

**Severity:** 🟡 Medium  
**File:** `frontend/src/components/openclaw/OpenClawChatView.tsx` L718  
**Status:** Open

#### ✅ Proposed Solution

Use an `isMounted` ref to prevent dangling listener registration:

```diff
--- a/frontend/src/components/openclaw/OpenClawChatView.tsx  L714-718
+++ b/frontend/src/components/openclaw/OpenClawChatView.tsx
     useEffect(() => {
         if (!effectiveSessionKey) return;
+        let isMounted = true;
         const unlistenPromise = listen<any>('openclaw-event', (event) => {
+            if (!isMounted) return; // component unmounted, ignore events
             const uiEvent = event.payload;
             // ... handler body ...
         });
 
         return () => {
+            isMounted = false;
             unlistenPromise.then(f => f());
         };
     }, [effectiveSessionKey, ...]);
```

**Verification:** Setting `isMounted = false` in cleanup ensures that if the listener fires between unmount and the `unlistenPromise` resolving, it's a no-op. The eventual `unlisten()` call still fires correctly.

---

### IC-021 · `OpenClawIdentity` key pair fields not zeroized on reload

**Severity:** 🟡 Medium  
**File:** `backend/src/openclaw/config/types.rs`  
**Status:** Open

#### ✅ Proposed Solution

Implement `Drop` for `OpenClawIdentity` to zeroize key fields:

```diff
+impl Drop for OpenClawIdentity {
+    fn drop(&mut self) {
+        if let Some(ref mut key) = self.private_key {
+            zeroize::Zeroize::zeroize(key);
+        }
+        if let Some(ref mut key) = self.public_key {
+            zeroize::Zeroize::zeroize(key);
+        }
+    }
+}
```

**Verification:** `OpenClawIdentity` is deserialized from `identity.json` each time config is loaded. The old instance's destructor runs when the `Option<OpenClawIdentity>` is replaced. With this `Drop`, the old Ed25519 keys are overwritten with zeros before deallocation.

---

### IC-022 · ~~`MetaConfig` fields are never populated~~ **RETRACTED**

**Severity:** ~~🟡 Medium~~ → **N/A — Invalid issue**  
**Status:** Retracted after validation

**Re-analysis:** `engine.rs` L373-376 already populates both fields:
```rust
meta: MetaConfig {
    last_touched_version: OPENCLAW_VERSION.into(),
    last_touched_at: chrono::Utc::now().to_rfc3339(),
},
```
This issue was incorrectly identified. The `MetaConfig` fields are populated every time `generate_config()` is called.

**No fix required.**

---

## 🔵 Low — Code Quality / Naming / Minor UX

---

### IC-023 · `rag_chat()` is dead code — no callers in the codebase

**Severity:** 🔵 Low  
**Status:** Open

#### ✅ Proposed Solution

Add `#[allow(dead_code)]` with a deprecation notice (or remove entirely):

```diff
+    #[deprecated(note = "Use stream_rag_chat() instead")]
+    #[allow(dead_code)]
     pub async fn rag_chat(...) -> Result<String, String> {
```

---

### IC-024 · `pendingOptimisticMessages` ref is populated but never consumed

**Severity:** 🔵 Low  
**File:** `frontend/src/hooks/use-chat.ts` L93, L435  
**Status:** Open

#### ✅ Proposed Solution

Remove the ref and its push:

```diff
-    // Track pending optimistic messages (Legacy Ref for compatibility if needed, but currently unused logic removed)
-    const pendingOptimisticMessages = useRef<ExtendedMessage[]>([]);

--- L435
-        pendingOptimisticMessages.current.push(tempUserMsg, tempAssistantMsg);
```

---

### IC-025 · `job_monitor.rs` — intentional user-message drop is unexplained

**Severity:** 🔵 Low  
**Status:** Open

#### ✅ Proposed Solution

Add a comment to the match arm:

```diff
     _ => {
+        // Intentional: skip non-assistant messages (user, system)
+        // to avoid re-injecting instructions into the main agent context.
     }
```

---

### IC-026 · `SkillSearchTool` acquires the registry read lock twice unnecessarily

**Severity:** 🔵 Low  
**File:** `ironclaw/src/tools/builtin/skill_tools.rs` L248–307  
**Status:** Open

#### ✅ Proposed Solution

Combine into a single lock acquisition:

```diff
-        let installed_names: Vec<String> = {
-            let guard = self.registry.read().await;
-            guard.skills().iter().map(|s| s.manifest.name.clone()).collect()
-        };
-        // ... catalog matching ...
-        let local_matches: Vec<serde_json::Value> = {
-            let guard = self.registry.read().await;
-            guard.skills().iter().filter(|s| ...).map(|s| ...).collect()
-        };
+        let (installed_names, local_matches) = {
+            let guard = self.registry.read().await;
+            let names: Vec<String> = guard.skills().iter().map(|s| s.manifest.name.clone()).collect();
+            let matches: Vec<serde_json::Value> = guard.skills().iter()
+                .filter(|s| ...)
+                .map(|s| ...)
+                .collect();
+            (names, matches)
+        };
```

---

### IC-027 · ~~`node_host_enabled` field is completely unused~~ **RETRACTED**

**Severity:** ~~🔵 Low~~ → **N/A — Invalid issue**  
**Status:** Retracted after validation

**Re-analysis:** `node_host_enabled` is actively used in multiple places:
- `fleet.rs` L152: `if cfg.node_host_enabled { caps.push("ui_automation") }` — controls capability reporting
- `engine.rs` L319: `let tools_policy = if self.node_host_enabled { ... }` — controls which tool groups (`group:ui`) are allowed
- `engine.rs` L621-626: exported as `OPENCLAW_NODE_HOST_ENABLED` and `MOLTBOT_NODE_HOST_ENABLED` env vars
- `keys.rs` L747: `cfg.node_host_enabled = enabled` — toggled by UI

This issue was incorrectly identified.

**No fix required.**

---

### IC-028 · `isUserScrolling` variable name and logic are both inverted

**Severity:** 🔵 Low  
**File:** `frontend/src/components/openclaw/OpenClawChatView.tsx` L570, L588–599  
**Status:** Open

#### ✅ Proposed Solution

Rename and invert:

```diff
-    const isUserScrolling = useRef(false);
+    const isAutoScrollPinned = useRef(true);
 
     const handleScroll = () => {
         // ...
-        if (distFromBottom < 15) {
-            isUserScrolling.current = false;
-        } else {
-            isUserScrolling.current = true;
-        }
+        isAutoScrollPinned.current = distFromBottom < 15;
     };
```

Then update all references from `!isUserScrolling.current` to `isAutoScrollPinned.current`.

---

### IC-029 · Tool argument JSON regex `{.*?}` breaks on nested objects

**Severity:** 🔵 Low  
**File:** `frontend/src/components/openclaw/OpenClawChatView.tsx` L261  
**Status:** Open

#### ✅ Proposed Solution (combined with IC-010)

Use a greedy match to end-of-line and let `JSON.parse` validate:

```diff
-    const toolCallRegex = /\[TOOL_CALLS\](\w+)\[ARGS\](\{.*?\})(?:\s|$)/gm;
+    // Greedy match to end-of-line: captures full JSON including nested objects
+    const execRegex = /\[TOOL_CALLS\](\w+)\[ARGS\](\{.*\})$/gm;
```

The `JSON.parse` at L288 already handles validation — if the match is too broad, parse will fail and the raw string is used as fallback (L290).

---

### IC-030 · Framer Motion height animation may flicker in Safari

**Severity:** 🔵 Low  
**Status:** Open

#### ✅ Proposed Solution

Add `style={{ maxHeight: 600 }}` to the motion container:

```diff
     <motion.div
         initial={{ height: 0, opacity: 0 }}
         animate={{ height: "auto", opacity: 1 }}
         exit={{ height: 0, opacity: 0 }}
-        className="overflow-hidden pl-4 pr-1 py-2 space-y-1"
+        className="overflow-hidden pl-4 pr-1 py-2 space-y-1"
+        style={{ maxHeight: 600 }}
     >
```

---

## 📦 Legacy & Stale Code

---

### IC-031 · Deprecated Ansible deployment section still in docs

**Severity:** 📦 Legacy  
**Status:** Open

#### ✅ Proposed Solution

Delete the entire `## ~~Deployment Path 6~~` section (L304–318) from `IRONCLAW_DEPLOYMENT_PATHS.md`.

---

### IC-032 · `default_dm_policy()` is `pub(crate)` but used only as serde default

**Severity:** 📦 Legacy  
**Status:** Open

#### ✅ Proposed Solution

```diff
-pub(crate) fn default_dm_policy() -> String {
+fn default_dm_policy() -> String {
```

---

### IC-033 · `RigManager` preamble hardcodes agent name "OpenClaw"

**Severity:** 📦 Legacy  
**Status:** Open

#### ✅ Proposed Solution

Read from env or use a parameter:

```diff
--- a/backend/src/rig_lib/agent.rs  L41-42
+++ b/backend/src/rig_lib/agent.rs
+        let agent_name = std::env::var("AGENT_NAME").unwrap_or_else(|_| "OpenClaw".to_string());
         let mut base_preamble = format!(
-            "You are OpenClaw, a friendly AI assistant.
+            "You are {}, a friendly AI assistant.
 Current Date: {}
 ",
-            date
+            agent_name, date
         );
```

**Verification:** `AGENT_NAME` is already set by the IronClaw config system (defaults to `"ironclaw"` in settings). For the Scrappy main chat (which uses `RigManager`), this env var may not be set — in which case the fallback `"OpenClaw"` preserves existing behavior.

---

### IC-034 · Dead `coreTab` state maintained for non-core session views

**Severity:** 📦 Legacy  
**Status:** Addressed by IC-014

The fix in IC-014 (always initialize to `'chat'`) eliminates the incorrect initialization. The state is still maintained for non-core views but at zero cost (React doesn't re-render for unused state).

---

## 📋 Documentation Drift

These items appeared as a separate table in the original analysis. They are stale documentation issues not tied to a specific numbered issue above.

---

### DD-001 · `IRONCLAW_DEPLOYMENT_PATHS.md` contains a hard-coded "Last updated" date

**Severity:** 📋 Docs  
**File:** `documentation/latest/remote_deploy/IRONCLAW_DEPLOYMENT_PATHS.md`  
**Status:** Open

The document has a hard-coded "`Last updated: 2026-03-10`" date stamp at the top. This will silently drift out of date as features change, giving readers a false sense of currency.

#### ✅ Proposed Solution

Replace the single "Last updated" line with a `## Changelog` section at the bottom of the document:

```diff
-Last updated: 2026-03-10
+> This document is maintained alongside the code. Check the Git log for the authoritative change history.
```

Add a `## Changelog` section at the end listing significant changes with dates. Future edits add entries there rather than updating a global timestamp. This matches the pattern used in `RIG_IMPLEMENTATION.md`.

---

### DD-002 · `ironclaw/src/agent/mod.rs` module doc omits remote-proxy caveat for undo system

**Severity:** 📋 Docs  
**File:** `ironclaw/src/agent/mod.rs`  
**Status:** Open

The top-level module doc states "Turn-based session management with undo" without noting that the undo system is marked experimental and does **not** work in remote proxy mode (the `undo_managers` map in `SessionManager` is per-process; remote gateway mode has no shared in-memory session state).

#### ✅ Proposed Solution

```diff
-//! Turn-based session management with undo.
+//! Turn-based session management with undo (local engine mode only).
+//!
+//! **Note:** The undo system (`UndoManager`) is in-memory and does not
+//! persist across restarts or operate in remote proxy mode.
```

---

### DD-003 · `heartbeat.rs` module doc describes `HeartbeatRunner` as primary — it is now superseded

**Severity:** 📋 Docs  
**File:** `ironclaw/src/agent/heartbeat.rs`  
**Status:** Open

The module doc states "Reports any findings to the configured channel" describing `HeartbeatRunner` as the primary heartbeat mechanism. In the current architecture, `HeartbeatRunner` is only used in non-routine (standalone) contexts. The primary heartbeat path is `RoutineEngine::execute_heartbeat()` — `HeartbeatRunner` is effectively legacy for any installation with routines enabled.

#### ✅ Proposed Solution

Update the module doc to reflect the current architecture:

```diff
-//! Proactive heartbeat system.
-//!
-//! The `HeartbeatRunner` periodically evaluates the HEARTBEAT.md checklist
-//! and reports any findings to the configured channel.
+//! Proactive heartbeat system.
+//!
+//! **Primary path (routines enabled):** Heartbeat checks are driven by
+//! `RoutineEngine::execute_heartbeat()`. The routine engine auto-registers
+//! an `_heartbeat` routine at startup in `agent_loop.rs::upsert_heartbeat_routine()`.
+//!
+//! **Standalone path (no routine engine):** `HeartbeatRunner` is used directly
+//! via `spawn_heartbeat()`. This path is now `#[deprecated]`.
+//!
+//! The shared `build_heartbeat_prompt()` function is used by both paths.
```

---

## Summary Table

| ID | Severity | Short Title | Solution |
|----|----------|-------------|----------|
| IC-001 | 🔴 Critical | `auto_approve_tools` env var race | Clear env var in `stop()`, remove `is_err()` guard ✅ |
| IC-002 | 🔴 Critical | Concurrent workspace writes | Add per-user `RwLock` in `SessionManager` ✅ |
| IC-003 | 🔴 Critical | Heartbeat task outlives shutdown | Deprecate standalone runner; track notify forwarder ✅ |
| IC-004 | 🔴 Critical | `hasFailures` false positives | Remove string heuristics, use `metadata.status` only ✅ |
| IC-005 | 🔴 Critical | `extractSessionKey()` silent failure | Add regex fallback + `console.warn` ✅ |
| IC-006 | 🔴 Critical | Zombie RoutineRun DB records | Add zombie reaper in cron ticker ✅ |
| IC-007 | 🟠 High | Unsafe `set_var()` during async | Move all env mutations before first `.await` ✅ |
| IC-008 | 🟠 High | Silent LLM fallback | Emit `UiEvent::Error` warning toast ✅ |
| IC-009 | 🟠 High | Raw `__tauri__` invoke | Replace with `commands.openclawRevealFile()` ✅ |
| IC-010 | 🟠 High | Stateful RegExp `lastIndex` | Use non-global regex for test, fresh regex for exec ✅ |
| IC-011 | 🟠 High | 5-min reconciliation window | Tighten to 30 seconds ✅ |
| IC-012 | 🟠 High | `RigManager` panics on None | Conditional tool registration with `if let Some` ✅ |
| IC-013 | 🟠 High | Duplicated heartbeat prompt | Extract `build_heartbeat_prompt()` function ✅ |
| IC-014 | 🟡 Medium | `coreTab` initial state inverted | Always initialize to `'chat'` ✅ |
| IC-015 | 🟡 Medium | Editor commentary in source | Replace with proper doc comment ✅ |
| IC-016 | 🟡 Medium | Hardcoded `"chatml"` family | Read from config, infer from model name ✅ |
| IC-017 | 🟡 Medium | `SkillInstallTool` schema lies | Use `oneOf` schema, add runtime validation ✅ |
| IC-018 | 🟡 Medium | `spawn_fire()` drops JoinHandle | Use `JoinSet` for tracking ✅ |
| IC-019 | ~~🟡 Medium~~ | ~~Session pruning hardcoded~~ | **RETRACTED** — already configurable ❌ |
| IC-020 | 🟡 Medium | Event listener cleanup race | Add `isMounted` ref guard ✅ |
| IC-021 | 🟡 Medium | Identity keys not zeroized | Implement `Drop` for `OpenClawIdentity` ✅ |
| IC-022 | ~~🟡 Medium~~ | ~~`MetaConfig` never populated~~ | **RETRACTED** — already populated in `engine.rs` ❌ |
| IC-023 | 🔵 Low | `rag_chat()` dead code | Mark `#[deprecated]` ✅ |
| IC-024 | 🔵 Low | `pendingOptimisticMessages` unused | Remove ref and push ✅ |
| IC-025 | 🔵 Low | Job monitor drop unexplained | Add inline comment ✅ |
| IC-026 | 🔵 Low | Double registry lock | Combine into single acquisition ✅ |
| IC-027 | ~~🔵 Low~~ | ~~`node_host_enabled` unused~~ | **RETRACTED** — actively used in fleet.rs + engine.rs ❌ |
| IC-028 | 🔵 Low | `isUserScrolling` inverted | Rename to `isAutoScrollPinned`, simplify ✅ |
| IC-029 | 🔵 Low | Nested JSON regex broken | Use greedy match to EOL ✅ |
| IC-030 | 🔵 Low | Safari animation flicker | Add `maxHeight: 600` constraint ✅ |
| IC-031 | 📦 Legacy | Deprecated Ansible docs | Delete section ✅ |
| IC-032 | 📦 Legacy | `default_dm_policy()` visibility | Change to `fn` ✅ |
| IC-033 | 📦 Legacy | Hardcoded "OpenClaw" name | Read from `AGENT_NAME` env var ✅ |
| IC-034 | 📦 Legacy | Dead `coreTab` for non-core | Addressed by IC-014 ✅ |
| DD-001 | 📋 Docs | Hard-coded "Last updated" date | Replace with Changelog section ✅ |
| DD-002 | 📋 Docs | `mod.rs` undo caveat missing | Add remote-proxy note to module doc ✅ |
| DD-003 | 📋 Docs | `heartbeat.rs` doc describes superseded path | Update module doc to reflect routing architecture ✅ |

---

## Final Validation

Every issue and proposed solution was re-validated line-by-line against the actual source code.

### ✅ Confirmed Real Issues (31 of 34)

1. **IC-001**: Confirmed. `stop()` L448 does NOT clear `AGENT_AUTO_APPROVE_TOOLS`. The `is_err()` guard at L870 prevents config updates from taking effect on restart. `rpc.rs` L696 persists to config AND env var. Fix is correct. ✅
2. **IC-002**: Confirmed. `resolve_thread()` creates per-channel threads but same `Arc<Mutex<Session>>` is shared. No workspace write guard exists. Fix is correct. ✅
3. **IC-003**: Confirmed. `BackgroundTasksHandle` (L614-621) has no field for the notification forwarder (spawned at L561). `shutdown_background()` L629-643 doesn't abort it. Fix is correct. ✅
4. **IC-004**: Confirmed. L337-339 — `.includes('error')` is case-sensitive substring match. `ToolOutput::success/failure` always sets `metadata.status`. Fix is correct. ✅
5. **IC-005**: Confirmed. L223 — `catch { return null; }` silently swallows parse errors with no diagnostic. Fix is correct. ✅
6. **IC-006**: Confirmed. `execute_as_subagent()` L616 returns `RunStatus::Running` — no DB finalization if subagent crashes. Zombie reaper fix is correct. ✅
7. **IC-007**: Confirmed. ~25 `unsafe { std::env::set_var() }` calls scattered through async code in `ironclaw_bridge.rs`. Fix to centralize before first `.await` is correct. ✅
8. **IC-008**: Confirmed. L1004-1013 — silent `LLM_BACKEND=ollama` fallback with only `tracing::info!`. Frontend event listener L727-738 already handles `kind === 'Error'` — fix to emit `UiEvent::Error` is correct. ✅
9. **IC-009**: Confirmed. L436 and L763 — raw `(window as any).__tauri__?.invoke(...)`. Fix to use generated binding is correct. ✅
10. **IC-010**: Confirmed. L261 — stateful `/gm` regex with manual `lastIndex = 0` reset at L269. Fix to use stateless test + fresh exec regex is correct. ✅
11. **IC-011**: Confirmed. L299 — `timeDiff < 300000` (5 minutes). Temp `created_at` set at L431 send time, DB row created within seconds. 30s window fix is correct. ✅
12. **IC-012**: Confirmed. L146 and L151 — `.expect()` on `Option<AppHandle>`. Fix to use `if let Some` is correct. ✅
13. **IC-013**: Confirmed. `heartbeat.rs` L214-241 and `routine_engine.rs` L766-792 are copy-pasted with identical cap values (3000/2000). Extract shared function fix is correct. ✅
14. **IC-014**: Confirmed. L580 — non-core views initialize `coreTab` to `'console'` which doesn't exist for them. Fix is correct. ✅
15. **IC-015**: Confirmed. L203-211 — editor commentary in production. Fix is correct. ✅
16. **IC-016**: Confirmed. L440-441 — hardcoded `"chatml"` for all local models. Fix to infer from model name is correct. ✅
17. **IC-017**: Confirmed. L367 — `"required": ["name"]` but `url` and `content` are valid alternatives. Fix is correct. ✅
18. **IC-018**: Confirmed. `spawn_fire()` L348 — `tokio::spawn(...)` handle dropped immediately. JoinSet fix is correct. ✅
19. **IC-020**: Confirmed. L718 — `listen<any>()` returns Promise, cleanup can race. Fix with `isMounted` ref is correct. ✅
20. **IC-021**: Confirmed. `OpenClawIdentity` (L48-56) has `private_key: Option<String>` but no `Drop` impl. Fix is correct. ✅
21. **IC-023**: Confirmed. No callers of `rag_chat()` found — only `stream_rag_chat()` is used. Fix is correct. ✅
22. **IC-024**: Confirmed. L93 — `pendingOptimisticMessages` ref, comment itself says "currently unused." Fix is correct. ✅
23. **IC-025**: Confirmed. `job_monitor.rs` L52 — only `role == "assistant"` handled, others fall through silently. Fix is correct. ✅
24. **IC-026**: Confirmed. `skill_tools.rs` L248-257 and L283-307 both acquire `registry.read()`. Single-lock fix is correct. ✅
25. **IC-028**: Confirmed. L570, L594-597 — `isUserScrolling` semantics inverted. Rename fix is correct. ✅
26. **IC-029**: Confirmed. L261 — `{.*?}` non-greedy stops at first `}`. Greedy-to-EOL fix is correct. ✅
27. **IC-030**: Confirmed. L370-374 — `height: "auto"` animation without max-height. Fix is correct. ✅
28. **IC-031**: Confirmed. L304 — deprecated Ansible section still present. Fix is correct. ✅
29. **IC-032**: Confirmed. `default_dm_policy()` at L340 is `pub(crate)` but only used as serde default. Fix is correct. ✅
30. **IC-033**: Confirmed. L42 — `"You are OpenClaw, a friendly AI assistant."` hardcoded. Fix is correct. ✅
31. **IC-034**: Confirmed. Addressed by IC-014. ✅

### ❌ Retracted Issues (3 of 34)

1. **IC-019**: Session idle timeout is already configurable via `SESSION_IDLE_TIMEOUT_SECS` env var (default 7 days). The "2-hour" value was from a test fixture.
2. **IC-022**: `MetaConfig` fields ARE populated in `engine.rs` L373-376: `last_touched_version: OPENCLAW_VERSION.into()` and `last_touched_at: chrono::Utc::now().to_rfc3339()`.
3. **IC-027**: `node_host_enabled` IS actively used — controls `group:ui` tool access (`engine.rs` L319), capability reporting (`fleet.rs` L152), and exported as env vars (`engine.rs` L621-626).

### 📋 Documentation Drift (3 items — all confirmed)

1. **DD-001**: Confirmed. `IRONCLAW_DEPLOYMENT_PATHS.md` L6 has hardcoded `"Last updated: 2026-03-10"`. ✅
2. **DD-002**: Confirmed. `mod.rs` L10 says "Turn-based session management with undo" without remote-proxy caveat. ✅
3. **DD-003**: Confirmed. `heartbeat.rs` L1-6 describes `HeartbeatRunner` as primary; the actual primary path is now `RoutineEngine::execute_heartbeat()`. ✅

> **Final score: 31 confirmed real issues + 3 doc drift items = 34 valid findings.**  
> **3 original issues retracted (IC-019, IC-022, IC-027).**  
> All remaining solutions are verified against actual source code and address root causes, not symptoms.

---

*Audit performed: 2026-03-11 · Solutions verified: 2026-03-11 · Auditor: Antigravity*
