//! `thinclaw automation projects` — manage the GitHub repository project supervisor
//! from the terminal. Commands talk directly to the database + secrets store via
//! the framework-free `crate::api::repo_projects` layer (the same one the
//! desktop commands and gateway handlers use).

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
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
        /// Public GitHub App slug (used to build the install URL).
        #[arg(long)]
        app_slug: Option<String>,
        #[arg(long)]
        default_coding_backend: Option<String>,
        #[arg(long)]
        default_write_mode: Option<String>,
        #[arg(long)]
        auto_merge: Option<bool>,
        #[arg(long)]
        watchdog_interval_secs: Option<u64>,
    },
    /// Create a purpose-bound GitHub credential source through secure local input.
    SetCredential {
        slot: String,
        #[arg(long, conflicts_with_all = ["from_env", "from_file"])]
        from_stdin: bool,
        #[arg(long, value_name = "VAR", conflicts_with_all = ["from_stdin", "from_file"])]
        from_env: Option<String>,
        #[arg(long, value_name = "FILE", conflicts_with_all = ["from_stdin", "from_env"])]
        from_file: Option<PathBuf>,
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
        #[arg(long)]
        write_mode: Option<String>,
        #[arg(long)]
        fork_owner: Option<String>,
        #[arg(long)]
        fork_repo: Option<String>,
    },
    /// Enroll an additional repository into a project.
    Enroll {
        project_id: String,
        #[arg(long)]
        repo_url: String,
        #[arg(long)]
        default_branch: Option<String>,
        #[arg(long)]
        fork_owner: Option<String>,
        #[arg(long)]
        fork_repo: Option<String>,
    },
    /// List the GitHub repositories the connected credential can act on
    /// (the connector repo picker), marking which are already enrolled.
    Repos,
    /// Bring repositories under supervision: a project is created for each.
    /// Pass one or more `owner/repo`, or `--all` for every accessible repo.
    Connect {
        /// owner/repo identifiers to connect.
        repos: Vec<String>,
        /// Connect every repository the credential can access.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        write_mode: Option<String>,
        #[arg(long)]
        fork_owner: Option<String>,
        #[arg(long)]
        fork_repo: Option<String>,
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
            app_slug,
            default_coding_backend,
            default_write_mode,
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
                app_slug,
                default_coding_backend,
                default_write_mode,
                auto_merge_default: auto_merge,
                watchdog_interval_secs,
                max_concurrent_projects: None,
                max_concurrent_tasks_per_project: None,
                workspace_base_dir: None,
            };
            print(api::configure_supervisor(&db, secrets.as_ref(), USER, input).await)
        }
        RepoProjectCommand::SetCredential {
            slot,
            from_stdin,
            from_env,
            from_file,
        } => {
            let slot = canonical_credential_slot(&slot)?;
            let value =
                resolve_credential_input(from_stdin, from_env.as_deref(), from_file.as_deref())?;
            let secrets = crate::cli::secrets::get_secrets_store().await?;
            print(api::store_repo_credential(&secrets, USER, slot.to_string(), value).await)
        }
        RepoProjectCommand::Create {
            name,
            repo_url,
            default_branch,
            description,
            write_mode,
            fork_owner,
            fork_repo,
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
                    write_mode: parse_write_mode_arg(write_mode)?,
                    fork_owner,
                    fork_repo,
                },
            )
            .await,
        ),
        RepoProjectCommand::Enroll {
            project_id,
            repo_url,
            default_branch,
            fork_owner,
            fork_repo,
        } => print(
            api::enroll_repo(
                &db,
                USER,
                parse(&project_id)?,
                api::RepoEnrollInput {
                    repo_url,
                    default_branch,
                    fork_owner,
                    fork_repo,
                },
            )
            .await,
        ),
        RepoProjectCommand::Repos => {
            let secrets = crate::cli::secrets::get_secrets_store().await?;
            print(api::list_connectable_repos(&db, &secrets, USER).await)
        }
        RepoProjectCommand::Connect {
            repos,
            all,
            write_mode,
            fork_owner,
            fork_repo,
        } => {
            let secrets = crate::cli::secrets::get_secrets_store().await?;
            print(
                api::connect_repos(
                    &db,
                    &secrets,
                    USER,
                    api::RepoConnectInput {
                        repos,
                        all,
                        write_mode: parse_write_mode_arg(write_mode)?,
                        fork_owner,
                        fork_repo,
                    },
                )
                .await,
            )
        }
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

