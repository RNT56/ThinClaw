//! Workspace memory browsing and search DTOs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct MemoryTreeResponse {
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Deserialize)]
pub struct TreeQuery {
    pub depth: Option<usize>,
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
}

#[derive(Debug, Deserialize)]
pub struct MemoryWriteRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct MemoryWriteResponse {
    pub path: String,
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct MemoryDeleteRequest {
    pub path: String,
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
