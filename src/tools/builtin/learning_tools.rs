//! Learning-tool suite for prompt and skill mutation plus learning ledger access.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::agent::{learning::LearningOrchestrator, outcomes};
use crate::context::JobContext;
use crate::db::Database;
use crate::error::WorkspaceError;
use crate::history::{
    LearningArtifactVersion as DbLearningArtifactVersion,
    OutcomeContractQuery as DbOutcomeContractQuery,
};
use crate::settings::LearningSettings;
use crate::skills::{
    MAX_PROMPT_FILE_SIZE, SkillSource, normalize_line_endings,
    parser::parse_skill_md,
    registry::{SkillRegistry, SkillRegistryError, check_gating},
};
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};
use crate::workspace::{Workspace, paths};
use thinclaw_tools::builtin::learning as learning_policy;
use thinclaw_tools::ports::{
    LearningToolHostPort, ToolHostError, ToolLearningActionRequest, ToolLearningActionResult,
    ToolLearningFeedbackRequest, ToolLearningHistoryQuery, ToolLearningProposalReview,
    ToolLearningRecord, job_context_from_tool_scope,
};

const SKILL_FILE_NAME: &str = learning_policy::SKILL_FILE_NAME;
const MAX_MANAGED_SKILL_FILE_BYTES: u64 = 8 * 1024 * 1024;

pub struct RootLearningToolHost {
    orchestrator: Arc<LearningOrchestrator>,
    store: Arc<dyn Database>,
    workspace: Option<Arc<Workspace>>,
    skill_registry: Option<Arc<RwLock<SkillRegistry>>>,
}

pub fn root_learning_tool_host(
    orchestrator: Arc<LearningOrchestrator>,
    store: Arc<dyn Database>,
    workspace: Option<Arc<Workspace>>,
    skill_registry: Option<Arc<RwLock<SkillRegistry>>>,
) -> Arc<dyn LearningToolHostPort> {
    Arc::new(RootLearningToolHost {
        orchestrator,
        store,
        workspace,
        skill_registry,
    })
}

fn learning_tool_host_error_from_tool(error: ToolError) -> ToolHostError {
    ToolHostError::OperationFailed {
        reason: error.to_string(),
    }
}

async fn execute_root_learning_tool<T>(
    tool: T,
    request: ToolLearningActionRequest,
    title: &str,
) -> Result<ToolLearningActionResult, ToolHostError>
where
    T: Tool,
{
    let ctx = job_context_from_tool_scope(request.scope, title);
    let output = tool
        .execute(request.params, &ctx)
        .await
        .map_err(learning_tool_host_error_from_tool)?;
    Ok(ToolLearningActionResult {
        output: output.result,
    })
}

#[async_trait]
impl LearningToolHostPort for RootLearningToolHost {
    async fn record_feedback(
        &self,
        _request: ToolLearningFeedbackRequest,
    ) -> Result<ToolLearningRecord, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "learning_feedback_structured".to_string(),
        })
    }

    async fn list_learning_history(
        &self,
        _query: ToolLearningHistoryQuery,
    ) -> Result<Vec<ToolLearningRecord>, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "learning_history_structured".to_string(),
        })
    }

    async fn review_learning_proposal(
        &self,
        _review: ToolLearningProposalReview,
    ) -> Result<ToolLearningRecord, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "learning_proposal_review_structured".to_string(),
        })
    }

    async fn prompt_manage_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError> {
        let workspace = self
            .workspace
            .clone()
            .ok_or_else(|| ToolHostError::Unavailable {
                service: "prompt_manage".to_string(),
            })?;
        execute_root_learning_tool(
            PromptManageTool::new(
                Arc::clone(&self.orchestrator),
                Arc::clone(&self.store),
                workspace,
            ),
            request,
            "prompt manage",
        )
        .await
    }

    async fn skill_manage_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError> {
        let skill_registry =
            self.skill_registry
                .clone()
                .ok_or_else(|| ToolHostError::Unavailable {
                    service: "skill_manage".to_string(),
                })?;
        execute_root_learning_tool(
            SkillManageTool::new(Arc::clone(&self.store), skill_registry),
            request,
            "skill manage",
        )
        .await
    }

    async fn learning_status_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError> {
        execute_root_learning_tool(
            LearningStatusTool::new(Arc::clone(&self.orchestrator), Arc::clone(&self.store)),
            request,
            "learning status",
        )
        .await
    }

    async fn learning_outcomes_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError> {
        execute_root_learning_tool(
            LearningOutcomesTool::new(Arc::clone(&self.store)),
            request,
            "learning outcomes",
        )
        .await
    }

    async fn learning_history_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError> {
        execute_root_learning_tool(
            LearningHistoryTool::new(Arc::clone(&self.store)),
            request,
            "learning history",
        )
        .await
    }

    async fn learning_feedback_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError> {
        execute_root_learning_tool(
            LearningFeedbackTool::new(Arc::clone(&self.orchestrator)),
            request,
            "learning feedback",
        )
        .await
    }

    async fn learning_proposal_review_action(
        &self,
        request: ToolLearningActionRequest,
    ) -> Result<ToolLearningActionResult, ToolHostError> {
        execute_root_learning_tool(
            LearningProposalReviewTool::new(Arc::clone(&self.orchestrator)),
            request,
            "learning proposal review",
        )
        .await
    }
}

