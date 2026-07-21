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

## Identity And Namespace Boundaries

Every memory operation is first scoped to a canonical principal, then to the exact conversation context:

- Direct conversations resolve caller-relative paths such as `MEMORY.md`, `USER.md`, `daily/`, and custom notes below `actors/<actor>/`.
- Group conversations resolve the same relative paths below `conversations/<canonical-scope-uuid>/`. A group context without that canonical scope fails closed.
- `shared/` is principal-wide knowledge. Conversation tools may read it but may not mutate it.
- `SOUL.md`, `SOUL.local.md`, `AGENTS.md`, root `IDENTITY.md`, hooks, and skills are trusted control-plane material. Unknown files default to conversation-authored evidence rather than silently becoming system instructions.
- Transcript recall uses the authoritative job principal/actor. Group recall is additionally authorized against the exact persisted conversation ID and stable conversation scope.

Gateway memory requests can explicitly select `conversation` or `principal_admin` scope. Omitted scope always means `conversation`, even for an Admin credential; principal-wide access must be requested explicitly. Conversation list/search responses use caller-relative paths and never expose internal actor or conversation prefixes.

ThinClaw Desktop presents a deliberate composite view: trusted control files plus the current actor's relative knowledge. Both local and remote mode use the same classifier, so editing `MEMORY.md` or a daily log changes what chat and routines recall, while editing `SOUL.md` changes the canonical home soul. Root `IDENTITY.md` is the trusted agent identity; the current direct actor's private identity overlay is exposed without collision as `actor/IDENTITY.md`. Sibling actor/group namespaces are excluded even when the remote connection uses an Admin token, and callers cannot address hidden canonical `actors/` or `conversations/` paths directly.

Startup hooks, `/context`, manual heartbeats, scheduled heartbeats, profile evolution, and agent-to-agent context all resolve through the same actor/conversation boundary. Preloaded startup memory is labeled as untrusted evidence rather than control instructions.

Learning candidates carry a reserved identity envelope copied from the persisted event record. Auto-apply recomputes direct scopes, verifies group candidates against the exact durable conversation, never invents a missing group scope, and requires the principal owner for global prompt, routine, skill, or code mutations. The gateway Learning Ledger is an Admin-only control surface because its candidates and artifact history are principal-wide.

Older desktop databases used the principal `default` and stored personal knowledge at the principal root. Startup copies missing documents into `local_user` with compare-and-swap protection, and the first owner-scoped access migrates legacy personal knowledge into the actor namespace. Durable hidden markers make both migrations resumable and prevent a later clear from repopulating old data.

## Stable Prompt Contracts And Live Knowledge

ThinClaw pins the prompt compiler contract for a thread while refreshing mutable knowledge on every turn.

- `prompt.session_freeze_enabled = true` keeps the selected prompt-contract rollout stable for the thread. It does not freeze memory or provider knowledge.
- Workspace and provider prompt blocks are refreshed each turn, so approved edits, provider changes, actor scope changes, and local-day rollovers take effect without starting a new thread.
- The last successful workspace block is retained only as a fallback for a transient workspace read failure. A disabled or unavailable provider fails closed instead of replaying stale provider instructions.
- `prompt.project_context_max_tokens` caps sanitized project-context payloads before they become part of the stable prompt.
- The runtime records a stable prompt hash and logs a cache-bust event if the effective stable prompt changes.

This preserves rollout consistency and cache observability without making durable knowledge stale for the lifetime of a task.

When context compaction summarizes older turns (automatically or via `/compress`), the generated summary is folded into the post-compaction fragment under a `## Summary of Earlier Conversation` heading, so the model keeps the gist of the dropped turns instead of resuming with no memory of them. The fragment persists in the thread runtime and survives rehydration.

## External Memory Providers

The external memory layer still supports active-provider recall/export flows, but it now also supports:

- provider-specific stable prompt blocks
- provider setup/off through agent tools
- secret-safe provider setup/off and health inspection through Desktop Learning
  Review (environment-variable references only; raw API keys are not accepted)
- optional Honcho user-modeling injection via `cadence`, `depth`, and `user_modeling_enabled`
- first-class provider adapters for `mem0`, `openmemory`, `letta`, `chroma`, and `qdrant`
- `custom_http` provider support through the registry-backed config map
- explicit `external_memory_export` for important facts or summaries that should be mirrored immediately

Backwards compatibility is preserved for legacy `learning.providers.honcho` and `learning.providers.zep` settings.

`custom_http` expects either `base_url` or explicit `recall_url` / `sync_url` values. Recall requests are POSTed as `{ user_id, query, limit }`; sync requests are POSTed as `{ user_id, payload }`. Responses may be an array or an object with `results`/`memories`, where each item exposes `summary`, `text`, or `content`.

`mem0` defaults to the hosted Mem0 API and accepts `api_key` or `api_key_env`. `openmemory` defaults to a local Mem0/OpenMemory REST server at `http://localhost:8888`. `letta` requires `agent_id` and uses archival memory search/export paths. `chroma` and `qdrant` require `embedding_url` plus `collection_id` or `collection` respectively, because their native APIs need vectors for recall and upsert.

Desktop provider mutation intentionally runs only against the embedded local
runtime because the gateway exposes health but not credential-bearing provider
configuration. The panel persists `api_key_env` / `embedding_api_key_env` names,
never secret values, and the generated IPC route returns a typed local-only gate
when Desktop is connected to a remote gateway.
