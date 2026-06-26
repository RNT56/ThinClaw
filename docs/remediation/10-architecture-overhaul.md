# WS-10 — Architecture Overhaul (god-files & crate migrations)

> **Status:** Not started · **Priority:** P2 · **Risk:** high · **Effort:** XL
> **Depends on:** WS-01, WS-02, WS-03, WS-04, WS-05, WS-06, WS-07, WS-08, WS-09 (land all behavior fixes first to avoid rebasing churn) · **Blocks:** none
> **Owns (symbols/files):** `src/history/store/**` (the root duplicate of the postgres store), `src/media/{audio,image,pdf,document,types}.rs` (the still-root-owned media extractors), `crates/thinclaw-channels/src/wasm/wrapper.rs`, `src/api/experiments.rs` decomposition (handler split only — coordinate the error-taxonomy edit to `src/api/error.rs` with WS-07), `src/agent/thread_ops.rs` decomposition, `src/llm/runtime_manager.rs` decomposition, `src/extensions/manager.rs` decomposition, `src/agent/routine_engine.rs` decomposition, `src/agent/agent_loop.rs` decomposition, `crates/thinclaw-workspace/src/workspace_core.rs` decomposition, `src/setup/wizard/{mod,llm}.rs` + `src/setup/channels.rs` decomposition, desktop `apps/desktop/backend/src/thinclaw/commands/rpc_dashboard.rs` + `apps/desktop/backend/src/thinclaw/remote_proxy.rs` + `apps/desktop/backend/src/sidecar.rs` decomposition, `JobToolHostPort` stub disposition (the half-stubbed structured tool-host port — wire-or-document, owned by no other WS).
>
> **Does NOT own:** `src/safety/*.rs` orphan deletion (WS-11). Behavior fixes inside any god-file (the owning behavior WS lands them first; WS-10 only moves the post-fix code). The `experiments.rs` controller/lease behavior fixes (WS-07).

## Vision & Goal

ThinClaw is a mature platform whose biggest remaining structural debt is concentrated, not pervasive: a handful of god-files and two half-finished crate migrations that force the same code to be maintained in two places (the `split_message` and `conversation_metadata_with_handoff` divergences already cost real bugs). This workstream realizes the project's own architecture-hygiene rules — façade `mod.rs`, narrow visibility, one domain per file — so future behavior work lands once, in one obvious place, with the public import paths (`thinclaw::history::Store`, `thinclaw::media::*`) preserved by re-export. The directive is to consolidate duplicates and decompose god-files **behavior-preserving**, one file per PR.

## Scope

**In scope:**
- **(1) HIGHEST LEVERAGE — `src/history/store/` → `thinclaw_db::postgres_store`** consolidation. Verified near-byte-for-byte duplication (see Current State). Make root `src/history` a thin façade over `thinclaw_db`, delete `src/history/store/`.
- **(2) Finish `src/media` extraction** from `crates/thinclaw-media` — move the 5 still-root-owned extractor files into the crate, leave `src/media/*` as façades.
- **(4) God-file decompositions**, one per PR, behavior-preserving, public path preserved via `pub use`:
  - `crates/thinclaw-channels/src/wasm/wrapper.rs` (5701L) — extract Telegram transport behind a trait.
  - `src/api/experiments.rs` (5434L) — split handlers per domain; coordinate the error-taxonomy fix with WS-07.
  - `src/agent/thread_ops.rs` (3032L) — extract `process_approval`.
  - `src/llm/runtime_manager.rs` (3096L), `src/extensions/manager.rs` (3343L), `src/agent/routine_engine.rs` (2536L), `src/agent/agent_loop.rs` (2467L), `crates/thinclaw-workspace/src/workspace_core.rs` (2248L), `src/setup/wizard/{mod,llm}.rs` + `src/setup/channels.rs`, desktop `rpc_dashboard.rs` / `remote_proxy.rs` / `sidecar.rs`.

**Out of scope (and which WS owns it):**
- **(3) `src/safety/*.rs` orphan deletion** — owned by **WS-11**. Reference only.
- **(5) Leaky-abstraction *behavior* fixes** — `Reasoning` unused `SafetyLayer` and `SpawnSubagentTool` unused `executor` are owned by **WS-08**; dual desktop MCP clients / provider builders by **WS-04**. WS-10 only absorbs the structural fallout once those land; it does not change their wiring.
- The `experiments.rs` behavior gaps (controller loop, lease/credit) — **WS-07**.
- The `split_message` UTF-8 fix and Discord Ed25519 — **WS-02 (WASM channels)**. WS-10 must not pre-empt those; the wrapper.rs split must rebase onto them.

