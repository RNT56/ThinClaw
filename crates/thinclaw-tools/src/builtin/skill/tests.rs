use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_tools_core::{ApprovalRequirement, Tool};
use thinclaw_types::JobContext;

use super::*;
use crate::ports::ToolOperationScope;
use crate::ports::*;

use crate::ports::{
    SkillInstallToolHostPort, SkillSearchToolHostPort, ToolSkillCheckRequest, ToolSkillCheckResult,
    ToolSkillCheckSource, ToolSkillInstallActionRequest, ToolSkillInstallRequest,
    ToolSkillMutationActionResult, ToolSkillPublishRequest, ToolSkillPublishResult, ToolSkillQuery,
    ToolSkillRead, ToolSkillRemoveResult, ToolSkillSearchCatalogEntry, ToolSkillSearchLocalEntry,
    ToolSkillSearchRemoteEntry, ToolSkillSearchRequest, ToolSkillSearchResult,
    ToolSkillSnapshotResult, ToolSkillSummary, ToolSkillTapList, ToolSkillTapMutationResult,
    ToolSkillTapRefreshResult, ToolSkillTrust, ToolSkillTrustMutationRequest,
    ToolSkillTrustMutationResult, ToolSkillUpdateActionRequest,
};

struct StubSkillHost;

struct StubSkillSearchHost;

struct StubSkillInstallHost;

#[async_trait]
impl SkillInstallToolHostPort for StubSkillInstallHost {
    async fn install_skill_action(
        &self,
        request: ToolSkillInstallActionRequest,
    ) -> Result<ToolSkillMutationActionResult, ToolHostError> {
        Ok(ToolSkillMutationActionResult {
            output: skill_install_output(
                &request.name,
                request.force,
                vec![skill_finding_output("network", "high", "curl")],
            ),
        })
    }

    async fn update_skill_action(
        &self,
        request: ToolSkillUpdateActionRequest,
    ) -> Result<ToolSkillMutationActionResult, ToolHostError> {
        Ok(ToolSkillMutationActionResult {
            output: skill_install_output(&request.name, true, Vec::new()),
        })
    }
}

#[async_trait]
impl SkillSearchToolHostPort for StubSkillSearchHost {
    async fn search_skills(
        &self,
        request: ToolSkillSearchRequest,
    ) -> Result<ToolSkillSearchResult, ToolHostError> {
        Ok(ToolSkillSearchResult {
            catalog: vec![ToolSkillSearchCatalogEntry {
                slug: "owner/docs".to_string(),
                name: "docs".to_string(),
                description: "Documentation helper".to_string(),
                version: "1.0.0".to_string(),
                score: 0.95,
                installed: true,
                stars: Some(10),
                downloads: Some(20),
                owner: Some("owner".to_string()),
            }],
            remote: vec![ToolSkillSearchRemoteEntry {
                slug: "owner/review".to_string(),
                name: "review".to_string(),
                description: "Review helper".to_string(),
                version: "0.1.0".to_string(),
                source: "github_tap".to_string(),
                source_label: "GitHub".to_string(),
                source_ref: "owner/skills".to_string(),
                manifest_url: Some("https://example.test/SKILL.md".to_string()),
                manifest_digest: Some("sha256:remote".to_string()),
                repo: Some("owner/skills".to_string()),
                path: Some("skills/review".to_string()),
                branch: Some("main".to_string()),
                trust_level: "trusted".to_string(),
            }],
            local: vec![ToolSkillSearchLocalEntry {
                name: "docs".to_string(),
                description: "Documentation helper".to_string(),
                trust: "trusted".to_string(),
                source_tier: "user".to_string(),
            }],
            registry_url: "https://registry.test".to_string(),
            catalog_error: (request.source == "clawhub").then(|| "offline".to_string()),
        })
    }
}

#[async_trait]
impl SkillToolHostPort for StubSkillHost {
    async fn list_skills(
        &self,
        _query: ToolSkillQuery,
    ) -> Result<Vec<ToolSkillSummary>, ToolHostError> {
        Ok(vec![ToolSkillSummary {
            name: "docs".to_string(),
            description: Some("Documentation helper".to_string()),
            trust: ToolSkillTrust::Trusted,
            enabled: true,
            metadata: serde_json::json!({
                "source_tier": "user",
                "source": "User",
                "keywords": ["docs"],
                "version": "1.0.0",
                "tags": ["writing"],
                "content_hash": "sha256:abc",
                "max_context_tokens": 1200,
                "provenance": {"kind": "test"},
                "lifecycle_status": "active",
                "outcome_score": 0.9,
                "reuse_count": 3,
                "activation_reason": "keyword"
            }),
        }])
    }

    async fn inspect_skill(
        &self,
        _scope: ToolOperationScope,
        name: String,
        include_content: bool,
        include_files: bool,
        audit: bool,
    ) -> Result<serde_json::Value, ToolHostError> {
        Ok(serde_json::json!({
            "name": name,
            "include_content": include_content,
            "include_files": include_files,
            "audit": audit
        }))
    }

    async fn read_skill(
        &self,
        _scope: ToolOperationScope,
        name: String,
    ) -> Result<ToolSkillRead, ToolHostError> {
        Ok(ToolSkillRead {
            name,
            version: "1.0.0".to_string(),
            description: "Documentation helper".to_string(),
            trust: ToolSkillTrust::Trusted,
            source_tier: "user".to_string(),
            content: "Use this skill for docs.".to_string(),
        })
    }

    async fn install_skill(
        &self,
        request: ToolSkillInstallRequest,
    ) -> Result<ToolSkillSummary, ToolHostError> {
        Ok(ToolSkillSummary {
            name: request.name,
            description: None,
            trust: ToolSkillTrust::Community,
            enabled: true,
            metadata: serde_json::Value::Null,
        })
    }

