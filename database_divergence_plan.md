# Database Backend Divergence Resolution Plan

> **Status:** Planning  
> **Updated:** 2026-04-14  
> **Target:** `src/db/`, `tests/`, `.github/workflows/ci.yml`, `docs/`  
> **Primary Goal:** make backend divergence visible, testable, and CI-enforced across PostgreSQL and libSQL  
> **Secondary Goal:** leave behind a coverage ledger so future DB changes cannot silently bypass parity checks

---

## 1. Scope

This plan covers four deliverables:

1. A shared database contract test suite that runs the same behavior checks against both backends.
2. A complete coverage ledger mapping every `Database`-surface method to either:
   - a backend contract test,
   - an adapter/default-method test, or
   - an explicit "intentionally backend-specific" note.
3. A schema divergence checker that compares normalized live schemas, not raw SQL text.
4. CI jobs that run the contract suite and schema checker in backend-specific configurations.

This plan does **not** include a large refactor of `src/db/postgres.rs`. That remains optional follow-up work after parity coverage is in place.

---

## 2. Current Code Reality

### 2.1 Trait Surface

The current DB abstraction is larger than the original estimate implied:

- `Database` composes **11 backend-owned traits**:
  - `ConversationStore`
  - `JobStore`
  - `SandboxStore`
  - `RoutineStore`
  - `IdentityRegistryStore`
  - `ToolFailureStore`
  - `ExperimentStore`
  - `SettingsStore`
  - `WorkspaceStore`
  - `AgentRegistryStore`
  - plus the compatibility-facing `IdentityStore`
- `IdentityStore` is not an independent backend implementation; it is a blanket adapter layered on top of `IdentityRegistryStore`.
- Several traits also contain **default helper methods** that should be tested separately from backend SQL behavior:
  - `SandboxStore`
  - `RoutineStore`
  - `WorkspaceStore`

### 2.2 Existing Test and Helper Reality

- `postgres.rs` currently has no local unit tests.
- libSQL has a small number of focused tests spread across `src/db/libsql/mod.rs`, `identity.rs`, `agent_registry.rs`, and `libsql_migrations.rs`.
- `src/testing.rs` already contains a libSQL test helper (`test_db()`), and `LibSqlBackend::new_memory()` already exists.
- The repo currently has no shared backend-agnostic DB contract suite.

### 2.3 Existing CI Reality

Current CI does **not** yet provide the needed parity signal:

- `feature-matrix` only compiles tests for different feature sets; it does not run backend-specific DB contracts.
- `tests` runs once under `--all-features` with `DATABASE_BACKEND=postgres`.
- There is no dedicated libSQL contract-test job.
- There is no schema divergence checker.

### 2.4 Migration Reality

The two schema sources are structurally different:

- PostgreSQL schema comes from `migrations/V*.sql`.
- libSQL schema comes from:
  - `SCHEMA`
  - `UPGRADES`
  - `DATA_REPAIRS`
  in `src/db/libsql_migrations.rs`.

That means a raw SQL diff is not reliable enough. The correct comparison is **normalized live schema metadata plus targeted migration-repair checks**.

---

## 3. Success Criteria

This work is complete only when all of the following are true:

- Every `Database`-surface method is accounted for in a coverage ledger.
- Every backend-owned contract is exercised against both backends.
- Adapter/default-method behavior is tested separately where needed.
- PostgreSQL and libSQL each have a dedicated CI contract-test path.
- A schema divergence checker fails CI on unclassified semantic drift.
- Intentional schema differences are documented in one human-readable file and one machine-readable allowlist.

Nice-to-have but not required for this plan:

- decomposing `src/db/postgres.rs`
- improving test runtime through parallel Postgres isolation after the first stable version lands

---

## 4. Deliverables

### 4.1 Test Files

Create a dedicated integration test target instead of adding test modules inside `src/db/`:

