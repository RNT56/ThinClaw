//! Learning-tool suite for prompt and skill mutation plus learning ledger access.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::agent::learning::LearningOrchestrator;
use crate::context::JobContext;
use crate::db::Database;
use crate::history::LearningArtifactVersion as DbLearningArtifactVersion;
use crate::skills::{
    MAX_PROMPT_FILE_SIZE, SkillSource, normalize_line_endings,
    parser::parse_skill_md,
    registry::{SkillRegistry, SkillRegistryError, check_gating},
};
use crate::tools::ToolRegistry;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};
use crate::workspace::{Workspace, paths};

const PROMPT_TARGETS: &[&str] = &[paths::SOUL, paths::AGENTS, paths::USER];
const SKILL_FILE_NAME: &str = "SKILL.md";

fn tool_error_from_skill(err: SkillRegistryError) -> ToolError {
    ToolError::ExecutionFailed(err.to_string())
}

fn normalize_prompt_target(target: &str) -> Result<&'static str, ToolError> {
    let trimmed = target.trim().trim_start_matches('/');
    PROMPT_TARGETS
        .iter()
        .copied()
        .find(|candidate| trimmed.eq_ignore_ascii_case(candidate))
        .ok_or_else(|| {
            ToolError::InvalidParameters(format!(
                "target must be one of: {}, got '{}'",
                PROMPT_TARGETS.join(", "),
                target
            ))
        })
}

fn validate_prompt_content(content: &str) -> Result<(), ToolError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(ToolError::InvalidParameters(
            "prompt content cannot be empty".to_string(),
        ));
    }
    if !trimmed.contains('#') {
        return Err(ToolError::InvalidParameters(
            "prompt content must include markdown headings".to_string(),
        ));
    }
    let lowered = trimmed.to_ascii_lowercase();
    let suspicious_markers = ["role: user", "role: assistant", "tool_result", "<tool_call"];
    if suspicious_markers
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        return Err(ToolError::InvalidParameters(
            "prompt content appears to include transcript/tool residue".to_string(),
        ));
    }
    Ok(())
}

fn validate_prompt_safety(target: &str, content: &str) -> Result<(), ToolError> {
    let lowered = content.to_ascii_lowercase();
    let required_markers: &[&str] = match target {
        paths::SOUL => &["boundar", "ask before", "private"],
        paths::AGENTS => &["red lines", "ask first", "don't"],
        _ => &[],
    };
    if required_markers.is_empty() {
        return Ok(());
    }
    if required_markers
        .iter()
        .all(|marker| !lowered.contains(marker))
    {
        return Err(ToolError::InvalidParameters(format!(
            "{} update rejected: core safety guidance appears to be missing",
            target
        )));
    }
    Ok(())
}

fn normalize_heading_name(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('#')
        .trim()
        .to_ascii_lowercase()
}

fn parse_markdown_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if level == 0 {
        return None;
    }
    let title = trimmed[level..].trim();
    if title.is_empty() {
        return None;
    }
    Some((level, title.to_string()))
}

fn find_section_byte_range(doc: &str, heading_name: &str) -> Option<(usize, usize, usize, String)> {
    let target = normalize_heading_name(heading_name);
    let mut offset = 0usize;
    let mut start: Option<(usize, usize, usize, String)> = None;

    for line in doc.split_inclusive('\n') {
        let line_start = offset;
        let line_end = offset + line.len();
        offset = line_end;

        if let Some((level, title)) = parse_markdown_heading(line) {
            if let Some((start_offset, current_level, _, current_title)) = &start {
                if level <= *current_level {
                    return Some((
                        *start_offset,
                        line_start,
                        *current_level,
                        current_title.clone(),
                    ));
                }
            }

            if normalize_heading_name(&title) == target {
                start = Some((line_start, level, line_end, title));
            }
        }
    }

    start.map(|(start_offset, level, _, title)| (start_offset, doc.len(), level, title))
}

fn upsert_markdown_section(doc: &str, heading: &str, section_content: &str) -> String {
    let normalized_content = section_content.trim();
    let body = if normalized_content.is_empty() {
        String::new()
    } else {
        format!("\n{}\n", normalized_content)
    };

    if let Some((start, end, level, title)) = find_section_byte_range(doc, heading) {
        let heading_line = format!("{} {}", "#".repeat(level.max(1)), title.trim());
        let replacement = format!("{heading_line}{body}");
        let mut merged = String::with_capacity(doc.len() + replacement.len());
        merged.push_str(&doc[..start]);
        merged.push_str(replacement.trim_end_matches('\n'));
        merged.push('\n');
        merged.push_str(doc[end..].trim_start_matches('\n'));
        return merged.trim().to_string() + "\n";
    }

    let mut merged = doc.trim().to_string();
    if !merged.is_empty() {
        merged.push_str("\n\n");
    }
    merged.push_str(&format!("## {}\n", heading.trim()));
    if !normalized_content.is_empty() {
        merged.push_str(normalized_content);
        merged.push('\n');
    }
    merged
}

