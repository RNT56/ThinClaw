use sqlx::sqlite::SqlitePoolOptions;
use std::fs;

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
pub mod config;
pub mod engine;
pub mod gguf;
pub mod hf_hub;
mod history;
pub mod image_gen;
pub mod images;
pub mod imagine;
pub mod model_manager;
pub mod openclaw;
pub mod permissions;
pub mod personas;
pub mod process_tracker;
pub mod projects;
pub mod rag;
pub mod reranker;
pub mod rig_cache;
pub mod rig_lib;
pub mod secret_store;
pub mod sidecar;
pub mod stt;
pub mod system;
pub mod templates;
pub mod tts;
pub mod vector_store;
pub mod web_search;

use sidecar::SidecarManager;
use std::str::FromStr;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    let specta_builder = tauri_specta::Builder::new().commands(tauri_specta::collect_commands![
        greet,
        chat::chat_stream,
        chat::chat_completion,
        chat::count_tokens,
        sidecar::start_chat_server,
        sidecar::stop_chat_server,
        sidecar::start_embedding_server,
        sidecar::start_summarizer_server,
        sidecar::get_sidecar_status,
        sidecar::get_chat_server_config,
        sidecar::start_stt_server,
        sidecar::start_image_server,
        sidecar::start_tts_server,
        sidecar::cancel_generation,
        tts::tts_synthesize,
        web_search::check_web_search,
        image_gen::generate_image,
        stt::transcribe_audio,
        rag::ingest_document,
        rag::upload_document,
        rag::retrieve_context,
        rag::check_vector_index_integrity,
        model_manager::list_models,
        model_manager::download_model,
        model_manager::cancel_download,
        model_manager::check_model_path,
        model_manager::open_models_folder,
        model_manager::delete_local_model,
        model_manager::open_url,
        model_manager::check_missing_standard_assets,
        model_manager::download_standard_asset,
        model_manager::open_standard_models_folder,
        model_manager::get_model_metadata,
        model_manager::update_remote_model_catalog,
        model_manager::get_remote_model_catalog,
        history::get_conversations,
        config::open_config_file,
        config::get_user_config,
        config::update_user_config,
        config::get_hf_token,
        history::create_conversation,
        history::delete_conversation,
        history::get_messages,
        history::save_message,
        history::edit_message,
        history::update_conversation_title,
        history::update_conversation_project,
        history::update_conversations_order,
        history::delete_all_history,
        images::upload_image,
        images::load_image,
        images::get_image_path,
        images::open_images_folder,
        imagine::imagine_generate,
        imagine::imagine_list_images,
        imagine::imagine_search_images,
        imagine::imagine_toggle_favorite,
        imagine::imagine_delete_image,
        imagine::imagine_get_stats,
        system::get_system_specs,
        projects::create_project,
        projects::list_projects,
        projects::delete_project,
        projects::update_project,
        projects::update_projects_order,
        projects::get_project_documents,
        projects::delete_document,
        rig_lib::rig_check_web_search,
        rig_lib::agent_chat,
        // OpenClaw commands
        openclaw::commands::openclaw_get_status,
        openclaw::commands::openclaw_save_anthropic_key,
        openclaw::commands::openclaw_get_anthropic_key,
        openclaw::commands::openclaw_save_brave_key,
        openclaw::commands::openclaw_get_brave_key,
        openclaw::commands::openclaw_save_openai_key,
        openclaw::commands::openclaw_get_openai_key,
        openclaw::commands::openclaw_save_openrouter_key,
        openclaw::commands::openclaw_get_openrouter_key,
        openclaw::commands::openclaw_save_gemini_key,
        openclaw::commands::openclaw_get_gemini_key,
        openclaw::commands::openclaw_save_groq_key,
        openclaw::commands::openclaw_get_groq_key,
        openclaw::commands::openclaw_save_selected_cloud_model,
        openclaw::commands::select_openclaw_brain,
        openclaw::commands::openclaw_save_cloud_config,
        openclaw::commands::openclaw_toggle_secret_access,
        openclaw::commands::openclaw_save_slack_config,
        openclaw::commands::openclaw_save_telegram_config,
        openclaw::commands::openclaw_save_gateway_settings,
        openclaw::commands::openclaw_add_agent_profile,
        openclaw::commands::openclaw_remove_agent_profile,
        openclaw::extra_commands::openclaw_switch_to_profile,
        openclaw::extra_commands::openclaw_test_connection,
        openclaw::fleet::openclaw_get_fleet_status,
        openclaw::fleet::openclaw_broadcast_command,
        openclaw::commands::openclaw_start_gateway,
        openclaw::commands::openclaw_stop_gateway,
        openclaw::commands::openclaw_get_sessions,
        openclaw::commands::openclaw_get_history,
        openclaw::commands::openclaw_delete_session,
        openclaw::commands::openclaw_reset_session,
        openclaw::commands::openclaw_send_message,
        openclaw::commands::openclaw_subscribe_session,
        openclaw::commands::openclaw_abort_chat,
        openclaw::commands::openclaw_resolve_approval,
        openclaw::commands::openclaw_get_diagnostics,
        openclaw::commands::openclaw_clear_memory,
        openclaw::commands::openclaw_get_memory,
        openclaw::commands::openclaw_get_file,
        openclaw::commands::openclaw_write_file,
        openclaw::commands::openclaw_save_memory,
        openclaw::commands::openclaw_list_workspace_files,
        openclaw::commands::openclaw_cron_list,
        openclaw::commands::openclaw_cron_run,
        openclaw::commands::openclaw_cron_history,
        openclaw::commands::openclaw_skills_list,
        openclaw::commands::openclaw_skills_status,
        openclaw::commands::openclaw_skills_toggle,
        openclaw::commands::openclaw_install_skill_repo,
        openclaw::commands::openclaw_install_skill_deps,
        openclaw::commands::openclaw_config_schema,
        openclaw::commands::openclaw_config_get,
        openclaw::commands::openclaw_config_set,
        openclaw::commands::openclaw_config_patch,
        openclaw::commands::openclaw_system_presence,
        openclaw::commands::openclaw_logs_tail,
        openclaw::commands::openclaw_update_run,
        openclaw::commands::openclaw_web_login_whatsapp,
        openclaw::commands::openclaw_web_login_telegram,
        openclaw::commands::openclaw_add_custom_secret,
        openclaw::commands::openclaw_remove_custom_secret,
        openclaw::commands::openclaw_toggle_custom_secret,
        openclaw::commands::openclaw_toggle_node_host,
        openclaw::commands::openclaw_toggle_local_inference,
        openclaw::commands::openclaw_toggle_expose_inference,
        openclaw::commands::openclaw_set_setup_completed,
        openclaw::commands::openclaw_toggle_auto_start,
        openclaw::commands::openclaw_set_dev_mode_wizard,
        openclaw::commands::openclaw_set_hf_token,
        openclaw::commands::openclaw_save_implicit_provider_key,
        openclaw::commands::openclaw_get_implicit_provider_key,
        openclaw::commands::openclaw_save_bedrock_credentials,
        openclaw::commands::openclaw_get_bedrock_credentials,
        openclaw::commands::openclaw_sync_local_llm,
        openclaw::deploy::openclaw_deploy_remote,
        // Orchestration & Canvas
        openclaw::commands::openclaw_spawn_session,
        openclaw::commands::openclaw_agents_list,
        openclaw::commands::openclaw_canvas_push,
        openclaw::commands::openclaw_canvas_navigate,
        permissions::get_permission_status,
        permissions::request_permission,
        toggle_spotlight,
        hide_spotlight,
        // Engine & HF Hub
        engine::get_active_engine_info,
        engine::get_engine_setup_status,
        engine::setup_engine,
        engine::start_engine,
        engine::stop_engine,
        engine::is_engine_ready,
        hf_hub::discover_hf_models,
        hf_hub::get_model_files,
        hf_hub::download_hf_model_files,
    ]);

    #[cfg(debug_assertions)]
    specta_builder
        .export(
            specta_typescript::Typescript::default(),
            "../frontend/src/lib/bindings.ts",
        )
        .expect("Failed to export typescript bindings");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let config_manager = app.state::<config::ConfigManager>();
                        let _sc_str = config_manager.get_config().spotlight_shortcut;

                        // If the shortcut matches "Command+Shift+K" (or whatever is in config)
                        // In a real app we'd compare the shortcut object or its string representation.
                        // For now, let's just trigger toggle_spotlight if ANY registered shortcut is pressed,
                        // or check if it matches our intended one.
                        toggle_spotlight(app.clone());
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
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
    app.manage(openclaw::OpenClawManager::new(app.handle().clone()));
    app.manage(rig_cache::RigManagerCache::new());

    // Setup Logic
    {
        let handle = app.handle().clone();

        // 1. Database Init
        tauri::async_runtime::block_on(async move {
            let app_data_dir = handle
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");
            fs::create_dir_all(&app_data_dir).expect("failed to create app data dir");

            // ── Load ALL API keys from Keychain in a single read ─────────────
            // This triggers exactly one macOS authorization prompt, then caches
            // everything in memory.  Must happen before OpenClawConfig::new()
            // or any other code that calls keychain::get_key().
            openclaw::config::keychain::load_all();

            // ── App-wide secret store (reads from the just-loaded keychain) ───
            let secret_store = secret_store::SecretStore::new();
            handle.manage(secret_store);

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
            // This prevents crashes when RAG commands demand State<RerankerWrapper>.
            let reranker_wrapper = reranker::RerankerWrapper::new(app_data_dir.clone()).await;
            handle.manage(reranker_wrapper);

            let db_path = app_data_dir.join("openclaw.db");
            let legacy_db = app_data_dir.join("scrappy.db");

            // Migration: rename legacy scrappy.db to openclaw.db
            if !db_path.exists() && legacy_db.exists() {
                println!("[main] Migrating legacy scrappy.db to openclaw.db...");
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

            // Init OpenClaw Config (Critical for paths to work before gateway start)
            let openclaw_state = handle.state::<openclaw::OpenClawManager>();
            if let Err(e) = openclaw_state.init_config().await {
                eprintln!("[main] Failed to init OpenClaw config: {}", e);
            } else {
                // Check if we should auto-start the gateway
                if let Some(cfg) = openclaw_state.get_config().await {
                    if cfg.auto_start_gateway {
                        println!("[main] Auto-starting OpenClaw gateway...");
                        let handle_clone = handle.clone();

                        tauri::async_runtime::spawn(async move {
                            let openclaw_mgr = handle_clone.state::<openclaw::OpenClawManager>();
                            let sidecar_mgr = handle_clone.state::<SidecarManager>();

                            if let Err(e) =
                                openclaw::commands::start_gateway_core(&openclaw_mgr, &sidecar_mgr)
                                    .await
                            {
                                eprintln!("[main] Failed to auto-start OpenClaw gateway: {}", e);
                            } else {
                                println!("[main] OpenClaw gateway auto-started successfully.");
                            }
                        });
                    }
                }
            }
        });

        // 2. Tray Icon
        let quit_i = MenuItem::with_id(&app, "quit", "Quit", true, None::<&str>);
        let show_i = MenuItem::with_id(&app, "show", "Show OpenClaw", true, None::<&str>);

        if let (Ok(quit_i), Ok(show_i)) = (quit_i, show_i) {
            let menu = Menu::with_items(&app, &[&show_i, &quit_i]);
            if let Ok(menu) = menu {
                let tray_icon = tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/tray-iconTemplate.png"
                ))
                .expect("failed to load tray icon");

                let _ = TrayIconBuilder::new()
                    .icon(tray_icon)
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "quit" => app.exit(0),
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: tauri::tray::MouseButton::Left,
                            ..
                        } = event
                        {
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    })
                    .build(&app);
            }
        }

        // 3. Global Shortcut
        let config_manager = app.state::<config::ConfigManager>();
        let shortcut_str = config_manager.get_config().spotlight_shortcut;

        if let Ok(shortcut) = Shortcut::from_str(&shortcut_str) {
            let _ = app.global_shortcut().register(shortcut);
        } else {
            // Fallback
            let shortcut = Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyK);
            let _ = app.global_shortcut().register(shortcut);
        }
    }

    app.run(|_app_handle, _event| {
        if let tauri::RunEvent::WindowEvent {
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } = _event
        {
            if let Some(window) = _app_handle.get_webview_window("main") {
                let _ = window.hide();
            }
            api.prevent_close();
        }
    });
}
