//! Registry catalog: loads manifests from disk, provides list/search/resolve operations.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use crate::registry::embedded;
use crate::registry::manifest::{BundleDefinition, BundlesFile, ExtensionManifest, ManifestKind};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_BUNDLES_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MANIFESTS: usize = 4_096;
const MAX_BUNDLES: usize = 1_024;
const MAX_BUNDLE_EXTENSIONS: usize = 256;

/// Error type for registry operations.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("Registry directory not found: {0}")]
    DirectoryNotFound(PathBuf),

    #[error("Failed to read manifest {path}: {reason}")]
    ManifestRead { path: PathBuf, reason: String },

    #[error("Failed to parse manifest {path}: {reason}")]
    ManifestParse { path: PathBuf, reason: String },

    #[error("Extension not found: {0}")]
    ExtensionNotFound(String),

    #[error("'{name}' already installed at {path}. Use --force to overwrite.")]
    AlreadyInstalled {
        name: String,
        path: std::path::PathBuf,
    },

    // `url` is stored for programmatic access (logs, retries) but intentionally
    // omitted from the Display message to avoid leaking internal artifact URLs
    // to end users.
    #[error("Artifact download failed: {reason}")]
    DownloadFailed { url: String, reason: String },

    #[error("Invalid extension manifest for '{name}' field '{field}': {reason}")]
    InvalidManifest {
        name: String,
        field: &'static str,
        reason: String,
    },

    #[error("Checksum verification failed: expected {expected_sha256}, got {actual_sha256}")]
    ChecksumMismatch {
        url: String,
        expected_sha256: String,
        actual_sha256: String,
    },

    #[error(
        "Source fallback unavailable for '{name}' after artifact install failed. Retry artifact download or run from a repository checkout."
    )]
    SourceFallbackUnavailable {
        name: String,
        source_dir: PathBuf,
        artifact_error: Box<RegistryError>,
    },

    #[error("Artifact install and source fallback both failed for '{name}'.")]
    InstallFallbackFailed {
        name: String,
        artifact_error: Box<RegistryError>,
        source_error: Box<RegistryError>,
    },

    #[error(
        "Ambiguous name '{name}': exists as both {kind_a} and {kind_b}. Use '{prefix_a}/{name}' or '{prefix_b}/{name}'."
    )]
    AmbiguousName {
        name: String,
        kind_a: &'static str,
        prefix_a: &'static str,
        kind_b: &'static str,
        prefix_b: &'static str,
    },

    #[error("Bundle not found: {0}")]
    BundleNotFound(String),

    #[error("Failed to read bundles file: {0}")]
    BundlesRead(String),

    #[error(
        "Cannot install '{name}': no pre-built binary available and cargo-component is not installed.\n\
         Fix options:\n\
         \x20 1. Build with: cargo build --release --features bundled-wasm\n\
         \x20 2. Install toolchain: cargo install cargo-component\n\
         \x20 3. Wait for a release with pre-built artifacts"
    )]
    ToolchainMissing { name: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Central catalog loaded from the `registry/` directory.
#[derive(Debug, Clone)]
pub struct RegistryCatalog {
    /// All loaded manifests, keyed by "<kind>/<name>" (e.g. "tools/github").
    manifests: HashMap<String, ExtensionManifest>,

    /// Bundle definitions from `_bundles.json`.
    bundles: HashMap<String, BundleDefinition>,

    /// Root directory of the registry.
    root: PathBuf,
}

impl RegistryCatalog {
    /// Find the `registry/` directory by searching relative to cwd, the executable,
    /// and `CARGO_MANIFEST_DIR`. Returns `None` if the directory cannot be found
    /// (non-fatal at startup).
    pub fn find_dir() -> Option<PathBuf> {
        // Try relative to current directory (for dev usage)
        if let Ok(cwd) = std::env::current_dir() {
            let candidate = cwd.join("registry");
            if candidate.is_dir() {
                return Some(candidate);
            }
        }

        // Try relative to executable (covers installed binary, target/debug/, target/release/)
        if let Ok(exe) = std::env::current_exe()
            && let Some(parent) = exe.parent()
        {
            // Walk up to 3 levels: exe dir, parent (target/release -> target), grandparent (-> repo root)
            let mut dir = Some(parent);
            for _ in 0..3 {
                if let Some(d) = dir {
                    let candidate = d.join("registry");
                    if candidate.is_dir() {
                        return Some(candidate);
                    }
                    dir = d.parent();
                }
            }
        }

        // Try CARGO_MANIFEST_DIR (compile-time, works in dev builds)
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let candidate = manifest_dir.join("registry");
        if candidate.is_dir() {
            return Some(candidate);
        }

        None
    }

    /// Try to load from disk; if `registry/` cannot be found, fall back to
    /// manifests embedded into the binary at compile time.
    pub fn load_or_embedded() -> Result<Self, RegistryError> {
        if let Some(dir) = Self::find_dir() {
            return Self::load(&dir);
        }

        // Fall back to embedded catalog
        let manifests = embedded::load_embedded();
        let bundles = embedded::load_embedded_bundles();
        Self::validate_manifest_map(&manifests)?;
        validate_bundles(&bundles)?;

        tracing::info!(
            "Loaded embedded registry catalog ({} extensions, {} bundles)",
            manifests.len(),
            bundles.len()
        );

        Ok(Self {
            manifests,
            bundles,
            root: PathBuf::new(),
        })
    }

    /// Load the catalog from a registry directory.
    ///
    /// Expects the structure:
    /// ```text
    /// registry/
    /// ├── tools/*.json
    /// ├── channels/*.json
    /// └── _bundles.json
    /// ```
    pub fn load(registry_dir: &Path) -> Result<Self, RegistryError> {
        let root_metadata = match std::fs::symlink_metadata(registry_dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(RegistryError::DirectoryNotFound(registry_dir.to_path_buf()));
            }
            Err(error) => {
                return Err(RegistryError::ManifestRead {
                    path: registry_dir.to_path_buf(),
                    reason: error.to_string(),
                });
            }
        };
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(RegistryError::ManifestRead {
                path: registry_dir.to_path_buf(),
                reason: "registry root must be a real directory".to_string(),
            });
        }

        let mut manifests = HashMap::new();

        // Load tools
        let tools_dir = registry_dir.join("tools");
        Self::load_manifest_directory_if_present(&tools_dir, "tools", &mut manifests)?;

        // Load channels
        let channels_dir = registry_dir.join("channels");
        Self::load_manifest_directory_if_present(&channels_dir, "channels", &mut manifests)?;

        // Load bundles
        let bundles_path = registry_dir.join("_bundles.json");
        let bundles = match std::fs::symlink_metadata(&bundles_path) {
            Ok(_) => {
                let content = read_regular_text_bounded(&bundles_path, MAX_BUNDLES_BYTES).map_err(
                    |error| {
                        RegistryError::BundlesRead(format!("{}: {}", bundles_path.display(), error))
                    },
                )?;
                let bundles_file: BundlesFile =
                    serde_json::from_str(&content).map_err(|error| {
                        RegistryError::BundlesRead(format!("{}: {}", bundles_path.display(), error))
                    })?;
                bundles_file.bundles
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(error) => {
                return Err(RegistryError::BundlesRead(format!(
                    "{}: {}",
                    bundles_path.display(),
                    error
                )));
            }
        };
        Self::validate_manifest_map(&manifests)?;
        validate_bundles(&bundles)?;

        Ok(Self {
            manifests,
            bundles,
            root: registry_dir.to_path_buf(),
        })
    }

    fn load_manifest_directory_if_present(
        dir: &Path,
        kind_prefix: &str,
        manifests: &mut HashMap<String, ExtensionManifest>,
    ) -> Result<(), RegistryError> {
        match std::fs::symlink_metadata(dir) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_dir() => {
                Self::load_manifests_from_dir(dir, kind_prefix, manifests)
            }
            Ok(_) => Err(RegistryError::ManifestRead {
                path: dir.to_path_buf(),
                reason: "manifest collection must be a real directory".to_string(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(RegistryError::ManifestRead {
                path: dir.to_path_buf(),
                reason: error.to_string(),
            }),
        }
    }

    fn validate_manifest_map(
        manifests: &HashMap<String, ExtensionManifest>,
    ) -> Result<(), RegistryError> {
        if manifests.len() > MAX_MANIFESTS {
            return Err(RegistryError::ManifestRead {
                path: PathBuf::new(),
                reason: format!("registry exceeds the {MAX_MANIFESTS}-manifest limit"),
            });
        }
        for (key, manifest) in manifests {
            crate::registry::installer::validate_manifest_install_inputs(manifest)?;
            let expected_prefix = match manifest.kind {
                ManifestKind::Tool => "tools",
                ManifestKind::Channel => "channels",
            };
            if key != &format!("{expected_prefix}/{}", manifest.name) {
                return Err(RegistryError::InvalidManifest {
                    name: manifest.name.clone(),
                    field: "name/kind",
                    reason: format!("does not match catalog key '{key}'"),
                });
            }
        }
        Ok(())
    }

    fn load_manifests_from_dir(
        dir: &Path,
        kind_prefix: &str,
        manifests: &mut HashMap<String, ExtensionManifest>,
    ) -> Result<(), RegistryError> {
        let entries = std::fs::read_dir(dir).map_err(|e| RegistryError::ManifestRead {
            path: dir.to_path_buf(),
            reason: e.to_string(),
        })?;

        for entry in entries {
            if manifests.len() >= MAX_MANIFESTS {
                return Err(RegistryError::ManifestRead {
                    path: dir.to_path_buf(),
                    reason: format!("registry exceeds the {MAX_MANIFESTS}-manifest limit"),
                });
            }
            let entry = entry.map_err(|e| RegistryError::ManifestRead {
                path: dir.to_path_buf(),
                reason: e.to_string(),
            })?;

            let path = entry.path();
            let file_type = entry.file_type().map_err(|e| RegistryError::ManifestRead {
                path: path.clone(),
                reason: e.to_string(),
            })?;
            if !file_type.is_file() || path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            let content = read_regular_text_bounded(&path, MAX_MANIFEST_BYTES).map_err(|e| {
                RegistryError::ManifestRead {
                    path: path.clone(),
                    reason: e.to_string(),
                }
            })?;

            let manifest: ExtensionManifest =
                serde_json::from_str(&content).map_err(|e| RegistryError::ManifestParse {
                    path: path.clone(),
                    reason: e.to_string(),
                })?;

            crate::registry::installer::validate_manifest_install_inputs(&manifest)?;
            let expected_kind = match kind_prefix {
                "tools" => ManifestKind::Tool,
                "channels" => ManifestKind::Channel,
                _ => {
                    return Err(RegistryError::ManifestRead {
                        path: dir.to_path_buf(),
                        reason: format!("unsupported manifest directory '{kind_prefix}'"),
                    });
                }
            };
            if manifest.kind != expected_kind {
                return Err(RegistryError::InvalidManifest {
                    name: manifest.name.clone(),
                    field: "kind",
                    reason: format!("does not match its '{kind_prefix}' directory"),
                });
            }
            let key = format!("{}/{}", kind_prefix, manifest.name);
            if manifests.insert(key.clone(), manifest).is_some() {
                return Err(RegistryError::ManifestRead {
                    path: path.clone(),
                    reason: format!("duplicate registry manifest key '{key}'"),
                });
            }
        }

        Ok(())
    }

    /// The root directory this catalog was loaded from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get all manifests.
    pub fn all(&self) -> Vec<&ExtensionManifest> {
        let mut items: Vec<_> = self.manifests.values().collect();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        items
    }

    /// List manifests, optionally filtered by kind and/or tag.
    pub fn list(&self, kind: Option<ManifestKind>, tag: Option<&str>) -> Vec<&ExtensionManifest> {
        let mut results: Vec<_> = self
            .manifests
            .values()
            .filter(|m| kind.is_none_or(|k| m.kind == k))
            .filter(|m| tag.is_none_or(|t| m.tags.iter().any(|mt| mt == t)))
            .collect();
        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }

    /// Get a manifest by name. Tries exact key match first ("tools/github"),
    /// then searches by bare name ("github").
    ///
    /// If a bare name matches both a tool and a channel, returns `None`.
    /// Use a qualified key ("tools/github" or "channels/telegram") to disambiguate.
    pub fn get(&self, name: &str) -> Option<&ExtensionManifest> {
        // Try exact key first
        if let Some(m) = self.manifests.get(name) {
            return Some(m);
        }

        // Try with kind prefix, detecting collisions
        let tool = self.manifests.get(&format!("tools/{}", name));
        let channel = self.manifests.get(&format!("channels/{}", name));

        match (tool, channel) {
            (Some(_), Some(_)) => None, // ambiguous
            (Some(m), None) => Some(m),
            (None, Some(m)) => Some(m),
            (None, None) => None,
        }
    }

    /// Get a manifest by name, returning a `Result` with an explicit error for
    /// ambiguous bare names.
    pub fn get_strict(&self, name: &str) -> Result<&ExtensionManifest, RegistryError> {
        // Try exact key first
        if let Some(m) = self.manifests.get(name) {
            return Ok(m);
        }

        let tool_key = format!("tools/{name}");
        let channel_key = format!("channels/{name}");
        let tool = self.manifests.get(&tool_key);
        let channel = self.manifests.get(&channel_key);

        match (tool, channel) {
            (Some(_), Some(_)) => Err(RegistryError::AmbiguousName {
                name: name.to_string(),
                kind_a: "tool",
                prefix_a: "tools",
                kind_b: "channel",
                prefix_b: "channels",
            }),
            (Some(manifest), None) | (None, Some(manifest)) => Ok(manifest),
            (None, None) => Err(RegistryError::ExtensionNotFound(name.to_string())),
        }
    }

    /// Get the full key ("tools/github" or "channels/telegram") for a manifest.
    pub fn key_for(&self, name: &str) -> Option<String> {
        if self.manifests.contains_key(name) {
            return Some(name.to_string());
        }

        let has_tool = self.manifests.contains_key(&format!("tools/{}", name));
        let has_channel = self.manifests.contains_key(&format!("channels/{}", name));

        match (has_tool, has_channel) {
            (true, true) => None, // ambiguous
            (true, false) => Some(format!("tools/{}", name)),
            (false, true) => Some(format!("channels/{}", name)),
            (false, false) => None,
        }
    }

    /// Search manifests by query string (matches name, display_name, description, keywords).
    pub fn search(&self, query: &str) -> Vec<&ExtensionManifest> {
        let query_lower = query.to_lowercase();
        let tokens: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(&ExtensionManifest, usize)> = self
            .manifests
            .values()
            .filter_map(|m| {
                let score = Self::score_manifest(m, &tokens);
                if score > 0 { Some((m, score)) } else { None }
            })
            .collect();

        scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.name.cmp(&b.0.name)));
        scored.into_iter().map(|(m, _)| m).collect()
    }

    fn score_manifest(manifest: &ExtensionManifest, tokens: &[&str]) -> usize {
        let mut score = 0;
        let name_lower = manifest.name.to_lowercase();
        let display_lower = manifest.display_name.to_lowercase();
        let desc_lower = manifest.description.to_lowercase();

        for token in tokens {
            if name_lower == *token {
                score += 10;
            } else if name_lower.contains(token) {
                score += 5;
            }

            if display_lower == *token {
                score += 8;
            } else if display_lower.contains(token) {
                score += 4;
            }

            if desc_lower.contains(token) {
                score += 2;
            }

            for kw in &manifest.keywords {
                if kw.to_lowercase() == *token {
                    score += 6;
                } else if kw.to_lowercase().contains(token) {
                    score += 3;
                }
            }

            for tag in &manifest.tags {
                if tag.to_lowercase() == *token {
                    score += 4;
                }
            }
        }

        score
    }

    /// Get a bundle definition by name.
    pub fn get_bundle(&self, name: &str) -> Option<&BundleDefinition> {
        self.bundles.get(name)
    }

    /// List all bundle names.
    pub fn bundle_names(&self) -> Vec<&str> {
        let mut names: Vec<_> = self.bundles.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }

    /// Resolve a bundle into its constituent manifests.
    /// Returns the manifests and any extension keys that couldn't be found.
    pub fn resolve_bundle(
        &self,
        bundle_name: &str,
    ) -> Result<(Vec<&ExtensionManifest>, Vec<String>), RegistryError> {
        let bundle = self
            .bundles
            .get(bundle_name)
            .ok_or_else(|| RegistryError::BundleNotFound(bundle_name.to_string()))?;

        let mut found = Vec::new();
        let mut missing = Vec::new();

        for ext_key in &bundle.extensions {
            if let Some(manifest) = self.manifests.get(ext_key) {
                found.push(manifest);
            } else {
                missing.push(ext_key.clone());
            }
        }

        Ok((found, missing))
    }

    /// Check if a name refers to a bundle rather than an individual extension.
    pub fn is_bundle(&self, name: &str) -> bool {
        self.bundles.contains_key(name)
    }

    /// Resolve a name to either a single manifest or the manifests in a bundle.
    /// Returns (manifests, bundle_definition_if_bundle).
    pub fn resolve(
        &self,
        name: &str,
    ) -> Result<(Vec<&ExtensionManifest>, Option<&BundleDefinition>), RegistryError> {
        // Check bundle first
        if let Some(bundle) = self.bundles.get(name) {
            let (manifests, missing) = self.resolve_bundle(name)?;
            if !missing.is_empty() {
                tracing::warn!(
                    "Bundle '{}' references missing extensions: {:?}",
                    name,
                    missing
                );
            }
            return Ok((manifests, Some(bundle)));
        }

        // Single extension (use get_strict to catch ambiguous bare names)
        let manifest = self.get_strict(name)?;
        Ok((vec![manifest], None))
    }
}

