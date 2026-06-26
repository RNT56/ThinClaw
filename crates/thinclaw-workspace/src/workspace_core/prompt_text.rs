//! Pure text helpers shared across prompt assembly and seeding.
//!
//! These functions have no `Workspace` dependency — they operate on raw
//! markdown/string content (truncation, section extraction, markdown field
//! parsing, timezone-line upsert, and emptiness checks).

/// Maximum characters per workspace file injected into the system prompt.
/// Matches openclaw's `bootstrapMaxChars` default (~20k chars ≈ 5k tokens).
pub(super) const FILE_MAX_CHARS: usize = 4_000;

/// Truncate `text` to at most `max` chars, appending a truncation notice.
pub(super) fn cap_chars(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let cut = text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i < max)
        .last()
        .unwrap_or(0);
    format!(
        "{}\n\n_[... truncated — file exceeds {max} chars. Use `memory_read` to see the rest.]_",
        &text[..cut]
    )
}

/// Extract essential operational instructions from AGENTS.md content.
///
/// Keeps only the operationally critical sections (startup, memory policy,
/// safety boundaries, external-action rules, group-chat behavior, tool-use
/// notes, and heartbeat conduct). Everything else can be read on demand via
/// `memory_read AGENTS.md`.
pub(super) fn extract_essential_instructions(agents_content: &str) -> String {
    let mut essential = Vec::new();
    let mut in_keep_section = false;

    // Section headers to KEEP in the system prompt (critical operational rules)
    let keep_keywords = [
        "First Run",
        "Session Startup",
        "Memory",
        "MEMORY.md",
        "Write It Down",
        "Mental Notes",
        "Red Lines",
        "Protected Repo Boundary Policy",
        "Feature Parity Update Policy",
        "External vs Internal",
        "Group Chats",
        "Know When to Speak",
        "Tools",
        "Platform Formatting",
        "Heartbeats",
        "Be Proactive",
    ];

    for line in agents_content.lines() {
        let trimmed = line.trim();

        // Detect top-level section headers.
        if trimmed.starts_with("## ") {
            // Strip markdown heading markers + emoji for clean matching
            let header_text = trimmed
                .trim_start_matches('#')
                .trim()
                .trim_start_matches(|c: char| !c.is_alphabetic())
                .trim();
            in_keep_section = keep_keywords.iter().any(|h| header_text.contains(h));
            if in_keep_section {
                essential.push(line.to_string());
            }
            continue;
        }

        // Keep nested headings if we're inside an already-kept top-level section.
        // If we're not, still allow known critical subsection headings through.
        if trimmed.starts_with("### ") {
            if in_keep_section {
                essential.push(line.to_string());
                continue;
            }
            let header_text = trimmed
                .trim_start_matches('#')
                .trim()
                .trim_start_matches(|c: char| !c.is_alphabetic())
                .trim();
            in_keep_section = keep_keywords.iter().any(|h| header_text.contains(h));
            if in_keep_section {
                essential.push(line.to_string());
            }
            continue;
        }

        if in_keep_section {
            essential.push(line.to_string());
        }
    }

    if essential.is_empty() {
        // Fallback: first 400 chars if no sections matched
        cap_chars(agents_content, 400)
    } else {
        essential.push(String::new());
        essential.push("Full instructions: `memory_read AGENTS.md`".to_string());
        essential.join("\n")
    }
}

pub(super) fn extract_markdown_fields(content: &str) -> Vec<String> {
    let mut fields = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("- **") && t.contains(":**") {
            let after_colon = t.split_once(":**").map(|x| x.1).unwrap_or("").trim();
            if !after_colon.is_empty() && !after_colon.starts_with("_(") && after_colon != "_" {
                fields.push(t.to_string());
            }
        }
    }
    fields
}

pub(super) fn upsert_timezone_line(content: &str, timezone: Option<&str>) -> String {
    let replacement = match timezone {
        Some(value) => format!("- **Timezone:** {}", value),
        None => "- **Timezone:**".to_string(),
    };
    let mut replaced = false;
    let mut lines = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("- **Timezone:**") || trimmed.starts_with("- **Timezone**:") {
            lines.push(replacement.clone());
            replaced = true;
        } else {
            lines.push(line.to_string());
        }
    }

    if !replaced && timezone.is_some() {
        if !lines.is_empty() && !lines.last().is_some_and(|line| line.is_empty()) {
            lines.push(String::new());
        }
        lines.push(replacement);
    }

    let mut updated = lines.join("\n");
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated
}

pub(super) fn is_effectively_empty(content: &str) -> bool {
    let without_comments = strip_html_comments(content);
    without_comments.lines().all(|line| {
        let trimmed = line.trim();
        trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed == "- [ ]"
            || trimmed == "- [x]"
            || trimmed == "-"
            || trimmed == "*"
    })
}

pub(super) fn strip_html_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("<!--") {
        result.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return result,
        }
    }
    result.push_str(rest);
    result
}
