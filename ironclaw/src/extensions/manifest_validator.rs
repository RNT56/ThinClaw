//! Plugin manifest validation.
//!
//! Validates `PluginInfo` entries before installation: checks required
//! fields, semver format, permission sets, and URL validity.

use serde::{Deserialize, Serialize};

/// Known valid permissions.
const KNOWN_PERMISSIONS: &[&str] = &[
    "network",
    "filesystem",
    "env",
    "exec",
    "secrets",
    "memory",
    "tools",
    "channels",
    "config",
    "http",
    "websocket",
];

/// Permissions considered high-privilege.
const HIGH_PRIVILEGE: &[&str] = &["exec", "secrets", "filesystem"];

/// Manifest validator.
#[derive(Debug, Clone)]
pub struct ManifestValidator {
    /// Whether to fail on warnings (strict mode).
    pub strict: bool,
    /// Maximum number of permissions allowed.
    pub max_permissions: usize,
    /// Maximum name length.
    pub max_name_length: usize,
}

impl Default for ManifestValidator {
    fn default() -> Self {
        Self {
            strict: false,
            max_permissions: 10,
            max_name_length: 64,
        }
    }
}

impl ManifestValidator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn strict() -> Self {
        Self {
            strict: true,
            ..Default::default()
        }
    }

    /// Validate a plugin info entry.
    pub fn validate(&self, info: &PluginInfoRef) -> ValidationResult {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Required fields
        if info.name.is_empty() {
            errors.push(ValidationError::MissingField("name".into()));
        }
        if info.name.len() > self.max_name_length {
            errors.push(ValidationError::NameTooLong {
                name: info.name.clone(),
                max: self.max_name_length,
            });
        }
        if let Some(v) = &info.version
            && !is_valid_semver(v)
        {
            errors.push(ValidationError::InvalidSemver(v.clone()));
        }

        // Description
        if info.description.is_none() {
            warnings.push(ValidationWarning::MissingDescription);
        }

        // Keywords
        if info.keywords.is_empty() {
            warnings.push(ValidationWarning::NoKeywords);
        }

        // Permissions
        if info.permissions.len() > self.max_permissions {
            errors.push(ValidationError::TooManyPermissions {
                count: info.permissions.len(),
                max: self.max_permissions,
            });
        }
        for perm in &info.permissions {
            if !KNOWN_PERMISSIONS.contains(&perm.as_str()) {
                errors.push(ValidationError::UnknownPermission(perm.clone()));
            }
            if HIGH_PRIVILEGE.contains(&perm.as_str()) {
                warnings.push(ValidationWarning::HighPrivilege(perm.clone()));
            }
        }

        // URLs
        if let Some(url) = &info.homepage_url
            && !url.starts_with("http://")
            && !url.starts_with("https://")
        {
            errors.push(ValidationError::InvalidUrl(url.clone()));
        }

        let valid = errors.is_empty() && (!self.strict || warnings.is_empty());

        ValidationResult {
            valid,
            errors,
            warnings,
        }
    }
}

/// Lightweight representation of plugin info for validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfoRef {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub keywords: Vec<String>,
    pub permissions: Vec<String>,
    pub homepage_url: Option<String>,
}

/// Validation result.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
}

impl ValidationResult {
    pub fn is_ok(&self) -> bool {
        self.valid
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// Convert to a serializable response for the `openclaw_manifest_validate` Tauri command.
    ///
    /// Returns `{ errors: Vec<String>, warnings: Vec<String> }` matching §17.4.
    pub fn to_response(&self) -> ValidationResponse {
        ValidationResponse {
            valid: self.valid,
            errors: self.errors.iter().map(|e| e.to_string()).collect(),
            warnings: self.warnings.iter().map(|w| w.to_string()).collect(),
        }
    }
}

/// Serializable validation response for `openclaw_manifest_validate`.
///
/// Matches §17.4 integration contract: `{ errors: Vec<String>, warnings: Vec<String> }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResponse {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Validation errors.
#[derive(Debug, Clone)]
pub enum ValidationError {
    MissingField(String),
    InvalidSemver(String),
    UnknownPermission(String),
    InvalidUrl(String),
    NameTooLong { name: String, max: usize },
    TooManyPermissions { count: usize, max: usize },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing required field: {}", field),
            Self::InvalidSemver(v) => write!(f, "invalid semver: {}", v),
            Self::UnknownPermission(p) => write!(f, "unknown permission: {}", p),
            Self::InvalidUrl(u) => write!(f, "invalid URL: {}", u),
            Self::NameTooLong { name, max } => {
                write!(f, "name '{}' exceeds max length {}", name, max)
            }
            Self::TooManyPermissions { count, max } => {
                write!(f, "too many permissions: {} (max {})", count, max)
            }
        }
    }
}

