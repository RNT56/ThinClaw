//! Skill download path restriction — security hardening.
//!
//! Restricts where skills can be downloaded/stored to prevent
//! path traversal attacks and unauthorized file placement.
//!
//! Configuration:
//! - `SKILL_DOWNLOAD_DIR` — allowed base directory for skill downloads
//!   (default: `$HOME/.ironclaw/skills`)
//! - `SKILL_ALLOW_SYMLINKS` — whether to allow symlinks in skill paths (default: false)

use std::path::{Path, PathBuf};

/// Validation errors for skill paths.
#[derive(Debug, Clone, PartialEq)]
pub enum SkillPathError {
    /// Path escapes the allowed base directory.
    PathTraversal { attempted: String, base: String },
    /// Path contains a symlink when not allowed.
    SymlinkDetected { path: String },
    /// Path is not valid UTF-8.
    InvalidUtf8,
    /// Base directory doesn't exist and couldn't be created.
    BaseDirUnavailable { path: String, reason: String },
}

impl std::fmt::Display for SkillPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillPathError::PathTraversal { attempted, base } => {
                write!(
                    f,
                    "Path traversal blocked: '{}' escapes base dir '{}'",
                    attempted, base
                )
            }
            SkillPathError::SymlinkDetected { path } => {
                write!(f, "Symlink detected in skill path: '{}'", path)
            }
            SkillPathError::InvalidUtf8 => write!(f, "Skill path contains invalid UTF-8"),
            SkillPathError::BaseDirUnavailable { path, reason } => {
                write!(f, "Skill base dir '{}' unavailable: {}", path, reason)
            }
        }
    }
}

impl std::error::Error for SkillPathError {}

/// Configuration for skill path restrictions.
#[derive(Debug, Clone)]
pub struct SkillPathConfig {
    /// Allowed base directory for skill downloads.
    pub base_dir: PathBuf,
    /// Whether to allow symlinks in resolved paths.
    pub allow_symlinks: bool,
}

impl Default for SkillPathConfig {
    fn default() -> Self {
        let base_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ironclaw")
            .join("skills");

        Self {
            base_dir,
            allow_symlinks: false,
        }
    }
}

impl SkillPathConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(dir) = std::env::var("SKILL_DOWNLOAD_DIR") {
            config.base_dir = PathBuf::from(dir);
        }

        if let Ok(val) = std::env::var("SKILL_ALLOW_SYMLINKS") {
            config.allow_symlinks = val == "1" || val.eq_ignore_ascii_case("true");
        }

        config
    }

    /// Ensure the base directory exists.
    pub fn ensure_base_dir(&self) -> Result<(), SkillPathError> {
        std::fs::create_dir_all(&self.base_dir).map_err(|e| SkillPathError::BaseDirUnavailable {
            path: self.base_dir.display().to_string(),
            reason: e.to_string(),
        })
    }

    /// Validate that a path is safe to use for skill storage.
    ///
    /// Checks:
    /// 1. Path resolves within the base directory (no traversal)
    /// 2. No symlinks (unless explicitly allowed)
    /// 3. Valid UTF-8
    pub fn validate_path(&self, path: &Path) -> Result<PathBuf, SkillPathError> {
        // Build the full path relative to the base dir
        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base_dir.join(path)
        };

        // Check UTF-8
        if full_path.to_str().is_none() {
            return Err(SkillPathError::InvalidUtf8);
        }

        // Normalize path components to catch ".." traversal
        let normalized = normalize_path(&full_path);
        let base_normalized = normalize_path(&self.base_dir);

        if !normalized.starts_with(&base_normalized) {
            return Err(SkillPathError::PathTraversal {
                attempted: full_path.display().to_string(),
                base: self.base_dir.display().to_string(),
            });
        }

        // Check for symlinks
        if !self.allow_symlinks && full_path.exists() {
            let metadata = std::fs::symlink_metadata(&full_path);
            if let Ok(meta) = metadata
                && meta.file_type().is_symlink()
            {
                return Err(SkillPathError::SymlinkDetected {
                    path: full_path.display().to_string(),
                });
            }
        }

        Ok(normalized)
    }

    /// Build a safe path for a skill by name. Returns the validated full path.
    pub fn skill_path(&self, skill_name: &str) -> Result<PathBuf, SkillPathError> {
        // Sanitize skill name — only allow alphanumeric, dash, underscore, dot
        let sanitized: String = skill_name
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
            .collect();

        if sanitized.is_empty() || sanitized.starts_with('.') {
            return Err(SkillPathError::PathTraversal {
                attempted: skill_name.to_string(),
                base: self.base_dir.display().to_string(),
            });
        }

        self.validate_path(Path::new(&sanitized))
    }
}

