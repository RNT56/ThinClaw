//! `thinclaw repo-projects` — manage the GitHub repository project supervisor
//! from the terminal. Commands talk directly to the database + secrets store via
//! the framework-free `crate::api::repo_projects` layer (the same one the
//! desktop commands and gateway handlers use).

use std::sync::Arc;

use clap::Subcommand;
use secrecy::ExposeSecret;
use uuid::Uuid;

use crate::api::repo_projects as api;
use crate::db::Database;

const USER: &str = "default";

#[derive(Subcommand, Debug, Clone)]
pub enum RepoProjectCommand {
    /// List all repository projects.
    List,
    /// Show one project's full status (backlog, workers, PRs, merge gates).
    Show { project_id: String },
    /// Show supervisor setup readiness (feature flag, credentials, policy).
    Status,
    /// Enable and configure the supervisor (writes settings).
    Setup {
        /// Enable the supervisor.
        #[arg(long)]
        enable: bool,
        /// Disable the supervisor.
        #[arg(long)]
        disable: bool,
        #[arg(long)]
        app_id: Option<u64>,
        #[arg(long)]
        installation_id: Option<u64>,
        /// Name of the secret holding the GitHub App PEM private key.
        #[arg(long)]
        private_key_secret: Option<String>,
        /// Name of the secret holding the GitHub webhook secret.
        #[arg(long)]
        webhook_secret_secret: Option<String>,
        #[arg(long)]
        default_coding_backend: Option<String>,
        #[arg(long)]
        auto_merge: Option<bool>,
        #[arg(long)]
        watchdog_interval_secs: Option<u64>,
    },
    /// Store a GitHub credential in the encrypted secrets store (prompts if
    /// `--value` is omitted).
    SetCredential {
        name: String,
        #[arg(long)]
        value: Option<String>,
    },
    /// Create a project and enroll its first repository.
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        repo_url: String,
        #[arg(long)]
        default_branch: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Enroll an additional repository into a project.
    Enroll {
        project_id: String,
        #[arg(long)]
        repo_url: String,
        #[arg(long)]
        default_branch: Option<String>,
    },
    /// Start a project.
    Start { project_id: String },
    /// Pause a project.
    Pause { project_id: String },
    /// Resume a paused project.
    Resume { project_id: String },
    /// Cancel a project.
    Cancel { project_id: String },
    /// List recent project events.
    Events {
        project_id: String,
        #[arg(long, default_value = "20")]
        limit: i64,
    },
}

pub async fn run_repo_projects_command(cmd: RepoProjectCommand) -> anyhow::Result<()> {
    let db = connect_db().await?;
    match cmd {
        RepoProjectCommand::List => print(api::list_projects(&db).await),
        RepoProjectCommand::Show { project_id } => {
            print(api::get_project(&db, parse(&project_id)?).await)
        }
        RepoProjectCommand::Status => {
            let secrets = crate::cli::secrets::get_secrets_store().await.ok();
            print(api::repo_projects_readiness(&db, secrets.as_ref(), USER).await)
        }
        RepoProjectCommand::Setup {
            enable,
            disable,
            app_id,
            installation_id,
            private_key_secret,
            webhook_secret_secret,
            default_coding_backend,
            auto_merge,
            watchdog_interval_secs,
        } => {
            let enabled = if enable {
                Some(true)
            } else if disable {
                Some(false)
            } else {
                None
            };
            let secrets = crate::cli::secrets::get_secrets_store().await.ok();
            let input = api::RepoProjectsConfigureInput {
                enabled,
                app_id,
                installation_id,
                private_key_secret,
                webhook_secret_secret,
                default_coding_backend,
                auto_merge_default: auto_merge,
                watchdog_interval_secs,
                max_concurrent_projects: None,
                max_concurrent_tasks_per_project: None,
                workspace_base_dir: None,
            };
            print(api::configure_supervisor(&db, secrets.as_ref(), USER, input).await)
        }
        RepoProjectCommand::SetCredential { name, value } => {
            let value = match value {
                Some(value) => value,
                None => crate::setup::secret_input("Credential value")?
                    .expose_secret()
                    .to_string(),
            };
            let secrets = crate::cli::secrets::get_secrets_store().await?;
            print(api::store_repo_credential(&secrets, USER, name, value).await)
        }
        RepoProjectCommand::Create {
            name,
            repo_url,
            default_branch,
            description,
        } => print(
            api::create_project(
                &db,
                USER,
                api::RepoProjectCreateInput {
                    name,
                    repo_url,
                    default_branch,
                    local_path: None,
                    description,
                },
            )
            .await,
        ),
        RepoProjectCommand::Enroll {
            project_id,
            repo_url,
            default_branch,
        } => print(
            api::enroll_repo(
                &db,
                USER,
                parse(&project_id)?,
                api::RepoEnrollInput {
                    repo_url,
                    default_branch,
                },
            )
            .await,
        ),
        RepoProjectCommand::Start { project_id } => {
            print(api::start_project(&db, USER, parse(&project_id)?).await)
        }
        RepoProjectCommand::Pause { project_id } => {
            print(api::pause_project(&db, USER, parse(&project_id)?).await)
        }
        RepoProjectCommand::Resume { project_id } => {
            print(api::resume_project(&db, USER, parse(&project_id)?).await)
        }
        RepoProjectCommand::Cancel { project_id } => {
            print(api::cancel_project(&db, USER, parse(&project_id)?).await)
        }
        RepoProjectCommand::Events { project_id, limit } => {
            print(api::list_events(&db, parse(&project_id)?, limit).await)
        }
    }
}

async fn connect_db() -> anyhow::Result<Arc<dyn Database>> {
    let config = crate::config::Config::from_env()
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    crate::db::connect_from_config(&config.database)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
}

fn parse(id: &str) -> anyhow::Result<Uuid> {
    Uuid::parse_str(id).map_err(|_| anyhow::anyhow!("project_id must be a UUID"))
}

fn print<T: serde::Serialize>(result: crate::api::ApiResult<T>) -> anyhow::Result<()> {
    let value = result.map_err(|error| anyhow::anyhow!("{error}"))?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