fn append_markdown_section(doc: &str, heading: &str, section_content: &str) -> String {
    let mut merged = doc.trim().to_string();
    if !merged.is_empty() {
        merged.push_str("\n\n");
    }
    merged.push_str(&format!("## {}\n", heading.trim()));
    let content = section_content.trim();
    if !content.is_empty() {
        merged.push_str(content);
        merged.push('\n');
    }
    merged
}

fn remove_markdown_section(doc: &str, heading: &str) -> Result<String, ToolError> {
    let Some((start, end, _, _)) = find_section_byte_range(doc, heading) else {
        return Err(ToolError::ExecutionFailed(format!(
            "section '{}' not found",
            heading
        )));
    };

    let mut merged = String::with_capacity(doc.len());
    merged.push_str(&doc[..start]);
    merged.push_str(doc[end..].trim_start_matches('\n'));
    Ok(merged.trim().to_string() + "\n")
}

fn validate_skill_admin_available(ctx: &JobContext, tool_name: &str) -> Result<(), ToolError> {
    if ToolRegistry::metadata_string_list(&ctx.metadata, "allowed_skills").is_some() {
        Err(ToolError::ExecutionFailed(format!(
            "Tool '{}' is not available when the current agent is restricted to a specific skill allowlist.",
            tool_name
        )))
    } else {
        Ok(())
    }
}

fn prompt_manage_user_target(scope: &str, ctx: &JobContext) -> Result<String, ToolError> {
    let actor_id = ctx
        .metadata
        .get("actor_id")
        .and_then(|v| v.as_str())
        .or_else(|| ctx.actor_id.as_deref());
    let conversation_kind = ctx
        .metadata
        .get("conversation_kind")
        .or_else(|| ctx.metadata.get("chat_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("direct")
        .to_ascii_lowercase();
    let is_group = matches!(
        conversation_kind.as_str(),
        "group" | "channel" | "supergroup"
    );

    match scope {
        "shared" => Ok(paths::USER.to_string()),
        "actor" => {
            let Some(actor_id) = actor_id else {
                return Err(ToolError::InvalidParameters(
                    "scope='actor' requires actor_id context".to_string(),
                ));
            };
            Ok(paths::actor_user(actor_id))
        }
        "auto" => {
            if !is_group && let Some(actor_id) = actor_id {
                return Ok(paths::actor_user(actor_id));
            }
            Ok(paths::USER.to_string())
        }
        other => Err(ToolError::InvalidParameters(format!(
            "unsupported scope '{}'; expected auto, actor, or shared",
            other
        ))),
    }
}

fn validate_relative_skill_path(path: &str) -> Result<PathBuf, ToolError> {
    let trimmed = path.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return Err(ToolError::InvalidParameters(
            "path cannot be empty".to_string(),
        ));
    }

    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err(ToolError::InvalidParameters(format!(
            "skill file path must be relative, got '{}'",
            path.display()
        )));
    }

    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => clean.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(ToolError::InvalidParameters(format!(
                    "skill file path '{}' must not contain path traversal components",
                    path.display()
                )));
            }
        }
    }

    if clean.as_os_str().is_empty() {
        return Err(ToolError::InvalidParameters(
            "path cannot resolve to an empty location".to_string(),
        ));
    }

    Ok(clean)
}

fn artifact_name_for_skill(skill_name: &str, path: &Path) -> String {
    let path_str = path.to_string_lossy();
    if path_str.eq_ignore_ascii_case(SKILL_FILE_NAME) {
        skill_name.to_string()
    } else {
        format!("{}/{}", skill_name, path_str)
    }
}

async fn read_text(path: &Path) -> Result<Option<String>, ToolError> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => Ok(Some(content)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(ToolError::ExecutionFailed(format!(
            "failed to read '{}': {}",
            path.display(),
            err
        ))),
    }
}

