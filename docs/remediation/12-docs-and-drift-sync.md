# WS-12 — Docs & Drift Sync

> **Status:** Not started · **Priority:** P1 · **Risk:** low · **Effort:** M
> **Depends on:** none (the structural/inventory tasks) · the two coordination tasks trail WS-03 (Discord verification) and WS-01 (deny.toml header)
> **Blocks:** none (no WS is gated on docs; this WS instead *trails* every other WS to absorb their canonical-doc updates)
> **Owns (symbols/files):** `docs/CRATE_OWNERSHIP.md` (crate table + repo-shape prose), `docs/CLI_REFERENCE.md`, `FEATURE_PARITY.md` §20 + §12 area, `src/config/provider_catalog.rs` module-doc + `crates/thinclaw-config/src/provider_catalog.rs:4` doc-comment, `src/workspace/README.md` Heartbeat section, the ~37 stale "Scrappy" *doc-comment* references (NOT legacy/migration code), and the CLAUDE.md "Common Update Triggers" discipline note. **Does NOT own:** `deny.toml:3` (WS-01), `channels-src/discord/README.md` correctness fix (WS-03 lands the code, WS-12 only re-words the README in the same PR or immediately after).

## Vision & Goal
ThinClaw's CLAUDE.md explicitly makes accurate, ownership-scoped docs an architectural rule ("Avoid brittle counts, stale inventories, and 'default forever' claims"; "When behavior changes, update the relevant canonical docs in the same branch"). The audit found the inventory docs have quietly drifted from a genuinely mature codebase: a whole shipping subsystem (the repo-project supervisor) is undocumented, four real crates are missing from the ownership table, and several hardcoded counts / renamed-product references lie. This workstream re-grounds the maintainer-facing map in code truth and *institutionalizes the discipline* so the other 11 workstreams keep their canonical docs in sync as they land. It is cheap, low-risk, and the connective tissue that keeps the rest of the remediation honest.

## Scope
**In scope:**
- Add the repo-project supervisor to all inventory docs (CRATE_OWNERSHIP.md, CLAUDE.md repo-shape, CLI_REFERENCE.md, FEATURE_PARITY.md).
- Add the 4 missing crates to the CRATE_OWNERSHIP.md crate table.
- Remove the dated hardcoded tool count from FEATURE_PARITY.md §20.
- Fix the "20+ providers" claim in the provider catalog doc-comment.
- Purge stale "Scrappy" *doc-comment* references (the drifted upstream-fork product-name refs only).
- Fix the stale `spawn_heartbeat` example in `src/workspace/README.md`.
- After WS-03 lands Discord Ed25519 verification: correct `channels-src/discord/README.md`.
- Document the "canonical doc updated in the same PR" discipline as an explicit, enforceable rule.

**Out of scope (and which WS owns it):**
- `deny.toml:3` header fix → **WS-01** (P0 security/CI). WS-12 must not touch `deny.toml`.
- The actual Discord Ed25519 host verification code → **WS-03** (WASM channels). WS-12 only rewords the README.
- The actual repo-project supervisor completion (NeedsPlanning planner, concurrency limits, merge-retry bound) → **WS-06**. WS-12 documents *what exists today*, not the future fixes.
- Any source-code behavior change beyond doc-comments and module-docs. WS-12 changes comments and `.md` files; it does not refactor.

## Current State (verified)

