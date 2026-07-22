//! Self-update CLI command.
//!
//! Checks for new versions of ThinClaw and optionally installs updates.
//! Supports multiple update channels:
//! - `stable`  — production releases
//! - `beta`    — pre-release builds
//! - `nightly` — latest development builds
//!
//! The update process:
//! 1. Fetch latest version info from the releases API
//! 2. Compare with current version
//! 3. Download the new binary (if desired)
//! 4. Apply the platform-native install flow

use std::path::PathBuf;
use std::time::Duration;

use clap::Subcommand;
use serde::{Deserialize, Serialize};

use crate::terminal_branding::TerminalBranding;

/// Current binary version (from Cargo.toml).
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default release API URL.
const DEFAULT_RELEASES_URL: &str = "https://api.github.com/repos/RNT56/ThinClaw/releases";
const MAX_RELEASE_METADATA_BYTES: usize = 8 * 1024 * 1024;
const MAX_CHECKSUMS_BYTES: usize = 1024 * 1024;
const MAX_UPDATE_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;
const MAX_UPDATE_BINARY_BYTES: usize = 256 * 1024 * 1024;
const MAX_RELEASES: usize = 100;
const MAX_RELEASE_ASSETS: usize = 256;
const MAX_RELEASE_NOTES_BYTES: usize = 128 * 1024;

#[derive(Subcommand, Debug, Clone)]
pub enum UpdateCommand {
    /// Check for available updates
    Check {
        /// Update channel: stable (default), beta, nightly
        #[arg(long, default_value = "stable")]
        channel: String,
    },

    /// Download and install the latest version
    Install {
        /// Update channel: stable (default), beta, nightly
        #[arg(long, default_value = "stable")]
        channel: String,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,

        /// Specific version to install (instead of latest)
        #[arg(long)]
        version: Option<String>,
    },

    /// Show the current version and build info
    Info,

    /// Rollback to the previous version (if a backup exists)
    Rollback,
}

/// Version information from the releases API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    /// Version string (e.g. "0.12.1").
    pub version: String,
    /// Release channel.
    pub channel: String,
    /// Release notes / changelog.
    pub notes: Option<String>,
    /// Release date (ISO 8601).
    pub published_at: Option<String>,
    /// Download URL for the current platform.
    pub download_url: Option<String>,
    /// Asset file size in bytes.
    pub size_bytes: Option<u64>,
    /// SHA-256 checksum of the binary.
    pub sha256: Option<String>,
}

/// Build info for the current binary.
#[derive(Debug, Serialize)]
pub struct BuildInfo {
    pub version: String,
    pub target: String,
    pub profile: String,
    pub rustc_version: String,
    pub build_date: String,
}

fn best_asset_for_target<'a>(
    assets: &'a [serde_json::Value],
    target_os: &str,
    target_arch: &str,
) -> Option<&'a serde_json::Value> {
    assets
        .iter()
        .filter_map(|asset| {
            let name = asset["name"].as_str()?.to_ascii_lowercase();
            let mut score = 0i32;

            if name.contains(target_os) {
                score += 10;
            }
            if name.contains(target_arch) {
                score += 10;
            }

            // Common alias support.
            if target_arch == "x86_64" && (name.contains("amd64") || name.contains("x64")) {
                score += 6;
            }
            if target_arch == "aarch64" && (name.contains("arm64") || name.contains("armv8")) {
                score += 6;
            }
            if target_os == "macos" && name.contains("darwin") {
                score += 6;
            }
            if target_os == "windows" && name.contains("win") {
                score += 6;
            }

            // Prefer executable archive formats over source blobs.
            if name.ends_with(".tar.gz")
                || name.ends_with(".tgz")
                || name.ends_with(".zip")
                || name.ends_with(".msi")
                || name.ends_with(".exe")
            {
                score += 3;
            }
            if name.contains("source") || name.contains("src") {
                score -= 5;
            }

            Some((score, asset))
        })
        .max_by_key(|(score, _)| *score)
        .filter(|(score, _)| *score >= 10)
        .map(|(_, asset)| asset)
}

fn expected_archive_name() -> Option<String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return None,
    };
    let target = match std::env::consts::OS {
        "macos" => format!("{arch}-apple-darwin"),
        "linux" if cfg!(target_env = "musl") => format!("{arch}-unknown-linux-musl"),
        "linux" => format!("{arch}-unknown-linux-gnu"),
        _ => return None,
    };
    Some(format!("thinclaw-{target}.tar.gz"))
}

