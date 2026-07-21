//! Install extensions from the registry: build-from-source or download pre-built artifacts.

use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};

use tokio::fs;

use crate::registry::catalog::RegistryError;
use crate::registry::manifest::{BundleDefinition, ExtensionManifest, ManifestKind};

// GitHub-only by design. New trusted hosts (e.g. a NEAR AI CDN) must be
// explicitly added here; unknown hosts fall back to source build with a
// warning rather than surfacing a clear "host not allowed" error.
const ALLOWED_ARTIFACT_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "github-releases.githubusercontent.com",
    "raw.githubusercontent.com",
];
const MAX_ARTIFACT_DOWNLOAD_BYTES: usize = 64 * 1024 * 1024;
const MAX_WASM_BYTES: usize = 64 * 1024 * 1024;
const MAX_CAPABILITIES_BYTES: usize = 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 128;
const MAX_ARTIFACT_URL_BYTES: usize = 16 * 1024;

fn should_attempt_source_fallback(err: &RegistryError) -> bool {
    match err {
        // Hard failures: never retry with source build
        RegistryError::AlreadyInstalled { .. }
        | RegistryError::ChecksumMismatch { .. }
        | RegistryError::ToolchainMissing { .. } => false,

        // InvalidManifest is non-retryable EXCEPT when the only issue is a
        // missing sha256 (common for manifests with placeholder artifact URLs
        // that haven't had a release published yet).
        RegistryError::InvalidManifest { reason, .. } => reason.contains("sha256 is required"),

        // Everything else (download failures, network errors, etc.) → retry
        _ => true,
    }
}

fn is_allowed_artifact_host(host: &str) -> bool {
    ALLOWED_ARTIFACT_HOSTS
        .iter()
        .any(|allowed| host.eq_ignore_ascii_case(allowed))
        || host.ends_with(".githubusercontent.com")
}

fn validate_artifact_url(
    manifest_name: &str,
    field: &'static str,
    url: &str,
) -> Result<(), RegistryError> {
    if url.is_empty() || url.len() > MAX_ARTIFACT_URL_BYTES || url.chars().any(char::is_control) {
        return Err(RegistryError::InvalidManifest {
            name: manifest_name.to_string(),
            field,
            reason: "URL is empty, malformed, or exceeds its size limit".to_string(),
        });
    }
    let parsed = reqwest::Url::parse(url).map_err(|e| RegistryError::InvalidManifest {
        name: manifest_name.to_string(),
        field,
        reason: format!("invalid URL: {}", e),
    })?;

    if parsed.scheme() != "https" {
        return Err(RegistryError::InvalidManifest {
            name: manifest_name.to_string(),
            field,
            reason: "URL must use https".to_string(),
        });
    }

    if !parsed.username().is_empty() || parsed.password().is_some() || parsed.fragment().is_some() {
        return Err(RegistryError::InvalidManifest {
            name: manifest_name.to_string(),
            field,
            reason: "URL cannot contain embedded credentials or a fragment".to_string(),
        });
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| RegistryError::InvalidManifest {
            name: manifest_name.to_string(),
            field,
            reason: "URL host is missing".to_string(),
        })?;

    if host.parse::<IpAddr>().is_ok()
        || !is_allowed_artifact_host(host)
        || parsed.port_or_known_default() != Some(443)
    {
        return Err(RegistryError::InvalidManifest {
            name: manifest_name.to_string(),
            field,
            reason: format!("host '{}' is not allowed", host),
        });
    }

    Ok(())
}

pub(crate) fn redacted_download_url(url: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return "<invalid-url>".to_string();
    };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

