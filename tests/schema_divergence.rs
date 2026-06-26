#![cfg(all(feature = "postgres", feature = "libsql"))]

use std::collections::{BTreeMap, BTreeSet};

use secrecy::SecretString;
use serde::Deserialize;
use thinclaw::config::{DatabaseBackend, DatabaseConfig};
use thinclaw::db::Database;
use thinclaw::db::libsql::LibSqlBackend;
use thinclaw::db::postgres::PgBackend;
use tokio_postgres::NoTls;
use uuid::Uuid;

#[derive(Debug, Deserialize, Default)]
struct SchemaAllowlist {
    #[serde(default)]
    ignore_tables: Vec<String>,
    #[serde(default)]
    ignore_columns: Vec<String>,
    /// Fully-qualified `table.column` entries whose normalized type may differ
    /// between backends without being treated as a divergence.
    #[serde(default)]
    ignore_types: Vec<String>,
    /// Fully-qualified `table.column` entries whose nullability may differ
    /// between backends without being treated as a divergence.
    #[serde(default)]
    ignore_nullability: Vec<String>,
    /// `table` entries whose index sets are intentionally allowed to differ
    /// between backends (e.g. backend-specific FTS shadow indexes).
    #[serde(default)]
    ignore_indexes: Vec<String>,
    /// Exact diff identifiers (the strings produced by `compare_snapshots`) that
    /// are accepted as intended divergences.
    #[serde(default)]
    allowed_exact: Vec<String>,
}

/// A single column's parity-relevant shape. Types are stored normalized to an
/// affinity class (see `normalize_type`) because Postgres and SQLite type
/// systems differ by design and a raw-string comparison would be a
/// false-positive generator.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ColumnInfo {
    normalized_type: String,
    not_null: bool,
}

/// A single index's parity-relevant shape. Indexes are keyed by their ordered
/// column list plus uniqueness so that the comparison is name-agnostic
/// (Postgres and SQLite generate different index names for the same intent).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct IndexInfo {
    columns: Vec<String>,
    unique: bool,
}

impl IndexInfo {
    fn render(&self) -> String {
        let cols = self.columns.join(",");
        if self.unique {
            format!("unique({cols})")
        } else {
            format!("({cols})")
        }
    }
}

#[derive(Debug, Default)]
struct TableSnapshot {
    columns: BTreeMap<String, ColumnInfo>,
    indexes: BTreeSet<IndexInfo>,
}

#[derive(Debug, Default)]
struct SchemaSnapshot {
    tables: BTreeMap<String, TableSnapshot>,
}

/// Which divergence dimensions are enforced this run.
///
/// Column-name presence is always enforced (the original, already-seeded
/// behavior). The richer dimensions — normalized types, nullability, and
/// indexes — are additionally enforced only when opted in via
/// `SCHEMA_DIVERGENCE_STRICT=1`. They are opt-in because the allowlist must
/// first be *seeded* from a real Postgres+libSQL run (the two schemas are
/// authored independently — `migrations/V*.sql` for Postgres,
/// `libsql_migrations::SCHEMA` for libSQL — so intended residue such as
/// Postgres' partial indexes cannot be enumerated statically). Until the
/// allowlist is seeded, enabling these would fail on pre-existing accepted
/// differences, which the WS-02 plan explicitly forbids ("starts green").
/// CI's `schema-divergence` job sets the flag after the seeding pass.
#[derive(Debug, Clone, Copy)]
struct DivergenceChecks {
    types: bool,
    nullability: bool,
    indexes: bool,
}

impl DivergenceChecks {
    fn from_env() -> Self {
        let strict = matches!(
            std::env::var("SCHEMA_DIVERGENCE_STRICT").ok().as_deref(),
            Some("1") | Some("true") | Some("yes")
        );
        Self {
            types: strict,
            nullability: strict,
            indexes: strict,
        }
    }
}

