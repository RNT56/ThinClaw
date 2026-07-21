//! Tauri command surface and the reusable embedding-server start core.
//!
//! These wrap `SidecarManager` lifecycle methods, perform `/health` readiness
//! polling, and emit `SidecarEvent`s to the frontend. The command names,
//! signatures, and paths are preserved so handler registration in
//! `setup/commands.rs` keeps resolving unchanged.

use std::sync::atomic::Ordering;
use std::sync::LazyLock;

use tauri::{AppHandle, Emitter, Manager, State};

use super::core::SidecarManager;
use super::types::{ChatServerConfig, ChatServerOptions, SidecarEvent, SidecarStatus};
use crate::inference::embedding::{local::LocalEmbeddingBackend, EmbeddingBackend};

static CHAT_START_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
static EMBEDDING_START_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
static SUMMARIZER_START_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

const MANAGED_STT_MARKER: &str = "THINCLAW_MANAGED_WHISPER_ENDPOINT";
const STT_ENDPOINT_KEY: &str = "WHISPER_HTTP_ENDPOINT";
const STT_TOKEN_KEY: &str = "WHISPER_HTTP_TOKEN";
const STT_MODEL_KEY: &str = "WHISPER_HTTP_MODEL";

fn clear_managed_stt_endpoint() {
    let managed = thinclaw_config::helpers::optional_env(MANAGED_STT_MARKER)
        .ok()
        .flatten()
        .is_some_and(|value| value == "1");
    if managed {
        thinclaw_config::helpers::remove_bridge_vars(&[
            MANAGED_STT_MARKER,
            STT_ENDPOINT_KEY,
            STT_TOKEN_KEY,
            STT_MODEL_KEY,
        ]);
    }
}

#[cfg(feature = "mlx")]
fn install_managed_stt_endpoint(port: u16, token: String) {
    let existing = thinclaw_config::helpers::optional_env(STT_ENDPOINT_KEY)
        .ok()
        .flatten();
    let already_managed = thinclaw_config::helpers::optional_env(MANAGED_STT_MARKER)
        .ok()
        .flatten()
        .is_some_and(|value| value == "1");
    if existing.is_some() && !already_managed {
        tracing::info!("Preserving the explicitly configured Whisper HTTP endpoint");
        return;
    }

    thinclaw_config::helpers::inject_bridge_vars(std::collections::HashMap::from([
        (MANAGED_STT_MARKER.to_string(), "1".to_string()),
        (
            STT_ENDPOINT_KEY.to_string(),
            format!("http://127.0.0.1:{port}/v1/audio/transcriptions"),
        ),
        (STT_TOKEN_KEY.to_string(), token),
        (STT_MODEL_KEY.to_string(), "thinclaw-whisper".to_string()),
    ]));
}