fn read_regular_text_bounded(path: &Path, max_bytes: u64) -> std::io::Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max_bytes {
        return Err(std::io::Error::other(format!(
            "must be a regular file no larger than {max_bytes} bytes"
        )));
    }

    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    let opened_metadata = file.metadata()?;
    if !opened_metadata.is_file() || opened_metadata.len() > max_bytes {
        return Err(std::io::Error::other(
            "registry file changed or exceeded its limit while being opened",
        ));
    }
    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    file.by_ref()
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(std::io::Error::other(
            "registry file exceeds its size limit",
        ));
    }
    String::from_utf8(bytes).map_err(|_| std::io::Error::other("registry file is not valid UTF-8"))
}

fn validate_bundles(bundles: &HashMap<String, BundleDefinition>) -> Result<(), RegistryError> {
    if bundles.len() > MAX_BUNDLES {
        return Err(RegistryError::BundlesRead(format!(
            "bundle count exceeds the {MAX_BUNDLES}-entry limit"
        )));
    }
    for (name, bundle) in bundles {
        let mut unique = std::collections::HashSet::with_capacity(bundle.extensions.len());
        let valid_extensions = bundle.extensions.len() <= MAX_BUNDLE_EXTENSIONS
            && bundle.extensions.iter().all(|extension| {
                let Some((prefix, extension_name)) = extension.split_once('/') else {
                    return false;
                };
                matches!(prefix, "tools" | "channels")
                    && valid_catalog_identifier(extension_name, 128)
                    && unique.insert(extension)
            });
        if !valid_catalog_identifier(name, 128)
            || !valid_catalog_text(&bundle.display_name, 256, false)
            || bundle
                .description
                .as_deref()
                .is_some_and(|value| !valid_catalog_text(value, 4 * 1024, false))
            || !valid_extensions
            || bundle
                .shared_auth
                .as_deref()
                .is_some_and(|value| !valid_catalog_identifier(value, 128))
        {
            return Err(RegistryError::BundlesRead(format!(
                "bundle '{name}' contains invalid or unbounded metadata"
            )));
        }
    }
    Ok(())
}