impl BuildInfo {
    pub fn current() -> Self {
        Self {
            version: CURRENT_VERSION.to_string(),
            target: std::env::consts::ARCH.to_string() + "-" + std::env::consts::OS,
            profile: if cfg!(debug_assertions) {
                "debug".to_string()
            } else {
                "release".to_string()
            },
            rustc_version: option_env!("RUSTC_VERSION")
                .unwrap_or("unknown")
                .to_string(),
            build_date: option_env!("BUILD_DATE").unwrap_or("unknown").to_string(),
        }
    }
}

/// Compare two SemVer version strings. Invalid versions never trigger updates.
pub fn is_newer_version(current: &str, available: &str) -> bool {
    let Ok(current) = semver::Version::parse(current.trim_start_matches('v')) else {
        return false;
    };
    let Ok(available) = semver::Version::parse(available.trim_start_matches('v')) else {
        return false;
    };
    available > current
}

/// Path for the backup binary (used for rollback).
fn backup_binary_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("thinclaw"));
    exe.with_extension("bak")
}

fn safe_backup_available() -> bool {
    std::fs::symlink_metadata(backup_binary_path())
        .is_ok_and(|metadata| metadata.file_type().is_file() && !metadata.file_type().is_symlink())
}

#[cfg(target_os = "windows")]
fn staged_windows_asset_path(download_url: &str, version: &str) -> PathBuf {
    let extension = url::Url::parse(download_url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|segments| segments.last().map(str::to_string))
        })
        .and_then(|name| {
            PathBuf::from(name)
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
        })
        .filter(|extension| matches!(extension.as_str(), "msi" | "zip" | "exe"))
        .unwrap_or_else(|| "bin".to_string());
    crate::platform::resolve_data_dir("updates")
        .join(uuid::Uuid::new_v4().simple().to_string())
        .join(format!("thinclaw-{version}.{extension}"))
}

#[cfg(target_os = "windows")]
async fn apply_windows_update_asset(
    branding: &TerminalBranding,
    download_url: &str,
    bytes: &[u8],
    version: &str,
) -> anyhow::Result<()> {
    let staged_path = staged_windows_asset_path(download_url, version);
    if let Some(parent) = staged_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    thinclaw_platform::write_private_file_atomic(&staged_path, bytes, false)?;

    println!(
        "{}",
        branding.key_value("Staged asset", staged_path.display())
    );

    let extension = staged_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());

    match extension.as_deref() {
        Some("msi") => {
            println!(
                "{}",
                branding.accent("Launching the Windows installer via msiexec...")
            );
            let mut command = tokio::process::Command::new("msiexec");
            command
                .args(["/i"])
                .arg(&staged_path)
                .args(["/passive", "/norestart"]);
            let output = thinclaw_platform::bounded_command_output(
                &mut command,
                Duration::from_secs(30 * 60),
                64 * 1024,
                64 * 1024,
            )
            .await?;
            if !output.status.success() {
                anyhow::bail!(
                    "msiexec failed for {} (exit code {:?})",
                    staged_path.display(),
                    output.status.code()
                );
            }
            println!(
                "{}",
                branding.good(format!(
                    "Installer launched for v{}. Close ThinClaw if Windows asks before finishing the upgrade.",
                    version
                ))
            );
        }
        Some("zip") => {
            println!(
                "{}",
                branding.warn(format!(
                    "Downloaded the portable ZIP to {}. Extract it and replace the portable ThinClaw files after ThinClaw exits.",
                    staged_path.display()
                ))
            );
        }
        _ => {
            println!(
                "{}",
                branding.warn(format!(
                    "Downloaded the Windows update asset to {}. Run it after ThinClaw exits.",
                    staged_path.display()
                ))
            );
        }
    }

    Ok(())
}

fn valid_official_asset_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let Some(segments) = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
    else {
        return false;
    };
    segments.len() >= 6
        && segments[0] == "RNT56"
        && segments[1] == "ThinClaw"
        && segments[2] == "releases"
        && segments[3] == "download"
}

fn valid_update_redirect_url(url: &url::Url) -> bool {
    const HOSTS: &[&str] = &[
        "api.github.com",
        "github.com",
        "objects.githubusercontent.com",
        "release-assets.githubusercontent.com",
        "github-releases.githubusercontent.com",
    ];
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.host_str().is_some_and(|host| HOSTS.contains(&host))
}

