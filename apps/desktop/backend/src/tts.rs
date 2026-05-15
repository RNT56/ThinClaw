/// tts.rs — Text-to-Speech synthesis via the Piper sidecar or cloud backends.
///
/// Supports two paths:
///   1. **Cloud backend** (OpenAI TTS, ElevenLabs, Gemini) — routed via `InferenceRouter`.
///      When a cloud TTS backend is active, the request is sent to the provider API
///      and the response (typically MP3) is returned as base64.
///   2. **Local backend** (Piper sidecar) — falls through when no cloud backend is active.
///      Piper is a fast, MIT-licensed neural TTS engine (single binary, ONNX-based,
///      macOS ARM native).  It reads text from stdin and writes raw PCM to stdout.
///
/// Frontend workflow:
///   1. User clicks "Read Aloud" on an assistant bubble.
///   2. Frontend calls `direct_media_tts_synthesize(text, model_path)`.
///   3. Cloud path: InferenceRouter → provider API → base64 audio (MP3/WAV).
///      Local path: Piper sidecar → base64 raw PCM.
///   4. Frontend decodes base64 → `AudioContext.decodeAudioData()` → plays.
///      (decodeAudioData handles both PCM and MP3 transparently.)
use base64::{engine::general_purpose, Engine as _};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

use crate::inference::tts::TtsRequest;
use crate::inference::InferenceRouter;

/// Synthesise `text` using the active TTS backend (cloud or local Piper).
///
/// Returns base64-encoded audio. The format depends on the backend:
///   - Cloud (OpenAI, ElevenLabs, Gemini): MP3 or WAV
///   - Local (Piper): raw PCM (16-bit signed, mono, 22050 Hz)
///
/// `AudioContext.decodeAudioData()` on the frontend handles both formats.
///
/// The `model_path` is only used for the local Piper backend — it should point
/// to the `.onnx` file (Piper locates the companion `.onnx.json` automatically).
#[tauri::command]
#[specta::specta]
pub async fn direct_media_tts_synthesize(
    app: AppHandle,
    state: State<'_, crate::sidecar::SidecarManager>,
    router: State<'_, InferenceRouter>,
    config_mgr: State<'_, crate::config::ConfigManager>,
    text: String,
    model_path: Option<String>,
) -> Result<String, String> {
    // Read user's preferred voice from config (set via InferenceModeTab voice selector)
    let user_voice = config_mgr
        .get_config()
        .inference_models
        .as_ref()
        .and_then(|m| m.get("tts_voice").cloned());
    // ── Cloud TTS backend ────────────────────────────────────────────────
    // If a cloud backend is active (user selected in InferenceModeTab),
    // use it directly — no need for a local Piper model or sidecar.
    if let Some(backend) = router.tts_backend().await {
        let info = backend.info();
        tracing::info!(
            "[tts] Using cloud TTS backend: {} ({} chars)",
            info.display_name,
            text.len()
        );

        let request = TtsRequest {
            text: text.clone(),
            voice: user_voice.clone(),
            format: None,
            speed: None,
        };

        let audio_bytes = backend
            .synthesize(request)
            .await
            .map_err(|e| format!("Cloud TTS failed ({}): {}", info.display_name, e))?;

        tracing::info!(
            "[tts] Cloud synthesis complete — {} bytes from {}",
            audio_bytes.len(),
            info.display_name
        );

        return Ok(general_purpose::STANDARD.encode(&audio_bytes));
    }

    // ── Local TTS backend (Piper sidecar) ────────────────────────────────
    // Resolve which model to use: explicit arg → stored TTS model path → error
    let resolved_model = model_path
        .filter(|p| !p.trim().is_empty())
        .or_else(|| state.inner().get_tts_model())
        .ok_or("No TTS model selected. Please select a Piper ONNX model in Settings, or enable a cloud TTS backend.")?;

    tracing::info!(
        "[tts] Using local Piper — synthesising {} chars with model: {}",
        text.len(),
        resolved_model
    );

    // Build piper command
    let mut command = app
        .shell()
        .sidecar("bin/tts")
        .map_err(|e| format!("TTS binary not found: {e}. Expected at backend/bin/tts-aarch64-apple-darwin — replace with the Piper binary."))?;

    // Set DYLD_LIBRARY_PATH on macOS so the binary can find bundled shared libs
    #[cfg(target_os = "macos")]
    {
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
    }

    // Piper arguments:
    //   -m <model.onnx>   — ONNX voice model
    //   --output_raw      — write raw PCM to stdout rather than a WAV file
    let args = vec!["-m".to_string(), resolved_model, "--output_raw".to_string()];

    let (mut rx, mut child) = command
        .args(&args)
        .spawn()
        .map_err(|e| format!("Failed to spawn piper: {e}"))?;

    // Write text to stdin and close it so piper starts processing
    child
        .write(text.as_bytes())
        .map_err(|e| format!("Failed to write to piper stdin: {e}"))?;

    // Collect stdout (raw PCM bytes) and stderr (log lines)
    let mut pcm_bytes: Vec<u8> = Vec::new();

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(chunk) => {
                pcm_bytes.extend_from_slice(&chunk);
            }
            CommandEvent::Stderr(line) => {
                // Piper logs progress to stderr — not an error
                let msg = String::from_utf8_lossy(&line);
                tracing::debug!("[piper] {}", msg);
            }
            CommandEvent::Terminated(payload) => {
                if let Some(code) = payload.code {
                    if code != 0 {
                        return Err(format!("piper exited with code {code}"));
                    }
                }
                break;
            }
            CommandEvent::Error(e) => {
                return Err(format!("piper process error: {e}"));
            }
            _ => {}
        }
    }

    if pcm_bytes.is_empty() {
        return Err(
            "piper produced no audio output. Check that the model path is valid.".to_string(),
        );
    }

    tracing::info!("[tts] Synthesis complete — {} PCM bytes", pcm_bytes.len());

    // Return as base64 so it travels safely over the Tauri IPC boundary
    Ok(general_purpose::STANDARD.encode(&pcm_bytes))
}

/// List available voices for the active TTS backend.
///
/// For cloud backends like ElevenLabs this calls the provider API;
/// for local Piper it returns a hardcoded set of bundled voices.
/// Returns an empty list if no TTS backend is active.
#[tauri::command]
#[specta::specta]
pub async fn direct_media_tts_list_voices(
    router: State<'_, InferenceRouter>,
) -> Result<Vec<crate::inference::VoiceInfo>, String> {
    if let Some(backend) = router.tts_backend().await {
        let voices = backend
            .available_voices()
            .await
            .map_err(|e| format!("Failed to list voices: {}", e))?;
        Ok(voices)
    } else {
        // No TTS backend active
        Ok(vec![])
    }
}
