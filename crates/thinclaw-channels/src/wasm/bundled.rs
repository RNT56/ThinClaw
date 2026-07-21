//! Known WASM channels that can be installed from build artifacts.
//!
//! Instead of embedding WASM binaries in the host binary via include_bytes!,
//! channels are compiled separately and installed from their build output
//! directories during onboarding.
//!
//! Channel source layout:
//!   channels-src/<name>/
//!     target/wasm32-wasip2/release/<name>_channel.wasm
//!     <name>.capabilities.json

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::fs;

use crate::wasm::schema::ChannelCapabilitiesFile;

const MAX_WASM_BYTES: usize = 64 * 1024 * 1024;
const MAX_CAPABILITIES_BYTES: usize = 2 * 1024 * 1024;
const BUILD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_BUILD_STDOUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_BUILD_STDERR_BYTES: usize = 4 * 1024 * 1024;

fn is_real_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| !metadata.file_type().is_symlink() && metadata.is_dir())
        .unwrap_or(false)
}

fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| !metadata.file_type().is_symlink() && metadata.is_file())
        .unwrap_or(false)
}

async fn read_regular_file_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>, String> {
    let display = path.display().to_string();
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > max_bytes as u64
        {
            return Err(std::io::Error::other(
                "channel artifact is not a bounded regular file",
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
            return Err(std::io::Error::other(
                "channel artifact changed or exceeds its size limit",
            ));
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(opened.len())
                .unwrap_or(max_bytes)
                .min(max_bytes),
        );
        file.by_ref()
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > max_bytes {
            return Err(std::io::Error::other(
                "channel artifact exceeds its size limit",
            ));
        }
        Ok(bytes)
    })
    .await
    .map_err(|error| format!("channel artifact reader panicked: {error}"))?
    .map_err(|error| format!("failed to read {display}: {error}"))
}

/// Compile-time crate root, used to locate the workspace `channels-src/` in dev builds.
const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Known channel names and their crate names (for locating build artifacts).
const KNOWN_CHANNELS: &[(&str, &str)] = &[
    ("telegram", "telegram_channel"),
    ("slack", "slack_channel"),
    ("discord", "discord_channel"),
    ("whatsapp", "whatsapp_channel"),
    ("matrix", "matrix_channel"),
    ("mattermost", "mattermost_channel"),
    ("twilio_sms", "twilio_sms_channel"),
    ("dingtalk", "dingtalk_channel"),
    ("feishu_lark", "feishu_lark_channel"),
    ("wecom", "wecom_channel"),
    ("weixin", "weixin_channel"),
    ("qq", "qq_channel"),
    ("line", "line_channel"),
    ("google_chat", "google_chat_channel"),
    ("ms_teams", "ms_teams_channel"),
    ("twitch", "twitch_channel"),
];

/// Names of known channels that can be installed.
pub fn bundled_channel_names() -> Vec<&'static str> {
    KNOWN_CHANNELS.iter().map(|(name, _)| *name).collect()
}

/// Resolve the channels source directory.
///
/// Checks (in order):
/// 1. `THINCLAW_CHANNELS_SRC` env var
/// 2. `<CARGO_MANIFEST_DIR>/channels-src/` (dev builds)
fn channels_src_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("THINCLAW_CHANNELS_SRC") {
        return PathBuf::from(dir);
    }
    PathBuf::from(CARGO_MANIFEST_DIR)
        .join("../..")
        .join("channels-src")
}

