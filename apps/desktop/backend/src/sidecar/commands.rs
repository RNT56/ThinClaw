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
) -> Result<(), String> {
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
        .map_err(|e| e.to_string())?;

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
) -> Result<(), String> {
    // Probe the actual embedding dimension from the model's config.json
    let actual_dim: Option<usize> = (|| -> Option<usize> {
        let p = std::path::Path::new(&model_path);
        let cfg_path = if p.is_dir() {
            p.join("config.json")
        } else {
            return None;
        };
        let content = std::fs::read_to_string(&cfg_path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&content).ok()?;
        v.get("hidden_size")
            .or_else(|| v.get("d_model"))
            .or_else(|| v.get("embedding_dim"))
            .and_then(|x| x.as_u64())
            .map(|n| n as usize)
    })();

    if let Some(dim) = actual_dim {
        let current_dim = vector_manager.dimensions();
        if dim != current_dim {
            eprintln!(
                "[embedding] Dimension changed: {} → {}. Purging stale vector indices.",
                current_dim, dim
            );
            vector_manager.purge_by_dimension(current_dim);
            vector_manager
                .reinit(dim)
                .map_err(|e| format!("Failed to reinit vector store: {}", e))?;
            let config_mgr = app.state::<crate::config::ConfigManager>();
            let mut cfg = config_mgr.get_config();
            cfg.vector_dimensions = dim as u32;
            config_mgr.save_config(&cfg).await?;
            println!(
                "[embedding] Vector store reinitialized at dimension {}.",
                dim
            );
        }
    }

    #[cfg(feature = "mlx")]
    {
        state
            .start_mlx_embedding_server(app.clone(), model_path)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(feature = "mlx"))]
    {
        state
            .direct_runtime_start_embedding_server(app.clone(), model_path)
            .map(|_| ())
            .map_err(|e| e.to_string())?;
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
    let res = state
        .direct_runtime_start_summarizer_server(app.clone(), model_path, context_size, -1)
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
pub async fn direct_runtime_start_stt_server(
    app: AppHandle,
    state: State<'_, SidecarManager>,
    model_path: String,
) -> Result<(), String> {
    // Route to MLX STT server when compiled with MLX feature
    #[cfg(feature = "mlx")]
    let res = state
        .start_mlx_stt_server(app.clone(), model_path)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string());

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
pub async fn direct_runtime_cancel_generation(
    state: State<'_, SidecarManager>,
) -> Result<(), String> {
    state.cancellation_token.store(true, Ordering::SeqCst);
    Ok(())
}
