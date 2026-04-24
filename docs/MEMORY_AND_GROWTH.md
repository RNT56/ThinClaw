# Memory And Growth

This document is the canonical public overview for ThinClaw's continuity model across sessions and surfaces.

## Core Concepts

- `MEMORY.md` is curated long-term memory.
- `daily/YYYY-MM-DD.md` captures recent raw context.
- `HEARTBEAT.md` drives proactive background follow-up.
- `THINCLAW_HOME/SOUL.md` is the canonical soul.
- `SOUL.local.md` is an optional workspace overlay.
- `USER.md` and `AGENTS.md` capture user context and working conventions.
- The Learning Ledger records explicit learning events, evaluations, proposals, rollbacks, and outcome-backed follow-up signals.

## User-Facing Vocabulary

ThinClaw's primary memory and continuity terms are:

- `/compress` for context compaction (`/compact` remains a compatibility alias)
- `/summarize` for thread summaries
- `memory_search`, `memory_read`, `memory_write`, and `memory_tree` for workspace memory operations
- `prompt_manage` for durable `SOUL.md`, `SOUL.local.md`, `USER.md`, and `AGENTS.md` rewrites
- the Learning Ledger `Outcomes` section for deferred consequence review

## What Gets Saved Where

- Routine notes and facts: `memory_write`
- Curated long-term continuity: `MEMORY.md`
- Day-by-day working context: `daily/`
- Durable identity and collaboration guidance: canonical `SOUL.md`, optional `SOUL.local.md`, `USER.md`, `AGENTS.md`
- Deferred downstream usefulness and durability signals: Learning Ledger outcome contracts and observations

Use [../src/workspace/README.md](../src/workspace/README.md) for the code-adjacent workspace model and [../FEATURE_PARITY.md](../FEATURE_PARITY.md) for the current learning and continuity feature ledger.

For the current outcome-backed learning behavior, manual review semantics, and rollout roadmap, use [OUTCOME_BACKED_LEARNING.md](OUTCOME_BACKED_LEARNING.md).

## Stable Prompt Freezing

ThinClaw now freezes the project/workspace prompt block at session runtime by default.

- `prompt.session_freeze_enabled = true` keeps the stable workspace/provider prompt blocks fixed across turns within the same thread runtime.
- `prompt.project_context_max_tokens` caps sanitized project-context payloads before they become part of the stable prompt.
- The runtime records a stable prompt hash and logs a cache-bust event if the effective stable prompt changes.

This keeps project guidance cache-friendly while leaving ephemeral recall, channel hints, and post-compaction fragments free to change turn by turn.

## External Memory Providers

The external memory layer still supports active-provider recall/export flows, but it now also supports:

- provider-specific stable prompt blocks
- provider setup/off through agent tools
- optional Honcho user-modeling injection via `cadence`, `depth`, and `user_modeling_enabled`
- `custom_http` provider support through the registry-backed config map

Backwards compatibility is preserved for legacy `learning.providers.honcho` and `learning.providers.zep` settings.

`custom_http` expects either `base_url` or explicit `recall_url` / `sync_url` values. Recall requests are POSTed as `{ user_id, query, limit }`; sync requests are POSTed as `{ user_id, payload }`. Responses may be an array or an object with `results`/`memories`, where each item exposes `summary`, `text`, or `content`.
