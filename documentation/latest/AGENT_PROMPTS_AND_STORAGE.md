# Agent Prompts & Storage Architecture

> **Scope:** IronClaw library (`ironclaw/`) as embedded in Scrappy.  
> **Last updated:** 2026-03-08

---

## 1. Two Storage Spaces

The agent has **two completely separate storage layers**. Confusing them is the most common source of unexpected behaviour.

| Layer | Name | Backend | Tool | Used for |
|---|---|---|---|---|
| **Agent memory** | DB workspace | libSQL / SQLite (on-device) | `memory_write` / `memory_read` | SOUL.md, MEMORY.md, IDENTITY.md, daily logs, HEARTBEAT.md |
| **Host filesystem** | Device filesystem | Real OS filesystem | `write_file` / `read_file` / `shell` | Code projects, websites, user documents, anything the user opens/runs |

### Key rule injected into every prompt
```
Agent memory (SOUL.md, MEMORY.md, daily logs) → use memory_write (database)
User-visible files (code, websites, documents) → use write_file (filesystem)
```

### The "workspace" terminology trap
The word **"workspace"** is overloaded and was causing confusion:

- Inside `AGENTS.md` (agent identity): "workspace" historically meant the DB memory space
- Inside `Desktop Capabilities` (system prompt): "workspace" can mean the filesystem root (sandboxed/project modes)

**Resolution applied:**
- `AGENTS.md` seed: "Work within this workspace" → "Work within your agent memory (read/write via `memory_write`)"
- `Desktop Capabilities` section uses "Desktop Capabilities" or "device" terminology, never "workspace"
- Safety line: "For internal memory writes, just do it" → "For memory/identity writes (use `memory_write`), just do it"

---

## 2. System Prompt Assembly Pipeline

Every LLM turn assembles the system prompt from multiple sources, in this order.

> **Design note:** There is intentionally **no preamble** (no "You are a personal AI assistant…").  
> Identity comes entirely from `SOUL.md` and `IDENTITY.md` injected in **Project Context**.  
> This matches the Openclaw philosophy: the agent discovers and defines its own nature  
> through bootstrap rather than having it pre-declared by the framework.

```
┌──────────────────────────────────────────────────────┐
│  1. ## Tooling                                        │
│     Auto-generated from registered tools              │
│     Each tool's .name() and .description() injected   │
│     ironclaw/src/tools/builtin/                       │
├──────────────────────────────────────────────────────┤
│  2. ## Safety                                         │
│     Hardcoded safety rules                            │
│     ironclaw/src/llm/reasoning.rs  ~line 976          │
├──────────────────────────────────────────────────────┤
│  3. ## Extensions  (if extension tools available)     │
│     Channel/tool/MCP install guidance                 │
│     fn build_extensions_section()  ~line 1018         │
├──────────────────────────────────────────────────────┤
│  4. ## Desktop Capabilities  (if dev tools available) │
│     Mode-specific filesystem guidance                 │
│     fn build_workspace_capabilities_section() ~1039   │
│     Driven by AgentConfig.workspace_mode & root       │
├──────────────────────────────────────────────────────┤
│  5. ## Channel  (if non-default channel)              │
│     Discord/Telegram/WhatsApp formatting rules        │
│     fn build_channel_section()  ~line 1139            │
├──────────────────────────────────────────────────────┤
│  6. ## Runtime  (always present)                      │
│     host=device | channel=... | model=...             │
│     fn build_runtime_section()  ~line 1174            │
├──────────────────────────────────────────────────────┤
│  7. ## Group Chat  (if is_group_chat)                 │
│     Group participation rules                         │
│     fn build_group_section()  ~line 1195              │
├──────────────────────────────────────────────────────┤
│  8. ## Project Context  ← IDENTITY LIVES HERE         │
│     workspace_system_prompt from DB:                  │
│     AGENTS.md + SOUL.md + USER.md + IDENTITY.md      │
│     + MEMORY.md (non-group only, capped 20k chars)    │
│     + Today's + yesterday's daily logs (capped)       │
│     + HEARTBEAT.md (non-group, non-comment only)      │
│     + BOOT.md tasks (non-group, non-comment only)     │
│     ironclaw/src/workspace/mod.rs                     │
│     fn system_prompt_for_context()  ~line 553         │
├──────────────────────────────────────────────────────┤
│  9. ## Active Skills  (if skills loaded)              │
│     Skill SKILL.md content wrapped in <skill> tags   │
│     ironclaw/src/agent/dispatcher.rs  ~line 70        │
└──────────────────────────────────────────────────────┘
```