```text
tests/
├── db_contract.rs
├── db_contract/
│   ├── mod.rs
│   ├── support.rs
│   ├── fixtures.rs
│   ├── coverage_ledger.rs
│   ├── conversations.rs
│   ├── identity.rs
│   ├── jobs.rs
│   ├── sandbox.rs
│   ├── routines.rs
│   ├── settings.rs
│   ├── workspace.rs
│   ├── agent_registry.rs
│   ├── tool_failures.rs
│   └── experiments.rs
├── schema_divergence.rs
└── schema_divergence_allowlist.json
```

Rationale:

- keeps production code free of test-module wiring
- lets us run one dedicated contract-test binary per backend
- gives us a single place for shared helpers and fixtures

### 4.2 Documentation

Add a human-readable schema notes file:

```text
docs/DATABASE_SCHEMA_NOTES.md
```

This should contain:

- every intentional Postgres/libSQL schema difference
- why it exists
- whether it is behaviorally equivalent
- what test protects it

### 4.3 Optional Helper Updates

If reuse makes the suite cleaner, extend `src/testing.rs` with backend-aware DB test helpers. This is optional; the contract harness can also live entirely under `tests/db_contract/support.rs`.

---

## 5. Test Architecture

### 5.1 Test Target Layout

Use a single root integration test target:

```rust
// tests/db_contract.rs
mod db_contract;
```

Then place all actual test modules under `tests/db_contract/`.

This gives one cohesive contract suite while keeping code organized by trait area.

### 5.2 Backend Selection

The harness should read:

- `DATABASE_BACKEND=libsql|postgres`
- `DATABASE_URL` for postgres

The contract suite should default to `libsql` for local development if `DATABASE_BACKEND` is unset.

### 5.3 Isolation Strategy

Isolation must be explicit. The earlier "connect to `DATABASE_URL` and assume isolation" approach is not sufficient.

#### libSQL

Use one fresh database per test:

- preferred: temp-file libSQL DB via `tempfile::TempDir`
- acceptable: in-memory DB if no test requires file-backed behavior

Each test:

1. creates a fresh libSQL database
2. runs libSQL migrations
3. returns an `Arc<dyn Database>` plus a guard that keeps the temp dir alive

This path is parallel-safe.

#### PostgreSQL

Use **one of two explicit modes**:

##### Primary CI mode: per-test disposable database

For CI and local environments with `CREATEDB` privilege:

1. connect to an admin database derived from `DATABASE_URL`
2. create a unique database name such as `thinclaw_contract_<uuid>`
3. connect `PgBackend` to that new database
4. run migrations in that database
5. drop the database in a guard on cleanup

This provides true isolation and is the preferred long-term mode.

##### Fallback local mode: shared contract DB with serialized reset

For local environments without `CREATEDB`:

1. use a dedicated contract database
2. run the contract target with `--test-threads=1`
3. before each test, reset the DB to a clean state
4. rerun migrations

This mode is slower, but it is simpler than adding pool-level schema routing hooks and is still fully actionable.

### 5.4 Harness API

The shared helper should return a struct instead of a bare `Arc<dyn Database>`:

```rust
pub(crate) struct ContractDb {
    pub db: Arc<dyn Database>,
    // guard fields kept alive for temp dirs / disposable postgres dbs
}
```

Responsibilities:

- backend selection
- isolation setup
- migration execution
- cleanup on drop
- helper methods for common assertions if useful

### 5.5 Fixture Strategy

Add `tests/db_contract/fixtures.rs` with reusable builders for:

- conversations and messages
- actors and endpoints
- jobs and actions
- routines and routine runs
- workspace documents and chunks
- experiment projects/campaigns/trials/targets/leases

This prevents every test file from inventing its own data shape and reduces accidental inconsistency between backend runs.

---

## 6. Coverage Model

The suite should not aim for "one test per method." It should aim for "every method covered by at least one explicit contract."

To make this enforceable, add a **coverage ledger** in `tests/db_contract/coverage_ledger.rs` or a nearby static data file that maps:

