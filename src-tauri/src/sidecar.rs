use anyhow::{anyhow, Result};
use rand::{distributions::Alphanumeric, Rng};
use std::net::TcpListener;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

pub struct SidecarProcess {
    pub child: Option<CommandChild>,
    pub port: u16,
    pub token: String,
    pub context_size: u32,
    pub model_family: String,
}

pub struct ChatServerOptions {
    pub model_path: String,
    pub context_size: u32,
    pub n_gpu: i32,
    pub template: Option<String>,
    pub mmproj: Option<String>,
    pub expose: bool,
    pub mlock: bool,
    pub quantize_kv: bool,
}

impl SidecarProcess {
    pub fn kill(mut self) -> Result<()> {
        if let Some(child) = self.child.take() {
            child
                .kill()
                .map_err(|e| anyhow!("Failed to kill sidecar: {}", e))
        } else {
            Ok(())
        }
    }
}

impl Drop for SidecarProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

use std::sync::Arc;

#[derive(Clone)]
pub struct SidecarManager {
    pub chat_process: Arc<Mutex<Option<SidecarProcess>>>,
    pub embedding_process: Arc<Mutex<Option<SidecarProcess>>>,
    pub summarizer_process: Arc<Mutex<Option<SidecarProcess>>>,
    pub stt_process: Arc<Mutex<Option<SidecarProcess>>>,

    // For CLI tools, we just track if they are "enabled" (model selected)
    // We store the active model path for them.
    pub stt_model_path: Arc<Mutex<Option<String>>>,
    pub image_model_path: Arc<Mutex<Option<String>>>,
    pub tts_model_path: Arc<Mutex<Option<String>>>,
    pub is_chat_stop_intentional: Arc<Mutex<bool>>,
    pub cancellation_token: Arc<AtomicBool>,
    pub generation_lock: Arc<tokio::sync::Mutex<()>>,
    /// Model family detected from GGUF metadata during sidecar startup
    pub detected_model_family: Arc<Mutex<Option<String>>>,
}

impl Default for SidecarManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SidecarManager {
    pub fn new() -> Self {
        Self {
            chat_process: Arc::new(Mutex::new(None)),
            embedding_process: Arc::new(Mutex::new(None)),
            summarizer_process: Arc::new(Mutex::new(None)),
            stt_process: Arc::new(Mutex::new(None)),
            stt_model_path: Arc::new(Mutex::new(None)),
            image_model_path: Arc::new(Mutex::new(None)),
            tts_model_path: Arc::new(Mutex::new(None)),
            is_chat_stop_intentional: Arc::new(Mutex::new(false)),
            cancellation_token: Arc::new(AtomicBool::new(false)),
            generation_lock: Arc::new(tokio::sync::Mutex::new(())),
            detected_model_family: Arc::new(Mutex::new(None)),
        }
    }

