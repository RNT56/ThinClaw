//! Cron / routine management CLI commands.
//!
//! Subcommands:
//! - `cron list` — list all routines
//! - `cron add` — create a new lightweight routine
//! - `cron remove` — delete a routine by UUID or name
//! - `cron trigger` — manually trigger a routine
//! - `cron runs` — show recent runs for a routine

use std::sync::Arc;

use clap::Subcommand;
use uuid::Uuid;

#[derive(Subcommand, Debug, Clone)]
pub enum CronCommand {
    /// List all routines
    List {
        /// Output format: table (default) or json
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// Create a new lightweight routine
    Add {
        /// Routine name (must be unique per user)
        name: String,

        /// Cron schedule (e.g. "0 9 * * MON-FRI" or "0 */2 * * *")
        #[arg(short, long)]
        schedule: String,

        /// Prompt to send to the LLM when triggered
        #[arg(short, long)]
        prompt: String,

        /// Optional description
        #[arg(short, long)]
        description: Option<String>,
    },

    /// Delete a routine by UUID or name
    Remove {
        /// Routine UUID or name
        id_or_name: String,
    },

    /// Manually trigger a routine
    Trigger {
        /// Routine UUID or name
        id_or_name: String,
    },

    /// Show recent runs for a routine
    Runs {
        /// Routine UUID or name
        id_or_name: String,

        /// Number of runs to show (default: 10)
        #[arg(short = 'n', long, default_value = "10")]
        limit: i64,
    },
}

/// Run a cron CLI command.
pub async fn run_cron_command(cmd: CronCommand) -> anyhow::Result<()> {
    let db = connect_db().await?;

    match cmd {
        CronCommand::List { format } => list_routines(&*db, &format).await,
        CronCommand::Add {
            name,
            schedule,
            prompt,
            description,
        } => add_routine(&*db, name, schedule, prompt, description).await,
        CronCommand::Remove { id_or_name } => remove_routine(&*db, &id_or_name).await,
        CronCommand::Trigger { id_or_name } => trigger_routine(&*db, &id_or_name).await,
        CronCommand::Runs { id_or_name, limit } => show_runs(&*db, &id_or_name, limit).await,
    }
}

const DEFAULT_USER_ID: &str = "default";

/// Bootstrap a DB connection.
async fn connect_db() -> anyhow::Result<Arc<dyn crate::db::Database>> {
    let config = crate::config::Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))
}

/// Resolve a routine by UUID or name.
async fn resolve_routine(
    db: &dyn crate::db::Database,
    id_or_name: &str,
) -> anyhow::Result<crate::agent::routine::Routine> {
    // Try UUID first
    if let Ok(id) = Uuid::parse_str(id_or_name) {
        if let Some(r) = db
            .get_routine(id)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?
        {
            return Ok(r);
        }
    }

    // Try by name
    if let Some(r) = db
        .get_routine_by_name(DEFAULT_USER_ID, id_or_name)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?
    {
        return Ok(r);
    }

    anyhow::bail!("Routine not found: '{}'", id_or_name)
}

/// List all routines.
async fn list_routines(db: &dyn crate::db::Database, format: &str) -> anyhow::Result<()> {
    let routines = db
        .list_routines(DEFAULT_USER_ID)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    if routines.is_empty() {
        println!("No routines found.");
        println!(
            "Create one with: ironclaw cron add <name> --schedule '<cron>' --prompt '<prompt>'"
        );
        return Ok(());
    }

    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&routines)?);
        return Ok(());
    }

    // Table format
    println!(
        "{:<36}  {:<20}  {:<8}  {:<15}  {:<6}  {}",
        "ID", "NAME", "ENABLED", "TRIGGER", "RUNS", "DESCRIPTION"
    );
    println!("{}", "-".repeat(120));

    for r in &routines {
        let trigger_info = match &r.trigger {
            crate::agent::routine::Trigger::Cron { schedule } => {
                if schedule.len() > 13 {
                    format!("{}…", &schedule[..12])
                } else {
                    schedule.clone()
                }
            }
            crate::agent::routine::Trigger::Event { pattern, .. } => {
                format!("event:{}", &pattern[..pattern.len().min(8)])
            }
            crate::agent::routine::Trigger::Webhook { .. } => "webhook".to_string(),
            crate::agent::routine::Trigger::Manual => "manual".to_string(),
        };

        let desc = if r.description.len() > 30 {
            format!("{}…", &r.description[..29])
        } else {
            r.description.clone()
        };

        println!(
            "{:<36}  {:<20}  {:<8}  {:<15}  {:<6}  {}",
            r.id,
            &r.name[..r.name.len().min(20)],
            if r.enabled { "✅" } else { "⏸" },
            trigger_info,
            r.run_count,
            desc,
        );
    }

    println!("\n{} routine(s) total.", routines.len());
    Ok(())
}

