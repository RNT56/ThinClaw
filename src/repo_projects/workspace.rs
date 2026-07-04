//! Local Git workspace provisioning for repo project tasks.

use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoWorkspaceError {
    InvalidOwner,
    InvalidRepo,
    InvalidProjectSlug,
    InvalidTaskId,
    CommandFailed {
        command: String,
        status: Option<i32>,
        stderr: String,
    },
    Io(String),
}

impl std::fmt::Display for RepoWorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidOwner => write!(f, "invalid GitHub owner"),
            Self::InvalidRepo => write!(f, "invalid GitHub repository"),
            Self::InvalidProjectSlug => write!(f, "invalid project slug"),
            Self::InvalidTaskId => write!(f, "invalid task id"),
            Self::CommandFailed {
                command,
                status,
                stderr,
            } => write!(
                f,
                "command failed ({command}, status {:?}): {}",
                status,
                stderr.trim()
            ),
            Self::Io(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for RepoWorkspaceError {}

#[derive(Debug, Clone)]
pub struct RepoWorkspaceProvisioner {
    base_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskWorktree {
    pub repo_dir: PathBuf,
    pub worktree_dir: PathBuf,
    pub branch: String,
}

impl RepoWorkspaceProvisioner {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub fn default_base_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".thinclaw")
            .join("projects")
    }

    pub fn repo_dir(&self, owner: &str, repo: &str) -> Result<PathBuf, RepoWorkspaceError> {
        validate_repo_component(owner).map_err(|_| RepoWorkspaceError::InvalidOwner)?;
        validate_repo_component(repo).map_err(|_| RepoWorkspaceError::InvalidRepo)?;
        Ok(self.base_dir.join(format!("{owner}__{repo}")))
    }

    pub fn task_branch(
        project_slug: &str,
        task_short_id: &str,
    ) -> Result<String, RepoWorkspaceError> {
        validate_branch_component(project_slug)
            .map_err(|_| RepoWorkspaceError::InvalidProjectSlug)?;
        validate_branch_component(task_short_id).map_err(|_| RepoWorkspaceError::InvalidTaskId)?;
        Ok(format!("thinclaw/{project_slug}/{task_short_id}"))
    }

    pub fn task_worktree_dir(
        &self,
        owner: &str,
        repo: &str,
        task_short_id: &str,
    ) -> Result<PathBuf, RepoWorkspaceError> {
        validate_branch_component(task_short_id).map_err(|_| RepoWorkspaceError::InvalidTaskId)?;
        Ok(self
            .repo_dir(owner, repo)?
            .with_file_name(format!("{owner}__{repo}__wt__{task_short_id}")))
    }

    pub async fn clone_or_fetch(
        &self,
        owner: &str,
        repo: &str,
        remote_url: &str,
        base_branch: &str,
    ) -> Result<PathBuf, RepoWorkspaceError> {
        let repo_dir = self.repo_dir(owner, repo)?;
        if !repo_dir.exists() {
            tokio::fs::create_dir_all(&self.base_dir)
                .await
                .map_err(|error| RepoWorkspaceError::Io(error.to_string()))?;
            run_git(
                &self.base_dir,
                &[
                    "clone",
                    "--origin",
                    "origin",
                    remote_url,
                    repo_dir.file_name().unwrap().to_str().unwrap(),
                ],
            )
            .await?;
        }

        run_git(&repo_dir, &["fetch", "--prune", "origin"]).await?;
        run_git(&repo_dir, &["checkout", base_branch]).await?;
        run_git(&repo_dir, &["pull", "--ff-only", "origin", base_branch]).await?;
        Ok(repo_dir)
    }

    pub async fn create_task_worktree(
        &self,
        owner: &str,
        repo: &str,
        project_slug: &str,
        task_short_id: &str,
        base_branch: &str,
    ) -> Result<TaskWorktree, RepoWorkspaceError> {
        let repo_dir = self.repo_dir(owner, repo)?;
        let worktree_dir = self.task_worktree_dir(owner, repo, task_short_id)?;
        let branch = Self::task_branch(project_slug, task_short_id)?;

        if worktree_dir.exists() {
            run_git(
                &repo_dir,
                &["worktree", "remove", "--force", path_str(&worktree_dir)?],
            )
            .await?;
        }

        run_git(
            &repo_dir,
            &[
                "worktree",
                "add",
                "-B",
                &branch,
                path_str(&worktree_dir)?,
                base_branch,
            ],
        )
        .await?;

        Ok(TaskWorktree {
            repo_dir,
            worktree_dir,
            branch,
        })
    }

    pub async fn upsert_remote(
        &self,
        owner: &str,
        repo: &str,
        remote_name: &str,
        remote_url: &str,
    ) -> Result<(), RepoWorkspaceError> {
        let repo_dir = self.repo_dir(owner, repo)?;
        validate_remote_name(remote_name).map_err(|_| RepoWorkspaceError::InvalidRepo)?;
        let _ = run_git(&repo_dir, &["remote", "remove", remote_name]).await;
        run_git(&repo_dir, &["remote", "add", remote_name, remote_url]).await
    }

    /// Create a detached worktree at the remote tip of the task branch, for a
    /// read-only review pass. Unlike [`create_task_worktree`], this checks out
    /// the *pushed* branch content (the implementation worker's commits) rather
    /// than resetting the branch to base.
    pub async fn create_review_worktree(
        &self,
        owner: &str,
        repo: &str,
        task_short_id: &str,
        branch_name: &str,
    ) -> Result<TaskWorktree, RepoWorkspaceError> {
        self.create_review_worktree_from_remote(owner, repo, task_short_id, branch_name, "origin")
            .await
    }

    pub async fn create_review_worktree_from_remote(
        &self,
        owner: &str,
        repo: &str,
        task_short_id: &str,
        branch_name: &str,
        remote_name: &str,
    ) -> Result<TaskWorktree, RepoWorkspaceError> {
        let repo_dir = self.repo_dir(owner, repo)?;
        let worktree_dir = self.review_worktree_dir(owner, repo, task_short_id)?;
        validate_remote_name(remote_name).map_err(|_| RepoWorkspaceError::InvalidRepo)?;

        // Fetch the latest tip of the branch under review.
        run_git(&repo_dir, &["fetch", remote_name, branch_name]).await?;

        if worktree_dir.exists() {
            run_git(
                &repo_dir,
                &["worktree", "remove", "--force", path_str(&worktree_dir)?],
            )
            .await?;
        }

        run_git(
            &repo_dir,
            &[
                "worktree",
                "add",
                "--force",
                "--detach",
                path_str(&worktree_dir)?,
                &format!("{remote_name}/{branch_name}"),
            ],
        )
        .await?;

        Ok(TaskWorktree {
            repo_dir,
            worktree_dir,
            branch: branch_name.to_string(),
        })
    }

    pub fn review_worktree_dir(
        &self,
        owner: &str,
        repo: &str,
        task_short_id: &str,
    ) -> Result<PathBuf, RepoWorkspaceError> {
        validate_branch_component(task_short_id).map_err(|_| RepoWorkspaceError::InvalidTaskId)?;
        Ok(self
            .repo_dir(owner, repo)?
            .with_file_name(format!("{owner}__{repo}__review__{task_short_id}")))
    }

    pub async fn remove_task_worktree(
        &self,
        owner: &str,
        repo: &str,
        task_short_id: &str,
    ) -> Result<(), RepoWorkspaceError> {
        let repo_dir = self.repo_dir(owner, repo)?;
        let worktree_dir = self.task_worktree_dir(owner, repo, task_short_id)?;
        if worktree_dir.exists() {
            run_git(
                &repo_dir,
                &["worktree", "remove", "--force", path_str(&worktree_dir)?],
            )
            .await?;
        }
        Ok(())
    }
}