- trait method name
- method category:
  - backend contract
  - adapter
  - default helper
  - intentionally backend-specific
- test function(s) covering it

The ledger must be reviewed as part of the implementation so no trait method is left unclassified.

### 6.1 What Counts as Backend Contract Coverage

Use backend contract tests for methods where SQL/storage behavior can diverge:

- CRUD and existence semantics
- ordering and pagination
- search behavior
- JSON handling
- uniqueness and nullability behavior
- transaction-sensitive behavior
- migration-created defaults and constraints

### 6.2 What Counts as Adapter/Default Coverage

Use separate tests for methods whose logic is mostly in trait/default Rust code:

- `IdentityStore` adapter behavior
- `SandboxStore` default actor-filter helpers
- `RoutineStore` actor-filter helpers
- `WorkspaceStore::replace_chunks` fallback semantics

These tests still run against real backends, but they should be labeled as adapter/default coverage in the ledger.

---

## 7. Contract Test Matrix

The following matrix is the implementation target. Counts are approximate; the ledger is the source of truth.

| Area | File | Coverage Type | Target Tests | Key Contracts |
|------|------|---------------|--------------|---------------|
| ConversationStore | `tests/db_contract/conversations.rs` | backend | 16-20 | create/ensure, preview ordering, assistant conversation reuse, metadata updates, handoff metadata, pagination cursors, transcript search, learning event/eval/candidate/artifact/feedback/rollback/code proposal flows |
| IdentityRegistryStore | `tests/db_contract/identity.rs` | backend | 8-10 | actor CRUD, status updates, endpoint upsert/delete, resolution, preferred endpoint, last-active endpoint |
| IdentityStore | `tests/db_contract/identity.rs` | adapter | 4-6 | string UUID parsing, upsert bridge, rename bridge, endpoint bridge methods |
| JobStore | `tests/db_contract/jobs.rs` | backend | 8-10 | save/get, status transitions, stuck detection, actions ordering, LLM calls, estimation snapshot actuals |
| SandboxStore | `tests/db_contract/sandbox.rs` | backend + default | 8-10 | save/get/list/update, cleanup, summary, user scoping, actor helper filters, mode persistence, job events |
| RoutineStore | `tests/db_contract/routines.rs` | backend + default | 10-12 | CRUD, name lookup, due cron/event listing, runtime updates, run lifecycle, cleanup stale runs, actor helper filtering |
| SettingsStore | `tests/db_contract/settings.rs` | backend | 6-8 | get/set/delete, full row retrieval, list ordering, bulk set/get, has-settings |
| WorkspaceStore | `tests/db_contract/workspace.rs` | backend + default | 10-12 | doc CRUD, list directory, list paths/documents, chunk CRUD, replace-chunks semantics, embeddings, pending chunks, hybrid search |
| AgentRegistryStore | `tests/db_contract/agent_registry.rs` | backend | 6-7 | insert/get/list/update/delete, slug uniqueness expectations |
| ToolFailureStore | `tests/db_contract/tool_failures.rs` | backend | 4-5 | failure recording, threshold detection, repair mark, repair attempts |
| ExperimentStore | `tests/db_contract/experiments.rs` | backend | 14-18 | project/profile/campaign/trial CRUD, artifacts replacement, targets, target links, model usage queries, leases |

Expected total after first complete pass:

- **contract tests:** roughly 80-100
- **schema tests:** 2-4
- **adapter/default helper tests:** included in the files above

---

## 8. High-Risk Contracts That Must Not Be Skipped

The following areas are mandatory because they are the most likely to drift:

### 8.1 Search Semantics

- transcript search ranking and ordering
- case sensitivity differences
- FTS tokenization edge cases
- workspace hybrid search behavior

### 8.2 JSON Semantics

- metadata read/write behavior
- null vs empty object behavior
- merge/update semantics for JSON fields

### 8.3 Ordering and Pagination

