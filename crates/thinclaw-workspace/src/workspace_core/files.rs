//! File operations and memory convenience methods on [`Workspace`].
//!
//! The DB-backed filesystem API (read/write/append/exists/delete/list) plus
//! convenience accessors for the curated memory documents (MEMORY.md, daily
//! logs, HEARTBEAT.md) and the memory/daily-log append helpers.

use chrono::NaiveDate;

use thinclaw_types::error::WorkspaceError;

use super::Workspace;
use super::seed::HEARTBEAT_SEED;
use super::{normalize_directory, normalize_path};
use crate::document::{MemoryDocument, WorkspaceEntry, paths};
use crate::is_control_plane_path;

impl Workspace {
    // ==================== File Operations ====================

    /// Read a file by path.
    ///
    /// Returns the document if it exists, or an error if not found.
    ///
    /// # Example
    /// ```ignore
    /// let doc = workspace.read("context/vision.md").await?;
    /// println!("{}", doc.content);
    /// ```
    pub async fn read(&self, path: &str) -> Result<MemoryDocument, WorkspaceError> {
        let path = normalize_path(path);
        self.storage
            .get_document_by_path(&self.user_id, self.agent_id, &path)
            .await
    }

    /// Write (create or update) a file.
    ///
    /// Creates parent directories implicitly (they're virtual in the DB).
    /// Re-indexes the document for search after writing.
    ///
    /// Reindex failures (e.g. missing vector extension, temporary DB lock) are
    /// logged as warnings but do NOT fail the write — content is always durably
    /// persisted even when the search index cannot be updated.
    ///
    /// # Example
    /// ```ignore
    /// workspace.write("projects/alpha/README.md", "# Project Alpha\n\nDescription here.").await?;
    /// ```
    pub async fn write(&self, path: &str, content: &str) -> Result<MemoryDocument, WorkspaceError> {
        let path = normalize_path(path);
        let doc = self
            .storage
            .get_or_create_document_by_path(&self.user_id, self.agent_id, &path)
            .await?;
        self.storage.update_document(doc.id, content).await?;

        // Reindex for search — non-fatal: a vector/FTS index failure must not
        // prevent a successful save (content is already durably written above).
        if let Err(e) = self.reindex_document(doc.id).await {
            tracing::warn!(
                doc_id = %doc.id,
                path = %path,
                error = %e,
                "Reindex failed after write — content saved but search index may be stale"
            );
        }

        // Return updated doc
        self.storage.get_document_by_id(doc.id).await
    }

    /// Append content to a file.
    ///
    /// Creates the file if it doesn't exist.
    /// Adds a newline separator between existing and new content.
    ///
    /// Reindex failures are logged as warnings but do NOT fail the append.
    pub async fn append(&self, path: &str, content: &str) -> Result<(), WorkspaceError> {
        self.append_with_separator(path, content, "\n").await?;
        Ok(())
    }

    /// Atomically append using an explicit separator. This is used for
    /// semantic memory entries (`\n\n`) as well as line-oriented logs (`\n`).
    pub async fn append_with_separator(
        &self,
        path: &str,
        content: &str,
        separator: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        let path = normalize_path(path);
        let doc = self
            .storage
            .append_document_by_path(&self.user_id, self.agent_id, &path, separator, content)
            .await?;

        // Reindex for search — non-fatal (same reasoning as write()).
        if let Err(e) = self.reindex_document(doc.id).await {
            tracing::warn!(
                doc_id = %doc.id,
                path = %path,
                error = %e,
                "Reindex failed after append — content saved but search index may be stale"
            );
        }

        self.storage.get_document_by_id(doc.id).await
    }

