//! Skill registry for discovering, loading, and managing available skills.
//!
//! Skills are discovered from two filesystem locations:
//! 1. Workspace skills directory (`<workspace>/skills/`) -- Trusted
//! 2. User skills directory (`~/.thinclaw/skills/`) -- Trusted
//!
//! Both flat (`skills/SKILL.md`) and subdirectory (`skills/<name>/SKILL.md`)
//! layouts are supported. Earlier locations win on name collision (workspace
//! overrides user). Uses async I/O throughout to avoid blocking the tokio runtime.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::skills::gating;
use crate::skills::parser::{SkillParseError, parse_skill_md};
use crate::skills::{
    GatingRequirements, LoadedSkill, MAX_PROMPT_FILE_SIZE, SkillSource, SkillSourceTier,
    SkillTrust, normalize_line_endings,
};

/// Maximum number of skills that can be discovered from a single directory.
/// Prevents resource exhaustion from a directory with thousands of entries.
const MAX_DISCOVERED_SKILLS: usize = 100;
const MAX_DISCOVERY_ENTRIES: usize = 10_000;

/// Error type for skill registry operations.
#[derive(Debug, thiserror::Error)]
pub enum SkillRegistryError {
    #[error("Skill not found: {0}")]
    NotFound(String),

    #[error("Failed to read skill file {path}: {reason}")]
    ReadError { path: String, reason: String },

    #[error("Failed to parse SKILL.md for '{name}': {reason}")]
    ParseError { name: String, reason: String },

    #[error("Skill file too large for '{name}': {size} bytes (max {max} bytes)")]
    FileTooLarge { name: String, size: u64, max: u64 },

    #[error("Symlink detected in skills directory: {path}")]
    SymlinkDetected { path: String },

    #[error("Skill '{name}' failed gating: {reason}")]
    GatingFailed { name: String, reason: String },

    #[error(
        "Skill '{name}' prompt exceeds token budget: ~{approx_tokens} tokens but declares max_context_tokens={declared}"
    )]
    TokenBudgetExceeded {
        name: String,
        approx_tokens: usize,
        declared: usize,
    },

    #[error("Skill '{name}' already exists")]
    AlreadyExists { name: String },

    #[error("Cannot remove skill '{name}': {reason}")]
    CannotRemove { name: String, reason: String },

    #[error("Failed to write skill file {path}: {reason}")]
    WriteError { path: String, reason: String },

    #[error("Invalid skill name '{name}': {reason}")]
    InvalidName { name: String, reason: String },
}

/// Registry of available skills.
pub struct SkillRegistry {
    /// All loaded skills.
    skills: Vec<LoadedSkill>,
    /// User skills directory (~/.thinclaw/skills/). Skills here are Trusted.
    user_dir: PathBuf,
    /// Registry-installed skills directory (~/.thinclaw/installed_skills/). Skills here are Installed.
    installed_dir: Option<PathBuf>,
    /// Optional workspace skills directory.
    workspace_dir: Option<PathBuf>,
    /// Bundled trusted skills compiled into the application.
    bundled_skills: Vec<(PathBuf, &'static str)>,
    /// Optional external read-only skill directories with display provenance.
    external_read_only_dirs: Vec<(PathBuf, SkillSourceTier)>,
}

impl SkillRegistry {
    /// Create a new skill registry.
    pub fn new(user_dir: PathBuf) -> Self {
        Self {
            skills: Vec::new(),
            user_dir,
            installed_dir: None,
            workspace_dir: None,
            bundled_skills: Vec::new(),
            external_read_only_dirs: Vec::new(),
        }
    }

    /// Build a fresh registry with the same source configuration but no loaded
    /// skills.
    ///
    /// This enables a load-then-swap reload: the caller runs the expensive
    /// `discover_all` IO on the returned instance *without* holding a lock on the
    /// live registry, then swaps it in under a brief write. That avoids stalling
    /// concurrent skill reads behind discovery IO and avoids the torn-state
    /// window of `reload` (which clears, then asynchronously repopulates, while
    /// holding the write lock).
    pub fn clone_config(&self) -> Self {
        Self {
            skills: Vec::new(),
            user_dir: self.user_dir.clone(),
            installed_dir: self.installed_dir.clone(),
            workspace_dir: self.workspace_dir.clone(),
            bundled_skills: self.bundled_skills.clone(),
            external_read_only_dirs: self.external_read_only_dirs.clone(),
        }
    }

    /// Set the registry-installed skills directory.
    ///
    /// Skills installed via ClawHub or the skill tools are written here and
    /// loaded with `SkillTrust::Installed` (read-only tool access). This
    /// directory is separate from the user dir so that trust levels survive
    /// restarts correctly.
    pub fn with_installed_dir(mut self, dir: PathBuf) -> Self {
        self.installed_dir = Some(dir);
        self
    }

    /// Set a workspace skills directory.
    pub fn with_workspace_dir(mut self, dir: PathBuf) -> Self {
        self.workspace_dir = Some(dir);
        self
    }

    pub fn with_external_read_only_dir(mut self, dir: PathBuf, tier: SkillSourceTier) -> Self {
        self.external_read_only_dirs.push((dir, tier));
        self
    }

    pub fn with_bundled_skill(mut self, path: PathBuf, content: &'static str) -> Self {
        self.bundled_skills.push((path, content));
        self
    }

    /// Return the configured directories that participate in discovery.
    pub fn discovery_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(dir) = self.workspace_dir.as_ref() {
            dirs.push(dir.clone());
        }
        dirs.push(self.user_dir.clone());
        if let Some(dir) = self.installed_dir.as_ref() {
            dirs.push(dir.clone());
        }
        dirs.extend(
            self.bundled_skills
                .iter()
                .filter_map(|(path, _)| path.parent().map(Path::to_path_buf)),
        );
        dirs.extend(
            self.external_read_only_dirs
                .iter()
                .map(|(dir, _)| dir.clone()),
        );
        dirs
    }