## Current State (verified)

**(1) history/store duplication — confirmed near-byte-for-byte:**
- `src/history/store/` (18 files, 4723L) and `crates/thinclaw-db/src/postgres_store/` (20 files, 6545L) define the same `pub struct Store` (`src/history/store/core.rs:34` vs `crates/thinclaw-db/src/postgres_store/core.rs:34`).
- `diff` of the shared files: `jobs.rs`, `learning.rs`, `outcomes.rs`, `routine_crud.rs` are **byte-identical (0 diff lines)**; `conversation_queries.rs`=4, `conversations.rs`=2; the only real drift is `core.rs`=26 (crate generalized `new<C>` at `postgres_store/core.rs:47` vs root concrete `new(&DatabaseConfig)` at `store/core.rs:46`) and `sandbox_jobs.rs`=24.
- **The exact dead-code divergence the audit flagged is present in BOTH copies:** `conversation_metadata_with_handoff` carries `#[allow(dead_code)] // Prepared for conversation handoff persistence path` at `src/history/store/conversation_queries.rs:45` AND `crates/thinclaw-db/src/postgres_store/conversation_queries.rs:45`. A third `#[allow(dead_code)]` lives at `postgres_store/core.rs:41`.
- The crate version is **ahead**: it additionally owns `experiments.rs` (1402L) and `repo_projects.rs` (406L) that the root copy lacks, and is reached through `crates/thinclaw-db/src/postgres.rs:14 use crate::postgres_store::Store;`.
- `src/history/mod.rs` re-exports `store::Store` and ~30 types **from the root copy**, not from `thinclaw_db`. `src/db/mod.rs:23 pub use thinclaw_db::*;` already re-exports the crate. So root has two live copies side by side.
- ~40 external callers use `crate::history::{Store, ConversationMessage, ...}` (e.g. `src/cli/mcp.rs:1256`, `src/setup/wizard/persistence.rs:20`, `src/agent/learning/mod.rs:18`, `src/channels/web/server.rs:227`). All go through `src/history`, none import `thinclaw_db::postgres_store` directly — so the façade boundary is intact and migratable. **Status: drifted duplicate cruft.**

**(2) media half-extracted:**
- `crates/thinclaw-media/src/lib.rs` owns `cache`, `comfyui`, `document_extraction`, `limits`, `sticker`, `tts`, `tts_streaming`, `video`.
- `src/media/{cache,sticker,tts,tts_streaming,video,limits}.rs` are already thin façades (e.g. `src/media/cache.rs` is 3 lines: `pub use thinclaw_media::cache::*;`).
- **Still root-owned (real code, not extracted):** `src/media/audio.rs` (265L), `src/media/image.rs` (314L), `src/media/pdf.rs` (370L), `src/media/document.rs` (118L), `src/media/types.rs` (166L `MediaContent/MediaExtractor/MediaPipeline/MediaType`). The crate re-exports `MediaContent, MediaType` from `thinclaw_types`, so `types.rs` partially overlaps. **Status: half-wired migration.**

