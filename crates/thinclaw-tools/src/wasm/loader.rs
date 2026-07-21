//! Generic WASM tool loader for loading tools from files or directories.
//!
//! This module provides a way to load WASM tools dynamically at runtime from:
//! - A directory containing `<name>.wasm` and `<name>.capabilities.json`
//! - Build artifacts in `tools-src/` (dev mode, auto-detected)
//! - Database storage (via [`WasmToolStore`])
//!
//! # Example: Loading from Directory
//!
//! ```text
//! ~/.thinclaw/tools/
//! ├── slack.wasm
//! ├── slack.capabilities.json
//! ├── github.wasm
//! └── github.capabilities.json
//! ```
//!
//! ```ignore
//! let loader = WasmToolLoader::new(runtime, registry);
//! loader.load_from_dir(Path::new("~/.thinclaw/tools/")).await?;
//! ```
//!
//! # Dev Mode
//!
//! When `load_dev_tools()` is called, the loader scans `tools-src/*/` for build
//! artifacts. Tools found there are loaded directly from the build output,
//! skipping the install directory. This means during development you just
//! rebuild the WASM and restart the host, no manual copy step needed.
//!
//! # Security
//!
//! Tools loaded from files are assigned `TrustLevel::User` by default, meaning
//! they run with the most restrictive permissions. Only tools explicitly marked
//! as `verified` or `system` in the database get elevated trust.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt as _;
use tokio::fs;

use crate::wasm::capabilities_schema::CapabilitiesFile;
use crate::wasm::ports::{WasmToolRegistrar, WasmToolRegistration};
use crate::wasm::{
    Capabilities, WasmError, WasmStorageError, WasmToolRuntime, WasmToolStore,
    resolve_oauth_refresh_config,
};

const MAX_WASM_MODULE_BYTES: usize = 64 * 1024 * 1024;
const MAX_CAPABILITIES_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOOLS_PER_DIRECTORY: usize = 64;
const MAX_STORED_TOOLS_PER_USER: usize = 256;
const MAX_PARALLEL_COMPILATIONS: usize = 4;

fn is_valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !crate::registry::PROTECTED_TOOL_NAMES.contains(&name)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn publication_journal_path(wasm_path: &Path) -> Option<PathBuf> {
    let parent = wasm_path.parent()?;
    let filename = wasm_path.file_name()?.to_str()?;
    Some(parent.join(format!(".{filename}.installing.json")))
}

fn publication_in_progress(wasm_path: &Path) -> Result<bool, std::io::Error> {
    let Some(path) = publication_journal_path(wasm_path) else {
        return Ok(false);
    };
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

async fn read_regular_file_bounded(
    path: &Path,
    max_bytes: usize,
) -> Result<Vec<u8>, std::io::Error> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "tool package path is not a regular file",
            ));
        }
        if metadata.len() > max_bytes as u64 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "tool package file exceeds the size limit",
            ));
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&path)?;
        let opened = file.metadata()?;
        if !opened.is_file() || opened.len() > max_bytes as u64 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "opened tool package file is invalid or oversized",
            ));
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(opened.len())
                .unwrap_or(max_bytes)
                .min(max_bytes),
        );
        file.by_ref()
            .take((max_bytes + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > max_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "tool package file exceeds the size limit",
            ));
        }
        Ok(bytes)
    })
    .await
    .map_err(|error| std::io::Error::other(format!("tool file reader panicked: {error}")))?
}

