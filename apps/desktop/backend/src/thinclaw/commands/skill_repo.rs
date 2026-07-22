//! Staged installation for externally sourced Git skill repositories.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use sha2::{Digest, Sha256};
use thinclaw_core::settings::SkillTapTrustLevel;
use thinclaw_core::skills::quarantine::{
    FindingSeverity, QuarantineManager, QuarantinedSkill, SkillContent, SkillProvenance,
    SkillScanFile, SkillScanReport,
};
use thinclaw_core::skills::registry::SkillRegistry;
use thinclaw_core::skills::{SkillSource, SkillTrust};
use thinclaw_platform::{bounded_command_output, find_executable_in_path, rename_no_replace};
use tokio::process::Command;

const MAX_REPOSITORY_URL_BYTES: usize = 2_048;
const MAX_REPOSITORY_FILES: usize = 2_048;
const MAX_REPOSITORY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PACKAGE_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SKILLS_PER_REPOSITORY: usize = 100;
const MAX_REPOSITORY_PATH_BYTES: usize = 1_024;
const MAX_PATH_COMPONENT_BYTES: usize = 255;
const MAX_GIT_TREE_OUTPUT_BYTES: usize = 3 * 1024 * 1024;
const GIT_STDERR_LIMIT: usize = 256 * 1024;
const GIT_STDOUT_LIMIT: usize = 64 * 1024;
const GIT_CLONE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const APPROVAL_MARKER: &str = "SKILL_REPO_APPROVAL_REQUIRED";

static INSTALL_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[derive(Debug, thiserror::Error)]
pub enum SkillRepoInstallError {
    #[error("Invalid skill repository URL: {0}")]
    InvalidUrl(String),
    #[error("Skill repository install failed: {0}")]
    InvalidRepository(String),
    #[error("Git is required to install a skill repository")]
    GitUnavailable,
    #[error("Git {operation} failed: {detail}")]
    GitFailure {
        operation: &'static str,
        detail: String,
    },
    #[error("{APPROVAL_MARKER}:{digest}:{summary}")]
    ApprovalRequired { digest: String, summary: String },
    #[error("Skill repository install failed: {0}")]
    Io(String),
}

#[derive(Debug)]
pub struct SkillRepoInstallOutcome {
    pub names: Vec<String>,
    pub commit_sha: String,
    pub finding_count: usize,
}

impl SkillRepoInstallOutcome {
    pub fn message(&self) -> String {
        let findings = if self.finding_count == 1 {
            "1 scanner finding".to_string()
        } else {
            format!("{} scanner findings", self.finding_count)
        };
        format!(
            "Installed {} skill(s) from commit {} ({}): {}",
            self.names.len(),
            self.commit_sha.chars().take(12).collect::<String>(),
            findings,
            self.names.join(", ")
        )
    }
}

#[derive(Debug, Clone)]
struct ParsedRepoUrl {
    canonical_url: String,
    source_ref: String,
    repo_name: String,
}

#[derive(Debug, Clone)]
struct GitRuntime {
    executable: PathBuf,
    home: PathBuf,
    global_config: PathBuf,
    hooks_dir: PathBuf,
    working_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct TreeEntry {
    path: String,
    object_id: String,
    bytes: u64,
}

#[derive(Debug)]
struct PackageSnapshot {
    raw_skill: String,
    scan_files: Vec<SkillScanFile>,
    package_digest: String,
    package_paths: Vec<String>,
}

#[derive(Debug)]
struct PreparedSkill {
    name: String,
    stage_root: PathBuf,
    relative_root: Option<String>,
    package_digest: String,
    package_paths: Vec<String>,
    content: SkillContent,
    report: SkillScanReport,
}

fn parse_repository_url(raw: &str) -> Result<ParsedRepoUrl, SkillRepoInstallError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > MAX_REPOSITORY_URL_BYTES {
        return Err(SkillRepoInstallError::InvalidUrl(format!(
            "URL must contain 1..={MAX_REPOSITORY_URL_BYTES} bytes"
        )));
    }
    if raw.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(SkillRepoInstallError::InvalidUrl(
            "control characters are not allowed".to_string(),
        ));
    }

    let parsed = reqwest::Url::parse(raw).map_err(|_| {
        SkillRepoInstallError::InvalidUrl("expected an absolute HTTPS URL".to_string())
    })?;
    if parsed.scheme() != "https" {
        return Err(SkillRepoInstallError::InvalidUrl(
            "only HTTPS repositories are supported".to_string(),
        ));
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(SkillRepoInstallError::InvalidUrl(
            "credentials, custom ports, query strings, and fragments are not allowed".to_string(),
        ));
    }

    let host = parsed
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| SkillRepoInstallError::InvalidUrl("URL has no host".to_string()))?;
    if !matches!(host.as_str(), "github.com" | "gitlab.com" | "codeberg.org") {
        return Err(SkillRepoInstallError::InvalidUrl(
            "host must be github.com, gitlab.com, or codeberg.org".to_string(),
        ));
    }

    let path = parsed.path().trim_matches('/');
    if path.is_empty() || path.contains('%') || path.contains('\\') {
        return Err(SkillRepoInstallError::InvalidUrl(
            "repository path is missing or encoded".to_string(),
        ));
    }
    let mut components = path.split('/').collect::<Vec<_>>();
    if !(2..=20).contains(&components.len()) {
        return Err(SkillRepoInstallError::InvalidUrl(
            "repository path must contain an owner/group and repository name".to_string(),
        ));
    }
    if components.iter().any(|component| {
        component.is_empty()
            || component.len() > 100
            || matches!(*component, "." | "..")
            || !component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    }) {
        return Err(SkillRepoInstallError::InvalidUrl(
            "repository path contains an unsupported component".to_string(),
        ));
    }

    let last = components
        .last_mut()
        .expect("repository component count was validated");
    *last = last.strip_suffix(".git").unwrap_or(last);
    if last.is_empty() || matches!(*last, "." | "..") {
        return Err(SkillRepoInstallError::InvalidUrl(
            "repository name is invalid".to_string(),
        ));
    }

    let repo_name = (*last).to_string();
    let repository_path = components.join("/");
    Ok(ParsedRepoUrl {
        canonical_url: format!("https://{host}/{repository_path}.git"),
        source_ref: format!("{host}/{repository_path}"),
        repo_name,
    })
}

