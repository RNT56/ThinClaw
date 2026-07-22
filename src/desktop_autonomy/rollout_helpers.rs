use super::*;
impl DesktopAutonomyManager {
    pub(super) fn promotion_journal_path(&self) -> PathBuf {
        self.state_root.join("promotion-journal.json")
    }

    pub(super) async fn commit_promotion(
        &self,
        mut rollout_state: RolloutState,
        manifest: BuildManifest,
    ) -> Result<(), String> {
        if !valid_build_id(&manifest.build_id)
            || rollout_state.last_promoted_build_id.as_deref() != Some(manifest.build_id.as_str())
        {
            return Err("promotion state and manifest identify different builds".to_string());
        }
        rollout_state
            .selected_build_history
            .retain(|build_id| valid_build_id(build_id));
        if rollout_state.selected_build_history.last() != Some(&manifest.build_id) {
            rollout_state
                .selected_build_history
                .push(manifest.build_id.clone());
        }
        if rollout_state.selected_build_history.len() > 16 {
            let remove = rollout_state.selected_build_history.len() - 16;
            rollout_state.selected_build_history.drain(..remove);
        }
        if load_json_file_async::<PromotionJournal>(self.promotion_journal_path())
            .await?
            .is_some()
        {
            return Err("another autonomy promotion transaction is pending".to_string());
        }
        let journal = PromotionJournal {
            version: 1,
            build_id: manifest.build_id.clone(),
            rollout_state,
            manifest,
        };
        let raw = serde_json::to_vec_pretty(&journal)
            .map_err(|error| format!("failed to serialize promotion journal: {error}"))?;
        write_autonomy_file(self.promotion_journal_path(), raw)
            .await
            .map_err(|error| format!("failed to stage promotion journal: {error}"))?;
        self.recover_pending_promotion_locked().await
    }

    pub(super) async fn recover_pending_promotion_locked(&self) -> Result<(), String> {
        let Some(journal) =
            load_json_file_async::<PromotionJournal>(self.promotion_journal_path()).await?
        else {
            return Ok(());
        };
        if journal.version != 1
            || !valid_build_id(&journal.build_id)
            || journal.manifest.build_id != journal.build_id
            || !journal.manifest.promoted
            || journal.rollout_state.last_promoted_build_id.as_deref()
                != Some(journal.build_id.as_str())
        {
            return Err("autonomy promotion journal is inconsistent".to_string());
        }

        let build_dir = self.builds_dir().join(&journal.build_id);
        let canonical_build = self.validate_promotable_build(&build_dir).await?;
        // Publish a launcher that resolves the stable `current` link before the
        // link itself changes. Existing sessions continue safely; the selected
        // build is consumed on the next launcher start.
        self.write_session_launcher().await?;
        self.save_rollout_state(&journal.rollout_state).await?;
        self.write_build_manifest(&journal.build_id, &journal.manifest)
            .await?;
        self.replace_current_build_link(&canonical_build).await?;
        remove_autonomy_file(self.promotion_journal_path())
            .await
            .map_err(|error| format!("promotion committed but journal cleanup failed: {error}"))
    }

    pub(super) async fn validate_promotable_build(
        &self,
        build_dir: &Path,
    ) -> Result<PathBuf, String> {
        let builds_dir = self.builds_dir();
        let build_dir = build_dir.to_path_buf();
        let binary_path = self.shadow_binary_path(&build_dir);
        tokio::task::spawn_blocking(move || {
            validate_promotable_build_sync(&builds_dir, &build_dir, &binary_path)
        })
        .await
        .map_err(|error| format!("promotable-build validator panicked: {error}"))?
    }

    pub(super) async fn replace_current_build_link(
        &self,
        canonical_build: &Path,
    ) -> Result<(), String> {
        let state_root = self.state_root.clone();
        let current = self.current_build_link();
        let canonical_build = canonical_build.to_path_buf();
        tokio::task::spawn_blocking(move || {
            if let Ok(metadata) = std::fs::symlink_metadata(&current)
                && !metadata.file_type().is_symlink()
            {
                return Err("autonomy current-build path is not a symlink".to_string());
            }
            if current.canonicalize().ok().as_deref() == Some(canonical_build.as_path()) {
                return Ok(());
            }

            let next = state_root.join(format!(".current.{}.next", Uuid::new_v4().simple()));
            create_symlink_dir(&canonical_build, &next)?;
            let result = thinclaw_platform::replace_path_atomic(&next, &current)
                .map_err(|error| format!("failed to atomically promote build: {error}"));
            if result.is_err() {
                let _ = std::fs::remove_file(&next);
            }
            result?;
            let selected = current
                .canonicalize()
                .map_err(|error| format!("failed to verify promoted build link: {error}"))?;
            if selected != canonical_build {
                return Err("promoted build link resolved to the wrong target".to_string());
            }
            if let Ok(directory) = std::fs::File::open(&state_root) {
                directory
                    .sync_all()
                    .map_err(|error| format!("failed to sync autonomy state directory: {error}"))?;
            }
            Ok(())
        })
        .await
        .map_err(|error| format!("build-link publisher panicked: {error}"))?
    }

