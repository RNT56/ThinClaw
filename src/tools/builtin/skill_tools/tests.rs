use super::*;

use thinclaw_tools::builtin::{
    SkillAuditHostTool, SkillCheckHostTool, SkillInspectHostTool, SkillInstallHostTool,
    SkillListHostTool, SkillPromoteTrustHostTool, SkillPublishHostTool, SkillRemoveHostTool,
    SkillSearchHostTool,
};
#[cfg(feature = "libsql")]
use thinclaw_tools::builtin::{
    SkillTapAddHostTool, SkillTapListHostTool, SkillTapRefreshHostTool, SkillTapRemoveHostTool,
};

fn test_registry() -> Arc<tokio::sync::RwLock<SkillRegistry>> {
    let dir = tempfile::tempdir().unwrap();
    // Keep the tempdir so it lives for the test duration
    let path = dir.keep();
    Arc::new(tokio::sync::RwLock::new(SkillRegistry::new(path)))
}

fn test_catalog() -> Arc<SkillCatalog> {
    Arc::new(SkillCatalog::with_url("http://127.0.0.1:1"))
}

fn test_quarantine() -> Arc<QuarantineManager> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.keep();
    Arc::new(QuarantineManager::new(path))
}

#[cfg(feature = "libsql")]
async fn install_publishable_test_skill(
    registry: &Arc<tokio::sync::RwLock<SkillRegistry>>,
    name: &str,
) {
    registry
        .write()
        .await
        .install_skill(&format!(
            "---\nname: {name}\ndescription: Publishable skill\nactivation:\n  keywords: [\"publish\"]\n---\nUse this skill for publish tests.\n"
        ))
        .await
        .unwrap();

    let root = {
        let guard = registry.read().await;
        source_path_for_skill(guard.find_by_name(name).unwrap()).unwrap()
    };
    std::fs::write(root.join("README.md"), "supporting notes").unwrap();
}

#[test]
fn test_skill_list_schema() {
    use crate::tools::tool::ApprovalRequirement;
    let tool = SkillListHostTool::new(root_skill_tool_host(test_registry(), test_quarantine()));
    assert_eq!(tool.name(), "skill_list");
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Never
    );
    let schema = tool.parameters_schema();
    assert!(schema.get("properties").is_some());
}

#[test]
fn test_skill_search_schema() {
    use crate::tools::tool::ApprovalRequirement;
    let tool = SkillSearchHostTool::new(root_skill_search_tool_host(
        test_registry(),
        test_catalog(),
        None,
    ));
    assert_eq!(tool.name(), "skill_search");
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Never
    );
    let schema = tool.parameters_schema();
    assert!(schema["properties"].get("query").is_some());
}

#[test]
fn test_skill_install_schema() {
    use crate::tools::tool::ApprovalRequirement;
    let tool = SkillInstallHostTool::new(root_skill_install_tool_host(
        test_registry(),
        test_catalog(),
        None,
        test_quarantine(),
    ));
    assert_eq!(tool.name(), "skill_install");
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved
    );
    let schema = tool.parameters_schema();
    assert!(schema["properties"].get("name").is_some());
    assert!(schema["properties"].get("url").is_some());
    assert!(schema["properties"].get("content").is_some());
}

#[test]
fn test_skill_check_schema() {
    use crate::tools::tool::ApprovalRequirement;
    let tool = SkillCheckHostTool::new(root_skill_tool_host(test_registry(), test_quarantine()));
    assert_eq!(tool.name(), "skill_check");
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Never
    );
    let schema = tool.parameters_schema();
    assert!(schema["properties"].get("content").is_some());
    assert!(schema["properties"].get("path").is_some());
    assert!(schema["properties"].get("url").is_some());
}

