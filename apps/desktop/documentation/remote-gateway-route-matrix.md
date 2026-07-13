# Remote Gateway Route Matrix

Absolute-completion checkpoint for ThinClaw Desktop remote mode. Desktop IPC names are
`thinclaw_*`; remote calls go through `RemoteGatewayProxy` to the root ThinClaw
HTTP gateway. Unsupported operations must return an `unavailable:` error with a
concrete reason.

Last updated: 2026-07-13

Per-command route modes are code-authoritative: every registered command is
classified in `ROUTE_TABLE` (`thinclaw/bridge.rs`) as `LocalAndRemote`,
`RemoteOnly`, or `LocalOnly`, and the `all_registered_commands_are_classified`
test fails the build if a newly-added command is left unclassified. The
generated table below is the exhaustive command classification; the
surface-level table remains the endpoint-oriented operational summary.

<!-- BEGIN GENERATED ROUTE TABLE -->
## Generated Per-Command Classification

> Generated from `apps/desktop/backend/src/thinclaw/bridge.rs`. Do not edit this block by hand. Regenerate it with `cargo run --locked --example export_bindings`.

### Remote only (10)

| Command | Route mode | Unsupported-mode behavior |
| --- | --- | --- |
| `thinclaw_deploy_remote` | `RemoteOnly` | Embedded mode returns a typed unavailable reason; connect a remote gateway. |
| `thinclaw_experiments_gpu_launch_test` | `RemoteOnly` | Embedded mode returns a typed unavailable reason; connect a remote gateway. |
| `thinclaw_experiments_gpu_validate` | `RemoteOnly` | Embedded mode returns a typed unavailable reason; connect a remote gateway. |
| `thinclaw_extension_reconnect` | `RemoteOnly` | Embedded mode returns a typed unavailable reason; connect a remote gateway. |
| `thinclaw_job_file_read` | `RemoteOnly` | Embedded mode returns a typed unavailable reason; connect a remote gateway. |
| `thinclaw_job_files_list` | `RemoteOnly` | Embedded mode returns a typed unavailable reason; connect a remote gateway. |
| `thinclaw_job_prompt` | `RemoteOnly` | Embedded mode returns a typed unavailable reason; connect a remote gateway. |
| `thinclaw_job_restart` | `RemoteOnly` | Embedded mode returns a typed unavailable reason; connect a remote gateway. |
| `thinclaw_learning_evaluate_outcomes` | `RemoteOnly` | Embedded mode returns a typed unavailable reason; connect a remote gateway. |
| `thinclaw_test_connection` | `RemoteOnly` | Embedded mode returns a typed unavailable reason; connect a remote gateway. |

### Local only (197)

