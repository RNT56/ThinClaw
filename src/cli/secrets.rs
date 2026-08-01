//! Commands for managing encrypted ThinClaw secrets.

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, Subcommand};
use secrecy::{ExposeSecret, SecretString};

use crate::config::Config;
use crate::secrets::{CreateSecretParams, SecretsCrypto, SecretsStore};
use crate::terminal_branding::TerminalBranding;

#[derive(Subcommand, Debug, Clone)]
pub enum SecretsCommand {
    /// Show secure-store and local encrypted-store status
    Status,
    /// List stored secret metadata without values
    List {
        /// User/principal id
        #[arg(long, default_value = "default")]
        user: String,
    },
    /// Store or replace one secret value
    Set(SecretSetCommand),
    /// Delete one secret
    Delete {
        /// Secret name
        name: String,
        /// User/principal id
        #[arg(long, default_value = "default")]
        user: String,
        /// Confirm deletion noninteractively
        #[arg(long)]
        yes: bool,
    },
    /// Rotate the local master key for future writes
    RotateMaster,
}

#[derive(Args, Debug, Clone)]
pub struct SecretSetCommand {
    /// Secret name
    pub name: String,
    /// Read the secret from standard input
    #[arg(long, conflicts_with_all = ["from_env", "from_file"])]
    pub from_stdin: bool,
    /// Read the secret from this environment variable
    #[arg(long, value_name = "VAR", conflicts_with_all = ["from_stdin", "from_file"])]
    pub from_env: Option<String>,
    /// Read the secret from a private regular file
    #[arg(long, value_name = "FILE", conflicts_with_all = ["from_stdin", "from_env"])]
    pub from_file: Option<PathBuf>,
    /// Optional provider label
    #[arg(long)]
    pub provider: Option<String>,
    /// User/principal id
    #[arg(long, default_value = "default")]
    pub user: String,
}

pub async fn run_secrets_command(cmd: SecretsCommand) -> anyhow::Result<()> {
    match cmd {
        SecretsCommand::Status => status().await,
        SecretsCommand::List { user } => list(&user).await,
        SecretsCommand::Set(args) => set(args).await,
        SecretsCommand::Delete { name, user, yes } => delete(&user, &name, yes).await,
        SecretsCommand::RotateMaster => rotate_master().await,
    }
}

async fn status() -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    branding.print_banner("ThinClaw Secrets", Some("Local encrypted secrets posture"));

    let probe = crate::platform::secure_store::probe_availability().await;
    let env_present = std::env::var_os("SECRETS_MASTER_KEY").is_some();
    let env_allowed = std::env::var("THINCLAW_ALLOW_ENV_MASTER_KEY")
        .ok()
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);

    println!(
        "{}",
        branding.key_value(
            "OS secure store",
            if probe.available {
                probe.detail
            } else {
                format!("unavailable: {} ({})", probe.detail, probe.guidance)
            }
        )
    );
    println!(
        "{}",
        branding.key_value(
            "Env fallback",
            if env_present && env_allowed {
                "explicitly allowed".to_string()
            } else if env_present {
                "present but ignored by strict defaults".to_string()
            } else {
                "not configured".to_string()
            }
        )
    );
    println!(
        "{}",
        branding.key_value("Local backend", "local_encrypted v2")
    );
    Ok(())
}

async fn list(user: &str) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let store = get_secrets_store().await?;
    let refs = store.list(user).await?;
    branding.print_banner("ThinClaw Secrets", Some("Stored metadata, no values"));
    for secret in refs {
        println!(
            "{}",
            branding.key_value(
                &secret.name,
                format!(
                    "source={} provider={} version={} key_version={} used={}",
                    secret
                        .id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "legacy-name-only".to_string()),
                    secret.provider.unwrap_or_else(|| "unknown".to_string()),
                    secret.encryption_version,
                    secret.key_version,
                    secret.usage_count
                )
            )
        );
    }
    Ok(())
}

async fn set(args: SecretSetCommand) -> anyhow::Result<()> {
    let value = resolve_secret_input(&args)?;
    validate_cli_secret_value(&value)?;
    let store = get_secrets_store().await?;
    let mut params =
        CreateSecretParams::new(args.name.clone(), value).with_created_by("cli.secrets.set");
    if let Some(provider) = args.provider {
        params = params.with_provider(provider);
    }
    let stored = store.create(&args.user, params).await?;
    let branding = TerminalBranding::current();
    println!(
        "{}",
        branding.good(format!(
            "Secret '{}' saved as source {}.",
            args.name, stored.id
        ))
    );
    Ok(())
}

async fn delete(user: &str, name: &str, yes: bool) -> anyhow::Result<()> {
    confirm_secret_deletion(name, yes)?;
    let store = get_secrets_store().await?;
    let deleted = store.delete(user, name).await?;
    let branding = TerminalBranding::current();
    if deleted {
        println!("{}", branding.good(format!("Secret '{name}' deleted.")));
    } else {
        println!(
            "{}",
            branding.warn(format!("Secret '{name}' was not present."))
        );
    }
    Ok(())
}