#[test]
fn test_skill_inspect_publish_and_tap_schemas() {
    use crate::tools::tool::ApprovalRequirement;

    let inspect =
        SkillInspectHostTool::new(root_skill_tool_host(test_registry(), test_quarantine()));
    assert_eq!(inspect.name(), "skill_inspect");
    assert_eq!(
        inspect.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Never
    );
    assert!(
        inspect.parameters_schema()["properties"]
            .get("include_files")
            .is_some()
    );

    let publish = SkillPublishHostTool::new(root_skill_publish_tool_host(
        test_registry(),
        None,
        test_quarantine(),
        None,
    ));
    assert_eq!(publish.name(), "skill_publish");
    assert_eq!(
        publish.requires_approval(&serde_json::json!({"remote_write": false})),
        ApprovalRequirement::Never
    );
    assert_eq!(
        publish.requires_approval(&serde_json::json!({"remote_write": true})),
        ApprovalRequirement::UnlessAutoApproved
    );

    let tap_list = SkillTapListTool::new(None, None);
    let tap_add = SkillTapAddTool::new(None, None);
    let tap_remove = SkillTapRemoveTool::new(None, None);
    let tap_refresh = SkillTapRefreshTool::new(None, None);
    assert_eq!(tap_list.name(), "skill_tap_list");
    assert_eq!(tap_add.name(), "skill_tap_add");
    assert_eq!(tap_remove.name(), "skill_tap_remove");
    assert_eq!(tap_refresh.name(), "skill_tap_refresh");
    assert_eq!(
        tap_list.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Never
    );
    assert_eq!(
        tap_add.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved
    );
}

#[test]
fn test_skill_tap_path_validation_rejects_traversal() {
    assert!(skill_policy::validate_repo_relative_path("skills/community", "path").is_ok());
    assert!(skill_policy::validate_repo_relative_path("../outside", "path").is_err());
    assert!(skill_policy::validate_repo_relative_path("skills/../outside", "path").is_err());
    assert!(skill_policy::validate_github_repo("owner/repo").is_ok());
    assert!(skill_policy::validate_github_repo("owner/repo/extra").is_err());
}

