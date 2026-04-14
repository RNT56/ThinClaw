use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::settings::{SkillTapConfig, SkillTapTrustLevel};

use super::quarantine::SkillContent;
use super::remote_source::{RemoteSkill, RemoteSkillSource};

const GITHUB_API: &str = "https://api.github.com";
const CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct SkillTap {
    pub repo: String,
    pub path: String,
    pub branch: Option<String>,
    pub trust_level: SkillTapTrustLevel,
}

impl From<SkillTapConfig> for SkillTap {
    fn from(value: SkillTapConfig) -> Self {
        Self {
            repo: value.repo,
            path: value.path.trim_matches('/').to_string(),
            branch: value.branch,
            trust_level: value.trust_level,
        }
    }
}

struct CachedRemoteSkills {
    skills: Vec<RemoteSkill>,
    fetched_at: Instant,
}

#[derive(Deserialize)]
struct RepoMeta {
    default_branch: String,
}

#[derive(Deserialize)]
struct TreeResponse {
    tree: Vec<TreeEntry>,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct ContentsResponse {
    content: String,
    sha: String,
    download_url: Option<String>,
}

pub struct GitHubSkillSource {
    client: reqwest::Client,
    taps: Vec<SkillTap>,
    cache: RwLock<Option<CachedRemoteSkills>>,
}

impl GitHubSkillSource {
    pub fn new(taps: Vec<SkillTapConfig>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("thinclaw/", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self {
            client,
            taps: taps.into_iter().map(Into::into).collect(),
            cache: RwLock::new(None),
        })
    }

    pub fn is_enabled(&self) -> bool {
        !self.taps.is_empty()
    }

    async fn maybe_sleep_for_rate_limit(&self, headers: &reqwest::header::HeaderMap) {
        let remaining = headers
            .get("x-ratelimit-remaining")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        let reset = headers
            .get("x-ratelimit-reset")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok());

        if matches!(remaining, Some(0))
            && let Some(reset) = reset
        {
            let now = chrono::Utc::now().timestamp();
            let wait = (reset - now).max(1) as u64;
            tokio::time::sleep(Duration::from_secs(wait.min(60))).await;
        }
    }

    async fn resolve_branch(&self, tap: &SkillTap) -> anyhow::Result<String> {
        if let Some(branch) = tap.branch.clone() {
            return Ok(branch);
        }

        let url = format!("{GITHUB_API}/repos/{}", tap.repo);
        let response = self.client.get(url).send().await?;
        self.maybe_sleep_for_rate_limit(response.headers()).await;
        let response = response.error_for_status()?;
        let meta: RepoMeta = response.json().await?;
        Ok(meta.default_branch)
    }

    async fn fetch_manifest_for_path(
        &self,
        repo: &str,
        path: &str,
        branch: &str,
    ) -> anyhow::Result<(crate::skills::SkillManifest, String)> {
        let url = format!("{GITHUB_API}/repos/{repo}/contents/{path}");
        let response = self
            .client
            .get(url)
            .query(&[("ref", branch)])
            .send()
            .await?;
        self.maybe_sleep_for_rate_limit(response.headers()).await;
        let response = response.error_for_status()?;
        let payload: ContentsResponse = response.json().await?;
        let content = payload.content.replace('\n', "");
        let decoded = base64::engine::general_purpose::STANDARD.decode(content)?;
        let raw = String::from_utf8(decoded)?;
        let normalized = crate::skills::normalize_line_endings(&raw);
        let parsed = crate::skills::parser::parse_skill_md(&normalized)?;
        Ok((parsed.manifest, payload.sha))
    }