/// Error during WASM tool loading.
#[derive(Debug, thiserror::Error)]
pub enum WasmLoadError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WASM file not found: {0}")]
    WasmNotFound(PathBuf),

    #[error("Capabilities file not found: {0}")]
    CapabilitiesNotFound(PathBuf),

    #[error("Invalid capabilities JSON: {0}")]
    InvalidCapabilities(String),

    #[error("WASM compilation error: {0}")]
    Compilation(#[from] WasmError),

    #[error("Storage error: {0}")]
    Storage(#[from] WasmStorageError),

    #[error("Registration error: {0}")]
    Registration(String),

    #[error("Invalid tool name: {0}")]
    InvalidName(String),
}

/// Loads WASM tools from files or storage into the registry.
pub struct WasmToolLoader<R: WasmToolRegistrar> {
    runtime: Arc<WasmToolRuntime>,
    registry: Arc<R>,
    secrets_store: Option<Arc<R::SecretResolver>>,
    tool_invoker: Option<Arc<R::ToolInvoker>>,
}

impl<R> WasmToolLoader<R>
where
    R: WasmToolRegistrar,
{
    /// Create a new loader with the given runtime and registry.
    pub fn new(runtime: Arc<WasmToolRuntime>, registry: Arc<R>) -> Self {
        Self {
            runtime,
            registry,
            secrets_store: None,
            tool_invoker: None,
        }
    }

    /// Set the secrets store for credential injection in WASM tools.
    pub fn with_secrets_store(mut self, store: Arc<R::SecretResolver>) -> Self {
        self.secrets_store = Some(store);
        self
    }

    /// Set the host-mediated tool invoker used by WASM `tool_invoke`.
    pub fn with_tool_invoker(mut self, invoker: Arc<R::ToolInvoker>) -> Self {
        self.tool_invoker = Some(invoker);
        self
    }

    /// Load a single WASM tool from a file pair.
    ///
    /// Expects:
    /// - `wasm_path`: Path to the `.wasm` file
    /// - `capabilities_path`: Path to the `.capabilities.json` file (optional)
    ///
    /// If no capabilities file is provided, the tool gets no capabilities (default deny).
    pub async fn load_from_files(
        &self,
        name: &str,
        wasm_path: &Path,
        capabilities_path: Option<&Path>,
    ) -> Result<(), WasmLoadError> {
        self.load_from_files_with_metadata(name, wasm_path, capabilities_path)
            .await
            .map(|_| ())
    }

    /// Load a file-backed tool and return the exact capabilities snapshot read
    /// under the package lock. Callers can use it for sidecar-derived behavior
    /// without reopening a possibly newer generation.
    pub async fn load_from_files_with_metadata(
        &self,
        name: &str,
        wasm_path: &Path,
        capabilities_path: Option<&Path>,
    ) -> Result<LoadedToolMetadata, WasmLoadError> {
        if !is_valid_tool_name(name) {
            return Err(WasmLoadError::InvalidName(name.to_string()));
        }

        if let Err(error) = fs::symlink_metadata(wasm_path).await {
            if error.kind() == std::io::ErrorKind::NotFound {
                return Err(WasmLoadError::WasmNotFound(wasm_path.to_path_buf()));
            }
            return Err(error.into());
        }
        let publication_guard =
            thinclaw_platform::acquire_artifact_read_lock(wasm_path.to_path_buf()).await?;
        if publication_in_progress(wasm_path)? {
            return Err(WasmLoadError::Io(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "tool package publication is incomplete",
            )));
        }

        // Read WASM bytes
        let wasm_bytes = read_regular_file_bounded(wasm_path, MAX_WASM_MODULE_BYTES).await?;
        if wasm_bytes.len() < 8 || !wasm_bytes.starts_with(b"\0asm") {
            return Err(WasmLoadError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "tool package is not a valid-looking WASM module",
            )));
        }

        // Read capabilities (optional) and extract OAuth refresh config
        let (capabilities, oauth_refresh, capabilities_file) = if let Some(cap_path) =
            capabilities_path
        {
            match fs::symlink_metadata(cap_path).await {
                Ok(_) => {
                    let cap_bytes =
                        read_regular_file_bounded(cap_path, MAX_CAPABILITIES_FILE_BYTES).await?;
                    let cap_file = CapabilitiesFile::from_bytes(&cap_bytes)
                        .map_err(|e| WasmLoadError::InvalidCapabilities(e.to_string()))?;
                    let caps = cap_file.to_capabilities();
                    let oauth = resolve_oauth_refresh_config(&cap_file);
                    (caps, oauth, Some(cap_file))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    tracing::warn!(
                        path = %cap_path.display(),
                        "Capabilities file not found, using default (no permissions)"
                    );
                    (Capabilities::default(), None, None)
                }
                Err(error) => return Err(error.into()),
            }
        } else {
            (Capabilities::default(), None, None)
        };
        // Register the tool
        self.registry
            .register_wasm(WasmToolRegistration {
                name,
                wasm_bytes: &wasm_bytes,
                runtime: &self.runtime,
                capabilities,
                limits: None,
                description: None,
                schema: None,
                secrets: self.secrets_store.clone(),
                oauth_refresh,
                tool_invoker: self.tool_invoker.clone(),
            })
            .await
            .map_err(|error| WasmLoadError::Registration(error.to_string()))?;
        drop(publication_guard);

        tracing::info!(
            name = name,
            wasm_path = %wasm_path.display(),
            "Loaded WASM tool from file"
        );

        Ok(LoadedToolMetadata { capabilities_file })
    }

    /// Load all WASM tools from a directory.
    ///
    /// Scans the directory for `*.wasm` files and loads each one, looking for
    /// a matching `*.capabilities.json` sidecar file.
    ///
    /// # Directory Layout
    ///
    /// ```text
    /// tools/
    /// ├── slack.wasm                  <- Tool WASM component
    /// ├── slack.capabilities.json     <- Capabilities (optional)
    /// ├── github.wasm
    /// └── github.capabilities.json
    /// ```
    ///
    /// Tools without a capabilities file get no permissions (default deny).
    pub async fn load_from_dir(&self, dir: &Path) -> Result<LoadResults, WasmLoadError> {
        let directory_metadata = fs::symlink_metadata(dir).await?;
        if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
            return Err(WasmLoadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                format!("{} is not a real directory", dir.display()),
            )));
        }

        let mut results = LoadResults::default();

        // Collect all .wasm entries first, then load in parallel
        let mut tool_entries = Vec::new();
        let mut entries = fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                continue;
            }

            let file_type = entry.file_type().await?;
            if !file_type.is_file() || file_type.is_symlink() {
                continue;
            }

            let name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => {
                    results.errors.push((
                        path.clone(),
                        WasmLoadError::InvalidName("invalid filename".to_string()),
                    ));
                    continue;
                }
            };
            if !is_valid_tool_name(&name) {
                results
                    .errors
                    .push((path.clone(), WasmLoadError::InvalidName(name)));
                continue;
            }
            if publication_in_progress(&path)? {
                results.errors.push((
                    path.clone(),
                    WasmLoadError::Io(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "tool package publication is incomplete",
                    )),
                ));
                continue;
            }
            if tool_entries.len() >= MAX_TOOLS_PER_DIRECTORY {
                results.errors.push((
                    path.clone(),
                    WasmLoadError::InvalidName(format!(
                        "tool directory exceeds the {MAX_TOOLS_PER_DIRECTORY}-tool limit"
                    )),
                ));
                continue;
            }

            let cap_path = path.with_extension("capabilities.json");
            tool_entries.push((name, path, Some(cap_path)));
        }

        // Keep Wasmtime compilation concurrency bounded: the directory is
        // operator-controlled and each compile can consume substantial memory.
        let mut load_results = futures::stream::iter(tool_entries.iter().enumerate().map(
            |(index, (name, path, cap_path))| async move {
                (
                    index,
                    self.load_from_files(name, path, cap_path.as_deref()).await,
                )
            },
        ))
        .buffer_unordered(MAX_PARALLEL_COMPILATIONS)
        .collect::<Vec<_>>()
        .await;
        load_results.sort_by_key(|(index, _)| *index);

        for ((name, path, _), (_, result)) in tool_entries.into_iter().zip(load_results) {
            match result {
                Ok(()) => {
                    results.loaded.push(name);
                }
                Err(e) => {
                    tracing::error!(
                        name = name,
                        path = %path.display(),
                        error = %e,
                        "Failed to load WASM tool"
                    );
                    results.errors.push((path, e));
                }
            }
        }

        if !results.loaded.is_empty() {
            tracing::info!(
                count = results.loaded.len(),
                tools = ?results.loaded,
                "Loaded WASM tools from directory"
            );
        }

        Ok(results)
    }

    /// Load a WASM tool from database storage.
    ///
    /// This is a convenience wrapper around [`ToolRegistry::register_wasm_from_storage`].
    pub async fn load_from_storage(
        &self,
        store: &dyn WasmToolStore,
        user_id: &str,
        tool_name: &str,
    ) -> Result<(), WasmLoadError> {
        self.registry
            .register_wasm_from_storage(
                store,
                &self.runtime,
                user_id,
                tool_name,
                self.tool_invoker.clone(),
            )
            .await
            .map_err(|error| WasmLoadError::Registration(error.to_string()))?;

        tracing::info!(
            user_id = user_id,
            name = tool_name,
            "Loaded WASM tool from storage"
        );

        Ok(())
    }

    /// Load all active WASM tools for a user from storage.
    pub async fn load_all_from_storage(
        &self,
        store: &dyn WasmToolStore,
        user_id: &str,
    ) -> Result<LoadResults, WasmLoadError> {
        let tools = store.list(user_id).await?;
        if tools.len() > MAX_STORED_TOOLS_PER_USER {
            return Err(WasmLoadError::Storage(WasmStorageError::InvalidData(
                format!("stored tool count exceeds the {MAX_STORED_TOOLS_PER_USER}-tool limit"),
            )));
        }
        let mut results = LoadResults::default();

        for tool in tools {
            // Skip non-active tools
            if tool.status != crate::wasm::ToolStatus::Active {
                continue;
            }

            match self.load_from_storage(store, user_id, &tool.name).await {
                Ok(()) => {
                    results.loaded.push(tool.name);
                }
                Err(e) => {
                    tracing::error!(
                        name = tool.name,
                        user_id = user_id,
                        error = %e,
                        "Failed to load WASM tool from storage"
                    );
                    results.errors.push((PathBuf::from(&tool.name), e));
                }
            }
        }

        Ok(results)
    }
}

