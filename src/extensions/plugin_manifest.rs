//! Persistent plugin manifest for tracking installed extensions/plugins.
//!
//! The manifest records which plugins are installed, their versions,
//! activation state, and source information. It is persisted to disk
//! as JSON and loaded on startup.
//!
//! ```text
//! ~/.thinclaw/plugins.json
//! ┌──────────────────────────────────────────┐
//! │ {                                        │
//! │   "version": 1,                          │
//! │   "plugins": {                           │
//! │     "github": { ... },                   │
//! │     "slack": { ... },                    │
//! │   }                                      │
//! │ }                                        │
//! └──────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Current manifest format version.
const MANIFEST_VERSION: u32 = 1;

/// Persistent plugin manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Format version for forward compatibility.
    pub version: u32,
    /// Map of plugin name → plugin info.
    pub plugins: HashMap<String, PluginInfo>,
}

/// Information about a single installed plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    /// Plugin name (unique identifier).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Version string (semver or freeform).
    pub version: String,
    /// Source where the plugin was installed from.
    pub source: PluginSource,
    /// Extension kind (wasm, mcp, channel, etc.).
    pub kind: String,
    /// Whether the plugin should auto-activate on startup.
    pub auto_activate: bool,
    /// When the plugin was first installed.
    pub installed_at: DateTime<Utc>,
    /// When the plugin was last updated.
    pub updated_at: DateTime<Utc>,
    /// Whether the plugin is currently enabled.
    pub enabled: bool,
    /// Optional trust level override.
    pub trust_level: Option<String>,
    /// Plugin-specific configuration (opaque JSON).
    #[serde(default)]
    pub config: serde_json::Value,
}

/// Where a plugin was sourced from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginSource {
    /// Installed from ClawHub registry.
    Registry {
        /// Registry slug (e.g. "nearai/github-tools").
        slug: String,
        /// SHA-256 hash of the downloaded artifact.
        artifact_hash: Option<String>,
    },
    /// Installed from a local path.
    Local {
        /// Absolute path to the plugin.
        path: String,
    },
    /// Installed from a URL.
    Url {
        /// Download URL.
        url: String,
    },
    /// Builtin extension (ships with thinclaw).
    Builtin,
}

impl PluginManifest {
    /// Create a new empty manifest.
    pub fn new() -> Self {
        Self {
            version: MANIFEST_VERSION,
            plugins: HashMap::new(),
        }
    }

