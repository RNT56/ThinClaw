use super::*;
use std::fs;

fn write_managed_package_lock(skill_dir: &Path, package_files: Vec<String>) {
    let provenance = crate::skills::quarantine::SkillProvenance {
        source_kind: "git".to_string(),
        source_adapter: "test".to_string(),
        source_ref: "github.com/acme/test".to_string(),
        source_repo: Some("github.com/acme/test".to_string()),
        source_url: Some("https://github.com/acme/test.git".to_string()),
        manifest_url: None,
        manifest_digest: Some("sha256:test".to_string()),
        path: None,
        branch: None,
        commit_sha: Some("0123456789012345678901234567890123456789".to_string()),
        trust_level: crate::settings::SkillTapTrustLevel::Community,
        downloaded_at: "2026-01-01T00:00:00Z".to_string(),
        findings: Vec::new(),
        scanner_version: None,
        content_sha256: Some("sha256:test".to_string()),
        finding_summary: None,
        package_files,
    };
    fs::write(
        skill_dir.join(".thinclaw-skill-lock.json"),
        serde_json::to_vec(&provenance).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn test_discover_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    let loaded = registry.discover_all().await;
    assert!(loaded.is_empty());
}

#[tokio::test]
async fn test_discover_nonexistent_dir() {
    let mut registry = SkillRegistry::new(PathBuf::from("/nonexistent/skills"));
    let loaded = registry.discover_all().await;
    assert!(loaded.is_empty());
}

#[tokio::test]
async fn test_load_subdirectory_layout() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("test-skill");
    fs::create_dir(&skill_dir).unwrap();

    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: test-skill\ndescription: A test skill\nactivation:\n  keywords: [\"test\"]\n---\n\nYou are a helpful test assistant.\n",
    ).unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    let loaded = registry.discover_all().await;

    assert_eq!(loaded, vec!["test-skill"]);
    assert_eq!(registry.count(), 1);

    let skill = &registry.skills()[0];
    assert_eq!(skill.trust, SkillTrust::Trusted);
    assert!(skill.prompt_content.contains("helpful test assistant"));
}

#[tokio::test]
async fn test_workspace_overrides_user() {
    let user_dir = tempfile::tempdir().unwrap();
    let ws_dir = tempfile::tempdir().unwrap();

    // Create skill in user dir
    let user_skill = user_dir.path().join("my-skill");
    fs::create_dir(&user_skill).unwrap();
    fs::write(
        user_skill.join("SKILL.md"),
        "---\nname: my-skill\n---\n\nUser version.\n",
    )
    .unwrap();

    // Create same-named skill in workspace dir
    let ws_skill = ws_dir.path().join("my-skill");
    fs::create_dir(&ws_skill).unwrap();
    fs::write(
        ws_skill.join("SKILL.md"),
        "---\nname: my-skill\n---\n\nWorkspace version.\n",
    )
    .unwrap();

    let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
        .with_workspace_dir(ws_dir.path().to_path_buf());
    let loaded = registry.discover_all().await;

    assert_eq!(loaded, vec!["my-skill"]);
    assert_eq!(registry.count(), 1);
    assert!(registry.skills()[0].prompt_content.contains("Workspace"));
}

#[tokio::test]
async fn test_gating_failure_skips_skill() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("gated-skill");
    fs::create_dir(&skill_dir).unwrap();

    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: gated-skill\nmetadata:\n  openclaw:\n    requires:\n      bins: [\"__nonexistent_bin__\"]\n---\n\nGated prompt.\n",
    ).unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    let loaded = registry.discover_all().await;
    assert!(loaded.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn test_symlink_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let real_dir = dir.path().join("real-skill");
    fs::create_dir(&real_dir).unwrap();
    fs::write(
        real_dir.join("SKILL.md"),
        "---\nname: real-skill\n---\n\nTest.\n",
    )
    .unwrap();

    let skills_dir = dir.path().join("skills");
    fs::create_dir(&skills_dir).unwrap();
    std::os::unix::fs::symlink(&real_dir, skills_dir.join("linked-skill")).unwrap();

    let mut registry = SkillRegistry::new(skills_dir);
    let loaded = registry.discover_all().await;
    assert!(loaded.is_empty());
}

