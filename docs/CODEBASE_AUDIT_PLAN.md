# Codebase Audit Plan

This document turns the current ThinClaw repo into parallel audit lanes so workers can inspect the codebase end to end without overlapping too much.

Use it for:

- broken code
- stale or unused code
- broken features
- feature gaps or incomplete features
- code style
- file structure
- code quality
- wire completion
- end-to-end flow completion

## How To Use This

1. Assign each worker one lane.
2. Have every worker follow the template at the end of this document.
3. Require each worker to inspect both code and the canonical docs for their lane.
4. Merge findings into one cross-cutting pass for duplication, drift, and dead code.

## Recommended Audit Lanes

| Lane | Focus | Primary code paths | Core flow to verify |
|---|---|---|---|
| 1 | Bootstrap, config, and onboarding | `src/main.rs`, `src/bootstrap.rs`, `src/main_helpers.rs`, `src/config/*`, `src/setup/*`, `docs/DEPLOYMENT.md`, `src/setup/README.md` | process start -> env/TOML/DB/secret resolution -> onboarding -> runtime readiness |
| 2 | App assembly and dependency wiring | `src/app.rs`, `src/lib.rs`, `src/settings.rs`, `src/hooks/bootstrap.rs` | config resolved -> DB/secrets/LLM/tools/workspace/hooks assembled -> runtime components exposed once |
| 3 | CLI, service, and operational wrappers | `src/cli/*`, `src/service.rs` | CLI command -> runtime call -> status/output -> service lifecycle |
| 4 | REPL, TUI, and shared command vocabulary | `src/channels/repl.rs`, `src/tui/mod.rs`, `src/agent/command_catalog.rs`, `docs/SURFACES_AND_COMMANDS.md` | user command -> parser/catalog -> action -> visible response across local surfaces |
| 5 | Web gateway and business API | `src/channels/web/*`, `src/api/*`, `src/channels/canvas_gateway.rs` | HTTP/WebSocket/SSE request -> handler -> core API -> runtime side effects -> UI payload |
| 6 | Channel manager and native channels | `src/channels/manager.rs`, `src/channels/channel.rs`, `src/channels/{http,signal,gmail,apple_mail,imessage,nostr,discord}.rs`, `src/channels/{gmail_wiring,imessage_wiring,health_monitor,status_view,tool_stream}.rs`, `channels-docs/*` | inbound event -> identity/session resolution -> agent turn -> outbound formatting/delivery |
| 7 | WASM channels and packaged channel registry | `src/channels/wasm/*`, `channels-src/*`, `registry/channels/*`, `registry/_bundles.json`, `docs/CHANNEL_ARCHITECTURE.md`, `channels-docs/*` | manifest/package -> load -> route -> host capability use -> delivery |
| 8 | Agent runtime core | `src/agent/{agent_loop,dispatcher,dispatcher_helpers,router,session,session_manager,thread_runtime,thread_ops,global_session,command_catalog,commands}.rs` | message -> routing/session/thread -> LLM/tool loop -> persistence/streaming |
| 9 | Routines, heartbeat, subagents, self-repair, and agent management | `src/agent/{routine,routine_engine,routine_audit,scheduler,heartbeat,subagent_executor,agent_registry,management_api,self_repair,worker,job_monitor,presence,undo,compaction}.rs` | scheduled/background work -> delegated work -> recovery/progress -> audit trail |
| 10 | LLM routing and provider runtime | `src/llm/*`, `docs/LLM_PROVIDERS.md` | request -> route planning -> provider build -> failover/retry/cache -> cost/usage/credential sync |
| 11 | Workspace, context, and memory | `src/workspace/*`, `src/context/*`, `docs/MEMORY_AND_GROWTH.md`, `src/workspace/README.md` | write/read/search -> context compaction -> reinjection -> continuity across sessions |
| 12 | Built-in tools and local execution surfaces | `src/tools/builtin/*`, `src/tools/{tool,toolset,registry,policy,rate_limiter,intent_display}.rs`, `src/tools/README.md` | tool registration -> approval/policy -> execution -> streaming/result formatting |
| 13 | WASM tools, MCP, extensions, registry, skills, and hooks | `src/tools/wasm/*`, `src/tools/mcp/*`, `src/extensions/*`, `src/registry/*`, `src/skills/*`, `src/hooks/*`, `tools-src/*`, `registry/tools/*`, `tools-docs/*`, `docs/EXTENSION_SYSTEM.md` | manifest/catalog -> install/discovery -> auth/secret injection -> runtime execution -> lifecycle hooks |
| 14 | Safety, secrets, database, pairing, platform, and observability | `src/safety/*`, `src/secrets/*`, `src/db/*`, `src/pairing/*`, `src/platform/*`, `src/observability/*`, `docs/SECURITY.md`, `src/NETWORK_SECURITY.md` | ingress/content/tool request -> policy check -> secret/storage access -> audit/logging -> safe completion |
| 15 | Identity, branding, media, and document extraction | `src/identity/*`, `src/branding/*`, `src/media/*`, `src/document_extraction/*`, `assets/personality_packs/*`, `assets/persona_seeds/*`, `docs/IDENTITY_AND_PERSONALITY.md` | identity/personality/media input -> normalization/extraction -> prompt/render output -> persistence |
| 16 | Experiments, orchestration, workers, history, estimation, and evaluation | `src/experiments/*`, `src/orchestrator/*`, `src/worker/*`, `src/history/*`, `src/estimation/*`, `src/evaluation/*`, `docs/RESEARCH_AND_EXPERIMENTS.md` | research project/campaign -> runner/worker launch -> execution -> metrics/history -> learning feedback |
| 17 | Docs, parity, and repository hygiene | `README.md`, `FEATURE_PARITY.md`, `docs/*`, `channels-docs/*`, `tools-docs/*`, `.github/*`, `deploy/*`, `Dockerfile*`, `docker-compose.yml` | documented feature -> actual wiring -> setup/testing/deploy path -> release hygiene |

