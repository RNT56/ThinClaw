use crate::config::ConfigManager;
use crate::images::ImageResponse;
use crate::sidecar::SidecarManager;
use tauri::AppHandle;
use tauri::Emitter;
use tauri::Manager;
use tauri::State;
use tauri_plugin_shell::ShellExt;
use tempfile::NamedTempFile;
use uuid::Uuid;

#[derive(serde::Deserialize, specta::Type)]
pub struct ImageGenParams {
    pub prompt: String,
    pub model: Option<String>,
    pub vae: Option<String>,
    pub clip_l: Option<String>,
    pub clip_g: Option<String>,
    pub t5xxl: Option<String>,
    pub negative_prompt: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub steps: Option<u32>,
    pub cfg_scale: Option<f32>,
    #[specta(type = f64)]
    pub seed: Option<i64>,
    pub schedule: Option<String>,
    pub sampling_method: Option<String>,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum DiffusionArchitecture {
    Flux1,
    Flux2Klein,
    SD15,
    SD21,
    SDXL,
    SD35Medium,
    SD35LargeTurbo,
    QwenImage,
    Wan21,
    Unknown,
}

impl DiffusionArchitecture {
    fn detect(model_path: &str) -> Self {
        let lower = model_path.to_lowercase();
        if lower.contains("flux") {
            if lower.contains("klein") {
                DiffusionArchitecture::Flux2Klein
            } else {
                DiffusionArchitecture::Flux1
            }
        } else if lower.contains("sd3.5") || lower.contains("sd 3.5") || lower.contains("sd35") {
            if lower.contains("turbo") {
                DiffusionArchitecture::SD35LargeTurbo
            } else {
                DiffusionArchitecture::SD35Medium
            }
        } else if lower.contains("sdxl") {
            DiffusionArchitecture::SDXL
        } else if lower.contains("qwen") && (lower.contains("image") || lower.contains("diffusion"))
        {
            DiffusionArchitecture::QwenImage
        } else if lower.contains("wan2") {
            DiffusionArchitecture::Wan21
        } else if lower.contains("sd1.5") || lower.contains("sd 1.5") || lower.contains("sd15") {
            DiffusionArchitecture::SD15
        } else if lower.contains("sd2.1") || lower.contains("sd 2.1") || lower.contains("sd21") {
            DiffusionArchitecture::SD21
        } else if lower.contains("sd3") {
            // Standard SD3 (treat as 3.5 Medium logic for flags)
            DiffusionArchitecture::SD35Medium
        } else {
            DiffusionArchitecture::Unknown
        }
    }

    fn is_flux(&self) -> bool {
        matches!(self, Self::Flux1 | Self::Flux2Klein)
    }

    fn is_modern_dit(&self) -> bool {
        // Models that use --diffusion-model and --diffusion-fa
        self.is_flux()
            || matches!(
                self,
                Self::SD35Medium | Self::SD35LargeTurbo | Self::QwenImage | Self::Wan21
            )
    }

