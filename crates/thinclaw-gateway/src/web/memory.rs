//! Root-independent memory DTO projection helpers.

use axum::http::StatusCode;

use crate::web::types::{
    ListEntry, MemoryDeleteResponse, MemoryListResponse, MemoryReadResponse, MemorySearchResponse,
    MemoryTreeResponse, MemoryWriteResponse, SearchHit, TreeEntry,
};

pub const MEMORY_WORKSPACE_UNAVAILABLE_MESSAGE: &str = "Workspace not available";

pub fn memory_workspace_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        MEMORY_WORKSPACE_UNAVAILABLE_MESSAGE.to_string(),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct GatewayMemoryListSourceEntry {
    pub path: String,
    pub is_directory: bool,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GatewayMemorySearchHit {
    pub path: String,
    pub content: String,
    pub score: f64,
}

pub fn tree_entries_from_paths<'a>(paths: impl IntoIterator<Item = &'a String>) -> Vec<TreeEntry> {
    let mut entries = Vec::new();
    let mut seen_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();

    for path in paths {
        let parts: Vec<&str> = path.split('/').collect();
        for i in 0..parts.len().saturating_sub(1) {
            let dir_path = parts[..=i].join("/");
            if seen_dirs.insert(dir_path.clone()) {
                entries.push(TreeEntry {
                    path: dir_path,
                    is_dir: true,
                });
            }
        }
        entries.push(TreeEntry {
            path: path.clone(),
            is_dir: false,
        });
    }

    entries
}

pub fn list_entries_from_source(
    entries: impl IntoIterator<Item = GatewayMemoryListSourceEntry>,
) -> Vec<ListEntry> {
    entries
        .into_iter()
        .map(|entry| ListEntry {
            name: entry
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&entry.path)
                .to_string(),
            path: entry.path,
            is_dir: entry.is_directory,
            updated_at: entry.updated_at,
        })
        .collect()
}

pub fn memory_list_response(
    path: impl Into<String>,
    entries: Vec<ListEntry>,
) -> MemoryListResponse {
    MemoryListResponse {
        path: path.into(),
        entries,
    }
}

pub fn memory_tree_response(entries: Vec<TreeEntry>) -> MemoryTreeResponse {
    MemoryTreeResponse { entries }
}

pub fn memory_read_response(
    path: impl Into<String>,
    content: impl Into<String>,
    updated_at: Option<String>,
) -> MemoryReadResponse {
    MemoryReadResponse {
        path: path.into(),
        content: content.into(),
        updated_at,
    }
}

pub fn memory_write_response(path: impl Into<String>) -> MemoryWriteResponse {
    MemoryWriteResponse {
        path: path.into(),
        status: "written",
    }
}

pub fn memory_delete_response(path: impl Into<String>) -> MemoryDeleteResponse {
    MemoryDeleteResponse {
        path: path.into(),
        status: "deleted",
    }
}

pub fn memory_search_response_from_hits(
    hits: impl IntoIterator<Item = GatewayMemorySearchHit>,
) -> MemorySearchResponse {
    MemorySearchResponse {
        results: hits
            .into_iter()
            .map(|hit| SearchHit {
                path: hit.path,
                content: hit.content,
                score: hit.score,
            })
            .collect(),
    }
}

pub fn root_list_with_virtual_home_soul(
    mut entries: Vec<ListEntry>,
    soul_path: &str,
    home_soul_available: bool,
) -> Vec<ListEntry> {
    let has_soul = entries.iter().any(|entry| entry.path == soul_path);
    if !has_soul && home_soul_available {
        entries.insert(
            0,
            ListEntry {
                name: soul_path.to_string(),
                path: soul_path.to_string(),
                is_dir: false,
                updated_at: None,
            },
        );
    }
    entries
}