/// Locate the build artifacts for a channel.
///
/// Checks two layouts:
/// 1. **Flat** (Docker/packaged): `<channels_src>/<name>/<name>.wasm`
/// 2. **Build tree** (dev): `<channels_src>/<name>/target/wasm32-wasip2/release/<crate_name>.wasm`
///
/// Returns (wasm_path, capabilities_path) or an error if files are missing.
fn locate_channel_artifacts(name: &str) -> Result<(PathBuf, PathBuf), String> {
    let (_, crate_name) = KNOWN_CHANNELS
        .iter()
        .find(|(n, _)| *n == name)
        .ok_or_else(|| format!("Unknown channel '{}'", name))?;

    let src_dir = channels_src_dir();
    let channel_dir = src_dir.join(name);
    if !is_real_directory(&channel_dir) {
        return Err(format!(
            "Channel '{}' source directory is missing or is not a real directory: {}",
            name,
            channel_dir.display()
        ));
    }

    let caps_path = channel_dir.join(format!("{}.capabilities.json", name));

    // Check flat layout first (Docker/packaged deployments)
    let flat_wasm = channel_dir.join(format!("{}.wasm", name));
    if is_regular_file(&flat_wasm) && is_regular_file(&caps_path) {
        return Ok((flat_wasm, caps_path));
    }

    // Fall back to build tree layout (dev builds) — search across all WASM triples
    if let Some(build_wasm) = find_wasm_artifact(&channel_dir, crate_name, "release")
        && is_regular_file(&caps_path)
    {
        return Ok((build_wasm, caps_path));
    }

    // Provide a helpful error with the paths we checked
    let expected_build = resolve_target_dir(&channel_dir)
        .join("wasm32-wasip2/release")
        .join(format!("{}.wasm", crate_name));

    Err(format!(
        "Channel '{}' WASM not found. Checked:\n  \
         - {} (flat/packaged)\n  \
         - {} (build tree, and other triples)\n  \
         Build it first:\n  \
         cd {} && cargo component build --release",
        name,
        flat_wasm.display(),
        expected_build.display(),
        channel_dir.display()
    ))
}

const WASM_TRIPLES: &[&str] = &[
    "wasm32-wasip1",
    "wasm32-wasip2",
    "wasm32-wasi",
    "wasm32-unknown-unknown",
];

fn resolve_target_dir(crate_dir: &Path) -> PathBuf {
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        let path = PathBuf::from(dir);
        if path.is_relative() {
            return crate_dir.join(path);
        }
        return path;
    }
    crate_dir.join("target")
}

