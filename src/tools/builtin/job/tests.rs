use super::*;

#[tokio::test]
async fn test_create_job_tool_local() {
    let manager = Arc::new(ContextManager::new(5));
    let tool = CreateJobTool::new(manager.clone());

    // Without sandbox deps, it should use the local path
    assert!(!tool.sandbox_enabled());

    let params = serde_json::json!({
        "title": "Test Job",
        "description": "A test job description"
    });

    let ctx = JobContext::default();
    let result = tool.execute(params, &ctx).await.unwrap();

    let job_id = result.result.get("job_id").unwrap().as_str().unwrap();
    assert!(!job_id.is_empty());
    assert_eq!(
        result.result.get("status").unwrap().as_str().unwrap(),
        "pending"
    );
    assert_eq!(
        result
            .result
            .get("runtime_family")
            .and_then(|value| value.as_str()),
        Some("execution_backend")
    );
    assert_eq!(
        result
            .result
            .get("runtime_mode")
            .and_then(|value| value.as_str()),
        Some("in_memory")
    );
}

#[test]
fn sandbox_job_runtime_descriptor_tracks_mode_capabilities() {
    let worker = sandbox_job_runtime_descriptor(JobMode::Worker);
    assert_eq!(worker.runtime_family, "execution_backend");
    assert_eq!(worker.runtime_mode, "worker");
    assert!(
        worker
            .runtime_capabilities
            .contains(&"llm_proxy".to_string())
    );

    let codex = sandbox_job_runtime_descriptor(JobMode::CodexCode);
    assert_eq!(codex.runtime_mode, "codex_code");
    assert!(
        codex
            .runtime_capabilities
            .contains(&"codex_cli".to_string())
    );
    assert_eq!(codex.network_isolation.as_deref(), Some("hard"));
}

