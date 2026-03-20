# 📚 All Documentation — Index

> Single source of truth for all `.md` files in the codebase.
> Excludes `node_modules/`, `binaries/`, and `ironclaw/target/`.
> Last updated: 2026-03-13 (content-verified against codebase)

> [!NOTE]
> **Status key:** ✅ Content-verified = deep-checked against live codebase · **Current** = reflects live codebase · ✅ Completed = finished plan/roadmap (kept as reference) · 📦 Archived = pre-IronClaw historical · ⚠️ Historical = superseded
>
> **Path note:** `src-tauri/` was renamed to `backend/` on 2026-02-22. Archived/completed docs may still reference the old path.

---

## 📁 Root Level

- [CONTRIBUTING.md](../CONTRIBUTING.md) — ✅ Content-verified (backend/ path correct, standard contrib guide)
- [License.md](../License.md)
- [README.md](../README.md) — ✅ Content-verified (rig_lib, sidecar arch, setup flow all accurate)
- [REMOTE_DEPLOYMENT.md](REMOTE_DEPLOYMENT.md) — ✅ Content-verified (Docker Compose approach, connection flow accurate)
- [setup.md](setup.md) — ✅ **Updated 2026-03-13** — Node.js sidecar removed (dead code cleanup); feature flags, binary paths, npm scripts verified
- [tauri_dropin_spec.md](tauri_dropin_spec.md) — ✅ Completed integration spec (historical reference, COMPLETED banner present)
- [tools.md](tools.md) — ✅ Content-verified (5 Rig tools, deep search pipeline, orchestrator flow accurate)

---

## 📁 documentation/

### Core Reference

- [documentation/scrappy_documentation_canonical.md](documentation/scrappy_documentation_canonical.md) — ✅ Content-verified, **single source of truth** (canonical architecture doc, all 20 sections verified)

### Topic Docs

- [documentation/agent_spec.md](documentation/agent_spec.md) — ✅ Completed (ToolPlan/Router implemented; v1 scope exceeded by cloud + IronClaw)
- [documentation/archive/agent_technical_review.md](documentation/archive/agent_technical_review.md) — ⚠️ Historical (early ReAct loop review)
- [documentation/ai_suite_phase.md](documentation/ai_suite_phase.md) — ✅ Content-verified (Whisper/TTS/Diffusion model recs, Imagine Studio section accurate)
- [documentation/chromium_oxide.md](chromium_oxide.md) — ✅ Completed (chromiumoxide integration implemented) ⚠️ Contains `src-tauri/` paths
- [documentation/archive/deep_research.md](documentation/archive/deep_research.md) — ⚠️ Historical (deep research planning)
- [documentation/hardware_and_library_plan.md](documentation/hardware_and_library_plan.md) — ✅ Completed ⚠️ Contains `src-tauri/` paths
- [documentation/archive/llm-chain-llama.md](documentation/archive/llm-chain-llama.md) — 📦 Archived (abandoned in favor of Rig)
- [documentation/tauri-fs.md](documentation/tauri-fs.md) — ✅ Content-verified, paths updated to `backend/` (FS plugin reference)
- [documentation/archive/tools.md](documentation/archive/tools.md) — ⚠️ Historical (early tool planning)
- [documentation/two_server_sane_implementation.md](documentation/two_server_sane_implementation.md) — ✅ Completed (implemented architecture)
- [documentation/web_search_rig.md](documentation/web_search_rig.md) — ✅ Completed (Phase 1 implemented)
- [documentation/search/brave_search_spec.md](documentation/search/brave_search_spec.md) — ✅ Content-verified, app name updated to Scrappy (API reference spec)
- [documentation/ui-system/ui.md](documentation/ui-system/ui.md) — ✅ Content-verified (glassmorphism design system, cn() utility, Tailwind patterns confirmed)
- [documentation/FastAPI/MCP-overhaul.md](documentation/FastAPI/MCP-overhaul.md) — ✅ **Updated 2026-03-15** — all `src-tauri/` paths fixed to `backend/`, §2/§7/§11 updated for IronClaw (Node.js sidecar → in-process Rust), Appendix B (IPC bridge) marked removed

