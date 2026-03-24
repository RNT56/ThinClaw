//! Cron / routine management CLI commands.
//!
//! Subcommands:
//! - `cron list` — list all routines
//! - `cron add` — create a new lightweight routine
//! - `cron remove` — delete a routine by UUID or name
//! - `cron trigger` — manually trigger a routine
//! - `cron runs` — show recent runs for a routine
//! - `cron lint` — validate a cron expression and show next fire times

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

    /// Edit an existing routine
    Edit {
        /// Routine UUID or name
        id_or_name: String,

        /// New cron schedule
        #[arg(short, long)]
        schedule: Option<String>,

        /// New prompt
        #[arg(short, long)]
        prompt: Option<String>,

        /// New description
        #[arg(short, long)]
        description: Option<String>,

        /// Enable or disable the routine
        #[arg(short, long)]
        enabled: Option<bool>,

        /// Model to use for this routine (e.g. "claude-sonnet-4-20250514")
        #[arg(long)]
        model: Option<String>,

        /// Thinking budget tokens (0 = disabled)
        #[arg(long)]
        thinking_budget: Option<u32>,
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

    /// Validate a cron expression and show next fire times
    Lint {
        /// Cron expression to validate (e.g. "0 9 * * MON-FRI")
        expression: String,

        /// Number of upcoming fire times to show (default: 5)
        #[arg(short = 'n', long, default_value = "5")]
        count: usize,
    },
}

