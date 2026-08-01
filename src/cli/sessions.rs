//! Durable conversation inspection and administration.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use serde::Serialize;
use uuid::Uuid;

use crate::cli::{CliContext, CliError};
use crate::db::Database;

const DEFAULT_PRINCIPAL: &str = "default";
const DEFAULT_CHANNEL: &str = "cli";
const MAX_LIST_LIMIT: i64 = 500;
const MAX_EXPORT_MESSAGES: i64 = 100_000;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ConversationArtifactFormat {
    Markdown,
    Json,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SessionCommand {
    /// List durable conversations
    List {
        #[arg(long, default_value = DEFAULT_PRINCIPAL)]
        principal: String,
        #[arg(long, default_value = DEFAULT_CHANNEL)]
        channel: String,
        #[arg(long, default_value_t = 50)]
        limit: i64,
        /// Reserved opaque pagination cursor
        #[arg(long)]
        cursor: Option<String>,
        /// Deprecated; use global --output-format
        #[arg(long, hide = true)]
        format: Option<String>,
    },
    /// Show a durable conversation and its recent messages
    Show {
        id: Uuid,
        #[arg(long, default_value = DEFAULT_PRINCIPAL)]
        principal: String,
        #[arg(long, default_value_t = 50)]
        messages: i64,
        #[arg(long)]
        before: Option<Uuid>,
    },
    /// Search durable conversation messages
    Search {
        query: String,
        #[arg(long, default_value = DEFAULT_PRINCIPAL)]
        principal: String,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
    /// Export one durable conversation
    Export {
        id: Uuid,
        #[arg(long, default_value = DEFAULT_PRINCIPAL)]
        principal: String,
        #[arg(long, value_enum, default_value_t = ConversationArtifactFormat::Markdown)]
        artifact_format: ConversationArtifactFormat,
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Permanently delete one durable conversation
    Delete {
        id: Uuid,
        #[arg(long, default_value = DEFAULT_PRINCIPAL)]
        principal: String,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Delete conversations older than a duration (defaults to dry-run)
    Prune {
        #[arg(long)]
        older_than: String,
        #[arg(long, default_value = DEFAULT_PRINCIPAL)]
        principal: String,
        #[arg(long, default_value = DEFAULT_CHANNEL)]
        channel: String,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Serialize)]
struct ConversationSummaryOutput {
    id: Uuid,
    principal: String,
    actor_id: Option<String>,
    channel: String,
    message_count: i64,
    preview: Option<String>,
    started_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
struct ConversationMessageOutput {
    id: Uuid,
    role: String,
    content: String,
    actor_id: Option<String>,
    actor_display_name: Option<String>,
    metadata: serde_json::Value,
    created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum ConversationResult {
    List {
        conversations: Vec<ConversationSummaryOutput>,
        next_cursor: Option<String>,
    },
    Show {
        id: Uuid,
        metadata: serde_json::Value,
        message_count: i64,
        messages: Vec<ConversationMessageOutput>,
    },
    Search {
        hits: Vec<thinclaw_history::SessionSearchHit>,
    },
    DeletePlan {
        conversation_ids: Vec<Uuid>,
        count: usize,
    },
    Deleted {
        conversation_ids: Vec<Uuid>,
        count: usize,
    },
    Exported {
        id: Uuid,
        path: PathBuf,
        bytes: usize,
    },
}

pub async fn run_sessions_command(
    cmd: SessionCommand,
    context: &CliContext,
) -> Result<(), CliError> {
    let database = context.database().await?;
    match cmd {
        SessionCommand::Export {
            id,
            principal,
            artifact_format,
            out,
            force,
        } => {
            export(
                database.as_ref(),
                context,
                id,
                &principal,
                artifact_format,
                out,
                force,
            )
            .await
        }
        other => {
            let result = execute(database.as_ref(), other).await?;
            context
                .output()
                .write_record("data.conversations", &result, render_human)
        }
    }
}

async fn execute(db: &dyn Database, cmd: SessionCommand) -> Result<ConversationResult, CliError> {
    match cmd {
        SessionCommand::List {
            principal,
            channel,
            limit,
            cursor,
            format: _,
        } => {
            if cursor.is_some() {
                return Err(CliError::usage(
                    "conversation cursors are not available with this database schema",
                ));
            }
            let limit = validate_limit(limit, MAX_LIST_LIMIT)?;
            let conversations = db
                .list_conversations_with_preview(&principal, &channel, limit)
                .await
                .map_err(database_error)?
                .into_iter()
                .map(summary_output)
                .collect();
            Ok(ConversationResult::List {
                conversations,
                next_cursor: None,
            })
        }
        SessionCommand::Show {
            id,
            principal,
            messages,
            before,
        } => {
            authorize(db, id, &principal).await?;
            let metadata = db
                .get_conversation_metadata(id)
                .await
                .map_err(database_error)?
                .ok_or_else(|| CliError::operational(format!("conversation '{id}' not found")))?;
            let all = load_messages(db, id).await?;
            let end = match before {
                Some(before) => all
                    .iter()
                    .position(|message| message.id == before)
                    .ok_or_else(|| {
                        CliError::operational(format!(
                            "message '{before}' was not found in conversation '{id}'"
                        ))
                    })?,
                None => all.len(),
            };
            let limit = validate_limit(messages, 500)? as usize;
            let start = end.saturating_sub(limit);
            let selected = all[start..end]
                .iter()
                .cloned()
                .map(message_output)
                .collect();
            Ok(ConversationResult::Show {
                id,
                metadata,
                message_count: all.len() as i64,
                messages: selected,
            })
        }
        SessionCommand::Search {
            query,
            principal,
            channel,
            limit,
        } => {
            if query.trim().is_empty() {
                return Err(CliError::usage("search query cannot be empty"));
            }
            let limit = validate_limit(limit, MAX_LIST_LIMIT)?;
            let hits = db
                .search_conversation_messages(
                    &principal,
                    query.trim(),
                    None,
                    channel.as_deref(),
                    None,
                    limit,
                )
                .await
                .map_err(database_error)?;
            Ok(ConversationResult::Search { hits })
        }
        SessionCommand::Delete {
            id,
            principal,
            dry_run,
            yes,
        } => {
            authorize(db, id, &principal).await?;
            if dry_run {
                return Ok(ConversationResult::DeletePlan {
                    conversation_ids: vec![id],
                    count: 1,
                });
            }
            require_confirmation(yes, &[id])?;
            if !db.delete_conversation(id).await.map_err(database_error)? {
                return Err(CliError::operational(format!(
                    "conversation '{id}' was not found"
                )));
            }
            Ok(ConversationResult::Deleted {
                conversation_ids: vec![id],
                count: 1,
            })
        }
        SessionCommand::Prune {
            older_than,
            principal,
            channel,
            dry_run,
            yes,
        } => {
            let age = parse_duration(&older_than)?;
            let cutoff = chrono::Utc::now()
                - chrono::Duration::from_std(age)
                    .map_err(|_| CliError::usage("duration is too large"))?;
            let ids: Vec<Uuid> = db
                .list_conversations_with_preview(&principal, &channel, 10_000)
                .await
                .map_err(database_error)?
                .into_iter()
                .filter(|conversation| conversation.last_activity < cutoff)
                .map(|conversation| conversation.id)
                .collect();
            if dry_run || !yes {
                return Ok(ConversationResult::DeletePlan {
                    count: ids.len(),
                    conversation_ids: ids,
                });
            }
            let mut deleted = Vec::with_capacity(ids.len());
            for id in ids {
                if db.delete_conversation(id).await.map_err(database_error)? {
                    deleted.push(id);
                }
            }
            Ok(ConversationResult::Deleted {
                count: deleted.len(),
                conversation_ids: deleted,
            })
        }
        SessionCommand::Export { .. } => unreachable!("export is handled at the artifact boundary"),
    }
}

async fn export(
    db: &dyn Database,
    context: &CliContext,
    id: Uuid,
    principal: &str,
    format: ConversationArtifactFormat,
    out: Option<PathBuf>,
    force: bool,
) -> Result<(), CliError> {
    authorize(db, id, principal).await?;
    let messages = load_messages(db, id).await?;
    let output_messages: Vec<_> = messages.into_iter().map(message_output).collect();
    let artifact = match format {
        ConversationArtifactFormat::Json => serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "conversation_id": id,
            "messages": output_messages,
        }))
        .map_err(|error| CliError::operational(format!("failed to encode export: {error}")))?,
        ConversationArtifactFormat::Markdown => render_markdown(id, &output_messages).into_bytes(),
    };

    if let Some(path) = out {
        write_artifact_atomically(&path, &artifact, force)?;
        let result = ConversationResult::Exported {
            id,
            path,
            bytes: artifact.len(),
        };
        context
            .output()
            .write_record("data.conversations.export", &result, render_human)
    } else {
        std::io::stdout()
            .lock()
            .write_all(&artifact)
            .and_then(|_| std::io::stdout().lock().flush())
            .map_err(|error| CliError::operational(format!("failed to write export: {error}")))
    }
}

async fn authorize(db: &dyn Database, id: Uuid, principal: &str) -> Result<(), CliError> {
    if !db
        .conversation_belongs_to_user(id, principal)
        .await
        .map_err(database_error)?
    {
        return Err(CliError::operational(format!(
            "conversation '{id}' not found"
        )));
    }
    Ok(())
}

async fn load_messages(
    db: &dyn Database,
    id: Uuid,
) -> Result<Vec<thinclaw_history::ConversationMessage>, CliError> {
    let count = db
        .count_conversation_messages(id)
        .await
        .map_err(database_error)?;
    if count > MAX_EXPORT_MESSAGES {
        return Err(CliError::operational(format!(
            "conversation has {count} messages; export limit is {MAX_EXPORT_MESSAGES}"
        )));
    }
    db.list_conversation_messages_window(id, 0, count)
        .await
        .map_err(database_error)
}

fn summary_output(summary: thinclaw_history::ConversationSummary) -> ConversationSummaryOutput {
    ConversationSummaryOutput {
        id: summary.id,
        principal: summary.user_id,
        actor_id: summary.actor_id,
        channel: summary.channel,
        message_count: summary.message_count,
        preview: summary.title,
        started_at: summary.started_at.to_rfc3339(),
        updated_at: summary.last_activity.to_rfc3339(),
    }
}

fn message_output(message: thinclaw_history::ConversationMessage) -> ConversationMessageOutput {
    ConversationMessageOutput {
        id: message.id,
        role: message.role,
        content: message.content,
        actor_id: message.actor_id,
        actor_display_name: message.actor_display_name,
        metadata: message.metadata,
        created_at: message.created_at.to_rfc3339(),
    }
}

fn validate_limit(limit: i64, max: i64) -> Result<i64, CliError> {
    if !(1..=max).contains(&limit) {
        return Err(CliError::usage(format!(
            "limit must be between 1 and {max}"
        )));
    }
    Ok(limit)
}

fn require_confirmation(yes: bool, ids: &[Uuid]) -> Result<(), CliError> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(CliError::usage(
            "conversation deletion requires --yes in noninteractive mode",
        ));
    }
    eprint!("Permanently delete conversation {}? [y/N] ", ids[0]);
    std::io::stderr()
        .flush()
        .map_err(|error| CliError::operational(format!("failed to prompt: {error}")))?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| CliError::operational(format!("failed to read confirmation: {error}")))?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Err(CliError::operational("conversation deletion cancelled"));
    }
    Ok(())
}