#[tokio::test]
async fn test_file_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("big-skill");
    fs::create_dir(&skill_dir).unwrap();

    let big_content = format!(
        "---\nname: big-skill\n---\n\n{}",
        "x".repeat((MAX_PROMPT_FILE_SIZE + 1) as usize)
    );
    fs::write(skill_dir.join("SKILL.md"), &big_content).unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    let loaded = registry.discover_all().await;
    assert!(loaded.is_empty());
}

#[tokio::test]
async fn test_invalid_skill_md_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("bad-skill");
    fs::create_dir(&skill_dir).unwrap();

    // Missing frontmatter
    fs::write(skill_dir.join("SKILL.md"), "Just plain text").unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    let loaded = registry.discover_all().await;
    assert!(loaded.is_empty());
}

#[tokio::test]
async fn test_validate_skill_file_does_not_mutate_registry() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("checked-skill");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: checked-skill\n---\n\nChecked prompt.\n",
    )
    .unwrap();

    let registry = SkillRegistry::new(dir.path().to_path_buf());
    let (name, loaded) = SkillRegistry::validate_skill_file(
        &skill_dir,
        SkillTrust::Installed,
        SkillSource::External(skill_dir.clone()),
    )
    .await
    .unwrap();

    assert_eq!(name, "checked-skill");
    assert_eq!(loaded.trust, SkillTrust::Installed);
    assert_eq!(registry.count(), 0);
}

#[tokio::test]
async fn test_validate_skill_content_reuses_token_budget_rules() {
    let big_prompt = "word ".repeat(4000);
    let content = format!(
        "---\nname: content-budget\nactivation:\n  max_context_tokens: 100\n---\n\n{}",
        big_prompt
    );

    let result = SkillRegistry::validate_skill_content(
        &content,
        SkillTrust::Installed,
        SkillSource::External(PathBuf::from(".")),
    )
    .await;

    assert!(matches!(
        result,
        Err(SkillRegistryError::TokenBudgetExceeded { .. })
    ));
}

#[tokio::test]
async fn test_line_ending_normalization() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("crlf-skill");
    fs::create_dir(&skill_dir).unwrap();

    fs::write(
        skill_dir.join("SKILL.md"),
        "---\r\nname: crlf-skill\r\n---\r\n\r\nline1\r\nline2\r\n",
    )
    .unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.discover_all().await;

    assert_eq!(registry.count(), 1);
    let skill = &registry.skills()[0];
    assert_eq!(skill.prompt_content, "line1\nline2\n");
}

#[tokio::test]
async fn test_token_budget_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("big-prompt");
    fs::create_dir(&skill_dir).unwrap();

    let big_prompt = "word ".repeat(4000);
    let content = format!(
        "---\nname: big-prompt\nactivation:\n  max_context_tokens: 100\n---\n\n{}",
        big_prompt
    );
    fs::write(skill_dir.join("SKILL.md"), &content).unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    let loaded = registry.discover_all().await;
    assert!(loaded.is_empty());
}

#[tokio::test]
async fn test_has_and_find_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("my-skill");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: my-skill\n---\n\nPrompt.\n",
    )
    .unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.discover_all().await;

    assert!(registry.has("my-skill"));
    assert!(!registry.has("nonexistent"));
    assert!(registry.find_by_name("my-skill").is_some());
    assert!(registry.find_by_name("nonexistent").is_none());
}

#[tokio::test]
async fn test_install_skill_from_content() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());

    let content =
        "---\nname: test-install\ndescription: Installed skill\n---\n\nInstalled prompt.\n";
    let name = registry.install_skill(content).await.unwrap();

    assert_eq!(name, "test-install");
    assert!(registry.has("test-install"));
    assert_eq!(registry.count(), 1);

    // Verify file was written to disk
    let skill_path = dir.path().join("test-install").join("SKILL.md");
    assert!(skill_path.exists());
}