- stable ordering when timestamps tie
- cursor pagination using `before`
- deterministic ordering when row IDs differ by backend

### 8.4 Constraints and Nullability

- uniqueness with nullable fields
- endpoint uniqueness rules
- actor or workspace uniqueness assumptions

### 8.5 Transaction-Like Behavior

- `replace_chunks` semantics
- multi-step experiment artifact replacement
- cleanup methods that should leave consistent state

### 8.6 Timestamps and UUIDs

- persisted timestamp round-tripping
- ordering by timestamp fields
- UUID parsing/serialization through adapter methods

---

## 9. Schema Divergence Audit

### 9.1 Deliverables

Add:

- `tests/schema_divergence.rs`
- `tests/schema_divergence_allowlist.json`
- `docs/DATABASE_SCHEMA_NOTES.md`

### 9.2 Comparison Strategy

Do **not** compare raw SQL files directly.

Instead:

1. Stand up a fresh migrated PostgreSQL test database.
2. Stand up a fresh migrated libSQL test database.
3. Extract live schema metadata from each backend.
4. Normalize the metadata into a shared structure.
5. Compare the normalized structures.
6. Fail if any difference is not present in the allowlist.

### 9.3 Metadata to Normalize

Normalize at least:

- tables
- columns
- normalized types
- nullability
- default values
- primary keys
- unique constraints
- foreign keys
- indexes
- generated/search-related structures where meaningful

Recommended sources:

- PostgreSQL:
  - `information_schema.columns`
  - `information_schema.table_constraints`
  - `information_schema.key_column_usage`
  - `pg_indexes`
- libSQL:
  - `sqlite_master`
  - `PRAGMA table_info`
  - `PRAGMA foreign_key_list`
  - `PRAGMA index_list`
  - `PRAGMA index_info`

### 9.4 Allowlist Policy

The machine-readable allowlist should only contain **intentional and justified** differences such as:

- native `UUID` vs `TEXT`
- native `JSONB` vs JSON text storage
- pgvector index vs libSQL blob/vector representation
- PostgreSQL `tsvector`/GIN vs libSQL FTS tables and triggers

Every allowlisted difference must also be described in `docs/DATABASE_SCHEMA_NOTES.md`.

### 9.5 Migration Repair Coverage

The schema checker alone is not enough because libSQL also has `UPGRADES` and `DATA_REPAIRS`.

Add targeted tests for:

- legacy libSQL DBs receiving missing columns
- data repairs repopulating identity/conversation fields
- FTS rebuild repair behavior

Existing libSQL migration regression tests should remain in place and can be extended where needed.

---

## 10. CI Integration

### 10.1 New Jobs

Add dedicated CI jobs instead of relying only on the current `tests` job.

#### `db-contract-libsql`

- no service containers
- command:

```bash
cargo test --test db_contract --no-default-features --features libsql -- --nocapture
```

#### `db-contract-postgres`

- postgres service container
- environment:
  - `DATABASE_BACKEND=postgres`
  - `DATABASE_URL=...`
- command:

```bash
cargo test --test db_contract --no-default-features --features postgres -- --nocapture --test-threads=1
```

If disposable per-test DB creation is confirmed stable, the serial restriction can be removed later.

#### `schema-divergence`

- postgres service container
- features: both backends
- command:

```bash
cargo test --test schema_divergence --no-default-features --features "postgres libsql" -- --nocapture --test-threads=1
```

### 10.2 Existing `tests` Job

Keep the current `cargo llvm-cov --all-features` job, but do not treat it as the sole parity signal.

Its role should be:

- overall regression detection
- coverage collection

The backend-specific jobs should be the authoritative parity gates.

### 10.3 Failure Policy

The branch is not ready to merge if any of the following fail:

- libSQL contract tests
- PostgreSQL contract tests
- schema divergence checker

---

## 11. Implementation Phases

### Phase 0: Inventory and Coverage Ledger