/// Validation warnings.
#[derive(Debug, Clone)]
pub enum ValidationWarning {
    MissingDescription,
    NoKeywords,
    HighPrivilege(String),
    DeprecatedField(String),
}

impl std::fmt::Display for ValidationWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingDescription => write!(f, "missing description"),
            Self::NoKeywords => write!(f, "no keywords specified"),
            Self::HighPrivilege(p) => write!(f, "high-privilege permission: {}", p),
            Self::DeprecatedField(field) => write!(f, "deprecated field: {}", field),
        }
    }
}

/// Simple semver check: major.minor.patch
fn is_valid_semver(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    parts.iter().all(|p| p.parse::<u32>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_info() -> PluginInfoRef {
        PluginInfoRef {
            name: "test-plugin".into(),
            version: Some("1.0.0".into()),
            description: Some("A test plugin".into()),
            keywords: vec!["test".into()],
            permissions: vec!["network".into()],
            homepage_url: Some("https://example.com".into()),
        }
    }

    #[test]
    fn test_valid_manifest() {
        let v = ManifestValidator::new();
        let result = v.validate(&valid_info());
        assert!(result.valid);
        assert!(!result.has_errors());
    }

    #[test]
    fn test_missing_name() {
        let v = ManifestValidator::new();
        let mut info = valid_info();
        info.name = "".into();
        let result = v.validate(&info);
        assert!(result.has_errors());
        assert!(!result.valid);
    }

    #[test]
    fn test_invalid_semver() {
        let v = ManifestValidator::new();
        let mut info = valid_info();
        info.version = Some("not-a-version".into());
        let result = v.validate(&info);
        assert!(result.has_errors());
    }

    #[test]
    fn test_unknown_permission() {
        let v = ManifestValidator::new();
        let mut info = valid_info();
        info.permissions = vec!["hacktheplanet".into()];
        let result = v.validate(&info);
        assert!(result.has_errors());
    }

    #[test]
    fn test_strict_mode_fails_on_warnings() {
        let v = ManifestValidator::strict();
        let mut info = valid_info();
        info.description = None; // triggers MissingDescription warning
        let result = v.validate(&info);
        assert!(!result.valid);
        assert!(result.has_warnings());
    }

    #[test]
    fn test_high_privilege_warning() {
        let v = ManifestValidator::new();
        let mut info = valid_info();
        info.permissions = vec!["exec".into()];
        let result = v.validate(&info);
        assert!(result.valid); // Not an error, just a warning
        assert!(result.has_warnings());
    }

    #[test]
    fn test_too_many_permissions() {
        let v = ManifestValidator {
            max_permissions: 2,
            ..Default::default()
        };
        let mut info = valid_info();
        info.permissions = vec!["network".into(), "http".into(), "env".into()];
        let result = v.validate(&info);
        assert!(result.has_errors());
    }

    #[test]
    fn test_invalid_url() {
        let v = ManifestValidator::new();
        let mut info = valid_info();
        info.homepage_url = Some("ftp://bad.com".into());
        let result = v.validate(&info);
        assert!(result.has_errors());
    }

    #[test]
    fn test_to_response_valid() {
        let v = ManifestValidator::new();
        let result = v.validate(&valid_info());
        let response = result.to_response();
        assert!(response.valid);
        assert!(response.errors.is_empty());
    }

    #[test]
    fn test_to_response_with_errors() {
        let v = ManifestValidator::new();
        let mut info = valid_info();
        info.name = "".into();
        info.version = Some("bad".into());
        let result = v.validate(&info);
        let response = result.to_response();
        assert!(!response.valid);
        assert!(response.errors.len() >= 2);
        assert!(response.errors[0].contains("missing required field"));
    }

    #[test]
    fn test_to_response_serializable() {
        let v = ManifestValidator::new();
        let result = v.validate(&valid_info());
        let response = result.to_response();
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"valid\":true"));
    }

    #[test]
    fn test_validation_error_display() {
        assert_eq!(
            ValidationError::MissingField("name".into()).to_string(),
            "missing required field: name"
        );
        assert_eq!(
            ValidationError::InvalidSemver("bad".into()).to_string(),
            "invalid semver: bad"
        );
    }

    #[test]
    fn test_validation_warning_display() {
        assert_eq!(
            ValidationWarning::MissingDescription.to_string(),
            "missing description"
        );
        assert_eq!(
            ValidationWarning::HighPrivilege("exec".into()).to_string(),
            "high-privilege permission: exec"
        );
    }
}
