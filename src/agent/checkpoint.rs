//! Shadow git checkpoint manager for filesystem rollback.
//!
//! This keeps a per-project shadow repository under `~/.thinclaw/checkpoints/`
//! and records snapshots before file mutations so `/rollback` can restore them.

use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, OnceLock};

use crate::context::JobContext;
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DEFAULT_MAX_CHECKPOINTS: usize = 50;
const GIT_AUTHOR_NAME: &str = "ThinClaw";
const GIT_AUTHOR_EMAIL: &str = "thinclaw@localhost";
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
}

impl Default for CheckpointManager {
    fn default() -> Self {
        let shadow_root = dirs::home_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .join(".thinclaw")
            .join("checkpoints");
        Self {
            enabled: false,
            max_checkpoints: DEFAULT_MAX_CHECKPOINTS,
            shadow_root,
            per_turn_dirs: HashMap::new(),
            thread_roots: HashMap::new(),
        }
    }
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
}

fn git_output(
    args: &[&str],
    git_dir: Option<&Path>,
    work_tree: Option<&Path>,
    current_dir: Option<&Path>,
) -> Result<Output, CheckpointError> {
    let mut command = Command::new("git");
    command.args(args);
    command.env("GIT_AUTHOR_NAME", GIT_AUTHOR_NAME);
    command.env("GIT_AUTHOR_EMAIL", GIT_AUTHOR_EMAIL);
    command.env("GIT_COMMITTER_NAME", GIT_AUTHOR_NAME);
    command.env("GIT_COMMITTER_EMAIL", GIT_AUTHOR_EMAIL);
    command.env("LC_ALL", "C");
    if let Some(dir) = git_dir {
        command.env("GIT_DIR", dir);
    }
    if let Some(tree) = work_tree {
        command.env("GIT_WORK_TREE", tree);
    }
    if let Some(dir) = current_dir.or(work_tree).or(git_dir) {
        command.current_dir(dir);
    }

    command.output().map_err(CheckpointError::Io)
}

fn git_ok(
    args: &[&str],
    git_dir: Option<&Path>,
    work_tree: Option<&Path>,
    current_dir: Option<&Path>,
) -> Result<(), CheckpointError> {
    let command = format!("git {}", args.join(" "));
    let output = git_output(args, git_dir, work_tree, current_dir)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CheckpointError::Git {
            command,
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

fn git_stdout(
    args: &[&str],
    git_dir: Option<&Path>,
    work_tree: Option<&Path>,
    current_dir: Option<&Path>,
) -> Result<String, CheckpointError> {
    let command = format!("git {}", args.join(" "));
    let output = git_output(args, git_dir, work_tree, current_dir)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(CheckpointError::Git {
            command,
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
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
    if regex::Regex::new(r"^[0-9a-f]{7,40}$")
        .expect("valid regex")
        .is_match(commit_hash)
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

    pub fn new_turn(&mut self, scope: impl Into<String>) {
        self.per_turn_dirs.insert(scope.into(), HashSet::new());
    }

    fn shadow_repo_path(&self, project_dir: &Path) -> PathBuf {
        let canonical = canonicalize_or_lexical(project_dir);
        let mut hasher = Sha256::new();
        hasher.update(canonical.to_string_lossy().as_bytes());
        let digest = hasher.finalize();
        let hash = format!("{:x}", digest);
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

        git_ok(&["init", "--bare"], None, None, Some(repo_dir))?;

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
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if stderr.contains("does not have any commits yet")
                || stderr.contains("unknown revision")
            {
                return Ok(Vec::new());
            }
            return Err(CheckpointError::Git {
                command: format!(
                    "git --no-pager log --pretty=format:%H\\t%ct\\t%s -n {}",
                    limit
                ),
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
            entries.push(CheckpointEntry {
                commit_hash: commit_hash.to_string(),
                timestamp,
                summary,
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
            &[
                "commit",
                "--allow-empty",
                "-m",
                &format!("[thinclaw] {}", reason),
            ],
            Some(&repo_dir),
            Some(&root),
            Some(&root),
        )?;

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
        let _ = self.create_checkpoint_inner(scope, &root, &safety_reason, true);

        if let Some(file) = file {
            let rel = validate_relative_path(file)?;
            let rel_str = rel.to_string_lossy().to_string();
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
                let target = root.join(&rel);
                if target.exists() {
                    if target.is_dir() {
                        std::fs::remove_dir_all(&target)?;
                    } else {
                        std::fs::remove_file(&target)?;
                    }
                }
            }
        } else {
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

            let untracked = git_stdout(
                &["ls-files", "--others", "--exclude-standard"],
                Some(&repo_dir),
                Some(&root),
                Some(&root),
            )?;
            for rel in untracked
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                if Path::new(rel)
                    .components()
                    .any(|component| matches!(component, Component::Normal(part) if part == ".git"))
                {
                    continue;
                }
                let target = root.join(rel);
                if target.is_dir() {
                    std::fs::remove_dir_all(&target)?;
                } else if target.exists() {
                    std::fs::remove_file(&target)?;
                }
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
                command: format!("git --no-pager diff --no-ext-diff --no-color {commit_hash}"),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
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
                command: format!("git ls-tree --name-only -r {commit_hash} -- {rel_path}"),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
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

pub fn new_turn(scope: impl Into<String>) {
    if let Ok(mut guard) = global_manager().lock() {
        guard.new_turn(scope);
    }
}

pub fn resolve_thread_root(scope: &str, fallback: Option<&Path>) -> Option<PathBuf> {
    global_manager().lock().ok().and_then(|guard| {
        guard
            .latest_thread_root(scope)
            .or_else(|| fallback.map(canonicalize_or_lexical))
            .or_else(|| std::env::current_dir().ok())
    })
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
        let mut guard = global_manager().lock().expect("checkpoint mutex poisoned");
        guard.create_checkpoint_inner(&scope, &project_root, &reason, false)
    })
    .await
    .map_err(|err| CheckpointError::Join(err.to_string()))?
}

pub async fn list_checkpoints(project_dir: &Path) -> Result<Vec<CheckpointEntry>, CheckpointError> {
    let project_dir = project_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let guard = global_manager().lock().expect("checkpoint mutex poisoned");
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
        let mut guard = global_manager().lock().expect("checkpoint mutex poisoned");
        guard.restore_inner(&scope, &project_dir, &commit_hash, file.as_deref())
    })
    .await
    .map_err(|err| CheckpointError::Join(err.to_string()))?
}

pub async fn diff(project_dir: &Path, commit_hash: &str) -> Result<String, CheckpointError> {
    let project_dir = project_dir.to_path_buf();
    let commit_hash = commit_hash.to_string();
    tokio::task::spawn_blocking(move || {
        let guard = global_manager().lock().expect("checkpoint mutex poisoned");
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

    #[tokio::test]
    async fn checkpoints_round_trip_and_restore() {
        configure(true, 10);
        new_turn("thread-a");

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
        new_turn("thread-b");

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
