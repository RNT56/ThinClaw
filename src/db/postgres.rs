//! Compatibility facade for the PostgreSQL backend.

pub use thinclaw_db::postgres::*;

impl PgBackendConfig for crate::config::DatabaseConfig {
    fn postgres_url(&self) -> &str {
        self.url()
    }

    fn postgres_pool_size(&self) -> usize {
        self.pool_size
    }
}
