/// Command registry — all Tauri IPC commands in one place.
///
/// Extracted from `lib.rs` to keep the entrypoint focused on app lifecycle.
/// Each command is declared via `tauri_specta::collect_commands!` and grouped
/// by domain for readability.

/// Build the Specta builder with all registered commands.
pub fn specta_builder() -> tauri_specta::Builder {
    tauri_specta::Builder::new().commands(tauri_specta::collect_commands![
        // ── Core ────────────────────────────────────────────────────────
        crate::greet,

        // ── Chat ────────────────────────────────────────────────────────
        crate::chat::chat_stream,
        crate::chat::chat_completion,
        crate::chat::count_tokens,

        // ── Sidecar Management ──────────────────────────────────────────
        crate::sidecar::start_chat_server,
        crate::sidecar::stop_chat_server,
        crate::sidecar::start_embedding_server,
        crate::sidecar::start_summarizer_server,
        crate::sidecar::get_sidecar_status,
        crate::sidecar::get_chat_server_config,
        crate::sidecar::start_stt_server,
        crate::sidecar::start_image_server,
        crate::sidecar::start_tts_server,
        crate::sidecar::cancel_generation,

        // ── Voice I/O ───────────────────────────────────────────────────
        crate::tts::tts_synthesize,
        crate::tts::tts_list_voices,
        crate::stt::transcribe_audio,

        // ── Web & Image ─────────────────────────────────────────────────
        crate::web_search::check_web_search,
        crate::image_gen::generate_image,

        // ── RAG ─────────────────────────────────────────────────────────
        crate::rag::ingest_document,
        crate::rag::upload_document,
        crate::rag::retrieve_context,
        crate::rag::check_vector_index_integrity,

        // ── Model Management ────────────────────────────────────────────
        crate::model_manager::list_models,
        crate::model_manager::download_model,
        crate::model_manager::cancel_download,
        crate::model_manager::check_model_path,
        crate::model_manager::open_models_folder,
        crate::model_manager::delete_local_model,
        crate::model_manager::open_url,
        crate::model_manager::check_missing_standard_assets,
        crate::model_manager::download_standard_asset,
        crate::model_manager::open_standard_models_folder,
        crate::model_manager::get_model_metadata,
        crate::model_manager::update_remote_model_catalog,
        crate::model_manager::get_remote_model_catalog,

        // ── History ─────────────────────────────────────────────────────
        crate::history::get_conversations,
        crate::history::create_conversation,
        crate::history::delete_conversation,
        crate::history::get_messages,
        crate::history::save_message,
        crate::history::edit_message,
        crate::history::update_conversation_title,
        crate::history::update_conversation_project,
        crate::history::update_conversations_order,
        crate::history::delete_all_history,

        // ── Config ──────────────────────────────────────────────────────
        crate::config::open_config_file,
        crate::config::get_user_config,
        crate::config::update_user_config,
        crate::config::get_hf_token,

        // ── Images & Imagine ────────────────────────────────────────────
        crate::images::upload_image,
        crate::images::load_image,
        crate::images::get_image_path,
        crate::images::open_images_folder,
        crate::imagine::imagine_generate,
        crate::imagine::imagine_list_images,
        crate::imagine::imagine_search_images,
        crate::imagine::imagine_toggle_favorite,
        crate::imagine::imagine_delete_image,
        crate::imagine::imagine_get_stats,

        // ── System & Projects ───────────────────────────────────────────
        crate::system::get_system_specs,
        crate::projects::create_project,
        crate::projects::list_projects,
        crate::projects::delete_project,
        crate::projects::update_project,
        crate::projects::update_projects_order,
        crate::projects::get_project_documents,
        crate::projects::delete_document,

        // ── Rig Agent ───────────────────────────────────────────────────
        crate::rig_lib::rig_check_web_search,
        crate::rig_lib::agent_chat,

        // ── OpenClaw / IronClaw ─────────────────────────────────────────
        crate::openclaw::commands::openclaw_get_status,
        crate::openclaw::commands::openclaw_save_anthropic_key,
        crate::openclaw::commands::openclaw_get_anthropic_key,
        crate::openclaw::commands::openclaw_save_brave_key,
        crate::openclaw::commands::openclaw_get_brave_key,
        crate::openclaw::commands::openclaw_save_openai_key,
        crate::openclaw::commands::openclaw_get_openai_key,
        crate::openclaw::commands::openclaw_save_openrouter_key,
        crate::openclaw::commands::openclaw_get_openrouter_key,
        crate::openclaw::commands::openclaw_save_gemini_key,
        crate::openclaw::commands::openclaw_get_gemini_key,
        crate::openclaw::commands::openclaw_save_groq_key,
        crate::openclaw::commands::openclaw_get_groq_key,
        crate::openclaw::commands::openclaw_save_selected_cloud_model,
        crate::openclaw::commands::select_openclaw_brain,
        crate::openclaw::commands::openclaw_save_cloud_config,
        crate::openclaw::commands::openclaw_toggle_secret_access,
        crate::openclaw::commands::openclaw_save_slack_config,
        crate::openclaw::commands::openclaw_save_telegram_config,
        crate::openclaw::commands::openclaw_save_gateway_settings,
        crate::openclaw::commands::openclaw_add_agent_profile,
        crate::openclaw::commands::openclaw_remove_agent_profile,
        crate::openclaw::commands::openclaw_switch_to_profile,
        crate::openclaw::commands::openclaw_test_connection,
        crate::openclaw::fleet::openclaw_get_fleet_status,
        crate::openclaw::fleet::openclaw_broadcast_command,
        crate::openclaw::commands::openclaw_start_gateway,
        crate::openclaw::commands::openclaw_stop_gateway,
        crate::openclaw::commands::openclaw_reload_secrets,
        crate::openclaw::commands::openclaw_get_sessions,
        crate::openclaw::commands::openclaw_get_history,
        crate::openclaw::commands::openclaw_delete_session,
        crate::openclaw::commands::openclaw_reset_session,
        crate::openclaw::commands::openclaw_send_message,
        crate::openclaw::commands::openclaw_subscribe_session,
        crate::openclaw::commands::openclaw_abort_chat,
        crate::openclaw::commands::openclaw_resolve_approval,
        crate::openclaw::commands::openclaw_get_diagnostics,
        crate::openclaw::commands::openclaw_clear_memory,
        crate::openclaw::commands::openclaw_get_memory,
        crate::openclaw::commands::openclaw_get_file,
        crate::openclaw::commands::openclaw_write_file,
        crate::openclaw::commands::openclaw_delete_file,
        crate::openclaw::commands::openclaw_save_memory,
        crate::openclaw::commands::openclaw_list_workspace_files,
        crate::openclaw::commands::openclaw_cron_list,
        crate::openclaw::commands::openclaw_cron_run,
        crate::openclaw::commands::openclaw_cron_history,
        crate::openclaw::commands::openclaw_cron_lint,
        crate::openclaw::commands::openclaw_routine_create,
        crate::openclaw::commands::openclaw_channels_list,
        crate::openclaw::commands::openclaw_skills_list,
        crate::openclaw::commands::openclaw_skills_status,
        crate::openclaw::commands::openclaw_skills_toggle,
        crate::openclaw::commands::openclaw_install_skill_repo,
        crate::openclaw::commands::openclaw_install_skill_deps,
        crate::openclaw::commands::openclaw_config_schema,
        crate::openclaw::commands::openclaw_config_get,
        crate::openclaw::commands::openclaw_config_set,
        crate::openclaw::commands::openclaw_config_patch,
        crate::openclaw::commands::openclaw_system_presence,
        crate::openclaw::commands::openclaw_logs_tail,
        crate::openclaw::commands::openclaw_update_run,
        crate::openclaw::commands::openclaw_web_login_whatsapp,
        crate::openclaw::commands::openclaw_web_login_telegram,
        crate::openclaw::commands::openclaw_add_custom_secret,
        crate::openclaw::commands::openclaw_remove_custom_secret,
        crate::openclaw::commands::openclaw_toggle_custom_secret,
        crate::openclaw::commands::openclaw_toggle_local_tools,
        crate::openclaw::commands::openclaw_set_workspace_mode,
        crate::openclaw::commands::openclaw_toggle_local_inference,
        crate::openclaw::commands::openclaw_toggle_expose_inference,
        crate::openclaw::commands::openclaw_set_setup_completed,
        crate::openclaw::commands::openclaw_toggle_auto_start,
        crate::openclaw::commands::openclaw_set_dev_mode_wizard,

        // Autonomy & bootstrap
        crate::openclaw::commands::openclaw_set_autonomy_mode,
        crate::openclaw::commands::openclaw_get_autonomy_mode,
        crate::openclaw::commands::openclaw_set_bootstrap_completed,
        crate::openclaw::commands::openclaw_check_bootstrap_needed,
        crate::openclaw::commands::openclaw_trigger_bootstrap,
        crate::openclaw::commands::openclaw_set_hf_token,
        crate::openclaw::commands::openclaw_save_implicit_provider_key,
        crate::openclaw::commands::openclaw_get_implicit_provider_key,
        crate::openclaw::commands::openclaw_save_bedrock_credentials,
        crate::openclaw::commands::openclaw_get_bedrock_credentials,
        crate::openclaw::commands::openclaw_sync_local_llm,
        crate::openclaw::deploy::openclaw_deploy_remote,

        // Orchestration & Canvas
        crate::openclaw::commands::openclaw_spawn_session,
        crate::openclaw::commands::openclaw_list_child_sessions,
        crate::openclaw::commands::openclaw_update_sub_agent_status,
        crate::openclaw::commands::openclaw_agents_list,
        crate::openclaw::commands::openclaw_canvas_push,
        crate::openclaw::commands::openclaw_canvas_navigate,
        crate::openclaw::commands::openclaw_canvas_panels_list,
        crate::openclaw::commands::openclaw_canvas_panel_get,
        crate::openclaw::commands::openclaw_canvas_panel_dismiss,
        crate::openclaw::commands::openclaw_routine_delete,
        crate::openclaw::commands::openclaw_routine_toggle,
        crate::openclaw::commands::openclaw_heartbeat_set_interval,

        // New feature commands
        crate::openclaw::commands::openclaw_set_thinking,
        crate::openclaw::commands::openclaw_memory_search,
        crate::openclaw::commands::openclaw_export_session,

        // Hooks & extensions management
        crate::openclaw::commands::openclaw_hooks_list,
        crate::openclaw::commands::openclaw_hooks_register,
        crate::openclaw::commands::openclaw_hooks_unregister,
        crate::openclaw::commands::openclaw_extensions_list,
        crate::openclaw::commands::openclaw_extension_activate,
        crate::openclaw::commands::openclaw_extension_remove,

        // Diagnostics & tools
        crate::openclaw::commands::openclaw_diagnostics,
        crate::openclaw::commands::openclaw_tools_list,
        crate::openclaw::commands::openclaw_tool_policy_get,
        crate::openclaw::commands::openclaw_tool_policy_set,

        // Pairing & compaction
        crate::openclaw::commands::openclaw_pairing_list,
        crate::openclaw::commands::openclaw_pairing_approve,
        crate::openclaw::commands::openclaw_compact_session,

        // Sprint 13 — New backend APIs
        crate::openclaw::commands::openclaw_cost_summary,
        crate::openclaw::commands::openclaw_cost_export_csv,
        crate::openclaw::commands::openclaw_cost_reset,
        crate::openclaw::commands::openclaw_channel_status_list,
        crate::openclaw::commands::openclaw_agents_set_default,
        crate::openclaw::commands::openclaw_clawhub_search,
        crate::openclaw::commands::openclaw_clawhub_install,
        crate::openclaw::commands::openclaw_routine_audit_list,
        crate::openclaw::commands::openclaw_clear_routine_runs,
        crate::openclaw::commands::openclaw_cache_stats,
        crate::openclaw::commands::openclaw_plugin_lifecycle_list,
        crate::openclaw::commands::openclaw_manifest_validate,
        crate::openclaw::commands::openclaw_routing_get,
        crate::openclaw::commands::openclaw_routing_set,
        crate::openclaw::commands::openclaw_routing_rules_list,
        crate::openclaw::commands::openclaw_routing_rules_save,
        crate::openclaw::commands::openclaw_routing_rules_add,
        crate::openclaw::commands::openclaw_routing_rules_remove,
        crate::openclaw::commands::openclaw_routing_rules_reorder,
        crate::openclaw::commands::openclaw_routing_status,
        crate::openclaw::commands::openclaw_gmail_oauth_start,
        crate::openclaw::commands::openclaw_gmail_status,

        // ── Permissions ─────────────────────────────────────────────────
        crate::permissions::get_permission_status,
        crate::permissions::request_permission,
        crate::permissions::open_permission_settings,

        // ── Spotlight ───────────────────────────────────────────────────
        crate::toggle_spotlight,
        crate::hide_spotlight,

        // ── Engine & HF Hub ─────────────────────────────────────────────
        crate::engine::get_active_engine_info,
        crate::engine::get_engine_setup_status,
        crate::engine::setup_engine,
        crate::engine::start_engine,
        crate::engine::stop_engine,
        crate::engine::is_engine_ready,
        crate::hf_hub::discover_hf_models,
        crate::hf_hub::get_model_files,
        crate::hf_hub::download_hf_model_files,
        crate::hf_hub::discover_embedding_dimension,

        // ── Inference Router ────────────────────────────────────────────
        crate::inference::get_inference_backends,
        crate::inference::update_inference_backend,
        crate::inference::discover_cloud_models,
        crate::inference::refresh_cloud_models,

        // ── Cloud Storage ───────────────────────────────────────────────
        crate::cloud::commands::cloud_get_status,
        crate::cloud::commands::cloud_test_connection,
        crate::cloud::commands::cloud_test_icloud,
        crate::cloud::commands::cloud_test_webdav,
        crate::cloud::commands::cloud_test_sftp,
        crate::cloud::commands::cloud_oauth_start,
        crate::cloud::commands::cloud_oauth_complete,
        crate::cloud::commands::cloud_migrate_to_cloud,
        crate::cloud::commands::cloud_migrate_to_local,
        crate::cloud::commands::cloud_cancel_migration,
        crate::cloud::commands::cloud_get_recovery_key,
        crate::cloud::commands::cloud_import_recovery_key,
        crate::cloud::commands::cloud_get_storage_breakdown,

        // ── Workspace ───────────────────────────────────────────────────
        crate::openclaw::commands::openclaw_get_workspace_path,
        crate::openclaw::commands::openclaw_reveal_workspace,
        crate::openclaw::commands::openclaw_list_agent_workspace_files,
        crate::openclaw::commands::openclaw_write_agent_workspace_file,
        crate::openclaw::commands::openclaw_reveal_file,
    ])
}