### Archived / Historical → `documentation/archive/`

- [documentation/archive/feature_set_analysis.md](documentation/archive/feature_set_analysis.md) — 📦 Archived (pre-IronClaw snapshot)
- [documentation/archive/openclaw.md](documentation/archive/openclaw.md) — 📦 Archived (pre-IronClaw spec)
- [documentation/archive/openclaw_data_flows.md](documentation/archive/openclaw_data_flows.md) — §1-3 current, §4 historical

---

### documentation/latest/ — Current Architecture Docs

#### architecture/
- [latest/architecture/TECHNICAL_ARCHITECTURE.md](latest/architecture/TECHNICAL_ARCHITECTURE.md) — ✅ Content-verified, `src-tauri/` → `backend/` paths fixed (comprehensive 62KB reference)
- [latest/architecture/MICROSERVICES_AND_SIDECARS.md](latest/architecture/MICROSERVICES_AND_SIDECARS.md) — ✅ Content-verified, stale openclaw-engine port entry removed (engine list, SidecarManager confirmed)
- [latest/architecture/FRONTEND_ARCHITECTURE.md](latest/architecture/FRONTEND_ARCHITECTURE.md) — ✅ **Updated 2026-03-13** — added 5 missing hooks (`use-cloud-models`, `use-cloud-status`, `use-inference-backends`, `use-voice-wake`, `useEngineSetup`) + `voice/` component dir
- [latest/architecture/SUBAGENT_SYSTEMS.md](latest/architecture/SUBAGENT_SYSTEMS.md) — ✅ Content-verified (architecture, tool API, nesting config all accurate)
- [latest/architecture/Scrappy Overview.md](<latest/architecture/Scrappy Overview.md>) — ✅ Content-verified, cloud provider list corrected (comprehensive 31KB reference)

#### implementation/
- [latest/implementation/RIG_IMPLEMENTATION.md](latest/implementation/RIG_IMPLEMENTATION.md) — ✅ Content-verified, file sizes updated (orchestrator 935→1181 LOC, calculator 801→1219 LOC)
- [latest/implementation/STORAGE_AND_DATABASE.md](latest/implementation/STORAGE_AND_DATABASE.md) — ✅ Content-verified (no stale refs, schema docs match)
- [latest/implementation/AGENT_PROMPTS_AND_STORAGE.md](latest/implementation/AGENT_PROMPTS_AND_STORAGE.md) — ✅ Content-verified, line numbers updated to match current `reasoning.rs` (7 refs drifted ~30 lines)
- [latest/implementation/HF_HUB_DISCOVERY.md](latest/implementation/HF_HUB_DISCOVERY.md) — ✅ Content-verified (engine-aware design, HFDiscovery.tsx confirmed)
- [latest/implementation/cloud_storage_implementation.md](latest/implementation/cloud_storage_implementation.md) — ✅ Content-verified, status fixed from "Not Yet Implemented" → "✅ Implemented" (573-LOC CloudManager, 7 providers: S3-compat/GDrive/Dropbox/OneDrive/iCloud/SFTP/WebDAV)
- [latest/implementation/OPENCLAW_IMPLEMENTATION.md](latest/implementation/OPENCLAW_IMPLEMENTATION.md) — ⚠️ Historical (superseded by IronClaw, deprecation banner present)
- [latest/implementation/legacy_tool_loop.md](latest/implementation/legacy_tool_loop.md) — ⚠️ Historical (contains `src-tauri/` refs — intentional, describes old tool loop)
- [latest/implementation/TIMESTAMP_MIGRATION_PLAN.md](latest/implementation/TIMESTAMP_MIGRATION_PLAN.md) — ✅ Completed

