//! Bridge contract primitives (TDO-001).
//!
//! Normalizes how desktop Tauri commands express dual-mode (embedded vs remote
//! gateway) availability. Historically "this isn't available in local mode" was
//! signalled two incompatible ways: some commands returned `Err(String)` (e.g.
//! `local_unavailable` in `commands/rpc_jobs_autonomy.rs`), others returned
//! `Ok(unavailable(...))` JSON. The frontend cannot reliably tell "gated, here's
//! why" from "failed". `BridgeError` makes a gated state a single, typed,
//! machine-readable outcome carrying its remediation, so the UI can render a CTA
//! instead of an error toast.
//!
//! This module is the foundation the rest of WS-1 (route-table registry, bridge
//! linter, generated route matrix) builds on. It is intentionally additive: it
//! does not yet replace existing `Result<_, String>` signatures — commands are
//! migrated incrementally.

use serde::{Deserialize, Serialize};

/// How a command behaves across the dual-mode runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RouteMode {
    /// Works in both embedded and remote-gateway mode.
    LocalAndRemote,
    /// Only meaningful against a remote gateway (e.g. sandbox job restart, GPU launch).
    RemoteOnly,
    /// Only meaningful in embedded mode (e.g. local sidecar control).
    LocalOnly,
}

/// A typed command outcome that distinguishes a *gated* capability (with its
/// reason + remediation) from a genuine runtime error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeError {
    /// The capability is intentionally unavailable in the current runtime mode.
    Unavailable {
        /// Short capability label, e.g. "manual outcome evaluation".
        capability: String,
        /// Why it is unavailable right now.
        reason: String,
        /// What the user must do to satisfy it (shown as a CTA), if anything.
        remediation: Option<String>,
        /// Which runtime mode *would* satisfy it.
        satisfied_by: RouteMode,
    },
    /// A genuine error (kept distinct from the gated state above).
    /// Struct variant (not a tuple) so the internally-tagged (`tag = "kind"`)
    /// representation stays valid for serde/specta export.
    Runtime { message: String },
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Unavailable {
                capability, reason, ..
            } => write!(f, "unavailable: {capability}: {reason}"),
            BridgeError::Runtime { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for BridgeError {}

/// Lets existing `?`/`.map_err(|e| e.to_string())` sites migrate to
/// `Result<T, BridgeError>` mechanically: any string error becomes `Runtime`.
impl From<String> for BridgeError {
    fn from(value: String) -> Self {
        BridgeError::Runtime { message: value }
    }
}

impl From<&str> for BridgeError {
    fn from(value: &str) -> Self {
        BridgeError::Runtime {
            message: value.to_string(),
        }
    }
}

/// Build a `BridgeError::Unavailable` for a capability that is gated in the
/// current runtime mode. Replaces the ad-hoc `local_unavailable`/`unavailable`
/// helpers with one typed, frontend-renderable shape.
pub fn gated(
    capability: impl Into<String>,
    reason: impl Into<String>,
    remediation: impl Into<String>,
    satisfied_by: RouteMode,
) -> BridgeError {
    BridgeError::Unavailable {
        capability: capability.into(),
        reason: reason.into(),
        remediation: Some(remediation.into()),
        satisfied_by,
    }
}

// ---------------------------------------------------------------------------
// Route table (TDO-002)
// ---------------------------------------------------------------------------
//
// Maps every Tauri command name to its RouteMode. This is the bridge linter's
// ground truth: total coverage is enforced by test, so the table is exhaustive
// over the registered command surface (see setup/commands.rs).
//
// Ordering within each RouteMode group is alphabetical. Do not mix modes within
// a group — keep RemoteOnly, LocalOnly, and LocalAndRemote entries together so
// that reviewers can verify the assignment at a glance.

/// Route table: classifies **every** registered Tauri command by [`RouteMode`].
/// Total coverage is enforced by `all_registered_commands_are_classified` — a
/// new command added to `setup/commands.rs` fails the build until it is
/// classified here. Gated commands (whose binding returns `BridgeError`) are
/// additionally checked by `all_gated_commands_are_classified`.
pub static ROUTE_TABLE: &[(&str, RouteMode)] = &[
    // ---- RemoteOnly ---------------------------------------------------------
    // Require a live remote gateway; the local/embedded path is gated (BridgeError
    // ::Unavailable) or absent. Includes sandbox job file/restart/prompt, GPU
    // experiment flows, remote-outcome eval, host deploy, and gateway connection tests.
    ("thinclaw_deploy_remote", RouteMode::RemoteOnly),
    (
        "thinclaw_experiments_gpu_launch_test",
        RouteMode::RemoteOnly,
    ),
    ("thinclaw_experiments_gpu_validate", RouteMode::RemoteOnly),
    ("thinclaw_extension_reconnect", RouteMode::RemoteOnly),
    ("thinclaw_job_file_read", RouteMode::RemoteOnly),
    ("thinclaw_job_files_list", RouteMode::RemoteOnly),
    ("thinclaw_job_prompt", RouteMode::RemoteOnly),
    ("thinclaw_job_restart", RouteMode::RemoteOnly),
    ("thinclaw_learning_evaluate_outcomes", RouteMode::RemoteOnly),
    ("thinclaw_test_connection", RouteMode::RemoteOnly),
    // ---- LocalOnly ----------------------------------------------------------
    // Embedded-only: local sidecar/inference/model management, filesystem features
    // (checkpoints, trajectory archive, session search), host-executing mutations
    // (git clone, Gmail OAuth, autonomy-mode), and reads the gateway keeps opaque
    // (raw secret values). The remote path, where present, returns a typed gate.
    ("agent_chat", RouteMode::LocalOnly),
    ("cancel_download", RouteMode::LocalOnly),
    ("check_missing_standard_assets", RouteMode::LocalOnly),
    ("check_model_path", RouteMode::LocalOnly),
    ("cloud_cancel_migration", RouteMode::LocalOnly),
    ("cloud_get_recovery_key", RouteMode::LocalOnly),
    ("cloud_get_status", RouteMode::LocalOnly),
    ("cloud_get_storage_breakdown", RouteMode::LocalOnly),
    ("cloud_import_recovery_key", RouteMode::LocalOnly),
    ("cloud_migrate_to_cloud", RouteMode::LocalOnly),
    ("cloud_migrate_to_local", RouteMode::LocalOnly),
    ("cloud_oauth_complete", RouteMode::LocalOnly),
    ("cloud_oauth_start", RouteMode::LocalOnly),
    ("cloud_test_connection", RouteMode::LocalOnly),
    ("cloud_test_icloud", RouteMode::LocalOnly),
    ("cloud_test_sftp", RouteMode::LocalOnly),
    ("cloud_test_webdav", RouteMode::LocalOnly),
    ("create_project", RouteMode::LocalOnly),
    ("delete_document", RouteMode::LocalOnly),
    ("delete_local_model", RouteMode::LocalOnly),
    ("delete_project", RouteMode::LocalOnly),
    ("direct_assets_get_image_path", RouteMode::LocalOnly),
    ("direct_assets_load_image", RouteMode::LocalOnly),
    ("direct_assets_open_images_folder", RouteMode::LocalOnly),
    ("direct_assets_upload_image", RouteMode::LocalOnly),
    ("direct_chat_completion", RouteMode::LocalOnly),
    ("direct_chat_count_tokens", RouteMode::LocalOnly),
    ("direct_chat_stream", RouteMode::LocalOnly),
    ("direct_history_create_conversation", RouteMode::LocalOnly),
    ("direct_history_delete_all_history", RouteMode::LocalOnly),
    ("direct_history_delete_conversation", RouteMode::LocalOnly),
    ("direct_history_edit_message", RouteMode::LocalOnly),
    ("direct_history_get_conversations", RouteMode::LocalOnly),
    ("direct_history_get_messages", RouteMode::LocalOnly),
    ("direct_history_save_message", RouteMode::LocalOnly),
    (
        "direct_history_update_conversation_project",
        RouteMode::LocalOnly,
    ),
    (
        "direct_history_update_conversation_title",
        RouteMode::LocalOnly,
    ),
    (
        "direct_history_update_conversations_order",
        RouteMode::LocalOnly,
    ),
    ("direct_imagine_delete_image", RouteMode::LocalOnly),
    ("direct_imagine_generate", RouteMode::LocalOnly),
    ("direct_imagine_get_stats", RouteMode::LocalOnly),
    ("direct_imagine_list_images", RouteMode::LocalOnly),
    ("direct_imagine_search_images", RouteMode::LocalOnly),
    ("direct_imagine_toggle_favorite", RouteMode::LocalOnly),
    ("direct_inference_get_backends", RouteMode::LocalOnly),
    ("direct_inference_update_backend", RouteMode::LocalOnly),
    ("direct_media_generate_image", RouteMode::LocalOnly),
    ("direct_media_transcribe_audio", RouteMode::LocalOnly),
    ("direct_media_tts_list_voices", RouteMode::LocalOnly),
    ("direct_media_tts_synthesize", RouteMode::LocalOnly),
    (
        "direct_rag_check_vector_index_integrity",
        RouteMode::LocalOnly,
    ),
    ("direct_rag_ingest_document", RouteMode::LocalOnly),
    ("direct_rag_retrieve_context", RouteMode::LocalOnly),
    ("direct_rag_upload_document", RouteMode::LocalOnly),
    ("direct_runtime_cancel_generation", RouteMode::LocalOnly),
    (
        "direct_runtime_discover_embedding_dimension",
        RouteMode::LocalOnly,
    ),
    ("direct_runtime_discover_hf_models", RouteMode::LocalOnly),
    (
        "direct_runtime_download_hf_model_files",
        RouteMode::LocalOnly,
    ),
    (
        "direct_runtime_get_active_engine_info",
        RouteMode::LocalOnly,
    ),
    (
        "direct_runtime_get_chat_server_config",
        RouteMode::LocalOnly,
    ),
    (
        "direct_runtime_get_engine_setup_status",
        RouteMode::LocalOnly,
    ),
    ("direct_runtime_get_model_files", RouteMode::LocalOnly),
    ("direct_runtime_get_sidecar_status", RouteMode::LocalOnly),
    ("direct_runtime_is_engine_ready", RouteMode::LocalOnly),
    ("direct_runtime_setup_engine", RouteMode::LocalOnly),
    ("direct_runtime_snapshot", RouteMode::LocalOnly),
    ("direct_runtime_start_chat_server", RouteMode::LocalOnly),
    (
        "direct_runtime_start_embedding_server",
        RouteMode::LocalOnly,
    ),
    ("direct_runtime_start_engine", RouteMode::LocalOnly),
    ("direct_runtime_start_image_server", RouteMode::LocalOnly),
    ("direct_runtime_start_stt_server", RouteMode::LocalOnly),
    (
        "direct_runtime_start_summarizer_server",
        RouteMode::LocalOnly,
    ),
    ("direct_runtime_start_tts_server", RouteMode::LocalOnly),
    ("direct_runtime_stop_chat_server", RouteMode::LocalOnly),
    ("direct_runtime_stop_engine", RouteMode::LocalOnly),
    ("download_model", RouteMode::LocalOnly),
    ("download_standard_asset", RouteMode::LocalOnly),
    ("get_hf_token", RouteMode::LocalOnly),
    ("get_model_metadata", RouteMode::LocalOnly),
    ("get_permission_status", RouteMode::LocalOnly),
    ("get_project_documents", RouteMode::LocalOnly),
    ("get_remote_model_catalog", RouteMode::LocalOnly),
    ("get_system_specs", RouteMode::LocalOnly),
    ("get_user_config", RouteMode::LocalOnly),
    ("greet", RouteMode::LocalOnly),
    ("hide_spotlight", RouteMode::LocalOnly),
    ("list_models", RouteMode::LocalOnly),
    ("list_projects", RouteMode::LocalOnly),
    ("open_config_file", RouteMode::LocalOnly),
    ("open_models_folder", RouteMode::LocalOnly),
    ("open_permission_settings", RouteMode::LocalOnly),
    ("open_standard_models_folder", RouteMode::LocalOnly),
    ("open_url", RouteMode::LocalOnly),
    ("request_permission", RouteMode::LocalOnly),
    ("thinclaw_add_agent_profile", RouteMode::LocalOnly),
    ("thinclaw_add_custom_secret", RouteMode::LocalOnly),
    ("thinclaw_agents_list", RouteMode::LocalOnly),
    ("thinclaw_broadcast_command", RouteMode::LocalOnly),
    ("thinclaw_canvas_navigate", RouteMode::LocalOnly),
    ("thinclaw_canvas_panel_dismiss", RouteMode::LocalOnly),
    ("thinclaw_canvas_panel_get", RouteMode::LocalOnly),
    ("thinclaw_canvas_panels_list", RouteMode::LocalOnly),
    ("thinclaw_canvas_push", RouteMode::LocalOnly),
    ("thinclaw_channel_config_schema", RouteMode::LocalOnly),
    ("thinclaw_channel_config_schemas", RouteMode::LocalOnly),
    ("thinclaw_channel_config_submit", RouteMode::LocalOnly),
    ("thinclaw_check_bootstrap_needed", RouteMode::LocalOnly),
    ("thinclaw_checkpoint_diff", RouteMode::LocalOnly),
    ("thinclaw_checkpoint_restore", RouteMode::LocalOnly),
    ("thinclaw_checkpoints_list", RouteMode::LocalOnly),
    ("thinclaw_config_schema", RouteMode::LocalOnly),
    ("thinclaw_cron_lint", RouteMode::LocalOnly),
    ("thinclaw_experiments_list_envs", RouteMode::LocalOnly),
    ("thinclaw_experiments_run_eval", RouteMode::LocalOnly),
    ("thinclaw_get_anthropic_key", RouteMode::LocalOnly),
    ("thinclaw_get_bedrock_credentials", RouteMode::LocalOnly),
    ("thinclaw_get_brave_key", RouteMode::LocalOnly),
    ("thinclaw_get_fleet_status", RouteMode::LocalOnly),
    ("thinclaw_get_gemini_key", RouteMode::LocalOnly),
    ("thinclaw_get_groq_key", RouteMode::LocalOnly),
    ("thinclaw_get_implicit_provider_key", RouteMode::LocalOnly),
    ("thinclaw_get_openai_key", RouteMode::LocalOnly),
    ("thinclaw_get_openrouter_key", RouteMode::LocalOnly),
    ("thinclaw_get_workspace_path", RouteMode::LocalOnly),
    ("thinclaw_gmail_oauth_start", RouteMode::LocalOnly),
    ("thinclaw_heartbeat_set_interval", RouteMode::LocalOnly),
    ("thinclaw_install_skill_repo", RouteMode::LocalOnly),
    ("thinclaw_list_agent_workspace_files", RouteMode::LocalOnly),
    ("thinclaw_list_child_sessions", RouteMode::LocalOnly),
    ("thinclaw_manifest_validate", RouteMode::LocalOnly),
    ("thinclaw_plugin_lifecycle_list", RouteMode::LocalOnly),
    ("thinclaw_reload_secrets", RouteMode::LocalOnly),
    ("thinclaw_remove_agent_profile", RouteMode::LocalOnly),
    ("thinclaw_remove_custom_secret", RouteMode::LocalOnly),
    ("thinclaw_repo_project_approve", RouteMode::LocalOnly),
    ("thinclaw_repo_project_cancel", RouteMode::LocalOnly),
    ("thinclaw_repo_project_create", RouteMode::LocalOnly),
    ("thinclaw_repo_project_enqueue", RouteMode::LocalOnly),
    ("thinclaw_repo_project_enroll", RouteMode::LocalOnly),
    ("thinclaw_repo_project_events", RouteMode::LocalOnly),
    ("thinclaw_repo_project_get", RouteMode::LocalOnly),
    ("thinclaw_repo_project_merge_gates", RouteMode::LocalOnly),
    ("thinclaw_repo_project_pause", RouteMode::LocalOnly),
    ("thinclaw_repo_project_plan", RouteMode::LocalOnly),
    ("thinclaw_repo_project_resume", RouteMode::LocalOnly),
    ("thinclaw_repo_project_start", RouteMode::LocalOnly),
    ("thinclaw_repo_projects_connect", RouteMode::LocalOnly),
    (
        "thinclaw_repo_projects_connectable_repos",
        RouteMode::LocalOnly,
    ),
    ("thinclaw_repo_projects_list", RouteMode::LocalOnly),
    ("thinclaw_repo_projects_readiness", RouteMode::LocalOnly),
    (
        "thinclaw_repo_projects_set_credential",
        RouteMode::LocalOnly,
    ),
    ("thinclaw_repo_projects_setup", RouteMode::LocalOnly),
    ("thinclaw_reveal_file", RouteMode::LocalOnly),
    ("thinclaw_reveal_workspace", RouteMode::LocalOnly),
    ("thinclaw_save_bedrock_credentials", RouteMode::LocalOnly),
    ("thinclaw_save_brave_key", RouteMode::LocalOnly),
    ("thinclaw_save_gateway_settings", RouteMode::LocalOnly),
    ("thinclaw_save_slack_config", RouteMode::LocalOnly),
    ("thinclaw_save_telegram_config", RouteMode::LocalOnly),
    ("thinclaw_session_search", RouteMode::LocalOnly),
    ("thinclaw_set_autonomy_mode", RouteMode::LocalOnly),
    ("thinclaw_set_bootstrap_completed", RouteMode::LocalOnly),
    ("thinclaw_set_dev_mode_wizard", RouteMode::LocalOnly),
    ("thinclaw_set_hf_token", RouteMode::LocalOnly),
    ("thinclaw_set_setup_completed", RouteMode::LocalOnly),
    ("thinclaw_set_workspace_mode", RouteMode::LocalOnly),
    ("thinclaw_skills_toggle", RouteMode::LocalOnly),
    ("thinclaw_spawn_session", RouteMode::LocalOnly),
    ("thinclaw_sync_local_llm", RouteMode::LocalOnly),
    ("thinclaw_toggle_auto_start", RouteMode::LocalOnly),
    ("thinclaw_toggle_custom_secret", RouteMode::LocalOnly),
    ("thinclaw_toggle_expose_inference", RouteMode::LocalOnly),
    ("thinclaw_toggle_local_inference", RouteMode::LocalOnly),
    ("thinclaw_toggle_local_tools", RouteMode::LocalOnly),
    ("thinclaw_toggle_secret_access", RouteMode::LocalOnly),
    ("thinclaw_trajectory_records", RouteMode::LocalOnly),
    ("thinclaw_trajectory_stats", RouteMode::LocalOnly),
    ("thinclaw_trigger_bootstrap", RouteMode::LocalOnly),
    ("thinclaw_update_run", RouteMode::LocalOnly),
    ("thinclaw_update_sub_agent_status", RouteMode::LocalOnly),
    ("thinclaw_web_login_telegram", RouteMode::LocalOnly),
    ("thinclaw_web_login_whatsapp", RouteMode::LocalOnly),
    ("thinclaw_write_agent_workspace_file", RouteMode::LocalOnly),
    ("toggle_spotlight", RouteMode::LocalOnly),
    ("update_project", RouteMode::LocalOnly),
    ("update_projects_order", RouteMode::LocalOnly),
    ("update_remote_model_catalog", RouteMode::LocalOnly),
    ("update_user_config", RouteMode::LocalOnly),
    // ---- LocalAndRemote -----------------------------------------------------
    // Work in both modes; the runtime dispatches to the embedded engine or the
    // RemoteGatewayProxy. Some (autonomy_*) still gate *execution* behind host
    // policy via BridgeError::Unavailable while remaining dual-mode for reads.
    (
        "direct_inference_discover_cloud_models",
        RouteMode::LocalAndRemote,
    ),
    (
        "direct_inference_refresh_cloud_models",
        RouteMode::LocalAndRemote,
    ),
    ("select_thinclaw_brain", RouteMode::LocalAndRemote),
    ("thinclaw_abort_chat", RouteMode::LocalAndRemote),
    ("thinclaw_agents_set_default", RouteMode::LocalAndRemote),
    ("thinclaw_autonomy_bootstrap", RouteMode::LocalAndRemote),
    ("thinclaw_autonomy_checks", RouteMode::LocalAndRemote),
    ("thinclaw_autonomy_evidence", RouteMode::LocalAndRemote),
    ("thinclaw_autonomy_pause", RouteMode::LocalAndRemote),
    ("thinclaw_autonomy_permissions", RouteMode::LocalAndRemote),
    ("thinclaw_autonomy_resume", RouteMode::LocalAndRemote),
    ("thinclaw_autonomy_rollback", RouteMode::LocalAndRemote),
    ("thinclaw_autonomy_rollouts", RouteMode::LocalAndRemote),
    ("thinclaw_autonomy_status", RouteMode::LocalAndRemote),
    ("thinclaw_cache_stats", RouteMode::LocalAndRemote),
    ("thinclaw_channel_status_list", RouteMode::LocalAndRemote),
    ("thinclaw_channels_list", RouteMode::LocalAndRemote),
    ("thinclaw_clawhub_install", RouteMode::LocalAndRemote),
    ("thinclaw_clawhub_search", RouteMode::LocalAndRemote),
    ("thinclaw_clear_memory", RouteMode::LocalAndRemote),
    ("thinclaw_clear_routine_runs", RouteMode::LocalAndRemote),
    ("thinclaw_compact_session", RouteMode::LocalAndRemote),
    ("thinclaw_config_get", RouteMode::LocalAndRemote),
    ("thinclaw_config_patch", RouteMode::LocalAndRemote),
    ("thinclaw_config_set", RouteMode::LocalAndRemote),
    ("thinclaw_cost_export_csv", RouteMode::LocalAndRemote),
    ("thinclaw_cost_reset", RouteMode::LocalAndRemote),
    ("thinclaw_cost_summary", RouteMode::LocalAndRemote),
    ("thinclaw_cron_history", RouteMode::LocalAndRemote),
    ("thinclaw_cron_list", RouteMode::LocalAndRemote),
    ("thinclaw_cron_run", RouteMode::LocalAndRemote),
    ("thinclaw_delete_file", RouteMode::LocalAndRemote),
    ("thinclaw_delete_session", RouteMode::LocalAndRemote),
    (
        "thinclaw_desktop_permission_status",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_diagnostics", RouteMode::LocalAndRemote),
    (
        "thinclaw_experiments_campaign_action",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_experiments_campaigns", RouteMode::LocalAndRemote),
    ("thinclaw_experiments_gpu_clouds", RouteMode::LocalAndRemote),
    (
        "thinclaw_experiments_model_usage",
        RouteMode::LocalAndRemote,
    ),
    (
        "thinclaw_experiments_opportunities",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_experiments_projects", RouteMode::LocalAndRemote),
    ("thinclaw_experiments_runners", RouteMode::LocalAndRemote),
    ("thinclaw_experiments_targets", RouteMode::LocalAndRemote),
    (
        "thinclaw_experiments_trial_artifacts",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_experiments_trials", RouteMode::LocalAndRemote),
    (
        "thinclaw_experiments_validate_runner",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_export_session", RouteMode::LocalAndRemote),
    ("thinclaw_extension_activate", RouteMode::LocalAndRemote),
    ("thinclaw_extension_install", RouteMode::LocalAndRemote),
    (
        "thinclaw_extension_registry_search",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_extension_remove", RouteMode::LocalAndRemote),
    ("thinclaw_extension_setup_get", RouteMode::LocalAndRemote),
    ("thinclaw_extension_setup_submit", RouteMode::LocalAndRemote),
    (
        "thinclaw_extension_validate_setup",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_extensions_list", RouteMode::LocalAndRemote),
    ("thinclaw_get_autonomy_mode", RouteMode::LocalAndRemote),
    ("thinclaw_get_diagnostics", RouteMode::LocalAndRemote),
    ("thinclaw_get_file", RouteMode::LocalAndRemote),
    ("thinclaw_get_history", RouteMode::LocalAndRemote),
    ("thinclaw_get_memory", RouteMode::LocalAndRemote),
    ("thinclaw_get_sessions", RouteMode::LocalAndRemote),
    ("thinclaw_get_status", RouteMode::LocalAndRemote),
    ("thinclaw_gmail_status", RouteMode::LocalAndRemote),
    ("thinclaw_hooks_list", RouteMode::LocalAndRemote),
    ("thinclaw_hooks_register", RouteMode::LocalAndRemote),
    ("thinclaw_hooks_unregister", RouteMode::LocalAndRemote),
    ("thinclaw_install_skill_deps", RouteMode::LocalAndRemote),
    ("thinclaw_job_cancel", RouteMode::LocalAndRemote),
    ("thinclaw_job_detail", RouteMode::LocalAndRemote),
    ("thinclaw_job_events", RouteMode::LocalAndRemote),
    ("thinclaw_jobs_list", RouteMode::LocalAndRemote),
    ("thinclaw_jobs_summary", RouteMode::LocalAndRemote),
    (
        "thinclaw_learning_artifact_versions",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_learning_candidates", RouteMode::LocalAndRemote),
    (
        "thinclaw_learning_code_proposals",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_learning_history", RouteMode::LocalAndRemote),
    ("thinclaw_learning_outcomes", RouteMode::LocalAndRemote),
    (
        "thinclaw_learning_provider_health",
        RouteMode::LocalAndRemote,
    ),
    (
        "thinclaw_learning_record_rollback",
        RouteMode::LocalAndRemote,
    ),
    (
        "thinclaw_learning_review_code_proposal",
        RouteMode::LocalAndRemote,
    ),
    (
        "thinclaw_learning_review_outcome",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_learning_rollbacks", RouteMode::LocalAndRemote),
    ("thinclaw_learning_status", RouteMode::LocalAndRemote),
    ("thinclaw_list_workspace_files", RouteMode::LocalAndRemote),
    ("thinclaw_logs_tail", RouteMode::LocalAndRemote),
    ("thinclaw_mcp_get_prompt", RouteMode::LocalAndRemote),
    (
        "thinclaw_mcp_interaction_respond",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_mcp_interactions", RouteMode::LocalAndRemote),
    ("thinclaw_mcp_oauth", RouteMode::LocalAndRemote),
    ("thinclaw_mcp_read_resource", RouteMode::LocalAndRemote),
    ("thinclaw_mcp_resource_templates", RouteMode::LocalAndRemote),
    ("thinclaw_mcp_server", RouteMode::LocalAndRemote),
    ("thinclaw_mcp_server_prompts", RouteMode::LocalAndRemote),
    ("thinclaw_mcp_server_resources", RouteMode::LocalAndRemote),
    ("thinclaw_mcp_server_tools", RouteMode::LocalAndRemote),
    ("thinclaw_mcp_servers", RouteMode::LocalAndRemote),
    ("thinclaw_mcp_set_log_level", RouteMode::LocalAndRemote),
    ("thinclaw_memory_search", RouteMode::LocalAndRemote),
    ("thinclaw_pairing_approve", RouteMode::LocalAndRemote),
    ("thinclaw_pairing_list", RouteMode::LocalAndRemote),
    ("thinclaw_redo", RouteMode::LocalAndRemote),
    ("thinclaw_reset_session", RouteMode::LocalAndRemote),
    ("thinclaw_resolve_approval", RouteMode::LocalAndRemote),
    ("thinclaw_routine_audit_list", RouteMode::LocalAndRemote),
    ("thinclaw_routine_create", RouteMode::LocalAndRemote),
    ("thinclaw_routine_delete", RouteMode::LocalAndRemote),
    ("thinclaw_routine_toggle", RouteMode::LocalAndRemote),
    ("thinclaw_routing_get", RouteMode::LocalAndRemote),
    ("thinclaw_routing_pools_save", RouteMode::LocalAndRemote),
    ("thinclaw_routing_rules_add", RouteMode::LocalAndRemote),
    ("thinclaw_routing_rules_list", RouteMode::LocalAndRemote),
    ("thinclaw_routing_rules_remove", RouteMode::LocalAndRemote),
    ("thinclaw_routing_rules_reorder", RouteMode::LocalAndRemote),
    ("thinclaw_routing_rules_save", RouteMode::LocalAndRemote),
    ("thinclaw_routing_set", RouteMode::LocalAndRemote),
    ("thinclaw_routing_simulate", RouteMode::LocalAndRemote),
    ("thinclaw_routing_status", RouteMode::LocalAndRemote),
    ("thinclaw_save_anthropic_key", RouteMode::LocalAndRemote),
    ("thinclaw_save_cloud_config", RouteMode::LocalAndRemote),
    ("thinclaw_save_gemini_key", RouteMode::LocalAndRemote),
    ("thinclaw_save_groq_key", RouteMode::LocalAndRemote),
    (
        "thinclaw_save_implicit_provider_key",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_save_memory", RouteMode::LocalAndRemote),
    ("thinclaw_save_openai_key", RouteMode::LocalAndRemote),
    ("thinclaw_save_openrouter_key", RouteMode::LocalAndRemote),
    (
        "thinclaw_save_selected_cloud_model",
        RouteMode::LocalAndRemote,
    ),
    ("thinclaw_send_message", RouteMode::LocalAndRemote),
    ("thinclaw_set_thinking", RouteMode::LocalAndRemote),
    ("thinclaw_skill_inspect", RouteMode::LocalAndRemote),
    ("thinclaw_skill_install", RouteMode::LocalAndRemote),
    ("thinclaw_skill_publish", RouteMode::LocalAndRemote),
    ("thinclaw_skill_reload", RouteMode::LocalAndRemote),
    ("thinclaw_skill_remove", RouteMode::LocalAndRemote),
    ("thinclaw_skill_trust", RouteMode::LocalAndRemote),
    ("thinclaw_skills_list", RouteMode::LocalAndRemote),
    ("thinclaw_skills_reload_all", RouteMode::LocalAndRemote),
    ("thinclaw_skills_search", RouteMode::LocalAndRemote),
    ("thinclaw_skills_status", RouteMode::LocalAndRemote),
    ("thinclaw_start_gateway", RouteMode::LocalAndRemote),
    ("thinclaw_stop_gateway", RouteMode::LocalAndRemote),
    ("thinclaw_subscribe_session", RouteMode::LocalAndRemote),
    ("thinclaw_switch_to_profile", RouteMode::LocalAndRemote),
    ("thinclaw_system_presence", RouteMode::LocalAndRemote),
    ("thinclaw_tool_policy_get", RouteMode::LocalAndRemote),
    ("thinclaw_tool_policy_set", RouteMode::LocalAndRemote),
    ("thinclaw_tools_list", RouteMode::LocalAndRemote),
    ("thinclaw_undo", RouteMode::LocalAndRemote),
    ("thinclaw_write_file", RouteMode::LocalAndRemote),
];

/// Look up the [`RouteMode`] for a Tauri command name.
///
/// Returns `None` when the command is not yet registered in the route table —
/// this is intentional: the table is additive and commands are enrolled
/// incrementally.
pub fn route_mode(command: &str) -> Option<RouteMode> {
    ROUTE_TABLE
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, mode)| *mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gated_builds_unavailable_with_remediation() {
        let err = gated(
            "manual outcome evaluation",
            "requires the gateway outcome service",
            "connect a remote gateway",
            RouteMode::RemoteOnly,
        );
        match &err {
            BridgeError::Unavailable {
                capability,
                remediation,
                satisfied_by,
                ..
            } => {
                assert_eq!(capability, "manual outcome evaluation");
                assert_eq!(remediation.as_deref(), Some("connect a remote gateway"));
                assert_eq!(*satisfied_by, RouteMode::RemoteOnly);
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
        assert!(err
            .to_string()
            .contains("unavailable: manual outcome evaluation"));
    }

    #[test]
    fn gated_serializes_with_kind_tag() {
        let err = gated("x", "y", "z", RouteMode::LocalOnly);
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "unavailable");
        assert_eq!(json["satisfied_by"], "local_only");
    }

    #[test]
    fn string_error_maps_to_runtime() {
        let err: BridgeError = "boom".to_string().into();
        assert_eq!(
            err,
            BridgeError::Runtime {
                message: "boom".to_string()
            }
        );
    }

    #[test]
    fn runtime_error_serializes_with_kind_tag() {
        // Regression guard: the internally-tagged enum must stay serde/specta
        // exportable — a tuple variant here breaks `cargo run --example export_bindings`.
        let err: BridgeError = "boom".to_string().into();
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "runtime");
        assert_eq!(json["message"], "boom");
    }

    // ---- route table tests (TDO-002) ----------------------------------------

    #[test]
    fn route_table_is_non_empty() {
        assert!(
            !ROUTE_TABLE.is_empty(),
            "ROUTE_TABLE must have at least one entry"
        );
    }

    #[test]
    fn route_table_command_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (name, _) in ROUTE_TABLE {
            assert!(
                seen.insert(*name),
                "duplicate command name in ROUTE_TABLE: {name}"
            );
        }
    }

    fn snake_to_camel(s: &str) -> String {
        let mut out = String::new();
        let mut upper = false;
        for c in s.chars() {
            if c == '_' {
                upper = true;
            } else if upper {
                out.push(c.to_ascii_uppercase());
                upper = false;
            } else {
                out.push(c);
            }
        }
        out
    }

    fn camel_to_snake(s: &str) -> String {
        let mut out = String::new();
        for c in s.chars() {
            if c.is_ascii_uppercase() {
                out.push('_');
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    }

    /// Every command listed in ROUTE_TABLE must be a real registered command
    /// (present in the generated bindings) — guards against typos/stale rows.
    #[test]
    fn route_table_commands_are_registered() {
        let bindings = include_str!("../../../frontend/src/lib/bindings.ts");
        for (cmd, _) in ROUTE_TABLE {
            let camel = snake_to_camel(cmd);
            assert!(
                bindings.contains(&format!("async {camel}(")),
                "ROUTE_TABLE references `{cmd}` (`{camel}`) which is not a registered command in bindings.ts"
            );
        }
    }

    /// TDO-002 linter: every gated command (its generated binding returns
    /// `BridgeError`) must be classified in ROUTE_TABLE, so the route-matrix can
    /// never silently omit a gated capability.
    #[test]
    fn all_gated_commands_are_classified() {
        let bindings = include_str!("../../../frontend/src/lib/bindings.ts");
        let mut checked = 0;
        for line in bindings.lines() {
            let line = line.trim();
            if !line.starts_with("async ") || !line.contains("BridgeError>") {
                continue;
            }
            let name = line["async ".len()..]
                .split('(')
                .next()
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                continue;
            }
            let snake = camel_to_snake(name);
            assert!(
                route_mode(&snake).is_some(),
                "gated command `{snake}` returns BridgeError but is not classified in ROUTE_TABLE"
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "expected at least one gated (BridgeError) command in bindings.ts"
        );
    }

    #[test]
    fn route_mode_remote_only_command() {
        assert_eq!(
            route_mode("thinclaw_job_restart"),
            Some(RouteMode::RemoteOnly),
            "thinclaw_job_restart must be RemoteOnly"
        );
    }

    #[test]
    fn route_mode_unknown_command_returns_none() {
        assert_eq!(route_mode("nope"), None);
    }

    /// TOTAL-coverage linter (B4): EVERY registered command (each `async NAME(` in
    /// the generated bindings) must be classified in ROUTE_TABLE. A newly-added
    /// command fails this test until its RouteMode is declared — no command can be
    /// silently omitted from the route matrix.
    #[test]
    fn all_registered_commands_are_classified() {
        let bindings = include_str!("../../../frontend/src/lib/bindings.ts");
        let mut checked = 0;
        let mut missing: Vec<String> = Vec::new();
        for line in bindings.lines() {
            let line = line.trim_start();
            let Some(rest) = line.strip_prefix("async ") else {
                continue;
            };
            let Some(name) = rest.split('(').next() else {
                continue;
            };
            // Real command declarations look like `async name(args) : Promise<..`.
            if name.is_empty() || !rest.contains(") : Promise<") {
                continue;
            }
            let snake = camel_to_snake(name);
            if route_mode(&snake).is_none() {
                missing.push(format!("{name} ({snake})"));
            }
            checked += 1;
        }
        assert!(
            checked >= 340,
            "expected >=340 registered commands in bindings.ts, found {checked}"
        );
        assert!(
            missing.is_empty(),
            "{} registered command(s) not classified in ROUTE_TABLE (add a RouteMode entry): {:?}",
            missing.len(),
            missing
        );
    }
}
