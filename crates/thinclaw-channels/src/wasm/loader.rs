//! WASM channel loader for loading channels from files or directories.
//!
//! Loads WASM channel modules from the filesystem (default: ~/.thinclaw/channels/).
//! Each channel consists of:
//! - `<name>.wasm` - The compiled WASM component
//! - `<name>.capabilities.json` - Channel capabilities and configuration

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt as _;
use tokio::fs;

use crate::pairing::PairingStore;
use crate::wasm::capabilities::{ChannelCapabilities, is_valid_channel_name};
use crate::wasm::error::WasmChannelError;
use crate::wasm::runtime::WasmChannelRuntime;
use crate::wasm::schema::{ChannelCapabilitiesFile, WebhookSecretValidation};
use crate::wasm::wrapper::WasmChannel;

const MAX_WASM_MODULE_BYTES: usize = 64 * 1024 * 1024;
const MAX_CAPABILITIES_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_CHANNELS_PER_DIRECTORY: usize = 64;
const MAX_PARALLEL_COMPILATIONS: usize = 4;

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
                "channel package path is not a regular file",
            ));
        }
        if metadata.len() > max_bytes as u64 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "channel package file exceeds the size limit",
            ));
        }

        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&path)?;
        let opened = file.metadata()?;
        if !opened.is_file() || opened.len() > max_bytes as u64 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "opened channel package file is invalid or oversized",
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
                "channel package file exceeds the size limit",
            ));
        }
        Ok(bytes)
    })
    .await
    .map_err(|error| std::io::Error::other(format!("channel file reader panicked: {error}")))?
}

fn validate_loaded_capabilities(
    capabilities: &ChannelCapabilities,
) -> Result<(), WasmChannelError> {
    capabilities
        .validate_workspace_path("validation-probe")
        .map_err(WasmChannelError::InvalidCapabilities)?;
    if capabilities.max_message_size == 0 || capabilities.max_message_size > 64 * 1024 {
        return Err(WasmChannelError::InvalidCapabilities(
            "max_message_size must be between 1 and 65536 bytes".to_string(),
        ));
    }
    if capabilities.callback_timeout.is_zero()
        || capabilities.callback_timeout > std::time::Duration::from_secs(120)
    {
        return Err(WasmChannelError::InvalidCapabilities(
            "callback timeout must be between 1ms and 120 seconds".to_string(),
        ));
    }
    if capabilities.allowed_paths.len() > 32
        || capabilities.allowed_paths.iter().any(|path| {
            path.len() > 256
                || !path.starts_with("/webhook/")
                || path.contains(['?', '#', '\\'])
                || path.chars().any(char::is_control)
                || path.split('/').any(|part| part == ".." || part == ".")
        })
    {
        return Err(WasmChannelError::InvalidCapabilities(
            "webhook path capability is malformed or exceeds its limit".to_string(),
        ));
    }
    if let Some(workspace) = &capabilities.tool_capabilities.workspace_read
        && (workspace.allowed_prefixes.len() > 64
            || workspace.allowed_prefixes.iter().any(|prefix| {
                prefix.is_empty()
                    || prefix.len() > 1024
                    || prefix.starts_with('/')
                    || prefix.contains('\\')
                    || prefix.chars().any(char::is_control)
                    || prefix
                        .trim_end_matches('/')
                        .split('/')
                        .any(|part| part.is_empty() || matches!(part, "." | ".."))
            }))
    {
        return Err(WasmChannelError::InvalidCapabilities(
            "workspace read prefix capability is malformed or exceeds its limit".to_string(),
        ));
    }
    Ok(())
}

/// Loads WASM channels from the filesystem.
pub struct WasmChannelLoader {
    runtime: Arc<WasmChannelRuntime>,
    pairing_store: Arc<PairingStore>,
}

impl WasmChannelLoader {
    /// Create a new loader with the given runtime and pairing store.
    pub fn new(runtime: Arc<WasmChannelRuntime>, pairing_store: Arc<PairingStore>) -> Self {
        Self {
            runtime,
            pairing_store,
        }
    }