**(4) god-files (line counts verified):** wrapper.rs 5701, experiments.rs 5434, extensions/manager.rs 3343, thread_ops.rs 3032, runtime_manager.rs 3096, routine_engine.rs 2536, agent_loop.rs 2467, workspace_core.rs 2248, setup/wizard/mod.rs 2280, setup/wizard/llm.rs 2092, setup/channels.rs 2045, desktop rpc_dashboard.rs 2332, remote_proxy.rs 1787, sidecar.rs 1710.
- **wrapper.rs:** 132 `telegram`-matching references; concrete Telegram coupling at `wrapper.rs:550 markdown_to_telegram_html`, `:648 if prepared.name == "telegram"`, `:1078 apply_telegram_runtime_state`, `:2233 struct TelegramWebhookInfoEnvelope`, `:2253 TELEGRAM_POLLING_OVERRIDE`, `:2318 telegram_webhook_url_from_tunnel_url`, `:2403 telegram_live_webhook_info`. The generic WASM-channel host is interleaved with Telegram-specific webhook/polling/topic logic.
- **experiments.rs:** **124** `ApiError::Internal(` sites, **139** `map_err`; verified handler domains group cleanly: projects (`list/get/create/update/delete_project`), runners (`*_runner`, `validate_runner`), campaigns (`list/get_campaign`, `start/pause/resume/cancel/promote_campaign`), trials (`list/get_trial`), targets (`list/create/update/delete/link_target`), leases (`lease_artifact/complete/credentials/event/job/owner_user_id/status`, `reissue_lease`), artifacts (`list_artifacts`), opportunities (`list_opportunities`), usage (`list_model_usage`, `list_gpu_cloud_providers`), controller (`start_experiment_controller_loop`).
- **`ApiError` taxonomy already exists** at `src/api/error.rs:10` with `InvalidInput, SessionNotFound, Unavailable, FeatureDisabled, Agent(#[from]), Serialization(#[from]), UuidParse(#[from]), Internal`, each mapped to a `GatewayApiErrorKind` at `error.rs:60-83`. The 124 `Internal` sites in experiments.rs are flattening *DB/validation/not-found* errors that already have better variants — the taxonomy doesn't need new variants, the call sites need to use them (and possibly add `NotFound`/`Conflict` if the audit shows gaps).
- **thread_ops.rs:** `pub(super) async fn process_approval` at `src/agent/thread_ops.rs:1924` (the ~850L block the audit cites). Uses `crate::history::ConversationKind` (`:36`) and `crate::media::*` (`:1109-1116`).
- **runtime_manager.rs:** natural seams — `RuntimeLlmProvider` struct+impl `:144-931`, `LlmRuntimeManager` `:933-2135`, pure validation/derive helpers `validate_providers_settings :2136`, `derive_runtime_defaults :2168`, `normalize_providers_settings :2177`.
- **extensions/manager.rs:** `ExtensionManager` `:134`, with cohesive method clusters — channel runtime (`:230-302`), install/auth/activate/list/remove (`:491-873`), MCP config/client (`:1026-1157`), OAuth callback (`:2771-2864`), setup schema/validate/save (`:2864-3343`).

WS-07/WS-04/WS-08/WS-11 sibling docs are not yet written (only `AUDIT-FINDINGS.md` exists in `docs/remediation/`); depends_on references them as the planned behavior workstreams that must land first.

## Decision Points

1. **history/store: thin-façade re-export vs. delete-and-redirect.**
   - *Option A (façade):* gut `src/history/store/` to a single `pub use thinclaw_db::postgres_store::*;` plus the root-only `new(&DatabaseConfig)` constructor adapter, keep `src/history/mod.rs` re-exports pointing at it.
   - *Option B (delete):* delete `src/history/store/` entirely; rewrite `src/history/mod.rs` to re-export from `thinclaw_db` directly; add a root constructor shim where `Store::new(&config.database)` is called (`src/cli/mcp.rs:1256`, `src/cli/tool.rs:587`, `src/cli/secrets.rs:264`).
   - **Recommendation: Option B (delete).** This is genuinely drifted duplicate cruft, not half-finished capability. The crate copy is strictly ahead (has experiments/repo_projects). Keeping a re-export-only `src/history/store/` directory just preserves a vestigial module. The one wrinkle is the root `new(&DatabaseConfig)` vs crate `new<C>` signature: resolve by having root call `Store::from_pool` or implement the `C` trait bound for `DatabaseConfig` (verify which `C` bound the crate expects before writing the redirect). Preserve every public path through `src/history/mod.rs` `pub use`.

2. **media types.rs ownership.**
   - The crate already re-exports `MediaContent, MediaType` from `thinclaw_types`, but `src/media/types.rs` (166L) also defines `MediaExtractor, MediaPipeline, MediaExtractError`. Move the extractor/pipeline traits into `thinclaw-media` (they belong with the extractors), keep the data records (`MediaContent`, `MediaType`) in `thinclaw-types` as they already are.
   - **Recommendation: move `audio/image/pdf/document` + the `MediaExtractor`/`MediaPipeline` traits into `thinclaw-media`; leave `src/media/*` as façades.** Realizes the vision (single owner). Watch the `document-extraction` feature gate — `document.rs` is already `#[cfg(feature = "document-extraction")]`.

3. **wrapper.rs Telegram extraction: trait-behind-port vs. inline submodule.**
   - *Option A:* define a `WasmChannelTransport` trait (webhook setup, polling offset, topic registry, runtime-state apply) in `crates/thinclaw-channels/src/wasm/`, implement a `TelegramTransport` adapter, leave wrapper.rs generic.
   - *Option B:* just move the Telegram-specific fns into `wrapper/telegram.rs` submodule without a trait.
   - **Recommendation: Option A (trait).** The audit explicitly calls for "separating generic WASM-channel host from ~30 hardcoded Telegram branches." A trait makes the next channel (Slack/Discord have similar but smaller hooks) pluggable and is the durable fix. Higher effort; do it as its own PR. Verify the `if prepared.name == "telegram"` branch at `:648` is the only dispatch point before designing the trait surface.

