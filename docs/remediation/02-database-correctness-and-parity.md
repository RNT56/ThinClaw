# WS-02 — Database Correctness & Backend Parity

> **Status:** ✅ Landed (2026-06-23), commit `4f88c43e` (Wave 0: security & CI hardening + DB correctness). All tasks shipped. This plan is complete; do not re-execute it.
> **Priority:** P0 · **Risk:** medium · **Effort:** M
> **Depends on:** none · **Blocks:** WS-13 (test-infra dual-backend CI gating consumes the assertions this WS adds)
> **Owns (symbols/files):**
> - `crates/thinclaw-db/src/libsql/conversations.rs` — `LibSqlBackend::search_conversation_messages` (FTS5 fix)
> - `crates/thinclaw-db/src/libsql/fts.rs` — **new** shared FTS5 sanitizer module (this WS creates it)
> - `crates/thinclaw-db/src/libsql/workspace.rs` — the inline sanitizer at lines 677-693 (refactor into the new module)
> - `tests/db_contract/conversations.rs` — conversation search contract assertions (punctuation regression cases)
> - `tests/schema_divergence.rs` — schema parity assertions (types/nullability/indexes + fail-not-skip)
> - `tests/schema_divergence_allowlist.json` — divergence allowlist schema
> - NOTE: `tests/db_contract/support.rs` is **shared** with WS-13 (test infra). Coordinate edits; see Decision Points / dependency note.

## Vision & Goal

ThinClaw ships two first-class database backends (Postgres for servers, libSQL for the desktop default) behind one `Database` trait. The product promise is that **the desktop default backend behaves identically to the server backend** — a transcript search that works on Postgres must not throw on libSQL. This workstream closes the one confirmed cross-backend correctness divergence (libSQL transcript search feeds raw user input to FTS5 `MATCH`, so any query containing `:`, `"`, or `-` errors where Postgres tolerates it via `websearch_to_tsquery`), proves no sibling divergences exist, and hardens the parity test suite so the next divergence is caught in CI rather than in a user's terminal.

## Scope

**In scope:**
- (1) Sanitize the libSQL transcript-search FTS5 `MATCH` input at `crates/thinclaw-db/src/libsql/conversations.rs:846` by reusing the existing tokenize-and-quote sanitizer at `crates/thinclaw-db/src/libsql/workspace.rs:677-693`, extracting it into a shared `libsql::fts` helper.
- (2) Cross-backend parity audit of every raw-user-input → text-search query site in both backends (completed during planning — see Current State; tasks below act on the result).
- (3) Strengthen `tests/schema_divergence.rs` beyond column-name presence: compare column **types**, **nullability**, and **indexes**; make the test **FAIL (not skip)** when `DATABASE_URL` is absent.
- (4) Parameterize `tests/db_contract` conversation-search assertions so punctuation queries are exercised on **both** Postgres and libSQL (the harness is already env-driven per backend; this WS adds the backend-agnostic assertions).

