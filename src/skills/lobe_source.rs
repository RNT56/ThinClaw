use async_trait::async_trait;
use serde_json::Value;

use super::quarantine::SkillContent;
use super::remote_source::{RemoteSkill, RemoteSkillSource};
use crate::settings::SkillTapTrustLevel;

const DEFAULT_LOBEHUB_SKILLS_URL: &str = "https://lobehub.com/api/skill-tower/skills";
const MAX_INDEX_BYTES: usize = 8 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_SKILLS_PER_INDEX: usize = 512;

pub struct LobeHubSkillSource {
    index_url: String,
}

impl LobeHubSkillSource {
    pub fn new(index_url: Option<String>) -> anyhow::Result<Self> {
        Ok(Self {
            index_url: index_url.unwrap_or_else(|| DEFAULT_LOBEHUB_SKILLS_URL.to_string()),
        })
    }

    fn parse_index(&self, value: Value) -> Vec<RemoteSkill> {
        let entries = value
            .as_array()
            .cloned()
            .or_else(|| value.get("skills").and_then(Value::as_array).cloned())
            .or_else(|| value.get("data").and_then(Value::as_array).cloned())
            .unwrap_or_default();

        entries
            .into_iter()
            .take(MAX_SKILLS_PER_INDEX)
            .filter_map(|entry| self.skill_from_entry(entry))
            .collect()
    }

    fn skill_from_entry(&self, entry: Value) -> Option<RemoteSkill> {
        let slug = first_string(&entry, &["slug", "identifier", "id", "name"])?;
        let name = first_string(&entry, &["name", "title", "displayName"]).unwrap_or(slug.clone());
        let description = first_string(&entry, &["description", "summary", "desc"])
            .unwrap_or_else(|| "LobeHub community skill".to_string());
        let manifest_url = first_string(
            &entry,
            &[
                "skillUrl",
                "skill_url",
                "manifestUrl",
                "manifest_url",
                "url",
            ],
        );
        Some(RemoteSkill {
            slug: format!("lobehub:{slug}"),
            name,
            description,
            version: first_string(&entry, &["version"]).unwrap_or_else(|| "unknown".to_string()),
            source_adapter: "lobehub".to_string(),
            source_label: "LobeHub".to_string(),
            source_ref: slug,
            manifest_url,
            manifest_digest: first_string(&entry, &["digest", "sha256", "hash"]),
            repo: first_string(&entry, &["repo", "repository"]),
            path: Some("SKILL.md".to_string()),
            branch: first_string(&entry, &["branch"]),
            trust_level: SkillTapTrustLevel::Community,
        })
    }
}

#[async_trait]
impl RemoteSkillSource for LobeHubSkillSource {
    fn adapter_name(&self) -> &'static str {
        "lobehub"
    }

    async fn search(&self, query: &str) -> anyhow::Result<Vec<RemoteSkill>> {
        let response = super::remote_http::get_public_https(
            &self.index_url,
            std::time::Duration::from_secs(15),
        )
        .await?;
        let value = crate::http_response::bounded_json::<Value>(response, MAX_INDEX_BYTES).await?;
        let query = query.to_ascii_lowercase();
        Ok(self
            .parse_index(value)
            .into_iter()
            .filter(|skill| {
                query.is_empty()
                    || skill.name.to_ascii_lowercase().contains(&query)
                    || skill.description.to_ascii_lowercase().contains(&query)
                    || skill.slug.to_ascii_lowercase().contains(&query)
            })
            .collect())
    }

    async fn resolve_skill(&self, name_or_slug: &str) -> anyhow::Result<Option<RemoteSkill>> {
        Ok(self.search(name_or_slug).await?.into_iter().find(|skill| {
            skill.slug.eq_ignore_ascii_case(name_or_slug)
                || skill.source_ref.eq_ignore_ascii_case(name_or_slug)
                || skill.name.eq_ignore_ascii_case(name_or_slug)
        }))
    }

    async fn download_skill(&self, skill: &RemoteSkill) -> anyhow::Result<SkillContent> {
        let raw_content = if let Some(url) = &skill.manifest_url {
            let response =
                super::remote_http::get_public_https(url, std::time::Duration::from_secs(15))
                    .await?;
            crate::http_response::bounded_text(response, MAX_MANIFEST_BYTES).await?
        } else {
            synthesized_skill_md(skill)
        };

        Ok(SkillContent {
            raw_content,
            source_kind: "marketplace".to_string(),
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

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

fn synthesized_skill_md(skill: &RemoteSkill) -> String {
    format!(
        "---\nname: {}\ndescription: {}\nsource_tier: community\n---\n\n{}\n",
        skill.name, skill.description, skill.description
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lobehub_index_shapes() {
        let source = LobeHubSkillSource::new(Some("https://example.test".to_string())).unwrap();
        let skills = source.parse_index(serde_json::json!({
            "data": [{ "identifier": "demo", "title": "Demo", "description": "Does demo work" }]
        }));
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source_adapter, "lobehub");
        assert_eq!(skills[0].name, "Demo");
    }
}