#[tokio::test]
async fn schema_columns_match_with_allowlist() {
    let allowlist: SchemaAllowlist =
        serde_json::from_str(include_str!("schema_divergence_allowlist.json"))
            .expect("allowlist JSON must be valid");

    // This test is gated behind `all(feature = "postgres", feature = "libsql")`
    // (see the crate-level cfg above), so it only compiles in the dual-feature
    // build CI uses for the `schema-divergence` job. A plain local
    // `cargo test` does not build it. The only place it runs is that CI job,
    // which always provisions Postgres — so a missing DATABASE_URL there is a
    // real failure, not a reason to silently pass. (WS-13 owns keeping the job
    // provisioned; see the WS-02 hand-off note.)
    let base_url = std::env::var("DATABASE_URL").expect(
        "schema_divergence requires DATABASE_URL; this test is gated behind the \
         postgres+libsql features and only runs in the schema-divergence CI job, \
         which always provisions Postgres",
    );

    let pg_schema = format!("contract_schema_{}", Uuid::new_v4().simple());
    create_postgres_schema(&base_url, &pg_schema)
        .await
        .expect("schema divergence test requires a reachable Postgres to create the test schema");
    let pg_url = postgres_url_with_search_path(&base_url, &pg_schema)
        .expect("schema divergence test requires a parseable DATABASE_URL");

    let pg_cfg = DatabaseConfig {
        backend: DatabaseBackend::Postgres,
        url: SecretString::from(pg_url),
        pool_size: 2,
        libsql_path: None,
        libsql_url: None,
        libsql_auth_token: None,
    };

    let pg_backend = PgBackend::new(&pg_cfg)
        .await
        .expect("schema divergence test requires a reachable Postgres backend");
    pg_backend
        .run_migrations()
        .await
        .expect("postgres migrations must succeed for the schema divergence test");

    let dir = tempfile::tempdir().expect("tempdir");
    let libsql_path = dir.path().join("schema-contract.db");
    let libsql_backend = LibSqlBackend::new_local(&libsql_path)
        .await
        .expect("libsql should open");
    libsql_backend
        .run_migrations()
        .await
        .expect("libsql migrations should succeed");

    let pg_snapshot = snapshot_postgres_schema(&pg_backend)
        .await
        .expect("postgres snapshot should succeed");
    let libsql_snapshot = snapshot_libsql_schema(&libsql_backend)
        .await
        .expect("libsql snapshot should succeed");

    let checks = DivergenceChecks::from_env();
    let diff = compare_snapshots(&pg_snapshot, &libsql_snapshot, &allowlist, checks);

    if !diff.is_empty() {
        let rendered = diff.join("\n");
        panic!("schema divergence not allowlisted:\n{rendered}");
    }
}

/// Normalize a backend-specific type name to a loose affinity class shared by
/// Postgres and SQLite. SQLite uses storage affinity, not strict types, so the
/// migrations intentionally map (for example) `timestamptz`/`uuid`/`jsonb` to
/// `TEXT`. Comparing raw strings would flood the diff with `text` vs `TEXT`
/// noise; comparing affinity classes catches genuine shape drift.
fn normalize_type(raw: &str) -> String {
    let lowered = raw.trim().to_ascii_lowercase();
    // Strip any length/precision qualifier, e.g. `character varying(255)`,
    // `numeric(10,2)`, `varchar(64)`.
    let base = lowered
        .split_once('(')
        .map(|(head, _)| head.trim())
        .unwrap_or(lowered.as_str());

    match base {
        // Text-like.
        "text" | "character varying" | "varchar" | "char" | "character" | "name" | "citext"
        | "bpchar" => "text".to_string(),
        // Integer-like.
        "integer" | "int" | "int2" | "int4" | "int8" | "bigint" | "smallint" | "serial"
        | "bigserial" => "integer".to_string(),
        // Boolean (SQLite stores as integer, but migrations keep a `bool`/`boolean`
        // affinity distinct so we preserve it as its own class).
        "boolean" | "bool" => "boolean".to_string(),
        // Floating point / real.
        "real" | "double precision" | "float" | "float4" | "float8" => "real".to_string(),
        // Arbitrary precision — SQLite has no native NUMERIC type; migrations may
        // store as TEXT or REAL, so map to a shared class.
        "numeric" | "decimal" => "numeric".to_string(),
        // Blob.
        "bytea" | "blob" => "blob".to_string(),
        // Everything else (timestamps, uuid, jsonb, json, inet, etc.). SQLite
        // stores all of these as TEXT, and the migrations do the same on the
        // Postgres side only where a richer type is intended. We collapse the
        // remainder to `text` so timestamp/uuid/json columns line up across
        // backends; genuinely-intended residue is recorded in `ignore_types`.
        _ => "text".to_string(),
    }
}

