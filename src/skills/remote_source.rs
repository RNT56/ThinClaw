use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use super::quarantine::SkillContent;
use crate::settings::{SkillTapConfig, SkillTapTrustLevel, WellKnownSkillRegistryConfig};

/// A remotely discoverable skill from a configured source adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSkill {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub source_adapter: String,
    pub source_label: String,
    pub source_ref: String,
    pub manifest_url: Option<String>,
    pub manifest_digest: Option<String>,
    pub repo: Option<String>,
    pub path: Option<String>,
    pub branch: Option<String>,
    pub trust_level: SkillTapTrustLevel,
}

#[async_trait]
pub trait RemoteSkillSource: Send + Sync {
    fn adapter_name(&self) -> &'static str;

    async fn search(&self, query: &str) -> anyhow::Result<Vec<RemoteSkill>>;

    async fn resolve_skill(&self, name_or_slug: &str) -> anyhow::Result<Option<RemoteSkill>>;

    async fn download_skill(&self, skill: &RemoteSkill) -> anyhow::Result<SkillContent>;
}

/// Aggregates multiple remote skill sources behind a single discovery API.
pub struct RemoteSkillHub {
    sources: Vec<Arc<dyn RemoteSkillSource>>,
}

impl RemoteSkillHub {
    pub fn new(sources: Vec<Arc<dyn RemoteSkillSource>>) -> Self {
        Self { sources }
    }

    pub fn is_enabled(&self) -> bool {
        !self.sources.is_empty()
    }

    pub async fn search(&self, query: &str) -> Vec<RemoteSkill> {
        let mut combined = Vec::new();

        for source in &self.sources {
            match source.search(query).await {
                Ok(skills) => combined.extend(skills),
                Err(error) => {
                    tracing::warn!(
                        adapter = source.adapter_name(),
                        error = %error,
                        "Remote skill source search failed"
                    );
                }
            }
        }

        dedupe_remote_skills(combined)
    }

    pub async fn resolve_skill(&self, name_or_slug: &str) -> Option<RemoteSkill> {
        for source in &self.sources {
            match source.resolve_skill(name_or_slug).await {
                Ok(Some(skill)) => return Some(skill),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        adapter = source.adapter_name(),
                        error = %error,
                        "Remote skill source resolution failed"
                    );
                }
            }
        }

        None
    }

    pub async fn download_skill(&self, skill: &RemoteSkill) -> anyhow::Result<SkillContent> {
        let Some(source) = self
            .sources
            .iter()
            .find(|source| source.adapter_name() == skill.source_adapter)
        else {
            anyhow::bail!(
                "No remote skill source registered for adapter '{}'",
                skill.source_adapter
            );
        };

        source.download_skill(skill).await
    }
}

/// Refreshable holder for the current remote skill sources.
#[derive(Clone, Default)]
pub struct SharedRemoteSkillHub {
    inner: Arc<RwLock<Option<Arc<RemoteSkillHub>>>>,
}

impl SharedRemoteSkillHub {
    pub fn new(hub: Option<Arc<RemoteSkillHub>>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(hub)),
        }
    }

    pub async fn get(&self) -> Option<Arc<RemoteSkillHub>> {
        self.inner.read().await.clone()
    }

    pub async fn replace(&self, hub: Option<Arc<RemoteSkillHub>>) {
        *self.inner.write().await = hub;
    }

    pub async fn is_enabled(&self) -> bool {
        self.get().await.is_some_and(|hub| hub.is_enabled())
    }

    pub async fn search(&self, query: &str) -> Vec<RemoteSkill> {
        match self.get().await {
            Some(hub) => hub.search(query).await,
            None => Vec::new(),
        }
    }

    pub async fn resolve_skill(&self, name_or_slug: &str) -> Option<RemoteSkill> {
        match self.get().await {
            Some(hub) => hub.resolve_skill(name_or_slug).await,
            None => None,
        }
    }

    pub async fn download_skill(&self, skill: &RemoteSkill) -> anyhow::Result<SkillContent> {
        let Some(hub) = self.get().await else {
            anyhow::bail!("No remote skill sources are configured");
        };
        hub.download_skill(skill).await
    }
}

