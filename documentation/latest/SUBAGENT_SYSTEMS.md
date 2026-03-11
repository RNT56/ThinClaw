# Sub-Agent Systems in Scrappy / IronClaw

> **Last updated:** 2026-03-08  
> **Status:** Both systems fully operational as of this commit.

---

## Overview

IronClaw implements **two distinct sub-agent architectures** that serve different use cases. Understanding the difference is essential before choosing which to use.

| | System A: In-Process Executor | System B: Session-Based |
|---|---|---|
| **Triggered by** | Agent tool call (`spawn_subagent`) | Frontend or agent API (`openclaw_spawn_session`) |
| **Persistence** | None — ephemeral tokio task | Full DB session (survives restarts) |
| **Memory/identity** | No — isolated context only | Yes — full workspace, SOUL.md, MEMORY.md |
| **User-visible** | Progress via `emit_user_message` | Full chat tab + Fleet panel |
| **Result return** | Injected back into parent LLM context | `SubAgentUpdate` Tauri event |
| **Nesting** | Configurable (default off) | Uncontrolled |
| **Timeout** | ✅ 300s default, configurable | None |
| **Cancellable** | ✅ `cancel_subagent` tool | ❌ Not yet |
| **Concurrency limit** | ✅ 5 max concurrent | None |
| **Source file** | `ironclaw/src/agent/subagent_executor.rs` | `backend/src/openclaw/commands/rpc.rs` |

---

## System A — In-Process SubagentExecutor

### What it is

A lightweight, fully isolated agentic loop that runs as a **tokio task** within the same process as the main agent. It shares the LLM provider, safety layer, and tool registry — but runs its own independent LLM conversation context with no session, no DB storage, and no workspace access.

Think of it as: *"agent asks another agent to do something in parallel, gets the answer back."*

### Architecture

```
Main Agent agentic loop
  │
  │  LLM decides to call: spawn_subagent(name="researcher", task="...", wait=true)
  ▼
SpawnSubagentTool.execute()
  │  Returns JSON: { "action": "spawn_subagent", "request": { ... } }
  ▼
dispatcher.rs intercepts (line ~1147 in dispatcher.rs)
  │  Detects action == "spawn_subagent"
  │  Calls SubagentExecutor::spawn(request, channel, metadata)
  ▼
SubagentExecutor::spawn()
  │
  ├── Checks concurrency limit (max 5 concurrent)
  ├── Generates UUID for sub-agent
  ├── Builds system prompt (custom or default)
  │
  ├── tokio::spawn(run_subagent_loop)  ← non-blocking
  │       │
  │       ├── StatusUpdate::AgentMessage "🔀 Sub-agent 'researcher' started: ..."
  │       ├── LLM call with task + system prompt
  │       ├── Tool execution loop (up to 30 iterations)
  │       │     ├── emit_user_message → forwarded to Tauri channel as status
  │       │     └── agent_think → recorded, no side effects
  │       └── Returns (response_text, iterations)
  │
  ├── If wait=true: await join_handle, return SubagentResult
  └── If wait=false: return immediately with agent ID
  │
  ▼
tool_result overridden with SubagentResult JSON
  │
  ▼
Parent LLM sees: { "response": "...", "iterations": N, "duration_ms": ... }
```

### How the tool is exposed to the agent

The agent has access to three tools:

```
spawn_subagent(name, task, [tools], [system_prompt], [timeout_secs], [wait])
  → Spawns a sub-agent. If wait=true (default), blocks until complete.

list_subagents()
  → Returns all active and recently completed sub-agents with status.

cancel_subagent(agent_id)
  → Cancels a running sub-agent by UUID.
```

### SubagentSpawnRequest fields

```rust
pub struct SubagentSpawnRequest {
    pub name: String,                     // Display name e.g. "researcher"
    pub task: String,                     // The task description (becomes user message)
    pub system_prompt: Option<String>,    // Custom system prompt; default: task-focused
    pub model: Option<String>,            // Model override (not yet wired)
    pub allowed_tools: Option<Vec<String>>, // Tool whitelist; None = all tools
    pub timeout_secs: Option<u64>,        // Default: 300s
    pub wait: bool,                       // Block parent until complete?
}
```

### Sub-agent tool filtering

When `allowed_tools` is specified, the sub-agent only has access to those tools **plus** `agent_think` and `emit_user_message` (always included). This allows running a sandboxed sub-agent with only `http` and `read_file`, for example.

