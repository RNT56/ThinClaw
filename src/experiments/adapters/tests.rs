use super::*;
use chrono::Utc;
use uuid::Uuid;

fn runner_profile(backend: ExperimentRunnerBackend) -> ExperimentRunnerProfile {
    ExperimentRunnerProfile {
        id: Uuid::new_v4(),
        owner_user_id: "default".to_string(),
        name: "runner".to_string(),
        backend,
        backend_config: serde_json::json!({}),
        image_or_runtime: None,
        gpu_requirements: serde_json::json!({}),
        env_grants: serde_json::json!({}),
        secret_references: Vec::new(),
        cache_policy: serde_json::json!({}),
        status: crate::experiments::ExperimentRunnerStatus::Draft,
        readiness_class: ExperimentRunnerReadinessClass::ManualOnly,
        launch_eligible: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn generic_remote_runner_validates_as_manual_only() {
    let settings = Settings::default();
    let runner = runner_profile(ExperimentRunnerBackend::GenericRemoteRunner);
    let outcome = validate_runner_profile(&runner, &settings, None).await;
    assert!(outcome.valid);
    assert_eq!(
        outcome.readiness_class,
        ExperimentRunnerReadinessClass::ManualOnly
    );
    assert!(!outcome.launch_eligible);
}

#[tokio::test]
async fn lambda_runner_without_launch_payload_stays_bootstrap_ready() {
    let settings = Settings::default();
    let mut runner = runner_profile(ExperimentRunnerBackend::Lambda);
    runner.image_or_runtime = Some("ghcr.io/thinclaw/research-runner:latest".to_string());
    runner.secret_references = vec!["research_lambda_api_key".to_string()];
    runner.backend_config = serde_json::json!({
        "template_id": "lambda-template"
    });

    let outcome = validate_runner_profile(&runner, &settings, None).await;
    assert!(outcome.valid);
    assert_eq!(
        outcome.readiness_class,
        ExperimentRunnerReadinessClass::BootstrapReady
    );
    assert!(!outcome.launch_eligible);
}

#[tokio::test]
async fn agent_env_runner_validates_camel_case_webui_template_config() {
    let settings = Settings::default();
    let mut runner = runner_profile(ExperimentRunnerBackend::AgentEnv);
    runner.backend_config = serde_json::json!({
        "benchmark": "terminal_bench",
        "cases": [{
            "name": "smoke",
            "command": "printf ok",
            "expectedStdoutContains": ["ok"],
            "expectedExitCode": 0,
            "timeoutSecs": 5
        }]
    });

    let outcome = validate_runner_profile(&runner, &settings, None).await;
    assert!(outcome.valid, "{}", outcome.message);
    assert!(outcome.launch_eligible);
}

#[tokio::test]
async fn agent_env_runner_rejects_malformed_benchmark_config() {
    let settings = Settings::default();
    let mut runner = runner_profile(ExperimentRunnerBackend::AgentEnv);
    runner.backend_config = serde_json::json!({
        "benchmark": "skill_bench",
        "cases": [{
            "name": "missing-content"
        }]
    });

    let outcome = validate_runner_profile(&runner, &settings, None).await;
    assert!(!outcome.valid);
    assert!(
        outcome
            .message
            .contains("Invalid AgentEnv benchmark config")
    );
}

fn lease_auth() -> ExperimentLeaseAuthentication {
    ExperimentLeaseAuthentication {
        lease_id: Uuid::new_v4(),
        token: "exp_0123456789ab_0123456789abcdef0123456789abcdef".to_string(),
    }
}

#[test]
fn gateway_urls_fail_closed_before_building_bootstrap_commands() {
    let auth = lease_auth();
    assert!(build_bootstrap_command("https://gateway.example/base", &auth).is_ok());
    assert!(build_bootstrap_command("http://127.0.0.1:3001", &auth).is_ok());
    for invalid in [
        "",
        "http://gateway.example",
        "https://user:pass@gateway.example",
        "https://gateway.example/?token=secret",
        "https://gateway.example/#fragment",
        "file:///tmp/gateway",
    ] {
        assert!(
            build_bootstrap_command(invalid, &auth).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn bootstrap_command_shell_quotes_every_variable_argument() {
    let auth = ExperimentLeaseAuthentication {
        lease_id: Uuid::new_v4(),
        token: "safe'quoted".to_string(),
    };
    let command = build_bootstrap_command("https://gateway.example/base", &auth).unwrap();
    assert!(command.contains("--gateway-url 'https://gateway.example/base'"));
    assert!(command.contains("--token 'safe'\"'\"'quoted'"));
}

#[test]
fn durable_provider_metadata_excludes_bootstrap_env_and_provider_echoes() {
    let token = "exp_0123456789ab_0123456789abcdef0123456789abcdef";
    let raw = serde_json::json!({
        "provider": "runpod",
        "pod_id": "pod-123",
        "launch_request": {
            "gpuTypeIds": ["NVIDIA H100"],
            "dataCenterIds": ["EU-1"],
            "dockerStartCmd": [format!("thinclaw --token {token}")],
            "env": { "API_KEY": "secret" }
        },
        "pod": {
            "id": "pod-123",
            "status": "RUNNING",
            "costPerHr": 1.25,
            "echo": token
        },
        "response": { "request": format!("--token {token}") }
    });
    let sanitized = sanitize_provider_job_metadata(ExperimentRunnerBackend::Runpod, &raw);
    let encoded = sanitized.to_string();
    assert!(!encoded.contains(token));
    assert!(!encoded.contains("dockerStartCmd"));
    assert!(!encoded.contains("API_KEY"));
    assert!(!encoded.contains("response"));
    assert_eq!(sanitized["pod_id"], "pod-123");
    assert_eq!(sanitized["pod"]["costPerHr"], 1.25);
    assert_eq!(sanitized["launch_request"]["gpuTypeIds"][0], "NVIDIA H100");
}

#[test]
fn provider_ids_and_slurm_output_are_strictly_parsed() {
    assert_eq!(
        validate_provider_id("RunPod", "pod_123-abc").unwrap(),
        "pod_123-abc"
    );
    assert!(validate_provider_id("RunPod", "../../pods/other").is_err());
    assert!(validate_provider_id("RunPod", "pod\nother").is_err());
    assert_eq!(
        parse_slurm_job_id("Submitted batch job 1842\n").as_deref(),
        Some("1842")
    );
    assert_eq!(parse_slurm_job_id("submission accepted"), None);
}

#[test]
fn kubernetes_manifest_quotes_configured_yaml_scalars() {
    let manifest = kubernetes_job_manifest(
        "job\nkind: Secret",
        "namespace\nmetadata: injected",
        "image:latest\nprivileged: true",
        "thinclaw experiment-runner",
        BTreeMap::from([("KEY\nvalue: injected".to_string(), "value".to_string())]),
        &serde_json::json!({ "gpu_count": 1 }),
    );
    assert!(!manifest.contains("\nkind: Secret\n"));
    assert!(!manifest.contains("\nprivileged: true\n"));
    assert!(manifest.contains("job\\nkind: Secret"));
    assert!(manifest.contains("image:latest\\nprivileged: true"));
}

#[test]
fn launch_outcome_debug_never_renders_bootstrap_material() {
    let token = lease_auth().token;
    let outcome = RunnerLaunchOutcome {
        message: "manual".to_string(),
        bootstrap_command: Some(format!("run --token {token}")),
        provider_template: Some(serde_json::json!({ "token": token })),
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({ "echo": token }),
        auto_launched: false,
        requires_operator_action: true,
    };
    let debug = format!("{outcome:?}");
    assert!(!debug.contains(&token));
    assert!(debug.contains("[REDACTED]"));
}
