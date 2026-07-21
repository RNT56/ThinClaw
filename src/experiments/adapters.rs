use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::Url;

use crate::experiments::{
    ExperimentLeaseAuthentication, ExperimentRunnerBackend, ExperimentRunnerProfile,
    ExperimentRunnerReadinessClass, parse_secret_reference,
};
use crate::settings::Settings;
use crate::tools::execution_backend::{
    ExecutionBackend, ExecutionResult, LocalHostExecutionBackend, ScriptExecutionRequest,
};

fn docker_sandbox_config(settings: &Settings) -> crate::sandbox::SandboxConfig {
    crate::config::SandboxModeConfig::resolve(settings)
        .unwrap_or_else(|_| crate::config::SandboxModeConfig {
            enabled: settings.sandbox.enabled,
            policy: settings.sandbox.policy.clone(),
            timeout_secs: settings.sandbox.timeout_secs,
            memory_limit_mb: settings.sandbox.memory_limit_mb,
            cpu_shares: settings.sandbox.cpu_shares,
            image: settings.sandbox.image.clone(),
            interactive_idle_timeout_secs: settings.sandbox.interactive_idle_timeout_secs,
            auto_pull_image: settings.sandbox.auto_pull_image,
            extra_allowed_domains: settings.sandbox.extra_allowed_domains.clone(),
        })
        .to_sandbox_config()
}

#[derive(Clone)]
pub struct RunnerLaunchOutcome {
    pub message: String,
    pub bootstrap_command: Option<String>,
    pub provider_template: Option<serde_json::Value>,
    pub provider_job_id: Option<String>,
    pub provider_job_metadata: serde_json::Value,
    pub auto_launched: bool,
    pub requires_operator_action: bool,
}

impl fmt::Debug for RunnerLaunchOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunnerLaunchOutcome")
            .field("message", &self.message)
            .field(
                "bootstrap_command",
                &self.bootstrap_command.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "provider_template",
                &self.provider_template.as_ref().map(|_| "[REDACTED]"),
            )
            .field("provider_job_id", &self.provider_job_id)
            .field("provider_job_metadata", &"[OMITTED]")
            .field("auto_launched", &self.auto_launched)
            .field("requires_operator_action", &self.requires_operator_action)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct RunnerValidationOutcome {
    pub valid: bool,
    pub message: String,
    pub readiness_class: ExperimentRunnerReadinessClass,
    pub launch_eligible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteLaunchAction {
    Pause,
    Cancel,
    Reissue,
}

const RUNPOD_API_BASE: &str = "https://rest.runpod.io/v1";
const VAST_API_BASE: &str = "https://console.vast.ai";
const LAMBDA_API_BASE: &str = "https://cloud.lambda.ai/api/v1";
const DEFAULT_RESEARCH_RUNNER_IMAGE: &str = "ghcr.io/thinclaw/research-runner:latest";
const REMOTE_ADAPTER_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const PROVIDER_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PROVIDER_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_PROVIDER_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_PROVIDER_ID_BYTES: usize = 128;
const MAX_METADATA_STRING_BYTES: usize = 2 * 1024;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "benchmark", rename_all = "snake_case")]
enum AgentEnvRunnerConfig {
    TerminalBench {
        #[serde(default)]
        cases: Vec<crate::agent::env::TerminalBenchCase>,
    },
    SkillBench {
        #[serde(default)]
        cases: Vec<crate::agent::env::SkillBenchCase>,
    },
}

fn validate_agent_env_runner_config(runner: &ExperimentRunnerProfile) -> Result<(), String> {
    let source = runner
        .backend_config
        .get("agent_env")
        .or_else(|| runner.backend_config.get("benchmark_config"))
        .unwrap_or(&runner.backend_config);
    if source.get("benchmark").is_none() {
        return Ok(());
    }
    serde_json::from_value::<AgentEnvRunnerConfig>(source.clone())
        .map(|config| match config {
            AgentEnvRunnerConfig::TerminalBench { cases } => {
                let _ = cases.len();
            }
            AgentEnvRunnerConfig::SkillBench { cases } => {
                let _ = cases.len();
            }
        })
        .map_err(|err| format!("Invalid AgentEnv benchmark config: {err}"))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct LambdaLaunchTemplateInput {
    #[serde(default)]
    pub region_name: Option<String>,
    pub instance_type_name: String,
    #[serde(default = "default_lambda_quantity")]
    pub quantity: u32,
    #[serde(default)]
    pub ssh_key_names: Vec<String>,
    #[serde(default)]
    pub file_system_names: Vec<String>,
}

fn default_lambda_quantity() -> u32 {
    1
}

pub async fn validate_runner_profile(
    runner: &ExperimentRunnerProfile,
    settings: &Settings,
    provider_api_key: Option<&str>,
) -> RunnerValidationOutcome {
    if runner.backend.is_remote() && !settings.experiments.allow_remote_runners {
        return RunnerValidationOutcome {
            valid: false,
            message: "Remote runners are disabled in settings.".to_string(),
            readiness_class: ExperimentRunnerReadinessClass::ManualOnly,
            launch_eligible: false,
        };
    }

    match runner.backend {
        ExperimentRunnerBackend::LocalDocker => {
            let mut sandbox_config = docker_sandbox_config(settings);
            sandbox_config.enabled = true;
            sandbox_config.policy = crate::sandbox::SandboxPolicy::WorkspaceWrite;
            if let Some(image) = runner
                .image_or_runtime
                .as_ref()
                .filter(|value| !value.trim().is_empty())
            {
                sandbox_config.image = image.trim().to_string();
            }
            let sandbox = crate::sandbox::SandboxManager::new(sandbox_config.clone());
            if sandbox.is_available().await {
                RunnerValidationOutcome {
                    valid: true,
                    message: format!(
                        "Docker sandbox is reachable for local_docker using image '{}'.",
                        sandbox_config.image
                    ),
                    readiness_class: ExperimentRunnerReadinessClass::LaunchReady,
                    launch_eligible: true,
                }
            } else {
                RunnerValidationOutcome {
                    valid: false,
                    message: "Docker sandbox is not reachable on this host.".to_string(),
                    readiness_class: ExperimentRunnerReadinessClass::ManualOnly,
                    launch_eligible: false,
                }
            }
        }
        ExperimentRunnerBackend::AgentEnv => match validate_agent_env_runner_config(runner) {
            Ok(()) => RunnerValidationOutcome {
                valid: true,
                message: "AgentEnv benchmark runner is available for local EnvRunner campaigns."
                    .to_string(),
                readiness_class: ExperimentRunnerReadinessClass::LaunchReady,
                launch_eligible: true,
            },
            Err(message) => RunnerValidationOutcome {
                valid: false,
                message,
                readiness_class: ExperimentRunnerReadinessClass::ManualOnly,
                launch_eligible: false,
            },
        },
        ExperimentRunnerBackend::GenericRemoteRunner => RunnerValidationOutcome {
            valid: true,
            message: "Generic remote runner lease flow is available.".to_string(),
            readiness_class: ExperimentRunnerReadinessClass::ManualOnly,
            launch_eligible: false,
        },
        ExperimentRunnerBackend::Ssh => {
            let host_ok = backend_string(runner, "host").is_some();
            if host_ok && command_exists("ssh").await {
                RunnerValidationOutcome {
                    valid: true,
                    message: "SSH backend is configured for outbound bootstrap.".to_string(),
                    readiness_class: ExperimentRunnerReadinessClass::LaunchReady,
                    launch_eligible: true,
                }
            } else if host_ok {
                RunnerValidationOutcome {
                    valid: true,
                    message: "SSH backend can build bootstrap commands, but this host cannot controller-launch without the ssh binary.".to_string(),
                    readiness_class: ExperimentRunnerReadinessClass::BootstrapReady,
                    launch_eligible: false,
                }
            } else {
                RunnerValidationOutcome {
                    valid: false,
                    message: "SSH backend requires backend_config.host and the ssh binary."
                        .to_string(),
                    readiness_class: ExperimentRunnerReadinessClass::ManualOnly,
                    launch_eligible: false,
                }
            }
        }
        ExperimentRunnerBackend::Slurm => {
            let login_host_ok = backend_string(runner, "login_host").is_some();
            if login_host_ok && command_exists("ssh").await {
                RunnerValidationOutcome {
                    valid: true,
                    message: "Slurm backend is configured for sbatch launch via SSH.".to_string(),
                    readiness_class: ExperimentRunnerReadinessClass::LaunchReady,
                    launch_eligible: true,
                }
            } else if login_host_ok {
                RunnerValidationOutcome {
                    valid: true,
                    message: "Slurm backend can build sbatch bootstrap instructions, but this host cannot controller-launch without the ssh binary.".to_string(),
                    readiness_class: ExperimentRunnerReadinessClass::BootstrapReady,
                    launch_eligible: false,
                }
            } else {
                RunnerValidationOutcome {
                    valid: false,
                    message: "Slurm backend requires backend_config.login_host and the ssh binary."
                        .to_string(),
                    readiness_class: ExperimentRunnerReadinessClass::ManualOnly,
                    launch_eligible: false,
                }
            }
        }
        ExperimentRunnerBackend::Kubernetes => {
            let namespace_ok = backend_string(runner, "namespace").is_some();
            let image_ok =
                runner.image_or_runtime.is_some() || backend_string(runner, "image").is_some();
            if namespace_ok && image_ok && command_exists("kubectl").await {
                RunnerValidationOutcome {
                    valid: true,
                    message: "Kubernetes backend is configured for Job launch.".to_string(),
                    readiness_class: ExperimentRunnerReadinessClass::LaunchReady,
                    launch_eligible: true,
                }
            } else if namespace_ok && image_ok {
                RunnerValidationOutcome {
                    valid: true,
                    message: "Kubernetes backend can build launch manifests, but this host cannot controller-launch without kubectl.".to_string(),
                    readiness_class: ExperimentRunnerReadinessClass::BootstrapReady,
                    launch_eligible: false,
                }
            } else {
                RunnerValidationOutcome {
                    valid: false,
                    message: "Kubernetes backend requires namespace, image/image_or_runtime, and kubectl."
                        .to_string(),
                    readiness_class: ExperimentRunnerReadinessClass::ManualOnly,
                    launch_eligible: false,
                }
            }
        }
        ExperimentRunnerBackend::Runpod
        | ExperimentRunnerBackend::Vast
        | ExperimentRunnerBackend::Lambda => {
            let secret_ok = gpu_cloud_secret_name(runner.backend)
                .map(|name| {
                    runner.secret_references.iter().any(|reference| {
                        parse_secret_reference(reference)
                            .is_some_and(|(secret_name, _)| secret_name == name)
                    })
                })
                .unwrap_or(false);
            let lambda_launch_payload_ok = runner.backend == ExperimentRunnerBackend::Lambda
                && runner
                    .backend_config
                    .get("launch_payload")
                    .is_some_and(|value| value.is_object());
            let template_ok = backend_string(runner, "template_id").is_some()
                || runner.image_or_runtime.is_some();
            let template_ok = template_ok || lambda_launch_payload_ok;
            if secret_ok && template_ok {
                if let Some(api_key) = provider_api_key {
                    match validate_gpu_cloud_credentials(runner.backend, api_key).await {
                        Ok(message) => {
                            if runner.backend == ExperimentRunnerBackend::Lambda
                                && !lambda_launch_payload_ok
                            {
                                RunnerValidationOutcome {
                                    valid: true,
                                    message: format!(
                                        "{message} Controller-managed Lambda launches require backend_config.launch_payload with the official Lambda Cloud API launch body; until then, this runner stays in manual bootstrap/template mode."
                                    ),
                                    readiness_class: ExperimentRunnerReadinessClass::BootstrapReady,
                                    launch_eligible: false,
                                }
                            } else {
                                RunnerValidationOutcome {
                                    valid: true,
                                    message,
                                    readiness_class: ExperimentRunnerReadinessClass::LaunchReady,
                                    launch_eligible: true,
                                }
                            }
                        }
                        Err(message) => RunnerValidationOutcome {
                            valid: false,
                            message,
                            readiness_class: ExperimentRunnerReadinessClass::ManualOnly,
                            launch_eligible: false,
                        },
                    }
                } else if runner.backend == ExperimentRunnerBackend::Lambda
                    && !lambda_launch_payload_ok
                {
                    RunnerValidationOutcome {
                        valid: true,
                        message: "Lambda runner is configured with provider credentials, but live validation was skipped because no decrypted API key is available in this process. Add backend_config.launch_payload to enable controller-managed launches; otherwise the runner will use the manual bootstrap/template path.".to_string(),
                        readiness_class: ExperimentRunnerReadinessClass::BootstrapReady,
                        launch_eligible: false,
                    }
                } else {
                    RunnerValidationOutcome {
                        valid: true,
                        message: format!(
                            "{} runner is configured with provider credentials and a launch template. Live provider validation was skipped because no decrypted API key is available in this process.",
                            gpu_cloud_display_name(runner.backend)
                        ),
                        readiness_class: ExperimentRunnerReadinessClass::BootstrapReady,
                        launch_eligible: false,
                    }
                }
            } else {
                RunnerValidationOutcome {
                    valid: false,
                    message: format!(
                        "{} backend requires its provider secret reference plus template/image metadata.",
                        gpu_cloud_display_name(runner.backend)
                    ),
                    readiness_class: ExperimentRunnerReadinessClass::ManualOnly,
                    launch_eligible: false,
                }
            }
        }
    }
}

pub fn validate_gateway_url(gateway_url: &str) -> Result<String, String> {
    let trimmed = gateway_url.trim();
    if trimmed.is_empty() {
        return Err(
            "Experiment runner launch requires a non-empty gateway_url (set campaign.gateway_url)."
                .to_string(),
        );
    }
    let url = Url::parse(trimmed)
        .map_err(|_| "Experiment gateway_url must be a valid absolute URL.".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(
            "Experiment gateway_url must use HTTP or HTTPS and include a host.".to_string(),
        );
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Experiment gateway_url must not contain embedded credentials.".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("Experiment gateway_url must not contain a query or fragment.".to_string());
    }
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if url.scheme() != "https" && !loopback {
        return Err(
            "Experiment gateway_url must use HTTPS unless it targets a loopback host.".to_string(),
        );
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub fn build_bootstrap_command(
    gateway_url: &str,
    auth: &ExperimentLeaseAuthentication,
) -> Result<String, String> {
    let gateway_url = validate_gateway_url(gateway_url)?;
    Ok(format!(
        "thinclaw experiment-runner --lease-id {} --gateway-url {} --token {}",
        auth.lease_id,
        sh_single_quote(&gateway_url),
        sh_single_quote(&auth.token)
    ))
}

pub fn gpu_cloud_display_name(backend: ExperimentRunnerBackend) -> &'static str {
    match backend {
        ExperimentRunnerBackend::Runpod => "RunPod",
        ExperimentRunnerBackend::Vast => "Vast.ai",
        ExperimentRunnerBackend::Lambda => "Lambda",
        _ => "Remote runner",
    }
}

pub fn default_research_runner_image() -> &'static str {
    DEFAULT_RESEARCH_RUNNER_IMAGE
}

pub fn gpu_cloud_default_runner_name(backend: ExperimentRunnerBackend) -> &'static str {
    match backend {
        ExperimentRunnerBackend::Runpod => "RunPod GPU Runner",
        ExperimentRunnerBackend::Vast => "Vast.ai GPU Runner",
        ExperimentRunnerBackend::Lambda => "Lambda GPU Runner",
        _ => "Research Runner",
    }
}