pub(crate) fn validate_manifest_install_inputs(
    manifest: &ExtensionManifest,
) -> Result<(), RegistryError> {
    let is_valid_name = !manifest.name.is_empty()
        && manifest.name.len() <= 128
        && manifest
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');

    if !is_valid_name {
        return Err(RegistryError::InvalidManifest {
            name: manifest.name.clone(),
            field: "name",
            reason: "name must contain only lowercase letters, digits, '-' or '_'".to_string(),
        });
    }
    if manifest.kind == ManifestKind::Tool
        && thinclaw_tools::registry::PROTECTED_TOOL_NAMES.contains(&manifest.name.as_str())
    {
        return Err(RegistryError::InvalidManifest {
            name: manifest.name.clone(),
            field: "name",
            reason: "tool name conflicts with a protected built-in tool".to_string(),
        });
    }

    if !valid_manifest_text(&manifest.display_name, 256, false) {
        return Err(invalid_manifest_field(
            manifest,
            "display_name",
            "must contain 1-256 bytes without control characters",
        ));
    }
    if manifest.version.len() > 64 || semver::Version::parse(&manifest.version).is_err() {
        return Err(invalid_manifest_field(
            manifest,
            "version",
            "must be a semantic version no longer than 64 bytes",
        ));
    }
    if !valid_manifest_text(&manifest.description, 4 * 1024, false) {
        return Err(invalid_manifest_field(
            manifest,
            "description",
            "must contain 1-4096 bytes without control characters",
        ));
    }
    if !valid_manifest_string_list(&manifest.keywords, 64, 128)
        || !valid_manifest_string_list(&manifest.tags, 64, 128)
    {
        return Err(invalid_manifest_field(
            manifest,
            "keywords/tags",
            "lists must contain at most 64 unique bounded values",
        ));
    }

    let expected_prefix = match manifest.kind {
        ManifestKind::Tool => "tools-src/",
        ManifestKind::Channel => "channels-src/",
    };

    if manifest.source.dir.len() > 1024
        || manifest.source.dir.chars().any(char::is_control)
        || !manifest.source.dir.starts_with(expected_prefix)
    {
        return Err(RegistryError::InvalidManifest {
            name: manifest.name.clone(),
            field: "source.dir",
            reason: format!("must start with '{}'", expected_prefix),
        });
    }

    let source_path = Path::new(&manifest.source.dir);
    let has_unsafe_component = source_path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) | Component::CurDir
        )
    });

    if source_path.is_absolute() || has_unsafe_component {
        return Err(RegistryError::InvalidManifest {
            name: manifest.name.clone(),
            field: "source.dir",
            reason: "must be a safe relative path without traversal segments".to_string(),
        });
    }

    let has_path_separator = manifest.source.capabilities.contains('/')
        || manifest.source.capabilities.contains('\\')
        || manifest.source.capabilities.contains("..");

    if has_path_separator {
        return Err(RegistryError::InvalidManifest {
            name: manifest.name.clone(),
            field: "source.capabilities",
            reason: "must be a file name without path separators".to_string(),
        });
    }

    if manifest.source.capabilities.is_empty()
        || manifest.source.capabilities.len() > 256
        || !manifest.source.capabilities.ends_with(".json")
    {
        return Err(RegistryError::InvalidManifest {
            name: manifest.name.clone(),
            field: "source.capabilities",
            reason: "must be a non-empty .json file name no longer than 256 bytes".to_string(),
        });
    }

    if manifest.source.crate_name.is_empty()
        || manifest.source.crate_name.len() > 128
        || !manifest
            .source
            .crate_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(RegistryError::InvalidManifest {
            name: manifest.name.clone(),
            field: "source.crate_name",
            reason: "must contain only ASCII letters, digits, hyphens, or underscores".to_string(),
        });
    }

    if manifest.artifacts.len() > 16 {
        return Err(invalid_manifest_field(
            manifest,
            "artifacts",
            "must contain at most 16 target entries",
        ));
    }
    for (target, artifact) in &manifest.artifacts {
        if target.is_empty()
            || target.len() > 128
            || !target
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(invalid_manifest_field(
                manifest,
                "artifacts",
                "target names must be bounded ASCII identifiers",
            ));
        }
        if let Some(url) = artifact.url.as_deref() {
            if url.len() > 4096 {
                return Err(invalid_manifest_field(
                    manifest,
                    "artifacts.url",
                    "URL exceeds the 4096-byte limit",
                ));
            }
            validate_artifact_url(&manifest.name, "artifacts.url", url)?;
        }
        if let Some(url) = artifact.capabilities_url.as_deref() {
            if url.len() > 4096 {
                return Err(invalid_manifest_field(
                    manifest,
                    "artifacts.capabilities_url",
                    "URL exceeds the 4096-byte limit",
                ));
            }
            validate_artifact_url(&manifest.name, "artifacts.capabilities_url", url)?;
        }
        if artifact.sha256.as_deref().is_some_and(|hash| {
            hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(invalid_manifest_field(
                manifest,
                "artifacts.sha256",
                "must be a 64-character hexadecimal SHA-256 digest",
            ));
        }
    }

    if let Some(auth) = manifest.auth_summary.as_ref() {
        if auth
            .method
            .as_deref()
            .is_some_and(|method| !matches!(method, "oauth" | "manual" | "none"))
            || auth
                .provider
                .as_deref()
                .is_some_and(|value| !valid_manifest_text(value, 256, false))
            || !valid_manifest_identifier_list(&auth.secrets, 64, 128)
            || auth
                .shared_auth
                .as_deref()
                .is_some_and(|value| !valid_manifest_identifier(value, 128))
        {
            return Err(invalid_manifest_field(
                manifest,
                "auth_summary",
                "contains an invalid method, provider, or secret identifier",
            ));
        }
        if let Some(setup_url) = auth.setup_url.as_deref() {
            let parsed = reqwest::Url::parse(setup_url).ok();
            if setup_url.len() > 4096
                || parsed.as_ref().is_none_or(|url| {
                    url.scheme() != "https"
                        || url.host_str().is_none()
                        || !url.username().is_empty()
                        || url.password().is_some()
                })
            {
                return Err(invalid_manifest_field(
                    manifest,
                    "auth_summary.setup_url",
                    "must be a bounded HTTPS URL without embedded credentials",
                ));
            }
        }
    }

    Ok(())
}

fn invalid_manifest_field(
    manifest: &ExtensionManifest,
    field: &'static str,
    reason: impl Into<String>,
) -> RegistryError {
    RegistryError::InvalidManifest {
        name: manifest.name.clone(),
        field,
        reason: reason.into(),
    }
}

fn valid_manifest_text(value: &str, max_bytes: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.trim().is_empty())
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
}

fn valid_manifest_string_list(values: &[String], max_entries: usize, max_bytes: usize) -> bool {
    let mut unique = std::collections::HashSet::with_capacity(values.len());
    values.len() <= max_entries
        && values
            .iter()
            .all(|value| valid_manifest_text(value, max_bytes, false) && unique.insert(value))
}

fn valid_manifest_identifier(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_manifest_identifier_list(values: &[String], max_entries: usize, max_bytes: usize) -> bool {
    let mut unique = std::collections::HashSet::with_capacity(values.len());
    values.len() <= max_entries
        && values
            .iter()
            .all(|value| valid_manifest_identifier(value, max_bytes) && unique.insert(value))
}

fn download_failure_reason(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "request timed out".to_string()
    } else if error.is_connect() {
        "connection failed".to_string()
    } else if error.is_request() {
        "request failed".to_string()
    } else {
        "network error".to_string()
    }
}

/// Result of installing a single extension from the registry.
#[derive(Debug)]
pub struct InstallOutcome {
    /// Extension name.
    pub name: String,
    /// Whether this is a tool or channel.
    pub kind: ManifestKind,
    /// Destination path of the installed WASM binary.
    pub wasm_path: PathBuf,
    /// Whether a capabilities file was also installed.
    pub has_capabilities: bool,
    /// Any warning messages.
    pub warnings: Vec<String>,
}

/// Handles installing extensions from registry manifests.
pub struct RegistryInstaller {
    /// Root of the repo (parent of `registry/`), used to resolve `source.dir`.
    repo_root: PathBuf,
    /// Directory for installed tools (`~/.thinclaw/tools/`).
    tools_dir: PathBuf,
    /// Directory for installed channels (`~/.thinclaw/channels/`).
    channels_dir: PathBuf,
}