    fn needs_model_flag(&self) -> bool {
        // Flux, Qwen and Wan strictly require --diffusion-model
        self.is_flux() || matches!(self, Self::QwenImage | Self::Wan21)
    }
}

async fn run_inference(
    app: &AppHandle,
    model_path: &str,
    params: &ImageGenParams,
    use_standard_fallbacks: bool,
    sd_threads_config: u32,
) -> Result<ImageResponse, String> {
    let output_temp =
        NamedTempFile::new().map_err(|e| format!("Failed to create temp file: {}", e))?;
    let output_path = output_temp.path().to_string_lossy().to_string();
    let output_png = format!("{}.png", output_path);

    let steps_val = params.steps.unwrap_or(20).to_string();
    let width_val = params.width.unwrap_or(512).to_string();
    let height_val = params.height.unwrap_or(512).to_string();
    let neg = params.negative_prompt.as_deref().unwrap_or_default();

    let total_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let threads = if sd_threads_config > 0 {
        sd_threads_config
    } else {
        // Flux Klein is heavy; use half cores or max 4 to keep UI alive
        let t = std::cmp::min(total_cores / 2, 4);
        std::cmp::max(t, 2) as u32
    };

    let arch = DiffusionArchitecture::detect(model_path);
    let is_flux = arch.is_flux();
    let is_klein = matches!(arch, DiffusionArchitecture::Flux2Klein);
    let is_sd35 = matches!(
        arch,
        DiffusionArchitecture::SD35Medium | DiffusionArchitecture::SD35LargeTurbo
    );

    let mut final_steps = steps_val;
    if params.steps.is_none() {
        match arch {
            DiffusionArchitecture::SD35LargeTurbo => {
                final_steps = "4".to_string();
            }
            DiffusionArchitecture::Flux2Klein => {
                final_steps = "50".to_string(); // User requested 50 for Base Klein
            }
            _ => {
                let lower_model = model_path.to_lowercase();
                if lower_model.contains("turbo") || lower_model.contains("lightning") {
                    final_steps = "4".to_string();
                } else if lower_model.contains("lcm") {
                    final_steps = "8".to_string();
                }
            }
        }
    }

    // --- ARGUMENT BUILDING ---
    let mut args = Vec::new();

    // Model selection
    if arch.needs_model_flag() {
        args.push("--diffusion-model".to_string());
        args.push(model_path.to_string());
    } else {
        args.push("-m".to_string());
        args.push(model_path.to_string());
    }

    // Basic Params
    args.push("-p".to_string());
    args.push(params.prompt.clone());
    args.push("-o".to_string());
    args.push(output_png.clone());
    args.push("--steps".to_string());
    args.push(final_steps);
    args.push("-W".to_string());
    args.push(width_val);
    args.push("-H".to_string());
    args.push(height_val);
    args.push("--vae-tiling".to_string());

    // Performance & Modern Features
    if arch.is_modern_dit() {
        args.push("--diffusion-fa".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        args.push("-t".to_string());
        args.push(if sd_threads_config == 0 {
            "-1".to_string()
        } else {
            threads.to_string()
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        if arch.is_modern_dit() {
            args.push("--offload-to-cpu".to_string());
        }
    }

    if let Some(cfg) = params.cfg_scale {
        args.push("--cfg-scale".into());
        args.push(cfg.to_string());
    } else {
        match arch {
            DiffusionArchitecture::Flux2Klein => {
                args.push("--cfg-scale".into());
                args.push("4.0".into());
            }
            DiffusionArchitecture::Flux1 => {
                args.push("--cfg-scale".into());
                args.push("1.0".into());
            }
            DiffusionArchitecture::SD35Medium | DiffusionArchitecture::SD35LargeTurbo => {
                args.push("--cfg-scale".into());
                args.push("4.5".into());
            }
            DiffusionArchitecture::QwenImage => {
                args.push("--cfg-scale".into());
                args.push("2.5".into());
            }
            DiffusionArchitecture::Wan21 => {
                args.push("--cfg-scale".into());
                args.push("5.0".into());
            }
            DiffusionArchitecture::SDXL => {
                args.push("--cfg-scale".into());
                args.push("7.0".into());
            }
            DiffusionArchitecture::SD15 | DiffusionArchitecture::SD21 => {
                args.push("--cfg-scale".into());
                args.push("7.5".into());
            }
            _ => {}
        }
    }

    args.push("-s".into());
    if let Some(seed) = params.seed {
        args.push(seed.to_string());
    } else {
        args.push("-1".into());
    }

    args.push("-v".into()); // Verbose logging to debug Metal initialization

    if arch.is_modern_dit() {
        args.push("--flow-shift".into());
        if matches!(arch, DiffusionArchitecture::QwenImage) {
            args.push("3.0".into());
        } else {
            args.push("1.15".into()); // CRITICAL: Restored for Flux/SD35
        }
    }

    #[cfg(target_os = "macos")]
    {
        if !is_klein {
            args.push("--vae-tiling".into());
        } else {
            args.push("--vae-on-cpu".into()); // CRITICAL: Fixes the noisy grid artifacts on Apple Silicon
        }
        args.push("--rng".into());
        args.push("cpu".into()); // More stable for Flux models
    }

    if is_flux {
        args.push("--guidance".into());
        args.push("3.5".into());
    }

    if !neg.is_empty() {
        args.push("-n".into());
        args.push(neg.into());
    }

    if let Some(m) = &params.sampling_method {
        args.push("--sampling-method".into());
        args.push(m.clone());
    }

    if let Some(s) = &params.schedule {
        args.push("--scheduler".into());
        args.push(s.clone());
    }

    // --- COMPONENT DISCOVERY ---
    let model_dir = std::path::Path::new(model_path).parent();
    let find_in_dir = |dir: &std::path::Path, keyword: &str| -> Option<String> {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut matches = Vec::new();
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    let name = p
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_ascii_lowercase();
                    if (name.contains(keyword) || (keyword == "vae" && name.contains("ae")))
                        && (name.ends_with(".safetensors")
                            || name.ends_with(".gguf")
                            || name.ends_with(".sft"))
                    {
                        matches.push(p.to_string_lossy().to_string());
                    }
                }
            }
            matches.sort_by_key(|a| a.len());
            if !matches.is_empty() {
                return Some(matches[0].clone());
            }
        }
        None
    };

    let find_standard_fallback = |category: &str, keyword: &str| -> Option<String> {
        if let Ok(app_dir) = app.path().app_data_dir() {
            let standard_dir = app_dir.join("models").join("standard").join(category);
            if let Ok(entries) = std::fs::read_dir(standard_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    let name = p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_ascii_lowercase();
                    if name.contains(keyword) || keyword == "*" {
                        return Some(p.to_string_lossy().to_string());
                    }
                }
            }
        }
        None
    };