fn validate_approval_digest(digest: Option<&str>) -> Result<(), SkillRepoInstallError> {
    let Some(digest) = digest else {
        return Ok(());
    };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SkillRepoInstallError::InvalidRepository(
            "approval digest must be exactly 64 hexadecimal characters".to_string(),
        ));
    }
    Ok(())
}

fn locate_git() -> Result<PathBuf, SkillRepoInstallError> {
    #[cfg(target_os = "macos")]
    const CANDIDATES: &[&str] = &[
        "/usr/bin/git",
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
    ];
    #[cfg(target_os = "linux")]
    const CANDIDATES: &[&str] = &["/usr/bin/git", "/usr/local/bin/git"];
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    const CANDIDATES: &[&str] = &[];

    CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .or_else(|| find_executable_in_path(if cfg!(windows) { "git.exe" } else { "git" }))
        .and_then(|path| path.canonicalize().ok().or(Some(path)))
        .ok_or(SkillRepoInstallError::GitUnavailable)
}

fn configure_git_command(runtime: &GitRuntime, allow_lazy_fetch: bool) -> Command {
    const PRESERVED_ENV: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "TMPDIR",
        "TMP",
        "TEMP",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "CURL_CA_BUNDLE",
        "DEVELOPER_DIR",
    ];

    let mut command = Command::new(&runtime.executable);
    command.env_clear();
    for key in PRESERVED_ENV {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("HOME", &runtime.home)
        .env("USERPROFILE", &runtime.home)
        .env("XDG_CONFIG_HOME", &runtime.home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &runtime.global_config)
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .current_dir(&runtime.working_dir);
    if !allow_lazy_fetch {
        command.env("GIT_NO_LAZY_FETCH", "1");
    }

    for config in [
        format!("core.hooksPath={}", runtime.hooks_dir.display()),
        "credential.helper=".to_string(),
        "credential.interactive=never".to_string(),
        "protocol.allow=never".to_string(),
        "protocol.https.allow=always".to_string(),
        "protocol.file.allow=never".to_string(),
        "protocol.ext.allow=never".to_string(),
        "protocol.version=2".to_string(),
        "submodule.recurse=false".to_string(),
        "transfer.fsckObjects=true".to_string(),
        "fetch.fsckObjects=true".to_string(),
        "core.protectHFS=true".to_string(),
        "core.protectNTFS=true".to_string(),
        "core.fsmonitor=false".to_string(),
        "filter.lfs.smudge=".to_string(),
        "filter.lfs.required=false".to_string(),
        "http.followRedirects=initial".to_string(),
        "http.lowSpeedLimit=1024".to_string(),
        "http.lowSpeedTime=30".to_string(),
    ] {
        command.arg("-c").arg(config);
    }
    command
}

fn clean_process_detail(bytes: &[u8]) -> String {
    let value = String::from_utf8_lossy(bytes);
    let mut cleaned = String::with_capacity(value.len().min(2_048));
    for character in value.chars() {
        if cleaned.len() >= 2_048 {
            break;
        }
        if character == '\n' || character == '\t' || !character.is_control() {
            cleaned.push(character);
        }
    }
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        "command returned a non-zero status".to_string()
    } else {
        cleaned
    }
}

async fn run_git(
    runtime: &GitRuntime,
    operation: &'static str,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    timeout: Duration,
    stdout_limit: usize,
    allow_lazy_fetch: bool,
) -> Result<Vec<u8>, SkillRepoInstallError> {
    let mut command = configure_git_command(runtime, allow_lazy_fetch);
    command.args(args);
    let output = bounded_command_output(&mut command, timeout, stdout_limit, GIT_STDERR_LIMIT)
        .await
        .map_err(|error| SkillRepoInstallError::GitFailure {
            operation,
            detail: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(SkillRepoInstallError::GitFailure {
            operation,
            detail: clean_process_detail(&output.stderr),
        });
    }
    Ok(output.stdout)
}

fn is_windows_reserved_component(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

fn validate_repository_path(path: &str) -> Result<(), SkillRepoInstallError> {
    if path.is_empty()
        || path.len() > MAX_REPOSITORY_PATH_BYTES
        || path.starts_with('/')
        || path.contains('\\')
    {
        return Err(SkillRepoInstallError::InvalidRepository(
            "repository contains an unsafe or oversized path".to_string(),
        ));
    }

    for component in path.split('/') {
        if component.is_empty()
            || component.len() > MAX_PATH_COMPONENT_BYTES
            || matches!(component, "." | "..")
            || component.eq_ignore_ascii_case(".git")
            || component.eq_ignore_ascii_case(".thinclaw-skill-lock.json")
            || component.ends_with(' ')
            || component.ends_with('.')
            || is_windows_reserved_component(component)
            || component.bytes().any(|byte| {
                byte.is_ascii_control()
                    || matches!(byte, b':' | b'*' | b'?' | b'"' | b'<' | b'>' | b'|')
            })
        {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "repository contains an unsupported path component in '{path}'"
            )));
        }
    }
    Ok(())
}

