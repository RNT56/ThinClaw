use std::sync::Arc;

use anyhow::anyhow;
use clap::{Args, Subcommand};
use reqwest::Method;
use uuid::Uuid;

use crate::api::experiments as experiments_api;
use crate::db::Database;
use crate::experiments::{
    ExperimentAutonomyMode, ExperimentMetricComparator, ExperimentMetricDefinition,
    ExperimentPreset, ExperimentRunnerBackend,
};

const DEFAULT_USER_ID: &str = "default";

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand, Debug, Clone)]
pub enum ExperimentsCommand {
    /// Enable the experiments subsystem.
    Enable,
    /// Disable the experiments subsystem.
    Disable,
    /// Manage experiment projects.
    #[command(subcommand)]
    Projects(ExperimentProjectsCommand),
    /// Manage experiment runner profiles.
    #[command(subcommand)]
    Runners(ExperimentRunnersCommand),
    /// Manage experiment campaigns.
    #[command(subcommand)]
    Campaigns(ExperimentCampaignsCommand),
    /// Inspect auto-detected improvement opportunities.
    #[command(subcommand)]
    Opportunities(ExperimentOpportunitiesCommand),
    /// Manage opportunity targets and target links.
    #[command(subcommand)]
    Targets(ExperimentTargetsCommand),
    /// Manage GPU cloud provider workflows.
    #[command(subcommand)]
    Providers(ExperimentProvidersCommand),
}

