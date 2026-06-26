//! Psychographic profile (`profile.json`) summarization for prompt assembly.
//!
//! Parses the confidence-gated tiered profile JSON into a compact markdown
//! summary that can be injected into the system prompt.

use super::prompt_text::{FILE_MAX_CHARS, cap_chars};

pub(super) fn summarize_profile_json(content: &str) -> Option<String> {
    let profile: serde_json::Value = match serde_json::from_str(content) {
        Ok(profile) => profile,
        Err(e) => {
            tracing::debug!("Failed to parse profile.json for system prompt: {}", e);
            return None;
        }
    };
    if !profile_is_populated(&profile) {
        return None;
    }

    let confidence = profile
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .or_else(|| {
            profile
                .pointer("/analysis_metadata/confidence_score")
                .and_then(serde_json::Value::as_f64)
        })
        .unwrap_or_default();
    if confidence >= 0.6 {
        Some(cap_chars(&profile_to_user_md(&profile), FILE_MAX_CHARS))
    } else if confidence >= 0.3 {
        Some(format!(
            "## User Profile (preliminary)\n\n{}",
            profile_basics(&profile).join("\n")
        ))
    } else {
        None
    }
}

fn json_str<'a>(value: &'a serde_json::Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

fn json_string_vec(value: &serde_json::Value, pointer: &str) -> Vec<String> {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|item| !item.trim().is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn profile_is_populated(profile: &serde_json::Value) -> bool {
    !json_str(profile, "/preferred_name").is_empty()
        || !json_str(profile, "/context/profession").is_empty()
        || !json_string_vec(profile, "/assistance/goals").is_empty()
}

fn cohort_label(raw: &str) -> &str {
    match raw {
        "busy_professional" => "busy professional",
        "new_parent" => "new parent",
        "student" => "student",
        "elder" => "elder",
        _ => "general",
    }
}

fn profile_basics(profile: &serde_json::Value) -> Vec<String> {
    let mut basics = Vec::new();
    let preferred_name = json_str(profile, "/preferred_name");
    if !preferred_name.is_empty() {
        basics.push(format!("**Name**: {}", preferred_name));
    }
    basics.push(format!(
        "**Communication**: {} tone, {} detail, {} formality",
        json_str(profile, "/communication/tone"),
        json_str(profile, "/communication/detail_level"),
        json_str(profile, "/communication/formality"),
    ));
    let cohort = json_str(profile, "/cohort/cohort");
    if !cohort.is_empty() && cohort != "other" {
        let confidence = profile
            .pointer("/cohort/confidence")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        basics.push(format!(
            "**User type**: {} ({}% confidence)",
            cohort_label(cohort),
            confidence
        ));
    }
    basics
}

fn profile_to_user_md(profile: &serde_json::Value) -> String {
    let mut sections = vec!["# User Profile\n".to_string()];
    sections.extend(profile_basics(profile));

    for (label, pointer) in [
        ("**Profession**", "/context/profession"),
        ("**Life stage**", "/context/life_stage"),
    ] {
        let value = json_str(profile, pointer);
        if !value.is_empty() {
            sections.push(format!("{label}: {value}"));
        }
    }

    for (label, pointer) in [
        ("**Interests**", "/context/interests"),
        ("**Goals**", "/assistance/goals"),
        ("**Focus areas**", "/assistance/focus_areas"),
        ("**Strengths**", "/behavior/strengths"),
        ("**Pain points**", "/behavior/pain_points"),
        ("**Core values**", "/relationship_values/primary"),
    ] {
        let values = json_string_vec(profile, pointer);
        if !values.is_empty() {
            sections.push(format!("{label}: {}", values.join(", ")));
        }
    }

    let proactivity = json_str(profile, "/assistance/proactivity");
    let interaction_style = json_str(profile, "/assistance/interaction_style");
    if !proactivity.is_empty() || !interaction_style.is_empty() {
        sections.push(format!(
            "\n## Assistance Preferences\n\n- **Proactivity**: {}\n- **Interaction style**: {}",
            proactivity, interaction_style
        ));
    }

    sections.join("\n")
}
