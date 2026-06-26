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
        let path = normalize_path(path);
        let doc = self
            .storage
            .get_or_create_document_by_path(&self.user_id, self.agent_id, &path)
            .await?;

        let new_content = if doc.content.is_empty() {
            content.to_string()
        } else {
            format!("{}\n{}", doc.content, content)
        };

        self.storage.update_document(doc.id, &new_content).await?;

        // Reindex for search — non-fatal (same reasoning as write()).
        if let Err(e) = self.reindex_document(doc.id).await {
            tracing::warn!(
                doc_id = %doc.id,
                path = %path,
                error = %e,
                "Reindex failed after append — content saved but search index may be stale"
            );
        }

        Ok(())
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
        // Use double newline for memory entries (semantic separation)
        let doc = self.memory().await?;
        let new_content = if doc.content.is_empty() {
            entry.to_string()
        } else {
            format!("{}\n\n{}", doc.content, entry)
        };
        self.storage.update_document(doc.id, &new_content).await?;
        self.reindex_document(doc.id).await?;
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