4. **experiments.rs error taxonomy ownership.**
   - The `s/ApiError::Internal/proper-variant/` sweep touches `src/api/error.rs` (shared) and is conceptually a behavior change (different HTTP/error codes surface to the UI). WS-07 owns experiments behavior.
   - **Recommendation: WS-07 lands the error-taxonomy *semantics* (which variant each failure maps to, plus any new `NotFound`/`Conflict` variant in `error.rs`); WS-10 does the mechanical handler-file split and carries the taxonomy edits along once WS-07's mapping is defined.** Do not split the file before WS-07's error mapping lands, or the 124 sites get touched twice.

5. **desktop decompositions: in-scope vs. defer.**
   - `apps/desktop/**` is a separate Tauri crate. WS-04 owns desktop behavior (dual MCP clients).
   - **Recommendation: keep the three desktop god-file *decompositions* in WS-10 (pure structural) but sequence them LAST and only after WS-04 lands**, so the dual-stack consolidation isn't fighting a file move.

## Tasks

Ordered. **Each god-file = one PR.** Recommended order: history/store consolidation first (highest leverage, unblocks clean DB edits), then media, then wrapper.rs and experiments.rs, then the rest.

- [ ] **T1: Consolidate `src/history/store/` onto `thinclaw_db::postgres_store` (delete the root copy).**
  - **Files:** delete `src/history/store/` (18 files); rewrite `src/history/mod.rs` to `pub use thinclaw_db::postgres_store::{...}` (preserve the exact symbol list currently re-exported at `src/history/mod.rs:14-30`) and re-export `analytics`/`experiments` as today; add a root constructor shim if `Store::new(&config.database)` callers (`src/cli/mcp.rs:1256`, `src/cli/tool.rs:587`, `src/cli/secrets.rs:264`, `src/setup/wizard/persistence.rs:20,215`, `src/setup/wizard/mod.rs:1669`) can't call the crate `new<C>` directly.
  - **Change:** make `src/history` a pure façade. Before deleting, `diff` each file against its crate twin and port any root-only behavior (the `core.rs` 26-line and `sandbox_jobs.rs` 24-line deltas) into the crate behind the existing port surface — do NOT silently drop root behavior. Remove the now-single `#[allow(dead_code)]` on `conversation_metadata_with_handoff` if still unused after consolidation, or wire it (it is "prepared for conversation handoff persistence" — check WS that owns handoff before deleting). **Confirmed: no WS owns conversation handoff persistence, so `conversation_metadata_with_handoff` has no consumer — disposition is ERASE (delete the dead fn, do not preserve it).**
  - **Acceptance:** `src/history/store/` no longer exists; `rg "history::store" src` returns only `src/history/mod.rs`; all ~40 callers compile unchanged; no duplicate `Store` definition in the tree; `rg "allow(dead_code)" crates/thinclaw-db/src/postgres_store src/history` shows the dead fn resolved.
  - **Effort:** L
  - **Verification:** `cargo fmt --all -- --check && cargo check --workspace --features full && cargo clippy --workspace --all-targets --features full -- -D warnings && cargo test -p thinclaw-db --features full && cargo test --workspace --no-run --features full`. DB tests need a Postgres (`pgvector/pgvector:pg17`) with `migrations/V*.sql` applied (see CLAUDE.md local-dev note).

- [ ] **T2: Finish `src/media` extraction into `thinclaw-media`.**
  - **Files:** move `src/media/{audio,image,pdf,document}.rs` into `crates/thinclaw-media/src/` (add to `lib.rs` `pub mod` list, honoring `#[cfg(feature = "document-extraction")]` on `document`); move the `MediaExtractor`/`MediaPipeline`/`MediaExtractError` traits from `src/media/types.rs` into a `thinclaw-media` module (data records `MediaContent`/`MediaType` stay in `thinclaw_types`); reduce `src/media/{audio,image,pdf,document,types}.rs` to façade re-exports matching the existing pattern in `src/media/cache.rs`.
  - **Change:** keep `src/media/mod.rs` public API byte-identical (same `pub use` list at `src/media/mod.rs:26-39`). Callers `src/agent/thread_ops.rs:1109-1116` (`crate::media::MediaType/MediaPipeline`) must compile unchanged.
  - **Acceptance:** `src/media/*.rs` are all façades; `thinclaw-media` owns the extractors; `rg "crate::media::" src` callers unchanged; the `document-extraction` feature still gates `DocumentExtractor`.
  - **Effort:** M
  - **Verification:** `cargo check --workspace --features full && cargo check --no-default-features --features edge && cargo clippy --workspace --all-targets --features full -- -D warnings && cargo test -p thinclaw-media`. (Run the `edge` check because `document-extraction`/media features vary by profile.)

