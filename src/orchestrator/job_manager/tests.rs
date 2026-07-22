use super::*;

#[test]
fn test_container_job_config_default() {
    let config = ContainerJobConfig::default();
    assert_eq!(config.orchestrator_port, 50051);
    assert_eq!(config.memory_limit_mb, 2048);
}

#[test]
fn sandbox_job_spec_validation_bounds_persisted_and_worker_input() {
    let manager = ContainerJobManager::new(ContainerJobConfig::default(), TokenStore::new());
    let job_id = Uuid::new_v4();
    let mut spec = SandboxJobSpec::new(
        "bounded job",
        "do the work",
        "principal",
        "actor",
        None,
        JobMode::Worker,
    );
    assert!(manager.validate_job_spec(job_id, &spec).is_ok());

    spec.idle_timeout_secs = u64::MAX;
    assert!(manager.validate_job_spec(job_id, &spec).is_err());
    spec.idle_timeout_secs = crate::sandbox_jobs::DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS;
    spec.description = "x".repeat(crate::sandbox_jobs::MAX_JOB_DESCRIPTION_BYTES + 1);
    assert!(manager.validate_job_spec(job_id, &spec).is_err());
    spec.description = "valid".to_string();
    spec.metadata = serde_json::json!({
        "blob": "x".repeat(crate::sandbox_jobs::MAX_JOB_METADATA_BYTES)
    });
    assert!(manager.validate_job_spec(job_id, &spec).is_err());
}

#[test]
fn test_container_state_display() {
    assert_eq!(ContainerState::Running.to_string(), "running");
    assert_eq!(ContainerState::Stopped.to_string(), "stopped");
}

#[test]
fn credential_grants_reject_runtime_redirection_and_duplicates() {
    let job_id = Uuid::new_v4();
    let reserved = validate_credential_grants(
        job_id,
        &[CredentialGrant {
            secret_name: "token".to_string(),
            env_var: "HTTP_PROXY".to_string(),
        }],
    );
    assert!(reserved.is_err());

    let duplicate = validate_credential_grants(
        job_id,
        &[
            CredentialGrant {
                secret_name: "one".to_string(),
                env_var: "GITHUB_TOKEN".to_string(),
            },
            CredentialGrant {
                secret_name: "two".to_string(),
                env_var: "GITHUB_TOKEN".to_string(),
            },
        ],
    );
    assert!(duplicate.is_err());
}

#[test]
fn test_validate_bind_mount_valid_path() {
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("projects");
    std::fs::create_dir_all(&base).unwrap();

    let test_dir = base.join("test_validate_bind");
    std::fs::create_dir_all(&test_dir).unwrap();

    let result = validate_bind_mount_path(&test_dir, &base, Uuid::new_v4());
    assert!(result.is_ok());
    let canonical = result.unwrap();
    assert!(canonical.starts_with(base.canonicalize().unwrap()));
}

#[test]
fn test_validate_bind_mount_rejects_outside_base() {
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("projects");
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().to_path_buf();

    let result = validate_bind_mount_path(&outside, &base, Uuid::new_v4());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("outside allowed base"),
        "expected 'outside allowed base', got: {}",
        err
    );
}

#[test]
fn test_validate_bind_mount_rejects_nonexistent() {
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("projects");
    let nonexistent = PathBuf::from("/no/such/path/at/all");
    let result = validate_bind_mount_path(&nonexistent, &base, Uuid::new_v4());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("canonicalize"),
        "expected canonicalize error, got: {}",
        err
    );
}

#[tokio::test]
async fn test_update_worker_status() {
    let store = TokenStore::new();
    let mgr = ContainerJobManager::new(ContainerJobConfig::default(), store);
    let job_id = Uuid::new_v4();

    // Insert a handle
    {
        let mut containers = mgr.containers.write().await;
        containers.insert(
            job_id,
            ContainerHandle {
                job_id,
                container_id: "test".to_string(),
                state: ContainerState::Running,
                mode: JobMode::Worker,
                created_at: chrono::Utc::now(),
                spec: SandboxJobSpec::new(
                    "test job",
                    "test job",
                    "default",
                    "default",
                    None,
                    JobMode::Worker,
                ),
                last_worker_status: None,
                worker_iteration: 0,
                completion_result: None,
            },
        );
    }

    mgr.update_worker_status(job_id, Some("Iteration 3".to_string()), 3)
        .await;

    let handle = mgr.get_handle(job_id).await.unwrap();
    assert_eq!(handle.worker_iteration, 3);
    assert_eq!(handle.last_worker_status.as_deref(), Some("Iteration 3"));
}

