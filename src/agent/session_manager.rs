//! Session manager compatibility facade.
//!
//! Thread/session lookup, pruning, ownership, and undo-manager policy live in
//! `thinclaw-agent`. This module keeps the root HookRegistry adapter and the
//! legacy public type used by root callers.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use thinclaw_agent::session_manager::SessionLifecycleHooks;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::agent::session::Session;
use crate::agent::undo::UndoManager;
use crate::hooks::HookRegistry;
use crate::identity::{ConversationKind, ResolvedIdentity};

/// Manages sessions, threads, and undo state for all users.
pub struct SessionManager {
    inner: thinclaw_agent::session_manager::SessionManager,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            inner: thinclaw_agent::session_manager::SessionManager::new(),
        }
    }

    pub fn with_hooks(mut self, hooks: Arc<HookRegistry>) -> Self {
        self.inner = self
            .inner
            .with_lifecycle_hooks(Arc::new(RootSessionLifecycleHooks { hooks }));
        self
    }

    pub fn scope_id_for_user_id(user_id: &str) -> Uuid {
        thinclaw_agent::session_manager::SessionManager::scope_id_for_user_id(user_id)
    }

    pub async fn workspace_lock(&self, user_id: &str) -> Arc<RwLock<()>> {
        self.inner.workspace_lock(user_id).await
    }

    pub async fn session_for_thread(&self, thread_id: Uuid) -> Option<Arc<Mutex<Session>>> {
        self.inner.session_for_thread(thread_id).await
    }

    pub async fn get_or_create_session_for_identity(
        &self,
        identity: &ResolvedIdentity,
    ) -> Arc<Mutex<Session>> {
        self.inner
            .get_or_create_session_for_identity(identity)
            .await
    }

    pub async fn get_or_create_session(&self, user_id: &str) -> Arc<Mutex<Session>> {
        self.inner.get_or_create_session(user_id).await
    }

    pub async fn resolve_thread(
        &self,
        user_id: &str,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        self.inner
            .resolve_thread(user_id, channel, external_thread_id)
            .await
    }

    pub async fn resolve_thread_for_identity(
        &self,
        identity: &ResolvedIdentity,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        self.inner
            .resolve_thread_for_identity(identity, channel, external_thread_id)
            .await
    }

    pub async fn register_thread(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        self.inner
            .register_thread(user_id, channel, thread_id, session)
            .await;
    }

    pub async fn register_direct_main_thread_for_scope(
        &self,
        scope_id: Uuid,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        self.inner
            .register_direct_main_thread_for_scope(scope_id, thread_id, session)
            .await;
    }

    pub async fn register_thread_for_scope(
        &self,
        scope_id: Uuid,
        conversation_kind: ConversationKind,
        channel: &str,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        self.inner
            .register_thread_for_scope(scope_id, conversation_kind, channel, thread_id, session)
            .await;
    }

    pub async fn get_undo_manager(&self, thread_id: Uuid) -> Arc<Mutex<UndoManager>> {
        self.inner.get_undo_manager(thread_id).await
    }

    pub async fn restore_undo_manager(&self, thread_id: Uuid, undo: UndoManager) {
        self.inner.restore_undo_manager(thread_id, undo).await;
    }

    pub async fn set_thread_owner(&self, thread_id: Uuid, owner: &str) -> bool {
        self.inner.set_thread_owner(thread_id, owner).await
    }

    pub async fn restore_thread_owner(&self, thread_id: Uuid, owner: &str) {
        self.inner.restore_thread_owner(thread_id, owner).await;
    }

    pub async fn get_thread_owner(&self, thread_id: Uuid) -> Option<String> {
        self.inner.get_thread_owner(thread_id).await
    }

    pub async fn is_thread_owned_by(&self, thread_id: Uuid, owner: &str) -> bool {
        self.inner.is_thread_owned_by(thread_id, owner).await
    }

    pub async fn prune_stale_sessions(&self, max_idle: Duration) -> usize {
        self.inner.prune_stale_sessions(max_idle).await
    }

    pub async fn list_sessions(&self) -> Vec<serde_json::Value> {
        self.inner.list_sessions().await
    }

    pub async fn describe_session(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Option<serde_json::Value> {
        self.inner.describe_session(user_id, channel).await
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

struct RootSessionLifecycleHooks {
    hooks: Arc<HookRegistry>,
}

#[async_trait]
impl SessionLifecycleHooks for RootSessionLifecycleHooks {
    async fn session_started(&self, user_id: String, session_id: String) {
        use crate::hooks::HookEvent;

        if let Err(error) = self
            .hooks
            .run(&HookEvent::SessionStart {
                user_id,
                session_id,
            })
            .await
        {
            tracing::warn!("OnSessionStart hook error: {}", error);
        }
    }

    async fn session_ended(&self, user_id: String, session_id: String) {
        use crate::hooks::HookEvent;

        if let Err(error) = self
            .hooks
            .run(&HookEvent::SessionEnd {
                user_id,
                session_id,
            })
            .await
        {
            tracing::warn!("OnSessionEnd hook error: {}", error);
        }
    }
}
