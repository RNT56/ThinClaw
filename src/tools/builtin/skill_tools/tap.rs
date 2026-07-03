//! Skill tool: tap.

use super::*;

pub struct SkillTapListTool {
    store: Option<Arc<dyn Database>>,
    remote_hub: Option<SharedRemoteSkillHub>,
}

pub struct SkillTapAddTool {
    store: Option<Arc<dyn Database>>,
    remote_hub: Option<SharedRemoteSkillHub>,
}

pub struct SkillTapRemoveTool {
    store: Option<Arc<dyn Database>>,
    remote_hub: Option<SharedRemoteSkillHub>,
}

pub struct SkillTapRefreshTool {
    store: Option<Arc<dyn Database>>,
    remote_hub: Option<SharedRemoteSkillHub>,
}

impl SkillTapListTool {
    pub fn new(store: Option<Arc<dyn Database>>, remote_hub: Option<SharedRemoteSkillHub>) -> Self {
        Self { store, remote_hub }
    }
}

impl SkillTapAddTool {
    pub fn new(store: Option<Arc<dyn Database>>, remote_hub: Option<SharedRemoteSkillHub>) -> Self {
        Self { store, remote_hub }
    }
}

impl SkillTapRemoveTool {
    pub fn new(store: Option<Arc<dyn Database>>, remote_hub: Option<SharedRemoteSkillHub>) -> Self {
        Self { store, remote_hub }
    }
}

impl SkillTapRefreshTool {
    pub fn new(store: Option<Arc<dyn Database>>, remote_hub: Option<SharedRemoteSkillHub>) -> Self {
        Self { store, remote_hub }
    }
}

#[async_trait]
impl Tool for SkillTapListTool {
    fn name(&self) -> &str {
        "skill_tap_list"
    }

    fn description(&self) -> &str {
        "List configured GitHub skill taps."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_tap_list_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let include_health = params
            .get("include_health")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let store = require_skill_tap_store(&self.store, self.name())?;
        let settings = load_settings_for_taps(store, &ctx.user_id).await?;
        let hub_enabled = if include_health {
            match self.remote_hub.as_ref() {
                Some(hub) => Some(hub.is_enabled().await),
                None => Some(false),
            }
        } else {
            None
        };
        Ok(ToolOutput::success(
            skill_policy::skill_tap_list_output(
                settings.skill_taps.iter().map(tap_json).collect::<Vec<_>>(),
                hub_enabled,
            ),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

#[async_trait]
impl Tool for SkillTapAddTool {
    fn name(&self) -> &str {
        "skill_tap_add"
    }

    fn description(&self) -> &str {
        "Persist a GitHub skill tap and refresh remote skill discovery."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_tap_add_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let store = require_skill_tap_store(&self.store, self.name())?;
        let remote_hub = require_shared_remote_hub(&self.remote_hub, self.name())?;
        let parsed = skill_policy::parse_skill_tap_add_params(&params)?;
        let repo = parsed.repo;
        let path = parsed.path;
        let branch = parsed.branch;
        let trust_level = parse_tap_trust_level(&parsed.trust_level)?;
        let replace = parsed.replace;
        let mut settings = load_settings_for_taps(store, &ctx.user_id).await?;
        let existing_idx = settings
            .skill_taps
            .iter()
            .position(|tap| tap_key_matches(tap, &repo, &path, branch.as_deref()));
        match (existing_idx, replace) {
            (Some(idx), true) => {
                settings.skill_taps[idx] = SkillTapConfig {
                    repo: repo.clone(),
                    path: path.clone(),
                    branch: branch.clone(),
                    trust_level,
                };
            }
            (Some(_), false) => {
                return Err(ToolError::ExecutionFailed(format!(
                    "Skill tap '{}:{}' already exists; use replace=true to update it",
                    repo, path
                )));
            }
            (None, _) => settings.skill_taps.push(SkillTapConfig {
                repo: repo.clone(),
                path: path.clone(),
                branch: branch.clone(),
                trust_level,
            }),
        }
        persist_skill_taps(store, &ctx.user_id, &settings.skill_taps).await?;
        let refreshed_count =
            refresh_remote_hub_from_settings(store, &ctx.user_id, remote_hub).await?;
        let replaced = existing_idx.is_some();
        Ok(ToolOutput::success(
            skill_policy::skill_tap_add_output(
                replaced,
                tap_json(&SkillTapConfig {
                    repo,
                    path,
                    branch,
                    trust_level,
                }),
                refreshed_count,
            ),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

#[async_trait]
impl Tool for SkillTapRemoveTool {
    fn name(&self) -> &str {
        "skill_tap_remove"
    }

    fn description(&self) -> &str {
        "Remove a persisted GitHub skill tap and refresh remote skill discovery."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_tap_remove_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let store = require_skill_tap_store(&self.store, self.name())?;
        let remote_hub = require_shared_remote_hub(&self.remote_hub, self.name())?;
        let parsed = skill_policy::parse_skill_tap_remove_params(&params)?;
        let repo = parsed.repo;
        let path = parsed.path;
        let branch = parsed.branch;
        let mut settings = load_settings_for_taps(store, &ctx.user_id).await?;
        let before = settings.skill_taps.len();
        settings
            .skill_taps
            .retain(|tap| !tap_key_matches(tap, &repo, &path, branch.as_deref()));
        if settings.skill_taps.len() == before {
            return Err(ToolError::ExecutionFailed(format!(
                "Skill tap '{}:{}' not found",
                repo, path
            )));
        }
        persist_skill_taps(store, &ctx.user_id, &settings.skill_taps).await?;
        let refreshed_count =
            refresh_remote_hub_from_settings(store, &ctx.user_id, remote_hub).await?;
        Ok(ToolOutput::success(
            skill_policy::skill_tap_remove_output(&repo, &path, branch.as_deref(), refreshed_count),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

#[async_trait]
impl Tool for SkillTapRefreshTool {
    fn name(&self) -> &str {
        "skill_tap_refresh"
    }

    fn description(&self) -> &str {
        "Rebuild remote skill discovery from persisted skill tap settings."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_tap_refresh_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let store = require_skill_tap_store(&self.store, self.name())?;
        let remote_hub = require_shared_remote_hub(&self.remote_hub, self.name())?;
        let parsed = skill_policy::parse_skill_tap_refresh_params(&params)?;
        let repo = parsed.repo;
        let path = parsed.path;

        if repo.is_some() || path.is_some() {
            let settings = load_settings_for_taps(store, &ctx.user_id).await?;
            let matches = settings.skill_taps.iter().any(|tap| {
                let repo_matches = match repo.as_ref() {
                    Some(repo) => tap.repo.eq_ignore_ascii_case(repo),
                    None => true,
                };
                let path_matches = match path.as_ref() {
                    Some(path) => normalize_tap_path(&tap.path) == *path,
                    None => true,
                };
                repo_matches && path_matches
            });
            if !matches {
                return Err(ToolError::ExecutionFailed(
                    "No configured skill tap matches the requested refresh filter".to_string(),
                ));
            }
        }

        let tap_count = refresh_remote_hub_from_settings(store, &ctx.user_id, remote_hub).await?;
        let hub_enabled = remote_hub.is_enabled().await;
        Ok(ToolOutput::success(
            skill_policy::skill_tap_refresh_output(
                tap_count,
                repo.as_deref(),
                path.as_deref(),
                hub_enabled,
            ),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}