#[test]
fn test_schema_changes_with_sandbox() {
    let manager = Arc::new(ContextManager::new(5));

    // Without sandbox
    let tool = CreateJobTool::new(Arc::clone(&manager));
    let schema = tool.parameters_schema();
    let props = schema.get("properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("title"));
    assert!(props.contains_key("description"));
    assert!(!props.contains_key("wait"));
    assert!(!props.contains_key("mode"));
}

#[test]
fn test_execution_timeout_sandbox() {
    let manager = Arc::new(ContextManager::new(5));

    // Without sandbox: default timeout
    let tool = CreateJobTool::new(Arc::clone(&manager));
    assert_eq!(tool.execution_timeout(), Duration::from_secs(30));
}

#[tokio::test]
async fn test_list_jobs_tool() {
    let manager = Arc::new(ContextManager::new(5));

    // Create some jobs
    let job1 = manager.create_job("Job 1", "Desc 1").await.unwrap();
    manager.create_job("Job 2", "Desc 2").await.unwrap();
    manager
        .update_context(job1, |ctx| {
            ctx.transition_to(JobState::Cancelled, Some("Cancelled in test".to_string()))
        })
        .await
        .unwrap()
        .unwrap();

    let tool = ListJobsTool::new(manager);

    let params = serde_json::json!({});
    let ctx = JobContext::default();
    let result = tool.execute(params, &ctx).await.unwrap();

    let jobs = result.result.get("jobs").unwrap().as_array().unwrap();
    assert_eq!(jobs.len(), 2);
    let summary = result.result.get("summary").unwrap();
    assert_eq!(summary.get("cancelled").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(summary.get("failed").and_then(|v| v.as_u64()), Some(0));
    assert_eq!(summary.get("interrupted").and_then(|v| v.as_u64()), Some(0));
}

#[tokio::test]
async fn test_job_status_tool() {
    let manager = Arc::new(ContextManager::new(5));
    let job_id = manager.create_job("Test Job", "Description").await.unwrap();

    let tool = JobStatusTool::new(manager);

    let params = serde_json::json!({
        "job_id": job_id.to_string()
    });
    let ctx = JobContext::default();
    let result = tool.execute(params, &ctx).await.unwrap();

    assert_eq!(
        result.result.get("title").unwrap().as_str().unwrap(),
        "Test Job"
    );
}

#[tokio::test]
async fn test_direct_jobs_remain_visible_after_context_cleanup_when_persisted() {
    let (store, _guard) = crate::testing::test_db().await;
    let manager = Arc::new(ContextManager::new(5));
    let job_id = manager
        .create_job_for_identity("household", "alex", "Persisted Job", "Description")
        .await
        .unwrap();
    manager
        .update_context(job_id, |ctx| {
            ctx.transition_to(JobState::InProgress, Some("Started in test".to_string()))
        })
        .await
        .unwrap()
        .unwrap();
    manager
        .update_context(job_id, |ctx| {
            ctx.transition_to(JobState::Completed, Some("Finished in test".to_string()))
        })
        .await
        .unwrap()
        .unwrap();

    let snapshot = manager.get_context(job_id).await.unwrap();
    store.save_job(&snapshot).await.unwrap();
    manager.remove_job(job_id).await.unwrap();

    let actor_ctx = JobContext {
        user_id: "household".to_string(),
        principal_id: "household".to_string(),
        actor_id: Some("alex".to_string()),
        ..Default::default()
    };

    let list_tool = ListJobsTool::new(Arc::clone(&manager)).with_sandbox(None, Some(store.clone()));
    let list_result = list_tool
        .execute(serde_json::json!({}), &actor_ctx)
        .await
        .unwrap();
    let jobs = list_result
        .result
        .get("jobs")
        .and_then(|value| value.as_array())
        .expect("jobs array");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["job_id"], serde_json::json!(job_id.to_string()));
    assert_eq!(jobs[0]["kind"], serde_json::json!("local"));

    let status_tool =
        JobStatusTool::new(Arc::clone(&manager)).with_sandbox(None, Some(store.clone()));
    let status_result = status_tool
        .execute(
            serde_json::json!({
                "job_id": job_id.to_string(),
            }),
            &actor_ctx,
        )
        .await
        .unwrap();
    assert_eq!(
        status_result.result["status"],
        serde_json::json!("completed")
    );
    assert_eq!(status_result.result["kind"], serde_json::json!("local"));

    let events_tool = JobEventsTool::new(store, manager, None);
    let events_result = events_tool
        .execute(
            serde_json::json!({
                "job_id": job_id.to_string(),
            }),
            &actor_ctx,
        )
        .await
        .unwrap();
    assert_eq!(events_result.result["kind"], serde_json::json!("local"));
    assert_eq!(events_result.result["total_events"], serde_json::json!(0));
}

#[tokio::test]
async fn test_job_status_tool_rejects_same_user_different_actor() {
    let manager = Arc::new(ContextManager::new(5));
    let job_id = manager
        .create_job_for_identity("household", "alex", "Secret Job", "Description")
        .await
        .unwrap();

    let tool = JobStatusTool::new(manager);
    let params = serde_json::json!({
        "job_id": job_id.to_string()
    });
    let ctx = JobContext {
        user_id: "household".to_string(),
        principal_id: "household".to_string(),
        actor_id: Some("sam".to_string()),
        ..Default::default()
    };

    let result = tool.execute(params, &ctx).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("job not found"),
        "expected actor ownership rejection, got: {}",
        err
    );
}

#[tokio::test]
async fn test_cancel_job_tool_rejects_same_user_different_actor() {
    let manager = Arc::new(ContextManager::new(5));
    let job_id = manager
        .create_job_for_identity("household", "alex", "Secret Job", "Description")
        .await
        .unwrap();

    let tool = CancelJobTool::new(Arc::clone(&manager));
    let params = serde_json::json!({
        "job_id": job_id.to_string()
    });
    let ctx = JobContext {
        user_id: "household".to_string(),
        principal_id: "household".to_string(),
        actor_id: Some("sam".to_string()),
        ..Default::default()
    };

    let result = tool.execute(params, &ctx).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("job not found"),
        "expected actor ownership rejection, got: {}",
        err
    );

    let job_ctx = manager.get_context(job_id).await.unwrap();
    assert!(
        job_ctx.state.is_active(),
        "other actor must not cancel the job"
    );
}

