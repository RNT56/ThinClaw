# Memory And Growth

This document is the canonical public overview for ThinClaw's continuity model across sessions and surfaces.

## Core Concepts

- `MEMORY.md` is curated long-term memory.
- `daily/YYYY-MM-DD.md` captures recent raw context.
- `HEARTBEAT.md` drives proactive background follow-up.
- `SOUL.md`, `USER.md`, and `AGENTS.md` capture identity, user context, and working conventions.
- The Learning Ledger records explicit learning events, evaluations, proposals, rollbacks, and outcome-backed follow-up signals.

## User-Facing Vocabulary

ThinClaw's primary memory and continuity terms are:

- `/compress` for context compaction (`/compact` remains a compatibility alias)
- `/summarize` for thread summaries
- `memory_search`, `memory_read`, `memory_write`, and `memory_tree` for workspace memory operations
- `prompt_manage` for durable `SOUL.md`, `USER.md`, and `AGENTS.md` rewrites
- the Learning Ledger `Outcomes` section for deferred consequence review

## What Gets Saved Where

- Routine notes and facts: `memory_write`
- Curated long-term continuity: `MEMORY.md`
- Day-by-day working context: `daily/`
- Durable identity and collaboration guidance: `SOUL.md`, `USER.md`, `AGENTS.md`
- Deferred downstream usefulness and durability signals: Learning Ledger outcome contracts and observations

Use [../src/workspace/README.md](../src/workspace/README.md) for the code-adjacent workspace model and [../FEATURE_PARITY.md](../FEATURE_PARITY.md) for the current learning and continuity feature ledger.

For the current outcome-backed learning behavior, manual review semantics, and rollout roadmap, use [OUTCOME_BACKED_LEARNING.md](OUTCOME_BACKED_LEARNING.md).