```
spawn_subagent(
  name: "web-researcher",
  task: "Find the current Rust edition release notes",
  tools: ["http"],          // Only HTTP, no file system, no memory writes
  wait: true
)
```

### Nesting

By default `allow_nested = false` — sub-agents **cannot** spawn further sub-agents. This prevents runaway recursive spawning. To enable orchestrator patterns:

```rust
// In ironclaw_bridge.rs SubagentConfig:
SubagentConfig {
    allow_nested: true,    // Enable nested spawning
    max_concurrent: 10,    // Increase if orchestrator spawns many workers
    ..Default::default()
}
```

Note: nested sub-agents detect the nesting depth via registry counting (not yet enforced in the in-process executor — `allow_nested` is a config contract for future enforcement).

### Status events

During execution, a sub-agent can use `emit_user_message` to send updates to the user:

```
The sub-agent calls:
  emit_user_message(content="Found 3 relevant papers so far...", message_type="progress")

The channel manager emits:
  StatusUpdate::AgentMessage { content: "...", message_type: "progress" }

The Tauri channel translates to:
  UiEvent::RunStatus → displayed as a tool progress card in the chat UI
```

Start and completion are automatically announced:
- `🔀 Sub-agent 'researcher' started: <task>`
- `✅ Sub-agent 'researcher' completed (5 iterations, 3.2s)`
- `❌ Sub-agent 'researcher' failed: <error>`
- `⏰ Sub-agent 'researcher' timed out after 300s`

### Where it's wired (Scrappy bridge)

**`backend/src/openclaw/ironclaw_bridge.rs`** — `IronClawState::build_inner()`:

```rust
// Step 5b — wired March 2026
let subagent_executor = Arc::new(
    ironclaw::agent::subagent_executor::SubagentExecutor::new(
        components.llm.clone(),
        components.safety.clone(),
        components.tools.clone(),
        channel_manager.clone(),   // same channel manager as main agent
        SubagentConfig {
            max_concurrent: 5,
            default_timeout_secs: 300,
            allow_nested: false,
            max_tool_iterations: 30,
        },
    ),
);

// AgentDeps:
subagent_executor: Some(subagent_executor),
```

**`ironclaw/src/agent/dispatcher.rs`** — `run_agentic_loop()`:

The dispatcher intercepts `spawn_subagent` tool calls **after** the tool's `execute()` returns the JSON action descriptor. It calls `executor.spawn()` and replaces the tool result with the real `SubagentResult`:

```rust
if tc.name == "spawn_subagent" {
    // Parse JSON action from tool output
    // Call executor.spawn(request, channel, metadata)
    // Override tool_result with SubagentResult JSON
}
```

### Limitations

| Limitation | Impact |
|---|---|
| No workspace access | Sub-agent cannot read SOUL.md, MEMORY.md, or write memories |
| No session persistence | Sub-agent conversation is lost when the task completes |
| No streaming to parent | Parent only sees the final result, not intermediate LLM tokens |
| Model override not wired | `model` field in SubagentSpawnRequest is parsed but not used to switch LLM |
| `allow_nested` not enforced | Config exists but depth counting not yet implemented |

---

## System B — Session-Based Sub-Agents

### What it is

A **full, persistent chat session** in the IronClaw database — identical to what a user would see in a main chat window. Session-based sub-agents have full access to the agent's identity, memory, and workspace. They persist across gateway restarts. The user can open them in their own chat tab.

This mirrors OpenClaw's `/subagents spawn` model exactly.

### Architecture

```
Frontend (FleetCommandCenter or SubAgentPanel)
  │
  │  User clicks "Spawn Sub-Agent" with a task
  ▼
openclaw_spawn_session(agent_id, task, parent_session?) [Tauri command]
  │
  ├── Generates child session key: "agent:main:task-<uuid>"
  │
  ├── ironclaw.activate_session(child_session_key)
  │     → TauriChannel registers child_session_key for event routing
  │     → Events for this session now flow to the frontend
  │
  ├── sub_agent_registry::register(parent, ChildSessionInfo)
  │     → In-memory map: parent_session → [child1, child2, ...]
  │     → Tracked for Fleet panel display only (not in DB)
  │
  ├── Emits UiEvent::SubAgentUpdate { status: "running", progress: 0.0 }
  │     → Frontend Fleet panel adds child card
  │
  └── ironclaw::api::chat::send_message(agent, child_session_key, task, streaming=true)
        │
        ├── Full dispatcher path: workspace system prompt + SOUL.md + MEMORY.md
        ├── All tools available
        ├── Streaming events → Tauri channel → frontend
        └── Result stored in DB under child_session_key
```