    pub(super) async fn trim_old_builds(&self) -> Result<(), String> {
        let manifests = self.list_build_manifests().await?;
        let current = self.current_build_id_checked()?;
        let mut promoted_kept = 0_usize;
        let mut failed_kept = 0_usize;
        let mut remove = Vec::new();
        for manifest in manifests {
            let is_current = current.as_deref() == Some(manifest.build_id.as_str());
            let keep = if is_current {
                true
            } else if manifest.promoted && promoted_kept < 3 {
                promoted_kept += 1;
                true
            } else if !manifest.promoted && failed_kept < 3 {
                failed_kept += 1;
                true
            } else {
                false
            };
            if !keep {
                remove.push(manifest.build_id);
            }
        }

        for build_id in remove {
            self.remove_rollout_artifacts(&build_id).await?;
        }
        if let Some(managed_source) = validated_managed_source(&self.state_root, false).await? {
            run_cmd(
                Command::new("git")
                    .arg("-C")
                    .arg(&managed_source)
                    .arg("worktree")
                    .arg("prune"),
            )
            .await?;
        }
        Ok(())
    }

    pub(super) async fn cleanup_failed_rollout_build(&self, build_id: &str) -> Result<(), String> {
        if self.current_build_id_checked()?.as_deref() == Some(build_id) {
            return Err("refusing to clean the selected autonomy build".to_string());
        }
        if load_json_file_async::<PromotionJournal>(self.promotion_journal_path())
            .await?
            .is_some_and(|journal| journal.build_id == build_id)
        {
            return Err("refusing to clean a build with a pending promotion journal".to_string());
        }
        self.remove_rollout_artifacts(build_id).await
    }