fn parse_git_tree(output: &[u8]) -> Result<Vec<TreeEntry>, SkillRepoInstallError> {
    let mut entries = Vec::new();
    let mut folded_paths = HashSet::new();

    for record in output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        if entries.len() >= MAX_REPOSITORY_FILES {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "repository exceeds the {MAX_REPOSITORY_FILES}-file limit"
            )));
        }
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| {
                SkillRepoInstallError::InvalidRepository(
                    "Git returned a malformed tree".to_string(),
                )
            })?;
        let header = std::str::from_utf8(&record[..tab]).map_err(|_| {
            SkillRepoInstallError::InvalidRepository("Git returned a malformed tree".to_string())
        })?;
        let path = std::str::from_utf8(&record[tab + 1..]).map_err(|_| {
            SkillRepoInstallError::InvalidRepository(
                "repository paths must be valid UTF-8".to_string(),
            )
        })?;
        let fields = header.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 || !matches!(fields[0], "100644" | "100755") || fields[1] != "blob" {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "repository contains a symlink, submodule, or unsupported entry at '{path}'"
            )));
        }
        let object_id = fields[2].to_ascii_lowercase();
        if !matches!(object_id.len(), 40 | 64)
            || !object_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "repository contains an invalid object identifier at '{path}'"
            )));
        }
        validate_repository_path(path)?;
        if !folded_paths.insert(path.to_ascii_lowercase()) {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "repository contains case-colliding paths at '{path}'"
            )));
        }
        entries.push(TreeEntry {
            path: path.to_string(),
            object_id,
            bytes: 0,
        });
    }

    if entries.is_empty() {
        return Err(SkillRepoInstallError::InvalidRepository(
            "repository has no files".to_string(),
        ));
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn attach_local_blob_sizes(
    entries: &mut [TreeEntry],
    output: &[u8],
) -> Result<(), SkillRepoInstallError> {
    let mut blob_sizes = HashMap::new();
    for line in output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let line = std::str::from_utf8(line).map_err(|_| {
            SkillRepoInstallError::InvalidRepository(
                "Git returned malformed local object metadata".to_string(),
            )
        })?;
        let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(SkillRepoInstallError::InvalidRepository(
                "Git returned malformed local object metadata".to_string(),
            ));
        }
        if fields[1] != "blob" {
            continue;
        }
        let object_id = fields[0].to_ascii_lowercase();
        if !matches!(object_id.len(), 40 | 64)
            || !object_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(SkillRepoInstallError::InvalidRepository(
                "Git returned an invalid local object identifier".to_string(),
            ));
        }
        let bytes = fields[2].parse::<u64>().map_err(|_| {
            SkillRepoInstallError::InvalidRepository(
                "Git returned an invalid local blob size".to_string(),
            )
        })?;
        if let Some(previous) = blob_sizes.insert(object_id, bytes) {
            if previous != bytes {
                return Err(SkillRepoInstallError::InvalidRepository(
                    "Git reported inconsistent blob sizes".to_string(),
                ));
            }
        }
    }

    let mut total_bytes = 0_u64;
    for entry in entries {
        let bytes = blob_sizes.get(&entry.object_id).copied().ok_or_else(|| {
            // A blob omitted by the size-limited partial clone is necessarily
            // larger than our per-file ceiling. Never lazily fetch it merely
            // to discover its size.
            SkillRepoInstallError::InvalidRepository(format!(
                "repository contains an unavailable or oversized blob at '{}'",
                entry.path
            ))
        })?;
        if bytes > MAX_PACKAGE_FILE_BYTES {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "file '{}' exceeds the {MAX_PACKAGE_FILE_BYTES}-byte limit",
                entry.path
            )));
        }
        if entry
            .path
            .rsplit('/')
            .next()
            .is_some_and(|name| name == "SKILL.md")
            && bytes > thinclaw_core::skills::MAX_PROMPT_FILE_SIZE
        {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "SKILL.md at '{}' exceeds the {}-byte prompt limit",
                entry.path,
                thinclaw_core::skills::MAX_PROMPT_FILE_SIZE
            )));
        }
        total_bytes = total_bytes.checked_add(bytes).ok_or_else(|| {
            SkillRepoInstallError::InvalidRepository("repository size overflow".to_string())
        })?;
        if total_bytes > MAX_REPOSITORY_BYTES {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "repository exceeds the {MAX_REPOSITORY_BYTES}-byte extracted-size limit"
            )));
        }
        entry.bytes = bytes;
    }
    Ok(())
}