    /// Return the writable directory used for installs prepared through the
    /// staged install pipeline.
    pub fn install_root(&self) -> &Path {
        self.installed_dir.as_deref().unwrap_or(&self.user_dir)
    }

    /// Discover and load skills from all configured directories.
    ///
    /// Discovery order (earlier wins on name collision):
    /// 1. Workspace skills directory (if set) -- Trusted
    /// 2. User skills directory -- Trusted
    /// 3. Installed skills directory (if set) -- Installed
    pub async fn discover_all(&mut self) -> Vec<String> {
        let mut loaded_names: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        // 1. Workspace skills (highest priority)
        if let Some(ws_dir) = self.workspace_dir.clone() {
            let ws_skills = self
                .discover_from_dir(&ws_dir, SkillTrust::Trusted, SkillSource::Workspace)
                .await;
            for (name, skill) in ws_skills {
                if seen.contains(&name) {
                    continue;
                }
                seen.insert(name.clone());
                loaded_names.push(name);
                self.skills.push(skill);
            }
        }

        // 2. Bundled trusted skills
        for (path, content) in self.bundled_skills.clone() {
            match validate_normalized_skill_content(
                &path.display().to_string(),
                &normalize_line_endings(content),
                SkillTrust::Trusted,
                SkillSource::Bundled(path.clone()),
            )
            .await
            {
                Ok((name, skill)) => {
                    if seen.contains(&name) {
                        tracing::debug!("Skipping bundled skill '{}' (overridden)", name);
                        continue;
                    }
                    seen.insert(name.clone());
                    loaded_names.push(name);
                    self.skills.push(skill);
                }
                Err(error) => {
                    tracing::warn!(path = %path.display(), error = %error, "Failed to load bundled skill");
                }
            }
        }

        // 3. User skills
        let user_dir = self.user_dir.clone();
        let user_skills = self
            .discover_from_dir(&user_dir, SkillTrust::Trusted, SkillSource::User)
            .await;
        for (name, skill) in user_skills {
            if seen.contains(&name) {
                tracing::debug!("Skipping user skill '{}' (overridden by workspace)", name);
                continue;
            }
            seen.insert(name.clone());
            loaded_names.push(name);
            self.skills.push(skill);
        }

        // 4. Installed skills (registry-installed, lowest priority)
        if let Some(inst_dir) = self.installed_dir.clone() {
            let inst_skills = self
                .discover_from_dir(&inst_dir, SkillTrust::Installed, SkillSource::User)
                .await;
            for (name, skill) in inst_skills {
                if seen.contains(&name) {
                    tracing::debug!(
                        "Skipping installed skill '{}' (overridden by user/workspace)",
                        name
                    );
                    continue;
                }
                seen.insert(name.clone());
                loaded_names.push(name);
                self.skills.push(skill);
            }
        }

        for (dir, tier) in self.external_read_only_dirs.clone() {
            let ext_skills = self
                .discover_from_dir(&dir, SkillTrust::Installed, SkillSource::External)
                .await;
            for (name, mut skill) in ext_skills {
                if seen.contains(&name) {
                    tracing::debug!(
                        "Skipping external skill '{}' (overridden by local registry tiers)",
                        name
                    );
                    continue;
                }
                skill.source_tier = tier;
                seen.insert(name.clone());
                loaded_names.push(name);
                self.skills.push(skill);
            }
        }

        loaded_names
    }