#[derive(Subcommand, Debug, Clone)]
pub enum ExperimentProjectsCommand {
    List,
    Show {
        id: Uuid,
    },
    Create(ProjectCreateArgs),
    Update {
        id: Uuid,
        #[command(flatten)]
        args: ProjectUpdateArgs,
    },
    Delete {
        id: Uuid,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ExperimentRunnersCommand {
    List,
    Show {
        id: Uuid,
    },
    Create(RunnerCreateArgs),
    Update {
        id: Uuid,
        #[command(flatten)]
        args: RunnerUpdateArgs,
    },
    Delete {
        id: Uuid,
    },
    Validate {
        id: Uuid,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ExperimentCampaignsCommand {
    Start {
        project_id: Uuid,
        #[arg(long)]
        runner_profile_id: Option<Uuid>,
        #[arg(long)]
        max_trials_override: Option<u32>,
    },
    List,
    Show {
        id: Uuid,
    },
    Pause {
        id: Uuid,
    },
    Resume {
        id: Uuid,
    },
    Cancel {
        id: Uuid,
    },
    Promote {
        id: Uuid,
    },
    ReissueLease {
        id: Uuid,
        #[command(flatten)]
        gateway: ExperimentGatewayArgs,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ExperimentOpportunitiesCommand {
    List(ExperimentGatewayArgs),
}

#[derive(Subcommand, Debug, Clone)]
pub enum ExperimentTargetsCommand {
    List(ExperimentGatewayArgs),
    Link(ExperimentTargetLinkArgs),
    Update {
        id: Uuid,
        #[command(flatten)]
        args: ExperimentTargetUpdateArgs,
    },
    Delete {
        id: Uuid,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ExperimentProvidersCommand {
    List(ExperimentGatewayArgs),
    Connect(ExperimentProviderActionArgs),
    Validate(ExperimentProviderActionArgs),
    LaunchTest(ExperimentProviderActionArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct ExperimentGatewayArgs {
    #[arg(long)]
    pub gateway_url: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ExperimentTargetLinkArgs {
    #[command(flatten)]
    pub gateway: ExperimentGatewayArgs,
    #[arg(long)]
    pub opportunity_id: String,
    #[arg(long)]
    pub target_type: String,
    #[arg(long)]
    pub target_id: String,
    #[arg(long)]
    pub target_name: Option<String>,
    #[arg(long)]
    pub location: Option<String>,
    #[arg(long)]
    pub metadata_json: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ExperimentTargetUpdateArgs {
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub kind: Option<String>,
    #[arg(long)]
    pub location: Option<String>,
    #[arg(long)]
    pub metadata_json: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ExperimentProviderActionArgs {
    #[command(flatten)]
    pub gateway: ExperimentGatewayArgs,
    #[arg(long)]
    pub provider: String,
    #[arg(long)]
    pub payload_json: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ProjectCreateArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub workspace_path: String,
    #[arg(long)]
    pub git_remote_name: String,
    #[arg(long)]
    pub base_branch: String,
    #[arg(long, default_value = ".")]
    pub workdir: String,
    #[arg(long)]
    pub prepare_command: Option<String>,
    #[arg(long)]
    pub run_command: String,
    #[arg(long)]
    pub mutable_path: Vec<String>,
    #[arg(long)]
    pub fixed_path: Vec<String>,
    #[arg(long)]
    pub primary_metric_name: String,
    #[arg(long)]
    pub primary_metric_regex: Option<String>,
    #[arg(long)]
    pub primary_metric_json_path: Option<String>,
    #[arg(long, default_value = "lower_is_better")]
    pub primary_metric_comparator: String,
    #[arg(long)]
    pub strategy_prompt: Option<String>,
    #[arg(long)]
    pub runner_profile_id: Option<Uuid>,
    #[arg(long)]
    pub promotion_mode: Option<String>,
    #[arg(long)]
    pub autonomy_mode: Option<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ProjectUpdateArgs {
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub workspace_path: Option<String>,
    #[arg(long)]
    pub git_remote_name: Option<String>,
    #[arg(long)]
    pub base_branch: Option<String>,
    #[arg(long)]
    pub workdir: Option<String>,
    #[arg(long)]
    pub prepare_command: Option<String>,
    #[arg(long)]
    pub run_command: Option<String>,
    #[arg(long)]
    pub mutable_path: Vec<String>,
    #[arg(long)]
    pub fixed_path: Vec<String>,
    #[arg(long)]
    pub primary_metric_name: Option<String>,
    #[arg(long)]
    pub primary_metric_regex: Option<String>,
    #[arg(long)]
    pub primary_metric_json_path: Option<String>,
    #[arg(long)]
    pub primary_metric_comparator: Option<String>,
    #[arg(long)]
    pub strategy_prompt: Option<String>,
    #[arg(long)]
    pub runner_profile_id: Option<Uuid>,
    #[arg(long)]
    pub promotion_mode: Option<String>,
    #[arg(long)]
    pub autonomy_mode: Option<String>,
    #[arg(long)]
    pub status: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct RunnerCreateArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub backend: String,
    #[arg(long)]
    pub backend_config_json: Option<String>,
    #[arg(long)]
    pub image_or_runtime: Option<String>,
    #[arg(long)]
    pub gpu_requirements_json: Option<String>,
    #[arg(long)]
    pub env_grants_json: Option<String>,
    #[arg(long)]
    pub secret_reference: Vec<String>,
    #[arg(long)]
    pub cache_policy_json: Option<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct RunnerUpdateArgs {
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub backend: Option<String>,
    #[arg(long)]
    pub backend_config_json: Option<String>,
    #[arg(long)]
    pub image_or_runtime: Option<String>,
    #[arg(long)]
    pub gpu_requirements_json: Option<String>,
    #[arg(long)]
    pub env_grants_json: Option<String>,
    #[arg(long)]
    pub secret_reference: Vec<String>,
    #[arg(long)]
    pub cache_policy_json: Option<String>,
    #[arg(long)]
    pub status: Option<String>,
}

pub async fn run_experiments_command(cmd: ExperimentsCommand) -> anyhow::Result<()> {
    let needs_db = matches!(
        &cmd,
        ExperimentsCommand::Enable
            | ExperimentsCommand::Disable
            | ExperimentsCommand::Projects(_)
            | ExperimentsCommand::Runners(_)
            | ExperimentsCommand::Campaigns(_)
    );
    let db = if needs_db {
        Some(connect_db().await?)
    } else {
        None
    };
    match cmd {
        ExperimentsCommand::Enable => {
            let db = db.as_ref().expect("experiments DB must be initialized");
            db.set_setting(
                DEFAULT_USER_ID,
                "experiments.enabled",
                &serde_json::Value::Bool(true),
            )
            .await
            .map_err(|e| anyhow!("{}", e))?;
            println!("Experiments enabled.");
        }
        ExperimentsCommand::Disable => {
            let db = db.as_ref().expect("experiments DB must be initialized");
            db.set_setting(
                DEFAULT_USER_ID,
                "experiments.enabled",
                &serde_json::Value::Bool(false),
            )
            .await
            .map_err(|e| anyhow!("{}", e))?;
            println!("Experiments disabled.");
        }
        ExperimentsCommand::Projects(sub) => match sub {
            ExperimentProjectsCommand::List => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let response = experiments_api::list_projects(db, DEFAULT_USER_ID).await?;
                println!("{}", serde_json::to_string_pretty(&response.projects)?);
            }
            ExperimentProjectsCommand::Show { id } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let project = experiments_api::get_project(db, DEFAULT_USER_ID, id).await?;
                println!("{}", serde_json::to_string_pretty(&project)?);
            }
            ExperimentProjectsCommand::Create(args) => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let request = experiments_api::CreateExperimentProjectRequest {
                    name: args.name,
                    workspace_path: args.workspace_path,
                    git_remote_name: args.git_remote_name,
                    base_branch: args.base_branch,
                    preset: Some(ExperimentPreset::AutoresearchSingleFile),
                    strategy_prompt: args.strategy_prompt,
                    workdir: args.workdir,
                    prepare_command: args.prepare_command,
                    run_command: args.run_command,
                    mutable_paths: args.mutable_path,
                    fixed_paths: args.fixed_path,
                    primary_metric: metric_from_parts(
                        args.primary_metric_name,
                        args.primary_metric_regex,
                        args.primary_metric_json_path,
                        Some(args.primary_metric_comparator),
                    )?,
                    secondary_metrics: Vec::new(),
                    comparison_policy: None,
                    stop_policy: None,
                    default_runner_profile_id: args.runner_profile_id,
                    promotion_mode: args.promotion_mode,
                    autonomy_mode: args
                        .autonomy_mode
                        .as_deref()
                        .map(parse_autonomy_mode)
                        .transpose()?,
                };
                let project = experiments_api::create_project(db, DEFAULT_USER_ID, request).await?;
                println!("{}", serde_json::to_string_pretty(&project)?);
            }
            ExperimentProjectsCommand::Update { id, args } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let request = experiments_api::UpdateExperimentProjectRequest {
                    name: args.name,
                    workspace_path: args.workspace_path,
                    git_remote_name: args.git_remote_name,
                    base_branch: args.base_branch,
                    preset: None,
                    strategy_prompt: args.strategy_prompt,
                    workdir: args.workdir,
                    prepare_command: args.prepare_command,
                    run_command: args.run_command,
                    mutable_paths: (!args.mutable_path.is_empty()).then_some(args.mutable_path),
                    fixed_paths: (!args.fixed_path.is_empty()).then_some(args.fixed_path),
                    primary_metric: args
                        .primary_metric_name
                        .map(|name| {
                            metric_from_parts(
                                name,
                                args.primary_metric_regex,
                                args.primary_metric_json_path,
                                args.primary_metric_comparator,
                            )
                        })
                        .transpose()?,
                    secondary_metrics: None,
                    comparison_policy: None,
                    stop_policy: None,
                    default_runner_profile_id: args.runner_profile_id,
                    promotion_mode: args.promotion_mode,
                    autonomy_mode: args
                        .autonomy_mode
                        .as_deref()
                        .map(parse_autonomy_mode)
                        .transpose()?,
                    status: args
                        .status
                        .as_deref()
                        .map(parse_project_status)
                        .transpose()?,
                };
                let project =
                    experiments_api::update_project(db, DEFAULT_USER_ID, id, request).await?;
                println!("{}", serde_json::to_string_pretty(&project)?);
            }
            ExperimentProjectsCommand::Delete { id } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let deleted = experiments_api::delete_project(db, DEFAULT_USER_ID, id).await?;
                println!("{}", serde_json::json!({ "deleted": deleted }));
            }
        },
        ExperimentsCommand::Runners(sub) => match sub {
            ExperimentRunnersCommand::List => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let response = experiments_api::list_runners(db, DEFAULT_USER_ID).await?;
                println!("{}", serde_json::to_string_pretty(&response.runners)?);
            }
            ExperimentRunnersCommand::Show { id } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let runner = experiments_api::get_runner(db, DEFAULT_USER_ID, id).await?;
                println!("{}", serde_json::to_string_pretty(&runner)?);
            }
            ExperimentRunnersCommand::Create(args) => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let request = experiments_api::CreateExperimentRunnerProfileRequest {
                    name: args.name,
                    backend: parse_backend(&args.backend)?,
                    backend_config: parse_json_arg(
                        args.backend_config_json,
                        serde_json::json!({}),
                    )?,
                    image_or_runtime: args.image_or_runtime,
                    gpu_requirements: parse_json_arg(
                        args.gpu_requirements_json,
                        serde_json::json!({}),
                    )?,
                    env_grants: parse_json_arg(args.env_grants_json, serde_json::json!({}))?,
                    secret_references: args.secret_reference,
                    cache_policy: parse_json_arg(args.cache_policy_json, serde_json::json!({}))?,
                };
                let runner = experiments_api::create_runner(db, DEFAULT_USER_ID, request).await?;
                println!("{}", serde_json::to_string_pretty(&runner)?);
            }
            ExperimentRunnersCommand::Update { id, args } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let request = experiments_api::UpdateExperimentRunnerProfileRequest {
                    name: args.name,
                    backend: args.backend.as_deref().map(parse_backend).transpose()?,
                    backend_config: args
                        .backend_config_json
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                    image_or_runtime: args.image_or_runtime,
                    gpu_requirements: args
                        .gpu_requirements_json
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                    env_grants: args
                        .env_grants_json
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                    secret_references: (!args.secret_reference.is_empty())
                        .then_some(args.secret_reference),
                    cache_policy: args
                        .cache_policy_json
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                    status: args
                        .status
                        .as_deref()
                        .map(parse_runner_status)
                        .transpose()?,
                };
                let runner =
                    experiments_api::update_runner(db, DEFAULT_USER_ID, id, request).await?;
                println!("{}", serde_json::to_string_pretty(&runner)?);
            }
            ExperimentRunnersCommand::Delete { id } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let deleted = experiments_api::delete_runner(db, DEFAULT_USER_ID, id).await?;
                println!("{}", serde_json::json!({ "deleted": deleted }));
            }
            ExperimentRunnersCommand::Validate { id } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let response = experiments_api::validate_runner(db, DEFAULT_USER_ID, id).await?;
                println!("{}", serde_json::to_string_pretty(&response)?);
            }
        },
        ExperimentsCommand::Campaigns(sub) => match sub {
            ExperimentCampaignsCommand::Start {
                project_id,
                runner_profile_id,
                max_trials_override,
            } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let response = experiments_api::start_campaign(
                    db,
                    DEFAULT_USER_ID,
                    project_id,
                    experiments_api::StartExperimentCampaignRequest {
                        runner_profile_id,
                        max_trials_override,
                        gateway_url: None,
                    },
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&response)?);
            }
            ExperimentCampaignsCommand::List => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let response = experiments_api::list_campaigns(db, DEFAULT_USER_ID).await?;
                println!("{}", serde_json::to_string_pretty(&response.campaigns)?);
            }
            ExperimentCampaignsCommand::Show { id } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let response = experiments_api::get_campaign(db, DEFAULT_USER_ID, id).await?;
                println!("{}", serde_json::to_string_pretty(&response)?);
            }
            ExperimentCampaignsCommand::Pause { id } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let response = experiments_api::pause_campaign(db, DEFAULT_USER_ID, id).await?;
                println!("{}", serde_json::to_string_pretty(&response)?);
            }
            ExperimentCampaignsCommand::Resume { id } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let response = experiments_api::resume_campaign(db, DEFAULT_USER_ID, id).await?;
                println!("{}", serde_json::to_string_pretty(&response)?);
            }
            ExperimentCampaignsCommand::Cancel { id } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let response = experiments_api::cancel_campaign(db, DEFAULT_USER_ID, id).await?;
                println!("{}", serde_json::to_string_pretty(&response)?);
            }
            ExperimentCampaignsCommand::Promote { id } => {
                let db = db.as_ref().expect("experiments DB must be initialized");
                let response = experiments_api::promote_campaign(db, DEFAULT_USER_ID, id).await?;
                println!("{}", serde_json::to_string_pretty(&response)?);
            }
            ExperimentCampaignsCommand::ReissueLease { id, gateway } => {
                let response = experiments_gateway_request(
                    Method::POST,
                    &format!("/campaigns/{id}/reissue-lease"),
                    gateway.gateway_url,
                    None,
                )
                .await?;
                println!("{response}");
            }
        },
        ExperimentsCommand::Opportunities(sub) => match sub {
            ExperimentOpportunitiesCommand::List(gateway) => {
                let response = experiments_gateway_request(
                    Method::GET,
                    "/opportunities",
                    gateway.gateway_url,
                    None,
                )
                .await?;
                println!("{response}");
            }
        },
        ExperimentsCommand::Targets(sub) => match sub {
            ExperimentTargetsCommand::List(gateway) => {
                let response =
                    experiments_gateway_request(Method::GET, "/targets", gateway.gateway_url, None)
                        .await?;
                println!("{response}");
            }
            ExperimentTargetsCommand::Link(args) => {
                let payload = serde_json::json!({
                    "opportunity_id": args.opportunity_id,
                    "target_type": parse_target_kind(&args.target_type)?,
                    "target_id": args.target_id,
                    "target_name": args.target_name,
                    "location": args.location,
                    "metadata": parse_json_arg(args.metadata_json, serde_json::json!({}))?,
                });
                let response = experiments_gateway_request(
                    Method::POST,
                    "/targets/link",
                    args.gateway.gateway_url,
                    Some(payload),
                )
                .await?;
                println!("{response}");
            }
            ExperimentTargetsCommand::Update { id, args } => {
                let payload = serde_json::json!({
                    "name": args.name,
                    "kind": args.kind.as_ref().map(|value| parse_target_kind(value)).transpose()?,
                    "location": args.location,
                    "metadata": args.metadata_json.map(|value| parse_json_arg(Some(value), serde_json::json!(null))).transpose()?,
                });
                let response = experiments_gateway_request(
                    Method::PATCH,
                    &format!("/targets/{id}"),
                    None,
                    Some(payload),
                )
                .await?;
                println!("{response}");
            }
            ExperimentTargetsCommand::Delete { id } => {
                let response = experiments_gateway_request(
                    Method::DELETE,
                    &format!("/targets/{id}"),
                    None,
                    None,
                )
                .await?;
                println!("{response}");
            }
        },
        ExperimentsCommand::Providers(sub) => match sub {
            ExperimentProvidersCommand::List(gateway) => {
                let response = experiments_gateway_request(
                    Method::GET,
                    "/providers/gpu-clouds",
                    gateway.gateway_url,
                    None,
                )
                .await?;
                println!("{response}");
            }
            ExperimentProvidersCommand::Connect(args) => {
                let payload = serde_json::json!({
                    "provider": args.provider,
                    "payload": parse_json_arg(args.payload_json, serde_json::json!({}))?,
                });
                let response = experiments_gateway_request(
                    Method::POST,
                    &format!("/providers/gpu-clouds/{}/connect", args.provider),
                    args.gateway.gateway_url,
                    Some(payload),
                )
                .await?;
                println!("{response}");
            }
            ExperimentProvidersCommand::Validate(args) => {
                let payload = serde_json::json!({
                    "provider": args.provider,
                    "payload": parse_json_arg(args.payload_json, serde_json::json!({}))?,
                });
                let response = experiments_gateway_request(
                    Method::POST,
                    &format!("/providers/gpu-clouds/{}/validate", args.provider),
                    args.gateway.gateway_url,
                    Some(payload),
                )
                .await?;
                println!("{response}");
            }
            ExperimentProvidersCommand::LaunchTest(args) => {
                let payload = serde_json::json!({
                    "provider": args.provider,
                    "payload": parse_json_arg(args.payload_json, serde_json::json!({}))?,
                });
                let response = experiments_gateway_request(
                    Method::POST,
                    &format!("/providers/gpu-clouds/{}/launch-test", args.provider),
                    args.gateway.gateway_url,
                    Some(payload),
                )
                .await?;
                println!("{response}");
            }
        },
    }
    Ok(())
}

