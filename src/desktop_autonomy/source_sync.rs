use super::*;
impl DesktopAutonomyManager {
    pub(super) async fn sync_managed_source_clone(&self) -> Result<PathBuf, String> {
        let repo_root = std::env::current_dir().map_err(|e| format!("current_dir: {e}"))?;
        let managed_source = self.state_root.join("agent-src");
        if !managed_source.exists() {
            run_cmd(
                Command::new("git")
                    .arg("clone")
                    .arg("--no-hardlinks")
                    .arg(repo_root.as_os_str())
                    .arg(managed_source.as_os_str()),
            )
            .await?;
            return Ok(managed_source);
        }

        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&managed_source)
                .arg("fetch")
                .arg("--all")
                .arg("--prune"),
        )
        .await?;
        let head_ref = run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&repo_root)
                .arg("rev-parse")
                .arg("HEAD"),
        )
        .await?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&managed_source)
                .arg("reset")
                .arg("--hard")
                .arg(head_ref.trim()),
        )
        .await?;
        Ok(managed_source)
    }
}
