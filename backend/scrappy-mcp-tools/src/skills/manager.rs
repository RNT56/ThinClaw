use super::manifest::SkillManifest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SkillError {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML Parse Error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("TOML Serialize Error: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("Skill not found: {0}")]
    NotFound(String),
    #[error("Invalid parameter: {0}")]
    InvalidParam(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedSkill {
    pub id: String, // Filename without extension
    pub manifest: SkillManifest,
    pub path: PathBuf,
}

pub struct SkillManager {
    // Priority order: user_path overrides builtin_path
    builtin_path: Option<PathBuf>,
    user_path: PathBuf,
}

impl SkillManager {
    /// Create a new SkillManager.
    /// `user_path` is where new skills are saved.
    /// `builtin_path` is an optional read-only directory for shipped skills.
    pub fn new<P: AsRef<Path>>(user_path: P, builtin_path: Option<P>) -> Self {
        let u_path = user_path.as_ref().to_path_buf();
        if !u_path.exists() {
            let _ = fs::create_dir_all(&u_path);
        }

        Self {
            user_path: u_path,
            builtin_path: builtin_path.map(|p| p.as_ref().to_path_buf()),
        }
    }

    /// List all available skills, aggregating from both paths.
    /// Skills in `user_path` with the same ID as `builtin_path` will override them.
    pub fn list_skills(&self) -> Result<Vec<LoadedSkill>, SkillError> {
        let mut skill_map: HashMap<String, LoadedSkill> = HashMap::new();

        // 1. Load built-in skills first
        if let Some(bp) = &self.builtin_path {
            if bp.exists() {
                self.scan_dir(bp, &mut skill_map)?;
            }
        }

        // 2. Load user skills (overriding built-ins)
        if self.user_path.exists() {
            self.scan_dir(&self.user_path, &mut skill_map)?;
        }

        Ok(skill_map.into_values().collect())
    }

    fn scan_dir(
        &self,
        dir: &Path,
        skill_map: &mut HashMap<String, LoadedSkill>,
    ) -> Result<(), SkillError> {
        // Handle case where dir doesn't exist or isn't a directory
        if !dir.exists() || !dir.is_dir() {
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Recursive scan
                self.scan_dir(&path, skill_map)?;
            } else if let Some(ext) = path.extension() {
                if ext == "toml" && path.to_string_lossy().ends_with(".skill.toml") {
                    // Try to read manifest
                    let content = match fs::read_to_string(&path) {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("Error reading skill file {:?}: {}", path, e);
                            continue;
                        }
                    };

                    match SkillManifest::from_toml(&content) {
                        Ok(manifest) => {
                            let file_stem = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("unknown")
                                .replace(".skill", ""); // remove .skill suffix

                            // Insert into map, possibly overwriting existing entry
                            skill_map.insert(
                                file_stem.clone(),
                                LoadedSkill {
                                    id: file_stem,
                                    manifest,
                                    path,
                                },
                            );
                        }
                        Err(e) => {
                            eprintln!("Failed to parse skill manifest {:?}: {}", path, e);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Load a specific skill by ID (name)
    pub fn get_skill(&self, skill_id: &str) -> Result<LoadedSkill, SkillError> {
        let skills = self.list_skills()?;
        skills
            .into_iter()
            .find(|s| s.id == skill_id)
            .ok_or_else(|| SkillError::NotFound(skill_id.to_string()))
    }

    /// Prepare a skill's script for execution by injecting parameter values
    pub fn prepare_script(
        &self,
        skill_id: &str,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<String, SkillError> {
        let skill = self.get_skill(skill_id)?;

        // Validate params
        let mut script_prefix = String::new();
        // params header
        script_prefix.push_str("// -- Injected Parameters --\n");

        for param_def in &skill.manifest.parameters {
            let value = params.get(&param_def.name).or(param_def.default.as_ref());

            if param_def.required && value.is_none() {
                return Err(SkillError::InvalidParam(format!(
                    "Missing required parameter: {}",
                    param_def.name
                )));
            }

            if let Some(val) = value {
                // Convert JSON value to Rhai literal
                let rhai_val = match val {
                    serde_json::Value::String(s) => format!("\"{}\"", s.replace("\"", "\\\"")),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    serde_json::Value::Null => "()".to_string(),
                    _ => serde_json::to_string(val).unwrap_or_else(|_| "()".to_string()),
                };
                script_prefix.push_str(&format!("const {} = {};\n", param_def.name, rhai_val));
            }
        }

        // Load script content
        let script_path = skill
            .path
            .parent()
            .unwrap()
            .join(&skill.manifest.script_file);

        if !script_path.exists() {
            return Err(SkillError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Script file not found: {:?}", script_path),
            )));
        }

        let script_content = fs::read_to_string(script_path)?;

        Ok(format!(
            "{}\n// -- Skill Script --\n{}",
            script_prefix, script_content
        ))
    }

    /// Save a new skill to the user directory
    pub fn save_skill(
        &self,
        id: &str,
        manifest: SkillManifest,
        script_content: &str,
    ) -> Result<(), SkillError> {
        let skill_dir = self.user_path.clone();
        if !skill_dir.exists() {
            fs::create_dir_all(&skill_dir)?;
        }

        let manifest_path = skill_dir.join(format!("{}.skill.toml", id));
        let script_path = skill_dir.join(&manifest.script_file);

        fs::write(&manifest_path, manifest.to_toml()?)?;
        fs::write(&script_path, script_content)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::manifest::SkillManifest;
    use tempfile::tempdir;

    #[test]
    fn test_save_and_retrieve_skill() {
        let dir = tempdir().unwrap();
        let user_path = dir.path().to_path_buf();
        let manager = SkillManager::new(user_path.clone(), None);

        let id = "test_skill";
        let script_content = r#"print("Hello");"#;
        let manifest = SkillManifest {
            name: "Test Skill".to_string(),
            description: "A test skill".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            tools_used: vec![],
            parameters: vec![],
            script_file: "test_skill.rhai".to_string(),
        };

        // 1. Save
        manager
            .save_skill(id, manifest.clone(), script_content)
            .expect("Failed to save skill");

        // 2. Verify files exist
        assert!(user_path.join("test_skill.skill.toml").exists());
        assert!(user_path.join("test_skill.rhai").exists());

        // 3. Retrieve
        let skill = manager.get_skill(id).expect("Failed to get skill");
        assert_eq!(skill.manifest.name, "Test Skill");

        let args = std::collections::HashMap::new();
        let prep = manager
            .prepare_script(id, &args)
            .expect("Failed to prepare");
        assert!(prep.contains(script_content));
    }
}
