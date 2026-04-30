#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;
#[cfg(feature = "postgres")]
pub(super) fn ensure_rustls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    #[test]
    fn rustls_provider_is_available_with_unified_features() {
        super::ensure_rustls_crypto_provider();
        let _ = rustls::ClientConfig::builder();
    }
}

#[cfg(feature = "postgres")]
pub(super) fn job_failure_reason(ctx: &JobContext) -> Option<String> {
    if matches!(
        ctx.state,
        JobState::Failed | JobState::Stuck | JobState::Cancelled | JobState::Abandoned
    ) {
        ctx.transitions
            .last()
            .and_then(|transition| transition.reason.clone())
    } else {
        None
    }
}

/// Database store for the agent.
#[cfg(feature = "postgres")]
pub struct Store {
    pool: Pool,
}

#[cfg(feature = "postgres")]
impl Store {
    /// Wrap an existing pool (useful when the caller already has a connection).
    #[allow(dead_code)]
    pub fn from_pool(pool: Pool) -> Self {
        Self { pool }
    }

    /// Create a new store and connect to the database.
    pub async fn new<C>(config: &C) -> Result<Self, DatabaseError>
    where
        C: PgBackendConfig + ?Sized,
    {
        let mut cfg = Config::new();
        cfg.url = Some(config.postgres_url().to_string());
        cfg.pool = Some(deadpool_postgres::PoolConfig {
            max_size: config.postgres_pool_size(),
            ..Default::default()
        });

        let pool = {
            // Try TLS first ("prefer" semantics) — uses system CA roots.
            // Falls back to NoTls if TLS negotiation fails (e.g. local dev PG without certs).
            let tls_result = (|| -> Result<_, Box<dyn std::error::Error>> {
                ensure_rustls_crypto_provider();
                let certs = rustls_native_certs::load_native_certs();
                let mut root_store = rustls::RootCertStore::empty();
                for cert in certs.certs {
                    root_store.add(cert)?;
                }
                let tls_config = rustls::ClientConfig::builder()
                    .with_root_certificates(root_store)
                    .with_no_client_auth();
                let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);
                Ok(cfg.create_pool(Some(Runtime::Tokio1), tls)?)
            })();

            match tls_result {
                Ok(pool) => {
                    tracing::debug!("PostgreSQL pool created with TLS (prefer mode)");
                    pool
                }
                Err(tls_err) => {
                    tracing::debug!("TLS pool creation failed ({tls_err}), falling back to NoTls");
                    cfg.create_pool(Some(Runtime::Tokio1), NoTls)
                        .map_err(|e| DatabaseError::Pool(e.to_string()))?
                }
            }
        };

        // Test connection
        let _ = pool.get().await?;

        Ok(Self { pool })
    }

    /// Run database migrations (embedded via refinery).
    pub async fn run_migrations(&self) -> Result<(), DatabaseError> {
        #[cfg(debug_assertions)]
        self.assert_refinery_migration_order()?;

        use refinery::embed_migrations;
        embed_migrations!("../../migrations");

        let mut client = self.pool.get().await?;
        migrations::runner()
            .run_async(&mut **client)
            .await
            .map_err(|e| DatabaseError::Migration(e.to_string()))?;
        Ok(())
    }

    #[cfg(debug_assertions)]
    fn assert_refinery_migration_order(&self) -> Result<(), DatabaseError> {
        let mut migration_versions = Vec::new();
        let migrations_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");

        let mut seen = std::collections::HashSet::new();

        for entry in std::fs::read_dir(&migrations_dir).map_err(|e| {
            DatabaseError::Migration(format!(
                "Cannot read migrations directory {:?}: {e}",
                migrations_dir
            ))
        })? {
            let entry = entry.map_err(|e| {
                DatabaseError::Migration(format!(
                    "Failed to iterate migrations directory {:?}: {e}",
                    migrations_dir
                ))
            })?;

            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("sql") {
                continue;
            }

            let file_name = match path.file_name().and_then(|name| name.to_str()) {
                Some(name) => name,
                None => {
                    return Err(DatabaseError::Migration(format!(
                        "Non-UTF8 migration filename in {:?}",
                        path
                    )));
                }
            };

            let version = match parse_migration_version(file_name) {
                Some(version) => version,
                None => {
                    return Err(DatabaseError::Migration(format!(
                        "Migration filename must be versioned as V<version>__*.sql: {file_name}"
                    )));
                }
            };

            if !seen.insert(version) {
                return Err(DatabaseError::Migration(format!(
                    "Duplicate migration version detected in postgres migrations: {version}"
                )));
            }

            migration_versions.push(version);
        }

        migration_versions.sort_unstable();

        if let Some(previous) = migration_versions.windows(2).find_map(|pair| {
            if pair[0] >= pair[1] {
                Some((pair[0], pair[1]))
            } else {
                None
            }
        }) {
            return Err(DatabaseError::Migration(format!(
                "PostgreSQL migration versions are not strictly increasing: {} -> {}",
                previous.0, previous.1
            )));
        }

        Ok(())
    }

    /// Get a connection from the pool.
    pub async fn conn(&self) -> Result<deadpool_postgres::Object, DatabaseError> {
        Ok(self.pool.get().await?)
    }

    /// Get a clone of the database pool.
    ///
    /// Useful for sharing the pool with other components like Workspace.
    pub fn pool(&self) -> Pool {
        self.pool.clone()
    }
}
