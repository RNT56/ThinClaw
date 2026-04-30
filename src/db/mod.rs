//! Database abstraction layer.
//!
//! Provides a backend-agnostic `Database` trait that unifies all persistence
//! operations. Two implementations exist behind feature flags:
//!
//! - `postgres` (default): Uses `deadpool-postgres` + `tokio-postgres`
//! - `libsql`: Uses libSQL (Turso's SQLite fork) for embedded/edge deployment
//!
//! The existing `Store`, `Repository`, `SecretsStore`, and `WasmToolStore`
//! types become thin wrappers that delegate to `Arc<dyn Database>`.

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "libsql")]
pub mod libsql;

#[cfg(feature = "libsql")]
pub mod libsql_migrations;

use std::sync::Arc;

use crate::error::DatabaseError;
pub use thinclaw_db::*;

/// Create a database backend from configuration, run migrations, and return it.
///
/// This is the shared helper for CLI commands and other call sites that need
/// a simple `Arc<dyn Database>` without retaining backend-specific handles
/// (e.g., `pg_pool` or `libsql_conn` for the secrets store). The main agent
/// startup in `main.rs` uses its own initialization block because it also
/// captures those backend-specific handles.
pub async fn connect_from_config(
    config: &crate::config::DatabaseConfig,
) -> Result<Arc<dyn Database>, DatabaseError> {
    match config.backend {
        #[cfg(feature = "libsql")]
        crate::config::DatabaseBackend::LibSql => {
            use secrecy::ExposeSecret as _;

            let default_path = crate::config::default_libsql_path();
            let db_path = config.libsql_path.as_deref().unwrap_or(&default_path);

            let backend = if let Some(ref url) = config.libsql_url {
                let token = config.libsql_auth_token.as_ref().ok_or_else(|| {
                    DatabaseError::Pool(
                        "LIBSQL_AUTH_TOKEN required when LIBSQL_URL is set".to_string(),
                    )
                })?;
                libsql::LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            } else {
                libsql::LibSqlBackend::new_local(db_path)
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            };
            backend.run_migrations().await?;
            Ok(Arc::new(backend))
        }
        #[cfg(feature = "postgres")]
        _ => {
            let pg = postgres::PgBackend::new(config)
                .await
                .map_err(|e| DatabaseError::Pool(e.to_string()))?;
            pg.run_migrations().await?;
            Ok(Arc::new(pg))
        }
        #[cfg(not(feature = "postgres"))]
        _ => Err(DatabaseError::Pool(
            "No database backend available. Enable 'postgres' or 'libsql' feature.".to_string(),
        )),
    }
}
