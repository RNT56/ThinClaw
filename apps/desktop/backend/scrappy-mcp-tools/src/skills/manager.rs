use super::manifest::SkillManifest;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const MAX_SKILLS: usize = 1_024;
const MAX_SKILL_SCAN_DEPTH: usize = 4;
const MAX_SKILL_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_SKILL_SCRIPT_BYTES: u64 = 1024 * 1024;
const MAX_SKILL_PARAMETERS: usize = 64;
const MAX_SKILL_SCAN_ENTRIES: usize = 8_192;
const MAX_PARAMETER_VALUE_BYTES: usize = 64 * 1024;
const MAX_TOTAL_PARAMETER_VALUE_BYTES: usize = 256 * 1024;

fn has_disallowed_control(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn valid_skill_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_rhai_identifier(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !matches!(
            value,
            "as" | "break"
                | "catch"
                | "const"
                | "continue"
                | "do"
                | "else"
                | "export"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "import"
                | "in"
                | "let"
                | "loop"
                | "private"
                | "return"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "until"
                | "while"
        )
}

fn value_matches_parameter_type(value: &serde_json::Value, parameter_type: &str) -> bool {
    match parameter_type {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    }
}

fn validate_manifest(id: &str, manifest: &SkillManifest) -> Result<(), SkillError> {
    if !valid_skill_id(id)
        || manifest.name.trim().is_empty()
        || manifest.name.len() > 256
        || manifest.name.chars().any(char::is_control)
        || manifest.description.len() > 16 * 1024
        || has_disallowed_control(&manifest.description)
        || manifest.version.is_empty()
        || manifest.version.len() > 128
        || manifest.version.chars().any(char::is_control)
        || manifest
            .author
            .as_ref()
            .is_some_and(|author| author.len() > 256 || has_disallowed_control(author))
        || manifest.tools_used.len() > 128
        || manifest.tools_used.iter().any(|tool| {
            tool.trim() != tool
                || tool.is_empty()
                || tool.len() > 256
                || has_disallowed_control(tool)
        })
        || manifest.parameters.len() > MAX_SKILL_PARAMETERS
        || manifest.script_file != format!("{id}.rhai")
    {
        return Err(SkillError::InvalidParam(
            "skill manifest is malformed or oversized".to_string(),
        ));
    }
    let mut names = HashSet::new();
    let mut total_default_bytes = 0usize;
    for parameter in &manifest.parameters {
        let default_size = parameter
            .default
            .as_ref()
            .and_then(|value| serde_json::to_vec(value).ok())
            .map(|encoded| encoded.len());
        if !valid_rhai_identifier(&parameter.name)
            || !names.insert(&parameter.name)
            || parameter.description.len() > 4 * 1024
            || has_disallowed_control(&parameter.description)
            || !matches!(
                parameter.param_type.as_str(),
                "string" | "number" | "boolean" | "array" | "object"
            )
            || parameter
                .default
                .as_ref()
                .is_some_and(|value| !value_matches_parameter_type(value, &parameter.param_type))
            || default_size.is_none() && parameter.default.is_some()
            || default_size.is_some_and(|size| size > MAX_PARAMETER_VALUE_BYTES)
        {
            return Err(SkillError::InvalidParam(
                "skill parameter metadata is malformed or oversized".to_string(),
            ));
        }
        total_default_bytes = total_default_bytes.saturating_add(default_size.unwrap_or_default());
        if total_default_bytes > MAX_TOTAL_PARAMETER_VALUE_BYTES {
            return Err(SkillError::InvalidParam(
                "skill parameter defaults exceed the aggregate size limit".to_string(),
            ));
        }
    }
    Ok(())
}

fn ensure_real_directory(path: &Path) -> Result<PathBuf, SkillError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(SkillError::InvalidParam(
                "skills directory is not a real directory".to_string(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            let metadata = fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(SkillError::InvalidParam(
                    "skills directory is not a real directory".to_string(),
                ));
            }
        }
        Err(error) => return Err(error.into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(path.canonicalize()?)
}

fn existing_real_directory(path: &Path) -> Result<Option<PathBuf>, SkillError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            Ok(Some(path.canonicalize()?))
        }
        Ok(_) => Err(SkillError::InvalidParam(
            "built-in skills path is not a real directory".to_string(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

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
        Self {
            user_path: user_path.as_ref().to_path_buf(),
            builtin_path: builtin_path.map(|p| p.as_ref().to_path_buf()),
        }
    }

    /// List all available skills, aggregating from both paths.
    /// Skills in `user_path` with the same ID as `builtin_path` will override them.
    pub fn list_skills(&self) -> Result<Vec<LoadedSkill>, SkillError> {
        let mut skill_map: HashMap<String, LoadedSkill> = HashMap::new();
        let mut scanned_entries = 0usize;

        // 1. Load built-in skills first
        if let Some(bp) = &self.builtin_path {
            if let Some(root) = existing_real_directory(bp)? {
                Self::scan_dir(&root, &root, 0, &mut scanned_entries, &mut skill_map)?;
            }
        }

        // 2. Load user skills (overriding built-ins)
        let user_root = ensure_real_directory(&self.user_path)?;
        Self::scan_dir(
            &user_root,
            &user_root,
            0,
            &mut scanned_entries,
            &mut skill_map,
        )?;

        let mut skills: Vec<_> = skill_map.into_values().collect();
        skills.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(skills)
    }

    fn scan_dir(
        root: &Path,
        dir: &Path,
        depth: usize,
        scanned_entries: &mut usize,
        skill_map: &mut HashMap<String, LoadedSkill>,
    ) -> Result<(), SkillError> {
        if depth > MAX_SKILL_SCAN_DEPTH {
            return Err(SkillError::InvalidParam(
                "skills directory exceeds the scan depth limit".to_string(),
            ));
        }
        let canonical_dir = dir.canonicalize()?;
        if !canonical_dir.starts_with(root) {
            return Err(SkillError::InvalidParam(
                "skills directory escapes its configured root".to_string(),
            ));
        }

        for entry in fs::read_dir(&canonical_dir)? {
            let entry = entry?;
            let path = entry.path();
            *scanned_entries = scanned_entries.saturating_add(1);
            if *scanned_entries > MAX_SKILL_SCAN_ENTRIES {
                return Err(SkillError::InvalidParam(
                    "skills directory exceeds the entry limit".to_string(),
                ));
            }
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }

            if metadata.is_dir() {
                if depth < MAX_SKILL_SCAN_DEPTH {
                    Self::scan_dir(root, &path, depth + 1, scanned_entries, skill_map)?;
                }
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(id) = filename.strip_suffix(".skill.toml") else {
                continue;
            };
            if !valid_skill_id(id) {
                continue;
            }
            if skill_map.len() >= MAX_SKILLS && !skill_map.contains_key(id) {
                return Err(SkillError::InvalidParam(
                    "skills directory exceeds the skill limit".to_string(),
                ));
            }
            let bytes = match thinclaw_platform::read_regular_file_bounded_single_link(
                &path,
                MAX_SKILL_MANIFEST_BYTES,
            ) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let content = match std::str::from_utf8(&bytes) {
                Ok(content) => content,
                Err(_) => continue,
            };
            let manifest = match SkillManifest::from_toml(content) {
                Ok(manifest) if validate_manifest(id, &manifest).is_ok() => manifest,
                _ => continue,
            };
            skill_map.insert(
                id.to_string(),
                LoadedSkill {
                    id: id.to_string(),
                    manifest,
                    path,
                },
            );
        }
        Ok(())
    }

    /// Load a specific skill by ID (name)
    pub fn get_skill(&self, skill_id: &str) -> Result<LoadedSkill, SkillError> {
        if !valid_skill_id(skill_id) {
            return Err(SkillError::InvalidParam("skill ID is invalid".to_string()));
        }
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
        if params.len() > MAX_SKILL_PARAMETERS {
            return Err(SkillError::InvalidParam(
                "too many skill parameters".to_string(),
            ));
        }

        let parent = skill.path.parent().ok_or_else(|| {
            SkillError::InvalidParam("skill manifest has no parent directory".to_string())
        })?;
        let script_path = parent.join(format!("{skill_id}.rhai"));
        let user_root = ensure_real_directory(&self.user_path)?;
        let _read_guard = if skill.path.starts_with(&user_root) {
            Some(thinclaw_platform::acquire_artifact_read_lock_sync(
                &script_path,
            )?)
        } else {
            None
        };
        let manifest_bytes = thinclaw_platform::read_regular_file_bounded_single_link(
            &skill.path,
            MAX_SKILL_MANIFEST_BYTES,
        )?;
        let manifest_text = std::str::from_utf8(&manifest_bytes).map_err(|_| {
            SkillError::InvalidParam("skill manifest is not valid UTF-8".to_string())
        })?;
        let manifest = SkillManifest::from_toml(manifest_text)?;
        validate_manifest(skill_id, &manifest)?;
        let script_bytes = thinclaw_platform::read_regular_file_bounded_single_link(
            &script_path,
            MAX_SKILL_SCRIPT_BYTES,
        )?;
        let script_content = std::str::from_utf8(&script_bytes)
            .map_err(|_| SkillError::InvalidParam("skill script is not valid UTF-8".to_string()))?;
        if script_content.contains('\0') {
            return Err(SkillError::InvalidParam(
                "skill script contains a NUL byte".to_string(),
            ));
        }

        let definitions: HashMap<_, _> = manifest
            .parameters
            .iter()
            .map(|parameter| (parameter.name.as_str(), parameter))
            .collect();
        for (name, value) in params {
            let definition = definitions.get(name.as_str()).ok_or_else(|| {
                SkillError::InvalidParam(format!("Unknown skill parameter: {name}"))
            })?;
            if !value_matches_parameter_type(value, &definition.param_type) {
                return Err(SkillError::InvalidParam(format!(
                    "Parameter {name} does not match type {}",
                    definition.param_type
                )));
            }
        }

        // Validate params
        let mut script_prefix = String::new();
        // params header
        script_prefix.push_str("// -- Injected Parameters --\n");

        let mut total_value_bytes = 0usize;
        for param_def in &manifest.parameters {
            let value = params.get(&param_def.name).or(param_def.default.as_ref());

            if param_def.required && value.is_none() {
                return Err(SkillError::InvalidParam(format!(
                    "Missing required parameter: {}",
                    param_def.name
                )));
            }

            if let Some(value) = value {
                if !value_matches_parameter_type(value, &param_def.param_type) {
                    return Err(SkillError::InvalidParam(format!(
                        "Parameter {} does not match type {}",
                        param_def.name, param_def.param_type
                    )));
                }
                let json = serde_json::to_string(value).map_err(|_| {
                    SkillError::InvalidParam(format!(
                        "Parameter {} cannot be encoded",
                        param_def.name
                    ))
                })?;
                if json.len() > MAX_PARAMETER_VALUE_BYTES {
                    return Err(SkillError::InvalidParam(format!(
                        "Parameter {} exceeds the size limit",
                        param_def.name
                    )));
                }
                total_value_bytes = total_value_bytes.saturating_add(json.len());
                if total_value_bytes > MAX_TOTAL_PARAMETER_VALUE_BYTES {
                    return Err(SkillError::InvalidParam(
                        "skill parameters exceed the aggregate size limit".to_string(),
                    ));
                }
                let quoted_json = serde_json::to_string(&json).map_err(|_| {
                    SkillError::InvalidParam(format!(
                        "Parameter {} cannot be quoted",
                        param_def.name
                    ))
                })?;
                script_prefix.push_str(&format!(
                    "const {} = parse_json({});\n",
                    param_def.name, quoted_json
                ));
            } else {
                script_prefix.push_str(&format!("const {} = ();\n", param_def.name));
            }
        }

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
        if !valid_skill_id(id) {
            return Err(SkillError::InvalidParam("skill ID is invalid".to_string()));
        }
        validate_manifest(id, &manifest)?;
        if script_content.len() > MAX_SKILL_SCRIPT_BYTES as usize || script_content.contains('\0') {
            return Err(SkillError::InvalidParam(
                "skill script is malformed or oversized".to_string(),
            ));
        }
        let manifest_content = manifest.to_toml()?;
        if manifest_content.len() > MAX_SKILL_MANIFEST_BYTES as usize {
            return Err(SkillError::InvalidParam(
                "skill manifest exceeds the size limit".to_string(),
            ));
        }

        let skill_dir = ensure_real_directory(&self.user_path)?;
        let manifest_path = skill_dir.join(format!("{id}.skill.toml"));
        let script_path = skill_dir.join(format!("{id}.rhai"));

        thinclaw_platform::publish_file_pair_sync(
            &script_path,
            &manifest_path,
            script_content.as_bytes(),
            Some(manifest_content.as_bytes()),
            thinclaw_platform::ExistingPairPolicy::Replace,
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NullReporter;
    use crate::sandbox::{Sandbox, SandboxConfig};
    use crate::skills::manifest::{SkillManifest, SkillParameter};
    use std::sync::Arc;
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

    fn parameterized_manifest(id: &str) -> SkillManifest {
        SkillManifest {
            name: "Parameterized Skill".to_string(),
            description: "Tests safe parameter handling".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            tools_used: vec![],
            parameters: vec![SkillParameter {
                name: "payload".to_string(),
                description: "Untrusted input".to_string(),
                param_type: "string".to_string(),
                required: true,
                default: None,
            }],
            script_file: format!("{id}.rhai"),
        }
    }

    #[test]
    fn rejects_traversal_ids_and_script_paths() {
        let dir = tempdir().unwrap();
        let manager = SkillManager::new(dir.path(), None);
        let mut manifest = parameterized_manifest("safe");

        assert!(manager
            .save_skill("../escape", manifest.clone(), "()")
            .is_err());
        manifest.script_file = "../escape.rhai".to_string();
        assert!(manager.save_skill("safe", manifest, "()").is_err());
        assert!(!dir.path().parent().unwrap().join("escape.rhai").exists());
    }

    #[test]
    fn parameter_injection_is_data_not_code() {
        let dir = tempdir().unwrap();
        let manager = SkillManager::new(dir.path(), None);
        manager
            .save_skill("safe", parameterized_manifest("safe"), "payload")
            .unwrap();
        let malicious = "\"); throw `injected`; //\\\n\u{2028}still data";
        let params = HashMap::from([(
            "payload".to_string(),
            serde_json::Value::String(malicious.to_string()),
        )]);
        let prepared = manager.prepare_script("safe", &params).unwrap();
        let sandbox = Sandbox::new(SandboxConfig::default(), Arc::new(NullReporter));
        let result = sandbox.execute(&prepared).unwrap();
        assert_eq!(result.output, malicious);
    }

    #[test]
    fn rejects_unknown_and_wrongly_typed_parameters() {
        let dir = tempdir().unwrap();
        let manager = SkillManager::new(dir.path(), None);
        manager
            .save_skill("safe", parameterized_manifest("safe"), "payload")
            .unwrap();
        let unknown = HashMap::from([("other".to_string(), serde_json::json!("value"))]);
        assert!(manager.prepare_script("safe", &unknown).is_err());
        let wrong_type = HashMap::from([("payload".to_string(), serde_json::json!(42))]);
        assert!(manager.prepare_script("safe", &wrong_type).is_err());
    }

    #[test]
    fn rejects_oversized_scripts_and_invalid_defaults() {
        let dir = tempdir().unwrap();
        let manager = SkillManager::new(dir.path(), None);
        let oversized = "x".repeat(MAX_SKILL_SCRIPT_BYTES as usize + 1);
        assert!(manager
            .save_skill("safe", parameterized_manifest("safe"), &oversized)
            .is_err());

        let mut manifest = parameterized_manifest("safe");
        manifest.parameters[0].default = Some(serde_json::json!(123));
        assert!(manager.save_skill("safe", manifest, "payload").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn ignores_symlink_manifests_and_rejects_symlink_scripts() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let manager = SkillManager::new(dir.path(), None);
        let manifest = parameterized_manifest("linked");
        fs::write(outside.path().join("manifest"), manifest.to_toml().unwrap()).unwrap();
        symlink(
            outside.path().join("manifest"),
            dir.path().join("linked.skill.toml"),
        )
        .unwrap();
        assert!(matches!(
            manager.get_skill("linked"),
            Err(SkillError::NotFound(_))
        ));

        fs::remove_file(dir.path().join("linked.skill.toml")).unwrap();
        fs::write(
            dir.path().join("linked.skill.toml"),
            manifest.to_toml().unwrap(),
        )
        .unwrap();
        fs::write(outside.path().join("script"), "payload").unwrap();
        symlink(
            outside.path().join("script"),
            dir.path().join("linked.rhai"),
        )
        .unwrap();
        let params = HashMap::from([("payload".to_string(), serde_json::json!("value"))]);
        assert!(manager.prepare_script("linked", &params).is_err());
    }
}
