//! Root skill-registry adapter for the extracted agent skill-context port.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::ports::{SkillContext, SkillContextPort, SkillContextRequest, SkillSummary};
use tokio::sync::RwLock;

use crate::config::SkillsConfig;
use crate::error::WorkspaceError;
use crate::skills::{LoadedSkill, SkillRegistry, prefilter_skills};

pub struct RootSkillContextPort {
    registry: Arc<RwLock<SkillRegistry>>,
    config: SkillsConfig,
}

impl RootSkillContextPort {
    pub fn shared(
        registry: Arc<RwLock<SkillRegistry>>,
        config: SkillsConfig,
    ) -> Arc<dyn SkillContextPort> {
        Arc::new(Self { registry, config })
    }
}

#[async_trait]
impl SkillContextPort for RootSkillContextPort {
    async fn skill_context(
        &self,
        request: SkillContextRequest,
    ) -> Result<SkillContext, WorkspaceError> {
        let guard = self.registry.read().await;
        let allowed_names = request
            .allowed_skills
            .as_ref()
            .map(|skills| skills.iter().map(String::as_str).collect::<HashSet<_>>());

        let available: Vec<LoadedSkill> = guard
            .skills()
            .iter()
            .filter(|skill| {
                allowed_names
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(skill.name()))
            })
            .cloned()
            .collect();

        let active = if request.include_active_matches {
            prefilter_skills(
                &request.user_input,
                &available,
                self.config.max_active_skills,
                self.config.max_context_tokens,
            )
            .into_iter()
            .cloned()
            .collect()
        } else {
            Vec::new()
        };

        let available_skills = available.iter().map(skill_summary).collect::<Vec<_>>();
        let active_skills = active.iter().map(skill_summary).collect::<Vec<_>>();
        let available_index_block = request
            .include_available_index
            .then(|| available_index_block(&available_skills))
            .flatten();
        let active_skill_block = request
            .include_active_matches
            .then(|| active_skill_block(&active_skills))
            .flatten();

        Ok(SkillContext {
            available_skills,
            active_skills,
            available_index_block,
            active_skill_block,
        })
    }

    async fn reload_skills(&self) -> Result<(), WorkspaceError> {
        self.registry.write().await.reload().await;
        Ok(())
    }
}

fn skill_summary(skill: &LoadedSkill) -> SkillSummary {
    SkillSummary {
        name: skill.name().to_string(),
        version: skill.version().to_string(),
        description: skill.manifest.description.clone(),
        trust: skill.trust.to_string(),
        path: skill.source.path().map(|path| path.display().to_string()),
    }
}

fn available_index_block(skills: &[SkillSummary]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut parts = vec!["### Available Skills".to_string()];
    for skill in skills {
        parts.push(format!("- **{}**: {}", skill.name, skill.description));
    }
    parts.push(
        "\nUse `skill_read` with a skill name to inspect full instructions before relying on a skill."
            .to_string(),
    );
    Some(parts.join("\n"))
}

fn active_skill_block(skills: &[SkillSummary]) -> Option<String> {
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
    parts.push(
        "\nUse `skill_read` with the skill name to load full instructions before using a skill."
            .to_string(),
    );
    Some(parts.join("\n"))
}

trait SkillSourcePath {
    fn path(&self) -> Option<&std::path::Path>;
}

impl SkillSourcePath for crate::skills::SkillSource {
    fn path(&self) -> Option<&std::path::Path> {
        match self {
            Self::Workspace(path)
            | Self::User(path)
            | Self::Bundled(path)
            | Self::External(path) => Some(path.as_path()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_blocks_match_dispatcher_shape() {
        let skills = vec![SkillSummary {
            name: "rust-fix".to_string(),
            version: "1.0.0".to_string(),
            description: "Repair Rust compiler errors".to_string(),
            trust: "trusted".to_string(),
            path: None,
        }];

        let available = available_index_block(&skills).expect("available block");
        assert!(available.contains("### Available Skills"));
        assert!(available.contains("rust-fix"));

        let active = active_skill_block(&skills).expect("active block");
        assert!(active.contains("v1.0.0, trusted"));
        assert!(active.contains("skill_read"));
    }
}