    // VAE
    if let Some(v) = &params.vae {
        args.push("--vae".into());
        args.push(v.clone());
    } else if let Some(found) = model_dir.and_then(|d| find_in_dir(d, "vae")) {
        args.push("--vae".into());
        args.push(found);
    } else if use_standard_fallbacks {
        if let Some(found) = find_standard_fallback("vae", "*") {
            args.push("--vae".into());
            args.push(found);
        }
    }

    // Text Encoders
    // Flux Klein uses Qwen as LLM. Standard Flux uses T5XXL + CLIP_L.
    // SD 3.5 requires Triple Mapping (CLIP_L, CLIP_G, T5XXL)
    let mut has_llm_or_t5 = false;

    // Explicit CLIP_G (Required for SD 3.5)
    if let Some(cg) = &params.clip_g {
        args.push("--clip_g".into());
        args.push(cg.clone());
    } else if is_sd35 && use_standard_fallbacks {
        if let Some(found) = find_standard_fallback("clip", "clip_g") {
            args.push("--clip_g".into());
            args.push(found);
        }
    } else if is_sd35 {
        // Look in model dir for clip_g
        if let Some(found) = model_dir.and_then(|d| find_in_dir(d, "clip_g")) {
            args.push("--clip_g".into());
            args.push(found);
        }
    }

    // Explicit T5XXL (or Qwen-based LLM for QwenImage/Klein)
    if let Some(t) = &params.t5xxl {
        if matches!(
            arch,
            DiffusionArchitecture::QwenImage | DiffusionArchitecture::Flux2Klein
        ) || t.to_lowercase().contains("qwen")
        {
            args.push("--llm".into());
        } else {
            args.push("--t5xxl".into());
        }
        args.push(t.clone());
        has_llm_or_t5 = true;
    }

    // Explicit CLIP_L (Skip for Klein unless explicitly passed)
    if let Some(c) = &params.clip_l {
        args.push("--clip_l".into());
        args.push(c.clone());
    } else if !is_klein && use_standard_fallbacks {
        // Only fallback to standard CLIP_L if NOT Klein
        if let Some(found) = find_standard_fallback("clip", "clip_l") {
            args.push("--clip_l".into());
            args.push(found);
        }
    } else if (is_flux || is_sd35) && !is_klein {
        // Look in model dir for clip_l
        if let Some(found) = model_dir.and_then(|d| find_in_dir(d, "clip_l")) {
            args.push("--clip_l".into());
            args.push(found);
        }
    }

    // Auto-discovery
    if !has_llm_or_t5 {
        // Check for Qwen (Klein)
        if let Some(found) = model_dir.and_then(|d| find_in_dir(d, "qwen")) {
            args.push("--llm".into());
            args.push(found);
        }
        // Check for T5XXL
        else if let Some(found) = model_dir.and_then(|d| find_in_dir(d, "t5xxl")) {
            args.push("--t5xxl".into());
            args.push(found);
        }
        // Fallback for T5
        else if use_standard_fallbacks && !is_klein {
            if let Some(found) = find_standard_fallback("t5", "t5") {
                args.push("--t5xxl".into());
                args.push(found);
            }
        }
    }

    println!("[image_gen] Executing: sd-sidecar {}", args.join(" "));

