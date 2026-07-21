use crate::*;

#[test]
fn db_settings_ignore_malformed_paths_without_losing_valid_values() {
    let mut map = std::collections::HashMap::new();
    map.insert("".to_string(), serde_json::json!("ignored"));
    map.insert("agent..name".to_string(), serde_json::json!("ignored"));
    map.insert("agent.name".to_string(), serde_json::json!("safe-name"));

    let restored = Settings::from_db_map(&map);
    assert_eq!(restored.agent.name, "safe-name");
}

#[test]
fn settings_debug_never_exposes_persisted_credentials_or_personal_allowlists() {
    let mut settings = Settings {
        database_url: Some("postgres://alice:database-password@db.example/thinclaw".into()),
        tunnel: TunnelSettings {
            cf_token: Some("cloudflare-secret".into()),
            ngrok_token: Some("ngrok-secret".into()),
            ..TunnelSettings::default()
        },
        ..Settings::default()
    };
    settings.channels.discord_bot_token = Some("discord-secret".into());
    settings.channels.slack_bot_token = Some("slack-bot-secret".into());
    settings.channels.slack_app_token = Some("slack-app-secret".into());
    settings.channels.bluebubbles_password = Some("bluebubbles-secret".into());
    settings.channels.gateway_auth_token = Some("gateway-secret".into());
    settings.channels.signal_allow_from = Some("+49123456789".into());
    settings
        .channels
        .gateway_principals
        .push(GatewayPrincipalConfig {
            token: "principal-secret".into(),
            principal_id: "operator".into(),
            actor_id: None,
            role: GatewayRole::Operator,
        });

    let debug = format!("{settings:?}");
    for sensitive in [
        "database-password",
        "cloudflare-secret",
        "ngrok-secret",
        "discord-secret",
        "slack-bot-secret",
        "slack-app-secret",
        "bluebubbles-secret",
        "gateway-secret",
        "principal-secret",
        "+49123456789",
    ] {
        assert!(!debug.contains(sensitive), "debug leaked {sensitive}");
    }
    assert!(debug.contains("[REDACTED]"));
}

#[test]
fn test_db_map_round_trip() {
    let settings = Settings {
        selected_model: Some("claude-3-5-sonnet-20241022".to_string()),
        ..Default::default()
    };

    let map = settings.to_db_map();
    let restored = Settings::from_db_map(&map);
    assert_eq!(
        restored.selected_model,
        Some("claude-3-5-sonnet-20241022".to_string())
    );
}

#[test]
fn test_get_setting() {
    let settings = Settings::default();

    assert_eq!(settings.get("agent.name"), Some("thinclaw".to_string()));
    assert_eq!(
        settings.get("agent.max_parallel_jobs"),
        Some("5".to_string())
    );
    assert_eq!(settings.get("heartbeat.enabled"), Some("false".to_string()));
    assert_eq!(settings.get("nonexistent"), None);
}

#[test]
fn test_set_setting() {
    let mut settings = Settings::default();

    settings.set("agent.name", "mybot").unwrap();
    assert_eq!(settings.agent.name, "mybot");

    settings.set("agent.max_parallel_jobs", "10").unwrap();
    assert_eq!(settings.agent.max_parallel_jobs, 10);

    settings.set("heartbeat.enabled", "true").unwrap();
    assert!(settings.heartbeat.enabled);

    // Array field: JSON array syntax works
    settings
        .set(
            "providers.fallback_chain",
            "[\"openai/gpt-4o\",\"groq/llama-3.3-70b\"]",
        )
        .unwrap();
    assert_eq!(
        settings.providers.fallback_chain,
        vec!["openai/gpt-4o", "groq/llama-3.3-70b"]
    );

    // Array field: comma-separated string is auto-split into array
    settings
        .set(
            "providers.fallback_chain",
            "openai/gpt-4o, groq/llama-3.3-70b",
        )
        .unwrap();
    assert_eq!(
        settings.providers.fallback_chain,
        vec!["openai/gpt-4o", "groq/llama-3.3-70b"]
    );

    // Array field: empty string results in empty array
    settings.set("providers.fallback_chain", "").unwrap();
    assert!(settings.providers.fallback_chain.is_empty());
}

#[test]
fn test_reset_setting() {
    let mut settings = Settings::default();

    settings.agent.name = "custom".to_string();
    settings.reset("agent.name").unwrap();
    assert_eq!(settings.agent.name, "thinclaw");
}