async fn guarded_update_get(
    initial_url: url::Url,
    allowed_hosts: &[&str],
    timeout: Duration,
    max_redirects: usize,
) -> anyhow::Result<reqwest::Response> {
    tokio::time::timeout(timeout, async move {
        let mut current = initial_url;
        for redirect_count in 0..=max_redirects {
            let guarded = thinclaw_tools_core::validate_outbound_url_pinned_async(
                current.as_str(),
                &thinclaw_tools_core::OutboundUrlGuardOptions {
                    require_https: true,
                    upgrade_http_to_https: false,
                    allowlist: allowed_hosts
                        .iter()
                        .map(|host| (*host).to_string())
                        .collect(),
                },
            )
            .await
            .map_err(|error| anyhow::anyhow!("update URL was rejected: {error}"))?;
            let host = guarded
                .url
                .host_str()
                .ok_or_else(|| anyhow::anyhow!("update URL has no host"))?;
            let mut builder = reqwest::Client::builder()
                .timeout(timeout)
                .connect_timeout(Duration::from_secs(10))
                .user_agent(format!("thinclaw/{CURRENT_VERSION}"))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy();
            if !guarded.pinned_addrs.is_empty() {
                builder = builder.resolve_to_addrs(host, &guarded.pinned_addrs);
            }
            let response = builder
                .build()?
                .get(guarded.url.clone())
                .send()
                .await
                .map_err(|error| anyhow::anyhow!(error.without_url()))?;
            if !response.status().is_redirection() {
                return Ok(response);
            }
            if redirect_count == max_redirects {
                anyhow::bail!("update download exceeded its redirect limit");
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .filter(|value| value.len() <= 4_096)
                .ok_or_else(|| anyhow::anyhow!("update redirect omitted a valid Location"))?;
            current = guarded
                .url
                .join(location)
                .map_err(|_| anyhow::anyhow!("update redirect URL is invalid"))?;
            if !valid_update_redirect_url(&current) {
                anyhow::bail!("update redirect target is not allowed");
            }
        }
        unreachable!("bounded redirect loop always returns")
    })
    .await
    .map_err(|_| anyhow::anyhow!("update request timed out after {timeout:?}"))?
}

async fn download_release_asset(value: &str, limit: usize) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        valid_official_asset_url(value),
        "release asset URL is not an official ThinClaw GitHub release URL"
    );
    let response = guarded_update_get(
        url::Url::parse(value)?,
        &[
            "github.com",
            "objects.githubusercontent.com",
            "release-assets.githubusercontent.com",
            "github-releases.githubusercontent.com",
        ],
        Duration::from_secs(180),
        5,
    )
    .await?;
    anyhow::ensure!(
        response.status().is_success(),
        "release asset returned HTTP {}",
        response.status()
    );
    Ok(crate::http_response::bounded_bytes(response, limit).await?)
}

fn checksum_for_asset(contents: &str, asset_name: &str) -> anyhow::Result<String> {
    let mut matched = None;
    for line in contents.lines().take(4096) {
        let mut fields = line.split_ascii_whitespace();
        let Some(hash) = fields.next() else {
            continue;
        };
        let Some(filename) = fields.next() else {
            continue;
        };
        if fields.next().is_some() || filename.trim_start_matches('*') != asset_name {
            continue;
        }
        anyhow::ensure!(
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "release checksum is not a SHA-256 digest"
        );
        let normalized = hash.to_ascii_lowercase();
        if let Some(existing) = matched.as_ref() {
            anyhow::ensure!(existing == &normalized, "release has conflicting checksums");
        } else {
            matched = Some(normalized);
        }
    }
    matched.ok_or_else(|| anyhow::anyhow!("release checksum for '{asset_name}' was not found"))
}

fn truncate_utf8(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut boundary = limit;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_string()
}