impl RegistryInstaller {
    pub fn new(repo_root: PathBuf, tools_dir: PathBuf, channels_dir: PathBuf) -> Self {
        Self {
            repo_root,
            tools_dir,
            channels_dir,
        }
    }

    /// Default installer using standard paths.
    pub fn with_defaults(repo_root: PathBuf) -> Self {
        let state_paths = crate::platform::state_paths();
        Self {
            repo_root,
            tools_dir: state_paths.tools_dir,
            channels_dir: state_paths.channels_dir,
        }
    }

    /// Install a single extension by building from source.
    pub async fn install_from_source(
        &self,
        manifest: &ExtensionManifest,
        force: bool,
    ) -> Result<InstallOutcome, RegistryError> {
        validate_manifest_install_inputs(manifest)?;

        let source_dir = self.repo_root.join(&manifest.source.dir);
        if !source_dir.exists() {
            return Err(RegistryError::ManifestRead {
                path: source_dir.clone(),
                reason: "source directory does not exist".to_string(),
            });
        }

        let target_dir = match manifest.kind {
            ManifestKind::Tool => &self.tools_dir,
            ManifestKind::Channel => &self.channels_dir,
        };

        fs::create_dir_all(target_dir)
            .await
            .map_err(RegistryError::Io)?;

        // Use manifest.name for installed filenames so discovery, auth, and
        // CLI commands (`thinclaw tool auth <name>`) all agree on the stem.
        let target_wasm = target_dir.join(format!("{}.wasm", manifest.name));

        // Check if already exists
        if target_wasm.exists() && !force {
            return Err(RegistryError::AlreadyInstalled {
                name: manifest.name.clone(),
                path: target_wasm,
            });
        }

        println!(
            "Building {} '{}' from {}...",
            manifest.kind,
            manifest.display_name,
            source_dir.display()
        );
        let crate_name = &manifest.source.crate_name;
        let wasm_path =
            crate::registry::artifacts::build_wasm_component(&source_dir, crate_name, true)
                .await
                .map_err(|error| match error {
                    crate::registry::artifacts::WasmBuildError::ToolchainUnavailable => {
                        RegistryError::ToolchainMissing {
                            name: manifest.name.clone(),
                        }
                    }
                    other => RegistryError::ManifestRead {
                        path: source_dir.clone(),
                        reason: format!("build failed: {other}"),
                    },
                })?;

        println!("  Installing to {}", target_wasm.display());
        let installed_wasm = crate::registry::artifacts::install_wasm_files(
            &wasm_path,
            &source_dir,
            &manifest.name,
            target_dir,
            manifest.kind,
            force,
        )
        .await
        .map_err(|e| RegistryError::ManifestRead {
            path: source_dir.clone(),
            reason: format!("failed to install built artifact: {e}"),
        })?;
        let target_caps = target_dir.join(format!("{}.capabilities.json", manifest.name));
        let has_capabilities = target_caps.exists();

        let mut warnings = Vec::new();
        if !has_capabilities {
            warnings.push(format!(
                "No capabilities file found for '{}' in source sidecars",
                manifest.name
            ));
        }

        Ok(InstallOutcome {
            name: manifest.name.clone(),
            kind: manifest.kind,
            wasm_path: installed_wasm,
            has_capabilities,
            warnings,
        })
    }

    pub async fn install_with_source_fallback(
        &self,
        manifest: &ExtensionManifest,
        force: bool,
    ) -> Result<InstallOutcome, RegistryError> {
        // Validate upfront so we fail fast on bad manifests regardless of
        // which install path runs, without relying on inner methods to
        // catch it first.
        validate_manifest_install_inputs(manifest)?;

        let has_artifact = manifest
            .artifacts
            .get("wasm32-wasip2")
            .and_then(|a| a.url.as_ref())
            .is_some();

        if !has_artifact {
            return self.install_from_source(manifest, force).await;
        }

        let source_dir = self.repo_root.join(&manifest.source.dir);

        match self.install_from_artifact(manifest, force).await {
            Ok(outcome) => Ok(outcome),
            Err(artifact_err) => {
                if !should_attempt_source_fallback(&artifact_err) {
                    return Err(artifact_err);
                }

                if !source_dir.is_dir() {
                    return Err(RegistryError::SourceFallbackUnavailable {
                        name: manifest.name.clone(),
                        source_dir,
                        artifact_error: Box::new(artifact_err),
                    });
                }

                tracing::warn!(
                    extension = %manifest.name,
                    error = %artifact_err,
                    "Artifact install failed; falling back to build-from-source"
                );

                match self.install_from_source(manifest, force).await {
                    Ok(mut outcome) => {
                        outcome.warnings.push(format!(
                            "Artifact install failed ({}); installed via source fallback.",
                            artifact_err
                        ));
                        Ok(outcome)
                    }
                    Err(source_err) => Err(RegistryError::InstallFallbackFailed {
                        name: manifest.name.clone(),
                        artifact_error: Box::new(artifact_err),
                        source_error: Box::new(source_err),
                    }),
                }
            }
        }
    }