    async fn check_skill(
        &self,
        request: ToolSkillCheckRequest,
    ) -> Result<ToolSkillCheckResult, ToolHostError> {
        let (source_kind, source_ref) = match request.source {
            ToolSkillCheckSource::InlineContent { .. } => {
                ("content".to_string(), "(inline content)".to_string())
            }
            ToolSkillCheckSource::Path { path } => ("path".to_string(), path),
            ToolSkillCheckSource::Url { url } => ("url".to_string(), url),
        };
        Ok(ToolSkillCheckResult {
            output: skill_check_success_output(
                &source_kind,
                &source_ref,
                "docs",
                "1.0.0",
                "Documentation helper",
                serde_json::json!({"keywords": ["docs"]}),
                "installed",
                "user",
                64,
                1200,
                "sha256:abc",
                "sha256:def",
                Vec::new(),
            ),
        })
    }

    async fn remove_skill(
        &self,
        _scope: ToolOperationScope,
        name: String,
    ) -> Result<ToolSkillRemoveResult, ToolHostError> {
        Ok(ToolSkillRemoveResult { name })
    }

    async fn promote_skill_trust(
        &self,
        request: ToolSkillTrustMutationRequest,
    ) -> Result<ToolSkillTrustMutationResult, ToolHostError> {
        Ok(ToolSkillTrustMutationResult {
            name: request.name,
            trust: request.target_trust,
            source_tier: "user".to_string(),
        })
    }

    async fn audit_skills(
        &self,
        _scope: ToolOperationScope,
        name: Option<String>,
    ) -> Result<Vec<serde_json::Value>, ToolHostError> {
        Ok(vec![skill_audit_entry_output(
            name.as_deref().unwrap_or("docs"),
            "trusted",
            "user",
            "/tmp/docs",
            vec![skill_finding_output("network", "high", "curl")],
        )])
    }

    async fn reload_skills(
        &self,
        _scope: ToolOperationScope,
        name: Option<String>,
    ) -> Result<Vec<ToolSkillSummary>, ToolHostError> {
        let names = name
            .map(|name| vec![name])
            .unwrap_or_else(|| vec!["docs".to_string(), "review".to_string()]);
        Ok(names
            .into_iter()
            .map(|name| ToolSkillSummary {
                name,
                description: None,
                trust: ToolSkillTrust::Trusted,
                enabled: true,
                metadata: serde_json::Value::Null,
            })
            .collect())
    }

    async fn snapshot_skills(
        &self,
        _scope: ToolOperationScope,
    ) -> Result<ToolSkillSnapshotResult, ToolHostError> {
        Ok(ToolSkillSnapshotResult {
            path: "/tmp/skills/snapshot.json".to_string(),
            count: 2,
        })
    }
}

struct StubSkillPublishHost;

#[async_trait]
impl SkillPublishToolHostPort for StubSkillPublishHost {
    async fn publish_skill(
        &self,
        request: ToolSkillPublishRequest,
    ) -> Result<ToolSkillPublishResult, ToolHostError> {
        Ok(ToolSkillPublishResult {
            status: if request.remote_write {
                "published".to_string()
            } else {
                "dry_run".to_string()
            },
            name: request.name,
            target_repo: request.target_repo,
            tap_path: "community".to_string(),
            package_path: "community/docs".to_string(),
            branch: "codex/skill-publish/docs-1234abcd".to_string(),
            base_branch: Some("main".to_string()),
            package_hash: "sha256:1234".to_string(),
            files: vec![serde_json::json!({"path": "SKILL.md", "bytes": 10})],
            findings: Vec::new(),
            trust: "trusted".to_string(),
            source_tier: "user".to_string(),
            source: skill_source_output("user", "/tmp/docs"),
            remote_write_plan: serde_json::Value::Null,
            metadata: serde_json::json!({
                "scanner_version": "test",
                "content_sha256": "sha256:content"
            }),
        })
    }
}

struct StubSkillTapHost;

#[async_trait]
impl SkillTapToolHostPort for StubSkillTapHost {
    async fn list_skill_taps(
        &self,
        query: ToolSkillTapQuery,
    ) -> Result<ToolSkillTapList, ToolHostError> {
        Ok(ToolSkillTapList::new(
            vec![ToolSkillTap {
                repo: "owner/skills".to_string(),
                path: "packs/core".to_string(),
                branch: Some("main".to_string()),
                trust_level: ToolSkillTapTrust::Trusted,
            }],
            Some(query.include_health),
        ))
    }

    async fn add_skill_tap(
        &self,
        request: ToolSkillTapAddRequest,
    ) -> Result<ToolSkillTapMutationResult, ToolHostError> {
        Ok(ToolSkillTapMutationResult {
            status: if request.replace {
                "replaced".to_string()
            } else {
                "added".to_string()
            },
            tap: Some(ToolSkillTap {
                repo: request.repo,
                path: request.path,
                branch: request.branch,
                trust_level: request.trust_level,
            }),
            tap_count: 1,
        })
    }

    async fn remove_skill_tap(
        &self,
        request: ToolSkillTapRemoveRequest,
    ) -> Result<ToolSkillTapMutationResult, ToolHostError> {
        Ok(ToolSkillTapMutationResult {
            status: "removed".to_string(),
            tap: Some(ToolSkillTap {
                repo: request.repo,
                path: request.path,
                branch: request.branch,
                trust_level: ToolSkillTapTrust::Community,
            }),
            tap_count: 0,
        })
    }

    async fn refresh_skill_taps(
        &self,
        request: ToolSkillTapRefreshRequest,
    ) -> Result<ToolSkillTapRefreshResult, ToolHostError> {
        Ok(ToolSkillTapRefreshResult {
            status: "refreshed".to_string(),
            tap_count: 1,
            repo: request.repo,
            path: request.path,
            hub_enabled: true,
        })
    }
}