fn compare_snapshots(
    pg: &SchemaSnapshot,
    libsql: &SchemaSnapshot,
    allowlist: &SchemaAllowlist,
    checks: DivergenceChecks,
) -> Vec<String> {
    let ignored_tables: BTreeSet<&str> =
        allowlist.ignore_tables.iter().map(String::as_str).collect();
    let ignored_columns: BTreeSet<&str> = allowlist
        .ignore_columns
        .iter()
        .map(String::as_str)
        .collect();
    let ignored_types: BTreeSet<&str> = allowlist.ignore_types.iter().map(String::as_str).collect();
    let ignored_nullability: BTreeSet<&str> = allowlist
        .ignore_nullability
        .iter()
        .map(String::as_str)
        .collect();
    let ignored_indexes: BTreeSet<&str> = allowlist
        .ignore_indexes
        .iter()
        .map(String::as_str)
        .collect();
    let allowed_exact: BTreeSet<&str> =
        allowlist.allowed_exact.iter().map(String::as_str).collect();

    let push_if_unallowed = |diffs: &mut Vec<String>, id: String| {
        if !allowed_exact.contains(id.as_str()) {
            diffs.push(id);
        }
    };

    let mut diffs = Vec::new();
    let all_tables: BTreeSet<_> = pg
        .tables
        .keys()
        .chain(libsql.tables.keys())
        .cloned()
        .collect();

    for table in all_tables {
        if ignored_tables.contains(table.as_str()) {
            continue;
        }

        match (pg.tables.get(&table), libsql.tables.get(&table)) {
            (Some(pg_tbl), Some(ls_tbl)) => {
                compare_columns(
                    &table,
                    pg_tbl,
                    ls_tbl,
                    &ignored_columns,
                    &ignored_types,
                    &ignored_nullability,
                    &allowed_exact,
                    checks,
                    &mut diffs,
                );

                if checks.indexes && !ignored_indexes.contains(table.as_str()) {
                    compare_indexes(&table, pg_tbl, ls_tbl, &push_if_unallowed, &mut diffs);
                }
            }
            (Some(_), None) => {
                push_if_unallowed(&mut diffs, format!("missing_table:libsql:{table}"));
            }
            (None, Some(_)) => {
                push_if_unallowed(&mut diffs, format!("missing_table:postgres:{table}"));
            }
            (None, None) => {}
        }
    }

    diffs
}

#[allow(clippy::too_many_arguments)]
fn compare_columns(
    table: &str,
    pg_tbl: &TableSnapshot,
    ls_tbl: &TableSnapshot,
    ignored_columns: &BTreeSet<&str>,
    ignored_types: &BTreeSet<&str>,
    ignored_nullability: &BTreeSet<&str>,
    allowed_exact: &BTreeSet<&str>,
    checks: DivergenceChecks,
    diffs: &mut Vec<String>,
) {
    let all_cols: BTreeSet<_> = pg_tbl
        .columns
        .keys()
        .chain(ls_tbl.columns.keys())
        .cloned()
        .collect();

    for col in all_cols {
        let fq = format!("{table}.{col}");
        if ignored_columns.contains(fq.as_str()) {
            continue;
        }
        let pg_col = pg_tbl.columns.get(&col);
        let ls_col = ls_tbl.columns.get(&col);

        match (pg_col, ls_col) {
            (Some(pg_info), Some(ls_info)) => {
                if checks.types
                    && pg_info.normalized_type != ls_info.normalized_type
                    && !ignored_types.contains(fq.as_str())
                {
                    let id = format!(
                        "type_mismatch:{table}:{col}:pg={},libsql={}",
                        pg_info.normalized_type, ls_info.normalized_type
                    );
                    if !allowed_exact.contains(id.as_str()) {
                        diffs.push(id);
                    }
                }
                if checks.nullability
                    && pg_info.not_null != ls_info.not_null
                    && !ignored_nullability.contains(fq.as_str())
                {
                    let id = format!(
                        "nullability_mismatch:{table}:{col}:pg_not_null={},libsql_not_null={}",
                        pg_info.not_null, ls_info.not_null
                    );
                    if !allowed_exact.contains(id.as_str()) {
                        diffs.push(id);
                    }
                }
            }
            (Some(_), None) => {
                let id = format!("missing_column:libsql:{table}:{col}");
                if !allowed_exact.contains(id.as_str()) {
                    diffs.push(id);
                }
            }
            (None, Some(_)) => {
                let id = format!("missing_column:postgres:{table}:{col}");
                if !allowed_exact.contains(id.as_str()) {
                    diffs.push(id);
                }
            }
            (None, None) => {}
        }
    }
}