    let mut command_runner = app.shell().sidecar("sd").map_err(|e| e.to_string())?;

    // Environment & Library Loading
    let mut bin_path = None;
    if let Ok(resource_dir) = app.path().resource_dir() {
        bin_path = Some(resource_dir.join("bin"));
    }

    // Fallback for Dev Mode
    if let Ok(cwd) = std::env::current_dir() {
        let dev_bin = cwd.join("src-tauri").join("bin");
        if dev_bin.exists() {
            bin_path = Some(dev_bin);
        }
    }

    if let Some(ref p) = bin_path {
        let path_str = p.to_string_lossy().to_string();

        // Set working directory - most robust way for it to find shaders/libs
        command_runner = command_runner.current_dir(p.clone());

        // Essential environment variables for Metal
        #[cfg(target_os = "macos")]
        {
            command_runner = command_runner.env("DYLD_LIBRARY_PATH", &path_str);
            command_runner = command_runner.env("LD_LIBRARY_PATH", &path_str);
            command_runner = command_runner.env("DYLD_FRAMEWORK_PATH", &path_str);
            command_runner = command_runner.env("GGML_METAL_PATH_RESOURCES", &path_str);
            command_runner = command_runner.env("GGML_METAL_RESOURCE_PATH", &path_str);
            command_runner = command_runner.env("SD_METAL_PATH_RESOURCES", &path_str);
            command_runner = command_runner.env("METAL_LIBRARY_PATH", &path_str);

            // Helpful if the system has multiple GPUs
            command_runner = command_runner.env("METAL_DEVICE", "0");

            // Critical for some Mac setups to find the .metal file
            // std::env::set_var("GGML_METAL_PATH_RESOURCES", &path_str); // REMOVED: Leaking to main process causes UI crashes
        }
    }

    let (mut rx, child) = command_runner
        .args(args)
        .spawn()
        .map_err(|e| format!("Failed to spawn sd: {}", e))?;

    let mut success = true;