#[tokio::test]
async fn finalization_survives_calling_request_cancellation_and_is_drained() {
    let manager = Arc::new(ContainerJobManager::new(
        ContainerJobConfig::default(),
        TokenStore::new(),
    ));
    let completed = Arc::new(AtomicBool::new(false));
    let (started_tx, started_rx) = oneshot::channel();
    let caller_manager = Arc::clone(&manager);
    let caller_completed = Arc::clone(&completed);
    let caller = tokio::spawn(async move {
        caller_manager
            .run_owned_finalization(async move {
                let _ = started_tx.send(());
                tokio::time::sleep(Duration::from_millis(25)).await;
                caller_completed.store(true, Ordering::Release);
                Ok(())
            })
            .await
    });

    started_rx.await.expect("finalization should start");
    caller.abort();
    let _ = caller.await;
    manager.shutdown_all().await;

    assert!(completed.load(Ordering::Acquire));
}

#[tokio::test]
async fn complete_job_revokes_worker_token_before_cleanup() {
    let token_store = TokenStore::new();
    let mgr = ContainerJobManager::new(ContainerJobConfig::default(), token_store.clone());
    let job_id = Uuid::new_v4();
    let token = token_store.create_token(job_id).await;
    mgr.containers.write().await.insert(
        job_id,
        ContainerHandle {
            job_id,
            container_id: String::new(),
            state: ContainerState::Running,
            mode: JobMode::Worker,
            created_at: Utc::now(),
            spec: SandboxJobSpec::new(
                "test job",
                "test job",
                "default",
                "default",
                None,
                JobMode::Worker,
            ),
            last_worker_status: None,
            worker_iteration: 0,
            completion_result: None,
        },
    );

    mgr.complete_job(
        job_id,
        CompletionResult {
            status: "completed".to_string(),
            session_id: None,
            success: true,
            message: None,
            iterations: 0,
        },
    )
    .await
    .expect("completion should succeed without a container id");

    assert!(!token_store.validate(job_id, &token).await);
}

#[tokio::test]
async fn shutdown_closes_container_job_admission() {
    let mgr = ContainerJobManager::new(ContainerJobConfig::default(), TokenStore::new());
    mgr.shutdown_all().await;

    let job_id = Uuid::new_v4();
    let error = mgr
        .create_job(
            job_id,
            SandboxJobSpec::new(
                "late job",
                "late job",
                "default",
                "default",
                None,
                JobMode::Worker,
            ),
            Vec::new(),
        )
        .await
        .expect_err("shutdown must reject late container jobs");
    assert!(matches!(
        error,
        OrchestratorError::InvalidContainerState { .. }
    ));
}

#[tokio::test]
async fn duplicate_job_id_cannot_rotate_token_or_overwrite_handle() {
    let token_store = TokenStore::new();
    let mgr = ContainerJobManager::new(ContainerJobConfig::default(), token_store.clone());
    let job_id = Uuid::new_v4();
    let original_token = token_store.create_token(job_id).await;
    mgr.containers.write().await.insert(
        job_id,
        ContainerHandle {
            job_id,
            container_id: "existing-container".to_string(),
            state: ContainerState::Running,
            mode: JobMode::Worker,
            created_at: Utc::now(),
            spec: SandboxJobSpec::new(
                "existing job",
                "existing job",
                "default",
                "default",
                None,
                JobMode::Worker,
            ),
            last_worker_status: None,
            worker_iteration: 0,
            completion_result: None,
        },
    );

    let error = mgr
        .create_job(
            job_id,
            SandboxJobSpec::new(
                "duplicate job",
                "duplicate job",
                "default",
                "default",
                None,
                JobMode::Worker,
            ),
            Vec::new(),
        )
        .await
        .expect_err("a duplicate job id must fail before touching Docker");

    assert!(matches!(
        error,
        OrchestratorError::InvalidContainerState { .. }
    ));
    assert!(token_store.validate(job_id, &original_token).await);
    assert_eq!(
        mgr.get_handle(job_id).await.unwrap().container_id,
        "existing-container"
    );
}

