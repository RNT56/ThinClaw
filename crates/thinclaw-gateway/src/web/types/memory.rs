//! Workspace memory browsing and search DTOs.

use serde::{Deserialize, Serialize};

/// Requested authorization boundary for a memory API operation.
///
/// Omitted scopes retain the gateway's legacy role-derived behavior for API
/// compatibility. New user-facing clients should request `conversation`
/// explicitly; trusted operator surfaces may request `principal_admin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAccessScope {
    Conversation,
    PrincipalAdmin,
}

#[derive(Debug, Serialize)]
pub struct MemoryTreeResponse {
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Deserialize)]
pub struct TreeQuery {
    pub depth: Option<usize>,
    pub scope: Option<MemoryAccessScope>,
}

#[derive(Debug, Serialize)]
pub struct TreeEntry {
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct MemoryListResponse {
    pub path: String,
    pub entries: Vec<ListEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub path: Option<String>,
    pub scope: Option<MemoryAccessScope>,
}

#[derive(Debug, Serialize)]
pub struct ListEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MemoryReadResponse {
    pub path: String,
    pub content: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReadQuery {
    pub path: String,
    pub scope: Option<MemoryAccessScope>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryWriteRequest {
    pub path: String,
    pub content: String,
    pub scope: Option<MemoryAccessScope>,
}

#[derive(Debug, Serialize)]
pub struct MemoryWriteResponse {
    pub path: String,
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct MemoryDeleteRequest {
    pub path: String,
    pub scope: Option<MemoryAccessScope>,
}

#[derive(Debug, Serialize)]
pub struct MemoryDeleteResponse {
    pub path: String,
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct MemorySearchRequest {
    pub query: String,
    pub limit: Option<usize>,
    pub scope: Option<MemoryAccessScope>,
}

#[derive(Debug, Serialize)]
pub struct MemorySearchResponse {
    pub results: Vec<SearchHit>,
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub content: String,
    pub score: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_scope_is_explicit_but_optional_for_legacy_clients() {
        let scoped: MemoryWriteRequest = serde_json::from_value(serde_json::json!({
            "path": "MEMORY.md",
            "content": "fact",
            "scope": "conversation"
        }))
        .unwrap();
        assert_eq!(scoped.scope, Some(MemoryAccessScope::Conversation));

        let legacy: MemoryWriteRequest = serde_json::from_value(serde_json::json!({
            "path": "MEMORY.md",
            "content": "fact"
        }))
        .unwrap();
        assert_eq!(legacy.scope, None);
    }
}