    while let Some(event) = rx.recv().await {
        match event {
            tauri_plugin_shell::process::CommandEvent::Stdout(line)
            | tauri_plugin_shell::process::CommandEvent::Stderr(line) => {
                let text = String::from_utf8_lossy(&line);
                println!("[image_gen] {}", text);

                // Detect progress bars: |====>   | 28/795
                if let Some(caps) = regex::Regex::new(r"\|\s*([|=]+)\s*\|\s*(\d+)/(\d+)")
                    .unwrap()
                    .captures(&text)
                {
                    let current = caps[2].parse::<f32>().unwrap_or(0.0);
                    let total = caps[3].parse::<f32>().unwrap_or(1.0);
                    let progress = current / total;

                    let stage = if total > 100.0 {
                        "Loading Weights"
                    } else {
                        "Generating"
                    };
                    let payload = serde_json::json!({
                        "stage": stage,
                        "progress": progress,
                        "text": format!("{} ({:.0}%)", stage, progress * 100.0)
                    });
                    app.emit("image_gen_progress", payload).ok();
                } else if text.contains("loading diffusion model") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Initializing", "progress": 0.05, "text": "Loading diffusion engine..."})).ok();
                } else if text.contains("Starting local generation") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Initializing", "progress": 0.02, "text": "Preparing local engine..."})).ok();
                } else if text.contains("Strict Mode") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Initializing", "progress": 0.04, "text": "Starting inference..."})).ok();
                } else if text.contains("Stopping chat server") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Memory Optimization", "progress": 0.01, "text": "Freeing VRAM..."})).ok();
                } else if text.contains("loading llm") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Initializing", "progress": 0.1, "text": "Loading language model..."})).ok();
                } else if text.contains("loading vae") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Initializing", "progress": 0.15, "text": "Loading VAE..."})).ok();
                } else if text.contains("sampling using") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Generating", "progress": 0.2, "text": "Starting sampling..."})).ok();
                } else if text.contains("sampling completed") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Saving", "progress": 0.9, "text": "Sampling completed..."})).ok();
                } else if text.contains("decoding") && text.contains("latents") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Saving", "progress": 0.92, "text": "Decoding image..."})).ok();
                } else if text.contains("latent") && text.contains("decoded") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Saving", "progress": 0.98, "text": "Finalizing image..."})).ok();
                } else if text.contains("generate_image completed") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Saving", "progress": 1.0, "text": "Generation finished!"})).ok();
                } else if text.contains("Using Metal backend") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Engine Setup", "progress": 0.25, "text": "Initializing Metal GPU..."})).ok();
                } else if text.contains("running in Flux2 FLOW mode") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Engine Setup", "progress": 0.3, "text": "Configuring Flux Flow..."})).ok();
                } else if text.contains("compiling pipeline")
                    || text.contains("ggml_metal_library_compile_pipeline")
                    || text.contains("ggml_extend.hpp")
                {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Engine Setup", "progress": 0.28, "text": "Compiling GPU shaders..."})).ok();
                } else if text.contains("loading tensors completed") {
                    app.emit("image_gen_progress", serde_json::json!({"stage": "Generating", "progress": 0.35, "text": "Model ready, starting generation..."})).ok();
                } else if text.trim().len() > 5 && !text.contains("|") {
                    // Fallback for other interesting output
                    app.emit("image_gen_progress", text.trim()).ok();
                }
            }
            tauri_plugin_shell::process::CommandEvent::Terminated(payload) => {
                if let Some(code) = payload.code {
                    if code != 0 {
                        success = false;
                    }
                }
            }
            _ => {}
        }
    }

    let _ = child.kill();
    if !success {
        return Err("Image Generation Failed".to_string());
    }

    let images_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("images");
    if !images_dir.exists() {
        std::fs::create_dir_all(&images_dir).map_err(|e| e.to_string())?;
    }

    let id = Uuid::new_v4().to_string();
    let final_path = images_dir.join(format!("{}.png", id));

    println!("[image_gen] Saving result to: {:?}", final_path);
    std::fs::copy(&output_png, &final_path).map_err(|e| format!("Failed to copy image: {}", e))?;
    let _ = std::fs::remove_file(&output_png);

    // Emit success event so UI can update immediately
    println!("[image_gen] Emitting image_gen_success for ID: {}", id);
    app.emit(
        "image_gen_success",
        serde_json::json!({
            "original_id": "pending_generation",
            "final_id": id
        }),
    )
    .ok();

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    Ok(ImageResponse {
        id,
        path: final_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn generate_image(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    config: State<'_, ConfigManager>,
    params: ImageGenParams,
) -> Result<ImageResponse, String> {
    // 1. Try params first
    let mut model_path = params.model.clone().unwrap_or_default();

    // 2. If empty, check SidecarManager (active model)
    if model_path.is_empty() {
        if let Some(active) = state.get_image_model() {
            model_path = active;
        }
    }

    println!(
        "[image_gen] Resolved model path: '{}' (Input was: '{:?}')",
        model_path, params.model
    );

    if model_path.is_empty() {
        return Err(
            "No model selected. Please select a model in Settings or the Chat interface.".into(),
        );
    }

    config.reload();
    let user_config = config.get_config();
    let final_params = params;

    println!(
        "[image_gen] Starting local generation for model: {}",
        model_path
    );

    // Stop chat server to free GPU memory for heavy Flux models
    // This prevents "CPU backend" fallback and "Black Screen" GPU crashes
    println!("[image_gen] Stopping chat server to free VRAM...");
    app.emit("image_gen_progress", serde_json::json!({"stage": "Memory Optimization", "progress": 0.01, "text": "Optimizing memory..."}).to_string()).ok();

    if let Err(e) = state.stop_chat_server() {
        println!("[image_gen] Warning: Failed to stop chat server: {}", e);
    }

    // Explicitly emit progress to UI to ensure it knows we are working
    app.emit("image_gen_progress", serde_json::json!({"stage": "Initializing", "progress": 0.03, "text": "Starting engine..."}).to_string()).ok();

    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

    println!("[image_gen] Attempt 1: Strict Mode...");
    let result = match run_inference(
        &app,
        &model_path,
        &final_params,
        false,
        user_config.sd_threads,
    )
    .await
    {
        Ok(res) => Ok(res),
        Err(_) => {
            println!("Attempt 2: Fallback Mode...");
            // Use longer sleep before fallback retry
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            run_inference(
                &app,
                &model_path,
                &final_params,
                true,
                user_config.sd_threads,
            )
            .await
        }
    };

    if result.is_ok() {
        println!("[image_gen] Cooldown: Waiting 3s for GPU memory release...");
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        // Also restart chat server here if needed? No, let user do it or auto-manager.
    }

    result
}