fn parse_duration(value: &str) -> Result<std::time::Duration, CliError> {
    let value = value.trim();
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .ok_or_else(|| CliError::usage("duration requires a unit: s, m, h, or d"))?;
    let amount = value[..split]
        .parse::<u64>()
        .map_err(|_| CliError::usage("duration amount is invalid"))?;
    let unit = &value[split..];
    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        _ => return Err(CliError::usage("duration unit must be s, m, h, or d")),
    };
    amount
        .checked_mul(multiplier)
        .map(std::time::Duration::from_secs)
        .ok_or_else(|| CliError::usage("duration is too large"))
}

fn write_artifact_atomically(path: &Path, bytes: &[u8], force: bool) -> Result<(), CliError> {
    if path.exists() && !force {
        return Err(CliError::operational(format!(
            "refusing to overwrite existing artifact '{}'; pass --force",
            path.display()
        )));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| CliError::usage("artifact output path needs a file name"))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        Uuid::new_v4()
    ));
    let write_result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    write_result.map_err(|error| {
        CliError::operational(format!(
            "failed to write artifact '{}': {error}",
            path.display()
        ))
    })
}

fn render_markdown(id: Uuid, messages: &[ConversationMessageOutput]) -> String {
    let mut markdown = format!("# Conversation {id}\n\n");
    for message in messages {
        markdown.push_str(&format!(
            "## {} — {}\n\n{}\n\n",
            message.role, message.created_at, message.content
        ));
    }
    markdown
}