#[test]
fn test_list_settings() {
    let settings = Settings::default();
    let list = settings.list();

    // Check some expected entries
    assert!(list.iter().any(|(k, _)| k == "agent.name"));
    assert!(list.iter().any(|(k, _)| k == "heartbeat.enabled"));
    assert!(list.iter().any(|(k, _)| k == "onboard_completed"));
}

#[test]
fn test_key_source_serialization() {
    let settings = Settings {
        secrets_master_key_source: KeySource::Keychain,
        ..Default::default()
    };

    let json = serde_json::to_string(&settings).unwrap();
    assert!(json.contains("\"keychain\""));

    let loaded: Settings = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded.secrets_master_key_source, KeySource::Keychain);
}

#[test]
fn test_embeddings_defaults() {
    let settings = Settings::default();
    assert!(!settings.embeddings.enabled);
    assert_eq!(settings.embeddings.provider, "openai");
    assert_eq!(settings.embeddings.model, "text-embedding-3-small");
}

#[test]
fn test_telegram_owner_id_db_round_trip() {
    let mut settings = Settings::default();
    settings.channels.telegram_owner_id = Some(123456789);

    let map = settings.to_db_map();
    let restored = Settings::from_db_map(&map);
    assert_eq!(restored.channels.telegram_owner_id, Some(123456789));
}

#[test]
fn test_telegram_owner_id_default_none() {
    let settings = Settings::default();
    assert_eq!(settings.channels.telegram_owner_id, None);
}

#[test]
fn test_telegram_owner_id_via_set() {
    let mut settings = Settings::default();
    settings
        .set("channels.telegram_owner_id", "987654321")
        .unwrap();
    assert_eq!(settings.channels.telegram_owner_id, Some(987654321));
}

#[test]
fn test_subagent_transparency_defaults_and_set() {
    let mut settings = Settings::default();
    assert_eq!(settings.agent.subagent_transparency_level, "balanced");

    settings
        .set("agent.subagent_transparency_level", "detailed")
        .unwrap();
    assert_eq!(settings.agent.subagent_transparency_level, "detailed");
}

#[test]
fn test_telegram_subagent_session_mode_defaults_and_round_trip() {
    let mut settings = Settings::default();
    assert_eq!(
        settings.channels.telegram_subagent_session_mode,
        "temp_topic"
    );

    settings
        .set("channels.telegram_subagent_session_mode", "reply_chain")
        .unwrap();
    assert_eq!(
        settings.channels.telegram_subagent_session_mode,
        "reply_chain"
    );

    let map = settings.to_db_map();
    let restored = Settings::from_db_map(&map);
    assert_eq!(
        restored.channels.telegram_subagent_session_mode,
        "reply_chain"
    );
}

#[test]
fn test_telegram_transport_mode_defaults_and_round_trip() {
    let mut settings = Settings::default();
    assert_eq!(settings.channels.telegram_transport_mode, "auto");

    settings
        .set("channels.telegram_transport_mode", "polling")
        .unwrap();
    assert_eq!(settings.channels.telegram_transport_mode, "polling");

    let map = settings.to_db_map();
    let restored = Settings::from_db_map(&map);
    assert_eq!(restored.channels.telegram_transport_mode, "polling");
}

/// Regression test: numeric-looking chat IDs stored as JSON strings in the
/// DB must round-trip correctly into Option<String> fields.
#[test]
fn test_notification_recipient_db_round_trip() {
    let mut settings = Settings::default();
    settings.notifications.recipient = Some("684480568".to_string());

    let map = settings.to_db_map();
    let restored = Settings::from_db_map(&map);
    assert_eq!(
        restored.notifications.recipient,
        Some("684480568".to_string()),
        "numeric-looking recipient must survive DB round-trip as String"
    );
}

/// Regression test: set() with a numeric-looking string into an
/// Option<String> field (existing value is Null) must produce Some(String).
#[test]
fn test_notification_recipient_via_set() {
    let mut settings = Settings::default();
    settings
        .set("notifications.recipient", "684480568")
        .unwrap();
    assert_eq!(
        settings.notifications.recipient,
        Some("684480568".to_string()),
        "set() must coerce numeric-looking value into String for Option<String> fields"
    );
}