**Repo-project supervisor — wired, shipping (default off), undocumented in inventory.**
- Crate `crates/thinclaw-repo-projects/` exists (926 LoC `src/lib.rs`) with a full state machine: `RepoProjectState`, `RepoProjectTaskState`, `CodingBackend`, `MergeMethod`, `GitHubAuthMode`, `ProjectPolicy`, `RepoProject`, `RepoProjectTask`, `RepoProjectRun`, `RepoWorkerRun`, `MergeGateDecision` (`crates/thinclaw-repo-projects/src/lib.rs:48-418`).
- CLI command surface is real and wired: `RepoProjectCommand` enum with List/Show/Status/Setup/SetCredential/Create/Enroll/Repos/Connect/Start/Pause/Resume/Cancel/Events (`src/cli/repo_projects.rs:18-105`), dispatched by `run_repo_projects_command` (`src/cli/repo_projects.rs:107`), registered in the top-level CLI as `RepoProjects(RepoProjectCommand)` with help text "Manage the GitHub repository project supervisor" (`src/cli/mod.rs:216-218`).
- **Drift:** `grep -ni "repo.project\|supervisor" CLAUDE.md` → 0 hits. `grep -ni "repo.project\|supervisor" FEATURE_PARITY.md` → 0 hits. `grep -n "repo_projects\|RepoProject" docs/CLI_REFERENCE.md` → 0 hits. The subsystem is invisible to the maintainer-facing map.

**CRATE_OWNERSHIP.md lists 22 of 26 crates.**
- `crates/` contains 26 crates (verified by `ls crates`). The "Current Runtime Crates" table (`docs/CRATE_OWNERSHIP.md:30-53`) has exactly 22 rows (`thinclaw-types` … `thinclaw-app`).
- **Missing rows (all exist on disk):** `thinclaw-identity` (367 LoC; conversation-scope/identity resolution — `ConversationScope`, `ResolvedIdentity`, `ActorEndpointRef`, `EndpointApprovalStatus` at `crates/thinclaw-identity/src/lib.rs:13-118`), `thinclaw-soul` (413 LoC; soul rendering — `CanonicalSoul`, `compose_seeded_soul`, `parse_canonical_soul`, `render_canonical_soul`, `pack_asset_markdown` at `crates/thinclaw-soul/src/soul.rs:22-168`), `thinclaw-repo-projects` (the supervisor DTO/state-machine crate above), `thinclaw-runtime-contracts` (34-line `lib.rs`; "implementation-free DTOs only" shared with Desktop host — modules `asset`/`direct`/`model`/`provider`/`runtime`/`secret`; `provider_catalog.rs` re-exports `ApiStyle`, `ProviderEndpoint` from it).

**FEATURE_PARITY.md §20 hardcoded dated count (violates CLAUDE.md anti-stale-count rule).**
- Heading: `## 20. Shipped Built-in Tools (80 max; some conditional or feature-gated)` (`FEATURE_PARITY.md:601`) plus `> **Updated:** 2026-05-14` (`FEATURE_PARITY.md:603`). The "80 max" and the date are exactly the kind of brittle dated count CLAUDE.md's Documentation Rules forbid.

**`provider_catalog.rs:4` "20+ providers" vs 16 in registry.**
- `src/config/provider_catalog.rs` is a 4-line façade (`pub use thinclaw_config::provider_catalog::*;`) — the real doc lives in `crates/thinclaw-config/src/provider_catalog.rs:4`: "This catalog enables ThinClaw to work with 20+ providers".
- `registry/providers.json` is the source of truth (loaded disk-then-embedded per the module doc) and contains **16** entries: openrouter, anthropic, openai, gemini, groq, mistral, xai, together, venice, moonshot, minimax, nvidia, deepseek, cerebras, cohere, tinfoil. The catalog is not the only path to a provider (env-configured OpenAI-compatible backends like ollama still work), so "16 catalog providers, plus additional env-configured OpenAI-compatible backends" is the accurate phrasing.