- [ ] **T3: Decompose `crates/thinclaw-channels/src/wasm/wrapper.rs` — extract Telegram transport behind a trait.**
  - **Files:** new `crates/thinclaw-channels/src/wasm/transport.rs` (`WasmChannelTransport` trait), `crates/thinclaw-channels/src/wasm/telegram_transport.rs` (impl); shrink `wrapper.rs`; `wasm/mod.rs` stays a façade with `pub use`.
  - **Change:** define the trait around the Telegram-specific seams verified at `wrapper.rs:648` (name dispatch), `:1078`/`:2334 apply_telegram_runtime_state`, `:2318 telegram_webhook_url_from_tunnel_url`, `:2403 telegram_live_webhook_info`, and the `TelegramWebhookInfo*` structs (`:2233-2242`). Move `markdown_to_telegram_html` call-through (`:550`) and `TELEGRAM_POLLING_OVERRIDE` (`:2253`) into the adapter. Keep `pub(super)`/`pub(in crate::wasm)` visibility; do not widen. **Rebase onto WS-02's `split_message` UTF-8 fix first** — do not land before it.
  - **Acceptance:** `wrapper.rs` no longer contains `if prepared.name == "telegram"` hardcoding; Telegram logic lives behind `WasmChannelTransport`; public channel API unchanged; `rg -ci telegram crates/thinclaw-channels/src/wasm/wrapper.rs` drops substantially (target: only generic references remain).
  - **Effort:** XL
  - **Verification:** `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --features full -- -D warnings && cargo test -p thinclaw-channels --features full`. Behavior-preserving: run the existing channel tests; no new test logic.

- [ ] **T4: Decompose `src/api/experiments.rs` + apply the error taxonomy (coordinate with WS-07).**
  - **Files:** new `src/api/experiments/` directory: `mod.rs` (façade `pub use`), `projects.rs`, `runners.rs`, `campaigns.rs`, `trials.rs`, `targets.rs`, `leases.rs`, `artifacts.rs`, `opportunities.rs`, `usage.rs`, `controller.rs`, plus a `shared.rs`/`helpers.rs` for `ensure_experiments_enabled` (`experiments.rs:250`), `resolve_project_workdir` (`:267`), `resolved_secret_env_pairs` (`:343`), executor/secrets registration (`:194-206`).
  - **Change:** mechanical move of the verified handler groups into per-domain files; `mod.rs` re-exports every `pub async fn` so `src/api/mod.rs` and route wiring are unchanged. Replace `ApiError::Internal(e.to_string())` with the correct existing variant per WS-07's mapping (DB miss → `NotFound`/`Unavailable`, validation → `InvalidInput`, disabled → `FeatureDisabled`). **Do not start until WS-07 defines the mapping.** Keep `#[ignore]`d quarantined test at `experiments.rs:5060` with its module.
  - **Acceptance:** no file in `src/api/experiments/` exceeds ~800L; `rg "ApiError::Internal" src/api/experiments` only remains for genuinely-internal failures (target: well under the current 124); all routes resolve; public handler paths preserved.
  - **Effort:** XL
  - **Verification:** `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --features full -- -D warnings && cargo test -p thinclaw --features full -- experiments` (excluding the quarantined ignore).

- [ ] **T5: Extract `process_approval` from `src/agent/thread_ops.rs`.**
  - **Files:** new `src/agent/thread_ops/` directory or sibling `src/agent/approval.rs`; move the ~850L `pub(super) async fn process_approval` (`thread_ops.rs:1924`) and its helpers; `thread_ops.rs`/`mod.rs` façade re-exports.
  - **Change:** keep `pub(super)` visibility; preserve the `crate::history::ConversationKind` (`:36`) and `crate::media::*` (`:1109`) imports in whichever file ends up owning them. Behavior-preserving move only.
  - **Acceptance:** `process_approval` lives in its own focused module; `thread_ops.rs` shrinks below ~2200L; callers compile unchanged.
  - **Effort:** L
  - **Verification:** `cargo clippy --workspace --all-targets --features full -- -D warnings && cargo test -p thinclaw --features full -- agent::`.