fn tool_error_from_skill(err: SkillRegistryError) -> ToolError {
    ToolError::ExecutionFailed(err.to_string())
}

fn validate_prompt_content(content: &str) -> Result<(), ToolError> {
    learning_policy::validate_prompt_content(content)
}

fn validate_prompt_safety(target: &str, content: &str) -> Result<(), ToolError> {
    match target {
        paths::SOUL => crate::identity::soul::validate_canonical_soul(content)
            .map_err(ToolError::InvalidParameters),
        paths::SOUL_LOCAL => crate::identity::soul::validate_local_overlay(content)
            .map_err(ToolError::InvalidParameters),
        paths::AGENTS => learning_policy::validate_agents_prompt_safety(content),
        _ => Ok(()),
    }
}

fn validate_skill_admin_available(ctx: &JobContext, tool_name: &str) -> Result<(), ToolError> {
    learning_policy::validate_skill_admin_available(&ctx.metadata, tool_name)
}

fn validate_prompt_manage_available(ctx: &JobContext) -> Result<(), ToolError> {
    learning_policy::validate_prompt_manage_available(&ctx.metadata)
}

fn validate_prompt_manage_settings(settings: &LearningSettings) -> Result<(), ToolError> {
    if settings.prompt_mutation.enabled {
        Ok(())
    } else {
        Err(ToolError::ExecutionFailed(
            "prompt mutation is disabled in the learning settings".to_string(),
        ))
    }
}

async fn read_prompt_target_content(
    workspace: &Workspace,
    resolved_target: &str,
) -> Result<String, ToolError> {
    if resolved_target.eq_ignore_ascii_case(paths::SOUL) {
        return match crate::identity::soul_store::read_home_soul() {
            Ok(content) => Ok(content),
            Err(WorkspaceError::DocumentNotFound { .. }) => Ok(String::new()),
            Err(err) => Err(ToolError::ExecutionFailed(format!(
                "failed to read canonical SOUL.md: {}",
                err
            ))),
        };
    }

    Ok(workspace
        .read(resolved_target)
        .await
        .ok()
        .map(|doc| doc.content)
        .unwrap_or_default())
}

async fn write_prompt_target_content(
    workspace: &Workspace,
    resolved_target: &str,
    content: &str,
) -> Result<(), ToolError> {
    if resolved_target.eq_ignore_ascii_case(paths::SOUL) {
        return crate::identity::soul_store::write_home_soul(content).map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to update canonical SOUL.md: {}", err))
        });
    }

    workspace
        .write(resolved_target, content)
        .await
        .map(|_| ())
        .map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to update '{}': {}", resolved_target, err))
        })
}

fn validate_relative_skill_path(path: &str) -> Result<PathBuf, ToolError> {
    learning_policy::validate_relative_skill_path(path)
}

fn artifact_name_for_skill(skill_name: &str, path: &Path) -> String {
    learning_policy::artifact_name_for_skill(skill_name, path)
}

async fn read_text(path: &Path) -> Result<Option<String>, ToolError> {
    match thinclaw_platform::read_regular_file_bounded_single_link_async(
        path.to_path_buf(),
        MAX_MANAGED_SKILL_FILE_BYTES,
    )
    .await
    {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|err| ToolError::ExecutionFailed(format!("skill file is not UTF-8: {err}"))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(ToolError::ExecutionFailed(format!(
            "failed to read skill file: {err}"
        ))),
    }
}

async fn write_text(path: &Path, content: &str) -> Result<(), ToolError> {
    if content.len() as u64 > MAX_MANAGED_SKILL_FILE_BYTES {
        return Err(ToolError::InvalidParameters(format!(
            "skill file exceeds the {MAX_MANAGED_SKILL_FILE_BYTES}-byte limit"
        )));
    }
    thinclaw_platform::write_private_file_atomic_async(
        path.to_path_buf(),
        content.as_bytes().to_vec(),
        true,
    )
    .await
    .map_err(|err| ToolError::ExecutionFailed(format!("failed to write skill file: {err}")))
}

async fn remove_path(path: &Path) -> Result<(), ToolError> {
    let exists = tokio::fs::try_exists(path).await.map_err(|err| {
        ToolError::ExecutionFailed(format!("failed to stat '{}': {}", path.display(), err))
    })?;
    if !exists {
        return Err(ToolError::ExecutionFailed(format!(
            "path '{}' does not exist",
            path.display()
        )));
    }
    tokio::fs::remove_file(path).await.map_err(|err| {
        ToolError::ExecutionFailed(format!("failed to remove '{}': {}", path.display(), err))
    })
}