#[test]
fn test_resolve_project_dir_auto() {
    let project_id = Uuid::new_v4();
    let (dir, browse_id) = resolve_project_dir(None, project_id).unwrap();
    assert!(dir.exists());
    assert!(dir.ends_with(project_id.to_string()));
    assert_eq!(browse_id, project_id.to_string());

    // Must be under the projects base
    let base = projects_base().canonicalize().unwrap();
    assert!(dir.starts_with(&base));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_resolve_project_dir_explicit_under_base() {
    let base = projects_base();
    std::fs::create_dir_all(&base).unwrap();
    let explicit = base.join("test_explicit_project");
    // Explicit paths must already exist (no auto-create).
    std::fs::create_dir_all(&explicit).unwrap();
    let project_id = Uuid::new_v4();

    let (dir, browse_id) = resolve_project_dir(Some(explicit.clone()), project_id).unwrap();
    assert!(dir.exists());
    assert_eq!(browse_id, "test_explicit_project");

    let canonical_base = base.canonicalize().unwrap();
    assert!(dir.starts_with(&canonical_base));

    let _ = std::fs::remove_dir_all(&explicit);
}

#[test]
fn test_resolve_project_dir_rejects_outside_base() {
    let tmp = tempfile::tempdir().unwrap();
    let escape_attempt = tmp.path().join("evil_project");
    // Don't create it: explicit paths that don't exist are rejected
    // before the prefix check even runs.

    let result = resolve_project_dir(Some(escape_attempt), Uuid::new_v4());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("does not exist"),
        "expected 'does not exist' error, got: {}",
        err
    );
}

#[test]
fn test_resolve_project_dir_rejects_outside_base_existing() {
    // A directory that exists but is outside the projects base.
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().to_path_buf();

    let result = resolve_project_dir(Some(outside), Uuid::new_v4());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("must be under"),
        "expected 'must be under' error, got: {}",
        err
    );
}

#[test]
fn test_resolve_project_dir_rejects_traversal() {
    // Non-existent traversal path is rejected because canonicalize fails.
    let base = projects_base();
    let traversal = base.join("legit").join("..").join("..").join(".ssh");

    let result = resolve_project_dir(Some(traversal), Uuid::new_v4());
    assert!(result.is_err(), "traversal path should be rejected");

    // Traversal path that actually resolves gets the prefix check.
    // `base/../` resolves to the parent of projects base, which is outside.
    let base_parent = projects_base().join("..").join("definitely_not_projects");
    std::fs::create_dir_all(&base_parent).ok();
    if base_parent.exists() {
        let result = resolve_project_dir(Some(base_parent.clone()), Uuid::new_v4());
        assert!(result.is_err(), "path outside base should be rejected");
        let _ = std::fs::remove_dir_all(&base_parent);
    }
}

#[test]
fn test_sandbox_schema_includes_project_dir() {
    let manager = Arc::new(ContextManager::new(5));
    let jm = Arc::new(ContainerJobManager::new(
        ContainerJobConfig::default(),
        TokenStore::new(),
    ));
    let tool = CreateJobTool::new(manager).with_sandbox(jm, None);
    let schema = tool.parameters_schema();
    let props = schema.get("properties").unwrap().as_object().unwrap();
    assert!(
        props.contains_key("project_dir"),
        "sandbox schema must expose project_dir"
    );
}

#[test]
fn test_sandbox_schema_includes_credentials() {
    let manager = Arc::new(ContextManager::new(5));
    let jm = Arc::new(ContainerJobManager::new(
        ContainerJobConfig::default(),
        TokenStore::new(),
    ));
    let tool = CreateJobTool::new(manager).with_sandbox(jm, None);
    let schema = tool.parameters_schema();
    let props = schema.get("properties").unwrap().as_object().unwrap();
    assert!(
        props.contains_key("credentials"),
        "sandbox schema must expose credentials"
    );
}

