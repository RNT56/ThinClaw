//! Root-independent skill gateway response policies.

use axum::http::StatusCode;

use crate::web::types::{
    ActionResponse, SkillCatalogSearchResult, SkillInfo, SkillListResponse, SkillSearchResponse,
};

pub const SKILLS_SYSTEM_UNAVAILABLE_MESSAGE: &str = "Skills system not enabled";
pub const SKILL_CATALOG_UNAVAILABLE_MESSAGE: &str = "Skill catalog not available";
pub const SKILL_QUARANTINE_UNAVAILABLE_MESSAGE: &str = "Skill quarantine not available";
pub const SKILL_PUBLISH_REMOTE_WRITE_CONFIRMATION_MESSAGE: &str =
    "Skill publish remote write requires X-Confirm-Action: true header";
pub const SKILL_TAP_ADD_CONFIRMATION_MESSAGE: &str =
    "Skill tap add requires X-Confirm-Action: true header";
pub const SKILL_TAP_REMOVE_CONFIRMATION_MESSAGE: &str =
    "Skill tap remove requires X-Confirm-Action: true header";
pub const SKILL_TAP_REFRESH_CONFIRMATION_MESSAGE: &str =
    "Skill tap refresh requires X-Confirm-Action: true header";
pub const SKILL_INSTALL_CONFIRMATION_MESSAGE: &str =
    "Skill install requires X-Confirm-Action: true header";
pub const SKILL_REMOVAL_CONFIRMATION_MESSAGE: &str =
    "Skill removal requires X-Confirm-Action: true header";
pub const SKILL_TRUST_CONFIRMATION_MESSAGE: &str =
    "Trust changes require X-Confirm-Action: true header";
pub const SKILL_RELOAD_CONFIRMATION_MESSAGE: &str =
    "Skill reload requires X-Confirm-Action: true header";

pub fn skills_system_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::NOT_IMPLEMENTED,
        SKILLS_SYSTEM_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn skill_catalog_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::NOT_IMPLEMENTED,
        SKILL_CATALOG_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn skill_quarantine_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::NOT_IMPLEMENTED,
        SKILL_QUARANTINE_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn skill_publish_remote_write_confirmation_error() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        SKILL_PUBLISH_REMOTE_WRITE_CONFIRMATION_MESSAGE.to_string(),
    )
}

pub fn skill_tap_add_confirmation_error() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        SKILL_TAP_ADD_CONFIRMATION_MESSAGE.to_string(),
    )
}

pub fn skill_tap_remove_confirmation_error() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        SKILL_TAP_REMOVE_CONFIRMATION_MESSAGE.to_string(),
    )
}

pub fn skill_tap_refresh_confirmation_error() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        SKILL_TAP_REFRESH_CONFIRMATION_MESSAGE.to_string(),
    )
}

pub fn skill_install_confirmation_error() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        SKILL_INSTALL_CONFIRMATION_MESSAGE.to_string(),
    )
}

pub fn skill_removal_confirmation_error() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        SKILL_REMOVAL_CONFIRMATION_MESSAGE.to_string(),
    )
}

pub fn skill_trust_confirmation_error() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        SKILL_TRUST_CONFIRMATION_MESSAGE.to_string(),
    )
}

pub fn skill_reload_confirmation_error() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        SKILL_RELOAD_CONFIRMATION_MESSAGE.to_string(),
    )
}

pub fn invalid_skill_trust_level_error(level: impl AsRef<str>) -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        format!(
            "Invalid trust level '{}'. Must be 'trusted' or 'installed'.",
            level.as_ref()
        ),
    )
}