pub fn gpu_cloud_default_gpu_requirements(backend: ExperimentRunnerBackend) -> serde_json::Value {
    match backend {
        ExperimentRunnerBackend::Runpod => {
            serde_json::json!({ "gpu_count": 1, "gpu_type": "H100" })
        }
        ExperimentRunnerBackend::Vast => {
            serde_json::json!({ "gpu_count": 1, "accelerator": "gpu" })
        }
        ExperimentRunnerBackend::Lambda => {
            serde_json::json!({ "gpu_count": 1, "gpu_type": "A100" })
        }
        _ => serde_json::json!({}),
    }
}

pub fn gpu_cloud_default_backend_config(backend: ExperimentRunnerBackend) -> serde_json::Value {
    match backend {
        ExperimentRunnerBackend::Runpod => serde_json::json!({
            "provider": "runpod",
            "template_mode": "lease",
        }),
        ExperimentRunnerBackend::Vast => serde_json::json!({
            "provider": "vast",
            "launch_mode": "template",
        }),
        ExperimentRunnerBackend::Lambda => serde_json::json!({
            "provider": "lambda",
            "launch_mode": "api",
        }),
        _ => serde_json::json!({}),
    }
}

pub fn gpu_cloud_template_hint(backend: ExperimentRunnerBackend) -> serde_json::Value {
    let mut hint = serde_json::json!({
        "backend": backend.slug(),
        "recommended_secret_reference": gpu_cloud_secret_name(backend),
        "default_runner_name": gpu_cloud_default_runner_name(backend),
        "default_image_or_runtime": default_research_runner_image(),
        "default_gpu_requirements": gpu_cloud_default_gpu_requirements(backend),
    });
    if backend == ExperimentRunnerBackend::Lambda {
        hint["launch_builder"] = serde_json::json!("normalized_lambda_form");
        hint["launch_mode"] = serde_json::json!("api");
        hint["quantity_limit"] = serde_json::json!(1);
        hint["quantity_note"] = serde_json::json!(
            "ThinClaw currently launches one Lambda instance per research trial so exactly one runner can claim the lease."
        );
        hint["field_defaults"] = serde_json::json!({
            "region_name": "",
            "instance_type_name": "",
            "quantity": 1,
            "ssh_key_names": [],
            "file_system_names": [],
        });
    }
    hint
}

pub fn build_lambda_backend_config(
    input: &LambdaLaunchTemplateInput,
) -> (serde_json::Value, Vec<String>) {
    let mut warnings = Vec::new();
    let normalized_quantity = if input.quantity == 0 {
        1
    } else {
        input.quantity
    };
    let launch_quantity = if normalized_quantity > 1 {
        warnings.push(
            "ThinClaw currently launches one Lambda instance per research trial, so quantity was normalized to 1."
                .to_string(),
        );
        1
    } else {
        normalized_quantity
    };
    let region_name = input
        .region_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let ssh_key_names = input
        .ssh_key_names
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let file_system_names = input
        .file_system_names
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut launch_payload = serde_json::Map::new();
    launch_payload.insert("name".to_string(), serde_json::json!("{{THINCLAW_NAME}}"));
    launch_payload.insert(
        "instance_type_name".to_string(),
        serde_json::json!(input.instance_type_name.trim()),
    );
    launch_payload.insert("quantity".to_string(), serde_json::json!(launch_quantity));
    launch_payload.insert("image".to_string(), serde_json::json!("{{THINCLAW_IMAGE}}"));
    launch_payload.insert(
        "cloud_init".to_string(),
        serde_json::json!("#cloud-config\nruncmd:\n  - {{THINCLAW_BOOTSTRAP}}"),
    );
    if let Some(region_name) = &region_name {
        launch_payload.insert("region_name".to_string(), serde_json::json!(region_name));
    }
    if !ssh_key_names.is_empty() {
        launch_payload.insert(
            "ssh_key_names".to_string(),
            serde_json::json!(ssh_key_names),
        );
    }
    if !file_system_names.is_empty() {
        launch_payload.insert(
            "file_system_names".to_string(),
            serde_json::json!(file_system_names),
        );
    }
    (
        serde_json::json!({
            "provider": "lambda",
            "launch_mode": "api",
            "region_name": region_name,
            "instance_type_name": input.instance_type_name.trim(),
            "quantity": launch_quantity,
            "ssh_key_names": ssh_key_names,
            "file_system_names": file_system_names,
            "launch_payload": serde_json::Value::Object(launch_payload),
            "terminate_payload": {
                "instance_ids": ["{{THINCLAW_PROVIDER_JOB_ID}}"],
            },
        }),
        warnings,
    )
}