async fn connect_db() -> anyhow::Result<Arc<dyn Database>> {
    let config = crate::config::Config::from_env()
        .await
        .map_err(|e| anyhow!("{}", e))?;
    crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow!("{}", e))
}

fn parse_json_arg(
    value: Option<String>,
    default: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    match value {
        Some(value) => serde_json::from_str(&value).map_err(|e| anyhow!("{}", e)),
        None => Ok(default),
    }
}

fn metric_from_parts(
    name: String,
    regex: Option<String>,
    json_path: Option<String>,
    comparator: Option<String>,
) -> anyhow::Result<ExperimentMetricDefinition> {
    Ok(ExperimentMetricDefinition {
        name,
        regex,
        json_path,
        comparator: comparator
            .as_deref()
            .map(parse_comparator)
            .transpose()?
            .unwrap_or(ExperimentMetricComparator::LowerIsBetter),
    })
}

fn parse_backend(value: &str) -> anyhow::Result<ExperimentRunnerBackend> {
    match value {
        "local_docker" => Ok(ExperimentRunnerBackend::LocalDocker),
        "generic_remote_runner" => Ok(ExperimentRunnerBackend::GenericRemoteRunner),
        "ssh" => Ok(ExperimentRunnerBackend::Ssh),
        "slurm" => Ok(ExperimentRunnerBackend::Slurm),
        "kubernetes" => Ok(ExperimentRunnerBackend::Kubernetes),
        "runpod" => Ok(ExperimentRunnerBackend::Runpod),
        "vast" => Ok(ExperimentRunnerBackend::Vast),
        "lambda" => Ok(ExperimentRunnerBackend::Lambda),
        other => Err(anyhow!("unknown backend: {other}")),
    }
}

