use clap::Subcommand;
use serde_json::json;
use std::path::PathBuf;
use tokio::process::Command;

use crate::config::ComfyUiConfig;
use crate::settings::Settings;

#[derive(Subcommand, Debug, Clone)]
pub enum ComfyCommand {
    /// Check configured ComfyUI health.
    Health,
    /// Print local hardware suitability information.
    HardwareCheck,
    /// Launch local ComfyUI through comfy-cli.
    Launch,
    /// Stop local ComfyUI through comfy-cli.
    Stop,
    /// Install ComfyUI through comfy-cli.
    Setup {
        #[arg(long, default_value = "cpu")]
        gpu: String,
    },
    /// List bundled workflow names.
    ListWorkflows,
    /// Generate media through the configured ComfyUI server.
    Generate {
        prompt: String,
        #[arg(long)]
        workflow: Option<String>,
        #[arg(long, default_value = "square")]
        aspect_ratio: String,
        #[arg(long)]
        negative_prompt: Option<String>,
        #[arg(long)]
        seed: Option<i64>,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
        #[arg(long)]
        steps: Option<u32>,
        #[arg(long)]
        cfg: Option<f64>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        input_image: Option<PathBuf>,
        #[arg(long)]
        mask_image: Option<PathBuf>,
        #[arg(long)]
        no_wait: bool,
    },
    /// Check dependencies for a bundled or approved workflow.
    CheckDeps { workflow: String },
}

pub async fn run_comfy_command(command: ComfyCommand) -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    crate::bootstrap::load_thinclaw_env();
    match command {
        ComfyCommand::Health => {
            let config = load_comfy_config()?;
            let client = comfy_client(&config, None)?;
            println!("{}", serde_json::to_string_pretty(&client.health().await)?);
        }
        ComfyCommand::HardwareCheck => {
            println!("{}", serde_json::to_string_pretty(&hardware_check())?);
        }
        ComfyCommand::Launch => {
            let config = load_comfy_config()?;
            let port = config.port.to_string();
            print_command(
                run_command(
                    "comfy",
                    &["launch", "--background", "--", "--port", &port],
                    Some(&config.workspace_dir),
                )
                .await?,
            )?;
        }
        ComfyCommand::Stop => {
            let config = load_comfy_config()?;
            print_command(run_command("comfy", &["stop"], Some(&config.workspace_dir)).await?)?;
        }
        ComfyCommand::Setup { gpu } => {
            let config = load_comfy_config()?;
            let flag = match gpu.as_str() {
                "nvidia" => "--nvidia",
                "amd" => "--amd",
                "m-series" => "--m-series",
                "cpu" => "--cpu",
                other => anyhow::bail!("invalid gpu '{}'", other),
            };
            print_command(
                run_command(
                    "comfy",
                    &["--skip-prompt", "install", flag],
                    Some(&config.workspace_dir),
                )
                .await?,
            )?;
        }
        ComfyCommand::ListWorkflows => {
            println!(
                "{}",
                serde_json::to_string_pretty(thinclaw_media::bundled_workflow_names())?
            );
        }
        ComfyCommand::Generate {
            prompt,
            workflow,
            aspect_ratio,
            negative_prompt,
            seed,
            width,
            height,
            steps,
            cfg,
            model,
            input_image,
            mask_image,
            no_wait,
        } => {
            let config = load_comfy_config()?;
            let client = comfy_client(&config, None)?;
            let workflow_name = workflow.unwrap_or_else(|| config.default_workflow.clone());
            let workflow_json =
                load_workflow(&workflow_name, config.allow_untrusted_workflows).await?;
            let aspect_ratio = aspect_ratio.parse::<thinclaw_media::ComfyAspectRatio>()?;
            let generation = client
                .generate(thinclaw_media::ComfyGenerateRequest {
                    prompt,
                    negative_prompt,
                    aspect_ratio,
                    width,
                    height,
                    seed,
                    steps,
                    cfg,
                    model,
                    workflow: workflow_json,
                    workflow_name,
                    input_image,
                    mask_image,
                    wait_for_completion: !no_wait,
                    use_websocket: true,
                })
                .await?;
            println!("{}", serde_json::to_string_pretty(&generation)?);
        }
        ComfyCommand::CheckDeps { workflow } => {
            let config = load_comfy_config()?;
            let client = comfy_client(&config, None)?;
            let workflow_json = load_workflow(&workflow, config.allow_untrusted_workflows).await?;
            let report = client.check_dependencies(&workflow_json).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
    }
    Ok(())
}