    pub async fn discover_skills(&self, tap: &SkillTap) -> anyhow::Result<Vec<RemoteSkill>> {
        let branch = self.resolve_branch(tap).await?;
        let url = format!(
            "{GITHUB_API}/repos/{}/git/trees/{}?recursive=1",
            tap.repo, branch
        );
        let response = self.client.get(url).send().await?;
        self.maybe_sleep_for_rate_limit(response.headers()).await;
        let response = response.error_for_status()?;
        let tree: TreeResponse = response.json().await?;

        let mut discovered = Vec::new();
        let prefix = tap.path.trim_matches('/');
        for entry in tree.tree {
            if entry.kind != "blob" || !entry.path.ends_with("SKILL.md") {
                continue;
            }
            if !prefix.is_empty() && !entry.path.starts_with(prefix) {
                continue;
            }

            let (manifest, _sha) = self
                .fetch_manifest_for_path(&tap.repo, &entry.path, &branch)
                .await?;
            discovered.push(RemoteSkill {
                slug: format!("github:{}/{}", tap.repo, manifest.name),
                name: manifest.name,
                description: manifest.description,
                version: manifest.version,
                source_adapter: "github_tap".to_string(),
                source_label: tap.repo.clone(),
                source_ref: format!("github:{}/{}@{}", tap.repo, entry.path, branch),
                manifest_url: Some(format!(
                    "https://raw.githubusercontent.com/{}/{}/{}",
                    tap.repo, branch, entry.path
                )),
                manifest_digest: None,
                repo: Some(tap.repo.clone()),
                path: Some(entry.path),
                branch: Some(branch.clone()),
                trust_level: tap.trust_level,
            });
        }

        Ok(discovered)
    }

    pub async fn discover_all(&self) -> anyhow::Result<Vec<RemoteSkill>> {
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.as_ref()
                && cached.fetched_at.elapsed() < CACHE_TTL
            {
                return Ok(cached.skills.clone());
            }
        }

        let mut all = Vec::new();
        for tap in &self.taps {
            match self.discover_skills(tap).await {
                Ok(mut skills) => all.append(&mut skills),
                Err(error) => {
                    tracing::warn!(repo = %tap.repo, error = %error, "Failed to discover GitHub skills");
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
impl RemoteSkillSource for GitHubSkillSource {
    fn adapter_name(&self) -> &'static str {
        "github_tap"
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
            })
            .collect())
    }

    async fn resolve_skill(&self, name_or_slug: &str) -> anyhow::Result<Option<RemoteSkill>> {
        let all = self.discover_all().await?;
        Ok(all.into_iter().find(|skill| {
            skill.slug.eq_ignore_ascii_case(name_or_slug)
                || skill.name.eq_ignore_ascii_case(name_or_slug)
                || skill.repo.as_ref().is_some_and(|repo| {
                    format!("{repo}/{}", skill.name).eq_ignore_ascii_case(name_or_slug)
                })
        }))
    }

    async fn download_skill(&self, skill: &RemoteSkill) -> anyhow::Result<SkillContent> {
        let repo = skill
            .repo
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("GitHub remote skill is missing repo metadata"))?;
        let path = skill
            .path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("GitHub remote skill is missing path metadata"))?;
        let branch = skill
            .branch
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("GitHub remote skill is missing branch metadata"))?;

        let url = format!("{GITHUB_API}/repos/{repo}/contents/{path}");
        let response = self
            .client
            .get(url)
            .query(&[("ref", branch)])
            .send()
            .await?;
        self.maybe_sleep_for_rate_limit(response.headers()).await;
        let response = response.error_for_status()?;
        let payload: ContentsResponse = response.json().await?;
        let content = payload.content.replace('\n', "");
        let decoded = base64::engine::general_purpose::STANDARD.decode(content)?;
        Ok(SkillContent {
            raw_content: String::from_utf8(decoded)?,
            source_kind: "github_tap".to_string(),
            source_adapter: "github_tap".to_string(),
            source_ref: skill.slug.clone(),
            source_repo: Some(repo.clone()),
            source_url: payload.download_url,
            manifest_url: skill.manifest_url.clone(),
            manifest_digest: skill.manifest_digest.clone().or(Some(payload.sha.clone())),
            path: Some(path.clone()),
            branch: Some(branch.to_string()),
            commit_sha: Some(payload.sha),
            trust_level: skill.trust_level,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_config_conversion_preserves_trust() {
        let tap = SkillTap::from(SkillTapConfig {
            repo: "owner/repo".to_string(),
            path: "skills".to_string(),
            branch: Some("main".to_string()),
            trust_level: SkillTapTrustLevel::Trusted,
        });

        assert_eq!(tap.repo, "owner/repo");
        assert_eq!(tap.path, "skills");
        assert_eq!(tap.branch.as_deref(), Some("main"));
        assert_eq!(tap.trust_level, SkillTapTrustLevel::Trusted);
    }
}
