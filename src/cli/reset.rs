//! Full ThinClaw runtime reset command.
//!
//! This command is intentionally destructive: it clears ThinClaw-owned
//! database state, removes the local `~/.thinclaw` runtime directory, and
//! deletes ThinClaw-managed OS secure-store entries so onboarding can start cleanly.

#[cfg(feature = "postgres")]
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::Context;
use clap::Args;

use crate::config::{DatabaseBackend, DatabaseConfig};
use crate::db::Database;
use crate::platform::secure_store::{
    CLAUDE_CODE_API_KEY_ACCOUNT, delete_api_key, delete_master_key, get_api_key, has_master_key,
};
use crate::setup::{confirm, print_warning};
use crate::terminal_branding::TerminalBranding;

#[derive(Args, Debug, Clone)]
pub struct ResetCommand {
    /// Skip interactive confirmation prompts
    #[arg(long)]
    pub yes: bool,
}

#[cfg(feature = "libsql")]
const SQLITE_RESET_TABLES: &[&str] = &[
    "job_actions",
    "job_events",
    "estimation_snapshots",
    "llm_calls",
    "routine_runs",
    "conversation_messages",
    "secret_usage_log",
    "tool_rate_limit_state",
    "leak_detection_events",
    "tool_capabilities",
    "memory_chunks",
    "actor_endpoints",
    "agent_jobs",
    "routines",
    "conversations",
    "secrets",
    "wasm_tools",
    "memory_documents",
    "actors",
    "dynamic_tools",
    "repair_attempts",
    "heartbeat_state",
    "tool_failures",
    "settings",
    "agent_workspaces",
];

#[cfg(feature = "postgres")]
const POSTGRES_RESET_TABLES: &[&str] = &[
    "conversation_messages",
    "conversations",
    "job_actions",
    "job_events",
    "llm_calls",
    "estimation_snapshots",
    "agent_jobs",
    "dynamic_tools",
    "repair_attempts",
    "memory_chunks",
    "memory_documents",
    "heartbeat_state",
    "secrets",
    "secret_usage_log",
    "tool_rate_limit_state",
    "leak_detection_events",
    "tool_capabilities",
    "wasm_tools",
    "tool_failures",
    "routine_runs",
    "routines",
    "settings",
    "actor_endpoints",
    "actors",
    "agent_workspaces",
];

pub async fn run_reset_command(cmd: ResetCommand) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    branding.print_banner("Reset", Some("Clear ThinClaw runtime state"));
    print_warning(
        "Full reset deletes ThinClaw state from the configured database, ~/.thinclaw, and ThinClaw-managed OS secure-store entries.",
    );
    println!(
        "{}",
        branding.body(
            "It does not uninstall the ThinClaw binary or remove launchd/systemd/Windows service definitions."
        )
    );
    println!(
        "{}",
        branding.body(
            "If a service is running, stop it first so it does not recreate state during the reset."
        )
    );
    println!();

    if !cmd.yes {
        confirm_full_reset()?;
    }

    let db_result = match DatabaseConfig::resolve() {
        Ok(config) => reset_database(&config).await?,
        Err(err) => ResetStepResult::Skipped(format!(
            "database reset skipped because no configured database was found ({err})"
        )),
    };

    let local_result = reset_local_state(&thinclaw_home_dir())?;
    let secure_store_result = reset_secure_store_entries().await;

    println!("{}", branding.good("ThinClaw reset complete."));
    println!("{}", branding.key_value("Database", db_result.describe()));
    println!(
        "{}",
        branding.key_value("Local state", local_result.describe())
    );
    println!(
        "{}",
        branding.key_value("OS secure store", secure_store_result.describe())
    );
    println!();
    println!(
        "{}",
        branding.muted("Next step: run `thinclaw onboard` to set ThinClaw up again.")
    );

    Ok(())
}

fn confirm_full_reset() -> anyhow::Result<()> {
    if !confirm("Proceed with a full ThinClaw reset?", false)? {
        anyhow::bail!("Reset cancelled.");
    }

    print!("Type RESET to continue: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    if input.trim() != "RESET" {
        anyhow::bail!("Reset cancelled (confirmation phrase did not match).");
    }

    Ok(())
}

fn thinclaw_home_dir() -> PathBuf {
    crate::platform::resolve_thinclaw_home()
}

async fn reset_database(config: &DatabaseConfig) -> anyhow::Result<ResetStepResult> {
    match config.backend {
        #[cfg(feature = "libsql")]
        DatabaseBackend::LibSql => reset_libsql_database(config).await,
        #[cfg(feature = "postgres")]
        DatabaseBackend::Postgres => reset_postgres_database(config).await,
        #[cfg(not(feature = "postgres"))]
        DatabaseBackend::Postgres => Ok(ResetStepResult::Skipped(
            "postgres reset skipped because this build does not include the postgres feature"
                .to_string(),
        )),
        #[cfg(not(feature = "libsql"))]
        DatabaseBackend::LibSql => Ok(ResetStepResult::Skipped(
            "libsql reset skipped because this build does not include the libsql feature"
                .to_string(),
        )),
    }
}

