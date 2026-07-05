use super::*;
use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
use secrecy::SecretString;

fn test_secrets() -> SharedSecrets {
    let key = "0123456789abcdef0123456789abcdef";
    let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
    Arc::new(InMemorySecretsStore::new(crypto))
}

#[tokio::test]
async fn setup_store_credential_and_enroll_roundtrip() {
    let (db, _guard) = crate::testing::test_db().await;
    let secrets = test_secrets();

    // Disabled before setup.
    let readiness = repo_projects_readiness(&db, Some(&secrets), "default")
        .await
        .unwrap();
    assert!(!readiness.enabled);
    assert!(!readiness.ready_for_live_runs);
    assert_eq!(readiness.credential_mode, "none");

    // Store a github_token credential securely.
    store_repo_credential(
        &secrets,
        "default",
        "github_token".to_string(),
        "ghp_test_value".to_string(),
    )
    .await
    .unwrap();
    store_repo_credential(
        &secrets,
        "default",
        "github_fork_token".to_string(),
        "ghp_fork_test_value".to_string(),
    )
    .await
    .unwrap();

    // Enable the supervisor + policy.
    let input = RepoProjectsConfigureInput {
        enabled: Some(true),
        auto_merge_default: Some(true),
        default_coding_backend: Some("codex_code".to_string()),
        ..Default::default()
    };
    let readiness = configure_supervisor(&db, Some(&secrets), "default", input)
        .await
        .unwrap();
    assert!(readiness.enabled);
    assert_eq!(readiness.github_token_secret_present, Some(true));
    assert_eq!(readiness.github_fork_token_secret_present, Some(true));
    assert_eq!(readiness.credential_mode, "github_token");
    assert!(readiness.ready_for_live_runs);
    assert!(readiness.auto_merge_default);
    assert_eq!(readiness.default_coding_backend, "codex_code");
    assert_eq!(readiness.default_write_mode, "fork_pr");

    // Create a project (now that the feature is enabled) + enroll a 2nd repo.
    create_project(
        &db,
        "default",
        RepoProjectCreateInput {
            name: "Proj".to_string(),
            repo_url: "acme/widgets".to_string(),
            default_branch: None,
            local_path: None,
            description: None,
            write_mode: None,
            fork_owner: None,
            fork_repo: None,
        },
    )
    .await
    .unwrap();
    let project = db.list_repo_projects().await.unwrap().pop().unwrap();
    enroll_repo(
        &db,
        "default",
        project.id,
        RepoEnrollInput {
            repo_url: "acme/gadgets".to_string(),
            default_branch: Some("develop".to_string()),
            fork_owner: None,
            fork_repo: None,
        },
    )
    .await
    .unwrap();
    let repos = db.list_repo_project_repos(project.id).await.unwrap();
    assert_eq!(repos.len(), 2);
    assert!(repos.iter().any(|repo| repo.repo == "gadgets"));
}

#[test]
fn install_url_is_built_from_app_slug() {
    assert_eq!(
        github_app_install_url(Some("thinclaw-supervisor")),
        Some("https://github.com/apps/thinclaw-supervisor/installations/new".to_string())
    );
    assert_eq!(github_app_install_url(Some("   ")), None);
    assert_eq!(github_app_install_url(None), None);
}

