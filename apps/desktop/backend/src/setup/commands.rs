//! Command registry — all Tauri IPC commands in one place.
//!
//! Extracted from `lib.rs` to keep the entrypoint focused on app lifecycle.
//! Each command is declared via `tauri_specta::collect_commands!` and grouped
//! by domain for readability.

/// Build the Specta builder with all registered commands.
pub fn specta_builder() -> tauri_specta::Builder {
    let builder = tauri_specta::Builder::new().typ::<crate::thinclaw::ui_types::UiEvent>();
    builder.commands(tauri_specta::collect_commands![
        // ── Core ────────────────────────────────────────────────────────
        crate::greet,
        // ── Chat ────────────────────────────────────────────────────────
        crate::chat::direct_chat_stream,
        crate::chat::direct_chat_completion,
        crate::chat::direct_chat_count_tokens,
        // ── Sidecar Management ──────────────────────────────────────────
        crate::sidecar::direct_runtime_start_chat_server,
        crate::sidecar::direct_runtime_stop_chat_server,
        crate::sidecar::direct_runtime_start_embedding_server,
        crate::sidecar::direct_runtime_start_summarizer_server,
        crate::sidecar::direct_runtime_get_sidecar_status,
        crate::sidecar::direct_runtime_get_chat_server_config,
        crate::sidecar::direct_runtime_start_stt_server,
        crate::sidecar::direct_runtime_start_image_server,
        crate::sidecar::direct_runtime_start_tts_server,
        crate::sidecar::direct_runtime_cancel_generation,
        // ── Voice I/O ───────────────────────────────────────────────────
        crate::tts::direct_media_tts_synthesize,
        crate::tts::direct_media_tts_list_voices,
        crate::stt::direct_media_transcribe_audio,
        // ── Web & Image ─────────────────────────────────────────────────
        crate::image_gen::direct_media_generate_image,
        // ── RAG ─────────────────────────────────────────────────────────
        crate::rag::direct_rag_ingest_document,
        crate::rag::direct_rag_upload_document,
        crate::rag::direct_rag_retrieve_context,
        crate::rag::direct_rag_check_vector_index_integrity,
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
        crate::history::direct_history_get_conversations,
        crate::history::direct_history_create_conversation,
        crate::history::direct_history_delete_conversation,
        crate::history::direct_history_get_messages,
        crate::history::direct_history_save_message,
        crate::history::direct_history_edit_message,
        crate::history::direct_history_update_conversation_title,
        crate::history::direct_history_update_conversation_project,
        crate::history::direct_history_update_conversations_order,
        crate::history::direct_history_delete_all_history,
        // ── Config ──────────────────────────────────────────────────────
        crate::config::open_config_file,
        crate::config::get_user_config,
        crate::config::update_user_config,
        crate::config::get_hf_token,
        // ── Images & Imagine ────────────────────────────────────────────
        crate::images::direct_assets_upload_image,
        crate::images::direct_assets_load_image,
        crate::images::direct_assets_get_image_path,
        crate::images::direct_assets_open_images_folder,
        crate::imagine::direct_imagine_generate,
        crate::imagine::direct_imagine_list_images,
        crate::imagine::direct_imagine_search_images,
        crate::imagine::direct_imagine_toggle_favorite,
        crate::imagine::direct_imagine_delete_image,
        crate::imagine::direct_imagine_get_stats,
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
        crate::rig_lib::agent_chat,
        // ── ThinClaw Agent Cockpit ──────────────────────────────────────
        crate::thinclaw::commands::thinclaw_get_status,
        crate::thinclaw::commands::thinclaw_save_anthropic_key,
        crate::thinclaw::commands::thinclaw_get_anthropic_key,
        crate::thinclaw::commands::thinclaw_save_brave_key,
        crate::thinclaw::commands::thinclaw_get_brave_key,
        crate::thinclaw::commands::thinclaw_save_openai_key,
        crate::thinclaw::commands::thinclaw_get_openai_key,
        crate::thinclaw::commands::thinclaw_save_openrouter_key,
        crate::thinclaw::commands::thinclaw_get_openrouter_key,
        crate::thinclaw::commands::thinclaw_save_gemini_key,
        crate::thinclaw::commands::thinclaw_get_gemini_key,
        crate::thinclaw::commands::thinclaw_save_groq_key,
        crate::thinclaw::commands::thinclaw_get_groq_key,
        crate::thinclaw::commands::thinclaw_save_selected_cloud_model,
        crate::thinclaw::commands::select_thinclaw_brain,
        crate::thinclaw::commands::thinclaw_save_cloud_config,
        crate::thinclaw::commands::thinclaw_toggle_secret_access,
        crate::thinclaw::commands::thinclaw_save_slack_config,
        crate::thinclaw::commands::thinclaw_save_telegram_config,
        crate::thinclaw::commands::thinclaw_save_gateway_settings,
        crate::thinclaw::commands::thinclaw_add_agent_profile,
        crate::thinclaw::commands::thinclaw_remove_agent_profile,
        crate::thinclaw::commands::thinclaw_switch_to_profile,
        crate::thinclaw::commands::thinclaw_test_connection,
        crate::thinclaw::fleet::thinclaw_get_fleet_status,
        crate::thinclaw::fleet::thinclaw_broadcast_command,
        crate::thinclaw::commands::thinclaw_start_gateway,
        crate::thinclaw::commands::thinclaw_stop_gateway,
        crate::thinclaw::commands::thinclaw_reload_secrets,
        crate::thinclaw::commands::thinclaw_get_sessions,
        crate::thinclaw::commands::thinclaw_get_history,
        crate::thinclaw::commands::thinclaw_delete_session,
        crate::thinclaw::commands::thinclaw_reset_session,
        crate::thinclaw::commands::thinclaw_send_message,
        crate::thinclaw::commands::thinclaw_subscribe_session,
        crate::thinclaw::commands::thinclaw_abort_chat,
        crate::thinclaw::commands::thinclaw_undo,
        crate::thinclaw::commands::thinclaw_redo,
        crate::thinclaw::commands::thinclaw_resolve_approval,
        crate::thinclaw::commands::thinclaw_get_diagnostics,
        crate::thinclaw::commands::thinclaw_clear_memory,
        crate::thinclaw::commands::thinclaw_get_memory,
        crate::thinclaw::commands::thinclaw_get_file,
        crate::thinclaw::commands::thinclaw_write_file,
        crate::thinclaw::commands::thinclaw_delete_file,
        crate::thinclaw::commands::thinclaw_save_memory,
        crate::thinclaw::commands::thinclaw_list_workspace_files,
        crate::thinclaw::commands::thinclaw_cron_list,
        crate::thinclaw::commands::thinclaw_cron_run,
        crate::thinclaw::commands::thinclaw_cron_history,
        crate::thinclaw::commands::thinclaw_cron_lint,
        crate::thinclaw::commands::thinclaw_routine_create,
        crate::thinclaw::commands::thinclaw_channels_list,
        crate::thinclaw::commands::thinclaw_skills_list,
        crate::thinclaw::commands::thinclaw_skills_status,
        crate::thinclaw::commands::thinclaw_skills_toggle,
        crate::thinclaw::commands::thinclaw_skills_search,
        crate::thinclaw::commands::thinclaw_skill_install,
        crate::thinclaw::commands::thinclaw_skill_remove,
        crate::thinclaw::commands::thinclaw_skill_trust,
        crate::thinclaw::commands::thinclaw_skill_reload,
        crate::thinclaw::commands::thinclaw_skills_reload_all,
        crate::thinclaw::commands::thinclaw_skill_inspect,
        crate::thinclaw::commands::thinclaw_skill_publish,
        crate::thinclaw::commands::thinclaw_install_skill_repo,
        crate::thinclaw::commands::thinclaw_install_skill_deps,
        crate::thinclaw::commands::thinclaw_config_schema,
        crate::thinclaw::commands::thinclaw_config_get,
        crate::thinclaw::commands::thinclaw_config_set,
        crate::thinclaw::commands::thinclaw_config_patch,
        crate::thinclaw::commands::thinclaw_system_presence,
        crate::thinclaw::commands::thinclaw_logs_tail,
        crate::thinclaw::commands::thinclaw_update_run,
        crate::thinclaw::commands::thinclaw_add_custom_secret,
        crate::thinclaw::commands::thinclaw_update_custom_secret,
        crate::thinclaw::commands::thinclaw_remove_custom_secret,
        crate::thinclaw::commands::thinclaw_toggle_custom_secret,
        crate::thinclaw::commands::thinclaw_toggle_local_tools,
        crate::thinclaw::commands::thinclaw_set_workspace_mode,
        crate::thinclaw::commands::thinclaw_toggle_local_inference,
        crate::thinclaw::commands::thinclaw_toggle_expose_inference,
        crate::thinclaw::commands::thinclaw_set_setup_completed,
        crate::thinclaw::commands::thinclaw_toggle_auto_start,
        crate::thinclaw::commands::thinclaw_set_dev_mode_wizard,
        // Autonomy & bootstrap
        crate::thinclaw::commands::thinclaw_set_autonomy_mode,
        crate::thinclaw::commands::thinclaw_get_autonomy_mode,
        crate::thinclaw::commands::thinclaw_set_bootstrap_completed,
        crate::thinclaw::commands::thinclaw_check_bootstrap_needed,
        crate::thinclaw::commands::thinclaw_trigger_bootstrap,
        crate::thinclaw::commands::thinclaw_jobs_list,
        crate::thinclaw::commands::thinclaw_jobs_summary,
        crate::thinclaw::commands::thinclaw_job_detail,
        crate::thinclaw::commands::thinclaw_job_cancel,
        crate::thinclaw::commands::thinclaw_job_restart,
        crate::thinclaw::commands::thinclaw_job_prompt,
        crate::thinclaw::commands::thinclaw_job_events,
        crate::thinclaw::commands::thinclaw_job_files_list,
        crate::thinclaw::commands::thinclaw_job_file_read,
        crate::thinclaw::commands::thinclaw_repo_projects_list,
        crate::thinclaw::commands::thinclaw_repo_project_get,
        crate::thinclaw::commands::thinclaw_repo_project_create,
        crate::thinclaw::commands::thinclaw_repo_project_plan,
        crate::thinclaw::commands::thinclaw_repo_project_start,
        crate::thinclaw::commands::thinclaw_repo_project_pause,
        crate::thinclaw::commands::thinclaw_repo_project_resume,
        crate::thinclaw::commands::thinclaw_repo_project_cancel,
        crate::thinclaw::commands::thinclaw_repo_project_approve,
        crate::thinclaw::commands::thinclaw_repo_project_enqueue,
        crate::thinclaw::commands::thinclaw_repo_project_events,
        crate::thinclaw::commands::thinclaw_repo_project_merge_gates,
        crate::thinclaw::commands::thinclaw_repo_projects_readiness,
        crate::thinclaw::commands::thinclaw_repo_projects_setup,
        crate::thinclaw::commands::thinclaw_repo_projects_set_credential,
        crate::thinclaw::commands::thinclaw_repo_projects_connectable_repos,
        crate::thinclaw::commands::thinclaw_repo_projects_connect,
        crate::thinclaw::commands::thinclaw_repo_project_enroll,
        crate::thinclaw::commands::thinclaw_autonomy_status,
        crate::thinclaw::commands::thinclaw_autonomy_bootstrap,
        crate::thinclaw::commands::thinclaw_autonomy_pause,
        crate::thinclaw::commands::thinclaw_autonomy_resume,
        crate::thinclaw::commands::thinclaw_autonomy_permissions,
        crate::thinclaw::commands::thinclaw_desktop_permission_status,
        crate::thinclaw::commands::thinclaw_autonomy_rollback,
        crate::thinclaw::commands::thinclaw_autonomy_rollouts,
        crate::thinclaw::commands::thinclaw_autonomy_checks,
        crate::thinclaw::commands::thinclaw_autonomy_evidence,
        crate::thinclaw::commands::thinclaw_set_hf_token,
        crate::thinclaw::commands::thinclaw_save_implicit_provider_key,
        crate::thinclaw::commands::thinclaw_get_implicit_provider_key,
        crate::thinclaw::commands::thinclaw_save_bedrock_credentials,
        crate::thinclaw::commands::thinclaw_get_bedrock_credentials,
        crate::thinclaw::commands::thinclaw_sync_local_llm,
        crate::thinclaw::deploy::thinclaw_deploy_remote,
        // Orchestration & Canvas
        crate::thinclaw::commands::thinclaw_spawn_session,
        crate::thinclaw::commands::thinclaw_list_child_sessions,
        crate::thinclaw::commands::thinclaw_update_sub_agent_status,
        crate::thinclaw::commands::thinclaw_agents_list,
        crate::thinclaw::commands::thinclaw_canvas_push,
        crate::thinclaw::commands::thinclaw_canvas_navigate,
        crate::thinclaw::commands::thinclaw_canvas_dispatch_event,
        crate::thinclaw::commands::thinclaw_canvas_panels_list,
        crate::thinclaw::commands::thinclaw_canvas_panel_get,
        crate::thinclaw::commands::thinclaw_canvas_panel_dismiss,
        crate::thinclaw::commands::thinclaw_routine_delete,
        crate::thinclaw::commands::thinclaw_routine_toggle,
        crate::thinclaw::commands::thinclaw_heartbeat_set_interval,
        // New feature commands
        crate::thinclaw::commands::thinclaw_set_thinking,
        crate::thinclaw::commands::thinclaw_memory_search,
        crate::thinclaw::commands::thinclaw_session_search,
        crate::thinclaw::commands::thinclaw_export_session,
        // Hooks & extensions management
        crate::thinclaw::commands::thinclaw_hooks_list,
        crate::thinclaw::commands::thinclaw_hooks_register,
        crate::thinclaw::commands::thinclaw_hooks_unregister,
        crate::thinclaw::commands::thinclaw_extensions_list,
        crate::thinclaw::commands::thinclaw_extension_install,
        crate::thinclaw::commands::thinclaw_extension_activate,
        crate::thinclaw::commands::thinclaw_extension_reconnect,
        crate::thinclaw::commands::thinclaw_extension_setup_get,
        crate::thinclaw::commands::thinclaw_extension_setup_submit,
        crate::thinclaw::commands::thinclaw_extension_validate_setup,
        crate::thinclaw::commands::thinclaw_extension_remove,
        crate::thinclaw::commands::thinclaw_extension_registry_search,
        crate::thinclaw::commands::thinclaw_mcp_servers,
        crate::thinclaw::commands::thinclaw_mcp_server,
        crate::thinclaw::commands::thinclaw_mcp_server_tools,
        crate::thinclaw::commands::thinclaw_mcp_server_resources,
        crate::thinclaw::commands::thinclaw_mcp_read_resource,
        crate::thinclaw::commands::thinclaw_mcp_resource_templates,
        crate::thinclaw::commands::thinclaw_mcp_server_prompts,
        crate::thinclaw::commands::thinclaw_mcp_get_prompt,
        crate::thinclaw::commands::thinclaw_mcp_oauth,
        crate::thinclaw::commands::thinclaw_mcp_set_log_level,
        crate::thinclaw::commands::thinclaw_mcp_interactions,
        crate::thinclaw::commands::thinclaw_mcp_interaction_respond,
        // Diagnostics & tools
        crate::thinclaw::commands::thinclaw_diagnostics,
        crate::thinclaw::commands::thinclaw_security_posture,
        crate::thinclaw::commands::thinclaw_secret_recovery_status,
        crate::thinclaw::commands::thinclaw_secret_recovery_export,
        crate::thinclaw::commands::thinclaw_secret_master_key_rotate,
        crate::thinclaw::commands::thinclaw_secret_recovery_import,
        crate::thinclaw::commands::thinclaw_tools_list,
        crate::thinclaw::commands::thinclaw_tool_policy_get,
        crate::thinclaw::commands::thinclaw_tool_policy_set,
        // Pairing & compaction
        crate::thinclaw::commands::thinclaw_pairing_list,
        crate::thinclaw::commands::thinclaw_pairing_approve,
        crate::thinclaw::commands::thinclaw_compact_session,
        // Filesystem checkpoints / rollback (TDO-103)
        crate::thinclaw::commands::thinclaw_checkpoints_list,
        crate::thinclaw::commands::thinclaw_checkpoint_diff,
        crate::thinclaw::commands::thinclaw_checkpoint_restore,
        // Trajectory viewer (TDO-106)
        crate::thinclaw::commands::thinclaw_trajectory_stats,
        crate::thinclaw::commands::thinclaw_trajectory_records,
        crate::thinclaw::commands::thinclaw_trajectory_export,
        crate::thinclaw::commands::thinclaw_profile_evolution_status,
        crate::thinclaw::commands::thinclaw_profile_evolution_run,
        // Sprint 13 — New backend APIs
        crate::thinclaw::commands::thinclaw_cost_summary,
        crate::thinclaw::commands::thinclaw_cost_export_csv,
        crate::thinclaw::commands::thinclaw_cost_reset,
        crate::thinclaw::commands::thinclaw_channel_status_list,
        crate::thinclaw::commands::thinclaw_channel_config_schema,
        crate::thinclaw::commands::thinclaw_channel_config_schemas,
        crate::thinclaw::commands::thinclaw_channel_config_submit,
        crate::thinclaw::commands::thinclaw_agents_set_default,
        crate::thinclaw::commands::thinclaw_clawhub_search,
        crate::thinclaw::commands::thinclaw_clawhub_install,
        crate::thinclaw::commands::thinclaw_routine_audit_list,
        crate::thinclaw::commands::thinclaw_clear_routine_runs,
        crate::thinclaw::commands::thinclaw_cache_stats,
        crate::thinclaw::commands::thinclaw_plugin_lifecycle_list,
        crate::thinclaw::commands::thinclaw_manifest_validate,
        crate::thinclaw::commands::thinclaw_routing_get,
        crate::thinclaw::commands::thinclaw_routing_set,
        crate::thinclaw::commands::thinclaw_routing_rules_list,
        crate::thinclaw::commands::thinclaw_routing_rules_save,
        crate::thinclaw::commands::thinclaw_routing_rules_add,
        crate::thinclaw::commands::thinclaw_routing_rules_remove,
        crate::thinclaw::commands::thinclaw_routing_rules_reorder,
        crate::thinclaw::commands::thinclaw_routing_pools_save,
        crate::thinclaw::commands::thinclaw_routing_status,
        crate::thinclaw::commands::thinclaw_routing_simulate,
        crate::thinclaw::commands::thinclaw_gmail_oauth_start,
        crate::thinclaw::commands::thinclaw_gmail_status,
        // Experiments & learning review
        crate::thinclaw::commands::thinclaw_learning_status,
        crate::thinclaw::commands::thinclaw_learning_history,
        crate::thinclaw::commands::thinclaw_learning_candidates,
        crate::thinclaw::commands::thinclaw_learning_artifact_versions,
        crate::thinclaw::commands::thinclaw_learning_provider_health,
        crate::thinclaw::commands::thinclaw_learning_code_proposals,
        crate::thinclaw::commands::thinclaw_learning_outcomes,
        crate::thinclaw::commands::thinclaw_learning_rollbacks,
        crate::thinclaw::commands::thinclaw_learning_review_code_proposal,
        crate::thinclaw::commands::thinclaw_learning_review_outcome,
        crate::thinclaw::commands::thinclaw_learning_record_rollback,
        crate::thinclaw::commands::thinclaw_learning_evaluate_outcomes,
        crate::thinclaw::commands::thinclaw_experiments_projects,
        crate::thinclaw::commands::thinclaw_experiments_campaigns,
        crate::thinclaw::commands::thinclaw_experiments_runners,
        crate::thinclaw::commands::thinclaw_experiments_targets,
        crate::thinclaw::commands::thinclaw_experiments_trials,
        crate::thinclaw::commands::thinclaw_experiments_trial_artifacts,
        crate::thinclaw::commands::thinclaw_experiments_model_usage,
        crate::thinclaw::commands::thinclaw_experiments_opportunities,
        crate::thinclaw::commands::thinclaw_experiments_gpu_clouds,
        crate::thinclaw::commands::thinclaw_experiments_validate_runner,
        crate::thinclaw::commands::thinclaw_experiments_campaign_action,
        crate::thinclaw::commands::thinclaw_experiments_gpu_validate,
        crate::thinclaw::commands::thinclaw_experiments_gpu_launch_test,
        crate::thinclaw::commands::thinclaw_experiments_list_envs,
        crate::thinclaw::commands::thinclaw_experiments_run_eval,
        // ── Permissions ─────────────────────────────────────────────────
        crate::permissions::get_permission_status,
        crate::permissions::request_permission,
        crate::permissions::open_permission_settings,
        // ── Spotlight ───────────────────────────────────────────────────
        crate::toggle_spotlight,
        crate::hide_spotlight,
        // ── Engine & HF Hub ─────────────────────────────────────────────
        crate::engine::direct_runtime_get_active_engine_info,
        crate::engine::direct_runtime_get_engine_setup_status,
        crate::engine::direct_runtime_setup_engine,
        crate::engine::direct_runtime_snapshot,
        crate::engine::direct_runtime_start_engine,
        crate::engine::direct_runtime_stop_engine,
        crate::engine::direct_runtime_is_engine_ready,
        crate::hf_hub::direct_runtime_discover_hf_models,
        crate::hf_hub::direct_runtime_get_model_files,
        crate::hf_hub::direct_runtime_download_hf_model_files,
        crate::hf_hub::direct_runtime_discover_embedding_dimension,
        // ── Inference Router ────────────────────────────────────────────
        crate::inference::direct_inference_get_backends,
        crate::inference::direct_inference_update_backend,
        crate::inference::direct_inference_discover_cloud_models,
        crate::inference::direct_inference_refresh_cloud_models,
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
        crate::thinclaw::commands::thinclaw_get_workspace_path,
        crate::thinclaw::commands::thinclaw_reveal_workspace,
        crate::thinclaw::commands::thinclaw_list_agent_workspace_files,
        crate::thinclaw::commands::thinclaw_write_agent_workspace_file,
        crate::thinclaw::commands::thinclaw_reveal_file,
    ])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    fn command_signatures(bindings: &str) -> Vec<(&str, &str)> {
        bindings
            .lines()
            .filter_map(|line| {
                let line = line.trim_start();
                let rest = line.strip_prefix("async ")?;
                let open = rest.find('(')?;
                let close = rest.rfind(") : Promise<")?;
                Some((&rest[..open], &rest[open + 1..close]))
            })
            .collect()
    }

    fn top_level_parameters(signature: &str) -> Vec<&str> {
        let mut depth = 0_u32;
        let mut start = 0;
        let mut parameters = Vec::new();

        for (index, character) in signature.char_indices() {
            match character {
                '<' | '(' | '[' | '{' => depth += 1,
                '>' | ')' | ']' | '}' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    parameters.push(signature[start..index].trim());
                    start = index + character.len_utf8();
                }
                _ => {}
            }
        }

        if start < signature.len() {
            parameters.push(signature[start..].trim());
        }
        parameters.retain(|parameter| !parameter.is_empty());
        parameters
    }

    #[test]
    fn every_registered_command_matches_the_sanitized_binding_contract() {
        let raw = super::specta_builder()
            .export_str(
                specta_typescript::Typescript::default()
                    .bigint(specta_typescript::BigIntExportBehavior::Number),
            )
            .expect("export command registry");
        let sanitized = crate::sanitize_typescript_bindings_source(&raw);
        let committed = include_str!("../../../frontend/src/lib/bindings.ts");

        assert_eq!(
            sanitized, committed,
            "bindings.ts is stale; run `cargo run --locked --example export_bindings` from apps/desktop/backend"
        );
        assert_eq!(
            sanitized,
            crate::sanitize_typescript_bindings_source(&sanitized),
            "binding sanitization must be idempotent"
        );

        let signatures = command_signatures(&sanitized);
        let names = signatures
            .iter()
            .map(|(name, _)| *name)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            names.len(),
            signatures.len(),
            "the generated command registry must not contain duplicate names"
        );
        assert!(
            signatures.len() >= 340,
            "expected the complete Desktop registry, found only {} commands",
            signatures.len()
        );
    }

    #[test]
    fn binding_sanitizer_preserves_channels_and_rejects_reserved_parameters() {
        let raw = super::specta_builder()
            .export_str(
                specta_typescript::Typescript::default()
                    .bigint(specta_typescript::BigIntExportBehavior::Number),
            )
            .expect("export command registry");
        let sanitized = crate::sanitize_typescript_bindings_source(&raw);

        assert!(raw.contains("export type TAURI_CHANNEL<TSend> = null"));
        assert!(sanitized.contains(
            "export type TAURI_CHANNEL<TSend> = import(\"@tauri-apps/api/core\").Channel<TSend>"
        ));
        assert!(sanitized.contains(
            "async directChatStream(payload: DirectChatPayload, onEvent: TAURI_CHANNEL<StreamChunk>)"
        ));
        assert!(sanitized.contains(
            "async thinclawMcpGetPrompt(serverName: string, promptName: string, promptArgs: JsonValue | null)"
        ));

        let reserved = BTreeSet::from([
            "arguments",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "debugger",
            "default",
            "delete",
            "do",
            "else",
            "enum",
            "eval",
            "export",
            "extends",
            "false",
            "finally",
            "for",
            "function",
            "if",
            "import",
            "in",
            "instanceof",
            "new",
            "null",
            "return",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typeof",
            "var",
            "void",
            "while",
            "with",
            "yield",
        ]);

        for (command, signature) in command_signatures(&sanitized) {
            for parameter in top_level_parameters(signature) {
                let name = parameter
                    .split_once(':')
                    .map(|(name, _)| name.trim())
                    .expect("generated command parameter should include a type");
                assert!(
                    !reserved.contains(name),
                    "generated command `{command}` exposes reserved TypeScript parameter `{name}`"
                );
            }
        }
    }

    #[test]
    fn generated_bindings_cover_phase_two_desktop_surfaces() {
        let bindings = include_str!("../../../frontend/src/lib/bindings.ts");

        assert!(
            bindings.contains("generated by [tauri-specta"),
            "frontend bindings must stay generated, not hand-authored"
        );
        assert!(
            bindings.contains(
                "async thinclawRoutineCreate(name: string, description: string, schedule: string, task: string, triggerType: string | null)"
            ),
            "routine create binding must expose the optional trigger type contract"
        );

        for command in [
            "directChatStream",
            "directChatCompletion",
            "directHistoryGetMessages",
            "directHistorySaveMessage",
            "directRuntimeSnapshot",
            "directInferenceGetBackends",
            "directAssetsUploadImage",
            "directMediaGenerateImage",
            "directImagineGenerate",
            "thinclawRoutineCreate",
            "thinclawRoutineToggle",
            "thinclawRoutineAuditList",
            "thinclawClearRoutineRuns",
            "thinclawTrajectoryExport",
            "thinclawProfileEvolutionStatus",
            "thinclawProfileEvolutionRun",
            "thinclawGmailOauthStart",
            "thinclawGmailStatus",
            "thinclawPairingList",
            "thinclawRoutingPoolsSave",
            "thinclawMemorySearch",
            "thinclawCanvasDispatchEvent",
            "thinclawCanvasPanelsList",
            "thinclawCanvasPanelGet",
            "thinclawJobsList",
            "thinclawAutonomyStatus",
            "thinclawLearningStatus",
            "thinclawExperimentsProjects",
        ] {
            assert!(
                bindings.contains(command),
                "generated bindings should include {command}"
            );
        }

        for removed_command in [
            "chatCompletion",
            "chatStream",
            "countTokens",
            "getMessages",
            "saveMessage",
            "getInferenceBackends",
            "uploadImage",
            "imagineGenerate(",
            "generateImage",
            "startChatServer",
        ] {
            assert!(
                !bindings.contains(removed_command),
                "generated bindings should not include removed Direct command {removed_command}"
            );
        }

        for event_variant in [
            "PlanUpdate",
            "UsageUpdate",
            "LifecycleUpdate",
            "ContextPressure",
            "ObserverRecord",
            "ApprovalRequested",
            "CredentialPrompt",
            "WebLogin",
            "CanvasUpdate",
            "SubAgentUpdate",
            "AgentMessage",
            "JobUpdate",
            "RoutineLifecycle",
            "CostAlert",
        ] {
            assert!(
                bindings.contains(event_variant),
                "UiEvent binding should include {event_variant}"
            );
        }
    }

    #[test]
    fn legacy_frontend_api_delegates_to_generated_bindings() {
        let compatibility_api = include_str!("../../../frontend/src/lib/thinclaw.ts");
        let command_client = include_str!("../../../frontend/src/lib/command-client.ts");

        assert!(
            !compatibility_api.contains("@tauri-apps/api/core"),
            "lib/thinclaw.ts must not bypass the generated binding surface"
        );
        assert!(
            !compatibility_api.contains("safeInvoke"),
            "the handwritten string-command bridge must stay retired"
        );
        assert!(
            command_client.contains("import { commands, type Result } from './bindings'"),
            "the frontend command client must derive from generated bindings.ts"
        );
    }

    #[test]
    fn custom_secret_updates_use_the_registered_generated_command() {
        let bindings = include_str!("../../../frontend/src/lib/bindings.ts");
        let secrets_tab = include_str!("../../../frontend/src/components/settings/SecretsTab.tsx");

        assert!(bindings.contains("async thinclawUpdateCustomSecret("));
        assert!(secrets_tab.contains("commands.thinclawUpdateCustomSecret(id, value)"));
        assert!(
            !secrets_tab.contains("Update for custom secrets not implemented"),
            "the visible custom-secret Update action must stay functional"
        );
    }

    #[test]
    fn production_frontend_has_one_command_calling_convention() {
        fn inspect(directory: &std::path::Path, violations: &mut Vec<String>) {
            for entry in std::fs::read_dir(directory).expect("read frontend source directory") {
                let path = entry.expect("read frontend source entry").path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|name| name == "tests") {
                        continue;
                    }
                    inspect(&path, violations);
                    continue;
                }

                if !matches!(
                    path.extension().and_then(|value| value.to_str()),
                    Some("ts" | "tsx")
                ) || path.file_name().is_some_and(|name| name == "bindings.ts")
                {
                    continue;
                }

                let source = std::fs::read_to_string(&path).expect("read frontend source file");
                let imports_raw_invoke = source
                    .lines()
                    .any(|line| line.contains("@tauri-apps/api/core") && line.contains("invoke"));
                if imports_raw_invoke || source.contains("invoke(") || source.contains("invoke<") {
                    violations.push(path.display().to_string());
                }
            }
        }

        let frontend = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../frontend/src");
        let mut violations = Vec::new();
        inspect(&frontend, &mut violations);
        assert!(
            violations.is_empty(),
            "production frontend files must call Rust through commandClient/generated adapters: {violations:?}"
        );
    }
}