/// Metadata parsed from the same locked generation as a loaded tool.
#[derive(Debug)]
pub struct LoadedToolMetadata {
    pub capabilities_file: Option<CapabilitiesFile>,
}

/// Results from loading multiple tools.
#[derive(Debug, Default)]
pub struct LoadResults {
    /// Names of successfully loaded tools.
    pub loaded: Vec<String>,

    /// Errors encountered (path/name, error).
    pub errors: Vec<(PathBuf, WasmLoadError)>,
}

impl LoadResults {
    /// Check if all tools loaded successfully.
    pub fn all_succeeded(&self) -> bool {
        self.errors.is_empty()
    }

    /// Get the count of successfully loaded tools.
    pub fn success_count(&self) -> usize {
        self.loaded.len()
    }

    /// Get the count of failed tools.
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }
}

/// Compile-time project root, used to locate tools-src/ in dev builds.
const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Resolve the WASM target directory for a given crate directory.
///
/// Checks (in order):
/// 1. `CARGO_TARGET_DIR` env var (shared target dir)
/// 2. `<crate_dir>/target/` (default per-crate layout)
pub fn resolve_wasm_target_dir(crate_dir: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate_dir.join("target"))
}

/// Return the expected path to a compiled WASM artifact for a given crate.
///
/// Combines [`resolve_wasm_target_dir`] with the `wasm32-wasip2/release/` subdirectory
/// and the binary name without extension (e.g. `slack_tool`).
///
/// `binary_name` should not include the `.wasm` extension; it is appended automatically.
///
/// This is a convenience function for callers that know the exact triple (wasip2)
/// and binary name. For multi-triple search, use
/// a discovery helper that searches the desired target triples instead.
pub fn wasm_artifact_path(crate_dir: &Path, binary_name: &str) -> PathBuf {
    resolve_wasm_target_dir(crate_dir)
        .join("wasm32-wasip2/release")
        .join(format!("{}.wasm", binary_name))
}