### Session key format

```
Parent:  "agent:main"
Child:   "agent:main:task-550e8400-e29b-41d4-a716-446655440000"
```

The child key includes the parent name as a prefix, making it easy to group related sessions visually.

### The sub_agent_registry (in-memory)

```rust
// backend/src/openclaw/commands/rpc.rs
mod sub_agent_registry {
    // parent_session → Vec<ChildSessionInfo>
    static REGISTRY: OnceLock<RwLock<SubAgentStore>>;
}

pub struct ChildSessionInfo {
    pub session_key: String,    // Child session key
    pub task: String,           // Original task description
    pub status: String,         // "running" | "completed" | "failed"
    pub spawned_at: f64,        // Unix timestamp (ms)
    pub result_summary: Option<String>, // Set by openclaw_update_child_session_status
}
```

> **Important:** The registry only tracks spawning relationships for the UI. It is **not persisted** to the database and is cleared when the engine stops. Session history itself is fully persistent in `ironclaw.db`.

### Tauri commands for session-based sub-agents

| Command | Purpose |
|---|---|
| `openclaw_spawn_session(agent_id, task, parent_session?)` | Spawn a child session and send the first task message |
| `openclaw_list_child_sessions(parent_session)` | List all children of a parent session |
| `openclaw_update_child_session_status(child, status, result_summary?)` | Mark a child done/failed, emits SubAgentUpdate event |

### SubAgentUpdate event

Emitted to the frontend whenever a sub-agent's state changes:

```typescript
interface SubAgentUpdateEvent {
    kind: 'SubAgentUpdate';
    parent_session: string;
    child_session: string;
    task: string;
    status: 'running' | 'completed' | 'failed';
    progress: number | null;    // 0.0–1.0
    result_preview: string | null;
}
```

The frontend `SubAgentPanel` listens for this event and updates its card list in real time.

### Frontend components

#### SubAgentPanel (`frontend/src/components/openclaw/SubAgentPanel.tsx`)

Rendered inside the main chat view for any session. Shows:
- Cards for each active child session: task description, status badge, elapsed time
- A "Spawn Sub-Agent" text input
- Click-to-view: clicking a child session card calls `onSpawnSubAgent(childSessionKey)` which opens it as a new chat tab

```tsx
// Listens for SubAgentUpdate events
await listen<SubAgentUpdateEvent>('openclaw-event', (event) => {
    if (data.kind !== 'SubAgentUpdate') return;
    if (data.parent_session !== sessionKey) return;
    // Update card state
});

// Spawn
await spawnSession('main', spawnTask.trim(), sessionKey);
```

#### FleetCommandCenter (`frontend/src/components/openclaw/fleet/FleetCommandCenter.tsx`)

A dedicated sidebar page (accessible via the "Fleet" nav item) showing all child sessions across all parents. Provides an orchestration overview with task summaries and status.

### How the agent's response is routed

When `ironclaw::api::chat::send_message()` processes the task in the child session, all streaming events carry the **child session key** in their metadata:

```
UiEvent::AssistantDelta { session_key: "agent:main:task-...", delta: "..." }
```

The frontend subscribes to events matching the child session key if the user has that chat tab open. Events for sessions not currently open are buffered in the DB and loaded via `fetchHistory()` when the tab opens.

### Limitations

| Limitation | Impact |
|---|---|
| No cancellation | Child sessions run until complete; no kill mechanism |
| No timeout | A stuck child session runs indefinitely |
| Registry not persisted | After gateway restart, Fleet panel loses tracking (history still in DB) |
| Completion not auto-detected | Status must be updated manually by calling `openclaw_update_child_session_status` |
| No orchestrator feedback loop | Parent session does not automatically receive child results |

---

## Choosing Between the Two Systems