- [ ] **T6: Decompose `src/llm/runtime_manager.rs`.**
  - **Files:** new `src/llm/runtime_manager/` dir: `mod.rs` façade, `provider.rs` (`RuntimeLlmProvider` `:144-931`), `manager.rs` (`LlmRuntimeManager` `:933-2135`), `validation.rs` (`validate_providers_settings :2136`, `derive_runtime_defaults :2168`, `normalize_providers_settings :2177`), `types.rs` (`RuntimeStatus :58`, `RouteSimulationScore :86`, `RouteSimulationResult :98`, snapshot/route structs).
  - **Change:** pure split at the verified seams. **Coordinate with WS-08** (Reasoning `SafetyLayer`) and the WS LLM owner (CheapSplit cascade) — if those land first, the split absorbs them; otherwise do not touch routing behavior. Preserve `pub use` of the public symbols from `src/llm/mod.rs`.
  - **Acceptance:** no submodule > ~1100L; `hydrate_runtime_credentials_from_secrets` (`:40`) and all public fns re-exported; behavior unchanged.
  - **Effort:** L
  - **Verification:** `cargo clippy --workspace --all-targets --features full -- -D warnings && cargo test -p thinclaw --features full -- llm::`.

- [ ] **T7: Decompose `src/extensions/manager.rs`.**
  - **Files:** new `src/extensions/manager/` dir: `mod.rs` façade owning `struct ExtensionManager` + `new`, `channels.rs` (runtime/active-channels `:230-302`), `lifecycle.rs` (install/auth/activate/list/remove `:491-873`), `mcp.rs` (MCP config/client `:1026-1157`), `oauth.rs` (`:2771-2864`), `setup.rs` (schema/validate/save `:2864-3343`). Keep `SetupResult :80`, `AuthRequestContext :88`, `ExtensionSetupSchema :95` near their owning module or a `types.rs`.
  - **Change:** `impl ExtensionManager` blocks split across files (Rust allows multiple impl blocks); keep field visibility `pub(super)`/`pub(in crate::extensions)`. **Coordinate with WS-04** (dual MCP clients) — if WS-04 consolidates clients, the `mcp.rs` split lands on top.
  - **Acceptance:** no submodule > ~1000L; public `ExtensionManager` API unchanged.
  - **Effort:** L
  - **Verification:** `cargo clippy --workspace --all-targets --features full -- -D warnings && cargo test -p thinclaw --features full -- extensions::`.

- [ ] **T8: Decompose `src/agent/routine_engine.rs` and `src/agent/agent_loop.rs`.**
  - **Files:** routine_engine → `src/agent/routine_engine/` (trigger eval, event dispatch, run lifecycle); agent_loop → `src/agent/agent_loop/` (submission, response handling, self-repair hook). Façade `mod.rs` each.
  - **Change:** behavior-preserving split. **agent_loop must rebase onto the WS that wires self-repair `with_builder` (`agent_loop.rs:605` per audit §4)** — do not alter that wiring here. routine_engine: do not change the break-on-first-error dispatch (`routine_engine.rs:898`, owned by the routines behavior WS).
  - **Acceptance:** each file < ~1200L; hot-path behavior identical; tests green.
  - **Effort:** L
  - **Verification:** `cargo clippy --workspace --all-targets --features full -- -D warnings && cargo test -p thinclaw --features full -- agent::`.

- [ ] **T9: Decompose `crates/thinclaw-workspace/src/workspace_core.rs`.**
  - **Files:** split into focused submodules under `crates/thinclaw-workspace/src/workspace_core/` by responsibility (core/manager, repository helpers, search/chunking already partly separate); `lib.rs`/façade re-exports preserved.
  - **Change:** pure structural; respect crate dependency-direction rules (no `use thinclaw::`).
  - **Acceptance:** file < ~1200L; `rg "use thinclaw::" crates/thinclaw-workspace` empty; public crate API unchanged.
  - **Effort:** M
  - **Verification:** `cargo clippy -p thinclaw-workspace --all-targets --features full -- -D warnings && cargo test -p thinclaw-workspace`.

- [ ] **T10: Decompose onboarding god-files (`src/setup/wizard/mod.rs`, `wizard/llm.rs`, `src/setup/channels.rs`).**
  - **Files:** keep `wizard/mod.rs` a façade; extract phases into focused submodules (provider selection, validation, persistence is already `wizard/persistence.rs`); split `channels.rs` by channel-family or step.
  - **Change:** behavior-preserving. Per CLAUDE.md, update `src/setup/README.md` if step ownership moves; do NOT restate wizard steps.
  - **Acceptance:** each file < ~1200L; onboarding flow unchanged; `src/setup/README.md` aligned.
  - **Effort:** L
  - **Verification:** `cargo clippy --workspace --all-targets --features full -- -D warnings && cargo test -p thinclaw --features full -- setup::`.