async fn write_text(path: &Path, content: &str) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            ToolError::ExecutionFailed(format!(
                "failed to create directories for '{}': {}",
                path.display(),
                err
            ))
        })?;
    }
    tokio::fs::write(path, content).await.map_err(|err| {
        ToolError::ExecutionFailed(format!("failed to write '{}': {}", path.display(), err))
    })
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
    store
        .insert_learning_artifact_version(&version)
        .await
        .map_err(|err| err.to_string())
}

fn serialized<T: serde::Serialize>(value: T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or_else(|_| serde_json::json!({}))
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
        "Update SOUL.md, AGENTS.md, or USER.md in workspace memory. Run session_search + memory_search before mutation, then apply bounded prompt edits with validation and version recording."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["replace", "upsert_section", "append_section", "remove_section"],
                    "description": "Prompt mutation operation",
                    "default": "replace"
                },
                "target": {
                    "type": "string",
                    "enum": [paths::SOUL, paths::AGENTS, paths::USER],
                    "description": "Which prompt file to update"
                },
                "scope": {
                    "type": "string",
                    "enum": ["auto", "actor", "shared"],
                    "description": "USER.md scope behavior. auto = actor USER.md in direct chats, shared USER.md in groups.",
                    "default": "auto"
                },
                "content": {
                    "type": "string",
                    "description": "Replacement markdown content for operation=replace"
                },
                "heading": {
                    "type": "string",
                    "description": "Section heading for section-aware operations"
                },
                "section_content": {
                    "type": "string",
                    "description": "Section body for upsert_section or append_section"
                },
            },
            "required": ["target"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let settings = self.orchestrator.load_settings_for_user(&ctx.user_id).await;
        if !settings.prompt_mutation.enabled {
            return Err(ToolError::ExecutionFailed(
                "prompt mutation is disabled in the learning settings".to_string(),
            ));
        }

        let operation = params
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("replace")
            .to_ascii_lowercase();
        let target = normalize_prompt_target(require_str(&params, "target")?)?;
        let scope = params
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("auto")
            .to_ascii_lowercase();
        if target != paths::USER && scope != "auto" {
            return Err(ToolError::InvalidParameters(
                "scope is only supported for target='USER.md'".to_string(),
            ));
        }
        let resolved_target = if target == paths::USER {
            prompt_manage_user_target(&scope, ctx)?
        } else {
            target.to_string()
        };
        let before = self
            .workspace
            .read(&resolved_target)
            .await
            .ok()
            .map(|doc| doc.content)
            .unwrap_or_default();

        let next_content = match operation.as_str() {
            "replace" => require_str(&params, "content")?.to_string(),
            "upsert_section" => {
                let heading = require_str(&params, "heading")?;
                let section_content = params
                    .get("section_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                upsert_markdown_section(&before, heading, section_content)
            }
            "append_section" => {
                let heading = require_str(&params, "heading")?;
                let section_content = params
                    .get("section_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                append_markdown_section(&before, heading, section_content)
            }
            "remove_section" => {
                let heading = require_str(&params, "heading")?;
                remove_markdown_section(&before, heading)?
            }
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "unknown prompt_manage operation '{}'",
                    other
                )));
            }
        };
        validate_prompt_content(&next_content)?;
        validate_prompt_safety(target, &next_content)?;

        self.workspace
            .write(&resolved_target, &next_content)
            .await
            .map_err(|err| {
                ToolError::ExecutionFailed(format!(
                    "failed to update '{}': {}",
                    resolved_target, err
                ))
            })?;
        let after = self
            .workspace
            .read(&resolved_target)
            .await
            .ok()
            .map(|doc| doc.content)
            .unwrap_or_default();

        let version_label = Some(Utc::now().to_rfc3339());
        let provenance = serde_json::json!({
            "tool": "prompt_manage",
            "target": target,
            "resolved_target": resolved_target.clone(),
            "scope": scope.clone(),
            "user_id": ctx.user_id,
        });
        let version_result = record_artifact_version(
            &self.store,
            &ctx.user_id,
            "prompt",
            &resolved_target,
            version_label.clone(),
            "applied",
            Some(format!("prompt {} via prompt_manage", operation)),
            Some(before),
            Some(after),
            provenance,
        )
        .await;

        let result = serde_json::json!({
            "status": "updated",
            "operation": operation,
            "target": resolved_target,
            "bytes_written": next_content.len(),
            "version_label": version_label,
            "artifact_version_recorded": version_result.is_ok(),
            "artifact_version_error": version_result.err(),
        });
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["create", "patch", "edit", "delete", "write_file", "remove_file", "reload"],
                    "description": "What to do with the skill"
                },
                "name": {
                    "type": "string",
                    "description": "Skill name"
                },
                "path": {
                    "type": "string",
                    "description": "Relative file path inside the skill directory (defaults to SKILL.md)",
                    "default": "SKILL.md"
                },
                "content": {
                    "type": "string",
                    "description": "New file content for create/write/edit/patch operations"
                },
                "all": {
                    "type": "boolean",
                    "description": "When operation=reload, reload every skill instead of one",
                    "default": false
                }
            },
            "required": ["operation", "name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        validate_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let operation = require_str(&params, "operation")?.to_ascii_lowercase();
        let name = require_str(&params, "name")?;
        let path_value = params
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(SKILL_FILE_NAME);
        let all = params.get("all").and_then(|v| v.as_bool()).unwrap_or(false);

        match operation.as_str() {
            "reload" => {
                let mut guard = self.registry.write().await;
                if all {
                    let loaded = guard.reload().await;
                    return Ok(ToolOutput::success(
                        serde_json::json!({
                            "status": "reloaded_all",
                            "skills": loaded,
                            "count": loaded.len(),
                        }),
                        start.elapsed(),
                    ));
                }

                let reloaded = guard.reload_skill(name).await.map_err(|err| {
                    ToolError::ExecutionFailed(format!(
                        "failed to reload skill '{}': {}",
                        name, err
                    ))
                })?;
                return Ok(ToolOutput::success(
                    serde_json::json!({
                        "status": "reloaded",
                        "name": reloaded,
                    }),
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
                if !parsed.manifest.name.eq_ignore_ascii_case(name) {
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
                let existed_already = tokio::fs::try_exists(install_dir.join(name))
                    .await
                    .unwrap_or(false);
                if existed_already {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Skill '{}' already exists; use edit/write_file instead",
                        name
                    )));
                }

                let (skill_name, loaded_skill) =
                    SkillRegistry::prepare_install_to_disk(&install_dir, name, &normalized)
                        .await
                        .map_err(tool_error_from_skill)?;

                let commit_result = {
                    let mut guard = self.registry.write().await;
                    guard.commit_install(&skill_name, loaded_skill)
                };
                if let Err(err) = commit_result {
                    let cleanup_path = install_dir.join(name);
                    let _ = SkillRegistry::delete_skill_files(&cleanup_path).await;
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
                    serde_json::json!({
                        "tool": "skill_manage",
                        "agent_generated": true,
                        "operation": "create",
                        "path": SKILL_FILE_NAME,
                    }),
                )
                .await;

                return Ok(ToolOutput::success(
                    serde_json::json!({
                        "status": "created",
                        "name": skill_name,
                        "path": SKILL_FILE_NAME,
                        "artifact_version_recorded": version_result.is_ok(),
                        "artifact_version_error": version_result.err(),
                    }),
                    start.elapsed(),
                ));
            }
            "delete" => {
                let mut guard = self.registry.write().await;
                let skill_path = guard.validate_remove(name).map_err(tool_error_from_skill)?;
                let before_content = read_text(&skill_path.join(SKILL_FILE_NAME)).await?;
                SkillRegistry::delete_skill_files(&skill_path)
                    .await
                    .map_err(tool_error_from_skill)?;
                guard.commit_remove(name).map_err(tool_error_from_skill)?;

                let version_result = record_artifact_version(
                    &self.store,
                    &ctx.user_id,
                    "skill",
                    name,
                    Some(Utc::now().to_rfc3339()),
                    "deleted",
                    Some("skill deleted via skill_manage".to_string()),
                    before_content,
                    None,
                    serde_json::json!({
                        "tool": "skill_manage",
                        "agent_generated": true,
                        "operation": "delete",
                    }),
                )
                .await;

                return Ok(ToolOutput::success(
                    serde_json::json!({
                        "status": "deleted",
                        "name": name,
                        "artifact_version_recorded": version_result.is_ok(),
                        "artifact_version_error": version_result.err(),
                    }),
                    start.elapsed(),
                ));
            }
            "remove_file" => {
                let relative = validate_relative_skill_path(path_value)?;
                if relative
                    .to_string_lossy()
                    .eq_ignore_ascii_case(SKILL_FILE_NAME)
                {
                    return Err(ToolError::InvalidParameters(
                        "remove_file cannot delete SKILL.md; use operation='delete' instead"
                            .to_string(),
                    ));
                }

                let root = loaded_skill_root(&self.registry, name).await?;
                let target = root.0.join(&relative);
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
                    &artifact_name_for_skill(name, &relative),
                    Some(Utc::now().to_rfc3339()),
                    "deleted",
                    Some("skill file removed via skill_manage".to_string()),
                    Some(before),
                    None,
                    serde_json::json!({
                        "tool": "skill_manage",
                        "agent_generated": true,
                        "operation": "remove_file",
                        "path": relative,
                    }),
                )
                .await;

                return Ok(ToolOutput::success(
                    serde_json::json!({
                        "status": "removed_file",
                        "name": name,
                        "path": relative,
                        "artifact_version_recorded": version_result.is_ok(),
                        "artifact_version_error": version_result.err(),
                    }),
                    start.elapsed(),
                ));
            }
            "write_file" | "edit" | "patch" => {
                let content = require_str(&params, "content")?;
                let relative = validate_relative_skill_path(path_value)?;
                let root = loaded_skill_root(&self.registry, name).await?;
                let target = root.0.join(&relative);

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
                    if !parsed.manifest.name.eq_ignore_ascii_case(name) {
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
                    let reloaded = guard.reload_skill(name).await.map_err(|err| {
                        ToolError::ExecutionFailed(format!(
                            "failed to reload skill after writing SKILL.md: {}",
                            err
                        ))
                    })?;
                    let after = read_text(&target).await?.unwrap_or_default();

                    let version_result = record_artifact_version(
                        &self.store,
                        &ctx.user_id,
                        "skill",
                        &artifact_name_for_skill(name, &relative),
                        Some(parsed.manifest.version.clone()),
                        "applied",
                        Some(format!("{} applied via skill_manage", operation)),
                        Some(before),
                        Some(after),
                        serde_json::json!({
                            "tool": "skill_manage",
                            "agent_generated": true,
                            "operation": operation,
                            "reloaded_name": reloaded,
                            "path": relative,
                        }),
                    )
                    .await;

                    return Ok(ToolOutput::success(
                        serde_json::json!({
                            "status": "updated",
                            "name": reloaded,
                            "path": SKILL_FILE_NAME,
                            "artifact_version_recorded": version_result.is_ok(),
                            "artifact_version_error": version_result.err(),
                        }),
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
                    &artifact_name_for_skill(name, &relative),
                    Some(Utc::now().to_rfc3339()),
                    "applied",
                    Some(format!("{} applied via skill_manage", operation)),
                    Some(before),
                    Some(after),
                    serde_json::json!({
                        "tool": "skill_manage",
                        "agent_generated": true,
                        "operation": operation,
                        "path": relative,
                    }),
                )
                .await;

                return Ok(ToolOutput::success(
                    serde_json::json!({
                        "status": "updated",
                        "name": name,
                        "path": relative,
                        "artifact_version_recorded": version_result.is_ok(),
                        "artifact_version_error": version_result.err(),
                    }),
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
        serde_json::json!({"type": "object", "properties": {}})
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

        let (events, evaluations, candidates, artifact_versions, feedback, rollbacks, proposals) =
            tokio::try_join!(
                events_fut,
                evals_fut,
                candidates_fut,
                versions_fut,
                feedback_fut,
                rollbacks_fut,
                proposals_fut,
            )
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;

        let summary = serde_json::json!({
            "settings": serialized(&settings),
            "provider_health": providers,
            "recent_activity": {
                "events": summarize_recent(events),
                "evaluations": summarize_recent(evaluations),
                "candidates": summarize_recent(candidates),
                "artifact_versions": summarize_recent(artifact_versions),
                "feedback": summarize_recent(feedback),
                "rollbacks": summarize_recent(rollbacks),
                "code_proposals": summarize_recent(proposals),
            }
        });

        Ok(ToolOutput::success(summary, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

fn summarize_recent<T: serde::Serialize>(items: Vec<T>) -> serde_json::Value {
    serde_json::json!({
        "count": items.len(),
        "items": items,
    })
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["all", "events", "evaluations", "candidates", "artifact_versions", "feedback", "rollbacks", "code_proposals"],
                    "default": "all"
                },
                "limit": {
                    "type": "integer",
                    "default": 20,
                    "minimum": 1,
                    "maximum": 100
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let kind = params
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("all")
            .to_ascii_lowercase();
        let limit = params
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(20)
            .clamp(1, 100);

        let output = match kind.as_str() {
            "events" => serde_json::json!({
                "kind": kind,
                "items": serialized(self.store.list_learning_events(&ctx.user_id, None, None, None, limit).await.map_err(|err| ToolError::ExecutionFailed(err.to_string()))?),
            }),
            "evaluations" => serde_json::json!({
                "kind": kind,
                "items": serialized(self.store.list_learning_evaluations(&ctx.user_id, limit).await.map_err(|err| ToolError::ExecutionFailed(err.to_string()))?),
            }),
            "candidates" => serde_json::json!({
                "kind": kind,
                "items": serialized(self.store.list_learning_candidates(&ctx.user_id, None, None, limit).await.map_err(|err| ToolError::ExecutionFailed(err.to_string()))?),
            }),
            "artifact_versions" => serde_json::json!({
                "kind": kind,
                "items": serialized(self.store.list_learning_artifact_versions(&ctx.user_id, None, None, limit).await.map_err(|err| ToolError::ExecutionFailed(err.to_string()))?),
            }),
            "feedback" => serde_json::json!({
                "kind": kind,
                "items": serialized(self.store.list_learning_feedback(&ctx.user_id, None, None, limit).await.map_err(|err| ToolError::ExecutionFailed(err.to_string()))?),
            }),
            "rollbacks" => serde_json::json!({
                "kind": kind,
                "items": serialized(self.store.list_learning_rollbacks(&ctx.user_id, None, None, limit).await.map_err(|err| ToolError::ExecutionFailed(err.to_string()))?),
            }),
            "code_proposals" => serde_json::json!({
                "kind": kind,
                "items": serialized(self.store.list_learning_code_proposals(&ctx.user_id, None, limit).await.map_err(|err| ToolError::ExecutionFailed(err.to_string()))?),
            }),
            _ => {
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
                serde_json::json!({
                    "kind": kind,
                    "events": serialized(events),
                    "evaluations": serialized(evaluations),
                    "candidates": serialized(candidates),
                    "artifact_versions": serialized(artifact_versions),
                    "feedback": serialized(feedback),
                    "rollbacks": serialized(rollbacks),
                    "code_proposals": serialized(proposals),
                })
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "target_type": {
                    "type": "string",
                    "description": "Type of target (for example: candidate, code_proposal, prompt, skill)"
                },
                "target_id": {
                    "type": "string",
                    "description": "Identifier for the target"
                },
                "verdict": {
                    "type": "string",
                    "description": "Feedback verdict (for example: helpful, harmful, reject, dont_learn)"
                },
                "note": {
                    "type": "string",
                    "description": "Optional note explaining the verdict"
                },
                "metadata": {
                    "type": "object",
                    "description": "Optional extra metadata"
                }
            },
            "required": ["target_type", "target_id", "verdict"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let target_type = require_str(&params, "target_type")?;
        let target_id = require_str(&params, "target_id")?;
        let verdict = require_str(&params, "verdict")?;
        let note = params.get("note").and_then(|v| v.as_str());
        let metadata = params.get("metadata");

        let id = self
            .orchestrator
            .submit_feedback(
                &ctx.user_id,
                target_type,
                target_id,
                verdict,
                note,
                metadata,
            )
            .await
            .map_err(ToolError::ExecutionFailed)?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "status": "recorded",
                "id": id,
                "target_type": target_type,
                "target_id": target_id,
                "verdict": verdict,
            }),
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "proposal_id": {
                    "type": "string",
                    "description": "UUID of the learning code proposal"
                },
                "decision": {
                    "type": "string",
                    "enum": ["approve", "reject"],
                    "description": "Review decision"
                },
                "note": {
                    "type": "string",
                    "description": "Optional reviewer note"
                }
            },
            "required": ["proposal_id", "decision"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let proposal_id = Uuid::parse_str(require_str(&params, "proposal_id")?)
            .map_err(|err| ToolError::InvalidParameters(format!("invalid proposal_id: {}", err)))?;
        let decision = require_str(&params, "decision")?;
        let note = params.get("note").and_then(|v| v.as_str());

        let proposal = self
            .orchestrator
            .review_code_proposal(&ctx.user_id, proposal_id, decision, note)
            .await
            .map_err(ToolError::ExecutionFailed)?;

        let Some(proposal) = proposal else {
            return Err(ToolError::ExecutionFailed(format!(
                "proposal '{}' was not found",
                proposal_id
            )));
        };

        Ok(ToolOutput::success(
            serde_json::json!({
                "status": proposal.status,
                "proposal": serialized(proposal),
            }),
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
}