#### ironclaw/
- [latest/ironclaw/Ironclaw Scrappy Data Contract.md](<latest/ironclaw/Ironclaw Scrappy Data Contract.md>) — ✅ Content-verified, command count updated (20→110), stale "not wired" items fixed
- [latest/ironclaw/ironclaw_pipelines.md](latest/ironclaw/ironclaw_pipelines.md) — ✅ Content-verified 2026-03-14 (22-pipeline file-level map of all IronClaw modules + frontend components)
- [latest/ironclaw/ironclaw_integration_roadmap.md](latest/ironclaw/ironclaw_integration_roadmap.md) — ✅ Completed roadmap (refs deleted files like `frames.rs`, `normalizer.rs` — intentional history)
- [latest/ironclaw/ironclaw_library_roadmap.md](latest/ironclaw/ironclaw_library_roadmap.md) — ✅ Completed roadmap
- [latest/ironclaw/ironclaw_phase1_spec.md](latest/ironclaw/ironclaw_phase1_spec.md) — ✅ Completed
- [latest/ironclaw/ironclaw_phase2_spec.md](latest/ironclaw/ironclaw_phase2_spec.md) — ✅ Completed
- [latest/ironclaw/ironclaw_feature_parity.md](latest/ironclaw/ironclaw_feature_parity.md) — ✅ Content-verified, cloud provider list corrected (1156-line parity matrix)

#### operations/
- [latest/operations/TODO.md](latest/operations/TODO.md) — ✅ **Updated 2026-03-15** — all `src-tauri/` paths corrected, header date updated, deleted-file refs annotated (3 open items: whisper auth ⛔, LIKE search accepted, skill deps stub)
- [latest/operations/todo_next.md](latest/operations/todo_next.md) — ✅ Completed (all work streams A/C/D/P done, all 11 open decisions resolved)
- [latest/operations/open_issues.md](latest/operations/open_issues.md) — ✅ Completed (31/31 issues resolved, 3 retracted)
- [latest/operations/upgrade.md](latest/operations/upgrade.md) — ✅ Completed, FULLY IMPLEMENTED banner present (architecture reference)

#### Top-level
- [latest/remote_deploy/IRONCLAW_DEPLOYMENT_PATHS.md](latest/remote_deploy/IRONCLAW_DEPLOYMENT_PATHS.md) — ✅ Content-verified (dual-mode embedded/standalone architecture accurate)
- [latest/remote_deploy/REMOTE_DEPLOY_IMPLEMENTATION_PLAN.md](latest/remote_deploy/REMOTE_DEPLOY_IMPLEMENTATION_PLAN.md) — ✅ Content-verified (no stale refs, Docker Compose setup accurate)
- [latest/scrappy_pipelines.md](latest/scrappy_pipelines.md) — ✅ Content-verified 2026-03-14 (10-pipeline file-level map of all Scrappy backend + frontend components)

> Items above are now organized by subdirectory under `latest/`.

---

### documentation/archive/openclaw/ — 📦 Archived (pre-IronClaw agent workspace)

- [documentation/archive/openclaw/AGENTS Kopie.md](<documentation/archive/openclaw/AGENTS Kopie.md>)
- [documentation/archive/openclaw/BOOTSTRAP.md](documentation/archive/openclaw/BOOTSTRAP.md)
- [documentation/archive/openclaw/MEMORY.md](documentation/archive/openclaw/MEMORY.md)
- [documentation/archive/openclaw/SOUL.md](documentation/archive/openclaw/SOUL.md)
- [documentation/archive/openclaw/TOOLS.md](documentation/archive/openclaw/TOOLS.md)
- [documentation/archive/openclaw/openclaw_archietecture.md](documentation/archive/openclaw/openclaw_archietecture.md)
- [documentation/archive/openclaw/openclaw_companioin_safety_review.md](documentation/archive/openclaw/openclaw_companioin_safety_review.md)
- [documentation/archive/openclaw/openclaw_companion_spec.md](documentation/archive/openclaw/openclaw_companion_spec.md)
- [documentation/archive/openclaw/openclaw_doc.md](documentation/archive/openclaw/openclaw_doc.md)
- [documentation/archive/openclaw/openclaw_flow.md](documentation/archive/openclaw/openclaw_flow.md)
- [documentation/archive/openclaw/openclaw_general_install.md](documentation/archive/openclaw/openclaw_general_install.md)
- [documentation/archive/openclaw/openclaw_webUI.md](documentation/archive/openclaw/openclaw_webUI.md)
- [documentation/archive/openclaw/report.md](documentation/archive/openclaw/report.md)

