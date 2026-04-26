use super::*;
pub(in crate::agent::learning) fn ensure_auto_apply_class(classes: &mut Vec<String>, value: &str) {
    if !classes
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(value))
    {
        classes.push(value.to_string());
    }
}

pub(in crate::agent::learning) fn generated_skill_triggers(
    turn: &crate::agent::session::Turn,
    user_input: &str,
    reuse_count: u32,
    min_tool_calls: u32,
) -> Vec<SkillSynthesisTrigger> {
    let distinct_categories = turn
        .tool_calls
        .iter()
        .map(|call| generated_tool_category(&call.name))
        .collect::<std::collections::HashSet<_>>()
        .len();
    let has_multi_tool_pattern =
        turn.tool_calls.len() as u32 >= min_tool_calls && distinct_categories >= 2;
    let recovered_from_failure = turn
        .tool_calls
        .iter()
        .any(|call| call.error.is_some() && call.result.is_none());
    let corrected_then_succeeded = detect_generated_skill_correction_signal(user_input)
        && !turn.tool_calls.is_empty()
        && turn.tool_calls.iter().all(|call| call.error.is_none());
    let repeated_workflow_match = reuse_count >= 2;
    let mut triggers = Vec::new();
    if has_multi_tool_pattern {
        triggers.push(SkillSynthesisTrigger::ComplexSuccess);
    }
    if recovered_from_failure {
        triggers.push(SkillSynthesisTrigger::DeadEndRecovery);
    }
    if corrected_then_succeeded {
        triggers.push(SkillSynthesisTrigger::UserCorrection);
    }
    if repeated_workflow_match {
        triggers.push(SkillSynthesisTrigger::NonTrivialWorkflow);
    }
    triggers
}

pub(in crate::agent::learning) fn detect_generated_skill_correction_signal(content: &str) -> bool {
    let normalized = content.trim().to_ascii_lowercase();
    [
        "actually",
        "correction:",
        "to clarify",
        "that's incorrect",
        "that is incorrect",
        "not quite",
        "use this instead",
        "please use",
        "instead:",
    ]
    .iter()
    .any(|prefix| normalized.starts_with(prefix))
}

pub(in crate::agent::learning) fn generated_tool_category(tool_name: &str) -> &'static str {
    match tool_name {
        name if name.contains("file") || name.contains("search") => "files",
        name if name.contains("memory") || name.contains("session") => "memory",
        name if name.contains("http") || name.contains("browser") => "web",
        name if name.contains("skill") || name.contains("prompt") => "learning",
        "execute_code" | "shell" | "process" | "create_job" => "execution",
        _ => "other",
    }
}

pub(in crate::agent::learning) fn generated_skill_lifecycle_for_reuse(
    reuse_count: u32,
) -> (GeneratedSkillLifecycle, Option<String>, bool) {
    if reuse_count >= 4 {
        (
            GeneratedSkillLifecycle::Proposed,
            Some("proposal_reuse_threshold".to_string()),
            false,
        )
    } else if reuse_count >= 2 {
        (
            GeneratedSkillLifecycle::Shadow,
            Some("shadow_candidate".to_string()),
            false,
        )
    } else {
        (GeneratedSkillLifecycle::Draft, None, false)
    }
}

pub(in crate::agent::learning) fn generated_skill_feedback_polarity(verdict: &str) -> i8 {
    let normalized = verdict.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "helpful" | "approve" | "approved" | "accept" | "accepted" | "good" | "works"
        | "success" | "positive" => 1,
        "harmful" | "reject" | "rejected" | "bad" | "broken" | "regression" | "dont_learn"
        | "negative" | "rollback" | "rolled_back" => -1,
        _ => 0,
    }
}