    async fn remove_rollout_artifacts(&self, build_id: &str) -> Result<(), String> {
        if !valid_build_id(build_id) {
            return Err("invalid autonomy build identifier during cleanup".to_string());
        }
        let build_dir = self.builds_dir().join(build_id);
        match tokio::fs::symlink_metadata(&build_dir).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(format!(
                    "refusing to clean non-directory autonomy build {}",
                    build_dir.display()
                ));
            }
            Ok(_) => {
                let builds_root = self
                    .builds_dir()
                    .canonicalize()
                    .map_err(|error| format!("failed to resolve autonomy builds dir: {error}"))?;
                let canonical_build = build_dir
                    .canonicalize()
                    .map_err(|error| format!("failed to resolve autonomy build: {error}"))?;
                if canonical_build.parent() != Some(builds_root.as_path()) {
                    return Err("autonomy cleanup build escaped the builds directory".to_string());
                }
                let managed_source = validated_managed_source(&self.state_root, true)
                    .await?
                    .ok_or_else(|| "managed autonomy source checkout is missing".to_string())?;
                run_cmd(
                    Command::new("git")
                        .arg("-C")
                        .arg(&managed_source)
                        .arg("worktree")
                        .arg("remove")
                        .arg("--force")
                        .arg("--")
                        .arg(&canonical_build),
                )
                .await
                .map_err(|error| {
                    format!("failed to remove autonomy worktree {build_id}: {error}")
                })?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("failed to inspect autonomy build: {error}")),
        }

        let canary_dir = self.canaries_dir().join(build_id);
        remove_real_child_directory(&self.canaries_dir(), &canary_dir).await?;
        remove_autonomy_file(
            self.state_root
                .join("manifests")
                .join(format!("{build_id}.json")),
        )
        .await
    }

    pub(super) async fn load_rollout_state(&self) -> Result<RolloutState, String> {
        load_json_file_async(self.rollout_state_path())
            .await
            .map(|state| state.unwrap_or_default())
    }

    pub(super) async fn save_rollout_state(&self, state: &RolloutState) -> Result<(), String> {
        let raw = serde_json::to_string_pretty(state)
            .map_err(|e| format!("failed to serialize rollout state: {e}"))?;
        write_autonomy_file(self.rollout_state_path(), raw.into_bytes())
            .await
            .map_err(|e| format!("failed to write rollout state: {e}"))
    }

    pub(super) async fn write_build_manifest(
        &self,
        build_id: &str,
        manifest: &BuildManifest,
    ) -> Result<(), String> {
        if !valid_build_id(build_id) || manifest.build_id != build_id {
            return Err("invalid autonomy build manifest identifier".to_string());
        }
        let path = self
            .state_root
            .join("manifests")
            .join(format!("{build_id}.json"));
        let raw = serde_json::to_string_pretty(manifest)
            .map_err(|e| format!("failed to serialize build manifest: {e}"))?;
        write_autonomy_file(path, raw.into_bytes())
            .await
            .map_err(|e| format!("failed to write build manifest: {e}"))
    }

    pub(super) async fn list_build_manifests(&self) -> Result<Vec<BuildManifest>, String> {
        const MAX_BUILD_MANIFESTS: usize = 10_000;
        let mut results = Vec::new();
        let manifest_dir = self.state_root.join("manifests");
        match tokio::fs::symlink_metadata(&manifest_dir).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err("autonomy manifest path is not a real directory".to_string());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(results),
            Err(error) => return Err(format!("failed to inspect manifest dir: {error}")),
        }
        let mut entries = tokio::fs::read_dir(&manifest_dir)
            .await
            .map_err(|e| format!("failed to read manifest dir: {e}"))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| format!("failed to iterate manifest dir: {e}"))?
        {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if results.len() >= MAX_BUILD_MANIFESTS {
                return Err("autonomy build manifest count exceeds its limit".to_string());
            }
            let expected_build_id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "autonomy manifest has no valid build identifier".to_string())?;
            let manifest: BuildManifest = load_json_file_async(path.clone())
                .await?
                .ok_or_else(|| format!("manifest disappeared: {}", path.display()))?;
            if manifest.build_id != expected_build_id {
                return Err(format!(
                    "autonomy manifest {} identifies a different build",
                    path.display()
                ));
            }
            if !valid_build_id(&manifest.build_id) {
                return Err(format!(
                    "autonomy manifest {} has an invalid build identifier",
                    path.display()
                ));
            }
            results.push(manifest);
        }
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(results)
    }

    pub(super) async fn record_rollback_observation(
        &self,
        current_build_id: &str,
    ) -> Result<(), String> {
        let Some(store) = self.store.as_ref() else {
            return Ok(());
        };
        let manifests = self.list_build_manifests().await?;
        let Some(current_manifest) = manifests
            .iter()
            .find(|manifest| manifest.build_id == current_build_id)
        else {
            return Ok(());
        };
        let versions = store
            .list_learning_artifact_versions(
                &current_manifest.user_id,
                Some("code"),
                Some(current_build_id),
                5,
            )
            .await
            .map_err(|e| format!("failed to list code artifact versions for rollback: {e}"))?;
        let Some(version) = versions.first() else {
            return Ok(());
        };
        let rollback = crate::history::LearningRollbackRecord {
            id: Uuid::new_v4(),
            user_id: current_manifest.user_id.clone(),
            artifact_type: "code".to_string(),
            artifact_name: current_build_id.to_string(),
            artifact_version_id: Some(version.id),
            reason: "desktop autonomy rollback".to_string(),
            metadata: serde_json::json!({
                "build_id": current_build_id,
                "rolled_back_at": Utc::now(),
                "autonomy_control": true,
            }),
            created_at: Utc::now(),
        };
        store
            .insert_learning_rollback(&rollback)
            .await
            .map_err(|e| format!("failed to persist rollback record: {e}"))?;
        crate::agent::outcomes::observe_rollback(store, &rollback).await?;
        Ok(())
    }
}

async fn validated_managed_source(
    state_root: &Path,
    required: bool,
) -> Result<Option<PathBuf>, String> {
    let managed_source = state_root.join("agent-src");
    let metadata = match tokio::fs::symlink_metadata(&managed_source).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("managed autonomy source checkout is missing".to_string());
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect managed source checkout: {error}"
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("managed autonomy source checkout is not a real directory".to_string());
    }
    let canonical_root = tokio::fs::canonicalize(state_root)
        .await
        .map_err(|error| format!("failed to resolve autonomy state root: {error}"))?;
    let canonical_source = tokio::fs::canonicalize(&managed_source)
        .await
        .map_err(|error| format!("failed to resolve managed source checkout: {error}"))?;
    if canonical_source.parent() != Some(canonical_root.as_path()) {
        return Err("managed autonomy source checkout escaped the state root".to_string());
    }
    let git_metadata = tokio::fs::symlink_metadata(canonical_source.join(".git"))
        .await
        .map_err(|error| format!("managed source checkout has no safe .git directory: {error}"))?;
    if git_metadata.file_type().is_symlink() || !git_metadata.is_dir() {
        return Err("managed source checkout .git path is not a real directory".to_string());
    }
    Ok(Some(canonical_source))
}