---

## 📁 ironclaw/ — IronClaw internal docs (managed separately)

> IronClaw is an upstream dependency (v0.12.0, 375 Rust source files). These docs are maintained by the IronClaw project. Scrappy-specific integration is documented in `documentation/latest/`.

- [ironclaw/README.md](ironclaw/README.md) — ✅ Content-verified (philosophy, 5 LLM backends, feature flags, architecture diagram all accurate)
- [ironclaw/AGENTS.md](ironclaw/AGENTS.md) — ✅ Content-verified (simple policy doc, accurate)
- [ironclaw/AGENT_FLOW.md](ironclaw/AGENT_FLOW.md) — ✅ **Updated 2026-03-13** — Line references fixed (were 47–267 lines off); logical content accurate: 9-step wizard, 5-phase AppBuilder, 7 identity files, message pipeline
- [ironclaw/CHANGELOG.md](ironclaw/CHANGELOG.md) — ✅ Content-verified (auto-generated by release-plz, v0.12.0 latest)
- [ironclaw/CLAUDE.md](ironclaw/CLAUDE.md) — ✅ **Updated 2026-03-13** — Project structure expanded to 375 files, NEAR AI removed, wizard→9-step, deleted files removed, new providers/channels/tools documented
- [ironclaw/CONTRIBUTING.md](ironclaw/CONTRIBUTING.md) — ✅ Content-verified (simple policy, accurate)
- [ironclaw/FEATURE_PARITY.md](ironclaw/FEATURE_PARITY.md) — ✅ Content-verified (1216-line parity matrix, last reconciled 2026-03-07, actively maintained)
- [ironclaw/ironclaw_agent_answers.md](ironclaw/ironclaw_agent_answers.md) — ⚠️ Historical (Sprint 13 planning snapshot from 2026-03-04; file links have drifted; says "50+ commands" but actual count is 110+)

### ironclaw/.claude/commands/ — CI/dev automation templates

- [ironclaw/.claude/commands/add-sse-event.md](ironclaw/.claude/commands/add-sse-event.md)
- [ironclaw/.claude/commands/add-tool.md](ironclaw/.claude/commands/add-tool.md)
- [ironclaw/.claude/commands/fix-issue.md](ironclaw/.claude/commands/fix-issue.md)
- [ironclaw/.claude/commands/respond-pr.md](ironclaw/.claude/commands/respond-pr.md)
- [ironclaw/.claude/commands/review-crate.md](ironclaw/.claude/commands/review-crate.md)
- [ironclaw/.claude/commands/review-pr.md](ironclaw/.claude/commands/review-pr.md)
- [ironclaw/.claude/commands/ship.md](ironclaw/.claude/commands/ship.md)
- [ironclaw/.claude/commands/trace.md](ironclaw/.claude/commands/trace.md)
- [ironclaw/.claude/commands/triage-issues.md](ironclaw/.claude/commands/triage-issues.md)
- [ironclaw/.claude/commands/triage-prs.md](ironclaw/.claude/commands/triage-prs.md)

### ironclaw/channels-src/ — Channel source code docs

- [ironclaw/channels-src/discord/README.md](ironclaw/channels-src/discord/README.md)

### ironclaw/docs/ — User-facing guides

