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
const MAX_GITHUB_METADATA_BYTES: usize = 32 * 1024 * 1024;
const MAX_SKILL_CONTENT_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DISCOVERED_SKILLS_PER_TAP: usize = 512;
const MAX_DISCOVERED_SKILLS_TOTAL: usize = 2048;
const MAX_GITHUB_SKILL_TAPS: usize = 64;
const MAX_GITHUB_TREE_ENTRIES: usize = 100_000;
const MAX_GITHUB_PATH_BYTES: usize = 1024;
const MAX_GITHUB_SHA_BYTES: usize = 128;
const MAX_GITHUB_DOWNLOAD_URL_BYTES: usize = 16 * 1024;
const MAX_GITHUB_DISCOVERY_DURATION: Duration = Duration::from_secs(120);
const MAX_GITHUB_TAP_DURATION: Duration = Duration::from_secs(30);

fn validate_tap_config(tap: &SkillTapConfig) -> anyhow::Result<()> {
    let repo_parts = tap.repo.split('/').collect::<Vec<_>>();
    let valid_repo = repo_parts.len() == 2
        && repo_parts.iter().all(|part| {
            !part.is_empty()
                && !matches!(*part, "." | "..")
                && part.len() <= 100
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        });
    let normalized_path = tap.path.trim_matches('/');
    let valid_path = tap.path.len() <= 1024
        && !tap.path.contains('\\')
        && !tap.path.chars().any(char::is_control)
        && normalized_path
            .split('/')
            .all(|segment| !matches!(segment, "." | ".."));
    let valid_branch = tap.branch.as_deref().is_none_or(valid_git_ref);
    if !valid_repo || !valid_path || !valid_branch {
        anyhow::bail!("invalid GitHub skill tap configuration for {:?}", tap.repo);
    }
    Ok(())
}

fn valid_git_ref(branch: &str) -> bool {
    !branch.is_empty()
        && branch.len() <= 255
        && !branch.chars().any(char::is_control)
        && !["..", "\\", "~", "^", ":", "?", "*", "[", "@{"]
            .iter()
            .any(|forbidden| branch.contains(forbidden))
        && !branch.starts_with('/')
        && !branch.ends_with('/')
        && !branch.ends_with('.')
}

