//! Memory / workspace proxy methods: file read/write/delete, listing, and
//! workspace memory search.

use super::core::RemoteGatewayProxy;

const ACTOR_IDENTITY_ALIAS: &str = "actor/IDENTITY.md";

fn memory_target_for_path(path: &str) -> Result<(&str, &'static str), String> {
    let normalized = path.trim().trim_matches('/');
    if normalized == "actors"
        || normalized.starts_with("actors/")
        || normalized == "conversations"
        || normalized.starts_with("conversations/")
        || normalized == ".thinclaw"
        || normalized.starts_with(".thinclaw/")
    {
        return Err(
            "canonical actor/conversation storage paths are hidden; use a caller-relative path"
                .to_string(),
        );
    }
    if normalized == ACTOR_IDENTITY_ALIAS {
        return Ok(("IDENTITY.md", "conversation"));
    }
    if normalized == "actor" || normalized.starts_with("actor/") {
        return Err(
            "actor/ is a reserved desktop alias; only actor/IDENTITY.md is supported".to_string(),
        );
    }
    if thinclaw_core::workspace::is_control_plane_path(path) {
        Ok((path, "principal_admin"))
    } else {
        Ok((path, "conversation"))
    }
}

fn visible_control_path(path: &str) -> bool {
    let normalized = path.trim().trim_matches('/');
    thinclaw_core::workspace::is_control_plane_path(normalized)
        && !normalized.starts_with("actors/")
        && normalized != "actors"
        && !normalized.starts_with("conversations/")
        && normalized != "conversations"
        && normalized != ".thinclaw"
        && !normalized.starts_with(".thinclaw/")
}

impl RemoteGatewayProxy {
    /// Read a workspace file.
    ///
    /// Remote endpoint: GET /api/memory/read?path={path}
    pub async fn get_file(&self, path: &str) -> Result<String, String> {
        let (target_path, scope) = memory_target_for_path(path)?;
        let resp = self
            .get_json(&format!(
                "/api/memory/read?path={}&scope={}",
                urlencoding::encode(target_path),
                scope,
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
        let (target_path, scope) = memory_target_for_path(path)?;
        self.post_json(
            "/api/memory/write",
            &serde_json::json!({
                "path": target_path,
                "content": content,
                "scope": scope,
            }),
        )
        .await
        .map(|_| ())
    }

    /// Delete a workspace file.
    pub async fn delete_file(&self, path: &str) -> Result<(), String> {
        let (target_path, scope) = memory_target_for_path(path)?;
        self.post_json(
            "/api/memory/delete",
            &serde_json::json!({
                "path": target_path,
                "scope": scope,
            }),
        )
        .await
        .map(|_| ())
    }

    /// List all workspace files.
    ///
    /// Remote endpoint: GET /api/memory/list
    pub async fn list_files(&self) -> Result<Vec<String>, String> {
        // The desktop Brain is a deliberate composite view: trusted runtime
        // control files plus the current actor's caller-relative knowledge.
        // Never expose sibling actor or group namespaces merely because the
        // desktop connects with the primary Admin token.
        let (control, conversation) = tokio::try_join!(
            self.get_json("/api/memory/tree?scope=principal_admin"),
            self.get_json("/api/memory/tree?scope=conversation"),
        )?;
        let mut paths = std::collections::BTreeSet::new();
        for (response, control_view) in [(control, true), (conversation, false)] {
            if let Some(entries) = response.get("entries").and_then(|value| value.as_array()) {
                for entry in entries {
                    if entry
                        .get("is_dir")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    let Some(path) = entry.get("path").and_then(|value| value.as_str()) else {
                        continue;
                    };
                    if !control_view || visible_control_path(path) {
                        paths.insert(if !control_view && path == "IDENTITY.md" {
                            ACTOR_IDENTITY_ALIAS.to_string()
                        } else {
                            path.to_string()
                        });
                    }
                }
            }
        }
        Ok(paths.into_iter().collect())
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
                "scope": "conversation",
            }),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_routes_control_and_personal_files_to_distinct_scopes() {
        assert_eq!(
            memory_target_for_path("SOUL.md").unwrap(),
            ("SOUL.md", "principal_admin")
        );
        assert_eq!(
            memory_target_for_path("hooks/a.hook.json").unwrap(),
            ("hooks/a.hook.json", "principal_admin")
        );
        assert_eq!(
            memory_target_for_path("MEMORY.md").unwrap(),
            ("MEMORY.md", "conversation")
        );
        assert_eq!(
            memory_target_for_path("daily/today.md").unwrap(),
            ("daily/today.md", "conversation")
        );
        assert_eq!(
            memory_target_for_path(ACTOR_IDENTITY_ALIAS).unwrap(),
            ("IDENTITY.md", "conversation")
        );
        assert!(memory_target_for_path("actor/notes.md").is_err());
        assert!(memory_target_for_path("actors/sibling/MEMORY.md").is_err());
        assert!(memory_target_for_path("conversations/scope/MEMORY.md").is_err());
    }

    #[test]
    fn composite_admin_view_hides_internal_identity_namespaces() {
        assert!(visible_control_path("IDENTITY.md"));
        assert!(visible_control_path("skills/research/SKILL.md"));
        assert!(!visible_control_path("actors/alice/MEMORY.md"));
        assert!(!visible_control_path("conversations/abc/MEMORY.md"));
    }
}