/// Normalize a path by resolving `.` and `..` without touching the filesystem.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();

    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            other => components.push(other),
        }
    }

    components.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SkillPathConfig {
        SkillPathConfig {
            base_dir: PathBuf::from("/tmp/ironclaw_test/skills"),
            allow_symlinks: false,
        }
    }

    #[test]
    fn test_valid_relative_path() {
        let config = test_config();
        let result = config.validate_path(Path::new("my-skill"));
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.starts_with("/tmp/ironclaw_test/skills"));
    }

    #[test]
    fn test_path_traversal_blocked() {
        let config = test_config();
        let result = config.validate_path(Path::new("../../etc/passwd"));
        assert!(matches!(result, Err(SkillPathError::PathTraversal { .. })));
    }

    #[test]
    fn test_absolute_path_outside_base() {
        let config = test_config();
        let result = config.validate_path(Path::new("/etc/passwd"));
        assert!(matches!(result, Err(SkillPathError::PathTraversal { .. })));
    }

    #[test]
    fn test_absolute_path_inside_base() {
        let config = test_config();
        let result = config.validate_path(Path::new("/tmp/ironclaw_test/skills/safe-skill"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_skill_path_sanitization() {
        let config = test_config();

        // Normal skill name
        let result = config.skill_path("my-cool-skill");
        assert!(result.is_ok());

        // Name with special chars gets sanitized — slashes removed, leaving just "evil"
        let result = config.skill_path("../evil");
        // After sanitization, ".." becomes ".." (dots kept), "/" removed = "..evil"
        // "..evil" starts with "." so it's rejected
        assert!(matches!(result, Err(SkillPathError::PathTraversal { .. })));

        // Pure traversal attempt
        let result = config.skill_path("../../etc/passwd");
        // After sanitization: "....etcpasswd", starts with "." → rejected
        assert!(matches!(result, Err(SkillPathError::PathTraversal { .. })));

        // Normal name with allowed chars
        let result = config.skill_path("web-search_v2.1");
        assert!(result.is_ok());
    }

    #[test]
    fn test_skill_path_dotfile_rejected() {
        let config = test_config();
        let result = config.skill_path(".hidden");
        assert!(matches!(result, Err(SkillPathError::PathTraversal { .. })));
    }

    #[test]
    fn test_skill_path_empty_name_rejected() {
        let config = test_config();
        let result = config.skill_path("");
        assert!(matches!(result, Err(SkillPathError::PathTraversal { .. })));
    }

    #[test]
    fn test_normalize_path_parent_dir() {
        let normalized = normalize_path(Path::new("/a/b/../c"));
        assert_eq!(normalized, PathBuf::from("/a/c"));
    }

    #[test]
    fn test_normalize_path_current_dir() {
        let normalized = normalize_path(Path::new("/a/./b/./c"));
        assert_eq!(normalized, PathBuf::from("/a/b/c"));
    }

    #[test]
    fn test_error_display() {
        let err = SkillPathError::PathTraversal {
            attempted: "../evil".to_string(),
            base: "/safe".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("Path traversal blocked"));
    }
}