fn valid_repo_path(path: &str) -> bool {
    let normalized = path.trim_matches('/');
    !normalized.is_empty()
        && path.len() <= MAX_GITHUB_PATH_BYTES
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && normalized
            .split('/')
            .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."))
}

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
    #[serde(default)]
    truncated: bool,
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
        if taps.len() > MAX_GITHUB_SKILL_TAPS {
            anyhow::bail!(
                "GitHub skill tap count exceeds the {}-tap limit",
                MAX_GITHUB_SKILL_TAPS
            );
        }
        for tap in &taps {
            validate_tap_config(tap)?;
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
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

    fn check_rate_limit(headers: &reqwest::header::HeaderMap) -> anyhow::Result<()> {
        let remaining = headers
            .get("x-ratelimit-remaining")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        if matches!(remaining, Some(0)) {
            anyhow::bail!("GitHub API rate limit is exhausted");
        }
        Ok(())
    }

    async fn send_github(
        &self,
        request: reqwest::RequestBuilder,
    ) -> anyhow::Result<reqwest::Response> {
        let response = request
            .send()
            .await
            .map_err(|error| anyhow::anyhow!(error.without_url().to_string()))?;
        Self::check_rate_limit(response.headers())?;
        if !response.status().is_success() {
            anyhow::bail!("GitHub API returned HTTP {}", response.status());
        }
        Ok(response)
    }

    async fn resolve_branch(&self, tap: &SkillTap) -> anyhow::Result<String> {
        if let Some(branch) = tap.branch.clone() {
            return Ok(branch);
        }

        let url = github_api_url(&tap.repo, &[])?;
        let response = self.send_github(self.client.get(url)).await?;
        let meta: RepoMeta = crate::http_response::bounded_json(response, 1024 * 1024).await?;
        if !valid_git_ref(&meta.default_branch) {
            anyhow::bail!("GitHub returned an invalid default branch");
        }
        Ok(meta.default_branch)
    }

    async fn fetch_manifest_for_path(
        &self,
        repo: &str,
        path: &str,
        branch: &str,
    ) -> anyhow::Result<(crate::skills::SkillManifest, String)> {
        if !valid_repo_path(path) || !valid_git_ref(branch) {
            anyhow::bail!("GitHub skill manifest path or branch is invalid");
        }
        let url = github_contents_url(repo, path, branch)?;
        let response = self.send_github(self.client.get(url)).await?;
        let payload: ContentsResponse =
            crate::http_response::bounded_json(response, MAX_SKILL_CONTENT_RESPONSE_BYTES).await?;
        validate_contents_response(&payload)?;
        let content = payload.content.replace('\n', "");
        let decoded = base64::engine::general_purpose::STANDARD.decode(content)?;
        let raw = String::from_utf8(decoded)?;
        let normalized = crate::skills::normalize_line_endings(&raw);
        let parsed = crate::skills::parser::parse_skill_md(&normalized)?;
        Ok((parsed.manifest, payload.sha))
    }

    pub async fn discover_skills(&self, tap: &SkillTap) -> anyhow::Result<Vec<RemoteSkill>> {
        let branch = self.resolve_branch(tap).await?;
        let mut url = github_api_url(&tap.repo, &["git", "trees", &branch])?;
        url.query_pairs_mut().append_pair("recursive", "1");
        let response = self.send_github(self.client.get(url)).await?;
        let tree: TreeResponse =
            crate::http_response::bounded_json(response, MAX_GITHUB_METADATA_BYTES).await?;
        if tree.truncated {
            anyhow::bail!(
                "GitHub returned a truncated repository tree for {}",
                tap.repo
            );
        }
        if tree.tree.len() > MAX_GITHUB_TREE_ENTRIES
            || tree.tree.iter().any(|entry| {
                !valid_repo_path(&entry.path)
                    || entry.kind.len() > 16
                    || entry.kind.chars().any(char::is_control)
            })
        {
            anyhow::bail!("GitHub returned malformed or excessive repository metadata");
        }

        let mut discovered = Vec::new();
        let prefix = tap.path.trim_matches('/');
        for entry in tree.tree {
            if entry.kind != "blob" || !entry.path.ends_with("SKILL.md") {
                continue;
            }
            if !prefix.is_empty()
                && entry.path != prefix
                && !entry
                    .path
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('/'))
            {
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
                manifest_url: Some(raw_manifest_url(&tap.repo, &branch, &entry.path)?.to_string()),
                manifest_digest: None,
                repo: Some(tap.repo.clone()),
                path: Some(entry.path),
                branch: Some(branch.clone()),
                trust_level: tap.trust_level,
            });
            if discovered.len() >= MAX_DISCOVERED_SKILLS_PER_TAP {
                tracing::warn!(
                    repo = %tap.repo,
                    limit = MAX_DISCOVERED_SKILLS_PER_TAP,
                    "GitHub skill discovery reached its per-tap limit"
                );
                break;
            }
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

        let started = Instant::now();
        let mut all = Vec::new();
        for tap in &self.taps {
            let Some(remaining) = MAX_GITHUB_DISCOVERY_DURATION.checked_sub(started.elapsed())
            else {
                tracing::warn!("GitHub skill discovery reached its total deadline");
                break;
            };
            match tokio::time::timeout(
                remaining.min(MAX_GITHUB_TAP_DURATION),
                self.discover_skills(tap),
            )
            .await
            {
                Ok(Ok(mut skills)) => {
                    let remaining_capacity = MAX_DISCOVERED_SKILLS_TOTAL.saturating_sub(all.len());
                    skills.truncate(remaining_capacity);
                    all.append(&mut skills);
                    if all.len() >= MAX_DISCOVERED_SKILLS_TOTAL {
                        tracing::warn!(
                            limit = MAX_DISCOVERED_SKILLS_TOTAL,
                            "GitHub skill discovery reached its total result limit"
                        );
                        break;
                    }
                }
                Ok(Err(error)) => {
                    tracing::warn!(repo = %tap.repo, error = %error, "Failed to discover GitHub skills");
                }
                Err(_) => {
                    tracing::warn!(repo = %tap.repo, "GitHub skill discovery timed out for tap")
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

fn valid_repo_parts(repo: &str) -> anyhow::Result<(&str, &str)> {
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("GitHub repository must be owner/name"))?;
    if name.contains('/')
        || [owner, name].iter().any(|part| {
            part.is_empty()
                || matches!(*part, "." | "..")
                || part.len() > 100
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        anyhow::bail!("GitHub repository identifier is invalid");
    }
    Ok((owner, name))
}

fn github_api_url(repo: &str, tail: &[&str]) -> anyhow::Result<reqwest::Url> {
    let (owner, name) = valid_repo_parts(repo)?;
    let mut url = reqwest::Url::parse(GITHUB_API)?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("GitHub API URL is not a valid base URL"))?;
        path.pop_if_empty();
        path.extend(["repos", owner, name]);
        for segment in tail {
            if segment.is_empty() || segment.chars().any(char::is_control) {
                anyhow::bail!("GitHub API path segment is invalid");
            }
            path.push(segment);
        }
    }
    Ok(url)
}

fn github_contents_url(repo: &str, path: &str, branch: &str) -> anyhow::Result<reqwest::Url> {
    if !valid_repo_path(path) || !valid_git_ref(branch) {
        anyhow::bail!("GitHub content path or branch is invalid");
    }
    let mut url = github_api_url(repo, &["contents"])?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("GitHub contents URL is not a valid base URL"))?;
        for segment in path.trim_matches('/').split('/') {
            segments.push(segment);
        }
    }
    url.query_pairs_mut().append_pair("ref", branch);
    Ok(url)
}

