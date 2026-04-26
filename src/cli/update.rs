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

use clap::Subcommand;
use serde::{Deserialize, Serialize};

use crate::terminal_branding::TerminalBranding;

/// Current binary version (from Cargo.toml).
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default release API URL.
const DEFAULT_RELEASES_URL: &str = "https://api.github.com/repos/RNT56/ThinClaw/releases";

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
        .map(|(_, asset)| asset)
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

/// Compare two semver-ish version strings. Returns true if `available` is newer.
pub fn is_newer_version(current: &str, available: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .filter_map(|s| {
                // Handle pre-release suffixes like "0.12.1-beta.1"
                s.split('-').next().and_then(|n| n.parse::<u64>().ok())
            })
            .collect()
    };

    let c = parse(current);
    let a = parse(available);

    for (cv, av) in c.iter().zip(a.iter()) {
        if av > cv {
            return true;
        }
        if av < cv {
            return false;
        }
    }

    // If all compared parts are equal, the longer one is newer
    a.len() > c.len()
}

/// Path for the backup binary (used for rollback).
fn backup_binary_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("thinclaw"));
    exe.with_extension("bak")
}

#[cfg(target_os = "windows")]
fn staged_windows_asset_path(download_url: &str, version: &str) -> PathBuf {
    let file_name = url::Url::parse(download_url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|segments| segments.last().map(str::to_string))
        })
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("thinclaw-{version}-windows-update.bin"));

    std::env::temp_dir()
        .join("thinclaw-updates")
        .join(file_name)
}

#[cfg(target_os = "windows")]
fn apply_windows_update_asset(
    branding: &TerminalBranding,
    download_url: &str,
    bytes: &[u8],
    version: &str,
) -> anyhow::Result<()> {
    let staged_path = staged_windows_asset_path(download_url, version);
    if let Some(parent) = staged_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&staged_path, bytes)?;

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
            let status = std::process::Command::new("msiexec")
                .args(["/i"])
                .arg(&staged_path)
                .args(["/passive"])
                .status()?;
            if !status.success() {
                anyhow::bail!(
                    "msiexec failed for {} (exit code {:?})",
                    staged_path.display(),
                    status.code()
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

/// Fetch the latest release info from the API.
async fn fetch_latest_release(channel: &str) -> anyhow::Result<ReleaseInfo> {
    let url =
        std::env::var("THINCLAW_RELEASES_URL").unwrap_or_else(|_| DEFAULT_RELEASES_URL.to_string());

    let client = reqwest::Client::builder()
        .user_agent(format!("thinclaw/{}", CURRENT_VERSION))
        .build()?;

    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to fetch releases: HTTP {}", response.status());
    }

    let releases: Vec<serde_json::Value> = response.json().await?;

    // Find the latest release matching the channel
    let target_os = std::env::consts::OS;
    let target_arch = std::env::consts::ARCH;

    for release in &releases {
        let tag = release["tag_name"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches('v');

        let is_prerelease = release["prerelease"].as_bool().unwrap_or(false);

        // Channel matching
        let matches_channel = match channel {
            "stable" => !is_prerelease,
            "beta" => is_prerelease && tag.contains("beta"),
            "nightly" => is_prerelease,
            _ => !is_prerelease,
        };

        if !matches_channel {
            continue;
        }

        // Pick the best matching asset for this platform deterministically.
        let selected_asset = release["assets"]
            .as_array()
            .and_then(|assets| best_asset_for_target(assets, target_os, target_arch));

        let download_url = selected_asset
            .and_then(|a| a["browser_download_url"].as_str())
            .map(String::from);

        let size_bytes = selected_asset.and_then(|a| a["size"].as_u64());

        return Ok(ReleaseInfo {
            version: tag.to_string(),
            channel: channel.to_string(),
            notes: release["body"].as_str().map(String::from),
            published_at: release["published_at"].as_str().map(String::from),
            download_url,
            size_bytes,
            sha256: None,
        });
    }

    anyhow::bail!("No {} release found", channel)
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
            if backup.exists() {
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
                let r = fetch_latest_release(&channel).await?;
                if r.version != v.trim_start_matches('v') {
                    anyhow::bail!("Requested version {} not found (latest: {})", v, r.version);
                }
                r
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
            let client = reqwest::Client::new();
            let response = client.get(download_url).send().await?;
            let bytes = response.bytes().await?;

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
                )?;
            }

            #[cfg(not(target_os = "windows"))]
            {
                // Backup current binary
                let current = std::env::current_exe()?;
                let backup = backup_binary_path();
                std::fs::copy(&current, &backup)?;
                println!("{}", branding.key_value("Backup", backup.display()));

                // Replace binary
                let temp_path = current.with_extension("new");
                std::fs::write(&temp_path, &bytes)?;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let perms = std::fs::Permissions::from_mode(0o755);
                    std::fs::set_permissions(&temp_path, perms)?;
                }

                std::fs::rename(&temp_path, &current)?;
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
                if !backup.exists() {
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

                let current = std::env::current_exe()?;
                std::fs::rename(&backup, &current)?;
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