- [ ] **T11: Decompose desktop god-files (LAST, after WS-04).**
  - **Files:** `apps/desktop/backend/src/thinclaw/commands/rpc_dashboard.rs` (2332L), `apps/desktop/backend/src/thinclaw/remote_proxy.rs` (1787L), `apps/desktop/backend/src/sidecar.rs` (1710L) — split by command-group / proxy-concern / sidecar-lifecycle.
  - **Change:** behavior-preserving; **rebase onto WS-04's dual-stack consolidation** so the MCP-client/provider-builder dedup isn't undone.
  - **Acceptance:** each file < ~1200L; Tauri commands resolve; desktop build green.
  - **Effort:** L
  - **Verification:** build the desktop backend crate (`cargo clippy -p <desktop-backend-crate> --all-targets -- -D warnings`); run any desktop-companion smoke that CI runs.

- [ ] **T12: Wire-or-document the `JobToolHostPort` stub.**
  - **Files:** the `JobToolHostPort` definition and its half-stubbed implementation (the structured tool-host path that currently returns `Unavailable`).
  - **Change:** the coverage critic found `JobToolHostPort` (the structured tool-host port whose impl is half-stubbed to return `Unavailable`) is owned by no other WS. Decide one of two dispositions: (a) implement the structured path so the port returns real results instead of `Unavailable`, OR (b) add a clear doc note at the stub site plus a tracking marker (e.g. a `TODO(WS-10)`/issue reference) stating the structured path is intentionally unavailable for now. Pick (b) unless implementing the structured path is trivial and self-contained; do not leave a silent `Unavailable` with no rationale.
  - **Acceptance:** the `JobToolHostPort` stub is either wired (no longer returns `Unavailable`) or carries an explicit doc note + tracking marker explaining the intentional-unavailable state; no unexplained `Unavailable` stub remains.
  - **Effort:** S
  - **Verification:** `cargo clippy --workspace --all-targets --features full -- -D warnings && cargo test --workspace --no-run --features full`.

## Best Practices (workstream-specific)

- **Façade `mod.rs` is the load-bearing pattern.** Copy the exact shape already in the repo: `src/media/cache.rs` (`pub use thinclaw_media::cache::*;`) for a pure re-export, and `crates/thinclaw-db/src/postgres_store/mod.rs` for a `mod x; pub use x::Symbol;` façade with `#[cfg(...)]`-gated submodules. `src/history/mod.rs:14-30` is the canonical "re-export a curated symbol list" example.
- **Multiple `impl` blocks across files** is the idiomatic Rust way to split a large struct (extensions/manager.rs) without changing its API — each file does `impl ExtensionManager { ... }`.
- **Narrow visibility:** keep cross-submodule helpers at `pub(super)` or `pub(in crate::...)` (the existing stores already do this — `conversation_metadata_with_handoff` is `pub(super)`). Never widen to `pub` just to make a split compile.
- **One PR per god-file**, behavior-preserving, with the diff dominated by moves. Use `git mv` where possible so review sees renames, not rewrites.
- **Diff before delete** (T1): the consolidation must `diff` each twin file and port real deltas, not assume identity (`core.rs` and `sandbox_jobs.rs` genuinely differ).
- Run `cargo test --no-run` after each split to catch test-module breakage early; keep `tests.rs`/`test_support.rs` next to the module they validate (CLAUDE.md), e.g. `src/agent/learning/tests.rs`.

## Common Pitfalls

- **The exact divergence the audit caught: a fix landing in only one of N copies.** `conversation_metadata_with_handoff` carries identical `#[allow(dead_code)]` at `src/history/store/conversation_queries.rs:45` AND `crates/thinclaw-db/src/postgres_store/conversation_queries.rs:45`; `split_message` was fixed in whatsapp but not telegram/slack/discord. Consolidating (T1) is the structural cure — do not "fix both copies," delete one.
- **Assuming the two stores are byte-identical and blind-deleting.** They are NOT: `core.rs` (26-line diff, `new` signature) and `sandbox_jobs.rs` (24-line diff) carry root-only behavior that must be ported into the crate first.
- **Splitting experiments.rs before WS-07 defines the error mapping** — you'd touch 124 `ApiError::Internal` sites twice. Gate T4 on WS-07.
- **Pre-empting WS-02/WS-04/WS-08 behavior fixes** inside the god-files. WS-10 is structural only; rebase the splits on top of the behavior PRs, never bundle a behavior change into a "move" PR.
- **Widening visibility to make a split compile** — violates CLAUDE.md and silently grows the API surface. Use `pub(in crate::...)`.
- **Crate dependency-direction violation:** when moving code into `thinclaw-media`/`thinclaw-db`/`thinclaw-workspace`, ensure no `use thinclaw::` sneaks in (run the `rg "use thinclaw::" crates` guard from CRATE_OWNERSHIP.md — CI enforces it in the code-style job).
- **Feature-matrix breakage:** media (`document-extraction`), postgres store (`postgres`), and WASM channels (`wasm-runtime`) are feature-gated and vary across edge/light/desktop/full. Always run at least `--features full` and `--no-default-features --features edge`.