### Bootstrap exception
When `BOOTSTRAP.md` exists in the agent's DB workspace, **only that file** is returned as the entire system prompt. All other sections (tooling, safety, identity files) are withheld. This ensures the agent starts completely blank.

```rust
// ironclaw/src/workspace/mod.rs  ~line 567
if !is_group_chat {
    if let Ok(doc) = self.read(paths::BOOTSTRAP).await {
        if !doc.content.is_empty() {
            return Ok(doc.content); // Only this. Nothing else.
        }
    }
}
```

---

## 3. Where to Find and Configure Each Part

### 3.1 Hardcoded prompts (Rust source)

| What | File | Location |
|---|---|---|
| Preamble, Safety, Tooling section | `ironclaw/src/llm/reasoning.rs` | `build_conversation_prompt()` ~L951 |
| Desktop Capabilities (sandboxed) | `ironclaw/src/llm/reasoning.rs` | `build_workspace_capabilities_section()` ~L1076 |
| Desktop Capabilities (project) | `ironclaw/src/llm/reasoning.rs` | `build_workspace_capabilities_section()` ~L1095 |
| Desktop Capabilities (unrestricted) | `ironclaw/src/llm/reasoning.rs` | `build_workspace_capabilities_section()` ~L1115 |
| Channel formatting (Discord/Telegram etc.) | `ironclaw/src/llm/reasoning.rs` | `build_channel_section()` ~L1139 |
| Group chat rules | `ironclaw/src/llm/reasoning.rs` | `build_group_section()` ~L1195 |
| Tool descriptions (seen by LLM) | `ironclaw/src/tools/builtin/*.rs` | Each tool's `fn description()` |
| `memory_write` description | `ironclaw/src/tools/builtin/memory.rs` | `MemoryWriteTool::description()` ~L148 |
| `write_file` description | `ironclaw/src/tools/builtin/file.rs` | `WriteFileTool::description()` ~L302 |

### 3.2 Seed files (DB-seeded on first boot, user-editable)

These are written to the DB workspace on first boot if missing. Users can edit them via the agent in chat.

| File | Seed source | Purpose |
|---|---|---|
| `BOOTSTRAP.md` | `ironclaw/src/workspace/mod.rs` ~L935 | First-run ritual (deleted after setup) |
| `AGENTS.md` | `ironclaw/src/workspace/mod.rs` ~L815 | Session routine, memory rules, operational instructions |
| `SOUL.md` | `ironclaw/src/workspace/mod.rs` ~L790 | Core values, personality, boundaries |
| `IDENTITY.md` | `ironclaw/src/workspace/mod.rs` ~L775 | Name, creature, vibe, emoji |
| `USER.md` | `ironclaw/src/workspace/mod.rs` ~L887 | Information about the user |
| `TOOLS.md` | `ironclaw/src/workspace/mod.rs` ~L902 | Environment-specific notes (project paths, device names) |
| `MEMORY.md` | `ironclaw/src/workspace/mod.rs` ~L766 | Long-term curated memory |
| `HEARTBEAT.md` | `ironclaw/src/workspace/mod.rs` ~L920 | Background task checklist |
| `BOOT.md` | `ironclaw/src/workspace/mod.rs` ~L921 | Startup hook (runs on every boot) |

> **Note:** Seed content is verbatim from the Openclaw reference implementation.  
> Files only seed if the path does not exist — user edits are **never overwritten**.

### 3.3 Runtime configuration (Gateway Settings → AgentConfig)

Configured in Scrappy's Gateway Settings UI, stored in `ironclaw/src/config/agent.rs`:

| Config field | Type | Effect on prompt |
|---|---|---|
| `workspace_mode` | `"unrestricted"` \| `"sandboxed"` \| `"project"` | Selects which Desktop Capabilities block is injected |
| `workspace_root` | `Option<PathBuf>` | The `{root}` placeholder in sandboxed/project blocks |
| `max_tool_iterations` | `usize` | Controls agentic loop ceiling |
| `max_context_messages` | `usize` | Hard cap on history sent to LLM |
| `thinking_enabled` | `bool` | Extended chain-of-thought |

