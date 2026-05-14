# Audit Closure: Wave 7 + Wave 8

This document records closure for the Wave 7 and Wave 8 remediation scope from the full audit plan.

## Wave 7: Experiments, Worker/Orchestrator, History, Evaluation

1. Owner-scoped reads moved to storage boundary:
- Added owner-scoped `ExperimentStore` read methods in `src/db/mod.rs`.
- Implemented owner-joined queries in:
  - `src/history/experiments.rs`
  - `src/db/libsql/experiments.rs`
  - `src/db/postgres.rs` (delegation path)
- API now uses storage-scoped methods in `src/api/experiments.rs` for campaign/trial/artifact reads.

2. Worker completion event schema standardized:
- Canonical completion fields now flow through `CompletionReport`:
  - `status`, `session_id`, `success`, `message`, `iterations`
- Updated:
  - `src/worker/api.rs`
  - `src/worker/runtime.rs`
  - `src/worker/claude_bridge.rs`
  - `src/worker/codex_bridge.rs`
  - `src/orchestrator/api.rs`
  - `src/orchestrator/job_manager.rs`

3. Structured tool result preservation:
- `job_tool_result` SSE now carries:
  - `output` (legacy projection)
  - `output_text` (canonical text projection)
  - `output_json` (canonical structured projection)
- Updated:
  - `src/channels/web/types.rs`
  - `src/orchestrator/api.rs`
  - worker event producers in:
    - `src/worker/runtime.rs`
    - `src/worker/claude_bridge.rs`
    - `src/worker/codex_bridge.rs`

4. Gateway URL validation tightened:
- Auto-launch now fails fast if `gateway_url` is empty:
  - `src/experiments/adapters.rs`
- Remote runner also rejects empty gateway URL:
  - `src/experiments/runner.rs`

5. Job mode observability contract:
- Unknown persisted job mode values are surfaced as runtime mode `unknown` with raw source preserved in `unknown_job_mode_raw` for diagnostics:
  - `src/channels/web/handlers/jobs.rs`
  - `src/channels/web/types.rs`

6. Deterministic top-error ordering:
- `MetricsCollector::summary()` now sorts errors by:
  - descending count, then ascending error name
- Updated:
  - `src/evaluation/metrics.rs`

7. Feature prerequisite parity:
- Research/experiments docs now call out backend prerequisites explicitly (`postgres` and `libsql`) and remove implicit postgres-only drift:
  - `docs/RESEARCH_AND_EXPERIMENTS.md`

## Wave 8: Docs, Parity, Stale Cleanup

1. Stale module outcomes (explicit):
- Kept:
  - `src/channels/status_view.rs` (actively wired via manager + Tauri command path)
- Removed as stale/unwired:
  - `src/agent/routine_audit.rs`
  - `src/agent/management_api.rs`
  - `src/agent/presence.rs`
  - `src/channels/tool_stream.rs`
  - `src/tools/toolset.rs`
  - `src/media/media_cache_config.rs`
- Export/module surface updated in:
  - `src/agent/mod.rs`
  - `src/channels/mod.rs`
  - `src/tools/mod.rs`
  - `src/media/mod.rs`

2. Parity/docs reconciliation:
- Research ownership-scoping language updated:
  - `docs/RESEARCH_AND_EXPERIMENTS.md`
- Security doc updated for keychain cache opt-in:
  - `docs/SECURITY.md`
- Security doc remote secret backend wording now matches runtime support:
  - `docs/SECURITY.md`
- Feature parity matrix aligned to runtime:
  - `FEATURE_PARITY.md`
- Audit plan module paths cleaned:
  - `docs/CODEBASE_AUDIT_PLAN.md`

3. Release hygiene gates:
- Package source trees are guarded against tracked Cargo `target/` artifacts:
  - `tests/repo_hygiene.rs`
- System status now exposes LLM runtime revision, health, and last error alongside legacy fields:
  - `src/api/system.rs`

## Notes

- This closure map is file-level because commit slicing for release PRs is still pending.
- Suggested release slicing: one PR for Wave 7 runtime/schema/storage, one PR for Wave 8 stale cleanup + docs parity.