#[tokio::test]
async fn test_install_duplicate_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());

    let content = "---\nname: dup-skill\n---\n\nPrompt.\n";
    registry.install_skill(content).await.unwrap();

    let result = registry.install_skill(content).await;
    assert!(matches!(
        result,
        Err(SkillRegistryError::AlreadyExists { .. })
    ));
}

#[tokio::test]
async fn test_remove_user_skill() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());

    let content = "---\nname: removable\n---\n\nPrompt.\n";
    registry.install_skill(content).await.unwrap();
    assert!(registry.has("removable"));

    registry.remove_skill("removable").await.unwrap();
    assert!(!registry.has("removable"));
    assert_eq!(registry.count(), 0);
}

#[tokio::test]
async fn test_remove_managed_package_deletes_support_files() {
    let user_dir = tempfile::tempdir().unwrap();
    let installed_dir = tempfile::tempdir().unwrap();
    let skill_dir = installed_dir.path().join("packaged-skill");
    fs::create_dir_all(skill_dir.join("references")).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: packaged-skill\n---\n\nPrompt.\n",
    )
    .unwrap();
    fs::write(skill_dir.join("references/guide.md"), "Guide").unwrap();
    write_managed_package_lock(
        &skill_dir,
        vec!["SKILL.md".to_string(), "references/guide.md".to_string()],
    );

    let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
        .with_installed_dir(installed_dir.path().to_path_buf());
    registry.discover_all().await;
    registry.remove_skill("packaged-skill").await.unwrap();

    assert!(!skill_dir.exists());
    assert!(!registry.has("packaged-skill"));
}

#[tokio::test]
async fn test_promote_trust_moves_entire_package_atomically() {
    let user_dir = tempfile::tempdir().unwrap();
    let installed_dir = tempfile::tempdir().unwrap();
    let old_dir = installed_dir.path().join("movable-skill");
    fs::create_dir_all(old_dir.join("scripts")).unwrap();
    fs::write(
        old_dir.join("SKILL.md"),
        "---\nname: movable-skill\n---\n\nPrompt.\n",
    )
    .unwrap();
    fs::write(old_dir.join("scripts/helper.txt"), "support").unwrap();

    let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
        .with_installed_dir(installed_dir.path().to_path_buf());
    registry.discover_all().await;
    registry
        .promote_trust("movable-skill", SkillTrust::Trusted)
        .await
        .unwrap();

    let new_dir = user_dir.path().join("movable-skill");
    assert!(!old_dir.exists());
    assert_eq!(
        fs::read_to_string(new_dir.join("scripts/helper.txt")).unwrap(),
        "support"
    );
    let moved = registry.find_by_name("movable-skill").unwrap();
    assert_eq!(moved.trust, SkillTrust::Trusted);
    assert_eq!(moved.source, SkillSource::User(new_dir));
}

#[tokio::test]
async fn test_remove_workspace_skill_rejected() {
    let user_dir = tempfile::tempdir().unwrap();
    let ws_dir = tempfile::tempdir().unwrap();

    let ws_skill = ws_dir.path().join("ws-skill");
    fs::create_dir(&ws_skill).unwrap();
    fs::write(
        ws_skill.join("SKILL.md"),
        "---\nname: ws-skill\n---\n\nWorkspace prompt.\n",
    )
    .unwrap();

    let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
        .with_workspace_dir(ws_dir.path().to_path_buf());
    registry.discover_all().await;

    let result = registry.remove_skill("ws-skill").await;
    assert!(matches!(
        result,
        Err(SkillRegistryError::CannotRemove { .. })
    ));
}

#[tokio::test]
async fn test_remove_nonexistent_fails() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = SkillRegistry::new(dir.path().to_path_buf());

    let result = registry.remove_skill("nonexistent").await;
    assert!(matches!(result, Err(SkillRegistryError::NotFound(_))));
}

#[tokio::test]
async fn test_reload_clears_and_rediscovers() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("persist-skill");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: persist-skill\n---\n\nPrompt.\n",
    )
    .unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.discover_all().await;
    assert_eq!(registry.count(), 1);

    let loaded = registry.reload().await;
    assert_eq!(loaded, vec!["persist-skill"]);
    assert_eq!(registry.count(), 1);
}