- [ironclaw/docs/BUILDING_CHANNELS.md](ironclaw/docs/BUILDING_CHANNELS.md)
- [ironclaw/docs/LLM_PROVIDERS.md](ironclaw/docs/LLM_PROVIDERS.md) — ✅ **Updated 2026-03-13** — NEAR AI removed as default, openai_compatible set as default, Tinfoil provider added
- [ironclaw/docs/TELEGRAM_SETUP.md](ironclaw/docs/TELEGRAM_SETUP.md)

### ironclaw/rewrite-docs/ — ⛔ ARCHIVED (Node.js→Rust migration guides, early 2026)

> **Do not reference as current documentation.** The rewrite is complete. All 34 files have deprecation banners prepended. A `README.md` in the directory explains what these are and where to find current docs. Old `src-tauri/` and Node.js references are intentional — they document what was ported.

- [ironclaw/rewrite-docs/AGENT_RS.md](ironclaw/rewrite-docs/AGENT_RS.md)
- [ironclaw/rewrite-docs/ARCHITECTURE.md](ironclaw/rewrite-docs/ARCHITECTURE.md)
- [ironclaw/rewrite-docs/AUTONOMY_RS.md](ironclaw/rewrite-docs/AUTONOMY_RS.md)
- [ironclaw/rewrite-docs/BROWSER_TOOL_RS.md](ironclaw/rewrite-docs/BROWSER_TOOL_RS.md)
- [ironclaw/rewrite-docs/CANVAS_RS.md](ironclaw/rewrite-docs/CANVAS_RS.md)
- [ironclaw/rewrite-docs/CHAT_COMMANDS_RS.md](ironclaw/rewrite-docs/CHAT_COMMANDS_RS.md)
- [ironclaw/rewrite-docs/CLIENT_SERVER_MODE_RS.md](ironclaw/rewrite-docs/CLIENT_SERVER_MODE_RS.md)
- [ironclaw/rewrite-docs/CLI_RS.md](ironclaw/rewrite-docs/CLI_RS.md)
- [ironclaw/rewrite-docs/CONFIG_RS.md](ironclaw/rewrite-docs/CONFIG_RS.md)
- [ironclaw/rewrite-docs/CRON_RS.md](ironclaw/rewrite-docs/CRON_RS.md)
- [ironclaw/rewrite-docs/HARDWARE_BRIDGE_RS.md](ironclaw/rewrite-docs/HARDWARE_BRIDGE_RS.md)
- [ironclaw/rewrite-docs/HOOKS_RS.md](ironclaw/rewrite-docs/HOOKS_RS.md)
- [ironclaw/rewrite-docs/INDEX_RS.md](ironclaw/rewrite-docs/INDEX_RS.md)
- [ironclaw/rewrite-docs/INFERENCE_PLACEMENT_RS.md](ironclaw/rewrite-docs/INFERENCE_PLACEMENT_RS.md)
- [ironclaw/rewrite-docs/INTERNAL_SYSTEMS_RS.md](ironclaw/rewrite-docs/INTERNAL_SYSTEMS_RS.md)
- [ironclaw/rewrite-docs/KNOWLEDGE_BASE_RS.md](ironclaw/rewrite-docs/KNOWLEDGE_BASE_RS.md)
- [ironclaw/rewrite-docs/LEAST_PRIVILEGE_RS.md](ironclaw/rewrite-docs/LEAST_PRIVILEGE_RS.md)
- [ironclaw/rewrite-docs/MODEL_DISCOVERY_RS.md](ironclaw/rewrite-docs/MODEL_DISCOVERY_RS.md)
- [ironclaw/rewrite-docs/MULTIMODAL_RS.md](ironclaw/rewrite-docs/MULTIMODAL_RS.md)
- [ironclaw/rewrite-docs/NETWORKING_RS.md](ironclaw/rewrite-docs/NETWORKING_RS.md)
- [ironclaw/rewrite-docs/PLUGINS_RS.md](ironclaw/rewrite-docs/PLUGINS_RS.md)
- [ironclaw/rewrite-docs/REMOTE_AND_PII_RS.md](ironclaw/rewrite-docs/REMOTE_AND_PII_RS.md)
- [ironclaw/rewrite-docs/REWRITE_TRACKER.md](ironclaw/rewrite-docs/REWRITE_TRACKER.md)
- [ironclaw/rewrite-docs/SANDBOX_RS.md](ironclaw/rewrite-docs/SANDBOX_RS.md)
- [ironclaw/rewrite-docs/SECRETS_RS.md](ironclaw/rewrite-docs/SECRETS_RS.md)
- [ironclaw/rewrite-docs/SETUP_WIZARD_RS.md](ironclaw/rewrite-docs/SETUP_WIZARD_RS.md)
- [ironclaw/rewrite-docs/SKILLS_RS.md](ironclaw/rewrite-docs/SKILLS_RS.md)
- [ironclaw/rewrite-docs/SUBAGENT_RS.md](ironclaw/rewrite-docs/SUBAGENT_RS.md)
- [ironclaw/rewrite-docs/TAURI_IMPLEMENTATION_ROADMAP.md](ironclaw/rewrite-docs/TAURI_IMPLEMENTATION_ROADMAP.md)
- [ironclaw/rewrite-docs/TAURI_INTEGRATION.md](ironclaw/rewrite-docs/TAURI_INTEGRATION.md)
- [ironclaw/rewrite-docs/TAURI_RELAY_RS.md](ironclaw/rewrite-docs/TAURI_RELAY_RS.md)
- [ironclaw/rewrite-docs/TRIGGER_MECHANICS_RS.md](ironclaw/rewrite-docs/TRIGGER_MECHANICS_RS.md)
- [ironclaw/rewrite-docs/TUI_RS.md](ironclaw/rewrite-docs/TUI_RS.md)
- [ironclaw/rewrite-docs/VECTOR_SEARCH_RS.md](ironclaw/rewrite-docs/VECTOR_SEARCH_RS.md)