    /// Discover skills from a single directory.
    ///
    /// Supports both layouts:
    /// - Flat: `dir/SKILL.md` (skill name derived from parent dir or file stem)
    /// - Subdirectory: `dir/<name>/SKILL.md`
    async fn discover_from_dir<F>(
        &self,
        dir: &Path,
        trust: SkillTrust,
        make_source: F,
    ) -> Vec<(String, LoadedSkill)>
    where
        F: Fn(PathBuf) -> SkillSource,
    {
        let mut results = Vec::new();

        match tokio::fs::symlink_metadata(dir).await {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                tracing::warn!("Skills directory is not a real directory: {:?}", dir);
                return results;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("Skills directory does not exist: {:?}", dir);
                return results;
            }
            Err(error) => {
                tracing::warn!("Failed to inspect skills directory {:?}: {}", dir, error);
                return results;
            }
        }

        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!("Failed to read skills directory {:?}: {}", dir, e);
                return results;
            }
        };

        let mut count = 0usize;
        let mut scanned = 0usize;
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!("Skill directory scan failed in {:?}: {}", dir, error);
                    break;
                }
            };
            scanned = scanned.saturating_add(1);
            if scanned > MAX_DISCOVERY_ENTRIES {
                tracing::warn!(
                    "Skill directory entry cap reached ({}), skipping remaining",
                    MAX_DISCOVERY_ENTRIES
                );
                break;
            }
            if count >= MAX_DISCOVERED_SKILLS {
                tracing::warn!(
                    "Skill discovery cap reached ({} skills), skipping remaining",
                    MAX_DISCOVERED_SKILLS
                );
                break;
            }

            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('.'))
            {
                continue;
            }
            let meta = match tokio::fs::symlink_metadata(&path).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::debug!("Failed to stat {:?}: {}", path, e);
                    continue;
                }
            };

            // Reject symlinks
            if meta.is_symlink() {
                tracing::warn!(
                    "Skipping symlink in skills directory: {:?}",
                    path.file_name().unwrap_or_default()
                );
                continue;
            }

            // Case 1: Subdirectory containing SKILL.md
            if meta.is_dir() {
                let skill_md = path.join("SKILL.md");
                if tokio::fs::try_exists(&skill_md).await.unwrap_or(false) {
                    count += 1;
                    let source = make_source(path.clone());
                    match self.load_skill_md(&skill_md, trust, source).await {
                        Ok((name, skill)) => {
                            tracing::debug!("Loaded skill: {}", name);
                            results.push((name, skill));
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to load skill from {:?}: {}",
                                path.file_name().unwrap_or_default(),
                                e
                            );
                        }
                    }
                }
                continue;
            }

            // Case 2: Flat SKILL.md directly in the directory
            if meta.is_file()
                && let Some(fname) = path.file_name().and_then(|f| f.to_str())
                && fname == "SKILL.md"
            {
                count += 1;
                let source = make_source(dir.to_path_buf());
                match self.load_skill_md(&path, trust, source).await {
                    Ok((name, skill)) => {
                        tracing::info!("Loaded skill: {}", name);
                        results.push((name, skill));
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load skill from {:?}: {}", fname, e);
                    }
                }
            }
        }

        results
    }

    /// Load a single SKILL.md file.
    async fn load_skill_md(
        &self,
        path: &Path,
        trust: SkillTrust,
        source: SkillSource,
    ) -> Result<(String, LoadedSkill), SkillRegistryError> {
        load_and_validate_skill(path, trust, source).await
    }

    /// Get all loaded skills.
    pub fn skills(&self) -> &[LoadedSkill] {
        &self.skills
    }

    /// Get the number of loaded skills.
    pub fn count(&self) -> usize {
        self.skills.len()
    }

    /// Check if a skill with the given name is loaded.
    pub fn has(&self, name: &str) -> bool {
        self.skills.iter().any(|s| s.manifest.name == name)
    }

    /// Find a skill by name.
    pub fn find_by_name(&self, name: &str) -> Option<&LoadedSkill> {
        self.skills.iter().find(|s| s.manifest.name == name)
    }

    /// Perform the disk I/O and loading for a skill install.
    ///
    /// This is a static method so it doesn't borrow `&self`, allowing callers
    /// to drop their registry lock before awaiting.
    pub async fn prepare_install_to_disk(
        user_dir: &Path,
        skill_name: &str,
        normalized_content: &str,
    ) -> Result<(String, LoadedSkill), SkillRegistryError> {
        // ── Path traversal protection ──────────────────────────────────
        // Reject skill names that could escape the target directory.
        if !crate::skills::validate_skill_name(skill_name) {
            return Err(SkillRegistryError::InvalidName {
                name: skill_name.to_string(),
                reason: "skill name must contain only safe filename characters".into(),
            });
        }
        if normalized_content.len() as u64 > MAX_PROMPT_FILE_SIZE {
            return Err(SkillRegistryError::FileTooLarge {
                name: skill_name.to_string(),
                size: normalized_content.len() as u64,
                max: MAX_PROMPT_FILE_SIZE,
            });
        }

        tokio::fs::create_dir_all(user_dir).await.map_err(|error| {
            SkillRegistryError::WriteError {
                path: user_dir.display().to_string(),
                reason: error.to_string(),
            }
        })?;
        let root_metadata = tokio::fs::symlink_metadata(user_dir)
            .await
            .map_err(|error| SkillRegistryError::WriteError {
                path: user_dir.display().to_string(),
                reason: error.to_string(),
            })?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(SkillRegistryError::WriteError {
                path: user_dir.display().to_string(),
                reason: "skill install root must be a real directory".to_string(),
            });
        }

        // ── Safety module pre-check ────────────────────────────────────
        // Run SkillPathConfig validation (normalize + containment + symlink
        // detection) before touching the filesystem. This catches edge cases
        // the inline checks above may miss (e.g. symlink-based escapes on
        // existing paths) and consolidates the two validation codepaths.
        {
            let safety_config = crate::safety::skill_path::SkillPathConfig {
                base_dir: user_dir.to_path_buf(),
                allow_symlinks: false,
            };
            safety_config
                .skill_path(skill_name)
                .map_err(|e| SkillRegistryError::InvalidName {
                    name: skill_name.to_string(),
                    reason: e.to_string(),
                })?;
        }

        let skill_dir = user_dir.join(skill_name);
        let staging_root = user_dir.join(".thinclaw-skill-staging");
        match tokio::fs::create_dir(&staging_root).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(SkillRegistryError::WriteError {
                    path: staging_root.display().to_string(),
                    reason: error.to_string(),
                });
            }
        }
        let staging_metadata =
            tokio::fs::symlink_metadata(&staging_root)
                .await
                .map_err(|error| SkillRegistryError::WriteError {
                    path: staging_root.display().to_string(),
                    reason: error.to_string(),
                })?;
        if staging_metadata.file_type().is_symlink() || !staging_metadata.is_dir() {
            return Err(SkillRegistryError::WriteError {
                path: staging_root.display().to_string(),
                reason: "skill staging root must be a real directory".to_string(),
            });
        }
        let stage_dir = staging_root.join(uuid::Uuid::new_v4().simple().to_string());
        tokio::fs::create_dir(&stage_dir).await.map_err(|error| {
            SkillRegistryError::WriteError {
                path: stage_dir.display().to_string(),
                reason: error.to_string(),
            }
        })?;
        let stage_skill_path = stage_dir.join("SKILL.md");
        if let Err(error) = thinclaw_platform::write_private_file_atomic_async(
            stage_skill_path.clone(),
            normalized_content.as_bytes().to_vec(),
            false,
        )
        .await
        {
            let _ = tokio::fs::remove_dir_all(&stage_dir).await;
            return Err(SkillRegistryError::WriteError {
                path: stage_skill_path.display().to_string(),
                reason: error.to_string(),
            });
        }

        let loaded = load_and_validate_skill(
            &stage_skill_path,
            SkillTrust::Installed,
            SkillSource::User(skill_dir.clone()),
        )
        .await;
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&stage_dir).await;
                return Err(error);
            }
        };
        if loaded.0 != skill_name {
            let _ = tokio::fs::remove_dir_all(&stage_dir).await;
            return Err(SkillRegistryError::InvalidName {
                name: skill_name.to_string(),
                reason: format!(
                    "SKILL.md declares '{}' but install target is '{}'",
                    loaded.0, skill_name
                ),
            });
        }

        let publish_root = user_dir.to_path_buf();
        let publish_stage = stage_dir.clone();
        let publish_target = skill_dir.clone();
        let publish_result = tokio::task::spawn_blocking(move || {
            publish_staged_skill_directory(&publish_root, &publish_stage, &publish_target)
        })
        .await
        .map_err(|error| SkillRegistryError::WriteError {
            path: skill_dir.display().to_string(),
            reason: format!("skill installer task failed: {error}"),
        })?;
        if let Err(error) = publish_result {
            let _ = tokio::fs::remove_dir_all(&stage_dir).await;
            return Err(SkillRegistryError::WriteError {
                path: skill_dir.display().to_string(),
                reason: error.to_string(),
            });
        }

        Ok(loaded)
    }

    /// Load and validate an already-written skill directory from disk.
    ///
    /// Used by the quarantine/install pipeline after it has copied a vetted
    /// SKILL.md into the final install location.
    pub async fn load_skill_from_path(
        skill_dir: &Path,
        trust: SkillTrust,
        source: SkillSource,
    ) -> Result<(String, LoadedSkill), SkillRegistryError> {
        load_and_validate_skill(&skill_dir.join("SKILL.md"), trust, source).await
    }

    /// Validate an existing SKILL.md file or skill directory without mutating registry state.
    pub async fn validate_skill_file(
        path: &Path,
        trust: SkillTrust,
        source: SkillSource,
    ) -> Result<(String, LoadedSkill), SkillRegistryError> {
        let skill_path = if path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
            path.to_path_buf()
        } else {
            path.join("SKILL.md")
        };
        load_and_validate_skill(&skill_path, trust, source).await
    }

    /// Validate SKILL.md content without writing it to disk or mutating registry state.
    pub async fn validate_skill_content(
        content: &str,
        trust: SkillTrust,
        source: SkillSource,
    ) -> Result<(String, LoadedSkill), SkillRegistryError> {
        if content.len() as u64 > MAX_PROMPT_FILE_SIZE {
            return Err(SkillRegistryError::FileTooLarge {
                name: "(inline content)".to_string(),
                size: content.len() as u64,
                max: MAX_PROMPT_FILE_SIZE,
            });
        }
        let normalized_content = normalize_line_endings(content);
        validate_normalized_skill_content("(inline content)", &normalized_content, trust, source)
            .await
    }

    /// Commit a prepared skill into the in-memory registry.
    ///
    /// This is a fast, synchronous operation that only adds to the Vec.
    /// Call after `prepare_install` completes.
    pub fn commit_install(
        &mut self,
        name: &str,
        skill: LoadedSkill,
    ) -> Result<(), SkillRegistryError> {
        // Re-check for duplicates (another thread may have installed between prepare and commit)
        if self.has(name) {
            return Err(SkillRegistryError::AlreadyExists {
                name: name.to_string(),
            });
        }
        self.skills.push(skill);
        tracing::info!("Installed skill: {}", name);
        Ok(())
    }

    /// Install a skill at runtime from SKILL.md content.
    ///
    /// Convenience method that parses, writes to disk, and commits in-memory.
    /// When called through tool execution where a lock is involved, prefer using
    /// `prepare_install_to_disk` + `commit_install` separately to minimize lock
    /// hold time.
    pub async fn install_skill(&mut self, content: &str) -> Result<String, SkillRegistryError> {
        let normalized = normalize_line_endings(content);
        let parsed = parse_skill_md(&normalized).map_err(|e: SkillParseError| match e {
            SkillParseError::InvalidName { ref name } => SkillRegistryError::ParseError {
                name: name.clone(),
                reason: e.to_string(),
            },
            _ => SkillRegistryError::ParseError {
                name: "(install)".to_string(),
                reason: e.to_string(),
            },
        })?;
        let skill_name = parsed.manifest.name.clone();
        if self.has(&skill_name) {
            return Err(SkillRegistryError::AlreadyExists { name: skill_name });
        }
        let user_dir = self.user_dir.clone();
        let (name, skill) =
            Self::prepare_install_to_disk(&user_dir, &skill_name, &normalized).await?;
        self.commit_install(&name, skill)?;
        Ok(name)
    }

    /// Validate that a skill can be removed and return its filesystem path.
    ///
    /// Performs validation without modifying state. Callers can then do async
    /// filesystem cleanup without holding the registry lock, and call
    /// `commit_remove` afterward.
    pub fn validate_remove(&self, name: &str) -> Result<PathBuf, SkillRegistryError> {
        let idx = self
            .skills
            .iter()
            .position(|s| s.manifest.name == name)
            .ok_or_else(|| SkillRegistryError::NotFound(name.to_string()))?;

        let skill = &self.skills[idx];

        match &skill.source {
            SkillSource::User(path) => Ok(path.clone()),
            SkillSource::Workspace(_) => Err(SkillRegistryError::CannotRemove {
                name: name.to_string(),
                reason: "workspace skills cannot be removed via this interface".to_string(),
            }),
            SkillSource::Bundled(_) => Err(SkillRegistryError::CannotRemove {
                name: name.to_string(),
                reason: "bundled skills cannot be removed".to_string(),
            }),
            SkillSource::External(_) => Err(SkillRegistryError::CannotRemove {
                name: name.to_string(),
                reason: "external read-only skills cannot be removed".to_string(),
            }),
        }
    }

    /// Remove a skill's files from disk (async I/O).
    ///
    /// Call after `validate_remove` and before `commit_remove`.
    pub async fn delete_skill_files(
        path: &Path,
        expected_name: &str,
    ) -> Result<(), SkillRegistryError> {
        if !crate::skills::validate_skill_name(expected_name) {
            return Err(SkillRegistryError::InvalidName {
                name: expected_name.to_string(),
                reason: "invalid expected skill name".to_string(),
            });
        }
        let root_metadata = tokio::fs::symlink_metadata(path).await.map_err(|e| {
            SkillRegistryError::WriteError {
                path: path.display().to_string(),
                reason: e.to_string(),
            }
        })?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(SkillRegistryError::WriteError {
                path: path.display().to_string(),
                reason: "skill root must be a real directory".to_string(),
            });
        }

        let skill_md = path.join("SKILL.md");
        let skill_metadata = tokio::fs::symlink_metadata(&skill_md).await.map_err(|e| {
            SkillRegistryError::WriteError {
                path: skill_md.display().to_string(),
                reason: e.to_string(),
            }
        })?;
        if skill_metadata.file_type().is_symlink() || !skill_metadata.is_file() {
            return Err(SkillRegistryError::WriteError {
                path: skill_md.display().to_string(),
                reason: "SKILL.md must be a bounded regular file".to_string(),
            });
        }
        if skill_metadata.len() > MAX_PROMPT_FILE_SIZE {
            return Err(SkillRegistryError::FileTooLarge {
                name: expected_name.to_string(),
                size: skill_metadata.len(),
                max: MAX_PROMPT_FILE_SIZE,
            });
        }
        let content = thinclaw_platform::read_regular_file_bounded_single_link_async(
            skill_md.clone(),
            MAX_PROMPT_FILE_SIZE,
        )
        .await
        .and_then(|bytes| {
            String::from_utf8(bytes)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })
        .map_err(|e| SkillRegistryError::ReadError {
            path: skill_md.display().to_string(),
            reason: e.to_string(),
        })?;
        let parsed = parse_skill_md(&normalize_line_endings(&content)).map_err(|e| {
            SkillRegistryError::ParseError {
                name: expected_name.to_string(),
                reason: e.to_string(),
            }
        })?;
        if !parsed.manifest.name.eq_ignore_ascii_case(expected_name) {
            return Err(SkillRegistryError::WriteError {
                path: skill_md.display().to_string(),
                reason: format!(
                    "on-disk skill name '{}' does not match expected name '{}'",
                    parsed.manifest.name, expected_name
                ),
            });
        }

        let lock_path = path.join(".thinclaw-skill-lock.json");
        let managed_package = match tokio::fs::symlink_metadata(&lock_path).await {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || metadata.len() > 1024 * 1024
                {
                    return Err(SkillRegistryError::WriteError {
                        path: lock_path.display().to_string(),
                        reason: "skill provenance lock must be a bounded regular file".to_string(),
                    });
                }
                let raw = thinclaw_platform::read_regular_file_bounded_single_link_async(
                    lock_path.clone(),
                    1024 * 1024,
                )
                .await
                .map_err(|e| SkillRegistryError::ReadError {
                    path: lock_path.display().to_string(),
                    reason: e.to_string(),
                })?;
                let provenance: crate::skills::quarantine::SkillProvenance =
                    serde_json::from_slice(&raw).map_err(|e| SkillRegistryError::ReadError {
                        path: lock_path.display().to_string(),
                        reason: format!("invalid provenance lock: {e}"),
                    })?;
                if provenance.package_files.is_empty() {
                    false
                } else {
                    if provenance.package_files.len() > 2048 {
                        return Err(SkillRegistryError::WriteError {
                            path: lock_path.display().to_string(),
                            reason: "provenance package file list exceeds 2048 entries".to_string(),
                        });
                    }
                    let mut seen = HashSet::new();
                    let valid_paths = provenance.package_files.iter().all(|relative| {
                        let relative_path = Path::new(relative);
                        !relative.is_empty()
                            && relative.len() <= 1024
                            && relative_path.components().all(|component| {
                                matches!(component, std::path::Component::Normal(_))
                            })
                            && relative != ".thinclaw-skill-lock.json"
                            && seen.insert(relative.clone())
                    });
                    if !valid_paths
                        || !provenance
                            .package_files
                            .iter()
                            .any(|relative| relative == "SKILL.md")
                    {
                        return Err(SkillRegistryError::WriteError {
                            path: lock_path.display().to_string(),
                            reason: "provenance contains an unsafe package file list".to_string(),
                        });
                    }
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| value.eq_ignore_ascii_case(expected_name))
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(SkillRegistryError::WriteError {
                    path: lock_path.display().to_string(),
                    reason: error.to_string(),
                });
            }
        };

        if managed_package {
            // remove_dir_all does not follow directory symlinks. Re-stat the
            // root immediately before deletion to fail closed on replacement.
            let current = tokio::fs::symlink_metadata(path).await.map_err(|e| {
                SkillRegistryError::WriteError {
                    path: path.display().to_string(),
                    reason: e.to_string(),
                }
            })?;
            if current.file_type().is_symlink() || !current.is_dir() {
                return Err(SkillRegistryError::WriteError {
                    path: path.display().to_string(),
                    reason: "skill root changed during removal".to_string(),
                });
            }
            tokio::fs::remove_dir_all(path)
                .await
                .map_err(|e| SkillRegistryError::WriteError {
                    path: path.display().to_string(),
                    reason: e.to_string(),
                })?;
        } else {
            tokio::fs::remove_file(&skill_md).await.map_err(|e| {
                SkillRegistryError::WriteError {
                    path: skill_md.display().to_string(),
                    reason: e.to_string(),
                }
            })?;
            // Legacy SKILL.md-only installs may have an old provenance lock
            // without a package inventory. Remove the validated regular lock,
            // then remove the directory only if nothing else remains.
            if tokio::fs::symlink_metadata(&lock_path)
                .await
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            {
                let _ = tokio::fs::remove_file(&lock_path).await;
            }
            let _ = tokio::fs::remove_dir(path).await;
        }
        Ok(())
    }

    /// Remove a skill from the in-memory registry.
    ///
    /// Fast synchronous operation. Call after filesystem cleanup.
    pub fn commit_remove(&mut self, name: &str) -> Result<(), SkillRegistryError> {
        let idx = self
            .skills
            .iter()
            .position(|s| s.manifest.name == name)
            .ok_or_else(|| SkillRegistryError::NotFound(name.to_string()))?;

        self.skills.remove(idx);
        tracing::info!("Removed skill: {}", name);
        Ok(())
    }

    /// Remove a skill by name.
    ///
    /// Convenience method that combines validation, file deletion, and in-memory
    /// removal. When called through tool execution, prefer using the split
    /// validate/delete/commit methods to minimize lock hold time.
    pub async fn remove_skill(&mut self, name: &str) -> Result<(), SkillRegistryError> {
        let path = self.validate_remove(name)?;
        Self::delete_skill_files(&path, name).await?;
        self.commit_remove(name)
    }

    /// Change a skill's trust level by moving it between directories.
    ///
    /// - `Trusted`: moves to `user_dir` (~/.thinclaw/skills/) — full tool access
    /// - `Installed`: moves to `installed_dir` (~/.thinclaw/installed_skills/) — read-only tools
    ///
    /// Trust is location-based, so this physically moves the files. The trust
    /// change survives restarts because `discover_all()` assigns trust based on
    /// which directory contains the skill.
    ///
    /// Only `User`-sourced skills can be promoted/demoted. Workspace and Bundled
    /// skills are managed externally and cannot change trust via this interface.
    pub async fn promote_trust(
        &mut self,
        name: &str,
        target_trust: SkillTrust,
    ) -> Result<(), SkillRegistryError> {
        let idx = self
            .skills
            .iter()
            .position(|s| s.manifest.name == name)
            .ok_or_else(|| SkillRegistryError::NotFound(name.to_string()))?;

        let skill = &self.skills[idx];

        // Only User-sourced skills can change trust
        let current_path = match &skill.source {
            SkillSource::User(path) => path.clone(),
            SkillSource::Workspace(_) => {
                return Err(SkillRegistryError::CannotRemove {
                    name: name.to_string(),
                    reason: "workspace skills cannot change trust via this interface".into(),
                });
            }
            SkillSource::Bundled(_) => {
                return Err(SkillRegistryError::CannotRemove {
                    name: name.to_string(),
                    reason: "bundled skills cannot change trust".into(),
                });
            }
            SkillSource::External(_) => {
                return Err(SkillRegistryError::CannotRemove {
                    name: name.to_string(),
                    reason: "external read-only skills cannot change trust".into(),
                });
            }
        };

        // Already at target trust?
        if skill.trust == target_trust {
            return Ok(());
        }

        // Determine the current and target tier roots. A trust move is only
        // safe for a dedicated `<tier>/<skill-name>` package directory; flat
        // SKILL.md layouts must be reorganized explicitly by the user.
        let current_root = match skill.trust {
            SkillTrust::Trusted => self.user_dir.clone(),
            SkillTrust::Installed => {
                self.installed_dir
                    .clone()
                    .ok_or_else(|| SkillRegistryError::WriteError {
                        path: name.to_string(),
                        reason: "no installed_dir configured".into(),
                    })?
            }
        };
        if current_path.parent() != Some(current_root.as_path())
            || current_path
                .file_name()
                .and_then(|value| value.to_str())
                .is_none_or(|value| !value.eq_ignore_ascii_case(name))
        {
            return Err(SkillRegistryError::WriteError {
                path: current_path.display().to_string(),
                reason: "trust changes require a dedicated skill-name package directory".into(),
            });
        }

        let target_dir = match target_trust {
            SkillTrust::Trusted => self.user_dir.clone(),
            SkillTrust::Installed => {
                self.installed_dir
                    .clone()
                    .ok_or_else(|| SkillRegistryError::WriteError {
                        path: name.to_string(),
                        reason: "no installed_dir configured, cannot demote skill".into(),
                    })?
            }
        };

        tokio::fs::create_dir_all(&target_dir).await.map_err(|e| {
            SkillRegistryError::WriteError {
                path: target_dir.display().to_string(),
                reason: e.to_string(),
            }
        })?;
        let target_metadata = tokio::fs::symlink_metadata(&target_dir)
            .await
            .map_err(|e| SkillRegistryError::WriteError {
                path: target_dir.display().to_string(),
                reason: e.to_string(),
            })?;
        if target_metadata.file_type().is_symlink() || !target_metadata.is_dir() {
            return Err(SkillRegistryError::WriteError {
                path: target_dir.display().to_string(),
                reason: "target tier root must be a real directory".into(),
            });
        }

        let new_skill_dir = target_dir.join(name);
        match tokio::fs::symlink_metadata(&new_skill_dir).await {
            Ok(_) => {
                return Err(SkillRegistryError::AlreadyExists {
                    name: name.to_string(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(SkillRegistryError::WriteError {
                    path: new_skill_dir.display().to_string(),
                    reason: error.to_string(),
                });
            }
        }

        // Validate the exact on-disk package under the target authority before
        // the atomic move. The returned skill already carries the final source.
        let (_, moved_skill) = Self::validate_skill_file(
            &current_path,
            target_trust,
            SkillSource::User(new_skill_dir.clone()),
        )
        .await?;
        let source_for_move = current_path.clone();
        let destination_for_move = new_skill_dir.clone();
        tokio::task::spawn_blocking(move || {
            thinclaw_platform::rename_no_replace(&source_for_move, &destination_for_move)
        })
        .await
        .map_err(|error| SkillRegistryError::WriteError {
            path: new_skill_dir.display().to_string(),
            reason: error.to_string(),
        })?
        .map_err(|error| SkillRegistryError::WriteError {
            path: new_skill_dir.display().to_string(),
            reason: if error.kind() == std::io::ErrorKind::CrossesDevices {
                "skill trust roots must be on the same filesystem for an atomic move".to_string()
            } else {
                error.to_string()
            },
        })?;

        self.skills[idx] = moved_skill;

        tracing::info!(
            skill = name,
            trust = %target_trust,
            "Skill trust level changed"
        );

        Ok(())
    }

    /// Clear all loaded skills and re-discover from disk.
    pub async fn reload(&mut self) -> Vec<String> {
        self.skills.clear();
        self.discover_all().await
    }

    /// Hot-reload a single skill from its current on-disk SKILL.md.
    ///
    /// Re-reads the file, re-parses the manifest, recompiles patterns, and
    /// replaces the in-memory entry atomically. The skill's source and trust
    /// level are preserved (it stays in the same directory with the same trust).
    ///
    /// Returns the skill name on success, or an error if the skill is not found
    /// in the registry, the file can't be read, or the SKILL.md is invalid.
    ///
    /// Use this after editing a skill file on disk so changes take effect
    /// immediately without a full restart or full registry reload.
    pub async fn reload_skill(&mut self, name: &str) -> Result<String, SkillRegistryError> {
        let idx = self
            .skills
            .iter()
            .position(|s| s.manifest.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| SkillRegistryError::NotFound(name.to_string()))?;

        // Derive the SKILL.md path from the current in-memory source
        let (skill_md_path, trust, source) = {
            let skill = &self.skills[idx];
            let md_path = match &skill.source {
                SkillSource::User(dir) => dir.join("SKILL.md"),
                SkillSource::Workspace(dir) => dir.join("SKILL.md"),
                SkillSource::Bundled(dir) => dir.join("SKILL.md"),
                SkillSource::External(dir) => dir.join("SKILL.md"),
            };
            (md_path, skill.trust, skill.source.clone())
        };

        // Re-load and validate from disk (full pipeline: read, parse, hash, compile)
        let (new_name, new_skill) = load_and_validate_skill(&skill_md_path, trust, source).await?;

        // Replace the in-memory entry atomically
        self.skills[idx] = new_skill;

        tracing::info!(
            skill = new_name,
            path = %skill_md_path.display(),
            "Skill hot-reloaded from disk"
        );

        Ok(new_name)
    }

    /// Get the user skills directory path.
    pub fn user_dir(&self) -> &Path {
        &self.user_dir
    }

    /// Get the installed skills directory path, if configured.
    pub fn installed_dir(&self) -> Option<&Path> {
        self.installed_dir.as_deref()
    }

    /// Get the directory where new registry installs should be written.
    ///
    /// Returns the installed_dir if configured (preferred), otherwise falls
    /// back to user_dir. In practice, the installed_dir is always set when
    /// the app is running; the fallback exists for test registries.
    pub fn install_target_dir(&self) -> &Path {
        self.installed_dir.as_deref().unwrap_or(&self.user_dir)
    }
}

fn publish_staged_skill_directory(
    install_root: &Path,
    stage_dir: &Path,
    target_dir: &Path,
) -> std::io::Result<()> {
    use fs4::FileExt;

    let root_metadata = std::fs::symlink_metadata(install_root)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(std::io::Error::other(
            "skill install root is not a real directory",
        ));
    }
    let stage_metadata = std::fs::symlink_metadata(stage_dir)?;
    if stage_metadata.file_type().is_symlink() || !stage_metadata.is_dir() {
        return Err(std::io::Error::other(
            "staged skill is not a real directory",
        ));
    }

    let lock_path = install_root.join(".thinclaw-skill-install.lock");
    let mut lock_options = std::fs::OpenOptions::new();
    lock_options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        lock_options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        lock_options
            .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let lock_file = lock_options.open(&lock_path)?;
    if !lock_file.metadata()?.is_file() {
        return Err(std::io::Error::other(
            "skill install lock is not a regular file",
        ));
    }
    FileExt::lock(&lock_file)?;

    match std::fs::symlink_metadata(target_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return thinclaw_platform::rename_no_replace(stage_dir, target_dir);
        }
        Err(error) => return Err(error),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(std::io::Error::other(
                "skill install target is not a real directory",
            ));
        }
        Ok(_) => {}
    }

    let backup_dir = install_root.join(format!(
        ".thinclaw-skill-backup-{}",
        uuid::Uuid::new_v4().simple()
    ));
    thinclaw_platform::rename_no_replace(target_dir, &backup_dir)?;
    if let Err(publish_error) = thinclaw_platform::rename_no_replace(stage_dir, target_dir) {
        return match thinclaw_platform::rename_no_replace(&backup_dir, target_dir) {
            Ok(()) => Err(publish_error),
            Err(rollback_error) => Err(std::io::Error::other(format!(
                "skill publication failed ({publish_error}) and rollback failed ({rollback_error})"
            ))),
        };
    }
    if let Err(error) = std::fs::remove_dir_all(&backup_dir) {
        tracing::warn!(
            path = %backup_dir.display(),
            error = %error,
            "Installed skill but could not remove the previous staged backup"
        );
    }
    Ok(())
}

