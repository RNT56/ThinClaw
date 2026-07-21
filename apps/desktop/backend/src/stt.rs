use crate::inference::stt::local::LocalSttBackend;
use crate::inference::stt::{
    audio_file_metadata, detect_audio_format, validate_stt_request, SttBackend, SttRequest,
};
#[cfg(not(feature = "mlx"))]
use crate::inference::stt::{validate_transcript, MAX_STT_TRANSCRIPT_BYTES};
use crate::inference::AudioFormat;
use crate::inference::InferenceRouter;
use crate::sidecar::SidecarManager;
#[cfg(not(feature = "mlx"))]
use std::ffi::OsString;
#[cfg(not(feature = "mlx"))]
use std::path::{Path, PathBuf};
#[cfg(not(feature = "mlx"))]
use std::time::Duration;
use tauri::AppHandle;
use tauri::Manager;
use tauri::State;
use thinclaw_runtime_contracts::{AssetKind, AssetOrigin, AssetRecord, DirectSttResponse};
use uuid::Uuid;

use crate::direct_assets::{DirectAssetStore, NewDirectAsset};
use crate::file_store::FileStore;

#[tauri::command]
#[specta::specta]
pub async fn direct_media_transcribe_audio(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    router: State<'_, InferenceRouter>,
    pool: State<'_, sqlx::SqlitePool>,
    audio_bytes: Vec<u8>,
) -> Result<DirectSttResponse, crate::thinclaw::bridge::BridgeError> {
    let format = detect_audio_format(&audio_bytes).ok_or_else(|| {
        "Unsupported or malformed audio. Recordings must be WAV, WebM, Ogg Opus, or MP3."
            .to_string()
    })?;
    let request = SttRequest {
        audio: audio_bytes.clone(),
        format,
        language: None,
    };
    validate_stt_request(&request).map_err(String::from)?;

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

        let text = backend
            .transcribe(request.clone())
            .await
            .map_err(|e| format!("Cloud STT failed ({}): {}", info.display_name, e))?;

        tracing::info!(
            "[stt] Cloud transcription complete — {} chars from {}",
            text.len(),
            info.display_name
        );

        let asset = persist_voice_input(
            &app,
            pool.inner(),
            &audio_bytes,
            format,
            Some(info.display_name.to_string()),
            &text,
        )
        .await?;
        return Ok(DirectSttResponse { text, asset });
    }

    // ── Local STT backend (whisper-server / whisper CLI) ──────────────────
    // CHECK FOR RUNNING SERVER FIRST
    let server_config = state.get_stt_server_config();

    if let Some((port, token, model_family)) = server_config {
        tracing::info!(
            "[stt] Using running server at port {} (engine: {})",
            port,
            model_family
        );
        let backend = LocalSttBackend {
            port,
            token,
            model_family: model_family.clone(),
        };
        let text = backend
            .transcribe(request)
            .await
            .map_err(|error| format!("Local STT failed: {error}"))?;
        let asset = persist_voice_input(
            &app,
            pool.inner(),
            &audio_bytes,
            format,
            Some(model_family),
            &text,
        )
        .await?;
        return Ok(DirectSttResponse { text, asset });
    }

    // -----------------------------------------------------------------------
    // FALLBACK TO CLI (only available for llama.cpp builds with whisper binary)
    // -----------------------------------------------------------------------
    #[cfg(feature = "mlx")]
    {
        // MLX builds don't ship the whisper CLI binary — the server must be running
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "STT server not running. Please start it from Settings → STT first."
                .to_string(),
        });
    }

    #[cfg(not(feature = "mlx"))]
    {
        use tauri_plugin_shell::ShellExt;
        use thinclaw_platform::{bounded_command_output, BoundedProcessError};

        if format != AudioFormat::Wav {
            return Err(
                crate::thinclaw::bridge::BridgeError::Runtime { message: "The local Whisper CLI fallback accepts WAV audio only. Start the STT server or select a cloud STT backend for this recording format."
                    .to_string() },
            );
        }

        let model_path = state.get_stt_model().ok_or(
            "STT model not selected. Select a Whisper model in Settings or enable cloud STT.",
        )?;
        validate_cli_model(Path::new(&model_path))?;

        tracing::info!("[stt] Server not running; using bounded Whisper CLI fallback");

        let temp_dir = tempfile::Builder::new()
            .prefix("thinclaw-stt-")
            .tempdir()
            .map_err(|error| format!("Failed to create private STT directory: {error}"))?;
        let input_path = temp_dir.path().join("input.wav");
        tokio::fs::write(&input_path, &audio_bytes)
            .await
            .map_err(|error| format!("Failed to stage STT audio: {error}"))?;

        let command = app
            .shell()
            .sidecar("whisper")
            .map_err(|e| format!("Failed to load sidecar: {}", e))?;

        // Inject DYLD_LIBRARY_PATH for macOS to find libwhisper.dylib
        #[cfg(target_os = "macos")]
        let command = {
            let mut command = command;
            if let Ok(resource_dir) = app.path().resource_dir() {
                let bin_dir = resource_dir.join("bin");
                let mut lib_path = bin_dir.to_string_lossy().to_string();
                if let Ok(cwd) = std::env::current_dir() {
                    let dev_bin = cwd.join("backend/bin");
                    if dev_bin.exists() {
                        lib_path = format!("{}:{}", dev_bin.to_string_lossy(), lib_path);
                    }
                }
                command = command.env("DYLD_LIBRARY_PATH", lib_path);
            }
            command
        };

        let input_path_arg = input_path.to_string_lossy().into_owned();
        let command = command.args([
            "-m",
            &model_path,
            "-f",
            &input_path_arg,
            "-otxt",           // Output to text file
            "--no-timestamps", // Just plain text
        ]);
        let std_command: std::process::Command = command.into();
        let mut command = tokio::process::Command::from(std_command);
        let output = bounded_command_output(
            &mut command,
            Duration::from_secs(10 * 60),
            64 * 1024,
            256 * 1024,
        )
        .await
        .map_err(|error| match error {
            BoundedProcessError::Timeout(_) => {
                "Whisper transcription exceeded its 10-minute deadline".to_string()
            }
            BoundedProcessError::OutputLimit { .. } => {
                "Whisper produced too much diagnostic output".to_string()
            }
            other => format!("Failed to execute Whisper safely: {other}"),
        })?;

        if !output.status.success() {
            return Err(format!(
                "Whisper exited unsuccessfully: {}",
                safe_process_message(&output.stderr)
            )
            .into());
        }

        let output_path = whisper_output_path(&input_path);
        let text = read_cli_transcript(&output_path).await?;
        let asset = persist_voice_input(
            &app,
            pool.inner(),
            &audio_bytes,
            format,
            Some("whisper-cli".to_string()),
            &text,
        )
        .await?;
        Ok(DirectSttResponse { text, asset })
    }
}