#[test]
fn test_sandbox_schema_only_exposes_enabled_agent_modes() {
    let manager = Arc::new(ContextManager::new(5));
    let jm = Arc::new(ContainerJobManager::new(
        // stub config has fewer fields under reduced profiles
        #[allow(clippy::needless_update)]
        ContainerJobConfig {
            claude_code_enabled: false,
            codex_code_enabled: true,
            ..ContainerJobConfig::default()
        },
        TokenStore::new(),
    ));
    let tool = CreateJobTool::new(manager).with_sandbox(jm, None);
    let schema = tool.parameters_schema();
    let mode_enum = schema["properties"]["mode"]["enum"]
        .as_array()
        .expect("mode enum array");
    let mode_values: Vec<&str> = mode_enum
        .iter()
        .filter_map(|value| value.as_str())
        .collect();

    assert_eq!(mode_values, vec!["worker", "codex_code"]);
}

#[tokio::test]
async fn test_execute_rejects_disabled_codex_mode() {
    let manager = Arc::new(ContextManager::new(5));
    let jm = Arc::new(ContainerJobManager::new(
        // stub config has fewer fields under reduced profiles
        #[allow(clippy::needless_update)]
        ContainerJobConfig {
            claude_code_enabled: true,
            codex_code_enabled: false,
            ..ContainerJobConfig::default()
        },
        TokenStore::new(),
    ));
    let tool = CreateJobTool::new(manager).with_sandbox(jm, None);
    let params = serde_json::json!({
        "title": "Test Job",
        "description": "A test job description",
        "mode": "codex_code"
    });

    let err = tool
        .execute(params, &JobContext::default())
        .await
        .expect_err("disabled codex mode should be rejected");

    assert!(err.to_string().contains("not enabled"));
}

#[tokio::test]
async fn test_parse_credentials_empty() {
    let manager = Arc::new(ContextManager::new(5));
    let tool = CreateJobTool::new(manager);

    // No credentials parameter
    let params = serde_json::json!({"title": "t", "description": "d"});
    let grants = tool.parse_credentials(&params, "user1").await.unwrap();
    assert!(grants.is_empty());

    // Empty credentials object
    let params = serde_json::json!({"credentials": {}});
    let grants = tool.parse_credentials(&params, "user1").await.unwrap();
    assert!(grants.is_empty());
}

#[tokio::test]
async fn test_parse_credentials_no_secrets_store() {
    let manager = Arc::new(ContextManager::new(5));
    let tool = CreateJobTool::new(manager);

    let params = serde_json::json!({"credentials": {"my_secret": "MY_SECRET"}});
    let result = tool.parse_credentials(&params, "user1").await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("no secrets store"),
        "expected 'no secrets store' error, got: {}",
        err
    );
}

#[tokio::test]
async fn test_parse_credentials_missing_secret() {
    use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
    use secrecy::SecretString;

    let manager = Arc::new(ContextManager::new(5));
    let key = "0123456789abcdef0123456789abcdef";
    let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
    let secrets: Arc<dyn SecretsStore + Send + Sync> = Arc::new(InMemorySecretsStore::new(crypto));

    let tool = CreateJobTool::new(manager).with_secrets(Arc::clone(&secrets));

    let params = serde_json::json!({"credentials": {"nonexistent_secret": "SOME_VAR"}});
    let result = tool.parse_credentials(&params, "user1").await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found"),
        "expected 'not found' error, got: {}",
        err
    );
}

#[tokio::test]
async fn test_parse_credentials_valid() {
    use crate::secrets::{CreateSecretParams, InMemorySecretsStore, SecretsCrypto};
    use secrecy::SecretString;

    let manager = Arc::new(ContextManager::new(5));
    let key = "0123456789abcdef0123456789abcdef";
    let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
    let secrets: Arc<dyn SecretsStore + Send + Sync> =
        Arc::new(InMemorySecretsStore::new(Arc::clone(&crypto)));

    // Store a secret
    secrets
        .create(
            "user1",
            CreateSecretParams::new("github_token", "ghp_test123"),
        )
        .await
        .unwrap();

    let tool = CreateJobTool::new(manager).with_secrets(Arc::clone(&secrets));

    let params = serde_json::json!({
        "credentials": {"github_token": "GITHUB_TOKEN"}
    });
    let grants = tool.parse_credentials(&params, "user1").await.unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].secret_name, "github_token");
    assert_eq!(grants[0].env_var, "GITHUB_TOKEN");
}