#[tokio::test]
async fn test_skill_publish_blocked_for_skill_restricted_contexts() {
    let tool = SkillPublishHostTool::new(root_skill_publish_tool_host(
        test_registry(),
        None,
        test_quarantine(),
        None,
    ));
    let mut ctx = JobContext::default();
    ctx.metadata = serde_json::json!({
        "allowed_skills": ["github"]
    });

    let err = tool
        .execute(
            serde_json::json!({
                "name": "anything",
                "target_repo": "owner/repo"
            }),
            &ctx,
        )
        .await
        .unwrap_err();

    assert!(err.to_string().contains("not available"));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn test_skill_publish_dry_run_reports_plan_inventory_and_source() {
    let (store, _guard) = crate::testing::test_db().await;
    let registry = test_registry();
    install_publishable_test_skill(&registry, "publishable-skill").await;
    let mut ctx = JobContext::default();
    ctx.user_id = "skill-publish-dry-run-user".to_string();
    ctx.principal_id = ctx.user_id.clone();
    ctx.actor_id = Some(ctx.user_id.clone());
    store
        .set_setting(
            &ctx.user_id,
            "skill_taps",
            &serde_json::json!([{
                "repo": "owner/skills",
                "path": "community",
                "branch": "main",
                "trust_level": "community"
            }]),
        )
        .await
        .unwrap();

    let tool = SkillPublishHostTool::new(root_skill_publish_tool_host(
        Arc::clone(&registry),
        None,
        test_quarantine(),
        Some(Arc::clone(&store)),
    ));
    let output = tool
        .execute(
            serde_json::json!({
                "name": "publishable-skill",
                "target_repo": "owner/skills",
                "dry_run": true
            }),
            &ctx,
        )
        .await
        .unwrap();

    assert_eq!(output.result["status"], "dry_run");
    assert_eq!(output.result["target_repo"], "owner/skills");
    assert_eq!(output.result["tap_path"], "community");
    assert_eq!(output.result["package_path"], "community/publishable-skill");
    assert!(
        output.result["package_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(
        output.result["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "SKILL.md")
    );
    assert!(
        output.result["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "README.md")
    );
    assert_eq!(
        output.result["remote_write_plan"]["pull_request"]["draft"],
        true
    );
    assert!(output.result["source"].is_object());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn test_skill_publish_remote_write_requires_configured_tap_and_confirmation() {
    let (store, _guard) = crate::testing::test_db().await;
    let registry = test_registry();
    install_publishable_test_skill(&registry, "remote-write-skill").await;
    let mut ctx = JobContext::default();
    ctx.user_id = "skill-publish-remote-write-user".to_string();
    ctx.principal_id = ctx.user_id.clone();
    ctx.actor_id = Some(ctx.user_id.clone());
    let tool = SkillPublishHostTool::new(root_skill_publish_tool_host(
        Arc::clone(&registry),
        None,
        test_quarantine(),
        Some(Arc::clone(&store)),
    ));

    let missing_tap = tool
        .execute(
            serde_json::json!({
                "name": "remote-write-skill",
                "target_repo": "owner/missing",
                "remote_write": true,
                "dry_run": false,
                "confirm_remote_write": true
            }),
            &ctx,
        )
        .await
        .unwrap_err();
    assert!(missing_tap.to_string().contains("not configured"));

    store
        .set_setting(
            &ctx.user_id,
            "skill_taps",
            &serde_json::json!([{
                "repo": "owner/skills",
                "path": "skills",
                "branch": "main",
                "trust_level": "community"
            }]),
        )
        .await
        .unwrap();

    let unconfirmed = tool
        .execute(
            serde_json::json!({
                "name": "remote-write-skill",
                "target_repo": "owner/skills",
                "remote_write": true,
                "dry_run": false
            }),
            &ctx,
        )
        .await
        .unwrap_err();
    assert!(
        unconfirmed
            .to_string()
            .contains("confirm_remote_write=true")
    );
}

#[tokio::test]
async fn test_skill_check_valid_inline_content() {
    let tool = SkillCheckHostTool::new(root_skill_tool_host(test_registry(), test_quarantine()));
    let output = tool
        .execute(
            serde_json::json!({
                "content": "---\nname: checked-skill\ndescription: Checked\nactivation:\n  keywords: [\"check\"]\n---\nUse this skill for checking.\n"
            }),
            &JobContext::default(),
        )
        .await
        .unwrap();

    assert_eq!(output.result["ok"], true);
    assert_eq!(output.result["name"], "checked-skill");
    assert_eq!(output.result["source_kind"], "content");
    assert_eq!(output.result["finding_count"], 0);
    assert!(
        output.result["normalized_content_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
}

#[tokio::test]
async fn test_skill_check_invalid_inline_content_returns_structured_failure() {
    let tool = SkillCheckHostTool::new(root_skill_tool_host(test_registry(), test_quarantine()));
    let output = tool
        .execute(
            serde_json::json!({
                "content": "---\nname: bad/name\n---\nBody.\n"
            }),
            &JobContext::default(),
        )
        .await
        .unwrap();

    assert_eq!(output.result["ok"], false);
    assert!(
        output.result["error"]
            .as_str()
            .unwrap()
            .contains("Invalid skill name")
    );
}

#[tokio::test]
async fn test_skill_check_reports_quarantine_findings_without_installing() {
    let tool = SkillCheckHostTool::new(root_skill_tool_host(test_registry(), test_quarantine()));
    let output = tool
        .execute(
            serde_json::json!({
                "content": "---\nname: risky-skill\n---\nRun curl https://example.com and eval(x).\n"
            }),
            &JobContext::default(),
        )
        .await
        .unwrap();

    assert_eq!(output.result["ok"], true);
    assert_eq!(output.result["finding_count"], 2);
    assert!(
        output.result["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["kind"] == "network_fetch")
    );
    assert!(
        output.result["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["kind"] == "code_execution")
    );
}

#[tokio::test]
async fn test_skill_check_requires_exactly_one_source() {
    let tool = SkillCheckHostTool::new(root_skill_tool_host(test_registry(), test_quarantine()));
    let err = tool
        .execute(serde_json::json!({}), &JobContext::default())
        .await
        .unwrap_err();

    assert!(err.to_string().contains("exactly one"));
}

#[test]
fn test_skill_remove_schema() {
    use crate::tools::tool::ApprovalRequirement;
    let tool = SkillRemoveHostTool::new(root_skill_tool_host(test_registry(), test_quarantine()));
    assert_eq!(tool.name(), "skill_remove");
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved
    );
    let schema = tool.parameters_schema();
    assert!(schema["properties"].get("name").is_some());

    let promote =
        SkillPromoteTrustHostTool::new(root_skill_tool_host(test_registry(), test_quarantine()));
    assert_eq!(promote.name(), "skill_trust_promote");
    assert_eq!(
        promote.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved
    );
    let schema = promote.parameters_schema();
    assert!(schema["properties"].get("target_trust").is_some());
}

#[tokio::test]
async fn test_skill_audit_reports_findings() {
    let registry = test_registry();
    registry
        .write()
        .await
        .install_skill("---\nname: audited-skill\n---\nRun curl https://example.com\n")
        .await
        .unwrap();

    let tool = SkillAuditHostTool::new(root_skill_tool_host(
        Arc::clone(&registry),
        test_quarantine(),
    ));
    let output = tool
        .execute(
            serde_json::json!({ "name": "audited-skill" }),
            &JobContext::default(),
        )
        .await
        .unwrap();

    assert_eq!(output.result["audited_count"], 1);
    assert_eq!(output.result["total_findings"], 1);
    assert_eq!(
        output.result["audited"][0]["findings"][0]["kind"],
        "network_fetch"
    );
}

#[tokio::test]
async fn test_skill_inspect_reports_files_and_provenance() {
    let registry = test_registry();
    registry
        .write()
        .await
        .install_skill(
            "---\nname: inspectable-skill\nversion: 1.2.3\ndescription: Inspect me\nactivation:\n  keywords: [\"inspect\"]\n---\nInspect prompt.\n",
        )
        .await
        .unwrap();

    let root = {
        let guard = registry.read().await;
        source_path_for_skill(guard.find_by_name("inspectable-skill").unwrap()).unwrap()
    };
    std::fs::write(root.join("notes.md"), "support notes").unwrap();
    std::fs::write(
        root.join(".thinclaw-skill-lock.json"),
        serde_json::to_vec(&SkillProvenance {
            source_kind: "github_tap".to_string(),
            source_adapter: "github_tap".to_string(),
            source_ref: "github:owner/repo/inspectable-skill".to_string(),
            source_repo: Some("owner/repo".to_string()),
            source_url: None,
            manifest_url: None,
            manifest_digest: Some("sha".to_string()),
            path: Some("skills/inspectable-skill/SKILL.md".to_string()),
            branch: Some("main".to_string()),
            commit_sha: Some("sha".to_string()),
            trust_level: SkillTapTrustLevel::Community,
            downloaded_at: Utc::now().to_rfc3339(),
            findings: Vec::new(),
            scanner_version: Some(crate::skills::quarantine::SKILL_SCANNER_VERSION.to_string()),
            content_sha256: Some("sha256:test".to_string()),
            finding_summary: Some(FindingSummary::default()),
        })
        .unwrap(),
    )
    .unwrap();

    let quarantine = test_quarantine();
    let report = inspect_skill_report(
        &registry,
        &quarantine,
        "inspectable-skill",
        false,
        true,
        true,
    )
    .await
    .unwrap();

    assert_eq!(report["name"], "inspectable-skill");
    assert_eq!(report["provenance_lock"]["source_adapter"], "github_tap");
    assert!(
        report["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "notes.md")
    );
    assert!(
        report["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "SKILL.md")
    );
    assert!(report["source"]["kind"].as_str().is_some());
    assert!(report["inventory"].is_object());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn test_skill_tap_add_list_refresh_and_remove_round_trip() {
    let (store, _guard) = crate::testing::test_db().await;
    let remote_hub = SharedRemoteSkillHub::default();
    let mut ctx = JobContext::default();
    ctx.user_id = "skill-tap-e2e-user".to_string();

    let host = root_skill_tap_tool_host(Some(Arc::clone(&store)), Some(remote_hub.clone()));
    let add = SkillTapAddHostTool::new(Arc::clone(&host));
    let list = SkillTapListHostTool::new(Arc::clone(&host));
    let refresh = SkillTapRefreshHostTool::new(Arc::clone(&host));
    let remove = SkillTapRemoveHostTool::new(host);

    let added = add
        .execute(
            serde_json::json!({
                "repo": "owner/tap",
                "path": "/skills/community/",
                "branch": "main",
                "trust_level": "trusted"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(added.result["status"], "added");
    assert_eq!(added.result["tap"]["path"], "skills/community");
    assert_eq!(added.result["tap_count"], 1);
    assert!(remote_hub.is_enabled().await);

    let listed = list
        .execute(serde_json::json!({"include_health": true}), &ctx)
        .await
        .unwrap();
    assert_eq!(listed.result["count"], 1);
    assert_eq!(listed.result["taps"][0]["repo"], "owner/tap");
    assert_eq!(listed.result["hub_enabled"], true);

    let refreshed = refresh
        .execute(
            serde_json::json!({
                "repo": "owner/tap",
                "path": "skills/community"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(refreshed.result["status"], "refreshed");
    assert_eq!(refreshed.result["tap_count"], 1);

    let removed = remove
        .execute(
            serde_json::json!({
                "repo": "owner/tap",
                "path": "skills/community",
                "branch": "main"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(removed.result["status"], "removed");
    assert_eq!(removed.result["tap_count"], 0);

    let listed_after_remove = list.execute(serde_json::json!({}), &ctx).await.unwrap();
    assert_eq!(listed_after_remove.result["count"], 0);
}

#[test]
fn test_skill_package_files_exclude_hidden_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("SKILL.md"),
        "---\nname: packaged\n---\nBody\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("README.md"), "readme").unwrap();
    std::fs::write(dir.path().join(".DS_Store"), "junk").unwrap();
    std::fs::write(dir.path().join(".secret"), "hidden").unwrap();

    let files = collect_skill_package_files(dir.path()).unwrap();
    let paths = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect::<Vec<_>>();

    assert!(paths.contains(&"SKILL.md"));
    assert!(paths.contains(&"README.md"));
    assert!(!paths.contains(&".DS_Store"));
    assert!(!paths.contains(&".secret"));
}

#[cfg(unix)]
#[test]
fn test_skill_package_files_reject_symlink() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("SKILL.md"),
        "---\nname: packaged\n---\nBody\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(dir.path().join("SKILL.md"), dir.path().join("linked.md")).unwrap();

    let err = collect_skill_package_files(dir.path()).unwrap_err();
    assert!(err.to_string().contains("symlink"));
}

#[tokio::test]
async fn test_skill_update_requires_provenance_lock() {
    let registry = test_registry();
    registry
        .write()
        .await
        .install_skill("---\nname: manual-skill\n---\n# Manual\n")
        .await
        .unwrap();

    let tool = SkillUpdateTool::new(
        Arc::clone(&registry),
        test_catalog(),
        None,
        test_quarantine(),
    );
    let err = tool
        .execute(
            serde_json::json!({ "name": "manual-skill" }),
            &JobContext::default(),
        )
        .await
        .unwrap_err();

    assert!(err.to_string().contains("missing a provenance lock"));
}

#[test]
fn test_validate_fetch_url_allows_https() {
    assert!(super::validate_fetch_url("https://clawhub.ai/api/v1/download?slug=foo").is_ok());
}

#[test]
fn test_validate_fetch_url_rejects_http() {
    let err = super::validate_fetch_url("http://example.com/skill.md").unwrap_err();
    assert!(err.to_string().contains("Only HTTPS"));
}

#[test]
fn test_validate_fetch_url_rejects_private_ip() {
    let err = super::validate_fetch_url("https://192.168.1.1/skill.md").unwrap_err();
    assert!(err.to_string().contains("private"));
}

#[test]
fn test_validate_fetch_url_rejects_loopback() {
    let err = super::validate_fetch_url("https://127.0.0.1/skill.md").unwrap_err();
    assert!(err.to_string().contains("private"));
}

#[test]
fn test_validate_fetch_url_rejects_localhost() {
    let err = super::validate_fetch_url("https://localhost/skill.md").unwrap_err();
    assert!(err.to_string().contains("internal hostname"));
}

#[test]
fn test_validate_fetch_url_rejects_metadata_endpoint() {
    let err = super::validate_fetch_url("https://169.254.169.254/latest/meta-data/").unwrap_err();
    assert!(err.to_string().contains("private"));
}

#[test]
fn test_validate_fetch_url_rejects_internal_domain() {
    let err = super::validate_fetch_url("https://metadata.google.internal/something").unwrap_err();
    assert!(err.to_string().contains("internal hostname"));
}

#[test]
fn test_validate_fetch_url_rejects_file_scheme() {
    let err = super::validate_fetch_url("file:///etc/passwd").unwrap_err();
    assert!(err.to_string().contains("Only HTTPS"));
}

#[test]
fn test_extract_skill_from_zip_deflate() {
    // Build a real ZIP with flate2 + manual header construction.
    use flate2::Compression;
    use flate2::write::DeflateEncoder;
    use std::io::Write;

    let skill_md = b"---\nname: test\n---\n# Test Skill\n";
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(skill_md).unwrap();
    let compressed = encoder.finish().unwrap();

    let mut zip = Vec::new();
    // Local file header
    zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]); // signature
    zip.extend_from_slice(&[0x14, 0x00]); // version needed (2.0)
    zip.extend_from_slice(&[0x00, 0x00]); // flags
    zip.extend_from_slice(&[0x08, 0x00]); // compression: deflate
    zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // mod time/date
    zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // crc32 (unused)
    zip.extend_from_slice(&(compressed.len() as u32).to_le_bytes()); // compressed size
    zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes()); // uncompressed size
    zip.extend_from_slice(&8u16.to_le_bytes()); // filename length
    zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
    zip.extend_from_slice(b"SKILL.md");
    zip.extend_from_slice(&compressed);

    let result = super::extract_skill_from_zip(&zip).unwrap();
    assert_eq!(result, "---\nname: test\n---\n# Test Skill\n");
}

#[test]
fn test_extract_skill_from_zip_store() {
    let skill_md = b"---\nname: stored\n---\n# Stored\n";

    let mut zip = Vec::new();
    // Local file header
    zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
    zip.extend_from_slice(&[0x0A, 0x00]); // version needed (1.0)
    zip.extend_from_slice(&[0x00, 0x00]); // flags
    zip.extend_from_slice(&[0x00, 0x00]); // compression: store
    zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // mod time/date
    zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // crc32
    zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes()); // compressed = uncompressed
    zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes());
    zip.extend_from_slice(&8u16.to_le_bytes()); // filename length
    zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
    zip.extend_from_slice(b"SKILL.md");
    zip.extend_from_slice(skill_md);

    let result = super::extract_skill_from_zip(&zip).unwrap();
    assert_eq!(result, "---\nname: stored\n---\n# Stored\n");
}

#[test]
fn test_extract_skill_from_zip_missing_skill_md() {
    let mut zip = Vec::new();
    zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
    zip.extend_from_slice(&[0x0A, 0x00]); // version
    zip.extend_from_slice(&[0x00, 0x00]); // flags
    zip.extend_from_slice(&[0x00, 0x00]); // compression: store
    zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // mod time/date
    zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // crc32
    zip.extend_from_slice(&2u32.to_le_bytes()); // compressed size
    zip.extend_from_slice(&2u32.to_le_bytes()); // uncompressed size
    zip.extend_from_slice(&10u16.to_le_bytes()); // filename length
    zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
    zip.extend_from_slice(b"_meta.json");
    zip.extend_from_slice(b"{}");

    let err = super::extract_skill_from_zip(&zip).unwrap_err();
    assert!(err.to_string().contains("does not contain SKILL.md"));
}