pub fn gpu_cloud_secret_name(backend: ExperimentRunnerBackend) -> Option<&'static str> {
    match backend {
        ExperimentRunnerBackend::Runpod => Some("research_runpod_api_key"),
        ExperimentRunnerBackend::Vast => Some("research_vast_api_key"),
        ExperimentRunnerBackend::Lambda => Some("research_lambda_api_key"),
        _ => None,
    }
}

pub fn gpu_cloud_signup_url(backend: ExperimentRunnerBackend) -> Option<&'static str> {
    match backend {
        ExperimentRunnerBackend::Runpod => Some("https://www.runpod.io"),
        ExperimentRunnerBackend::Vast => Some("https://vast.ai"),
        ExperimentRunnerBackend::Lambda => Some("https://cloud.lambda.ai"),
        _ => None,
    }
}

pub fn gpu_cloud_docs_url(backend: ExperimentRunnerBackend) -> Option<&'static str> {
    match backend {
        ExperimentRunnerBackend::Runpod => Some("https://docs.runpod.io"),
        ExperimentRunnerBackend::Vast => Some("https://docs.vast.ai"),
        ExperimentRunnerBackend::Lambda => {
            Some("https://docs.lambda.ai/public-cloud/on-demand/creating-managing-instances/")
        }
        _ => None,
    }
}

pub fn build_gpu_cloud_template(
    runner: &ExperimentRunnerProfile,
    command: &str,
) -> Option<serde_json::Value> {
    let image = runner
        .image_or_runtime
        .clone()
        .or_else(|| backend_string(runner, "image"))
        .unwrap_or_else(|| DEFAULT_RESEARCH_RUNNER_IMAGE.to_string());
    let gpu_requirements = runner.gpu_requirements.clone();

    match runner.backend {
        ExperimentRunnerBackend::Runpod => Some(serde_json::json!({
            "provider": "runpod",
            "template_id": backend_string(runner, "template_id"),
            "image_name": image,
            "gpu_requirements": gpu_requirements,
            "docker_args": command,
        })),
        ExperimentRunnerBackend::Vast => Some(serde_json::json!({
            "provider": "vast",
            "image": image,
            "gpu_requirements": gpu_requirements,
            "onstart_cmd": command,
        })),
        ExperimentRunnerBackend::Lambda => Some(serde_json::json!({
            "provider": "lambda",
            "image": image,
            "gpu_requirements": gpu_requirements,
            "cloud_init": format!("#cloud-config\nruncmd:\n  - {}", command),
        })),
        _ => None,
    }
}

pub async fn try_auto_launch(
    runner: &ExperimentRunnerProfile,
    gateway_url: Option<&str>,
    auth: &ExperimentLeaseAuthentication,
    provider_api_key: Option<&str>,
) -> Result<RunnerLaunchOutcome, String> {
    let gateway_url = validate_gateway_url(gateway_url.unwrap_or_default())?;
    let bootstrap_command = build_bootstrap_command(&gateway_url, auth)?;

    match runner.backend {
        ExperimentRunnerBackend::GenericRemoteRunner => Ok(RunnerLaunchOutcome {
            message: "Remote trial prepared. Launch the generic remote runner manually."
                .to_string(),
            bootstrap_command: Some(bootstrap_command),
            provider_template: None,
            provider_job_id: None,
            provider_job_metadata: serde_json::json!({}),
            auto_launched: false,
            requires_operator_action: true,
        }),
        ExperimentRunnerBackend::Runpod => {
            let api_key = provider_api_key.ok_or_else(|| {
                "RunPod launch requires a connected research_runpod_api_key secret.".to_string()
            })?;
            launch_runpod_pod(runner, auth, &bootstrap_command, api_key).await
        }
        ExperimentRunnerBackend::Vast => {
            let api_key = provider_api_key.ok_or_else(|| {
                "Vast.ai launch requires a connected research_vast_api_key secret.".to_string()
            })?;
            launch_vast_instance(runner, auth, &bootstrap_command, api_key).await
        }
        ExperimentRunnerBackend::Lambda => {
            let api_key = provider_api_key.ok_or_else(|| {
                "Lambda launch requires a connected research_lambda_api_key secret.".to_string()
            })?;
            if lambda_launch_payload(runner, &bootstrap_command, auth).is_some() {
                launch_lambda_instance(runner, auth, &bootstrap_command, api_key).await
            } else {
                Ok(RunnerLaunchOutcome {
                    message: "Lambda credentials are connected, but controller-managed launch requires backend_config.launch_payload matching the Lambda Cloud API launch schema.".to_string(),
                    bootstrap_command: Some(bootstrap_command.clone()),
                    provider_template: build_gpu_cloud_template(runner, &bootstrap_command),
                    provider_job_id: None,
                    provider_job_metadata: serde_json::json!({
                        "provider": "lambda",
                        "status": "manual_launch_required",
                    }),
                    auto_launched: false,
                    requires_operator_action: true,
                })
            }
        }
        ExperimentRunnerBackend::Ssh => {
            let host = ssh_host(runner)?;
            let remote_cmd = format!(
                "mkdir -p ~/.thinclaw-experiments && nohup {} > ~/.thinclaw-experiments/{}.log 2>&1 < /dev/null & echo $! > ~/.thinclaw-experiments/{}.pid",
                bootstrap_command,
                auth.lease_id.simple(),
                auth.lease_id.simple()
            );
            let mut args = ssh_args(runner);
            args.push(host.clone());
            args.push(remote_cmd);
            run_local_cli_command("ssh", args, true)
                .await
                .map_err(|err| format!("ssh launch failed: {err}"))?;
            Ok(RunnerLaunchOutcome {
                message: "SSH runner launched.".to_string(),
                bootstrap_command: None,
                provider_template: None,
                provider_job_id: Some(auth.lease_id.simple().to_string()),
                provider_job_metadata: sanitize_provider_job_metadata(
                    ExperimentRunnerBackend::Ssh,
                    &serde_json::json!({
                        "provider": "ssh",
                        "host": host,
                        "pid_file": format!("~/.thinclaw-experiments/{}.pid", auth.lease_id.simple()),
                    }),
                ),
                auto_launched: true,
                requires_operator_action: false,
            })
        }
        ExperimentRunnerBackend::Slurm => {
            let host = ssh_login_host(runner)?;
            let job_name = remote_job_name(auth);
            let sbatch_args = backend_string(runner, "sbatch_args").unwrap_or_default();
            let remote_cmd = format!(
                "cat <<'EOF' | sbatch --job-name={} {}\n#!/bin/bash\nset -euo pipefail\n{}\nEOF",
                job_name, sbatch_args, bootstrap_command
            );
            let mut args = ssh_args(runner);
            args.push(host.clone());
            args.push(remote_cmd);
            let output = run_local_cli_command("ssh", args, true)
                .await
                .map_err(|err| format!("slurm launch failed: {err}"))?;
            let provider_job_id = parse_slurm_job_id(&output.stdout).ok_or_else(|| {
                "Slurm submission succeeded but did not return a numeric job id.".to_string()
            })?;
            Ok(RunnerLaunchOutcome {
                message: format!("Slurm job {provider_job_id} submitted."),
                bootstrap_command: None,
                provider_template: None,
                provider_job_id: Some(provider_job_id),
                provider_job_metadata: sanitize_provider_job_metadata(
                    ExperimentRunnerBackend::Slurm,
                    &serde_json::json!({
                        "provider": "slurm",
                        "login_host": host,
                    }),
                ),
                auto_launched: true,
                requires_operator_action: false,
            })
        }
        ExperimentRunnerBackend::Kubernetes => {
            let namespace = backend_string(runner, "namespace")
                .ok_or_else(|| "kubernetes backend requires namespace".to_string())?;
            let image = runner
                .image_or_runtime
                .clone()
                .or_else(|| backend_string(runner, "image"))
                .ok_or_else(|| "kubernetes backend requires image".to_string())?;
            let job_name = remote_job_name(auth);
            let manifest = kubernetes_job_manifest(
                &job_name,
                &namespace,
                &image,
                &bootstrap_command,
                env_pairs(runner),
                &runner.gpu_requirements,
            );
            run_command_with_stdin("kubectl", &["apply", "-f", "-"], &manifest)
                .await
                .map_err(|err| format!("failed to apply kubernetes job: {err}"))?;
            Ok(RunnerLaunchOutcome {
                message: format!("Kubernetes job {job_name} applied."),
                bootstrap_command: None,
                provider_template: None,
                provider_job_id: Some(job_name.clone()),
                provider_job_metadata: sanitize_provider_job_metadata(
                    ExperimentRunnerBackend::Kubernetes,
                    &serde_json::json!({
                        "provider": "kubernetes",
                        "job_name": job_name,
                        "namespace": namespace,
                    }),
                ),
                auto_launched: true,
                requires_operator_action: false,
            })
        }
        ExperimentRunnerBackend::LocalDocker | ExperimentRunnerBackend::AgentEnv => {
            Ok(RunnerLaunchOutcome {
                message: if runner.backend == ExperimentRunnerBackend::AgentEnv {
                    "AgentEnv runner executes inside the local controller.".to_string()
                } else {
                    "Local runner executes inside the controller-managed Docker sandbox."
                        .to_string()
                },
                bootstrap_command: None,
                provider_template: None,
                provider_job_id: None,
                provider_job_metadata: serde_json::json!({}),
                auto_launched: false,
                requires_operator_action: false,
            })
        }
    }
}

