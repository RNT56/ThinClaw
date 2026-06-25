//! Memory / workspace proxy methods: file read/write/delete, listing, and
//! workspace memory search.

use super::core::RemoteGatewayProxy;

impl RemoteGatewayProxy {
    /// Read a workspace file.
    ///
    /// Remote endpoint: GET /api/memory/read?path={path}
    pub async fn get_file(&self, path: &str) -> Result<String, String> {
        let resp = self
            .get_json(&format!(
                "/api/memory/read?path={}",
                urlencoding::encode(path)
            ))
            .await?;

        // Gateway returns: { path, content, created_at, ... }
        Ok(resp
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }

    /// Write a workspace file.
    ///
    /// Remote endpoint: POST /api/memory/write
    pub async fn write_file(&self, path: &str, content: &str) -> Result<(), String> {
        self.post_json(
            "/api/memory/write",
            &serde_json::json!({
                "path": path,
                "content": content,
            }),
        )
        .await
        .map(|_| ())
    }

    /// Delete a workspace file.
    pub async fn delete_file(&self, path: &str) -> Result<(), String> {
        self.post_json("/api/memory/delete", &serde_json::json!({ "path": path }))
            .await
            .map(|_| ())
    }

    /// List all workspace files.
    ///
    /// Remote endpoint: GET /api/memory/list
    pub async fn list_files(&self) -> Result<Vec<String>, String> {
        let resp = self.get_json("/api/memory/tree").await?;
        let paths: Vec<String> = resp
            .get("entries")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|v| !v.get("is_dir").and_then(|d| d.as_bool()).unwrap_or(false))
                    .filter_map(|v| {
                        v.get("path")
                            .and_then(|p| p.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(paths)
    }

    /// Search workspace memory.
    ///
    /// Remote endpoint: POST /api/memory/search
    pub async fn search_memory(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<serde_json::Value, String> {
        self.post_json(
            "/api/memory/search",
            &serde_json::json!({
                "query": query,
                "limit": limit,
            }),
        )
        .await
    }
}