fn skill_roots(entries: &[TreeEntry]) -> Result<Vec<PathBuf>, SkillRepoInstallError> {
    let mut roots = entries
        .iter()
        .filter_map(|entry| {
            let path = Path::new(&entry.path);
            (path.file_name().and_then(OsStr::to_str) == Some("SKILL.md"))
                .then(|| path.parent().unwrap_or_else(|| Path::new("")).to_path_buf())
        })
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        return Err(SkillRepoInstallError::InvalidRepository(
            "repository does not contain a SKILL.md".to_string(),
        ));
    }
    if roots.len() > MAX_SKILLS_PER_REPOSITORY {
        return Err(SkillRepoInstallError::InvalidRepository(format!(
            "repository exceeds the {MAX_SKILLS_PER_REPOSITORY}-skill limit"
        )));
    }
    for (index, root) in roots.iter().enumerate() {
        if roots
            .iter()
            .enumerate()
            .any(|(other_index, other)| index != other_index && root.starts_with(other))
        {
            return Err(SkillRepoInstallError::InvalidRepository(
                "nested skill packages are ambiguous; each SKILL.md must have a disjoint package root"
                    .to_string(),
            ));
        }
    }
    Ok(roots)
}

fn validate_checked_out_tree(
    checkout: &Path,
    expected: &[TreeEntry],
) -> Result<(), SkillRepoInstallError> {
    let expected = expected
        .iter()
        .map(|entry| (entry.path.as_str(), entry.bytes))
        .collect::<HashMap<_, _>>();
    let mut observed = HashSet::new();
    let mut stack = vec![checkout.to_path_buf()];

    while let Some(directory) = stack.pop() {
        let entries = std::fs::read_dir(&directory).map_err(|error| {
            SkillRepoInstallError::Io(format!(
                "failed to read staged directory '{}': {error}",
                directory.display()
            ))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
            let path = entry.path();
            let relative = path.strip_prefix(checkout).map_err(|_| {
                SkillRepoInstallError::InvalidRepository(
                    "staged file escaped the checkout root".to_string(),
                )
            })?;
            let relative = relative.to_str().ok_or_else(|| {
                SkillRepoInstallError::InvalidRepository(
                    "checked-out paths must be valid UTF-8".to_string(),
                )
            })?;
            validate_repository_path(&relative.replace(std::path::MAIN_SEPARATOR, "/"))?;
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
            if metadata.file_type().is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
                return Err(SkillRepoInstallError::InvalidRepository(format!(
                    "checkout produced a symlink or special file at '{relative}'"
                )));
            }
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            let normalized = relative.replace(std::path::MAIN_SEPARATOR, "/");
            let expected_bytes = expected.get(normalized.as_str()).ok_or_else(|| {
                SkillRepoInstallError::InvalidRepository(format!(
                    "checkout produced an unexpected file at '{normalized}'"
                ))
            })?;
            if metadata.len() != *expected_bytes {
                return Err(SkillRepoInstallError::InvalidRepository(format!(
                    "checked-out size for '{normalized}' differs from the validated Git tree"
                )));
            }
            observed.insert(normalized);
        }
    }

    if observed.len() != expected.len() {
        return Err(SkillRepoInstallError::InvalidRepository(
            "checkout is missing one or more validated files".to_string(),
        ));
    }
    Ok(())
}

fn snapshot_package(
    checkout: &Path,
    root: &Path,
    entries: &[TreeEntry],
) -> Result<PackageSnapshot, SkillRepoInstallError> {
    let mut hasher = Sha256::new();
    let mut scan_files = Vec::new();
    let mut package_paths = Vec::new();
    let mut raw_skill = None;

    for entry in entries {
        let repository_path = Path::new(&entry.path);
        let package_path = if root.as_os_str().is_empty() {
            repository_path
        } else if let Ok(relative) = repository_path.strip_prefix(root) {
            relative
        } else {
            continue;
        };
        let relative = package_path
            .to_str()
            .expect("Git tree paths were validated as UTF-8")
            .replace(std::path::MAIN_SEPARATOR, "/");
        let source = checkout.join(repository_path);
        let metadata = std::fs::symlink_metadata(&source)
            .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != entry.bytes
        {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "package file '{relative}' changed after validation"
            )));
        }
        let bytes = thinclaw_platform::read_regular_file_bounded_single_link(&source, entry.bytes)
            .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
        if bytes.len() as u64 != entry.bytes {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "package file '{relative}' changed while it was read"
            )));
        }
        hasher.update(relative.as_bytes());
        hasher.update(b"\0");
        hasher.update(&bytes);
        hasher.update(b"\0");
        if relative == "SKILL.md" {
            raw_skill = Some(String::from_utf8(bytes.clone()).map_err(|_| {
                SkillRepoInstallError::InvalidRepository(
                    "SKILL.md must contain valid UTF-8".to_string(),
                )
            })?);
        }
        scan_files.push(SkillScanFile {
            relative_path: relative.clone(),
            content: String::from_utf8_lossy(&bytes).into_owned(),
        });
        package_paths.push(relative);
    }

    let raw_skill = raw_skill.ok_or_else(|| {
        SkillRepoInstallError::InvalidRepository("skill package is missing SKILL.md".to_string())
    })?;
    Ok(PackageSnapshot {
        raw_skill,
        scan_files,
        package_digest: format!("sha256:{}", hex::encode(hasher.finalize())),
        package_paths,
    })
}