#[cfg(not(feature = "mlx"))]
fn validate_cli_model(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect the selected STT model: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("The selected STT model must be a regular, non-symlink file".to_string());
    }
    Ok(())
}

#[cfg(not(feature = "mlx"))]
fn whisper_output_path(input: &Path) -> PathBuf {
    let mut path: OsString = input.as_os_str().to_owned();
    path.push(".txt");
    PathBuf::from(path)
}

#[cfg(not(feature = "mlx"))]
async fn read_cli_transcript(path: &Path) -> Result<String, String> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|error| format!("Whisper did not produce a readable transcript: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Whisper transcript output is not a regular file".to_string());
    }
    if metadata.len() > MAX_STT_TRANSCRIPT_BYTES as u64 {
        return Err(format!(
            "Whisper transcript exceeds the {MAX_STT_TRANSCRIPT_BYTES}-byte limit"
        ));
    }
    let bytes = thinclaw_platform::read_regular_file_bounded_single_link_async(
        path.to_path_buf(),
        MAX_STT_TRANSCRIPT_BYTES as u64,
    )
    .await
    .map_err(|error| format!("Failed to read Whisper transcript: {error}"))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| "Whisper transcript is not valid UTF-8".to_string())?;
    validate_transcript(text).map_err(String::from)
}

#[cfg(not(feature = "mlx"))]
fn safe_process_message(bytes: &[u8]) -> String {
    let message: String = String::from_utf8_lossy(bytes)
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .collect();
    let message = message.trim();
    if message.is_empty() {
        "no diagnostic message".to_string()
    } else {
        message.to_string()
    }
}

async fn persist_voice_input(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    audio_bytes: &[u8],
    format: AudioFormat,
    provider: Option<String>,
    transcript: &str,
) -> Result<AssetRecord, String> {
    let file_store = app.state::<FileStore>();
    file_store
        .create_dir_all("voice/input")
        .await
        .map_err(|e| e.to_string())?;
    let id = Uuid::new_v4().to_string();
    let (_, mime_type, extension) = audio_file_metadata(format);
    let relative_path = format!("voice/input/{id}.{extension}");
    file_store
        .write(&relative_path, audio_bytes)
        .await
        .map_err(|e| format!("Failed to persist STT audio: {}", e))?;
    let path = file_store
        .resolve_path(&relative_path)
        .await
        .map_err(|error| error.to_string())?;

    let mut metadata = std::collections::HashMap::new();
    metadata.insert("transcript".to_string(), transcript.to_string());
    metadata.insert(
        "transcript_length".to_string(),
        transcript.len().to_string(),
    );

    let result = DirectAssetStore::upsert(
        pool,
        NewDirectAsset {
            id,
            kind: AssetKind::Audio,
            origin: AssetOrigin::VoiceInput,
            path: path.to_string_lossy().to_string(),
            mime_type: Some(mime_type.to_string()),
            size_bytes: Some(audio_bytes.len() as u64),
            sha256: None,
            prompt: None,
            provider,
            style_id: None,
            aspect_ratio: None,
            resolution: None,
            width: None,
            height: None,
            seed: None,
            thumbnail_path: None,
            is_favorite: false,
            tags: None,
            metadata,
        },
    )
    .await;
    if result.is_err() {
        // The asset record and backing file are one logical write. A failed DB
        // insertion must not leave an untracked recording behind.
        let _ = file_store.delete(&relative_path).await;
    }
    result
}