    /// Download and install a pre-built artifact.
    ///
    /// Supports two formats:
    /// - **tar.gz bundle**: Contains `{name}.wasm` + `{name}.capabilities.json`
    /// - **bare .wasm file**: Just the WASM binary (capabilities fetched separately if available)
    pub async fn install_from_artifact(
        &self,
        manifest: &ExtensionManifest,
        force: bool,
    ) -> Result<InstallOutcome, RegistryError> {
        validate_manifest_install_inputs(manifest)?;

        let artifact = manifest.artifacts.get("wasm32-wasip2").ok_or_else(|| {
            RegistryError::ExtensionNotFound(format!(
                "No wasm32-wasip2 artifact for '{}'",
                manifest.name
            ))
        })?;

        let url = artifact.url.as_ref().ok_or_else(|| {
            RegistryError::ExtensionNotFound(format!(
                "No artifact URL for '{}'. Use --build to build from source.",
                manifest.name
            ))
        })?;

        validate_artifact_url(&manifest.name, "artifacts.wasm32-wasip2.url", url)?;

        // Require SHA256 — refuse to install unverified binaries. Check before
        // downloading to avoid wasting bandwidth on manifests that are missing
        // checksums.
        let expected_sha =
            artifact
                .sha256
                .as_ref()
                .ok_or_else(|| RegistryError::InvalidManifest {
                    name: manifest.name.clone(),
                    field: "artifacts.wasm32-wasip2.sha256",
                    reason: "sha256 is required for artifact downloads".to_string(),
                })?;

        let target_dir = match manifest.kind {
            ManifestKind::Tool => &self.tools_dir,
            ManifestKind::Channel => &self.channels_dir,
        };

        fs::create_dir_all(target_dir)
            .await
            .map_err(RegistryError::Io)?;

        let target_wasm = target_dir.join(format!("{}.wasm", manifest.name));

        if target_wasm.exists() && !force {
            return Err(RegistryError::AlreadyInstalled {
                name: manifest.name.clone(),
                path: target_wasm,
            });
        }

        // Download
        println!(
            "Downloading {} '{}'...",
            manifest.kind, manifest.display_name
        );
        let bytes = download_artifact(url, MAX_ARTIFACT_DOWNLOAD_BYTES).await?;
        verify_sha256(&bytes, expected_sha, url)?;

        let target_caps = target_dir.join(format!("{}.capabilities.json", manifest.name));

        // Decode and validate the entire payload before publishing either file.
        // This prevents a malformed archive from leaving an executable behind
        // after the install reports failure.
        let (wasm_bytes, capabilities_bytes) = if is_gzip(&bytes) {
            let extracted = extract_tar_gz(&bytes, &manifest.name, url)?;
            (extracted.wasm, extracted.capabilities)
        } else {
            // Try to get capabilities from:
            // 1. Separate capabilities_url in the artifact
            // 2. Source tree (legacy, requires repo)
            let capabilities = if let Some(ref caps_url) = artifact.capabilities_url {
                validate_artifact_url(
                    &manifest.name,
                    "artifacts.wasm32-wasip2.capabilities_url",
                    caps_url,
                )?;
                match download_artifact(caps_url, MAX_CAPABILITIES_BYTES).await {
                    Ok(caps_bytes) => Some(caps_bytes.to_vec()),
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to download artifact capabilities");
                        None
                    }
                }
            } else {
                // Legacy fallback: try source tree
                let caps_source = self
                    .repo_root
                    .join(&manifest.source.dir)
                    .join(&manifest.source.capabilities);
                read_regular_file_bounded(caps_source, MAX_CAPABILITIES_BYTES).await?
            };
            (bytes.to_vec(), capabilities)
        };

        validate_wasm_payload(&wasm_bytes, url)?;
        if let Some(capabilities) = capabilities_bytes.as_deref() {
            validate_capabilities_payload(manifest.kind, &manifest.name, capabilities, url)?;
        }
        let has_capabilities = capabilities_bytes.is_some();
        publish_extension_files(
            target_wasm.clone(),
            target_caps,
            wasm_bytes,
            capabilities_bytes,
            force,
        )
        .await?;

        println!("  Installed to {}", target_wasm.display());

        let mut warnings = Vec::new();
        if !has_capabilities {
            warnings.push(format!(
                "No capabilities file found for '{}'. Auth and hooks may not work.",
                manifest.name
            ));
        }

        Ok(InstallOutcome {
            name: manifest.name.clone(),
            kind: manifest.kind,
            wasm_path: target_wasm,
            has_capabilities,
            warnings,
        })
    }

    /// Install a single manifest, choosing build vs download based on artifact availability and flags.
    pub async fn install(
        &self,
        manifest: &ExtensionManifest,
        force: bool,
        prefer_build: bool,
    ) -> Result<InstallOutcome, RegistryError> {
        let has_artifact = manifest
            .artifacts
            .get("wasm32-wasip2")
            .and_then(|a| a.url.as_ref())
            .is_some();

        if prefer_build || !has_artifact {
            self.install_from_source(manifest, force).await
        } else {
            self.install_from_artifact(manifest, force).await
        }
    }

    /// Install all extensions in a bundle.
    /// Returns the outcomes and any shared auth hints.
    pub async fn install_bundle(
        &self,
        manifests: &[&ExtensionManifest],
        bundle: &BundleDefinition,
        force: bool,
        prefer_build: bool,
    ) -> (Vec<InstallOutcome>, Vec<String>) {
        let mut outcomes = Vec::new();
        let mut errors = Vec::new();

        for manifest in manifests {
            match self.install(manifest, force, prefer_build).await {
                Ok(outcome) => outcomes.push(outcome),
                Err(e) => errors.push(format!("{}: {}", manifest.name, e)),
            }
        }

        // Collect auth hints
        let mut auth_hints = Vec::new();
        if let Some(shared) = &bundle.shared_auth {
            auth_hints.push(format!(
                "Bundle uses shared auth '{}'. Run `thinclaw tool auth <any-member>` to authenticate all members.",
                shared
            ));
        }

        // Collect unique auth providers that need setup
        let mut seen_providers = std::collections::HashSet::new();
        for manifest in manifests {
            if let Some(auth) = &manifest.auth_summary {
                let key = auth
                    .shared_auth
                    .as_deref()
                    .unwrap_or(manifest.name.as_str());
                if seen_providers.insert(key.to_string())
                    && let Some(url) = &auth.setup_url
                {
                    auth_hints.push(format!(
                        "  {} ({}): {}",
                        auth.provider.as_deref().unwrap_or(&manifest.name),
                        auth.method.as_deref().unwrap_or("manual"),
                        url
                    ));
                }
            }
        }

        if !errors.is_empty() {
            auth_hints.push(format!(
                "\nFailed to install {} extension(s):",
                errors.len()
            ));
            for err in errors {
                auth_hints.push(format!("  - {}", err));
            }
        }

        (outcomes, auth_hints)
    }
}

fn payload_error(url: &str, reason: impl Into<String>) -> RegistryError {
    RegistryError::DownloadFailed {
        url: redacted_download_url(url),
        reason: reason.into(),
    }
}