    /// Check if a file exists.
    pub async fn exists(&self, path: &str) -> Result<bool, WorkspaceError> {
        let path = normalize_path(path);
        match self
            .storage
            .get_document_by_path(&self.user_id, self.agent_id, &path)
            .await
        {
            Ok(_) => Ok(true),
            Err(WorkspaceError::DocumentNotFound { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Delete a file.
    ///
    /// Also deletes associated chunks.
    pub async fn delete(&self, path: &str) -> Result<(), WorkspaceError> {
        let path = normalize_path(path);
        self.storage
            .delete_document_by_path(&self.user_id, self.agent_id, &path)
            .await
    }

    /// List files and directories in a path.
    ///
    /// Returns immediate children (not recursive).
    /// Use empty string or "/" for root directory.
    ///
    /// # Example
    /// ```ignore
    /// let entries = workspace.list("projects/").await?;
    /// for entry in entries {
    ///     if entry.is_directory {
    ///         println!("📁 {}/", entry.name());
    ///     } else {
    ///         println!("📄 {}", entry.name());
    ///     }
    /// }
    /// ```
    pub async fn list(&self, directory: &str) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let directory = normalize_directory(directory);
        self.storage
            .list_directory(&self.user_id, self.agent_id, &directory)
            .await
    }

    /// List all files recursively (flat list of all paths).
    pub async fn list_all(&self) -> Result<Vec<String>, WorkspaceError> {
        self.storage
            .list_all_paths(&self.user_id, self.agent_id)
            .await
    }

    /// Copy legacy principal-root knowledge into the owner's actor-private
    /// namespace exactly once. The migration is resumable and race-safe: each
    /// destination is populated only through a content compare-and-swap, and
    /// the marker is written only after every path has been considered.
    ///
    /// This is deliberately restricted to `actor_id == principal_id`; linked
    /// household actors must never inherit the principal owner's private root.
    pub async fn migrate_legacy_owner_knowledge(
        &self,
        actor_id: &str,
    ) -> Result<usize, WorkspaceError> {
        if actor_id.trim().is_empty() || actor_id != self.user_id.as_str() {
            return Ok(0);
        }

        let actor_root = paths::actor_root(actor_id);
        let marker = format!("{actor_root}/.thinclaw/legacy-root-v1");
        let cache_key = format!("owner:{}:{:?}:{}", self.user_id, self.agent_id, actor_root);
        if self
            .migration_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&cache_key)
        {
            return Ok(0);
        }
        if self.exists(&marker).await? {
            self.migration_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(cache_key);
            return Ok(0);
        }

        let mut migrated = 0;
        for source_path in self.list_all().await? {
            if source_path == paths::SHARED_DIR
                || source_path.starts_with(&format!("{}/", paths::SHARED_DIR))
                || is_control_plane_path(&source_path)
            {
                continue;
            }

            let source = match self.read(&source_path).await {
                Ok(document) if !document.content.is_empty() => document,
                Ok(_) | Err(WorkspaceError::DocumentNotFound { .. }) => continue,
                Err(error) => return Err(error),
            };
            let destination = format!("{actor_root}/{source_path}");
            let document = self
                .storage
                .get_or_create_document_by_path(&self.user_id, self.agent_id, &destination)
                .await?;
            if document.content.is_empty()
                && self
                    .storage
                    .update_document_if_current(document.id, "", &source.content)
                    .await?
            {
                migrated += 1;
                if let Err(error) = self.reindex_document(document.id).await {
                    tracing::warn!(
                        path = %destination,
                        error = %error,
                        "Legacy owner knowledge migrated but indexing remains dirty"
                    );
                }
            }
        }

        // An empty marker is sufficient and intentionally produces no search
        // chunks. Its hidden namespace is never projected to conversation UIs.
        self.write(&marker, "").await?;
        self.migration_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(cache_key);
        Ok(migrated)
    }

    /// Copy every missing document from a legacy principal scope into this
    /// workspace. This is used by the desktop's `default` → `local_user`
    /// compatibility migration before channels start accepting work.
    pub async fn migrate_missing_principal_scope(
        &self,
        legacy_principal_id: &str,
    ) -> Result<usize, WorkspaceError> {
        let legacy = legacy_principal_id.trim();
        if legacy.is_empty() || legacy == self.user_id.as_str() {
            return Ok(0);
        }

        let marker_component = legacy
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let marker = format!(".thinclaw/migrations/principal-{marker_component}-v1");
        let cache_key = format!(
            "principal:{}:{:?}:{}",
            self.user_id, self.agent_id, marker_component
        );
        if self
            .migration_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&cache_key)
        {
            return Ok(0);
        }
        if self.exists(&marker).await? {
            self.migration_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(cache_key);
            return Ok(0);
        }

        let source = self.scoped_clone(legacy.to_string(), self.agent_id);
        let mut migrated = 0;
        for path in source.list_all().await? {
            let source_document = match source.read(&path).await {
                Ok(document) => document,
                Err(WorkspaceError::DocumentNotFound { .. }) => continue,
                Err(error) => return Err(error),
            };
            let destination = self
                .storage
                .get_or_create_document_by_path(&self.user_id, self.agent_id, &path)
                .await?;
            if destination.content.is_empty()
                && self
                    .storage
                    .update_document_if_current(destination.id, "", &source_document.content)
                    .await?
            {
                migrated += 1;
                if let Err(error) = self.reindex_document(destination.id).await {
                    tracing::warn!(
                        path = %path,
                        error = %error,
                        "Legacy principal document migrated but indexing remains dirty"
                    );
                }
            }
        }
        self.write(&marker, "").await?;
        self.migration_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(cache_key);
        Ok(migrated)
    }

    // ==================== Convenience Methods ====================

    /// Get the main MEMORY.md document (long-term curated memory).
    ///
    /// Creates it if it doesn't exist.
    pub async fn memory(&self) -> Result<MemoryDocument, WorkspaceError> {
        self.read_or_create(paths::MEMORY).await
    }

    /// Get today's daily log.
    ///
    /// Daily logs are append-only and keyed by date.
    pub async fn today_log(&self) -> Result<MemoryDocument, WorkspaceError> {
        let today = self.local_today();
        self.daily_log(today).await
    }

    /// Get a daily log for a specific date.
    pub async fn daily_log(&self, date: NaiveDate) -> Result<MemoryDocument, WorkspaceError> {
        let path = format!("daily/{}.md", date.format("%Y-%m-%d"));
        self.read_or_create(&path).await
    }

    /// Get the heartbeat checklist (HEARTBEAT.md).
    ///
    /// Returns the DB-stored checklist if it exists, otherwise falls back
    /// to the in-memory seed template. The seed is never written to the
    /// database; the user creates the real file via `memory_write` when
    /// they actually want periodic checks. The seed content is all HTML
    /// comments, which the heartbeat runner treats as "effectively empty"
    /// and skips the LLM call.
    pub async fn heartbeat_checklist(&self) -> Result<Option<String>, WorkspaceError> {
        match self.read(paths::HEARTBEAT).await {
            Ok(doc) => Ok(Some(doc.content)),
            Err(WorkspaceError::DocumentNotFound { .. }) => Ok(Some(HEARTBEAT_SEED.to_string())),
            Err(e) => Err(e),
        }
    }

    /// Helper to read or create a file.
    async fn read_or_create(&self, path: &str) -> Result<MemoryDocument, WorkspaceError> {
        self.storage
            .get_or_create_document_by_path(&self.user_id, self.agent_id, path)
            .await
    }

    // ==================== Memory Operations ====================

    /// Append an entry to the main MEMORY.md document.
    ///
    /// This is for important facts, decisions, and preferences worth
    /// remembering long-term.
    pub async fn append_memory(&self, entry: &str) -> Result<(), WorkspaceError> {
        self.append_with_separator(paths::MEMORY, entry, "\n\n")
            .await?;
        Ok(())
    }

    /// Append an entry to today's daily log.
    ///
    /// Daily logs are raw, append-only notes for the current day.
    pub async fn append_daily_log(&self, entry: &str) -> Result<(), WorkspaceError> {
        let now = self.local_now();
        let today = now.date_naive();
        let path = format!("daily/{}.md", today.format("%Y-%m-%d"));
        let timestamp = now.format("%H:%M:%S");
        let timestamped_entry = format!("[{}] {}", timestamp, entry);
        self.append(&path, &timestamped_entry).await
    }
}
