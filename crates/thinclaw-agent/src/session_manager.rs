//! Root-independent session manager for multi-user, multi-thread conversations.
//!
//! Direct sessions are principal-scoped and share the default direct thread
//! across channels. Group sessions remain isolated by conversation scope.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeDelta, Utc};
use thinclaw_identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::session::Session;
use crate::undo::UndoManager;

pub const SESSION_COUNT_WARNING_THRESHOLD: usize = 1000;
pub const DIRECT_MAIN_THREAD_KEY: &str = "__direct_main__";

/// Key for mapping external thread IDs to internal thread UUIDs.
#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct ThreadKey {
    pub scope_id: Uuid,
    pub external_thread_id: Option<String>,
}

/// Session lifecycle hook surface supplied by root adapters.
#[async_trait]
pub trait SessionLifecycleHooks: Send + Sync {
    async fn session_started(&self, user_id: String, session_id: String);
    async fn session_ended(&self, user_id: String, session_id: String);
}

/// Manages sessions, threads, undo state, and transient ownership.
pub struct SessionManager {
    sessions: RwLock<HashMap<Uuid, Arc<Mutex<Session>>>>,
    thread_map: RwLock<HashMap<ThreadKey, Uuid>>,
    undo_managers: RwLock<HashMap<Uuid, Arc<Mutex<UndoManager>>>>,
    thread_owners: RwLock<HashMap<Uuid, String>>,
    hooks: Option<Arc<dyn SessionLifecycleHooks>>,
    workspace_locks: RwLock<HashMap<String, Arc<RwLock<()>>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            thread_map: RwLock::new(HashMap::new()),
            undo_managers: RwLock::new(HashMap::new()),
            thread_owners: RwLock::new(HashMap::new()),
            hooks: None,
            workspace_locks: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_lifecycle_hooks(mut self, hooks: Arc<dyn SessionLifecycleHooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    pub fn scope_id_for_user_id(user_id: &str) -> Uuid {
        scope_id_for_user_id(user_id)
    }

    pub fn session_scope_for_identity(identity: &ResolvedIdentity) -> Uuid {
        session_scope_for_identity(identity)
    }

    pub async fn workspace_lock(&self, user_id: &str) -> Arc<RwLock<()>> {
        {
            let locks = self.workspace_locks.read().await;
            if let Some(lock) = locks.get(user_id) {
                return Arc::clone(lock);
            }
        }

        let mut locks = self.workspace_locks.write().await;
        locks
            .entry(user_id.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }

    pub async fn session_for_thread(&self, thread_id: Uuid) -> Option<Arc<Mutex<Session>>> {
        let sessions = self.sessions.read().await;
        let candidates: Vec<_> = sessions.values().cloned().collect();
        drop(sessions);

        for session in candidates {
            let guard = session.lock().await;
            let has_thread = guard.threads.contains_key(&thread_id);
            drop(guard);
            if has_thread {
                return Some(session);
            }
        }

        None
    }

    pub async fn get_or_create_session_for_identity(
        &self,
        identity: &ResolvedIdentity,
    ) -> Arc<Mutex<Session>> {
        self.get_or_create_session_scoped(
            Self::session_scope_for_identity(identity),
            identity.principal_id.as_str(),
            identity.actor_id.as_str(),
            identity.conversation_kind,
        )
        .await
    }

    pub async fn get_or_create_session(&self, user_id: &str) -> Arc<Mutex<Session>> {
        self.get_or_create_session_scoped(
            Self::scope_id_for_user_id(user_id),
            user_id,
            user_id,
            ConversationKind::Direct,
        )
        .await
    }

    async fn get_or_create_session_scoped(
        &self,
        scope_id: Uuid,
        principal_id: &str,
        actor_id: &str,
        conversation_kind: ConversationKind,
    ) -> Arc<Mutex<Session>> {
        {
            let sessions = self.sessions.read().await;
            if let Some(session) = sessions.get(&scope_id) {
                return Arc::clone(session);
            }
        }

        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get(&scope_id) {
            return Arc::clone(session);
        }

        let new_session = Session::new_scoped(principal_id, actor_id, scope_id, conversation_kind);
        let session_id = new_session.id.to_string();
        let session = Arc::new(Mutex::new(new_session));
        sessions.insert(scope_id, Arc::clone(&session));

        if sessions.len() >= SESSION_COUNT_WARNING_THRESHOLD && sessions.len() % 100 == 0 {
            tracing::warn!(
                "High session count: {} active sessions. \
                 Pruning runs every 10 minutes; consider reducing session_idle_timeout.",
                sessions.len()
            );
        }

        if let Some(hooks) = self.hooks.clone() {
            let user_id = principal_id.to_string();
            tokio::spawn(async move {
                hooks.session_started(user_id, session_id).await;
            });
        }

        session
    }

    pub async fn resolve_thread(
        &self,
        user_id: &str,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        let session = self.get_or_create_session(user_id).await;
        self.resolve_thread_with_scope(
            Self::scope_id_for_user_id(user_id),
            ConversationKind::Direct,
            session,
            channel,
            external_thread_id,
        )
        .await
    }

    pub async fn resolve_thread_for_identity(
        &self,
        identity: &ResolvedIdentity,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        let scope_id = Self::session_scope_for_identity(identity);
        let session = self.get_or_create_session_for_identity(identity).await;
        self.resolve_thread_with_scope(
            scope_id,
            identity.conversation_kind,
            session,
            channel,
            external_thread_id,
        )
        .await
    }

    async fn resolve_thread_with_scope(
        &self,
        scope_id: Uuid,
        conversation_kind: ConversationKind,
        session: Arc<Mutex<Session>>,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        let external_thread_id =
            normalize_external_thread_key(conversation_kind, channel, external_thread_id);
        let key = ThreadKey {
            scope_id,
            external_thread_id: external_thread_id.clone(),
        };

        {
            let thread_map = self.thread_map.read().await;
            if let Some(&thread_id) = thread_map.get(&key) {
                let sess = session.lock().await;
                if sess.threads.contains_key(&thread_id) {
                    return (Arc::clone(&session), thread_id);
                }
            }
        }

        if let Some(ext_tid) = external_thread_id.as_deref()
            && let Ok(ext_uuid) = Uuid::parse_str(ext_tid)
        {
            let sess = session.lock().await;
            if sess.threads.contains_key(&ext_uuid) {
                drop(sess);

                self.thread_map.write().await.insert(key, ext_uuid);
                self.ensure_undo_manager(ext_uuid).await;
                return (session, ext_uuid);
            }
        }

        let thread_id = {
            let mut sess = session.lock().await;
            sess.create_thread().id
        };

        self.thread_map.write().await.insert(key, thread_id);
        self.ensure_undo_manager(thread_id).await;

        (session, thread_id)
    }

    pub async fn register_thread(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        self.register_thread_for_scope(
            Self::scope_id_for_user_id(user_id),
            ConversationKind::Direct,
            channel,
            thread_id,
            session,
        )
        .await;
    }

    pub async fn register_direct_main_thread_for_scope(
        &self,
        scope_id: Uuid,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.insert(
                ThreadKey {
                    scope_id,
                    external_thread_id: Some(DIRECT_MAIN_THREAD_KEY.to_string()),
                },
                thread_id,
            );
            thread_map.insert(
                ThreadKey {
                    scope_id,
                    external_thread_id: Some(thread_id.to_string()),
                },
                thread_id,
            );
        }

        self.ensure_undo_manager(thread_id).await;
        self.sessions
            .write()
            .await
            .entry(scope_id)
            .or_insert(session);
    }

    pub async fn register_thread_for_scope(
        &self,
        scope_id: Uuid,
        conversation_kind: ConversationKind,
        channel: &str,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        let key = ThreadKey {
            scope_id,
            external_thread_id: normalize_external_thread_key(
                conversation_kind,
                channel,
                Some(&thread_id.to_string()),
            ),
        };

        self.thread_map.write().await.insert(key, thread_id);
        self.ensure_undo_manager(thread_id).await;
        self.sessions
            .write()
            .await
            .entry(scope_id)
            .or_insert(session);
    }

    async fn ensure_undo_manager(&self, thread_id: Uuid) -> Arc<Mutex<UndoManager>> {
        let mut managers = self.undo_managers.write().await;
        managers
            .entry(thread_id)
            .or_insert_with(|| Arc::new(Mutex::new(UndoManager::new())))
            .clone()
    }

    pub async fn get_undo_manager(&self, thread_id: Uuid) -> Arc<Mutex<UndoManager>> {
        {
            let managers = self.undo_managers.read().await;
            if let Some(manager) = managers.get(&thread_id) {
                return Arc::clone(manager);
            }
        }

        self.ensure_undo_manager(thread_id).await
    }

    pub async fn restore_undo_manager(&self, thread_id: Uuid, undo: UndoManager) {
        self.undo_managers
            .write()
            .await
            .insert(thread_id, Arc::new(Mutex::new(undo)));
    }

    pub async fn set_thread_owner(&self, thread_id: Uuid, owner: &str) -> bool {
        let mut owners = self.thread_owners.write().await;
        if owners.contains_key(&thread_id) {
            return false;
        }
        owners.insert(thread_id, owner.to_string());
        true
    }

    pub async fn restore_thread_owner(&self, thread_id: Uuid, owner: &str) {
        self.thread_owners
            .write()
            .await
            .insert(thread_id, owner.to_string());
    }

    pub async fn get_thread_owner(&self, thread_id: Uuid) -> Option<String> {
        self.thread_owners.read().await.get(&thread_id).cloned()
    }

    pub async fn is_thread_owned_by(&self, thread_id: Uuid, owner: &str) -> bool {
        self.thread_owners
            .read()
            .await
            .get(&thread_id)
            .map(|current| current == owner)
            .unwrap_or(false)
    }

    pub async fn prune_stale_sessions(&self, max_idle: Duration) -> usize {
        let cutoff = Utc::now() - TimeDelta::seconds(max_idle.as_secs() as i64);
        let stale: Vec<(Uuid, String, String, Vec<Uuid>)> = {
            let sessions = self.sessions.read().await;
            sessions
                .iter()
                .filter_map(|(scope_id, session)| {
                    let sess = session.try_lock().ok()?;
                    if sess.last_active_at < cutoff {
                        Some((
                            *scope_id,
                            sess.principal_id.clone(),
                            sess.id.to_string(),
                            sess.threads.keys().cloned().collect(),
                        ))
                    } else {
                        None
                    }
                })
                .collect()
        };

        let stale_scopes: Vec<_> = stale.iter().map(|(scope_id, _, _, _)| *scope_id).collect();
        if stale_scopes.is_empty() {
            return 0;
        }

        let stale_principals: Vec<_> = stale
            .iter()
            .map(|(_, principal, _, _)| principal.clone())
            .collect();
        let stale_sessions: Vec<_> = stale
            .iter()
            .map(|(_, principal, session_id, _)| (principal.clone(), session_id.clone()))
            .collect();
        let stale_thread_ids: Vec<_> = stale
            .into_iter()
            .flat_map(|(_, _, _, thread_ids)| thread_ids)
            .collect();

        if let Some(hooks) = self.hooks.clone() {
            for (user_id, session_id) in stale_sessions {
                let hooks = hooks.clone();
                tokio::spawn(async move {
                    hooks.session_ended(user_id, session_id).await;
                });
            }
        }

        let count = {
            let mut sessions = self.sessions.write().await;
            let before = sessions.len();
            for scope_id in &stale_scopes {
                sessions.remove(scope_id);
            }
            before - sessions.len()
        };

        self.thread_map
            .write()
            .await
            .retain(|key, _| !stale_scopes.contains(&key.scope_id));

        {
            let mut undo_managers = self.undo_managers.write().await;
            for thread_id in &stale_thread_ids {
                undo_managers.remove(thread_id);
            }
        }

        {
            let mut thread_owners = self.thread_owners.write().await;
            for thread_id in &stale_thread_ids {
                thread_owners.remove(thread_id);
            }
        }

        {
            let mut locks = self.workspace_locks.write().await;
            for user_id in &stale_principals {
                locks.remove(user_id);
            }
        }

        if count > 0 {
            tracing::info!(
                "Pruned {} stale session(s) (idle > {}s)",
                count,
                max_idle.as_secs()
            );
        }

        count
    }

    pub async fn list_sessions(&self) -> Vec<serde_json::Value> {
        let sessions = self.sessions.read().await;
        let thread_owners = self.thread_owners.read().await;
        let mut result = Vec::new();

        for (scope_id, session_arc) in sessions.iter() {
            let session = session_arc.lock().await;
            let owner = session
                .threads
                .keys()
                .find_map(|thread_id| thread_owners.get(thread_id))
                .cloned()
                .unwrap_or_else(|| "-".to_string());
            let last_active = session
                .threads
                .values()
                .find_map(|thread| thread.turns.last().map(|_| "active"))
                .unwrap_or("idle");

            result.push(serde_json::json!({
                "session_scope_id": scope_id.to_string(),
                "user_id": session.user_id.clone(),
                "principal_id": session.principal_id.clone(),
                "actor_id": session.actor_id.clone(),
                "conversation_kind": session.conversation_kind.as_str(),
                "channel": "unknown",
                "thread_count": session.threads.len(),
                "last_active": last_active,
                "owner": owner,
            }));
        }

        result
    }

    pub async fn describe_session(
        &self,
        user_id: &str,
        _channel: &str,
    ) -> Option<serde_json::Value> {
        let sessions = self.sessions.read().await;
        let scope_id = Self::scope_id_for_user_id(user_id);
        let session_arc = sessions.get(&scope_id)?;
        let session = session_arc.lock().await;
        let thread_owners = self.thread_owners.read().await;

        let threads: Vec<_> = session
            .threads
            .iter()
            .map(|(thread_id, thread)| {
                serde_json::json!({
                    "thread_id": thread_id.to_string(),
                    "owner": thread_owners
                        .get(thread_id)
                        .cloned()
                        .unwrap_or_else(|| "(unowned)".to_string()),
                    "state": format!("{:?}", thread.state),
                    "message_count": thread.turns.len(),
                })
            })
            .collect();

        Some(serde_json::json!({
            "session_scope_id": scope_id.to_string(),
            "user_id": session.user_id.clone(),
            "principal_id": session.principal_id.clone(),
            "actor_id": session.actor_id.clone(),
            "conversation_kind": session.conversation_kind.as_str(),
            "thread_count": threads.len(),
            "threads": threads,
        }))
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn scope_id_for_user_id(user_id: &str) -> Uuid {
    scope_id_from_key(&format!("principal:{user_id}"))
}

pub fn session_scope_for_identity(identity: &ResolvedIdentity) -> Uuid {
    match identity.conversation_kind {
        ConversationKind::Direct => scope_id_for_user_id(&identity.principal_id),
        ConversationKind::Group => identity.conversation_scope_id,
    }
}

pub fn normalize_external_thread_key(
    conversation_kind: ConversationKind,
    channel: &str,
    external_thread_id: Option<&str>,
) -> Option<String> {
    let raw = external_thread_id
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty());

    match conversation_kind {
        ConversationKind::Direct => Some(
            raw.map(|raw| {
                if raw.eq_ignore_ascii_case(DIRECT_MAIN_THREAD_KEY) {
                    DIRECT_MAIN_THREAD_KEY.to_string()
                } else if let Ok(uuid) = Uuid::parse_str(raw) {
                    uuid.to_string()
                } else {
                    raw.to_string()
                }
            })
            .unwrap_or_else(|| DIRECT_MAIN_THREAD_KEY.to_string()),
        ),
        ConversationKind::Group => Some(format!("{channel}:{}", raw?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Thread;

    fn identity(kind: ConversationKind) -> ResolvedIdentity {
        ResolvedIdentity {
            principal_id: "principal-1".to_string(),
            actor_id: "actor-1".to_string(),
            conversation_scope_id: Uuid::new_v4(),
            conversation_kind: kind,
            raw_sender_id: "sender-1".to_string(),
            stable_external_conversation_key: "conversation-1".to_string(),
        }
    }

    #[test]
    fn direct_scope_uses_principal_id() {
        let identity = identity(ConversationKind::Direct);
        assert_eq!(
            session_scope_for_identity(&identity),
            scope_id_for_user_id("principal-1")
        );
    }

    #[test]
    fn group_scope_uses_conversation_scope() {
        let identity = identity(ConversationKind::Group);
        assert_eq!(
            session_scope_for_identity(&identity),
            identity.conversation_scope_id
        );
    }

    #[test]
    fn direct_thread_key_defaults_to_main_and_normalizes_uuid() {
        assert_eq!(
            normalize_external_thread_key(ConversationKind::Direct, "web", None).as_deref(),
            Some(DIRECT_MAIN_THREAD_KEY)
        );

        let id = Uuid::new_v4();
        assert_eq!(
            normalize_external_thread_key(
                ConversationKind::Direct,
                "web",
                Some(&id.hyphenated().to_string().to_uppercase())
            )
            .as_deref(),
            Some(id.to_string().as_str())
        );
    }

    #[test]
    fn group_thread_key_requires_external_thread() {
        assert_eq!(
            normalize_external_thread_key(ConversationKind::Group, "discord", Some("thread-1"))
                .as_deref(),
            Some("discord:thread-1")
        );
        assert!(normalize_external_thread_key(ConversationKind::Group, "discord", None).is_none());
    }

    #[tokio::test]
    async fn get_or_create_session_is_scoped_by_user() {
        let manager = SessionManager::new();
        let session1 = manager.get_or_create_session("user-1").await;
        let session2 = manager.get_or_create_session("user-1").await;
        let session3 = manager.get_or_create_session("user-2").await;

        assert!(Arc::ptr_eq(&session1, &session2));
        assert!(!Arc::ptr_eq(&session1, &session3));
    }

    #[tokio::test]
    async fn direct_default_thread_is_shared_across_channels() {
        let manager = SessionManager::new();
        let (session1, thread1) = manager.resolve_thread("user-1", "cli", None).await;
        let (session2, thread2) = manager.resolve_thread("user-1", "http", None).await;

        assert!(Arc::ptr_eq(&session1, &session2));
        assert_eq!(thread1, thread2);
    }

    #[tokio::test]
    async fn explicit_direct_aliases_are_channel_agnostic() {
        let manager = SessionManager::new();
        let (_, thread1) = manager
            .resolve_thread("user-1", "gateway", Some("thread-x"))
            .await;
        let (_, thread2) = manager
            .resolve_thread("user-1", "telegram", Some("thread-x"))
            .await;

        assert_eq!(thread1, thread2);
    }

    #[tokio::test]
    async fn stale_thread_mapping_creates_new_thread() {
        let manager = SessionManager::new();
        let (session, original) = manager
            .resolve_thread("user-1", "gateway", Some("ext-1"))
            .await;

        session.lock().await.threads.remove(&original);

        let (_, replacement) = manager
            .resolve_thread("user-1", "gateway", Some("ext-1"))
            .await;

        assert_ne!(original, replacement);
        assert!(session.lock().await.threads.contains_key(&replacement));
    }

    #[tokio::test]
    async fn register_thread_preserves_uuid_on_resolve() {
        let manager = SessionManager::new();
        let thread_id = Uuid::new_v4();
        let session = Arc::new(Mutex::new(Session::new("user-web")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(thread_id, sess.id);
            sess.threads.insert(thread_id, thread);
        }

        manager
            .register_thread("user-web", "gateway", thread_id, Arc::clone(&session))
            .await;

        let (_, resolved) = manager
            .resolve_thread("user-web", "telegram", Some(&thread_id.to_string()))
            .await;
        assert_eq!(resolved, thread_id);
    }

    #[tokio::test]
    async fn register_direct_main_thread_alias_reuses_default_and_uuid() {
        let manager = SessionManager::new();
        let thread_id = Uuid::new_v4();
        let scope_id = SessionManager::scope_id_for_user_id("user-alias");
        let session = Arc::new(Mutex::new(Session::new("user-alias")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(thread_id, sess.id);
            sess.threads.insert(thread_id, thread);
        }

        manager
            .register_direct_main_thread_for_scope(scope_id, thread_id, Arc::clone(&session))
            .await;

        let identity = ResolvedIdentity {
            principal_id: "user-alias".to_string(),
            actor_id: "user-alias".to_string(),
            conversation_scope_id: scope_id_from_key("telegram://direct/user-alias"),
            conversation_kind: ConversationKind::Direct,
            raw_sender_id: "user-alias".to_string(),
            stable_external_conversation_key: "telegram://direct/user-alias".to_string(),
        };

        let (_, default_thread) = manager
            .resolve_thread_for_identity(&identity, "telegram", None)
            .await;
        let (_, uuid_thread) = manager
            .resolve_thread_for_identity(&identity, "gateway", Some(&thread_id.to_string()))
            .await;

        assert_eq!(default_thread, thread_id);
        assert_eq!(uuid_thread, thread_id);
    }

    #[tokio::test]
    async fn group_scopes_stay_isolated() {
        let manager = SessionManager::new();
        let identity_signal = ResolvedIdentity {
            principal_id: "user-group".to_string(),
            actor_id: "user-group".to_string(),
            conversation_scope_id: scope_id_from_key("signal:group:grp-1"),
            conversation_kind: ConversationKind::Group,
            raw_sender_id: "user-group".to_string(),
            stable_external_conversation_key: "signal:group:grp-1".to_string(),
        };
        let identity_telegram = ResolvedIdentity {
            principal_id: "user-group".to_string(),
            actor_id: "user-group".to_string(),
            conversation_scope_id: scope_id_from_key("telegram:group:grp-1"),
            conversation_kind: ConversationKind::Group,
            raw_sender_id: "user-group".to_string(),
            stable_external_conversation_key: "telegram:group:grp-1".to_string(),
        };

        let (_, thread1) = manager
            .resolve_thread_for_identity(&identity_signal, "signal", Some("grp-1"))
            .await;
        let (_, thread2) = manager
            .resolve_thread_for_identity(&identity_telegram, "telegram", Some("grp-1"))
            .await;

        assert_ne!(thread1, thread2);
    }

    #[tokio::test]
    async fn prune_removes_sessions_maps_and_locks() {
        let manager = SessionManager::new();
        let (session, thread_id) = manager.resolve_thread("user-stale", "cli", None).await;
        session.lock().await.last_active_at = Utc::now() - TimeDelta::days(30);
        manager.workspace_lock("user-stale").await;

        let pruned = manager
            .prune_stale_sessions(Duration::from_secs(7 * 86400))
            .await;

        assert_eq!(pruned, 1);
        assert!(manager.thread_map.read().await.is_empty());
        assert!(!manager.undo_managers.read().await.contains_key(&thread_id));
        assert!(
            !manager
                .workspace_locks
                .read()
                .await
                .contains_key("user-stale")
        );
    }
}