    pub fn start_chat_server<F>(
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
        *self.detected_model_family.lock().unwrap_or_else(|e| e.into_inner()) = Some(detected_family.clone());
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
                    "gemma" => None,     // Let llama-server handle Gemma natively (uses 'model' role)
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
            *self.is_chat_stop_intentional.lock().unwrap_or_else(|e| e.into_inner()) = true;
            let _ = proc.kill();
            
            // Wait for port to clear (max 2s)
            for _ in 0..20 {
               if std::net::TcpListener::bind("127.0.0.1:53755").is_ok() {
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
                   let dev_bin = cwd.join("src-tauri/bin");
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
            if expose { "0.0.0.0".to_string() } else { "127.0.0.1".to_string() },
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
        // They are enforced at the API request level via OpenClaw model config (Layer 2 in config.rs).
        println!("[sidecar] Stop tokens for family '{}' will be enforced at API request level", detected_family);

        // Handles MMProj (Vision)
        // Priority: Explicit Override > .mmproj file > Smart Discovery
        let mut found_mmproj = false;

        if let Some(path) = mmproj_path_override {
             if !path.trim().is_empty() {
                 println!("[sidecar] Using explicit mmproj: {}", path);
                 args.push("--mmproj".to_string());
                 args.push(path);
                 found_mmproj = true;
             }
        }

        if !found_mmproj {
            // Check for mmproj file
            let mmproj_path = format!("{}.mmproj", model_path);
            if std::path::Path::new(&mmproj_path).exists() {
                println!("[sidecar] Found mmproj: {}", mmproj_path);
                args.push("--mmproj".to_string());
                args.push(mmproj_path);
                // found_mmproj = true; // Optimization: variable not used after this block
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
                                        break; // Use the first one found
                                    }
                                }
                            }
                        }
                    }
                }
                if !found_mmproj {
                    println!("[sidecar] No mmproj found for: {}", model_path);
                }
            }
        }

        let bind_host = if expose { "0.0.0.0" } else { "127.0.0.1" };
        println!(
            "[sidecar] Spawning chat server: llama-server {} (listening on {}:{})",
            args.join(" "), bind_host, port
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
                                  let end = num_str.find(|c: char| !c.is_numeric() && c != '.').unwrap_or(num_str.len());
                                  if let Ok(val) = num_str[..end].trim().parse::<f32>() {
                                       monitor_app.emit("sidecar_event", SidecarEvent::Progress {
                                            service: "chat".into(),
                                            message: "Reading Context".into(),
                                            progress: val,
                                            total: 1.0
                                       }).ok();
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
                                  let end = num_str.find(|c: char| !c.is_numeric() && c != '.').unwrap_or(num_str.len());
                                  if let Ok(val) = num_str[..end].trim().parse::<f32>() {
                                       monitor_app.emit("sidecar_event", SidecarEvent::Progress {
                                            service: "chat".into(),
                                            message: "Reading Context".into(),
                                            progress: val,
                                            total: 1.0
                                       }).ok();
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
        *self.is_chat_stop_intentional.lock().unwrap_or_else(|e| e.into_inner()) = false;

        Ok((port, token))
    }

    pub fn start_embedding_server(
        &self,
        app: AppHandle,
        model_path: String,
    ) -> Result<(u16, String)> {
        // Get ProcessTracker
        let tracker = app.state::<crate::process_tracker::ProcessTracker>();
        tracker.cleanup_by_service("embedding");

        let mut process_guard = self.embedding_process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = process_guard.take() {
            let _ = proc.kill();
        }

        let (port, token) = Self::generate_config(Some(53756));

        let command = app
            .shell()
            .sidecar("llama-server")
            .map_err(|e| anyhow!("Failed to create sidecar command: {}", e))?;

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

    pub fn start_summarizer_server(
        &self,
        app: AppHandle,
        model_path: String,
        context_size: u32,
        n_gpu: i32,
    ) -> Result<(u16, String)> {
        // Get ProcessTracker
        let tracker = app.state::<crate::process_tracker::ProcessTracker>();
        tracker.cleanup_by_service("summarizer");

        let mut process_guard = self.summarizer_process.lock().unwrap_or_else(|e| e.into_inner());
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

    pub fn start_stt_server(&self, app: AppHandle, model_path: String) -> Result<(u16, String)> {
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
                   let dev_bin = cwd.join("src-tauri/bin");
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
        *self.stt_model_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(model_path);

        Ok((port, token))
    }

    pub fn start_image_server(&self, _app: AppHandle, model_path: String) -> Result<()> {
        let mut model_guard = self.image_model_path.lock().unwrap_or_else(|e| e.into_inner());
        *model_guard = Some(model_path);
        Ok(())
    }

    pub fn start_tts_server(&self, _app: AppHandle, model_path: String) -> Result<()> {
        let mut model_guard = self.tts_model_path.lock().unwrap_or_else(|e| e.into_inner());
        *model_guard = Some(model_path);
        Ok(())
    }

    pub fn stop_chat_server(&self) -> Result<()> {
        *self.is_chat_stop_intentional.lock().unwrap_or_else(|e| e.into_inner()) = true;
        let mut process_guard = self.chat_process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = process_guard.take() {
            proc.kill()?;
        }
        Ok(())
    }

    pub fn stop_all(&self) -> Result<()> {
        let mut chat = self.chat_process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = chat.take() {
            proc.kill()?;
        }

        let mut embed = self.embedding_process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = embed.take() {
            proc.kill()?;
        }

        let mut summ = self.summarizer_process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = summ.take() {
            proc.kill()?;
        }

        let mut stt = self.stt_process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = stt.take() {
            proc.kill()?;
        }

        // Just clear paths
        *self.stt_model_path.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.image_model_path.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.tts_model_path.lock().unwrap_or_else(|e| e.into_inner()) = None;

        Ok(())
    }

    pub fn set_chat_intentional_stop(&self, val: bool) {
        *self.is_chat_stop_intentional.lock().unwrap_or_else(|e| e.into_inner()) = val;
    }

    pub fn get_chat_config(&self) -> Option<(u16, String, u32, String)> {
        let guard = self.chat_process.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|p| (p.port, p.token.clone(), p.context_size, p.model_family.clone()))
    }

    pub fn get_embedding_config(&self) -> Option<(u16, String)> {
        let guard = self.embedding_process.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|p| (p.port, p.token.clone()))
    }

    pub fn get_summarizer_config(&self) -> Option<(u16, String)> {
        let guard = self.summarizer_process.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|p| (p.port, p.token.clone()))
    }

    // No config for CLI, just state
    pub fn is_stt_active(&self) -> bool {
        self.stt_model_path.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    pub fn is_image_active(&self) -> bool {
        self.image_model_path.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    pub fn is_tts_active(&self) -> bool {
        self.tts_model_path.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    pub fn get_stt_model(&self) -> Option<String> {
        self.stt_model_path.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn get_image_model(&self) -> Option<String> {
        self.image_model_path.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn get_tts_model(&self) -> Option<String> {
        self.tts_model_path.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn get_status(&self) -> (bool, bool, bool, bool, bool, bool) {
        let chat = self.chat_process.lock().unwrap_or_else(|e| e.into_inner()).is_some();
        let embed = self.embedding_process.lock().unwrap_or_else(|e| e.into_inner()).is_some();
        let summ = self.summarizer_process.lock().unwrap_or_else(|e| e.into_inner()).is_some();
        let stt = self.stt_process.lock().unwrap_or_else(|e| e.into_inner()).is_some();
        // tts and image still CLI for now? Plan says verify Streaming Voice Services (stt).
        // Let's keep tts as path check for now until implemented.
        let tts = self.tts_model_path.lock().unwrap_or_else(|e| e.into_inner()).is_some();
        let image = self.image_model_path.lock().unwrap_or_else(|e| e.into_inner()).is_some();
        (chat, embed, stt, tts, image, summ)
    }

    fn generate_config(preferred_port: Option<u16>) -> (u16, String) {
        let port = {
            let p = preferred_port.unwrap_or(0);
            if p > 0 && TcpListener::bind(format!("0.0.0.0:{}", p)).is_ok() {
                p
            } else {
                // Fallback to random port
                let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind to random port");
                listener
                    .local_addr()
                    .expect("Failed to get local address")
                    .port()
            }
        };

        let token: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

        (port, token)
    }
}

#[derive(Clone, serde::Serialize, specta::Type)]
#[serde(tag = "type")]
pub enum SidecarEvent {
    Started { service: String },
    Stopped { service: String },
    Crashed { service: String, code: i32 },
    Progress { service: String, message: String, progress: f32, total: f32 },
}

// Commands

use tauri::Emitter;

#[tauri::command]
#[specta::specta]
pub async fn start_chat_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
    context_size: u32,
    template: Option<String>,
    mmproj: Option<String>,
    expose_network: Option<bool>,
    mlock: Option<bool>,
    quantize_kv: Option<bool>,
) -> Result<(), String> {
    // 1. Start Chat Server
    // Clone app handle for the closure
    let app_handle_for_closure = app.clone();

    let (port, _) = state
        .start_chat_server(
            app.clone(),
            ChatServerOptions {
                model_path: model_path.clone(),
                context_size,
                n_gpu: -1,
                template,
                mmproj,
                expose: expose_network.unwrap_or(false),
                mlock: mlock.unwrap_or(false),
                quantize_kv: quantize_kv.unwrap_or(false),
            },
            move |code| {
                // This callback runs when the process terminates
                if code != 0 {
                    // Check if this was intentional
                    let manager = app_handle_for_closure.state::<SidecarManager>();
                    let intentional = *manager.is_chat_stop_intentional.lock().unwrap_or_else(|e| e.into_inner());
                    
                    if intentional {
                         println!("[sidecar] Chat server stopped intentionally (code {}). Suppressing crash alert.", code);
                    } else {
                        eprintln!("[sidecar] Chat server crashed unexpectedly.");
                        
                        // Clear the process from state
                        if let Ok(mut guard) = manager.chat_process.lock() {
                            *guard = None;
                        }

                        // Emit event
                        app_handle_for_closure
                            .emit(
                                "sidecar_event",
                                SidecarEvent::Crashed {
                                    service: "chat".into(),
                                    code,
                                },
                            )
                            .ok();
                    }
                } else {
                    // Clean exit (0) logic
                    let manager = app_handle_for_closure.state::<SidecarManager>();
                    if let Ok(mut guard) = manager.chat_process.lock() {
                        *guard = None;
                    }
                    // Emit stopped event
                    app_handle_for_closure
                        .emit(
                            "sidecar_event",
                            SidecarEvent::Stopped {
                                service: "chat".into(),
                            },
                        )
                        .ok();
                }
            },
        )
        .map_err(|e| e.to_string())?;

    // Wait for server to be ready (poll /health)
    let start = std::time::Instant::now();
    let client = reqwest::Client::new();
    println!("[sidecar] Waiting for chat server to be ready on port {}...", port);
    
    loop {
        if start.elapsed().as_secs() > 120 {
            eprintln!("[sidecar] Timeout waiting for chat server readiness.");
            break;
        }

        // Check if process died
        if state.chat_process.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
            return Err("Chat server process exited prematurely during startup".into());
        }

        match client.get(format!("http://127.0.0.1:{}/health", port)).send().await {
            Ok(res) => {
                if res.status().is_success() {
                    println!("[sidecar] Chat server is ready!");
                    break;
                }
                // 503 means loading...
            }
            Err(_) => {
                // Connection refused...
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    app.emit(
        "sidecar_event",
        SidecarEvent::Started {
            service: "chat".into(),
        },
    )
    .ok();

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn start_embedding_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
) -> Result<(), String> {
    let res = state
        .start_embedding_server(app.clone(), model_path)
        .map(|_| ())
        .map_err(|e| e.to_string());

    if res.is_ok() {
        app.emit(
            "sidecar_event",
            SidecarEvent::Started {
                service: "embedding".into(),
            },
        )
        .ok();
    }

    res
}

#[tauri::command]
#[specta::specta]
pub async fn start_summarizer_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
    context_size: u32,
) -> Result<(), String> {
    let res = state
        .start_summarizer_server(app.clone(), model_path, context_size, -1)
        .map(|_| ())
        .map_err(|e| e.to_string());

    if res.is_ok() {
        app.emit(
            "sidecar_event",
            SidecarEvent::Started {
                service: "summarizer".into(),
            },
        )
        .ok();
    }

    res
}

#[tauri::command]
#[specta::specta]
pub async fn start_stt_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
) -> Result<(), String> {
    let res = state
        .start_stt_server(app.clone(), model_path)
        .map(|_| ())
        .map_err(|e| e.to_string());

    if res.is_ok() {
        app.emit(
            "sidecar_event",
            SidecarEvent::Started {
                service: "stt".into(),
            },
        )
        .ok();
    }

    res
}

#[tauri::command]
#[specta::specta]
pub async fn start_image_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
) -> Result<(), String> {
    state
        .start_image_server(app, model_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn start_tts_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
) -> Result<(), String> {
    state
        .start_tts_server(app, model_path)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

use specta::Type;

#[derive(Debug, Clone, serde::Serialize, Type)]
pub struct SidecarStatus {
    chat_running: bool,
    embedding_running: bool,
    stt_running: bool,
    tts_running: bool,
    image_running: bool,
    summarizer_running: bool,
}

#[derive(Debug, Clone, serde::Serialize, Type)]
pub struct ChatServerConfig {
    pub port: u16,
    pub token: String,
    pub context_size: u32,
    pub model_family: String,
}

#[tauri::command]
#[specta::specta]
pub fn get_chat_server_config(state: State<'_, SidecarManager>) -> Option<ChatServerConfig> {
    state.get_chat_config().map(|(port, token, context_size, model_family)| ChatServerConfig {
        port,
        token,
        context_size,
        model_family,
    })
}

#[tauri::command]
#[specta::specta]
pub fn get_sidecar_status(state: State<'_, SidecarManager>) -> SidecarStatus {
    let (chat, embed, stt, tts, image, summ) = state.get_status();
    SidecarStatus {
        chat_running: chat,
        embedding_running: embed,
        stt_running: stt,
        tts_running: tts,
        image_running: image,
        summarizer_running: summ,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn stop_chat_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    _model_path: String,
) -> Result<(), String> {
    state.stop_all().map_err(|e| e.to_string())?;
    app.emit(
        "sidecar_event",
        SidecarEvent::Stopped {
            service: "chat".into(),
        },
    )
    .ok();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_generation(state: State<'_, SidecarManager>) -> Result<(), String> {
    state.cancellation_token.store(true, Ordering::SeqCst);
    Ok(())
}
