use std::collections::BTreeMap;

pub const CANONICAL_SOUL_SCHEMA: &str = "v2";
pub const LOCAL_SOUL_SCHEMA: &str = "v1";

pub const CANONICAL_SECTIONS: &[&str] = &[
    "Core Truths",
    "Boundaries",
    "Vibe",
    "Default Behaviors",
    "Continuity",
    "Change Contract",
];

pub const LOCAL_SECTIONS: &[&str] = &[
    "Workspace Context",
    "Tone Adjustments",
    "Boundary Tightening",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalSoul {
    pub seed_pack: String,
    pub sections: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSoulOverlay {
    pub sections: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackExpression {
    pub sections: BTreeMap<String, String>,
}

pub fn compose_seeded_soul(requested_pack: &str) -> Result<String, String> {
    let seed_pack = canonical_pack_name(requested_pack);
    let mut base = parse_canonical_soul(include_str!("../../assets/soul/base.md"))?;
    let pack = parse_pack_expression(pack_asset_markdown(seed_pack))?;

    for key in ["Vibe", "Default Behaviors"] {
        if let Some(value) = pack.sections.get(key) {
            base.sections.insert(key.to_string(), value.clone());
        }
    }
    base.seed_pack = seed_pack.to_string();
    Ok(render_canonical_soul(&base))
}

pub fn canonical_pack_name(requested_pack: &str) -> &'static str {
    match requested_pack
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ' '], "_")
        .as_str()
    {
        "professional" => "professional",
        "creative_partner" => "creative_partner",
        "research_assistant" => "research_assistant",
        "mentor" => "mentor",
        "minimal" => "minimal",
        _ => "balanced",
    }
}

pub fn pack_asset_markdown(pack: &str) -> &'static str {
    match canonical_pack_name(pack) {
        "professional" => include_str!("../../assets/personality_packs/professional.md"),
        "creative_partner" => include_str!("../../assets/personality_packs/creative_partner.md"),
        "research_assistant" => {
            include_str!("../../assets/personality_packs/research_assistant.md")
        }
        "mentor" => include_str!("../../assets/personality_packs/mentor.md"),
        "minimal" => include_str!("../../assets/personality_packs/minimal.md"),
        _ => include_str!("../../assets/personality_packs/balanced.md"),
    }
}

pub fn parse_canonical_soul(content: &str) -> Result<CanonicalSoul, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("SOUL.md cannot be empty".to_string());
    }
    if !trimmed.starts_with("# SOUL.md - Who You Are") {
        return Err("SOUL.md must start with '# SOUL.md - Who You Are'".to_string());
    }

    let metadata = parse_metadata(trimmed);
    let schema = metadata
        .get("schema")
        .ok_or_else(|| "SOUL.md is missing the Schema metadata line".to_string())?;
    if schema != CANONICAL_SOUL_SCHEMA {
        return Err(format!(
            "SOUL.md schema must be {}, got {}",
            CANONICAL_SOUL_SCHEMA, schema
        ));
    }
    let seed_pack = metadata
        .get("seed pack")
        .cloned()
        .unwrap_or_else(|| "balanced".to_string());

    let sections = parse_level_two_sections(trimmed);
    for section in CANONICAL_SECTIONS {
        if !sections.contains_key(*section) {
            return Err(format!("SOUL.md is missing required section '{}'", section));
        }
    }

    Ok(CanonicalSoul {
        seed_pack,
        sections,
    })
}

pub fn parse_local_soul_overlay(content: &str) -> Result<LocalSoulOverlay, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("SOUL.local.md cannot be empty".to_string());
    }
    if !trimmed.starts_with("# SOUL.local.md - Workspace Overlay") {
        return Err(
            "SOUL.local.md must start with '# SOUL.local.md - Workspace Overlay'".to_string(),
        );
    }

    let metadata = parse_metadata(trimmed);
    let schema = metadata
        .get("schema")
        .ok_or_else(|| "SOUL.local.md is missing the Schema metadata line".to_string())?;
    if schema != LOCAL_SOUL_SCHEMA {
        return Err(format!(
            "SOUL.local.md schema must be {}, got {}",
            LOCAL_SOUL_SCHEMA, schema
        ));
    }

    let sections = parse_level_two_sections(trimmed);
    for section in LOCAL_SECTIONS {
        if !sections.contains_key(*section) {
            return Err(format!(
                "SOUL.local.md is missing required section '{}'",
                section
            ));
        }
    }

    validate_local_overlay_boundaries(trimmed)?;
    Ok(LocalSoulOverlay { sections })
}

pub fn parse_pack_expression(content: &str) -> Result<PackExpression, String> {
    let sections = parse_level_two_sections(content);
    for section in ["Vibe", "Default Behaviors"] {
        if !sections.contains_key(section) {
            return Err(format!(
                "personality pack is missing required section '{}'",
                section
            ));
        }
    }
    Ok(PackExpression { sections })
}