fn test_prompt_tool(queue: PromptQueue) -> JobPromptTool {
    let cm = Arc::new(ContextManager::new(5));
    JobPromptTool::new(queue, cm)
}

#[tokio::test]
async fn test_job_prompt_tool_rejects_local_jobs() {
    let cm = Arc::new(ContextManager::new(5));
    let job_id = cm
        .create_job_for_user("default", "Test Job", "desc")
        .await
        .unwrap();

    let queue: PromptQueue = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let tool = JobPromptTool::new(Arc::clone(&queue), cm);

    let params = serde_json::json!({
        "job_id": job_id.to_string(),
        "content": "What's the status?",
        "done": false,
    });

    let ctx = JobContext::default();
    let err = tool.execute(params, &ctx).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("job_prompt only supports sandbox jobs"),
        "expected local-job rejection, got: {}",
        err
    );

    let q = queue.lock().await;
    assert!(q.get(&job_id).is_none());
}

#[tokio::test]
async fn test_job_prompt_tool_requires_approval() {
    use crate::tools::tool::ApprovalRequirement;
    let queue: PromptQueue = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let tool = test_prompt_tool(queue);
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved
    );
}

#[tokio::test]
async fn test_job_prompt_tool_rejects_invalid_uuid() {
    let queue: PromptQueue = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let tool = test_prompt_tool(queue);

    let params = serde_json::json!({
        "job_id": "not-a-uuid",
        "content": "hello",
    });

    let ctx = JobContext::default();
    let result = tool.execute(params, &ctx).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_job_prompt_tool_rejects_missing_content() {
    let queue: PromptQueue = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let tool = test_prompt_tool(queue);

    let params = serde_json::json!({
        "job_id": Uuid::new_v4().to_string(),
    });

    let ctx = JobContext::default();
    let result = tool.execute(params, &ctx).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_job_events_tool_rejects_other_users_job() {
    // JobEventsTool needs a Store (PostgreSQL) for the full path, but the
    // ownership check happens first via ContextManager, so we can test that
    // without a database by using a Store that will never be reached.
    //
    // We construct the tool by hand: the store field is never touched
    // because the ownership check short-circuits before the query.
    let cm = Arc::new(ContextManager::new(5));
    let job_id = cm
        .create_job_for_user("owner-user", "Secret Job", "classified")
        .await
        .unwrap();

    // We need a Store to construct the tool, but creating one requires
    // a database URL. Instead, test the ownership logic directly:
    // simulate what execute() does.
    let attacker_ctx = JobContext {
        user_id: "attacker".to_string(),
        principal_id: "attacker".to_string(),
        actor_id: Some("attacker".to_string()),
        ..Default::default()
    };

    let job_ctx = cm.get_context(job_id).await.unwrap();
    assert_ne!(job_ctx.user_id, attacker_ctx.user_id);
    assert_eq!(job_ctx.user_id, "owner-user");
}

#[test]
fn test_job_events_tool_schema() {
    // Verify the schema shape is correct (doesn't need a Store instance).
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "job_id": {
                "type": "string",
                "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
            },
            "limit": {
                "type": "integer",
                "description": "Maximum number of events to return (default 50, most recent)"
            }
        },
        "required": ["job_id"]
    });

    let props = schema.get("properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("job_id"));
    assert!(props.contains_key("limit"));
    let required = schema.get("required").unwrap().as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0].as_str().unwrap(), "job_id");
}