fn compare_indexes(
    table: &str,
    pg_tbl: &TableSnapshot,
    ls_tbl: &TableSnapshot,
    push_if_unallowed: &impl Fn(&mut Vec<String>, String),
    diffs: &mut Vec<String>,
) {
    for idx in pg_tbl.indexes.difference(&ls_tbl.indexes) {
        push_if_unallowed(
            diffs,
            format!("missing_index:libsql:{table}:{}", idx.render()),
        );
    }
    for idx in ls_tbl.indexes.difference(&pg_tbl.indexes) {
        push_if_unallowed(
            diffs,
            format!("missing_index:postgres:{table}:{}", idx.render()),
        );
    }
}

async fn snapshot_postgres_schema(pg: &PgBackend) -> Result<SchemaSnapshot, String> {
    let conn = pg
        .pool()
        .get()
        .await
        .map_err(|e| format!("pool get failed: {e}"))?;

    let table_rows = conn
        .query(
            r#"
            SELECT table_name
            FROM information_schema.tables
            WHERE table_schema = current_schema()
              AND table_type = 'BASE TABLE'
            ORDER BY table_name
            "#,
            &[],
        )
        .await
        .map_err(|e| format!("table query failed: {e}"))?;

    let mut snapshot = SchemaSnapshot::default();
    for row in table_rows {
        let table: String = row.get("table_name");
        snapshot.tables.entry(table).or_default();
    }

    let column_rows = conn
        .query(
            r#"
            SELECT table_name, column_name, data_type, udt_name, is_nullable
            FROM information_schema.columns
            WHERE table_schema = current_schema()
            ORDER BY table_name, ordinal_position
            "#,
            &[],
        )
        .await
        .map_err(|e| format!("column query failed: {e}"))?;

    for row in column_rows {
        let table: String = row.get("table_name");
        let column: String = row.get("column_name");
        // `data_type` is the SQL-standard name (e.g. "character varying"); for
        // domain/array/user types it is "USER-DEFINED" and `udt_name` carries
        // the concrete name. Prefer `data_type` unless it is the catch-all.
        let data_type: String = row.get("data_type");
        let udt_name: String = row.get("udt_name");
        let raw_type = if data_type.eq_ignore_ascii_case("USER-DEFINED") {
            udt_name
        } else {
            data_type
        };
        let is_nullable: String = row.get("is_nullable");
        let info = ColumnInfo {
            normalized_type: normalize_type(&raw_type),
            not_null: is_nullable.eq_ignore_ascii_case("NO"),
        };
        snapshot
            .tables
            .entry(table)
            .or_default()
            .columns
            .insert(column, info);
    }

    // Index introspection. `pg_indexes` gives the names; we then resolve each
    // index's ordered column list and uniqueness from the catalog so the shape
    // is comparable to SQLite's `PRAGMA index_info`. Skip index entries backing
    // primary keys/expressions where a column cannot be resolved (those compare
    // poorly across backends and are covered by column nullability).
    let index_rows = conn
        .query(
            r#"
            SELECT
                t.relname        AS table_name,
                i.relname        AS index_name,
                ix.indisunique   AS is_unique,
                a.attname        AS column_name,
                k.ord            AS column_ordinal
            FROM pg_class t
            JOIN pg_namespace n ON n.oid = t.relnamespace
            JOIN pg_index ix ON ix.indrelid = t.oid
            JOIN pg_class i ON i.oid = ix.indexrelid
            JOIN LATERAL unnest(ix.indkey) WITH ORDINALITY AS k(attnum, ord) ON TRUE
            JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum
            WHERE n.nspname = current_schema()
              AND t.relkind = 'r'
              AND k.attnum <> 0
            ORDER BY t.relname, i.relname, k.ord
            "#,
            &[],
        )
        .await
        .map_err(|e| format!("index query failed: {e}"))?;

    // Accumulate columns per (table, index) preserving order.
    let mut index_acc: BTreeMap<(String, String), (bool, Vec<String>)> = BTreeMap::new();
    for row in index_rows {
        let table: String = row.get("table_name");
        let index: String = row.get("index_name");
        let unique: bool = row.get("is_unique");
        let column: String = row.get("column_name");
        let entry = index_acc
            .entry((table, index))
            .or_insert_with(|| (unique, Vec::new()));
        entry.1.push(column);
    }
    for ((table, _index), (unique, columns)) in index_acc {
        snapshot
            .tables
            .entry(table)
            .or_default()
            .indexes
            .insert(IndexInfo { columns, unique });
    }

    Ok(snapshot)
}