async fn fetch_release(channel: &str, requested: Option<&str>) -> anyhow::Result<ReleaseInfo> {
    anyhow::ensure!(
        matches!(channel, "stable" | "beta" | "nightly"),
        "update channel must be stable, beta, or nightly"
    );

    let requested_version = requested
        .map(|value| semver::Version::parse(value.trim_start_matches('v')))
        .transpose()
        .map_err(|error| anyhow::anyhow!("invalid requested version: {error}"))?;
    let mut url = url::Url::parse(DEFAULT_RELEASES_URL)?;
    if let Some(version) = requested_version.as_ref() {
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid official releases URL"))?
            .extend(["tags", &format!("v{version}")]);
    } else {
        url.query_pairs_mut()
            .append_pair("per_page", &MAX_RELEASES.to_string());
    }

    let response = guarded_update_get(url, &["api.github.com"], Duration::from_secs(30), 2).await?;
    anyhow::ensure!(
        response.status().is_success(),
        "Failed to fetch releases: HTTP {}",
        response.status()
    );
    let releases = if requested_version.is_some() {
        vec![
            crate::http_response::bounded_json::<serde_json::Value>(
                response,
                MAX_RELEASE_METADATA_BYTES,
            )
            .await?,
        ]
    } else {
        crate::http_response::bounded_json::<Vec<serde_json::Value>>(
            response,
            MAX_RELEASE_METADATA_BYTES,
        )
        .await?
    };

    for release in releases.into_iter().take(MAX_RELEASES) {
        let Some(tag) = release["tag_name"].as_str() else {
            continue;
        };
        let Ok(version) = semver::Version::parse(tag.trim_start_matches('v')) else {
            continue;
        };
        if requested_version
            .as_ref()
            .is_some_and(|requested| requested != &version)
        {
            continue;
        }
        let is_prerelease = release["prerelease"].as_bool().unwrap_or(false);
        let matches_channel = match channel {
            "stable" => !is_prerelease && version.pre.is_empty(),
            "beta" => is_prerelease && version.pre.as_str().contains("beta"),
            "nightly" => is_prerelease,
            _ => false,
        };
        if !matches_channel {
            continue;
        }

        let assets = release["assets"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("release has no asset list"))?;
        anyhow::ensure!(
            assets.len() <= MAX_RELEASE_ASSETS,
            "release has too many assets"
        );
        let selected_asset = if let Some(expected_name) = expected_archive_name() {
            assets
                .iter()
                .find(|asset| asset["name"].as_str() == Some(expected_name.as_str()))
        } else {
            best_asset_for_target(assets, std::env::consts::OS, std::env::consts::ARCH)
        }
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No official update archive is available for {}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;
        let asset_name = selected_asset["name"]
            .as_str()
            .filter(|name| {
                !name.is_empty()
                    && name.len() <= 256
                    && !name.contains(['/', '\\', '\0'])
                    && *name != "."
                    && *name != ".."
            })
            .ok_or_else(|| anyhow::anyhow!("release asset has an invalid filename"))?;
        let download_url = selected_asset["browser_download_url"]
            .as_str()
            .filter(|url| url.len() <= 4096 && valid_official_asset_url(url))
            .ok_or_else(|| anyhow::anyhow!("release asset URL is invalid"))?;
        let size_bytes = selected_asset["size"].as_u64();
        anyhow::ensure!(
            size_bytes.is_none_or(|size| size > 0 && size <= MAX_UPDATE_ARCHIVE_BYTES as u64),
            "release archive size is invalid"
        );

        let checksums_url = assets
            .iter()
            .find(|asset| asset["name"].as_str() == Some("checksums.txt"))
            .and_then(|asset| asset["browser_download_url"].as_str())
            .filter(|url| url.len() <= 4096 && valid_official_asset_url(url))
            .ok_or_else(|| anyhow::anyhow!("release is missing its official checksums.txt"))?;
        let checksum_bytes = download_release_asset(checksums_url, MAX_CHECKSUMS_BYTES).await?;
        let checksum_text = std::str::from_utf8(&checksum_bytes)
            .map_err(|_| anyhow::anyhow!("release checksum file is not valid UTF-8"))?;
        let sha256 = checksum_for_asset(checksum_text, asset_name)?;

        return Ok(ReleaseInfo {
            version: version.to_string(),
            channel: channel.to_string(),
            notes: release["body"]
                .as_str()
                .map(|notes| truncate_utf8(notes, MAX_RELEASE_NOTES_BYTES)),
            published_at: release["published_at"]
                .as_str()
                .filter(|value| value.len() <= 128)
                .map(String::from),
            download_url: Some(download_url.to_string()),
            size_bytes,
            sha256: Some(sha256),
        });
    }

    if let Some(version) = requested_version {
        anyhow::bail!("Release v{version} does not belong to the {channel} channel")
    }
    anyhow::bail!("No {channel} release found")
}

/// Fetch the latest release info from the official release API.
async fn fetch_latest_release(channel: &str) -> anyhow::Result<ReleaseInfo> {
    fetch_release(channel, None).await
}

async fn fetch_specific_release(channel: &str, version: &str) -> anyhow::Result<ReleaseInfo> {
    fetch_release(channel, Some(version)).await
}