fn find_wasm_artifact(crate_dir: &Path, crate_name: &str, profile: &str) -> Option<PathBuf> {
    let target_base = resolve_target_dir(crate_dir);
    let snake_name = crate_name.replace('-', "_");

    for triple in WASM_TRIPLES {
        let dir = target_base.join(triple).join(profile);
        for candidate in [
            dir.join(format!("{}.wasm", crate_name)),
            dir.join(format!("{}.wasm", snake_name)),
        ] {
            if is_regular_file(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

/// Install a channel from build artifacts into the channels directory.
pub async fn install_bundled_channel(
    name: &str,
    target_dir: &Path,
    force: bool,
) -> Result<(), String> {
    if !KNOWN_CHANNELS.iter().any(|(known, _)| *known == name) {
        return Err(format!("Unknown channel '{name}'"));
    }
    fs::create_dir_all(target_dir)
        .await
        .map_err(|e| format!("Failed to create channels directory: {e}"))?;
    let target_metadata = fs::symlink_metadata(target_dir)
        .await
        .map_err(|error| format!("Failed to inspect channels directory: {error}"))?;
    if target_metadata.file_type().is_symlink() || !target_metadata.is_dir() {
        return Err(format!(
            "Channels target is not a real directory: {}",
            target_dir.display()
        ));
    }

    let wasm_dst = target_dir.join(format!("{name}.wasm"));
    let caps_dst = target_dir.join(format!("{name}.capabilities.json"));
    let has_existing = match fs::symlink_metadata(&wasm_dst).await {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("Failed to inspect existing channel: {error}")),
    } || match fs::symlink_metadata(&caps_dst).await {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("Failed to inspect existing channel: {error}")),
    };
    if has_existing && !force {
        return Err(format!(
            "Channel '{name}' already exists at {}",
            target_dir.display()
        ));
    }

    let (wasm_src, caps_src) = match locate_channel_artifacts(name) {
        Ok(paths) => paths,
        Err(initial_error) => {
            build_channel_artifact(name).await.map_err(|build_error| {
                format!("{initial_error}\nAutomatic build also failed: {build_error}")
            })?;
            locate_channel_artifacts(name)?
        }
    };

    let wasm = read_regular_file_bounded(&wasm_src, MAX_WASM_BYTES).await?;
    if wasm.len() < 8 || !wasm.starts_with(b"\0asm") {
        return Err(format!(
            "Channel '{name}' build artifact is not a valid-looking WASM module"
        ));
    }
    let capabilities = read_regular_file_bounded(&caps_src, MAX_CAPABILITIES_BYTES).await?;
    let manifest = ChannelCapabilitiesFile::from_bytes(&capabilities)
        .map_err(|error| format!("Channel '{name}' capabilities are invalid: {error}"))?;
    if manifest.r#type != "channel" || manifest.name != name {
        return Err(format!(
            "Channel '{name}' capabilities type/name does not match the requested package"
        ));
    }

    thinclaw_platform::publish_file_pair(
        wasm_dst,
        caps_dst,
        wasm,
        Some(capabilities),
        if force {
            thinclaw_platform::ExistingPairPolicy::Replace
        } else {
            thinclaw_platform::ExistingPairPolicy::Refuse
        },
    )
    .await
    .map_err(|error| format!("Failed to publish channel '{name}': {error}"))?;

    Ok(())
}

async fn build_channel_artifact(name: &str) -> Result<(), String> {
    let (_, _) = KNOWN_CHANNELS
        .iter()
        .find(|(n, _)| *n == name)
        .ok_or_else(|| format!("Unknown channel '{}'", name))?;

    let channel_dir = channels_src_dir().join(name);
    if !is_real_directory(&channel_dir) || !is_regular_file(&channel_dir.join("Cargo.toml")) {
        return Err(format!(
            "Channel '{}' has no regular source Cargo.toml at {}",
            name,
            channel_dir.display()
        ));
    }

    let mut command = tokio::process::Command::new("cargo");
    command
        .args(["component", "build", "--release"])
        .current_dir(&channel_dir);
    let output = thinclaw_platform::bounded_command_output(
        &mut command,
        BUILD_TIMEOUT,
        MAX_BUILD_STDOUT_BYTES,
        MAX_BUILD_STDERR_BYTES,
    )
    .await
    .map_err(|error| {
        format!(
            "failed to execute bounded `cargo component build --release` in {}: {error}",
            channel_dir.display()
        )
    })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "`cargo component build --release` exited with status {} in {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        channel_dir.display(),
        stdout.trim(),
        stderr.trim()
    ))
}

/// Check which known channels have build artifacts available.
pub fn available_channel_names() -> Vec<&'static str> {
    KNOWN_CHANNELS
        .iter()
        .filter(|(name, _)| locate_channel_artifacts(name).is_ok())
        .map(|(name, _)| *name)
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use tokio::fs;

    use super::*;

    #[test]
    fn test_known_channels_includes_packaged_messaging_channels() {
        let names = bundled_channel_names();
        for expected in [
            "telegram",
            "slack",
            "discord",
            "whatsapp",
            "matrix",
            "mattermost",
            "twilio_sms",
            "dingtalk",
            "feishu_lark",
            "wecom",
            "weixin",
            "qq",
            "line",
            "google_chat",
            "ms_teams",
            "twitch",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
    }

    #[test]
    fn test_channels_src_dir_default() {
        let dir = channels_src_dir();
        assert!(dir.ends_with("channels-src"));
    }

    #[test]
    fn test_locate_unknown_channel_errors() {
        assert!(locate_channel_artifacts("nonexistent").is_err());
    }

    #[tokio::test]
    async fn test_install_refuses_overwrite_without_force() {
        let dir = tempdir().unwrap();
        let wasm_path = dir.path().join("telegram.wasm");
        fs::write(&wasm_path, b"custom").await.unwrap();

        let result = install_bundled_channel("telegram", dir.path(), false).await;
        // Either fails because artifacts missing OR because file exists
        assert!(result.is_err());

        // Original file should be untouched
        let existing = fs::read(&wasm_path).await.unwrap();
        assert_eq!(existing, b"custom");
    }
}