| Command | Route mode | Unsupported-mode behavior |
| --- | --- | --- |
| `agent_chat` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cancel_download` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `check_missing_standard_assets` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `check_model_path` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_cancel_migration` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_get_recovery_key` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_get_status` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_get_storage_breakdown` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_import_recovery_key` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_migrate_to_cloud` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_migrate_to_local` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_oauth_complete` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_oauth_start` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_test_connection` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_test_icloud` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_test_sftp` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `cloud_test_webdav` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `create_project` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `delete_document` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `delete_local_model` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `delete_project` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_assets_get_image_path` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_assets_load_image` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_assets_open_images_folder` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_assets_upload_image` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_chat_completion` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_chat_count_tokens` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_chat_stream` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_history_create_conversation` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_history_delete_all_history` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_history_delete_conversation` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_history_edit_message` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_history_get_conversations` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_history_get_messages` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_history_save_message` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_history_update_conversation_project` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_history_update_conversation_title` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_history_update_conversations_order` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_i18n_get_catalog` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_imagine_delete_image` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_imagine_generate` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_imagine_get_stats` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_imagine_list_images` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_imagine_search_images` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_imagine_toggle_favorite` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_inference_get_backends` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_inference_update_backend` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_media_generate_image` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_media_transcribe_audio` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_media_tts_list_voices` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_media_tts_synthesize` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_rag_check_vector_index_integrity` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_rag_ingest_document` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_rag_retrieve_context` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_rag_upload_document` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_cancel_generation` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_discover_embedding_dimension` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_discover_hf_models` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_download_hf_model_files` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_get_active_engine_info` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_get_chat_server_config` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_get_engine_setup_status` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_get_model_files` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_get_sidecar_status` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_is_engine_ready` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_setup_engine` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_snapshot` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_start_chat_server` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_start_embedding_server` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_start_engine` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_start_image_server` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_start_stt_server` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_start_summarizer_server` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_start_tts_server` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_stop_chat_server` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `direct_runtime_stop_engine` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `download_model` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `download_standard_asset` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `get_hf_token` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `get_model_metadata` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `get_permission_status` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `get_project_documents` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `get_remote_model_catalog` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `get_system_specs` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `get_user_config` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `greet` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `hide_spotlight` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `list_models` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `list_projects` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `open_config_file` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `open_models_folder` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `open_permission_settings` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `open_standard_models_folder` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `open_url` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `request_permission` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_add_agent_profile` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_add_custom_secret` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_agents_list` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_canvas_dispatch_event` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_canvas_navigate` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_canvas_panel_dismiss` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_canvas_panel_get` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_canvas_panels_list` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_canvas_push` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_check_bootstrap_needed` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_checkpoint_diff` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_checkpoint_restore` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_checkpoints_list` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_config_schema` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_cron_lint` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_experiments_list_envs` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_experiments_run_eval` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_external_memory_configure` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_external_memory_disable` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_get_anthropic_key` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_get_bedrock_credentials` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_get_brave_key` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_get_gemini_key` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_get_groq_key` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_get_implicit_provider_key` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_get_openai_key` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_get_openrouter_key` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_get_workspace_path` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_gmail_oauth_start` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_heartbeat_set_interval` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_install_skill_repo` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_list_agent_workspace_files` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_list_child_sessions` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_manifest_validate` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_plugin_lifecycle_list` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_profile_evolution_run` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_profile_evolution_status` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_reload_secrets` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_remote_access_start` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_remote_access_status` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_remote_access_stop` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_remove_agent_profile` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_remove_custom_secret` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_project_approve` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_project_cancel` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_project_create` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_project_enqueue` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_project_enroll` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_project_events` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_project_get` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_project_merge_gates` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_project_pause` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_project_plan` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_project_resume` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_project_start` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_projects_connect` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_projects_connectable_repos` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_projects_list` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_projects_readiness` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_projects_set_credential` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_repo_projects_setup` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_reveal_file` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_reveal_gateway_token` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_reveal_workspace` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_save_bedrock_credentials` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_save_brave_key` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_save_gateway_settings` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_save_slack_config` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_save_telegram_config` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_secret_master_key_rotate` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_secret_recovery_export` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_secret_recovery_import` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_secret_recovery_status` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_security_posture` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_session_search` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_set_autonomy_mode` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_set_bootstrap_completed` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_set_dev_mode_wizard` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_set_hf_token` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_set_setup_completed` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_set_workspace_mode` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_skills_toggle` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_sync_local_llm` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_toggle_auto_start` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_toggle_custom_secret` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_toggle_expose_inference` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_toggle_local_inference` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_toggle_local_tools` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_toggle_secret_access` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_trajectory_export` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_trajectory_records` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_trajectory_stats` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_trigger_bootstrap` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_update_custom_secret` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_update_run` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_update_sub_agent_status` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `thinclaw_write_agent_workspace_file` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `toggle_spotlight` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `update_project` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `update_projects_order` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `update_remote_model_catalog` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |
| `update_user_config` | `LocalOnly` | Remote mode returns a typed unavailable reason; use the embedded runtime. |

### Local and remote (154)

| Command | Route mode | Unsupported-mode behavior |
| --- | --- | --- |
| `direct_inference_discover_cloud_models` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `direct_inference_refresh_cloud_models` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `select_thinclaw_brain` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_abort_chat` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_agents_set_default` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_autonomy_bootstrap` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_autonomy_checks` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_autonomy_evidence` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_autonomy_pause` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_autonomy_permissions` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_autonomy_resume` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_autonomy_rollback` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_autonomy_rollouts` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_autonomy_status` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_broadcast_command` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_cache_stats` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_channel_config_schema` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_channel_config_schemas` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_channel_config_submit` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_channel_status_list` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_channels_list` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_clawhub_install` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_clawhub_search` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_clear_memory` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_clear_routine_runs` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_compact_session` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_config_get` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_config_patch` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_config_set` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_cost_export_csv` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_cost_reset` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_cost_summary` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_cron_history` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_cron_list` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_cron_run` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_delete_file` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_delete_session` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_desktop_permission_status` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_diagnostics` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_experiments_campaign_action` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_experiments_campaigns` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_experiments_gpu_clouds` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_experiments_model_usage` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_experiments_opportunities` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_experiments_projects` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_experiments_runners` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_experiments_targets` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_experiments_trial_artifacts` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_experiments_trials` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_experiments_validate_runner` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_export_session` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_extension_activate` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_extension_install` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_extension_registry_search` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_extension_remove` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_extension_setup_get` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_extension_setup_submit` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_extension_validate_setup` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_extensions_list` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_get_autonomy_mode` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_get_diagnostics` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_get_file` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_get_fleet_status` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_get_history` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_get_memory` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_get_sessions` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_get_status` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_gmail_status` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_hooks_list` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_hooks_register` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_hooks_unregister` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_install_skill_deps` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_job_cancel` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_job_detail` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_job_events` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_jobs_list` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_jobs_summary` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_learning_artifact_versions` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_learning_candidates` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_learning_code_proposals` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_learning_history` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_learning_outcomes` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_learning_provider_health` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_learning_record_rollback` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_learning_review_code_proposal` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_learning_review_outcome` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_learning_rollbacks` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_learning_status` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_list_workspace_files` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_logs_tail` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_mcp_get_prompt` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_mcp_interaction_respond` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_mcp_interactions` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_mcp_oauth` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_mcp_read_resource` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_mcp_resource_templates` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_mcp_server` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_mcp_server_prompts` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_mcp_server_resources` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_mcp_server_tools` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_mcp_servers` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_mcp_set_log_level` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_memory_search` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_pairing_approve` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_pairing_list` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_redo` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_reset_session` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_resolve_approval` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routine_audit_list` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routine_create` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routine_delete` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routine_toggle` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routing_get` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routing_pools_save` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routing_rules_add` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routing_rules_list` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routing_rules_remove` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routing_rules_reorder` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routing_rules_save` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routing_set` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routing_simulate` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_routing_status` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_save_anthropic_key` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_save_cloud_config` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_save_gemini_key` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_save_groq_key` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_save_implicit_provider_key` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_save_memory` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_save_openai_key` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_save_openrouter_key` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_save_selected_cloud_model` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_send_message` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_set_thinking` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_skill_inspect` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_skill_install` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_skill_publish` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_skill_reload` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_skill_remove` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_skill_trust` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_skills_list` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_skills_reload_all` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_skills_search` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_skills_status` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_spawn_session` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_start_gateway` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_stop_gateway` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_subscribe_session` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_switch_to_profile` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_system_presence` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_tool_policy_get` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_tool_policy_set` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_tools_list` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_undo` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |
| `thinclaw_write_file` | `LocalAndRemote` | Supported in both embedded and remote-gateway modes. |

<!-- END GENERATED ROUTE TABLE -->

## Surface-Level Endpoint Coverage

| Surface | Desktop command/proxy coverage | Remote endpoint | Status |
| --- | --- | --- | --- |
| Chat send | `thinclaw_send_message` | `POST /api/chat/send` | wired |
| Chat abort | `thinclaw_abort_chat` | `POST /api/chat/abort` | wired |
| Approvals | `thinclaw_resolve_approval` | `POST /api/chat/approval` | wired |
| Sessions list/history/delete | `thinclaw_get_sessions`, `thinclaw_get_history`, `thinclaw_delete_session` | `GET /api/chat/threads`, `GET /api/chat/history`, `DELETE /api/chat/thread/{id}` | wired |
| Session reset/export/compact | `thinclaw_reset_session`, `thinclaw_export_session`, `thinclaw_compact_session` | `POST /api/chat/thread/{id}/reset`, `GET /api/chat/thread/{id}/export`, `POST /api/chat/thread/{id}/compact` | wired |
| Memory read/write/list/search | memory/file commands | `/api/memory/read`, `/api/memory/write`, `/api/memory/tree`, `/api/memory/search` | wired |
| Memory delete | `thinclaw_delete_file` | `POST /api/memory/delete` | wired |
| Routines list/run/history/toggle/delete | routine commands | `/api/routines`, `/api/routines/{id}/trigger`, `/api/routines/{id}/runs`, `/api/routines/{id}/toggle`, `DELETE /api/routines/{id}` | wired |
| Routine create/clear-runs | routine create/clear commands | `POST /api/routines`, `DELETE /api/routines/runs` | wired |
| Skills list/status/search/install/remove/trust/reload/inspect/publish | skill commands | `GET /api/skills`, `POST /api/skills/search`, `POST /api/skills/install`, `DELETE /api/skills/{name}`, `PUT /api/skills/{name}/trust`, `POST /api/skills/{name}/reload`, `POST /api/skills/reload-all`, `POST /api/skills/{name}/inspect`, `POST /api/skills/{name}/publish` | wired |
| Skill toggle/repo clone | skill commands | none | unavailable: no enable toggle; arbitrary git clone is local-only |
| Extensions list/install/registry/activate/reconnect/validate/remove/setup/tools | extension/tool commands | `/api/extensions`, `/api/extensions/install`, `/api/extensions/registry`, `/api/extensions/{name}/activate`, `/api/extensions/{name}/reconnect`, `/api/extensions/{name}/validate`, `/api/extensions/{name}/remove`, `/api/extensions/{name}/setup`, `/api/extensions/tools` | wired |
| Hooks/lifecycle audit/manifest validation/cache stats | dashboard/extension commands | `GET /api/hooks`, `POST /api/hooks`, `DELETE /api/hooks/{name}`, `GET /api/cache/stats`, local-only manifest/lifecycle internals | hook routes and cache stats wired; local-only internals return explicit reason |
| MCP servers/tools/resources/templates/prompts/OAuth/log-level/interactions | MCP desktop commands | `/api/mcp/servers`, `/api/mcp/servers/{name}`, `/api/mcp/servers/{name}/tools`, `/api/mcp/servers/{name}/resources`, `/api/mcp/servers/{name}/resources/read`, `/api/mcp/servers/{name}/resource-templates`, `/api/mcp/servers/{name}/prompts`, `/api/mcp/servers/{name}/prompts/{prompt_name}`, `/api/mcp/servers/{name}/oauth`, `/api/mcp/servers/{name}/log-level`, `/api/mcp/interactions`, `/api/mcp/interactions/{interaction_id}/respond` | wired |
| Settings | config get/set/patch | `/api/settings`, `/api/settings/{key}` | wired |
| Gateway status/presence | status/diagnostics/presence commands | `/api/health`, `/api/gateway/status` | wired |
| Logs | `thinclaw_logs_tail` | `/api/logs/recent`, `/api/logs/events` | wired |
| Costs | cost commands | `/api/costs/summary`, `/api/costs/export`, `/api/costs/reset` | wired |
| Routing/provider config | routing/cloud config/simulation commands | `/api/providers`, `/api/providers/config`, `/api/providers/{slug}/models`, `/api/providers/route/simulate` | wired for config/status/simulation/rule mutation/pool updates |
| Provider vault | key commands/proxy | `POST/DELETE /api/providers/{slug}/key` | wired for save/delete/status only; raw secret reads denied |
| Pairing/channels/Gmail | pairing/channel/Gmail commands | `/api/pairing/{channel}`, `/api/pairing/{channel}/approve`, `/api/gateway/status`, `/api/settings/*` | wired where gateway exposes status/config |
| Jobs | `thinclaw_jobs_list`, `thinclaw_jobs_summary`, `thinclaw_job_detail`, `thinclaw_job_cancel`, `thinclaw_job_restart`, `thinclaw_job_prompt`, `thinclaw_job_events`, `thinclaw_job_files_list`, `thinclaw_job_file_read` | `/api/jobs/*` | wired; local direct jobs expose list/detail/events/cancel and explicit unavailable reasons for sandbox-only restart/prompt/files |
| Autonomy | `thinclaw_autonomy_status`, `thinclaw_autonomy_bootstrap`, `thinclaw_autonomy_pause`, `thinclaw_autonomy_resume`, `thinclaw_autonomy_permissions`, `thinclaw_autonomy_rollback`, `thinclaw_autonomy_rollouts`, `thinclaw_autonomy_checks`, `thinclaw_autonomy_evidence` | `/api/autonomy/*` | wired for status/review surfaces; host-executing mutation remains gated by remote or local host policy |
| Experiments | experiment IPC wrappers and proxy helpers | `/api/experiments/*` | wired for status/review/action surfaces exposed by the gateway |
| Learning | learning IPC wrappers and proxy helpers | `/api/learning/*` | wired for status/history/candidates/review surfaces exposed by the gateway |
| Session search | `thinclaw_session_search` | none | unavailable: LocalOnly — full-text search runs over the embedded session store |
| Agent eval | `thinclaw_experiments_list_envs`, `thinclaw_experiments_run_eval` | none | unavailable: `run_eval` LocalOnly — drives the embedded agent in throwaway sessions |
| Channel config | `thinclaw_channel_config_schema`, `thinclaw_channel_config_schemas`, `thinclaw_channel_config_submit` | none | unavailable: LocalOnly — schema read + submit operate on the embedded channel manager |

Known intentional gaps are external host-policy gates, not silent desktop no-ops.
The backend fixture suite currently exercises the chat/session/memory/log/cache/hook
family plus provider routing, provider vault save/delete/status shape, costs,
jobs, autonomy, learning, experiments, MCP, and confirmed skill mutation routes
through `RemoteGatewayProxy`. A release candidate is route-complete only when
new route families added to this matrix are also added to that executable fixture.

## Remote Mode Rules

- Mutating skill endpoints must send `X-Confirm-Action: true`.
- Remote provider-vault commands may save/delete/status keys, but must never read raw secret values.
- Local-only host actions, especially arbitrary git clone, Gmail OAuth launched on the desktop host, and autonomy execution, must report explicit unavailable reasons in remote mode.
- Remote SSE events must be normalized into `UiEvent` on `thinclaw-event`; the frontend should not subscribe to a separate remote event schema.
