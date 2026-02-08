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
        .resizable(false)
        .always_on_top(true)
        .visible(false)
        .inner_size(800.0, 600.0)
        .center()
        .skip_taskbar(true);

        #[cfg(target_os = "macos")]
        {
            win_builder = win_builder
                .hidden_title(true)
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .visible_on_all_workspaces(true);
        }

        if let Ok(window) = win_builder.build() {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

pub mod chat;
pub mod clawdbot;
pub mod config;
pub mod gguf;
mod history;
pub mod image_gen;
pub mod images;
pub mod model_manager;
pub mod permissions;
pub mod personas;
pub mod process_tracker;
pub mod projects;
pub mod rag;
pub mod reranker;
pub mod rig_lib;
pub mod sidecar;
pub mod stt;
pub mod system;
pub mod templates;
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
        // Clawdbot commands
        clawdbot::commands::get_clawdbot_status,
        clawdbot::commands::save_anthropic_key,
        clawdbot::commands::get_anthropic_key,
        clawdbot::commands::save_brave_key,
        clawdbot::commands::get_brave_key,
        clawdbot::commands::save_openai_key,
        clawdbot::commands::get_openai_key,
        clawdbot::commands::save_openrouter_key,
        clawdbot::commands::get_openrouter_key,
        clawdbot::commands::save_gemini_key,
        clawdbot::commands::get_gemini_key,
        clawdbot::commands::save_groq_key,
        clawdbot::commands::get_groq_key,
        clawdbot::commands::save_selected_cloud_model,
        clawdbot::commands::clawdbot_toggle_secret_access,
        clawdbot::commands::save_slack_config,
        clawdbot::commands::save_telegram_config,
        clawdbot::commands::save_gateway_settings,
        clawdbot::commands::start_clawdbot_gateway,
        clawdbot::commands::stop_clawdbot_gateway,
        clawdbot::commands::get_clawdbot_sessions,
        clawdbot::commands::get_clawdbot_history,
        clawdbot::commands::delete_clawdbot_session,
        clawdbot::commands::send_clawdbot_message,
        clawdbot::commands::subscribe_clawdbot_session,
        clawdbot::commands::abort_clawdbot_chat,
        clawdbot::commands::resolve_clawdbot_approval,
        clawdbot::commands::get_clawdbot_diagnostics,
        clawdbot::commands::clear_clawdbot_memory,
        clawdbot::commands::get_clawdbot_memory,
        clawdbot::commands::get_clawdbot_file,
        clawdbot::commands::write_clawdbot_file,
        clawdbot::commands::save_clawdbot_memory,
        clawdbot::commands::list_workspace_files,
        clawdbot::commands::clawdbot_cron_list,
        clawdbot::commands::clawdbot_cron_run,
        clawdbot::commands::clawdbot_cron_history,
        clawdbot::commands::clawdbot_skills_list,
        clawdbot::commands::clawdbot_skills_status,
        clawdbot::commands::clawdbot_skills_toggle,
        clawdbot::commands::clawdbot_install_skill_repo,
        clawdbot::commands::clawdbot_install_skill_deps,
        clawdbot::commands::clawdbot_config_schema,
        clawdbot::commands::clawdbot_config_get,
        clawdbot::commands::clawdbot_config_set,
        clawdbot::commands::clawdbot_config_patch,
        clawdbot::commands::clawdbot_system_presence,
        clawdbot::commands::clawdbot_logs_tail,
        clawdbot::commands::clawdbot_update_run,
        clawdbot::commands::clawdbot_web_login_whatsapp,
        clawdbot::commands::clawdbot_web_login_telegram,
        clawdbot::commands::add_custom_secret,
        clawdbot::commands::remove_custom_secret,
        clawdbot::commands::clawdbot_toggle_custom_secret,
        clawdbot::commands::clawdbot_toggle_node_host,
        clawdbot::commands::clawdbot_toggle_local_inference,
        clawdbot::commands::clawdbot_toggle_expose_inference,
        clawdbot::commands::clawdbot_set_setup_completed,
        clawdbot::commands::clawdbot_toggle_auto_start,
        clawdbot::commands::clawdbot_set_dev_mode_wizard,
        clawdbot::commands::set_hf_token,
        permissions::get_permission_status,
        permissions::request_permission,
        toggle_spotlight,
        hide_spotlight,
    ]);

    #[cfg(debug_assertions)]
    specta_builder
        .export(
            specta_typescript::Typescript::default().header("// @ts-nocheck\n"),
            "../src/lib/bindings.ts",
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
    app.manage(clawdbot::ClawdbotManager::new(app.handle().clone()));

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

            // Process Tracker - Cleanup orphans from previous runs
            let tracker = process_tracker::ProcessTracker::new(app_data_dir.clone());
            tracker.cleanup_all();
            handle.manage(tracker);

            // Vector Store Init
            // We use the dimension in the filename to automatically "reset" if we switch defaults
            let dims = 384;
            let vector_path = app_data_dir.join(format!("vector_index_{}.usearch", dims));
            let vector_store = vector_store::VectorStore::new(vector_path, dims)
                .expect("failed to init vector store");
            handle.manage(vector_store);

            // Reranker Init (Downloads if needed)
            // Using RerankerWrapper to gracefully handle initialization failures.
            // This prevents crashes when RAG commands demand State<RerankerWrapper>.
            // This prevents crashes when RAG commands demand State<RerankerWrapper>.
            let reranker_wrapper = reranker::RerankerWrapper::new(app_data_dir.clone()).await;
            handle.manage(reranker_wrapper);

            let db_path = app_data_dir.join("scrappy.db");
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
            let vec_state = handle.state::<vector_store::VectorStore>();

            // Need to use inner because perform_integrity_check takes &T
            // State<T> derefs to T? Yes, but check signature.
            // perform_integrity_check(pool: &SqlitePool, ...)
            // Perform integrity check
            if let Err(e) = rag::perform_integrity_check(&pool_state, &vec_state).await {
                eprintln!("[main] Integrity Check Failed: {}", e);
            }

            // Init Clawdbot Config (Critical for paths to work before gateway start)
            let clawdbot_state = handle.state::<clawdbot::ClawdbotManager>();
            if let Err(e) = clawdbot_state.init_config().await {
                eprintln!("[main] Failed to init Clawdbot config: {}", e);
            } else {
                // Check if we should auto-start the gateway
                if let Some(cfg) = clawdbot_state.get_config().await {
                    if cfg.auto_start_gateway {
                        println!("[main] Auto-starting Clawdbot gateway...");
                        let handle_clone = handle.clone();

                        tauri::async_runtime::spawn(async move {
                            let clawdbot_mgr = handle_clone.state::<clawdbot::ClawdbotManager>();
                            let sidecar_mgr = handle_clone.state::<SidecarManager>();

                            if let Err(e) =
                                clawdbot::commands::start_gateway_core(&clawdbot_mgr, &sidecar_mgr)
                                    .await
                            {
                                eprintln!("[main] Failed to auto-start Clawdbot gateway: {}", e);
                            } else {
                                println!("[main] Clawdbot gateway auto-started successfully.");
                            }
                        });
                    }
                }
            }
        });

        // 2. Tray Icon
        let quit_i = MenuItem::with_id(&app, "quit", "Quit", true, None::<&str>);
        let show_i = MenuItem::with_id(&app, "show", "Show Scrappy", true, None::<&str>);

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