    pub async fn invalidate(&self, name: &str) {
        self.runtime.remove(name).await;
    }

    /// Load a single WASM channel from a file pair.
    ///
    /// Expects:
    /// - `wasm_path`: Path to the `.wasm` file
    /// - `capabilities_path`: Path to the `.capabilities.json` file (optional)
    ///
    /// If no capabilities file is provided, the channel gets minimal capabilities.
    pub async fn load_from_files(
        &self,
        name: &str,
        wasm_path: &Path,
        capabilities_path: Option<&Path>,
    ) -> Result<LoadedChannel, WasmChannelError> {
        // Validate name
        if !is_valid_channel_name(name) {
            return Err(WasmChannelError::InvalidName(name.to_string()));
        }
        if let Err(error) = fs::symlink_metadata(wasm_path).await {
            if error.kind() == std::io::ErrorKind::NotFound {
                return Err(WasmChannelError::WasmNotFound(wasm_path.to_path_buf()));
            }
            return Err(error.into());
        }
        let publication_guard =
            thinclaw_platform::acquire_artifact_read_lock(wasm_path.to_path_buf()).await?;
        if publication_in_progress(wasm_path)? {
            return Err(WasmChannelError::Io(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "channel package publication is incomplete",
            )));
        }

        // Read WASM bytes
        let wasm_bytes = read_regular_file_bounded(wasm_path, MAX_WASM_MODULE_BYTES)
            .await
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    WasmChannelError::WasmNotFound(wasm_path.to_path_buf())
                } else {
                    WasmChannelError::Io(error)
                }
            })?;
        if wasm_bytes.len() < 8 || !wasm_bytes.starts_with(b"\0asm") {
            return Err(WasmChannelError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "channel package is not a valid-looking WASM module",
            )));
        }

        // Read capabilities file
        let (capabilities, config_json, description, formatting_hints, cap_file) =
            if let Some(cap_path) = capabilities_path {
                match read_regular_file_bounded(cap_path, MAX_CAPABILITIES_FILE_BYTES).await {
                    Ok(cap_bytes) => {
                        let cap_file = ChannelCapabilitiesFile::from_bytes(&cap_bytes)
                            .map_err(|e| WasmChannelError::InvalidCapabilities(e.to_string()))?;
                        if cap_file.r#type != "channel" || cap_file.name != name {
                            return Err(WasmChannelError::InvalidCapabilities(
                                "capabilities manifest type/name does not match the channel file"
                                    .to_string(),
                            ));
                        }

                        let caps = cap_file.to_capabilities();
                        validate_loaded_capabilities(&caps)?;

                        // Debug: log resulting capabilities
                        tracing::info!(
                            channel = name,
                            http_allowed = caps.tool_capabilities.http.is_some(),
                            http_allowlist_count = caps
                                .tool_capabilities
                                .http
                                .as_ref()
                                .map(|h| h.allowlist.len())
                                .unwrap_or(0),
                            "Channel capabilities loaded"
                        );

                        let config = cap_file.config_json();
                        let desc = cap_file.description.clone();
                        let formatting_hints = cap_file.formatting_hints().map(str::to_owned);

                        (caps, config, desc, formatting_hints, Some(cap_file))
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        tracing::warn!(
                            path = %cap_path.display(),
                            "Capabilities file not found, using defaults"
                        );
                        (
                            ChannelCapabilities::for_channel(name),
                            "{}".to_string(),
                            None,
                            None,
                            None,
                        )
                    }
                    Err(error) => return Err(WasmChannelError::Io(error)),
                }
            } else {
                (
                    ChannelCapabilities::for_channel(name),
                    "{}".to_string(),
                    None,
                    None,
                    None,
                )
            };
        // Prepare the module
        let prepared = self
            .runtime
            .prepare(name, &wasm_bytes, None, description)
            .await?;

        // Create the channel
        let channel = WasmChannel::new(
            self.runtime.clone(),
            prepared,
            capabilities,
            config_json,
            formatting_hints,
            self.pairing_store.clone(),
        );

        tracing::info!(
            name = name,
            wasm_path = %wasm_path.display(),
            "Loaded WASM channel from file"
        );

        Ok(LoadedChannel {
            channel,
            capabilities_file: cap_file,
            _publication_guard: publication_guard,
        })
    }

    /// Load all WASM channels from a directory.
    ///
    /// Scans the directory for `*.wasm` files and loads each one, looking for
    /// a matching `*.capabilities.json` sidecar file.
    ///
    /// # Directory Layout
    ///
    /// ```text
    /// channels/
    /// ├── slack.wasm                  <- Channel WASM component
    /// ├── slack.capabilities.json     <- Capabilities (optional)
    /// ├── telegram.wasm
    /// └── telegram.capabilities.json
    /// ```
    pub async fn load_from_dir(&self, dir: &Path) -> Result<LoadResults, WasmChannelError> {
        let directory_metadata = fs::symlink_metadata(dir).await?;
        if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
            return Err(WasmChannelError::Io(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                format!("{} is not a real directory", dir.display()),
            )));
        }

        let mut results = LoadResults::default();

        // Collect all .wasm entries first, then load in parallel
        let mut channel_entries = Vec::new();
        let mut entries = fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                continue;
            }

            let file_type = entry.file_type().await?;
            if !file_type.is_file() || file_type.is_symlink() {
                results.errors.push((
                    path.clone(),
                    WasmChannelError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "WASM channel path is not a regular file",
                    )),
                ));
                continue;
            }

            let name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => {
                    results.errors.push((
                        path.clone(),
                        WasmChannelError::InvalidName("invalid filename".to_string()),
                    ));
                    continue;
                }
            };
            if !is_valid_channel_name(&name) {
                results
                    .errors
                    .push((path.clone(), WasmChannelError::InvalidName(name)));
                continue;
            }
            if publication_in_progress(&path)? {
                results.errors.push((
                    path.clone(),
                    WasmChannelError::Io(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "channel package publication is incomplete",
                    )),
                ));
                continue;
            }

            if channel_entries.len() >= MAX_CHANNELS_PER_DIRECTORY {
                results.errors.push((
                    path.clone(),
                    WasmChannelError::InvalidCapabilities(format!(
                        "channel directory exceeds the {MAX_CHANNELS_PER_DIRECTORY}-channel limit"
                    )),
                ));
                continue;
            }

            let cap_path = path.with_extension("capabilities.json");
            channel_entries.push((name, path, Some(cap_path)));
        }

        // Keep compilation parallelism bounded: each Wasmtime compile is CPU-
        // and memory-intensive, and channel directories are operator-controlled.
        let mut load_results = futures::stream::iter(channel_entries.iter().enumerate().map(
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

        for ((name, path, _), (_, result)) in channel_entries.into_iter().zip(load_results) {
            match result {
                Ok(loaded) => {
                    results.loaded.push(loaded);
                }
                Err(e) => {
                    tracing::error!(
                        name = name,
                        path = %path.display(),
                        error = %e,
                        "Failed to load WASM channel"
                    );
                    results.errors.push((path, e));
                }
            }
        }

        if !results.loaded.is_empty() {
            tracing::info!(
                count = results.loaded.len(),
                channels = ?results.loaded.iter().map(|c| c.name()).collect::<Vec<_>>(),
                "Loaded WASM channels from directory"
            );
        }

        Ok(results)
    }
}