fn raw_manifest_url(repo: &str, branch: &str, path: &str) -> anyhow::Result<reqwest::Url> {
    let (owner, name) = valid_repo_parts(repo)?;
    if !valid_git_ref(branch) || !valid_repo_path(path) {
        anyhow::bail!("GitHub raw manifest path or branch is invalid");
    }
    let mut url = reqwest::Url::parse("https://raw.githubusercontent.com")?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("GitHub raw URL is not a valid base URL"))?;
        segments.pop_if_empty();
        segments.extend([owner, name, branch]);
        for segment in path.trim_matches('/').split('/') {
            segments.push(segment);
        }
    }
    Ok(url)
}

fn validate_contents_response(payload: &ContentsResponse) -> anyhow::Result<()> {
    if payload.content.len() > MAX_SKILL_CONTENT_RESPONSE_BYTES
        || payload.sha.is_empty()
        || payload.sha.len() > MAX_GITHUB_SHA_BYTES
        || !payload.sha.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        anyhow::bail!("GitHub returned malformed skill content metadata");
    }
    if let Some(download_url) = &payload.download_url {
        if download_url.is_empty()
            || download_url.len() > MAX_GITHUB_DOWNLOAD_URL_BYTES
            || download_url.chars().any(char::is_control)
        {
            anyhow::bail!("GitHub returned an invalid skill download URL");
        }
        let url = reqwest::Url::parse(download_url)?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url
                .host_str()
                .is_none_or(|host| !host.eq_ignore_ascii_case("raw.githubusercontent.com"))
        {
            anyhow::bail!("GitHub returned an untrusted skill download URL");
        }
    }
    Ok(())
}

#[async_trait]
impl RemoteSkillSource for GitHubSkillSource {
    fn adapter_name(&self) -> &'static str {
        "github_tap"
    }

    async fn search(&self, query: &str) -> anyhow::Result<Vec<RemoteSkill>> {
        if query.len() > 1024 || query.chars().any(char::is_control) {
            anyhow::bail!("GitHub skill search query is malformed or exceeds its size limit");
        }
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
        if name_or_slug.len() > 1024 || name_or_slug.chars().any(char::is_control) {
            anyhow::bail!("GitHub skill identifier is malformed or exceeds its size limit");
        }
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

        if !valid_repo_path(path) || !valid_git_ref(branch) {
            anyhow::bail!("GitHub remote skill path or branch is invalid");
        }
        let url = github_contents_url(repo, path, branch)?;
        let response = self.send_github(self.client.get(url)).await?;
        let payload: ContentsResponse =
            crate::http_response::bounded_json(response, MAX_SKILL_CONTENT_RESPONSE_BYTES).await?;
        validate_contents_response(&payload)?;
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

    #[test]
    fn tap_validation_rejects_ambiguous_repo_paths_and_refs() {
        let base = SkillTapConfig {
            repo: "owner/repo".to_string(),
            path: "skills".to_string(),
            branch: Some("main".to_string()),
            trust_level: SkillTapTrustLevel::Community,
        };
        assert!(validate_tap_config(&base).is_ok());
        assert!(
            validate_tap_config(&SkillTapConfig {
                repo: "owner/repo/extra".to_string(),
                ..base.clone()
            })
            .is_err()
        );
        assert!(
            validate_tap_config(&SkillTapConfig {
                path: "skills/../secrets".to_string(),
                ..base.clone()
            })
            .is_err()
        );
        assert!(
            validate_tap_config(&SkillTapConfig {
                branch: Some("main..other".to_string()),
                ..base
            })
            .is_err()
        );
    }
}
