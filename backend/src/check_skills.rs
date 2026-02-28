// Test module removed — the old McpRequestHandler import (ipc.rs) was deleted in Phase 4.

#[cfg(test)]
mod skill_tests {
    use scrappy_mcp_tools::skills::manager::SkillManager;
    use scrappy_mcp_tools::skills::manifest::SkillManifest;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_skill_manager_lifecycle() {
        let user_dir = tempdir().unwrap();
        let built_in_dir = tempdir().unwrap();

        let manager = SkillManager::new(user_dir.path(), Some(built_in_dir.path()));

        // 1. Create a dummy built-in skill
        let manifest = r#"
            name = "Test Skill"
            description = "A test skill"
            version = "1.0.0"
            tools_used = []
            script_file = "test.rhai"
            [[parameters]]
            name = "arg1"
            description = "An argument"
            param_type = "string"
            required = true
        "#;
        let script = r#"print("Hello " + arg1);"#;

        let skill_path = built_in_dir.path().join("test_skill.skill.toml");
        fs::write(&skill_path, manifest).unwrap();
        fs::write(built_in_dir.path().join("test.rhai"), script).unwrap();

        // 2. List skills
        let skills = manager.list_skills().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "test_skill");

        // 3. Prepare script
        let mut params = std::collections::HashMap::new();
        params.insert(
            "arg1".to_string(),
            serde_json::Value::String("World".to_string()),
        );

        let prepared = manager.prepare_script("test_skill", &params).unwrap();

        println!("Prepared script:\n{}", prepared);
        assert!(prepared.contains(r#"const arg1 = "World";"#));
        assert!(prepared.contains(r#"print("Hello " + arg1);"#));
    }
}