**~37 stale "Scrappy" doc-comment references (drifted upstream-fork product name).**
- `FEATURE_PARITY.md:461` declares the rename complete: "Historical Scrappy/OpenClaw component inventories were removed from this parity ledger…". Yet doc-comments still treat "Scrappy" as a live host/UI surface, e.g. `crates/thinclaw-tools/src/builtin/agent_control.rs:101` ("the user's channel (Scrappy, Telegram, CLI…)"), `crates/thinclaw-settings/src/providers.rs:19`, `crates/thinclaw-config/src/llm.rs:325`, `crates/thinclaw-config/src/provider_catalog.rs:17`, `crates/thinclaw-llm-core/src/routing_policy.rs:469`, `crates/thinclaw-channels/src/canvas_gateway.rs:10`, `crates/thinclaw-channels/src/status_view.rs:190`, `src/talk_mode.rs:10/15/94/95/112/606`, `src/tauri_commands.rs:3/7/162/746`, `src/voice_wake.rs:16`, `src/hardware_bridge.rs:3/8/16/17/152/155/226`, `src/app.rs:96/121/162/176/186/332/379`, `src/config/mod.rs:85`, `src/platform/linux_readiness.rs:516`, `src/cli/session_export.rs:188`, `src/cli/oauth_defaults.rs:119`, `src/api/config.rs:105`. Count verified ≈37 drifted comment refs (the audit's "42" includes a handful since renamed).
- **MUST NOT touch (intentional legacy/migration code, NOT drift):** env-var fallbacks `SCRAPPY_MCP_URL`/`SCRAPPY_MCP_TOKEN`/`SCRAPPY_PROMPT` (`apps/desktop/backend/src/config.rs:267/274`, `image_gen.rs:140/152/227`), keychain service `com.schack.scrappy` / `com.scrappy.*` (`apps/desktop/backend/src/thinclaw/config/keychain.rs:86`, `cloud/oauth.rs:371`, `cloud/encryption.rs:255`), `scrappy.db` migration (`apps/desktop/backend/src/lib.rs:460-464`), legacy cloud roots `LEGACY_OBJECT_ROOT="scrappy/"` / `"Scrappy"` folder fallbacks (`cloud/provider.rs:138`, `cloud/providers/*.rs`), persona alias `"scrappy" → "thinclaw"` (`personas.rs:30`, `config.rs:280/538`), and the MLX patch-detection string `"PATCH (scrappy)"` (`apps/desktop/backend/src/engine/engine_mlx.rs:398/457`) which is load-bearing back-compat detection. Removing any of these breaks migration from old Scrappy installs.

**`src/workspace/README.md:99` stale `spawn_heartbeat` example.**
- README shows a 4-arg call `spawn_heartbeat(config, workspace, llm, response_tx)` and a builder `HeartbeatConfig::default().with_interval(...).with_notify(...)` (`src/workspace/README.md:98-106`).
- Reality: `spawn_heartbeat` takes **7** args — `config, hygiene_config, workspace, llm, safety, response_tx, cost_tracker` (`src/agent/heartbeat.rs:154-162`). `HeartbeatConfig` is now a settings-resolved config struct (`crates/thinclaw-config/src/heartbeat.rs:41` `default()`, resolved via `HeartbeatConfig::resolve(settings)` at `src/config/mod.rs:286`) with no `with_interval`/`with_notify` builder methods. Heartbeat behavior is driven through the routine engine now (`crates/thinclaw-agent/src/routine_engine.rs`). Both the arg list and the builder are wrong.

**Discord WASM README false verification claim (WS-03 owns the code).**
- `channels-src/discord/README.md:105-108` claims `discord_public_key` signature validation "happens on the host before reaching the WASM." Verified false today: the Discord WASM declares `require_signature_verification` as `#[allow(dead_code)]` (`channels-src/discord/src/lib.rs:148-149`) and nowhere in `src/` or `crates/` is there an Ed25519/`discord_public_key` host check. The only host webhook validation is HMAC-SHA256 (`crates/thinclaw-channels/src/wasm/channel_watcher.rs:408-459`, `schema.rs:523`), which is not Discord's Ed25519 scheme. WS-03 implements the host verification; WS-12 corrects the README in the same PR.

**`deny.toml:3` header (WS-01 owns it).**
- `deny.toml:3` points CI at `.github/workflows/code_style.yml`. The audit lists this under WS-01's P0 set; WS-12 does not touch it.

## Decision Points

- **D1 — FEATURE_PARITY.md §20 tool list: regenerate vs. drop the count vs. delete the section.**
  - Options: (a) Keep the full per-category tool tables but **remove only the "(80 max)" count from the heading and the "Updated: 2026-05-14" line**, replacing with a pointer to the registry as source of truth. (b) Auto-generate the whole section from the tool registry. (c) Delete §20 entirely.
  - Trade-offs: (a) is minimal-risk, keeps the genuinely useful per-tool reference, and removes exactly the brittle artifacts CLAUDE.md forbids. (b) needs a generator + a CI check to stay honest — real value but scope creep for a P1-low docs WS, and there is no existing generator to copy. (c) throws away a useful reference.
  - **Recommendation: (a).** Drop the dated count and the date; keep the tables; add a one-line note that the count is intentionally omitted and the registry is authoritative. Flag (b) as a future enhancement, not part of this WS.

- **D2 — Stale "Scrappy" comments: rename to "ThinClaw Desktop" vs. delete the parenthetical.**
  - Options: (a) Replace "Scrappy" with "ThinClaw Desktop" (the current product name for the Tauri host). (b) Delete the product-name parenthetical entirely where it adds nothing (e.g. "the user's channel (Scrappy, Telegram, CLI…)" → "the user's channel (Telegram, CLI…)").
  - Trade-offs: most refs describe the desktop host's role, so (a) preserves intent. A few are pure noise where (b) reads cleaner.
  - **Recommendation: (a) as default — rename "Scrappy" → "ThinClaw Desktop"** for host/UI references; use (b) only where the name is a bare list item with no informational value. Per-ref judgment; never touch the legacy/migration identifiers listed above.

- **D3 — Discord README fix timing.** Edit the README in WS-03's PR (preferred, satisfies "same PR" rule) or as a fast WS-12 follow-up. **Recommendation: fold the README edit into WS-03's PR**; if WS-03 ships without it, WS-12 picks it up immediately. Either way the README must not claim verification before the code exists.

## Tasks

- [ ] **T1: Add the 4 missing crates to `docs/CRATE_OWNERSHIP.md` crate table.**
  - **Files:** `docs/CRATE_OWNERSHIP.md` (table at lines 30–53).
  - **Change:** Add four rows, placed near their domain neighbors, matching the existing one-line "Owns" prose style:
    - `thinclaw-identity` — "conversation-scope and identity resolution DTOs: conversation kind/scope, resolved identity, actor endpoint references, and endpoint approval status."
    - `thinclaw-soul` — "canonical/local soul parsing and rendering, seeded-soul composition, pack name canonicalization, and pack asset markdown."
    - `thinclaw-repo-projects` — "repo-project supervisor domain types and state machines: project/task/run states and transitions, coding backend, merge method, GitHub auth mode, project policy, and merge-gate decision DTOs."
    - `thinclaw-runtime-contracts` — "implementation-free shared runtime DTOs for ThinClaw clients and the Desktop host: asset, direct-runtime, model, provider (incl. `ApiStyle`/`ProviderEndpoint`), runtime, and secret contracts."
  - **Acceptance:** Table has 26 rows; `rg "thinclaw-identity|thinclaw-soul|thinclaw-repo-projects|thinclaw-runtime-contracts" docs/CRATE_OWNERSHIP.md` returns all four; row count matches `ls crates | wc -l`.
  - **Effort:** S
  - **Verification:** `ls /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop/crates | wc -l` (=26) and `grep -c '^| `thinclaw' docs/CRATE_OWNERSHIP.md` (=26).

- [ ] **T2: Document the repo-project supervisor in CLAUDE.md repo-shape + a new CRATE_OWNERSHIP architecture note.**
  - **Files:** `CLAUDE.md` ("Current Architecture Notes" bullets near lines 107–110; "Repo Shape" list near `src/cli/`), `docs/CRATE_OWNERSHIP.md` (the T1 row covers the crate; add a one-line mention in the root-owned-runtime prose if a concrete supervisor/pipeline lives in root).
  - **Change:** Add a CLAUDE.md architecture bullet describing the supervisor as a wired-but-default-off subsystem (CLI `thinclaw repo-projects`, GitHub App backed, `thinclaw-repo-projects` owns the domain types). Mirror the style of the existing `- thinclaw-agent owns …` bullet. Do not restate WS-06's open items.
  - **Acceptance:** `grep -ni "repo.project\|supervisor" CLAUDE.md` returns the new bullet(s); wording matches code (default-off, CLI-driven, GitHub App).
  - **Effort:** S
  - **Verification:** `grep -ni "repo-project\|supervisor" /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop/CLAUDE.md`.

- [ ] **T3: Add `thinclaw repo-projects` to `docs/CLI_REFERENCE.md`.**
  - **Files:** `docs/CLI_REFERENCE.md` (add under "Background Work & Operations", after the `experiments` entry near line 99, or a dedicated subsection).
  - **Change:** Document the subcommands from `src/cli/repo_projects.rs:18-105`: `list`, `show <project_id>`, `status`, `setup [--enable|--disable --app-id --installation-id --private-key-secret --webhook-secret-secret --app-slug --default-coding-backend --auto-merge --watchdog-interval-secs]`, `set-credential <name> [--value]`, `create --name --repo-url [--default-branch --description]`, `enroll <project_id> --repo-url [--default-branch]`, `repos`, `connect [repos…] [--all]`, `start/pause/resume/cancel <project_id>`, `events <project_id> [--limit]`. Pull one-line descriptions from the clap doc-comments verbatim. Note it is default-off (gated by the supervisor feature flag / settings).
  - **Acceptance:** Every `RepoProjectCommand` variant appears; flag names match the `#[arg(long)]` names exactly.
  - **Effort:** M
  - **Verification:** Cross-check each documented flag against `grep -n '#\[arg(long' /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop/src/cli/repo_projects.rs`.

- [ ] **T4: Add a repo-project supervisor entry to `FEATURE_PARITY.md`.**
  - **Files:** `FEATURE_PARITY.md` (Automation §14 or a new dedicated subsection; follow the existing `| Feature | OpenClaw | ThinClaw | Priority | Notes |` table shape used in §13/§14).
  - **Change:** Add a row describing the supervisor (GitHub App backed repo-project automation, CLI + gateway-wired, default off) with a link to `crates/thinclaw-repo-projects` and `src/cli/repo_projects.rs`. Keep notes factual to current state; defer completion gaps to WS-06.
  - **Acceptance:** `grep -ni "repo.project\|supervisor" FEATURE_PARITY.md` returns the new row.
  - **Effort:** S
  - **Verification:** `grep -ni "supervisor" /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop/FEATURE_PARITY.md`.

- [ ] **T5: Drop the dated hardcoded tool count in FEATURE_PARITY.md §20 (D1 → option a).**
  - **Files:** `FEATURE_PARITY.md:601,603`.
  - **Change:** Rewrite heading `## 20. Shipped Built-in Tools (80 max; some conditional or feature-gated)` → `## 20. Shipped Built-in Tools` and remove the `> **Updated:** 2026-05-14` line. Add a one-line note: "Counts are intentionally omitted; the live tool registry is authoritative — see `src/tools/README.md` and `crates/thinclaw-tools-core`." Keep the per-category tables.
  - **Acceptance:** No "80 max" and no hardcoded "Updated: <date>" in §20; tables intact.
  - **Effort:** S
  - **Verification:** `grep -n "80 max\|Updated:" /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop/FEATURE_PARITY.md` returns nothing in §20.

- [ ] **T6: Fix the "20+ providers" doc-comment (D2-adjacent — count only).**
  - **Files:** `crates/thinclaw-config/src/provider_catalog.rs:4`.
  - **Change:** Rewrite "This catalog enables ThinClaw to work with 20+ providers without requiring explicit base_url configuration." → "This catalog ships 16 built-in provider endpoints (see `registry/providers.json`); ThinClaw also works with additional env-configured OpenAI-compatible backends not in the catalog." Avoid re-introducing a brittle count if the registry can grow — prefer "the providers in `registry/providers.json`" if the exact number is likely to drift soon.
  - **Acceptance:** No "20+" in the file; the source-of-truth (`registry/providers.json`) is named.
  - **Effort:** S
  - **Verification:** `grep -n "20+" /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop/crates/thinclaw-config/src/provider_catalog.rs` returns nothing; `cargo doc -p thinclaw-config --no-deps` builds.

- [ ] **T7: Fix the stale `spawn_heartbeat` example in `src/workspace/README.md`.**
  - **Files:** `src/workspace/README.md:98-106`.
  - **Change:** Replace the wrong 4-arg call and the nonexistent `HeartbeatConfig::default().with_interval(...).with_notify(...)` builder with either (preferred) prose pointing at the routine-driven heartbeat (`crates/thinclaw-agent/src/routine_engine.rs`, settings-resolved `HeartbeatConfig` via `HeartbeatConfig::resolve(settings)` at `src/config/mod.rs:286`), or a corrected 7-arg signature matching `src/agent/heartbeat.rs:154-162` (`config, hygiene_config, workspace, llm, safety, response_tx, cost_tracker`). Do not invent APIs.
  - **Acceptance:** The code block compiles against the current signature or is replaced by accurate prose; no `with_interval`/`with_notify` calls remain.
  - **Effort:** S
  - **Verification:** `grep -n "with_interval\|with_notify\|spawn_heartbeat(config, workspace" /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop/src/workspace/README.md` returns nothing.

- [ ] **T8: Purge stale "Scrappy" doc-comment references (D2 → rename to "ThinClaw Desktop").**
  - **Files (drifted comments only):** `crates/thinclaw-tools/src/builtin/agent_control.rs:101`, `crates/thinclaw-settings/src/providers.rs:19`, `crates/thinclaw-config/src/llm.rs:325`, `crates/thinclaw-config/src/provider_catalog.rs:17`, `crates/thinclaw-llm-core/src/routing_policy.rs:469`, `crates/thinclaw-channels/src/canvas_gateway.rs:10`, `crates/thinclaw-channels/src/status_view.rs:190`, `src/talk_mode.rs:10,15,94,95,112,606`, `src/tauri_commands.rs:3,7,162,746`, `src/voice_wake.rs:16`, `src/hardware_bridge.rs:3,8,16,17,152,155,226`, `src/app.rs:96,121,162,176,186,332,379`, `src/config/mod.rs:85`, `src/platform/linux_readiness.rs:516`, `src/cli/session_export.rs:188`, `src/cli/oauth_defaults.rs:119`, `src/api/config.rs:105`.
  - **Change:** Rename "Scrappy" → "ThinClaw Desktop" in these doc-comments (or delete the bare parenthetical where it adds nothing). **Do NOT touch** any legacy/migration identifier (env vars `SCRAPPY_*`, keychain `com.schack.scrappy`/`com.scrappy.*`, `scrappy.db`, `LEGACY_OBJECT_ROOT`/`"Scrappy"` cloud roots, persona alias `"scrappy"`, MLX `"PATCH (scrappy)"` detection — all enumerated in Current State). Work crate-by-crate to keep each crate's edit reviewable in isolation.
  - **Acceptance:** The targeted comment lines no longer say "Scrappy"; the legacy/migration grep set is unchanged; nothing compiles differently (comments only).
  - **Effort:** M
  - **Verification:** Re-run the audit's drift grep and confirm only legacy/migration hits remain: `grep -rni "scrappy" --include="*.rs" /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop | grep -vi "legacy\|migrat\|fallback\|SCRAPPY_MCP\|SCRAPPY_PROMPT\|com.schack.scrappy\|com.scrappy\|scrappy.db\|LEGACY\|persona\|PATCH (scrappy)"` should drop to ~0.

- [ ] **T9: Correct the Discord WASM README verification claim (coordinate with WS-03).**
  - **Files:** `channels-src/discord/README.md:105-108`.
  - **Change:** Only after WS-03 lands host-side Ed25519 verification, update the "Invalid Signature" section to describe the real mechanism. If WS-03 has not landed when WS-12 reaches this task, instead reword the claim to reflect reality ("signature verification is configured via …") rather than asserting a guarantee that does not exist. Do not edit `channels-src/discord/src/lib.rs` — that is WS-03.
  - **Acceptance:** README matches whatever WS-03 actually implemented; no claim of host verification that the code does not provide.
  - **Effort:** S
  - **Verification:** Diff the README claim against WS-03's merged code in `crates/thinclaw-channels/` / `channels-src/discord/src/lib.rs`.

- [ ] **T10: Document the "canonical doc updated in the same PR" discipline (institutionalize the rule).**
  - **Files:** `CLAUDE.md` ("Documentation Rules" / "Common Update Triggers" sections), `docs/remediation/README.md` (if/when it exists — otherwise reference from this WS doc).
  - **Change:** Add an explicit, checkable rule: every WS PR that changes behavior in an area with a canonical doc (per the CLAUDE.md "Canonical Docs" table and "Common Update Triggers") MUST update that doc in the same PR, and MUST update `FEATURE_PARITY.md` if tracked behavior changed. Recommend wiring this into `/code-review` and `/ship` review checklists. Add a short "WS-12 trails each wave" note so reviewers know docs sync is expected per-wave, not deferred to the end.
  - **Acceptance:** The rule is written as an imperative checklist item, not a vague aspiration; it names the same-PR requirement and the FEATURE_PARITY trigger.
  - **Effort:** S
  - **Verification:** `grep -ni "same PR\|same branch\|canonical doc" /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop/CLAUDE.md` shows the strengthened rule.

## Best Practices (workstream-specific)
- **Match the existing doc voice.** Copy the one-line "Owns" prose style already in the CRATE_OWNERSHIP.md table (`docs/CRATE_OWNERSHIP.md:32-53`) and the architecture-note bullet style in CLAUDE.md (`CLAUDE.md:107-110`). Do not introduce a new format.
- **Cite source of truth, not snapshots.** Per CLAUDE.md Documentation Rules, point at the registry/code that owns the fact (e.g. "see `registry/providers.json`") rather than baking in a number that will drift. This is exactly why §20's count and `provider_catalog.rs`'s "20+" are being removed.
- **Pull CLI text verbatim from clap doc-comments.** The `///` lines in `src/cli/repo_projects.rs` are already good user-facing copy; reuse them rather than paraphrasing (reduces future drift between `--help` and the doc).
- **Respect ownership boundaries.** WS-12 edits `.md` files and Rust *doc-comments/module-docs only*. It never changes a function body. The Discord and deny.toml fixes are other WS's code; WS-12 only handles their prose.
- **Edit per-crate, not per-repo, for the Scrappy purge.** Keep each crate's comment rename in its own reviewable hunk; this mirrors the architecture-hygiene preference for narrow, owned changes.

## Common Pitfalls
- **Deleting legacy/migration "Scrappy" code as if it were drift.** The audit's "42 Scrappy refs" mixes drifted comments with *load-bearing back-compat*: env-var fallbacks, keychain service IDs, `scrappy.db` migration, cloud-root read fallbacks, persona alias, and the MLX `"PATCH (scrappy)"` detection string. Removing any of these silently breaks upgrades from old Scrappy installs. The Current State section enumerates the exact must-not-touch set — re-verify against it before every edit in T8.
- **The fix landing in only one of N copies.** This is the audit's recurring failure mode (e.g. the `split_message` fix that landed in one WASM crate of four). For docs the analog is: the supervisor must be added to *all four* inventory docs (CRATE_OWNERSHIP, CLAUDE, CLI_REFERENCE, FEATURE_PARITY) — partial coverage just relocates the drift. T1–T4 must all land.
- **Editing the façade, not the source.** `src/config/provider_catalog.rs` is a 4-line `pub use` façade; the "20+" string lives in `crates/thinclaw-config/src/provider_catalog.rs:4`. Don't waste an edit on the façade.
- **Re-introducing a brittle count.** When fixing "20+ providers" and the §20 tool count, do not replace one stale number with another that will rot — prefer naming the authoritative file (CLAUDE.md explicitly forbids "brittle counts, stale inventories").
- **Documenting WS-06/WS-03 future state as current.** WS-12 describes what is wired *today* (supervisor exists, default off; Discord verification absent). It must not document the planner/concurrency fixes (WS-06) or Ed25519 verification (WS-03) as if already shipped.
- **Asserting Discord verification before WS-03 lands (T9).** Sequencing matters: the README claim must never precede the code.

## Multi-Worker Execution Plan (ultracode)
- **Worker decomposition:**
  - **Wave A (parallel, no code dependencies, can start immediately):** T1 (crate table), T2 (CLAUDE.md supervisor), T3 (CLI_REFERENCE), T4 (FEATURE_PARITY supervisor), T5 (§20 count), T6 (provider count), T7 (heartbeat README), T10 (discipline rule). These touch distinct files/sections and can fan out across subagents.
  - **T8 (Scrappy purge)** is a single coherent worker (or one subagent per crate) — keep its hunks separate but it is independent of the others.
  - **Wave B (trailing):** T9 runs after WS-03 merges. This is the only cross-WS-sequenced task.
- **Isolation:** Wave A tasks edit mostly disjoint files; a single branch is fine. If fanning out to parallel subagents that touch the *same* file concurrently (e.g. T2 and T10 both touch CLAUDE.md; T4 and T5 both touch FEATURE_PARITY.md), serialize those pairs or use git worktree isolation to avoid merge conflicts. T8's per-crate edits are disjoint and parallel-safe. Recommended: one worktree for the doc/`.md` tasks, one for the T8 comment purge, since they share no files.
- **Workflow shape:** implement (fan-out Wave A + T8) → verify (grep assertions per task + `cargo fmt --all -- --check` + `cargo doc --no-deps` to catch broken doc-comments) → review (`/code-review` low effort — these are docs, scan for accidental code edits and stale-count regressions) → fix → (later) Wave B T9 after WS-03 → re-verify. No DB/Docker prerequisites — this WS is pure docs/comments.
- **Verification gate:**
  - `cargo fmt --all -- --check` (catches doc-comment reflow issues)
  - `cargo doc --workspace --no-deps` (ensures module-doc edits in `provider_catalog.rs` etc. still parse)
  - `cargo check --workspace` (comment-only edits must not break the build; cheap sanity)
  - Per-task grep assertions listed under each Verification bullet.
  - `/code-review` (low) on the diff; `/ship` is overkill for a docs-only WS but run `cargo fmt`/`cargo check` from it if convenient.
  - No Postgres/libSQL/Docker needed.

## Definition of Done
- [ ] `docs/CRATE_OWNERSHIP.md` crate table has 26 rows including `thinclaw-identity`, `thinclaw-soul`, `thinclaw-repo-projects`, `thinclaw-runtime-contracts` (T1).
- [ ] Repo-project supervisor is documented in CLAUDE.md repo-shape/architecture notes (T2), `docs/CLI_REFERENCE.md` with all `RepoProjectCommand` subcommands/flags (T3), and `FEATURE_PARITY.md` (T4).
- [ ] FEATURE_PARITY.md §20 no longer carries "80 max" or a hardcoded "Updated: <date>"; registry named as authoritative (T5).
- [ ] `crates/thinclaw-config/src/provider_catalog.rs:4` no longer says "20+ providers"; the 16-entry `registry/providers.json` (plus env backends) is named accurately (T6).
- [ ] `src/workspace/README.md` heartbeat example matches the current 7-arg signature or routine-driven prose; no `with_interval`/`with_notify` (T7).
- [ ] The drift grep (legacy/migration excluded) returns ~0 "Scrappy" hits; the must-not-touch legacy set is verified unchanged (T8).
- [ ] Discord README matches WS-03's merged verification code; no false guarantee (T9, after WS-03).
- [ ] CLAUDE.md states the same-PR canonical-doc-update discipline as an enforceable rule (T10).
- [ ] Gate green: `cargo fmt --all -- --check`, `cargo doc --workspace --no-deps`, `cargo check --workspace` all pass; `/code-review` shows no accidental code changes.
- [ ] D1 and D2 resolved as recommended (count dropped, not regenerated; Scrappy renamed to "ThinClaw Desktop").
