# Database Schema Divergence Notes

This file tracks intentional schema differences between PostgreSQL and libSQL.

## Intentional Differences

1. `UUID` columns in PostgreSQL are stored as `TEXT` in libSQL.
2. `JSONB` columns in PostgreSQL are stored as JSON-encoded `TEXT` in libSQL.
3. PostgreSQL `tsvector`/GIN full-text columns are represented by FTS virtual tables and triggers in libSQL:
- `conversation_messages_fts`
- `memory_chunks_fts`
4. PostgreSQL migration bookkeeping uses `refinery_schema_history`; libSQL uses `_migrations`.
5. PostgreSQL `memory_chunks.content_tsv` generated column does not exist in libSQL.
6. Vector storage differs:
- PostgreSQL uses `VECTOR(...)`
- libSQL uses `BLOB`-based embeddings with compatibility columns

## Enforcement

Schema drift checks run in `tests/schema_divergence.rs` and apply the machine-readable allowlist from:

- `tests/schema_divergence_allowlist.json`

Any new intentional divergence should be documented here and added to the allowlist in the same change.