fn parse_comparator(value: &str) -> anyhow::Result<ExperimentMetricComparator> {
    match value {
        "lower_is_better" => Ok(ExperimentMetricComparator::LowerIsBetter),
        "higher_is_better" => Ok(ExperimentMetricComparator::HigherIsBetter),
        "equal_is_better" => Ok(ExperimentMetricComparator::EqualIsBetter),
        other => Err(anyhow!("unknown comparator: {other}")),
    }
}

fn parse_project_status(
    value: &str,
) -> anyhow::Result<crate::experiments::ExperimentProjectStatus> {
    match value {
        "draft" => Ok(crate::experiments::ExperimentProjectStatus::Draft),
        "ready" => Ok(crate::experiments::ExperimentProjectStatus::Ready),
        "archived" => Ok(crate::experiments::ExperimentProjectStatus::Archived),
        other => Err(anyhow!("unknown project status: {other}")),
    }
}

fn parse_autonomy_mode(value: &str) -> anyhow::Result<ExperimentAutonomyMode> {
    match value {
        "autonomous" => Ok(ExperimentAutonomyMode::Autonomous),
        "manual_candidate" => Ok(ExperimentAutonomyMode::ManualCandidate),
        "suggest_only" => Ok(ExperimentAutonomyMode::SuggestOnly),
        other => Err(anyhow!("unknown autonomy mode: {other}")),
    }
}

