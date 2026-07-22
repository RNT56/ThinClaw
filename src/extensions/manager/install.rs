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

async fn download_extension_bytes(url: &str, max_bytes: usize) -> Result<Vec<u8>, ExtensionError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(120),
        download_extension_bytes_inner(url, max_bytes),
    )
    .await
    .map_err(|_| ExtensionError::DownloadFailed("extension download timed out".to_string()))?
}

async fn download_extension_bytes_inner(
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, ExtensionError> {
    let mut current = url.to_string();
    for redirect_count in 0..=5 {
        let guarded = thinclaw_tools_core::validate_outbound_url_pinned_async(
            &current,
            &thinclaw_tools_core::OutboundUrlGuardOptions {
                require_https: true,
                upgrade_http_to_https: false,
                allowlist: Vec::new(),
            },
        )
        .await
        .map_err(|error| ExtensionError::InvalidUrl(error.to_string()))?;
        if guarded.url.fragment().is_some() {
            return Err(ExtensionError::InvalidUrl(
                "extension download URLs cannot contain fragments".to_string(),
            ));
        }

        let host = guarded.url.host_str().ok_or_else(|| {
            ExtensionError::InvalidUrl("extension download URL has no host".to_string())
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
            .map_err(|error| ExtensionError::DownloadFailed(error.to_string()))?;
        let response = client
            .get(guarded.url.clone())
            .send()
            .await
            .map_err(|error| ExtensionError::DownloadFailed(error.without_url().to_string()))?;

        if response.status().is_redirection() {
            if redirect_count == 5 {
                return Err(ExtensionError::DownloadFailed(
                    "extension download exceeded 5 redirects".to_string(),
                ));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| {
                    ExtensionError::DownloadFailed(
                        "extension redirect omitted its Location header".to_string(),
                    )
                })?
                .to_str()
                .map_err(|_| {
                    ExtensionError::DownloadFailed(
                        "extension redirect Location is not valid text".to_string(),
                    )
                })?;
            current = guarded
                .url
                .join(location)
                .map_err(|error| ExtensionError::InvalidUrl(error.to_string()))?
                .to_string();
            continue;
        }

        if !response.status().is_success() {
            return Err(ExtensionError::DownloadFailed(format!(
                "extension download returned HTTP {}",
                response.status()
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > u64::try_from(max_bytes).unwrap_or(u64::MAX))
        {
            return Err(ExtensionError::InstallFailed(format!(
                "Extension download exceeds the {max_bytes}-byte limit"
            )));
        }
        return crate::http_response::bounded_bytes(response, max_bytes)
            .await
            .map_err(|error| ExtensionError::DownloadFailed(error.to_string()));
    }
    Err(ExtensionError::DownloadFailed(
        "extension redirect processing failed".to_string(),
    ))
}

fn read_extension_archive_entry(
    entry: &mut impl std::io::Read,
    limit: usize,
    filename: &str,
) -> Result<Vec<u8>, ExtensionError> {
    use std::io::Read as _;

    let mut data = Vec::new();
    entry
        .take(limit as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|error| {
            ExtensionError::InstallFailed(format!(
                "Failed to read archive entry '{filename}': {error}"
            ))
        })?;
    if data.len() > limit {
        return Err(ExtensionError::InstallFailed(format!(
            "Archive entry '{filename}' exceeds the {limit}-byte limit"
        )));
    }
    Ok(data)
}

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
                let (Err(primary_err), Some(fallback)) =
                    (primary_result, entry.fallback_source.as_deref())
                else {
                    return Err(ExtensionError::Other(
                        "extension fallback policy returned an inconsistent decision".to_string(),
                    ));
                };
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
        let _operation = self.mcp_operation_lock.lock().await;
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

        tracing::info!(
            "Installed MCP server '{}' at {}",
            name,
            crate::registry::installer::redacted_download_url(url)
        );

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
        if thinclaw_tools::registry::PROTECTED_TOOL_NAMES.contains(&name) {
            return Err(ExtensionError::InstallFailed(format!(
                "WASM tool '{}' conflicts with a protected built-in tool name",
                name
            )));
        }
        self.download_and_install_wasm(
            name,
            url,
            capabilities_url,
            &self.wasm_tools_dir,
            ExtensionKind::WasmTool,
        )
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
        self.download_and_install_wasm(
            name,
            url,
            capabilities_url,
            &self.wasm_channels_dir,
            ExtensionKind::WasmChannel,
        )
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
        kind: ExtensionKind,
    ) -> Result<(), ExtensionError> {
        // 50 MB cap to prevent disk-fill DoS
        const MAX_DOWNLOAD_SIZE: usize = 50 * 1024 * 1024;

        tracing::debug!(
            extension = %name,
            url = %crate::registry::installer::redacted_download_url(url),
            "Downloading WASM extension"
        );
        let bytes = download_extension_bytes(url, MAX_DOWNLOAD_SIZE).await?;

        let wasm_path = target_dir.join(format!("{}.wasm", name));
        let caps_path = target_dir.join(format!("{}.capabilities.json", name));

        // Decode and validate all bytes before publishing either file.
        let (wasm_bytes, capabilities_bytes) = if bytes.starts_with(&[0x1f, 0x8b]) {
            self.extract_wasm_tar_gz(name, &bytes)?
        } else {
            let capabilities = match capabilities_url {
                Some(caps_url) => {
                    const MAX_CAPS_SIZE: usize = 1024 * 1024;
                    Some(download_extension_bytes(caps_url, MAX_CAPS_SIZE).await?)
                }
                None => None,
            };
            (bytes, capabilities)
        };

        let manifest_kind = match kind {
            ExtensionKind::WasmTool => crate::registry::manifest::ManifestKind::Tool,
            ExtensionKind::WasmChannel => crate::registry::manifest::ManifestKind::Channel,
            _ => {
                return Err(ExtensionError::InstallFailed(
                    "WASM installer received a non-WASM extension kind".to_string(),
                ));
            }
        };
        crate::registry::installer::validate_wasm_payload(&wasm_bytes, url)
            .map_err(|error| ExtensionError::InstallFailed(error.to_string()))?;
        if let Some(capabilities) = capabilities_bytes.as_deref() {
            crate::registry::installer::validate_capabilities_payload(
                manifest_kind,
                name,
                capabilities,
                capabilities_url.unwrap_or(url),
            )
            .map_err(|error| ExtensionError::InstallFailed(error.to_string()))?;
        }
        crate::registry::installer::publish_extension_files(
            wasm_path.clone(),
            caps_path,
            wasm_bytes,
            capabilities_bytes,
            true,
        )
        .await
        .map_err(|error| ExtensionError::InstallFailed(error.to_string()))?;

        tracing::info!(
            "Installed WASM extension '{}' from {} to {}",
            name,
            crate::registry::installer::redacted_download_url(url),
            wasm_path.display()
        );

        Ok(())
    }

    /// Extract a tar.gz bundle into the WASM tools directory.
    fn extract_wasm_tar_gz(
        &self,
        name: &str,
        bytes: &[u8],
    ) -> Result<(Vec<u8>, Option<Vec<u8>>), ExtensionError> {
        use flate2::read::GzDecoder;
        use tar::Archive;

        let decoder = GzDecoder::new(bytes);
        let mut archive = Archive::new(decoder);
        // Defense-in-depth: do not preserve permissions or extended attributes
        archive.set_preserve_permissions(false);
        #[cfg(any(unix, target_os = "redox"))]
        archive.set_unpack_xattrs(false);

        const MAX_ARCHIVE_ENTRIES: usize = 1024;
        const MAX_WASM_SIZE: usize = 50 * 1024 * 1024;
        const MAX_CAPS_SIZE: usize = 1024 * 1024;

        let wasm_filename = format!("{}.wasm", name);
        let caps_filename = format!("{}.capabilities.json", name);
        let mut wasm = None;
        let mut capabilities = None;

        let entries = archive
            .entries()
            .map_err(|e| ExtensionError::InstallFailed(format!("Bad tar.gz archive: {}", e)))?;

        for (entry_index, entry) in entries.enumerate() {
            if entry_index >= MAX_ARCHIVE_ENTRIES {
                return Err(ExtensionError::InstallFailed(format!(
                    "Archive exceeds the {MAX_ARCHIVE_ENTRIES}-entry limit"
                )));
            }
            let mut entry = entry
                .map_err(|e| ExtensionError::InstallFailed(format!("Bad tar.gz entry: {}", e)))?;

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
                if wasm.is_some()
                    || !entry.header().entry_type().is_file()
                    || entry.size() > MAX_WASM_SIZE as u64
                {
                    return Err(ExtensionError::InstallFailed(format!(
                        "Archive entry '{wasm_filename}' is duplicate, non-regular, or oversized"
                    )));
                }
                wasm = Some(read_extension_archive_entry(
                    &mut entry,
                    MAX_WASM_SIZE,
                    &wasm_filename,
                )?);
            } else if filename == caps_filename {
                if capabilities.is_some()
                    || !entry.header().entry_type().is_file()
                    || entry.size() > MAX_CAPS_SIZE as u64
                {
                    return Err(ExtensionError::InstallFailed(format!(
                        "Archive entry '{caps_filename}' is duplicate, non-regular, or oversized"
                    )));
                }
                capabilities = Some(read_extension_archive_entry(
                    &mut entry,
                    MAX_CAPS_SIZE,
                    &caps_filename,
                )?);
            }
        }

        let wasm = wasm.ok_or_else(|| {
            ExtensionError::InstallFailed(format!(
                "tar.gz archive does not contain '{}'",
                wasm_filename
            ))
        })?;

        Ok((wasm, capabilities))
    }

    /// Install a WASM extension from local build artifacts (WasmBuildable source).
    ///
    /// Resolves the build directory relative to `CARGO_MANIFEST_DIR`,
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

        let requested_dir = std::path::Path::new(build_dir.unwrap_or("."));
        if requested_dir.is_absolute()
            || requested_dir.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(ExtensionError::InstallFailed(
                "WASM build directory must remain inside the ThinClaw source checkout".to_string(),
            ));
        }
        let manifest_root = std::fs::canonicalize(manifest_dir).map_err(|error| {
            ExtensionError::InstallFailed(format!(
                "Failed to resolve ThinClaw source checkout: {error}"
            ))
        })?;
        let resolved_dir =
            std::fs::canonicalize(manifest_dir.join(requested_dir)).map_err(|error| {
                ExtensionError::InstallFailed(format!(
                    "Failed to resolve WASM build directory '{}': {error}",
                    requested_dir.display()
                ))
            })?;
        if !resolved_dir.starts_with(&manifest_root) || !resolved_dir.is_dir() {
            return Err(ExtensionError::InstallFailed(
                "WASM build directory must be a real directory inside the ThinClaw source checkout"
                    .to_string(),
            ));
        }

        // Determine the binary name to look for
        let binary_name = crate_name.unwrap_or(name);
        if binary_name.is_empty()
            || binary_name.len() > 128
            || !binary_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(ExtensionError::InstallFailed(
                "WASM crate name must contain only ASCII letters, digits, hyphens, or underscores"
                    .to_string(),
            ));
        }

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
            match kind {
                ExtensionKind::WasmTool => crate::registry::manifest::ManifestKind::Tool,
                ExtensionKind::WasmChannel => crate::registry::manifest::ManifestKind::Channel,
                _ => {
                    return Err(ExtensionError::InstallFailed(
                        "WASM build installer received a non-WASM extension kind".to_string(),
                    ));
                }
            },
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