### ironclaw/skills/

- [ironclaw/skills/web-ui-test/SKILL.md](ironclaw/skills/web-ui-test/SKILL.md)

### ironclaw/src/ — Module specs (code follows spec, spec is tiebreaker)

- [ironclaw/src/NETWORK_SECURITY.md](ironclaw/src/NETWORK_SECURITY.md) — ✅ Content-verified (no stale refs, security audit findings)
- [ironclaw/src/setup/README.md](ironclaw/src/setup/README.md) — ✅ **Updated 2026-03-13** — wizard 8→9 steps (Docker Sandbox added), NEAR AI provider/auth removed, Tinfoil added, remote auth section rewritten
- [ironclaw/src/tools/README.md](ironclaw/src/tools/README.md) — ✅ Content-verified (137 lines, WASM vs MCP decision guide, tool auth architecture)
- [ironclaw/src/workspace/README.md](ironclaw/src/workspace/README.md) — ✅ Content-verified (111-line workspace API spec, no stale refs)

### ironclaw/tests/

- [ironclaw/tests/test-pages/cnn/expected.md](ironclaw/tests/test-pages/cnn/expected.md)
- [ironclaw/tests/test-pages/medium/expected.md](ironclaw/tests/test-pages/medium/expected.md)
- [ironclaw/tests/test-pages/yahoo/expected.md](ironclaw/tests/test-pages/yahoo/expected.md)

### ironclaw/tools-src/

- [ironclaw/tools-src/TOOLS.md](ironclaw/tools-src/TOOLS.md)
- [ironclaw/tools-src/github/README.md](ironclaw/tools-src/github/README.md)
- [ironclaw/tools-src/slack/README.md](ironclaw/tools-src/slack/README.md)

---

## 📁 patches/ — Vendored dependency patches

- [patches/libsql-0.6.0/DEVELOPING.md](patches/libsql-0.6.0/DEVELOPING.md)
- [patches/libsql-0.6.0/README.md](patches/libsql-0.6.0/README.md)
