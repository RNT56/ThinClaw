//! Sidecar process lifecycle: spawning the chat/embedding/summarizer/stt
//! servers (native llama.cpp/whisper + optional MLX Python backends), the
//! CLI-tool path trackers (image/tts), and the stop/teardown methods.

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

use super::core::SidecarManager;
use super::types::{ChatServerOptions, SidecarChild, SidecarEvent, SidecarProcess};

impl SidecarManager {
    fn resolve_inventory_model(
        app: &AppHandle,
        model_path: &str,
        runtime: &str,
        category: &str,
    ) -> Result<crate::model_manager::ResolvedInventoryModel> {
        crate::model_manager::resolve_compatible_inventory_model(app, model_path, runtime, category)
            .map_err(anyhow::Error::msg)
    }

    fn inventory_path_string(path: &std::path::Path, purpose: &str) -> Result<String> {
        path.to_str()
            .ok_or_else(|| anyhow!("The selected {purpose} model path is not valid UTF-8"))
            .map(str::to_string)
    }

    fn select_inventory_projector(
        declared: Option<&std::path::Path>,
        requested: Option<&str>,
    ) -> Result<Option<String>> {
        let requested = requested
            .filter(|path| !path.trim().is_empty())
            .map(|path| Self::validate_projector_path(std::path::Path::new(path), true))
            .transpose()?
            .map(std::path::PathBuf::from);
        if requested.is_some() && requested.as_deref() != declared {
            return Err(anyhow!(
                "The selected vision projector is not the companion declared by this model installation"
            ));
        }
        declared
            .map(|path| Self::inventory_path_string(path, "vision projector"))
            .transpose()
    }

    pub(crate) fn validate_managed_model_path(
        app: &AppHandle,
        model_path: &str,
        category: &str,
        purpose: &str,
        allow_directory: bool,
        allowed_extensions: &[&str],
    ) -> Result<String> {
        if model_path.is_empty()
            || model_path.len() > 4_096
            || model_path.chars().any(char::is_control)
        {
            return Err(anyhow!("The selected {purpose} model path is invalid"));
        }
        let path = std::path::Path::new(model_path);
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| anyhow!("Could not inspect the selected {purpose} model: {error}"))?;
        if metadata.file_type().is_symlink()
            || !(metadata.is_file() || allow_directory && metadata.is_dir())
            || (metadata.is_file() && metadata.len() == 0)
        {
            return Err(anyhow!(
                "The selected {purpose} model must be a real, non-empty managed artifact"
            ));
        }

