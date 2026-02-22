/// tts.rs — Text-to-Speech synthesis via the Piper sidecar.
///
/// Piper is a fast, MIT-licensed neural TTS engine (single binary, ONNX-based,
/// macOS ARM native).  It reads text from stdin and writes raw PCM audio to
/// stdout when invoked with `--output_raw`.
///
/// The binary is registered in `tauri.conf.json` as `"bin/tts"` and Tauri
/// resolves it to `backend/bin/tts-aarch64-apple-darwin` (or the
/// appropriate target-triple suffix) at runtime.  To enable TTS, replace the
/// placeholder file with the real Piper binary:
///   cp /path/to/piper backend/bin/tts-aarch64-apple-darwin
///   chmod +x backend/bin/tts-aarch64-apple-darwin
///
/// Frontend workflow:
///   1. User clicks "Read Aloud" on an assistant bubble.
///   2. Frontend calls `tts_synthesize(text, model_path)`.
///   3. This function spawns the `bin/tts` sidecar, feeds text via stdin, collects PCM.
///   4. PCM bytes are returned as base64 to the frontend.
///   5. Frontend decodes base64 → `AudioContext` → plays.
use base64::{engine::general_purpose, Engine as _};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

/// Synthesise `text` using the Piper TTS ONNX model at `model_path`.
///
/// Returns base64-encoded raw PCM (16-bit signed, mono, 22050 Hz by default)
/// suitable for decoding with `AudioContext.decodeAudioData()` on the frontend.
///
/// The `model_path` should point to the `.onnx` file (Piper will automatically
/// locate the companion `.onnx.json` config file in the same directory).
#[tauri::command]
#[specta::specta]
pub async fn tts_synthesize(
    app: AppHandle,
    state: State<'_, crate::sidecar::SidecarManager>,
    text: String,
    model_path: Option<String>,
) -> Result<String, String> {
    // Resolve which model to use: explicit arg → stored TTS model path → error
    let resolved_model = model_path
        .filter(|p| !p.trim().is_empty())
        .or_else(|| state.inner().get_tts_model())
        .ok_or("No TTS model selected. Please select a Piper ONNX model in Settings.")?;

    tracing::info!(
        "[tts] Synthesising {} chars using model: {}",
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