fn stub_tap_host() -> Arc<dyn SkillTapToolHostPort> {
    Arc::new(StubSkillTapHost)
}

#[test]
fn package_skip_policy_filters_generated_and_hidden_names() {
    assert!(is_skipped_package_name(".git"));
    assert!(is_skipped_package_name("node_modules"));
    assert!(is_skipped_package_name(".hidden"));
    assert!(!is_skipped_package_name("SKILL.md"));
}

#[test]
fn skill_allowlist_policy_allows_only_listed_skills() {
    let metadata = serde_json::json!({ "allowed_skills": ["github"] });
    assert!(ensure_skill_allowed(&metadata, "github").is_ok());
    assert!(ensure_skill_allowed(&metadata, "calendar").is_err());
    assert_eq!(
        restricted_skill_names(&metadata).unwrap(),
        std::collections::HashSet::from(["github".to_string()])
    );
    assert!(ensure_skill_admin_available(&metadata, "skill_install").is_err());
    assert!(ensure_skill_admin_available(&serde_json::json!({}), "skill_install").is_ok());
}

#[test]
fn skill_check_input_converts_to_and_from_port_source() {
    let input = SkillCheckInput::InlineContent("name: test".to_string());
    let source: ToolSkillCheckSource = input.clone().into();
    assert_eq!(
        source,
        ToolSkillCheckSource::InlineContent {
            content: "name: test".to_string(),
        }
    );
    assert_eq!(SkillCheckInput::from(source), input);

    let path_source = ToolSkillCheckSource::Path {
        path: "/tmp/example".to_string(),
    };
    assert_eq!(
        SkillCheckInput::from(path_source),
        SkillCheckInput::Path("/tmp/example".to_string())
    );

    let url_input = SkillCheckInput::Url("https://example.test/SKILL.md".to_string());
    assert_eq!(
        ToolSkillCheckSource::from(url_input),
        ToolSkillCheckSource::Url {
            url: "https://example.test/SKILL.md".to_string(),
        }
    );
}

