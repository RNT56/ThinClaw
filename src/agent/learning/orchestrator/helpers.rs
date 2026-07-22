use super::*;
pub(in crate::agent::learning) use thinclaw_agent::learning_policy::{
    LearningRouteAction, LearningRoutePolicy, SafeModeTripInput, auto_apply_allowed_for_class,
    build_code_proposal_fields, code_proposal_review_metadata, ensure_auto_apply_class,
    evaluate_learning_event, generated_skill_feedback_polarity,
    generated_skill_lifecycle_for_reuse, generated_skill_transition_entry,
    generated_skill_triggers, generated_workflow_digest, is_negative_learning_feedback_verdict,
    is_prompt_target_supported, materialize_prompt_candidate_content, proposal_fingerprint,
    rejected_code_proposal_suppression_message, render_generated_skill_markdown,
    route_learning_candidate, safe_mode_should_trip, stable_json_hash, validate_prompt_content,
    validate_prompt_target_content,
};

pub(in crate::agent::learning) const CANDIDATE_IDENTITY_CONTEXT_KEY: &str =
    "_thinclaw_identity_context";

/// Immutable authorization context copied from the trusted, top-level fields
/// of a persisted learning event. Proposal fields are model/event content and
/// must never be treated as authority on their own.
#[derive(Debug, Clone)]
pub(in crate::agent::learning) struct CandidateIdentityContext {
    pub principal_id: String,
    pub actor_id: String,
    pub channel: String,
    pub conversation_id: Option<Uuid>,
    pub conversation_kind: crate::identity::ConversationKind,
    pub conversation_scope_id: Uuid,
    pub stable_external_conversation_key: String,
}

impl CandidateIdentityContext {
    pub fn is_principal_owner(&self) -> bool {
        self.conversation_kind == crate::identity::ConversationKind::Direct
            && self.actor_id == self.principal_id
    }

    pub fn resolved_identity(&self) -> crate::identity::ResolvedIdentity {
        crate::identity::ResolvedIdentity {
            principal_id: self.principal_id.clone(),
            actor_id: self.actor_id.clone(),
            conversation_scope_id: self.conversation_scope_id,
            conversation_kind: self.conversation_kind,
            raw_sender_id: self.actor_id.clone(),
            stable_external_conversation_key: self.stable_external_conversation_key.clone(),
        }
    }
}

/// Preserve the candidate payload while overwriting a reserved field with
/// authorization data sourced from the event record rather than its payload.
pub(in crate::agent::learning) fn proposal_with_event_identity(
    event: &DbLearningEvent,
) -> serde_json::Value {
    let mut proposal = match event.payload.clone() {
        serde_json::Value::Object(fields) => fields,
        payload => serde_json::Map::from_iter([("payload".to_string(), payload)]),
    };
    let actor_id = event
        .actor_id
        .as_deref()
        .filter(|actor| !actor.trim().is_empty())
        .unwrap_or(&event.user_id);
    let conversation_kind = event
        .payload
        .get("conversation_kind")
        .and_then(|value| value.as_str())
        .and_then(crate::identity::parse_conversation_kind_hint)
        .unwrap_or(crate::identity::ConversationKind::Direct);
    let supplied_scope = event
        .payload
        .get("conversation_scope_id")
        .and_then(|value| value.as_str())
        .and_then(|value| Uuid::parse_str(value).ok());
    let conversation_scope_id = match conversation_kind {
        crate::identity::ConversationKind::Direct => {
            Some(crate::identity::direct_scope_id(&event.user_id, actor_id))
        }
        crate::identity::ConversationKind::Group => supplied_scope,
    };
    let channel = event
        .channel
        .as_deref()
        .filter(|channel| !channel.trim().is_empty())
        .unwrap_or("learning");
    let stable_external_conversation_key = event
        .payload
        .get("stable_external_conversation_key")
        .and_then(|value| value.as_str())
        .unwrap_or_default();

    proposal.insert(
        CANDIDATE_IDENTITY_CONTEXT_KEY.to_string(),
        serde_json::json!({
            "version": 1,
            "principal_id": event.user_id,
            "actor_id": actor_id,
            "channel": channel,
            "conversation_id": event.conversation_id,
            "conversation_kind": conversation_kind.as_str(),
            "conversation_scope_id": conversation_scope_id,
            "stable_external_conversation_key": stable_external_conversation_key,
        }),
    );
    serde_json::Value::Object(proposal)
}

pub(in crate::agent::learning) fn proposal_with_resolved_identity(
    proposal: serde_json::Value,
    identity: &crate::identity::ResolvedIdentity,
    channel: &str,
    conversation_id: Option<Uuid>,
) -> serde_json::Value {
    let mut proposal = match proposal {
        serde_json::Value::Object(fields) => fields,
        payload => serde_json::Map::from_iter([("payload".to_string(), payload)]),
    };
    proposal.insert(
        CANDIDATE_IDENTITY_CONTEXT_KEY.to_string(),
        serde_json::json!({
            "version": 1,
            "principal_id": identity.principal_id,
            "actor_id": identity.actor_id,
            "channel": channel,
            "conversation_id": conversation_id,
            "conversation_kind": identity.conversation_kind.as_str(),
            "conversation_scope_id": identity.conversation_scope_id,
            "stable_external_conversation_key": identity.stable_external_conversation_key,
        }),
    );
    serde_json::Value::Object(proposal)
}