fn parse_runner_status(value: &str) -> anyhow::Result<crate::experiments::ExperimentRunnerStatus> {
    match value {
        "draft" => Ok(crate::experiments::ExperimentRunnerStatus::Draft),
        "validated" => Ok(crate::experiments::ExperimentRunnerStatus::Validated),
        "unavailable" => Ok(crate::experiments::ExperimentRunnerStatus::Unavailable),
        other => Err(anyhow!("unknown runner status: {other}")),
    }
}

fn parse_target_kind(value: &str) -> anyhow::Result<crate::experiments::ExperimentTargetKind> {
    match value {
        "prompt_asset" => Ok(crate::experiments::ExperimentTargetKind::PromptAsset),
        "routing_policy" => Ok(crate::experiments::ExperimentTargetKind::RoutingPolicy),
        "rag_config" => Ok(crate::experiments::ExperimentTargetKind::RagConfig),
        "tool_policy" => Ok(crate::experiments::ExperimentTargetKind::ToolPolicy),
        "evaluator" => Ok(crate::experiments::ExperimentTargetKind::Evaluator),
        "parser" => Ok(crate::experiments::ExperimentTargetKind::Parser),
        "inference_config" => Ok(crate::experiments::ExperimentTargetKind::InferenceConfig),
        "training_config" => Ok(crate::experiments::ExperimentTargetKind::TrainingConfig),
        "training_code" => Ok(crate::experiments::ExperimentTargetKind::TrainingCode),
        "serving_config" => Ok(crate::experiments::ExperimentTargetKind::ServingConfig),
        other => Err(anyhow!("unknown target kind: {other}")),
    }
}