pub fn has_confirm_action_header(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("x-confirm-action")
        .and_then(|value| value.to_str().ok())
        == Some("true")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInfoInput {
    pub name: String,
    pub description: String,
    pub version: String,
    pub trust: String,
    pub source: String,
    pub keywords: Vec<String>,
}

pub fn skill_info(input: SkillInfoInput) -> SkillInfo {
    SkillInfo {
        name: input.name,
        description: input.description,
        version: input.version,
        trust: input.trust,
        source: input.source,
        keywords: input.keywords,
    }
}

pub fn skill_list_response(skills: Vec<SkillInfo>) -> SkillListResponse {
    let count = skills.len();
    SkillListResponse { skills, count }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillSearchMatchInput<'a> {
    pub name: &'a str,
    pub description: &'a str,
}

pub fn skill_matches_query(input: SkillSearchMatchInput<'_>, query: &str) -> bool {
    let query_lower = query.to_lowercase();
    input.name.to_lowercase().contains(&query_lower)
        || input.description.to_lowercase().contains(&query_lower)
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillCatalogSearchResultInput {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub score: f64,
    pub updated_at: Option<u64>,
    pub stars: Option<u64>,
    pub downloads: Option<u64>,
    pub owner: Option<String>,
}

pub fn skill_catalog_search_result(
    input: SkillCatalogSearchResultInput,
) -> SkillCatalogSearchResult {
    SkillCatalogSearchResult {
        slug: input.slug,
        name: input.name,
        description: input.description,
        version: input.version,
        score: input.score,
        updated_at: input.updated_at,
        stars: input.stars,
        downloads: input.downloads,
        owner: input.owner,
    }
}

pub fn skill_search_response(
    catalog: Vec<SkillCatalogSearchResult>,
    installed: Vec<SkillInfo>,
    registry_url: impl Into<String>,
    catalog_error: Option<String>,
) -> SkillSearchResponse {
    SkillSearchResponse {
        catalog,
        installed,
        registry_url: registry_url.into(),
        catalog_error,
    }
}

pub fn skill_action_error_response(message: impl Into<String>) -> ActionResponse {
    ActionResponse::fail(message)
}

pub fn skill_api_install_response(name: impl AsRef<str>) -> ActionResponse {
    ActionResponse::ok(format!("Installed skill '{}'", name.as_ref()))
}

pub fn skill_api_remove_response(name: impl AsRef<str>) -> ActionResponse {
    ActionResponse::ok(format!("Removed skill '{}'", name.as_ref()))
}

pub fn skill_install_missing_source_response() -> ActionResponse {
    ActionResponse::fail("Provide 'content' or 'url' to install a skill")
}

pub fn skill_duplicate_response(name: impl AsRef<str>) -> ActionResponse {
    ActionResponse::fail(format!(
        "Skill '{}' already exists (use force=true to update)",
        name.as_ref()
    ))
}

pub fn skill_install_commit_response(name: impl AsRef<str>, force: bool) -> ActionResponse {
    let action = if force { "updated" } else { "installed" };
    ActionResponse::ok(format!("Skill '{}' {}", name.as_ref(), action))
}

pub fn skill_remove_response(name: impl AsRef<str>) -> ActionResponse {
    ActionResponse::ok(format!("Skill '{}' removed", name.as_ref()))
}

pub fn skill_trust_response(name: impl AsRef<str>, trust_label: impl AsRef<str>) -> ActionResponse {
    ActionResponse::ok(format!(
        "Skill '{}' is now {}",
        name.as_ref(),
        trust_label.as_ref()
    ))
}

pub fn skill_reload_response(name: impl AsRef<str>) -> ActionResponse {
    ActionResponse::ok(format!("Skill '{}' reloaded from disk", name.as_ref()))
}

pub fn skill_reload_all_response(loaded: &[String]) -> ActionResponse {
    ActionResponse::ok(format!(
        "Reloaded {} skill(s): {}",
        loaded.len(),
        loaded.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_list_response_counts_rows() {
        let response = skill_list_response(vec![skill_info(SkillInfoInput {
            name: "compose".to_string(),
            description: "Write structured output".to_string(),
            version: "1.0.0".to_string(),
            trust: "trusted".to_string(),
            source: "User".to_string(),
            keywords: vec!["writing".to_string()],
        })]);
        let value = serde_json::to_value(response).expect("serialize skill list");

        assert_eq!(
            value,
            serde_json::json!({
                "skills": [{
                    "name": "compose",
                    "description": "Write structured output",
                    "version": "1.0.0",
                    "trust": "trusted",
                    "source": "User",
                    "keywords": ["writing"],
                }],
                "count": 1,
            })
        );
    }

    #[test]
    fn skill_search_match_checks_name_and_description() {
        let input = SkillSearchMatchInput {
            name: "compose",
            description: "Write structured output",
        };

        assert!(skill_matches_query(input, "COMP"));
        assert!(skill_matches_query(input, "structured"));
        assert!(!skill_matches_query(input, "deploy"));
    }

    #[test]
    fn skill_search_response_preserves_catalog_shape() {
        let catalog = vec![skill_catalog_search_result(SkillCatalogSearchResultInput {
            slug: "owner/compose".to_string(),
            name: "compose".to_string(),
            description: "Write structured output".to_string(),
            version: "1.0.0".to_string(),
            score: 0.9,
            updated_at: Some(42),
            stars: Some(7),
            downloads: None,
            owner: Some("owner".to_string()),
        })];

        let value = serde_json::to_value(skill_search_response(
            catalog,
            Vec::new(),
            "https://registry.example",
            Some("offline".to_string()),
        ))
        .expect("serialize skill search");

        assert_eq!(
            value,
            serde_json::json!({
                "catalog": [{
                    "slug": "owner/compose",
                    "name": "compose",
                    "description": "Write structured output",
                    "version": "1.0.0",
                    "score": 0.9,
                    "updatedAt": 42,
                    "stars": 7,
                    "owner": "owner",
                }],
                "installed": [],
                "registry_url": "https://registry.example",
                "catalog_error": "offline",
            })
        );
    }

    #[test]
    fn skill_availability_errors_preserve_web_statuses() {
        assert_eq!(
            skills_system_unavailable_error(),
            (
                StatusCode::NOT_IMPLEMENTED,
                SKILLS_SYSTEM_UNAVAILABLE_MESSAGE.to_string()
            )
        );
        assert_eq!(
            skill_catalog_unavailable_error(),
            (
                StatusCode::NOT_IMPLEMENTED,
                SKILL_CATALOG_UNAVAILABLE_MESSAGE.to_string()
            )
        );
        assert_eq!(
            skill_quarantine_unavailable_error(),
            (
                StatusCode::NOT_IMPLEMENTED,
                SKILL_QUARANTINE_UNAVAILABLE_MESSAGE.to_string()
            )
        );
    }

    #[test]
    fn skill_request_validation_errors_preserve_web_statuses() {
        let errors = [
            (
                skill_publish_remote_write_confirmation_error(),
                SKILL_PUBLISH_REMOTE_WRITE_CONFIRMATION_MESSAGE,
            ),
            (
                skill_tap_add_confirmation_error(),
                SKILL_TAP_ADD_CONFIRMATION_MESSAGE,
            ),
            (
                skill_tap_remove_confirmation_error(),
                SKILL_TAP_REMOVE_CONFIRMATION_MESSAGE,
            ),
            (
                skill_tap_refresh_confirmation_error(),
                SKILL_TAP_REFRESH_CONFIRMATION_MESSAGE,
            ),
            (
                skill_install_confirmation_error(),
                SKILL_INSTALL_CONFIRMATION_MESSAGE,
            ),
            (
                skill_removal_confirmation_error(),
                SKILL_REMOVAL_CONFIRMATION_MESSAGE,
            ),
            (
                skill_trust_confirmation_error(),
                SKILL_TRUST_CONFIRMATION_MESSAGE,
            ),
            (
                skill_reload_confirmation_error(),
                SKILL_RELOAD_CONFIRMATION_MESSAGE,
            ),
        ];

        for (actual, expected_message) in errors {
            assert_eq!(
                actual,
                (StatusCode::BAD_REQUEST, expected_message.to_string())
            );
        }
        assert_eq!(
            invalid_skill_trust_level_error("owner"),
            (
                StatusCode::BAD_REQUEST,
                "Invalid trust level 'owner'. Must be 'trusted' or 'installed'.".to_string()
            )
        );
    }

    #[test]
    fn confirm_action_header_requires_literal_true() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(!has_confirm_action_header(&headers));

        headers.insert(
            "x-confirm-action",
            axum::http::HeaderValue::from_static("true"),
        );
        assert!(has_confirm_action_header(&headers));

        headers.insert(
            "x-confirm-action",
            axum::http::HeaderValue::from_static("TRUE"),
        );
        assert!(!has_confirm_action_header(&headers));
    }

    #[test]
    fn skill_action_responses_preserve_existing_messages() {
        assert_eq!(
            serde_json::to_value(skill_api_install_response("compose")).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Installed skill 'compose'",
            })
        );
        assert_eq!(
            serde_json::to_value(skill_api_remove_response("compose")).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Removed skill 'compose'",
            })
        );
        assert_eq!(
            serde_json::to_value(skill_install_missing_source_response()).unwrap(),
            serde_json::json!({
                "success": false,
                "message": "Provide 'content' or 'url' to install a skill",
            })
        );
        assert_eq!(
            serde_json::to_value(skill_duplicate_response("compose")).unwrap(),
            serde_json::json!({
                "success": false,
                "message": "Skill 'compose' already exists (use force=true to update)",
            })
        );
        assert_eq!(
            serde_json::to_value(skill_install_commit_response("compose", false)).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Skill 'compose' installed",
            })
        );
        assert_eq!(
            serde_json::to_value(skill_install_commit_response("compose", true)).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Skill 'compose' updated",
            })
        );
        assert_eq!(
            serde_json::to_value(skill_remove_response("compose")).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Skill 'compose' removed",
            })
        );
        assert_eq!(
            serde_json::to_value(skill_trust_response("compose", "trusted")).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Skill 'compose' is now trusted",
            })
        );
        assert_eq!(
            serde_json::to_value(skill_reload_response("compose")).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Skill 'compose' reloaded from disk",
            })
        );
        assert_eq!(
            serde_json::to_value(skill_reload_all_response(&[
                "compose".to_string(),
                "deploy".to_string()
            ]))
            .unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Reloaded 2 skill(s): compose, deploy",
            })
        );
    }
}