#[cfg(feature = "postgres")]
async fn reset_postgres_database(config: &DatabaseConfig) -> anyhow::Result<ResetStepResult> {
    let backend = crate::db::postgres::PgBackend::new(config)
        .await
        .context("failed to connect to PostgreSQL for reset")?;
    backend
        .run_migrations()
        .await
        .context("failed to run PostgreSQL migrations before reset")?;

    let client = backend
        .pool()
        .get()
        .await
        .context("failed to acquire PostgreSQL connection for reset")?;

    let rows = client
        .query(
            "SELECT tablename FROM pg_tables WHERE schemaname = current_schema()",
            &[],
        )
        .await
        .context("failed to inspect PostgreSQL tables before reset")?;
    let existing: HashSet<String> = rows.into_iter().map(|row| row.get(0)).collect();

    let tables: Vec<&str> = POSTGRES_RESET_TABLES
        .iter()
        .copied()
        .filter(|table| existing.contains(*table))
        .collect();

    if tables.is_empty() {
        return Ok(ResetStepResult::Skipped(
            "no ThinClaw PostgreSQL tables were present".to_string(),
        ));
    }

    let joined_tables = tables
        .iter()
        .map(|table| format!("\"{table}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("TRUNCATE TABLE {joined_tables} RESTART IDENTITY CASCADE");

    client
        .execute(sql.as_str(), &[])
        .await
        .context("failed to clear ThinClaw PostgreSQL tables")?;

    Ok(ResetStepResult::Completed(format!(
        "cleared {} ThinClaw tables from PostgreSQL",
        tables.len()
    )))
}

#[cfg(feature = "libsql")]
async fn reset_libsql_database(config: &DatabaseConfig) -> anyhow::Result<ResetStepResult> {
    use secrecy::ExposeSecret as _;

    let default_path = crate::config::default_libsql_path();
    let db_path = config.libsql_path.as_deref().unwrap_or(&default_path);
    let backend = if let Some(ref url) = config.libsql_url {
        let token = config.libsql_auth_token.as_ref().ok_or_else(|| {
            anyhow::anyhow!("LIBSQL_AUTH_TOKEN is required when LIBSQL_URL is set")
        })?;
        crate::db::libsql::LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
            .await
            .context("failed to connect to libSQL remote replica for reset")?
    } else {
        crate::db::libsql::LibSqlBackend::new_local(db_path)
            .await
            .context("failed to open local libSQL database for reset")?
    };

    backend
        .run_migrations()
        .await
        .context("failed to run libSQL migrations before reset")?;

    let conn = backend
        .connect()
        .await
        .context("failed to open libSQL connection for reset")?;
    let tx = conn
        .transaction()
        .await
        .context("failed to start libSQL reset transaction")?;

    for table in SQLITE_RESET_TABLES {
        let sql = format!("DELETE FROM {table}");
        tx.execute(sql.as_str(), ())
            .await
            .with_context(|| format!("failed to clear libSQL table `{table}`"))?;
    }

    tx.execute("DELETE FROM sqlite_sequence WHERE name = 'job_events'", ())
        .await
        .context("failed to reset libSQL job event sequence")?;
    tx.commit()
        .await
        .context("failed to commit libSQL reset transaction")?;

    Ok(ResetStepResult::Completed(format!(
        "cleared {} ThinClaw tables from libSQL",
        SQLITE_RESET_TABLES.len()
    )))
}

fn reset_local_state(path: &std::path::Path) -> anyhow::Result<ResetStepResult> {
    if !path.exists() {
        return Ok(ResetStepResult::Skipped(format!(
            "{} did not exist",
            path.display()
        )));
    }

    std::fs::remove_dir_all(path)
        .with_context(|| format!("failed to remove {}", path.display()))?;
    Ok(ResetStepResult::Completed(format!(
        "removed {}",
        path.display()
    )))
}

async fn reset_secure_store_entries() -> ResetStepResult {
    let mut removed = Vec::new();
    let mut failures = Vec::new();

    if has_master_key().await {
        match delete_master_key().await {
            Ok(()) => removed.push("master_key".to_string()),
            Err(err) => failures.push(format!("master_key ({err})")),
        }
    }

    for account in known_secure_store_accounts() {
        if get_api_key(&account).await.is_none() {
            continue;
        }

        match delete_api_key(&account).await {
            Ok(()) => removed.push(account),
            Err(err) => failures.push(format!("{account} ({err})")),
        }
    }

    match (removed.is_empty(), failures.is_empty()) {
        (true, true) => ResetStepResult::Skipped(
            "no ThinClaw-managed OS secure-store entries were present".to_string(),
        ),
        (_, true) => ResetStepResult::Completed(format!(
            "removed {} ThinClaw-managed OS secure-store entr{}",
            removed.len(),
            if removed.len() == 1 { "y" } else { "ies" }
        )),
        (_, false) => ResetStepResult::CompletedWithWarnings(format!(
            "removed {} entr{}; some entries could not be deleted: {}",
            removed.len(),
            if removed.len() == 1 { "y" } else { "ies" },
            failures.join(", ")
        )),
    }
}

fn known_secure_store_accounts() -> Vec<String> {
    let mut accounts: Vec<String> = crate::config::provider_catalog::catalog()
        .values()
        .map(|endpoint| endpoint.secret_name.to_string())
        .collect();
    accounts.push(CLAUDE_CODE_API_KEY_ACCOUNT.to_string());
    accounts.sort();
    accounts.dedup();
    accounts
}

#[derive(Debug, Clone)]
enum ResetStepResult {
    Completed(String),
    CompletedWithWarnings(String),
    Skipped(String),
}

impl ResetStepResult {
    fn describe(&self) -> &str {
        match self {
            Self::Completed(message)
            | Self::CompletedWithWarnings(message)
            | Self::Skipped(message) => message,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn known_secure_store_accounts_include_claude_key_and_are_unique() {
        let accounts = known_secure_store_accounts();
        assert!(accounts.contains(&CLAUDE_CODE_API_KEY_ACCOUNT.to_string()));

        let deduped: HashSet<String> = accounts.iter().cloned().collect();
        assert_eq!(deduped.len(), accounts.len());
    }

    #[test]
    fn thinclaw_home_dir_resolves_to_dot_thinclaw() {
        let path = thinclaw_home_dir();
        assert!(path.ends_with(".thinclaw"));
    }
}