**Wire-up path:**
```
Gateway Settings UI (Scrappy/Swift)
  → AgentConfig (ironclaw/src/config/agent.rs)
    → Reasoning::with_workspace_mode() (dispatcher.rs ~L109)
      → build_workspace_capabilities_section() (reasoning.rs)
        → Injected into system prompt every turn
```

---

## 4. Context Window Management

### Per-file cap
Large workspace files are truncated before injection to prevent token exhaustion:

```rust
// ironclaw/src/workspace/mod.rs
const FILE_MAX_CHARS: usize = 20_000; // ~5k tokens
```

Files affected: AGENTS.md, SOUL.md, USER.md, IDENTITY.md, MEMORY.md, daily logs, HEARTBEAT.md.

### Hard message history cap
```rust
// ironclaw/src/config/agent.rs (default: 200)
max_context_messages: usize
```
System messages are always kept. Oldest non-system messages are dropped first.

### Tool-result pruning
Before each LLM call in the agentic loop, tool results from turns older than the last 3 are replaced with `[tool result pruned — see session history]`. This prevents token burn in long multi-tool sessions.

```rust
// ironclaw/src/agent/dispatcher.rs ~L288
const TOOL_RESULT_KEEP_TURNS: usize = 3;
```

The JSONL/DB history is never modified — only the in-memory slice sent to the LLM.

---

## 5. Tool Routing Guard

The `write_file` tool **actively rejects** known workspace paths:

```rust
// ironclaw/src/tools/builtin/file.rs
const WORKSPACE_FILES: &[&str] = &[
    "HEARTBEAT.md", "MEMORY.md", "IDENTITY.md", "SOUL.md",
    "AGENTS.md", "USER.md", "README.md",
];

// If path matches, returns an error directing the LLM to use memory_write
if is_workspace_path(path_str) {
    return Err("Use memory_write instead of write_file for workspace files.");
}
```

Paths matching `daily/`, `context/` prefixes are also blocked.

---

## 6. Lifecycle Events

Every agent turn emits lifecycle SSE events for frontend indicator support:

| Event | When | Payload |
|---|---|---|
| `LifecycleStart` | Immediately before first LLM call | `{ run_id, phase: "start" }` |
| `LifecycleEnd` | On all exit paths | `{ run_id, phase: "response"/"interrupted"/"error" }` |

```rust
// ironclaw/src/channels/channel.rs
pub enum StatusUpdate {
    LifecycleStart { run_id: String },
    LifecycleEnd { run_id: String, phase: String },
    // ...
}
```

Emitted in: `ironclaw/src/agent/thread_ops.rs` ~L298 and ~L368.

---

## 7. In-Chat Commands

Users can query agent internals directly in chat:

| Command | Output |
|---|---|
| `/status` | Model name, workspace mode |
| `/context` | Which workspace files are loaded, with char counts |
| `/context detail` | Full content of each loaded source |
| `/help` | All available commands |

Implemented in: `ironclaw/src/agent/commands.rs` and `ironclaw/src/agent/submission.rs`.

---

## 8. Quick Reference: Which Tool for What

```
User says "remember X"          → memory_write (target: "memory" or "daily_log")
User says "update your soul"    → memory_write (target: "SOUL.md", append: false)
User says "note in heartbeat"   → memory_write (target: "heartbeat")
User says "create a website"    → write_file + shell (host filesystem)
User says "build a project"     → write_file + shell (check TOOLS.md for path first)
User says "run this script"     → shell (host filesystem)
User says "what did I tell you" → memory_search (DB workspace search)
```

---

## 9. File Map Summary

```
ironclaw/src/
├── workspace/mod.rs           ← DB workspace, seed files, system_prompt_for_context()
├── llm/reasoning.rs           ← Hardcoded prompt sections, assembled every turn
├── agent/
│   ├── dispatcher.rs          ← Wires AgentConfig → Reasoning, tool-result pruning
│   ├── thread_ops.rs          ← Lifecycle events (Start/End)
│   ├── commands.rs            ← /status, /context, /help handlers
│   └── submission.rs          ← Command parser
├── tools/builtin/
│   ├── memory.rs              ← memory_write, memory_read, memory_search, memory_tree
│   └── file.rs                ← write_file, read_file, list_dir (with workspace guard)
└── config/agent.rs            ← AgentConfig (workspace_mode, workspace_root, etc.)
```