/// A loaded WASM channel with its capabilities file.
pub struct LoadedChannel {
    /// The loaded channel.
    pub channel: WasmChannel,

    /// The parsed capabilities file (if present).
    pub capabilities_file: Option<ChannelCapabilitiesFile>,

    // Keep the package generation stable until the caller finishes activating
    // the compiled channel. This closes the load-then-uninstall resurrection
    // race in hot-reload and extension-manager paths.
    _publication_guard: thinclaw_platform::ArtifactReadGuard,
}

impl LoadedChannel {
    /// Get the channel name.
    pub fn name(&self) -> &str {
        self.channel.channel_name()
    }

    /// Return the exact manifest snapshot read with the WASM generation.
    pub fn capabilities_file(&self) -> Option<&ChannelCapabilitiesFile> {
        self.capabilities_file.as_ref()
    }

    /// Get the webhook secret header name from capabilities.
    pub fn webhook_secret_header(&self) -> Option<&str> {
        self.capabilities_file
            .as_ref()
            .and_then(|f| f.webhook_secret_header())
    }

    /// Get the webhook secret name from capabilities.
    pub fn webhook_secret_name(&self) -> String {
        self.capabilities_file
            .as_ref()
            .map(|f| f.webhook_secret_name())
            .unwrap_or_else(|| format!("{}_webhook_secret", self.channel.channel_name()))
    }