fn path_str(path: &Path) -> Result<&str, RepoWorkspaceError> {
    path.to_str()
        .ok_or_else(|| RepoWorkspaceError::Io("path is not valid UTF-8".to_string()))
}

fn validate_repo_component(value: &str) -> Result<(), ()> {
    let valid = !value.is_empty()
        && !value.starts_with('.')
        && !value.contains("..")
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
    valid.then_some(()).ok_or(())
}

fn validate_branch_component(value: &str) -> Result<(), ()> {
    let valid = !value.is_empty()
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
        && !value.contains('@')
        && !value.contains("//")
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
    valid.then_some(()).ok_or(())
}

fn validate_remote_name(value: &str) -> Result<(), ()> {
    let valid = !value.is_empty()
        && !value.starts_with('.')
        && !value.contains("..")
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
    valid.then_some(()).ok_or(())
}

async fn run_git(cwd: &Path, args: &[&str]) -> Result<(), RepoWorkspaceError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|error| RepoWorkspaceError::Io(error.to_string()))?;
    if output.status.success() {
        return Ok(());
    }

    Err(RepoWorkspaceError::CommandFailed {
        command: format!("git {}", args.join(" ")),
        status: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_dir_uses_owner_repo_layout() {
        let provisioner = RepoWorkspaceProvisioner::new("/tmp/projects");
        assert_eq!(
            provisioner.repo_dir("owner", "repo").unwrap(),
            PathBuf::from("/tmp/projects/owner__repo")
        );
    }

    #[test]
    fn repo_dir_rejects_traversal() {
        let provisioner = RepoWorkspaceProvisioner::new("/tmp/projects");
        assert!(matches!(
            provisioner.repo_dir("../owner", "repo"),
            Err(RepoWorkspaceError::InvalidOwner)
        ));
        assert!(matches!(
            provisioner.repo_dir("owner", "../repo"),
            Err(RepoWorkspaceError::InvalidRepo)
        ));
    }

    #[test]
    fn task_branch_uses_thinclaw_namespace() {
        assert_eq!(
            RepoWorkspaceProvisioner::task_branch("my-project", "abc123").unwrap(),
            "thinclaw/my-project/abc123"
        );
    }

    #[test]
    fn task_branch_rejects_unsafe_components() {
        assert!(matches!(
            RepoWorkspaceProvisioner::task_branch("my/project", "abc123"),
            Err(RepoWorkspaceError::InvalidProjectSlug)
        ));
        assert!(matches!(
            RepoWorkspaceProvisioner::task_branch("project", "abc@{1}"),
            Err(RepoWorkspaceError::InvalidTaskId)
        ));
    }

    async fn git(cwd: &Path, args: &[&str]) {
        run_git(cwd, args)
            .await
            .unwrap_or_else(|e| panic!("git {args:?} failed: {e}"));
    }

    #[tokio::test]
    async fn review_worktree_checks_out_pushed_branch_content() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let root = tmp.path();
        let bare = root.join("remote.git");
        let seed = root.join("seed");
        let base_dir = root.join("workspace");

        // A bare "origin" with a main branch and a pushed feature branch.
        git(
            root,
            &["init", "--bare", "-b", "main", bare.to_str().unwrap()],
        )
        .await;
        git(
            root,
            &["clone", bare.to_str().unwrap(), seed.to_str().unwrap()],
        )
        .await;
        git(&seed, &["config", "user.email", "t@example.com"]).await;
        git(&seed, &["config", "user.name", "Test"]).await;
        tokio::fs::write(seed.join("README.md"), b"base\n")
            .await
            .unwrap();
        git(&seed, &["add", "."]).await;
        git(&seed, &["commit", "-m", "base"]).await;
        git(&seed, &["push", "-u", "origin", "main"]).await;
        git(&seed, &["checkout", "-b", "thinclaw/p/abc123"]).await;
        tokio::fs::write(seed.join("FEATURE.txt"), b"feature work\n")
            .await
            .unwrap();
        git(&seed, &["add", "."]).await;
        git(&seed, &["commit", "-m", "feature"]).await;
        git(&seed, &["push", "-u", "origin", "thinclaw/p/abc123"]).await;

        let provisioner = RepoWorkspaceProvisioner::new(&base_dir);
        provisioner
            .clone_or_fetch("owner", "repo", bare.to_str().unwrap(), "main")
            .await
            .expect("clone_or_fetch");
        let worktree = provisioner
            .create_review_worktree("owner", "repo", "abc123", "thinclaw/p/abc123")
            .await
            .expect("create_review_worktree");

        // The review worktree must contain the pushed feature content, not base.
        let feature = tokio::fs::read_to_string(worktree.worktree_dir.join("FEATURE.txt"))
            .await
            .expect("feature file present in review worktree");
        assert_eq!(feature, "feature work\n");
    }
}