    /// Load a manifest from a JSON file.
    ///
    /// Returns a new empty manifest if the file doesn't exist.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        if !path.exists() {
            return Ok(Self::new());
        }

        let content = std::fs::read_to_string(path).map_err(|e| ManifestError::Io {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;

        let manifest: Self = serde_json::from_str(&content).map_err(|e| ManifestError::Parse {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;

        if manifest.version > MANIFEST_VERSION {
            return Err(ManifestError::UnsupportedVersion {
                found: manifest.version,
                max: MANIFEST_VERSION,
            });
        }

        Ok(manifest)
    }

    /// Save the manifest to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), ManifestError> {
        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ManifestError::Io {
                path: parent.to_path_buf(),
                reason: e.to_string(),
            })?;
        }

        let content = serde_json::to_string_pretty(self).map_err(|e| ManifestError::Parse {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;

        // Write atomically via temp file + rename.
        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &content).map_err(|e| ManifestError::Io {
            path: tmp_path.clone(),
            reason: e.to_string(),
        })?;

        std::fs::rename(&tmp_path, path).map_err(|e| ManifestError::Io {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;

        Ok(())
    }

    /// Install or update a plugin in the manifest.
    pub fn install(&mut self, info: PluginInfo) {
        self.plugins.insert(info.name.clone(), info);
    }

    /// Remove a plugin from the manifest. Returns the removed info if present.
    pub fn remove(&mut self, name: &str) -> Option<PluginInfo> {
        self.plugins.remove(name)
    }

    /// Get a plugin by name.
    pub fn get(&self, name: &str) -> Option<&PluginInfo> {
        self.plugins.get(name)
    }

    /// Get a mutable reference to a plugin by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut PluginInfo> {
        self.plugins.get_mut(name)
    }

    /// List all installed plugins.
    pub fn list(&self) -> Vec<&PluginInfo> {
        let mut plugins: Vec<_> = self.plugins.values().collect();
        plugins.sort_by_key(|p| &p.name);
        plugins
    }

    /// List enabled plugins.
    pub fn enabled(&self) -> Vec<&PluginInfo> {
        self.list().into_iter().filter(|p| p.enabled).collect()
    }

    /// Toggle a plugin's enabled state. Returns the new state.
    pub fn toggle(&mut self, name: &str) -> Option<bool> {
        self.plugins.get_mut(name).map(|p| {
            p.enabled = !p.enabled;
            p.updated_at = Utc::now();
            p.enabled
        })
    }
}

impl Default for PluginManifest {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors for manifest operations.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("I/O error at {path}: {reason}")]
    Io { path: PathBuf, reason: String },

    #[error("Parse error at {path}: {reason}")]
    Parse { path: PathBuf, reason: String },

    #[error("Unsupported manifest version {found} (max supported: {max})")]
    UnsupportedVersion { found: u32, max: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_plugin(name: &str) -> PluginInfo {
        PluginInfo {
            name: name.to_string(),
            description: format!("{} plugin", name),
            version: "1.0.0".to_string(),
            source: PluginSource::Builtin,
            kind: "wasm".to_string(),
            auto_activate: true,
            installed_at: Utc::now(),
            updated_at: Utc::now(),
            enabled: true,
            trust_level: None,
            config: serde_json::json!({}),
        }
    }

    #[test]
    fn test_manifest_new_empty() {
        let manifest = PluginManifest::new();
        assert_eq!(manifest.version, MANIFEST_VERSION);
        assert!(manifest.plugins.is_empty());
    }

    #[test]
    fn test_install_and_get() {
        let mut manifest = PluginManifest::new();
        manifest.install(make_plugin("github"));

        let plugin = manifest.get("github").unwrap();
        assert_eq!(plugin.name, "github");
        assert_eq!(plugin.version, "1.0.0");
    }

    #[test]
    fn test_remove() {
        let mut manifest = PluginManifest::new();
        manifest.install(make_plugin("github"));
        assert!(manifest.get("github").is_some());

        let removed = manifest.remove("github");
        assert!(removed.is_some());
        assert!(manifest.get("github").is_none());
    }

    #[test]
    fn test_toggle() {
        let mut manifest = PluginManifest::new();
        manifest.install(make_plugin("github"));

        let new_state = manifest.toggle("github").unwrap();
        assert!(!new_state);
        assert!(!manifest.get("github").unwrap().enabled);

        let new_state = manifest.toggle("github").unwrap();
        assert!(new_state);
    }

    #[test]
    fn test_list_sorted() {
        let mut manifest = PluginManifest::new();
        manifest.install(make_plugin("slack"));
        manifest.install(make_plugin("github"));
        manifest.install(make_plugin("calendar"));

        let names: Vec<_> = manifest.list().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["calendar", "github", "slack"]);
    }

    #[test]
    fn test_enabled_filter() {
        let mut manifest = PluginManifest::new();
        manifest.install(make_plugin("github"));

        let mut disabled = make_plugin("slack");
        disabled.enabled = false;
        manifest.install(disabled);

        let enabled = manifest.enabled();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "github");
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.json");

        let mut manifest = PluginManifest::new();
        manifest.install(make_plugin("github"));
        manifest.save(&path).unwrap();

        let loaded = PluginManifest::load(&path).unwrap();
        assert_eq!(loaded.plugins.len(), 1);
        assert_eq!(loaded.get("github").unwrap().name, "github");
    }

    #[test]
    fn test_load_nonexistent_returns_empty() {
        let path = Path::new("/tmp/nonexistent_manifest_12345.json");
        let manifest = PluginManifest::load(path).unwrap();
        assert!(manifest.plugins.is_empty());
    }

    #[test]
    fn test_plugin_source_serde() {
        let sources = vec![
            PluginSource::Registry {
                slug: "nearai/tools".to_string(),
                artifact_hash: Some("abc123".to_string()),
            },
            PluginSource::Local {
                path: "/tmp/plugin.wasm".to_string(),
            },
            PluginSource::Url {
                url: "https://example.com/plugin.wasm".to_string(),
            },
            PluginSource::Builtin,
        ];

        for source in sources {
            let json = serde_json::to_string(&source).unwrap();
            let parsed: PluginSource = serde_json::from_str(&json).unwrap();
            // Ensure roundtrip works (just check it doesn't panic).
            let _ = format!("{:?}", parsed);
        }
    }
}