async fn resolve_managed_skill_target(
    root: &Path,
    relative: &Path,
    create_parents: bool,
) -> Result<PathBuf, ToolError> {
    let root_metadata = tokio::fs::symlink_metadata(root).await.map_err(|err| {
        ToolError::ExecutionFailed(format!("failed to inspect skill root: {err}"))
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(ToolError::ExecutionFailed(
            "skill root is not a real directory".to_string(),
        ));
    }
    let canonical_root = tokio::fs::canonicalize(root).await.map_err(|err| {
        ToolError::ExecutionFailed(format!("failed to resolve skill root: {err}"))
    })?;
    let components = relative.components().collect::<Vec<_>>();
    let Some((filename, parents)) = components.split_last() else {
        return Err(ToolError::InvalidParameters(
            "skill file path cannot be empty".to_string(),
        ));
    };
    let std::path::Component::Normal(filename) = filename else {
        return Err(ToolError::InvalidParameters(
            "skill file path is invalid".to_string(),
        ));
    };
    let mut parent = canonical_root;
    for component in parents {
        let std::path::Component::Normal(component) = component else {
            return Err(ToolError::InvalidParameters(
                "skill file path is invalid".to_string(),
            ));
        };
        parent.push(component);
        match tokio::fs::symlink_metadata(&parent).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(ToolError::ExecutionFailed(
                    "skill file path traverses a non-directory or symlink".to_string(),
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound && create_parents => {
                tokio::fs::create_dir(&parent).await.map_err(|err| {
                    ToolError::ExecutionFailed(format!(
                        "failed to create skill file directory: {err}"
                    ))
                })?;
            }
            Err(err) => {
                return Err(ToolError::ExecutionFailed(format!(
                    "failed to inspect skill file directory: {err}"
                )));
            }
        }
    }
    Ok(parent.join(filename))
}

async fn record_artifact_version(
    store: &Arc<dyn Database>,
    user_id: &str,
    artifact_type: &str,
    artifact_name: &str,
    version_label: Option<String>,
    status: &str,
    diff_summary: Option<String>,
    before_content: Option<String>,
    after_content: Option<String>,
    provenance: serde_json::Value,
) -> Result<Uuid, String> {
    let version = DbLearningArtifactVersion {
        id: Uuid::new_v4(),
        candidate_id: None,
        user_id: user_id.to_string(),
        artifact_type: artifact_type.to_string(),
        artifact_name: artifact_name.to_string(),
        version_label,
        status: status.to_string(),
        diff_summary,
        before_content,
        after_content,
        provenance,
        created_at: Utc::now(),
    };
    let id = store
        .insert_learning_artifact_version(&version)
        .await
        .map_err(|err| err.to_string())?;
    if let Err(err) = outcomes::maybe_create_artifact_contract(store, &version).await {
        tracing::debug!(
            artifact_type = %artifact_type,
            artifact_name = %artifact_name,
            error = %err,
            "Outcome-backed artifact contract creation skipped"
        );
    }
    Ok(id)
}

async fn loaded_skill_root(
    registry: &Arc<RwLock<SkillRegistry>>,
    name: &str,
) -> Result<(PathBuf, bool), ToolError> {
    let guard = registry.read().await;
    let skill = guard
        .find_by_name(name)
        .ok_or_else(|| ToolError::ExecutionFailed(format!("Skill '{}' not found", name)))?;

    match &skill.source {
        SkillSource::User(path) => Ok((path.clone(), true)),
        SkillSource::Workspace(_) => Err(ToolError::ExecutionFailed(format!(
            "Skill '{}' is workspace-managed and cannot be changed through skill_manage",
            name
        ))),
        SkillSource::Bundled(_) => Err(ToolError::ExecutionFailed(format!(
            "Skill '{}' is bundled and cannot be changed through skill_manage",
            name
        ))),
        SkillSource::External(_) => Err(ToolError::ExecutionFailed(format!(
            "Skill '{}' is external read-only and cannot be changed through skill_manage",
            name
        ))),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// prompt_manage
// ─────────────────────────────────────────────────────────────────────────────

pub struct PromptManageTool {
    orchestrator: Arc<LearningOrchestrator>,
    store: Arc<dyn Database>,
    workspace: Arc<Workspace>,
}

impl PromptManageTool {
    pub fn new(
        orchestrator: Arc<LearningOrchestrator>,
        store: Arc<dyn Database>,
        workspace: Arc<Workspace>,
    ) -> Self {
        Self {
            orchestrator,
            store,
            workspace,
        }
    }
}

#[async_trait]
impl Tool for PromptManageTool {
    fn name(&self) -> &str {
        "prompt_manage"
    }

    fn description(&self) -> &str {
        "Update the canonical SOUL.md, workspace SOUL.local.md, AGENTS.md, or USER.md. Run session_search + memory_search before mutation, then apply bounded prompt edits with validation and version recording."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        learning_policy::prompt_manage_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        validate_prompt_manage_available(ctx)?;
        let settings = self.orchestrator.load_settings_for_user(&ctx.user_id).await;
        validate_prompt_manage_settings(&settings)?;

        let parsed = learning_policy::parse_prompt_manage_params(&params)?;
        let operation = parsed.operation;
        let target = parsed.target;
        let scope = parsed.scope;
        let target_resolution = learning_policy::resolve_prompt_manage_target(
            target,
            &scope,
            &ctx.metadata,
            Some(ctx.owner_actor_id()),
            &ctx.user_id,
        )?;
        let resolved_target = target_resolution.resolved_target;
        let timezone_sync_target = target_resolution.timezone_sync_target;
        let timezone_actor_id = target_resolution.timezone_actor_id;
        let agent_id = ctx
            .metadata
            .get("agent_workspace_id")
            .and_then(|value| value.as_str())
            .and_then(|value| Uuid::parse_str(value).ok())
            .or(self.workspace.agent_id());
        let principal_workspace = self
            .workspace
            .scoped_clone(ctx.principal_id.clone(), agent_id);
        let before = read_prompt_target_content(&principal_workspace, &resolved_target).await?;
        let before_timezone = if timezone_sync_target {
            crate::timezone::extract_markdown_timezone(&before)
        } else {
            None
        };

        let next_content =
            learning_policy::prompt_manage_next_content(&params, &before, &operation)?;
        validate_prompt_content(&next_content)?;
        validate_prompt_safety(target, &next_content)?;
        if target == paths::USER {
            crate::timezone::validate_markdown_timezone_field(&next_content)
                .map_err(ToolError::InvalidParameters)?;
        }

        write_prompt_target_content(&principal_workspace, &resolved_target, &next_content).await?;
        let after = read_prompt_target_content(&principal_workspace, &resolved_target).await?;
        let after_timezone = if timezone_sync_target {
            crate::timezone::extract_markdown_timezone(&after)
        } else {
            None
        };
        let actor_private = resolved_target.starts_with(&format!("{}/", paths::ACTORS_DIR));
        if !actor_private {
            let provider_access =
                crate::agent::learning::provider_access_context_from_job(ctx).ok();
            let mirror_payload = learning_policy::prompt_manage_mirror_payload(
                target,
                &resolved_target,
                &scope,
                &operation,
                &after,
            );
            if let Some(access) = provider_access
                && tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    self.orchestrator
                        .mirror_workspace_write(&access, &mirror_payload),
                )
                .await
                .is_err()
            {
                tracing::warn!(target = %resolved_target, "Learning-provider prompt mirror timed out");
            }
        }

        let version_label = Some(Utc::now().to_rfc3339());
        let provenance = learning_policy::prompt_manage_provenance(
            target,
            &resolved_target,
            &scope,
            &ctx.user_id,
        );
        let version_result = record_artifact_version(
            &self.store,
            &ctx.user_id,
            "prompt",
            &resolved_target,
            version_label.clone(),
            "applied",
            Some(format!("prompt {} via prompt_manage", operation)),
            (!actor_private).then_some(before.clone()),
            (!actor_private).then_some(after.clone()),
            provenance,
        )
        .await;

        if timezone_sync_target && before_timezone != after_timezone {
            if let Some(actor_id) = timezone_actor_id.as_deref() {
                crate::timezone::apply_actor_timezone_change(
                    &self.store,
                    &ctx.principal_id,
                    actor_id,
                    after_timezone.as_deref(),
                )
                .await
            } else {
                crate::timezone::apply_user_timezone_change(
                    &self.store,
                    Some(&principal_workspace),
                    &ctx.principal_id,
                    after_timezone.as_deref(),
                )
                .await
                .map(|_| ())
            }
            .map_err(|err| {
                ToolError::ExecutionFailed(format!("failed to apply timezone update: {}", err))
            })?;
        }

        let result = learning_policy::prompt_manage_output(
            &operation,
            &resolved_target,
            next_content.len(),
            target == paths::SOUL || target == paths::SOUL_LOCAL,
            version_label,
            version_result.is_ok(),
            version_result.err(),
        );
        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// skill_manage
// ─────────────────────────────────────────────────────────────────────────────

pub struct SkillManageTool {
    store: Arc<dyn Database>,
    registry: Arc<RwLock<SkillRegistry>>,
}

impl SkillManageTool {
    pub fn new(store: Arc<dyn Database>, registry: Arc<RwLock<SkillRegistry>>) -> Self {
        Self { store, registry }
    }
}

#[async_trait]
impl Tool for SkillManageTool {
    fn name(&self) -> &str {
        "skill_manage"
    }

    fn description(&self) -> &str {
        "Create, patch, edit, delete, write files, remove files, or reload skills. Run session_search + memory_search before mutation; all skill changes are validated and versioned."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        learning_policy::skill_manage_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        validate_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let parsed = learning_policy::parse_skill_manage_params(&params)?;
        let operation = parsed.operation;
        let name = parsed.name;
        let path_value = parsed.path;
        let all = parsed.all;

        match operation.as_str() {
            "reload" => {
                if all {
                    // Load-then-swap: build a fresh registry and run the expensive
                    // discovery IO off-lock, then swap it in under a brief write.
                    // Concurrent skill reads are not blocked behind discovery, and
                    // never observe the partially-populated state that the in-place
                    // `reload` (clear-then-async-repopulate under the write lock)
                    // exposes.
                    let mut fresh = self.registry.read().await.clone_config();
                    let loaded = fresh.discover_all().await;
                    *self.registry.write().await = fresh;
                    return Ok(ToolOutput::success(
                        learning_policy::skill_manage_reload_all_output(loaded),
                        start.elapsed(),
                    ));
                }

                let mut guard = self.registry.write().await;
                let reloaded = guard.reload_skill(&name).await.map_err(|err| {
                    ToolError::ExecutionFailed(format!(
                        "failed to reload skill '{}': {}",
                        name, err
                    ))
                })?;
                return Ok(ToolOutput::success(
                    learning_policy::skill_manage_reload_output(&reloaded),
                    start.elapsed(),
                ));
            }
            "create" => {
                let content = require_str(&params, "content")?;
                if !path_value.eq_ignore_ascii_case(SKILL_FILE_NAME) {
                    return Err(ToolError::InvalidParameters(
                        "create only supports path='SKILL.md'".to_string(),
                    ));
                }
                let normalized = normalize_line_endings(content);
                if normalized.len() as u64 > MAX_PROMPT_FILE_SIZE {
                    return Err(ToolError::InvalidParameters(format!(
                        "skill content too large: {} bytes (max {} bytes)",
                        normalized.len(),
                        MAX_PROMPT_FILE_SIZE
                    )));
                }
                let parsed = parse_skill_md(&normalized)
                    .map_err(|err| ToolError::InvalidParameters(err.to_string()))?;
                if !parsed.manifest.name.eq_ignore_ascii_case(&name) {
                    return Err(ToolError::InvalidParameters(format!(
                        "skill name '{}' does not match SKILL.md frontmatter name '{}'",
                        name, parsed.manifest.name
                    )));
                }
                if let Some(meta) = parsed
                    .manifest
                    .metadata
                    .as_ref()
                    .and_then(|m| m.openclaw.as_ref())
                {
                    let gating = check_gating(&meta.requires).await;
                    if !gating.passed {
                        return Err(ToolError::ExecutionFailed(format!(
                            "skill gating failed for '{}': {}",
                            name,
                            gating.failures.join("; ")
                        )));
                    }
                }

                let install_dir = {
                    let guard = self.registry.read().await;
                    guard.install_target_dir().to_path_buf()
                };
                let existed_already = tokio::fs::try_exists(install_dir.join(&name))
                    .await
                    .unwrap_or(false);
                if existed_already {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Skill '{}' already exists; use edit/write_file instead",
                        name
                    )));
                }

                let (skill_name, loaded_skill) =
                    SkillRegistry::prepare_install_to_disk(&install_dir, &name, &normalized)
                        .await
                        .map_err(tool_error_from_skill)?;

                let commit_result = {
                    let mut guard = self.registry.write().await;
                    guard.commit_install(&skill_name, loaded_skill)
                };
                if let Err(err) = commit_result {
                    let cleanup_path = install_dir.join(&name);
                    let _ = SkillRegistry::delete_skill_files(&cleanup_path, &name).await;
                    return Err(tool_error_from_skill(err));
                }

                let version_result = record_artifact_version(
                    &self.store,
                    &ctx.user_id,
                    "skill",
                    &skill_name,
                    Some(parsed.manifest.version.clone()),
                    "applied",
                    Some("skill created via skill_manage".to_string()),
                    None,
                    Some(normalized),
                    learning_policy::skill_manage_provenance("create", Some(SKILL_FILE_NAME), None),
                )
                .await;

                return Ok(ToolOutput::success(
                    learning_policy::skill_manage_created_output(
                        &skill_name,
                        version_result.is_ok(),
                        version_result.err(),
                    ),
                    start.elapsed(),
                ));
            }
            "delete" => {
                let mut guard = self.registry.write().await;
                let skill_path = guard
                    .validate_remove(&name)
                    .map_err(tool_error_from_skill)?;
                let before_content = read_text(&skill_path.join(SKILL_FILE_NAME)).await?;
                SkillRegistry::delete_skill_files(&skill_path, &name)
                    .await
                    .map_err(tool_error_from_skill)?;
                guard.commit_remove(&name).map_err(tool_error_from_skill)?;
                drop(guard);

                let version_result = record_artifact_version(
                    &self.store,
                    &ctx.user_id,
                    "skill",
                    &name,
                    Some(Utc::now().to_rfc3339()),
                    "deleted",
                    Some("skill deleted via skill_manage".to_string()),
                    before_content,
                    None,
                    learning_policy::skill_manage_provenance("delete", Option::<&str>::None, None),
                )
                .await;

                return Ok(ToolOutput::success(
                    learning_policy::skill_manage_deleted_output(
                        &name,
                        version_result.is_ok(),
                        version_result.err(),
                    ),
                    start.elapsed(),
                ));
            }
            "remove_file" => {
                let relative = validate_relative_skill_path(&path_value)?;
                if relative
                    .to_string_lossy()
                    .eq_ignore_ascii_case(SKILL_FILE_NAME)
                {
                    return Err(ToolError::InvalidParameters(
                        "remove_file cannot delete SKILL.md; use operation='delete' instead"
                            .to_string(),
                    ));
                }

                let root = loaded_skill_root(&self.registry, &name).await?;
                let target = resolve_managed_skill_target(&root.0, &relative, false).await?;
                let before = read_text(&target).await?.ok_or_else(|| {
                    ToolError::ExecutionFailed(format!(
                        "skill file '{}' does not exist",
                        target.display()
                    ))
                })?;
                remove_path(&target).await?;

                let version_result = record_artifact_version(
                    &self.store,
                    &ctx.user_id,
                    "skill_file",
                    &artifact_name_for_skill(&name, &relative),
                    Some(Utc::now().to_rfc3339()),
                    "deleted",
                    Some("skill file removed via skill_manage".to_string()),
                    Some(before),
                    None,
                    learning_policy::skill_manage_provenance("remove_file", Some(&relative), None),
                )
                .await;

                return Ok(ToolOutput::success(
                    learning_policy::skill_manage_removed_file_output(
                        &name,
                        &relative,
                        version_result.is_ok(),
                        version_result.err(),
                    ),
                    start.elapsed(),
                ));
            }
            "write_file" | "edit" | "patch" => {
                let content = require_str(&params, "content")?;
                let relative = validate_relative_skill_path(&path_value)?;
                let root = loaded_skill_root(&self.registry, &name).await?;
                let target = resolve_managed_skill_target(&root.0, &relative, true).await?;

                if relative
                    .to_string_lossy()
                    .eq_ignore_ascii_case(SKILL_FILE_NAME)
                {
                    let normalized = normalize_line_endings(content);
                    if normalized.len() as u64 > MAX_PROMPT_FILE_SIZE {
                        return Err(ToolError::InvalidParameters(format!(
                            "skill content too large: {} bytes (max {} bytes)",
                            normalized.len(),
                            MAX_PROMPT_FILE_SIZE
                        )));
                    }
                    let parsed = parse_skill_md(&normalized)
                        .map_err(|err| ToolError::InvalidParameters(err.to_string()))?;
                    if !parsed.manifest.name.eq_ignore_ascii_case(&name) {
                        return Err(ToolError::InvalidParameters(format!(
                            "skill name '{}' does not match existing SKILL.md frontmatter name '{}'",
                            name, parsed.manifest.name
                        )));
                    }
                    if let Some(meta) = parsed
                        .manifest
                        .metadata
                        .as_ref()
                        .and_then(|m| m.openclaw.as_ref())
                    {
                        let gating = check_gating(&meta.requires).await;
                        if !gating.passed {
                            return Err(ToolError::ExecutionFailed(format!(
                                "skill gating failed for '{}': {}",
                                name,
                                gating.failures.join("; ")
                            )));
                        }
                    }
                    let before = read_text(&target).await?.unwrap_or_default();
                    write_text(&target, &normalized).await?;
                    let mut guard = self.registry.write().await;
                    let reloaded = match guard.reload_skill(&name).await {
                        Ok(reloaded) => reloaded,
                        Err(err) => {
                            drop(guard);
                            let rollback = write_text(&target, &before).await;
                            if rollback.is_ok() {
                                let _ = self.registry.write().await.reload_skill(&name).await;
                            }
                            return Err(ToolError::ExecutionFailed(format!(
                                "failed to reload skill after writing SKILL.md: {err}; rollback {}",
                                if rollback.is_ok() {
                                    "succeeded"
                                } else {
                                    "failed"
                                }
                            )));
                        }
                    };
                    drop(guard);
                    let after = read_text(&target).await?.unwrap_or_default();

                    let version_result = record_artifact_version(
                        &self.store,
                        &ctx.user_id,
                        "skill",
                        &artifact_name_for_skill(&name, &relative),
                        Some(parsed.manifest.version.clone()),
                        "applied",
                        Some(format!("{} applied via skill_manage", operation)),
                        Some(before),
                        Some(after),
                        learning_policy::skill_manage_provenance(
                            &operation,
                            Some(&relative),
                            Some(&reloaded),
                        ),
                    )
                    .await;

                    return Ok(ToolOutput::success(
                        learning_policy::skill_manage_updated_output(
                            &reloaded,
                            SKILL_FILE_NAME,
                            version_result.is_ok(),
                            version_result.err(),
                        ),
                        start.elapsed(),
                    ));
                }

                let before = read_text(&target).await?.unwrap_or_default();
                write_text(&target, content).await?;
                let after = read_text(&target).await?.unwrap_or_default();

                let version_result = record_artifact_version(
                    &self.store,
                    &ctx.user_id,
                    "skill_file",
                    &artifact_name_for_skill(&name, &relative),
                    Some(Utc::now().to_rfc3339()),
                    "applied",
                    Some(format!("{} applied via skill_manage", operation)),
                    Some(before),
                    Some(after),
                    learning_policy::skill_manage_provenance(&operation, Some(&relative), None),
                )
                .await;

                return Ok(ToolOutput::success(
                    learning_policy::skill_manage_updated_output(
                        &name,
                        &relative,
                        version_result.is_ok(),
                        version_result.err(),
                    ),
                    start.elapsed(),
                ));
            }
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "unknown operation: {}",
                    other
                )));
            }
        }
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// learning_status
// ─────────────────────────────────────────────────────────────────────────────