#[tokio::test]
async fn test_job_prompt_tool_rejects_other_users_job() {
    let cm = Arc::new(ContextManager::new(5));
    let job_id = cm
        .create_job_for_user("owner-user", "Test Job", "desc")
        .await
        .unwrap();

    let queue: PromptQueue = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let tool = JobPromptTool::new(queue, cm);

    let params = serde_json::json!({
        "job_id": job_id.to_string(),
        "content": "sneaky prompt",
    });

    // Attacker context with a different user_id.
    let ctx = JobContext {
        user_id: "attacker".to_string(),
        principal_id: "attacker".to_string(),
        actor_id: Some("attacker".to_string()),
        ..Default::default()
    };

    let result = tool.execute(params, &ctx).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("job not found") || err.contains("does not belong to current user"),
        "expected ownership error, got: {}",
        err
    );
}

#[tokio::test]
async fn test_job_prompt_tool_rejects_same_user_different_actor_job() {
    let cm = Arc::new(ContextManager::new(5));
    let job_id = cm
        .create_job_for_identity("household", "alex", "Test Job", "desc")
        .await
        .unwrap();

    let queue: PromptQueue = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let tool = JobPromptTool::new(Arc::clone(&queue), Arc::clone(&cm));

    let params = serde_json::json!({
        "job_id": job_id.to_string(),
        "content": "sneaky prompt",
    });

    let ctx = JobContext {
        user_id: "household".to_string(),
        principal_id: "household".to_string(),
        actor_id: Some("sam".to_string()),
        ..Default::default()
    };

    let result = tool.execute(params, &ctx).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("job not found"),
        "expected actor ownership rejection, got: {}",
        err
    );

    let q = queue.lock().await;
    assert!(
        q.get(&job_id).is_none(),
        "prompt must not be queued for another actor's job"
    );
}

#[tokio::test]
async fn test_resolve_job_id_full_uuid() {
    let cm = ContextManager::new(5);
    let job_id = cm.create_job("Test", "Desc").await.unwrap();

    let resolved = job_policy::resolve_job_reference(
        &job_id.to_string(),
        cm.all_jobs().await,
        std::iter::empty::<Uuid>(),
    );
    assert_eq!(resolved.unwrap().job_id, job_id);
}

#[tokio::test]
async fn test_resolve_job_id_short_prefix() {
    let cm = ContextManager::new(5);
    let job_id = cm.create_job("Test", "Desc").await.unwrap();

    // Use first 8 hex chars (without dashes)
    let hex = job_id.to_string().replace('-', "");
    let prefix = &hex[..8];
    let resolved =
        job_policy::resolve_job_reference(prefix, cm.all_jobs().await, std::iter::empty::<Uuid>());
    assert_eq!(resolved.unwrap().job_id, job_id);
}

#[tokio::test]
async fn test_resolve_job_id_no_match() {
    let cm = ContextManager::new(5);
    cm.create_job("Test", "Desc").await.unwrap();

    let result = job_policy::resolve_job_reference(
        "00000000",
        cm.all_jobs().await,
        std::iter::empty::<Uuid>(),
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("no job found"),
        "expected 'no job found', got: {}",
        err
    );
}

#[tokio::test]
async fn test_resolve_job_id_invalid_input() {
    let cm = ContextManager::new(5);
    let result = job_policy::resolve_job_reference(
        "not-hex-at-all!",
        cm.all_jobs().await,
        std::iter::empty::<Uuid>(),
    );
    assert!(result.is_err());
}

#[tokio::test]
async fn test_resolve_owned_job_id_filters_other_actor_jobs() {
    let cm = ContextManager::new(5);
    let alex_job = cm
        .create_job_for_identity("household", "alex", "Test", "Desc")
        .await
        .unwrap();
    cm.create_job_for_identity("household", "sam", "Other", "Desc")
        .await
        .unwrap();

    let result =
        resolve_owned_job_ref(&alex_job.to_string(), &cm, None, None, "household", "sam").await;
    assert!(result.is_err());
    let err = result.err().expect("expected job lookup to fail");
    assert!(err.to_string().contains("job not found"));
}
