//! Shadow git checkpoint manager for filesystem rollback.
//!
//! This keeps a per-project shadow repository under `~/.thinclaw/checkpoints/`
//! and records snapshots before file mutations so `/rollback` can restore them.

use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thinclaw_tools::execution::{BoundedProcessOutput, bounded_command_output};
use thinclaw_types::JobContext;
use tokio::process::Command;

const DEFAULT_MAX_CHECKPOINTS: usize = 50;
const GIT_AUTHOR_NAME: &str = "ThinClaw";
const GIT_AUTHOR_EMAIL: &str = "thinclaw@localhost";
const GIT_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_GIT_STDOUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_GIT_STDERR_BYTES: usize = 1024 * 1024;
const MAX_CHECKPOINT_REASON_CHARS: usize = 4096;
const ROOT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointEntry {
    pub commit_hash: String,
    pub timestamp: DateTime<Utc>,
    pub summary: String,
    /// Conversation turn this checkpoint was created during, parsed from the
    /// commit tag. `None` for untagged (older) checkpoints.
    pub turn: Option<usize>,
}

#[derive(Debug)]
pub enum CheckpointError {
    Disabled,
    InvalidCommitHash(String),
    InvalidPath(String),
    Io(std::io::Error),
    Git { command: String, stderr: String },
    Parse(String),
    Join(String),
}

impl Display for CheckpointError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "filesystem checkpoints are disabled"),
            Self::InvalidCommitHash(hash) => write!(f, "invalid commit hash: {hash}"),
            Self::InvalidPath(path) => write!(f, "invalid path: {path}"),
            Self::Io(err) => write!(f, "{err}"),
            Self::Git { command, stderr } => {
                write!(f, "git command failed ({command}): {}", stderr.trim())
            }
            Self::Parse(msg) => write!(f, "{msg}"),
            Self::Join(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for CheckpointError {}

impl From<std::io::Error> for CheckpointError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug)]
pub struct CheckpointManager {
    enabled: bool,
    max_checkpoints: usize,
    shadow_root: PathBuf,
    per_turn_dirs: HashMap<String, HashSet<PathBuf>>,
    thread_roots: HashMap<String, PathBuf>,
    /// Current conversation turn number per scope, so checkpoint commits can be
    /// tagged with the turn they belong to (enables `/rewind <n>` to restore
    /// files to the exact turn's checkpoint).
    turn_for_scope: HashMap<String, usize>,
}

impl Default for CheckpointManager {
    fn default() -> Self {
        let shadow_root = thinclaw_platform::resolve_data_dir("checkpoints");
        Self {
            enabled: false,
            max_checkpoints: DEFAULT_MAX_CHECKPOINTS,
            shadow_root,
            per_turn_dirs: HashMap::new(),
            thread_roots: HashMap::new(),
            turn_for_scope: HashMap::new(),
        }
    }
}

/// Extract the turn number a checkpoint commit was tagged with, if any.
/// Commit subjects created by [`CheckpointManager`] look like
/// `[thinclaw][t3] <reason>`; older/un-tagged commits return `None`.
pub(crate) fn parse_turn_tag(summary: &str) -> Option<usize> {
    let rest = summary.strip_prefix("[thinclaw][t")?;
    let end = rest.find(']')?;
    rest[..end].parse::<usize>().ok()
}

static GLOBAL_MANAGER: OnceLock<Mutex<CheckpointManager>> = OnceLock::new();

fn global_manager() -> &'static Mutex<CheckpointManager> {
    GLOBAL_MANAGER.get_or_init(|| Mutex::new(CheckpointManager::default()))
}

fn canonicalize_or_lexical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                Component::ParentDir => {
                    if components
                        .last()
                        .is_some_and(|c| matches!(c, Component::Normal(_)))
                    {
                        components.pop();
                    }
                }
                Component::CurDir => {}
                other => components.push(other),
            }
        }
        components.iter().collect()
    })
}

fn sanitize_reason(reason: &str) -> String {
    reason
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_CHECKPOINT_REASON_CHARS)
        .collect()
}