1. Enumerate every method on the `Database` surface from `src/db/mod.rs`.
2. Classify each method as:
   - backend contract
   - adapter
   - default helper
   - intentionally backend-specific
3. Create the initial coverage ledger.

Verification:

- no method remains unclassified

### Phase 1: Harness and Isolation

1. Add `tests/db_contract.rs` and module tree.
2. Implement shared backend selection helper.
3. Implement libSQL per-test DB creation.
4. Implement PostgreSQL isolation:
   - primary disposable-DB mode
   - serialized reset fallback mode
5. Add fixture builders.

Verification:

- one pilot test passes on libSQL
- one pilot test passes on PostgreSQL

### Phase 2: Pilot Slice

Start with `SettingsStore`, because it is small and exercises the pattern cleanly.

Required pilot contracts:

- get/set/delete
- list settings
- full-row retrieval
- bulk set/get
- `has_settings`

Verification:

- pilot file green on both backends
- ledger updated to show settings coverage

### Phase 3: Full Contract Suite

Implement the remaining test files in this order:

1. conversations
2. identity
3. jobs
4. routines
5. workspace
6. sandbox
7. agent registry
8. tool failures
9. experiments

For each file:

1. add tests
2. update ledger
3. run under both backends
4. fix any discovered divergence or document it if intentional

Verification:

- every file green on both backends
- ledger fully mapped

### Phase 4: Schema Divergence Checker

1. implement normalized schema extraction for both backends
2. add allowlist file
3. add documentation file for intentional diffs
4. add migration repair checks where schema comparison alone is insufficient

Verification:

- checker passes on current mainline schema
- checker fails when an unallowlisted semantic difference is introduced

### Phase 5: CI Wiring

1. add backend-specific jobs
2. ensure service setup and env vars are correct
3. confirm the new jobs do not rely on the coverage job

Verification:

- local dry run commands work
- CI runs all three gates successfully

---

## 12. Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| PostgreSQL test isolation is hard with pooled connections | flaky tests | use disposable DBs first; fall back to serialized reset mode if needed |
| Search ranking differs structurally between FTS engines | false positives | assert stable behavior categories and ordering guarantees, not identical ranking formulas |
| Schema checker becomes noisy due to dialect differences | low signal CI | normalize aggressively and keep a tight allowlist |
| Experiment coverage balloons in size | schedule slip | fixture builders plus ledger-first method mapping |
| Contract runtime becomes too slow | CI drag | keep libSQL parallel; keep postgres serial first; shard later only if needed |

---

## 13. Estimated Effort

This upgraded plan is larger than the original rough estimate because it now includes real Postgres isolation, a coverage ledger, and a normalized schema checker.

| Phase | Estimated Time |
|------|-----------------|
| Phase 0: inventory + ledger | 2-3 hours |
| Phase 1: harness + isolation | 4-6 hours |
| Phase 2: pilot slice | 2-3 hours |
| Phase 3: full contract suite | 14-18 hours |
| Phase 4: schema checker + notes | 6-8 hours |
| Phase 5: CI integration + polish | 3-4 hours |
| **Total** | **31-42 hours** |

If PostgreSQL disposable test DB creation works on the first pass, expect the lower end. If local fallback handling and schema normalization need extra iteration, expect the upper end.

---

## 14. Done Definition

This plan is considered executed only when:

- `tests/db_contract.rs` exists and is green on both backends
- `tests/schema_divergence.rs` exists and is green
- `tests/schema_divergence_allowlist.json` exists and is minimal
- `docs/DATABASE_SCHEMA_NOTES.md` exists and explains every intentional divergence
- the coverage ledger accounts for the full `Database` surface
- CI enforces all three gates

---

## 15. Follow-Up Work (Out of Scope for This Plan)

Once parity is enforced, consider a separate plan for:

- splitting `src/db/postgres.rs` into modules parallel to `src/db/libsql/`
- reducing contract-test runtime
- adding property/fuzz-style backend equivalence tests for selected stores