pub async fn revoke_remote_launch(
    runner: &ExperimentRunnerProfile,
    auth: &ExperimentLeaseAuthentication,
    provider_job_id: Option<&str>,
    provider_job_metadata: &serde_json::Value,
    action: RemoteLaunchAction,
    provider_api_key: Option<&str>,
) -> Result<Option<String>, String> {
    match runner.backend {
        ExperimentRunnerBackend::Ssh => {
            let host = ssh_host(runner)?;
            let remote_cmd = format!(
                "if [ -f ~/.thinclaw-experiments/{}.pid ]; then kill $(cat ~/.thinclaw-experiments/{}.pid) >/dev/null 2>&1 || true; rm -f ~/.thinclaw-experiments/{}.pid; fi",
                auth.lease_id.simple(),
                auth.lease_id.simple(),
                auth.lease_id.simple()
            );
            let mut args = ssh_args(runner);
            args.push(host);
            args.push(remote_cmd);
            let _ = run_local_cli_command("ssh", args, true).await;
            Ok(Some(
                "Best-effort SSH runner termination requested.".to_string(),
            ))
        }
        ExperimentRunnerBackend::Slurm => {
            let host = ssh_login_host(runner)?;
            let mut args = ssh_args(runner);
            args.push(host);
            args.push(format!("scancel --name {}", remote_job_name(auth)));
            let _ = run_local_cli_command("ssh", args, true).await;
            Ok(Some(
                "Best-effort Slurm job cancellation requested.".to_string(),
            ))
        }
        ExperimentRunnerBackend::Kubernetes => {
            let namespace = backend_string(runner, "namespace")
                .ok_or_else(|| "kubernetes backend requires namespace".to_string())?;
            let output = run_local_cli_command(
                "kubectl",
                vec![
                    "delete".to_string(),
                    "job".to_string(),
                    remote_job_name(auth),
                    "-n".to_string(),
                    namespace,
                ],
                true,
            )
            .await
            .map_err(|err| format!("failed to delete kubernetes job: {err}"))?;
            Ok(Some(output.stdout.trim().to_string()))
        }
        ExperimentRunnerBackend::Runpod => {
            let api_key = provider_api_key.ok_or_else(|| {
                "RunPod revoke requires a connected research_runpod_api_key secret.".to_string()
            })?;
            let pod_id = provider_job_id
                .map(ToOwned::to_owned)
                .or_else(|| {
                    provider_job_metadata
                        .get("pod_id")
                        .and_then(|v| v.as_str())
                        .map(ToOwned::to_owned)
                })
                .or_else(|| {
                    provider_job_metadata
                        .pointer("/pod/id")
                        .and_then(|v| v.as_str())
                        .map(ToOwned::to_owned)
                })
                .ok_or_else(|| "RunPod revoke requires a recorded provider pod ID.".to_string())?;
            revoke_runpod_pod(api_key, &pod_id, action).await.map(Some)
        }
        ExperimentRunnerBackend::Vast => {
            let api_key = provider_api_key.ok_or_else(|| {
                "Vast revoke requires a connected research_vast_api_key secret.".to_string()
            })?;
            let instance_id = provider_job_id
                .map(ToOwned::to_owned)
                .or_else(|| {
                    provider_job_metadata
                        .get("instance_id")
                        .and_then(value_to_string)
                })
                .or_else(|| {
                    provider_job_metadata
                        .pointer("/instance/id")
                        .and_then(value_to_string)
                })
                .ok_or_else(|| {
                    "Vast revoke requires a recorded provider instance ID.".to_string()
                })?;
            revoke_vast_instance(api_key, &instance_id, action)
                .await
                .map(Some)
        }
        ExperimentRunnerBackend::Lambda => {
            let api_key = provider_api_key.ok_or_else(|| {
                "Lambda revoke requires a connected research_lambda_api_key secret.".to_string()
            })?;
            let instance_id = provider_job_id
                .map(ToOwned::to_owned)
                .or_else(|| {
                    provider_job_metadata
                        .get("instance_ids")
                        .and_then(|value| value.as_array())
                        .and_then(|items| items.first())
                        .and_then(value_to_string)
                })
                .or_else(|| {
                    provider_job_metadata
                        .get("instance_id")
                        .and_then(value_to_string)
                })
                .ok_or_else(|| {
                    "Lambda revoke requires a recorded provider instance ID.".to_string()
                })?;
            revoke_lambda_instance(runner, api_key, &instance_id, action)
                .await
                .map(Some)
        }
        _ => Ok(None),
    }
}

pub async fn validate_gpu_cloud_credentials(
    backend: ExperimentRunnerBackend,
    api_key: &str,
) -> Result<String, String> {
    match backend {
        ExperimentRunnerBackend::Runpod => validate_runpod_credentials(api_key).await,
        ExperimentRunnerBackend::Vast => validate_vast_credentials(api_key).await,
        ExperimentRunnerBackend::Lambda => validate_lambda_credentials(api_key).await,
        _ => Err("Backend is not a GPU cloud provider.".to_string()),
    }
}

pub async fn command_exists(binary: &str) -> bool {
    thinclaw_platform::find_executable_in_path(binary).is_some()
}

fn backend_string(runner: &ExperimentRunnerProfile, key: &str) -> Option<String> {
    runner
        .backend_config
        .get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn backend_u64(runner: &ExperimentRunnerProfile, key: &str) -> Option<u64> {
    runner.backend_config.get(key).and_then(value_to_u64)
}

fn backend_bool(runner: &ExperimentRunnerProfile, key: &str) -> Option<bool> {
    runner
        .backend_config
        .get(key)
        .and_then(|value| match value {
            serde_json::Value::Bool(flag) => Some(*flag),
            serde_json::Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            },
            _ => None,
        })
}

fn backend_array_strings(runner: &ExperimentRunnerProfile, key: &str) -> Vec<String> {
    runner
        .backend_config
        .get(key)
        .map(json_string_array)
        .unwrap_or_default()
}

fn sh_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn ssh_host(runner: &ExperimentRunnerProfile) -> Result<String, String> {
    let host = backend_string(runner, "host")
        .ok_or_else(|| "ssh backend requires backend_config.host".to_string())?;
    Ok(match backend_string(runner, "user") {
        Some(user) => format!("{user}@{host}"),
        None => host,
    })
}

fn ssh_login_host(runner: &ExperimentRunnerProfile) -> Result<String, String> {
    let host = backend_string(runner, "login_host")
        .ok_or_else(|| "slurm backend requires backend_config.login_host".to_string())?;
    Ok(match backend_string(runner, "user") {
        Some(user) => format!("{user}@{host}"),
        None => host,
    })
}

fn cli_workdir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn ssh_args(runner: &ExperimentRunnerProfile) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(port) = backend_string(runner, "port") {
        args.push("-p".to_string());
        args.push(port);
    }
    if let Some(identity) = backend_string(runner, "identity_file") {
        args.push("-i".to_string());
        args.push(identity);
    }
    args
}

async fn run_local_cli_command(
    binary: &str,
    args: Vec<String>,
    allow_network: bool,
) -> Result<ExecutionResult, String> {
    let output = local_cli_execution_backend()
        .run_script(ScriptExecutionRequest {
            program: binary.to_string(),
            args,
            workdir: cli_workdir(),
            timeout: REMOTE_ADAPTER_COMMAND_TIMEOUT,
            extra_env: HashMap::new(),
            allow_network,
        })
        .await
        .map_err(|err| err.to_string())?;
    if output.exit_code != 0 {
        // Arguments can contain a live lease token, and subprocess output can
        // echo those arguments. Keep the durable/API-visible failure generic.
        return Err(format!("{binary} exited with status {}", output.exit_code));
    }
    Ok(output)
}

fn local_cli_execution_backend() -> Arc<dyn ExecutionBackend> {
    LocalHostExecutionBackend::shared()
}

fn remote_job_name(auth: &ExperimentLeaseAuthentication) -> String {
    format!("thinclaw-exp-{}", &auth.lease_id.simple().to_string()[..12])
}

