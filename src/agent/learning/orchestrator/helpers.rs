use super::*;
pub(in crate::agent::learning) use thinclaw_agent::learning_policy::{
    LearningRouteAction, LearningRoutePolicy, auto_apply_allowed_for_class,
    build_code_proposal_fields, code_proposal_review_metadata, ensure_auto_apply_class,
    evaluate_learning_event, generated_skill_feedback_polarity,
    generated_skill_lifecycle_for_reuse, generated_skill_transition_entry,
    generated_skill_triggers, generated_workflow_digest, is_negative_learning_feedback_verdict,
    is_prompt_target_supported, materialize_prompt_candidate_content, proposal_fingerprint,
    rejected_code_proposal_suppression_message, render_generated_skill_markdown,
    route_learning_candidate, safe_mode_should_trip, stable_json_hash, validate_prompt_content,
    validate_prompt_target_content,
};

pub(in crate::agent::learning) fn updated_generated_skill_proposal(
    candidate: &DbLearningCandidate,
    lifecycle: GeneratedSkillLifecycle,
    activation_reason: Option<&str>,
    feedback_verdict: Option<&str>,
    feedback_note: Option<&str>,
    artifact_version_id: Option<Uuid>,
    transition_at: DateTime<Utc>,
) -> serde_json::Value {
    thinclaw_agent::learning_policy::updated_generated_skill_proposal(
        &candidate.proposal,
        lifecycle,
        activation_reason,
        feedback_verdict,
        feedback_note,
        artifact_version_id,
        transition_at,
    )
}

pub(in crate::agent::learning) fn synthesize_generated_skill_markdown(
    skill_name: &str,
    user_input: &str,
    tool_calls: &[crate::agent::session::TurnToolCall],
    lifecycle: GeneratedSkillLifecycle,
    reuse_count: u32,
    activation_reason: Option<String>,
) -> Result<String, String> {
    let content = render_generated_skill_markdown(
        skill_name,
        user_input,
        tool_calls,
        lifecycle,
        reuse_count,
        activation_reason,
    );
    crate::skills::parser::parse_skill_md(&crate::skills::normalize_line_endings(&content))
        .map_err(|err| err.to_string())?;
    Ok(content)
}

#[cfg(test)]
pub(in crate::agent::learning) fn classify_event(event: &DbLearningEvent) -> ImprovementClass {
    thinclaw_agent::learning_policy::classify_learning_event(&event.event_type, &event.payload)
}

pub(in crate::agent::learning) async fn read_prompt_target_content(
    workspace: Option<&Workspace>,
    target: &str,
) -> Result<String, String> {
    if target.eq_ignore_ascii_case(paths::SOUL) {
        return match crate::identity::soul_store::read_home_soul() {
            Ok(content) => Ok(content),
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => Ok(String::new()),
            Err(err) => Err(format!("failed to read canonical SOUL.md: {}", err)),
        };
    }

    let Some(workspace) = workspace else {
        return Err(format!(
            "workspace unavailable for prompt target '{}'",
            target
        ));
    };

    Ok(workspace
        .read(target)
        .await
        .ok()
        .map(|doc| doc.content)
        .unwrap_or_default())
}

pub(in crate::agent::learning) async fn write_prompt_target_content(
    workspace: Option<&Workspace>,
    target: &str,
    content: &str,
) -> Result<(), String> {
    if target.eq_ignore_ascii_case(paths::SOUL) {
        return crate::identity::soul_store::write_home_soul(content)
            .map_err(|err| format!("failed to update canonical SOUL.md: {}", err));
    }

    let Some(workspace) = workspace else {
        return Err(format!(
            "workspace unavailable for prompt target '{}'",
            target
        ));
    };

    workspace
        .write(target, content)
        .await
        .map(|_| ())
        .map_err(|err| format!("failed to update '{}': {}", target, err))
}

pub(in crate::agent::learning) async fn run_cmd(cmd: &mut Command) -> Result<String, String> {
    let output = cmd.output().await.map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        return Err(format!("command failed: {}", detail));
    }
    Ok(stdout)
}