pub(crate) fn validate_wasm_payload(bytes: &[u8], url: &str) -> Result<(), RegistryError> {
    if bytes.len() < 8 || bytes.len() > MAX_WASM_BYTES || !bytes.starts_with(b"\0asm") {
        return Err(payload_error(
            url,
            format!(
                "downloaded WASM must be a valid-looking module between 8 and {MAX_WASM_BYTES} bytes"
            ),
        ));
    }
    Ok(())
}

pub(crate) fn validate_capabilities_payload(
    kind: ManifestKind,
    name: &str,
    bytes: &[u8],
    url: &str,
) -> Result<(), RegistryError> {
    if bytes.len() > MAX_CAPABILITIES_BYTES {
        return Err(payload_error(
            url,
            format!("capabilities exceed the {MAX_CAPABILITIES_BYTES}-byte limit"),
        ));
    }
    match kind {
        ManifestKind::Tool => {
            crate::tools::wasm::CapabilitiesFile::from_bytes(bytes).map_err(|error| {
                payload_error(url, format!("invalid tool capabilities: {error}"))
            })?;
        }
        ManifestKind::Channel => {
            let capabilities = crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(bytes)
                .map_err(|error| {
                    payload_error(url, format!("invalid channel capabilities: {error}"))
                })?;
            if capabilities.r#type != "channel" || capabilities.name != name {
                return Err(payload_error(
                    url,
                    "channel capabilities type/name does not match the registry manifest",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) async fn read_regular_file_bounded(
    path: PathBuf,
    limit: usize,
) -> Result<Option<Vec<u8>>, RegistryError> {
    tokio::task::spawn_blocking(move || read_regular_file_bounded_sync(&path, limit))
        .await
        .map_err(|error| {
            RegistryError::Io(std::io::Error::other(format!(
                "capabilities read task failed: {error}"
            )))
        })?
        .map_err(RegistryError::Io)
}

fn read_regular_file_bounded_sync(path: &Path, limit: usize) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read as _;

    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > limit as u64
    {
        return Err(std::io::Error::other(
            "capabilities source is not a bounded regular file",
        ));
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
    if !opened_metadata.is_file() || opened_metadata.len() > limit as u64 {
        return Err(std::io::Error::other(
            "capabilities source changed or exceeds the size limit",
        ));
    }
    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    file.by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(std::io::Error::other(
            "capabilities source exceeds the size limit",
        ));
    }
    Ok(Some(bytes))
}

pub(crate) async fn publish_extension_files(
    target_wasm: PathBuf,
    target_caps: PathBuf,
    wasm: Vec<u8>,
    capabilities: Option<Vec<u8>>,
    force: bool,
) -> Result<(), RegistryError> {
    let existing_path = target_wasm.clone();
    let existing_name = target_wasm
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("extension")
        .to_owned();
    thinclaw_platform::publish_file_pair(
        target_wasm,
        target_caps,
        wasm,
        capabilities,
        if force {
            thinclaw_platform::ExistingPairPolicy::Replace
        } else {
            thinclaw_platform::ExistingPairPolicy::Refuse
        },
    )
    .await
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            RegistryError::AlreadyInstalled {
                name: existing_name,
                path: existing_path,
            }
        } else {
            RegistryError::Io(error)
        }
    })
}

#[cfg(test)]
fn publish_extension_files_sync(
    target_wasm: &Path,
    target_caps: &Path,
    wasm: &[u8],
    capabilities: Option<&[u8]>,
) -> std::io::Result<()> {
    thinclaw_platform::publish_file_pair_sync(
        target_wasm,
        target_caps,
        wasm,
        capabilities,
        thinclaw_platform::ExistingPairPolicy::Replace,
    )
}

/// Download an artifact from a URL.
async fn download_artifact(url: &str, max_bytes: usize) -> Result<bytes::Bytes, RegistryError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(120),
        download_artifact_inner(url, max_bytes),
    )
    .await
    .map_err(|_| RegistryError::DownloadFailed {
        url: redacted_download_url(url),
        reason: "artifact download timed out".to_string(),
    })?
}

async fn download_artifact_inner(
    url: &str,
    max_bytes: usize,
) -> Result<bytes::Bytes, RegistryError> {
    let mut current = url.to_string();
    let allowlist = vec![
        "github.com".to_string(),
        "objects.githubusercontent.com".to_string(),
        "github-releases.githubusercontent.com".to_string(),
        "raw.githubusercontent.com".to_string(),
        "*.githubusercontent.com".to_string(),
    ];

    for redirect_count in 0..=5 {
        let guarded = thinclaw_tools_core::validate_outbound_url_pinned_async(
            &current,
            &thinclaw_tools_core::OutboundUrlGuardOptions {
                require_https: true,
                upgrade_http_to_https: false,
                allowlist: allowlist.clone(),
            },
        )
        .await
        .map_err(|_| RegistryError::DownloadFailed {
            url: redacted_download_url(&current),
            reason: "artifact URL failed outbound network validation".to_string(),
        })?;
        if guarded.url.port_or_known_default() != Some(443) {
            return Err(RegistryError::DownloadFailed {
                url: redacted_download_url(&current),
                reason: "artifact URL must use the standard HTTPS port".to_string(),
            });
        }
        let host = guarded
            .url
            .host_str()
            .ok_or_else(|| RegistryError::DownloadFailed {
                url: redacted_download_url(&current),
                reason: "artifact URL has no host".to_string(),
            })?;
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .connect_timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        if !guarded.pinned_addrs.is_empty() {
            builder = builder.resolve_to_addrs(host, &guarded.pinned_addrs);
        }
        let client = builder
            .build()
            .map_err(|error| RegistryError::DownloadFailed {
                url: redacted_download_url(&current),
                reason: download_failure_reason(&error),
            })?;
        let response = client
            .get(guarded.url.clone())
            .send()
            .await
            .map_err(|error| RegistryError::DownloadFailed {
                url: redacted_download_url(&current),
                reason: download_failure_reason(&error),
            })?;

        if response.status().is_redirection() {
            if redirect_count == 5 {
                return Err(RegistryError::DownloadFailed {
                    url: redacted_download_url(&current),
                    reason: "artifact download exceeded 5 redirects".to_string(),
                });
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| RegistryError::DownloadFailed {
                    url: redacted_download_url(&current),
                    reason: "artifact redirect omitted its Location header".to_string(),
                })?
                .to_str()
                .map_err(|_| RegistryError::DownloadFailed {
                    url: redacted_download_url(&current),
                    reason: "artifact redirect Location is not valid text".to_string(),
                })?;
            current = guarded
                .url
                .join(location)
                .map_err(|_| RegistryError::DownloadFailed {
                    url: redacted_download_url(&current),
                    reason: "artifact redirect Location is invalid".to_string(),
                })?
                .to_string();
            continue;
        }

        if !response.status().is_success() {
            return Err(RegistryError::DownloadFailed {
                url: redacted_download_url(&current),
                reason: format!("http status {}", response.status().as_u16()),
            });
        }
        return crate::http_response::bounded_bytes(response, max_bytes)
            .await
            .map(bytes::Bytes::from)
            .map_err(|error| RegistryError::DownloadFailed {
                url: redacted_download_url(&current),
                reason: format!("failed to read response body: {error}"),
            });
    }

    Err(RegistryError::DownloadFailed {
        url: redacted_download_url(url),
        reason: "artifact redirect processing failed".to_string(),
    })
}