#[tokio::test]
async fn test_clone_config_supports_load_then_swap() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("swap-skill");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: swap-skill\n---\n\nPrompt.\n",
    )
    .unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.discover_all().await;
    assert_eq!(registry.count(), 1);

    // A config-clone starts empty but rediscovers the same skills off-lock,
    // so it can be swapped in for the live registry without a torn state.
    let mut fresh = registry.clone_config();
    assert_eq!(fresh.count(), 0);
    let loaded = fresh.discover_all().await;
    assert_eq!(loaded, vec!["swap-skill"]);
    assert_eq!(fresh.count(), 1);
}

#[tokio::test]
async fn test_load_flat_layout() {
    let dir = tempfile::tempdir().unwrap();

    // Place a SKILL.md directly in the skills directory (flat layout)
    fs::write(
        dir.path().join("SKILL.md"),
        "---\nname: flat-skill\ndescription: A flat layout skill\nactivation:\n  keywords: [\"flat\"]\n---\n\nYou are a flat layout test skill.\n",
    ).unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    let loaded = registry.discover_all().await;

    assert_eq!(loaded, vec!["flat-skill"]);
    assert_eq!(registry.count(), 1);

    let skill = &registry.skills()[0];
    assert_eq!(skill.trust, SkillTrust::Trusted);
    assert!(skill.prompt_content.contains("flat layout test skill"));
}

#[tokio::test]
async fn test_mixed_flat_and_subdirectory_layout() {
    let dir = tempfile::tempdir().unwrap();

    // Flat layout: SKILL.md directly in the skills directory
    fs::write(
        dir.path().join("SKILL.md"),
        "---\nname: flat-skill\n---\n\nFlat prompt.\n",
    )
    .unwrap();

    // Subdirectory layout: <name>/SKILL.md
    let sub_dir = dir.path().join("sub-skill");
    fs::create_dir(&sub_dir).unwrap();
    fs::write(
        sub_dir.join("SKILL.md"),
        "---\nname: sub-skill\n---\n\nSub prompt.\n",
    )
    .unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    let loaded = registry.discover_all().await;

    assert_eq!(registry.count(), 2);
    assert!(loaded.contains(&"flat-skill".to_string()));
    assert!(loaded.contains(&"sub-skill".to_string()));
}

#[tokio::test]
async fn test_lowercased_fields_populated() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("case-skill");
    fs::create_dir(&skill_dir).unwrap();

    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: case-skill\nactivation:\n  keywords: [\"Write\", \"EDIT\"]\n  tags: [\"Email\", \"PROSE\"]\n---\n\nTest prompt.\n",
    ).unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.discover_all().await;

    let skill = registry.find_by_name("case-skill").unwrap();
    assert_eq!(skill.lowercased_keywords, vec!["write", "edit"]);
    assert_eq!(skill.lowercased_tags, vec!["email", "prose"]);
}

#[test]
fn test_compute_hash_deterministic() {
    let h1 = compute_hash("hello world");
    let h2 = compute_hash("hello world");
    assert_eq!(h1, h2);
    assert!(h1.starts_with("sha256:"));
}

#[test]
fn test_compute_hash_different_content() {
    let h1 = compute_hash("hello");
    let h2 = compute_hash("world");
    assert_ne!(h1, h2);
}

/// Skills in the installed_dir are discovered with SkillTrust::Installed,
/// not Trusted. This ensures registry-installed skills do not gain full
/// tool access after an agent restart.
#[tokio::test]
async fn test_installed_dir_uses_installed_trust() {
    let user_dir = tempfile::tempdir().unwrap();
    let inst_dir = tempfile::tempdir().unwrap();

    // Place a skill in the installed dir
    let skill_dir = inst_dir.path().join("registry-skill");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: registry-skill\nversion: \"1.2.3\"\n---\n\nInstalled prompt.\n",
    )
    .unwrap();

    let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
        .with_installed_dir(inst_dir.path().to_path_buf());
    let loaded = registry.discover_all().await;

    assert_eq!(loaded, vec!["registry-skill"]);
    let skill = registry.find_by_name("registry-skill").unwrap();
    assert_eq!(
        skill.trust,
        SkillTrust::Installed,
        "installed_dir skills must be Installed"
    );
    assert_eq!(skill.manifest.version, "1.2.3");
}

