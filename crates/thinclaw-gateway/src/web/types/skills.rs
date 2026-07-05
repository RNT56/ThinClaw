//! Skill registry, catalog search, and tap management DTOs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub trust: String,
    pub source: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SkillListResponse {
    pub skills: Vec<SkillInfo>,
    pub count: usize,
}

#[derive(Debug, Deserialize)]
pub struct SkillSearchRequest {
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillCatalogSearchResult {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub score: f64,
    #[serde(rename = "updatedAt", skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stars: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloads: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SkillSearchResponse {
    pub catalog: Vec<SkillCatalogSearchResult>,
    pub installed: Vec<SkillInfo>,
    pub registry_url: String,
    /// If the catalog registry was unreachable or errored, a human-readable message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SkillInstallRequest {
    pub name: String,
    pub url: Option<String>,
    pub content: Option<String>,
    /// When true, removes the existing skill before installing the new version.
    /// This enables atomic update/upgrade without requiring a separate remove call.
    #[serde(default)]
    pub force: Option<bool>,
}

/// Request to change a skill's trust level.
#[derive(Debug, Deserialize)]
pub struct SkillTrustRequest {
    /// Target trust level: "trusted" or "installed".
    pub trust: String,
}

#[derive(Debug, Deserialize)]
pub struct SkillInspectRequest {
    #[serde(default)]
    pub include_content: Option<bool>,
    #[serde(default)]
    pub include_files: Option<bool>,
    #[serde(default)]
    pub audit: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SkillPublishRequest {
    pub target_repo: String,
    #[serde(default)]
    pub dry_run: Option<bool>,
    #[serde(default)]
    pub remote_write: Option<bool>,
    #[serde(default)]
    pub confirm_remote_write: Option<bool>,
    #[serde(default)]
    pub approve_risky: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SkillTapAddRequest {
    pub repo: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub trust_level: Option<String>,
    #[serde(default)]
    pub replace: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SkillTapRemoveRequest {
    pub repo: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SkillTapRefreshRequest {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_catalog_search_result_preserves_existing_json_shape() {
        let result = SkillCatalogSearchResult {
            slug: "owner/example".to_string(),
            name: "Example".to_string(),
            description: "A catalog skill".to_string(),
            version: "1.2.3".to_string(),
            score: 0.95,
            updated_at: Some(1_700_000_000_000),
            stars: Some(42),
            downloads: Some(1000),
            owner: Some("owner".to_string()),
        };

        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::json!({
                "slug": "owner/example",
                "name": "Example",
                "description": "A catalog skill",
                "version": "1.2.3",
                "score": 0.95,
                "updatedAt": 1700000000000u64,
                "stars": 42,
                "downloads": 1000,
                "owner": "owner"
            })
        );
    }
}