/// Verify SHA256 of downloaded bytes.
fn verify_sha256(bytes: &[u8], expected: &str, url: &str) -> Result<(), RegistryError> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hex::encode(hasher.finalize());

    if actual != expected {
        return Err(RegistryError::ChecksumMismatch {
            url: redacted_download_url(url),
            expected_sha256: expected.to_string(),
            actual_sha256: actual,
        });
    }
    Ok(())
}

/// Check if bytes start with gzip magic number (0x1f 0x8b).
fn is_gzip(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b
}

/// Result of extracting a tar.gz bundle.
struct ExtractResult {
    wasm: Vec<u8>,
    capabilities: Option<Vec<u8>>,
}

/// Extract a tar.gz archive, looking for `{name}.wasm` and `{name}.capabilities.json`.
fn extract_tar_gz(bytes: &[u8], name: &str, url: &str) -> Result<ExtractResult, RegistryError> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let url = redacted_download_url(url);
    let decoder = GzDecoder::new(bytes);
    let mut archive = Archive::new(decoder);
    // Defense-in-depth: do not preserve permissions or extended attributes
    archive.set_preserve_permissions(false);
    #[cfg(any(unix, target_os = "redox"))]
    archive.set_unpack_xattrs(false);

    let wasm_filename = format!("{}.wasm", name);
    let caps_filename = format!("{}.capabilities.json", name);
    let mut wasm = None;
    let mut capabilities = None;

    let entries = archive
        .entries()
        .map_err(|e| RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!("failed to read tar.gz entries: {}", e),
        })?;

    for (entry_index, entry) in entries.enumerate() {
        if entry_index >= MAX_ARCHIVE_ENTRIES {
            return Err(RegistryError::DownloadFailed {
                url: url.to_string(),
                reason: format!("archive exceeds the {MAX_ARCHIVE_ENTRIES}-entry limit"),
            });
        }
        let mut entry = entry.map_err(|e| RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!("failed to read tar.gz entry: {}", e),
        })?;

        let entry_path = entry
            .path()
            .map_err(|e| RegistryError::DownloadFailed {
                url: url.to_string(),
                reason: format!("invalid path in tar.gz: {}", e),
            })?
            .to_path_buf();

        // Match by filename (ignoring any directory prefix in the archive)
        let filename = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if filename == wasm_filename {
            if wasm.is_some() {
                return Err(RegistryError::DownloadFailed {
                    url: url.to_string(),
                    reason: format!("archive contains duplicate '{wasm_filename}' entries"),
                });
            }
            if !entry.header().entry_type().is_file() || entry.size() > MAX_WASM_BYTES as u64 {
                return Err(RegistryError::DownloadFailed {
                    url: url.to_string(),
                    reason: format!(
                        "archive WASM entry must be a regular file no larger than {MAX_WASM_BYTES} bytes"
                    ),
                });
            }
            wasm = Some(read_archive_entry(
                &mut entry,
                MAX_WASM_BYTES,
                &wasm_filename,
                &url,
            )?);
        } else if filename == caps_filename {
            if capabilities.is_some() {
                return Err(RegistryError::DownloadFailed {
                    url: url.to_string(),
                    reason: format!("archive contains duplicate '{caps_filename}' entries"),
                });
            }
            if !entry.header().entry_type().is_file()
                || entry.size() > MAX_CAPABILITIES_BYTES as u64
            {
                return Err(RegistryError::DownloadFailed {
                    url: url.to_string(),
                    reason: format!(
                        "archive capabilities entry must be a regular file no larger than {MAX_CAPABILITIES_BYTES} bytes"
                    ),
                });
            }
            capabilities = Some(read_archive_entry(
                &mut entry,
                MAX_CAPABILITIES_BYTES,
                &caps_filename,
                &url,
            )?);
        }
    }

    let wasm = wasm.ok_or_else(|| RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!(
                "tar.gz archive does not contain a wasm binary (expected '{}'). Archive may be malformed.",
                wasm_filename,
            ),
        })?;

    Ok(ExtractResult { wasm, capabilities })
}

