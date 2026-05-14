use crate::inference::stt::SttRequest;
use crate::inference::AudioFormat;
use crate::inference::InferenceRouter;
use crate::sidecar::SidecarManager;
#[cfg(not(feature = "mlx"))]
use std::io::Write;
use tauri::AppHandle;
#[cfg(not(feature = "mlx"))]
use tauri::Manager;
use tauri::State;
#[cfg(not(feature = "mlx"))]
use tempfile::NamedTempFile;

#[tauri::command]
#[specta::specta]
pub async fn transcribe_audio(
    #[allow(unused_variables)] app: AppHandle,
    state: State<'_, SidecarManager>,
    router: State<'_, InferenceRouter>,
    audio_bytes: Vec<u8>,
) -> Result<String, String> {
    // ── Cloud STT backend ────────────────────────────────────────────────
    // If a cloud backend is active (user selected in InferenceModeTab),
    // send audio directly to the provider API — no local server needed.
    if let Some(backend) = router.stt_backend().await {
        let info = backend.info();
        tracing::info!(
            "[stt] Using cloud STT backend: {} ({} audio bytes)",
            info.display_name,
            audio_bytes.len()
        );

        let request = SttRequest {
            audio: audio_bytes.clone(),
            format: AudioFormat::Wav,
            language: None,
        };

        let text = backend
            .transcribe(request)
            .await
            .map_err(|e| format!("Cloud STT failed ({}): {}", info.display_name, e))?;

        tracing::info!(
            "[stt] Cloud transcription complete — {} chars from {}",
            text.len(),
            info.display_name
        );

        return Ok(text);
    }

    // ── Local STT backend (whisper-server / whisper CLI) ──────────────────
    // 1. Get Model
    #[allow(unused_variables)]
    let model_path = state.get_stt_model().ok_or("STT Model not selected. Please select a Whisper model in Settings, or enable a cloud STT backend.")?;

    // 2. Write audio to temp file (only needed for CLI fallback — not available in MLX builds)
    // IMPORTANT: Keep the NamedTempFile alive (_temp_file) so the file isn't
    // deleted before whisper CLI reads it. The file is deleted on drop.
    #[cfg(not(feature = "mlx"))]
    let (_temp_file, _temp_path) = {
        let mut temp_file =
            NamedTempFile::new().map_err(|e| format!("Failed to create temp file: {}", e))?;
        temp_file
            .write_all(&audio_bytes)
            .map_err(|e| format!("Failed to write audio: {}", e))?;
        temp_file
            .flush()
            .map_err(|e| format!("Failed to flush audio temp file: {}", e))?;
        let path = temp_file.path().to_string_lossy().to_string();
        (temp_file, path)
    };

    // CHECK FOR RUNNING SERVER FIRST
    let server_config = {
        let guard = state.stt_process.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .as_ref()
            .map(|p| (p.port, p.token.clone(), p.model_family.clone()))
    };

    if let Some((port, token, model_family)) = server_config {
        println!(
            "[stt] Using running server at port {} (engine: {})",
            port, model_family
        );

        let client = reqwest::Client::new();

        // Determine endpoint based on the running server type:
        // - MLX whisper server: /v1/audio/transcriptions (OpenAI-compatible)
        // - whisper.cpp server: /inference
        let endpoint = if model_family == "mlx-whisper" {
            format!("http://127.0.0.1:{}/v1/audio/transcriptions", port)
        } else {
            format!("http://127.0.0.1:{}/inference", port)
        };

        let part = reqwest::multipart::Part::bytes(audio_bytes.clone())
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| e.to_string())?;

        let form = reqwest::multipart::Form::new().part("file", part);

        let res = client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("Server request failed: {}", e))?;

        if !res.status().is_success() {
            return Err(format!(
                "Server returned error {}: {}",
                res.status(),
                res.text().await.unwrap_or_default()
            ));
        }

        // Both whisper.cpp and MLX STT servers return JSON: { "text": "..." }
        #[derive(serde::Deserialize)]
        struct WhisperResponse {
            text: String,
        }

        let json: WhisperResponse = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse JSON: {}", e))?;
        return Ok(json.text.trim().to_string());
    }

    // -----------------------------------------------------------------------
    // FALLBACK TO CLI (only available for llama.cpp builds with whisper binary)
    // -----------------------------------------------------------------------
    #[cfg(feature = "mlx")]
    {
        // MLX builds don't ship the whisper CLI binary — the server must be running
        return Err(
            "STT server not running. Please start it from Settings → STT first.".to_string(),
        );
    }

    #[cfg(not(feature = "mlx"))]
    {
        use tauri_plugin_shell::ShellExt;

        println!("[stt] Server not running, falling back to CLI");

        let mut command = app
            .shell()
            .sidecar("whisper")
            .map_err(|e| format!("Failed to load sidecar: {}", e))?;

        // Inject DYLD_LIBRARY_PATH for macOS to find libwhisper.dylib
        if let Ok(resource_dir) = app.path().resource_dir() {
            let bin_dir = resource_dir.join("bin");
            #[cfg(target_os = "macos")]
            {
                let mut lib_path = bin_dir.to_string_lossy().to_string();
                if let Ok(cwd) = std::env::current_dir() {
                    let dev_bin = cwd.join("backend/bin");
                    if dev_bin.exists() {
                        lib_path = format!("{}:{}", dev_bin.to_string_lossy(), lib_path);
                    }
                }
                command = command.env("DYLD_LIBRARY_PATH", lib_path);
            }
        }

        let output = command
            .args([
                "-m",
                &model_path,
                "-f",
                &_temp_path,
                "-otxt",           // Output to text file
                "--no-timestamps", // Just plain text
            ])
            .output()
            .await
            .map_err(|e| format!("Failed to execute whisper: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Whisper Error: {}", stderr));
        }

        // Read output
        let output_path = format!("{}.txt", _temp_path);

        if std::path::Path::new(&output_path).exists() {
            let text = std::fs::read_to_string(&output_path)
                .map_err(|e| format!("Failed to read output: {}", e))?;
            // Cleanup output file
            let _ = std::fs::remove_file(output_path);
            Ok(text.trim().to_string())
        } else {
            Err("No output file generated by whisper".to_string())
        }
    }
}
