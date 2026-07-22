//! The core [`Workspace`] type providing the database-backed memory API.
//!
//! This façade owns the [`Workspace`] struct, its constructors, scope/accessor
//! methods, and the path-normalization helpers. The behavioral surface is split
//! across focused submodules, each contributing an `impl Workspace` block:
//!
//! - [`files`]: DB-backed file ops (read/write/append/exists/delete/list) and
//!   memory convenience accessors (MEMORY.md, daily logs, HEARTBEAT.md).
//! - [`prompt`]: trusted system prompt assembly plus separately typed
//!   actor/group evidence and a scope-aware context manifest.
//! - [`search`]: hybrid search, document re-indexing, and embedding backfill.
//! - [`seed`]: workspace seeding (default file templates + `seed_if_empty`).
//! - [`timezone`]: timezone <-> `USER.md` synchronization.
//!
//! Supporting helpers (no `Workspace` dependency) live in [`prompt_text`],
//! [`profile`], [`redaction`], and [`soul`].

use std::sync::Arc;

use chrono::NaiveDate;
#[cfg(feature = "postgres")]
use deadpool_postgres::Pool;
use uuid::Uuid;

use crate::document::paths;
use crate::embeddings::EmbeddingProvider;
#[cfg(feature = "postgres")]
use crate::repository::Repository;
use crate::{WorkspaceBackend, WorkspaceStore};

mod files;
mod profile;
mod prompt;
mod prompt_text;
mod redaction;
mod search;
mod seed;
mod soul;
mod timezone;

#[cfg(test)]
mod tests;

/// Workspace provides database-backed memory storage for an agent.
///
/// Each workspace is scoped to a user (and optionally an agent).
/// Documents are persisted to the database and indexed for search.
/// Supports both PostgreSQL (via Repository) and libSQL (via Database trait).
#[derive(Clone)]
pub struct Workspace {
    /// User identifier (from channel).
    pub(super) user_id: String,
    /// Optional agent ID for multi-agent isolation.
    pub(super) agent_id: Option<Uuid>,
    /// Database storage backend.
    pub(super) storage: WorkspaceBackend,
    /// Embedding provider for semantic search.
    pub(super) embeddings: Option<Arc<dyn EmbeddingProvider>>,
    /// Process-local fast path for completed compatibility migrations. The DB
    /// marker remains authoritative across restarts/processes.
    pub(super) migration_cache: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

impl Workspace {
    /// Create a new workspace backed by a PostgreSQL connection pool.
    #[cfg(feature = "postgres")]
    pub fn new(user_id: impl Into<String>, pool: Pool) -> Self {
        let store: WorkspaceBackend = Arc::new(Repository::new(pool));
        Self::new_with_store(user_id, store)
    }

    /// Create a new workspace backed by any workspace store implementation.
    pub fn new_with_store(user_id: impl Into<String>, store: WorkspaceBackend) -> Self {
        Self {
            user_id: user_id.into(),
            agent_id: None,
            storage: store,
            embeddings: None,
            migration_cache: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    /// Create a new workspace backed by any Database implementation.
    ///
    /// Use this for libSQL or any other backend that implements the Database trait.
    pub fn new_with_db<T>(user_id: impl Into<String>, db: Arc<T>) -> Self
    where
        T: WorkspaceStore + 'static + ?Sized,
    {
        Self::new_with_store(user_id, Arc::new(db))
    }

    /// Create a workspace with a specific agent ID.
    pub fn with_agent(mut self, agent_id: Uuid) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    /// Set the embedding provider for semantic search.
    pub fn with_embeddings(mut self, provider: Arc<dyn EmbeddingProvider>) -> Self {
        self.embeddings = Some(provider);
        self
    }

    /// Get the user ID.
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// Get the agent ID.
    pub fn agent_id(&self) -> Option<Uuid> {
        self.agent_id
    }

    /// Resolve the workspace's effective timezone.
    pub fn effective_timezone(&self) -> chrono_tz::Tz {
        thinclaw_platform::timezone::resolve_effective_timezone(Some(&self.user_id), None)
    }

    /// Get today's local date for this workspace.
    pub fn local_today(&self) -> NaiveDate {
        thinclaw_platform::timezone::today_for_user(Some(&self.user_id), None)
    }

    /// Get the current local time for this workspace.
    pub fn local_now(&self) -> chrono::DateTime<chrono_tz::Tz> {
        thinclaw_platform::timezone::now_for_user(Some(&self.user_id), None)
    }

    /// Clone this workspace's backend/embeddings while changing the scope.
    pub fn scoped_clone(&self, user_id: impl Into<String>, agent_id: Option<Uuid>) -> Self {
        Self {
            user_id: user_id.into(),
            agent_id,
            storage: self.storage.clone(),
            embeddings: self.embeddings.clone(),
            migration_cache: Arc::clone(&self.migration_cache),
        }
    }

    /// Resolve the path to an actor-private file.
    pub fn actor_path(actor_id: &str, file: &str) -> String {
        format!(
            "{}/{}",
            paths::actor_root(actor_id),
            file.trim_start_matches('/')
        )
    }

    /// Get the actor-private USER.md path.
    pub fn actor_user_path(actor_id: &str) -> String {
        paths::actor_user(actor_id)
    }

    /// Get the actor-private MEMORY.md path.
    pub fn actor_memory_path(actor_id: &str) -> String {
        paths::actor_memory(actor_id)
    }

    /// Get the actor-private profile path.
    pub fn actor_profile_path(actor_id: &str) -> String {
        paths::actor_profile(actor_id)
    }
}

/// Normalize a file path (remove leading/trailing slashes, collapse //).
pub(super) fn normalize_path(path: &str) -> String {
    let path = path.trim().trim_matches('/');
    // Collapse multiple slashes
    let mut result = String::new();
    let mut last_was_slash = false;
    for c in path.chars() {
        if c == '/' {
            if !last_was_slash {
                result.push(c);
            }
            last_was_slash = true;
        } else {
            result.push(c);
            last_was_slash = false;
        }
    }
    result
}

/// Normalize a directory path (ensure no trailing slash for consistency).
pub(super) fn normalize_directory(path: &str) -> String {
    let path = normalize_path(path);
    path.trim_end_matches('/').to_string()
}