**Out of scope (and owning WS):**
- The CI **job** that executes these tests against a live Postgres + libSQL matrix (the `db-contract-*` and `schema-divergence` jobs in `.github/workflows/ci.yml:640-723`) is owned by **WS-13 (test infra)**. This WS owns the *assertions*; WS-13 owns the *execution/gating*. Do not edit `ci.yml` here — file the wiring requirement as a dependency for WS-13.
- The empty-`gateway_auth_token` auth bypass (finding #1) — **WS-01 (security/gateway)**.
- The wasmtime-wasi RUSTSEC bump and `deny.toml` (finding #2) — **WS-01**.
- Sandbox proxy credential confinement (findings #6/#7) — **WS-01 (sandbox/secrets)**.
- Desktop cloud-sync wiring (finding #8) — **WS-04 (desktop)**.
- The `src/history/store/` vs `crates/thinclaw-db/src/postgres_store/` near-duplicate migration — **WS-10 (crate migration / dedup)**; do not touch it here even though it is DB-adjacent.

## Current State (verified)

- **WIRED (correct, reference pattern):** `crates/thinclaw-db/src/libsql/workspace.rs:677-693` — `hybrid_search` sanitizes the FTS5 query by splitting on non-`[alphanumeric_]` chars and quoting each token (`"time" "sensitive" "notes"`), or OR-joining quoted morphological keywords from `expand_query_keywords`. Its comment explicitly names "hyphens, colons, etc." This is the pattern to reuse. The `MATCH` is at line 706 and only ever receives the sanitized string.
- **WIRED (correct, no fix needed):** `crates/thinclaw-db/src/postgres_store/conversation_queries.rs:459-510` — Postgres `search_conversation_messages` uses `websearch_to_tsquery('simple', $2)` (lines 492, 498), which is the punctuation-tolerant Postgres parser. Raw user input is fine here.
- **WIRED (correct, no fix needed):** `crates/thinclaw-db/src/postgres_workspace.rs:532-559` — Postgres workspace `fts_search` uses `plainto_tsquery('english', $3)` (lines 545, 550); `plainto_tsquery` also tolerates punctuation. The expanded query is OR-joined with `|` upstream at line 465 but Postgres parses it safely.
- **FIXED (was the target):** `crates/thinclaw-db/src/libsql/conversations/mod.rs` — `LibSqlBackend::search_conversation_messages` previously bound the trimmed-but-otherwise-raw `query` directly to `conversation_messages_fts MATCH ?1`, so `foo:bar`, `"unterminated`, or `re-enable` threw `DatabaseError::Query` where Postgres tolerated them. Now it computes `let match_query = super::fts::sanitize_fts5_match(query);` (`conversations/mod.rs:574`) and binds the sanitized value. This was the sole unsanitized raw-input → FTS5 `MATCH` site; it is closed. (The file also moved from `libsql/conversations.rs` to `libsql/conversations/mod.rs` during the god-file decomposition.)
- **PARITY AUDIT RESULT (item 2 — complete):** grep of both backends for `MATCH` / `tsquery` / `LIKE` text-search construction yields exactly four query sites plus one LIKE:
  - `libsql/conversations.rs:846` → **unsanitized FTS5 MATCH (the bug).**
  - `libsql/workspace.rs:706` → sanitized FTS5 MATCH (correct).
  - `postgres_store/conversation_queries.rs:492,498` → `websearch_to_tsquery` (correct).
  - `postgres_workspace.rs:545,550` → `plainto_tsquery` (correct).
  - `libsql/workspace.rs:273` → `path LIKE ?3` is a **parameterized** `LIKE` over a path, not FTS5 `MATCH`; it cannot throw on punctuation and is not a divergence. No action.
  - **Conclusion: no sibling FTS5-MATCH divergence. The fix lands in exactly one place.** (This directly avoids the "fix landing in only one of N copies" trap — there is genuinely only one copy to fix, now confirmed.)
- **FIXED (item 3 — schema_divergence):** `tests/schema_divergence.rs` previously compared only column-name sets per table and skipped (returned Ok) when `DATABASE_URL` was absent, so a missing DB silently passed. Now the snapshot model is `columns: BTreeMap<String, ColumnInfo { normalized_type, not_null }>` plus `indexes: BTreeSet<IndexInfo { columns, unique }>` (`schema_divergence.rs:43-71`), comparing normalized types, nullability, and indexes; and `DATABASE_URL` is read via `.expect(...)` (`:126`) so a missing URL panics (hard failure) rather than skipping. The `SchemaAllowlist` carries `ignore_types`/`ignore_nullability`/`ignore_indexes`/`allowed_exact` so intended backend differences stay green.
- **ALREADY PARAMETERIZED OVER BACKENDS (item 4 — partial):** `tests/db_contract/support.rs:22-40` selects the backend from `DATABASE_BACKEND` env (defaults to libsql if compiled with the feature, else postgres) and returns a single `Arc<dyn Database>`. CI runs the suite **twice** — `db-contract-libsql` (`ci.yml:640-654`) and `db-contract-postgres` (`ci.yml:656-689`) — each setting `DATABASE_BACKEND`. So the *suite* already runs on both backends; the missing **search assertion that exercises punctuation** has since been added — `conversation_search_tolerates_punctuation_contract` (`tests/db_contract/conversations.rs:129`) now runs on both backends via the per-`DATABASE_BACKEND` jobs. The `db_contract` tests still **`return` (skip)** when no DB is available — same fail-vs-skip shape as the old schema_divergence, but item 3 deliberately only flipped schema_divergence (Decision 4).

## Decision Points

1. **WIRE vs ERASE the libSQL sanitizer — and where it lives.**
   - Options: (a) copy the 677-693 sanitizer block inline into `conversations.rs`; (b) **extract it once** into a new `crates/thinclaw-db/src/libsql/fts.rs` module with `pub(super) fn sanitize_fts5_match(query: &str) -> String`, call it from both `conversations.rs` and `workspace.rs`.
   - Trade-offs: (a) is faster but re-creates the exact "two copies, fix drifts" anti-pattern the audit warns about (§5 cross-channel drift). (b) honors CLAUDE.md ("extract a cohesive submodule before adding more behavior", "preserve public paths with `pub use` re-exports", "narrow visibility `pub(super)`") and gives one tested code path.
   - **Recommendation: (b) — extract.** Domain-named module `fts` (not a vague `util`/`common` bucket), `pub(super)` visibility, re-export through `mod.rs`'s existing submodule list. This is realizing the vision (one correct shared primitive) rather than papering over it.

2. **Should `conversation_messages_fts` search adopt `expand_query_keywords` like workspace does?**
   - Options: (a) port the full workspace sanitizer including the `expand_query_keywords` morphological-OR branch; (b) port only the quote-each-token branch (the `keywords.is_empty()` arm at lines 680-686).
   - Trade-offs: (a) changes transcript-search *recall semantics* (adds stemming/OR expansion) — a behavior change beyond the bug fix, and Postgres uses `websearch_to_tsquery` not keyword expansion, so (a) would *introduce* a new cross-backend divergence in ranking. (b) fixes the throw without changing recall semantics and keeps both backends' transcript search behaving like a phrase/token search.
   - **Recommendation: (b).** The shared `sanitize_fts5_match` should be the *quote-each-token-only* form (no `expand_query_keywords`). Keep keyword expansion as a separate concern that `workspace.rs` layers on top before calling the sanitizer, so workspace recall is unchanged and transcript search gains only safety. (Implementation detail in T1: extract the quoting core; `workspace.rs` keeps its keyword-expansion branch and feeds the result through the shared quoter, or keeps its own join — see T1 Acceptance.)

3. **schema_divergence: fail-not-skip on missing `DATABASE_URL` (build vs gate).**
   - Options: (a) `panic!` immediately when `DATABASE_URL` is unset; (b) keep the early-skip.
   - Trade-offs: (a) is what finding §4/P1 asks for and what makes the test a real gate; risk is that *local* `cargo test` (no DB) now fails loudly. (b) is the status quo that let the test rot. The test is already `#![cfg(all(feature = "postgres", feature = "libsql"))]` (line 1), so it only compiles in the dual-feature build CI uses; a plain local `cargo test` does not even build it.
   - **Recommendation: (a) — fail.** Because the cfg-gate already prevents it from running in single-feature local builds, the only place it runs is the CI `schema-divergence` job which always has `DATABASE_URL`. Failing there if the DB is missing is correct. Mirror this in the planning note to WS-13 so they know the job must always provision the DB.

4. **db_contract fail-vs-skip (out of asked scope, flag only).** Item (4) asks to *parameterize*, not to flip db_contract from skip to fail. The harness already skips per `contract_db_or_skip`. **Recommendation: leave db_contract as skip-on-missing-DB; only schema_divergence flips to fail** (item 3). Flipping db_contract too would break every developer's local `cargo test --features libsql,postgres` run with no local Postgres. If WS-13 wants a hard gate, it belongs in the CI job, not the test body.

## Tasks

- [x] **T1: Extract a shared libSQL FTS5 sanitizer and apply it to transcript search.**
  - **Files:** create `crates/thinclaw-db/src/libsql/fts.rs`; edit `crates/thinclaw-db/src/libsql/mod.rs` (add `mod fts;` to the submodule list, lines 9-19); edit `crates/thinclaw-db/src/libsql/conversations.rs` (`search_conversation_messages`, ~line 820-854); edit `crates/thinclaw-db/src/libsql/workspace.rs` (refactor inline sanitizer at 677-693 to call the shared fn).
  - **Change:**
    - New `fts.rs` with `pub(super) fn sanitize_fts5_match(query: &str) -> String` that reproduces the quote-each-token branch from `workspace.rs:680-686`: split on `|c: char| !c.is_alphanumeric() && c != '_'`, drop empties, wrap each token in double quotes, join with a space. Returns `String::new()` when no tokens survive. Add a module doc comment naming the hazard (FTS5 treats `-`, `:`, `"`, `*`, `^`, `(`, `)`, `AND/OR/NOT` as operators). Add `#[cfg(test)] mod tests` with unit cases: `foo:bar` → `"foo" "bar"`, `re-enable` → `"re" "enable"`, `"unterminated` → `"unterminated"`, empty/whitespace → `""`, `hello_world` → `"hello_world"`.
    - In `conversations.rs::search_conversation_messages`, after the existing `let query = query.trim();` / empty-guard (lines 820-823), compute `let match_query = super::fts::sanitize_fts5_match(query);` and early-return `Ok(Vec::new())` if it is empty; bind `match_query` (not raw `query`) to `?1` in the `params![...]` at line 854.
    - In `workspace.rs`, replace the inline `keywords.is_empty()` quoting arm (lines 680-686) with a call to `super::fts::sanitize_fts5_match(query)`; keep the `else` keyword-OR branch as-is (per Decision Point 2 this is workspace-only recall behavior). Confirm the resulting `sanitized_query` still equals the previous output for the non-keyword path.
  - **Acceptance:** `conversations.rs` no longer binds raw `query` to `MATCH`; both call sites import the same `super::fts::sanitize_fts5_match`; `workspace.rs` non-keyword path output is byte-identical to before; no inline quoting logic remains duplicated. New unit tests pass under `--features libsql`.
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw-db --no-default-features --features libsql fts` (unit tests); `cargo clippy -p thinclaw-db --no-default-features --features libsql --all-targets -- -D warnings`.

- [x] **T2: Add a punctuation-query regression test to the db_contract conversation suite.**
  - **Files:** `tests/db_contract/conversations.rs` (new `#[tokio::test]`, alongside `conversation_message_flow_contract` at line 49).
  - **Change:** add `async fn conversation_search_tolerates_punctuation_contract()` that: skips via `contract_db_or_skip()` (matching the existing pattern at line 51); creates a conversation; inserts messages whose content contains tokens that would be FTS5-hostile if searched raw (e.g. body `"re-enable the time:sensitive feature"`); then calls `search_conversation_messages(&user, q, ..)` for each of these queries: `"re-enable"`, `"time:sensitive"`, `"\"quoted"`, `"foo AND bar"`. For each, assert the call returns `Ok` (does **not** error) — i.e. `.expect("punctuation query must not error")`. At least the `re-enable`/`time:sensitive` cases should also return ≥1 hit (the tokens exist in the body). This test runs on **both** backends because CI invokes the suite once per `DATABASE_BACKEND` (libsql + postgres jobs at `ci.yml:640-689`), satisfying item (4) for the search path. Before T1, the libsql run of this test fails (proving the bug); after T1 it passes; the postgres run passes throughout (proving parity).
  - **Acceptance:** new test exists; on a libSQL DB it fails pre-T1 and passes post-T1; on Postgres it passes both pre and post. The four punctuation queries each return `Ok`.
  - **Effort:** S
  - **Verification (libSQL, no external DB needed):** `DATABASE_BACKEND=libsql cargo test --test db_contract --no-default-features --features libsql conversation_search_tolerates_punctuation -- --nocapture`. **Verification (Postgres, needs DB):** `DATABASE_BACKEND=postgres DATABASE_URL=postgres://thinclaw:thinclaw@localhost:5432/thinclaw_test cargo test --test db_contract --no-default-features --features postgres conversation_search_tolerates_punctuation -- --nocapture --test-threads=1` (requires a local `pgvector/pgvector:pg17` Postgres with migrations applied — see CLAUDE.md local-dev Postgres note).

- [x] **T3: Strengthen schema_divergence to compare types, nullability, and indexes.**
  - **Files:** `tests/schema_divergence.rs`; `tests/schema_divergence_allowlist.json` (extend schema as needed).
  - **Change:**
    - Replace `SchemaSnapshot { tables: BTreeMap<String, BTreeSet<String>> }` (lines 24-27) with a richer column model, e.g. `BTreeMap<String /*table*/, BTreeMap<String /*column*/, ColumnInfo>>` where `ColumnInfo { normalized_type: String, not_null: bool }`, plus a per-table `BTreeSet<IndexInfo>` (`{ columns: Vec<String>, unique: bool }`).
    - Postgres snapshot (`snapshot_postgres_schema`, lines 168-214): extend the `information_schema.columns` query to also select `data_type`/`udt_name` and `is_nullable`; query `pg_indexes` / `pg_index` (joined to `pg_attribute`) for index column lists and uniqueness in `current_schema()`.
    - libSQL snapshot (`snapshot_libsql_schema`, lines 216-269): `PRAGMA table_info('<t>')` already returns `type` (col 2) and `notnull` (col 3) and `pk` (col 5) — read those; use `PRAGMA index_list('<t>')` + `PRAGMA index_info('<idx>')` for indexes.
    - Add a **type-normalization map** (Postgres↔SQLite affinity is intentionally loose — e.g. PG `text`/`character varying` ≈ SQLite `TEXT`, PG `bigint`/`integer` ≈ SQLite `INTEGER`, PG `timestamptz` ≈ SQLite `TEXT`/`INTEGER`, PG `jsonb` ≈ SQLite `TEXT`, PG `uuid` ≈ SQLite `TEXT`, PG `numeric` ≈ SQLite `TEXT`/`REAL`). Compare on normalized class, not raw type string, to avoid a flood of false positives. Differences that survive normalization, plus nullability and index mismatches, become diff entries (e.g. `type_mismatch:<table>:<col>:pg=<x>,libsql=<y>`, `nullability_mismatch:...`, `missing_index:<backend>:<table>:<cols>`).
    - Extend `SchemaAllowlist` (lines 14-22) and `schema_divergence_allowlist.json` with `ignore_types`, `ignore_nullability`, `ignore_indexes`, and `allowed_exact` entries so genuinely-intended backend differences (the existing column allowances) can be recorded with a comment-style key.
    - **Seed the allowlist by running once and recording the current (intended) divergences** so the strengthened test starts green, then tighten over time. Do not let it fail on day one for pre-existing accepted differences.
  - **Acceptance:** test compares names + normalized types + nullability + indexes; pre-existing intended differences are allowlisted so the test passes on current `main`; an injected type/nullability/index mismatch (verify by temporarily editing a migration locally) is detected.
  - **Effort:** L
  - **Verification:** `DATABASE_URL=postgres://thinclaw:thinclaw@localhost:5432/thinclaw_test cargo test --test schema_divergence --no-default-features --features "postgres libsql" -- --nocapture --test-threads=1` (needs live Postgres + migrations). Also `cargo clippy --test schema_divergence --no-default-features --features "postgres libsql" --all-targets -- -D warnings`.

- [x] **T4: Make schema_divergence FAIL (not skip) when DATABASE_URL is absent.**
  - **Files:** `tests/schema_divergence.rs` (lines 35-38, and the connect/migrate `eprintln!+return` arms at 41-72).
  - **Change:** replace the `let Some(base_url) = std::env::var("DATABASE_URL").ok() else { eprintln!(...); return; }` skip (lines 35-38) with `let base_url = std::env::var("DATABASE_URL").expect("schema_divergence requires DATABASE_URL; this test is gated behind the postgres+libsql features and only runs in the schema-divergence CI job, which always provisions Postgres");`. Convert the schema-create / url-build / connect / migrate `eprintln!("skipping...") + return` arms (41-72) into `.expect(...)`/`panic!` so an unreachable/broken DB is a hard failure, not a silent pass. Keep the `#![cfg(all(feature = "postgres", feature = "libsql"))]` gate (line 1) — that is what stops it from breaking single-feature local builds.
  - **Acceptance:** running the test with `DATABASE_URL` unset (but with both features) panics with a clear message instead of returning Ok; running it with a healthy DB still passes.
  - **Effort:** S
  - **Verification:** `cargo test --test schema_divergence --no-default-features --features "postgres libsql"` with `DATABASE_URL` **unset** → expect a test FAILURE (panic). Then set `DATABASE_URL` to a live DB → expect PASS.

- [x] **T5: Record the dual-backend CI-gating requirement for WS-13 (no ci.yml edits here).**
  - **Files:** none in this WS (do not edit `.github/workflows/ci.yml`). Capture as a hand-off note in the WS-13 doc / execution playbook.
  - **Change:** document for WS-13: (a) `schema-divergence` job (`ci.yml:691-723`) must keep `DATABASE_URL` always set — T4 makes a missing URL a hard failure, which is now the desired gate; (b) the `db-contract-libsql` and `db-contract-postgres` jobs (`ci.yml:640-689`) already cover the new punctuation regression test from T2 because they invoke the whole `db_contract` target per backend — no new job needed, but WS-13 should confirm both jobs stay required-for-merge so the parity assertion can't be bypassed.
  - **Acceptance:** WS-13 acknowledges the gating note; no `ci.yml` change is attributed to WS-02.
  - **Effort:** S
  - **Verification:** cross-reference exists in WS-13's plan; `git blame ci.yml` shows no WS-02 edits.

## Best Practices (workstream-specific)

- **Reuse the proven sanitizer, don't reinvent it.** The canonical good example is `crates/thinclaw-db/src/libsql/workspace.rs:677-693` — its comment already documents *why* (FTS5 operator chars). Extract that exact logic; do not write a new escaping scheme.
- **Backend-specific SQL belongs in backend-specific query construction, behind the same `Database` trait method.** Postgres uses `websearch_to_tsquery`/`plainto_tsquery` (parser handles punctuation); libSQL must pre-sanitize because raw FTS5 `MATCH` has no tolerant parser. Both ends must accept the same user input without erroring — that is the parity contract this WS enforces.
- **Honor the libsql module façade.** `crates/thinclaw-db/src/libsql/mod.rs` is the façade: it declares submodules (lines 9-19) and exports shared helpers (`fmt_ts`, `get_text`, etc.) that submodules pull via `use super::{...}` (see `conversations.rs:8`). The new `fts` module follows that exact convention — `mod fts;` in the façade, `pub(super) fn` visibility, `super::fts::sanitize_fts5_match` at call sites. Do not widen to `pub`.
- **Loose type comparison for schema parity.** Postgres and SQLite type systems differ by design (SQLite affinity). The schema_divergence type check must normalize to affinity classes and allowlist intended differences, or it will be a false-positive generator. Model the allowlist after the existing `SchemaAllowlist` JSON (`tests/schema_divergence_allowlist.json`) — additive `ignore_*` lists, not code edits.
- **Feature matrix:** all changes touch only the `libsql` and `postgres` features of `thinclaw-db` (`crates/thinclaw-db/Cargo.toml:50-54,39-49`). The sanitizer fix is `libsql`-only code; schema_divergence requires *both* features (its `#![cfg(all(...))]` gate). No edge/light/desktop/full profile shifts — the desktop default already enables `libsql`, so this hardens the desktop path specifically.

## Common Pitfalls

- **Fix-lands-in-only-one-copy (the audit's recurring trap, §5/§10).** The whole reason to extract `fts.rs` (T1) is so the next FTS5 change can't drift between `conversations.rs` and `workspace.rs`. We confirmed during planning there is exactly **one** unsanitized site today — do not "helpfully" also rewrite the Postgres paths; they are correct and rewriting them risks introducing a *new* divergence (Decision Point 2).
- **Changing recall semantics while fixing the throw.** Do **not** wire `expand_query_keywords` into transcript search. Postgres transcript search uses `websearch_to_tsquery` (no stemming-OR); adding keyword expansion only to libSQL would make ranking diverge — replacing a throw-divergence with a results-divergence. Keep the shared sanitizer to quote-only.
- **Empty-after-sanitization queries.** A query of pure punctuation (e.g. `:::`) sanitizes to `""`. FTS5 `MATCH ''` errors. The shared fn must return empty and the caller must early-return `Ok(Vec::new())` (mirroring `workspace.rs:695-697`). Don't bind an empty match string.
- **schema_divergence false-positive flood.** If T3 compares raw type strings, every `text` vs `TEXT` and `timestamptz` vs `TEXT` will diff. Normalize first, then allowlist the genuinely-intended residue; seed the allowlist from a real run so the strengthened test starts green.
- **Flipping the wrong test to fail-not-skip.** Item (3) targets `schema_divergence` only. Do **not** also make `db_contract`'s `contract_db_or_skip` panic — that would break local `cargo test` for any developer without a local Postgres (Decision Point 4). The hard-gate for db_contract lives in CI (WS-13), not the test body.
- **Touching ci.yml.** The CI jobs are WS-13's. Editing `ci.yml` here creates a merge-ownership conflict. Hand the requirement off via T5.
- **libSQL `params!` macro binding.** In `conversations.rs:854` the bind list is positional (`params![query, user_id, ...]` → `?1, ?2, ...`). Swap the *value* (`query` → `match_query`), not the position. Keep the trim/empty guard above it intact.

## Multi-Worker Execution Plan (ultracode)

- **Worker decomposition:**
  - **Worker A (product fix):** T1 + T2. Self-contained in `crates/thinclaw-db/src/libsql/*` + `tests/db_contract/conversations.rs`. Can be verified entirely with the libSQL feature and no external DB (the libSQL leg of T2 needs only a temp-file DB).
  - **Worker B (test hardening):** T3 + T4 in `tests/schema_divergence.rs` + allowlist JSON. Independent file set from Worker A.
  - **Sequential tail:** T5 (hand-off note) after A and B land, so the note references final test names/line anchors.
- **Isolation:** Worker A and Worker B touch disjoint files (libsql crate + db_contract test vs schema_divergence test). They can run in **parallel git worktrees** without conflict. The only shared file in this WS's surface is `tests/db_contract/support.rs`, which **neither task edits** (T2 reuses `contract_db_or_skip` as-is) — so no intra-WS contention. Use one worktree per worker to allow parallel `cargo test` runs against different feature sets.
- **Workflow shape:**
  1. **implement** (fan-out A‖B): A writes `fts.rs` + edits two call sites + adds the punctuation test; B rewrites the schema snapshot model + allowlist + fail-not-skip.
  2. **verify** (per worker): A runs the libSQL gate locally (no DB); B runs schema_divergence against a local `pgvector/pgvector:pg17` with migrations applied (per CLAUDE.md Docker/Postgres note — apply `migrations/V*.sql` into `thinclaw_test` first).
  3. **review** (`/code-review` on the combined diff): focus on Decision Point 2 (no recall-semantics change), empty-query handling, and type-normalization false-positive risk.
  4. **fix** (fold review findings), then T5.
- **Verification gate (exact commands):**
  - `cargo fmt --all`
  - `cargo clippy -p thinclaw-db --no-default-features --features libsql --all-targets -- -D warnings`
  - `cargo clippy --test schema_divergence --no-default-features --features "postgres libsql" --all-targets -- -D warnings`
  - `cargo test -p thinclaw-db --no-default-features --features libsql fts` (sanitizer unit tests)
  - `DATABASE_BACKEND=libsql cargo test --test db_contract --no-default-features --features libsql conversation_search_tolerates_punctuation -- --nocapture` (Worker A, no DB)
  - **DB-required (Worker B + Postgres leg of A):** local `pgvector/pgvector:pg17` container, migrations applied, then:
    - `DATABASE_BACKEND=postgres DATABASE_URL=postgres://thinclaw:thinclaw@localhost:5432/thinclaw_test cargo test --test db_contract --no-default-features --features postgres conversation_search_tolerates_punctuation -- --nocapture --test-threads=1`
    - `DATABASE_URL=postgres://thinclaw:thinclaw@localhost:5432/thinclaw_test cargo test --test schema_divergence --no-default-features --features "postgres libsql" -- --nocapture --test-threads=1`
    - Negative check for T4: run schema_divergence with `DATABASE_URL` **unset** → must FAIL.
  - `/ship` (full Rust quality gate) before opening the PR; `/code-review` at `high` on the diff.
  - **Docker prerequisite:** if Docker is unhealthy, follow the CLAUDE.md recovery note (check `df -h /System/Volumes/Data`, clear `target*`, restart Docker Desktop) before assuming a product failure.

## Definition of Done

- [x] `crates/thinclaw-db/src/libsql/fts.rs` exists with `pub(super) fn sanitize_fts5_match` + unit tests; declared in `mod.rs`.
- [x] `libsql/conversations.rs::search_conversation_messages` binds the sanitized match string, never raw user input, and early-returns `Ok(vec![])` on empty-after-sanitize.
- [x] `libsql/workspace.rs` non-keyword path now calls the shared sanitizer; its output is unchanged for that path; no duplicated quoting logic remains.
- [x] `tests/db_contract/conversations.rs` has a punctuation-tolerance test that fails on libSQL pre-fix and passes post-fix, and passes on Postgres throughout (proving parity); it runs on both backends via the existing per-`DATABASE_BACKEND` CI jobs.
- [x] Parity audit conclusion recorded: the single FTS5-MATCH divergence is fixed; no sibling site exists (Postgres uses tolerant `*_tsquery`, libSQL workspace already sanitized, the lone libSQL `LIKE` is parameterized).
- [x] `tests/schema_divergence.rs` compares column names **+ normalized types + nullability + indexes**; allowlist extended and seeded so it passes on current `main`; an injected mismatch is detected.
- [x] `tests/schema_divergence.rs` **panics (fails)** when `DATABASE_URL` is absent under the dual-feature build, instead of silently returning Ok.
- [x] Verification gate green: `cargo fmt`, both `clippy --all-targets -D warnings` invocations, libSQL unit + contract tests, and (with a live DB) the Postgres contract + schema_divergence tests.
- [x] No `.github/workflows/ci.yml` edits attributed to WS-02; dual-backend gating requirement handed to WS-13 (T5).
- [x] Decision Points 1-4 resolved as recommended (extract sanitizer; quote-only no keyword expansion; schema_divergence fail-not-skip; db_contract stays skip).