/// Load and validate a single SKILL.md file from disk.
///
/// Shared implementation used by both `SkillRegistry::load_skill_md` (discovery)
/// and `SkillRegistry::prepare_install_to_disk` (installation). This avoids
/// duplicating the read/parse/validate/hash pipeline.
async fn load_and_validate_skill(
    path: &Path,
    trust: SkillTrust,
    source: SkillSource,
) -> Result<(String, LoadedSkill), SkillRegistryError> {
    // Check for symlink at the file level
    let file_meta =
        tokio::fs::symlink_metadata(path)
            .await
            .map_err(|e| SkillRegistryError::ReadError {
                path: path.display().to_string(),
                reason: e.to_string(),
            })?;

    if file_meta.is_symlink() {
        return Err(SkillRegistryError::SymlinkDetected {
            path: path.display().to_string(),
        });
    }

    if !file_meta.is_file() {
        return Err(SkillRegistryError::ReadError {
            path: path.display().to_string(),
            reason: "skill path is not a regular file".to_string(),
        });
    }

    if file_meta.len() > MAX_PROMPT_FILE_SIZE {
        return Err(SkillRegistryError::FileTooLarge {
            name: path.display().to_string(),
            size: file_meta.len(),
            max: MAX_PROMPT_FILE_SIZE,
        });
    }

    // Read and check size
    let raw_bytes = thinclaw_platform::read_regular_file_bounded_single_link_async(
        path.to_path_buf(),
        MAX_PROMPT_FILE_SIZE,
    )
    .await
    .map_err(|e| SkillRegistryError::ReadError {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;

    let raw_content = String::from_utf8(raw_bytes).map_err(|e| SkillRegistryError::ReadError {
        path: path.display().to_string(),
        reason: format!("Invalid UTF-8: {}", e),
    })?;

    // Normalize line endings before parsing to handle CRLF
    let normalized_content = normalize_line_endings(&raw_content);

    validate_normalized_skill_content(
        &path.display().to_string(),
        &normalized_content,
        trust,
        source,
    )
    .await
}

async fn validate_normalized_skill_content(
    error_name: &str,
    normalized_content: &str,
    trust: SkillTrust,
    source: SkillSource,
) -> Result<(String, LoadedSkill), SkillRegistryError> {
    // Parse SKILL.md
    let parsed = parse_skill_md(normalized_content).map_err(|e: SkillParseError| match e {
        SkillParseError::InvalidName { ref name } => SkillRegistryError::ParseError {
            name: name.clone(),
            reason: e.to_string(),
        },
        _ => SkillRegistryError::ParseError {
            name: error_name.to_string(),
            reason: e.to_string(),
        },
    })?;

    let manifest = parsed.manifest;
    let prompt_content = parsed.prompt_content;

    // Check gating requirements
    if let Some(ref meta) = manifest.metadata
        && let Some(ref openclaw) = meta.openclaw
    {
        let result = gating::check_requirements(&openclaw.requires).await;
        if !result.passed {
            return Err(SkillRegistryError::GatingFailed {
                name: manifest.name.clone(),
                reason: result.failures.join("; "),
            });
        }
    }

    // Check token budget (reject if prompt is > 2x declared budget)
    // ~4 bytes per token for English prose = ~0.25 tokens per byte
    let approx_tokens = (prompt_content.len() as f64 * 0.25) as usize;
    let declared = manifest.activation.max_context_tokens;
    if declared > 0 && approx_tokens > declared * 2 {
        return Err(SkillRegistryError::TokenBudgetExceeded {
            name: manifest.name.clone(),
            approx_tokens,
            declared,
        });
    }

    // Compute content hash
    let content_hash = compute_hash(&prompt_content);
    let source_tier = source_tier_for_skill(&manifest, trust, &source);

    // Compile regex patterns
    let compiled_patterns = LoadedSkill::compile_patterns(&manifest.activation.patterns);

    // Pre-compute lowercased keywords and tags for efficient scoring
    let lowercased_keywords = manifest
        .activation
        .keywords
        .iter()
        .map(|k| k.to_lowercase())
        .collect();
    let lowercased_tags = manifest
        .activation
        .tags
        .iter()
        .map(|t| t.to_lowercase())
        .collect();
    // Pre-compute lowercased description words for broad semantic matching.
    // Filter out short words (< 3 chars) to avoid noisy matches.
    let lowercased_description_words: Vec<String> = manifest
        .description
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| w.len() >= 3)
        .collect();

    let name = manifest.name.clone();
    let skill = LoadedSkill {
        manifest,
        prompt_content,
        trust,
        source,
        source_tier,
        content_hash,
        compiled_patterns,
        lowercased_keywords,
        lowercased_tags,
        lowercased_description_words,
    };

    Ok((name, skill))
}

fn source_tier_for_skill(
    manifest: &crate::skills::SkillManifest,
    trust: SkillTrust,
    source: &SkillSource,
) -> SkillSourceTier {
    if let Some(provenance) = manifest
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.openclaw.as_ref())
        .and_then(|openclaw| openclaw.provenance.as_deref())
    {
        match provenance.trim().to_ascii_lowercase().as_str() {
            "builtin" => return SkillSourceTier::Builtin,
            "official" => return SkillSourceTier::Official,
            "trusted" => return SkillSourceTier::Trusted,
            "unvetted" => return SkillSourceTier::Unvetted,
            "community" | "generated" => return SkillSourceTier::Community,
            _ => {}
        }
    }

    match source {
        SkillSource::Bundled(_) => SkillSourceTier::Builtin,
        SkillSource::Workspace(_) | SkillSource::User(_) if trust == SkillTrust::Trusted => {
            SkillSourceTier::Trusted
        }
        SkillSource::External(_) => SkillSourceTier::Community,
        _ => SkillSourceTier::Community,
    }
}

/// Compute SHA-256 hash of content in the format "sha256:hex...".
pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("sha256:{}", hex::encode(result))
}

/// Helper to check gating for a `GatingRequirements`. Useful for callers that
/// don't have the full skill loaded yet.
pub async fn check_gating(
    requirements: &GatingRequirements,
) -> crate::skills::gating::GatingResult {
    gating::check_requirements(requirements).await
}

#[cfg(test)]
mod tests;