fn canonical_credential_slot(slot: &str) -> anyhow::Result<&'static str> {
    match slot.trim() {
        "github_token" => Ok("github_token"),
        "github_fork_token" => Ok("github_fork_token"),
        "github_app_private_key" => Ok("repo_projects_github_private_key"),
        "github_webhook_secret" => Ok("repo_projects_github_webhook"),
        _ => anyhow::bail!(
            "unsupported slot; expected github_token, github_fork_token, github_app_private_key, or github_webhook_secret"
        ),
    }
}

fn resolve_credential_input(
    from_stdin: bool,
    from_env: Option<&str>,
    from_file: Option<&std::path::Path>,
) -> anyhow::Result<String> {
    const MAX_BYTES: u64 = 1024 * 1024;
    if from_stdin {
        let mut value = String::new();
        std::io::stdin()
            .take(MAX_BYTES + 1)
            .read_to_string(&mut value)?;
        if value.len() as u64 > MAX_BYTES {
            anyhow::bail!("credential exceeds 1 MiB");
        }
        return Ok(value.trim_end_matches(['\r', '\n']).to_string());
    }
    if let Some(variable) = from_env {
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
    if let Some(path) = from_file {
        if !path.is_absolute() {
            anyhow::bail!("--from-file requires an absolute path");
        }
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_BYTES {
            anyhow::bail!("credential source must be a regular non-symlink file up to 1 MiB");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.mode() & 0o077 != 0 {
                anyhow::bail!("credential source must not grant group or other permissions");
            }
        }
        return String::from_utf8(thinclaw_platform::read_regular_file_bounded_single_link(
            path, MAX_BYTES,
        )?)
        .map(|value| value.trim_end_matches(['\r', '\n']).to_string())
        .map_err(|_| anyhow::anyhow!("credential source is not UTF-8"));
    }
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("secure input is unavailable; use --from-stdin, --from-env, or --from-file");
    }
    Ok(crate::setup::secret_input("Credential value")?
        .expose_secret()
        .to_string())
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

fn parse_write_mode_arg(
    value: Option<String>,
) -> anyhow::Result<Option<thinclaw_repo_projects::RepoWriteMode>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let mode = match value.trim().to_ascii_lowercase().as_str() {
        "read_only_clone" | "read-only-clone" | "read_only" | "readonly" => {
            thinclaw_repo_projects::RepoWriteMode::ReadOnlyClone
        }
        "fork_pr" | "fork-pr" | "fork" => thinclaw_repo_projects::RepoWriteMode::ForkPr,
        "maintainer_branch_pr" | "maintainer-branch-pr" | "branch_pr" | "branch" => {
            thinclaw_repo_projects::RepoWriteMode::MaintainerBranchPr
        }
        "maintainer_auto_merge" | "maintainer-auto-merge" | "auto_merge" | "auto" => {
            thinclaw_repo_projects::RepoWriteMode::MaintainerAutoMerge
        }
        _ => {
            return Err(anyhow::anyhow!(
                "write mode must be one of read_only_clone, fork_pr, maintainer_branch_pr, maintainer_auto_merge"
            ));
        }
    };
    Ok(Some(mode))
}

fn print<T: serde::Serialize>(result: crate::api::ApiResult<T>) -> anyhow::Result<()> {
    let value = result.map_err(|error| anyhow::anyhow!("{error}"))?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