fn read_archive_entry(
    entry: &mut impl std::io::Read,
    limit: usize,
    filename: &str,
    url: &str,
) -> Result<Vec<u8>, RegistryError> {
    use std::io::Read as _;

    let url = redacted_download_url(url);
    let mut data = Vec::new();
    entry
        .take(limit as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|error| RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!("failed to read {filename} from archive: {error}"),
        })?;
    if data.len() > limit {
        return Err(RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!("archive entry '{filename}' exceeds the {limit}-byte limit"),
        });
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::registry::manifest::{ArtifactSpec, SourceSpec};

    fn test_manifest(
        name: &str,
        source_dir: &str,
        artifact_url: Option<String>,
        sha256: Option<&str>,
    ) -> ExtensionManifest {
        test_manifest_with_kind(name, source_dir, artifact_url, sha256, ManifestKind::Tool)
    }

    fn test_manifest_with_kind(
        name: &str,
        source_dir: &str,
        artifact_url: Option<String>,
        sha256: Option<&str>,
        kind: ManifestKind,
    ) -> ExtensionManifest {
        let mut artifacts = HashMap::new();
        if artifact_url.is_some() || sha256.is_some() {
            artifacts.insert(
                "wasm32-wasip2".to_string(),
                ArtifactSpec {
                    url: artifact_url,
                    sha256: sha256.map(ToString::to_string),
                    capabilities_url: None,
                },
            );
        }

        ExtensionManifest {
            name: name.to_string(),
            display_name: name.to_string(),
            kind,
            version: "0.1.0".to_string(),
            description: "test manifest".to_string(),
            keywords: Vec::new(),
            source: SourceSpec {
                dir: source_dir.to_string(),
                capabilities: format!("{}.capabilities.json", name),
                crate_name: name.to_string(),
            },
            artifacts,
            auth_summary: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn test_installer_creation() {
        let installer = RegistryInstaller::new(
            PathBuf::from("/repo"),
            PathBuf::from("/home/.thinclaw/tools"),
            PathBuf::from("/home/.thinclaw/channels"),
        );
        assert_eq!(installer.repo_root, PathBuf::from("/repo"));
    }

    #[test]
    fn test_is_gzip() {
        assert!(is_gzip(&[0x1f, 0x8b, 0x08]));
        assert!(!is_gzip(&[0x00, 0x61, 0x73, 0x6d])); // WASM magic
        assert!(!is_gzip(&[0x1f])); // Too short
        assert!(!is_gzip(&[]));
    }

    #[test]
    fn test_verify_sha256_valid() {
        use sha2::{Digest, Sha256};
        let data = b"hello world";
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = hex::encode(hasher.finalize());
        assert!(verify_sha256(data, &hash, "test://url").is_ok());
    }

    #[test]
    fn test_verify_sha256_invalid() {
        let err = verify_sha256(b"data", "0000", "test://url").expect_err("checksum mismatch");
        assert!(matches!(err, RegistryError::ChecksumMismatch { .. }));
    }

    #[tokio::test]
    async fn test_install_from_source_rejects_path_traversal_name() {
        let temp = tempfile::tempdir().expect("tempdir");
        let installer = RegistryInstaller::new(
            temp.path().to_path_buf(),
            temp.path().join("tools"),
            temp.path().join("channels"),
        );

        let manifest = test_manifest("../evil", "tools-src/evil", None, None);

        let result = installer.install_from_source(&manifest, false).await;
        match result {
            Err(RegistryError::InvalidManifest { field, .. }) => {
                assert_eq!(field, "name");
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_install_from_artifact_rejects_non_https_url() {
        let temp = tempfile::tempdir().expect("tempdir");
        let installer = RegistryInstaller::new(
            temp.path().to_path_buf(),
            temp.path().join("tools"),
            temp.path().join("channels"),
        );

        let manifest = test_manifest(
            "demo",
            "tools-src/demo",
            Some(
                "http://github.com/nearai/thinclaw/releases/latest/download/demo.wasm".to_string(),
            ),
            None,
        );

        let result = installer.install_from_artifact(&manifest, false).await;
        match result {
            Err(RegistryError::InvalidManifest { field, .. }) => {
                assert_eq!(field, "artifacts.url");
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_install_from_artifact_rejects_disallowed_host() {
        let temp = tempfile::tempdir().expect("tempdir");
        let installer = RegistryInstaller::new(
            temp.path().to_path_buf(),
            temp.path().join("tools"),
            temp.path().join("channels"),
        );

        let manifest = test_manifest(
            "demo",
            "tools-src/demo",
            Some("https://169.254.169.254/latest/meta-data".to_string()),
            None,
        );

        let result = installer.install_from_artifact(&manifest, false).await;
        match result {
            Err(RegistryError::InvalidManifest { field, .. }) => {
                assert_eq!(field, "artifacts.url");
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_install_from_artifact_rejects_null_sha256() {
        let temp = tempfile::tempdir().expect("tempdir");
        let installer = RegistryInstaller::new(
            temp.path().to_path_buf(),
            temp.path().join("tools"),
            temp.path().join("channels"),
        );

        // Valid URL but no sha256 — should be rejected before any download attempt
        let manifest = test_manifest(
            "demo",
            "tools-src/demo",
            Some(
                "https://github.com/nearai/thinclaw/releases/latest/download/demo-wasm32-wasip2.tar.gz".to_string(),
            ),
            None, // sha256 = null
        );

        let result = installer.install_from_artifact(&manifest, false).await;
        match result {
            Err(RegistryError::InvalidManifest { field, reason, .. }) => {
                assert_eq!(field, "artifacts.wasm32-wasip2.sha256");
                assert!(reason.contains("required"), "reason: {}", reason);
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn test_should_attempt_source_fallback_policy() {
        let download = RegistryError::DownloadFailed {
            url: "https://github.com/nearai/thinclaw/releases/latest/download/demo.wasm"
                .to_string(),
            reason: "http status 404".to_string(),
        };
        assert!(should_attempt_source_fallback(&download));

        let already = RegistryError::AlreadyInstalled {
            name: "demo".to_string(),
            path: PathBuf::from("/tmp/demo.wasm"),
        };
        assert!(!should_attempt_source_fallback(&already));

        let checksum = RegistryError::ChecksumMismatch {
            url: "https://github.com/nearai/thinclaw/releases/latest/download/demo.wasm"
                .to_string(),
            expected_sha256: "deadbeef".to_string(),
            actual_sha256: "feedface".to_string(),
        };
        assert!(!should_attempt_source_fallback(&checksum));

        // InvalidManifest for host not allowed → no fallback
        let invalid = RegistryError::InvalidManifest {
            name: "demo".to_string(),
            field: "artifacts.wasm32-wasip2.url",
            reason: "host not allowed".to_string(),
        };
        assert!(!should_attempt_source_fallback(&invalid));

        // InvalidManifest for missing sha256 → DO fall back (placeholder artifacts)
        let missing_sha = RegistryError::InvalidManifest {
            name: "demo".to_string(),
            field: "artifacts.wasm32-wasip2.sha256",
            reason: "sha256 is required for artifact downloads".to_string(),
        };
        assert!(should_attempt_source_fallback(&missing_sha));

        // ToolchainMissing → no fallback (source build IS the problem)
        let toolchain = RegistryError::ToolchainMissing {
            name: "demo".to_string(),
        };
        assert!(!should_attempt_source_fallback(&toolchain));
    }

    #[test]
    fn test_extract_tar_gz() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use tar::Builder;

        // Create a tar.gz in memory with test.wasm and test.capabilities.json
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = Builder::new(&mut encoder);

            let wasm_data = b"\0asm\x01\x00\x00\x00";
            let mut header = tar::Header::new_gnu();
            header.set_size(wasm_data.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, "test.wasm", &wasm_data[..])
                .unwrap();

            let caps_data = br#"{"auth":null}"#;
            let mut header = tar::Header::new_gnu();
            header.set_size(caps_data.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, "test.capabilities.json", &caps_data[..])
                .unwrap();

            builder.finish().unwrap();
        }
        let gz_bytes = encoder.finish().unwrap();

        let result = extract_tar_gz(&gz_bytes, "test", "test://url").unwrap();

        assert_eq!(result.wasm, b"\0asm\x01\x00\x00\x00");
        assert_eq!(
            result.capabilities.as_deref(),
            Some(br#"{"auth":null}"#.as_slice())
        );
    }

    #[tokio::test]
    async fn test_install_from_source_rejects_wrong_prefix_for_channel() {
        let temp = tempfile::tempdir().expect("tempdir");
        let installer = RegistryInstaller::new(
            temp.path().to_path_buf(),
            temp.path().join("tools"),
            temp.path().join("channels"),
        );

        // Channel manifest with tools-src/ prefix should be rejected
        let manifest = test_manifest_with_kind(
            "telegram",
            "tools-src/telegram",
            None,
            None,
            ManifestKind::Channel,
        );

        let result = installer.install_from_source(&manifest, false).await;
        match result {
            Err(RegistryError::InvalidManifest { field, reason, .. }) => {
                assert_eq!(field, "source.dir");
                assert!(reason.contains("channels-src/"), "reason: {}", reason);
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_install_from_source_accepts_correct_channel_prefix() {
        let temp = tempfile::tempdir().expect("tempdir");
        let installer = RegistryInstaller::new(
            temp.path().to_path_buf(),
            temp.path().join("tools"),
            temp.path().join("channels"),
        );

        // Channel manifest with channels-src/ prefix should pass validation
        // (will fail later because source dir doesn't exist, which is fine)
        let manifest = test_manifest_with_kind(
            "telegram",
            "channels-src/telegram",
            None,
            None,
            ManifestKind::Channel,
        );

        let result = installer.install_from_source(&manifest, false).await;
        match result {
            Err(RegistryError::ManifestRead { reason, .. }) => {
                assert!(
                    reason.contains("source directory does not exist"),
                    "reason: {}",
                    reason
                );
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn test_extract_tar_gz_missing_wasm() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use tar::Builder;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = Builder::new(&mut encoder);

            let data = b"not a wasm file";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, "wrong.txt", &data[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let gz_bytes = encoder.finish().unwrap();

        let result = extract_tar_gz(&gz_bytes, "test", "test://url");

        assert!(result.is_err());
    }

    #[test]
    fn publish_extension_files_replaces_both_files_and_removes_stale_capabilities() {
        let temp = tempfile::tempdir().unwrap();
        let wasm_path = temp.path().join("test.wasm");
        let caps_path = temp.path().join("test.capabilities.json");
        std::fs::write(&wasm_path, b"old wasm").unwrap();
        std::fs::write(&caps_path, b"old caps").unwrap();

        publish_extension_files_sync(&wasm_path, &caps_path, b"\0asm\x01\x00\x00\x00", None)
            .unwrap();

        assert_eq!(std::fs::read(&wasm_path).unwrap(), b"\0asm\x01\x00\x00\x00");
        assert!(!caps_path.exists());
        assert!(std::fs::read_dir(temp.path()).unwrap().all(|entry| {
            let name = entry.unwrap().file_name();
            let name = name.to_string_lossy();
            !name.contains(".install.tmp") && !name.contains(".install.bak")
        }));
    }

    #[test]
    fn concurrent_publishers_leave_one_coherent_generation() {
        let temp = tempfile::tempdir().unwrap();
        let wasm_path = temp.path().join("test.wasm");
        let caps_path = temp.path().join("test.capabilities.json");
        let mut workers = Vec::new();
        for generation in 0..16 {
            let wasm_path = wasm_path.clone();
            let caps_path = caps_path.clone();
            workers.push(std::thread::spawn(move || {
                let wasm = format!("wasm-{generation}");
                let caps = format!("caps-{generation}");
                publish_extension_files_sync(
                    &wasm_path,
                    &caps_path,
                    wasm.as_bytes(),
                    Some(caps.as_bytes()),
                )
                .unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }

        let wasm = std::fs::read_to_string(wasm_path).unwrap();
        let caps = std::fs::read_to_string(caps_path).unwrap();
        assert_eq!(wasm.strip_prefix("wasm-"), caps.strip_prefix("caps-"));
    }

    #[cfg(unix)]
    #[test]
    fn publish_extension_files_refuses_existing_symlink_targets() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        let wasm_path = temp.path().join("test.wasm");
        let caps_path = temp.path().join("test.capabilities.json");
        std::fs::write(&outside, b"outside").unwrap();
        symlink(&outside, &wasm_path).unwrap();

        assert!(
            publish_extension_files_sync(&wasm_path, &caps_path, b"\0asm\x01\x00\x00\x00", None,)
                .is_err()
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
    }
}
