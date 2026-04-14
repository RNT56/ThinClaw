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
    #[serde(default)]
    allowed_exact: Vec<String>,
}

#[derive(Debug, Default)]
struct SchemaSnapshot {
    tables: BTreeMap<String, BTreeSet<String>>,
}

#[tokio::test]
async fn schema_columns_match_with_allowlist() {
    let allowlist: SchemaAllowlist =
        serde_json::from_str(include_str!("schema_divergence_allowlist.json"))
            .expect("allowlist JSON must be valid");

    let Some(base_url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("skipping schema divergence test: DATABASE_URL is not set");
        return;
    };

    let pg_schema = format!("contract_schema_{}", Uuid::new_v4().simple());
    if let Err(err) = create_postgres_schema(&base_url, &pg_schema).await {
        eprintln!("skipping schema divergence test: cannot create schema: {err}");
        return;
    }
    let pg_url = match postgres_url_with_search_path(&base_url, &pg_schema) {
        Ok(url) => url,
        Err(err) => {
            eprintln!("skipping schema divergence test: cannot build postgres url: {err}");
            return;
        }
    };

    let pg_cfg = DatabaseConfig {
        backend: DatabaseBackend::Postgres,
        url: SecretString::from(pg_url),
        pool_size: 2,
        libsql_path: None,
        libsql_url: None,
        libsql_auth_token: None,
    };

    let pg_backend = match PgBackend::new(&pg_cfg).await {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("skipping schema divergence test: postgres connect failed: {err}");
            return;
        }
    };
    if let Err(err) = pg_backend.run_migrations().await {
        eprintln!("skipping schema divergence test: postgres migrations failed: {err}");
        return;
    }

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

    let diff = compare_snapshots(&pg_snapshot, &libsql_snapshot, &allowlist);

    if !diff.is_empty() {
        let rendered = diff.join("\n");
        panic!("schema divergence not allowlisted:\n{rendered}");
    }
}

fn compare_snapshots(
    pg: &SchemaSnapshot,
    libsql: &SchemaSnapshot,
    allowlist: &SchemaAllowlist,
) -> Vec<String> {
    let ignored_tables: BTreeSet<&str> =
        allowlist.ignore_tables.iter().map(String::as_str).collect();
    let ignored_columns: BTreeSet<&str> = allowlist
        .ignore_columns
        .iter()
        .map(String::as_str)
        .collect();
    let allowed_exact: BTreeSet<&str> =
        allowlist.allowed_exact.iter().map(String::as_str).collect();

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
            (Some(pg_cols), Some(ls_cols)) => {
                let all_cols: BTreeSet<_> = pg_cols.union(ls_cols).cloned().collect();
                for col in all_cols {
                    let fq = format!("{table}.{col}");
                    if ignored_columns.contains(fq.as_str()) {
                        continue;
                    }
                    let pg_has = pg_cols.contains(&col);
                    let ls_has = ls_cols.contains(&col);
                    if pg_has != ls_has {
                        let id = if pg_has {
                            format!("missing_column:libsql:{table}:{col}")
                        } else {
                            format!("missing_column:postgres:{table}:{col}")
                        };
                        if !allowed_exact.contains(id.as_str()) {
                            diffs.push(id);
                        }
                    }
                }
            }
            (Some(_), None) => {
                let id = format!("missing_table:libsql:{table}");
                if !allowed_exact.contains(id.as_str()) {
                    diffs.push(id);
                }
            }
            (None, Some(_)) => {
                let id = format!("missing_table:postgres:{table}");
                if !allowed_exact.contains(id.as_str()) {
                    diffs.push(id);
                }
            }
            (None, None) => {}
        }
    }

    diffs
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
            SELECT table_name, column_name
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
        snapshot.tables.entry(table).or_default().insert(column);
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
            snapshot
                .tables
                .entry(table.clone())
                .or_default()
                .insert(col_name);
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