/// Resolve the tools source directory.
///
/// Checks (in order):
/// 1. `THINCLAW_TOOLS_SRC` env var
/// 2. `<CARGO_MANIFEST_DIR>/tools-src/` (dev builds)
fn tools_src_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("THINCLAW_TOOLS_SRC") {
        return PathBuf::from(dir);
    }
    PathBuf::from(CARGO_MANIFEST_DIR).join("tools-src")
}

/// Discover WASM tools available as build artifacts in `tools-src/`.
///
/// Scans each subdirectory for:
/// - `tools-src/<name>/target/wasm32-wasip2/release/<crate_name>_tool.wasm`
/// - `tools-src/<name>/<name>-tool.capabilities.json`
///
/// Returns a map of install-name (e.g. "gmail-tool") to paths.
pub async fn discover_dev_tools() -> Result<HashMap<String, DiscoveredTool>, std::io::Error> {
    let src_dir = tools_src_dir();
    let mut tools = HashMap::new();

    let source_metadata = match fs::symlink_metadata(&src_dir).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(tools),
        Err(error) => return Err(error),
    };
    if source_metadata.file_type().is_symlink() || !source_metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "tool source directory is not a real directory",
        ));
    }

    let mut entries = fs::read_dir(&src_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let file_type = entry.file_type().await?;
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }

        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Convention: crate name uses underscores, directory uses hyphens
        let crate_name = dir_name.replace('-', "_");
        let install_name = format!("{}-tool", dir_name);
        if !is_valid_tool_name(&install_name) || tools.len() >= MAX_TOOLS_PER_DIRECTORY {
            continue;
        }

        let wasm_path = wasm_artifact_path(&path, &format!("{}_tool", crate_name));
        let wasm_metadata = match fs::symlink_metadata(&wasm_path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if wasm_metadata.file_type().is_symlink()
            || !wasm_metadata.is_file()
            || publication_in_progress(&wasm_path)?
        {
            continue;
        }

        let caps_path = path.join(format!("{}-tool.capabilities.json", dir_name));
        let capabilities_path = match fs::symlink_metadata(&caps_path).await {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                Some(caps_path)
            }
            Ok(_) => None,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };

        tools.insert(
            install_name,
            DiscoveredTool {
                wasm_path,
                capabilities_path,
            },
        );
    }

    Ok(tools)
}