    /// Get the webhook secret validation mode from capabilities.
    pub fn webhook_secret_validation(&self) -> WebhookSecretValidation {
        self.capabilities_file
            .as_ref()
            .map(|f| f.webhook_secret_validation())
            .unwrap_or_default()
    }

    /// Get the verify-token query parameter name from capabilities.
    pub fn webhook_verify_token_param(&self) -> Option<&str> {
        self.capabilities_file
            .as_ref()
            .and_then(|f| f.webhook_verify_token_param())
    }

    /// Get the verify-token secret name from capabilities.
    pub fn webhook_verify_token_secret_name(&self) -> Option<String> {
        self.capabilities_file
            .as_ref()
            .and_then(|f| f.webhook_verify_token_secret_name())
    }
}

/// Results from loading multiple channels.
#[derive(Default)]
pub struct LoadResults {
    /// Successfully loaded channels with their capabilities.
    pub loaded: Vec<LoadedChannel>,

    /// Errors encountered (path, error).
    pub errors: Vec<(PathBuf, WasmChannelError)>,
}

impl LoadResults {
    /// Check if all channels loaded successfully.
    pub fn all_succeeded(&self) -> bool {
        self.errors.is_empty()
    }

    /// Get the count of successfully loaded channels.
    pub fn success_count(&self) -> usize {
        self.loaded.len()
    }

    /// Get the count of failed channels.
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    /// Take ownership of loaded channels (extracts just the WasmChannel).
    pub fn take_channels(self) -> Vec<WasmChannel> {
        self.loaded.into_iter().map(|l| l.channel).collect()
    }
}

/// Discover WASM channel files in a directory without loading them.
///
/// Returns a map of channel name -> (wasm_path, capabilities_path).
#[allow(dead_code)]
pub async fn discover_channels(
    dir: &Path,
) -> Result<HashMap<String, DiscoveredChannel>, std::io::Error> {
    let mut channels = HashMap::new();

    let directory_metadata = match fs::symlink_metadata(dir).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(channels),
        Err(error) => return Err(error),
    };
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "channel package directory is not a real directory",
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
        if !is_valid_channel_name(&name)
            || publication_in_progress(&path)?
            || channels.len() >= MAX_CHANNELS_PER_DIRECTORY
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

        channels.insert(
            name,
            DiscoveredChannel {
                wasm_path: path,
                capabilities_path,
            },
        );
    }

    Ok(channels)
}

/// A discovered WASM channel (not yet loaded).
#[derive(Debug)]
pub struct DiscoveredChannel {
    /// Path to the WASM file.
    pub wasm_path: PathBuf,

    /// Path to the capabilities file (if present).
    pub capabilities_path: Option<PathBuf>,
}