fn resolve_gateway_base_url(override_url: Option<String>) -> String {
    override_url.unwrap_or_else(|| {
        if let Ok(url) = std::env::var("GATEWAY_URL") {
            return url;
        }

        let port = std::env::var("GATEWAY_PORT").unwrap_or_else(|_| "3000".to_string());
        let host = std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        format!("http://{}:{}", host, port)
    })
}

async fn experiments_gateway_request(
    method: Method,
    path: &str,
    gateway_url: Option<String>,
    body: Option<serde_json::Value>,
) -> anyhow::Result<String> {
    let base_url = resolve_gateway_base_url(gateway_url);
    let url = format!("{}/api/experiments{}", base_url.trim_end_matches('/'), path);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let mut request = client.request(method, &url);
    if let Ok(token) = std::env::var("GATEWAY_AUTH_TOKEN") {
        request = request.bearer_auth(token);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }

    let response = request.send().await.map_err(|e| {
        if e.is_connect() {
            anyhow!(
                "Could not connect to gateway at {}. Start it with `thinclaw gateway start` or pass --gateway-url.",
                base_url
            )
        } else {
            anyhow!("Request to {} failed: {}", url, e)
        }
    })?;

    let status = response.status();
    let body_text = response.text().await.unwrap_or_default();

    if !status.is_success() {
        if status == reqwest::StatusCode::NOT_FOUND {
            anyhow::bail!(
                "Gateway returned HTTP 404 for {}. The experiments API route is not available yet.",
                url
            );
        }
        anyhow::bail!("Gateway returned HTTP {}: {}", status.as_u16(), body_text);
    }

    let trimmed = body_text.trim();
    if trimmed.is_empty() {
        return Ok("{}".to_string());
    }

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::to_string_pretty(&json)?)
    } else {
        Ok(body_text)
    }
}