fn render_human(result: &ConversationResult) -> String {
    match result {
        ConversationResult::List { conversations, .. } if conversations.is_empty() => {
            "No durable conversations found.".to_string()
        }
        ConversationResult::List { conversations, .. } => conversations
            .iter()
            .map(|conversation| {
                format!(
                    "{}\t{}\t{}\t{}\t{}",
                    conversation.id,
                    conversation.channel,
                    conversation.principal,
                    conversation.message_count,
                    conversation.preview.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        ConversationResult::Show { id, messages, .. } => {
            let mut rendered = format!("Conversation {id}");
            for message in messages {
                rendered.push_str(&format!(
                    "\n\n{} {}\n{}",
                    message.created_at, message.role, message.content
                ));
            }
            rendered
        }
        ConversationResult::Search { hits } if hits.is_empty() => "No matches found.".to_string(),
        ConversationResult::Search { hits } => hits
            .iter()
            .map(|hit| {
                format!(
                    "{}\t{}\t{}\t{}",
                    hit.conversation_id, hit.created_at, hit.role, hit.excerpt
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        ConversationResult::DeletePlan {
            conversation_ids,
            count,
        } => format!(
            "Dry run: {count} conversation(s) would be deleted: {}",
            conversation_ids
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ConversationResult::Deleted { count, .. } => {
            format!("Deleted {count} durable conversation(s).")
        }
        ConversationResult::Exported { path, bytes, .. } => {
            format!("Wrote {bytes} bytes to '{}'.", path.display())
        }
    }
}

fn database_error(error: impl std::fmt::Display) -> CliError {
    CliError::operational(format!("conversation store operation failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_parser_is_bounded_and_explicit() {
        assert_eq!(parse_duration("2h").unwrap().as_secs(), 7200);
        assert!(parse_duration("2").is_err());
        assert!(parse_duration("2weeks").is_err());
    }

    #[test]
    fn markdown_export_is_deterministic() {
        let id = Uuid::nil();
        let rendered = render_markdown(id, &[]);
        assert_eq!(rendered, format!("# Conversation {id}\n\n"));
    }
}