#[test]
fn skill_discovery_schemas_and_search_params_are_root_independent() {
    assert_eq!(skill_inspect_parameters_schema()["required"][0], "name");
    let inspect = parse_skill_inspect_params(&serde_json::json!({
        "name": "github",
        "include_content": true,
        "include_files": false,
        "audit": false
    }))
    .unwrap();
    assert_eq!(inspect.name, "github");
    assert!(inspect.include_content);
    assert!(!inspect.include_files);
    assert!(!inspect.audit);

    assert_eq!(skill_read_parameters_schema()["required"][0], "name");
    assert_eq!(
        parse_skill_name_param(&serde_json::json!({"name": "github"})).unwrap(),
        "github"
    );
    let read = skill_read_output("github", "1.0.0", "desc", "trusted", "user", "body");
    assert_eq!(read["name"], "github");
    assert_eq!(read["content"], "body");
    let source = skill_source_output("user", "/tmp/github");
    assert_eq!(source["kind"], "user");
    let inspect = skill_inspect_output(
        "github",
        "1.0.0",
        "desc",
        serde_json::json!({"keywords": ["git"]}),
        serde_json::json!({"owner": "dev"}),
        "trusted",
        "user",
        source,
        "abc",
        12,
        Some(serde_json::json!({"source": "lock"})),
        vec![skill_finding_output("network", "high", "curl")],
        vec![skill_inventory_error_output("missing file")],
        Some("prompt"),
    );
    assert_eq!(inspect["finding_count"], 1);
    assert_eq!(inspect["inventory"]["file_count"], 1);
    assert_eq!(inspect["content"], "prompt");

    assert_eq!(
        skill_list_parameters_schema()["properties"]["verbose"]["default"],
        false
    );
    assert!(parse_skill_list_params(&serde_json::json!({"verbose": true})).verbose);
    let mut entry = skill_list_entry(
        "github",
        "desc",
        "trusted",
        "user",
        "User(/tmp/skill)",
        serde_json::json!(["git"]),
    );
    add_skill_list_verbose_fields(
        &mut entry,
        SkillListVerboseFields {
            version: "1.0.0".to_string(),
            tags: serde_json::json!(["code"]),
            content_hash: "abc".to_string(),
            max_context_tokens: serde_json::json!(1024),
            provenance: Some(serde_json::json!("manual")),
            lifecycle_status: None,
            outcome_score: None,
            reuse_count: Some(serde_json::json!(3)),
            activation_reason: None,
        },
    );
    assert_eq!(entry["version"], "1.0.0");
    let listed = skill_list_output(vec![entry]);
    assert_eq!(listed["count"], 1);

    assert_eq!(
        skill_search_parameters_schema()["properties"]["source"]["default"],
        "all"
    );

    let params = parse_skill_search_params(&serde_json::json!({
        "query": "browser",
        "source": "GITHUB"
    }))
    .unwrap();
    assert_eq!(params.query, "browser");
    assert_eq!(params.source, "github");
    assert!(parse_skill_search_params(&serde_json::json!({})).is_err());

    let local = skill_search_local_entry("github", "desc", "trusted", "user");
    assert_eq!(local["name"], "github");
    let catalog = skill_search_catalog_entry(
        "owner/github",
        "github",
        "desc",
        "1.0.0",
        0.9,
        true,
        Some(10),
        Some(20),
        Some("owner"),
    );
    assert_eq!(catalog["installed"], true);
    let remote = skill_search_remote_entry(
        "owner/github",
        "github",
        "desc",
        "1.0.0",
        "github_tap",
        "GitHub",
        "owner/repo",
        Some("https://example.test/SKILL.md"),
        Some("sha256:abc"),
        Some("owner/repo"),
        Some("skills/github"),
        Some("main"),
        "trusted",
    );
    let search = skill_search_output(
        "all",
        vec![catalog],
        vec![remote],
        vec![local],
        "https://registry.test",
        Some("offline".to_string()),
    );
    assert_eq!(search["github_count"], 1);
    assert_eq!(search["installed_count"], 1);
    assert_eq!(search["catalog_error"], "offline");

    assert_eq!(
        skill_check_parameters_schema()["properties"]["url"]["type"],
        "string"
    );
    assert_eq!(
        parse_skill_check_input(&serde_json::json!({"content": "name: test"})).unwrap(),
        SkillCheckInput::InlineContent("name: test".to_string())
    );
    assert!(parse_skill_check_input(&serde_json::json!({})).is_err());
    assert!(
        parse_skill_check_input(&serde_json::json!({
            "content": "x",
            "url": "https://example.test/SKILL.md"
        }))
        .is_err()
    );
    assert_eq!(
        skill_check_path_for_read("/tmp/example")
            .file_name()
            .and_then(|name| name.to_str()),
        Some("SKILL.md")
    );
    let findings = vec![skill_finding_output("network", "high", "curl")];
    let check = skill_check_success_output(
        "content",
        "(inline content)",
        "github",
        "1.0.0",
        "desc",
        serde_json::json!({"keywords": ["git"]}),
        "installed",
        "user",
        10,
        1024,
        "abc",
        "def",
        findings.clone(),
    );
    assert_eq!(check["ok"], true);
    assert_eq!(check["finding_count"], 1);
    let check_error =
        skill_check_error_output("content", "(inline content)", "invalid", "def", findings);
    assert_eq!(check_error["ok"], false);
    assert_eq!(check_error["error"], "invalid");

    assert_eq!(skill_install_parameters_schema()["required"][0], "name");
    let install_output = skill_install_output("github", false, Vec::new());
    assert_eq!(install_output["status"], "installed");
    assert_eq!(
        skill_audit_parameters_schema()["properties"]["name"]["type"],
        "string"
    );
    assert_eq!(
        parse_skill_audit_target_name(&serde_json::json!({"name": "github"})),
        Some("github")
    );
    let audit = skill_audit_output(vec![serde_json::json!({
        "finding_count": 2
    })]);
    assert_eq!(audit["total_findings"], 2);
    let audit_entry = skill_audit_entry_output(
        "github",
        "trusted",
        "user",
        "/tmp/github",
        vec![skill_finding_output("network", "high", "curl")],
    );
    assert_eq!(audit_entry["finding_count"], 1);

    assert_eq!(skill_update_parameters_schema()["required"][0], "name");
    let mut update_params = skill_update_install_params("github", true, true);
    add_skill_update_url(
        &mut update_params,
        "https://example.test/SKILL.md".to_string(),
    );
    assert_eq!(update_params["url"], "https://example.test/SKILL.md");
    assert_eq!(
        skill_publish_parameters_schema()["required"][1],
        "target_repo"
    );
    assert_eq!(
        skill_tap_list_parameters_schema()["properties"]["include_health"]["default"],
        false
    );
    assert_eq!(skill_tap_add_parameters_schema()["required"][0], "repo");
    let tap_add = parse_skill_tap_add_params(&serde_json::json!({
        "repo": "owner/repo",
        "path": "/skills/github/",
        "branch": " main ",
        "trust_level": "Trusted",
        "replace": true
    }))
    .unwrap();
    assert_eq!(tap_add.path, "skills/github");
    assert_eq!(tap_add.branch.as_deref(), Some("main"));
    assert_eq!(tap_add.trust_level, "trusted");
    assert!(tap_add.replace);
    let tap = skill_tap_json("owner/repo", "skills/github", Some("main"), "trusted");
    assert_eq!(tap["trust_level"], "trusted");
    let tap_list = skill_tap_list_output(vec![tap.clone()], Some(true));
    assert_eq!(tap_list["count"], 1);
    let tap_added = skill_tap_add_output(true, tap, 3);
    assert_eq!(tap_added["status"], "replaced");

    assert_eq!(skill_tap_remove_parameters_schema()["required"][0], "repo");
    let tap_remove = parse_skill_tap_remove_params(&serde_json::json!({
        "repo": "owner/repo",
        "path": "skills/github"
    }))
    .unwrap();
    assert_eq!(tap_remove.repo, "owner/repo");
    assert_eq!(
        skill_tap_remove_output("owner/repo", "skills/github", None, 2)["status"],
        "removed"
    );
    assert_eq!(
        skill_tap_refresh_parameters_schema()["properties"]["repo"]["default"],
        serde_json::Value::Null
    );
    let refresh = parse_skill_tap_refresh_params(&serde_json::json!({
        "repo": "owner/repo",
        "path": "/skills"
    }))
    .unwrap();
    assert_eq!(refresh.path.as_deref(), Some("skills"));
    let refresh_output =
        skill_tap_refresh_output(2, refresh.repo.as_deref(), refresh.path.as_deref(), true);
    assert_eq!(refresh_output["status"], "refreshed");
    assert_eq!(skill_snapshot_parameters_schema()["type"], "object");
    let snapshot_entry = skill_snapshot_entry("github", "1.0.0", "trusted", "user", "abc", None);
    let snapshot = skill_snapshot_document("now".to_string(), vec![snapshot_entry]);
    assert_eq!(snapshot["skills"].as_array().unwrap().len(), 1);
    assert_eq!(skill_snapshot_output("/tmp/snapshot.json", 1)["count"], 1);
    assert_eq!(
        skill_trust_promote_parameters_schema()["required"][1],
        "target_trust"
    );
    let promote = parse_skill_trust_promote_params(&serde_json::json!({
        "name": "github",
        "target_trust": "Trusted"
    }))
    .unwrap();
    assert_eq!(promote.name, "github");
    assert_eq!(promote.target_trust, "trusted");
    assert!(
        parse_skill_trust_promote_params(&serde_json::json!({
            "name": "github",
            "target_trust": "system"
        }))
        .is_err()
    );
    let promote_output = skill_trust_promote_output("github", "trusted", "user");
    assert_eq!(promote_output["status"], "updated");

    assert_eq!(skill_remove_parameters_schema()["required"][0], "name");
    assert_eq!(skill_remove_output("github")["status"], "removed");
    assert_eq!(
        skill_reload_parameters_schema()["properties"]["all"]["default"],
        false
    );

    let install = parse_skill_install_params(&serde_json::json!({
        "name": "docs",
        "force": true
    }))
    .unwrap();
    assert_eq!(install.name, "docs");
    assert!(install.force);
    assert!(!install.approve_risky);

    let update = parse_skill_update_params(&serde_json::json!({
        "name": "docs",
        "approve_risky": true
    }))
    .unwrap();
    assert_eq!(update.name, "docs");
    assert!(update.approve_risky);

    let publish = parse_skill_publish_params(&serde_json::json!({
        "name": "docs",
        "target_repo": " owner/repo "
    }))
    .unwrap();
    assert_eq!(publish.target_repo, "owner/repo");
    assert!(publish.dry_run);
    assert!(!publish.remote_write);
    let publish_plan = skill_publish_plan_output(
        "planned",
        "docs",
        "owner/repo",
        "skills",
        "skills/docs",
        "codex/skill-publish/docs-1234abcd",
        Some("main"),
        "sha256:1234",
        vec![serde_json::json!({"path": "SKILL.md", "bytes": 10})],
        Vec::new(),
        "trusted",
        "user",
        skill_source_output("user", "/tmp/docs"),
    );
    assert_eq!(publish_plan["file_count"], 1);
    assert_eq!(
        publish_plan["remote_write_plan"]["pull_request"]["title"],
        "[skills] publish docs"
    );

    let reload = parse_skill_reload_params(&serde_json::json!({
        "name": "docs"
    }));
    assert_eq!(reload.name.as_deref(), Some("docs"));
    assert!(!reload.all);
    assert_eq!(
        skill_reload_all_output(vec!["docs".to_string()])["count"],
        1
    );
    assert_eq!(skill_reload_output("docs")["status"], "reloaded");
}