## Cross-Cutting Watchlist

These are the strongest likely drift or incomplete-code signals found during the inventory pass. They are not confirmed bugs yet; they are high-value audit targets.

| Signal | Why it matters | Likely owners |
|---|---|---|
| Shared command docs are narrower than implemented commands | Command parity can drift across docs, REPL, and TUI | Lanes 3, 4, 17 |
| REPL command list and shared command catalog appear to be maintained in parallel | Duplicate command sources often create stale help, hidden commands, or mismatched behavior | Lanes 4, 8 |
| `src/api/*` and web handlers split similar logic | Duplicate business logic raises behavior drift risk between API and WebUI | Lanes 5, 17 |
| Compatibility aliases remain active (`/vibe`, `persona_seed`, `/compact`, legacy provider/settings aliases) | Useful for migration, but prime stale-code and inconsistent-UX risk | Lanes 4, 10, 15, 17 |
| Observability persistence looks env-heavy while setup persists many runtime settings elsewhere | Easy place for setup/runtime mismatch or settings that do not actually reload | Lanes 1, 14, 17 |
| Extension/plugin manifest logic appears split across `src/extensions/*` and `src/registry/*` | Parallel manifest systems can drift or duplicate validation/install responsibilities | Lanes 13, 17 |
| `history`, `route_planner`, and other areas contain `allow(dead_code)` or explicit future/legacy notes | Strong stale-or-partially-wired signal | Lanes 9, 10, 16 |
| Packaged tool/channel naming differs for Slack and Telegram | Name collisions or ambiguous install/auth UX can break routing and docs | Lanes 7, 13, 17 |
| `channels-src/telegram/target/` exists inside the source tree | Generated artifacts inside package source are a repo-hygiene and stale-code smell | Lanes 7, 17 |
| `src/api/system.rs` exposes placeholder-style status fields | User-facing status may not be authoritative yet | Lanes 5, 17 |

## Repo Shape Snapshot