pub(in crate::agent::learning) fn generated_skill_transition_entry(
    lifecycle: GeneratedSkillLifecycle,
    activation_reason: Option<&str>,
    feedback_verdict: Option<&str>,
    feedback_note: Option<&str>,
    artifact_version_id: Option<Uuid>,
    transition_at: DateTime<Utc>,
) -> serde_json::Value {
    serde_json::json!({
        "status": lifecycle.as_str(),
        "at": transition_at,
        "activation_reason": activation_reason,
        "feedback_verdict": feedback_verdict,
        "feedback_note": feedback_note,
        "artifact_version_id": artifact_version_id,
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
    let mut proposal = if candidate.proposal.is_object() {
        candidate.proposal.clone()
    } else {
        serde_json::json!({})
    };
    let entry = generated_skill_transition_entry(
        lifecycle,
        activation_reason,
        feedback_verdict,
        feedback_note,
        artifact_version_id,
        transition_at,
    );
    let obj = proposal
        .as_object_mut()
        .expect("generated skill proposal should be object");
    obj.insert("provenance".to_string(), serde_json::json!("generated"));
    obj.insert(
        "lifecycle_status".to_string(),
        serde_json::json!(lifecycle.as_str()),
    );
    obj.insert(
        "last_transition_at".to_string(),
        serde_json::json!(transition_at),
    );
    if let Some(reason) = activation_reason.filter(|value| !value.trim().is_empty()) {
        obj.insert(
            "activation_reason".to_string(),
            serde_json::json!(reason.to_string()),
        );
    }
    if let Some(version_id) = artifact_version_id {
        obj.insert(
            "last_artifact_version_id".to_string(),
            serde_json::json!(version_id),
        );
    }
    if let Some(verdict) = feedback_verdict {
        obj.insert(
            "last_feedback".to_string(),
            serde_json::json!({
                "verdict": verdict,
                "note": feedback_note,
                "at": transition_at,
            }),
        );
    }
    match lifecycle {
        GeneratedSkillLifecycle::Active => {
            obj.insert("activated_at".to_string(), serde_json::json!(transition_at));
        }
        GeneratedSkillLifecycle::Frozen => {
            obj.insert("frozen_at".to_string(), serde_json::json!(transition_at));
        }
        GeneratedSkillLifecycle::RolledBack => {
            obj.insert(
                "rolled_back_at".to_string(),
                serde_json::json!(transition_at),
            );
        }
        _ => {}
    }
    let history = obj
        .entry("state_history".to_string())
        .or_insert_with(|| serde_json::json!([]));
    if !history.is_array() {
        *history = serde_json::json!([]);
    }
    history
        .as_array_mut()
        .expect("state_history should be array")
        .push(entry);
    proposal
}

pub(in crate::agent::learning) fn generated_workflow_digest(
    user_input: &str,
    tool_calls: &[crate::agent::session::TurnToolCall],
) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(normalize_generated_skill_text(user_input).as_bytes());
    for call in tool_calls {
        hasher.update(b"|tool:");
        hasher.update(call.name.as_bytes());
        hasher.update(b"|params:");
        hasher.update(canonicalize_json_value(&call.parameters).as_bytes());
        hasher.update(b"|status:");
        hasher.update(if call.error.is_some() {
            b"error".as_slice()
        } else {
            b"ok".as_slice()
        });
        hasher.update(b"|signature:");
        hasher.update(compact_tool_outcome_signature(call).as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

pub(in crate::agent::learning) fn normalize_generated_skill_text(content: &str) -> String {
    content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub(in crate::agent::learning) fn canonicalize_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => {
            serde_json::to_string(value).unwrap_or_else(|_| "\"<string>\"".to_string())
        }
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonicalize_json_value)
                .collect::<Vec<_>>()
                .join(",")
        ),
        serde_json::Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| {
                        let value = map
                            .get(key)
                            .map(canonicalize_json_value)
                            .unwrap_or_else(|| "null".to_string());
                        format!(
                            "{}:{}",
                            serde_json::to_string(key).unwrap_or_else(|_| "\"<key>\"".to_string()),
                            value
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

pub(in crate::agent::learning) fn compact_tool_outcome_signature(
    call: &crate::agent::session::TurnToolCall,
) -> String {
    use sha2::{Digest, Sha256};

    let signature_input = if let Some(error) = call.error.as_deref() {
        format!("error:{}", normalize_generated_skill_text(error))
    } else if let Some(result) = call.result.as_ref() {
        format!("ok:{}", canonicalize_json_value(result))
    } else {
        "ok:null".to_string()
    };

    let mut hasher = Sha256::new();
    hasher.update(signature_input.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("sha256:{}", &digest[..16])
}

pub(in crate::agent::learning) fn synthesize_generated_skill_markdown(
    skill_name: &str,
    user_input: &str,
    tool_calls: &[crate::agent::session::TurnToolCall],
    lifecycle: GeneratedSkillLifecycle,
    reuse_count: u32,
    activation_reason: Option<String>,
) -> Result<String, String> {
    let description = user_input
        .split_whitespace()
        .take(18)
        .collect::<Vec<_>>()
        .join(" ");
    let keywords = tool_calls
        .iter()
        .map(|call| call.name.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let workflow_steps = tool_calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            let parameter_keys = call
                .parameters
                .as_object()
                .map(|object| object.keys().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            if parameter_keys.is_empty() {
                format!("{}. Use `{}`.", index + 1, call.name)
            } else {
                format!(
                    "{}. Use `{}` with parameters touching: {}.",
                    index + 1,
                    call.name,
                    parameter_keys
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let yaml_keywords = if keywords.is_empty() {
        "[]".to_string()
    } else {
        format!(
            "[{}]",
            keywords
                .iter()
                .map(|keyword| format!("\"{keyword}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let activation_reason = activation_reason.unwrap_or_else(|| "draft".to_string());
    let content = format!(
        "---\nname: {skill_name}\nversion: 0.1.0\ndescription: \"Generated workflow skill for {description}\"\nactivation:\n  keywords: {yaml_keywords}\nmetadata:\n  openclaw:\n    provenance: generated\n    lifecycle_status: {}\n    outcome_score: {}\n    reuse_count: {reuse_count}\n    activation_reason: \"{}\"\n---\n\nYou are a reusable workflow skill distilled from a successful ThinClaw turn.\n\nUse this skill when the user is asking for work that resembles:\n- {description}\n\nPreferred workflow:\n{workflow_steps}\n\nSafety notes:\n- Verify tool results before moving to the next step.\n- Prefer deterministic file/memory reads before mutations.\n- Stop and surface blockers instead of guessing when required tools fail.\n",
        lifecycle.as_str(),
        if reuse_count >= 2 { "0.92" } else { "0.78" },
        activation_reason,
    );
    crate::skills::parser::parse_skill_md(&crate::skills::normalize_line_endings(&content))
        .map_err(|err| err.to_string())?;
    Ok(content)
}

pub(in crate::agent::learning) fn stable_json_hash(value: &serde_json::Value) -> u64 {
    let serialized = serde_json::to_string(value).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    hasher.finish()
}

pub(in crate::agent::learning) fn proposal_fingerprint(
    title: &str,
    rationale: &str,
    target_files: &[String],
    diff: &str,
) -> String {
    let canonical = serde_json::json!({
        "title": title.trim(),
        "rationale": rationale.trim(),
        "target_files": target_files,
        "diff": diff.trim(),
    });
    format!("{:016x}", stable_json_hash(&canonical))
}

pub(in crate::agent::learning) fn classify_event(event: &DbLearningEvent) -> ImprovementClass {
    let et = event.event_type.to_ascii_lowercase();
    if et.contains("code") || event.payload.get("diff").is_some() {
        return ImprovementClass::Code;
    }
    if et.contains("prompt") {
        return ImprovementClass::Prompt;
    }
    if et.contains("skill") {
        return ImprovementClass::Skill;
    }
    if et.contains("routine") {
        return ImprovementClass::Routine;
    }
    if let Some(target) = event.payload.get("target").and_then(|v| v.as_str())
        && matches!(
            target,
            "SOUL.md" | "SOUL.local.md" | "AGENTS.md" | "USER.md"
        )
    {
        return ImprovementClass::Prompt;
    }
    if event.payload.get("skill_content").is_some() {
        return ImprovementClass::Skill;
    }
    ImprovementClass::Memory
}

pub(in crate::agent::learning) fn validate_prompt_content(content: &str) -> Result<(), String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("prompt content cannot be empty".to_string());
    }
    if !trimmed.contains('#') {
        return Err("prompt content must include markdown headings".to_string());
    }
    let lowered = trimmed.to_ascii_lowercase();
    let suspicious_markers = ["role: user", "role: assistant", "tool_result", "<tool_call"];
    if suspicious_markers
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        return Err("prompt content appears to include transcript/tool residue".to_string());
    }
    Ok(())
}

pub(in crate::agent::learning) fn validate_prompt_target_content(
    target: &str,
    content: &str,
) -> Result<(), String> {
    if target.eq_ignore_ascii_case(paths::SOUL) {
        return crate::identity::soul::validate_canonical_soul(content);
    }
    if target.eq_ignore_ascii_case(paths::SOUL_LOCAL) {
        return crate::identity::soul::validate_local_overlay(content);
    }
    if target.eq_ignore_ascii_case(paths::AGENTS) {
        let lowered = content.to_ascii_lowercase();
        let required_markers = ["red lines", "ask first", "don't"];
        if required_markers
            .iter()
            .all(|marker| !lowered.contains(marker))
        {
            return Err(format!(
                "{} update rejected: core safety guidance appears to be missing",
                target
            ));
        }
    }
    Ok(())
}

pub(in crate::agent::learning) fn is_prompt_target_supported(target: &str) -> bool {
    matches!(
        target,
        paths::SOUL | paths::SOUL_LOCAL | paths::AGENTS | paths::USER
    ) || target
        .to_ascii_lowercase()
        .ends_with(&format!("/{}", paths::USER.to_ascii_lowercase()))
}

pub(in crate::agent::learning) fn materialize_prompt_candidate_content(
    current: &str,
    proposal: &serde_json::Value,
    target: &str,
) -> Result<String, String> {
    let patch = proposal
        .get("prompt_patch")
        .ok_or_else(|| "prompt candidate missing content".to_string())?;
    let operation = patch
        .get("operation")
        .and_then(|value| value.as_str())
        .unwrap_or("replace");
    let base = ensure_prompt_document_root(current, target);
    let next = match operation {
        "replace" => patch
            .get("content")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "prompt patch missing content".to_string())?
            .to_string(),
        "upsert_section" => {
            let heading = patch
                .get("heading")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "prompt patch missing heading".to_string())?;
            let section_content = patch
                .get("section_content")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            upsert_markdown_section(&base, heading, section_content)
        }
        "append_section" => {
            let heading = patch
                .get("heading")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "prompt patch missing heading".to_string())?;
            let section_content = patch
                .get("section_content")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            append_markdown_section(&base, heading, section_content)
        }
        "remove_section" => {
            let heading = patch
                .get("heading")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "prompt patch missing heading".to_string())?;
            remove_markdown_section(&base, heading)?
        }
        other => return Err(format!("unsupported prompt patch operation '{}'", other)),
    };
    Ok(ensure_prompt_trailing_newline(&next))
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

pub(in crate::agent::learning) fn ensure_prompt_document_root(
    current: &str,
    target: &str,
) -> String {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        return ensure_prompt_trailing_newline(trimmed);
    }
    if target.ends_with(paths::SOUL_LOCAL) {
        let mut sections = BTreeMap::new();
        for section in crate::identity::soul::LOCAL_SECTIONS {
            sections.insert((*section).to_string(), String::new());
        }
        return crate::identity::soul::render_local_soul_overlay(
            &crate::identity::soul::LocalSoulOverlay { sections },
        );
    }
    if target.ends_with(paths::SOUL) {
        return crate::identity::soul::compose_seeded_soul("balanced").unwrap_or_else(|_| {
            "# SOUL.md - Who You Are\n\n- **Schema:** v2\n- **Seed Pack:** balanced\n\n## Core Truths\n\n## Boundaries\n\n## Vibe\n\n## Default Behaviors\n\n## Continuity\n\n## Change Contract\n"
                .to_string()
        });
    }
    let title = if target.ends_with(paths::USER) {
        "USER.md"
    } else if target.ends_with(paths::AGENTS) {
        "AGENTS.md"
    } else {
        target.rsplit('/').next().unwrap_or("PROMPT.md")
    };
    format!("# {title}\n")
}

pub(in crate::agent::learning) fn ensure_prompt_trailing_newline(content: &str) -> String {
    let trimmed = content.trim_end();
    format!("{trimmed}\n")
}

pub(in crate::agent::learning) fn normalize_heading_name(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('#')
        .trim()
        .to_ascii_lowercase()
}

pub(in crate::agent::learning) fn parse_markdown_heading(line: &str) -> Option<(usize, String)> {
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

pub(in crate::agent::learning) fn find_section_byte_range(
    doc: &str,
    heading_name: &str,
) -> Option<(usize, usize, usize, String)> {
    let target = normalize_heading_name(heading_name);
    let mut offset = 0usize;
    let mut start: Option<(usize, usize, usize, String)> = None;

    for line in doc.split_inclusive('\n') {
        let line_start = offset;
        let line_end = offset + line.len();
        offset = line_end;

        if let Some((level, title)) = parse_markdown_heading(line) {
            if let Some((start_offset, current_level, _, current_title)) = &start
                && level <= *current_level
            {
                return Some((
                    *start_offset,
                    line_start,
                    *current_level,
                    current_title.clone(),
                ));
            }

            if normalize_heading_name(&title) == target {
                start = Some((line_start, level, line_end, title));
            }
        }
    }

    start.map(|(start_offset, level, _, title)| (start_offset, doc.len(), level, title))
}

pub(in crate::agent::learning) fn upsert_markdown_section(
    doc: &str,
    heading: &str,
    section_content: &str,
) -> String {
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
        return ensure_prompt_trailing_newline(merged.trim());
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
    ensure_prompt_trailing_newline(&merged)
}

pub(in crate::agent::learning) fn append_markdown_section(
    doc: &str,
    heading: &str,
    section_content: &str,
) -> String {
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
    ensure_prompt_trailing_newline(&merged)
}

pub(in crate::agent::learning) fn remove_markdown_section(
    doc: &str,
    heading: &str,
) -> Result<String, String> {
    let Some((start, end, _, _)) = find_section_byte_range(doc, heading) else {
        return Err(format!("section '{}' not found", heading));
    };

    let mut merged = String::with_capacity(doc.len());
    merged.push_str(&doc[..start]);
    merged.push_str(doc[end..].trim_start_matches('\n'));
    Ok(ensure_prompt_trailing_newline(merged.trim()))
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