fn git_command_label(args: &[&str]) -> String {
    let subcommand = args
        .iter()
        .copied()
        .find(|argument| !argument.starts_with('-'))
        .unwrap_or("command");
    format!("git {subcommand}")
}

fn sanitized_git_stderr(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .map(|character| {
            if character == '\n' || character == '\t' || !character.is_control() {
                character
            } else {
                '�'
            }
        })
        .collect()
}

fn git_output(
    args: &[&str],
    git_dir: Option<&Path>,
    work_tree: Option<&Path>,
    current_dir: Option<&Path>,
) -> Result<BoundedProcessOutput, CheckpointError> {
    let mut command = Command::new("git");
    command.args(args);
    command.env("GIT_AUTHOR_NAME", GIT_AUTHOR_NAME);
    command.env("GIT_AUTHOR_EMAIL", GIT_AUTHOR_EMAIL);
    command.env("GIT_COMMITTER_NAME", GIT_AUTHOR_NAME);
    command.env("GIT_COMMITTER_EMAIL", GIT_AUTHOR_EMAIL);
    command.env("LC_ALL", "C");
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_PAGER", "cat");
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    command.env(
        "GIT_CONFIG_GLOBAL",
        if cfg!(windows) { "NUL" } else { "/dev/null" },
    );
    if let Some(dir) = git_dir {
        command.env("GIT_DIR", dir);
    }
    if let Some(tree) = work_tree {
        command.env("GIT_WORK_TREE", tree);
    }
    if let Some(dir) = current_dir.or(work_tree).or(git_dir) {
        command.current_dir(dir);
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(CheckpointError::Io)?;
    runtime
        .block_on(bounded_command_output(
            &mut command,
            GIT_TIMEOUT,
            MAX_GIT_STDOUT_BYTES,
            MAX_GIT_STDERR_BYTES,
            "checkpoint git command",
        ))
        .map_err(|error| CheckpointError::Git {
            command: git_command_label(args),
            stderr: error.to_string(),
        })
}

fn git_ok(
    args: &[&str],
    git_dir: Option<&Path>,
    work_tree: Option<&Path>,
    current_dir: Option<&Path>,
) -> Result<(), CheckpointError> {
    let command = git_command_label(args);
    let output = git_output(args, git_dir, work_tree, current_dir)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CheckpointError::Git {
            command,
            stderr: sanitized_git_stderr(&output.stderr),
        })
    }
}

fn git_path_list(
    args: &[&str],
    git_dir: Option<&Path>,
    work_tree: Option<&Path>,
    current_dir: Option<&Path>,
) -> Result<Vec<PathBuf>, CheckpointError> {
    let command = git_command_label(args);
    let output = git_output(args, git_dir, work_tree, current_dir)?;
    if output.status.success() {
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(git_path_from_bytes)
            .collect()
    } else {
        Err(CheckpointError::Git {
            command,
            stderr: sanitized_git_stderr(&output.stderr),
        })
    }
}

fn git_path_from_bytes(bytes: &[u8]) -> Result<PathBuf, CheckpointError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes.to_vec())
            .map(PathBuf::from)
            .map_err(|_| CheckpointError::Parse("git returned a non-UTF-8 path".to_string()))
    }
}

fn project_root_from_target(target: &Path, fallback: Option<&Path>) -> PathBuf {
    if let Some(root) = detect_project_root(target) {
        return canonicalize_or_lexical(&root);
    }

    if let Some(fallback) = fallback {
        return canonicalize_or_lexical(fallback);
    }

    if let Some(parent) = target.parent() {
        return canonicalize_or_lexical(parent);
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn thread_scope_from_context(ctx: &JobContext) -> String {
    ctx.metadata
        .get("thread_id")
        .and_then(|v| v.as_str())
        .or_else(|| {
            ctx.metadata
                .get("conversation_scope_id")
                .and_then(|v| v.as_str())
        })
        .map(str::to_string)
        .unwrap_or_else(|| "global".to_string())
}

fn validate_commit_hash(commit_hash: &str) -> Result<(), CheckpointError> {
    if (7..=40).contains(&commit_hash.len())
        && commit_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(CheckpointError::InvalidCommitHash(commit_hash.to_string()))
    }
}

fn validate_relative_path(path: &str) -> Result<PathBuf, CheckpointError> {
    let rel = Path::new(path);
    if rel.is_absolute() {
        return Err(CheckpointError::InvalidPath(path.to_string()));
    }

    let mut normalized = PathBuf::new();
    for component in rel.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            _ => return Err(CheckpointError::InvalidPath(path.to_string())),
        }
    }

    if normalized.as_os_str().is_empty() {
        Err(CheckpointError::InvalidPath(path.to_string()))
    } else {
        Ok(normalized)
    }
}

