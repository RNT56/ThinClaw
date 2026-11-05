# WS-10 — Data Layer & Crypto Correctness

| | |
|---|---|
| **Priority** | P2 |
| **Effort** | S |
| **Status** | ⏳ Not started |
| **Depends on** | WS-01 |
| **Findings** | DATA-4, DATA-8, SEC-10 |
| **Canonical docs** | `docs/POSTGRES_VECTOR_SEARCH.md`, `docs/DATABASE_SCHEMA_NOTES.md`, `src/workspace/README.md`, `docs/SECURITY.md` (secret storage paragraph) |

## Goal

Backend parity for scoped search, and defense-in-depth tightening of secret storage. No SQL injection was found; this workstream is correctness and hardening.

## Tasks

- [ ] **10.1 — Postgres HNSW scoped recall (DATA-4).**
  `crates/thinclaw-workspace/src/postgres_vector.rs:~226-252` orders by `(c.embedding::vector(n)) <=> $3 LIMIT $4` and applies user/agent/model/path predicates after the index scan; nothing sets `hnsw.ef_search` / `hnsw.iterative_scan`. With pgvector's default `ef_search = 40` (below the default `pre_fusion_limit` 50), per-scope recall on a multi-user/multi-agent DB can be empty or truncated. libSQL already grows the candidate window and has an exact fallback (`crates/thinclaw-db/src/libsql/workspace.rs:~1101-1106, ~1230`). Use `SET LOCAL hnsw.iterative_scan = relaxed_order` + `hnsw.ef_search >= limit` (pgvector ≥ 0.8) or an over-fetch + exact fallback.
  *Accept:* a `db_contract` test with many users where the target user's rows are a small fraction still returns `limit` results on both backends. Update `docs/POSTGRES_VECTOR_SEARCH.md`.

- [ ] **10.2 — FTS keyword quoting (DATA-8).**
  `expand_query_keywords` (`crates/thinclaw-workspace/src/search.rs:~447`) trims only leading/trailing punctuation, so `say"hi` yields a malformed FTS5 phrase (`crates/thinclaw-db/src/libsql/workspace.rs:~1021-1029`); the error propagates through `?` out of `hybrid_search`, discarding vector results too. Escape `"` as `""` (or drop it) per keyword, and degrade to vector-only results on an FTS error instead of failing the whole search. (Conversation search at `conversations/mod.rs:~743` is already correct.)

- [ ] **10.3 — Unambiguous AAD (SEC-10).**
  AAD `v{}|user={}|name={}|provider={}…` (`crates/thinclaw-secrets/src/store.rs:~99-111`) has no length prefixes and secret names may contain `|` (`types.rs:~273`), so distinct (user, name) pairs can collide → an undetected row swap given DB write access. Introduce a v3 AAD with length-prefixed fields (or forbid `|` in names), re-encrypt on rotation, keep v2 decrypt for migration.

- [ ] **10.4 — Zeroize key material (SEC-10).**
  The derived `[u8; 32]` key and the intermediate master-key `Vec<u8>` (`crates/thinclaw-config/src/secrets.rs:~67`) aren't zeroized. Wrap in `zeroize::Zeroizing` / `secrecy` types end-to-end.

## Verification (workstream exit)

```bash
cargo test --locked -p thinclaw-db -p thinclaw-workspace -p thinclaw-secrets
cargo test --locked --features full --test db_contract
cargo test --locked --features full --test schema_divergence
```