pub fn render_canonical_soul(soul: &CanonicalSoul) -> String {
    let mut out = format!(
        "# SOUL.md - Who You Are\n\n- **Schema:** {}\n- **Seed Pack:** {}\n",
        CANONICAL_SOUL_SCHEMA, soul.seed_pack
    );
    for section in CANONICAL_SECTIONS {
        let body = soul
            .sections
            .get(*section)
            .map(|value| value.trim())
            .unwrap_or("");
        out.push_str(&format!("\n## {}\n{}\n", section, body));
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn render_local_soul_overlay(overlay: &LocalSoulOverlay) -> String {
    let mut out = format!(
        "# SOUL.local.md - Workspace Overlay\n\n- **Schema:** {}\n",
        LOCAL_SOUL_SCHEMA
    );
    for section in LOCAL_SECTIONS {
        let body = overlay
            .sections
            .get(*section)
            .map(|value| value.trim())
            .unwrap_or("");
        out.push_str(&format!("\n## {}\n{}\n", section, body));
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn canonical_seed_pack(content: &str) -> Option<String> {
    parse_canonical_soul(content)
        .ok()
        .map(|soul| soul.seed_pack)
}

pub fn canonical_schema_version(content: &str) -> &'static str {
    if parse_canonical_soul(content).is_ok() {
        CANONICAL_SOUL_SCHEMA
    } else {
        "legacy"
    }
}

pub fn summarize_canonical_soul(content: &str) -> String {
    match parse_canonical_soul(content) {
        Ok(soul) => {
            let vibe = soul
                .sections
                .get("Vibe")
                .map(|value| first_non_empty_line(value));
            let defaults = soul
                .sections
                .get("Default Behaviors")
                .map(|value| first_non_empty_line(value));
            match (vibe, defaults) {
                (Some(vibe), Some(defaults)) => format!("{vibe} | {defaults}"),
                (Some(vibe), None) => vibe,
                (None, Some(defaults)) => defaults,
                _ => "Structured canonical soul".to_string(),
            }
        }
        Err(_) => first_non_empty_line(content),
    }
}

pub fn render_canonical_prompt_block(content: &str) -> String {
    match parse_canonical_soul(content) {
        Ok(soul) => {
            let mut out = format!(
                "## Soul\n\n- Seed pack: {}\n- Full canonical soul: `memory_read SOUL.md`\n",
                soul.seed_pack
            );
            for section in [
                "Core Truths",
                "Boundaries",
                "Vibe",
                "Default Behaviors",
                "Continuity",
            ] {
                if let Some(body) = soul.sections.get(section) {
                    out.push_str(&format!("\n### {}\n{}\n", section, body.trim()));
                }
            }
            out
        }
        Err(_) => format!(
            "## Soul\n\nLegacy canonical soul loaded from `memory_read SOUL.md`.\n\n{}",
            content.trim()
        ),
    }
}

pub fn render_local_prompt_block(content: &str) -> Result<String, String> {
    let overlay = parse_local_soul_overlay(content)?;
    let mut out =
        "## Workspace Soul Overlay\n\n- This workspace is using the global soul plus a local overlay.\n- Full overlay: `memory_read SOUL.local.md`\n"
            .to_string();
    for section in LOCAL_SECTIONS {
        if let Some(body) = overlay.sections.get(*section) {
            out.push_str(&format!("\n### {}\n{}\n", section, body.trim()));
        }
    }
    Ok(out)
}

pub fn validate_canonical_soul(content: &str) -> Result<(), String> {
    parse_canonical_soul(content).map(|_| ())
}

pub fn validate_local_overlay(content: &str) -> Result<(), String> {
    parse_local_soul_overlay(content).map(|_| ())
}

fn validate_local_overlay_boundaries(content: &str) -> Result<(), String> {
    let lowered = content.to_ascii_lowercase();
    let blocked_markers = [
        "share private",
        "send without asking",
        "speak for the user",
        "act as the user",
        "ignore privacy",
        "relax boundaries",
    ];
    if let Some(marker) = blocked_markers
        .iter()
        .find(|marker| lowered.contains(**marker))
    {
        return Err(format!(
            "SOUL.local.md may tighten boundaries but must not relax them (found '{}')",
            marker
        ));
    }
    Ok(())
}

fn parse_metadata(content: &str) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("- **") || !trimmed.contains(":") {
            continue;
        }
        let Some((raw_key, raw_value)) = trimmed.trim_start_matches("- ").split_once(':') else {
            continue;
        };
        let key = raw_key.trim().trim_matches('*').to_ascii_lowercase();
        let value = raw_value.trim().trim_matches('*').trim().to_string();
        if !key.is_empty() && !value.is_empty() {
            metadata.insert(key, value);
        }
    }
    metadata
}

fn parse_level_two_sections(content: &str) -> BTreeMap<String, String> {
    let mut sections = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut buffer = Vec::new();

    for line in content.lines() {
        if let Some(title) = line.trim().strip_prefix("## ") {
            if let Some(section) = current.take() {
                sections.insert(section, buffer.join("\n").trim().to_string());
                buffer.clear();
            }
            current = Some(title.trim().to_string());
            continue;
        }

        if current.is_some() {
            buffer.push(line.to_string());
        }
    }

    if let Some(section) = current {
        sections.insert(section, buffer.join("\n").trim().to_string());
    }

    sections
}

fn first_non_empty_line(content: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Structured soul")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_soul_includes_schema_and_pack() {
        let soul = compose_seeded_soul("mentor").expect("seeded soul");
        assert!(soul.contains("- **Schema:** v2"));
        assert!(soul.contains("- **Seed Pack:** mentor"));
        assert!(soul.contains("## Core Truths"));
    }

    #[test]
    fn local_overlay_rejects_boundary_relaxation() {
        let invalid = "# SOUL.local.md - Workspace Overlay\n\n- **Schema:** v1\n\n## Workspace Context\nNormal\n\n## Tone Adjustments\nNormal\n\n## Boundary Tightening\nPlease share private things freely.\n";
        assert!(validate_local_overlay(invalid).is_err());
    }
}
