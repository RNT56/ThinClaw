use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ports::{SkillContext, SkillSummary, WorkspacePromptMaterials};

pub const DEFAULT_AVAILABLE_SKILL_INSTRUCTION: &str =
    "Use `skill_read` with a skill name to inspect full instructions before relying on a skill.";
pub const SUBAGENT_AVAILABLE_SKILL_INSTRUCTION: &str = "If a task would benefit from one of these skills, use `skill_read` to load its full instructions first.";
pub const ACTIVE_SKILL_INSTRUCTION: &str =
    "Use `skill_read` with the skill name to load full instructions before using a skill.";
pub const CHANNEL_TRANSCRIPT_GUIDANCE: &str = "Channel transcript guidance: when the user asks about prior Telegram, WebUI, or other channel conversations, use session_search to inspect transcript history. Do not use communication/action tools like telegram_actions to read transcript history or infer account login state; those tools perform live platform actions only.";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatcherPromptMaterials {
    pub workspace_prompt: Option<String>,
    pub provider_system_prompt: Option<String>,
    pub skill_index_context: Option<String>,
    pub provider_recall_context: Option<String>,
    pub linked_recall_context: Option<String>,
    pub channel_formatting_context: Option<String>,
    pub personality_overlay_context: Option<String>,
    pub runtime_capability_hint: Option<String>,
    pub active_skill_context: Option<String>,
    pub post_compaction_fragment: Option<String>,
    pub provider_context_refs: Vec<String>,
}

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

pub fn render_skill_index_context(block: &str) -> String {
    format!("## Skills\n{block}")
}

pub fn render_active_skill_context(block: &str) -> String {
    format!("## Skill Expansion\n{block}")
}

pub fn assemble_workspace_prompt_materials(
    materials: &WorkspacePromptMaterials,
    skills: &SkillContext,
) -> PromptAssemblyResult {
    let skill_index = skills
        .available_index_block
        .as_deref()
        .map(render_skill_index_context)
        .unwrap_or_default();
    let active_skills = skills
        .active_skill_block
        .as_deref()
        .map(render_active_skill_context)
        .unwrap_or_default();

    PromptAssemblyV2::new()
        .push_stable(
            "workspace_prompt",
            materials.workspace_prompt.clone().unwrap_or_default(),
        )
        .push_stable(
            "provider_system_prompt",
            materials.provider_system_prompt.clone().unwrap_or_default(),
        )
        .push_stable("skills_index", skill_index)
        .push_ephemeral(
            "provider_recall",
            materials.provider_recall_block.clone().unwrap_or_default(),
        )
        .push_ephemeral(
            "linked_recall",
            materials.linked_recall_block.clone().unwrap_or_default(),
        )
        .push_ephemeral(
            "channel_formatting_hints",
            materials
                .channel_formatting_hints
                .clone()
                .unwrap_or_default(),
        )
        .push_ephemeral(
            "runtime_capabilities",
            materials
                .runtime_capability_hint
                .clone()
                .unwrap_or_default(),
        )
        .push_ephemeral("active_skills", active_skills)
        .push_ephemeral(
            "post_compaction_fragment",
            materials
                .post_compaction_context
                .clone()
                .unwrap_or_default(),
        )
        .with_provider_context_refs(materials.provider_context_refs.clone())
        .build()
}