Top-level `src/` file-count snapshot from this audit pass:

| Area | Approx. file count |
|---|---:|
| `src/tools` | 78 |
| `src/channels` | 66 |
| `src/agent` | 39 |
| `src/llm` | 31 |
| `src/cli` | 29 |
| `src/config` | 25 |
| `src/setup` | 18 |
| `src/safety` | 16 |
| `src/db` | 14 |
| `src/workspace` | 13 |
| `src/sandbox` | 13 |
| `src/media` | 13 |
| `src/skills` | 12 |
| `src/extensions` | 12 |
| `src/api` | 12 |

This is only for sizing the audit lanes. It is not a quality score.

## Worker Audit Template

Copy this into each worker prompt and replace the lane-specific scope.

```md
# ThinClaw Audit Worker

You are auditing one bounded slice of the ThinClaw repo.

## Scope

- Lane: <lane name>
- Primary paths:
  - <path 1>
  - <path 2>
  - <path 3>
- Canonical docs:
  - <doc 1>
  - <doc 2>

## Goals

Audit this scope for:

- broken code
- stale or unused code
- broken features
- feature gaps or incomplete features
- code style
- file structure
- code quality
- wire completion
- end-to-end flow completion

## Required Method

1. Build a feature map for your scope before judging quality.
2. Trace every major user or system flow end to end.
3. Verify wiring, not just implementation:
   - registration
   - config
   - CLI/API/UI exposure
   - runtime execution
   - persistence
   - observability
   - tests/docs
4. Treat docs and parity trackers as claims that must be validated.
5. Flag legacy shims, compatibility aliases, `allow(dead_code)`, feature-gated branches, duplicate implementations, and generated artifacts in source trees.
6. Prefer concrete evidence over guesses.

## Deliverable Format

### 1. Scope Summary
- What this lane owns
- Main feature families
- Main end-to-end flows

### 2. Findings
For each finding include:
- Severity: `critical`, `high`, `medium`, or `low`
- Type: `broken-code`, `broken-feature`, `stale-code`, `feature-gap`, `style`, `structure`, `quality`, `wiring`, or `flow`
- File/path references
- Why it matters
- Repro or verification notes
- Suggested fix direction

### 3. Stale / Unused / Drift Signals
- Dead code candidates
- Legacy alias/shim paths
- Duplicate logic
- Docs or parity drift
- Generated artifacts or misplaced files

### 4. End-to-End Flow Check
For each core flow in your lane, mark:
- `complete`
- `partial`
- `broken`
- `unclear`

Use this format:
- Flow: `<name>`
- Entry points: `<files/endpoints/commands>`
- Runtime path: `<key files>`
- Exit/output path: `<responses/persistence/UI>`
- Status: `<complete|partial|broken|unclear>`
- Notes: `<brief evidence>`

### 5. Wiring Checklist
Confirm whether each feature is wired through:
- source registration
- config/settings
- CLI or API exposure
- runtime invocation
- persistence or side effects
- tests
- docs

### 6. Tests / Verification Run
- Commands run
- What passed
- What failed
- What was not tested

### 7. Recommended Follow-Ups
- Immediate fixes
- Cleanup/refactor items
- Missing tests
- Docs updates
```

## Suggested Master Prompt

Use this when handing work to each agent:

```md
Audit the ThinClaw repo lane described below and return only findings plus a short scope summary.

Lane: <lane name>
Primary paths: <paths>
Canonical docs: <docs>

Focus on:
- broken code
- stale or unused code
- broken features
- feature gaps or incomplete features
- code style
- file structure
- code quality
- wire completion
- flow completion end to end

Requirements:
- Build a feature map first.
- Trace each major flow from entry point to runtime path to output/persistence.
- Call out duplicate logic, compatibility shims, feature-gated branches, `allow(dead_code)`, placeholder implementations, and generated artifacts in source trees.
- Treat docs as claims to verify.
- Prioritize concrete findings with file references.
- End with a short list of recommended fixes and missing tests.
```