fn env_pairs(runner: &ExperimentRunnerProfile) -> BTreeMap<String, String> {
    runner
        .env_grants
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn gpu_count(runner: &ExperimentRunnerProfile) -> u64 {
    runner
        .gpu_requirements
        .get("gpu_count")
        .or_else(|| runner.gpu_requirements.get("count"))
        .and_then(value_to_u64)
        .unwrap_or(1)
}

fn gpu_type_hint(runner: &ExperimentRunnerProfile) -> Option<String> {
    runner
        .gpu_requirements
        .get("gpu_type")
        .or_else(|| runner.gpu_requirements.get("gpu_type_id"))
        .or_else(|| runner.gpu_requirements.get("sku"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn min_vram_gb(runner: &ExperimentRunnerProfile) -> Option<u64> {
    runner
        .gpu_requirements
        .get("min_vram_gb")
        .and_then(value_to_u64)
}

fn value_to_u64(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(number) => number.as_u64(),
        serde_json::Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn value_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.trim().to_string()).filter(|s| !s.is_empty()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn looks_like_lease_token(text: &str) -> bool {
    text.as_bytes().windows(49).any(|window| {
        window.starts_with(b"exp_")
            && window[4..16].iter().all(u8::is_ascii_hexdigit)
            && window[16] == b'_'
            && window[17..].iter().all(u8::is_ascii_hexdigit)
    })
}

fn safe_metadata_value(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Null => Some(serde_json::Value::Null),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => Some(value.clone()),
        serde_json::Value::String(text)
            if text.len() <= MAX_METADATA_STRING_BYTES
                && !text.chars().any(char::is_control)
                && !looks_like_lease_token(text) =>
        {
            Some(value.clone())
        }
        serde_json::Value::Array(items) if items.len() <= 32 => Some(
            items
                .iter()
                .filter_map(safe_metadata_value)
                .filter(|item| !item.is_object())
                .collect::<Vec<_>>()
                .into(),
        ),
        _ => None,
    }
}

fn select_metadata_fields(source: &serde_json::Value, fields: &[&str]) -> serde_json::Value {
    let mut selected = serde_json::Map::new();
    for field in fields {
        if let Some(value) = source.get(*field).and_then(safe_metadata_value) {
            selected.insert((*field).to_string(), value);
        }
    }
    serde_json::Value::Object(selected)
}

fn safe_cost_estimate(source: &serde_json::Value) -> Option<serde_json::Value> {
    let value = select_metadata_fields(
        source.get("cost_estimate")?,
        &[
            "estimated",
            "usd",
            "hourly_rate_usd",
            "native_hourly_rate",
            "native_currency",
            "normalization",
            "source",
        ],
    );
    value
        .as_object()
        .is_some_and(|map| !map.is_empty())
        .then_some(value)
}

fn provider_name(backend: ExperimentRunnerBackend) -> &'static str {
    match backend {
        ExperimentRunnerBackend::Runpod => "runpod",
        ExperimentRunnerBackend::Vast => "vast",
        ExperimentRunnerBackend::Lambda => "lambda",
        ExperimentRunnerBackend::Ssh => "ssh",
        ExperimentRunnerBackend::Slurm => "slurm",
        ExperimentRunnerBackend::Kubernetes => "kubernetes",
        ExperimentRunnerBackend::GenericRemoteRunner => "generic_remote_runner",
        ExperimentRunnerBackend::LocalDocker => "local_docker",
        ExperimentRunnerBackend::AgentEnv => "agent_env",
    }
}

/// Reduce durable/API-visible provider metadata to the fields required for
/// lifecycle control, operator status, and cost attribution. Launch commands,
/// environment values, arbitrary provider echoes, and legacy lease tokens are
/// deliberately excluded.
pub fn sanitize_provider_job_metadata(
    backend: ExperimentRunnerBackend,
    metadata: &serde_json::Value,
) -> serde_json::Value {
    let mut result = serde_json::Map::new();
    result.insert(
        "provider".to_string(),
        serde_json::Value::String(provider_name(backend).to_string()),
    );
    if let Some(status) = metadata.get("status").and_then(safe_metadata_value) {
        result.insert("status".to_string(), status);
    }

    match backend {
        ExperimentRunnerBackend::Runpod => {
            for field in ["pod_id"] {
                if let Some(value) = metadata.get(field).and_then(safe_metadata_value) {
                    result.insert(field.to_string(), value);
                }
            }
            let request = select_metadata_fields(
                metadata
                    .get("launch_request")
                    .unwrap_or(&serde_json::Value::Null),
                &[
                    "name",
                    "imageName",
                    "computeType",
                    "gpuCount",
                    "cloudType",
                    "gpuTypeIds",
                    "dataCenterIds",
                    "ports",
                    "containerDiskInGb",
                    "volumeInGb",
                    "templateId",
                    "interruptible",
                    "supportPublicIp",
                    "costPerHr",
                ],
            );
            if request.as_object().is_some_and(|map| !map.is_empty()) {
                result.insert("launch_request".to_string(), request);
            }
            let pod = select_metadata_fields(
                metadata.get("pod").unwrap_or(&serde_json::Value::Null),
                &[
                    "id",
                    "name",
                    "desiredStatus",
                    "status",
                    "adjustedCostPerHr",
                    "costPerHr",
                    "gpuCount",
                    "gpuTypeId",
                    "dataCenterId",
                ],
            );
            if pod.as_object().is_some_and(|map| !map.is_empty()) {
                result.insert("pod".to_string(), pod);
            }
        }
        ExperimentRunnerBackend::Vast => {
            for field in ["instance_id", "ask_id"] {
                if let Some(value) = metadata.get(field).and_then(safe_metadata_value) {
                    result.insert(field.to_string(), value);
                }
            }
            let mut offer = select_metadata_fields(
                metadata
                    .get("selected_offer")
                    .unwrap_or(&serde_json::Value::Null),
                &[
                    "id",
                    "source",
                    "dph_total",
                    "totalHour",
                    "gpu_name",
                    "gpu_name_array",
                    "num_gpus",
                    "gpu_ram",
                    "geolocation",
                    "reliability",
                ],
            );
            if let Some(search) = metadata
                .pointer("/selected_offer/search")
                .map(|value| select_metadata_fields(value, &["totalHour"]))
                .filter(|value| value.as_object().is_some_and(|map| !map.is_empty()))
                && let Some(map) = offer.as_object_mut()
            {
                map.insert("search".to_string(), search);
            }
            if offer.as_object().is_some_and(|map| !map.is_empty()) {
                result.insert("selected_offer".to_string(), offer);
            }
            let mut instance = select_metadata_fields(
                metadata.get("instance").unwrap_or(&serde_json::Value::Null),
                &["id", "status", "state", "dph_total", "totalHour"],
            );
            if let Some(search) = metadata
                .pointer("/instance/search")
                .map(|value| select_metadata_fields(value, &["totalHour"]))
                .filter(|value| value.as_object().is_some_and(|map| !map.is_empty()))
                && let Some(map) = instance.as_object_mut()
            {
                map.insert("search".to_string(), search);
            }
            if instance.as_object().is_some_and(|map| !map.is_empty()) {
                result.insert("instance".to_string(), instance);
            }
        }
        ExperimentRunnerBackend::Lambda => {
            for field in ["instance_id", "instance_ids"] {
                if let Some(value) = metadata.get(field).and_then(safe_metadata_value) {
                    result.insert(field.to_string(), value);
                }
            }
            let request = select_metadata_fields(
                metadata
                    .get("launch_request")
                    .unwrap_or(&serde_json::Value::Null),
                &[
                    "instance_type_name",
                    "region_name",
                    "quantity",
                    "ssh_key_names",
                    "file_system_names",
                    "hourly_cost_usd",
                    "usd_per_hour",
                    "price_usd_per_hour",
                    "price_cents_per_hour",
                ],
            );
            if request.as_object().is_some_and(|map| !map.is_empty()) {
                result.insert("launch_request".to_string(), request);
            }
            let instance = select_metadata_fields(
                metadata.get("instance").unwrap_or(&serde_json::Value::Null),
                &[
                    "id",
                    "instance_id",
                    "instance_type_name",
                    "region_name",
                    "status",
                    "state",
                    "hostname",
                    "name",
                    "public_ip",
                    "ip",
                    "ip_address",
                    "private_ip",
                    "private_ipv4",
                    "ssh_key_names",
                    "file_system_names",
                    "jupyter_url",
                    "ide_url",
                    "hourly_cost_usd",
                    "usd_per_hour",
                    "price_usd_per_hour",
                    "price_cents_per_hour",
                ],
            );
            if instance.as_object().is_some_and(|map| !map.is_empty()) {
                result.insert("instance".to_string(), instance);
            }
        }
        ExperimentRunnerBackend::Ssh => {
            for field in ["host", "pid_file"] {
                if let Some(value) = metadata.get(field).and_then(safe_metadata_value) {
                    result.insert(field.to_string(), value);
                }
            }
        }
        ExperimentRunnerBackend::Slurm => {
            if let Some(value) = metadata.get("login_host").and_then(safe_metadata_value) {
                result.insert("login_host".to_string(), value);
            }
        }
        ExperimentRunnerBackend::Kubernetes => {
            for field in ["job_name", "namespace"] {
                if let Some(value) = metadata.get(field).and_then(safe_metadata_value) {
                    result.insert(field.to_string(), value);
                }
            }
        }
        ExperimentRunnerBackend::GenericRemoteRunner
        | ExperimentRunnerBackend::LocalDocker
        | ExperimentRunnerBackend::AgentEnv => {}
    }

    if let Some(cost_estimate) = safe_cost_estimate(metadata) {
        result.insert("cost_estimate".to_string(), cost_estimate);
    }
    serde_json::Value::Object(result)
}

fn validate_provider_id(label: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_PROVIDER_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(format!("{label} returned an invalid provider job id."));
    }
    Ok(value.to_string())
}

fn validate_numeric_provider_id(label: &str, value: &str) -> Result<String, String> {
    value
        .trim()
        .parse::<u64>()
        .map(|value| value.to_string())
        .map_err(|_| format!("{label} returned an invalid numeric provider job id."))
}

fn provider_url(base: &str, segments: &[&str]) -> Result<Url, String> {
    let mut url = Url::parse(base).map_err(|_| "invalid built-in provider API URL".to_string())?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| "invalid built-in provider API URL".to_string())?;
        for segment in segments {
            path.push(segment);
        }
    }
    Ok(url)
}

fn json_string_array(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        serde_json::Value::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn provider_env_map(
    runner: &ExperimentRunnerProfile,
) -> serde_json::Map<String, serde_json::Value> {
    env_pairs(runner)
        .into_iter()
        .map(|(key, value)| (key, serde_json::Value::String(value)))
        .collect()
}

fn replace_placeholders_in_json(
    value: &serde_json::Value,
    replacements: &HashMap<&str, String>,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => {
            let replaced = replacements
                .iter()
                .fold(text.clone(), |acc, (needle, value)| {
                    acc.replace(needle, value)
                });
            serde_json::Value::String(replaced)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|item| replace_placeholders_in_json(item, replacements))
                .collect(),
        ),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        replace_placeholders_in_json(value, replacements),
                    )
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn lambda_launch_payload(
    runner: &ExperimentRunnerProfile,
    bootstrap_command: &str,
    auth: &ExperimentLeaseAuthentication,
) -> Option<serde_json::Value> {
    let template = runner.backend_config.get("launch_payload")?;
    if !template.is_object() {
        return None;
    }
    let mut replacements = HashMap::new();
    replacements.insert("__THINCLAW_BOOTSTRAP__", bootstrap_command.to_string());
    replacements.insert("{{THINCLAW_BOOTSTRAP}}", bootstrap_command.to_string());
    replacements.insert("__THINCLAW_NAME__", short_launch_name("thinclaw-exp", auth));
    replacements.insert("{{THINCLAW_NAME}}", short_launch_name("thinclaw-exp", auth));
    let image = runner
        .image_or_runtime
        .clone()
        .or_else(|| backend_string(runner, "image"))
        .unwrap_or_default();
    replacements.insert("__THINCLAW_IMAGE__", image.clone());
    replacements.insert("{{THINCLAW_IMAGE}}", image);
    Some(replace_placeholders_in_json(template, &replacements))
}

