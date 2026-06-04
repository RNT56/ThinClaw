use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ports::SkillSummary;

pub const DEFAULT_AVAILABLE_SKILL_INSTRUCTION: &str =
    "Use `skill_read` with a skill name to inspect full instructions before relying on a skill.";
pub const SUBAGENT_AVAILABLE_SKILL_INSTRUCTION: &str = "If a task would benefit from one of these skills, use `skill_read` to load its full instructions first.";
pub const ACTIVE_SKILL_INSTRUCTION: &str =
    "Use `skill_read` with the skill name to load full instructions before using a skill.";

#[derive(Debug, Clone, Default)]
pub struct PromptAssemblyV2 {
    stable_segments: Vec<(String, String)>,
    ephemeral_segments: Vec<(String, String)>,
    provider_context_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptAssemblyResult {
    pub stable_snapshot: String,
    pub stable_hash: String,
    pub ephemeral_hash: String,
    pub ephemeral_documents: Vec<String>,
    pub segment_order: Vec<String>,
    pub provider_context_refs: Vec<String>,
}

impl PromptAssemblyV2 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_stable(
        mut self,
        segment_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let content = content.into();
        if !content.trim().is_empty() {
            self.stable_segments.push((segment_name.into(), content));
        }
        self
    }

    pub fn push_ephemeral(
        mut self,
        segment_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let content = content.into();
        if !content.trim().is_empty() {
            self.ephemeral_segments.push((segment_name.into(), content));
        }
        self
    }

    pub fn with_provider_context_refs(mut self, refs: Vec<String>) -> Self {
        self.provider_context_refs = refs;
        self
    }

    pub fn build(self) -> PromptAssemblyResult {
        let stable_snapshot = render_segments(&self.stable_segments);
        let stable_hash = sha256_hex(&stable_snapshot);
        let ephemeral_hash = sha256_hex(&render_segments(&self.ephemeral_segments));
        let ephemeral_documents = self
            .ephemeral_segments
            .iter()
            .map(|(_, content)| content.clone())
            .collect();
        let mut segment_order = Vec::new();
        segment_order.extend(
            self.stable_segments
                .iter()
                .map(|(name, _)| format!("stable:{name}")),
        );
        segment_order.extend(
            self.ephemeral_segments
                .iter()
                .map(|(name, _)| format!("ephemeral:{name}")),
        );

        PromptAssemblyResult {
            stable_snapshot,
            stable_hash,
            ephemeral_hash,
            ephemeral_documents,
            segment_order,
            provider_context_refs: self.provider_context_refs,
        }
    }
}