```
┌─────────────────────────────────────────────────────────┐
│                                                         │
│   Do you need the result back in the parent's LLM?     │
│                                                         │
│      YES → System A (spawn_subagent tool)               │
│      NO  → either works                                 │
│                                                         │
│   Does the task need memory / identity context?         │
│                                                         │
│      YES → System B (session-based)                     │
│      NO  → System A (faster, simpler)                   │
│                                                         │
│   Should the user be able to talk to the sub-agent?    │
│                                                         │
│      YES → System B                                     │
│      NO  → System A                                     │
│                                                         │
│   Must the task survive a gateway restart?              │
│                                                         │
│      YES → System B                                     │
│      NO  → System A                                     │
│                                                         │
│   Running multiple parallel analyses?                   │
│                                                         │
│      Short / focused → System A (up to 5 parallel)     │
│      Long / complex  → System B                         │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### Practical examples

```
# System A — agent orchestration within a single turn
spawn_subagent(
  name: "code-reviewer",
  task: "Review the diff below for security issues:\n\n{diff}",
  tools: ["agent_think"],   // Only thinking, no side effects
  wait: true
)

spawn_subagent(
  name: "web-researcher",
  task: "Find the latest CVE advisories for libssl",
  tools: ["http"],
  wait: false   // Fire and forget, check with list_subagents later
)

# System B — long-running background project agent
openclaw_spawn_session(
  agent_id: "main",
  task: "Refactor the authentication module. Start with auth.rs.",
  parent_session: "agent:main"
)
// The user can open this session, talk to it, guide it,
// and it persists everything it does to the database.
```

---

## Shared Infrastructure

Both systems use the same underlying LLM provider, safety layer, and tool registry — configured once in `IronClawState::build_inner()` and shared via `Arc` clones.

```
IronClawState::build_inner()
  │
  ├── components.llm          ─────────────────┐
  ├── components.safety       ──────────────┐  │
  ├── components.tools        ───────────┐  │  │
  ├── channel_manager         ────────┐  │  │  │
  │                                   │  │  │  │
  │                                   ▼  ▼  ▼  ▼
  ├── SubagentExecutor(tools, safety, llm, channels)
  │
  └── AgentDeps(tools, safety, llm, subagent_executor)
        → Agent (main agentic loop)
              → dispatcher.rs (intercepts spawn_subagent)
                    → SubagentExecutor.spawn()
```

Sub-agents spawned via System A inherit **the same model and safety rules** as the parent. They cannot circumvent safety sanitization or cost guardrails.

---

## Comparison with OpenClaw

| Feature | OpenClaw | IronClaw System A | IronClaw System B |
|---|---|---|---|
| Tool-initiated spawning | `sessions_spawn` tool | `spawn_subagent` tool | N/A |
| CLI-initiated spawning | `/subagents spawn` | N/A | `openclaw_spawn_session` |
| Result returned to parent | ✅ completion handoff | ✅ `SubagentResult` JSON | ❌ Event only |
| Inspect sub-agents | `/subagents list` | `list_subagents` tool | Fleet panel UI |
| Kill sub-agents | `/subagents kill` | `cancel_subagent` tool | ❌ Not yet |
| Nested sub-agents | `maxSpawnDepth` config | `allow_nested` config | N/A |
| Max concurrency | N/A documented | 5 (configurable) | N/A |
| Persistence | ✅ Session-based | ❌ Ephemeral | ✅ Session-based |
| Memory/identity access | Inherits parent | ❌ None | ✅ Full |
| Streaming to user | ✅ | ✅ via emit_user_message | ✅ Full streaming |

---

## Key Source Files

| File | Role |
|---|---|
| `ironclaw/src/agent/subagent_executor.rs` | System A: executor, runner, config, status types |
| `ironclaw/src/tools/builtin/subagent.rs` | System A: SpawnSubagentTool, ListSubagentsTool, CancelSubagentTool |
| `ironclaw/src/agent/dispatcher.rs` (L~1142) | System A: intercepts tool call, routes to executor |
| `backend/src/openclaw/ironclaw_bridge.rs` | System A wiring (SubagentExecutor created, injected into AgentDeps) |
| `backend/src/openclaw/commands/rpc.rs` | System B: openclaw_spawn_session, registry, update_child_status |
| `backend/src/openclaw/commands/types.rs` | Shared types: SpawnSessionResponse, ChildSessionInfo |
| `backend/src/openclaw/ui_types.rs` | UiEvent::SubAgentUpdate definition |
| `frontend/src/components/openclaw/SubAgentPanel.tsx` | System B: in-chat child session list + spawn input |
| `frontend/src/components/openclaw/fleet/FleetCommandCenter.tsx` | System B: global fleet orchestration view |
| `frontend/src/lib/openclaw.ts` | Frontend API: spawnSession(), listChildSessions() |
