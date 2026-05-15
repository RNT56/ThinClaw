use sqlx::sqlite::SqlitePoolOptions;
use std::fs;
use std::sync::OnceLock;

/// Global log broadcaster — shared between the tracing subscriber (WebLogLayer)
/// and the ThinClaw bridge so all tracing::* events reach the UI Logs panel.
///
/// Initialized once in `run()` before any threads are spawned.
pub(crate) static GLOBAL_LOG_BROADCASTER: OnceLock<
    Arc<thinclaw_core::channels::web::log_layer::LogBroadcaster>,
> = OnceLock::new();

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
#[specta::specta]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
#[specta::specta]
fn hide_spotlight(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("spotlight") {
        let _ = window.hide();
    }
}

#[tauri::command]
#[specta::specta]
fn toggle_spotlight(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("spotlight") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.center();
            let _ = window.set_focus();
            let _ = window.show();
        }
    } else {
        let mut win_builder = tauri::WebviewWindowBuilder::new(
            &app,
            "spotlight",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .decorations(false)
        .resizable(true)
        .min_inner_size(600.0, 150.0)
        .always_on_top(true)
        .visible(false)
        .transparent(true)
        .inner_size(600.0, 150.0)
        .center()
        .skip_taskbar(true);

        #[cfg(target_os = "macos")]
        {
            win_builder = win_builder
                .hidden_title(true)
                .shadow(false)
                .visible_on_all_workspaces(true);
        }

        if let Ok(window) = win_builder.build() {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

pub mod chat;
pub mod cloud;
pub mod config;
pub mod direct_assets;
pub mod engine;
pub mod file_store;
pub mod gguf;
pub mod hf_hub;
mod history;
pub mod image_gen;
pub mod images;
pub mod imagine;
pub mod inference;
pub mod model_manager;
pub mod permissions;
pub mod personas;
pub mod process_tracker;
pub mod projects;
pub mod rag;
pub mod reranker;
pub mod rig_cache;
pub mod rig_lib;
pub mod secret_store;
pub mod setup;
pub mod sidecar;
pub mod stt;
pub mod system;
pub mod templates;
pub mod thinclaw;
pub mod tts;
pub mod vector_store;
pub mod web_search;

use sidecar::SidecarManager;
use std::str::FromStr;
use std::sync::Arc;
use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

#[cfg(debug_assertions)]
pub fn sanitize_typescript_bindings(path: &str) -> std::io::Result<()> {
    let mut source = std::fs::read_to_string(path)?;
    let original = source.clone();

    source = source.replace(
        "export type TAURI_CHANNEL<TSend> = null",
        "export type TAURI_CHANNEL<TSend> = import(\"@tauri-apps/api/core\").Channel<TSend>",
    );
    source = source.replace(
        "import {\n\tinvoke as TAURI_INVOKE,\n\tChannel as TAURI_CHANNEL,\n} from \"@tauri-apps/api/core\";",
        "import { invoke as TAURI_INVOKE } from \"@tauri-apps/api/core\";",
    );
    source = source.replace(
        "async thinclawMcpGetPrompt(serverName: string, promptName: string, arguments: JsonValue | null)",
        "async thinclawMcpGetPrompt(serverName: string, promptName: string, promptArguments: JsonValue | null)",
    );
    source = source.replace(
        "TAURI_INVOKE(\"thinclaw_mcp_get_prompt\", { serverName, promptName, arguments })",
        "TAURI_INVOKE(\"thinclaw_mcp_get_prompt\", { serverName, promptName, arguments: promptArguments })",
    );

    if !source.contains("export const events =") {
        if let (Some(import_start), Some(result_start)) = (
            source.find("import * as TAURI_API_EVENT"),
            source.find("export type Result<T, E>"),
        ) {
            source.replace_range(import_start..result_start, "");
        }
        if let Some(events_start) = source.find("\nfunction __makeEvents__") {
            let commands_start = source.find("export const commands");
            if commands_start
                .map(|start| events_start > start)
                .unwrap_or(events_start > 0)
            {
                source.truncate(events_start);
                source.push('\n');
            }
        }
    }

    if source.contains("export type DirectChatMessage")
        && !source.contains("export type Message =")
    {
        source.push_str(
            r#"

// Compatibility aliases for existing Desktop UI code. DirectChat* is the
// canonical contract surface; these aliases keep older snake_case UI state
// readable while call sites migrate to the shared DTO names.
export type AttachedDoc = DirectAttachedDocument & { asset_ref?: AssetRef | null }
export type Message = {
  role: string;
  content: string;
  images?: string[] | null;
  assets?: AssetRef[] | null;
  attached_docs?: AttachedDoc[] | null;
  attachedDocs?: DirectAttachedDocument[] | null;
  is_summary?: boolean | null;
  isSummary?: boolean | null;
  original_messages?: Message[] | null;
  originalMessages?: DirectChatMessage[] | null;
}
export type ChatPayload = DirectChatPayload
export type TokenUsage = DirectTokenUsage & {
  prompt_tokens?: number;
  completion_tokens?: number;
  total_tokens?: number;
}
"#,
        );
    }

    let mut lines = source.lines().map(str::trim_end).collect::<Vec<_>>();
    while matches!(lines.last(), Some(line) if line.is_empty()) {
        lines.pop();
    }
    source = lines.join("\n");
    source.push('\n');

    if source != original {
        std::fs::write(path, source)?;
    }

    Ok(())
}

#[cfg(all(test, debug_assertions))]
mod binding_sanitizer_tests {
    use super::sanitize_typescript_bindings;

    #[test]
    fn sanitizer_restores_tauri_channel_type_and_prompt_argument_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("bindings.ts");
        std::fs::write(
            &path,
            r#"import {
	invoke as TAURI_INVOKE,
	Channel as TAURI_CHANNEL,
} from "@tauri-apps/api/core";
import * as TAURI_API_EVENT from "@tauri-apps/api/event";
export type Result<T, E> = { status: "ok"; data: T } | { status: "error"; error: E }
export type TAURI_CHANNEL<TSend> = null
export const commands = {
async thinclawMcpGetPrompt(serverName: string, promptName: string, arguments: JsonValue | null) {
    return { status: "ok", data: await TAURI_INVOKE("thinclaw_mcp_get_prompt", { serverName, promptName, arguments }) };
}
}
function __makeEvents__() {}
"#,
        )
        .expect("write binding fixture");

        sanitize_typescript_bindings(path.to_str().expect("utf8 path")).expect("sanitize");
        let sanitized = std::fs::read_to_string(path).expect("read sanitized bindings");

        assert!(sanitized.contains("import { invoke as TAURI_INVOKE }"));
        assert!(sanitized.contains(
            "export type TAURI_CHANNEL<TSend> = import(\"@tauri-apps/api/core\").Channel<TSend>"
        ));
        assert!(sanitized.contains("promptArguments: JsonValue | null"));
        assert!(sanitized.contains("arguments: promptArguments"));
        assert!(!sanitized.contains("import * as TAURI_API_EVENT"));
        assert!(!sanitized.contains("function __makeEvents__"));
        assert!(sanitized.ends_with('\n'));
    }
}

#[cfg(target_os = "macos")]
fn migrate_legacy_app_data(app_data_dir: &std::path::Path) {
    if app_data_dir.exists() {
        return;
    }

    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let legacy_dir = std::path::PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("com.schack.scrappy");

    if !legacy_dir.exists() {
        return;
    }

    if let Err(error) = copy_dir_contents(&legacy_dir, app_data_dir) {
        tracing::warn!(
            legacy = %legacy_dir.display(),
            target = %app_data_dir.display(),
            error = %error,
            "Failed to import legacy desktop app data"
        );
    } else {
        tracing::info!(
            legacy = %legacy_dir.display(),
            target = %app_data_dir.display(),
            "Imported legacy desktop app data to ThinClaw Desktop"
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn migrate_legacy_app_data(_app_data_dir: &std::path::Path) {}

#[cfg(target_os = "macos")]
fn copy_dir_contents(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_contents(&source, &target)?;
        } else if file_type.is_file() && !target.exists() {
            fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ── Tracing / Logging init ───────────────────────────────────────────
    // ThinClaw's init_tracing() installs:
    //   1. A reloadable EnvFilter (ironclaw=debug by default)
    //   2. A fmt layer writing to stderr (visible in terminal / macOS Console)
    //   3. A WebLogLayer that feeds LogBroadcaster → UI Logs tab
    //
    // We MUST NOT call tracing_subscriber::fmt::init() before this — doing so
    // sets the global subscriber and silences all ironclaw tracing::* calls.
    //
    // The broadcaster is stored in a OnceLock so it survives bridge restarts.
    use std::sync::OnceLock;
    static TRACING_INIT: OnceLock<()> = OnceLock::new();
    TRACING_INIT.get_or_init(|| {
        // Default: show ironclaw internals at debug level, noisy crates at warn
        if std::env::var("RUST_LOG").is_err() {
            // Safety: single-threaded at app startup, before any threads are spawned.
            // set_var is unavoidable here — tracing_subscriber reads it during init.
            #[allow(unused_unsafe)]
            unsafe {
                std::env::set_var(
                    "RUST_LOG",
                    "ironclaw=debug,backend=info,tower_http=warn,sqlx=warn,hyper=warn",
                );
            }
        }
        // Create a shared broadcaster and install the full tracing stack.
        let broadcaster =
            std::sync::Arc::new(thinclaw_core::channels::web::log_layer::LogBroadcaster::new());
        // Store globally so the bridge can retrieve it.
        GLOBAL_LOG_BROADCASTER
            .set(std::sync::Arc::clone(&broadcaster))
            .ok();
        thinclaw_core::channels::web::log_layer::init_tracing(broadcaster, cfg!(debug_assertions));
    });

    let specta_builder = setup::commands::specta_builder();

    #[cfg(debug_assertions)]
    specta_builder
        .export(
            specta_typescript::Typescript::default()
                .bigint(specta_typescript::BigIntExportBehavior::Number),
            "../frontend/src/lib/bindings.ts",
        )
        .expect("Failed to export typescript bindings");
    #[cfg(debug_assertions)]
    sanitize_typescript_bindings("../frontend/src/lib/bindings.ts")
        .expect("Failed to sanitize typescript bindings");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let config_manager = app.state::<config::ConfigManager>();
                        let config = config_manager.get_config();

                        // Parse both shortcuts for comparison
                        let spotlight_sc = Shortcut::from_str(&config.spotlight_shortcut)
                            .unwrap_or(Shortcut::new(
                                Some(Modifiers::SUPER | Modifiers::SHIFT),
                                Code::KeyK,
                            ));
                        let ptt_sc = Shortcut::from_str(&config.ptt_shortcut).unwrap_or(
                            Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyV),
                        );

                        if shortcut == &ptt_sc {
                            // Push-to-talk: emit event to frontend
                            let _ = app.emit("ptt_toggle", "pressed");
                        } else if shortcut == &spotlight_sc {
                            toggle_spotlight(app.clone());
                        } else {
                            // Fallback: toggle spotlight for unknown shortcuts
                            toggle_spotlight(app.clone());
                        }
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(specta_builder.invoke_handler())
        .setup(move |app| {
            specta_builder.mount_events(app);

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error building tauri application");

    app.manage(SidecarManager::new());
    app.manage(model_manager::DownloadManager::new());
    app.manage(config::ConfigManager::new(app.handle()));
    app.manage(thinclaw::ThinClawManager::new(app.handle().clone()));
    app.manage(rig_cache::RigManagerCache::new());

    // FileStore — centralized file I/O abstraction (local-first, cloud-ready)

    // Setup Logic
    {
        let handle = app.handle().clone();

        // 1. Database Init
        tauri::async_runtime::block_on(async move {
            let app_data_dir = handle
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");
            migrate_legacy_app_data(&app_data_dir);
            fs::create_dir_all(&app_data_dir).expect("failed to create app data dir");

            // ── Load ALL API keys from Keychain in a single read ─────────────
            // This triggers exactly one macOS authorization prompt, then caches
            // everything in memory.  Must happen before ThinClawConfig::new()
            // or any other code that calls keychain::get_key().
            thinclaw::config::keychain::load_all();

            // ── App-wide secret store (reads from the just-loaded keychain) ───
            let secret_store = secret_store::SecretStore::new();
            // InferenceRouter needs an Arc handle to the store.  Since
            // SecretStore is a zero-state wrapper over keychain (module-level
            // Mutex cache), a second instance is safe — they share the same
            // underlying cache.
            let secret_store_for_router = std::sync::Arc::new(secret_store::SecretStore::new());
            handle.manage(secret_store);

            // ── Inference Router — routes all AI modalities to backends ───
            let inference_router = inference::InferenceRouter::new(secret_store_for_router.clone());
            handle.manage(inference_router);

            // ── Cloud Model Discovery Registry ───
            let model_registry = inference::CloudModelRegistry::new(secret_store_for_router);
            handle.manage(model_registry);

            // Engine Manager — singleton inference engine instance
            let engine_manager = engine::EngineManager::new(app_data_dir.clone());
            handle.manage(engine_manager);

            // Process Tracker - Cleanup orphans from previous runs
            let tracker = process_tracker::ProcessTracker::new(app_data_dir.clone());
            tracker.cleanup_all();
            handle.manage(tracker);

            // Vector Store Manager Init (per-scope index files)
            // Use the dimension stored in user config (updated whenever a new
            // embedding model with a different hidden_size is loaded).
            let dims = handle
                .state::<config::ConfigManager>()
                .get_config()
                .vector_dimensions as usize;
            println!("[main] Initializing vector store with dimension {}.", dims);
            let vectors_dir = app_data_dir.join("vectors");
            let vector_manager = vector_store::VectorStoreManager::new(vectors_dir, dims)
                .expect("failed to init vector store manager");
            handle.manage(vector_manager);

            // Reranker Init (Downloads if needed)
            // Using RerankerWrapper to gracefully handle initialization failures.
            // This prevents crashes when RAG commands demand State<RerankerWrapper>.
            let reranker_wrapper = reranker::RerankerWrapper::new(app_data_dir.clone()).await;
            handle.manage(reranker_wrapper);

            let db_path = app_data_dir.join("thinclaw.db");
            let legacy_db = app_data_dir.join("scrappy.db");

            // Migration: rename legacy scrappy.db to thinclaw.db
            if !db_path.exists() && legacy_db.exists() {
                println!("[main] Migrating legacy scrappy.db to thinclaw.db...");
                let _ = fs::rename(&legacy_db, &db_path);
            }

            let db_url = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());

            let pool = SqlitePoolOptions::new()
                .max_connections(5)
                .connect(&db_url)
                .await
                .expect("failed to connect to database");

            sqlx::migrate!("./migrations")
                .run(&pool)
                .await
                .expect("failed to run migrations");

            handle.manage(pool);

            // Cloud Storage Manager
            let cloud_manager = cloud::CloudManager::new(app_data_dir.clone());
            {
                let pool_ref = handle.state::<sqlx::SqlitePool>();
                if let Err(e) = cloud_manager.init_from_db(&pool_ref).await {
                    eprintln!("[main] Cloud manager init warning: {}", e);
                }
            }
            handle.manage(cloud_manager);

            // FileStore — centralized file I/O (local-first, cloud-ready)
            let file_store = file_store::FileStore::new(app_data_dir.clone());
            handle.manage(file_store);

            // 2. Integrity Check
            println!("[main] Running Integrity Check...");
            let pool_state = handle.state::<sqlx::SqlitePool>();
            let vec_manager = handle.state::<vector_store::VectorStoreManager>();

            // Need to use inner because perform_integrity_check takes &T
            // State<T> derefs to T? Yes, but check signature.
            // perform_integrity_check(pool: &SqlitePool, ...)
            // Perform integrity check
            if let Err(e) = rag::perform_integrity_check(&pool_state, &vec_manager).await {
                eprintln!("[main] Integrity Check Failed: {}", e);
            }

            // Init ThinClaw config (critical for paths to work before engine start)
            let thinclaw_state = handle.state::<thinclaw::ThinClawManager>();
            if let Err(e) = thinclaw_state.init_config().await {
                eprintln!("[main] Failed to init ThinClaw config: {}", e);
            } else {
                // ThinClaw is in-process — no separate gateway to auto-start
            }

            // ── ThinClaw Engine Init (async — safe now that libsql bootstrap ran) ──
            // Pre-register the state container in "stopped" mode so all Tauri
            // commands can access it immediately. Then auto-start the engine.
            let ironclaw_state_dir = app_data_dir.clone();
            let ironclaw_state = thinclaw::runtime_bridge::ThinClawRuntimeState::new_stopped(
                handle.clone(),
                ironclaw_state_dir,
            );
            handle.manage(ironclaw_state);

            let ironclaw_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                use tauri::Manager;

                // ── Respect auto_start_gateway setting ───────────────────
                // Only auto-start ThinClaw if the user has explicitly enabled
                // it in Gateway Settings. When false, the user starts/stops
                // the engine manually via the Gateway panel.
                let should_auto_start = {
                    let thinclaw_mgr = ironclaw_handle.state::<thinclaw::ThinClawManager>();
                    let oc_config = thinclaw_mgr.get_config().await;
                    oc_config
                        .as_ref()
                        .map(|cfg| cfg.auto_start_gateway)
                        .unwrap_or(false)
                };

                if !should_auto_start {
                    println!(
                        "[main] ThinClaw auto-start disabled (auto_start_gateway=false). \
                              Start manually via Gateway settings."
                    );
                    return;
                }

                // If local inference is selected but no server is running yet,
                // wait for the engine to come online before starting ThinClaw.
                // This handles the common case where MLX boots after ThinClaw init.
                let needs_local_wait = {
                    let thinclaw_mgr = ironclaw_handle.state::<thinclaw::ThinClawManager>();
                    let oc_config = thinclaw_mgr.get_config().await;
                    if let Some(ref cfg) = oc_config {
                        if cfg.local_inference_enabled {
                            let sidecar_mgr = ironclaw_handle.state::<sidecar::SidecarManager>();
                            let has_sidecar = sidecar_mgr.get_chat_config().is_some();
                            let has_engine = {
                                let engine_mgr = ironclaw_handle.state::<engine::EngineManager>();
                                let guard = engine_mgr.engine.lock().await;
                                guard.as_ref().and_then(|e| e.base_url()).is_some()
                            };
                            !has_sidecar && !has_engine
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                if needs_local_wait {
                    println!("[main] ThinClaw: waiting for local inference engine (up to 45s)...");

                    let mut engine_ready = false;
                    for attempt in 1..=90 {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                        // Check sidecar (llamacpp builds)
                        let sidecar_mgr = ironclaw_handle.state::<sidecar::SidecarManager>();
                        if sidecar_mgr.get_chat_config().is_some() {
                            println!(
                                "[main] ThinClaw: sidecar detected after {}ms",
                                attempt * 500
                            );
                            engine_ready = true;
                            break;
                        }

                        // Check engine manager (MLX/vLLM/Ollama)
                        let engine_mgr = ironclaw_handle.state::<engine::EngineManager>();
                        let guard = engine_mgr.engine.lock().await;
                        if let Some(eng) = guard.as_ref() {
                            if eng.is_ready().await {
                                println!("[main] ThinClaw: engine ready after {}ms", attempt * 500);
                                engine_ready = true;
                                break;
                            }
                        }
                    }

                    if !engine_ready {
                        eprintln!(
                            "[main] ThinClaw auto-start skipped: local inference engine \
                             did not come online within 45s. Start manually via gateway."
                        );
                        return;
                    }
                }

                // Bridge ThinClaw Desktop's macOS Keychain to ThinClaw's SecretsStore trait.
                let secrets_store: Option<
                    std::sync::Arc<dyn thinclaw_core::secrets::SecretsStore + Send + Sync>,
                > = Some(std::sync::Arc::new(
                    thinclaw::secrets_adapter::KeychainSecretsAdapter::new(),
                ));

                let state =
                    ironclaw_handle.state::<thinclaw::runtime_bridge::ThinClawRuntimeState>();
                match state.start(secrets_store).await {
                    Ok(true) => {
                        println!("[main] ThinClaw runtime initialized successfully.");
                    }
                    Ok(false) => {
                        println!("[main] ThinClaw runtime was already running.");
                    }
                    Err(e) => {
                        eprintln!("[main] ThinClaw init failed (non-fatal): {}", e);
                    }
                }
            });
        });

        // 2. Tray Icon
        setup::tray::setup_tray(&app);

        // 3. Global Shortcuts
        setup::shortcuts::register_shortcuts(&app);
    }

    app.run(|_app_handle, _event| {
        match _event {
            tauri::RunEvent::WindowEvent {
                event: WindowEvent::CloseRequested { api, .. },
                ..
            } => {
                if let Some(window) = _app_handle.get_webview_window("main") {
                    let _ = window.hide();
                }
                api.prevent_close();
            }
            tauri::RunEvent::Exit => {
                // Shutdown ThinClaw runtime gracefully
                if let Some(state) =
                    _app_handle.try_state::<thinclaw::runtime_bridge::ThinClawRuntimeState>()
                {
                    tauri::async_runtime::block_on(state.shutdown());
                }
            }
            _ => {}
        }
    });
}
