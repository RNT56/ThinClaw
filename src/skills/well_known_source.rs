use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use url::Url;

use crate::settings::{SkillTapTrustLevel, WellKnownSkillRegistryConfig};

use super::quarantine::SkillContent;
use super::remote_source::{RemoteSkill, RemoteSkillSource};

const CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct WellKnownRegistry {
    pub url: String,
    pub trust_level: SkillTapTrustLevel,
}

impl From<WellKnownSkillRegistryConfig> for WellKnownRegistry {
    fn from(value: WellKnownSkillRegistryConfig) -> Self {
        Self {
            url: value.url,
            trust_level: value.trust_level,
        }
    }
}

struct CachedRemoteSkills {
    skills: Vec<RemoteSkill>,
    fetched_at: Instant,
}

#[derive(Debug, Deserialize)]
struct WellKnownIndex {
    #[serde(default)]
    skills: Vec<WellKnownIndexEntry>,
}

#[derive(Debug, Deserialize)]
struct WellKnownIndexEntry {
    #[serde(default, rename = "name")]
    _name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    files: Vec<String>,
}

pub struct WellKnownSkillSource {
    client: reqwest::Client,
    registries: Vec<WellKnownRegistry>,
    cache: RwLock<Option<CachedRemoteSkills>>,
}

impl WellKnownSkillSource {
    pub fn new(registries: Vec<WellKnownSkillRegistryConfig>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("thinclaw/", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self {
            client,
            registries: registries.into_iter().map(Into::into).collect(),
            cache: RwLock::new(None),
        })
    }

    pub fn is_enabled(&self) -> bool {
        !self.registries.is_empty()
    }

    fn resolve_index_url(base: &str) -> anyhow::Result<Url> {
        let parsed = Url::parse(base)?;
        if parsed.path().ends_with("/index.json") {
            return Ok(parsed);
        }
        if parsed.path().contains("/.well-known/skills") {
            let mut normalized = parsed;
            if !normalized.path().ends_with('/') {
                let mut path = normalized.path().to_string();
                path.push('/');
                normalized.set_path(&path);
            }
            return Ok(normalized.join("index.json")?);
        }
        Ok(parsed.join("/.well-known/skills/index.json")?)
    }

    fn manifest_path(entry: &WellKnownIndexEntry) -> Option<&str> {
        entry
            .files
            .iter()
            .find(|file| file.ends_with("SKILL.md"))
            .map(String::as_str)
            .or_else(|| entry.files.first().map(String::as_str))
    }

    async fn discover_registry(
        &self,
        registry: &WellKnownRegistry,
    ) -> anyhow::Result<Vec<RemoteSkill>> {
        let index_url = Self::resolve_index_url(&registry.url)?;
        let response = self.client.get(index_url.clone()).send().await?;
        let response = response.error_for_status()?;
        let index: WellKnownIndex = response.json().await?;

        let mut discovered = Vec::new();
        for entry in index.skills {
            let Some(path) = Self::manifest_path(&entry).map(str::to_string) else {
                continue;
            };
            let manifest_url = index_url.join(&path)?;
            let raw = self
                .client
                .get(manifest_url.clone())
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let normalized = crate::skills::normalize_line_endings(&raw);
            let parsed = crate::skills::parser::parse_skill_md(&normalized)?;
            let digest = format!("{:x}", Sha256::digest(normalized.as_bytes()));
            discovered.push(RemoteSkill {
                slug: format!("well_known:{}#{}", registry.url, parsed.manifest.name),
                name: parsed.manifest.name,
                description: if parsed.manifest.description.is_empty() {
                    entry.description.clone()
                } else {
                    parsed.manifest.description
                },
                version: if parsed.manifest.version == "0.0.0" {
                    entry.version.unwrap_or_else(|| "0.0.0".to_string())
                } else {
                    parsed.manifest.version
                },
                source_adapter: "well_known".to_string(),
                source_label: registry.url.clone(),
                source_ref: format!("{}#{}", registry.url, path),
                manifest_url: Some(manifest_url.to_string()),
                manifest_digest: Some(digest),
                repo: None,
                path: Some(path),
                branch: None,
                trust_level: registry.trust_level,
            });
        }

        Ok(discovered)
    }