fn safe_checkpoint_target(root: &Path, relative: &Path) -> Result<PathBuf, CheckpointError> {
    let canonical_root = root.canonicalize()?;
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty() {
        return Err(CheckpointError::InvalidPath(relative.display().to_string()));
    }
    let mut current = canonical_root.clone();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(part) = component else {
            return Err(CheckpointError::InvalidPath(relative.display().to_string()));
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if index + 1 != components.len() {
                    return Err(CheckpointError::InvalidPath(format!(
                        "{} traverses a symbolic-link ancestor",
                        relative.display()
                    )));
                }
            }
            Ok(_) => {
                let canonical = current.canonicalize()?;
                if !canonical.starts_with(&canonical_root) {
                    return Err(CheckpointError::InvalidPath(relative.display().to_string()));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(CheckpointError::Io(error)),
        }
    }
    if !current.starts_with(&canonical_root) {
        return Err(CheckpointError::InvalidPath(relative.display().to_string()));
    }
    Ok(current)
}

fn remove_checkpoint_target(root: &Path, relative: &Path) -> Result<(), CheckpointError> {
    let target = safe_checkpoint_target(root, relative)?;
    let metadata = match std::fs::symlink_metadata(&target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(CheckpointError::Io(error)),
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        std::fs::remove_file(target)?;
    } else if metadata.is_dir() {
        std::fs::remove_dir_all(target)?;
    }
    Ok(())
}