## Multi-Worker Execution Plan (ultracode)

- **Worker decomposition:** Each task T1–T12 is an independent god-file/migration/port-disposition and maps to one worker + one PR. **T1 must complete first** (highest leverage, removes the duplicate `Store` that other DB-adjacent edits would otherwise have to track). After T1, T2–T12 are mostly independent and can fan out in parallel — EXCEPT the cross-WS gates: T4 waits on WS-07's error mapping, T3 on WS-02's split_message fix, T6/T7 on WS-08/WS-04, T11 on WS-04. Sequence T11 last; T12 (the `JobToolHostPort` stub disposition, Effort S) is independent and can land anytime.
- **Isolation:** **Yes — git worktree isolation required.** Parallel workers each touch large, overlapping module trees (`src/agent`, `src/llm`, `src/api`); running them in separate worktrees avoids index contention and lets each PR land independently. T1 (history) and T2 (media) touch shared callers (`thread_ops.rs` imports both `crate::history` and `crate::media`) — serialize T1→T2 or rebase carefully.
- **Workflow shape:** per task — **implement** (move + façade) → **verify** (`/ship` gate below) → **review** (`/code-review high` focused on "is this behavior-preserving? any public-path drift? any widened visibility?") → **fix**. Fan-out: spawn one implement→verify→review chain per task after T1 merges; collect into a tracking checklist. For T3/T4 (XL trait/handler splits) run `/code-review` at `high` or `ultra` since they have the most semantic surface.
- **Verification gate (exact):**
  ```bash
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --features full -- -D warnings
  cargo check --no-default-features --features edge
  cargo test -p <touched-crate> --features full          # e.g. thinclaw-db, thinclaw-media, thinclaw-channels, thinclaw
  cargo test --workspace --no-run --features full
  rg "use thinclaw::" crates                              # must be empty (CRATE_OWNERSHIP guard)
  rg 'package\s*=\s*"thinclaw"' crates -g Cargo.toml      # must be empty
  ```
  Then `/ship` and `/code-review high`.
  **DB/Docker prerequisites for T1:** a running `pgvector/pgvector:pg17` Postgres with `migrations/V*.sql` applied into `thinclaw_test` (mirror CI; see CLAUDE.md local-dev note); `db_contract` and `schema_divergence` need the container, broader workspace integration needs the migrations too.

## Definition of Done

- [ ] **T1:** `src/history/store/` deleted; `src/history` is a façade over `thinclaw_db`; single `Store` definition in the tree; root-only deltas (`core.rs`, `sandbox_jobs.rs`) ported into the crate; `conversation_metadata_with_handoff` dead-code resolved (wired or erased); all callers compile.
- [ ] **T2:** `thinclaw-media` owns audio/image/pdf/document + extractor traits; `src/media/*` are façades; `MediaContent`/`MediaType` still from `thinclaw_types`; feature gates intact.
- [ ] **T3–T11:** each god-file split into focused submodules below target size, behavior-preserving, public paths preserved by `pub use`, narrow visibility maintained, no `use thinclaw::` in crates.
- [ ] **T12:** `JobToolHostPort` stub resolved — structured path wired, or an explicit doc note + tracking marker records the intentional-unavailable state (no silent `Unavailable`).
- [ ] Cross-WS gates honored: T4 after WS-07 error mapping; T3 after WS-02 split_message; T6/T7 after WS-08/WS-04; T11 after WS-04.
- [ ] Full verification gate green on every PR (fmt, clippy `--all-targets -D warnings`, edge check, per-crate tests, `--no-run` workspace, CRATE_OWNERSHIP rg guards empty).
- [ ] Docs updated where ownership moved: `docs/CRATE_OWNERSHIP.md` (media now fully crate-owned; history store no longer dual), CLAUDE.md repo-shape notes, `src/setup/README.md` (T10).
- [ ] Decision points 1–5 resolved and recorded in PR descriptions.
- [ ] No behavior change introduced by any WS-10 PR (verified by review + green existing tests; no new test logic added by the move PRs).