#[test]
fn test_llm_backend_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");

    let settings = Settings {
        llm_backend: Some("anthropic".to_string()),
        ollama_base_url: Some("http://localhost:11434".to_string()),
        openai_compatible_base_url: Some("http://my-vllm:8000/v1".to_string()),
        ..Default::default()
    };
    let json = serde_json::to_string_pretty(&settings).unwrap();
    std::fs::write(&path, json).unwrap();

    let loaded = Settings::load_from(&path);
    assert_eq!(loaded.llm_backend, Some("anthropic".to_string()));
    assert_eq!(
        loaded.ollama_base_url,
        Some("http://localhost:11434".to_string())
    );
    assert_eq!(
        loaded.openai_compatible_base_url,
        Some("http://my-vllm:8000/v1".to_string())
    );
}

#[test]
fn test_openai_compatible_db_map_round_trip() {
    let settings = Settings {
        llm_backend: Some("openai_compatible".to_string()),
        openai_compatible_base_url: Some("http://my-vllm:8000/v1".to_string()),
        embeddings: EmbeddingsSettings {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let map = settings.to_db_map();
    let restored = Settings::from_db_map(&map);

    assert_eq!(
        restored.llm_backend,
        Some("openai_compatible".to_string()),
        "llm_backend must survive DB round-trip"
    );
    assert_eq!(
        restored.openai_compatible_base_url,
        Some("http://my-vllm:8000/v1".to_string()),
        "openai_compatible_base_url must survive DB round-trip"
    );
    assert!(
        !restored.embeddings.enabled,
        "embeddings.enabled=false must survive DB round-trip"
    );
}

#[test]
fn toml_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut settings = Settings::default();
    settings.agent.name = "toml-bot".to_string();
    settings.heartbeat.enabled = true;
    settings.heartbeat.interval_secs = 900;

    settings.save_toml(&path).unwrap();
    let loaded = Settings::load_toml(&path).unwrap().unwrap();

    assert_eq!(loaded.agent.name, "toml-bot");
    assert!(loaded.heartbeat.enabled);
    assert_eq!(loaded.heartbeat.interval_secs, 900);
}

#[test]
fn toml_missing_file_returns_none() {
    let result = Settings::load_toml(std::path::Path::new("/tmp/nonexistent_config.toml"));
    assert!(result.unwrap().is_none());
}

#[test]
fn toml_invalid_content_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.toml");
    std::fs::write(&path, "this is not valid toml [[[").unwrap();

    let result = Settings::load_toml(&path);
    assert!(result.is_err());
}

#[test]
fn toml_partial_config_uses_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("partial.toml");

    // Only set agent name, everything else should be default
    std::fs::write(&path, "[agent]\nname = \"partial-bot\"\n").unwrap();

    let loaded = Settings::load_toml(&path).unwrap().unwrap();
    assert_eq!(loaded.agent.name, "partial-bot");
    // Defaults preserved
    assert_eq!(loaded.agent.max_parallel_jobs, 5);
    assert!(!loaded.heartbeat.enabled);
}

#[test]
fn toml_header_comment_present() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    Settings::default().save_toml(&path).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();

    assert!(content.starts_with("# ThinClaw configuration file."));
    assert!(content.contains("[agent]"));
    assert!(content.contains("[heartbeat]"));
}

#[test]
fn merge_only_overrides_non_default_values() {
    let mut base = Settings::default();
    base.agent.name = "from-db".to_string();
    base.heartbeat.interval_secs = 600;

    let mut toml_overlay = Settings::default();
    toml_overlay.agent.name = "from-toml".to_string();

    base.merge_from(&toml_overlay);

    assert_eq!(base.agent.name, "from-toml");
    assert_eq!(base.heartbeat.interval_secs, 600);
}

#[test]
fn merge_preserves_base_when_overlay_is_default() {
    let mut base = Settings::default();
    base.agent.name = "custom-name".to_string();
    base.heartbeat.enabled = true;

    let overlay = Settings::default();
    base.merge_from(&overlay);

    assert_eq!(base.agent.name, "custom-name");
    assert!(base.heartbeat.enabled);
}

#[test]
fn toml_creates_parent_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("deep").join("config.toml");

    Settings::default().save_toml(&path).unwrap();
    assert!(path.exists());
}

#[test]
fn default_toml_path_under_thinclaw() {
    let path = Settings::default_toml_path();
    assert!(path.to_string_lossy().contains(".thinclaw"));
    assert!(path.to_string_lossy().ends_with("config.toml"));
}