/// Get the default channels directory path.
///
/// Returns ~/.thinclaw/channels/
#[allow(dead_code)]
pub fn default_channels_dir() -> PathBuf {
    thinclaw_platform::state_paths().channels_dir
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use crate::pairing::PairingStore;
    use crate::wasm::loader::{WasmChannelLoader, discover_channels};
    use crate::wasm::runtime::{WasmChannelRuntime, WasmChannelRuntimeConfig};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_discover_channels_empty_dir() {
        let dir = TempDir::new().unwrap();
        let channels = discover_channels(dir.path()).await.unwrap();
        assert!(channels.is_empty());
    }

    #[tokio::test]
    async fn test_discover_channels_with_wasm() {
        let dir = TempDir::new().unwrap();

        // Create a fake .wasm file
        let wasm_path = dir.path().join("slack.wasm");
        std::fs::File::create(&wasm_path).unwrap();

        let channels = discover_channels(dir.path()).await.unwrap();
        assert_eq!(channels.len(), 1);
        assert!(channels.contains_key("slack"));
        assert!(channels["slack"].capabilities_path.is_none());
    }

    #[tokio::test]
    async fn test_discover_channels_with_capabilities() {
        let dir = TempDir::new().unwrap();

        // Create wasm and capabilities files
        std::fs::File::create(dir.path().join("telegram.wasm")).unwrap();
        let mut cap_file =
            std::fs::File::create(dir.path().join("telegram.capabilities.json")).unwrap();
        cap_file.write_all(b"{}").unwrap();

        let channels = discover_channels(dir.path()).await.unwrap();
        assert_eq!(channels.len(), 1);
        assert!(channels["telegram"].capabilities_path.is_some());
    }

    #[tokio::test]
    async fn test_discover_channels_ignores_non_wasm() {
        let dir = TempDir::new().unwrap();

        // Create non-wasm files
        std::fs::File::create(dir.path().join("readme.md")).unwrap();
        std::fs::File::create(dir.path().join("config.json")).unwrap();
        std::fs::File::create(dir.path().join("channel.wasm")).unwrap();

        let channels = discover_channels(dir.path()).await.unwrap();
        assert_eq!(channels.len(), 1);
        assert!(channels.contains_key("channel"));
    }

    #[tokio::test]
    async fn test_discover_channels_hides_incomplete_publication() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("slack.wasm"), b"\0asm\x01\0\0\0").unwrap();
        std::fs::write(
            dir.path().join(".slack.wasm.installing.json"),
            br#"{"version":1}"#,
        )
        .unwrap();

        let channels = discover_channels(dir.path()).await.unwrap();
        assert!(!channels.contains_key("slack"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_discover_channels_rejects_symlink_packages() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let outside_wasm = outside.path().join("outside.wasm");
        std::fs::write(&outside_wasm, b"\0asm\x01\0\0\0").unwrap();
        symlink(&outside_wasm, dir.path().join("slack.wasm")).unwrap();

        let channels = discover_channels(dir.path()).await.unwrap();
        assert!(!channels.contains_key("slack"));
    }

    #[tokio::test]
    async fn test_discover_channels_enforces_directory_limit() {
        let dir = TempDir::new().unwrap();
        for index in 0..(super::MAX_CHANNELS_PER_DIRECTORY + 5) {
            std::fs::write(
                dir.path().join(format!("channel_{index}.wasm")),
                b"\0asm\x01\0\0\0",
            )
            .unwrap();
        }

        let channels = discover_channels(dir.path()).await.unwrap();
        assert_eq!(channels.len(), super::MAX_CHANNELS_PER_DIRECTORY);
    }

    #[tokio::test]
    async fn test_loader_invalid_name() {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());
        let loader = WasmChannelLoader::new(runtime, Arc::new(PairingStore::new()));

        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("test.wasm");

        // Invalid name with path separator
        let result = loader.load_from_files("../escape", &wasm_path, None).await;
        assert!(result.is_err());

        // Empty name
        let result = loader.load_from_files("", &wasm_path, None).await;
        assert!(result.is_err());
    }
}