#[test]
fn relative_path_policy_blocks_traversal() {
    assert!(relative_path_is_safe(Path::new("docs/NOTE.md")));
    assert!(relative_path_is_safe(Path::new("./docs/NOTE.md")));
    assert!(!relative_path_is_safe(Path::new("../outside")));
    assert!(!relative_path_is_safe(Path::new("/absolute")));
}

#[test]
fn package_file_json_reports_relative_paths_and_sizes() {
    let package = tempfile::tempdir().unwrap();
    std::fs::write(package.path().join("SKILL.md"), vec![b'x'; 42]).unwrap();
    let files = collect_skill_package_files(package.path()).unwrap();

    assert_eq!(
        package_file_json(&files),
        vec![serde_json::json!({"path": "SKILL.md", "bytes": 42})]
    );
}

#[test]
fn tap_path_normalization_trims_outer_slashes() {
    assert_eq!(normalize_tap_path("/skills/community/"), "skills/community");
    assert!(skill_tap_key_matches(
        "Owner/Repo",
        "/skills/community/",
        Some("main"),
        "owner/repo",
        "skills/community",
        Some("main"),
    ));
    assert!(skill_findings_require_approval("community", 1));
    assert_eq!(
        skill_findings_summary([skill_finding_summary("network", "high", "curl")]),
        "network (high): curl"
    );
}

#[test]
fn finding_policy_outputs_details_and_scan_metadata() {
    let finding = SkillFindingPolicy {
        kind: "network",
        severity: "warning",
        excerpt: "curl example.com",
        rule_id: Some("net-001"),
        file: Some("SKILL.md"),
        line: Some(12),
        recommendation: Some("Review network access"),
        scanner_version: Some("test-scanner"),
    };

    let outputs = skill_finding_detail_outputs([finding]);
    assert_eq!(outputs[0]["kind"], "network");
    assert_eq!(outputs[0]["severity"], "warning");
    assert_eq!(outputs[0]["rule_id"], "net-001");
    assert_eq!(outputs[0]["line"], 12);
    assert_eq!(
        skill_findings_detail_summary([finding]),
        "network (warning): curl example.com"
    );

    let mut output = serde_json::json!({"ok": true});
    add_skill_scan_report_fields(
        &mut output,
        "test-scanner",
        "sha256:content",
        SkillFindingSummary {
            total: 1,
            warnings: 1,
            critical: 0,
            categories: vec!["network".to_string()],
        },
    );
    assert_eq!(output["scanner_version"], "test-scanner");
    assert_eq!(output["finding_summary"]["categories"][0], "network");
}

#[test]
fn finding_policy_applies_approval_and_rejection_thresholds() {
    let warning = SkillFindingPolicy {
        kind: "network",
        severity: "warning",
        excerpt: "curl",
        rule_id: None,
        file: None,
        line: None,
        recommendation: None,
        scanner_version: None,
    };
    let critical = SkillFindingPolicy {
        severity: "critical",
        ..warning
    };
    let traversal = SkillFindingPolicy {
        kind: "path_traversal",
        ..critical
    };

    assert!(!skill_findings_require_approval_for_details(
        "community",
        [warning]
    ));
    assert!(skill_findings_require_approval_for_details(
        "community",
        [warning, warning]
    ));
    assert!(skill_findings_require_approval_for_details(
        "trusted",
        [critical]
    ));
    assert!(!skill_findings_require_approval_for_details(
        "builtin",
        [warning]
    ));
    assert!(skill_findings_require_rejection_for_details([traversal]));
    assert!(!skill_findings_require_rejection_for_details([critical]));
}