/// Load WASM tools from build artifacts in `tools-src/`.
///
/// In dev mode, tools can be loaded directly from their build output without
/// needing to install them to `~/.thinclaw/tools/` first. Build artifacts
/// that are newer than installed copies take priority.
///
/// Set `THINCLAW_TOOLS_SRC` env var to override the source directory.
pub async fn load_dev_tools<R>(
    loader: &WasmToolLoader<R>,
    install_dir: &Path,
) -> Result<LoadResults, WasmLoadError>
where
    R: WasmToolRegistrar,
{
    let dev_tools = discover_dev_tools().await?;
    let mut results = LoadResults::default();

    if dev_tools.is_empty() {
        return Ok(results);
    }

    for (name, discovered) in &dev_tools {
        // Check if the build artifact is newer than the installed copy
        let installed_path = install_dir.join(format!("{}.wasm", name));
        let should_load = if installed_path.exists() {
            // Compare modification times: prefer fresher build artifact
            match (
                fs::metadata(&discovered.wasm_path).await,
                fs::metadata(&installed_path).await,
            ) {
                (Ok(dev_meta), Ok(inst_meta)) => {
                    let dev_modified = dev_meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                    let inst_modified = inst_meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                    dev_modified > inst_modified
                }
                _ => true,
            }
        } else {
            true
        };

        if !should_load {
            continue;
        }

        tracing::info!(
            name = name,
            wasm_path = %discovered.wasm_path.display(),
            "Loading dev tool from build artifacts (newer than installed)"
        );

        match loader
            .load_from_files(
                name,
                &discovered.wasm_path,
                discovered.capabilities_path.as_deref(),
            )
            .await
        {
            Ok(()) => {
                results.loaded.push(name.clone());
            }
            Err(e) => {
                tracing::error!(
                    name = name,
                    error = %e,
                    "Failed to load dev tool"
                );
                results.errors.push((discovered.wasm_path.clone(), e));
            }
        }
    }

    if !results.loaded.is_empty() {
        tracing::info!(
            count = results.loaded.len(),
            tools = ?results.loaded,
            "Loaded dev tools from build artifacts"
        );
    }

    Ok(results)
}