fn load_comfy_config() -> anyhow::Result<ComfyUiConfig> {
    let mut settings = Settings::load();
    let path = Settings::default_toml_path();
    match Settings::load_toml(&path) {
        Ok(Some(toml_settings)) => settings.merge_from(&toml_settings),
        Ok(None) => {}
        Err(error) => anyhow::bail!("failed to load {}: {}", path.display(), error),
    }
    Ok(ComfyUiConfig::resolve(&settings)?)
}

fn comfy_client(
    config: &ComfyUiConfig,
    api_key: Option<String>,
) -> anyhow::Result<thinclaw_media::ComfyUiClient> {
    let mode = match config.mode.as_str() {
        "local_existing" => thinclaw_media::ComfyUiMode::LocalExisting,
        "local_managed" => thinclaw_media::ComfyUiMode::LocalManaged,
        "cloud" => thinclaw_media::ComfyUiMode::Cloud,
        other => anyhow::bail!("invalid comfyui.mode '{}'", other),
    };
    Ok(thinclaw_media::ComfyUiClient::new(
        thinclaw_media::ComfyUiConfig {
            mode,
            host: config.host.clone(),
            api_key: api_key.or_else(|| std::env::var("COMFY_CLOUD_API_KEY").ok()),
            output_dir: config.output_dir.clone(),
            request_timeout: config.request_timeout,
            max_output_bytes: config.max_output_bytes,
        },
    )?)
}

async fn load_workflow(
    name_or_path: &str,
    allow_untrusted: bool,
) -> anyhow::Result<serde_json::Value> {
    if let Some(workflow) = thinclaw_media::bundled_workflow(name_or_path) {
        return Ok(workflow);
    }
    if !allow_untrusted {
        anyhow::bail!(
            "unknown bundled workflow '{}'; set comfyui.allow_untrusted_workflows=true to load workflow files",
            name_or_path
        );
    }
    let content = tokio::fs::read_to_string(name_or_path).await?;
    let workflow = serde_json::from_str(&content)?;
    thinclaw_media::validate_api_workflow(&workflow)?;
    Ok(workflow)
}

fn hardware_check() -> serde_json::Value {
    let mut system = sysinfo::System::new_all();
    system.refresh_all();
    let total_memory_gib = system.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    let verdict = if cfg!(target_os = "macos") && total_memory_gib >= 16.0 {
        "ok_m_series_or_cpu"
    } else if total_memory_gib >= 8.0 {
        "ok_if_gpu_available"
    } else {
        "cloud_recommended"
    };
    json!({
        "os": sysinfo::System::name().unwrap_or_else(|| std::env::consts::OS.to_string()),
        "arch": std::env::consts::ARCH,
        "cpu_count": system.cpus().len(),
        "total_memory_gib": (total_memory_gib * 10.0).round() / 10.0,
        "verdict": verdict
    })
}

async fn run_command(
    program: &str,
    args: &[&str],
    cwd: Option<&std::path::Path>,
) -> anyhow::Result<serde_json::Value> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        tokio::fs::create_dir_all(cwd).await?;
        command.current_dir(cwd);
    }
    let output = command.output().await?;
    Ok(json!({
        "program": program,
        "args": args,
        "success": output.status.success(),
        "status": output.status.code(),
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }))
}

fn print_command(value: serde_json::Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
