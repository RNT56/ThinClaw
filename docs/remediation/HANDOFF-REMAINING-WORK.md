# ThinClaw — Remaining Work Handoff (Execution-Ready, v2)

> **Audience:** the next implementation session. This document is self-contained — you do
> not need to read the prior handoff or any analysis thread. Every claim below was verified
> against the working tree on branch `remediation/execution`. Where the older
> `docs/remediation/FOLLOWUPS.md` ledger is stale, this doc states the corrected truth and
> the corrected scope.
>
> **Work in** `/Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop` on `remediation/execution`.
> The outer `ThinClaw/` folder is **not** the repo.

---

## 0. Orientation & ground truth

- The 13-workstream remediation already landed; what remains are the `F-xx` residuals plus a
  small set of newly-surfaced gaps. `FOLLOWUPS.md` tracks `F-01…F-19` but several of its
  entries and line numbers are **stale** after the WS-10 god-file decomposition (e.g.
  `src/api/experiments.rs` is now the directory module `src/api/experiments/`).
- **Already DONE — do not schedule, just mark resolved with evidence:**
  - **F-16** DDG live test quarantine — `#[ignore]` present at `apps/desktop/backend/src/rig_lib/tools/web_search.rs:667`; nightly `--ignored` job exists.
  - **F-17** CI ALSA for `full` — **resolved by reversal.** `voice` was *removed* from `full` (`Cargo.toml:296-309`, with an explicit comment). CI installs `libasound2-dev` only for `all-features`. **Do NOT add ALSA to the `full` CI leg** — that would be wrong.
  - **image_gen divide-by-zero** — fixed (`apps/desktop/backend/src/image_gen.rs:12` zero-guard).
  - **Shim CI-compile gate** — already exists (`.github/workflows/ci.yml:735-794` builds all 12 shims to `wasm32-wasip2`).
  - **Shim config-load round-trip test** — already exists (`tests/registry_channel_catalog.rs` parses every shim's `*.capabilities.json`). The WS-03 doc's "never compiled / no round-trip test" claims are stale.
  - **F-05** `search_files` base_dir — accepted no-op (`crates/thinclaw-tools/src/builtin/search_files.rs:57` is consistent with `file.rs`). No action.
  - **`api/experiments` god-file split** — done (9 modules). Only the F-14 *error-taxonomy* work remains.

- **Two stale ledger entries to correct in the same PR that touches their area:**
  - `FOLLOWUPS.md` **F-17** still says "voice is now in the full feature" → false; mark resolved-by-reversal.
  - `FOLLOWUPS.md` **F-18** says "thread a dispatcher/session handle into the spawn block" → architecturally impossible at that seam (see Lane 4 / F-18). Fix the wording.

---

## 1. Cross-cutting rules (apply to every lane)

1. **Same-PR canonical-doc rule (enforced by `CLAUDE.md`).** Any PR that changes behavior in
   an area with a canonical doc MUST update that doc in the same PR, and update
   `FEATURE_PARITY.md` if it changes tracked feature behavior. Per-item doc targets are listed
   inline below.
2. **Kill-switch rule.** Every new capability that changes autonomy or network egress ships
   **default-off** with a documented switch: F-06 autoplan (`REPO_PROJECTS_AUTOPLAN`), F-13 S3
   artifact store (config + secret-name), voice (already `THINCLAW_VOICE_WAKE`). New egress
   (F-13) also gets a `src/NETWORK_SECURITY.md` note.
3. **Merge order / collision control.** `F-01` and `F-14` **both edit `src/api/experiments/`** —
   land them as a **single experiments-package PR** (F-01 first, then the F-14 taxonomy sweep
   over the same modules) or worktree-isolate. Land `F-10` (capabilities markers) **before** any
   `F-09` shared-SDK extraction so the dedup rebases onto the final per-shim layout.
4. **Ledger close-out checklist** (run on every F-item you close — these are
   append/checkbox edits, never history rewrites):
   - `docs/remediation/FOLLOWUPS.md` (mark done / strike remaining)
   - `docs/remediation/EXECUTION-SUMMARY.md` (Deferred list)
   - `docs/remediation/README.md` **Status Tracker + Decision Register row + Wave gate**
     (this file is *already* stale — all 13 WS show unchecked while EXECUTION-SUMMARY says
     "complete"; fix the contradiction while you're there)
   - `DEFERRED-DELETIONS.md` / `WS-03-shim-classification.md` where the F-item maps to one
   - the relevant **canonical product doc** per rule (1)
   - Do **not** edit the frozen numbered WS docs `01-13` (they read "Status: Not started" by
     design); use README as the live roll-up.

---

## LANE 0 — Build baseline, CI gates, and verification hygiene

### 0.1 Baseline-profile decision (resolve this FIRST — it changes the rest)

**Ground truth:** `cargo check -p thinclaw --no-default-features` (zero features) **fails today**
(`error[E0432]` at `src/cli/mod.rs:80` — `setup::{GuideTopic,OnboardingProfile,UiMode}` are gated
at `src/setup/mod.rs:32`). A bare zero-feature root build has **never** been a CI gate or shipped
artifact; a zero-feature binary has **no DB backend** and cannot onboard/persist/store secrets
(`src/db/mod.rs:68`, `src/main.rs:290` bail at runtime). The smallest *meaningful & shipped*
profile is `edge` (= `libsql`), which **compiles clean with zero warnings today** and is already
a CI leg.

> **Do not call this "restoring hygiene" — a zero-feature root gate is net-new.**

**Recommended (small, coherent):** lock **`edge` as the baseline** and make the no-backend case
an *intentional* failure:

```rust
// src/lib.rs (top)
#[cfg(not(any(feature = "postgres", feature = "libsql")))]
compile_error!("thinclaw requires a database backend: enable `postgres` or `libsql` (or a profile that includes one, e.g. `edge`).");
```

This is one line, codifies the real runtime requirement, and turns the accidental `E0432` into a
clear message. `edge`/`minimal-libsql`/`minimal-postgres` CI legs already lock the smallest real
profiles.

**Alternative (only if a literal DB-less build is a real product target — it is not today):**
treat zero-feature green as a **bounded multi-file cfg-audit**, not a small edit:
- Gating the import alone cascades into **6 new `E0412/E0433` errors** because the clap
  `Command::Onboard` variant fields *are* the gated types (`src/cli/mod.rs:148-168`). You must
  gate the **whole variant** and its dispatch arm at `src/main.rs:264-296` (which destructures and
  derefs `guide`/`ui`/`profile`) together, plus the `#[cfg(test)]` construction sites for a
  zero-feature `cargo test`.
- Then iterate `cargo check -p thinclaw --no-default-features` to fixpoint, clearing
  unreachable (`src/app.rs:335`, `src/cli/tool.rs:635`) and unused-var/import warnings
  (`src/app.rs:268/440`, `src/cli/secrets.rs:224`, `src/cli/tool.rs:543/584`, `src/cli/reset.rs:16`,
  `src/config/mod.rs:44`, `src/setup/channels/mod.rs:25/28`). Budget ~10-15 of the 38 cfg-gated files.
- Then add `-D warnings` **and** a matching `ci.yml` matrix leg (else the gate is hand-only).

**Done-when:** the chosen baseline compiles `-D warnings` clean and is enforced in CI.
**Docs:** update `docs/BUILD_PROFILES.md` to state the minimum-supported profile contract.

### 0.2 Test-gate corrections (the prior plan's gates were weaker than CI)

CI runs clippy as `--workspace --all-targets … -- -D warnings` (`ci.yml:56` + `:125`), never
`-p thinclaw`. `-p thinclaw` lints only the root crate, so a regression in any of the 26 sibling
crates passes locally but fails CI. Use the corrected gate list in §6.

### 0.3 schema_divergence strict mode is dark in CI (NEW — owned by this lane)

`tests/schema_divergence.rs:101-106` gates type/nullability/index parity behind
`SCHEMA_DIVERGENCE_STRICT=1`, which **no workflow sets** — the Postgres↔libSQL parity guarantee
is half-enforced. Seed the intended-divergence allowlist (recorded in `DEFERRED-DELETIONS.md:30`),
then run `schema_divergence` with `SCHEMA_DIVERGENCE_STRICT=1` in CI (or nightly).

---

## LANE 1 — Trust boundaries

### F-01 — Experiments LocalDocker proxy uses the env-only credential resolver (SMALLER than the ledger says)

- **Status:** open, real. `experiment_execution_backend` (`src/api/experiments/git.rs:108`) builds
  `DockerSandboxExecutionBackend::from_sandbox(Arc::new(SandboxManager::new(sandbox_config)), …)`
  at `git.rs:124-127` **without** `.with_credential_store(...)`. A store-less `SandboxManager`
  falls back to the env resolver (`crates/.../sandbox/manager.rs:199-218`), so experiment Docker
  trials resolve allowlisted-host creds from host env instead of the encrypted, audited
  `get_for_injection` path.
- **Minimal fix (do NOT thread a fresh `Arc<dyn SecretsStore>` through callers):** a module-local
  accessor already exists and is startup-populated:
  - `research_secrets_store() -> Option<Arc<dyn SecretsStore + Send + Sync>>` at
    `src/api/experiments/types.rs:66` (`pub(super)`, registered via `register_experiment_secrets_store`
    from `src/main.rs:1626`, gated on `components.secrets_store` being present).
  - `git.rs` already does `use super::*;`, so it can call it directly.
  - Add **one** `user_id: &str` param to `experiment_execution_backend`; in the LocalDocker branch,
    after `SandboxManager::new(...)`, chain `.with_credential_store(store, user_id)` when the accessor
    returns `Some`, else keep env fallback (the OnceLock can legitimately be unset).
  - Pass `user_id` from the single caller `execute_local_trial` (`src/api/experiments/execution.rs:465`;
    `user_id` is in scope at `:410` and is the real **campaign owner** — strictly better than the main
    runtime's hardcoded `"default"` at `app.rs:226`).
- **Scope:** ~1 param + 1 accessor call + 1 conditional builder chain + 1 call-site edit.
- **Test:** prove the LocalDocker backend resolves creds via the store when registered (no secret
  values logged) and falls back to env when unset.
- **Docs/ledger:** FOLLOWUPS F-01; no canonical product doc.

### F-02 — MCP HTTP/OAuth clients are not DNS-rebind pinned (MEDIUM, not a thin helper)

- **Status:** open. `crates/thinclaw-tools/src/mcp/client.rs:577` builds a long-lived shared
  `http_client` with no pin; send sites `:769 / :812 / :857` reuse it; client cloned at `:1489`.
  OAuth clients in `crates/thinclaw-tools/src/mcp/auth.rs` (`:366 / :402 / :489`, also the
  registration/token builders) are unpinned. Config-time `validate_outbound_url` at
  `mcp/config.rs:253` is the non-pinning variant (leave it).
- **Reuse, don't reinvent:** the canonical pinned helpers already exist —
  `validate_outbound_url_pinned` (`crates/thinclaw-tools-core/src/url_guard.rs:55`) and the
  per-request rebuild pattern in `crates/thinclaw-tools/src/builtin/http.rs:108` (`pinned_client`,
  using `reqwest` `resolve_to_addrs`). `reqwest` only accepts the DNS override at **build time**.
- **Fix:**
  1. `client.rs`: the `server_url` is fixed per `McpClient`, so validate it with
     `validate_outbound_url_pinned` and build a **host-pinned client once at construction**; store
     that instead of the unpinned `:577` client. Keep the SSE streaming path (`parse_sse_response`,
     `client.rs:990`) and session reuse working with the pinned client.
  2. `auth.rs`: endpoints are discovered dynamically (different hosts), so each builder needs its
     own per-call `validate_outbound_url_pinned` + `resolve_to_addrs` rebuild.
  3. Factor a shared pinned-client constructor in `thinclaw-tools` (promote `builtin/http.rs::pinned_client`)
     rather than duplicating the rebuild 5+ times.
- **Test:** a rebind between validation and send is caught (pinned addrs honored).
- **Docs/ledger:** FOLLOWUPS F-02; mention in `src/NETWORK_SECURITY.md` if you want the MCP path
  reflected in the egress model.

### F-03 — Dedupe MCP OAuth state compare onto `subtle` (TRIVIAL)

- **Status:** consistency item, not a live vuln. `auth.rs:816-830` hand-rolls a constant-time
  comparator (used at `:883`, tested at `:1217`); `src/cli/oauth_defaults.rs` uses
  `subtle::ConstantTimeEq`. `subtle` is **not** a dep of `crates/thinclaw-tools/Cargo.toml` (it is
  in the workspace root and in `thinclaw-channels`/`thinclaw-gateway` as `subtle = "2"`).
- **Fix:** add `subtle = "2"` to `crates/thinclaw-tools/Cargo.toml` `[dependencies]` (version-literal,
  matching the crate's convention — it does not use `.workspace = true`), then replace the
  `oauth_state_matches` body with `expected.as_bytes().ct_eq(received.as_bytes()).into()`; keep the
  existing test.

---

## LANE 2 — Experiments error taxonomy (serialize after F-01; same package)

### F-14 — Error-taxonomy normalization (split is DONE; FACTS corrected)

- **Status:** the structural god-file split already landed; only the error-taxonomy work remains.
- **CRITICAL CORRECTION — the variant inventory in the old plan is wrong.** `ApiError`
  (`src/api/error.rs:10-42`) has: `InvalidInput, SessionNotFound, Unavailable, FeatureDisabled,
  Agent(#[from]), Serialization(#[from]), UuidParse(#[from]), Internal`. **There is NO `NotFound`
  and NO `Conflict` variant.** Any instruction to "convert Internal→NotFound/Conflict" will not
  compile.
- **Count correction:** the literal `map_err(ApiError::Internal)` matches **1** site only; the real
  idiom is `map_err(|e| ApiError::Internal(e.to_string()))` / `ok_or_else(|| ApiError::Internal(...))`.
  Real surface ≈ **114-118 closure-form conversions / 128 total `ApiError::Internal` sites (excl.
  `tests.rs`)** across 7 submodules (crud 37, controller 24, execution 20, campaign 19, leases 16,
  git 8, subagents 4). **Search the closure form, not the fn-pointer form.**
- **The concrete consistency fix (small, ~7 sites):** the same "campaign worktree/branch is None"
  precondition raises `Internal` in some arms and `InvalidInput` in others. Normalize onto
  **`InvalidInput`** (matching the already-correct `campaign.rs:265` / `:667`). Convert:
  `campaign.rs:173`, `campaign.rs:177`, `leases.rs:288`, `execution.rs:418`, `subagents.rs:372`,
  `subagents.rs:413`, `subagents.rs:465`. Collapse the two helper messages
  (`…missing_worktree_path_field_message` vs `…has_no_worktree_message`) onto one. Leave the
  `campaign.rs:367` `CandidateGenerationError` path as-is.
- **The broader audit (larger):** reclassify the ~114 blanket `Internal` flattenings into the
  available taxonomy (`InvalidInput` for bad input, `SessionNotFound`/`Unavailable`/`FeatureDisabled`
  where they fit, `Internal` only for true server faults). If a genuine not-found/conflict semantic
  is needed, **first** add the variant in three coordinated places: the enum, `error_code()`
  (`error.rs:46`), and `From<ApiError> for GatewayApiError` (`error.rs:60`) — that is a precursor,
  not a one-liner. Scope the 7-site consistency fix separately from the 114-site sweep.

---

## LANE 3 — Repo projects, routines, observability

### F-06 — Concrete `SubagentRepoTaskPlanner` (+ `REPO_PROJECTS_AUTOPLAN` kill switch)

- **Status:** port shipped, adapter missing. `RepoTaskPlanner` trait + `PlannedTask` at
  `src/repo_projects/planner.rs:24/:48`; `with_planner(Option<Arc<dyn RepoTaskPlanner>>)` at
  `src/repo_projects/supervisor.rs:118` (field `:76`); `AwaitingHuman` fallback at
  `supervisor.rs:609-631/686-692`; wiring passes `with_planner(None)` at
  `src/agent/agent_loop.rs:997` (inside the `repo_projects_config.enabled` block `:983-1011`).
  `REPO_PROJECTS_AUTOPLAN` does **not** exist yet (greenfield).
- **Build `SubagentRepoTaskPlanner`:**
  - Takes `Arc<SubagentExecutor>`; if `self.deps.subagent_executor` is `None` (`agent_loop.rs:818`),
    leave `with_planner(None)`.
  - `SubagentExecutor::spawn` (`src/agent/subagent_executor.rs:298`) needs
    `channel_name / channel_metadata / parent_user_id / parent_identity / parent_thread_id` — the
    supervisor is headless, so **synthesize** them (e.g. channel `"repo-projects"`, empty metadata,
    owner `parent_user_id`, `None` identity/thread) and set `request.wait = true` for an inline result.
  - Request **strict JSON** planned tasks; parse into `Vec<PlannedTask>`, mapping each to a valid
    `repo_id` (the supervisor drops unknown-repo tasks at `supervisor.rs:644`).
  - **Fallbacks → `AwaitingHuman`:** missing executor, parse failure, or empty plan.
  - **Kill switch:** only construct the planner when `REPO_PROJECTS_AUTOPLAN=true`; otherwise keep
    `with_planner(None)` at `agent_loop.rs:997`.
- **Test:** new subagent **stub** driving success + each fallback (the existing `FakePlanner` at
  `pipeline_tests.rs:786` tests the port, not the adapter).
- **Docs:** `docs/RESEARCH_AND_EXPERIMENTS.md`/repo-projects surface as applicable + `FEATURE_PARITY.md`;
  `src/NETWORK_SECURITY.md` note (autonomous LLM spawning).

### F-07 — WebUI SSE consumer for repo-project events (LISTEN FOR THE REAL WIRE NAMES)

- **Status:** backend emits; frontend polls only. `crates/thinclaw-gateway/src/web/static/app.js`
  uses `EventSource` at `:818` with the debounce-on-SSE idiom to mirror at `:974-978`; the
  repo-projects tab (`loadRepoProjectsDashboard()` at `:2992`, `apiFetch('/api/repo-projects')` at
  `:6027`) refreshes only on tab-switch — **no `repo_*` SSE listener**.
- **Correction:** the **wire** SSE variants are five (`SseEvent`, `types.rs:1442-1469`,
  `event_type()` `:1536`): `repo_project_updated`, `repo_task_updated`, `repo_worker_run_updated`,
  `repo_project_event`, `repo_merge_gate_updated`. **`ProjectStateChanged` / `TaskCreated` named in
  the ledger are `RepoProjectEventKind` DB-record kinds, NOT SSE events** — do not listen for them.
- **Fix:** `addEventListener` for the five wire names → debounced `loadRepoProjectsDashboard()`
  guarded by `currentTab === 'repo-projects'` (mirror the cost/experiments idiom at `app.js:974-978`).
  Small, backend-independent.

### F-08 — Channel broadcast for light-context worker heartbeats (CROSS-CUTTING)

- **Status:** `WorkerDeps` (`src/agent/worker.rs:48-76`) has **no** `notify_tx`; `target=<channel>`
  today only *tags* the SSE summary string (`worker.rs:1023-1043`, format at `:1034`). The real
  channel-broadcast forwarder lives in the agent loop (`agent_loop.rs:880-918`), reading
  `response.metadata['notify_user']` (`:884`) and `['notify_channel']` (`:891`), and force-mirroring
  to `web`. `notify_tx` is created at `agent_loop.rs:797` and passed only into `RoutineEngine::new`
  (`:805`).
- **Fix:** thread the **same** `notify_tx: mpsc::Sender<OutgoingResponse>` into `WorkerDeps`; have the
  worker set `response.metadata` `notify_user` + `notify_channel` (matching `agent_loop.rs:884/891`)
  so the **existing** forwarder routes uniformly — do not write a second forwarder.
- **Scope:** `WorkerDeps` is constructed at ~7 sites (`worker.rs:1703/2000`,
  `dispatcher_helpers.rs:241/528/564`, `subagent_executor.rs:1705`, scheduler) — genuinely
  cross-cutting, not a single-field add.
- **Test:** worker heartbeat with `target=<channel>` actually broadcasts (and `target=none` still
  suppresses).
- **Docs:** `docs/SURFACES_AND_COMMANDS.md:21` (`/heartbeat`) — **note:** the WS-09 update was
  promised but never applied; finish it. Also `docs/CHANNEL_ARCHITECTURE.md` if delivery changes.

### F-04 — Shell-scanner health line in `thinclaw status`

- **Status:** `ShellTool::scanner_status()` exists (`crates/thinclaw-tools/src/builtin/shell.rs:278`,
  returns mode/available/fail_open/last_error/provenance). `run_status_command`
  (`src/cli/status.rs:12`) has no scanner line; it already loads `Settings`.
- **Fix (health tier, ~10 lines):** build a throwaway
  `ShellTool::new().with_safety_options(ShellSafetyOptions{ external_scanner_mode,
  external_scanner_path, external_scanner_require_verified })` from `settings.safety`
  (`crates/thinclaw-settings/src/safety.rs:39-48`), call `scanner_status()`, print
  mode + available + fail_open + last_error. (Cheap tier = print `settings.safety` fields directly
  for just mode/path.) No DB/registry needed.
- **Docs:** `docs/CLI_REFERENCE.md` (`thinclaw status`) — WS-01 omitted this pairing; add it.

### F-11 — Observability per-turn/per-tool events (small-but-real plumbing, NOT pure emission)

- **Status:** `create_observer` is wired (`src/app.rs:1721`), emits startup `AgentStart` (`:1722`),
  stored on `AppComponents.observer` (`:110`). `NoopObserver` default makes it safe to call
  anywhere unless `OBSERVABILITY_BACKEND=log`. **But the observer is a dead field past startup** —
  `AgentDeps` has no observer field; the agent loop/dispatcher never see it.
- **Fix:**
  1. Add `pub observer: Arc<dyn crate::observability::Observer>` to `AgentDeps` (`src/agent/agent_loop.rs:69`).
  2. Populate at **all 7 `AgentDeps{…}` literals**: production — `src/main.rs:1855`,
     `src/bin/thinclaw-acp.rs:138` and `:211`, and **the desktop embed
     `apps/desktop/backend/src/thinclaw/runtime_builder.rs:814`** (read `components.observer`);
     tests — `src/testing.rs:354`, `src/agent/dispatcher/test_support.rs:678`,
     `src/agent/dispatcher_helpers.rs:232` (default `Arc::new(NoopObserver)`).
  3. Emit (all sites are `impl Agent` methods reaching `self.deps.observer`, no new fn signatures):
     `LlmRequest`/`LlmResponse` around `dispatcher/llm_turn.rs:321/419/449` (model name from
     `current_llm().active_model_name()`, `Instant` for duration); `ToolCallStart`/`End` around the
     per-tool execute in `dispatcher/tool_execution.rs` (duration near `:832`); `TurnComplete` at the
     end of `execute_tool_calls_phase`.
- Keep behind the no-op default so `OBSERVABILITY_BACKEND=off` stays zero-overhead.
- **Docs/ledger:** FOLLOWUPS F-11; `FEATURE_PARITY.md` (observability now produces per-turn telemetry).

---

## LANE 4 — Platform completion

### F-09 — Shared WASM channel/tool SDK (NARROWER surface than the ledger; add a drift guard)

- **Status / corrected surface:**
  - **Channels (truly shareable):** `split_message` + nested `byte_index_for_char_limit` are
    byte-for-byte identical across all 4 custom channels (`discord:767/777`, `telegram:2174/2184`,
    `whatsapp:2096/2101`, `slack:1044/1054`). `json_response` is identical in **3** (discord/telegram/
    whatsapp); **slack differs** (logs the serde error) — reconcile or share the 3-way version.
  - **Do NOT share** `conversation_scope_id` / `external_conversation_key` — they have **divergent
    signatures** per channel and exist only in telegram (`:427/:491`) + whatsapp (`:473/:477`).
  - **Tools (truly shareable):** only `github`↔`notion` are byte-for-byte pairs
    (`url_encode_path` + `validate_input_length`/`MAX_TEXT_LENGTH=65536`: `github/src/lib.rs:26/38`,
    `notion/src/lib.rs:35/46`). The wider `url_encode` family has **genuine semantic drift** — gmail
    (`api.rs:450`, adds `~`), brave-search (`lib.rs:95`, adds `~` + space→`+` = query semantics), and
    a 5× google-* cluster (with `~`). **Reconcile path-vs-query semantics before consolidating** —
    do not lump all ~10 into one helper.
- **Mechanism:** `include!`-style shared source module mirroring `channels-src/shared_webhook_channel`
  (Option B / WS-03). Consumers are workspace-excluded (`Cargo.toml:3-20`) or fully standalone
  (notion, brave-search, the 12 shims are outside even the exclude list), so `include!` is the only
  viable mechanism; account for differing path-relative depths.
- **Minimum even if full extraction slips:** add a **drift-guard test** asserting the byte-boundary
  copies (`split_message`/`byte_index_for_char_limit`) are identical — this is a latent
  *correctness* risk (the last bug here was a multibyte-UTF-8 panic that landed in only one copy),
  not just maintainability.

### F-10 — Thin-shim dispositions (needs a SCHEMA field + a real sink, not just JSON edits)

- **Status / correction:** `production_status` exists in **zero** capabilities.json **and** is not a
  field on `ChannelCapabilitiesFile` (`crates/thinclaw-channels/src/wasm/schema.rs:53-83`), which has
  no `deny_unknown_fields` → adding the JSON key would **silently parse-and-drop**. `secret_validation`
  values match the classification doc exactly.
- **Dispositions (from `WS-03-shim-classification.md`):**
  - **Production (3 shims):** `line` (`hmac_sha256_base64_body`), `twitch`
    (`twitch_eventsub_hmac_sha256`), `twilio_sms` (`twilio_request_signature`). (discord is a custom
    channel, production via its own Ed25519 — not a shim.)
  - **Beta / "inbound auth = shared-secret `equals` only" (9 shims):** `dingtalk`, `feishu_lark`,
    `google_chat`, `matrix`, `mattermost`, `ms_teams`, `qq`, `wecom`, `weixin`.
- **Fix:**
  1. Add `production_status` (enum `production|beta|experimental`, `#[serde(default)]` defaulting to
     `beta`/`experimental`) to `ChannelCapabilitiesFile` in `schema.rs`.
  2. Wire **one real sink** (registry manifest / setup descriptor) so maturity surfaces to operators —
     none exists today; this is net-new, not a tweak.
  3. Set the 3/9 values across the 12 shim JSONs.
  4. Add a catalog-test assertion in `tests/registry_channel_catalog.rs` (e.g. no
     `secret_validation: equals` shim is marked `production`).
- **Honesty-control framing (treat as trust-boundary, not a doc chore):** the 9 beta shims advertise
  `require_secret` semantics they cannot enforce (DingTalk HMAC-timestamp, QQ Ed25519, Google/Teams
  signed JWT, etc.). Per-shim **README** must carry the precise auth caveat (matrix's
  `matrix_webhook_secret` is a ThinClaw route/proxy secret, **not** platform auth). **Optional code
  stretch:** promote `qq` to production by generalizing the Discord Ed25519 helper to an
  `Ed25519Body` host variant.
- **Residual test coverage (narrow):** deep mapping/response assertions in
  `registry_channel_catalog.rs` already cover `line/dingtalk/twilio_sms/twitch/feishu_lark/wecom/weixin`.
  **Extend deep assertions to the 5 uncovered shims:** `google_chat, matrix, mattermost, ms_teams, qq`
  (a `mapping.text`/response typo in those 5 can still ship). Do **not** re-add the CI-compile gate or
  the round-trip test — both already exist.
- **Docs:** `docs/CHANNEL_ARCHITECTURE.md` summary table + per-shim READMEs + `FEATURE_PARITY.md`.

### F-13 — Object-store `ArtifactStore` backend (net-new dep + reaper coordination + kill switch)

- **Status:** port + `LocalArtifactStore` at `src/experiments/artifact_store.rs:25-34/:41-113`
  (root, not `crates/thinclaw-experiments`). Upload path `src/api/experiments/leases.rs:188-198`;
  16 MiB inline cap `MAX_INLINE_ARTIFACT_BYTES` at `src/experiments/runner.rs:24`. The trait is
  **put-only**. `opendal` is **not** in the main workspace (it *is* in
  `apps/desktop/backend/Cargo.toml:104` — reuse the version/license precedent).
- **Fix:**
  1. Add `opendal` to root `Cargo.toml` (→ `cargo deny` must pass).
  2. Implement an S3/opendal `ArtifactStore` behind the existing port; default stays
     `LocalArtifactStore` (kill switch via config; **secret-name references only**, resolved through
     the same `SecretsStore` path as `research_runpod_*`/`research_vast_*` in
     `src/experiments/adapters.rs`).
  3. **Reaper coordination:** the daily retention reaper path-validates locators against the local
     root; an object-store backend returns a **URI**, so update the reaper to handle URI locators or
     it will silently no-op for cloud artifacts.
  4. Decide whether put-only suffices or the gateway needs a fetch/get path to serve `fetchable:true`
     URIs.
- **Test:** S3 round-trip against a mock/local backend; no secret values logged.
- **Docs:** extend the Operability section of `docs/RESEARCH_AND_EXPERIMENTS.md:115` + `FEATURE_PARITY.md`;
  egress note in `src/NETWORK_SECURITY.md`.

### F-15 — Skill parameters from `save_skill` (backward-compatible overload)

- **Status:** the Rhai builtin is registered as a 3-arg closure
  `move |id: String, script: String, description: String| -> String`
  (`apps/desktop/backend/src/rig_lib/sandbox_factory.rs:347-349`); `parameters: vec![]` at `:380`.
  `SkillManifest.parameters` exists (`#[serde(default)] Vec<SkillParameter>`,
  `apps/desktop/backend/scrappy-mcp-tools/src/skills/manifest.rs:19`); `SkillParameter` defined at
  `:3-10`. The read path already surfaces `parameters` (`sandbox_factory.rs:133`). Rhai 1.x supports
  arity/type overloading.
- **Fix:** register a second overload
  `save_skill(id, script, description, params)` (params = Rhai `Map` or JSON string → `Vec<SkillParameter>`)
  populating `SkillManifest.parameters`; keep the existing 3-arg signature valid. Update the
  orchestrator tool description (`rig_lib/orchestrator.rs`) to advertise the optional param.
- **Docs:** none required (the existing WS-04 note recommends just removing the misleading TODO; the
  skill-parameter vocabulary is internal — do not invent IDENTITY/SURFACES pairings).

### F-18 / F-19 — Voice wake: typed config + capture/transcribe/dispatch (RE-LOCATED; under-scoped before)

- **Status:** `VoiceWakeRuntime` (`src/voice_wake.rs:185-208`) is constructed under
  `#[cfg(feature="voice")]` + `THINCLAW_VOICE_WAKE` in `AppBuilder::build_all`
  (`src/app.rs:1648-1711`, env gate `:1650-1656`, `VoiceWakeConfig::default()` `:1659-1661`). The
  `WakeWordDetected` arm at `src/app.rs:1674-1687` **only logs**. No typed config exists in
  `crates/thinclaw-config` (the `voice_call_*` config there is the unrelated telephony feature).
  `WakeBackend::SherpaOnnx` is `#[allow(dead_code)]` (`voice_wake.rs:77`) with no selection path;
  default is `EnergyDetector` (`:64`).
- **CRITICAL CORRECTION — dispatch cannot be "threaded into the spawn block."** `build_all` returns
  `AppComponents` (`src/app.rs:69-111`), which has **no agent / session / dispatcher / inject handle**
  (only `mcp_session_manager`, unrelated). The real `SessionManager` (`main.rs:1118`), `Agent`
  (`main.rs:1886`), `ChannelManager` (`main.rs:635`), and `inject_sender` (`main.rs:1139`) are all
  built **after** `build_all()` returns (`main.rs:446`). Also, `capture_and_transcribe` **does not
  exist** — the logic is private inside `TalkModeTool::execute` (`src/talk_mode.rs:731-790`; per-OS
  `record_audio` at `:237/395/498`, `transcribe_whisper_http` `:612`, `transcribe_whisper_api` `:548`).
- **Re-scoped steps (ordered; config before dispatch):**
  1. **(small)** Promote `VoiceWakeConfig` into a typed `VoiceConfig` — add a serde
     `VoiceWakeSettings` to `crates/thinclaw-settings` (mirror `ChannelSettings` in `channels.rs`),
     build the runtime struct in `thinclaw-config` with the `parse_bool_env` overlay style, and
     construct it at `src/app.rs:1660` instead of `::default()`. The `backend` field is the
     `WakeBackend` enum, so serde-derive it (or map a string); keep everything behind
     `#[cfg(feature="voice")]`.
  2. **(medium)** Extract `pub async fn capture_and_transcribe(...)` from
     `src/talk_mode.rs:731-790` (lift the per-OS `record_audio` + WHISPER_HTTP/OPENAI backend
     selection out of the Tool impl so both the tool and the wake path share it; keep OS cfg-gating
     composing).
  3. **(medium)** **Relocate** the voice consumer out of `build_all` into `main.rs` (after channels
     at `:635`) — or return the started `VoiceWakeRuntime`/its receiver on `AppComponents` and run the
     consumer in `main.rs`. The `WakeWordDetected` arm then calls `capture_and_transcribe` and routes
     via `inject_tx.send(IncomingMessage::new(channel, user, transcript)).await` (mirror the
     subagent-result producer at `main.rs:1667-1681`; `IncomingMessage::new` at
     `crates/thinclaw-channels-core/src/channel.rs:34`, `inject_sender()` at
     `crates/thinclaw-channels/src/manager.rs:356`). Decide a synthetic channel/user/thread identity
     for voice-originated messages.
  4. **(future work — keep documented, do not implement)** true "hey thinclaw" keyword spotting needs
     an external `sherpa-onnx-keyword-spotter` binary + ~3 ONNX models + tokens/keywords files; and
     enabling `SherpaOnnx` later also needs a **backend selector** in the new `VoiceConfig` (the enum
     variant is dead-code today). Documented at `docs/BUILD_PROFILES.md:118-142`.
- **Test:** wake → `capture_and_transcribe` (stub STT) → `IncomingMessage` injected (dispatch seam
  exercised).
- **Docs:** `docs/BUILD_PROFILES.md` (**fix the `:121` self-contradiction**: it says voice "is part
  of `full`" while `:127`/`:276` and `Cargo.toml:296-309` say it is not), `docs/DEPLOYMENT.md:74`
  (libasound2-dev when enabling voice), `src/setup/README.md`, `FEATURE_PARITY.md`. Fix `FOLLOWUPS.md`
  F-18's "thread a handle into the spawn block" wording.

---

## LANE 5 — Docs & ledgers

### F-12 — Native-plugin safety docs (docs-only; EDIT, don't rewrite; gateway stays unexposed)

- **Status:** code model fully grounded and correct — default-off fail-closed before `dlopen`
  (`src/extensions/native_activation.rs:128-133`, `native.rs:77-79`), all gates before the single
  `Library::new` (`native.rs:71-123`, ed25519 + ABI + allowlist + SHA-256), `catch_unwind` isolation
  (`native_activation.rs:203`), in-process/full-host-privilege (`:5-7/170-171`). **Gateway is
  intentionally NOT exposed** (`crates/thinclaw-gateway/src/web/extensions.rs:46` and
  `src/api/extensions.rs:22-28` have only McpServer/WasmTool/WasmChannel) — keep it that way.
- **Remaining (narrow):**
  1. `docs/EXTENSION_SYSTEM.md` — **EDIT** the existing Native plugins section (`:55-66`), adding only
     `catch_unwind` panic-isolation and the "runs in-process with full host privilege / not
     WASM-sandboxed; signature gate + default-off + operator allowlist are the only controls" caveat.
     Do **not** recreate the section.
  2. `src/NETWORK_SECURITY.md` — **net-new and most load-bearing:** add native plugins as a fifth
     trust boundary in the Threat Model table (`:13-18`) + Key Assumptions (`:20-25`), explicitly
     operator-fully-trusted, in-process, **not sandboxed** (contrast with Docker/WASM).
  3. `FEATURE_PARITY.md` section 8 (`:345-372`) — add the native-plugin row (wired, default-off,
     operator-only, not gateway-exposed).
  4. Record explicitly that gateway exposure is intentionally NOT done, so a future reviewer does not
     "fix" the missing `NativePlugin` arm.

### Doc fixes the prior plan omitted (most pairings already exist in the per-WS docs)

- `docs/BUILD_PROFILES.md:121` voice/`full` contradiction — fix in any voice PR.
- `docs/CLI_REFERENCE.md` — add to F-04's pairing.
- `docs/SURFACES_AND_COMMANDS.md:21` `/heartbeat` — the WS-09 target/include_reasoning update was
  promised but never applied; finish it as part of F-08.
- Add a note to `FOLLOWUPS.md` that the deferred residuals inherit the same-PR canonical-doc rule
  when picked up.

---

## 6. Required final gates (corrected to match CI exactly)

```bash
cargo fmt --check
git diff --check

# Per-profile clippy — use --workspace (NOT -p thinclaw); mirror ci.yml:80-93 + :125
cargo clippy --workspace --all-targets -- -D warnings                                   # light/default
cargo clippy --workspace --all-targets --no-default-features --features edge -- -D warnings
cargo clippy --workspace --all-targets --no-default-features --features libsql -- -D warnings
cargo clippy --workspace --all-targets --no-default-features --features postgres -- -D warnings
cargo clippy --workspace --all-targets --no-default-features --features desktop -- -D warnings
cargo clippy --workspace --all-targets --features full -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings                    # == ci.yml:56

# Baseline guard (per Lane 0 decision): either the compile_error guard compiles under edge,
# OR — if you chose literal zero-feature — add and enforce:
#   cargo clippy --workspace --all-targets --no-default-features -- -D warnings
#   (and add the matching ci.yml matrix leg)

# Targeted suites
cargo test -p thinclaw-repo-projects
cargo test -p thinclaw --features desktop repo_projects --lib
cargo test -p thinclaw-db --features libsql repo_projects
SCHEMA_DIVERGENCE_STRICT=1 cargo test --features postgres,libsql schema_divergence   # Lane 0.3

cargo deny check
```

Also confirm the existing smoke jobs still pass where relevant: `host_runtime_smoke`
(`ci.yml:247`), ACP check (`ci.yml:262`), edge-dependency-guard (`ci.yml:213-229`), and the
`channel-crates` / `tool-crates` wasm build matrices (`ci.yml:735+`).

**Per-feature focused tests to add:** MCP pinning rebind test (F-02); experiments store-backed
credential resolution, no secret logging (F-01); planner success + each fallback with a subagent
stub (F-06); WebUI SSE reducer over the 5 wire names (F-07); worker heartbeat channel broadcast
(F-08); observer per-turn/tool/LLM/turn emission with NoopObserver (F-11); shim
`production_status` schema + 5-shim deep mapping assertions (F-10); S3 artifact round-trip on a
mock backend + reaper handling URI locators (F-13); `save_skill` 4-arg overload populates
`parameters` while 3-arg still works (F-15); voice wake → capture_and_transcribe (stub) →
IncomingMessage injection (F-18); byte-boundary helper drift guard (F-09).

---

## 7. Suggested commit lanes (order)

1. **Lane 0** — baseline decision + `compile_error!` guard (or zero-feature audit), corrected CI
   gates, schema_divergence strict, mark F-16/F-17/image_gen DONE, strike already-done shim gates.
2. **Lane 1** — F-03 (trivial), F-01 (minimal), F-02 (medium). *(Trust boundaries first.)*
3. **Lane 2** — F-14, **as the same PR as F-01** (or immediately after) since both edit
   `src/api/experiments/`.
4. **Lane 3** — F-04, F-11, F-08, F-06, F-07.
5. **Lane 4** — F-10 (before F-09), F-09, F-15, F-13, F-18/F-19.
6. **Lane 5** — F-12 docs + the omitted doc fixes + ledger close-out.

> **Assumptions locked for this pass:** `edge`/libSQL is the minimum-supported profile (zero-feature
> is *not* a product target — see Lane 0); native plugins stay operator-only (no gateway surface);
> voice ships typed config + STT dispatch but **no** packaged keyword model. New autonomy/egress
> capabilities (F-06, F-13) ship default-off with a documented kill switch. Do not delete or rewrite
> remediation history — close-out edits are append/checkbox only.
