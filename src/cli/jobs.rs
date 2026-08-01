//! Authenticated administration of running-runtime jobs.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use uuid::Uuid;

use crate::cli::{CliContext, CliError, GatewayClient};
use thinclaw_gateway::web::jobs::{
    JobEventsResponse, JobPromptQueuedResponse, JobRestartResponse, JobStatusActionResponse,
};
use thinclaw_gateway::web::types::{
    JobDetailResponse, JobListResponse, JobSummaryResponse, ProjectFileReadResponse,
    ProjectFilesResponse,
};

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobBackendFilter {
    Direct,
    Sandbox,
}

#[derive(Subcommand, Debug, Clone)]
pub enum JobCommand {
    /// List jobs owned by the authenticated principal
    List {
        #[arg(long)]
        state: Option<String>,
        #[arg(long, value_enum)]
        backend: Option<JobBackendFilter>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Summarize jobs by state
    Summary,
    /// Show one job
    Show { id: Uuid },
    /// Cancel one job
    Cancel {
        id: Uuid,
        #[arg(long)]
        yes: bool,
    },
    /// Restart one job
    Restart {
        id: Uuid,
        #[arg(long)]
        yes: bool,
    },
    /// Queue text for an interactive job
    Prompt { id: Uuid, text: String },
    /// Read a bounded event snapshot
    Events {
        id: Uuid,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        after: Option<i64>,
    },
    /// Inspect sandbox project files
    Files(JobFilesCommand),
}

#[derive(Args, Debug, Clone)]
pub struct JobFilesCommand {
    #[command(subcommand)]
    pub command: JobFilesSubcommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum JobFilesSubcommand {
    /// List a project directory
    List { id: Uuid, path: Option<String> },
    /// Read a bounded UTF-8 project file
    Read {
        id: Uuid,
        path: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Serialize)]
struct EmptyQuery {}

#[derive(Debug, Serialize)]
struct JobListQuery<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backend: Option<JobBackendFilter>,
    limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct JobEventsQuery {
    limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    after: Option<i64>,
}

#[derive(Debug, Serialize)]
struct FileQuery<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<&'a str>,
}

pub async fn run_jobs_command(cmd: JobCommand, context: &CliContext) -> Result<(), CliError> {
    let config = context.config().await?;
    let client = GatewayClient::resolve_from_config(None, None, config)
        .map_err(|error| CliError::operational(error.to_string()))?;

    match cmd {
        JobCommand::List {
            state,
            backend,
            limit,
            cursor,
        } => {
            if !(1..=200).contains(&limit) {
                return Err(CliError::usage("job list limit must be between 1 and 200"));
            }
            let response: JobListResponse = client
                .get_json(
                    "/api/jobs",
                    &JobListQuery {
                        state: state.as_deref(),
                        backend,
                        limit,
                        cursor: cursor.as_deref(),
                    },
                )
                .await
                .map_err(gateway_error)?;
            context
                .output()
                .write_record("automation.jobs.list", &response, |response| match response
                    .jobs
                    .as_slice()
                {
                    [] => "No jobs found.".to_string(),
                    jobs => jobs
                        .iter()
                        .map(|job| {
                            format!(
                                "{}\t{}\t{}\t{}",
                                job.id,
                                job.state,
                                job.execution_backend.as_deref().unwrap_or("direct"),
                                job.title
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                })
        }
        JobCommand::Summary => {
            let response: JobSummaryResponse = client
                .get_json("/api/jobs/summary", &EmptyQuery {})
                .await
                .map_err(gateway_error)?;
            context
                .output()
                .write_record("automation.jobs.summary", &response, |summary| {
                    format!(
                        "Total: {} (pending {}, running {}, completed {}, failed {}, cancelled {})",
                        summary.total,
                        summary.pending,
                        summary.in_progress,
                        summary.completed,
                        summary.failed,
                        summary.cancelled
                    )
                })
        }
        JobCommand::Show { id } => {
            let response: JobDetailResponse = client
                .get_json(&format!("/api/jobs/{id}"), &EmptyQuery {})
                .await
                .map_err(gateway_error)?;
            context
                .output()
                .write_record("automation.jobs.show", &response, |job| {
                    format!(
                        "Job: {}\nTitle: {}\nState: {}\nBackend: {}\nInteractive: {}",
                        job.id,
                        job.title,
                        job.state,
                        job.execution_backend.as_deref().unwrap_or("direct"),
                        job.interactive
                    )
                })
        }
        JobCommand::Cancel { id, yes } => {
            confirm_high_impact("cancel", id, yes)?;
            let response: JobStatusActionResponse = client
                .post_json(&format!("/api/jobs/{id}/cancel"), &serde_json::json!({}))
                .await
                .map_err(gateway_error)?;
            context
                .output()
                .write_record("automation.jobs.cancel", &response, |result| {
                    format!("Job {}: {}", result.job_id, result.status)
                })
        }
        JobCommand::Restart { id, yes } => {
            confirm_high_impact("restart", id, yes)?;
            let response: JobRestartResponse = client
                .post_json(&format!("/api/jobs/{id}/restart"), &serde_json::json!({}))
                .await
                .map_err(gateway_error)?;
            context
                .output()
                .write_record("automation.jobs.restart", &response, |result| {
                    format!(
                        "Job {} restarted as {}.",
                        result.old_job_id, result.new_job_id
                    )
                })
        }
        JobCommand::Prompt { id, text } => {
            if text.trim().is_empty() {
                return Err(CliError::usage("job prompt text cannot be empty"));
            }
            let response: JobPromptQueuedResponse = client
                .post_json(
                    &format!("/api/jobs/{id}/prompt"),
                    &serde_json::json!({"content": text, "done": false}),
                )
                .await
                .map_err(gateway_error)?;
            context
                .output()
                .write_record("automation.jobs.prompt", &response, |result| {
                    format!("Prompt {} for job {}.", result.status, result.job_id)
                })
        }
        JobCommand::Events { id, limit, after } => {
            if !(1..=1000).contains(&limit) {
                return Err(CliError::usage(
                    "job event limit must be between 1 and 1000",
                ));
            }
            let response: JobEventsResponse = client
                .get_json(
                    &format!("/api/jobs/{id}/events"),
                    &JobEventsQuery { limit, after },
                )
                .await
                .map_err(gateway_error)?;
            context
                .output()
                .write_record("automation.jobs.events", &response, |result| {
                    result
                        .events
                        .iter()
                        .map(|event| {
                            format!("{}\t{}\t{}", event.id, event.created_at, event.event_type)
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
        }
        JobCommand::Files(files) => match files.command {
            JobFilesSubcommand::List { id, path } => {
                let response: ProjectFilesResponse = client
                    .get_json(
                        &format!("/api/jobs/{id}/files/list"),
                        &FileQuery {
                            path: path.as_deref(),
                        },
                    )
                    .await
                    .map_err(gateway_error)?;
                context
                    .output()
                    .write_record("automation.jobs.files.list", &response, |result| {
                        result
                            .entries
                            .iter()
                            .map(|entry| {
                                format!(
                                    "{}\t{}",
                                    if entry.is_dir { "directory" } else { "file" },
                                    entry.path
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
            }
            JobFilesSubcommand::Read {
                id,
                path,
                out,
                force,
            } => {
                let response: ProjectFileReadResponse = client
                    .get_json(
                        &format!("/api/jobs/{id}/files/read"),
                        &FileQuery { path: Some(&path) },
                    )
                    .await
                    .map_err(gateway_error)?;
                if let Some(out) = out {
                    write_text_atomically(&out, &response.content, force)?;
                    context.output().write_record(
                        "automation.jobs.files.read",
                        &serde_json::json!({
                            "job_id": id,
                            "remote_path": response.path,
                            "out": out,
                            "bytes": response.content.len(),
                        }),
                        |_| format!("Wrote job file to '{}'.", out.display()),
                    )
                } else {
                    std::io::stdout()
                        .lock()
                        .write_all(response.content.as_bytes())
                        .and_then(|_| std::io::stdout().lock().flush())
                        .map_err(|error| {
                            CliError::operational(format!("failed to write job file: {error}"))
                        })
                }
            }
        },
    }
}

fn confirm_high_impact(action: &str, id: Uuid, yes: bool) -> Result<(), CliError> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(CliError::usage(format!(
            "job {action} requires --yes in noninteractive mode"
        )));
    }
    eprint!("{action} job {id}? [y/N] ");
    std::io::stderr()
        .flush()
        .map_err(|error| CliError::operational(format!("failed to prompt: {error}")))?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| CliError::operational(format!("failed to confirm: {error}")))?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Err(CliError::operational(format!("job {action} cancelled")));
    }
    Ok(())
}

fn write_text_atomically(path: &Path, content: &str, force: bool) -> Result<(), CliError> {
    if path.exists() && !force {
        return Err(CliError::operational(format!(
            "refusing to overwrite '{}'; pass --force",
            path.display()
        )));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| CliError::usage("output path needs a file name"))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|error| CliError::operational(format!("failed to write output: {error}")))
}

fn gateway_error(error: impl std::fmt::Display) -> CliError {
    CliError::operational(format!("running runtime job operation failed: {error}"))
}