fn repository_approval_digest(
    source_ref: &str,
    commit_sha: &str,
    skills: &[PreparedSkill],
) -> String {
    let mut rows = skills
        .iter()
        .map(|skill| {
            format!(
                "{}\0{}\0{}",
                skill.name,
                skill.relative_root.as_deref().unwrap_or("."),
                skill.package_digest
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"thinclaw-skill-repository-approval-v1\0");
    hasher.update(source_ref.as_bytes());
    hasher.update(b"\0");
    hasher.update(commit_sha.as_bytes());
    hasher.update(b"\0");
    for row in rows {
        hasher.update(row.as_bytes());
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

fn approval_summary(skills: &[PreparedSkill]) -> String {
    let critical = skills
        .iter()
        .map(|skill| skill.report.summary.critical)
        .sum::<usize>();
    let warnings = skills
        .iter()
        .map(|skill| skill.report.summary.warnings)
        .sum::<usize>();
    let mut categories = skills
        .iter()
        .flat_map(|skill| skill.report.summary.categories.iter().cloned())
        .collect::<Vec<_>>();
    categories.sort();
    categories.dedup();
    format!(
        "Scanner found {critical} critical and {warnings} warning finding(s) across {} skill(s){}",
        skills.len(),
        if categories.is_empty() {
            String::new()
        } else {
            format!(" [{}]", categories.join(", "))
        }
    )
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), SkillRepoInstallError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| SkillRepoInstallError::Io(error.to_string()))
}

fn normalize_package_permissions(root: &Path) -> Result<(), SkillRepoInstallError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut stack = vec![root.to_path_buf()];
        while let Some(path) = stack.pop() {
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
            if metadata.file_type().is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
                return Err(SkillRepoInstallError::InvalidRepository(format!(
                    "package contains an unsupported entry at '{}'",
                    path.display()
                )));
            }
            let mode = if metadata.is_dir() { 0o700 } else { 0o600 };
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
            if metadata.is_dir() {
                for entry in std::fs::read_dir(&path)
                    .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?
                {
                    stack.push(
                        entry
                            .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?
                            .path(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn ensure_install_root(path: &Path) -> Result<PathBuf, SkillRepoInstallError> {
    std::fs::create_dir_all(path).map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SkillRepoInstallError::InvalidRepository(
            "configured installed-skills root must be a real directory".to_string(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
    }
    path.canonicalize()
        .map_err(|error| SkillRepoInstallError::Io(error.to_string()))
}

fn ensure_destinations_available(
    install_root: &Path,
    names: &[String],
) -> Result<(), SkillRepoInstallError> {
    let wanted = names
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for entry in std::fs::read_dir(install_root)
        .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?
    {
        let entry = entry.map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
        if wanted.contains(&entry.file_name().to_string_lossy().to_ascii_lowercase()) {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "destination '{}' already exists",
                entry.file_name().to_string_lossy()
            )));
        }
    }
    Ok(())
}

async fn rollback_installed_paths(paths: &[PathBuf]) {
    for path in paths.iter().rev() {
        match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                let _ = tokio::fs::remove_dir_all(path).await;
            }
            Ok(_) => {
                let _ = tokio::fs::remove_file(path).await;
            }
            Err(_) => {}
        }
    }
}

pub async fn install_skill_repository(
    registry: &Arc<tokio::sync::RwLock<SkillRegistry>>,
    repo_url: &str,
    approved_digest: Option<&str>,
) -> Result<SkillRepoInstallOutcome, SkillRepoInstallError> {
    validate_approval_digest(approved_digest)?;
    let parsed = parse_repository_url(repo_url)?;
    let _install_guard = INSTALL_LOCK.lock().await;

    let configured_install_root = {
        let guard = registry.read().await;
        guard.install_target_dir().to_path_buf()
    };
    let install_root =
        tokio::task::spawn_blocking(move || ensure_install_root(&configured_install_root))
            .await
            .map_err(|error| SkillRepoInstallError::Io(error.to_string()))??;

    let staging = tempfile::Builder::new()
        .prefix(".thinclaw-repo-install-")
        .tempdir_in(&install_root)
        .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
    let git_home = staging.path().join("git-home");
    let hooks_dir = staging.path().join("empty-hooks");
    std::fs::create_dir(&git_home)
        .and_then(|_| std::fs::create_dir(&hooks_dir))
        .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
    let global_config = git_home.join("config");
    write_private_file(&global_config, b"")?;
    let checkout = staging.path().join("checkout");
    let git = GitRuntime {
        executable: locate_git()?,
        home: git_home,
        global_config,
        hooks_dir,
        working_dir: install_root.clone(),
    };

    tracing::info!(
        repository = %parsed.source_ref,
        target = %parsed.repo_name,
        "Staging external skill repository"
    );
    let clone_args = vec![
        OsString::from("clone"),
        OsString::from("--quiet"),
        OsString::from("--no-checkout"),
        OsString::from("--depth=1"),
        OsString::from("--single-branch"),
        OsString::from("--no-tags"),
        OsString::from(format!(
            "--filter=blob:limit={}",
            MAX_PACKAGE_FILE_BYTES + 1
        )),
        OsString::from("--"),
        OsString::from(&parsed.canonical_url),
        checkout.as_os_str().to_os_string(),
    ];
    run_git(
        &git,
        "clone",
        &clone_args,
        GIT_CLONE_TIMEOUT,
        GIT_STDOUT_LIMIT,
        true,
    )
    .await?;

    let promisor_args = [
        OsString::from("-C"),
        checkout.as_os_str().to_os_string(),
        OsString::from("config"),
        OsString::from("--local"),
        OsString::from("--get"),
        OsString::from("remote.origin.promisor"),
    ];
    let promisor = run_git(
        &git,
        "partial-clone verification",
        &promisor_args,
        GIT_COMMAND_TIMEOUT,
        64,
        false,
    )
    .await?;
    if !String::from_utf8_lossy(&promisor)
        .trim()
        .eq_ignore_ascii_case("true")
    {
        return Err(SkillRepoInstallError::InvalidRepository(
            "remote did not honor the required size-limited partial clone".to_string(),
        ));
    }

    let rev_args = [
        OsString::from("-C"),
        checkout.as_os_str().to_os_string(),
        OsString::from("rev-parse"),
        OsString::from("--verify"),
        OsString::from("HEAD^{commit}"),
    ];
    let commit_output = run_git(
        &git,
        "commit resolution",
        &rev_args,
        GIT_COMMAND_TIMEOUT,
        128,
        false,
    )
    .await?;
    let commit_sha = String::from_utf8_lossy(&commit_output)
        .trim()
        .to_ascii_lowercase();
    if !matches!(commit_sha.len(), 40 | 64)
        || !commit_sha.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(SkillRepoInstallError::InvalidRepository(
            "Git returned an invalid commit identifier".to_string(),
        ));
    }

    let tree_args = [
        OsString::from("-C"),
        checkout.as_os_str().to_os_string(),
        OsString::from("ls-tree"),
        OsString::from("-r"),
        OsString::from("-z"),
        OsString::from("--full-tree"),
        OsString::from("HEAD"),
    ];
    let tree_output = run_git(
        &git,
        "tree validation",
        &tree_args,
        GIT_COMMAND_TIMEOUT,
        MAX_GIT_TREE_OUTPUT_BYTES,
        false,
    )
    .await?;
    let mut tree = parse_git_tree(&tree_output)?;
    let object_args = [
        OsString::from("-C"),
        checkout.as_os_str().to_os_string(),
        OsString::from("cat-file"),
        OsString::from("--batch-all-objects"),
        OsString::from("--batch-check=%(objectname) %(objecttype) %(objectsize)"),
        OsString::from("--unordered"),
    ];
    let object_output = run_git(
        &git,
        "local object validation",
        &object_args,
        GIT_COMMAND_TIMEOUT,
        MAX_GIT_TREE_OUTPUT_BYTES,
        false,
    )
    .await?;
    attach_local_blob_sizes(&mut tree, &object_output)?;
    let roots = skill_roots(&tree)?;

    let checkout_args = [
        OsString::from("-C"),
        checkout.as_os_str().to_os_string(),
        OsString::from("checkout"),
        OsString::from("--quiet"),
        OsString::from("--detach"),
        OsString::from("--force"),
        OsString::from("HEAD"),
    ];
    run_git(
        &git,
        "checkout",
        &checkout_args,
        GIT_COMMAND_TIMEOUT,
        GIT_STDOUT_LIMIT,
        false,
    )
    .await?;

    let git_dir = checkout.join(".git");
    let git_metadata = std::fs::symlink_metadata(&git_dir)
        .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
    if !git_metadata.is_dir() || git_metadata.file_type().is_symlink() {
        return Err(SkillRepoInstallError::InvalidRepository(
            "staged Git metadata is not a real directory".to_string(),
        ));
    }
    tokio::fs::remove_dir_all(&git_dir)
        .await
        .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;

    let checkout_for_validation = checkout.clone();
    let tree_for_validation = tree.clone();
    tokio::task::spawn_blocking(move || {
        validate_checked_out_tree(&checkout_for_validation, &tree_for_validation)
    })
    .await
    .map_err(|error| SkillRepoInstallError::Io(error.to_string()))??;

    let mut prepared = Vec::with_capacity(roots.len());
    for root in roots {
        let stage_root = checkout.join(&root);
        let checkout_for_snapshot = checkout.clone();
        let root_for_snapshot = root.clone();
        let tree_for_snapshot = tree.clone();
        let snapshot = tokio::task::spawn_blocking(move || {
            snapshot_package(
                &checkout_for_snapshot,
                &root_for_snapshot,
                &tree_for_snapshot,
            )
        })
        .await
        .map_err(|error| SkillRepoInstallError::Io(error.to_string()))??;

        let (name, _) = SkillRegistry::validate_skill_file(
            &stage_root,
            SkillTrust::Installed,
            SkillSource::External(stage_root.clone()),
        )
        .await
        .map_err(|error| SkillRepoInstallError::InvalidRepository(error.to_string()))?;
        let relative_root = (!root.as_os_str().is_empty()).then(|| {
            root.to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/")
        });
        let content = SkillContent {
            raw_content: snapshot.raw_skill,
            source_kind: "git".to_string(),
            source_adapter: "desktop_git".to_string(),
            source_ref: parsed.source_ref.clone(),
            source_repo: Some(parsed.source_ref.clone()),
            source_url: Some(parsed.canonical_url.clone()),
            manifest_url: None,
            manifest_digest: Some(snapshot.package_digest.clone()),
            path: relative_root.clone(),
            branch: None,
            commit_sha: Some(commit_sha.clone()),
            trust_level: SkillTapTrustLevel::Community,
        };
        let quarantined = QuarantinedSkill {
            skill_name: name.clone(),
            dir: stage_root.clone(),
            content: content.clone(),
            package_files: snapshot.scan_files,
        };
        let report = tokio::task::spawn_blocking(move || {
            QuarantineManager::new(PathBuf::new()).scan_report(&quarantined)
        })
        .await
        .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
        if report.findings.iter().any(|finding| {
            finding.severity == FindingSeverity::Critical && finding.kind == "path_traversal"
        }) {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "skill '{name}' was rejected for an unsafe package path"
            )));
        }
        prepared.push(PreparedSkill {
            name,
            stage_root,
            relative_root,
            package_digest: snapshot.package_digest,
            package_paths: snapshot.package_paths,
            content,
            report,
        });
    }

    prepared.sort_by(|left, right| left.name.cmp(&right.name));
    let mut unique_names = HashSet::new();
    for skill in &prepared {
        if !unique_names.insert(skill.name.to_ascii_lowercase()) {
            return Err(SkillRepoInstallError::InvalidRepository(format!(
                "repository contains duplicate skill name '{}'",
                skill.name
            )));
        }
    }
    let names = prepared
        .iter()
        .map(|skill| skill.name.clone())
        .collect::<Vec<_>>();
    {
        let guard = registry.read().await;
        for name in &names {
            if guard
                .skills()
                .iter()
                .any(|skill| skill.manifest.name.eq_ignore_ascii_case(name))
            {
                return Err(SkillRepoInstallError::InvalidRepository(format!(
                    "skill '{name}' is already installed"
                )));
            }
        }
    }
    ensure_destinations_available(&install_root, &names)?;

    let total_critical = prepared
        .iter()
        .map(|skill| skill.report.summary.critical)
        .sum::<usize>();
    let total_warnings = prepared
        .iter()
        .map(|skill| skill.report.summary.warnings)
        .sum::<usize>();
    let approval_digest = repository_approval_digest(&parsed.source_ref, &commit_sha, &prepared);
    if thinclaw_tools::builtin::skill::skill_findings_require_approval_by_counts(
        "community",
        total_critical,
        total_warnings,
    ) && approved_digest.map(str::to_ascii_lowercase).as_deref()
        != Some(approval_digest.as_str())
    {
        return Err(SkillRepoInstallError::ApprovalRequired {
            digest: approval_digest,
            summary: approval_summary(&prepared),
        });
    }

    let downloaded_at = chrono::Utc::now().to_rfc3339();
    for skill in &prepared {
        let provenance = SkillProvenance {
            source_kind: skill.content.source_kind.clone(),
            source_adapter: skill.content.source_adapter.clone(),
            source_ref: skill.content.source_ref.clone(),
            source_repo: skill.content.source_repo.clone(),
            source_url: skill.content.source_url.clone(),
            manifest_url: skill.content.manifest_url.clone(),
            manifest_digest: skill.content.manifest_digest.clone(),
            path: skill.content.path.clone(),
            branch: skill.content.branch.clone(),
            commit_sha: skill.content.commit_sha.clone(),
            trust_level: skill.content.trust_level,
            downloaded_at: downloaded_at.clone(),
            findings: skill.report.findings.clone(),
            scanner_version: Some(skill.report.scanner_version.clone()),
            content_sha256: Some(skill.package_digest.clone()),
            finding_summary: Some(skill.report.summary.clone()),
            package_files: skill.package_paths.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&provenance)
            .map_err(|error| SkillRepoInstallError::Io(error.to_string()))?;
        write_private_file(&skill.stage_root.join(".thinclaw-skill-lock.json"), &bytes)?;
        normalize_package_permissions(&skill.stage_root)?;
    }

    let mut installed_paths = Vec::with_capacity(prepared.len());
    for skill in &prepared {
        let destination = install_root.join(&skill.name);
        if let Err(error) = rename_no_replace(&skill.stage_root, &destination) {
            rollback_installed_paths(&installed_paths).await;
            return Err(SkillRepoInstallError::Io(format!(
                "could not atomically install '{}': {error}",
                skill.name
            )));
        }
        installed_paths.push(destination);
    }

    let mut loaded = Vec::with_capacity(prepared.len());
    for (skill, path) in prepared.iter().zip(&installed_paths) {
        match SkillRegistry::load_skill_from_path(
            path,
            SkillTrust::Installed,
            SkillSource::User(path.clone()),
        )
        .await
        {
            Ok((name, loaded_skill)) if name == skill.name => loaded.push((name, loaded_skill)),
            Ok((name, _)) => {
                rollback_installed_paths(&installed_paths).await;
                return Err(SkillRepoInstallError::InvalidRepository(format!(
                    "skill name changed from '{}' to '{name}' after installation",
                    skill.name
                )));
            }
            Err(error) => {
                rollback_installed_paths(&installed_paths).await;
                return Err(SkillRepoInstallError::InvalidRepository(error.to_string()));
            }
        }
    }

    let mut committed = Vec::new();
    let commit_error = {
        let mut guard = registry.write().await;
        let duplicate = loaded.iter().find(|(name, _)| {
            guard
                .skills()
                .iter()
                .any(|skill| skill.manifest.name.eq_ignore_ascii_case(name))
        });
        if let Some((name, _)) = duplicate {
            Some(format!("skill '{name}' was installed concurrently"))
        } else {
            let mut error = None;
            for (name, loaded_skill) in loaded {
                match guard.commit_install(&name, loaded_skill) {
                    Ok(()) => committed.push(name),
                    Err(commit_failure) => {
                        error = Some(commit_failure.to_string());
                        break;
                    }
                }
            }
            if error.is_some() {
                for name in committed.iter().rev() {
                    let _ = guard.commit_remove(name);
                }
            }
            error
        }
    };
    if let Some(error) = commit_error {
        rollback_installed_paths(&installed_paths).await;
        return Err(SkillRepoInstallError::InvalidRepository(error));
    }

    let finding_count = prepared
        .iter()
        .map(|skill| skill.report.summary.total)
        .sum();
    tracing::info!(
        repository = %parsed.source_ref,
        commit = %commit_sha,
        skills = names.len(),
        "Installed external skill repository"
    );
    Ok(SkillRepoInstallOutcome {
        names,
        commit_sha,
        finding_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_url_rejects_credentials_traversal_and_untrusted_hosts() {
        for value in [
            "https://user:secret@github.com/acme/skill.git",
            "https://github.com/acme/%2e%2e.git",
            "https://github.com/acme/../skill.git",
            "file:///tmp/skill",
            "https://example.com/acme/skill.git",
            "https://github.com/acme/skill.git?token=secret",
        ] {
            assert!(parse_repository_url(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn repository_url_is_canonicalized_without_secrets() {
        let parsed = parse_repository_url(" https://GitHub.com/acme/skill ").unwrap();
        assert_eq!(parsed.canonical_url, "https://github.com/acme/skill.git");
        assert_eq!(parsed.source_ref, "github.com/acme/skill");
        assert_eq!(parsed.repo_name, "skill");
    }

    #[test]
    fn tree_parser_rejects_symlinks_oversized_files_and_case_collisions() {
        let symlink = b"120000 blob 0123456789012345678901234567890123456789\tlink\0";
        assert!(parse_git_tree(symlink).is_err());

        let mut oversized =
            parse_git_tree(b"100644 blob 0123456789012345678901234567890123456789\tlarge.bin\0")
                .unwrap();
        let object_metadata = format!(
            "0123456789012345678901234567890123456789 blob {}\n",
            MAX_PACKAGE_FILE_BYTES + 1
        );
        assert!(attach_local_blob_sizes(&mut oversized, object_metadata.as_bytes()).is_err());

        let collision = b"100644 blob 0123456789012345678901234567890123456789\tA.txt\0\
100644 blob 1123456789012345678901234567890123456789\ta.txt\0";
        assert!(parse_git_tree(collision).is_err());
    }

    #[test]
    fn missing_partial_clone_blob_is_rejected_without_fetching() {
        let mut entries =
            parse_git_tree(b"100644 blob 0123456789012345678901234567890123456789\tSKILL.md\0")
                .unwrap();
        assert!(attach_local_blob_sizes(&mut entries, b"").is_err());
    }

    #[test]
    fn nested_skill_roots_are_rejected() {
        let entries = vec![
            TreeEntry {
                path: "SKILL.md".to_string(),
                object_id: "0123456789012345678901234567890123456789".to_string(),
                bytes: 1,
            },
            TreeEntry {
                path: "nested/SKILL.md".to_string(),
                object_id: "1123456789012345678901234567890123456789".to_string(),
                bytes: 1,
            },
        ];
        assert!(skill_roots(&entries).is_err());
    }

    #[test]
    fn approval_digest_is_content_bound_and_stable() {
        fn prepared(name: &str, digest: &str) -> PreparedSkill {
            PreparedSkill {
                name: name.to_string(),
                stage_root: PathBuf::new(),
                relative_root: None,
                package_digest: digest.to_string(),
                package_paths: vec!["SKILL.md".to_string()],
                content: SkillContent {
                    raw_content: String::new(),
                    source_kind: "git".to_string(),
                    source_adapter: "desktop_git".to_string(),
                    source_ref: "github.com/acme/repo".to_string(),
                    source_repo: None,
                    source_url: None,
                    manifest_url: None,
                    manifest_digest: None,
                    path: None,
                    branch: None,
                    commit_sha: None,
                    trust_level: SkillTapTrustLevel::Community,
                },
                report: SkillScanReport {
                    scanner_version: "test".to_string(),
                    content_sha256: String::new(),
                    summary: Default::default(),
                    findings: Vec::new(),
                },
            }
        }

        let first = prepared("a", "sha256:first");
        let second = prepared("a", "sha256:second");
        let digest = repository_approval_digest("github.com/acme/repo", "abc", &[first]);
        assert_eq!(digest.len(), 64);
        assert_ne!(
            digest,
            repository_approval_digest("github.com/acme/repo", "abc", &[second])
        );
    }
}