fn verify_update_archive(
    bytes: &[u8],
    expected_sha256: &str,
    expected_size: Option<u64>,
) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};

    anyhow::ensure!(!bytes.is_empty(), "downloaded update archive is empty");
    if let Some(expected_size) = expected_size {
        anyhow::ensure!(
            bytes.len() as u64 == expected_size,
            "downloaded update archive size does not match release metadata"
        );
    }
    let actual = hex::encode(Sha256::digest(bytes));
    anyhow::ensure!(
        actual == expected_sha256,
        "downloaded update archive failed SHA-256 verification"
    );
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn extract_update_binary(archive_bytes: &[u8], asset_name: &str) -> anyhow::Result<Vec<u8>> {
    use std::io::Read as _;

    anyhow::ensure!(
        asset_name.ends_with(".tar.gz") || asset_name.ends_with(".tgz"),
        "self-update requires an official tar.gz archive on this platform"
    );
    let decoder = flate2::read::GzDecoder::new(archive_bytes);
    let mut archive = tar::Archive::new(decoder);
    archive.set_preserve_permissions(false);
    #[cfg(any(unix, target_os = "redox"))]
    archive.set_unpack_xattrs(false);

    let mut binary = None;
    for (index, entry) in archive.entries()?.enumerate() {
        anyhow::ensure!(index < 256, "update archive contains too many entries");
        let mut entry = entry?;
        let path = entry.path()?;
        let filename = path.file_name().and_then(|name| name.to_str());
        if filename != Some("thinclaw") {
            continue;
        }
        anyhow::ensure!(
            binary.is_none(),
            "update archive contains duplicate binaries"
        );
        anyhow::ensure!(
            entry.header().entry_type().is_file()
                && entry.size() > 0
                && entry.size() <= MAX_UPDATE_BINARY_BYTES as u64,
            "update binary is not a bounded regular archive entry"
        );
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .by_ref()
            .take(MAX_UPDATE_BINARY_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() <= MAX_UPDATE_BINARY_BYTES,
            "update binary exceeds the size limit"
        );
        binary = Some(bytes);
    }
    let binary = binary.ok_or_else(|| anyhow::anyhow!("update archive has no ThinClaw binary"))?;
    validate_native_update_binary(&binary)?;
    Ok(binary)
}

#[cfg(not(target_os = "windows"))]
fn validate_native_update_binary(bytes: &[u8]) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        anyhow::ensure!(
            bytes.len() >= 20 && bytes.starts_with(b"\x7fELF") && bytes[4] == 2 && bytes[5] == 1,
            "update payload is not a 64-bit little-endian ELF binary"
        );
        let machine = u16::from_le_bytes([bytes[18], bytes[19]]);
        let expected = match std::env::consts::ARCH {
            "x86_64" => 62,
            "aarch64" => 183,
            other => anyhow::bail!("self-update is unsupported on architecture {other}"),
        };
        anyhow::ensure!(
            machine == expected,
            "update binary architecture does not match"
        );
    }
    #[cfg(target_os = "macos")]
    {
        anyhow::ensure!(
            bytes.len() >= 8 && bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]),
            "update payload is not a 64-bit Mach-O binary"
        );
        let cpu_type = u32::from_le_bytes(bytes[4..8].try_into().expect("four-byte CPU type"));
        let expected = match std::env::consts::ARCH {
            "x86_64" => 0x0100_0007,
            "aarch64" => 0x0100_000c,
            other => anyhow::bail!("self-update is unsupported on architecture {other}"),
        };
        anyhow::ensure!(
            cpu_type == expected,
            "update binary architecture does not match"
        );
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    anyhow::bail!("self-update is unsupported on this operating system");
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn publish_update_binary(bytes: &[u8]) -> anyhow::Result<PathBuf> {
    use std::io::Write as _;

    let current = std::env::current_exe()?;
    let parent = current
        .parent()
        .ok_or_else(|| anyhow::anyhow!("current executable has no parent directory"))?;
    let current_metadata = std::fs::symlink_metadata(&current)?;
    anyhow::ensure!(
        current_metadata.file_type().is_file() && !current_metadata.file_type().is_symlink(),
        "current executable is not a regular file"
    );

    let stage = parent.join(format!(
        ".thinclaw.{}.update.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let backup_stage = parent.join(format!(
        ".thinclaw.{}.backup.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let backup = backup_binary_path();

    let result = (|| -> anyhow::Result<()> {
        let mut stage_options = std::fs::OpenOptions::new();
        stage_options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            stage_options.mode(0o755);
        }
        let mut staged = stage_options.open(&stage)?;
        staged.write_all(bytes)?;
        staged.sync_all()?;

        let mut source_options = std::fs::OpenOptions::new();
        source_options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            source_options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut source = source_options.open(&current)?;
        anyhow::ensure!(source.metadata()?.is_file(), "current executable changed");
        let mut backup_options = std::fs::OpenOptions::new();
        backup_options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            backup_options.mode(0o700);
        }
        let mut staged_backup = backup_options.open(&backup_stage)?;
        std::io::copy(&mut source, &mut staged_backup)?;
        staged_backup.sync_all()?;
        std::fs::rename(&backup_stage, &backup)?;
        std::fs::rename(&stage, &current)?;
        if let Ok(directory) = std::fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&stage);
        let _ = std::fs::remove_file(&backup_stage);
    }
    result?;
    Ok(backup)
}