fn render_segments(segments: &[(String, String)]) -> String {
    segments
        .iter()
        .map(|(_, content)| content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

pub fn render_available_skill_index(skills: &[SkillSummary]) -> Option<String> {
    render_available_skill_index_with_instruction(skills, DEFAULT_AVAILABLE_SKILL_INSTRUCTION)
}

pub fn render_available_skill_index_with_instruction(
    skills: &[SkillSummary],
    instruction: &str,
) -> Option<String> {
    if skills.is_empty() {
        return None;
    }

    let mut parts = vec!["### Available Skills".to_string()];
    for skill in skills {
        parts.push(format!("- **{}**: {}", skill.name, skill.description));
    }
    parts.push(format!("\n{instruction}"));
    Some(parts.join("\n"))
}

pub fn render_active_skill_block(skills: &[SkillSummary]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }

    let mut parts = vec!["### Active Skills".to_string()];
    for skill in skills {
        parts.push(format!(
            "- **{}** (v{}, {}): {}",
            skill.name, skill.version, skill.trust, skill.description
        ));
    }
    parts.push(format!("\n{ACTIVE_SKILL_INSTRUCTION}"));
    Some(parts.join("\n"))
}

pub fn render_skill_sections(
    active_skills: &[SkillSummary],
    available_skills: &[SkillSummary],
    available_instruction: &str,
) -> Option<String> {
    let mut sections = Vec::new();
    if let Some(active) = render_active_skill_block(active_skills) {
        sections.push(active);
    }
    if let Some(available) =
        render_available_skill_index_with_instruction(available_skills, available_instruction)
    {
        sections.push(available);
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_hash_ignores_ephemeral_changes() {
        let first = PromptAssemblyV2::new()
            .push_stable("identity", "You are ThinClaw.")
            .push_stable("skills", "skill-index")
            .push_ephemeral("provider", "provider recall one")
            .build();
        let second = PromptAssemblyV2::new()
            .push_stable("identity", "You are ThinClaw.")
            .push_stable("skills", "skill-index")
            .push_ephemeral("provider", "provider recall two")
            .build();

        assert_eq!(first.stable_hash, second.stable_hash);
        assert_ne!(first.ephemeral_hash, second.ephemeral_hash);
    }

    #[test]
    fn stable_hash_changes_when_stable_segments_change() {
        let first = PromptAssemblyV2::new()
            .push_stable("identity", "You are ThinClaw.")
            .push_stable("skills", "skill-index")
            .build();
        let second = PromptAssemblyV2::new()
            .push_stable("identity", "You are ThinClaw, upgraded.")
            .push_stable("skills", "skill-index")
            .build();

        assert_ne!(first.stable_hash, second.stable_hash);
    }

    #[test]
    fn segment_order_is_deterministic() {
        let result = PromptAssemblyV2::new()
            .push_stable("identity", "Identity")
            .push_stable("skills", "Skills")
            .push_ephemeral("provider", "Provider")
            .push_ephemeral("post_compaction", "Compaction")
            .with_provider_context_refs(vec!["mem-2".to_string(), "mem-1".to_string()])
            .build();

        assert_eq!(
            result.segment_order,
            vec![
                "stable:identity".to_string(),
                "stable:skills".to_string(),
                "ephemeral:provider".to_string(),
                "ephemeral:post_compaction".to_string(),
            ]
        );
        assert_eq!(result.ephemeral_documents.len(), 2);
        assert_eq!(result.provider_context_refs, vec!["mem-2", "mem-1"]);
    }

    #[test]
    fn skill_blocks_render_stable_prompt_shape() {
        let skills = vec![SkillSummary {
            name: "rust-fix".to_string(),
            version: "1.0.0".to_string(),
            description: "Repair Rust compiler errors".to_string(),
            trust: "trusted".to_string(),
            path: None,
        }];

        let available = render_available_skill_index(&skills).expect("available block");
        assert!(available.contains("### Available Skills"));
        assert!(available.contains("- **rust-fix**: Repair Rust compiler errors"));
        assert!(available.contains(DEFAULT_AVAILABLE_SKILL_INSTRUCTION));

        let active = render_active_skill_block(&skills).expect("active block");
        assert!(active.contains("### Active Skills"));
        assert!(active.contains("- **rust-fix** (v1.0.0, trusted): Repair Rust compiler errors"));
        assert!(active.contains(ACTIVE_SKILL_INSTRUCTION));
    }

    #[test]
    fn skill_sections_can_use_subagent_available_instruction() {
        let active = vec![SkillSummary {
            name: "writer".to_string(),
            version: "2.0.0".to_string(),
            description: "Write drafts".to_string(),
            trust: "installed".to_string(),
            path: None,
        }];
        let available = vec![SkillSummary {
            name: "reviewer".to_string(),
            version: "1.0.0".to_string(),
            description: "Review drafts".to_string(),
            trust: "trusted".to_string(),
            path: None,
        }];

        let rendered =
            render_skill_sections(&active, &available, SUBAGENT_AVAILABLE_SKILL_INSTRUCTION)
                .expect("sections");

        assert!(rendered.contains("### Active Skills"));
        assert!(rendered.contains("### Available Skills"));
        assert!(rendered.contains(SUBAGENT_AVAILABLE_SKILL_INSTRUCTION));
    }
}