pub(in crate::agent::learning) fn candidate_identity_context(
    candidate: &DbLearningCandidate,
) -> Result<CandidateIdentityContext, String> {
    let context = candidate
        .proposal
        .get(CANDIDATE_IDENTITY_CONTEXT_KEY)
        .and_then(|value| value.as_object())
        .ok_or_else(|| {
            "learning candidate has no authoritative identity context; manual review is required"
                .to_string()
        })?;
    if context.get("version").and_then(|value| value.as_u64()) != Some(1) {
        return Err("learning candidate identity context version is unsupported".to_string());
    }
    let principal_id = context
        .get("principal_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "learning candidate identity is missing a principal".to_string())?;
    if principal_id != candidate.user_id {
        return Err("learning candidate principal does not match its identity context".to_string());
    }
    let actor_id = context
        .get("actor_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "learning candidate identity is missing an actor".to_string())?;
    let channel = context
        .get("channel")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("learning");
    let conversation_id = context
        .get("conversation_id")
        .and_then(|value| value.as_str())
        .and_then(|value| Uuid::parse_str(value).ok());
    let conversation_kind = context
        .get("conversation_kind")
        .and_then(|value| value.as_str())
        .and_then(crate::identity::parse_conversation_kind_hint)
        .ok_or_else(|| "learning candidate identity has no valid conversation kind".to_string())?;
    let supplied_scope = context
        .get("conversation_scope_id")
        .and_then(|value| value.as_str())
        .and_then(|value| Uuid::parse_str(value).ok());
    let conversation_scope_id = match conversation_kind {
        crate::identity::ConversationKind::Direct => {
            crate::identity::direct_scope_id(principal_id, actor_id)
        }
        crate::identity::ConversationKind::Group => supplied_scope.ok_or_else(|| {
            "group learning candidate is missing its persisted conversation scope".to_string()
        })?,
    };
    if conversation_kind == crate::identity::ConversationKind::Group && conversation_id.is_none() {
        return Err(
            "group learning candidate is missing its persisted conversation id".to_string(),
        );
    }

    Ok(CandidateIdentityContext {
        principal_id: principal_id.to_string(),
        actor_id: actor_id.to_string(),
        channel: channel.to_string(),
        conversation_id,
        conversation_kind,
        conversation_scope_id,
        stable_external_conversation_key: context
            .get("stable_external_conversation_key")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

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

const LEARNING_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
const LEARNING_COMMAND_OUTPUT_LIMIT: usize = 1024 * 1024;
const LEARNING_COMMAND_ERROR_PREVIEW: usize = 16 * 1024;

fn learning_command_preview(bytes: &[u8]) -> String {
    let retained = bytes.get(..LEARNING_COMMAND_ERROR_PREVIEW).unwrap_or(bytes);
    let mut preview = String::from_utf8_lossy(retained).trim().to_string();
    if bytes.len() > retained.len() {
        preview.push_str("\n[output truncated]");
    }
    preview
}

pub(in crate::agent::learning) async fn run_cmd(cmd: &mut Command) -> Result<String, String> {
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env("GIT_PAGER", "cat")
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_0", "core.hooksPath")
        .env(
            "GIT_CONFIG_VALUE_0",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_CONFIG_KEY_1", "commit.gpgSign")
        .env("GIT_CONFIG_VALUE_1", "false");
    let output = thinclaw_platform::bounded_command_output(
        cmd,
        LEARNING_COMMAND_TIMEOUT,
        LEARNING_COMMAND_OUTPUT_LIMIT,
        LEARNING_COMMAND_OUTPUT_LIMIT,
    )
    .await
    .map_err(|error| error.to_string())?;
    let stdout = learning_command_preview(&output.stdout);
    let stderr = learning_command_preview(&output.stderr);
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

pub(in crate::agent::learning) fn validate_learning_publish_mode(
    publish_mode: &str,
) -> Result<&'static str, String> {
    match publish_mode.trim().to_ascii_lowercase().as_str() {
        "branch_pr_draft" => Ok("branch_pr_draft"),
        "branch_only" => Ok("branch_only"),
        "bundle_only" => Ok("bundle_only"),
        "local_autorollout" => Ok("local_autorollout"),
        _ => Err(format!(
            "unsupported learning publish mode '{}'; expected branch_pr_draft, branch_only, bundle_only, or local_autorollout",
            publish_mode.trim()
        )),
    }
}

pub(in crate::agent::learning) fn validate_learning_git_ref(value: &str) -> Result<(), String> {
    let components_are_safe = value.split('/').all(|component| {
        !component.is_empty()
            && !component.starts_with('.')
            && !component.ends_with('.')
            && !component.ends_with(".lock")
    });
    let valid = !value.is_empty()
        && value.len() <= 255
        && value != "@"
        && !value.eq_ignore_ascii_case("HEAD")
        && !value
            .as_bytes()
            .first()
            .is_some_and(|byte| matches!(byte, b'-' | b'/' | b'.'))
        && !value
            .as_bytes()
            .last()
            .is_some_and(|byte| matches!(byte, b'/' | b'.'))
        && !value.contains("..")
        && !value.contains("//")
        && !value.contains("@{")
        && components_are_safe
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'));
    if valid {
        Ok(())
    } else {
        Err("learning publication resolved an unsafe Git ref".to_string())
    }
}