pub struct LearningStatusTool {
    orchestrator: Arc<LearningOrchestrator>,
    store: Arc<dyn Database>,
}

impl LearningStatusTool {
    pub fn new(orchestrator: Arc<LearningOrchestrator>, store: Arc<dyn Database>) -> Self {
        Self {
            orchestrator,
            store,
        }
    }
}

#[async_trait]
impl Tool for LearningStatusTool {
    fn name(&self) -> &str {
        "learning_status"
    }

    fn description(&self) -> &str {
        "Summarize learning settings, provider health, and recent learning activity."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        learning_policy::learning_status_parameters_schema()
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let settings = self.orchestrator.load_settings_for_user(&ctx.user_id).await;
        let providers = self.orchestrator.provider_health(&ctx.user_id).await;

        let store = Arc::clone(&self.store);
        let user_id = ctx.user_id.clone();
        let events_fut = store.list_learning_events(&user_id, None, None, None, 5);
        let evals_fut = store.list_learning_evaluations(&user_id, 5);
        let candidates_fut = store.list_learning_candidates(&user_id, None, None, 5);
        let versions_fut = store.list_learning_artifact_versions(&user_id, None, None, 5);
        let feedback_fut = store.list_learning_feedback(&user_id, None, None, 5);
        let rollbacks_fut = store.list_learning_rollbacks(&user_id, None, None, 5);
        let proposals_fut = store.list_learning_code_proposals(&user_id, None, 5);
        let outcome_stats_fut = store.outcome_summary_stats(&user_id);
        let outcome_query = DbOutcomeContractQuery {
            user_id: user_id.clone(),
            actor_id: None,
            status: None,
            contract_type: None,
            source_kind: None,
            source_id: None,
            thread_id: None,
            limit: 5,
        };
        let outcomes_fut = store.list_outcome_contracts(&outcome_query);

        let (
            events,
            evaluations,
            candidates,
            artifact_versions,
            feedback,
            rollbacks,
            proposals,
            outcome_stats,
            outcome_contracts,
        ) = tokio::try_join!(
            events_fut,
            evals_fut,
            candidates_fut,
            versions_fut,
            feedback_fut,
            rollbacks_fut,
            proposals_fut,
            outcome_stats_fut,
            outcomes_fut,
        )
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;

        let summary = learning_policy::learning_status_output(
            learning_policy::serialize_value(&settings),
            learning_policy::serialize_value(providers),
            settings.enabled && settings.outcomes.enabled,
            learning_policy::serialize_value(outcome_stats),
            learning_policy::recent_items_output(outcome_contracts),
            learning_policy::learning_recent_activity_output(
                learning_policy::recent_items_output(events),
                learning_policy::recent_items_output(evaluations),
                learning_policy::recent_items_output(candidates),
                learning_policy::recent_items_output(artifact_versions),
                learning_policy::recent_items_output(feedback),
                learning_policy::recent_items_output(rollbacks),
                learning_policy::recent_items_output(proposals),
            ),
        );

        Ok(ToolOutput::success(summary, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// learning_outcomes
// ─────────────────────────────────────────────────────────────────────────────

pub struct LearningOutcomesTool {
    store: Arc<dyn Database>,
}

impl LearningOutcomesTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for LearningOutcomesTool {
    fn name(&self) -> &str {
        "learning_outcomes"
    }

    fn description(&self) -> &str {
        "Inspect outcome-backed learning contracts and their observations."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        learning_policy::learning_outcomes_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let parsed = learning_policy::parse_learning_outcomes_params(&params)?;
        if let Some(contract_id) = parsed.contract_id {
            let contract = self
                .store
                .get_outcome_contract(&ctx.user_id, contract_id)
                .await
                .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?
                .ok_or_else(|| {
                    ToolError::ExecutionFailed(format!(
                        "Outcome contract '{}' not found",
                        contract_id
                    ))
                })?;
            let observations = self
                .store
                .list_outcome_observations(contract_id)
                .await
                .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
            return Ok(ToolOutput::success(
                learning_policy::learning_contract_detail_output(contract, observations),
                start.elapsed(),
            ));
        }

        let contracts = self
            .store
            .list_outcome_contracts(&DbOutcomeContractQuery {
                user_id: ctx.user_id.clone(),
                actor_id: None,
                status: parsed.status,
                contract_type: parsed.contract_type,
                source_kind: parsed.source_kind,
                source_id: None,
                thread_id: parsed.thread_id,
                limit: parsed.limit,
            })
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;

        Ok(ToolOutput::success(
            learning_policy::learning_items_output(contracts),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// learning_history
// ─────────────────────────────────────────────────────────────────────────────

pub struct LearningHistoryTool {
    store: Arc<dyn Database>,
}

impl LearningHistoryTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for LearningHistoryTool {
    fn name(&self) -> &str {
        "learning_history"
    }

    fn description(&self) -> &str {
        "Inspect stored learning events, candidates, artifact versions, feedback, rollbacks, and proposals."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        learning_policy::learning_history_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let parsed = learning_policy::parse_learning_history_params(&params);
        let kind = parsed.kind;
        let limit = parsed.limit;

        let output = match learning_policy::learning_history_kind(&kind) {
            learning_policy::LearningHistoryKind::Events => {
                learning_policy::learning_history_single_output(
                    &kind,
                    self.store
                        .list_learning_events(&ctx.user_id, None, None, None, limit)
                        .await
                        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?,
                )
            }
            learning_policy::LearningHistoryKind::Evaluations => {
                learning_policy::learning_history_single_output(
                    &kind,
                    self.store
                        .list_learning_evaluations(&ctx.user_id, limit)
                        .await
                        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?,
                )
            }
            learning_policy::LearningHistoryKind::Candidates => {
                learning_policy::learning_history_single_output(
                    &kind,
                    self.store
                        .list_learning_candidates(&ctx.user_id, None, None, limit)
                        .await
                        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?,
                )
            }
            learning_policy::LearningHistoryKind::ArtifactVersions => {
                learning_policy::learning_history_single_output(
                    &kind,
                    self.store
                        .list_learning_artifact_versions(&ctx.user_id, None, None, limit)
                        .await
                        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?,
                )
            }
            learning_policy::LearningHistoryKind::Feedback => {
                learning_policy::learning_history_single_output(
                    &kind,
                    self.store
                        .list_learning_feedback(&ctx.user_id, None, None, limit)
                        .await
                        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?,
                )
            }
            learning_policy::LearningHistoryKind::Rollbacks => {
                learning_policy::learning_history_single_output(
                    &kind,
                    self.store
                        .list_learning_rollbacks(&ctx.user_id, None, None, limit)
                        .await
                        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?,
                )
            }
            learning_policy::LearningHistoryKind::CodeProposals => {
                learning_policy::learning_history_single_output(
                    &kind,
                    self.store
                        .list_learning_code_proposals(&ctx.user_id, None, limit)
                        .await
                        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?,
                )
            }
            learning_policy::LearningHistoryKind::All => {
                let (
                    events,
                    evaluations,
                    candidates,
                    artifact_versions,
                    feedback,
                    rollbacks,
                    proposals,
                ) = tokio::try_join!(
                    self.store
                        .list_learning_events(&ctx.user_id, None, None, None, limit),
                    self.store.list_learning_evaluations(&ctx.user_id, limit),
                    self.store
                        .list_learning_candidates(&ctx.user_id, None, None, limit),
                    self.store
                        .list_learning_artifact_versions(&ctx.user_id, None, None, limit),
                    self.store
                        .list_learning_feedback(&ctx.user_id, None, None, limit),
                    self.store
                        .list_learning_rollbacks(&ctx.user_id, None, None, limit),
                    self.store
                        .list_learning_code_proposals(&ctx.user_id, None, limit),
                )
                .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
                learning_policy::learning_history_all_output(
                    &kind,
                    events,
                    evaluations,
                    candidates,
                    artifact_versions,
                    feedback,
                    rollbacks,
                    proposals,
                )
            }
        };

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// learning_feedback
// ─────────────────────────────────────────────────────────────────────────────

pub struct LearningFeedbackTool {
    orchestrator: Arc<LearningOrchestrator>,
}

impl LearningFeedbackTool {
    pub fn new(orchestrator: Arc<LearningOrchestrator>) -> Self {
        Self { orchestrator }
    }
}

#[async_trait]
impl Tool for LearningFeedbackTool {
    fn name(&self) -> &str {
        "learning_feedback"
    }

    fn description(&self) -> &str {
        "Record feedback on a learning target such as a candidate or proposal."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        learning_policy::learning_feedback_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let parsed = learning_policy::parse_learning_feedback_params(&params)?;

        let id = self
            .orchestrator
            .submit_feedback(
                &ctx.user_id,
                &parsed.target_type,
                &parsed.target_id,
                &parsed.verdict,
                parsed.note.as_deref(),
                parsed.metadata.as_ref(),
            )
            .await
            .map_err(ToolError::ExecutionFailed)?;

        Ok(ToolOutput::success(
            learning_policy::learning_feedback_output(
                id,
                &parsed.target_type,
                &parsed.target_id,
                &parsed.verdict,
            ),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// learning_proposal_review
// ─────────────────────────────────────────────────────────────────────────────

pub struct LearningProposalReviewTool {
    orchestrator: Arc<LearningOrchestrator>,
}

impl LearningProposalReviewTool {
    pub fn new(orchestrator: Arc<LearningOrchestrator>) -> Self {
        Self { orchestrator }
    }
}

#[async_trait]
impl Tool for LearningProposalReviewTool {
    fn name(&self) -> &str {
        "learning_proposal_review"
    }

    fn description(&self) -> &str {
        "Approve or reject a learning code proposal and return the updated proposal record."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        learning_policy::learning_proposal_review_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let parsed = learning_policy::parse_learning_proposal_review_params(&params)?;

        let proposal = self
            .orchestrator
            .review_code_proposal(
                &ctx.user_id,
                parsed.proposal_id,
                &parsed.decision,
                parsed.note.as_deref(),
            )
            .await
            .map_err(ToolError::ExecutionFailed)?;

        Ok(ToolOutput::success(
            learning_policy::learning_proposal_review_result(
                parsed.proposal_id,
                proposal,
                |proposal| proposal.status.clone(),
            )?,
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use learning_policy::{
        append_markdown_section, remove_markdown_section, upsert_markdown_section,
    };

    #[test]
    fn upsert_section_replaces_existing_block() {
        let source = "# Root\n\n## Preferences\nold\n\n## Next\nstay\n";
        let updated = upsert_markdown_section(source, "Preferences", "new");
        assert!(updated.contains("## Preferences\nnew"));
        assert!(updated.contains("## Next\nstay"));
        assert!(!updated.contains("old"));
    }

    #[test]
    fn append_section_adds_new_block() {
        let source = "# Root\n\nBody\n";
        let updated = append_markdown_section(source, "Habits", "- concise");
        assert!(updated.contains("## Habits\n- concise"));
    }

    #[test]
    fn remove_section_drops_target_only() {
        let source = "# Root\n\n## A\none\n\n## B\ntwo\n";
        let updated = remove_markdown_section(source, "A").expect("section A should exist");
        assert!(!updated.contains("## A"));
        assert!(updated.contains("## B\ntwo"));
    }

    #[test]
    fn prompt_manage_is_blocked_for_skill_restricted_contexts() {
        let mut ctx = JobContext::with_user("learning-test", "chat", "prompt gate");
        ctx.metadata = serde_json::json!({
            "allowed_skills": ["github"]
        });

        let err = validate_prompt_manage_available(&ctx).expect_err("should block");
        assert!(matches!(err, ToolError::NotAuthorized(_)));
    }

    #[test]
    fn prompt_manage_respects_learning_settings_gate() {
        let mut settings = LearningSettings::default();
        settings.prompt_mutation.enabled = false;

        let err = validate_prompt_manage_settings(&settings).expect_err("should block");
        assert!(matches!(err, ToolError::ExecutionFailed(_)));
    }
}
