//! Skill tool policy: package.

use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use thinclaw_tools_core::ToolError;

pub fn is_skipped_package_name(name: &str) -> bool {
    name == ".git"
        || name == ".DS_Store"
        || name == ".thinclaw-skill-lock.json"
        || name == ".cache"
        || name == "__pycache__"
        || name == "target"
        || name == "node_modules"
        || name == "tmp"
        || name == "temp"
        || name.starts_with('.')
}

pub fn relative_path_is_safe(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

#[derive(Debug, Clone)]
pub struct SkillPackageFile {
    pub relative_path: String,
    pub source_path: PathBuf,
    pub bytes: u64,
}

pub fn collect_skill_package_files(root: &Path) -> Result<Vec<SkillPackageFile>, ToolError> {
    fn walk(root: &Path, dir: &Path, files: &mut Vec<SkillPackageFile>) -> Result<(), ToolError> {
        let entries = std::fs::read_dir(dir).map_err(|err| {
            ToolError::ExecutionFailed(format!(
                "Failed to read skill directory '{}': {}",
                dir.display(),
                err
            ))
        })?;

        for entry in entries {
            let entry = entry.map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if is_skipped_package_name(&name) {
                continue;
            }

            let meta = std::fs::symlink_metadata(&path).map_err(|err| {
                ToolError::ExecutionFailed(format!("Failed to stat '{}': {}", path.display(), err))
            })?;
            if meta.file_type().is_symlink() {
                return Err(ToolError::ExecutionFailed(format!(
                    "Refusing to publish symlink '{}'",
                    path.display()
                )));
            }
            if meta.is_dir() {
                walk(root, &path, files)?;
                continue;
            }
            if !meta.is_file() {
                continue;
            }

            let relative = path.strip_prefix(root).map_err(|err| {
                ToolError::ExecutionFailed(format!(
                    "Failed to derive package path for '{}': {}",
                    path.display(),
                    err
                ))
            })?;
            if !relative_path_is_safe(relative) {
                return Err(ToolError::ExecutionFailed(format!(
                    "Refusing unsafe package path '{}'",
                    relative.display()
                )));
            }
            files.push(SkillPackageFile {
                relative_path: relative.to_string_lossy().replace('\\', "/"),
                source_path: path,
                bytes: meta.len(),
            });
        }
        Ok(())
    }

    if !root.join("SKILL.md").is_file() {
        return Err(ToolError::ExecutionFailed(format!(
            "Skill directory '{}' is missing SKILL.md",
            root.display()
        )));
    }

    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    if !files.iter().any(|file| file.relative_path == "SKILL.md") {
        return Err(ToolError::ExecutionFailed(
            "Skill package must include SKILL.md".to_string(),
        ));
    }
    Ok(files)
}

pub fn package_hash(files: &[SkillPackageFile]) -> Result<String, ToolError> {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.relative_path.as_bytes());
        hasher.update(b"\0");
        let bytes = std::fs::read(&file.source_path).map_err(|err| {
            ToolError::ExecutionFailed(format!(
                "Failed to read package file '{}': {}",
                file.source_path.display(),
                err
            ))
        })?;
        hasher.update(&bytes);
        hasher.update(b"\0");
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

pub fn package_scan_content(files: &[SkillPackageFile]) -> String {
    let mut out = String::new();
    for file in files {
        if let Ok(bytes) = std::fs::read(&file.source_path) {
            out.push_str("\n--- ");
            out.push_str(&file.relative_path);
            out.push_str(" ---\n");
            out.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    out
}

pub fn package_file_json(files: &[SkillPackageFile]) -> Vec<serde_json::Value> {
    files
        .iter()
        .map(|file| {
            serde_json::json!({
                "path": file.relative_path,
                "bytes": file.bytes,
            })
        })
        .collect()
}

pub fn validate_fetch_url(url_str: &str) -> Result<(), ToolError> {
    let parsed = url::Url::parse(url_str)
        .map_err(|e| ToolError::ExecutionFailed(format!("Invalid URL '{}': {}", url_str, e)))?;

    if parsed.scheme() != "https" {
        return Err(ToolError::ExecutionFailed(format!(
            "Only HTTPS URLs are allowed for skill fetching, got scheme '{}'",
            parsed.scheme()
        )));
    }

    let host = parsed
        .host()
        .ok_or_else(|| ToolError::ExecutionFailed("URL has no host".to_string()))?;

    match host {
        url::Host::Domain(host) => {
            let host_lower = host.to_lowercase();
            if host_lower == "localhost"
                || host_lower == "metadata.google.internal"
                || host_lower.ends_with(".internal")
                || host_lower.ends_with(".local")
            {
                return Err(ToolError::ExecutionFailed(format!(
                    "URL points to an internal hostname: {}",
                    host
                )));
            }
        }
        url::Host::Ipv4(ip) => {
            let ip = std::net::IpAddr::V4(ip);
            if ip.is_loopback()
                || ip.is_unspecified()
                || is_private_ip(&ip)
                || is_link_local_ip(&ip)
            {
                return Err(ToolError::ExecutionFailed(format!(
                    "URL points to a private/loopback/link-local address: {}",
                    ip
                )));
            }
        }
        url::Host::Ipv6(ip) => {
            let ip = ip
                .to_ipv4_mapped()
                .map(std::net::IpAddr::V4)
                .unwrap_or(std::net::IpAddr::V6(ip));
            if ip.is_loopback()
                || ip.is_unspecified()
                || is_private_ip(&ip)
                || is_link_local_ip(&ip)
            {
                return Err(ToolError::ExecutionFailed(format!(
                    "URL points to a private/loopback/link-local address: {}",
                    ip
                )));
            }
        }
    }

    Ok(())
}

/// Extract `SKILL.md` from a ZIP archive returned by the skill download API.
///
/// Walks ZIP local file headers looking for an entry named `SKILL.md`.
/// Supports Store (method 0) and Deflate (method 8) compression.
pub fn extract_skill_from_zip(data: &[u8]) -> Result<String, ToolError> {
    use flate2::read::DeflateDecoder;
    use std::io::Read;

    const MAX_DECOMPRESSED: usize = 1_024 * 1_024;

    let mut offset = 0;
    while offset + 30 <= data.len() {
        if data[offset..offset + 4] != [0x50, 0x4B, 0x03, 0x04] {
            break;
        }

        let compression = u16::from_le_bytes([data[offset + 8], data[offset + 9]]);
        let compressed_size = u32::from_le_bytes([
            data[offset + 18],
            data[offset + 19],
            data[offset + 20],
            data[offset + 21],
        ]) as usize;
        let uncompressed_size = u32::from_le_bytes([
            data[offset + 22],
            data[offset + 23],
            data[offset + 24],
            data[offset + 25],
        ]) as usize;
        let name_len = u16::from_le_bytes([data[offset + 26], data[offset + 27]]) as usize;
        let extra_len = u16::from_le_bytes([data[offset + 28], data[offset + 29]]) as usize;

        let name_start = offset + 30;
        let name_end = name_start + name_len;
        if name_end > data.len() {
            break;
        }
        let file_name = std::str::from_utf8(&data[name_start..name_end]).unwrap_or("");

        let data_start = name_end
            .checked_add(extra_len)
            .ok_or_else(|| ToolError::ExecutionFailed("ZIP header offset overflow".to_string()))?;
        let data_end = data_start
            .checked_add(compressed_size)
            .ok_or_else(|| ToolError::ExecutionFailed("ZIP header size overflow".to_string()))?;

        if file_name == "SKILL.md" {
            if data_end > data.len() {
                return Err(ToolError::ExecutionFailed(
                    "ZIP archive truncated".to_string(),
                ));
            }

            if uncompressed_size > MAX_DECOMPRESSED {
                return Err(ToolError::ExecutionFailed(
                    "ZIP entry too large to decompress safely".to_string(),
                ));
            }

            let raw = &data[data_start..data_end];
            let decompressed = match compression {
                0 => raw.to_vec(),
                8 => {
                    let mut decoder = DeflateDecoder::new(raw).take(MAX_DECOMPRESSED as u64);
                    let mut buf = Vec::with_capacity(uncompressed_size.min(MAX_DECOMPRESSED));
                    decoder.read_to_end(&mut buf).map_err(|e| {
                        ToolError::ExecutionFailed(format!("Failed to decompress SKILL.md: {}", e))
                    })?;
                    buf
                }
                other => {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Unsupported ZIP compression method: {}",
                        other
                    )));
                }
            };

            return String::from_utf8(decompressed).map_err(|e| {
                ToolError::ExecutionFailed(format!("SKILL.md in archive is not valid UTF-8: {}", e))
            });
        }

        offset = data_end;
    }

    Err(ToolError::ExecutionFailed(
        "ZIP archive does not contain SKILL.md".to_string(),
    ))
}

fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            let segments = v6.segments();
            (segments[0] & 0xfe00) == 0xfc00
        }
    }
}

fn is_link_local_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            let segments = v6.segments();
            (segments[0] & 0xffc0) == 0xfe80
        }
    }
}

pub fn normalize_tap_path(path: &str) -> String {
    path.trim().trim_matches('/').to_string()
}

pub fn validate_github_repo(repo: &str) -> Result<(), ToolError> {
    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || owner.is_empty()
        || name.is_empty()
        || [owner, name].iter().any(|part| {
            part == &"."
                || part == &".."
                || part
                    .chars()
                    .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.'))
        })
    {
        return Err(ToolError::InvalidParameters(
            "repo must be in owner/name form".to_string(),
        ));
    }
    Ok(())
}

pub fn validate_repo_relative_path(path: &str, field: &str) -> Result<(), ToolError> {
    if path.is_empty() {
        return Ok(());
    }
    let candidate = Path::new(path);
    if candidate.is_absolute()
        || !candidate
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(ToolError::InvalidParameters(format!(
            "{} must be a relative repository path without traversal",
            field
        )));
    }
    Ok(())
}

pub fn validate_repo_path_component(value: &str, field: &str) -> Result<(), ToolError> {
    validate_repo_relative_path(value, field)?;
    if Path::new(value).components().count() != 1 {
        return Err(ToolError::InvalidParameters(format!(
            "{} must be a single repository path component",
            field
        )));
    }
    Ok(())
}