pub fn assemble_dispatcher_prompt_materials(
    materials: &DispatcherPromptMaterials,
) -> PromptAssemblyResult {
    PromptAssemblyV2::new()
        .push_stable(
            "workspace_prompt",
            materials.workspace_prompt.clone().unwrap_or_default(),
        )
        .push_stable(
            "provider_system_prompt",
            materials.provider_system_prompt.clone().unwrap_or_default(),
        )
        .push_stable(
            "skills_index",
            materials.skill_index_context.clone().unwrap_or_default(),
        )
        .push_ephemeral("transcript_guidance", CHANNEL_TRANSCRIPT_GUIDANCE)
        .push_ephemeral(
            "provider_recall",
            materials
                .provider_recall_context
                .clone()
                .unwrap_or_default(),
        )
        .push_ephemeral(
            "linked_recall",
            materials.linked_recall_context.clone().unwrap_or_default(),
        )
        .push_ephemeral(
            "channel_formatting_hints",
            materials
                .channel_formatting_context
                .clone()
                .unwrap_or_default(),
        )
        .push_ephemeral(
            "personality_overlay",
            materials
                .personality_overlay_context
                .clone()
                .unwrap_or_default(),
        )
        .push_ephemeral(
            "runtime_capabilities",
            materials
                .runtime_capability_hint
                .clone()
                .unwrap_or_default(),
        )
        .push_ephemeral(
            "active_skills",
            materials.active_skill_context.clone().unwrap_or_default(),
        )
        .push_ephemeral(
            "post_compaction_fragment",
            materials
                .post_compaction_fragment
                .clone()
                .unwrap_or_default(),
        )
        .with_provider_context_refs(materials.provider_context_refs.clone())
        .build()
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

    #[test]
    fn workspace_prompt_material_assembly_preserves_segment_policy() {
        let materials = WorkspacePromptMaterials {
            workspace_prompt: Some("You are ThinClaw.".to_string()),
            provider_system_prompt: Some("Provider guidance.".to_string()),
            provider_recall_block: Some("Memory".to_string()),
            provider_context_refs: vec!["ctx-1".to_string()],
            linked_recall_block: Some("Linked".to_string()),
            channel_formatting_hints: Some("Use markdown".to_string()),
            runtime_capability_hint: Some("Runtime hints".to_string()),
            post_compaction_context: Some("Compacted".to_string()),
        };
        let skills = SkillContext {
            available_index_block: Some("### Available Skills\n- **rust-fix**: Repair".to_string()),
            active_skill_block: Some(
                "### Active Skills\n- **rust-fix** (v1, trusted): Repair".to_string(),
            ),
            ..SkillContext::default()
        };

        let result = assemble_workspace_prompt_materials(&materials, &skills);

        assert!(result.stable_snapshot.contains("You are ThinClaw."));
        assert!(result.stable_snapshot.contains("## Skills"));
        assert!(
            result
                .ephemeral_documents
                .iter()
                .any(|doc| doc.contains("Memory"))
        );
        assert!(
            result
                .ephemeral_documents
                .iter()
                .any(|doc| doc.contains("## Skill Expansion"))
        );
        assert_eq!(result.provider_context_refs, vec!["ctx-1"]);
        assert_eq!(
            result.segment_order,
            vec![
                "stable:workspace_prompt".to_string(),
                "stable:provider_system_prompt".to_string(),
                "stable:skills_index".to_string(),
                "ephemeral:provider_recall".to_string(),
                "ephemeral:linked_recall".to_string(),
                "ephemeral:channel_formatting_hints".to_string(),
                "ephemeral:runtime_capabilities".to_string(),
                "ephemeral:active_skills".to_string(),
                "ephemeral:post_compaction_fragment".to_string(),
            ]
        );
    }

    #[test]
    fn dispatcher_prompt_material_assembly_preserves_runtime_segment_policy() {
        let materials = DispatcherPromptMaterials {
            workspace_prompt: Some("Workspace".to_string()),
            provider_system_prompt: Some("Provider".to_string()),
            skill_index_context: Some("## Skills\nskills".to_string()),
            provider_recall_context: Some("## External Memory Recall\nmemory".to_string()),
            linked_recall_context: Some("## Linked Recall\nlinked".to_string()),
            channel_formatting_context: Some("## Platform Formatting (web)\nhints".to_string()),
            personality_overlay_context: Some("## Temporary Personality\n\nplayful".to_string()),
            runtime_capability_hint: Some("Runtime capability hints".to_string()),
            active_skill_context: Some("## Skill Expansion\nactive".to_string()),
            post_compaction_fragment: Some("Compacted".to_string()),
            provider_context_refs: vec!["ctx-1".to_string()],
        };

        let result = assemble_dispatcher_prompt_materials(&materials);

        assert!(result.stable_snapshot.contains("Workspace"));
        assert!(
            result
                .ephemeral_documents
                .iter()
                .any(|doc| doc == CHANNEL_TRANSCRIPT_GUIDANCE)
        );
        assert!(
            result
                .ephemeral_documents
                .iter()
                .any(|doc| doc.contains("Temporary Personality"))
        );
        assert_eq!(result.provider_context_refs, vec!["ctx-1"]);
        assert_eq!(
            result.segment_order,
            vec![
                "stable:workspace_prompt".to_string(),
                "stable:provider_system_prompt".to_string(),
                "stable:skills_index".to_string(),
                "ephemeral:transcript_guidance".to_string(),
                "ephemeral:provider_recall".to_string(),
                "ephemeral:linked_recall".to_string(),
                "ephemeral:channel_formatting_hints".to_string(),
                "ephemeral:personality_overlay".to_string(),
                "ephemeral:runtime_capabilities".to_string(),
                "ephemeral:active_skills".to_string(),
                "ephemeral:post_compaction_fragment".to_string(),
            ]
        );
    }
}