#[test]
fn publish_metadata_and_result_helpers_preserve_policy_shape() {
    let metadata = skill_publish_metadata_output(
        "test-scanner",
        "sha256:content",
        SkillFindingSummary {
            total: 0,
            warnings: 0,
            critical: 0,
            categories: Vec::new(),
        },
        [(
            "pr_url",
            serde_json::json!("https://github.com/owner/repo/pull/1"),
        )],
    );
    assert_eq!(metadata["scanner_version"], "test-scanner");
    assert_eq!(metadata["pr_url"], "https://github.com/owner/repo/pull/1");

    let result = skill_publish_result_output(
        "published",
        "docs",
        "owner/repo",
        "community",
        "community/docs",
        "codex/skill-publish/docs-1234abcd",
        Some("main".to_string()),
        "sha256:1234",
        vec![serde_json::json!({"path": "SKILL.md", "bytes": 10})],
        Vec::new(),
        "trusted",
        "user",
        skill_source_output("user", "/tmp/docs"),
        serde_json::Value::Null,
        metadata,
    );
    assert_eq!(result.status, "published");
    assert_eq!(result.package_path, "community/docs");
    assert_eq!(result.files[0]["path"], "SKILL.md");
    assert_eq!(result.metadata["scanner_version"], "test-scanner");
}

#[test]
fn publish_projection_output_preserves_plan_scan_and_remote_shape() {
    let output = skill_publish_projection_output(SkillPublishProjection {
        status: "published".to_string(),
        name: "docs".to_string(),
        target_repo: "owner/repo".to_string(),
        tap_path: "community".to_string(),
        package_path: "community/docs".to_string(),
        branch: "codex/skill-publish/docs-1234abcd".to_string(),
        base_branch: Some("main".to_string()),
        package_hash: "sha256:1234".to_string(),
        files: vec![serde_json::json!({"path": "SKILL.md", "bytes": 10})],
        findings: vec![skill_finding_output("network", "warning", "curl")],
        trust: "trusted".to_string(),
        source_tier: "user".to_string(),
        source: skill_source_output("user", "/tmp/docs"),
        scan: Some(SkillPublishScanProjection {
            scanner_version: "test-scanner".to_string(),
            content_sha256: "sha256:content".to_string(),
            finding_summary: SkillFindingSummary {
                total: 1,
                warnings: 1,
                critical: 0,
                categories: vec!["network".to_string()],
            },
        }),
        remote_write_plan: Some(serde_json::json!({
            "pull_request": {
                "title": "custom title"
            }
        })),
        metadata: Some(serde_json::json!({
            "pr_url": "https://github.com/owner/repo/pull/1"
        })),
    });

    assert_eq!(output["status"], "published");
    assert_eq!(output["file_count"], 1);
    assert_eq!(output["finding_count"], 1);
    assert_eq!(output["scanner_version"], "test-scanner");
    assert_eq!(output["content_sha256"], "sha256:content");
    assert_eq!(output["finding_summary"]["warnings"], 1);
    assert_eq!(
        output["remote_write_plan"]["pull_request"]["title"],
        "custom title"
    );
    assert_eq!(output["pr_url"], "https://github.com/owner/repo/pull/1");
}

#[tokio::test]
async fn skill_list_host_tool_preserves_basic_and_verbose_output_shapes() {
    let ctx = JobContext::with_identity("user-1", "actor-1", "list", "test");
    let tool = SkillListHostTool::new(Arc::new(StubSkillHost));

    let output = tool
        .execute(serde_json::json!({ "verbose": true }), &ctx)
        .await
        .unwrap();

    assert_eq!(output.result["count"], 1);
    assert_eq!(output.result["skills"][0]["name"], "docs");
    assert_eq!(
        output.result["skills"][0]["description"],
        "Documentation helper"
    );
    assert_eq!(output.result["skills"][0]["trust"], "trusted");
    assert_eq!(output.result["skills"][0]["source_tier"], "user");
    assert_eq!(output.result["skills"][0]["keywords"][0], "docs");
    assert_eq!(output.result["skills"][0]["version"], "1.0.0");
    assert_eq!(output.result["skills"][0]["content_hash"], "sha256:abc");
    assert_eq!(output.result["skills"][0]["reuse_count"], 3);
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Never
    );
}

#[tokio::test]
async fn skill_search_host_tool_preserves_existing_output_shape() {
    let ctx = JobContext::with_identity("user-1", "actor-1", "search", "test");
    let tool = SkillSearchHostTool::new(Arc::new(StubSkillSearchHost));

    let output = tool
        .execute(
            serde_json::json!({
                "query": "docs",
                "source": "all"
            }),
            &ctx,
        )
        .await
        .unwrap();

    assert_eq!(output.result["installed_count"], 1);
    assert_eq!(output.result["catalog_count"], 1);
    assert_eq!(output.result["github_count"], 1);
    assert_eq!(output.result["registry_url"], "https://registry.test");
    assert_eq!(
        output.result["catalog"][0]["slug"],
        serde_json::json!("owner/docs")
    );
    assert_eq!(
        output.result["github"][0]["source"],
        serde_json::json!("github_tap")
    );
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Never
    );
}

#[tokio::test]
async fn skill_snapshot_host_tool_preserves_existing_output_shape() {
    let ctx = JobContext::with_identity("user-1", "actor-1", "snapshot", "test");
    let tool = SkillSnapshotHostTool::new(Arc::new(StubSkillHost));

    let output = tool.execute(serde_json::json!({}), &ctx).await.unwrap();

    assert_eq!(output.result["path"], "/tmp/skills/snapshot.json");
    assert_eq!(output.result["count"], 2);
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved
    );
}