fn lambda_terminate_payload(
    runner: &ExperimentRunnerProfile,
    instance_id: &str,
) -> serde_json::Value {
    let mut replacements = HashMap::new();
    replacements.insert("__THINCLAW_PROVIDER_JOB_ID__", instance_id.to_string());
    replacements.insert("{{THINCLAW_PROVIDER_JOB_ID}}", instance_id.to_string());
    if let Some(template) = runner.backend_config.get("terminate_payload")
        && template.is_object()
    {
        return replace_placeholders_in_json(template, &replacements);
    }
    serde_json::json!({
        "instance_ids": [instance_id],
    })
}

fn vast_env_flags(runner: &ExperimentRunnerProfile) -> String {
    env_pairs(runner)
        .into_iter()
        .map(|(key, value)| format!("-e {}={}", key, sh_single_quote(&value)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn short_launch_name(prefix: &str, auth: &ExperimentLeaseAuthentication) -> String {
    format!("{}-{}", prefix, &auth.lease_id.simple().to_string()[..12])
}

fn parse_slurm_job_id(text: &str) -> Option<String> {
    text.split_whitespace()
        .rev()
        .find(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| validate_numeric_provider_id("Slurm", value).ok())
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("ThinClaw/{}", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(PROVIDER_CONNECT_TIMEOUT)
        .timeout(PROVIDER_REQUEST_TIMEOUT)
        .build()
        .map_err(|err| format!("failed to build HTTP client: {err}"))
}

async fn response_error(context: &str, response: reqwest::Response) -> String {
    let status = response.status();
    // Provider errors may echo submitted cloud-init, environment values, or the
    // lease bootstrap command. Never reflect a remote body into durable campaign
    // summaries or operator-visible errors.
    format!("{context}: HTTP {status}")
}

async fn validate_runpod_credentials(api_key: &str) -> Result<String, String> {
    let client = http_client()?;
    let response = client
        .get(format!("{RUNPOD_API_BASE}/pods"))
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|err| format!("RunPod validation request failed: {err}"))?;
    match response.status() {
        StatusCode::OK => {
            Ok("RunPod credentials validated against the official Pods API.".to_string())
        }
        StatusCode::UNAUTHORIZED => {
            Err("RunPod credentials were rejected by the Pods API.".to_string())
        }
        _ => Err(response_error("RunPod validation failed", response).await),
    }
}

async fn validate_vast_credentials(api_key: &str) -> Result<String, String> {
    let client = http_client()?;
    let response = client
        .get(format!("{VAST_API_BASE}/api/v0/users/current/"))
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|err| format!("Vast.ai validation request failed: {err}"))?;
    match response.status() {
        StatusCode::OK => {
            Ok("Vast.ai credentials validated against the official user API.".to_string())
        }
        StatusCode::UNAUTHORIZED => {
            Err("Vast.ai credentials were rejected by the API.".to_string())
        }
        _ => Err(response_error("Vast.ai validation failed", response).await),
    }
}

async fn validate_lambda_credentials(api_key: &str) -> Result<String, String> {
    let client = http_client()?;
    let response = client
        .get(format!("{LAMBDA_API_BASE}/instance-types"))
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|err| format!("Lambda validation request failed: {err}"))?;
    match response.status() {
        StatusCode::OK => {
            Ok("Lambda credentials validated against the instance-types API.".to_string())
        }
        StatusCode::UNAUTHORIZED => {
            Err("Lambda credentials were rejected by the Cloud API.".to_string())
        }
        _ => Err(response_error("Lambda validation failed", response).await),
    }
}

fn lambda_response_instance_id(value: &serde_json::Value) -> Option<String> {
    value
        .get("instance_id")
        .and_then(value_to_string)
        .or_else(|| value.get("id").and_then(value_to_string))
        .or_else(|| {
            value
                .get("instance_ids")
                .and_then(|items| items.as_array())
                .and_then(|items| items.first())
                .and_then(value_to_string)
        })
        .or_else(|| value.get("data").and_then(lambda_response_instance_id))
}

fn lambda_response_instance_metadata(value: &serde_json::Value) -> serde_json::Value {
    if let Some(instance) = value.get("instance")
        && instance.is_object()
    {
        return instance.clone();
    }
    if let Some(items) = value.get("instances").and_then(|entry| entry.as_array())
        && let Some(instance) = items.iter().find(|entry| entry.is_object())
    {
        return instance.clone();
    }
    if let Some(data) = value.get("data") {
        if let Some(instance) = data.get("instance")
            && instance.is_object()
        {
            return instance.clone();
        }
        if let Some(items) = data.get("instances").and_then(|entry| entry.as_array())
            && let Some(instance) = items.iter().find(|entry| entry.is_object())
        {
            return instance.clone();
        }
        if data.is_object() {
            return data.clone();
        }
    }
    value.clone()
}

async fn launch_lambda_instance(
    runner: &ExperimentRunnerProfile,
    auth: &ExperimentLeaseAuthentication,
    bootstrap_command: &str,
    api_key: &str,
) -> Result<RunnerLaunchOutcome, String> {
    let payload = lambda_launch_payload(runner, bootstrap_command, auth).ok_or_else(|| {
        "Lambda launch requires backend_config.launch_payload with the official Lambda Cloud API request body.".to_string()
    })?;
    let client = http_client()?;
    let response = client
        .post(format!("{LAMBDA_API_BASE}/instances/launch"))
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|err| format!("Lambda launch request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(response_error("Lambda launch failed", response).await);
    }
    let body: serde_json::Value =
        crate::http_response::bounded_json(response, MAX_PROVIDER_RESPONSE_BYTES)
            .await
            .map_err(|err| format!("failed to decode Lambda launch response: {err}"))?;
    let instance_id = validate_provider_id(
        "Lambda launch",
        &lambda_response_instance_id(&body).ok_or_else(|| {
            "Lambda launch succeeded but response did not include an instance id.".to_string()
        })?,
    )?;
    let provider_job_metadata = sanitize_provider_job_metadata(
        ExperimentRunnerBackend::Lambda,
        &serde_json::json!({
            "provider": "lambda",
            "instance_id": instance_id,
            "launch_request": payload,
            "instance": lambda_response_instance_metadata(&body),
        }),
    );
    Ok(RunnerLaunchOutcome {
        message: format!("Lambda instance {instance_id} launched."),
        bootstrap_command: None,
        provider_template: None,
        provider_job_id: Some(instance_id.clone()),
        provider_job_metadata,
        auto_launched: true,
        requires_operator_action: false,
    })
}

async fn revoke_lambda_instance(
    runner: &ExperimentRunnerProfile,
    api_key: &str,
    instance_id: &str,
    _action: RemoteLaunchAction,
) -> Result<String, String> {
    let instance_id = validate_provider_id("Lambda terminate", instance_id)?;
    let client = http_client()?;
    let payload = lambda_terminate_payload(runner, &instance_id);
    let response = client
        .post(format!("{LAMBDA_API_BASE}/instances/terminate"))
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|err| format!("Lambda terminate request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(response_error("Lambda terminate failed", response).await);
    }
    Ok(format!(
        "Lambda instance termination requested: {instance_id}"
    ))
}

async fn launch_runpod_pod(
    runner: &ExperimentRunnerProfile,
    auth: &ExperimentLeaseAuthentication,
    bootstrap_command: &str,
    api_key: &str,
) -> Result<RunnerLaunchOutcome, String> {
    let client = http_client()?;
    let image = runner
        .image_or_runtime
        .clone()
        .or_else(|| backend_string(runner, "image"))
        .ok_or_else(|| {
            "RunPod launch requires image_or_runtime or backend_config.image".to_string()
        })?;
    let mut payload = serde_json::Map::new();
    payload.insert(
        "name".to_string(),
        serde_json::json!(short_launch_name("thinclaw-exp", auth)),
    );
    payload.insert("imageName".to_string(), serde_json::json!(image));
    payload.insert("computeType".to_string(), serde_json::json!("GPU"));
    payload.insert("gpuCount".to_string(), serde_json::json!(gpu_count(runner)));
    payload.insert(
        "env".to_string(),
        serde_json::Value::Object(provider_env_map(runner)),
    );
    payload.insert(
        "dockerEntrypoint".to_string(),
        serde_json::json!(["sh", "-lc"]),
    );
    payload.insert(
        "dockerStartCmd".to_string(),
        serde_json::json!([bootstrap_command]),
    );
    if let Some(cloud_type) = backend_string(runner, "cloud_type") {
        payload.insert("cloudType".to_string(), serde_json::json!(cloud_type));
    }
    let gpu_type_ids = if backend_array_strings(runner, "gpu_type_ids").is_empty() {
        gpu_type_hint(runner).into_iter().collect::<Vec<_>>()
    } else {
        backend_array_strings(runner, "gpu_type_ids")
    };
    if !gpu_type_ids.is_empty() {
        payload.insert("gpuTypeIds".to_string(), serde_json::json!(gpu_type_ids));
    }
    let data_center_ids = backend_array_strings(runner, "data_center_ids");
    if !data_center_ids.is_empty() {
        payload.insert(
            "dataCenterIds".to_string(),
            serde_json::json!(data_center_ids),
        );
    }
    let ports = backend_array_strings(runner, "ports");
    if !ports.is_empty() {
        payload.insert("ports".to_string(), serde_json::json!(ports));
    }
    if let Some(container_disk_gb) =
        backend_u64(runner, "container_disk_gb").or_else(|| backend_u64(runner, "disk_gb"))
    {
        payload.insert(
            "containerDiskInGb".to_string(),
            serde_json::json!(container_disk_gb),
        );
    }
    if let Some(volume_gb) = backend_u64(runner, "volume_gb") {
        payload.insert("volumeInGb".to_string(), serde_json::json!(volume_gb));
    }
    if let Some(template_id) = backend_string(runner, "template_id") {
        payload.insert("templateId".to_string(), serde_json::json!(template_id));
    }
    if let Some(interruptible) = backend_bool(runner, "interruptible") {
        payload.insert(
            "interruptible".to_string(),
            serde_json::json!(interruptible),
        );
    }
    if let Some(public_ip) = backend_bool(runner, "support_public_ip") {
        payload.insert("supportPublicIp".to_string(), serde_json::json!(public_ip));
    }

    let response = client
        .post(format!("{RUNPOD_API_BASE}/pods"))
        .bearer_auth(api_key)
        .json(&serde_json::Value::Object(payload.clone()))
        .send()
        .await
        .map_err(|err| format!("RunPod launch request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(response_error("RunPod launch failed", response).await);
    }
    let pod: serde_json::Value =
        crate::http_response::bounded_json(response, MAX_PROVIDER_RESPONSE_BYTES)
            .await
            .map_err(|err| format!("failed to decode RunPod launch response: {err}"))?;
    let pod_id = validate_provider_id(
        "RunPod launch",
        pod.get("id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                "RunPod launch succeeded but response did not include a pod id.".to_string()
            })?,
    )?;
    let provider_job_metadata = sanitize_provider_job_metadata(
        ExperimentRunnerBackend::Runpod,
        &serde_json::json!({
            "provider": "runpod",
            "pod_id": pod_id,
            "launch_request": payload,
            "pod": pod,
        }),
    );
    Ok(RunnerLaunchOutcome {
        message: format!("RunPod pod {pod_id} launched."),
        bootstrap_command: None,
        provider_template: None,
        provider_job_id: Some(pod_id.clone()),
        provider_job_metadata,
        auto_launched: true,
        requires_operator_action: false,
    })
}

async fn revoke_runpod_pod(
    api_key: &str,
    pod_id: &str,
    action: RemoteLaunchAction,
) -> Result<String, String> {
    let pod_id = validate_provider_id("RunPod revoke", pod_id)?;
    let client = http_client()?;
    let (request, label) = match action {
        RemoteLaunchAction::Cancel => (
            client
                .delete(provider_url(RUNPOD_API_BASE, &["pods", &pod_id])?)
                .bearer_auth(api_key),
            "RunPod pod deleted",
        ),
        RemoteLaunchAction::Pause | RemoteLaunchAction::Reissue => (
            client
                .post(provider_url(RUNPOD_API_BASE, &["pods", &pod_id, "stop"])?)
                .bearer_auth(api_key),
            "RunPod pod stopped",
        ),
    };
    let response = request
        .send()
        .await
        .map_err(|err| format!("RunPod revoke request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(response_error("RunPod revoke failed", response).await);
    }
    Ok(format!("{label}: {pod_id}"))
}

fn normalized_vast_gpu_name(runner: &ExperimentRunnerProfile) -> Option<String> {
    backend_string(runner, "gpu_name").or_else(|| {
        gpu_type_hint(runner).map(|value| {
            value
                .replace("NVIDIA ", "")
                .replace("AMD ", "")
                .replace("GeForce ", "")
                .replace("  ", " ")
                .replace([' ', '-'], "_")
        })
    })
}

async fn select_vast_offer(
    runner: &ExperimentRunnerProfile,
    api_key: &str,
) -> Result<(u64, serde_json::Value), String> {
    let client = http_client()?;
    let mut body = serde_json::Map::new();
    body.insert("limit".to_string(), serde_json::json!(3));
    body.insert(
        "type".to_string(),
        serde_json::json!(
            backend_string(runner, "offer_type").unwrap_or_else(|| "ondemand".to_string())
        ),
    );
    body.insert("verified".to_string(), serde_json::json!({ "eq": true }));
    body.insert("rentable".to_string(), serde_json::json!({ "eq": true }));
    body.insert("rented".to_string(), serde_json::json!({ "eq": false }));
    body.insert(
        "order".to_string(),
        serde_json::json!([["dph_total", "asc"]]),
    );
    body.insert(
        "num_gpus".to_string(),
        serde_json::json!({ "gte": gpu_count(runner) }),
    );
    if let Some(min_vram_gb) = min_vram_gb(runner) {
        body.insert(
            "gpu_ram".to_string(),
            serde_json::json!({ "gte": min_vram_gb * 1024 }),
        );
    }
    if let Some(gpu_name) = normalized_vast_gpu_name(runner) {
        body.insert(
            "gpu_name".to_string(),
            serde_json::json!({ "in": [gpu_name] }),
        );
    }
    let response = client
        .post(format!("{VAST_API_BASE}/api/v0/bundles/"))
        .bearer_auth(api_key)
        .json(&serde_json::Value::Object(body.clone()))
        .send()
        .await
        .map_err(|err| format!("Vast.ai offer search failed: {err}"))?;
    if !response.status().is_success() {
        return Err(response_error("Vast.ai offer search failed", response).await);
    }
    let result: serde_json::Value =
        crate::http_response::bounded_json(response, MAX_PROVIDER_RESPONSE_BYTES)
            .await
            .map_err(|err| format!("failed to decode Vast.ai offer search response: {err}"))?;
    let offer = result
        .get("offers")
        .and_then(|value| value.as_array())
        .and_then(|offers| offers.first())
        .cloned()
        .ok_or_else(|| {
            "Vast.ai search returned no matching offers for the configured GPU requirements."
                .to_string()
        })?;
    let ask_id = offer
        .get("id")
        .and_then(value_to_u64)
        .ok_or_else(|| "Vast.ai offer search response did not include an offer id.".to_string())?;
    Ok((ask_id, offer))
}

async fn launch_vast_instance(
    runner: &ExperimentRunnerProfile,
    auth: &ExperimentLeaseAuthentication,
    bootstrap_command: &str,
    api_key: &str,
) -> Result<RunnerLaunchOutcome, String> {
    let client = http_client()?;
    let image = runner
        .image_or_runtime
        .clone()
        .or_else(|| backend_string(runner, "image"))
        .ok_or_else(|| {
            "Vast.ai launch requires image_or_runtime or backend_config.image".to_string()
        })?;
    let explicit_ask_id = backend_u64(runner, "offer_id").or_else(|| backend_u64(runner, "ask_id"));
    let (ask_id, selected_offer) = match explicit_ask_id {
        Some(id) => (
            id,
            serde_json::json!({ "id": id, "source": "backend_config" }),
        ),
        None => select_vast_offer(runner, api_key).await?,
    };
    let mut payload = serde_json::Map::new();
    payload.insert("image".to_string(), serde_json::json!(image));
    payload.insert(
        "label".to_string(),
        serde_json::json!(short_launch_name("thinclaw-exp", auth)),
    );
    payload.insert("target_state".to_string(), serde_json::json!("running"));
    payload.insert(
        "disk".to_string(),
        serde_json::json!(backend_u64(runner, "disk_gb").unwrap_or(50)),
    );
    payload.insert(
        "runtype".to_string(),
        serde_json::json!(backend_string(runner, "runtype").unwrap_or_else(|| "ssh".to_string())),
    );
    payload.insert("onstart".to_string(), serde_json::json!(bootstrap_command));
    if let Some(template_hash_id) = backend_string(runner, "template_hash_id") {
        payload.insert(
            "template_hash_id".to_string(),
            serde_json::json!(template_hash_id),
        );
    }
    if let Some(cancel_unavail) = backend_bool(runner, "cancel_unavail") {
        payload.insert(
            "cancel_unavail".to_string(),
            serde_json::json!(cancel_unavail),
        );
    }
    let env = vast_env_flags(runner);
    if !env.is_empty() {
        payload.insert("env".to_string(), serde_json::json!(env));
    }
    let response = client
        .put(format!("{VAST_API_BASE}/api/v0/asks/{ask_id}/"))
        .bearer_auth(api_key)
        .json(&serde_json::Value::Object(payload.clone()))
        .send()
        .await
        .map_err(|err| format!("Vast.ai launch request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(response_error("Vast.ai launch failed", response).await);
    }
    let instance: serde_json::Value =
        crate::http_response::bounded_json(response, MAX_PROVIDER_RESPONSE_BYTES)
            .await
            .map_err(|err| format!("failed to decode Vast.ai launch response: {err}"))?;
    let instance_id = validate_numeric_provider_id(
        "Vast.ai launch",
        &instance
            .get("new_contract")
            .and_then(value_to_u64)
            .map(|value| value.to_string())
            .or_else(|| instance.get("instance_id").and_then(value_to_string))
            .ok_or_else(|| {
                "Vast.ai launch succeeded but response did not include an instance id.".to_string()
            })?,
    )?;
    let provider_job_metadata = sanitize_provider_job_metadata(
        ExperimentRunnerBackend::Vast,
        &serde_json::json!({
            "provider": "vast",
            "instance_id": instance_id,
            "ask_id": ask_id,
            "selected_offer": selected_offer,
            "instance": instance,
        }),
    );
    Ok(RunnerLaunchOutcome {
        message: format!("Vast.ai instance {instance_id} launched."),
        bootstrap_command: None,
        provider_template: None,
        provider_job_id: Some(instance_id.clone()),
        provider_job_metadata,
        auto_launched: true,
        requires_operator_action: false,
    })
}

async fn revoke_vast_instance(
    api_key: &str,
    instance_id: &str,
    action: RemoteLaunchAction,
) -> Result<String, String> {
    let instance_id = validate_numeric_provider_id("Vast.ai revoke", instance_id)?;
    let client = http_client()?;
    let response = match action {
        RemoteLaunchAction::Cancel => client
            .delete(provider_url(
                VAST_API_BASE,
                &["api", "v0", "instances", &instance_id, ""],
            )?)
            .bearer_auth(api_key)
            .send()
            .await
            .map_err(|err| format!("Vast.ai destroy request failed: {err}"))?,
        RemoteLaunchAction::Pause | RemoteLaunchAction::Reissue => client
            .put(provider_url(
                VAST_API_BASE,
                &["api", "v0", "instances", &instance_id, ""],
            )?)
            .bearer_auth(api_key)
            .json(&serde_json::json!({ "state": "stopped" }))
            .send()
            .await
            .map_err(|err| format!("Vast.ai stop request failed: {err}"))?,
    };
    if !response.status().is_success() {
        return Err(response_error("Vast.ai revoke failed", response).await);
    }
    Ok(match action {
        RemoteLaunchAction::Cancel => format!("Vast.ai instance destroyed: {instance_id}"),
        RemoteLaunchAction::Pause | RemoteLaunchAction::Reissue => {
            format!("Vast.ai instance stopped: {instance_id}")
        }
    })
}

fn kubernetes_job_manifest(
    job_name: &str,
    namespace: &str,
    image: &str,
    bootstrap_command: &str,
    env: BTreeMap<String, String>,
    gpu_requirements: &serde_json::Value,
) -> String {
    let mut env_lines = String::new();
    for (key, value) in env {
        env_lines.push_str(&format!(
            "        - name: {}\n          value: {}\n",
            serde_json::to_string(&key).unwrap_or_else(|_| "\"\"".to_string()),
            serde_json::to_string(&value).unwrap_or_else(|_| "\"\"".to_string())
        ));
    }
    let gpu_count = gpu_requirements
        .get("gpu_count")
        .and_then(|value| value.as_u64())
        .unwrap_or(1);
    format!(
        "apiVersion: batch/v1\nkind: Job\nmetadata:\n  name: {job_name}\n  namespace: {namespace}\nspec:\n  template:\n    spec:\n      restartPolicy: Never\n      containers:\n      - name: runner\n        image: {image}\n        command:\n        - sh\n        - -lc\n        - {command}\n        env:\n{env_lines}        resources:\n          limits:\n            nvidia.com/gpu: {gpu_count}\n",
        job_name = serde_json::to_string(job_name).unwrap_or_else(|_| "\"\"".to_string()),
        namespace = serde_json::to_string(namespace).unwrap_or_else(|_| "\"\"".to_string()),
        image = serde_json::to_string(image).unwrap_or_else(|_| "\"\"".to_string()),
        command = serde_json::to_string(bootstrap_command).unwrap_or_else(|_| "\"\"".to_string())
    )
}

async fn run_command_with_stdin(
    binary: &str,
    args: &[&str],
    stdin_payload: &str,
) -> Result<String, std::io::Error> {
    let temp_dir = tempfile::tempdir()?;
    let manifest_path = temp_dir.path().join("stdin-payload.txt");
    tokio::fs::write(&manifest_path, stdin_payload).await?;
    let mut rendered_args = Vec::with_capacity(args.len() + 1);
    for arg in args {
        if *arg == "-" {
            rendered_args.push(manifest_path.to_string_lossy().to_string());
        } else {
            rendered_args.push((*arg).to_string());
        }
    }
    let output = run_local_cli_command(binary, rendered_args, true)
        .await
        .map_err(std::io::Error::other)?;
    Ok(output.output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn runner_profile(backend: ExperimentRunnerBackend) -> ExperimentRunnerProfile {
        ExperimentRunnerProfile {
            id: Uuid::new_v4(),
            owner_user_id: "default".to_string(),
            name: "runner".to_string(),
            backend,
            backend_config: serde_json::json!({}),
            image_or_runtime: None,
            gpu_requirements: serde_json::json!({}),
            env_grants: serde_json::json!({}),
            secret_references: Vec::new(),
            cache_policy: serde_json::json!({}),
            status: crate::experiments::ExperimentRunnerStatus::Draft,
            readiness_class: ExperimentRunnerReadinessClass::ManualOnly,
            launch_eligible: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn generic_remote_runner_validates_as_manual_only() {
        let settings = Settings::default();
        let runner = runner_profile(ExperimentRunnerBackend::GenericRemoteRunner);
        let outcome = validate_runner_profile(&runner, &settings, None).await;
        assert!(outcome.valid);
        assert_eq!(
            outcome.readiness_class,
            ExperimentRunnerReadinessClass::ManualOnly
        );
        assert!(!outcome.launch_eligible);
    }

    #[tokio::test]
    async fn lambda_runner_without_launch_payload_stays_bootstrap_ready() {
        let settings = Settings::default();
        let mut runner = runner_profile(ExperimentRunnerBackend::Lambda);
        runner.image_or_runtime = Some("ghcr.io/thinclaw/research-runner:latest".to_string());
        runner.secret_references = vec!["research_lambda_api_key".to_string()];
        runner.backend_config = serde_json::json!({
            "template_id": "lambda-template"
        });

        let outcome = validate_runner_profile(&runner, &settings, None).await;
        assert!(outcome.valid);
        assert_eq!(
            outcome.readiness_class,
            ExperimentRunnerReadinessClass::BootstrapReady
        );
        assert!(!outcome.launch_eligible);
    }

    #[tokio::test]
    async fn agent_env_runner_validates_camel_case_webui_template_config() {
        let settings = Settings::default();
        let mut runner = runner_profile(ExperimentRunnerBackend::AgentEnv);
        runner.backend_config = serde_json::json!({
            "benchmark": "terminal_bench",
            "cases": [{
                "name": "smoke",
                "command": "printf ok",
                "expectedStdoutContains": ["ok"],
                "expectedExitCode": 0,
                "timeoutSecs": 5
            }]
        });

        let outcome = validate_runner_profile(&runner, &settings, None).await;
        assert!(outcome.valid, "{}", outcome.message);
        assert!(outcome.launch_eligible);
    }

    #[tokio::test]
    async fn agent_env_runner_rejects_malformed_benchmark_config() {
        let settings = Settings::default();
        let mut runner = runner_profile(ExperimentRunnerBackend::AgentEnv);
        runner.backend_config = serde_json::json!({
            "benchmark": "skill_bench",
            "cases": [{
                "name": "missing-content"
            }]
        });

        let outcome = validate_runner_profile(&runner, &settings, None).await;
        assert!(!outcome.valid);
        assert!(
            outcome
                .message
                .contains("Invalid AgentEnv benchmark config")
        );
    }

    fn lease_auth() -> ExperimentLeaseAuthentication {
        ExperimentLeaseAuthentication {
            lease_id: Uuid::new_v4(),
            token: "exp_0123456789ab_0123456789abcdef0123456789abcdef".to_string(),
        }
    }

    #[test]
    fn gateway_urls_fail_closed_before_building_bootstrap_commands() {
        let auth = lease_auth();
        assert!(build_bootstrap_command("https://gateway.example/base", &auth).is_ok());
        assert!(build_bootstrap_command("http://127.0.0.1:3001", &auth).is_ok());
        for invalid in [
            "",
            "http://gateway.example",
            "https://user:pass@gateway.example",
            "https://gateway.example/?token=secret",
            "https://gateway.example/#fragment",
            "file:///tmp/gateway",
        ] {
            assert!(
                build_bootstrap_command(invalid, &auth).is_err(),
                "accepted {invalid}"
            );
        }
    }

    #[test]
    fn bootstrap_command_shell_quotes_every_variable_argument() {
        let auth = ExperimentLeaseAuthentication {
            lease_id: Uuid::new_v4(),
            token: "safe'quoted".to_string(),
        };
        let command = build_bootstrap_command("https://gateway.example/base", &auth).unwrap();
        assert!(command.contains("--gateway-url 'https://gateway.example/base'"));
        assert!(command.contains("--token 'safe'\"'\"'quoted'"));
    }

    #[test]
    fn durable_provider_metadata_excludes_bootstrap_env_and_provider_echoes() {
        let token = "exp_0123456789ab_0123456789abcdef0123456789abcdef";
        let raw = serde_json::json!({
            "provider": "runpod",
            "pod_id": "pod-123",
            "launch_request": {
                "gpuTypeIds": ["NVIDIA H100"],
                "dataCenterIds": ["EU-1"],
                "dockerStartCmd": [format!("thinclaw --token {token}")],
                "env": { "API_KEY": "secret" }
            },
            "pod": {
                "id": "pod-123",
                "status": "RUNNING",
                "costPerHr": 1.25,
                "echo": token
            },
            "response": { "request": format!("--token {token}") }
        });
        let sanitized = sanitize_provider_job_metadata(ExperimentRunnerBackend::Runpod, &raw);
        let encoded = sanitized.to_string();
        assert!(!encoded.contains(token));
        assert!(!encoded.contains("dockerStartCmd"));
        assert!(!encoded.contains("API_KEY"));
        assert!(!encoded.contains("response"));
        assert_eq!(sanitized["pod_id"], "pod-123");
        assert_eq!(sanitized["pod"]["costPerHr"], 1.25);
        assert_eq!(sanitized["launch_request"]["gpuTypeIds"][0], "NVIDIA H100");
    }

    #[test]
    fn provider_ids_and_slurm_output_are_strictly_parsed() {
        assert_eq!(
            validate_provider_id("RunPod", "pod_123-abc").unwrap(),
            "pod_123-abc"
        );
        assert!(validate_provider_id("RunPod", "../../pods/other").is_err());
        assert!(validate_provider_id("RunPod", "pod\nother").is_err());
        assert_eq!(
            parse_slurm_job_id("Submitted batch job 1842\n").as_deref(),
            Some("1842")
        );
        assert_eq!(parse_slurm_job_id("submission accepted"), None);
    }

    #[test]
    fn kubernetes_manifest_quotes_configured_yaml_scalars() {
        let manifest = kubernetes_job_manifest(
            "job\nkind: Secret",
            "namespace\nmetadata: injected",
            "image:latest\nprivileged: true",
            "thinclaw experiment-runner",
            BTreeMap::from([("KEY\nvalue: injected".to_string(), "value".to_string())]),
            &serde_json::json!({ "gpu_count": 1 }),
        );
        assert!(!manifest.contains("\nkind: Secret\n"));
        assert!(!manifest.contains("\nprivileged: true\n"));
        assert!(manifest.contains("job\\nkind: Secret"));
        assert!(manifest.contains("image:latest\\nprivileged: true"));
    }

    #[test]
    fn launch_outcome_debug_never_renders_bootstrap_material() {
        let token = lease_auth().token;
        let outcome = RunnerLaunchOutcome {
            message: "manual".to_string(),
            bootstrap_command: Some(format!("run --token {token}")),
            provider_template: Some(serde_json::json!({ "token": token })),
            provider_job_id: None,
            provider_job_metadata: serde_json::json!({ "echo": token }),
            auto_launched: false,
            requires_operator_action: true,
        };
        let debug = format!("{outcome:?}");
        assert!(!debug.contains(&token));
        assert!(debug.contains("[REDACTED]"));
    }
}