pub fn build_remote_skill_hub(
    skill_taps: Vec<SkillTapConfig>,
    well_known_skill_registries: Vec<WellKnownSkillRegistryConfig>,
) -> Option<Arc<RemoteSkillHub>> {
    let mut remote_sources: Vec<Arc<dyn RemoteSkillSource>> = Vec::new();

    if !skill_taps.is_empty() {
        match crate::skills::github_source::GitHubSkillSource::new(skill_taps) {
            Ok(source) if source.is_enabled() => remote_sources.push(Arc::new(source)),
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(error = %error, "Failed to initialize GitHub skill source");
            }
        }
    }

    if !well_known_skill_registries.is_empty() {
        match crate::skills::well_known_source::WellKnownSkillSource::new(
            well_known_skill_registries,
        ) {
            Ok(source) if source.is_enabled() => remote_sources.push(Arc::new(source)),
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "Failed to initialize well-known skill source"
                );
            }
        }
    }

    if std::env::var("LOBEHUB_SKILLS_ENABLED")
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(true)
    {
        match crate::skills::lobe_source::LobeHubSkillSource::new(
            std::env::var("LOBEHUB_SKILLS_INDEX_URL").ok(),
        ) {
            Ok(source) => remote_sources.push(Arc::new(source)),
            Err(error) => {
                tracing::warn!(error = %error, "Failed to initialize LobeHub skill source");
            }
        }
    }

    if let Ok(index_url) = std::env::var("SKILLS_SH_INDEX_URL") {
        match crate::skills::skills_sh_source::SkillsShSource::new(index_url) {
            Ok(source) => remote_sources.push(Arc::new(source)),
            Err(error) => {
                tracing::warn!(error = %error, "Failed to initialize skills.sh source");
            }
        }
    }

    (!remote_sources.is_empty()).then(|| Arc::new(RemoteSkillHub::new(remote_sources)))
}

fn dedupe_remote_skills(skills: Vec<RemoteSkill>) -> Vec<RemoteSkill> {
    let mut seen_slugs = HashSet::new();
    let mut seen_names = HashSet::new();
    let mut deduped = Vec::new();

    for skill in skills {
        let slug_key = skill.slug.to_lowercase();
        let name_key = skill.name.to_lowercase();
        if seen_slugs.contains(&slug_key) || seen_names.contains(&name_key) {
            continue;
        }
        seen_slugs.insert(slug_key);
        seen_names.insert(name_key);
        deduped.push(skill);
    }

    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeSource {
        adapter: &'static str,
        skills: Vec<RemoteSkill>,
    }

    #[async_trait]
    impl RemoteSkillSource for FakeSource {
        fn adapter_name(&self) -> &'static str {
            self.adapter
        }

        async fn search(&self, _query: &str) -> anyhow::Result<Vec<RemoteSkill>> {
            Ok(self.skills.clone())
        }

        async fn resolve_skill(&self, name_or_slug: &str) -> anyhow::Result<Option<RemoteSkill>> {
            Ok(self
                .skills
                .iter()
                .find(|skill| {
                    skill.slug.eq_ignore_ascii_case(name_or_slug)
                        || skill.name.eq_ignore_ascii_case(name_or_slug)
                })
                .cloned())
        }

        async fn download_skill(&self, skill: &RemoteSkill) -> anyhow::Result<SkillContent> {
            Ok(SkillContent {
                raw_content: format!("---\nname: {}\n---\ncontent", skill.name),
                source_kind: skill.source_adapter.clone(),
                source_adapter: skill.source_adapter.clone(),
                source_ref: skill.source_ref.clone(),
                source_repo: skill.repo.clone(),
                source_url: skill.manifest_url.clone(),
                manifest_url: skill.manifest_url.clone(),
                manifest_digest: skill.manifest_digest.clone(),
                path: skill.path.clone(),
                branch: skill.branch.clone(),
                commit_sha: None,
                trust_level: skill.trust_level,
            })
        }
    }

    fn fake_skill(adapter: &str, slug: &str, name: &str) -> RemoteSkill {
        RemoteSkill {
            slug: slug.to_string(),
            name: name.to_string(),
            description: format!("{name} description"),
            version: "1.0.0".to_string(),
            source_adapter: adapter.to_string(),
            source_label: adapter.to_string(),
            source_ref: slug.to_string(),
            manifest_url: Some(format!("https://example.com/{name}/SKILL.md")),
            manifest_digest: Some(format!("digest-{name}")),
            repo: None,
            path: Some("SKILL.md".to_string()),
            branch: None,
            trust_level: SkillTapTrustLevel::Community,
        }
    }

    #[tokio::test]
    async fn hub_dedupes_by_skill_name_in_source_order() {
        let hub = RemoteSkillHub::new(vec![
            Arc::new(FakeSource {
                adapter: "github_tap",
                skills: vec![fake_skill("github_tap", "github:owner/demo", "demo")],
            }),
            Arc::new(FakeSource {
                adapter: "well_known",
                skills: vec![fake_skill(
                    "well_known",
                    "well_known:https://skills.example/demo",
                    "demo",
                )],
            }),
        ]);

        let results = hub.search("demo").await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source_adapter, "github_tap");
    }

    #[tokio::test]
    async fn hub_resolve_skill_prefers_first_matching_source() {
        let hub = RemoteSkillHub::new(vec![
            Arc::new(FakeSource {
                adapter: "github_tap",
                skills: vec![fake_skill("github_tap", "github:owner/demo", "demo")],
            }),
            Arc::new(FakeSource {
                adapter: "well_known",
                skills: vec![fake_skill(
                    "well_known",
                    "well_known:https://skills.example/demo",
                    "demo",
                )],
            }),
        ]);

        let resolved = hub
            .resolve_skill("demo")
            .await
            .expect("skill should resolve");

        assert_eq!(resolved.source_adapter, "github_tap");
        assert_eq!(resolved.slug, "github:owner/demo");
    }
}