#[tokio::test]
async fn skill_check_host_tool_preserves_existing_output_shape_and_restrictions() {
    let ctx = JobContext::with_identity("user-1", "actor-1", "check", "test");
    let tool = SkillCheckHostTool::new(Arc::new(StubSkillHost));

    let output = tool
        .execute(serde_json::json!({ "content": "# docs" }), &ctx)
        .await
        .unwrap();

    assert_eq!(output.result["ok"], true);
    assert_eq!(output.result["source_kind"], "content");
    assert_eq!(output.result["source_ref"], "(inline content)");
    assert_eq!(output.result["name"], "docs");
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Never
    );

    let mut restricted = ctx.clone();
    restricted.metadata = serde_json::json!({ "allowed_skills": ["docs"] });
    let err = tool
        .execute(serde_json::json!({ "content": "# docs" }), &restricted)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not available"));
}

#[tokio::test]
async fn skill_install_and_update_host_tools_preserve_output_shapes() {
    let ctx = JobContext::with_identity("user-1", "actor-1", "install", "test");

    let install = SkillInstallHostTool::new(Arc::new(StubSkillInstallHost));
    let output = install
        .execute(
            serde_json::json!({
                "name": "docs",
                "force": true,
                "approve_risky": true,
                "content": "---\nname: docs\n---\nBody\n"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(output.result["status"], "updated");
    assert_eq!(output.result["name"], "docs");
    assert_eq!(output.result["findings"].as_array().unwrap().len(), 1);
    assert_eq!(
        install.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved
    );

    let update = SkillUpdateHostTool::new(Arc::new(StubSkillInstallHost));
    let output = update
        .execute(
            serde_json::json!({
                "name": "docs",
                "approve_risky": true
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(output.result["status"], "updated");
    assert_eq!(output.result["name"], "docs");
    assert_eq!(
        update.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved
    );
}

#[tokio::test]
async fn skill_remove_and_promote_host_tools_preserve_output_shapes() {
    let ctx = JobContext::with_identity("user-1", "actor-1", "mutate", "test");

    let remove = SkillRemoveHostTool::new(Arc::new(StubSkillHost));
    let output = remove
        .execute(serde_json::json!({ "name": "docs" }), &ctx)
        .await
        .unwrap();
    assert_eq!(output.result["status"], "removed");
    assert_eq!(output.result["name"], "docs");
    assert_eq!(
        remove.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved
    );

    let promote = SkillPromoteTrustHostTool::new(Arc::new(StubSkillHost));
    let output = promote
        .execute(
            serde_json::json!({
                "name": "docs",
                "target_trust": "trusted"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(output.result["status"], "updated");
    assert_eq!(output.result["name"], "docs");
    assert_eq!(output.result["trust"], "trusted");
    assert_eq!(output.result["source_tier"], "user");
    assert_eq!(
        promote.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved
    );
}

#[tokio::test]
async fn skill_audit_host_tool_preserves_existing_output_shape() {
    let ctx = JobContext::with_identity("user-1", "actor-1", "audit", "test");
    let tool = SkillAuditHostTool::new(Arc::new(StubSkillHost));

    let output = tool
        .execute(serde_json::json!({ "name": "docs" }), &ctx)
        .await
        .unwrap();

    assert_eq!(output.result["audited_count"], 1);
    assert_eq!(output.result["total_findings"], 1);
    assert_eq!(output.result["audited"][0]["name"], "docs");
    assert_eq!(output.result["audited"][0]["finding_count"], 1);
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Never
    );
}

#[tokio::test]
async fn skill_reload_host_tool_preserves_single_and_all_output_shapes() {
    let ctx = JobContext::with_identity("user-1", "actor-1", "reload", "test");
    let tool = SkillReloadHostTool::new(Arc::new(StubSkillHost));

    let single = tool
        .execute(serde_json::json!({ "name": "docs" }), &ctx)
        .await
        .unwrap();
    assert_eq!(single.result["status"], "reloaded");
    assert_eq!(single.result["name"], "docs");
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved
    );

    let all = tool
        .execute(serde_json::json!({ "all": true }), &ctx)
        .await
        .unwrap();
    assert_eq!(all.result["status"], "reloaded_all");
    assert_eq!(all.result["count"], 2);
    assert_eq!(all.result["skills"][0], "docs");
}

#[tokio::test]
async fn skill_read_host_tool_preserves_existing_output_shape() {
    let ctx = JobContext::with_identity("user-1", "actor-1", "read", "test");
    let tool = SkillReadHostTool::new(Arc::new(StubSkillHost));

    let output = tool
        .execute(serde_json::json!({ "name": "docs" }), &ctx)
        .await
        .unwrap();

    assert_eq!(output.result["name"], "docs");
    assert_eq!(output.result["version"], "1.0.0");
    assert_eq!(output.result["description"], "Documentation helper");
    assert_eq!(output.result["trust"], "trusted");
    assert_eq!(output.result["source_tier"], "user");
    assert_eq!(output.result["content"], "Use this skill for docs.");
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Never
    );
}

#[tokio::test]
async fn skill_inspect_host_tool_preserves_request_shape_and_restrictions() {
    let ctx = JobContext::with_identity("user-1", "actor-1", "inspect", "test");
    let tool = SkillInspectHostTool::new(Arc::new(StubSkillHost));
    let output = tool
        .execute(
            serde_json::json!({
                "name": "docs",
                "include_content": true,
                "include_files": false,
                "audit": false
            }),
            &ctx,
        )
        .await
        .unwrap();

    assert_eq!(output.result["name"], "docs");
    assert_eq!(output.result["include_content"], true);
    assert_eq!(output.result["include_files"], false);
    assert_eq!(output.result["audit"], false);

    let mut restricted = ctx.clone();
    restricted.metadata = serde_json::json!({ "allowed_skills": ["other"] });
    let err = tool
        .execute(serde_json::json!({ "name": "docs" }), &restricted)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not allowed"));
}

#[tokio::test]
async fn skill_publish_host_tool_preserves_existing_output_shape() {
    let ctx = JobContext::with_identity("user-1", "actor-1", "publish", "test");
    let tool = SkillPublishHostTool::new(Arc::new(StubSkillPublishHost));

    let output = tool
        .execute(
            serde_json::json!({
                "name": "docs",
                "target_repo": "owner/skills",
                "dry_run": true
            }),
            &ctx,
        )
        .await
        .unwrap();

    assert_eq!(output.result["status"], "dry_run");
    assert_eq!(output.result["name"], "docs");
    assert_eq!(output.result["target_repo"], "owner/skills");
    assert_eq!(output.result["package_path"], "community/docs");
    assert_eq!(output.result["file_count"], 1);
    assert_eq!(output.result["scanner_version"], "test");
    assert_eq!(
        output.result["remote_write_plan"]["pull_request"]["title"],
        "[skills] publish docs"
    );
    assert_eq!(
        tool.requires_approval(&serde_json::json!({"remote_write": true})),
        ApprovalRequirement::UnlessAutoApproved
    );
}

#[tokio::test]
async fn skill_tap_host_tools_preserve_existing_output_shapes() {
    let ctx = JobContext::with_identity("user-1", "actor-1", "tap", "test");

    let list = SkillTapListHostTool::new(stub_tap_host());
    let output = list
        .execute(serde_json::json!({ "include_health": true }), &ctx)
        .await
        .unwrap();
    assert_eq!(output.result["count"], 1);
    assert_eq!(output.result["hub_enabled"], true);
    assert_eq!(output.result["taps"][0]["trust_level"], "trusted");

    let add = SkillTapAddHostTool::new(stub_tap_host());
    let output = add
        .execute(
            serde_json::json!({
                "repo": "owner/skills",
                "path": "/packs/core/",
                "branch": "main",
                "trust_level": "trusted",
                "replace": true
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(output.result["status"], "replaced");
    assert_eq!(output.result["tap"]["path"], "packs/core");
    assert_eq!(
        add.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved
    );

    let remove = SkillTapRemoveHostTool::new(stub_tap_host());
    let output = remove
        .execute(
            serde_json::json!({
                "repo": "owner/skills",
                "path": "packs/core",
                "branch": "main"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(output.result["status"], "removed");
    assert_eq!(output.result["tap_count"], 0);

    let refresh = SkillTapRefreshHostTool::new(stub_tap_host());
    let output = refresh
        .execute(
            serde_json::json!({
                "repo": "owner/skills",
                "path": "packs/core"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(output.result["status"], "refreshed");
    assert_eq!(output.result["filter"]["repo"], "owner/skills");
    assert_eq!(output.result["hub_enabled"], true);
}

#[test]
fn repo_validation_requires_owner_name() {
    assert!(validate_github_repo("owner/repo").is_ok());
    assert!(validate_github_repo("owner/repo/extra").is_err());
    assert!(validate_github_repo("../repo").is_err());
    assert!(validate_github_repo("owner/").is_err());
}

#[test]
fn repo_relative_path_validation_rejects_traversal() {
    assert!(validate_repo_relative_path("skills/community", "path").is_ok());
    assert!(validate_repo_relative_path("", "path").is_ok());
    assert!(validate_repo_relative_path("../outside", "path").is_err());
    assert!(validate_repo_relative_path("skills/../outside", "path").is_err());
    assert!(validate_repo_path_component("skills", "path").is_ok());
    assert!(validate_repo_path_component("skills/community", "path").is_err());
}

#[test]
fn fetch_url_validation_blocks_internal_targets() {
    assert!(validate_fetch_url("https://example.com/skill.zip").is_ok());
    assert!(validate_fetch_url("http://example.com/skill.zip").is_err());
    assert!(validate_fetch_url("https://localhost/skill.zip").is_err());
    assert!(validate_fetch_url("https://127.0.0.1/skill.zip").is_err());
    assert!(validate_fetch_url("https://[::ffff:192.168.0.1]/skill.zip").is_err());
    assert!(validate_fetch_url("https://metadata.google.internal/skill.zip").is_err());
}

#[test]
fn extracts_stored_skill_from_zip() {
    let skill_md = b"---\nname: stored\n---\n# Stored\n";
    let mut zip = Vec::new();
    zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
    zip.extend_from_slice(&[0x0A, 0x00]);
    zip.extend_from_slice(&[0x00, 0x00]);
    zip.extend_from_slice(&[0x00, 0x00]);
    zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes());
    zip.extend_from_slice(&8u16.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(b"SKILL.md");
    zip.extend_from_slice(skill_md);

    assert_eq!(
        extract_skill_from_zip(&zip).unwrap(),
        "---\nname: stored\n---\n# Stored\n"
    );
}

#[test]
fn zip_extraction_requires_skill_md() {
    let mut zip = Vec::new();
    zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
    zip.extend_from_slice(&[0x0A, 0x00]);
    zip.extend_from_slice(&[0x00, 0x00]);
    zip.extend_from_slice(&[0x00, 0x00]);
    zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    zip.extend_from_slice(&2u32.to_le_bytes());
    zip.extend_from_slice(&2u32.to_le_bytes());
    zip.extend_from_slice(&10u16.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(b"_meta.json");
    zip.extend_from_slice(b"{}");

    assert!(
        extract_skill_from_zip(&zip)
            .unwrap_err()
            .to_string()
            .contains("does not contain SKILL.md")
    );
}