#[test]
fn tunnel_settings_round_trip() {
    let settings = Settings {
        tunnel: TunnelSettings {
            provider: Some("ngrok".to_string()),
            ngrok_token: Some("tok_abc123".to_string()),
            ngrok_domain: Some("my.ngrok.dev".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };

    // JSON round-trip
    let json = serde_json::to_string(&settings).unwrap();
    let restored: Settings = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.tunnel.provider, Some("ngrok".to_string()));
    assert_eq!(restored.tunnel.ngrok_token, Some("tok_abc123".to_string()));
    assert_eq!(
        restored.tunnel.ngrok_domain,
        Some("my.ngrok.dev".to_string())
    );
    assert!(restored.tunnel.public_url.is_none());

    // DB map round-trip
    let map = settings.to_db_map();
    let from_db = Settings::from_db_map(&map);
    assert_eq!(from_db.tunnel.provider, Some("ngrok".to_string()));
    assert_eq!(from_db.tunnel.ngrok_token, Some("tok_abc123".to_string()));

    // get/set round-trip
    let mut s = Settings::default();
    s.set("tunnel.provider", "cloudflare").unwrap();
    s.set("tunnel.cf_token", "cf_tok_xyz").unwrap();
    s.set("tunnel.ts_funnel", "true").unwrap();
    assert_eq!(s.tunnel.provider, Some("cloudflare".to_string()));
    assert_eq!(s.tunnel.cf_token, Some("cf_tok_xyz".to_string()));
    assert!(s.tunnel.ts_funnel);
}

/// Simulates the wizard recovery scenario:
///
/// 1. A prior partial run saved steps 1-4 to the DB
/// 2. User re-runs the wizard, Step 1 sets a new database_url
/// 3. Prior settings are loaded from the DB
/// 4. Step 1's fresh choices must win over stale DB values
///
/// This tests the ordering: load DB → merge_from(step1_overrides).
#[test]
fn wizard_recovery_step1_overrides_stale_db() {
    // Simulate prior partial run (steps 1-4 completed):
    let prior_run = Settings {
        database_backend: Some("postgres".to_string()),
        database_url: Some("postgres://old-host/thinclaw".to_string()),
        llm_backend: Some("anthropic".to_string()),
        selected_model: Some("claude-sonnet-4-5".to_string()),
        embeddings: EmbeddingsSettings {
            enabled: true,
            provider: "openai".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };

    // Save to DB and reload (simulates persistence round-trip)
    let db_map = prior_run.to_db_map();
    let from_db = Settings::from_db_map(&db_map);

    // Step 1 of the new wizard run: user enters a NEW database_url
    let step1_settings = Settings {
        database_backend: Some("postgres".to_string()),
        database_url: Some("postgres://new-host/thinclaw".to_string()),
        ..Settings::default()
    };

    // Wizard flow: load DB → merge_from(step1_overrides)
    let mut current = step1_settings.clone();
    // try_load_existing_settings: merge DB into current
    current.merge_from(&from_db);
    // Re-apply Step 1 choices on top
    current.merge_from(&step1_settings);

    // Step 1's fresh database_url wins over stale DB value
    assert_eq!(
        current.database_url,
        Some("postgres://new-host/thinclaw".to_string()),
        "Step 1 fresh choice must override stale DB value"
    );

    // Prior run's steps 2-4 settings are preserved
    assert_eq!(
        current.llm_backend,
        Some("anthropic".to_string()),
        "Prior run's LLM backend must be recovered"
    );
    assert_eq!(
        current.selected_model,
        Some("claude-sonnet-4-5".to_string()),
        "Prior run's model must be recovered"
    );
    assert!(
        current.embeddings.enabled,
        "Prior run's embeddings setting must be recovered"
    );
}

/// Verifies that persisting defaults doesn't clobber prior settings
/// when the merge ordering is correct.
#[test]
fn wizard_recovery_defaults_dont_clobber_prior() {
    // Prior run saved non-default settings
    let prior_run = Settings {
        llm_backend: Some("openai".to_string()),
        selected_model: Some("gpt-4o".to_string()),
        heartbeat: HeartbeatSettings {
            enabled: true,
            interval_secs: 900,
            ..Default::default()
        },
        ..Default::default()
    };
    let db_map = prior_run.to_db_map();
    let from_db = Settings::from_db_map(&db_map);

    // New wizard run: Step 1 only sets DB fields (rest is default)
    let step1 = Settings {
        database_backend: Some("libsql".to_string()),
        ..Default::default()
    };

    // Correct merge ordering
    let mut current = step1.clone();
    current.merge_from(&from_db);
    current.merge_from(&step1);

    // Prior settings preserved (Step 1 doesn't touch these)
    assert_eq!(current.llm_backend, Some("openai".to_string()));
    assert_eq!(current.selected_model, Some("gpt-4o".to_string()));
    assert!(current.heartbeat.enabled);
    assert_eq!(current.heartbeat.interval_secs, 900);

    // Step 1's choice applied
    assert_eq!(current.database_backend, Some("libsql".to_string()));
}

/// Regression test: per-provider model slots stored in the `provider_models`
/// HashMap must survive the `to_db_map` → `from_db_map` roundtrip.
///
/// The old `from_db_map` used `set()` per-key, which silently failed for
/// dynamic HashMap keys like `providers.provider_models.openai.cheap`
/// because the intermediate `"openai"` key didn't exist in the default
/// empty map.  This caused the user's cheap model selection to be lost
/// after every save.
#[test]
fn test_provider_models_db_round_trip() {
    let mut settings = Settings::default();
    settings.providers.provider_models.insert(
        "openai".to_string(),
        ProviderModelSlots {
            primary: Some("gpt-4o".to_string()),
            cheap: Some("gpt-4o-mini".to_string()),
        },
    );
    settings.providers.provider_models.insert(
        "anthropic".to_string(),
        ProviderModelSlots {
            primary: Some("claude-opus-4-7".to_string()),
            cheap: Some("claude-sonnet-4-6".to_string()),
        },
    );
    settings.providers.enabled = vec!["openai".to_string(), "anthropic".to_string()];
    settings.providers.primary = Some("anthropic".to_string());
    settings.providers.primary_model = Some("claude-opus-4-7".to_string());
    settings.providers.cheap_model = Some("openai/gpt-4o-mini".to_string());
    settings.providers.preferred_cheap_provider = Some("openai".to_string());

    let map = settings.to_db_map();
    let restored = Settings::from_db_map(&map);

    // Primary provider settings survive
    assert_eq!(restored.providers.primary, Some("anthropic".to_string()));
    assert_eq!(
        restored.providers.primary_model,
        Some("claude-opus-4-7".to_string())
    );

    // Cheap model settings survive
    assert_eq!(
        restored.providers.cheap_model,
        Some("openai/gpt-4o-mini".to_string())
    );
    assert_eq!(
        restored.providers.preferred_cheap_provider,
        Some("openai".to_string())
    );

    // Per-provider model slots survive (this was the bug)
    let openai_slots = restored
        .providers
        .provider_models
        .get("openai")
        .expect("openai provider_models entry must survive roundtrip");
    assert_eq!(openai_slots.primary, Some("gpt-4o".to_string()));
    assert_eq!(openai_slots.cheap, Some("gpt-4o-mini".to_string()));

    let anthropic_slots = restored
        .providers
        .provider_models
        .get("anthropic")
        .expect("anthropic provider_models entry must survive roundtrip");
    assert_eq!(anthropic_slots.primary, Some("claude-opus-4-7".to_string()));
    assert_eq!(anthropic_slots.cheap, Some("claude-sonnet-4-6".to_string()));
}

#[test]
fn test_learning_mutations_require_explicit_opt_in() {
    let settings = Settings::default();
    assert!(!settings.learning.prompt_mutation.enabled);
    assert!(settings.learning.auto_apply_classes.is_empty());
}

#[test]
fn test_repo_projects_defaults_and_set_round_trip() {
    let mut settings = Settings::default();
    assert!(!settings.repo_projects.enabled);
    assert_eq!(settings.repo_projects.max_concurrent_projects, 1);
    assert_eq!(settings.repo_projects.max_concurrent_tasks_per_project, 1);
    assert_eq!(settings.repo_projects.default_coding_backend, "worker");
    assert_eq!(settings.repo_projects.default_write_mode, "fork_pr");
    assert!(!settings.repo_projects.auto_merge_default);
    assert_eq!(settings.repo_projects.watchdog_interval_secs, 60);
    assert!(settings.repo_projects.workspace_base_dir.is_none());
    assert!(settings.repo_projects.github_app.app_id.is_none());
    assert!(settings.repo_projects.github_app.installation_id.is_none());
    assert!(
        settings
            .repo_projects
            .github_app
            .private_key_secret
            .is_none()
    );
    assert!(
        settings
            .repo_projects
            .github_app
            .webhook_secret_secret
            .is_none()
    );

    settings.set("repo_projects.enabled", "true").unwrap();
    settings
        .set("repo_projects.max_concurrent_projects", "3")
        .unwrap();
    settings
        .set("repo_projects.max_concurrent_tasks_per_project", "2")
        .unwrap();
    settings
        .set("repo_projects.default_coding_backend", "codex_code")
        .unwrap();
    settings
        .set("repo_projects.default_write_mode", "maintainer_branch_pr")
        .unwrap();
    settings
        .set("repo_projects.auto_merge_default", "true")
        .unwrap();
    settings
        .set("repo_projects.watchdog_interval_secs", "45")
        .unwrap();
    settings
        .set("repo_projects.workspace_base_dir", "/tmp/thinclaw-repos")
        .unwrap();
    settings
        .set("repo_projects.github_app.app_id", "123")
        .unwrap();
    settings
        .set("repo_projects.github_app.installation_id", "456")
        .unwrap();
    settings
        .set(
            "repo_projects.github_app.private_key_secret",
            "repo_projects_github_private_key",
        )
        .unwrap();
    settings
        .set(
            "repo_projects.github_app.webhook_secret_secret",
            "repo_projects_github_webhook",
        )
        .unwrap();

    let restored = Settings::from_db_map(&settings.to_db_map());
    assert!(restored.repo_projects.enabled);
    assert_eq!(restored.repo_projects.max_concurrent_projects, 3);
    assert_eq!(restored.repo_projects.max_concurrent_tasks_per_project, 2);
    assert_eq!(restored.repo_projects.default_coding_backend, "codex_code");
    assert_eq!(
        restored.repo_projects.default_write_mode,
        "maintainer_branch_pr"
    );
    assert!(restored.repo_projects.auto_merge_default);
    assert_eq!(restored.repo_projects.watchdog_interval_secs, 45);
    assert_eq!(
        restored.repo_projects.workspace_base_dir.as_deref(),
        Some("/tmp/thinclaw-repos")
    );
    assert_eq!(restored.repo_projects.github_app.app_id, Some(123));
    assert_eq!(restored.repo_projects.github_app.installation_id, Some(456));
    assert_eq!(
        restored
            .repo_projects
            .github_app
            .private_key_secret
            .as_deref(),
        Some("repo_projects_github_private_key")
    );
    assert_eq!(
        restored
            .repo_projects
            .github_app
            .webhook_secret_secret
            .as_deref(),
        Some("repo_projects_github_webhook")
    );
}

#[test]
fn test_repo_projects_toml_section() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repo-projects.toml");

    std::fs::write(
        &path,
        r#"
[repo_projects]
enabled = true
max_concurrent_projects = 4
max_concurrent_tasks_per_project = 2
default_coding_backend = "claude_code"
default_write_mode = "read_only_clone"
auto_merge_default = true
watchdog_interval_secs = 30
workspace_base_dir = "/tmp/repo-project-workspaces"

[repo_projects.github_app]
app_id = 123
installation_id = 456
private_key_secret = "repo_projects_github_private_key"
webhook_secret_secret = "repo_projects_github_webhook"
"#,
    )
    .unwrap();

    let loaded = Settings::load_toml(&path).unwrap().unwrap();
    assert!(loaded.repo_projects.enabled);
    assert_eq!(loaded.repo_projects.max_concurrent_projects, 4);
    assert_eq!(loaded.repo_projects.max_concurrent_tasks_per_project, 2);
    assert_eq!(loaded.repo_projects.default_coding_backend, "claude_code");
    assert_eq!(loaded.repo_projects.default_write_mode, "read_only_clone");
    assert!(loaded.repo_projects.auto_merge_default);
    assert_eq!(loaded.repo_projects.watchdog_interval_secs, 30);
    assert_eq!(
        loaded.repo_projects.workspace_base_dir.as_deref(),
        Some("/tmp/repo-project-workspaces")
    );
    assert_eq!(loaded.repo_projects.github_app.app_id, Some(123));
    assert_eq!(loaded.repo_projects.github_app.installation_id, Some(456));
    assert_eq!(
        loaded
            .repo_projects
            .github_app
            .private_key_secret
            .as_deref(),
        Some("repo_projects_github_private_key")
    );
    assert_eq!(
        loaded
            .repo_projects
            .github_app
            .webhook_secret_secret
            .as_deref(),
        Some("repo_projects_github_webhook")
    );
}
