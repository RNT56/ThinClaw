//! Sidecar process lifecycle: spawning the chat/embedding/summarizer/stt
//! servers (native llama.cpp/whisper + optional MLX Python backends), the
//! CLI-tool path trackers (image/tts), and the stop/teardown methods.

use anyhow::{anyhow, Result};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

use super::core::SidecarManager;
use super::types::{ChatServerOptions, SidecarEvent, SidecarProcess};

impl SidecarManager {
    pub fn direct_runtime_start_chat_server<F>(
        &self,
        app: AppHandle,
        options: ChatServerOptions,
        on_exit: F,
    ) -> Result<(u16, String)>
    where
        F: Fn(i32) + Send + Sync + 'static,
    {
        let model_path = options.model_path;
        let context_size = options.context_size;
        let n_gpu = options.n_gpu;
        let template_name = options.template;
        let mmproj_path_override = options.mmproj;
        let expose = options.expose;
        let mlock = options.mlock;
        let quantize_kv = options.quantize_kv;

        // Resolve Template + detect model family from GGUF metadata
        let gguf_meta = crate::gguf::read_gguf_metadata(&model_path).ok();
        let detected_family = gguf_meta
            .as_ref()
            .and_then(|m| m.model_family.clone())
            .unwrap_or_else(|| "chatml".to_string());

        println!("[sidecar] Detected model family: {}", detected_family);
        *self
            .detected_model_family
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(detected_family.clone());
        if let Some(ref meta) = gguf_meta {
            if let Some(ref tpl) = meta.chat_template {
                println!("[sidecar] GGUF chat_template present ({} chars)", tpl.len());
            }
        }

        let template_opt = match template_name.as_deref() {
            Some("llama3") => Some(crate::templates::LLAMA3_TEMPLATE),
            Some("mistral") => Some(crate::templates::MISTRAL_TEMPLATE),
            Some("gemma") => None, // Let llama-server use native GGUF template
            Some("qwen") => Some(crate::templates::QWEN_TEMPLATE),
            Some("chatml") => Some(crate::templates::CHATML_TEMPLATE),
            Some("auto") => None, // Let llama-server detect from GGUF
            None => {
                // Auto-detect from GGUF family
                match detected_family.as_str() {
                    "llama3" => Some(crate::templates::LLAMA3_TEMPLATE),
                    "mistral" => Some(crate::templates::MISTRAL_TEMPLATE),
                    "gemma" => None, // Let llama-server handle Gemma natively (uses 'model' role)
                    "qwen" => Some(crate::templates::QWEN_TEMPLATE),
                    "deepseek" => None, // Let llama-server handle deepseek natively
                    "glm" => None,      // Let llama-server handle GLM natively
                    _ => Some(crate::templates::CHATML_TEMPLATE),
                }
            }
            _ => Some(crate::templates::CHATML_TEMPLATE), // Unknown name -> ChatML
        };

        // Get ProcessTracker
        let tracker = app.state::<crate::process_tracker::ProcessTracker>();
        // Clean up any old zombies first
        tracker.cleanup_by_service("chat");

        // Reset intentional stop flag
        let mut process_guard = self.chat_process.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(proc) = process_guard.take() {
            // Signal that this stop is intentional (restart)
            *self
                .is_chat_stop_intentional
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = true;
            let old_port = proc.port;
            let _ = proc.kill();

            // Wait for the old port to clear (max 2s)
            for _ in 0..20 {
                if std::net::TcpListener::bind(format!("127.0.0.1:{}", old_port)).is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }

        let (port, token) = Self::generate_config(Some(53755));

        // Resolve cache path
        let app_data_dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."));
        // Ensure dir exists
        if !app_data_dir.exists() {
            let _ = std::fs::create_dir_all(&app_data_dir);
        }
        let cache_dir = app_data_dir.join("prompt_cache");
        if !cache_dir.exists() {
            let _ = std::fs::create_dir_all(&cache_dir);
        }
        let cache_path_str = cache_dir.to_string_lossy().to_string();

        let mut command = app
            .shell()
            .sidecar("llama-server")
            .map_err(|e| anyhow!("Failed to create sidecar command: {}", e))?;

        // Resolve bin dir for libraries (DYLD_LIBRARY_PATH on macOS)
        if let Ok(resource_dir) = app.path().resource_dir() {
            let bin_dir = resource_dir.join("bin");
            #[cfg(target_os = "macos")]
            {
                let mut lib_path = bin_dir.to_string_lossy().to_string();

                // Fallback for dev mode
                if let Ok(cwd) = std::env::current_dir() {
                    let dev_bin = cwd.join("backend/bin");
                    if dev_bin.exists() {
                        lib_path = format!("{}:{}", dev_bin.to_string_lossy(), lib_path);
                    }
                }

                println!("[sidecar-chat] Setting DYLD_LIBRARY_PATH: {}", lib_path);
                command = command.env("DYLD_LIBRARY_PATH", lib_path);
            }
        }

        let mut args = vec![
            "--model".to_string(),
            model_path.clone(),
            "--ctx-size".to_string(),
            context_size.to_string(),
            "--n-gpu-layers".to_string(),
            n_gpu.to_string(),
            "--host".to_string(),
            if expose {
                "0.0.0.0".to_string()
            } else {
                "127.0.0.1".to_string()
            },
            "--port".to_string(),
            port.to_string(),
            "--api-key".to_string(),
            token.clone(),
            "--cache-prompt".to_string(),
            "--slot-save-path".to_string(),
            cache_path_str,
            // Performance: Flash Attention (Metal/CUDA) for faster inference
            "--flash-attn".to_string(),
            "on".to_string(),
            // Performance: Continuous batching for better prompt processing throughput
            "--cont-batching".to_string(),
        ];

        if mlock {
            args.push("--mlock".to_string());
        }

        if quantize_kv {
            args.push("--cache-type-k".to_string());
            args.push("q4_0".to_string());
            args.push("--cache-type-v".to_string());
            args.push("q4_0".to_string());
        }

        if let Some(t) = template_opt {
            args.push("--chat-template".to_string());
            args.push(t.to_string());
        }

        // NOTE: Stop tokens are NOT injected as CLI args (llama-server doesn't support --stop).
        // They are enforced at the API request level via ThinClaw model config (Layer 2 in config.rs).
        println!(
            "[sidecar] Stop tokens for family '{}' will be enforced at API request level",
            detected_family
        );

        // Handles MMProj (Vision)
        // Priority: Explicit Override > .mmproj file > Smart Discovery
        let mut _found_mmproj = false;

        if let Some(path) = mmproj_path_override {
            if !path.trim().is_empty() {
                println!("[sidecar] Using explicit mmproj: {}", path);
                args.push("--mmproj".to_string());
                args.push(path);
                _found_mmproj = true;
            }
        }

        if !_found_mmproj {
            // Check for mmproj file
            let mmproj_path = format!("{}.mmproj", model_path);
            if std::path::Path::new(&mmproj_path).exists() {
                println!("[sidecar] Found mmproj: {}", mmproj_path);
                args.push("--mmproj".to_string());
                args.push(mmproj_path);
                _found_mmproj = true;
            } else {
                // Fallback: Smart Discovery if in a subfolder
                // If the model is in "models/UseSpecificFolder/", we scan that folder for any "mmproj"
                let path_obj = std::path::Path::new(&model_path);
                if let Some(parent) = path_obj.parent() {
                    let parent_name = parent.file_name().unwrap_or_default().to_string_lossy();
                    // Ensure we are in a subfolder, not the root models dir
                    if parent_name != "models" {
                        if let Ok(entries) = std::fs::read_dir(parent) {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p.is_file() {
                                    let fname = p
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_ascii_lowercase();
                                    if fname.contains("mmproj")
                                        && (fname.ends_with(".gguf") || fname.ends_with(".bin"))
                                    {
                                        println!(
                                            "[sidecar] Auto-detected mmproj in subfolder: {:?}",
                                            p
                                        );
                                        args.push("--mmproj".to_string());
                                        args.push(p.to_string_lossy().to_string());
                                        _found_mmproj = true;
                                        break; // Use the first one found
                                    }
                                }
                            }
                        }
                    }
                }
                if !_found_mmproj {
                    println!("[sidecar] No mmproj found for: {}", model_path);
                }
            }
        }

        let bind_host = if expose { "0.0.0.0" } else { "127.0.0.1" };
        println!(
            "[sidecar] Spawning chat server: llama-server {} (listening on {}:{})",
            args.join(" "),
            bind_host,
            port
        );

        let (mut rx, child) = command
            .args(&args)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn llama-server: {}", e))?;

        let pid = child.pid();
        tracker.add_pid(pid, "llama-server", "chat");

        // Clone for async block
        let monitor_app = app.clone();

        // Log output & Monitor
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        let msg = String::from_utf8_lossy(&line);
                        println!("[llama-chat] {}", msg);

                        // Parse progress (sometimes on stdout)
                        if msg.contains("prompt processing progress") {
                            if let Some(idx) = msg.find("progress = ") {
                                let num_str = &msg[idx + 11..];
                                let end = num_str
                                    .find(|c: char| !c.is_numeric() && c != '.')
                                    .unwrap_or(num_str.len());
                                if let Ok(val) = num_str[..end].trim().parse::<f32>() {
                                    monitor_app
                                        .emit(
                                            "sidecar_event",
                                            SidecarEvent::Progress {
                                                service: "chat".into(),
                                                message: "Reading Context".into(),
                                                progress: val,
                                                total: 1.0,
                                            },
                                        )
                                        .ok();
                                }
                            }
                        }
                    }
                    CommandEvent::Stderr(line) => {
                        let msg = String::from_utf8_lossy(&line);
                        eprintln!("[llama-chat] {}", msg);

                        // Parse progress (usually on stderr for llama.cpp)
                        if msg.contains("prompt processing progress") {
                            if let Some(idx) = msg.find("progress = ") {
                                let num_str = &msg[idx + 11..];
                                let end = num_str
                                    .find(|c: char| !c.is_numeric() && c != '.')
                                    .unwrap_or(num_str.len());
                                if let Ok(val) = num_str[..end].trim().parse::<f32>() {
                                    monitor_app
                                        .emit(
                                            "sidecar_event",
                                            SidecarEvent::Progress {
                                                service: "chat".into(),
                                                message: "Reading Context".into(),
                                                progress: val,
                                                total: 1.0,
                                            },
                                        )
                                        .ok();
                                }
                            }
                        }
                    }
                    CommandEvent::Terminated(payload) => {
                        // Cleanup PID
                        monitor_app
                            .state::<crate::process_tracker::ProcessTracker>()
                            .remove_pid(pid);

                        if let Some(code) = payload.code {
                            println!("[sidecar] Chat Server terminated with code {:?}", code);
                            on_exit(code);
                        } else {
                            // Terminated without code (signal?)
                            on_exit(-1);
                        }
                    }
                    _ => {}
                }
            }
            // Ensure cleanup if loop exits otherwise (unlikely for spawned command)
            monitor_app
                .state::<crate::process_tracker::ProcessTracker>()
                .remove_pid(pid);
        });

        *process_guard = Some(SidecarProcess {
            child: Some(child),
            port,
            token: token.clone(),
            context_size,
            model_family: detected_family.clone(),
        });

        // Reset intentional stop flag for the new process lifecycle
        *self
            .is_chat_stop_intentional
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = false;

        Ok((port, token))
    }

    pub fn direct_runtime_start_embedding_server(
        &self,
        app: AppHandle,
        model_path: String,
    ) -> Result<(u16, String)> {
        // Get ProcessTracker
        let tracker = app.state::<crate::process_tracker::ProcessTracker>();
        tracker.cleanup_by_service("embedding");

        let mut process_guard = self
            .embedding_process
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = process_guard.take() {
            let _ = proc.kill();
        }

        let (port, token) = Self::generate_config(Some(53756));

        let mut command = app
            .shell()
            .sidecar("llama-server")
            .map_err(|e| anyhow!("Failed to create sidecar command: {}", e))?;

        // Resolve bin dir for libraries (DYLD_LIBRARY_PATH on macOS)
        if let Ok(resource_dir) = app.path().resource_dir() {
            let bin_dir = resource_dir.join("bin");
            #[cfg(target_os = "macos")]
            {
                let mut lib_path = bin_dir.to_string_lossy().to_string();

                // Fallback for dev mode
                if let Ok(cwd) = std::env::current_dir() {
                    let dev_bin = cwd.join("backend/bin");
                    if dev_bin.exists() {
                        lib_path = format!("{}:{}", dev_bin.to_string_lossy(), lib_path);
                    }
                }

                println!("[sidecar-embed] Setting DYLD_LIBRARY_PATH: {}", lib_path);
                command = command.env("DYLD_LIBRARY_PATH", lib_path);
            }
        }

        let mut args = vec![
            "--model".to_string(),
            model_path.clone(),
            "--embedding".to_string(),
            "--host".to_string(),
            "127.0.0.1".to_string(),
            "--port".to_string(),
            port.to_string(),
            "--api-key".to_string(),
            token.clone(),
            "--ctx-size".to_string(),
            "4096".to_string(),
            "--batch-size".to_string(),
            "512".to_string(),
            "--ubatch-size".to_string(),
            "512".to_string(),
            "--n-gpu-layers".to_string(),
            "0".to_string(),
        ];

        // Check for mmproj file
        let mmproj_path = format!("{}.mmproj", model_path);
        if std::path::Path::new(&mmproj_path).exists() {
            println!("[sidecar-embed] Found mmproj: {}", mmproj_path);
            args.push("--mmproj".to_string());
            args.push(mmproj_path);
        }

        println!("[sidecar-embed] Spawning: llama-server {}", args.join(" "));

        let (mut rx, child) = command
            .args(&args)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn embedding server: {}", e))?;

        let pid = child.pid();
        tracker.add_pid(pid, "llama-server", "embedding");

        let monitor_app = app.clone();

        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let CommandEvent::Stderr(line) = event {
                    let msg = String::from_utf8_lossy(&line);
                    eprintln!("[llama-embed] {}", msg);
                }
            }
            // Cleanup
            monitor_app
                .state::<crate::process_tracker::ProcessTracker>()
                .remove_pid(pid);
        });

        *process_guard = Some(SidecarProcess {
            child: Some(child),
            port,
            token: token.clone(),
            context_size: 4096, // Fixed for embedding
            model_family: "none".into(),
        });

        Ok((port, token))
    }

    #[cfg(feature = "mlx")]
    pub async fn start_mlx_embedding_server(
        &self,
        app: AppHandle,
        model_path: String,
    ) -> Result<(u16, String)> {
        let tracker = app.state::<crate::process_tracker::ProcessTracker>();
        tracker.cleanup_by_service("embedding");

        // Kill existing process, then DROP the lock before any await point.
        {
            let mut process_guard = self
                .embedding_process
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(proc) = process_guard.take() {
                let _ = proc.kill();
            }
        } // lock released here

        let (port, token) = Self::generate_config(Some(53756));

        let python_path = app
            .path()
            .app_data_dir()
            .map_err(|e| anyhow!("Failed to resolve app data dir: {}", e))?
            .join("mlx-env")
            .join("bin")
            .join("python3");

        if !python_path.exists() {
            return Err(anyhow!(
                "MLX venv not bootstrapped — Python not found at {:?}",
                python_path
            ));
        }

        let script_path = Self::resolve_mlx_script(&app, "mlx_embed_server.py")?;

        let mut args = vec![
            script_path.to_string_lossy().to_string(),
            "--model".to_string(),
            model_path.clone(),
            "--port".to_string(),
            port.to_string(),
            "--host".to_string(),
            "127.0.0.1".to_string(),
        ];
        if !token.is_empty() {
            args.push("--api-key".to_string());
            args.push(token.clone());
        }

        println!(
            "[sidecar-embed-mlx] Spawning: {} {}",
            python_path.display(),
            args.join(" ")
        );

        let command = app
            .shell()
            .command(python_path.to_string_lossy().as_ref())
            .args(&args);

        let (mut rx, child) = command
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn MLX embedding server: {}", e))?;

        let pid = child.pid();
        tracker.add_pid(pid, "mlx-embed-server", "embedding");

        // --- Async startup wait (no lock held) ---
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut startup_error: Option<String> = None;
        let mut ready = false;

        while !ready && tokio::time::Instant::now() < deadline {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(CommandEvent::Stdout(line))) => {
                    let msg = String::from_utf8_lossy(&line);
                    println!("[mlx-embed] {}", msg);
                    if msg.contains("ERROR:") {
                        startup_error = Some(msg.trim().to_string());
                        break;
                    }
                    if msg.contains("listening") || msg.contains("Model loaded") {
                        ready = true;
                    }
                }
                Ok(Some(CommandEvent::Stderr(line))) => {
                    let msg = String::from_utf8_lossy(&line);
                    eprintln!("[mlx-embed] {}", msg);
                    if msg.contains("ERROR:") || msg.contains("Error:") {
                        startup_error = Some(msg.trim().to_string());
                    }
                }
                Ok(Some(CommandEvent::Terminated(status))) => {
                    let code = status.code.unwrap_or(-1);
                    if code != 0 {
                        let msg = startup_error.take().unwrap_or_else(|| {
                            format!("Embedding server exited with code {}", code)
                        });
                        return Err(anyhow!("{}", msg));
                    }
                    break;
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => break, // Deadline elapsed — proceed; startup_error handles real failures
            }
        }

        if let Some(err) = startup_error {
            return Err(anyhow!("{}", err));
        }

        // Background monitor
        let monitor_app = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stderr(l) => {
                        eprintln!("[mlx-embed] {}", String::from_utf8_lossy(&l))
                    }
                    CommandEvent::Stdout(l) => {
                        println!("[mlx-embed] {}", String::from_utf8_lossy(&l))
                    }
                    _ => {}
                }
            }
            monitor_app
                .state::<crate::process_tracker::ProcessTracker>()
                .remove_pid(pid);
        });

        // Re-acquire lock to store process
        *self
            .embedding_process
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(SidecarProcess {
            child: Some(child),
            port,
            token: token.clone(),
            context_size: 4096,
            model_family: "mlx-embedding".into(),
        });

        Ok((port, token))
    }

    #[cfg(feature = "mlx")]
    pub async fn start_mlx_stt_server(
        &self,
        app: AppHandle,
        model_path: String,
    ) -> Result<(u16, String)> {
        let tracker = app.state::<crate::process_tracker::ProcessTracker>();
        tracker.cleanup_by_service("stt");

        // Kill existing, drop lock before await
        {
            let mut process_guard = self.stt_process.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(proc) = process_guard.take() {
                let _ = proc.kill();
            }
        }

        let (port, token) = Self::generate_config(Some(53757));

        let python_path = app
            .path()
            .app_data_dir()
            .map_err(|e| anyhow!("Failed to resolve app data dir: {}", e))?
            .join("mlx-env")
            .join("bin")
            .join("python3");

        if !python_path.exists() {
            return Err(anyhow!(
                "MLX venv not bootstrapped — Python not found at {:?}",
                python_path
            ));
        }

        let script_path = Self::resolve_mlx_script(&app, "mlx_stt_server.py")?;

        let mut args = vec![
            script_path.to_string_lossy().to_string(),
            "--model".to_string(),
            model_path.clone(),
            "--port".to_string(),
            port.to_string(),
            "--host".to_string(),
            "127.0.0.1".to_string(),
        ];
        if !token.is_empty() {
            args.push("--api-key".to_string());
            args.push(token.clone());
        }

        println!(
            "[sidecar-stt-mlx] Spawning: {} {}",
            python_path.display(),
            args.join(" ")
        );

        let command = app
            .shell()
            .command(python_path.to_string_lossy().as_ref())
            .args(&args);

        let (mut rx, child) = command
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn MLX STT server: {}", e))?;

        let pid = child.pid();
        tracker.add_pid(pid, "mlx-stt-server", "stt");

        // Async startup wait (no lock held)
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut startup_error: Option<String> = None;
        let mut ready = false;

        while !ready && tokio::time::Instant::now() < deadline {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(CommandEvent::Stdout(line))) => {
                    let msg = String::from_utf8_lossy(&line);
                    println!("[mlx-stt] {}", msg);
                    if msg.contains("ERROR:") {
                        startup_error = Some(msg.trim().to_string());
                        break;
                    }
                    if msg.contains("listening") {
                        ready = true;
                    }
                }
                Ok(Some(CommandEvent::Stderr(line))) => {
                    let msg = String::from_utf8_lossy(&line);
                    eprintln!("[mlx-stt] {}", msg);
                    if msg.contains("ERROR:") || msg.contains("Error:") {
                        startup_error = Some(msg.trim().to_string());
                    }
                }
                Ok(Some(CommandEvent::Terminated(status))) => {
                    let code = status.code.unwrap_or(-1);
                    if code != 0 {
                        let msg = startup_error
                            .take()
                            .unwrap_or_else(|| format!("STT server exited with code {}", code));
                        return Err(anyhow!("{}", msg));
                    }
                    break;
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => break, // Deadline elapsed — proceed; startup_error handles real failures
            }
        }

        if let Some(err) = startup_error {
            return Err(anyhow!("{}", err));
        }

        // Background monitor
        let monitor_app = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stderr(l) => {
                        eprintln!("[mlx-stt] {}", String::from_utf8_lossy(&l))
                    }
                    CommandEvent::Stdout(l) => {
                        println!("[mlx-stt] {}", String::from_utf8_lossy(&l))
                    }
                    _ => {}
                }
            }
            monitor_app
                .state::<crate::process_tracker::ProcessTracker>()
                .remove_pid(pid);
        });

        // Re-acquire lock to store process
        *self.stt_process.lock().unwrap_or_else(|e| e.into_inner()) = Some(SidecarProcess {
            child: Some(child),
            port,
            token: token.clone(),
            context_size: 0,
            model_family: "mlx-whisper".into(),
        });
        *self
            .stt_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(model_path);

        Ok((port, token))
    }

    /// Resolve a Python script path from backend/scripts/
    #[cfg(feature = "mlx")]
    fn resolve_mlx_script(app: &AppHandle, script_name: &str) -> Result<std::path::PathBuf> {
        // Check resource dir first (production bundle)
        if let Ok(resource_dir) = app.path().resource_dir() {
            let script = resource_dir.join("scripts").join(script_name);
            if script.exists() {
                return Ok(script);
            }
        }

        // Dev mode: backend/scripts/
        if let Ok(cwd) = std::env::current_dir() {
            let script = cwd.join("backend/scripts").join(script_name);
            if script.exists() {
                return Ok(script);
            }
        }

        // Fallback: check CARGO_MANIFEST_DIR (compile-time)
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let script = std::path::PathBuf::from(manifest_dir)
            .join("scripts")
            .join(script_name);
        if script.exists() {
            return Ok(script);
        }

        Err(anyhow!("Cannot find MLX script: {}", script_name))
    }

    pub fn direct_runtime_start_summarizer_server(
        &self,
        app: AppHandle,
        model_path: String,
        context_size: u32,
        n_gpu: i32,
    ) -> Result<(u16, String)> {
        // Get ProcessTracker
        let tracker = app.state::<crate::process_tracker::ProcessTracker>();
        tracker.cleanup_by_service("summarizer");

        let mut process_guard = self
            .summarizer_process
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = process_guard.take() {
            let _ = proc.kill();
        }

        let (port, token) = Self::generate_config(Some(53758));

        let command = app
            .shell()
            .sidecar("llama-server")
            .map_err(|e| anyhow!("Failed to create sidecar command: {}", e))?;

        let args = vec![
            "--model".to_string(),
            model_path.clone(),
            "--ctx-size".to_string(),
            context_size.to_string(),
            "--n-gpu-layers".to_string(),
            n_gpu.to_string(),
            "--host".to_string(),
            "127.0.0.1".to_string(),
            "--port".to_string(),
            port.to_string(),
            "--api-key".to_string(),
            token.clone(),
        ];

        println!("[sidecar-summ] Spawning: llama-server {}", args.join(" "));

        let (mut rx, child) = command
            .args(&args)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn summarizer server: {}", e))?;

        let pid = child.pid();
        tracker.add_pid(pid, "llama-server", "summarizer");

        let monitor_app = app.clone();

        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let CommandEvent::Stderr(line) = event {
                    let msg = String::from_utf8_lossy(&line);
                    eprintln!("[llama-summ] {}", msg);
                }
            }
            // Cleanup
            monitor_app
                .state::<crate::process_tracker::ProcessTracker>()
                .remove_pid(pid);
        });

        *process_guard = Some(SidecarProcess {
            child: Some(child),
            port,
            token: token.clone(),
            context_size,
            model_family: "none".into(),
        });

        Ok((port, token))
    }

    pub fn direct_runtime_start_stt_server(
        &self,
        app: AppHandle,
        model_path: String,
    ) -> Result<(u16, String)> {
        // Get ProcessTracker
        let tracker = app.state::<crate::process_tracker::ProcessTracker>();
        tracker.cleanup_by_service("stt");

        let mut process_guard = self.stt_process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = process_guard.take() {
            let _ = proc.kill();
        }

        let (port, token) = Self::generate_config(Some(53757));

        let mut command = app
            .shell()
            .sidecar("whisper-server")
            .map_err(|e| anyhow!("Failed to create sidecar command: {}", e))?;

        // Resolve bin dir for libraries (DYLD_LIBRARY_PATH on macOS)
        if let Ok(resource_dir) = app.path().resource_dir() {
            let bin_dir = resource_dir.join("bin");
            #[cfg(target_os = "macos")]
            {
                let mut lib_path = bin_dir.to_string_lossy().to_string();

                // Fallback for dev mode
                if let Ok(cwd) = std::env::current_dir() {
                    let dev_bin = cwd.join("backend/bin");
                    if dev_bin.exists() {
                        lib_path = format!("{}:{}", dev_bin.to_string_lossy(), lib_path);
                    }
                }

                println!("[sidecar-stt] Setting DYLD_LIBRARY_PATH: {}", lib_path);
                command = command.env("DYLD_LIBRARY_PATH", lib_path);
            }
        }

        let args = vec![
            "-m".to_string(),
            model_path.clone(),
            "--port".to_string(),
            port.to_string(),
            "--host".to_string(),
            "127.0.0.1".to_string(),
            // No api-key arg for whisper.cpp server usually? checking docs... assuming no for now or custom
            // Actually whisper-server might not support API key unless custom fork.
            // We will omit API key for now.
        ];

        println!("[sidecar-stt] Spawning: whisper-server {}", args.join(" "));

        let (mut rx, child) = command
            .args(&args)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn stt server: {}", e))?;

        let pid = child.pid();
        tracker.add_pid(pid, "whisper-server", "stt");

        let monitor_app = app.clone();

        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let CommandEvent::Stderr(line) = event {
                    let msg = String::from_utf8_lossy(&line);
                    eprintln!("[whisper-stt] {}", msg);
                }
            }
            // Cleanup
            monitor_app
                .state::<crate::process_tracker::ProcessTracker>()
                .remove_pid(pid);
        });

        *process_guard = Some(SidecarProcess {
            child: Some(child),
            port,
            token: token.clone(),
            context_size: 0,
            model_family: "none".into(),
        });

        // Also update model path for legacy check
        *self
            .stt_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(model_path);

        Ok((port, token))
    }

    pub fn direct_runtime_start_image_server(
        &self,
        _app: AppHandle,
        model_path: String,
    ) -> Result<()> {
        let mut model_guard = self
            .image_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *model_guard = Some(model_path);
        Ok(())
    }

    pub fn direct_runtime_start_tts_server(
        &self,
        _app: AppHandle,
        model_path: String,
    ) -> Result<()> {
        let mut model_guard = self
            .tts_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *model_guard = Some(model_path);
        Ok(())
    }

    pub fn direct_runtime_stop_chat_server(&self) -> Result<()> {
        *self
            .is_chat_stop_intentional
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = true;
        let mut process_guard = self.chat_process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = process_guard.take() {
            proc.kill()?;
        }
        Ok(())
    }

    pub fn stop_chat_server(&self) -> Result<()> {
        self.direct_runtime_stop_chat_server()
    }

    pub fn stop_all(&self) -> Result<()> {
        let mut chat = self.chat_process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = chat.take() {
            proc.kill()?;
        }

        let mut embed = self
            .embedding_process
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = embed.take() {
            proc.kill()?;
        }

        let mut summ = self
            .summarizer_process
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = summ.take() {
            proc.kill()?;
        }

        let mut stt = self.stt_process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = stt.take() {
            proc.kill()?;
        }

        // Just clear paths
        *self
            .stt_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        *self
            .image_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        *self
            .tts_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;

        Ok(())
    }
}