/// Discover WASM tool files in a directory without loading them.
///
/// Returns a map of tool name -> (wasm_path, capabilities_path).
pub async fn discover_tools(dir: &Path) -> Result<HashMap<String, DiscoveredTool>, std::io::Error> {
    let mut tools = HashMap::new();

    let directory_metadata = match fs::symlink_metadata(dir).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(tools),
        Err(error) => return Err(error),
    };
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "tool package directory is not a real directory",
        ));
    }

    let mut entries = fs::read_dir(dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }

        let file_type = entry.file_type().await?;
        if !file_type.is_file() || file_type.is_symlink() {
            continue;
        }

        let name = match path.file_stem().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !is_valid_tool_name(&name)
            || publication_in_progress(&path)?
            || tools.len() >= MAX_TOOLS_PER_DIRECTORY
        {
            continue;
        }

        let cap_path = path.with_extension("capabilities.json");
        let capabilities_path = match fs::symlink_metadata(&cap_path).await {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                Some(cap_path)
            }
            _ => None,
        };

        tools.insert(
            name,
            DiscoveredTool {
                wasm_path: path,
                capabilities_path,
            },
        );
    }

    Ok(tools)
}

/// A discovered WASM tool (not yet loaded).
#[derive(Debug)]
pub struct DiscoveredTool {
    /// Path to the WASM file.
    pub wasm_path: PathBuf,

    /// Path to the capabilities file (if present).
    pub capabilities_path: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use crate::wasm::loader::{WasmLoadError, discover_tools};

    #[tokio::test]
    async fn test_discover_tools_empty_dir() {
        let dir = TempDir::new().unwrap();
        let tools = discover_tools(dir.path()).await.unwrap();
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn test_discover_tools_with_wasm() {
        let dir = TempDir::new().unwrap();

        // Create a fake .wasm file
        let wasm_path = dir.path().join("test_tool.wasm");
        std::fs::File::create(&wasm_path).unwrap();

        let tools = discover_tools(dir.path()).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert!(tools.contains_key("test_tool"));
        assert!(tools["test_tool"].capabilities_path.is_none());
    }

    #[tokio::test]
    async fn test_discover_tools_with_capabilities() {
        let dir = TempDir::new().unwrap();

        // Create wasm and capabilities files
        std::fs::File::create(dir.path().join("slack.wasm")).unwrap();
        let mut cap_file =
            std::fs::File::create(dir.path().join("slack.capabilities.json")).unwrap();
        cap_file.write_all(b"{}").unwrap();

        let tools = discover_tools(dir.path()).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert!(tools["slack"].capabilities_path.is_some());
    }

    #[tokio::test]
    async fn test_discover_tools_ignores_non_wasm() {
        let dir = TempDir::new().unwrap();

        // Create non-wasm files
        std::fs::File::create(dir.path().join("readme.md")).unwrap();
        std::fs::File::create(dir.path().join("config.json")).unwrap();
        std::fs::File::create(dir.path().join("tool.wasm")).unwrap();

        let tools = discover_tools(dir.path()).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert!(tools.contains_key("tool"));
    }

    #[tokio::test]
    async fn test_discover_tools_hides_incomplete_publication() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("tool.wasm"), b"\0asm\x01\0\0\0").unwrap();
        std::fs::write(
            dir.path().join(".tool.wasm.installing.json"),
            br#"{"version":1}"#,
        )
        .unwrap();