#[cfg(not(target_os = "windows"))]
fn rollback_update_binary() -> anyhow::Result<PathBuf> {
    use std::io::{Read as _, Write as _};

    let backup = backup_binary_path();
    let metadata = std::fs::symlink_metadata(&backup)?;
    anyhow::ensure!(
        metadata.file_type().is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() > 0
            && metadata.len() <= MAX_UPDATE_BINARY_BYTES as u64,
        "rollback backup is not a bounded regular file"
    );
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(&backup)?;
    anyhow::ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= MAX_UPDATE_BINARY_BYTES as u64,
        "rollback backup changed while opening"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::io::Read::by_ref(&mut file)
        .take(MAX_UPDATE_BINARY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= MAX_UPDATE_BINARY_BYTES,
        "rollback backup exceeds the size limit"
    );
    validate_native_update_binary(&bytes)?;

    let current = std::env::current_exe()?;
    let current_metadata = std::fs::symlink_metadata(&current)?;
    anyhow::ensure!(
        current_metadata.file_type().is_file() && !current_metadata.file_type().is_symlink(),
        "current executable is not a regular file"
    );
    let parent = current
        .parent()
        .ok_or_else(|| anyhow::anyhow!("current executable has no parent directory"))?;
    let stage = parent.join(format!(
        ".thinclaw.{}.rollback.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> anyhow::Result<()> {
        let mut stage_options = std::fs::OpenOptions::new();
        stage_options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            stage_options.mode(0o755);
        }
        let mut staged = stage_options.open(&stage)?;
        staged.write_all(&bytes)?;
        staged.sync_all()?;
        std::fs::rename(&stage, &current)?;
        let _ = std::fs::remove_file(&backup);
        if let Ok(directory) = std::fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&stage);
    }
    result?;
    Ok(current)
}