        let managed_root = app
            .path()
            .app_data_dir()
            .map_err(|error| anyhow!("Could not resolve managed model storage: {error}"))?
            .join("models")
            .join(category);
        let root_metadata = std::fs::symlink_metadata(&managed_root)
            .map_err(|error| anyhow!("Managed {purpose} model storage is unavailable: {error}"))?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(anyhow!(
                "Managed {purpose} model storage is not a real directory"
            ));
        }
        let managed_root = managed_root
            .canonicalize()
            .map_err(|error| anyhow!("Could not resolve managed {purpose} storage: {error}"))?;
        let resolved = path
            .canonicalize()
            .map_err(|error| anyhow!("Could not resolve the selected {purpose} model: {error}"))?;
        if resolved == managed_root || !resolved.starts_with(&managed_root) {
            return Err(anyhow!(
                "The selected {purpose} model is outside managed model storage"
            ));
        }
        if metadata.is_file()
            && !resolved
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    allowed_extensions
                        .iter()
                        .any(|allowed| extension.eq_ignore_ascii_case(allowed))
                })
        {
            return Err(anyhow!(
                "The selected {purpose} model has an unsupported file type"
            ));
        }
        resolved
            .to_str()
            .ok_or_else(|| anyhow!("The selected {purpose} model path is not valid UTF-8"))
            .map(str::to_string)
    }

    fn model_artifact_identity(path: &std::path::Path) -> Result<String> {
        let path = path
            .canonicalize()
            .map_err(|error| anyhow!("Could not resolve model artifact: {error}"))?;
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| anyhow!("Could not inspect model artifact: {error}"))?;
        if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(anyhow!("Model artifact must be a real file or directory"));
        }

        fn hash_metadata(hasher: &mut Sha256, metadata: &std::fs::Metadata) {
            hasher.update(metadata.len().to_le_bytes());
            if let Ok(modified) = metadata.modified() {
                if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                    hasher.update(duration.as_secs().to_le_bytes());
                    hasher.update(duration.subsec_nanos().to_le_bytes());
                }
            }
        }

        let mut hasher = Sha256::new();
        hasher.update(path.to_string_lossy().as_bytes());
        hash_metadata(&mut hasher, &metadata);
        if metadata.is_file() {
            let mut file = std::fs::File::open(&path)
                .map_err(|error| anyhow!("Could not read model artifact: {error}"))?;
            let mut sample = vec![0_u8; 64 * 1024];
            let first = file
                .read(&mut sample)
                .map_err(|error| anyhow!("Could not fingerprint model artifact: {error}"))?;
            hasher.update(&sample[..first]);
            if metadata.len() > sample.len() as u64 {
                file.seek(SeekFrom::End(-(sample.len() as i64)))
                    .map_err(|error| anyhow!("Could not fingerprint model artifact: {error}"))?;
                let last = file
                    .read(&mut sample)
                    .map_err(|error| anyhow!("Could not fingerprint model artifact: {error}"))?;
                hasher.update(&sample[..last]);
            }
        } else {
            let directory = std::fs::read_dir(&path)
                .map_err(|error| anyhow!("Could not list model artifact directory: {error}"))?;
            let mut entries = Vec::new();
            for entry in directory {
                if entries.len() >= 512 {
                    return Err(anyhow!(
                        "Model artifact directory contains too many entries"
                    ));
                }
                entries.push(entry.map_err(|error| {
                    anyhow!("Could not inspect model artifact directory: {error}")
                })?);
            }
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                let entry_metadata = std::fs::symlink_metadata(entry.path())
                    .map_err(|error| anyhow!("Could not inspect model artifact entry: {error}"))?;
                if entry_metadata.file_type().is_symlink() {
                    return Err(anyhow!("Model artifact directory contains a symlink"));
                }
                hasher.update(entry.file_name().to_string_lossy().as_bytes());
                hasher.update([u8::from(entry_metadata.is_dir())]);
                hash_metadata(&mut hasher, &entry_metadata);
                if entry.file_name() == "config.json" && entry_metadata.is_file() {
                    let config = thinclaw_platform::read_regular_file_bounded_single_link(
                        &entry.path(),
                        1024 * 1024,
                    )
                    .map_err(|error| anyhow!("Could not read model config: {error}"))?;
                    hasher.update(config);
                }
            }
        }
        Ok(hex::encode(hasher.finalize()))
    }

    fn validate_gguf_model_path(
        model_path: String,
        purpose: &str,
    ) -> Result<(String, crate::gguf::GGUFMetadata)> {
        if model_path.is_empty() || model_path.len() > 4096 {
            return Err(anyhow!(
                "Selected {purpose} model path is empty or too long"
            ));
        }
        let path = std::path::PathBuf::from(model_path);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| anyhow!("Could not inspect selected {purpose} model: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
            return Err(anyhow!(
                "Selected {purpose} model must be a non-empty regular, non-symlink file"
            ));
        }
        let gguf_metadata = crate::gguf::read_gguf_metadata(
            path.to_str()
                .ok_or_else(|| anyhow!("Selected {purpose} model path is not valid UTF-8"))?,
        )
        .map_err(|error| anyhow!("Selected {purpose} model is not a valid GGUF file: {error}"))?;
        let resolved = path
            .canonicalize()
            .map_err(|error| anyhow!("Could not resolve selected {purpose} model: {error}"))?
            .to_str()
            .ok_or_else(|| anyhow!("Resolved {purpose} model path is not valid UTF-8"))
            .map(str::to_string)?;
        Ok((resolved, gguf_metadata))
    }

    fn validate_projector_path(path: &std::path::Path, explicit: bool) -> Result<String> {
        let metadata = std::fs::symlink_metadata(path).map_err(|error| {
            if explicit {
                anyhow!("Could not inspect the selected vision projector: {error}")
            } else {
                anyhow!("Vision projector candidate is unavailable")
            }
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
            return Err(anyhow!(
                "Vision projector must be a non-empty regular, non-symlink file"
            ));
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(extension.as_deref(), Some("gguf" | "bin" | "mmproj")) {
            return Err(anyhow!("Vision projector has an unsupported file type"));
        }
        path.canonicalize()
            .map_err(|error| anyhow!("Could not resolve the vision projector: {error}"))?
            .to_str()
            .ok_or_else(|| anyhow!("Vision projector path is not valid UTF-8"))
            .map(str::to_string)
    }

    fn ensure_private_directory(path: &std::path::Path, label: &str) -> Result<()> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(anyhow!("{label} path is not a real directory"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir_all(path)
                    .map_err(|error| anyhow!("Could not create {label} directory: {error}"))?;
                let metadata = std::fs::symlink_metadata(path)
                    .map_err(|error| anyhow!("Could not inspect {label} directory: {error}"))?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(anyhow!("{label} path is not a real directory"));
                }
            }
            Err(error) => {
                return Err(anyhow!("Could not inspect {label} directory: {error}"));
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| anyhow!("Could not secure {label} directory: {error}"))?;
        }
        Ok(())
    }

    fn current_target_triple() -> Result<&'static str> {
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Ok("aarch64-apple-darwin")
        } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
            Ok("x86_64-apple-darwin")
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Ok("x86_64-unknown-linux-gnu")
        } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            Ok("x86_64-pc-windows-msvc")
        } else {
            Err(anyhow!(
                "This build has no reviewed llama.cpp runtime target"
            ))
        }
    }

    fn hash_regular_runtime_file(path: &std::path::Path) -> Result<String> {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| anyhow!("Could not inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
            return Err(anyhow!(
                "llama.cpp runtime file must be a non-empty regular file: {}",
                path.display()
            ));
        }
        let mut file = std::fs::File::open(path)
            .map_err(|error| anyhow!("Could not open {}: {error}", path.display()))?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| anyhow!("Could not read {}: {error}", path.display()))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hex::encode(hasher.finalize()))
    }

    /// Resolve a complete, manifest-backed llama.cpp runtime. Release bundles
    /// trust the platform/app signature for payload integrity because signing
    /// mutates Mach-O/PE files after the source manifest is generated. Debug
    /// builds additionally compare the source payload hashes.
    fn sidecar_library_path(app: &AppHandle) -> Result<String> {
        let target = Self::current_target_triple()?;
        let manifest_name = format!("llama-runtime-{target}.json");
        let mut candidates = Vec::new();
        if let Ok(resource_dir) = app.path().resource_dir() {
            candidates.push(resource_dir.join("bin"));
        }
        if cfg!(debug_assertions) {
            candidates.push(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin"));
        }

        for candidate in candidates {
            let Ok(metadata) = std::fs::symlink_metadata(&candidate) else {
                continue;
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            let resolved = candidate
                .canonicalize()
                .map_err(|error| anyhow!("Could not resolve sidecar library directory: {error}"))?;
            let manifest_path = resolved.join(&manifest_name);
            let Ok(manifest_bytes) =
                thinclaw_platform::read_regular_file_bounded_single_link(&manifest_path, 64 * 1024)
            else {
                continue;
            };
            let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
                .map_err(|error| anyhow!("Invalid llama.cpp runtime manifest: {error}"))?;
            if manifest
                .get("schemaVersion")
                .and_then(|value| value.as_u64())
                != Some(1)
                || manifest.get("engine").and_then(|value| value.as_str()) != Some("llamacpp")
                || manifest.get("version").and_then(|value| value.as_str())
                    != Some(env!("THINCLAW_LLAMA_CPP_VERSION"))
                || manifest.get("target").and_then(|value| value.as_str()) != Some(target)
            {
                return Err(anyhow!(
                    "Bundled llama.cpp runtime manifest does not match this build"
                ));
            }
            let libraries = manifest
                .get("libraries")
                .and_then(|value| value.as_object())
                .ok_or_else(|| anyhow!("llama.cpp runtime manifest has no library inventory"))?;
            if cfg!(debug_assertions) {
                let server_name = format!(
                    "llama-server-{target}{}",
                    if cfg!(windows) { ".exe" } else { "" }
                );
                let actual = Self::hash_regular_runtime_file(&resolved.join(server_name))?;
                if manifest
                    .get("serverSha256")
                    .and_then(|value| value.as_str())
                    != Some(actual.as_str())
                {
                    return Err(anyhow!("llama.cpp server hash does not match its manifest"));
                }
            }
            for (name, expected_hash) in libraries {
                if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
                    return Err(anyhow!(
                        "llama.cpp runtime manifest contains an invalid path"
                    ));
                }
                let library = resolved.join(name);
                if cfg!(debug_assertions) {
                    let actual = Self::hash_regular_runtime_file(&library)?;
                    if expected_hash.as_str() != Some(&actual) {
                        return Err(anyhow!("llama.cpp runtime library hash mismatch: {name}"));
                    }
                } else {
                    let metadata = std::fs::symlink_metadata(&library).map_err(|error| {
                        anyhow!("Bundled llama.cpp library {name} is unavailable: {error}")
                    })?;
                    if metadata.file_type().is_symlink()
                        || !metadata.is_file()
                        || metadata.len() == 0
                    {
                        return Err(anyhow!("Bundled llama.cpp library is invalid: {name}"));
                    }
                }
            }
            return resolved
                .to_str()
                .ok_or_else(|| anyhow!("Sidecar library directory is not valid UTF-8"))
                .map(str::to_string);
        }
        Err(anyhow!(
            "Bundled llama.cpp runtime is incomplete or does not match version {}. Reinstall or repair this ThinClaw build.",
            env!("THINCLAW_LLAMA_CPP_VERSION")
        ))
    }

    pub fn direct_runtime_start_chat_server<F>(
        &self,
        app: AppHandle,
        options: ChatServerOptions,
        on_exit: F,
    ) -> Result<(u16, String)>
    where
        F: Fn(i32, u16, uuid::Uuid) + Send + Sync + 'static,
    {
        let resolved = Self::resolve_inventory_model(&app, &options.model_path, "llamacpp", "LLM")?;
        let model_path = Self::inventory_path_string(&resolved.path, "chat")?;
        let mmproj_path_override = Self::select_inventory_projector(
            resolved.companion_path.as_deref(),
            options.mmproj.as_deref(),
        )?;
        let (model_path, gguf_meta) = Self::validate_gguf_model_path(model_path, "chat")?;
        let context_size = options.context_size;
        if context_size == 0 || context_size > 1_048_576 {
            return Err(anyhow!(
                "Chat context size must be between 1 and 1,048,576 tokens"
            ));
        }
        let n_gpu = options.n_gpu;
        let template_name = options.template;
        let expose = options.expose;
        if expose {
            return Err(anyhow!(
                "Direct model-server network exposure is disabled; use the authenticated gateway"
            ));
        }
        let mlock = options.mlock;
        let quantize_kv = options.quantize_kv;

        // Resolve Template + detect model family from GGUF metadata
        let detected_family = gguf_meta
            .model_family
            .clone()
            .unwrap_or_else(|| "chatml".to_string());

        println!("[sidecar] Detected model family: {}", detected_family);
        *self
            .detected_model_family
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(detected_family.clone());
        if let Some(ref template) = gguf_meta.chat_template {
            println!(
                "[sidecar] GGUF chat_template present ({} chars)",
                template.len()
            );
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

        let (port, token) = Self::generate_config(Some(53755))?;

        // Resolve cache path
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| anyhow!("Failed to resolve app data directory: {error}"))?;
        Self::ensure_private_directory(&app_data_dir, "app data")?;
        let cache_dir = app_data_dir.join("prompt_cache");
        Self::ensure_private_directory(&cache_dir, "prompt cache")?;
        let cache_path_str = cache_dir
            .to_str()
            .ok_or_else(|| anyhow!("Prompt cache path is not valid UTF-8"))?
            .to_string();

        let command = app
            .shell()
            .sidecar("llama-server")
            .map_err(|e| anyhow!("Failed to create sidecar command: {}", e))?;

        let library_path = Self::sidecar_library_path(&app)?;
        #[cfg(target_os = "macos")]
        let command = command.env("DYLD_LIBRARY_PATH", library_path);
        #[cfg(target_os = "linux")]
        let command = command.env("LD_LIBRARY_PATH", library_path);
        #[cfg(target_os = "windows")]
        let command = {
            let mut paths = vec![std::path::PathBuf::from(library_path)];
            if let Some(existing) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&existing));
            }
            let path = std::env::join_paths(paths)
                .map_err(|error| anyhow!("Could not construct llama.cpp loader PATH: {error}"))?;
            command.env("PATH", path.to_string_lossy())
        };

        let mut args = vec![
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
            "--alias".to_string(),
            "default".to_string(),
            "--slot-save-path".to_string(),
            cache_path_str,
            "--no-webui".to_string(),
            "--log-disable".to_string(),
            "--timeout".to_string(),
            "300".to_string(),
            "--threads-http".to_string(),
            "4".to_string(),
            "--flash-attn".to_string(),
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
            args.push("--jinja".to_string());
            args.push("--chat-template".to_string());
            args.push(t.to_string());
        }

        // NOTE: Stop tokens are NOT injected as CLI args (llama-server doesn't support --stop).
        // They are enforced at the API request level via ThinClaw model config (Layer 2 in config.rs).
        println!(
            "[sidecar] Stop tokens for family '{}' will be enforced at API request level",
            detected_family
        );

        // Only a manifest-declared projector may be attached. The command
        // wrapper resolves an omitted selection to that companion and rejects
        // an explicit path that differs, so adjacent-file discovery cannot
        // silently cross logical inventory installations.
        if let Some(path) = mmproj_path_override {
            println!("[sidecar] Using the inventory-declared vision projector");
            args.push("--mmproj".to_string());
            args.push(path);
        }

        let bind_host = "127.0.0.1";
        println!(
            "[sidecar] Spawning authenticated chat server (listening on {}:{})",
            bind_host, port
        );

        let (mut rx, child) = command
            .args(&args)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn llama-server: {}", e))?;

        let pid = child.pid();
        let instance_id = uuid::Uuid::new_v4();
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
                            on_exit(code, port, instance_id);
                        } else {
                            // Terminated without code (signal?)
                            on_exit(-1, port, instance_id);
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
            instance_id,
            child: Some(SidecarChild::Plugin(child)),
            port,
            token: token.clone(),
            context_size,
            model_family: detected_family.clone(),
            model_path: std::path::PathBuf::from(&model_path),
            model_identity: None,
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
        let resolved = Self::resolve_inventory_model(&app, &model_path, "llamacpp", "Embedding")?;
        let model_path = Self::inventory_path_string(&resolved.path, "embedding")?;
        let (model_path, _) = Self::validate_gguf_model_path(model_path, "embedding")?;
        let model_identity = Self::model_artifact_identity(std::path::Path::new(&model_path))?;
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

        let (port, token) = Self::generate_config(Some(53756))?;

        let command = app
            .shell()
            .sidecar("llama-server")
            .map_err(|e| anyhow!("Failed to create sidecar command: {}", e))?;

        let library_path = Self::sidecar_library_path(&app)?;
        #[cfg(target_os = "macos")]
        let command = command.env("DYLD_LIBRARY_PATH", library_path);
        #[cfg(target_os = "linux")]
        let command = command.env("LD_LIBRARY_PATH", library_path);
        #[cfg(target_os = "windows")]
        let command = {
            let mut paths = vec![std::path::PathBuf::from(library_path)];
            if let Some(existing) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&existing));
            }
            let path = std::env::join_paths(paths)
                .map_err(|error| anyhow!("Could not construct llama.cpp loader PATH: {error}"))?;
            command.env("PATH", path.to_string_lossy())
        };

        let args = vec![
            "--model".to_string(),
            model_path.clone(),
            "--embedding".to_string(),
            "--host".to_string(),
            "127.0.0.1".to_string(),
            "--port".to_string(),
            port.to_string(),
            "--api-key".to_string(),
            token.clone(),
            "--alias".to_string(),
            "thinclaw-embedding".to_string(),
            "--no-webui".to_string(),
            "--log-disable".to_string(),
            "--timeout".to_string(),
            "300".to_string(),
            "--threads-http".to_string(),
            "4".to_string(),
            "--ctx-size".to_string(),
            "4096".to_string(),
            "--batch-size".to_string(),
            "512".to_string(),
            "--ubatch-size".to_string(),
            "512".to_string(),
            "--n-gpu-layers".to_string(),
            "0".to_string(),
        ];

        println!("[sidecar-embed] Spawning authenticated embedding server on port {port}");

        let (mut rx, child) = command
            .args(&args)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn embedding server: {}", e))?;

        let pid = child.pid();
        let instance_id = uuid::Uuid::new_v4();
        tracker.add_pid(pid, "llama-server", "embedding");

        let monitor_app = app.clone();

        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stderr(line) => {
                        let msg = String::from_utf8_lossy(&line);
                        eprintln!("[llama-embed] {}", msg);
                    }
                    CommandEvent::Terminated(payload) => {
                        let code = payload.code.unwrap_or(-1);
                        let manager = monitor_app.state::<SidecarManager>();
                        if SidecarManager::clear_process_instance(
                            &manager.embedding_process,
                            instance_id,
                        ) {
                            let event = if code == 0 {
                                SidecarEvent::Stopped {
                                    service: "embedding".into(),
                                }
                            } else {
                                SidecarEvent::Crashed {
                                    service: "embedding".into(),
                                    code,
                                }
                            };
                            let _ = monitor_app.emit("sidecar_event", event);
                        }
                        break;
                    }
                    _ => {}
                }
            }
            let manager = monitor_app.state::<SidecarManager>();
            if SidecarManager::clear_process_instance(&manager.embedding_process, instance_id) {
                let _ = monitor_app.emit(
                    "sidecar_event",
                    SidecarEvent::Crashed {
                        service: "embedding".into(),
                        code: -1,
                    },
                );
            }
            // Cleanup
            monitor_app
                .state::<crate::process_tracker::ProcessTracker>()
                .remove_pid(pid);
        });

        *process_guard = Some(SidecarProcess {
            instance_id,
            child: Some(SidecarChild::Plugin(child)),
            port,
            token: token.clone(),
            context_size: 4096, // Fixed for embedding
            model_family: "none".into(),
            model_path: std::path::PathBuf::from(&model_path),
            model_identity: Some(model_identity),
        });

        Ok((port, token))
    }

    #[cfg(feature = "mlx")]
    pub async fn start_mlx_embedding_server(
        &self,
        app: AppHandle,
        model_path: String,
    ) -> Result<(u16, String)> {
        let resolved = Self::resolve_inventory_model(&app, &model_path, "mlx", "Embedding")?;
        let model_path = Self::inventory_path_string(&resolved.path, "embedding")?;
        let tracker = app.state::<crate::process_tracker::ProcessTracker>();
        {
            let mut process_guard = self
                .embedding_process
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(proc) = process_guard.take() {
                let _ = proc.kill();
            }
        }
        tracker.cleanup_by_service("embedding");

        let process = Self::spawn_mlx_python_service(
            &app,
            model_path,
            "embeddings",
            "thinclaw-embedding",
            "embedding",
            "mlx-embedding",
            53756,
            4096,
        )
        .await?;
        let port = process.port;
        let token = process.token.clone();
        *self
            .embedding_process
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(process);
        Ok((port, token))
    }

    #[cfg(feature = "mlx")]
    pub async fn start_mlx_stt_server(
        &self,
        app: AppHandle,
        model_path: String,
    ) -> Result<(u16, String)> {
        let resolved = Self::resolve_inventory_model(&app, &model_path, "mlx", "STT")?;
        let model_path = Self::inventory_path_string(&resolved.path, "STT")?;
        let tracker = app.state::<crate::process_tracker::ProcessTracker>();
        {
            let mut process_guard = self.stt_process.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(process) = process_guard.as_mut() {
                if process.model_path == resolved.path && process.is_running() {
                    return Ok((process.port, process.token.clone()));
                }
            }
            if let Some(proc) = process_guard.take() {
                let _ = proc.kill();
            }
        }
        tracker.cleanup_by_service("stt");

        let process = Self::spawn_mlx_python_service(
            &app,
            model_path.clone(),
            "whisper",
            "thinclaw-whisper",
            "stt",
            "mlx-whisper",
            53757,
            0,
        )
        .await?;
        let port = process.port;
        let token = process.token.clone();
        let canonical_model_path = process
            .model_path
            .to_str()
            .ok_or_else(|| anyhow!("MLX STT model path is not valid UTF-8"))?
            .to_string();
        *self.stt_process.lock().unwrap_or_else(|e| e.into_inner()) = Some(process);
        *self
            .stt_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(canonical_model_path);

        Ok((port, token))
    }

    #[cfg(feature = "mlx")]
    #[allow(clippy::too_many_arguments)]
    async fn spawn_mlx_python_service(
        app: &AppHandle,
        model_path: String,
        model_type: &'static str,
        served_model_name: &'static str,
        service: &'static str,
        model_family: &'static str,
        preferred_port: u16,
        context_size: u32,
    ) -> Result<SidecarProcess> {
        use std::process::Stdio;
        use std::time::Duration;
        use thinclaw_platform::OwnedChild;

        if model_path.is_empty() || model_path.len() > 4096 {
            return Err(anyhow!("MLX model path is empty or too long"));
        }
        let model_path = std::path::PathBuf::from(model_path);
        let model_metadata = std::fs::symlink_metadata(&model_path)
            .map_err(|error| anyhow!("Could not inspect MLX model directory: {error}"))?;
        if model_metadata.file_type().is_symlink() || !model_metadata.is_dir() {
            return Err(anyhow!("MLX model path must be a real local directory"));
        }
        let config = model_path.join("config.json");
        let config_metadata = std::fs::symlink_metadata(&config)
            .map_err(|error| anyhow!("Could not inspect MLX model config: {error}"))?;
        if config_metadata.file_type().is_symlink()
            || !config_metadata.is_file()
            || config_metadata.len() > 1024 * 1024
        {
            return Err(anyhow!("MLX model config must be a bounded regular file"));
        }
        let model_path = model_path
            .canonicalize()
            .map_err(|error| anyhow!("Could not resolve MLX model path: {error}"))?;
        let model_path = model_path
            .to_str()
            .ok_or_else(|| anyhow!("MLX model path is not valid UTF-8"))?
            .to_string();
        let model_identity = Self::model_artifact_identity(std::path::Path::new(&model_path))?;

        let app_data = app
            .path()
            .app_data_dir()
            .map_err(|error| anyhow!("Failed to resolve app data directory: {error}"))?;
        let engine = crate::engine::engine_mlx::MlxEngine::new();
        engine.set_app_data_dir(app_data.clone());
        if !engine.is_bootstrapped() {
            return Err(anyhow!(
                "MLX environment is incomplete or does not match the pinned version"
            ));
        }
        let server_path = app_data.join("mlx-env/bin/mlx-openai-server");
        let server_metadata = std::fs::symlink_metadata(&server_path)
            .map_err(|error| anyhow!("MLX server executable is unavailable: {error}"))?;
        if server_metadata.file_type().is_symlink() || !server_metadata.is_file() {
            return Err(anyhow!("MLX server executable is not a regular file"));
        }

        let (port, token) = Self::generate_config(Some(preferred_port))?;
        let port_arg = port.to_string();
        let mut command = tokio::process::Command::new(&server_path);
        command
            .args([
                "launch",
                "--model-path",
                &model_path,
                "--model-type",
                model_type,
                "--served-model-name",
                served_model_name,
                "--port",
                &port_arg,
                "--host",
                "127.0.0.1",
                "--queue-size",
                "8",
                "--no-log-file",
                "--log-level",
                "ERROR",
            ])
            .env_remove("PYTHONPATH")
            .env_remove("PYTHONHOME")
            .env_remove("VIRTUAL_ENV")
            .env("PYTHONNOUSERSITE", "1")
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("THINCLAW_MLX_API_KEY", &token)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = OwnedChild::spawn(&mut command)
            .map_err(|error| anyhow!("Failed to spawn MLX {service} server: {error}"))?;

        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|error| anyhow!("Could not build local MLX client: {error}"))?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3 * 60);
        loop {
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill().await;
                return Err(anyhow!(
                    "MLX {service} startup exceeded its 3-minute deadline"
                ));
            }
            if let Some(status) = child
                .try_wait()
                .map_err(|error| anyhow!("Could not inspect MLX {service} process: {error}"))?
            {
                return Err(anyhow!(
                    "MLX {service} server exited during startup with code {:?}",
                    status.code()
                ));
            }
            if client
                .get(format!("http://127.0.0.1:{port}/v1/models"))
                .bearer_auth(&token)
                .send()
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        let pid = child
            .id()
            .ok_or_else(|| anyhow!("MLX {service} process has no PID"))?;
        app.state::<crate::process_tracker::ProcessTracker>()
            .add_pid(pid, &format!("mlx-{service}-server"), service);
        Ok(SidecarProcess {
            instance_id: uuid::Uuid::new_v4(),
            child: Some(SidecarChild::Owned(child)),
            port,
            token,
            context_size,
            model_family: model_family.to_string(),
            model_path: std::path::PathBuf::from(&model_path),
            model_identity: Some(model_identity),
        })
    }

    pub fn direct_runtime_start_summarizer_server(
        &self,
        app: AppHandle,
        model_path: String,
        context_size: u32,
        n_gpu: i32,
    ) -> Result<(u16, String)> {
        let resolved = Self::resolve_inventory_model(&app, &model_path, "llamacpp", "LLM")?;
        let model_path = Self::inventory_path_string(&resolved.path, "summarizer")?;
        let (model_path, _) = Self::validate_gguf_model_path(model_path, "summarizer")?;
        if context_size == 0 || context_size > 1_048_576 {
            return Err(anyhow!(
                "Summarizer context size must be between 1 and 1,048,576 tokens"
            ));
        }
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

        let (port, token) = Self::generate_config(Some(53758))?;

        let command = app
            .shell()
            .sidecar("llama-server")
            .map_err(|e| anyhow!("Failed to create sidecar command: {}", e))?;

        let library_path = Self::sidecar_library_path(&app)?;
        #[cfg(target_os = "macos")]
        let command = command.env("DYLD_LIBRARY_PATH", library_path);
        #[cfg(target_os = "linux")]
        let command = command.env("LD_LIBRARY_PATH", library_path);
        #[cfg(target_os = "windows")]
        let command = {
            let mut paths = vec![std::path::PathBuf::from(library_path)];
            if let Some(existing) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&existing));
            }
            let path = std::env::join_paths(paths)
                .map_err(|error| anyhow!("Could not construct llama.cpp loader PATH: {error}"))?;
            command.env("PATH", path.to_string_lossy())
        };

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
            "--alias".to_string(),
            "thinclaw-summarizer".to_string(),
            "--no-webui".to_string(),
            "--log-disable".to_string(),
            "--timeout".to_string(),
            "300".to_string(),
            "--threads-http".to_string(),
            "4".to_string(),
        ];

        println!("[sidecar-summ] Spawning authenticated summarizer server on port {port}");

        let (mut rx, child) = command
            .args(&args)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn summarizer server: {}", e))?;

        let pid = child.pid();
        let instance_id = uuid::Uuid::new_v4();
        tracker.add_pid(pid, "llama-server", "summarizer");

        let monitor_app = app.clone();

        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stderr(line) => {
                        let msg = String::from_utf8_lossy(&line);
                        eprintln!("[llama-summ] {}", msg);
                    }
                    CommandEvent::Terminated(payload) => {
                        let code = payload.code.unwrap_or(-1);
                        let manager = monitor_app.state::<SidecarManager>();
                        if SidecarManager::clear_process_instance(
                            &manager.summarizer_process,
                            instance_id,
                        ) {
                            let event = if code == 0 {
                                SidecarEvent::Stopped {
                                    service: "summarizer".into(),
                                }
                            } else {
                                SidecarEvent::Crashed {
                                    service: "summarizer".into(),
                                    code,
                                }
                            };
                            let _ = monitor_app.emit("sidecar_event", event);
                        }
                        break;
                    }
                    _ => {}
                }
            }
            let manager = monitor_app.state::<SidecarManager>();
            if SidecarManager::clear_process_instance(&manager.summarizer_process, instance_id) {
                let _ = monitor_app.emit(
                    "sidecar_event",
                    SidecarEvent::Crashed {
                        service: "summarizer".into(),
                        code: -1,
                    },
                );
            }
            // Cleanup
            monitor_app
                .state::<crate::process_tracker::ProcessTracker>()
                .remove_pid(pid);
        });

        *process_guard = Some(SidecarProcess {
            instance_id,
            child: Some(SidecarChild::Plugin(child)),
            port,
            token: token.clone(),
            context_size,
            model_family: "none".into(),
            model_path: std::path::PathBuf::from(&model_path),
            model_identity: None,
        });

        Ok((port, token))
    }

    pub fn direct_runtime_start_stt_server(
        &self,
        app: AppHandle,
        model_path: String,
    ) -> Result<(u16, String)> {
        let resolved = Self::resolve_inventory_model(&app, &model_path, "llamacpp", "STT")?;
        let model_path = Self::inventory_path_string(&resolved.path, "STT")?;
        // whisper.cpp's bundled HTTP server has no authentication and exposes
        // a model-reload endpoint. Keep the selected model as configuration and
        // run the bounded, descendant-owned CLI per transcription instead.
        if model_path.is_empty() || model_path.len() > 4096 {
            return Err(anyhow!("STT model path is empty or too long"));
        }
        let path = std::path::PathBuf::from(&model_path);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| anyhow!("Could not inspect the selected STT model: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
            return Err(anyhow!(
                "The selected STT model must be a non-empty regular, non-symlink file"
            ));
        }
        let model_path = path
            .canonicalize()
            .map_err(|error| anyhow!("Could not resolve the selected STT model: {error}"))?
            .to_str()
            .ok_or_else(|| anyhow!("The selected STT model path is not valid UTF-8"))?
            .to_string();

        if self
            .stt_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_deref()
            == Some(model_path.as_str())
        {
            return Ok((0, String::new()));
        }

        app.state::<crate::process_tracker::ProcessTracker>()
            .cleanup_by_service("stt");
        let mut process_guard = self.stt_process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(process) = process_guard.take() {
            let _ = process.kill();
        }
        drop(process_guard);

        *self
            .stt_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(model_path);

        Ok((0, String::new()))
    }

    pub fn direct_runtime_start_image_server(
        &self,
        app: AppHandle,
        model_path: String,
    ) -> Result<()> {
        #[cfg(feature = "mlx")]
        let runtime = "mlx";
        #[cfg(not(feature = "mlx"))]
        let runtime = "llamacpp";
        let resolved = Self::resolve_inventory_model(&app, &model_path, runtime, "Diffusion")?;
        let model_path = Self::inventory_path_string(&resolved.path, "image")?;
        let mut model_guard = self
            .image_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *model_guard = Some(model_path);
        Ok(())
    }

    pub fn direct_runtime_start_tts_server(
        &self,
        app: AppHandle,
        model_path: String,
    ) -> Result<()> {
        let resolved = Self::resolve_inventory_model(&app, &model_path, "llamacpp", "TTS")?;
        let model_path = Self::inventory_path_string(&resolved.path, "TTS")?;
        let config_path = std::path::PathBuf::from(format!("{model_path}.json"));
        let config =
            thinclaw_platform::read_regular_file_bounded_single_link(&config_path, 4 * 1024 * 1024)
                .map_err(|error| anyhow!("The selected Piper model config is invalid: {error}"))?;
        serde_json::from_slice::<serde_json::Value>(&config).map_err(|error| {
            anyhow!("The selected Piper model config is not valid JSON: {error}")
        })?;
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

        if thinclaw_config::helpers::optional_env("THINCLAW_MANAGED_WHISPER_ENDPOINT")
            .ok()
            .flatten()
            .is_some_and(|value| value == "1")
        {
            thinclaw_config::helpers::remove_bridge_vars(&[
                "THINCLAW_MANAGED_WHISPER_ENDPOINT",
                "WHISPER_HTTP_ENDPOINT",
                "WHISPER_HTTP_TOKEN",
                "WHISPER_HTTP_MODEL",
            ]);
        }

        Ok(())
    }
}

#[cfg(test)]
mod inventory_lifecycle_tests {
    use super::SidecarManager;

    #[test]
    fn projector_selection_defaults_to_declared_and_rejects_other_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let declared = temp.path().join("mmproj-F16.gguf");
        let other = temp.path().join("mmproj-other.gguf");
        std::fs::write(&declared, b"projector").expect("declared projector");
        std::fs::write(&other, b"projector").expect("other projector");
        let declared = declared.canonicalize().expect("canonical declared");

        assert_eq!(
            SidecarManager::select_inventory_projector(Some(&declared), None)
                .expect("default projector"),
            Some(declared.to_string_lossy().to_string())
        );
        assert_eq!(
            SidecarManager::select_inventory_projector(
                Some(&declared),
                Some(declared.to_string_lossy().as_ref()),
            )
            .expect("matching explicit projector"),
            Some(declared.to_string_lossy().to_string())
        );
        assert!(SidecarManager::select_inventory_projector(
            Some(&declared),
            Some(other.to_string_lossy().as_ref()),
        )
        .is_err());
        assert!(SidecarManager::select_inventory_projector(
            None,
            Some(declared.to_string_lossy().as_ref()),
        )
        .is_err());
    }
}