/// Add a new lightweight routine.
async fn add_routine(
    db: &dyn crate::db::Database,
    name: String,
    schedule: String,
    prompt: String,
    description: Option<String>,
) -> anyhow::Result<()> {
    // Validate cron schedule
    crate::agent::routine::next_cron_fire(&schedule)
        .map_err(|e| anyhow::anyhow!("Invalid cron schedule: {}", e))?;

    let next_fire = crate::agent::routine::next_cron_fire(&schedule)?;

    let routine = crate::agent::routine::Routine {
        id: Uuid::new_v4(),
        name: name.clone(),
        description: description.unwrap_or_default(),
        user_id: DEFAULT_USER_ID.to_string(),
        enabled: true,
        trigger: crate::agent::routine::Trigger::Cron { schedule },
        action: crate::agent::routine::RoutineAction::Lightweight {
            prompt,
            context_paths: Vec::new(),
            max_tokens: 4096,
        },
        guardrails: crate::agent::routine::RoutineGuardrails::default(),
        notify: crate::agent::routine::NotifyConfig::default(),
        last_run_at: None,
        next_fire_at: next_fire,
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::json!({}),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    db.create_routine(&routine)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create routine: {}", e))?;

    println!("✅ Created routine '{}' ({})", name, routine.id);
    if let Some(next) = next_fire {
        println!("   Next fire: {}", next.format("%Y-%m-%d %H:%M:%S UTC"));
    }

    Ok(())
}

/// Remove a routine.
async fn remove_routine(db: &dyn crate::db::Database, id_or_name: &str) -> anyhow::Result<()> {
    let routine = resolve_routine(db, id_or_name).await?;

    let deleted = db
        .delete_routine(routine.id)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    if deleted {
        println!("✅ Deleted routine '{}' ({})", routine.name, routine.id);
    } else {
        println!("⚠️  Routine not found (may have been already deleted).");
    }

    Ok(())
}

/// Trigger a routine manually.
async fn trigger_routine(db: &dyn crate::db::Database, id_or_name: &str) -> anyhow::Result<()> {
    let routine = resolve_routine(db, id_or_name).await?;

    let prompt = match &routine.action {
        crate::agent::routine::RoutineAction::Lightweight { prompt, .. } => prompt.clone(),
        crate::agent::routine::RoutineAction::FullJob {
            title, description, ..
        } => {
            format!("{}: {}", title, description)
        }
    };

    println!("🔄 Triggering routine '{}' ({})", routine.name, routine.id);
    println!(
        "   Prompt: {}",
        if prompt.len() > 60 {
            format!("{}…", &prompt[..57])
        } else {
            prompt
        }
    );
    println!();
    println!("Note: Manual trigger via CLI logs the intent. For live execution,");
    println!(
        "use the gateway API: POST /api/routines/{}/trigger",
        routine.id
    );

    Ok(())
}

/// Show recent runs for a routine.
async fn show_runs(
    db: &dyn crate::db::Database,
    id_or_name: &str,
    limit: i64,
) -> anyhow::Result<()> {
    let routine = resolve_routine(db, id_or_name).await?;

    let runs = db
        .list_routine_runs(routine.id, limit)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    if runs.is_empty() {
        println!(
            "No runs found for routine '{}' ({}).",
            routine.name, routine.id
        );
        return Ok(());
    }

    println!(
        "Runs for '{}' ({}) — showing last {}:\n",
        routine.name,
        routine.id,
        runs.len()
    );
    println!(
        "{:<36}  {:<10}  {:<20}  {:<10}  {}",
        "RUN ID", "STATUS", "STARTED", "TOKENS", "SUMMARY"
    );
    println!("{}", "-".repeat(110));

    for run in &runs {
        let summary = run
            .result_summary
            .as_deref()
            .unwrap_or("-")
            .chars()
            .take(40)
            .collect::<String>();
        let tokens = run
            .tokens_used
            .map(|t| t.to_string())
            .unwrap_or_else(|| "-".to_string());

        println!(
            "{:<36}  {:<10}  {:<20}  {:<10}  {}",
            run.id,
            run.status,
            run.started_at.format("%Y-%m-%d %H:%M:%S"),
            tokens,
            summary,
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cron_command_parse() {
        // Verify CLI schema is valid
        #[derive(clap::Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: CronCommand,
        }
        TestCli::command().debug_assert();
    }
}