/// Run an update CLI command.
pub async fn run_update_command(cmd: UpdateCommand) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    match cmd {
        UpdateCommand::Info => {
            branding.print_banner("Update", Some("Inspect the installed build"));
            let info = BuildInfo::current();
            println!(
                "{}",
                branding.key_value("Version", format!("ThinClaw v{}", info.version))
            );
            println!("{}", branding.key_value("Target", info.target));
            println!("{}", branding.key_value("Profile", info.profile));
            println!("{}", branding.key_value("Rustc", info.rustc_version));
            println!("{}", branding.key_value("Built", info.build_date));

            // Check for backup
            let backup = backup_binary_path();
            if safe_backup_available() {
                println!(
                    "{}",
                    branding.key_value(
                        "Backup",
                        format!("{} (rollback available)", backup.display())
                    )
                );
            }
        }

        UpdateCommand::Check { channel } => {
            branding.print_banner("Update", Some("Check for a new release"));
            println!(
                "{}",
                branding.accent(format!("Checking for updates ({} channel)...", channel))
            );

            match fetch_latest_release(&channel).await {
                Ok(release) => {
                    if is_newer_version(CURRENT_VERSION, &release.version) {
                        println!(
                            "{}",
                            branding.good(format!(
                                "Update available: v{} -> v{}",
                                CURRENT_VERSION, release.version
                            ))
                        );

                        if let Some(ref date) = release.published_at {
                            println!("{}", branding.key_value("Published", date));
                        }

                        if let Some(size) = release.size_bytes {
                            println!(
                                "{}",
                                branding.key_value(
                                    "Size",
                                    format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
                                )
                            );
                        }

                        if let Some(ref notes) = release.notes {
                            let preview: String = notes.chars().take(200).collect();
                            println!();
                            println!("{}", branding.key_value("Release notes", preview));
                        }

                        println!();
                        println!(
                            "{}",
                            branding.muted("Run `thinclaw update install` to apply the update.")
                        );
                    } else {
                        println!(
                            "{}",
                            branding.good(format!("Already up to date (v{}).", CURRENT_VERSION))
                        );
                    }
                }
                Err(e) => {
                    println!(
                        "{}",
                        branding.warn(format!("Could not check for updates: {}", e))
                    );
                    println!(
                        "{}",
                        branding.muted("This may be due to network issues or rate limiting.")
                    );
                }
            }
        }

        UpdateCommand::Install {
            channel,
            yes,
            version,
        } => {
            branding.print_banner("Update", Some("Download and install a release"));
            let release = if let Some(ref v) = version {
                println!(
                    "{}",
                    branding.accent(format!("Looking for version {}...", v))
                );
                fetch_specific_release(&channel, v).await?
            } else {
                println!(
                    "{}",
                    branding.accent(format!("Checking for updates ({} channel)...", channel))
                );
                fetch_latest_release(&channel).await?
            };

            if !is_newer_version(CURRENT_VERSION, &release.version) && version.is_none() {
                println!(
                    "{}",
                    branding.good(format!("Already up to date (v{}).", CURRENT_VERSION))
                );
                return Ok(());
            }

            println!(
                "{}",
                branding.good(format!(
                    "Update available: v{} -> v{}",
                    CURRENT_VERSION, release.version
                ))
            );

            let download_url = release.download_url.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "No download available for {}-{}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )
            })?;

            if !yes {
                println!();
                println!("{}", branding.key_value("Download URL", download_url));
                println!(
                    "{}",
                    branding.muted("Run again with `--yes` to confirm the update.")
                );
                return Ok(());
            }

            // Download
            println!(
                "{}",
                branding.accent(format!("Downloading v{}...", release.version))
            );
            let expected_sha256 = release
                .sha256
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("Release is missing a required SHA-256 checksum"))?;
            let bytes = download_release_asset(download_url, MAX_UPDATE_ARCHIVE_BYTES).await?;
            verify_update_archive(&bytes, expected_sha256, release.size_bytes)?;

            println!(
                "{}",
                branding.key_value(
                    "Downloaded",
                    format!("{:.1} MB", bytes.len() as f64 / (1024.0 * 1024.0))
                )
            );

            #[cfg(target_os = "windows")]
            {
                apply_windows_update_asset(
                    &branding,
                    download_url,
                    bytes.as_ref(),
                    &release.version,
                )
                .await?;
            }

            #[cfg(not(target_os = "windows"))]
            {
                let asset_name = url::Url::parse(download_url)?
                    .path_segments()
                    .and_then(|mut segments| segments.next_back().map(str::to_string))
                    .ok_or_else(|| anyhow::anyhow!("release asset URL has no filename"))?;
                let binary = extract_update_binary(&bytes, &asset_name)?;
                let backup = publish_update_binary(&binary)?;
                println!("{}", branding.key_value("Backup", backup.display()));
                println!(
                    "{}",
                    branding.good(format!(
                        "Updated to v{}. Restart ThinClaw for changes to take effect.",
                        release.version
                    ))
                );
            }
        }

        UpdateCommand::Rollback => {
            branding.print_banner("Update", Some("Rollback to the previous build"));

            #[cfg(target_os = "windows")]
            {
                println!(
                    "{}",
                    branding.warn(
                        "Windows rollback is installer-based. Reinstall the previous MSI/ZIP instead of swapping the running executable."
                    )
                );
            }

            #[cfg(not(target_os = "windows"))]
            {
                let backup = backup_binary_path();
                if !safe_backup_available() {
                    println!(
                        "{}",
                        branding.warn(format!("No backup found at {}.", backup.display()))
                    );
                    println!(
                        "{}",
                        branding.muted("Rollback is only available after a successful update.")
                    );
                    return Ok(());
                }

                let _current = rollback_update_binary()?;
                println!("{}", branding.good("Rolled back to the previous version."));
                println!(
                    "{}",
                    branding.muted("Restart ThinClaw for changes to take effect.")
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_version() {
        assert!(!CURRENT_VERSION.is_empty());
    }

    #[test]
    fn test_is_newer_version_major() {
        assert!(is_newer_version("0.12.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "0.12.0"));
    }

    #[test]
    fn test_is_newer_version_minor() {
        assert!(is_newer_version("0.12.0", "0.13.0"));
        assert!(!is_newer_version("0.13.0", "0.12.0"));
    }

    #[test]
    fn test_is_newer_version_patch() {
        assert!(is_newer_version("0.12.0", "0.12.1"));
        assert!(!is_newer_version("0.12.1", "0.12.0"));
    }

    #[test]
    fn test_is_newer_version_equal() {
        assert!(!is_newer_version("0.12.0", "0.12.0"));
    }

    #[test]
    fn test_is_newer_version_with_v_prefix() {
        assert!(is_newer_version("v0.12.0", "v0.13.0"));
    }

    #[test]
    fn test_is_newer_version_with_prerelease() {
        assert!(is_newer_version("0.12.0", "0.13.0-beta.1"));
        assert!(is_newer_version("0.13.0-beta.1", "0.13.0"));
        assert!(!is_newer_version("0.13.0", "0.13.0-beta.1"));
        assert!(!is_newer_version("invalid", "99.0.0"));
    }

    #[test]
    fn test_official_asset_url_validation() {
        assert!(valid_official_asset_url(
            "https://github.com/RNT56/ThinClaw/releases/download/v0.15.0/thinclaw-aarch64-apple-darwin.tar.gz"
        ));
        assert!(!valid_official_asset_url(
            "https://github.com/attacker/ThinClaw/releases/download/v0.15.0/payload"
        ));
        assert!(!valid_official_asset_url(
            "https://127.0.0.1/RNT56/ThinClaw/releases/download/v0.15.0/payload"
        ));
    }

    #[test]
    fn test_checksum_parser_requires_exact_filename() {
        let contents = concat!(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  other.tar.gz\n",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  thinclaw-test.tar.gz\n"
        );
        assert_eq!(
            checksum_for_asset(contents, "thinclaw-test.tar.gz").unwrap(),
            "b".repeat(64)
        );
        assert!(checksum_for_asset(contents, "thin.tar.gz").is_err());
    }

    #[test]
    fn test_update_archive_checksum_verification() {
        use sha2::{Digest, Sha256};

        let bytes = b"archive";
        let checksum = hex::encode(Sha256::digest(bytes));
        assert!(verify_update_archive(bytes, &checksum, Some(bytes.len() as u64)).is_ok());
        assert!(verify_update_archive(bytes, &"0".repeat(64), None).is_err());
        assert!(verify_update_archive(bytes, &checksum, Some(1)).is_err());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn test_update_archive_extracts_only_native_thinclaw_binary() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use tar::Builder;

        #[cfg(target_os = "macos")]
        let binary = {
            let mut bytes = vec![0xcf, 0xfa, 0xed, 0xfe];
            let cpu_type: u32 = if cfg!(target_arch = "aarch64") {
                0x0100_000c
            } else {
                0x0100_0007
            };
            bytes.extend_from_slice(&cpu_type.to_le_bytes());
            bytes
        };
        #[cfg(target_os = "linux")]
        let binary = {
            let mut bytes = vec![0_u8; 20];
            bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
            let machine: u16 = if cfg!(target_arch = "aarch64") {
                183
            } else {
                62
            };
            bytes[18..20].copy_from_slice(&machine.to_le_bytes());
            bytes
        };

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = Builder::new(&mut encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(binary.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, "package/thinclaw", binary.as_slice())
                .unwrap();
            builder.finish().unwrap();
        }
        let archive = encoder.finish().unwrap();
        assert_eq!(
            extract_update_binary(&archive, "thinclaw-test.tar.gz").unwrap(),
            binary
        );
    }

    #[test]
    fn test_build_info() {
        let info = BuildInfo::current();
        assert_eq!(info.version, CURRENT_VERSION);
        assert!(!info.target.is_empty());
    }

    #[test]
    fn test_backup_binary_path() {
        let path = backup_binary_path();
        assert!(path.extension().is_some_and(|e| e == "bak"));
    }

    #[test]
    fn test_release_info_serialization() {
        let release = ReleaseInfo {
            version: "0.13.0".to_string(),
            channel: "stable".to_string(),
            notes: Some("Bug fixes".to_string()),
            published_at: Some("2026-03-04T00:00:00Z".to_string()),
            download_url: Some("https://example.com/thinclaw".to_string()),
            size_bytes: Some(15_000_000),
            sha256: None,
        };
        let json = serde_json::to_string(&release).unwrap();
        assert!(json.contains("0.13.0"));
        assert!(json.contains("stable"));
    }
}
