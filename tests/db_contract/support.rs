use std::sync::Arc;

use thinclaw::db::Database;
use uuid::Uuid;

pub(crate) struct ContractDb {
    pub db: Arc<dyn Database>,
    #[cfg(feature = "libsql")]
    _temp_dir: Option<tempfile::TempDir>,
    #[cfg(feature = "postgres")]
    _postgres_schema: Option<String>,
}

pub(crate) fn unique_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

pub(crate) async fn contract_db_or_skip() -> Option<ContractDb> {
    let backend = std::env::var("DATABASE_BACKEND").unwrap_or_else(|_| {
        if cfg!(feature = "libsql") {
            "libsql".to_string()
        } else {
            "postgres".to_string()
        }
    });

    match backend.as_str() {
        "postgres" => connect_postgres().await,
        "libsql" => connect_libsql().await,
        other => {
            eprintln!("skipping db contract test: unsupported DATABASE_BACKEND={other}");
            None
        }
    }
}

#[cfg(feature = "libsql")]
async fn connect_libsql() -> Option<ContractDb> {
    use thinclaw::db::libsql::LibSqlBackend;

    let dir = tempfile::tempdir().ok()?;
    let path = dir.path().join("contract.db");
    let backend = match LibSqlBackend::new_local(&path).await {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("skipping db contract test: libsql open failed: {err}");
            return None;
        }
    };

    if let Err(err) = backend.run_migrations().await {
        eprintln!("skipping db contract test: libsql migrations failed: {err}");
        return None;
    }

    Some(ContractDb {
        db: Arc::new(backend),
        _temp_dir: Some(dir),
        #[cfg(feature = "postgres")]
        _postgres_schema: None,
    })
}

#[cfg(not(feature = "libsql"))]
async fn connect_libsql() -> Option<ContractDb> {
    eprintln!("skipping db contract test: binary was built without libsql feature");
    None
}

#[cfg(feature = "postgres")]
async fn connect_postgres() -> Option<ContractDb> {
    use secrecy::SecretString;
    use thinclaw::config::{DatabaseBackend, DatabaseConfig};
    use thinclaw::db::postgres::PgBackend;

    let base_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("skipping db contract test: DATABASE_URL is not set");
            return None;
        }
    };

    let schema = format!("contract_{}", Uuid::new_v4().simple());
    if let Err(err) = create_postgres_schema(&base_url, &schema).await {
        eprintln!("skipping db contract test: cannot create schema {schema}: {err}");
        return None;
    }

    let isolated_url = match postgres_url_with_search_path(&base_url, &schema) {
        Ok(url) => url,
        Err(err) => {
            eprintln!("skipping db contract test: cannot build postgres URL: {err}");
            return None;
        }
    };

    let config = DatabaseConfig {
        backend: DatabaseBackend::Postgres,
        url: SecretString::from(isolated_url),
        pool_size: 4,
        libsql_path: None,
        libsql_url: None,
        libsql_auth_token: None,
    };

    let backend = match PgBackend::new(&config).await {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("skipping db contract test: postgres connect failed: {err}");
            return None;
        }
    };

    if let Err(err) = backend.run_migrations().await {
        eprintln!("skipping db contract test: postgres migrations failed: {err}");
        return None;
    }

    Some(ContractDb {
        db: Arc::new(backend),
        #[cfg(feature = "libsql")]
        _temp_dir: None,
        _postgres_schema: Some(schema),
    })
}

#[cfg(not(feature = "postgres"))]
async fn connect_postgres() -> Option<ContractDb> {
    eprintln!("skipping db contract test: binary was built without postgres feature");
    None
}

#[cfg(feature = "postgres")]
async fn create_postgres_schema(base_url: &str, schema: &str) -> Result<(), String> {
    use tokio_postgres::NoTls;

    let cfg: tokio_postgres::Config = base_url
        .parse()
        .map_err(|e| format!("invalid DATABASE_URL: {e}"))?;
    let (client, connection) = cfg
        .connect(NoTls)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            eprintln!("postgres admin connection ended: {err}");
        }
    });

    // Generated schema names are UUID-based and contain only [a-z0-9_].
    let stmt = format!("CREATE SCHEMA IF NOT EXISTS {schema}");
    client
        .batch_execute(&stmt)
        .await
        .map_err(|e| format!("CREATE SCHEMA failed: {e}"))?;
    Ok(())
}

#[cfg(feature = "postgres")]
fn postgres_url_with_search_path(base_url: &str, schema: &str) -> Result<String, String> {
    let mut url = url::Url::parse(base_url).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema},public"));
    Ok(url.to_string())
}