#[tauri::command]
#[specta::specta]
#[allow(unused_variables)] // params are intentionally unused in MLX/vLLM builds that return early
#[allow(clippy::too_many_arguments)] // Flat Tauri command ABI for generated bindings.
pub async fn direct_runtime_start_chat_server(
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
    let _start_guard = CHAT_START_LOCK.lock().await;
    if expose_network.unwrap_or(false) {
        return Err(
            "Direct model-server network exposure is disabled; use the authenticated ThinClaw gateway for remote access"
                .to_string(),
        );
    }
    if context_size == 0 || context_size > 1_048_576 {
        return Err("Context size must be between 1 and 1,048,576 tokens".to_string());
    }

    // Guard: this command starts the llama.cpp sidecar and is only meaningful
    // in llamacpp builds.  In MLX/vLLM builds the binary may still be on disk
    // from a previous install, but we must NOT launch it — the user should
    // pick a model through the engine's own selector instead.
    #[cfg(feature = "mlx")]
    {
        return Err(
            "This build uses the MLX engine. GGUF/llama.cpp models are not supported. \
             Please select an MLX-compatible model (safetensors directory) from the model list."
                .to_string(),
        );
    }

    #[cfg(all(feature = "vllm", not(feature = "mlx")))]
    {
        return Err(
            "This build uses the vLLM engine. GGUF/llama.cpp models are not supported. \
             Please select a vLLM-compatible model (safetensors directory) from the model list."
                .to_string(),
        );
    }

    // llama.cpp path — only reached in llamacpp builds
    // The allow is needed because cfg(mlx/vllm) returns above make this unreachable in those builds.
    #[allow(unreachable_code)]
    let app_handle_for_closure = app.clone();
    let (port, token) = state
        .direct_runtime_start_chat_server(
            app.clone(),
            ChatServerOptions {
                model_path: model_path.clone(),
                context_size,
                n_gpu: -1,
                template,
                mmproj,
                expose: false,
                mlock: mlock.unwrap_or(false),
                quantize_kv: quantize_kv.unwrap_or(false),
            },
            move |code, exited_port| {
                // This callback runs when the process terminates
                let manager = app_handle_for_closure.state::<SidecarManager>();
                let is_current = manager
                    .chat_process
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_ref()
                    .is_some_and(|process| process.port == exited_port);
                if !is_current {
                    return;
                }

                if code != 0 {
                    eprintln!("[sidecar] Chat server crashed unexpectedly.");

                    if let Ok(mut guard) = manager.chat_process.lock() {
                        if guard
                            .as_ref()
                            .is_some_and(|process| process.port == exited_port)
                        {
                            *guard = None;
                        }
                    }

                    app_handle_for_closure
                        .emit(
                            "sidecar_event",
                            SidecarEvent::Crashed {
                                service: "chat".into(),
                                code,
                            },
                        )
                        .ok();
                } else {
                    // Clean exit (0) logic
                    if let Ok(mut guard) = manager.chat_process.lock() {
                        if guard
                            .as_ref()
                            .is_some_and(|process| process.port == exited_port)
                        {
                            *guard = None;
                        }
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
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|error| format!("Could not create the local readiness client: {error}"))?;
    println!(
        "[sidecar] Waiting for chat server to be ready on port {}...",
        port
    );

    loop {
        if start.elapsed().as_secs() > 120 {
            let _ = state.direct_runtime_stop_chat_server();
            return Err("Chat server startup exceeded its 2-minute deadline".to_string());
        }

        // Check if process died
        if state
            .chat_process
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
        {
            return Err("Chat server process exited prematurely during startup".into());
        }

        match client
            .get(format!("http://127.0.0.1:{}/health", port))
            .bearer_auth(&token)
            .send()
            .await
        {
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

/// Core logic for starting the embedding server.
/// Extracted so `rag.rs` can call it for on-demand auto-start during ingestion.
pub async fn start_embedding_server_core(
    app: &AppHandle,
    state: &SidecarManager,
    vector_manager: &crate::vector_store::VectorStoreManager,
    model_path: String,
) -> Result<(), String> {
    let _start_guard = EMBEDDING_START_LOCK.lock().await;

    #[cfg(feature = "mlx")]
    let (port, token) = state
        .start_mlx_embedding_server(app.clone(), model_path)
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(not(feature = "mlx"))]
    let (port, token) = state
        .direct_runtime_start_embedding_server(app.clone(), model_path)
        .map_err(|e| e.to_string())?;

    let readiness_client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|error| {
            stop_embedding_process(app, state);
            format!("Could not create the embedding readiness client: {error}")
        })?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3 * 60);
    loop {
        if tokio::time::Instant::now() >= deadline {
            stop_embedding_process(app, state);
            return Err("Embedding server startup exceeded its 3-minute deadline".to_string());
        }
        if state.get_embedding_config().is_none() {
            return Err("Embedding server exited during startup".to_string());
        }
        if readiness_client
            .get(format!("http://127.0.0.1:{port}/health"))
            .bearer_auth(&token)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Probe the serving API itself instead of trusting heterogeneous model
    // config schemas. This also verifies response shape and finite values.
    let probe_backend = LocalEmbeddingBackend {
        port,
        token,
        model_name: "thinclaw-embedding".to_string(),
        dimensions: 0,
        profile_id: state
            .get_embedding_snapshot()
            .map(|(_, _, identity)| identity)
            .ok_or_else(|| "Embedding server model identity is unavailable".to_string())?,
    };
    let model_identity = probe_backend.profile_id.clone();
    let actual_dim = match probe_backend
        .embed("ThinClaw dimension probe".to_string())
        .await
    {
        Ok(vector) => vector.len(),
        Err(error) => {
            stop_embedding_process(app, state);
            return Err(format!("Embedding server probe failed: {error}"));
        }
    };
    let current_dim = vector_manager.dimensions();
    let profile = format!("local:{model_identity}:{actual_dim}");
    let pool = app.state::<sqlx::SqlitePool>();
    crate::rag::activate_embedding_profile(pool.inner(), vector_manager, &profile, actual_dim)
        .await
        .map_err(|error| format!("Failed to activate embedding profile: {error}"))?;
    if actual_dim != current_dim {
        let config_manager = app.state::<crate::config::ConfigManager>();
        let mut config = config_manager.get_config();
        config.vector_dimensions = u32::try_from(actual_dim)
            .map_err(|_| "Embedding dimension exceeds the supported range".to_string())?;
        config_manager.save_config(&config)?;
    }

    app.emit(
        "sidecar_event",
        SidecarEvent::Started {
            service: "embedding".into(),
        },
    )
    .ok();
    Ok(())
}

fn stop_embedding_process(app: &AppHandle, state: &SidecarManager) {
    let process = state
        .embedding_process
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    if let Some(process) = process {
        let _ = process.kill();
    }
    app.state::<crate::process_tracker::ProcessTracker>()
        .cleanup_by_service("embedding");
}

#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_start_embedding_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
) -> Result<(), String> {
    let vec_manager = app.state::<crate::vector_store::VectorStoreManager>();
    let res = start_embedding_server_core(&app, &state, &vec_manager, model_path).await;
    res
}

#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_start_summarizer_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
    context_size: u32,
) -> Result<(), String> {
    let _start_guard = SUMMARIZER_START_LOCK.lock().await;
    let (port, token) = state
        .direct_runtime_start_summarizer_server(app.clone(), model_path, context_size, -1)
        .map_err(|e| e.to_string())?;

    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|error| {
            stop_summarizer_process(&app, state.inner());
            format!("Could not create the summarizer readiness client: {error}")
        })?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3 * 60);
    loop {
        if tokio::time::Instant::now() >= deadline {
            stop_summarizer_process(&app, state.inner());
            return Err("Summarizer server startup exceeded its 3-minute deadline".to_string());
        }
        if state.get_summarizer_config().is_none() {
            return Err("Summarizer server exited during startup".to_string());
        }
        if client
            .get(format!("http://127.0.0.1:{port}/health"))
            .bearer_auth(&token)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    app.emit(
        "sidecar_event",
        SidecarEvent::Started {
            service: "summarizer".into(),
        },
    )
    .ok();
    Ok(())
}

fn stop_summarizer_process(app: &AppHandle, state: &SidecarManager) {
    let process = state
        .summarizer_process
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    if let Some(process) = process {
        let _ = process.kill();
    }
    app.state::<crate::process_tracker::ProcessTracker>()
        .cleanup_by_service("summarizer");
}

#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_start_stt_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
) -> Result<(), String> {
    clear_managed_stt_endpoint();
    // Route to MLX STT server when compiled with MLX feature
    #[cfg(feature = "mlx")]
    let res = match state.start_mlx_stt_server(app.clone(), model_path).await {
        Ok((port, token)) => {
            install_managed_stt_endpoint(port, token);
            Ok(())
        }
        Err(error) => Err(error.to_string()),
    };

    #[cfg(not(feature = "mlx"))]
    let res = state
        .direct_runtime_start_stt_server(app.clone(), model_path)
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
pub async fn direct_runtime_start_image_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
) -> Result<(), String> {
    state
        .direct_runtime_start_image_server(app, model_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_start_tts_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
) -> Result<(), String> {
    state
        .direct_runtime_start_tts_server(app, model_path)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn direct_runtime_get_chat_server_config(
    state: State<'_, SidecarManager>,
) -> Option<ChatServerConfig> {
    state.get_chat_config().map(
        |(port, _token, context_size, model_family)| ChatServerConfig {
            port,
            // Runtime credentials are backend state and never renderer state.
            token: String::new(),
            context_size,
            model_family,
        },
    )
}

#[tauri::command]
#[specta::specta]
pub fn direct_runtime_get_sidecar_status(state: State<'_, SidecarManager>) -> SidecarStatus {
    let (chat, embed, stt, tts, image, summ) = state.get_status();
    SidecarStatus {
        chat_running: chat,
        embedding_running: embed,
        stt_running: stt,
        tts_configured: tts,
        image_configured: image,
        summarizer_running: summ,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_stop_chat_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    _model_path: String,
) -> Result<(), String> {
    state
        .direct_runtime_stop_chat_server()
        .map_err(|e| e.to_string())?;
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
pub async fn direct_runtime_cancel_generation(
    state: State<'_, SidecarManager>,
) -> Result<(), String> {
    state.cancellation_token.store(true, Ordering::SeqCst);
    Ok(())
}