impl CheckpointManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn configure(&mut self, enabled: bool, max_checkpoints: usize) {
        self.enabled = enabled;
        self.max_checkpoints = max_checkpoints.max(1);
        if !self.shadow_root.exists() {
            let _ = std::fs::create_dir_all(&self.shadow_root);
        }
    }

    pub fn new_turn(&mut self, scope: impl Into<String>, turn_number: Option<usize>) {
        let scope = scope.into();
        match turn_number {
            Some(turn) => {
                self.turn_for_scope.insert(scope.clone(), turn);
            }
            // Clear any prior turn tag so a checkpoint committed before the
            // real turn number is set (e.g. `new_turn(scope, None)` early in the
            // turn) is left untagged rather than mislabeled with the previous
            // turn's number.
            None => {
                self.turn_for_scope.remove(&scope);
            }
        }
        self.per_turn_dirs.insert(scope, HashSet::new());
    }

    fn shadow_repo_path(&self, project_dir: &Path) -> PathBuf {
        let canonical = canonicalize_or_lexical(project_dir);
        let mut hasher = Sha256::new();
        hasher.update(canonical.to_string_lossy().as_bytes());
        let digest = hasher.finalize();
        let hash = hex::encode(digest);
        self.shadow_root.join(&hash[..16])
    }

    fn ensure_repo_initialized(
        &self,
        repo_dir: &Path,
        project_dir: &Path,
    ) -> Result<(), CheckpointError> {
        std::fs::create_dir_all(repo_dir)?;
        if repo_dir.join("HEAD").exists() && repo_dir.join("objects").exists() {
            return Ok(());
        }

        git_ok(
            &["init", "--bare", "--template="],
            None,
            None,
            Some(repo_dir),
        )?;

        // A bare repo created in-place should now be ready.
        if !repo_dir.join("HEAD").exists() {
            return Err(CheckpointError::Parse(format!(
                "failed to initialize checkpoint repo for {}",
                project_dir.display()
            )));
        }

        Ok(())
    }

    fn record_thread_root(&mut self, scope: &str, project_root: &Path) {
        self.thread_roots
            .insert(scope.to_string(), canonicalize_or_lexical(project_root));
    }

    fn latest_thread_root(&self, scope: &str) -> Option<PathBuf> {
        self.thread_roots.get(scope).cloned()
    }

    fn list_inner(&self, project_dir: &Path) -> Result<Vec<CheckpointEntry>, CheckpointError> {
        if !self.enabled {
            return Err(CheckpointError::Disabled);
        }

        let root = canonicalize_or_lexical(project_dir);
        let repo_dir = self.shadow_repo_path(&root);
        if !repo_dir.exists() {
            return Ok(Vec::new());
        }

        let limit = self.max_checkpoints.max(1).to_string();
        let output = git_output(
            &[
                "--no-pager",
                "log",
                "--pretty=format:%H\t%ct\t%s",
                "-n",
                &limit,
            ],
            Some(&repo_dir),
            Some(&root),
            Some(&root),
        )?;

        if !output.status.success() {
            let stderr = sanitized_git_stderr(&output.stderr);
            if stderr.contains("does not have any commits yet")
                || stderr.contains("unknown revision")
            {
                return Ok(Vec::new());
            }
            return Err(CheckpointError::Git {
                command: "git log".to_string(),
                stderr,
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut entries = Vec::new();
        for line in stdout.lines() {
            let mut parts = line.splitn(3, '\t');
            let Some(commit_hash) = parts.next() else {
                continue;
            };
            let Some(timestamp) = parts.next() else {
                continue;
            };
            let summary = parts.next().unwrap_or("").to_string();
            let timestamp = timestamp
                .parse::<i64>()
                .map_err(|err| CheckpointError::Parse(err.to_string()))?;
            let timestamp = Utc
                .timestamp_opt(timestamp, 0)
                .single()
                .ok_or_else(|| CheckpointError::Parse("invalid commit timestamp".to_string()))?;
            let turn = parse_turn_tag(&summary);
            entries.push(CheckpointEntry {
                commit_hash: commit_hash.to_string(),
                timestamp,
                summary,
                turn,
            });
        }

        Ok(entries)
    }

    fn create_checkpoint_inner(
        &mut self,
        scope: &str,
        project_dir: &Path,
        reason: &str,
        force: bool,
    ) -> Result<bool, CheckpointError> {
        if !self.enabled {
            return Err(CheckpointError::Disabled);
        }

        let root = canonicalize_or_lexical(project_dir);
        let repo_dir = self.shadow_repo_path(&root);
        self.ensure_repo_initialized(&repo_dir, &root)?;

        {
            let bucket = self.per_turn_dirs.entry(scope.to_string()).or_default();
            if !force && bucket.contains(&root) {
                return Ok(false);
            }

            bucket.insert(root.clone());
        }

        let reason = sanitize_reason(reason);
        // Tag the commit with the current turn so `/rewind <n>` can locate the
        // exact checkpoint for a turn. Untagged form is kept when no turn is
        // known for the scope, so behavior is unchanged for non-conversation
        // callers.
        let commit_message = match self.turn_for_scope.get(scope) {
            Some(turn) => format!("[thinclaw][t{turn}] {reason}"),
            None => format!("[thinclaw] {reason}"),
        };
        let result = (|| {
            git_ok(
                &[
                    "add",
                    "-A",
                    "--",
                    ".",
                    ":(exclude).git",
                    ":(exclude)**/.git",
                ],
                Some(&repo_dir),
                Some(&root),
                Some(&root),
            )?;
            git_ok(
                &["commit", "--allow-empty", "-m", &commit_message],
                Some(&repo_dir),
                Some(&root),
                Some(&root),
            )
        })();
        if let Err(error) = result {
            if let Some(bucket) = self.per_turn_dirs.get_mut(scope) {
                bucket.remove(&root);
            }
            return Err(error);
        }

        self.record_thread_root(scope, &root);
        Ok(true)
    }

    fn restore_inner(
        &mut self,
        scope: &str,
        project_dir: &Path,
        commit_hash: &str,
        file: Option<&str>,
    ) -> Result<(), CheckpointError> {
        if !self.enabled {
            return Err(CheckpointError::Disabled);
        }
        validate_commit_hash(commit_hash)?;

        let root = canonicalize_or_lexical(project_dir);
        let repo_dir = self.shadow_repo_path(&root);
        if !repo_dir.exists() {
            return Ok(());
        }

        let safety_reason = format!("pre-rollback snapshot before restoring {commit_hash}");
        self.create_checkpoint_inner(scope, &root, &safety_reason, true)?;

        if let Some(file) = file {
            let rel = validate_relative_path(file)?;
            let rel_str = rel.to_string_lossy().to_string();
            let _ = safe_checkpoint_target(&root, &rel)?;
            let exists_in_checkpoint =
                self.tracked_path_exists_at_commit(&repo_dir, &root, commit_hash, &rel_str)?;
            if exists_in_checkpoint {
                git_ok(
                    &["checkout", commit_hash, "--", &rel_str],
                    Some(&repo_dir),
                    Some(&root),
                    Some(&root),
                )?;
            } else {
                remove_checkpoint_target(&root, &rel)?;
            }
        } else {
            let current_paths = git_path_list(
                &["ls-files", "-z"],
                Some(&repo_dir),
                Some(&root),
                Some(&root),
            )?;
            let target_paths = git_path_list(
                &["ls-tree", "-r", "--name-only", "-z", commit_hash],
                Some(&repo_dir),
                Some(&root),
                Some(&root),
            )?
            .into_iter()
            .collect::<HashSet<_>>();
            git_ok(
                &[
                    "checkout",
                    commit_hash,
                    "--",
                    ".",
                    ":(exclude).git",
                    ":(exclude)**/.git",
                ],
                Some(&repo_dir),
                Some(&root),
                Some(&root),
            )?;

            // `git checkout <tree> -- .` does not remove paths that exist in
            // the current index but not in the target tree. The safety
            // checkpoint above gives us an exact current-path set, so delete
            // that difference explicitly without ever traversing symlinks.
            for rel in current_paths {
                if !target_paths.contains(&rel) {
                    remove_checkpoint_target(&root, &rel)?;
                }
            }

            let untracked = git_path_list(
                &["ls-files", "--others", "--exclude-standard", "-z"],
                Some(&repo_dir),
                Some(&root),
                Some(&root),
            )?;
            for rel in untracked {
                if rel
                    .components()
                    .any(|component| matches!(component, Component::Normal(part) if part == ".git"))
                {
                    continue;
                }
                remove_checkpoint_target(&root, &rel)?;
            }
        }

        self.record_thread_root(scope, &root);
        Ok(())
    }

    fn diff_inner(&self, project_dir: &Path, commit_hash: &str) -> Result<String, CheckpointError> {
        if !self.enabled {
            return Err(CheckpointError::Disabled);
        }
        validate_commit_hash(commit_hash)?;

        let root = canonicalize_or_lexical(project_dir);
        let repo_dir = self.shadow_repo_path(&root);
        if !repo_dir.exists() {
            return Ok(String::new());
        }

        let output = git_output(
            &[
                "--no-pager",
                "diff",
                "--no-ext-diff",
                "--no-color",
                commit_hash,
                "--",
                ".",
                ":(exclude).git",
                ":(exclude)**/.git",
            ],
            Some(&repo_dir),
            Some(&root),
            Some(&root),
        )?;

        if !output.status.success() {
            return Err(CheckpointError::Git {
                command: "git diff".to_string(),
                stderr: sanitized_git_stderr(&output.stderr),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn tracked_path_exists_at_commit(
        &self,
        repo_dir: &Path,
        project_dir: &Path,
        commit_hash: &str,
        rel_path: &str,
    ) -> Result<bool, CheckpointError> {
        let output = git_output(
            &["ls-tree", "--name-only", "-r", commit_hash, "--", rel_path],
            Some(repo_dir),
            Some(project_dir),
            Some(project_dir),
        )?;
        if !output.status.success() {
            return Err(CheckpointError::Git {
                command: "git ls-tree".to_string(),
                stderr: sanitized_git_stderr(&output.stderr),
            });
        }
        Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
    }
}

pub fn configure(enabled: bool, max_checkpoints: usize) {
    if let Ok(mut guard) = global_manager().lock() {
        guard.configure(enabled, max_checkpoints);
    }
}

pub fn new_turn(scope: impl Into<String>, turn_number: Option<usize>) {
    if let Ok(mut guard) = global_manager().lock() {
        guard.new_turn(scope, turn_number);
    }
}

pub fn resolve_thread_root(scope: &str, fallback: Option<&Path>) -> Option<PathBuf> {
    let guard = global_manager()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .latest_thread_root(scope)
        .or_else(|| fallback.map(canonicalize_or_lexical))
        .or_else(|| std::env::current_dir().ok())
}

pub fn detect_project_root(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_dir() {
        Some(path.to_path_buf())
    } else {
        path.parent().map(Path::to_path_buf)
    }?;

    loop {
        if ROOT_MARKERS
            .iter()
            .any(|marker| current.join(marker).exists())
        {
            return Some(current);
        }
        let Some(parent) = current.parent().map(Path::to_path_buf) else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent;
    }

    None
}

pub async fn ensure_checkpoint(
    ctx: &JobContext,
    target_path: &Path,
    fallback_root: Option<&Path>,
    reason: &str,
) -> Result<bool, CheckpointError> {
    let scope = thread_scope_from_context(ctx);
    let project_root = project_root_from_target(target_path, fallback_root);
    let reason = reason.to_string();
    tokio::task::spawn_blocking(move || {
        let mut guard = global_manager()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.create_checkpoint_inner(&scope, &project_root, &reason, false)
    })
    .await
    .map_err(|err| CheckpointError::Join(err.to_string()))?
}

pub async fn list_checkpoints(project_dir: &Path) -> Result<Vec<CheckpointEntry>, CheckpointError> {
    let project_dir = project_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let guard = global_manager()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.list_inner(&project_dir)
    })
    .await
    .map_err(|err| CheckpointError::Join(err.to_string()))?
}

pub async fn restore(
    project_dir: &Path,
    commit_hash: &str,
    file: Option<&str>,
) -> Result<(), CheckpointError> {
    restore_with_scope("global", project_dir, commit_hash, file).await
}

pub async fn restore_with_scope(
    scope: &str,
    project_dir: &Path,
    commit_hash: &str,
    file: Option<&str>,
) -> Result<(), CheckpointError> {
    let scope = scope.to_string();
    let project_dir = project_dir.to_path_buf();
    let commit_hash = commit_hash.to_string();
    let file = file.map(str::to_string);
    tokio::task::spawn_blocking(move || {
        let mut guard = global_manager()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.restore_inner(&scope, &project_dir, &commit_hash, file.as_deref())
    })
    .await
    .map_err(|err| CheckpointError::Join(err.to_string()))?
}

pub async fn diff(project_dir: &Path, commit_hash: &str) -> Result<String, CheckpointError> {
    let project_dir = project_dir.to_path_buf();
    let commit_hash = commit_hash.to_string();
    tokio::task::spawn_blocking(move || {
        let guard = global_manager()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.diff_inner(&project_dir, &commit_hash)
    })
    .await
    .map_err(|err| CheckpointError::Join(err.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    fn ctx_with_thread(thread_id: &str) -> JobContext {
        let mut ctx = JobContext::with_user("user", "chat", "test");
        if !ctx.metadata.is_object() {
            ctx.metadata = serde_json::json!({});
        }
        if let Some(map) = ctx.metadata.as_object_mut() {
            map.insert("thread_id".to_string(), serde_json::json!(thread_id));
        }
        ctx
    }

    #[test]
    fn parse_turn_tag_reads_tagged_and_untagged_commits() {
        assert_eq!(parse_turn_tag("[thinclaw][t3] edited files"), Some(3));
        assert_eq!(parse_turn_tag("[thinclaw][t0] initial"), Some(0));
        assert_eq!(parse_turn_tag("[thinclaw] legacy untagged"), None);
        assert_eq!(parse_turn_tag("unrelated commit"), None);
        assert_eq!(parse_turn_tag("[thinclaw][tX] bad"), None);
    }

    #[tokio::test]
    async fn checkpoints_round_trip_and_restore() {
        configure(true, 10);
        new_turn("thread-a", Some(0));

        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        let file_path = dir.path().join("src");
        std::fs::create_dir_all(&file_path).unwrap();
        let file_path = file_path.join("main.rs");
        std::fs::write(&file_path, "fn main() {}\n").unwrap();

        let ctx = ctx_with_thread("thread-a");
        let created = ensure_checkpoint(&ctx, &file_path, Some(dir.path()), "pre: write_file")
            .await
            .unwrap();
        assert!(created);

        std::fs::write(&file_path, "fn main() { println!(\"changed\"); }\n").unwrap();

        let entries = list_checkpoints(dir.path()).await.unwrap();
        assert!(!entries.is_empty());
        let hash = entries[0].commit_hash.clone();
        let diff = diff(dir.path(), &hash).await.unwrap();
        assert!(diff.contains("println!"));

        restore_with_scope("thread-a", dir.path(), &hash, Some("src/main.rs"))
            .await
            .unwrap();
        let restored = std::fs::read_to_string(&file_path).unwrap();
        assert!(restored.contains("fn main() {}"));
    }

    #[tokio::test]
    async fn checkpoint_dedup_is_per_turn_and_thread() {
        configure(true, 10);
        new_turn("thread-b", Some(0));

        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        let path = dir.path().join("file.txt");
        std::fs::write(&path, "alpha").unwrap();

        let ctx = ctx_with_thread("thread-b");
        let first = ensure_checkpoint(&ctx, &path, Some(dir.path()), "pre: write_file")
            .await
            .unwrap();
        let second = ensure_checkpoint(&ctx, &path, Some(dir.path()), "pre: write_file")
            .await
            .unwrap();

        assert!(first);
        assert!(!second);
    }

    #[tokio::test]
    async fn whole_restore_removes_paths_absent_from_target_checkpoint() {
        configure(true, 10);
        new_turn("thread-delete", Some(0));

        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        let original = dir.path().join("original.txt");
        std::fs::write(&original, "original").unwrap();
        let ctx = ctx_with_thread("thread-delete");
        ensure_checkpoint(&ctx, &original, Some(dir.path()), "initial")
            .await
            .unwrap();
        let checkpoint = list_checkpoints(dir.path()).await.unwrap()[0]
            .commit_hash
            .clone();

        let later = dir.path().join("later.txt");
        std::fs::write(&later, "must disappear").unwrap();
        restore_with_scope("thread-delete", dir.path(), &checkpoint, None)
            .await
            .unwrap();

        assert_eq!(std::fs::read_to_string(original).unwrap(), "original");
        assert!(!later.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn restore_refuses_symbolic_link_ancestor_escape() {
        use std::os::unix::fs::symlink;

        configure(true, 10);
        new_turn("thread-symlink", Some(0));
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        let original = dir.path().join("original.txt");
        std::fs::write(&original, "original").unwrap();
        let ctx = ctx_with_thread("thread-symlink");
        ensure_checkpoint(&ctx, &original, Some(dir.path()), "initial")
            .await
            .unwrap();
        let checkpoint = list_checkpoints(dir.path()).await.unwrap()[0]
            .commit_hash
            .clone();

        let outside_file = outside.path().join("outside.txt");
        std::fs::write(&outside_file, "keep").unwrap();
        symlink(outside.path(), dir.path().join("escape")).unwrap();
        let error = restore_with_scope(
            "thread-symlink",
            dir.path(),
            &checkpoint,
            Some("escape/outside.txt"),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, CheckpointError::InvalidPath(_)));
        assert_eq!(std::fs::read_to_string(outside_file).unwrap(), "keep");
    }

    #[test]
    fn detect_root_finds_cargo_toml() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        let nested = dir.path().join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(detect_project_root(&nested), Some(dir.path().to_path_buf()));
    }

    #[test]
    fn reject_bad_commit_hash_and_path() {
        assert!(validate_commit_hash("bad-hash").is_err());
        assert!(validate_relative_path("../escape").is_err());
        assert!(validate_relative_path("/absolute").is_err());
    }
}
