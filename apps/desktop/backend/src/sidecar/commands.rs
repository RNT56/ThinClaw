//! Tauri command surface and the reusable embedding-server start core.
//!
//! These wrap `SidecarManager` lifecycle methods, perform `/health` readiness
//! polling, and emit `SidecarEvent`s to the frontend. The command names,
//! signatures, and paths are preserved so handler registration in
//! `setup/commands.rs` keeps resolving unchanged.

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter, Manager, State};

use super::core::SidecarManager;
use super::types::{ChatServerConfig, ChatServerOptions, SidecarEvent, SidecarStatus};

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
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    // Guard: this command starts the llama.cpp sidecar and is only meaningful
    // in llamacpp builds.  In MLX/vLLM builds the binary may still be on disk
    // from a previous install, but we must NOT launch it — the user should
    // pick a model through the engine's own selector instead.
    #[cfg(feature = "mlx")]
    {
        return Err(
            ("This build uses the MLX engine. GGUF/llama.cpp models are not supported. \
             Please select an MLX-compatible model (safetensors directory) from the model list."
                .to_string())
            .into(),
        );
    }

    #[cfg(all(feature = "vllm", not(feature = "mlx")))]
    {
        return Err(
            ("This build uses the vLLM engine. GGUF/llama.cpp models are not supported. \
             Please select a vLLM-compatible model (safetensors directory) from the model list."
                .to_string())
            .into(),
        );
    }

    // llama.cpp path — only reached in llamacpp builds
    // The allow is needed because cfg(mlx/vllm) returns above make this unreachable in those builds.
    #[allow(unreachable_code)]
    let app_handle_for_closure = app.clone();
    let (port, _) = state
        .direct_runtime_start_chat_server(
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
        .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?;

    // Wait for server to be ready (poll /health)
    let start = std::time::Instant::now();
    let client = reqwest::Client::new();
    println!(
        "[sidecar] Waiting for chat server to be ready on port {}...",
        port
    );

    loop {
        if start.elapsed().as_secs() > 120 {
            eprintln!("[sidecar] Timeout waiting for chat server readiness.");
            break;
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
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    // Config metadata is a useful hint, but is not authoritative enough to
    // purge an existing index. The live server response is probed below.
    let config_dimension: Option<usize> = (|| -> Option<usize> {
        let p = std::path::Path::new(&model_path);
        let cfg_path = if p.is_dir() {
            p.join("config.json")
        } else {
            return None;
        };
        let content = std::fs::read_to_string(&cfg_path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&content).ok()?;
        crate::hf_hub::embedding_dimension_from_config(&v)
    })();

    #[cfg(feature = "mlx")]
    {
        state
            .start_mlx_embedding_server(app.clone(), model_path)
            .await
            .map(|_| ())
            .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?;
    }

    let (port, token) = state.get_embedding_config().ok_or_else(|| {
        crate::thinclaw::bridge::BridgeError::from(
            "Embedding server started without publishing its connection configuration",
        )
    })?;
    let live_dimension = match probe_embedding_dimension(port, &token).await {
        Ok(dimension) => dimension,
        Err(error) => {
            stop_failed_embedding_server(state);
            return Err(error);
        }
    };
    if config_dimension.is_some_and(|hint| hint != live_dimension) {
        tracing::warn!(
            config_dimension,
            live_dimension,
            "embedding model config disagrees with live server output; using live output"
        );
    }
    if let Err(error) = crate::inference::reconcile_embedding_dimensions(
        app,
        vector_manager,
        live_dimension,
        "local embedding server",
    )
    .await
    {
        stop_failed_embedding_server(state);
        return Err(error);
    }
    #[cfg(not(feature = "mlx"))]
    {
        state
            .direct_runtime_start_embedding_server(app.clone(), model_path)
            .map(|_| ())
            .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?;
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

fn stop_failed_embedding_server(state: &SidecarManager) {
    let mut process = state
        .embedding_process
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(process) = process.take() {
        let _ = process.kill();
    }
}

async fn probe_embedding_dimension(
    port: u16,
    token: &str,
) -> Result<usize, crate::thinclaw::bridge::BridgeError> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| format!("Failed to construct embedding probe client: {error}"))?;
    let url = format!("http://127.0.0.1:{port}/v1/embeddings");
    let mut last_error = "embedding server did not become ready".to_string();

    for _ in 0..120 {
        match client
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "input": "ThinClaw dimension probe",
                "model": "default"
            }))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                let body: serde_json::Value = response
                    .json()
                    .await
                    .map_err(|error| format!("Invalid embedding probe response: {error}"))?;
                let embedding = body
                    .get("data")
                    .and_then(|data| data.as_array())
                    .and_then(|data| data.first())
                    .and_then(|item| item.get("embedding"))
                    .and_then(|embedding| embedding.as_array())
                    .ok_or_else(|| "Embedding probe returned no vector".to_string())?;
                let dimensions = embedding.len();
                if !(1..=crate::inference::embedding::MAX_EMBEDDING_DIMENSIONS)
                    .contains(&dimensions)
                {
                    return Err(
                        format!("Embedding probe returned invalid dimension {dimensions}").into(),
                    );
                }
                if embedding
                    .iter()
                    .any(|value| value.as_f64().is_none_or(|value| !value.is_finite()))
                {
                    return Err("Embedding probe returned a non-finite vector".into());
                }
                return Ok(dimensions);
            }
            Ok(response) => {
                last_error = format!("embedding server returned HTTP {}", response.status());
            }
            Err(error) => last_error = error.to_string(),
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    Err(format!("Embedding server readiness probe failed: {last_error}").into())
}

#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_start_embedding_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
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
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let res = state
        .direct_runtime_start_summarizer_server(app.clone(), model_path, context_size, -1)
        .map(|_| ())
        .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()));

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
pub async fn direct_runtime_start_stt_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    // Route to MLX STT server when compiled with MLX feature
    #[cfg(feature = "mlx")]
    let res = state
        .start_mlx_stt_server(app.clone(), model_path)
        .await
        .map(|_| ())
        .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()));

    #[cfg(not(feature = "mlx"))]
    let res = state
        .direct_runtime_start_stt_server(app.clone(), model_path)
        .map(|_| ())
        .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()));

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
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    state
        .direct_runtime_start_image_server(app, model_path)
        .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_start_tts_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    state
        .direct_runtime_start_tts_server(app, model_path)
        .map(|_| ())
        .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub fn direct_runtime_get_chat_server_config(
    state: State<'_, SidecarManager>,
) -> Option<ChatServerConfig> {
    state.get_chat_config().map(
        |(port, token, context_size, model_family)| ChatServerConfig {
            port,
            token,
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
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    state
        .stop_all()
        .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?;
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
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    state.cancellation_token.store(true, Ordering::SeqCst);
    Ok(())
}