async fn snapshot_libsql_schema(libsql: &LibSqlBackend) -> Result<SchemaSnapshot, String> {
    let conn = libsql.connect().await.map_err(|e| e.to_string())?;
    let mut rows = conn
        .query(
            r#"
            SELECT name
            FROM sqlite_master
            WHERE type = 'table'
              AND name NOT LIKE 'sqlite_%'
            ORDER BY name
            "#,
            (),
        )
        .await
        .map_err(|e| format!("sqlite table query failed: {e}"))?;

    let mut snapshot = SchemaSnapshot::default();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| format!("sqlite table row failed: {e}"))?
    {
        let table = row
            .get::<String>(0)
            .map_err(|e| format!("sqlite table read failed: {e}"))?;
        snapshot.tables.entry(table).or_default();
    }

    let table_names: Vec<String> = snapshot.tables.keys().cloned().collect();
    for table in table_names {
        let escaped = table.replace('\'', "''");

        // Columns: PRAGMA table_info returns
        //   0=cid, 1=name, 2=type, 3=notnull, 4=dflt_value, 5=pk
        let pragma = format!("PRAGMA table_info('{escaped}')");
        let mut col_rows = conn
            .query(&pragma, ())
            .await
            .map_err(|e| format!("pragma table_info failed for {table}: {e}"))?;
        while let Some(col_row) = col_rows
            .next()
            .await
            .map_err(|e| format!("pragma row failed for {table}: {e}"))?
        {
            let col_name = col_row
                .get::<String>(1)
                .map_err(|e| format!("pragma col parse failed for {table}: {e}"))?;
            let col_type = col_row.get::<String>(2).unwrap_or_default();
            let not_null = col_row.get::<i64>(3).unwrap_or(0) != 0;
            let info = ColumnInfo {
                normalized_type: normalize_type(&col_type),
                not_null,
            };
            snapshot
                .tables
                .entry(table.clone())
                .or_default()
                .columns
                .insert(col_name, info);
        }

        // Indexes: PRAGMA index_list returns
        //   0=seq, 1=name, 2=unique, 3=origin, 4=partial
        // We skip auto-indexes (origin 'pk'/'u' that SQLite synthesizes) only
        // when they cannot be resolved; explicit and uniqueness-constraint
        // indexes are compared by their column list.
        let index_list = format!("PRAGMA index_list('{escaped}')");
        let mut idx_rows = conn
            .query(&index_list, ())
            .await
            .map_err(|e| format!("pragma index_list failed for {table}: {e}"))?;
        let mut index_names: Vec<(String, bool)> = Vec::new();
        while let Some(idx_row) = idx_rows
            .next()
            .await
            .map_err(|e| format!("pragma index_list row failed for {table}: {e}"))?
        {
            let idx_name = idx_row
                .get::<String>(1)
                .map_err(|e| format!("index name parse failed for {table}: {e}"))?;
            let unique = idx_row.get::<i64>(2).unwrap_or(0) != 0;
            index_names.push((idx_name, unique));
        }

        for (idx_name, unique) in index_names {
            let escaped_idx = idx_name.replace('\'', "''");
            // PRAGMA index_info returns 0=seqno, 1=cid, 2=name
            let index_info = format!("PRAGMA index_info('{escaped_idx}')");
            let mut info_rows = conn
                .query(&index_info, ())
                .await
                .map_err(|e| format!("pragma index_info failed for {idx_name}: {e}"))?;
            let mut columns = Vec::new();
            while let Some(info_row) = info_rows
                .next()
                .await
                .map_err(|e| format!("pragma index_info row failed for {idx_name}: {e}"))?
            {
                // Column name may be NULL for expression indexes; skip those.
                if let Ok(col) = info_row.get::<String>(2) {
                    columns.push(col);
                }
            }
            if columns.is_empty() {
                continue;
            }
            snapshot
                .tables
                .entry(table.clone())
                .or_default()
                .indexes
                .insert(IndexInfo { columns, unique });
        }
    }

    Ok(snapshot)
}

async fn create_postgres_schema(base_url: &str, schema: &str) -> Result<(), String> {
    let cfg: tokio_postgres::Config = base_url
        .parse()
        .map_err(|e| format!("invalid DATABASE_URL: {e}"))?;
    let (client, connection) = cfg
        .connect(NoTls)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            eprintln!("schema setup postgres connection ended: {err}");
        }
    });
    client
        .batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
        .await
        .map_err(|e| format!("CREATE SCHEMA failed: {e}"))?;
    Ok(())
}

fn postgres_url_with_search_path(base_url: &str, schema: &str) -> Result<String, String> {
    let mut url = url::Url::parse(base_url).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema},public"));
    Ok(url.to_string())
}