    async fn discover_all(&self) -> anyhow::Result<Vec<RemoteSkill>> {
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.as_ref()
                && cached.fetched_at.elapsed() < CACHE_TTL
            {
                return Ok(cached.skills.clone());
            }
        }

        let mut all = Vec::new();
        for registry in &self.registries {
            match self.discover_registry(registry).await {
                Ok(mut skills) => all.append(&mut skills),
                Err(error) => {
                    tracing::warn!(
                        registry = %registry.url,
                        error = %error,
                        "Failed to discover .well-known skills"
                    );
                }
            }
        }

        let mut cache = self.cache.write().await;
        *cache = Some(CachedRemoteSkills {
            skills: all.clone(),
            fetched_at: Instant::now(),
        });
        Ok(all)
    }
}

#[async_trait]
impl RemoteSkillSource for WellKnownSkillSource {
    fn adapter_name(&self) -> &'static str {
        "well_known"
    }

    async fn search(&self, query: &str) -> anyhow::Result<Vec<RemoteSkill>> {
        let query_lower = query.to_lowercase();
        let all = self.discover_all().await?;
        Ok(all
            .into_iter()
            .filter(|skill| {
                skill.slug.to_lowercase().contains(&query_lower)
                    || skill.name.to_lowercase().contains(&query_lower)
                    || skill.description.to_lowercase().contains(&query_lower)
                    || skill.source_label.to_lowercase().contains(&query_lower)
            })
            .collect())
    }

    async fn resolve_skill(&self, name_or_slug: &str) -> anyhow::Result<Option<RemoteSkill>> {
        let all = self.discover_all().await?;
        Ok(all.into_iter().find(|skill| {
            skill.slug.eq_ignore_ascii_case(name_or_slug)
                || skill.name.eq_ignore_ascii_case(name_or_slug)
        }))
    }

    async fn download_skill(&self, skill: &RemoteSkill) -> anyhow::Result<SkillContent> {
        let manifest_url = skill
            .manifest_url
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Well-known skill is missing manifest_url"))?;
        let raw_content = self
            .client
            .get(manifest_url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        Ok(SkillContent {
            raw_content,
            source_kind: "well_known".to_string(),
            source_adapter: "well_known".to_string(),
            source_ref: skill.source_ref.clone(),
            source_repo: None,
            source_url: Some(skill.source_label.clone()),
            manifest_url: Some(manifest_url.clone()),
            manifest_digest: skill.manifest_digest.clone(),
            path: skill.path.clone(),
            branch: None,
            commit_sha: None,
            trust_level: skill.trust_level,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_index_url_handles_origin_and_full_path() {
        assert_eq!(
            WellKnownSkillSource::resolve_index_url("https://skills.example")
                .unwrap()
                .as_str(),
            "https://skills.example/.well-known/skills/index.json"
        );
        assert_eq!(
            WellKnownSkillSource::resolve_index_url(
                "https://skills.example/.well-known/skills/index.json",
            )
            .unwrap()
            .as_str(),
            "https://skills.example/.well-known/skills/index.json"
        );
        assert_eq!(
            WellKnownSkillSource::resolve_index_url("https://skills.example/.well-known/skills")
                .unwrap()
                .as_str(),
            "https://skills.example/.well-known/skills/index.json"
        );
    }

    #[test]
    fn manifest_path_prefers_skill_md() {
        let entry = WellKnownIndexEntry {
            _name: "demo".to_string(),
            description: String::new(),
            version: None,
            files: vec!["README.md".to_string(), "skills/demo/SKILL.md".to_string()],
        };

        assert_eq!(
            WellKnownSkillSource::manifest_path(&entry),
            Some("skills/demo/SKILL.md")
        );
    }
}