pub(super) fn valid_build_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub(super) fn rollback_target_and_history(
    manifests: &[BuildManifest],
    selected_history: &[String],
    current: Option<&str>,
) -> (Option<String>, Vec<String>) {
    let Some(current) = current else {
        return (None, Vec::new());
    };
    let promoted_ids: std::collections::HashSet<&str> = manifests
        .iter()
        .filter(|manifest| manifest.promoted)
        .map(|manifest| manifest.build_id.as_str())
        .collect();
    let mut history: Vec<String> = selected_history
        .iter()
        .filter(|build_id| promoted_ids.contains(build_id.as_str()))
        .cloned()
        .collect();
    if history.is_empty() {
        history.extend(
            manifests
                .iter()
                .rev()
                .filter(|manifest| manifest.promoted)
                .map(|manifest| manifest.build_id.clone()),
        );
    }
    if let Some(position) = history.iter().rposition(|build_id| build_id == current) {
        history.truncate(position + 1);
    } else if promoted_ids.contains(current) {
        history.push(current.to_string());
    } else {
        return (None, history);
    }
    if history.last().is_some_and(|build_id| build_id == current) {
        history.pop();
    }
    (history.last().cloned(), history)
}

async fn remove_real_child_directory(root: &Path, child: &Path) -> Result<(), String> {
    let metadata = match tokio::fs::symlink_metadata(child).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to inspect {}: {error}", child.display())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "refusing to remove non-directory path {}",
            child.display()
        ));
    }
    let canonical_root = tokio::fs::canonicalize(root)
        .await
        .map_err(|error| format!("failed to resolve {}: {error}", root.display()))?;
    let canonical_child = tokio::fs::canonicalize(child)
        .await
        .map_err(|error| format!("failed to resolve {}: {error}", child.display()))?;
    if canonical_child.parent() != Some(canonical_root.as_path()) {
        return Err(format!("cleanup path {} escaped its root", child.display()));
    }
    tokio::fs::remove_dir_all(canonical_child)
        .await
        .map_err(|error| format!("failed to remove {}: {error}", child.display()))
}

pub(super) fn validate_promotable_build_sync(
    builds_dir: &Path,
    build_dir: &Path,
    binary_path: &Path,
) -> Result<PathBuf, String> {
    const MAX_PROMOTED_BINARY_BYTES: u64 = 2 * 1024 * 1024 * 1024;

    let builds_root = builds_dir
        .canonicalize()
        .map_err(|error| format!("failed to resolve autonomy builds directory: {error}"))?;
    let build_metadata = std::fs::symlink_metadata(build_dir)
        .map_err(|error| format!("failed to inspect promotion build: {error}"))?;
    if build_metadata.file_type().is_symlink() || !build_metadata.is_dir() {
        return Err("promotion build is not a real directory".to_string());
    }
    let canonical_build = build_dir
        .canonicalize()
        .map_err(|error| format!("failed to resolve promotion build: {error}"))?;
    if canonical_build.parent() != Some(builds_root.as_path()) {
        return Err("promotion build is outside the managed builds directory".to_string());
    }

    let binary_metadata = std::fs::symlink_metadata(binary_path)
        .map_err(|error| format!("failed to inspect promotion binary: {error}"))?;
    if binary_metadata.file_type().is_symlink()
        || !binary_metadata.is_file()
        || binary_metadata.len() == 0
        || binary_metadata.len() > MAX_PROMOTED_BINARY_BYTES
    {
        return Err("promotion binary is not a bounded regular file".to_string());
    }
    let canonical_binary = binary_path
        .canonicalize()
        .map_err(|error| format!("failed to resolve promotion binary: {error}"))?;
    if !canonical_binary.starts_with(&canonical_build) {
        return Err("promotion binary escapes the selected build".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if binary_metadata.permissions().mode() & 0o111 == 0 {
            return Err("promotion binary is not executable".to_string());
        }
    }
    Ok(canonical_build)
}