/// install_target_dir() returns installed_dir when set, user_dir otherwise.
#[test]
fn test_install_target_dir_prefers_installed_dir() {
    let user_dir = PathBuf::from("/tmp/user-skills");
    let inst_dir = PathBuf::from("/tmp/installed-skills");

    let registry = SkillRegistry::new(user_dir.clone()).with_installed_dir(inst_dir.clone());
    assert_eq!(registry.install_target_dir(), inst_dir.as_path());

    let registry_no_inst = SkillRegistry::new(user_dir.clone());
    assert_eq!(registry_no_inst.install_target_dir(), user_dir.as_path());
}

/// User skills (user_dir) remain Trusted even when installed_dir is set.
#[tokio::test]
async fn test_user_dir_stays_trusted_with_installed_dir() {
    let user_dir = tempfile::tempdir().unwrap();
    let inst_dir = tempfile::tempdir().unwrap();

    let skill_dir = user_dir.path().join("my-skill");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: my-skill\n---\n\nUser prompt.\n",
    )
    .unwrap();

    let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
        .with_installed_dir(inst_dir.path().to_path_buf());
    registry.discover_all().await;

    let skill = registry.find_by_name("my-skill").unwrap();
    assert_eq!(skill.trust, SkillTrust::Trusted);
}

// ── Path traversal protection tests ────────────────────────────────

#[tokio::test]
async fn test_install_rejects_path_traversal() {
    let dir = tempfile::tempdir().unwrap();

    let result = SkillRegistry::prepare_install_to_disk(
        dir.path(),
        "../escape",
        "---\nname: escape\n---\n\nEvil.\n",
    )
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Invalid skill name"));
}

#[tokio::test]
async fn test_install_rejects_slash_in_name() {
    let dir = tempfile::tempdir().unwrap();

    let result = SkillRegistry::prepare_install_to_disk(
        dir.path(),
        "foo/bar",
        "---\nname: foo-bar\n---\n\nTest.\n",
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_install_rejects_hidden_name() {
    let dir = tempfile::tempdir().unwrap();

    let result = SkillRegistry::prepare_install_to_disk(
        dir.path(),
        ".hidden",
        "---\nname: hidden\n---\n\nTest.\n",
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_install_rejects_empty_name() {
    let dir = tempfile::tempdir().unwrap();

    let result =
        SkillRegistry::prepare_install_to_disk(dir.path(), "", "---\nname: x\n---\n\nTest.\n")
            .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn prepare_install_atomically_replaces_existing_skill_directory() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("replace-me");
    fs::create_dir_all(skill_dir.join("references")).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: replace-me\n---\n\nOld prompt.\n",
    )
    .unwrap();
    fs::write(skill_dir.join("references/stale.md"), "stale").unwrap();

    let (_, loaded) = SkillRegistry::prepare_install_to_disk(
        dir.path(),
        "replace-me",
        "---\nname: replace-me\n---\n\nNew prompt.\n",
    )
    .await
    .unwrap();

    assert_eq!(loaded.prompt_content, "New prompt.\n");
    assert_eq!(
        fs::read_to_string(skill_dir.join("SKILL.md")).unwrap(),
        "---\nname: replace-me\n---\n\nNew prompt.\n"
    );
    assert!(!skill_dir.join("references/stale.md").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_install_rejects_planted_target_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let victim_dir = tempfile::tempdir().unwrap();
    let victim = victim_dir.path().join("SKILL.md");
    fs::write(&victim, "keep-me").unwrap();
    symlink(victim_dir.path(), dir.path().join("linked-skill")).unwrap();

    let result = SkillRegistry::prepare_install_to_disk(
        dir.path(),
        "linked-skill",
        "---\nname: linked-skill\n---\n\nNew prompt.\n",
    )
    .await;

    assert!(result.is_err());
    assert_eq!(fs::read_to_string(victim).unwrap(), "keep-me");
}