fn valid_catalog_identifier(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn valid_catalog_text(value: &str, max_bytes: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.trim().is_empty())
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_test_registry(dir: &Path) {
        let tools_dir = dir.join("tools");
        let channels_dir = dir.join("channels");
        fs::create_dir_all(&tools_dir).unwrap();
        fs::create_dir_all(&channels_dir).unwrap();

        fs::write(
            tools_dir.join("slack.json"),
            r#"{
                "name": "slack",
                "display_name": "Slack",
                "kind": "tool",
                "version": "0.1.0",
                "description": "Post messages via Slack API",
                "keywords": ["messaging", "chat"],
                "source": {
                    "dir": "tools-src/slack",
                    "capabilities": "slack-tool.capabilities.json",
                    "crate_name": "slack-tool"
                },
                "auth_summary": {
                    "method": "oauth",
                    "provider": "Slack",
                    "secrets": ["slack_bot_token"]
                },
                "tags": ["default", "messaging"]
            }"#,
        )
        .unwrap();

        fs::write(
            tools_dir.join("github.json"),
            r#"{
                "name": "github",
                "display_name": "GitHub",
                "kind": "tool",
                "version": "0.1.0",
                "description": "GitHub integration for issues and PRs",
                "keywords": ["code", "git"],
                "source": {
                    "dir": "tools-src/github",
                    "capabilities": "github-tool.capabilities.json",
                    "crate_name": "github-tool"
                },
                "tags": ["default", "development"]
            }"#,
        )
        .unwrap();

        fs::write(
            channels_dir.join("telegram.json"),
            r#"{
                "name": "telegram",
                "display_name": "Telegram",
                "kind": "channel",
                "version": "0.1.0",
                "description": "Telegram Bot API channel",
                "source": {
                    "dir": "channels-src/telegram",
                    "capabilities": "telegram.capabilities.json",
                    "crate_name": "telegram-channel"
                },
                "tags": ["messaging"]
            }"#,
        )
        .unwrap();

        fs::write(
            dir.join("_bundles.json"),
            r#"{
                "bundles": {
                    "default": {
                        "display_name": "Recommended",
                        "extensions": ["tools/slack", "tools/github", "channels/telegram"]
                    },
                    "messaging": {
                        "display_name": "Messaging",
                        "extensions": ["tools/slack", "channels/telegram"],
                        "shared_auth": null
                    }
                }
            }"#,
        )
        .unwrap();
    }

    #[test]
    fn test_load_catalog() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_registry(tmp.path());

        let catalog = RegistryCatalog::load(tmp.path()).unwrap();
        assert_eq!(catalog.all().len(), 3);
    }

    #[test]
    fn test_list_by_kind() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_registry(tmp.path());

        let catalog = RegistryCatalog::load(tmp.path()).unwrap();
        let tools = catalog.list(Some(ManifestKind::Tool), None);
        assert_eq!(tools.len(), 2);

        let channels = catalog.list(Some(ManifestKind::Channel), None);
        assert_eq!(channels.len(), 1);
    }

    #[test]
    fn test_list_by_tag() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_registry(tmp.path());

        let catalog = RegistryCatalog::load(tmp.path()).unwrap();
        let defaults = catalog.list(None, Some("default"));
        assert_eq!(defaults.len(), 2);

        let messaging = catalog.list(None, Some("messaging"));
        assert_eq!(messaging.len(), 2); // slack (tool) and telegram (channel) both have "messaging" tag
    }

    #[test]
    fn test_get_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_registry(tmp.path());

        let catalog = RegistryCatalog::load(tmp.path()).unwrap();

        // Full key
        assert!(catalog.get("tools/slack").is_some());

        // Bare name
        assert!(catalog.get("slack").is_some());
        assert!(catalog.get("telegram").is_some());

        // Missing
        assert!(catalog.get("nonexistent").is_none());
    }

    #[test]
    fn test_search() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_registry(tmp.path());

        let catalog = RegistryCatalog::load(tmp.path()).unwrap();

        let results = catalog.search("slack");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "slack");

        let results = catalog.search("messaging");
        assert!(!results.is_empty());

        let results = catalog.search("nonexistent query");
        assert!(results.is_empty());
    }

    #[test]
    fn test_resolve_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_registry(tmp.path());

        let catalog = RegistryCatalog::load(tmp.path()).unwrap();

        let (manifests, missing) = catalog.resolve_bundle("default").unwrap();
        assert_eq!(manifests.len(), 3);
        assert!(missing.is_empty());

        assert!(catalog.resolve_bundle("nonexistent").is_err());
    }

    #[test]
    fn test_resolve_single_or_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_registry(tmp.path());

        let catalog = RegistryCatalog::load(tmp.path()).unwrap();

        // Single extension
        let (manifests, bundle) = catalog.resolve("slack").unwrap();
        assert_eq!(manifests.len(), 1);
        assert!(bundle.is_none());

        // Bundle
        let (manifests, bundle) = catalog.resolve("default").unwrap();
        assert_eq!(manifests.len(), 3);
        assert!(bundle.is_some());
    }

    #[test]
    fn test_bundle_names() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_registry(tmp.path());

        let catalog = RegistryCatalog::load(tmp.path()).unwrap();
        let names = catalog.bundle_names();
        assert_eq!(names, vec!["default", "messaging"]);
    }

    #[test]
    fn test_directory_not_found() {
        let result = RegistryCatalog::load(Path::new("/nonexistent/path"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_or_embedded_succeeds() {
        // Should always succeed: either finds registry/ on disk or falls back to embedded
        let catalog = RegistryCatalog::load_or_embedded().unwrap();
        // At minimum, the embedded catalog from the repo should have entries
        assert!(!catalog.all().is_empty() || !catalog.bundle_names().is_empty());
    }

    #[test]
    fn test_bundle_entries_resolve_against_real_registry() {
        // Load the actual registry/ directory (catches stale bundle refs after renames)
        let catalog = RegistryCatalog::load_or_embedded().unwrap();

        for bundle_name in catalog.bundle_names() {
            let (manifests, missing) = catalog.resolve_bundle(bundle_name).unwrap();
            assert!(
                missing.is_empty(),
                "Bundle '{}' has unresolved entries: {:?}. \
                 Check that _bundles.json entries match manifest name fields.",
                bundle_name,
                missing
            );
            assert!(
                !manifests.is_empty(),
                "Bundle '{}' resolved to zero manifests",
                bundle_name
            );
        }
    }
}