fn resolve_secret_input(args: &SecretSetCommand) -> anyhow::Result<String> {
    const MAX_SECRET_BYTES: u64 = 1024 * 1024;
    if args.from_stdin {
        let mut value = String::new();
        std::io::stdin()
            .take(MAX_SECRET_BYTES + 1)
            .read_to_string(&mut value)?;
        if value.len() as u64 > MAX_SECRET_BYTES {
            anyhow::bail!("secret input exceeds 1 MiB");
        }
        return Ok(value.trim_end_matches(['\r', '\n']).to_string());
    }
    if let Some(variable) = args.from_env.as_deref() {
        if variable.is_empty()
            || variable.len() > 128
            || !variable
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            anyhow::bail!("environment variable name is invalid");
        }
        return std::env::var(variable)
            .map_err(|_| anyhow::anyhow!("environment variable {variable} is unavailable"));
    }
    if let Some(path) = args.from_file.as_deref() {
        if !path.is_absolute() {
            anyhow::bail!("--from-file requires an absolute path");
        }
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_SECRET_BYTES
        {
            anyhow::bail!("secret source must be a regular non-symlink file no larger than 1 MiB");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.mode() & 0o077 != 0 {
                anyhow::bail!("secret source file must not grant group or other permissions");
            }
        }
        let bytes =
            thinclaw_platform::read_regular_file_bounded_single_link(path, MAX_SECRET_BYTES)?;
        return String::from_utf8(bytes)
            .map(|value| value.trim_end_matches(['\r', '\n']).to_string())
            .map_err(|_| anyhow::anyhow!("secret source file is not UTF-8"));
    }
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("secure input is unavailable; use --from-stdin, --from-env, or --from-file");
    }
    Ok(crate::setup::secret_input("Secret value")?
        .expose_secret()
        .to_string())
}

fn confirm_secret_deletion(name: &str, yes: bool) -> anyhow::Result<()> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("secret deletion requires --yes in noninteractive mode");
    }
    eprint!("Delete secret source '{name}'? [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        anyhow::bail!("secret deletion cancelled");
    }
    Ok(())
}

async fn rotate_master() -> anyhow::Result<()> {
    let store = get_secrets_store().await?;
    let old_key = crate::platform::secure_store::get_master_key()
        .await
        .map_err(|e| {
            anyhow::anyhow!("rotate-master requires an OS secure-store master key: {e}")
        })?;
    let key = crate::platform::secure_store::generate_master_key();
    let key_hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    let new_crypto = Arc::new(SecretsCrypto::new(SecretString::from(key_hex))?);
    crate::platform::secure_store::store_master_key(&key).await?;
    let report = match store.rotate_master_key(new_crypto).await {
        Ok(report) => report,
        Err(error) => {
            let restore_result = crate::platform::secure_store::store_master_key(&old_key).await;
            if let Err(restore_error) = restore_result {
                anyhow::bail!(
                    "master-key rotation failed ({error}) and the old OS secure-store key could not be restored ({restore_error})"
                );
            }
            return Err(error.into());
        }
    };
    let branding = TerminalBranding::current();
    println!(
        "{}",
        branding.good(format!(
            "Rotated local master key from version {} to {}.",
            report.old_key_version, report.new_key_version
        ))
    );
    println!(
        "{}",
        branding.key_value(
            "Re-encrypted active secrets",
            report.rotated_secrets.to_string()
        )
    );
    Ok(())
}

fn validate_cli_secret_value(value: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("secret value cannot be empty");
    }
    if value
        .chars()
        .any(|ch| ch.is_control() || ch == '\n' || ch == '\r')
    {
        anyhow::bail!("secret value must be a single line without control characters");
    }
    Ok(())
}

pub(crate) async fn get_secrets_store() -> anyhow::Result<Arc<dyn SecretsStore + Send + Sync>> {
    let config = Config::from_env().await?;
    let master_key = config
        .secrets
        .master_key()
        .ok_or_else(|| anyhow::anyhow!("secrets are not configured; run `thinclaw onboard`"))?;
    let crypto = Arc::new(SecretsCrypto::new(SecretString::from(
        master_key.expose_secret().to_string(),
    ))?);

    #[cfg(feature = "libsql")]
    if config.database.backend == crate::config::DatabaseBackend::LibSql {
        use crate::db::Database as _;
        use crate::db::libsql::LibSqlBackend;
        use secrecy::ExposeSecret as _;

        let default_path = crate::config::default_libsql_path();
        let db_path = config
            .database
            .libsql_path
            .as_deref()
            .unwrap_or(&default_path);
        let backend = if let Some(ref url) = config.database.libsql_url {
            let token = config.database.libsql_auth_token.as_ref().ok_or_else(|| {
                anyhow::anyhow!("LIBSQL_AUTH_TOKEN is required when LIBSQL_URL is set")
            })?;
            LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?
        } else {
            LibSqlBackend::new_local(db_path)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?
        };
        backend
            .run_migrations()
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        return Ok(Arc::new(crate::secrets::LibSqlSecretsStore::new(
            backend.shared_db(),
            crypto,
        )));
    }

    #[cfg(feature = "postgres")]
    {
        use crate::db::Database as _;
        let store = crate::db::postgres::PgBackend::new(&config.database).await?;
        store.run_migrations().await?;
        Ok(Arc::new(crate::secrets::PostgresSecretsStore::new(
            store.pool(),
            crypto,
        )))
    }

    #[cfg(not(feature = "postgres"))]
    {
        anyhow::bail!("No database backend available for secrets.");
    }
}
