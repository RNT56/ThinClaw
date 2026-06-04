//! Database configuration.

use std::path::PathBuf;

use secrecy::{ExposeSecret, SecretString};
use thinclaw_platform::resolve_data_dir;
use thinclaw_types::error::ConfigError;

use crate::helpers::{optional_env, parse_optional_env};

/// Which database backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DatabaseBackend {
    /// PostgreSQL via deadpool-postgres (default).
    #[default]
    Postgres,
    /// libSQL/Turso embedded database.
    LibSql,
}

impl std::fmt::Display for DatabaseBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Postgres => write!(f, "postgres"),
            Self::LibSql => write!(f, "libsql"),
        }
    }
}

impl std::str::FromStr for DatabaseBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "postgres" | "postgresql" | "pg" => Ok(Self::Postgres),
            "libsql" | "turso" | "sqlite" => Ok(Self::LibSql),
            _ => Err(format!(
                "invalid database backend '{}', expected 'postgres' or 'libsql'",
                s
            )),
        }
    }
}

/// Database configuration.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// Which backend to use (default: Postgres).
    pub backend: DatabaseBackend,
    /// PostgreSQL URL.
    pub url: SecretString,
    /// PostgreSQL connection pool size.
    pub pool_size: usize,
    /// Path to local libSQL database file (default: ~/.thinclaw/thinclaw.db).
    pub libsql_path: Option<PathBuf>,
    /// Turso cloud URL for remote sync (optional).
    pub libsql_url: Option<String>,
    /// Turso auth token (required when libsql_url is set).
    pub libsql_auth_token: Option<SecretString>,
}

impl DatabaseConfig {
    pub fn disabled() -> Self {
        let backend = std::env::var("DATABASE_BACKEND")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DatabaseBackend::LibSql);

        Self {
            backend,
            url: SecretString::from("disabled://no-db"),
            pool_size: 0,
            libsql_path: if backend == DatabaseBackend::LibSql {
                Some(default_libsql_path())
            } else {
                None
            },
            libsql_url: None,
            libsql_auth_token: None,
        }
    }

    pub fn resolve() -> Result<Self, ConfigError> {
        let backend: DatabaseBackend = if let Some(b) = optional_env("DATABASE_BACKEND")? {
            b.parse().map_err(|e| ConfigError::InvalidValue {
                key: "DATABASE_BACKEND".to_string(),
                message: e,
            })?
        } else {
            DatabaseBackend::default()
        };

        let url = optional_env("DATABASE_URL")?
            .or_else(|| {
                if backend == DatabaseBackend::LibSql {
                    Some("unused://libsql".to_string())
                } else {
                    None
                }
            })
            .ok_or_else(|| ConfigError::MissingRequired {
                key: "DATABASE_URL".to_string(),
                hint: "Run 'thinclaw onboard' or set DATABASE_URL environment variable".to_string(),
            })?;

        let pool_size = parse_optional_env("DATABASE_POOL_SIZE", 10)?;

        let libsql_path = optional_env("LIBSQL_PATH")?.map(PathBuf::from).or_else(|| {
            if backend == DatabaseBackend::LibSql {
                Some(default_libsql_path())
            } else {
                None
            }
        });

        let libsql_url = optional_env("LIBSQL_URL")?;
        let libsql_auth_token = optional_env("LIBSQL_AUTH_TOKEN")?.map(SecretString::from);

        if libsql_url.is_some() && libsql_auth_token.is_none() {
            return Err(ConfigError::MissingRequired {
                key: "LIBSQL_AUTH_TOKEN".to_string(),
                hint: "LIBSQL_AUTH_TOKEN is required when LIBSQL_URL is set".to_string(),
            });
        }

        Ok(Self {
            backend,
            url: SecretString::from(url),
            pool_size,
            libsql_path,
            libsql_url,
            libsql_auth_token,
        })
    }

    /// Get the database URL (exposes the secret).
    pub fn url(&self) -> &str {
        self.url.expose_secret()
    }
}

/// Default libSQL database path (~/.thinclaw/thinclaw.db).
pub fn default_libsql_path() -> PathBuf {
    resolve_data_dir("thinclaw.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::lock_env;

    fn clear_database_env() {
        unsafe {
            std::env::remove_var("DATABASE_BACKEND");
            std::env::remove_var("DATABASE_URL");
            std::env::remove_var("DATABASE_POOL_SIZE");
            std::env::remove_var("LIBSQL_PATH");
            std::env::remove_var("LIBSQL_URL");
            std::env::remove_var("LIBSQL_AUTH_TOKEN");
        }
    }

    #[test]
    fn backend_parses_aliases() {
        assert_eq!("postgresql".parse(), Ok(DatabaseBackend::Postgres));
        assert_eq!("sqlite".parse(), Ok(DatabaseBackend::LibSql));
        assert!("unknown".parse::<DatabaseBackend>().is_err());
    }

    #[test]
    fn resolve_libsql_supplies_placeholder_url_and_path() {
        let _guard = lock_env();
        clear_database_env();
        unsafe {
            std::env::set_var("DATABASE_BACKEND", "libsql");
        }

        let cfg = DatabaseConfig::resolve().expect("libsql config");
        assert_eq!(cfg.backend, DatabaseBackend::LibSql);
        assert_eq!(cfg.url(), "unused://libsql");
        assert!(cfg.libsql_path.is_some());

        clear_database_env();
    }

    #[test]
    fn resolve_remote_libsql_requires_token() {
        let _guard = lock_env();
        clear_database_env();
        unsafe {
            std::env::set_var("DATABASE_BACKEND", "libsql");
            std::env::set_var("LIBSQL_URL", "libsql://example.turso.io");
        }

        let err = DatabaseConfig::resolve().expect_err("missing token");
        assert!(err.to_string().contains("LIBSQL_AUTH_TOKEN"));

        clear_database_env();
    }
}
