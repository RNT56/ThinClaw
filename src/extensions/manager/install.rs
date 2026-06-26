//! Install internals: the bundled/standard/fallback install chain, per-source
//! dispatch, WASM download + tar.gz extraction, and local build-artifact
//! installation.

use crate::extensions::{
    ExtensionError, ExtensionKind, ExtensionSource, InstallResult, RegistryEntry,
};
use crate::tools::mcp::config::McpServerConfig;
use thinclaw_tools::builtin::extension_tools as extension_tool_policy;
use thinclaw_tools::builtin::extension_tools::{CombinedInstallError, FallbackDecision};

use super::ExtensionManager;
use super::core::{install_error_kind, install_outcome};

impl ExtensionManager {
    pub(super) async fn install_from_entry(
        &self,
        entry: &RegistryEntry,
    ) -> Result<InstallResult, ExtensionError> {
        // Priority 1: Try bundled WASM (compiled into binary via --features bundled-wasm).
        // This is the fastest path and requires zero network access.
        if crate::registry::bundled_wasm::is_bundled(&entry.name) {
            let target_dir = match entry.kind {
                ExtensionKind::WasmTool => &self.wasm_tools_dir,
                ExtensionKind::WasmChannel => &self.wasm_channels_dir,
                ExtensionKind::McpServer => {
                    // MCP servers can't be bundled as WASM; fall through
                    return self.try_standard_install(entry).await;
                }
                ExtensionKind::NativePlugin => {
                    // Native plugins are never bundled as WASM and are not
                    // installed via this path; fall through to the standard
                    // install chain, which returns the operator-side guidance.
                    return self.try_standard_install(entry).await;
                }
            };

            tracing::info!(
                extension = %entry.name,
                "Installing from bundled WASM (embedded in binary)"
            );

            match crate::registry::bundled_wasm::extract_bundled(&entry.name, target_dir).await {
                Ok(()) => {
                    let kind_label = match entry.kind {
                        ExtensionKind::WasmTool => "WASM tool",
                        ExtensionKind::WasmChannel => "WASM channel",
                        ExtensionKind::McpServer => "MCP server",
                        ExtensionKind::NativePlugin => "native plugin",
                    };
                    return Ok(InstallResult {
                        name: entry.name.clone(),
                        kind: entry.kind,
                        message: format!(
                            "{} '{}' installed from bundled binary. Run activate to load it.",
                            kind_label, entry.name,
                        ),
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        extension = %entry.name,
                        error = %e,
                        "Bundled WASM extraction failed, falling back to standard install"
                    );
                    // Fall through to standard install chain
                }
            }
        }

        // Priority 2 & 3: WasmDownload → WasmBuildable (with fallback chain)
        self.try_standard_install(entry).await
    }

    /// Standard install chain: primary source → fallback source.
    async fn try_standard_install(
        &self,
        entry: &RegistryEntry,
    ) -> Result<InstallResult, ExtensionError> {
        let primary_result = self.try_install_from_source(entry, &entry.source).await;
        match extension_tool_policy::fallback_decision(
            install_outcome(&primary_result),
            entry.fallback_source.is_some(),
        ) {
            FallbackDecision::Return => primary_result,
            FallbackDecision::TryFallback => {
                let primary_err =
                    primary_result.expect_err("TryFallback requires primary install to fail");
                let fallback = entry
                    .fallback_source
                    .as_ref()
                    .expect("TryFallback requires fallback_source");
                tracing::info!(
                    extension = %entry.name,
                    primary_error = %primary_err,
                    "Primary install failed, trying fallback source"
                );
                self.try_install_from_source(entry, fallback)
                    .await
                    .map_err(|fallback_err| {
                        tracing::error!(
                            extension = %entry.name,
                            fallback_error = %fallback_err,
                            "Fallback install also failed"
                        );
                        match extension_tool_policy::combine_install_errors(
                            &primary_err.to_string(),
                            &fallback_err.to_string(),
                            install_error_kind(&fallback_err),
                        ) {
                            CombinedInstallError::PreserveFallback => fallback_err,
                            CombinedInstallError::CombinedMessage(message) => {
                                ExtensionError::Other(message)
                            }
                        }
                    })
            }
        }
    }

    /// Attempt to install an extension using a specific source.
    async fn try_install_from_source(
        &self,
        entry: &RegistryEntry,
        source: &ExtensionSource,
    ) -> Result<InstallResult, ExtensionError> {
        match entry.kind {
            ExtensionKind::McpServer => {
                let url = match source {
                    ExtensionSource::McpUrl { url } => url.clone(),
                    ExtensionSource::Discovered { url } => url.clone(),
                    _ => {
                        return Err(ExtensionError::InstallFailed(
                            "Registry entry for MCP server has no URL".to_string(),
                        ));
                    }
                };
                self.install_mcp_from_url(&entry.name, &url).await
            }
            ExtensionKind::WasmTool => match source {
                ExtensionSource::WasmDownload {
                    wasm_url,
                    capabilities_url,
                } => {
                    self.install_wasm_tool_from_url_with_caps(
                        &entry.name,
                        wasm_url,
                        capabilities_url.as_deref(),
                    )
                    .await
                }
                ExtensionSource::WasmBuildable {
                    build_dir,
                    crate_name,
                    ..
                } => {
                    self.install_wasm_from_buildable(
                        &entry.name,
                        build_dir.as_deref(),
                        crate_name.as_deref(),
                        &self.wasm_tools_dir,
                        ExtensionKind::WasmTool,
                    )
                    .await
                }
                _ => Err(ExtensionError::InstallFailed(
                    "WASM tool entry has no download URL or build info".to_string(),
                )),
            },
            ExtensionKind::WasmChannel => match source {
                ExtensionSource::WasmDownload {
                    wasm_url,
                    capabilities_url,
                } => {
                    self.install_wasm_channel_from_url(
                        &entry.name,
                        wasm_url,
                        capabilities_url.as_deref(),
                    )
                    .await
                }
                ExtensionSource::WasmBuildable {
                    build_dir,
                    crate_name,
                    ..
                } => {
                    self.install_wasm_from_buildable(
                        &entry.name,
                        build_dir.as_deref(),
                        crate_name.as_deref(),
                        &self.wasm_channels_dir,
                        ExtensionKind::WasmChannel,
                    )
                    .await
                }
                _ => Err(ExtensionError::InstallFailed(
                    "WASM channel entry has no download URL or build info".to_string(),
                )),
            },
            // Native plugins are not installed from a registry source. They are
            // operator-placed signed manifests discovered via a manifest scan.
            ExtensionKind::NativePlugin => Err(ExtensionError::InstallFailed(
                "native plugins are installed by placing a signed manifest in an allowlisted \
                 directory (extensions.native_plugin_allowlist_dirs), not from a registry source"
                    .to_string(),
            )),
        }
    }

    pub(super) async fn install_mcp_from_url(
        &self,
        name: &str,
        url: &str,
    ) -> Result<InstallResult, ExtensionError> {
        // Check if already installed
        if self.get_mcp_server(name).await.is_ok() {
            return Err(ExtensionError::AlreadyInstalled(name.to_string()));
        }

        let config = McpServerConfig::new(name, url);
        config
            .validate()
            .map_err(|e| ExtensionError::InvalidUrl(e.to_string()))?;

        self.add_mcp_server(config)
            .await
            .map_err(|e| ExtensionError::Config(e.to_string()))?;

        tracing::info!("Installed MCP server '{}' at {}", name, url);

        Ok(InstallResult {
            name: name.to_string(),
            kind: ExtensionKind::McpServer,
            message: format!(
                "MCP server '{}' installed. Run auth next to authenticate.",
                name
            ),
        })
    }

    pub(super) async fn install_wasm_tool_from_url(
        &self,
        name: &str,
        url: &str,
    ) -> Result<InstallResult, ExtensionError> {
        self.install_wasm_tool_from_url_with_caps(name, url, None)
            .await
    }

    async fn install_wasm_tool_from_url_with_caps(
        &self,
        name: &str,
        url: &str,
        capabilities_url: Option<&str>,
    ) -> Result<InstallResult, ExtensionError> {
        self.download_and_install_wasm(name, url, capabilities_url, &self.wasm_tools_dir)
            .await?;

        Ok(InstallResult {
            name: name.to_string(),
            kind: ExtensionKind::WasmTool,
            message: format!("WASM tool '{}' installed. Run activate to load it.", name),
        })
    }

    pub(super) async fn install_wasm_channel_from_url(
        &self,
        name: &str,
        url: &str,
        capabilities_url: Option<&str>,
    ) -> Result<InstallResult, ExtensionError> {
        self.download_and_install_wasm(name, url, capabilities_url, &self.wasm_channels_dir)
            .await?;

        Ok(InstallResult {
            name: name.to_string(),
            kind: ExtensionKind::WasmChannel,
            message: format!(
                "WASM channel '{}' installed. Run activate to start it.",
                name,
            ),
        })
    }

    /// Download a WASM extension (tool or channel) from URL and install to target directory.
    ///
    /// Handles both tar.gz bundles (containing `.wasm` + `.capabilities.json`) and bare
    /// `.wasm` files. Validates HTTPS, size limits, and file format.
    async fn download_and_install_wasm(
        &self,
        name: &str,
        url: &str,
        capabilities_url: Option<&str>,
        target_dir: &std::path::Path,
    ) -> Result<(), ExtensionError> {
        // Require HTTPS to prevent downgrade attacks
        if !url.starts_with("https://") {
            return Err(ExtensionError::InstallFailed(
                "Only HTTPS URLs are allowed for extension downloads".to_string(),
            ));
        }

        // 50 MB cap to prevent disk-fill DoS
        const MAX_DOWNLOAD_SIZE: usize = 50 * 1024 * 1024;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| ExtensionError::DownloadFailed(e.to_string()))?;

        tracing::debug!(extension = %name, url = %url, "Downloading WASM extension");

        let response = client.get(url).send().await.map_err(|e| {
            tracing::error!(extension = %name, url = %url, error = %e, "Download request failed");
            ExtensionError::DownloadFailed(e.to_string())
        })?;

        if !response.status().is_success() {
            let status = response.status();
            tracing::error!(
                extension = %name,
                url = %url,
                status = %status,
                "Download returned non-success HTTP status"
            );
            return Err(ExtensionError::DownloadFailed(format!(
                "HTTP {} from {}",
                status, url
            )));
        }

        // Check Content-Length header before downloading the full body
        if let Some(len) = response.content_length()
            && len as usize > MAX_DOWNLOAD_SIZE
        {
            return Err(ExtensionError::InstallFailed(format!(
                "Download too large ({} bytes, max {} bytes)",
                len, MAX_DOWNLOAD_SIZE
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| ExtensionError::DownloadFailed(e.to_string()))?;

        if bytes.len() > MAX_DOWNLOAD_SIZE {
            return Err(ExtensionError::InstallFailed(format!(
                "Download too large ({} bytes, max {} bytes)",
                bytes.len(),
                MAX_DOWNLOAD_SIZE
            )));
        }

        // Ensure target directory exists
        tokio::fs::create_dir_all(target_dir)
            .await
            .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;

        let wasm_path = target_dir.join(format!("{}.wasm", name));
        let caps_path = target_dir.join(format!("{}.capabilities.json", name));

        // Detect format: gzip (tar.gz bundle) or bare WASM
        if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
            // tar.gz bundle: extract {name}.wasm and {name}.capabilities.json
            self.extract_wasm_tar_gz(name, &bytes, &wasm_path, &caps_path)?;
        } else {
            // Bare WASM file: validate magic number
            if bytes.len() < 4 || &bytes[..4] != b"\0asm" {
                return Err(ExtensionError::InstallFailed(
                    "Downloaded file is not a valid WASM binary (bad magic number)".to_string(),
                ));
            }

            tokio::fs::write(&wasm_path, &bytes)
                .await
                .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;

            // Download capabilities separately if URL provided
            if let Some(caps_url) = capabilities_url {
                const MAX_CAPS_SIZE: usize = 1024 * 1024; // 1 MB
                match client.get(caps_url).send().await {
                    Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                        Ok(caps_bytes) if caps_bytes.len() <= MAX_CAPS_SIZE => {
                            if let Err(e) = tokio::fs::write(&caps_path, &caps_bytes).await {
                                tracing::warn!(
                                    "Failed to write capabilities for '{}': {}",
                                    name,
                                    e
                                );
                            }
                        }
                        Ok(caps_bytes) => {
                            tracing::warn!(
                                "Capabilities file for '{}' too large ({} bytes, max {})",
                                name,
                                caps_bytes.len(),
                                MAX_CAPS_SIZE
                            );
                        }
                        Err(e) => {
                            tracing::warn!("Failed to download capabilities for '{}': {}", name, e);
                        }
                    },
                    _ => {
                        tracing::warn!(
                            "Failed to download capabilities for '{}' from {}",
                            name,
                            caps_url
                        );
                    }
                }
            }
        }

        tracing::info!(
            "Installed WASM extension '{}' from {} to {}",
            name,
            url,
            wasm_path.display()
        );

        Ok(())
    }

    /// Extract a tar.gz bundle into the WASM tools directory.
    fn extract_wasm_tar_gz(
        &self,
        name: &str,
        bytes: &[u8],
        target_wasm: &std::path::Path,
        target_caps: &std::path::Path,
    ) -> Result<(), ExtensionError> {
        use flate2::read::GzDecoder;
        use tar::Archive;

        use std::io::Read as _;

        let decoder = GzDecoder::new(bytes);
        let mut archive = Archive::new(decoder);
        // Defense-in-depth: do not preserve permissions or extended attributes
        archive.set_preserve_permissions(false);
        #[cfg(any(unix, target_os = "redox"))]
        archive.set_unpack_xattrs(false);

        // 100 MB cap on decompressed entry size to prevent decompression bombs
        const MAX_ENTRY_SIZE: u64 = 100 * 1024 * 1024;

        let wasm_filename = format!("{}.wasm", name);
        let caps_filename = format!("{}.capabilities.json", name);
        let mut found_wasm = false;

        let entries = archive
            .entries()
            .map_err(|e| ExtensionError::InstallFailed(format!("Bad tar.gz archive: {}", e)))?;

        for entry in entries {
            let mut entry = entry
                .map_err(|e| ExtensionError::InstallFailed(format!("Bad tar.gz entry: {}", e)))?;

            if entry.size() > MAX_ENTRY_SIZE {
                return Err(ExtensionError::InstallFailed(format!(
                    "Archive entry too large ({} bytes, max {} bytes)",
                    entry.size(),
                    MAX_ENTRY_SIZE
                )));
            }

            let entry_path = entry
                .path()
                .map_err(|e| {
                    ExtensionError::InstallFailed(format!("Invalid path in tar.gz: {}", e))
                })?
                .to_path_buf();

            let filename = entry_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            if filename == wasm_filename {
                let mut data = Vec::with_capacity(entry.size() as usize);
                std::io::Read::read_to_end(&mut entry.by_ref().take(MAX_ENTRY_SIZE), &mut data)
                    .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;
                std::fs::write(target_wasm, &data)
                    .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;
                found_wasm = true;
            } else if filename == caps_filename {
                let mut data = Vec::with_capacity(entry.size() as usize);
                std::io::Read::read_to_end(&mut entry.by_ref().take(MAX_ENTRY_SIZE), &mut data)
                    .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;
                std::fs::write(target_caps, &data)
                    .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;
            }
        }

        if !found_wasm {
            return Err(ExtensionError::InstallFailed(format!(
                "tar.gz archive does not contain '{}'",
                wasm_filename
            )));
        }

        Ok(())
    }

    /// Install a WASM extension from local build artifacts (WasmBuildable source).
    ///
    /// Resolves the build directory (relative to `CARGO_MANIFEST_DIR` or absolute),
    /// looks for the compiled WASM artifact, and copies it (plus capabilities.json)
    /// to the install directory. Falls back to an error if artifacts don't exist.
    async fn install_wasm_from_buildable(
        &self,
        name: &str,
        build_dir: Option<&str>,
        crate_name: Option<&str>,
        target_dir: &std::path::Path,
        kind: ExtensionKind,
    ) -> Result<InstallResult, ExtensionError> {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

        // Resolve build directory
        let resolved_dir = match build_dir {
            Some(dir) => {
                let p = std::path::Path::new(dir);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    manifest_dir.join(dir)
                }
            }
            None => manifest_dir.to_path_buf(),
        };

        // Determine the binary name to look for
        let binary_name = crate_name.unwrap_or(name);

        let wasm_src =
            crate::registry::artifacts::find_wasm_artifact(&resolved_dir, binary_name, "release")
                .ok_or_else(|| {
                ExtensionError::InstallFailed(format!(
                    "'{}' requires building from source. Build artifact not found. \
                         Run `cargo component build --release` in {} first, \
                         or use `thinclaw registry install {}`.",
                    name,
                    resolved_dir.display(),
                    name,
                ))
            })?;

        let wasm_dst = crate::registry::artifacts::install_wasm_files(
            &wasm_src,
            &resolved_dir,
            name,
            target_dir,
            true,
        )
        .await
        .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;

        let kind_label = match kind {
            ExtensionKind::WasmTool => "WASM tool",
            ExtensionKind::WasmChannel => "WASM channel",
            ExtensionKind::McpServer => "MCP server",
            // Native plugins never reach the WASM buildable-install path.
            ExtensionKind::NativePlugin => "native plugin",
        };

        tracing::info!(
            "Installed {} '{}' from build artifacts at {}",
            kind_label,
            name,
            wasm_dst.display(),
        );

        Ok(InstallResult {
            name: name.to_string(),
            kind,
            message: format!(
                "{} '{}' installed from local build artifacts. Run activate to load it.",
                kind_label, name,
            ),
        })
    }
}
