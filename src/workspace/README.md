# Workspace & Memory System

The workspace provides persistent memory for agents with a flexible filesystem-like structure, a durable identity stack, and hybrid search.

## Key Principles

1. **"Memory is database, not RAM"** - If you want to remember something, write it explicitly
2. **Flexible structure** - Create any directory/file hierarchy you need
3. **Self-documenting** - Use README.md files to describe directory structure
4. **Hybrid search** - Combines FTS (keyword) + vector (semantic) via Reciprocal Rank Fusion

## Filesystem Structure

```
workspace/
├── README.md              <- Root runbook/index
├── MEMORY.md              <- Long-term curated memory
├── HEARTBEAT.md           <- Periodic checklist
├── IDENTITY.md            <- Agent name, nature, and presentation
├── SOUL.local.md          <- Optional workspace overlay (explicit-only)
├── AGENTS.md              <- Behavior instructions
├── USER.md                <- User context
├── context/               <- Identity-related docs
│   ├── vision.md
│   └── priorities.md
├── daily/                 <- Daily logs
│   ├── 2024-01-15.md
│   └── 2024-01-16.md
├── projects/              <- Arbitrary structure
│   └── alpha/
│       ├── README.md
│       └── notes.md
└── ...
```

## Using the Workspace

```rust
use crate::workspace::{Workspace, OpenAiEmbeddings, paths};

// Create workspace for a user
let workspace = Workspace::new("user_123", pool)
    .with_embeddings(Arc::new(OpenAiEmbeddings::new(api_key)));

// Read/write any path
let doc = workspace.read("projects/alpha/notes.md").await?;
workspace.write("context/priorities.md", "# Priorities\n\n1. Feature X").await?;
workspace.append("daily/2024-01-15.md", "Completed task X").await?;

// Convenience methods for well-known files
workspace.append_memory("User prefers dark mode").await?;
workspace.append_daily_log("Session note").await?;

// List directory contents
let entries = workspace.list("projects/").await?;

// Search (hybrid FTS + vector)
let results = workspace.search("dark mode preference", 5).await?;

// Get system prompt from identity files
let prompt = workspace.system_prompt().await?;
```

The onboarding-selected `personality_pack` seeds the canonical home `SOUL.md` in `THINCLAW_HOME`. Workspaces inherit that soul by default; `SOUL.local.md` exists only when explicitly created. Temporary `/personality` overlays do not rewrite durable files.

## Memory Tools

Four tools for LLM use:

- **`memory_search`** - Hybrid search, MUST be called before answering questions about prior work
- **`memory_write`** - Write to any path (memory, daily_log, or custom paths)
- **`memory_read`** - Read any file by path
- **`memory_tree`** - View workspace structure as a tree (depth parameter, default 1)

## Hybrid Search (RRF)

Combines full-text search and vector similarity using Reciprocal Rank Fusion:

```
score(d) = Σ 1/(k + rank(d)) for each method where d appears
```

Default k=60. Results from both methods are combined, with documents appearing in both getting boosted scores.

**Backend differences:**
- **PostgreSQL:** `ts_rank_cd` for FTS, pgvector cosine distance for vectors, full RRF
- **libSQL:** FTS5 for keyword search, `libsql_vector_idx` with `vector_top_k` for vector search, full RRF fusion. Note: MMR diversity re-ranking is a no-op (vector results don't include embeddings).

## Heartbeat System

Proactive periodic execution (default: 30 minutes):

1. Reads `HEARTBEAT.md` checklist
2. Runs an agent turn with the checklist prompt
3. Honors the heartbeat `target` knob: `none` runs silently (log only),
   `chat` delivers to the default surface, and a channel name overrides the
   delivery channel; `include_reasoning` retains the reasoning chain in the
   emitted summary
4. If nothing needs attention, the agent replies `HEARTBEAT_OK` (no delivery)

Heartbeat scheduling is owned by the **routine engine**, not a standalone
loop. The agent loop registers a heartbeat routine at startup via
`upsert_heartbeat_routine` (`src/agent/agent_loop/heartbeat.rs`), and the engine fires
it on its cron schedule like any other routine. The interactive `/heartbeat`
command runs a one-shot check through `HeartbeatRunner::check_heartbeat`.

Heartbeat behavior is configured through `HeartbeatConfig`
(`crates/thinclaw-config/src/heartbeat.rs`), which is resolved from settings
and environment and threaded into the registered `RoutineAction::Heartbeat`.

## Chunking Strategy

Documents are chunked for search indexing:
- Default: 800 words per chunk (roughly 800 tokens for English)
- 15% overlap between chunks for context preservation
- Minimum chunk size: 50 words (tiny trailing chunks merge with previous)
