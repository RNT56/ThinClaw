use super::*;

fn signature(secret: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[test]
fn github_api_path_encodes_segments() {
    assert_eq!(
        repo_path("thin claw", "repo/name", &["branches", "feature/a b"]),
        "repos/thin%20claw/repo%2Fname/branches/feature%2Fa%20b"
    );
    assert_eq!(
        git_ref_path("owner", "repo", "feature/supervisor"),
        "repos/owner/repo/git/ref/heads%2Ffeature%2Fsupervisor"
    );
}

#[test]
fn repository_permission_check_honors_github_hierarchy() {
    let permissions = GitHubRepositoryPermissions {
        push: true,
        ..GitHubRepositoryPermissions::default()
    };

    assert!(permissions.allows(GitHubRepoPermission::Pull));
    assert!(permissions.allows(GitHubRepoPermission::Triage));
    assert!(permissions.allows(GitHubRepoPermission::Push));
    assert!(!permissions.allows(GitHubRepoPermission::Maintain));
    assert!(!permissions.allows(GitHubRepoPermission::Admin));
}

#[test]
fn merge_method_serializes_as_github_payload() {
    let body = serde_json::to_value(GitHubMergePullRequestRequest {
        commit_title: Some("merge it".to_string()),
        commit_message: None,
        sha: Some("abc123".to_string()),
        merge_method: Some(GitHubMergeMethod::Squash),
    })
    .unwrap();

    assert_eq!(
        body,
        serde_json::json!({
            "commit_title": "merge it",
            "sha": "abc123",
            "merge_method": "squash"
        })
    );
}

#[test]
fn api_error_preserves_status_message_documentation_and_body() {
    let body = r#"{
        "message": "Validation Failed",
        "documentation_url": "https://docs.github.com/rest",
        "errors": [{"resource": "PullRequest", "code": "missing_field"}]
    }"#;
    let error = github_api_error_from_body(
        StatusCode::UNPROCESSABLE_ENTITY,
        "POST",
        "https://api.github.test/repos/o/r/pulls",
        Some("abc123".to_string()),
        body.to_string(),
    );

    let GitHubApiError::Api {
        status,
        message,
        documentation_url,
        errors,
        request_id,
        body: captured_body,
        ..
    } = error
    else {
        panic!("expected api error");
    };

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(message.as_deref(), Some("Validation Failed"));
    assert_eq!(
        documentation_url.as_deref(),
        Some("https://docs.github.com/rest")
    );
    assert!(errors.is_some());
    assert_eq!(request_id.as_deref(), Some("abc123"));
    assert_eq!(captured_body, body);
}

#[test]
fn commit_comparison_reports_up_to_date_only_when_not_behind() {
    let behind = GitHubCommitComparison {
        status: "behind".to_string(),
        ahead_by: 0,
        behind_by: 3,
        total_commits: 3,
    };
    let ahead = GitHubCommitComparison {
        status: "ahead".to_string(),
        ahead_by: 2,
        behind_by: 0,
        total_commits: 2,
    };

    assert!(!behind.is_up_to_date());
    assert!(ahead.is_up_to_date());
}

#[test]
fn compare_and_delete_ref_paths_are_encoded() {
    assert_eq!(
        repo_path(
            "o",
            "r",
            &["compare", &format!("{}...{}", "main", "thinclaw/p/abc")]
        ),
        "repos/o/r/compare/main...thinclaw%2Fp%2Fabc"
    );
    assert_eq!(
        repo_path(
            "o",
            "r",
            &["git", "refs", &format!("heads/{}", "thinclaw/p/abc")]
        ),
        "repos/o/r/git/refs/heads%2Fthinclaw%2Fp%2Fabc"
    );
}

#[test]
fn github_webhook_signature_accepts_valid_hmac() {
    let body = br#"{"action":"opened"}"#;
    let sig = signature("secret", body);
    verify_github_webhook_signature("secret", body, Some(&sig)).unwrap();
}

#[test]
fn github_webhook_signature_rejects_invalid_hmac() {
    let body = br#"{"action":"opened"}"#;
    let err = verify_github_webhook_signature(
        "secret",
        body,
        Some("sha256=0000000000000000000000000000000000000000000000000000000000000000"),
    )
    .unwrap_err();
    assert_eq!(err, GitHubWebhookError::SignatureMismatch);
}

#[test]
fn webhook_envelope_extracts_routing_fields() {
    let body = br#"{
        "action": "completed",
        "installation": {"id": 42},
        "repository": {"full_name": "owner/repo"}
    }"#;
    let envelope = parse_github_webhook_envelope("workflow_run", Some("abc"), body).unwrap();
    assert_eq!(envelope.event, "workflow_run");
    assert_eq!(envelope.delivery_id, "abc");
    assert_eq!(envelope.installation_id, Some(42));
    assert_eq!(envelope.repository_full_name.as_deref(), Some("owner/repo"));
    assert_eq!(envelope.action.as_deref(), Some("completed"));
}

#[test]
fn delivery_deduper_rejects_duplicate_delivery() {
    let deduper = GitHubDeliveryDeduper::new(Duration::from_secs(60));
    deduper.accept("delivery-1").unwrap();
    assert_eq!(
        deduper.accept("delivery-1").unwrap_err(),
        GitHubWebhookError::DuplicateDelivery
    );
}