#[tokio::test]
async fn connector_lists_installation_repos_and_connects_selected() {
    use crate::repo_projects::github_provider::FixedTokenGitHubClientProvider;
    use axum::http::{Method, StatusCode, Uri};
    use axum::response::{IntoResponse, Response};
    use axum::{Json, Router};

    async fn fake(method: Method, uri: Uri) -> Response {
        if method == Method::GET && uri.path() == "/installation/repositories" {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "total_count": 3,
                    "repositories": [
                        { "id": 1, "name": "widgets", "full_name": "acme/widgets",
                          "private": true, "default_branch": "main",
                          "owner": { "login": "acme", "id": 10 } },
                        { "id": 2, "name": "gadgets", "full_name": "acme/gadgets",
                          "private": false, "default_branch": "develop",
                          "owner": { "login": "acme", "id": 10 } },
                        { "id": 3, "name": "legacy", "full_name": "octo/legacy",
                          "private": false, "archived": true, "default_branch": "main",
                          "owner": { "login": "octo", "id": 20 } }
                    ]
                })),
            )
                .into_response();
        }
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "message": "not found" })),
        )
            .into_response()
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let app = Router::new().fallback(fake);
        let _ = axum::serve(listener, app).await;
    });
    let base_url = format!("http://{addr}");

    let (db, _guard) = crate::testing::test_db().await;
    let secrets = test_secrets();

    // Enable the feature + enroll acme/widgets via a project.
    configure_supervisor(
        &db,
        Some(&secrets),
        "default",
        RepoProjectsConfigureInput {
            enabled: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    create_project(
        &db,
        "default",
        RepoProjectCreateInput {
            name: "Widgets".to_string(),
            repo_url: "acme/widgets".to_string(),
            default_branch: None,
            local_path: None,
            description: None,
            write_mode: None,
            fork_owner: None,
            fork_repo: None,
        },
    )
    .await
    .unwrap();

    // Discovery: the installation lists all three; widgets is marked enrolled.
    let provider = FixedTokenGitHubClientProvider::new(base_url, "tok");
    let listing = list_connectable_repos_with_provider(&db, &provider)
        .await
        .unwrap();
    assert_eq!(listing.source, "github_app");
    assert_eq!(listing.total, 3);
    let widgets = listing.repos.iter().find(|r| r.repo == "widgets").unwrap();
    assert!(widgets.enrolled);
    assert!(widgets.project_id.is_some());
    let gadgets = listing.repos.iter().find(|r| r.repo == "gadgets").unwrap();
    assert!(!gadgets.enrolled);
    assert_eq!(gadgets.default_branch, "develop");
    assert_eq!(gadgets.recommended_write_mode, "read_only_clone");

    // Select specific repos: gadgets is new, widgets already enrolled → skipped.
    let result = connect_repos(
        &db,
        &secrets,
        "default",
        RepoConnectInput {
            repos: vec!["acme/gadgets".to_string(), "acme/widgets".to_string()],
            all: false,
            write_mode: None,
            fork_owner: None,
            fork_repo: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(result.connected, vec!["acme/gadgets".to_string()]);
    assert_eq!(result.skipped, vec!["acme/widgets".to_string()]);
    let projects = db.list_repo_projects().await.unwrap();
    let gadgets_project = projects
        .iter()
        .find(|project| project.name == "gadgets")
        .expect("gadgets project");
    assert_eq!(gadgets_project.policy.write_mode, RepoWriteMode::ForkPr);
}

#[tokio::test]
async fn maintainer_write_mode_requires_worker_push_token_even_with_github_app() {
    let (db, _guard) = crate::testing::test_db().await;
    let secrets = test_secrets();
    store_repo_credential(
        &secrets,
        "default",
        "repo_projects_github_private_key".to_string(),
        "pem".to_string(),
    )
    .await
    .unwrap();

    let readiness = configure_supervisor(
        &db,
        Some(&secrets),
        "default",
        RepoProjectsConfigureInput {
            enabled: Some(true),
            app_id: Some(123),
            installation_id: Some(456),
            private_key_secret: Some("repo_projects_github_private_key".to_string()),
            default_write_mode: Some("maintainer_branch_pr".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(readiness.credential_mode, "github_app");
    assert!(!readiness.ready_for_live_runs);
    assert_eq!(readiness.github_token_secret_present, Some(false));

    store_repo_credential(
        &secrets,
        "default",
        "github_token".to_string(),
        "ghp_upstream_push".to_string(),
    )
    .await
    .unwrap();
    let readiness = repo_projects_readiness(&db, Some(&secrets), "default")
        .await
        .unwrap();
    assert!(readiness.ready_for_live_runs);
}
