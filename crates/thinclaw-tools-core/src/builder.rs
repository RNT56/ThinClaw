//! Core builder configuration shared by root adapters and config crates.

use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Configuration for the software builder.
#[derive(Debug, Clone)]
pub struct BuilderConfig {
    /// Directory where builds happen.
    pub build_dir: PathBuf,
    /// Maximum iterations before giving up.
    pub max_iterations: u32,
    /// Timeout for the entire build.
    pub timeout: Duration,
    /// Whether to clean up failed builds.
    pub cleanup_on_failure: bool,
    /// Whether to validate WASM tools after building.
    pub validate_wasm: bool,
    /// Whether to run tests after building.
    pub run_tests: bool,
    /// Whether to auto-register successful WASM tool builds.
    pub auto_register: bool,
    /// Directory to copy successful WASM tools for persistence.
    pub wasm_output_dir: Option<PathBuf>,
}

impl Default for BuilderConfig {
    fn default() -> Self {
        Self {
            build_dir: std::env::temp_dir().join("thinclaw-builds"),
            max_iterations: 10,
            timeout: Duration::from_secs(600),
            cleanup_on_failure: false,
            validate_wasm: true,
            run_tests: true,
            auto_register: true,
            wasm_output_dir: None,
        }
    }
}

/// Requirement specification for building software.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildRequirement {
    /// Name for the software.
    pub name: String,
    /// Description of what it should do.
    pub description: String,
    /// Type of software to build.
    pub software_type: SoftwareType,
    /// Target language/runtime.
    pub language: Language,
    /// Expected input format (for tools/CLIs).
    pub input_spec: Option<String>,
    /// Expected output format.
    pub output_spec: Option<String>,
    /// External dependencies needed.
    pub dependencies: Vec<String>,
    /// Security/capability requirements (for WASM tools).
    pub capabilities: Vec<String>,
}

/// Type of software being built.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SoftwareType {
    /// A WASM tool for the agent.
    WasmTool,
    /// A standalone CLI application.
    CliBinary,
    /// A library/crate.
    Library,
    /// A script (Python, Bash, etc.).
    Script,
    /// A web service/API.
    WebService,
}

/// Programming language for the build.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Rust,
    Python,
    #[serde(alias = "typescript")]
    TypeScript,
    #[serde(alias = "javascript")]
    JavaScript,
    Go,
    Bash,
}

impl Language {
    /// Get the file extension for this language.
    pub fn extension(&self) -> &'static str {
        match self {
            Language::Rust => "rs",
            Language::Python => "py",
            Language::TypeScript => "ts",
            Language::JavaScript => "js",
            Language::Go => "go",
            Language::Bash => "sh",
        }
    }

    /// Get the build command for this language.
    pub fn build_command(&self, project_dir: &str) -> Option<String> {
        match self {
            Language::Rust => Some(format!("cd {} && cargo build --release", project_dir)),
            Language::TypeScript => Some(format!("cd {} && npm run build", project_dir)),
            Language::Go => Some(format!("cd {} && go build ./...", project_dir)),
            Language::Python | Language::JavaScript | Language::Bash => None,
        }
    }

    /// Get the test command for this language.
    pub fn test_command(&self, project_dir: &str) -> String {
        match self {
            Language::Rust => format!("cd {} && cargo test", project_dir),
            Language::Python => format!("cd {} && python -m pytest", project_dir),
            Language::TypeScript | Language::JavaScript => {
                format!("cd {} && npm test", project_dir)
            }
            Language::Go => format!("cd {} && go test ./...", project_dir),
            Language::Bash => format!("cd {} && shellcheck *.sh", project_dir),
        }
    }
}

/// Result of a build operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildResult {
    /// Unique ID for this build.
    pub build_id: Uuid,
    /// The requirement that was built.
    pub requirement: BuildRequirement,
    /// Path to the output artifact.
    pub artifact_path: PathBuf,
    /// Build logs.
    pub logs: Vec<BuildLog>,
    /// Whether the build succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
    /// When the build started.
    pub started_at: DateTime<Utc>,
    /// When the build completed.
    pub completed_at: DateTime<Utc>,
    /// Number of iterations to complete.
    pub iterations: u32,
    /// Validation warnings (for WASM tools).
    #[serde(default)]
    pub validation_warnings: Vec<String>,
    /// Test results summary.
    #[serde(default)]
    pub tests_passed: u32,
    /// Number of tests that failed.
    #[serde(default)]
    pub tests_failed: u32,
    /// Whether the tool was auto-registered (for WASM tools).
    #[serde(default)]
    pub registered: bool,
}

/// A log entry from the build process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildLog {
    pub timestamp: DateTime<Utc>,
    pub phase: BuildPhase,
    pub message: String,
    pub details: Option<String>,
}

/// Phases of the build process.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BuildPhase {
    Analyzing,
    Scaffolding,
    Implementing,
    Building,
    Testing,
    Fixing,
    Validating,
    Registering,
    Packaging,
    Complete,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::{Language, SoftwareType};

    #[test]
    fn language_extensions_are_stable() {
        assert_eq!(Language::Rust.extension(), "rs");
        assert_eq!(Language::Python.extension(), "py");
        assert_eq!(Language::TypeScript.extension(), "ts");
    }

    #[test]
    fn language_build_commands_cover_compiled_runtimes() {
        assert!(Language::Rust.build_command("/tmp/project").is_some());
        assert!(Language::Python.build_command("/tmp/project").is_none());
    }

    #[test]
    fn software_type_serialization_preserves_public_shape() {
        let json = serde_json::to_string(&SoftwareType::WasmTool).unwrap();
        assert_eq!(json, "\"wasm_tool\"");
    }
}