#[test]
fn test_extend_mode_runtime_adds_codex_env_and_mount() {
    let codex_home = tempfile::tempdir().unwrap();
    let config = ContainerJobConfig {
        codex_code_enabled: true,
        codex_code_api_key: Some("sk-test".to_string()),
        codex_code_home_dir: codex_home.path().to_path_buf(),
        ..ContainerJobConfig::default()
    };
    let mgr = ContainerJobManager::new(config, TokenStore::new());
    let mut env_vec = Vec::new();
    let mut mounts = Vec::new();

    mgr.extend_mode_runtime(
        Uuid::new_v4(),
        JobMode::CodexCode,
        &mut env_vec,
        &mut mounts,
    )
    .unwrap();

    assert!(
        env_vec
            .iter()
            .any(|entry| entry == "OPENAI_API_KEY=sk-test")
    );
    assert!(
        env_vec
            .iter()
            .any(|entry| entry == "CODEX_HOME=/home/sandbox/.codex")
    );
    assert_eq!(mounts.len(), 1);
    assert_eq!(
        mounts[0].target.as_deref(),
        Some("/home/sandbox/.codex-host")
    );
    assert_eq!(mounts[0].source.as_deref(), codex_home.path().to_str());
    assert_eq!(mounts[0].read_only, Some(true));
}

#[tokio::test]
async fn test_codex_container_command_uses_cached_model_and_resets_to_default() {
    let job_id = Uuid::new_v4();
    let config = ContainerJobConfig {
        codex_code_enabled: true,
        codex_code_model: "gpt-5.3-codex".to_string(),
        ..ContainerJobConfig::default()
    };
    let mgr = ContainerJobManager::new(config, TokenStore::new());

    mgr.update_codex_code_settings(Some("gpt-5.4".to_string()))
        .await
        .unwrap();
    let updated = mgr
        .container_cmd(
            job_id,
            "http://orchestrator".to_string(),
            JobMode::CodexCode,
        )
        .await;
    assert_eq!(
        updated,
        vec![
            "codex-bridge".to_string(),
            "--job-id".to_string(),
            job_id.to_string(),
            "--orchestrator-url".to_string(),
            "http://orchestrator".to_string(),
            "--model".to_string(),
            "gpt-5.4".to_string(),
        ]
    );

    mgr.update_codex_code_settings(None).await.unwrap();
    let reset = mgr
        .container_cmd(
            job_id,
            "http://orchestrator".to_string(),
            JobMode::CodexCode,
        )
        .await;
    assert_eq!(reset.last().map(String::as_str), Some("gpt-5.3-codex"));
}

#[tokio::test]
async fn runtime_code_settings_reject_invalid_values_and_support_resets() {
    let config = ContainerJobConfig {
        claude_code_model: "claude-default".to_string(),
        claude_code_max_turns: 50,
        codex_code_model: "codex-default".to_string(),
        ..ContainerJobConfig::default()
    };
    let mgr = ContainerJobManager::new(config, TokenStore::new());

    assert!(
        mgr.update_claude_code_settings(Some(Some("\n".to_string())), None)
            .await
            .is_err()
    );
    assert!(
        mgr.update_claude_code_settings(None, Some(Some(0)))
            .await
            .is_err()
    );
    assert!(
        mgr.update_codex_code_settings(Some(String::new()))
            .await
            .is_err()
    );

    mgr.update_claude_code_settings(Some(Some("claude-updated".to_string())), Some(Some(12)))
        .await
        .unwrap();
    mgr.update_claude_code_settings(Some(None), Some(None))
        .await
        .unwrap();
    let command = mgr
        .container_cmd(
            Uuid::new_v4(),
            "http://orchestrator".to_string(),
            JobMode::ClaudeCode,
        )
        .await;
    assert_eq!(
        command
            .windows(2)
            .find(|pair| pair[0] == "--model")
            .map(|pair| pair[1].as_str()),
        Some("claude-default")
    );
    assert_eq!(
        command
            .windows(2)
            .find(|pair| pair[0] == "--max-turns")
            .map(|pair| pair[1].as_str()),
        Some("50")
    );
}