/// Run a cron CLI command.
pub async fn run_cron_command(cmd: CronCommand) -> anyhow::Result<()> {
    // Lint doesn't need DB, handle it separately
    if let CronCommand::Lint { expression, count } = cmd {
        return run_lint(&expression, count);
    }

    let db = connect_db().await?;

    match cmd {
        CronCommand::List { format } => list_routines(&*db, &format).await,
        CronCommand::Add {
            name,
            schedule,
            prompt,
            description,
        } => add_routine(&*db, name, schedule, prompt, description).await,
        CronCommand::Edit {
            id_or_name,
            schedule,
            prompt,
            description,
            enabled,
            model,
            thinking_budget,
        } => {
            edit_routine(
                &*db,
                &id_or_name,
                schedule,
                prompt,
                description,
                enabled,
                model,
                thinking_budget,
            )
            .await
        }
        CronCommand::Remove { id_or_name } => remove_routine(&*db, &id_or_name).await,
        CronCommand::Trigger { id_or_name } => trigger_routine(&*db, &id_or_name).await,
        CronCommand::Runs { id_or_name, limit } => show_runs(&*db, &id_or_name, limit).await,
        CronCommand::Lint { .. } => unreachable!(), // handled above
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
    if let Ok(id) = Uuid::parse_str(id_or_name)
        && let Some(r) = db
            .get_routine(id)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?
    {
        return Ok(r);
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
            "Create one with: thinclaw cron add <name> --schedule '<cron>' --prompt '<prompt>'"
        );
        return Ok(());
    }

    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&routines)?);
        return Ok(());
    }

    // Table format
    println!(
        "{:<36}  {:<20}  {:<8}  {:<15}  {:<6}  DESCRIPTION",
        "ID", "NAME", "ENABLED", "TRIGGER", "RUNS"
    );
    println!("{}", "-".repeat(120));

    for r in &routines {
        let trigger_info = match &r.trigger {
            crate::agent::routine::Trigger::Cron { schedule } => {
                if schedule.len() > 13 {
                    let end = schedule
                        .char_indices()
                        .map(|(i, _)| i)
                        .take_while(|&i| i < 12)
                        .last()
                        .unwrap_or(0);
                    format!("{}…", &schedule[..end])
                } else {
                    schedule.clone()
                }
            }
            crate::agent::routine::Trigger::Event { pattern, .. } => {
                format!("event:{}", pattern.chars().take(8).collect::<String>())
            }
            crate::agent::routine::Trigger::Webhook { .. } => "webhook".to_string(),
            crate::agent::routine::Trigger::Manual => "manual".to_string(),
            crate::agent::routine::Trigger::SystemEvent { .. } => "sys_event".to_string(),
        };

        let desc = if r.description.len() > 30 {
            let end = r
                .description
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i < 29)
                .last()
                .unwrap_or(0);
            format!("{}…", &r.description[..end])
        } else {
            r.description.clone()
        };

        println!(
            "{:<36}  {:<20}  {:<8}  {:<15}  {:<6}  {}",
            r.id,
            {
                let name_end = r
                    .name
                    .char_indices()
                    .map(|(i, _)| i)
                    .take_while(|&i| i < 20)
                    .last()
                    .map(|i| {
                        // include the char at this position
                        r.name[i..]
                            .chars()
                            .next()
                            .map(|c| i + c.len_utf8())
                            .unwrap_or(i)
                    })
                    .unwrap_or(r.name.len())
                    .min(r.name.len());
                &r.name[..name_end]
            },
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
    // Auto-normalize 5/6-field to 7-field
    let schedule = crate::agent::routine::normalize_cron_expr(&schedule);
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

/// Edit an existing routine.
#[allow(clippy::too_many_arguments)]
async fn edit_routine(
    db: &dyn crate::db::Database,
    id_or_name: &str,
    schedule: Option<String>,
    prompt: Option<String>,
    description: Option<String>,
    enabled: Option<bool>,
    model: Option<String>,
    thinking_budget: Option<u32>,
) -> anyhow::Result<()> {
    let mut routine = resolve_routine(db, id_or_name).await?;
    let mut changes = Vec::new();

    if let Some(new_schedule) = schedule {
        // Auto-normalize 5/6-field to 7-field, then validate.
        let normalized = crate::agent::routine::normalize_cron_expr(&new_schedule);
        crate::agent::routine::next_cron_fire(&normalized)
            .map_err(|e| anyhow::anyhow!("Invalid cron schedule: {}", e))?;

        routine.trigger = crate::agent::routine::Trigger::Cron {
            schedule: normalized.clone(),
        };
        routine.next_fire_at = crate::agent::routine::next_cron_fire(&normalized)?;
        changes.push(format!("schedule → {}", normalized));
    }

    if let Some(new_prompt) = prompt {
        match &mut routine.action {
            crate::agent::routine::RoutineAction::Lightweight { prompt, .. } => {
                *prompt = new_prompt.clone();
            }
            crate::agent::routine::RoutineAction::FullJob { description, .. } => {
                *description = new_prompt.clone();
            }
            crate::agent::routine::RoutineAction::Heartbeat { prompt, .. } => {
                *prompt = Some(new_prompt.clone());
            }
        }
        changes.push("prompt updated".to_string());
    }

    if let Some(new_desc) = description {
        routine.description = new_desc;
        changes.push("description updated".to_string());
    }

    if let Some(new_enabled) = enabled {
        routine.enabled = new_enabled;
        changes.push(format!(
            "enabled → {}",
            if new_enabled { "true" } else { "false" }
        ));
    }

    // Model and thinking budget are stored in the routine's state JSON.
    if let Some(new_model) = model {
        routine.state["model"] = serde_json::json!(new_model);
        changes.push(format!("model → {}", routine.state["model"]));
    }

    if let Some(budget) = thinking_budget {
        if budget == 0 {
            routine.state["thinking_budget_tokens"] = serde_json::json!(null);
            changes.push("thinking → disabled".to_string());
        } else {
            routine.state["thinking_budget_tokens"] = serde_json::json!(budget);
            changes.push(format!("thinking budget → {} tokens", budget));
        }
    }

    if changes.is_empty() {
        println!("No changes specified. Use --help to see available options.");
        return Ok(());
    }

    routine.updated_at = chrono::Utc::now();

    db.update_routine(&routine)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to update routine: {}", e))?;

    println!("✅ Updated routine '{}' ({}):", routine.name, routine.id);
    for change in &changes {
        println!("   • {}", change);
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
        crate::agent::routine::RoutineAction::Heartbeat { prompt, .. } => prompt
            .clone()
            .unwrap_or_else(|| "Heartbeat check".to_string()),
    };

    println!("🔄 Triggering routine '{}' ({})", routine.name, routine.id);
    println!(
        "   Prompt: {}",
        if prompt.len() > 60 {
            let end = prompt
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i < 57)
                .last()
                .unwrap_or(0);
            format!("{}…", &prompt[..end])
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
        "{:<36}  {:<10}  {:<20}  {:<10}  SUMMARY",
        "RUN ID", "STATUS", "STARTED", "TOKENS"
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

/// Validate a cron expression and show the next N fire times.
///
/// This runs offline — no DB connection needed.
fn run_lint(expression: &str, count: usize) -> anyhow::Result<()> {
    use std::str::FromStr;

    // Auto-normalize 5/6-field to 7-field so lint works with standard cron
    let normalized = crate::agent::routine::normalize_cron_expr(expression);
    if normalized != expression {
        println!(
            "ℹ️  Auto-normalized: \"{}\" → \"{}\"",
            expression, normalized
        );
    }

    // Try to parse the cron expression
    let schedule = match cron::Schedule::from_str(&normalized) {
        Ok(s) => s,
        Err(e) => {
            println!("❌ Invalid cron expression: \"{}\"", normalized);
            println!("   Error: {}", e);
            return Err(anyhow::anyhow!("Invalid cron expression"));
        }
    };

    println!("✅ Valid cron expression: \"{}\"", normalized);
    println!();

    // Show next N fire times
    let count = count.clamp(1, 50);
    let upcoming: Vec<_> = schedule.upcoming(chrono::Utc).take(count).collect();

    if upcoming.is_empty() {
        println!("   No upcoming fire times (expression may never match).");
        return Ok(());
    }

    println!(
        "   Next {} fire time{}:",
        upcoming.len(),
        if upcoming.len() == 1 { "" } else { "s" }
    );
    println!();

    let now = chrono::Utc::now();
    for (i, fire_time) in upcoming.iter().enumerate() {
        let delta = *fire_time - now;
        let delta_str = humanize_duration(delta);
        let local = fire_time.with_timezone(&chrono::Local);

        println!(
            "   {:>2}. {}  ({}  \u{2014}  in {})",
            i + 1,
            fire_time.format("%Y-%m-%d %H:%M:%S UTC"),
            local.format("%H:%M:%S %Z"),
            delta_str,
        );
    }

    // Show interval between fires if more than one
    if upcoming.len() >= 2 {
        let intervals: Vec<_> = upcoming
            .windows(2)
            .map(|w| (w[1] - w[0]).to_std().unwrap_or_default())
            .collect();

        let min_interval = intervals.iter().min().copied().unwrap_or_default();
        let max_interval = intervals.iter().max().copied().unwrap_or_default();

        println!();
        if min_interval == max_interval {
            println!("   Interval: every {}", humanize_std_duration(min_interval));
        } else {
            println!(
                "   Interval: {} \u{2013} {}",
                humanize_std_duration(min_interval),
                humanize_std_duration(max_interval),
            );
        }
    }

    Ok(())
}

/// Humanize a chrono::Duration.
fn humanize_duration(d: chrono::Duration) -> String {
    let secs = d.num_seconds();
    if secs < 0 {
        return "now".to_string();
    }
    humanize_seconds(secs as u64)
}

/// Humanize a std::time::Duration.
fn humanize_std_duration(d: std::time::Duration) -> String {
    humanize_seconds(d.as_secs())
}

fn humanize_seconds(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;

    if days > 0 {
        format!("{}d {}h {}m", days, hours, mins)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else if mins > 0 {
        format!("{}m", mins)
    } else {
        format!("{}s", secs)
    }
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

    #[test]
    fn test_run_lint_valid_expression() {
        // 7-field: every minute
        let result = run_lint("0 * * * * * *", 3);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_lint_invalid_expression() {
        let result = run_lint("not a cron", 3);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_lint_weekday_schedule() {
        // Every weekday at 9 AM UTC
        let result = run_lint("0 0 9 * * MON-FRI *", 5);
        assert!(result.is_ok());
    }

    #[test]
    fn test_humanize_duration() {
        assert_eq!(humanize_seconds(30), "30s");
        assert_eq!(humanize_seconds(90), "1m");
        assert_eq!(humanize_seconds(3661), "1h 1m");
        assert_eq!(humanize_seconds(90061), "1d 1h 1m");
    }
}