        let tools = discover_tools(dir.path()).await.unwrap();
        assert!(!tools.contains_key("tool"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_discover_tools_rejects_symlink_packages() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let outside_wasm = outside.path().join("outside.wasm");
        std::fs::write(&outside_wasm, b"\0asm\x01\0\0\0").unwrap();
        symlink(&outside_wasm, dir.path().join("linked.wasm")).unwrap();

        let tools = discover_tools(dir.path()).await.unwrap();
        assert!(!tools.contains_key("linked"));
    }

    #[tokio::test]
    async fn test_discover_tools_enforces_directory_limit() {
        let dir = TempDir::new().unwrap();
        for index in 0..(super::MAX_TOOLS_PER_DIRECTORY + 5) {
            std::fs::write(
                dir.path().join(format!("tool_{index}.wasm")),
                b"\0asm\x01\0\0\0",
            )
            .unwrap();
        }

        let tools = discover_tools(dir.path()).await.unwrap();
        assert_eq!(tools.len(), super::MAX_TOOLS_PER_DIRECTORY);
    }

    #[test]
    fn test_load_error_display() {
        let err = WasmLoadError::InvalidName("bad/name".to_string());
        assert!(err.to_string().contains("bad/name"));

        let err = WasmLoadError::WasmNotFound(std::path::PathBuf::from("/foo/bar.wasm"));
        assert!(err.to_string().contains("/foo/bar.wasm"));
    }

    #[test]
    fn test_tools_src_dir_default() {
        let dir = super::tools_src_dir();
        assert!(dir.ends_with("tools-src"));
    }

    #[tokio::test]
    async fn test_discover_dev_tools_finds_build_artifacts() {
        // This test relies on the actual tools-src/ directory in the repo.
        // If build artifacts exist, they should be discovered.
        let tools = super::discover_dev_tools().await.unwrap();

        // If any tools have been built, they should appear with "-tool" suffix
        for (name, discovered) in &tools {
            assert!(
                name.ends_with("-tool"),
                "Dev tool name should end with -tool: {}",
                name
            );
            assert!(
                discovered.wasm_path.exists(),
                "WASM should exist: {:?}",
                discovered.wasm_path
            );
        }
    }

    #[test]
    fn test_resolve_oauth_refresh_config_with_oauth() {
        use crate::wasm::capabilities_schema::{
            AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema,
        };

        let caps = CapabilitiesFile {
            auth: Some(AuthCapabilitySchema {
                secret_name: "google_oauth_token".to_string(),
                provider: Some("google".to_string()),
                oauth: Some(OAuthConfigSchema {
                    authorization_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
                    token_url: "https://oauth2.googleapis.com/token".to_string(),
                    client_id: Some("test-client-id".to_string()),
                    client_secret: Some("test-client-secret".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let config = crate::wasm::resolve_oauth_refresh_config(&caps);
        assert!(config.is_some());

        let config = config.unwrap();
        assert_eq!(config.token_url, "https://oauth2.googleapis.com/token");
        assert_eq!(config.client_id, "test-client-id");
        assert_eq!(config.client_secret, Some("test-client-secret".to_string()));
        assert_eq!(config.secret_name, "google_oauth_token");
        assert_eq!(config.provider, Some("google".to_string()));
    }

    #[test]
    fn test_resolve_oauth_refresh_config_no_auth() {
        use crate::wasm::capabilities_schema::CapabilitiesFile;

        let caps = CapabilitiesFile::default();
        let config = crate::wasm::resolve_oauth_refresh_config(&caps);
        assert!(config.is_none());
    }

    #[test]
    fn test_resolve_oauth_refresh_config_no_oauth() {
        use crate::wasm::capabilities_schema::{AuthCapabilitySchema, CapabilitiesFile};

        let caps = CapabilitiesFile {
            auth: Some(AuthCapabilitySchema {
                secret_name: "manual_token".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };

        let config = crate::wasm::resolve_oauth_refresh_config(&caps);
        assert!(config.is_none());
    }

    #[test]
    fn test_resolve_oauth_refresh_config_no_client_id() {
        use crate::wasm::capabilities_schema::{
            AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema,
        };

        // A non-Google provider with no client_id anywhere should return None
        let caps = CapabilitiesFile {
            auth: Some(AuthCapabilitySchema {
                secret_name: "unknown_provider_token".to_string(),
                oauth: Some(OAuthConfigSchema {
                    authorization_url: "https://example.com/auth".to_string(),
                    token_url: "https://example.com/token".to_string(),
                    // No client_id, no client_id_env, no builtin
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let config = crate::wasm::resolve_oauth_refresh_config(&caps);
        assert!(config.is_none());
    }

    #[test]
    fn test_resolve_oauth_refresh_config_builtin_google() {
        use crate::wasm::capabilities_schema::{
            AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema,
        };

        // google_oauth_token should fall back to built-in credentials
        let caps = CapabilitiesFile {
            auth: Some(AuthCapabilitySchema {
                secret_name: "google_oauth_token".to_string(),
                provider: Some("google".to_string()),
                oauth: Some(OAuthConfigSchema {
                    authorization_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
                    token_url: "https://oauth2.googleapis.com/token".to_string(),
                    // No inline client_id, should fall back to builtin
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let config = crate::wasm::resolve_oauth_refresh_config(&caps);
        assert!(config.is_some());
        let config = config.unwrap();
        assert!(!config.client_id.is_empty());
        assert!(config.client_secret.is_some());
    }
}