pub fn tree_with_virtual_home_soul(
    mut entries: Vec<TreeEntry>,
    soul_path: &str,
    soul_path_present: bool,
    home_soul_available: bool,
) -> Vec<TreeEntry> {
    if !soul_path_present && home_soul_available {
        entries.push(TreeEntry {
            path: soul_path.to_string(),
            is_dir: false,
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_unavailable_error_uses_service_unavailable() {
        assert_eq!(
            memory_workspace_unavailable_error(),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                MEMORY_WORKSPACE_UNAVAILABLE_MESSAGE.to_string()
            )
        );
    }

    #[test]
    fn root_list_injects_home_soul_once_at_front() {
        let entries = vec![ListEntry {
            name: "notes.md".to_string(),
            path: "notes.md".to_string(),
            is_dir: false,
            updated_at: None,
        }];

        let entries = root_list_with_virtual_home_soul(entries, "SOUL.md", true);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "SOUL.md");
        assert_eq!(entries[1].path, "notes.md");
    }

    #[test]
    fn tree_entries_from_paths_includes_parent_directories() {
        let paths = vec!["daily/today.md".to_string(), "MEMORY.md".to_string()];
        let entries = tree_entries_from_paths(&paths);

        assert!(
            entries
                .iter()
                .any(|entry| entry.path == "daily" && entry.is_dir)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.path == "daily/today.md" && !entry.is_dir)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.path == "MEMORY.md" && !entry.is_dir)
        );
    }

    #[test]
    fn list_entries_from_source_uses_last_path_segment_as_name() {
        let entries = list_entries_from_source(vec![GatewayMemoryListSourceEntry {
            path: "daily/today.md".to_string(),
            is_directory: false,
            updated_at: Some("now".to_string()),
        }]);

        assert_eq!(entries[0].name, "today.md");
        assert_eq!(entries[0].path, "daily/today.md");
        assert_eq!(entries[0].updated_at.as_deref(), Some("now"));
    }

    #[test]
    fn memory_response_wrappers_preserve_shapes() {
        let list = memory_list_response(
            "",
            vec![ListEntry {
                name: "today.md".to_string(),
                path: "daily/today.md".to_string(),
                is_dir: false,
                updated_at: None,
            }],
        );
        assert_eq!(list.path, "");
        assert_eq!(list.entries[0].name, "today.md");

        let tree = memory_tree_response(vec![TreeEntry {
            path: "daily".to_string(),
            is_dir: true,
        }]);
        assert_eq!(tree.entries[0].path, "daily");

        let read = memory_read_response("SOUL.md", "body", None);
        assert_eq!(read.path, "SOUL.md");
        assert_eq!(read.content, "body");

        assert_eq!(memory_write_response("SOUL.md").status, "written");
        assert_eq!(memory_delete_response("SOUL.md").status, "deleted");
    }

    #[test]
    fn memory_search_response_projects_hits() {
        let response = memory_search_response_from_hits(vec![GatewayMemorySearchHit {
            path: "MEMORY.md".to_string(),
            content: "hit".to_string(),
            score: 0.7,
        }]);

        assert_eq!(response.results[0].path, "MEMORY.md");
        assert_eq!(response.results[0].score, 0.7);
    }

    #[test]
    fn root_list_does_not_duplicate_existing_home_soul() {
        let entries = vec![ListEntry {
            name: "SOUL.md".to_string(),
            path: "SOUL.md".to_string(),
            is_dir: false,
            updated_at: None,
        }];

        let entries = root_list_with_virtual_home_soul(entries, "SOUL.md", true);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "SOUL.md");
    }

    #[test]
    fn tree_injects_home_soul_and_sorts() {
        let entries = vec![TreeEntry {
            path: "z.md".to_string(),
            is_dir: false,
        }];

        let entries = tree_with_virtual_home_soul(entries, "SOUL.md", false, true);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "SOUL.md");
        assert_eq!(entries[1].path, "z.md");
    }

    #[test]
    fn tree_skips_unavailable_home_soul() {
        let entries = tree_with_virtual_home_soul(Vec::new(), "SOUL.md", false, false);

        assert!(entries.is_empty());
    }
}
