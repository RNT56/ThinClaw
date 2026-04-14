//! Session manager for multi-user, multi-thread conversation handling.
//!
//! Maps external thread aliases to internal UUIDs and manages undo state for
//! each thread. Direct sessions are principal-scoped (cross-channel), while
//! group sessions remain scope-isolated.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::agent::session::Session;
use crate::agent::undo::UndoManager;
use crate::hooks::HookRegistry;
use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};

/// Warn when session count exceeds this threshold.
const SESSION_COUNT_WARNING_THRESHOLD: usize = 1000;

/// Key for mapping external thread IDs to internal ones.
#[derive(Clone, Hash, Eq, PartialEq)]
struct ThreadKey {
    scope_id: Uuid,
    external_thread_id: Option<String>,
}

fn normalize_external_thread_key(
    conversation_kind: ConversationKind,
    channel: &str,
    external_thread_id: Option<&str>,
) -> Option<String> {
    let raw = external_thread_id
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())?;

    match conversation_kind {
        ConversationKind::Direct => {
            if let Ok(uuid) = Uuid::parse_str(raw) {
                Some(uuid.to_string())
            } else {
                Some(raw.to_string())
            }
        }
        ConversationKind::Group => Some(format!("{channel}:{raw}")),
    }
}

/// Manages sessions, threads, and undo state for all users.
pub struct SessionManager {
    sessions: RwLock<HashMap<Uuid, Arc<Mutex<Session>>>>,
    thread_map: RwLock<HashMap<ThreadKey, Uuid>>,
    undo_managers: RwLock<HashMap<Uuid, Arc<Mutex<UndoManager>>>>,
    /// Thread ownership: maps thread UUID → owner agent/channel name.
    thread_owners: RwLock<HashMap<Uuid, String>>,
    hooks: Option<Arc<HookRegistry>>,
    /// IC-002: Per-user workspace write lock — prevents concurrent MEMORY.md / daily log writes.
    workspace_locks: RwLock<HashMap<String, Arc<tokio::sync::RwLock<()>>>>,
}

impl SessionManager {
    fn session_scope_for_identity(identity: &ResolvedIdentity) -> Uuid {
        match identity.conversation_kind {
            ConversationKind::Direct => Self::scope_id_for_user_id(&identity.principal_id),
            ConversationKind::Group => identity.conversation_scope_id,
        }
    }

    /// Create a new session manager.
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

    /// Attach a hook registry for session lifecycle events.
    pub fn with_hooks(mut self, hooks: Arc<HookRegistry>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// IC-002: Get or create a per-user workspace write lock.
    ///
    /// The dispatcher should acquire `.write()` before workspace-mutating
    /// operations (e.g., MEMORY.md, daily log) to prevent concurrent
    /// writes from multiple channels for the same user.
    pub async fn workspace_lock(&self, user_id: &str) -> Arc<tokio::sync::RwLock<()>> {
        {
            let locks = self.workspace_locks.read().await;
            if let Some(lock) = locks.get(user_id) {
                return Arc::clone(lock);
            }
        }
        let mut locks = self.workspace_locks.write().await;
        locks
            .entry(user_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::RwLock::new(())))
            .clone()
    }

    /// Find the in-memory session that owns a thread ID.
    pub async fn session_for_thread(&self, thread_id: Uuid) -> Option<Arc<Mutex<Session>>> {
        let sessions = self.sessions.read().await;
        let candidates: Vec<Arc<Mutex<Session>>> = sessions.values().cloned().collect();
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

    /// Resolve the stable session scope for a principal-only legacy user ID.
    pub fn scope_id_for_user_id(user_id: &str) -> Uuid {
        scope_id_from_key(&format!("principal:{user_id}"))
    }

    /// Resolve or create a session for a full ingress identity.
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

    /// Get or create a session for a user.
    pub async fn get_or_create_session(&self, user_id: &str) -> Arc<Mutex<Session>> {
        let scope_id = Self::scope_id_for_user_id(user_id);
        self.get_or_create_session_scoped(scope_id, user_id, user_id, ConversationKind::Direct)
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

        if let Some(ref hooks) = self.hooks {
            let hooks = hooks.clone();
            let uid = principal_id.to_string();
            let sid = session_id;
            tokio::spawn(async move {
                use crate::hooks::HookEvent;
                let event = HookEvent::SessionStart {
                    user_id: uid,
                    session_id: sid,
                };
                if let Err(e) = hooks.run(&event).await {
                    tracing::warn!("OnSessionStart hook error: {}", e);
                }
            });
        }

        session
    }

    /// Resolve an external thread ID to an internal thread.
    ///
    /// Returns the session and thread ID. Creates both if they don't exist.
    pub async fn resolve_thread(
        &self,
        user_id: &str,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        let session = self.get_or_create_session(user_id).await;
        let scope_id = Self::scope_id_for_user_id(user_id);
        self.resolve_thread_with_scope(
            scope_id,
            ConversationKind::Direct,
            session,
            channel,
            external_thread_id,
        )
        .await
    }

    /// Resolve a thread using a resolved ingress identity.
    pub async fn resolve_thread_for_identity(
        &self,
        identity: &ResolvedIdentity,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        let scope_id = Self::session_scope_for_identity(identity);
        let session = self
            .get_or_create_session_scoped(
                scope_id,
                identity.principal_id.as_str(),
                identity.actor_id.as_str(),
                identity.conversation_kind,
            )
            .await;
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

        // Check if we have a mapping
        {
            let thread_map = self.thread_map.read().await;
            if let Some(&thread_id) = thread_map.get(&key) {
                // Verify thread still exists in session
                let sess = session.lock().await;
                if sess.threads.contains_key(&thread_id) {
                    return (Arc::clone(&session), thread_id);
                }
            }
        }

        // Check if external_thread_id is itself a known thread UUID that
        // exists in the session but was never registered in the thread_map
        // (e.g. created by chat_new_thread_handler or hydrated from DB).
        if let Some(ext_tid) = external_thread_id.as_deref()
            && let Ok(ext_uuid) = Uuid::parse_str(ext_tid)
        {
            let sess = session.lock().await;
            if sess.threads.contains_key(&ext_uuid) {
                drop(sess);

                let mut thread_map = self.thread_map.write().await;
                thread_map.insert(key, ext_uuid);
                drop(thread_map);

                // Ensure undo manager exists
                let mut undo_managers = self.undo_managers.write().await;
                undo_managers
                    .entry(ext_uuid)
                    .or_insert_with(|| Arc::new(Mutex::new(UndoManager::new())));
                return (session, ext_uuid);
            }
        }

        // Create new thread (always create a new one for a new key)
        let thread_id = {
            let mut sess = session.lock().await;
            let thread = sess.create_thread();
            thread.id
        };

        // Store mapping
        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.insert(key, thread_id);
        }

        // Create undo manager for thread
        {
            let mut undo_managers = self.undo_managers.write().await;
            undo_managers.insert(thread_id, Arc::new(Mutex::new(UndoManager::new())));
        }

        (session, thread_id)
    }

    /// Register a hydrated thread so subsequent `resolve_thread` calls find it.
    ///
    /// Inserts into the thread_map and creates an undo manager for the thread.
    pub async fn register_thread(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        let scope_id = Self::scope_id_for_user_id(user_id);
        self.register_thread_for_scope(
            scope_id,
            ConversationKind::Direct,
            channel,
            thread_id,
            session,
        )
        .await;
    }

    /// Register a hydrated thread for a specific conversation scope.
    pub async fn register_thread_for_scope(
        &self,
        scope_id: Uuid,
        conversation_kind: ConversationKind,
        channel: &str,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        let external_thread_id =
            normalize_external_thread_key(conversation_kind, channel, Some(&thread_id.to_string()));
        let key = ThreadKey {
            scope_id,
            external_thread_id,
        };

        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.insert(key, thread_id);
        }

        {
            let mut undo_managers = self.undo_managers.write().await;
            undo_managers
                .entry(thread_id)
                .or_insert_with(|| Arc::new(Mutex::new(UndoManager::new())));
        }

        // Ensure the session is tracked
        {
            let mut sessions = self.sessions.write().await;
            sessions.entry(scope_id).or_insert(session);
        }
    }

    /// Get undo manager for a thread.
    pub async fn get_undo_manager(&self, thread_id: Uuid) -> Arc<Mutex<UndoManager>> {
        // Fast path
        {
            let managers = self.undo_managers.read().await;
            if let Some(mgr) = managers.get(&thread_id) {
                return Arc::clone(mgr);
            }
        }

        // Create if missing
        let mut managers = self.undo_managers.write().await;
        // Double-check
        if let Some(mgr) = managers.get(&thread_id) {
            return Arc::clone(mgr);
        }

        let mgr = Arc::new(Mutex::new(UndoManager::new()));
        managers.insert(thread_id, Arc::clone(&mgr));
        mgr
    }

    /// Set the owner of a thread. Returns `true` if ownership was set,
    /// `false` if the thread was already owned (first-responder wins).
    pub async fn set_thread_owner(&self, thread_id: Uuid, owner: &str) -> bool {
        let mut owners = self.thread_owners.write().await;
        if owners.contains_key(&thread_id) {
            return false; // Already owned
        }
        owners.insert(thread_id, owner.to_string());
        true
    }

    /// Restore thread ownership from persisted state, replacing any stale in-memory value.
    pub async fn restore_thread_owner(&self, thread_id: Uuid, owner: &str) {
        let mut owners = self.thread_owners.write().await;
        owners.insert(thread_id, owner.to_string());
    }

    /// Get the owner of a thread, if any.
    pub async fn get_thread_owner(&self, thread_id: Uuid) -> Option<String> {
        let owners = self.thread_owners.read().await;
        owners.get(&thread_id).cloned()
    }

    /// Restore an undo manager snapshot for a hydrated thread.
    pub async fn restore_undo_manager(&self, thread_id: Uuid, undo: UndoManager) {
        let mut managers = self.undo_managers.write().await;
        managers.insert(thread_id, Arc::new(Mutex::new(undo)));
    }

    /// Check if a thread is owned by a specific agent.
    pub async fn is_thread_owned_by(&self, thread_id: Uuid, owner: &str) -> bool {
        let owners = self.thread_owners.read().await;
        owners.get(&thread_id).map(|o| o == owner).unwrap_or(false)
    }

    /// Remove sessions that have been idle for longer than the given duration.
    ///
    /// Returns the number of sessions pruned.
    pub async fn prune_stale_sessions(&self, max_idle: std::time::Duration) -> usize {
        let cutoff = chrono::Utc::now() - chrono::TimeDelta::seconds(max_idle.as_secs() as i64);

        // Collect stale sessions (scope_id, principal_id, session_id, thread_ids) in one pass
        // to avoid TOCTOU between finding stale sessions and reading their threads (Bug 25).
        let stale: Vec<(Uuid, String, String, Vec<Uuid>)> = {
            let sessions = self.sessions.read().await;
            sessions
                .iter()
                .filter_map(|(scope_id, session)| {
                    // Try to lock; skip if contended (someone is actively using it)
                    let sess = session.try_lock().ok()?;
                    if sess.last_active_at < cutoff {
                        let thread_ids: Vec<Uuid> = sess.threads.keys().cloned().collect();
                        Some((
                            *scope_id,
                            sess.principal_id.clone(),
                            sess.id.to_string(),
                            thread_ids,
                        ))
                    } else {
                        None
                    }
                })
                .collect()
        };

        let stale_scopes: Vec<Uuid> = stale.iter().map(|(scope_id, _, _, _)| *scope_id).collect();
        let stale_principals: Vec<String> = stale
            .iter()
            .map(|(_, principal, _, _)| principal.clone())
            .collect();
        let stale_sessions: Vec<(String, String)> = stale
            .iter()
            .map(|(_, principal, sid, _)| (principal.clone(), sid.clone()))
            .collect();
        let stale_thread_ids: Vec<Uuid> =
            stale.into_iter().flat_map(|(_, _, _, tids)| tids).collect();

        if stale_scopes.is_empty() {
            return 0;
        }

        // Fire OnSessionEnd hooks for stale sessions (fire-and-forget)
        if let Some(ref hooks) = self.hooks {
            for (user_id, session_id) in &stale_sessions {
                let hooks = hooks.clone();
                let uid = user_id.clone();
                let sid = session_id.clone();
                tokio::spawn(async move {
                    use crate::hooks::HookEvent;
                    let event = HookEvent::SessionEnd {
                        user_id: uid,
                        session_id: sid,
                    };
                    if let Err(e) = hooks.run(&event).await {
                        tracing::warn!("OnSessionEnd hook error: {}", e);
                    }
                });
            }
        }

        // Remove sessions
        let count = {
            let mut sessions = self.sessions.write().await;
            let before = sessions.len();
            for scope_id in &stale_scopes {
                sessions.remove(scope_id);
            }
            before - sessions.len()
        };

        // Clean up thread mappings that point to stale sessions
        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.retain(|key, _| !stale_scopes.contains(&key.scope_id));
        }

        // Clean up undo managers for stale threads
        {
            let mut undo_managers = self.undo_managers.write().await;
            for thread_id in &stale_thread_ids {
                undo_managers.remove(thread_id);
            }
        }

        // Clean up thread ownership for stale threads
        {
            let mut thread_owners = self.thread_owners.write().await;
            for thread_id in &stale_thread_ids {
                thread_owners.remove(thread_id);
            }
        }

        // Clean up workspace locks for pruned users (IC-002)
        // Without this, per-user locks accumulate forever in multi-user deployments.
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

    /// List all active sessions as a summary suitable for CLI display.
    ///
    /// Returns a vec of JSON values with scope/principal details, channel, thread_count, last_active, and owner.
    pub async fn list_sessions(&self) -> Vec<serde_json::Value> {
        let sessions = self.sessions.read().await;
        let thread_owners = self.thread_owners.read().await;
        let mut result = Vec::new();

        for (scope_id, session_arc) in sessions.iter() {
            let session = session_arc.lock().await;
            let thread_count = session.threads.len();

            // Determine the last activity across all threads
            let last_active = session
                .threads
                .values()
                .filter_map(|t| t.turns.last().map(|_| "active"))
                .next()
                .unwrap_or("idle");

            // Check if threads have owners
            let owner = session
                .threads
                .keys()
                .find_map(|tid| thread_owners.get(tid))
                .cloned()
                .unwrap_or_else(|| "—".to_string());

            result.push(serde_json::json!({
                "session_scope_id": scope_id.to_string(),
                "user_id": session.user_id.clone(),
                "principal_id": session.principal_id.clone(),
                "actor_id": session.actor_id.clone(),
                "conversation_kind": session.conversation_kind.as_str(),
                "channel": "unknown",
                "thread_count": thread_count,
                "last_active": last_active,
                "owner": owner,
            }));
        }

        result
    }

    /// Describe a specific session with thread-level detail.
    ///
    /// Returns `None` if no session exists for the given user.
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

        let threads: Vec<serde_json::Value> = session
            .threads
            .iter()
            .map(|(tid, thread)| {
                let owner = thread_owners
                    .get(tid)
                    .cloned()
                    .unwrap_or_else(|| "(unowned)".to_string());
                let msg_count = thread.turns.len();
                let state = format!("{:?}", thread.state);

                serde_json::json!({
                    "thread_id": tid.to_string(),
                    "owner": owner,
                    "state": state,
                    "message_count": msg_count,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scope_id(user_id: &str) -> Uuid {
        SessionManager::scope_id_for_user_id(user_id)
    }

    #[tokio::test]
    async fn test_get_or_create_session() {
        let manager = SessionManager::new();

        let session1 = manager.get_or_create_session("user-1").await;
        let session2 = manager.get_or_create_session("user-1").await;

        // Same user should get same session
        assert!(Arc::ptr_eq(&session1, &session2));

        let session3 = manager.get_or_create_session("user-2").await;
        assert!(!Arc::ptr_eq(&session1, &session3));
    }

    #[tokio::test]
    async fn test_resolve_thread() {
        let manager = SessionManager::new();

        let (session1, thread1) = manager.resolve_thread("user-1", "cli", None).await;
        let (session2, thread2) = manager.resolve_thread("user-1", "cli", None).await;

        // Same channel+user should get same thread
        assert!(Arc::ptr_eq(&session1, &session2));
        assert_eq!(thread1, thread2);

        // Direct sessions now share the default thread across channels.
        let (_, thread3) = manager.resolve_thread("user-1", "http", None).await;
        assert_eq!(thread1, thread3);
    }

    #[tokio::test]
    async fn test_undo_manager() {
        let manager = SessionManager::new();
        let (_, thread_id) = manager.resolve_thread("user-1", "cli", None).await;

        let undo1 = manager.get_undo_manager(thread_id).await;
        let undo2 = manager.get_undo_manager(thread_id).await;

        assert!(Arc::ptr_eq(&undo1, &undo2));
    }

    #[tokio::test]
    async fn test_prune_stale_sessions() {
        let manager = SessionManager::new();

        // Create two sessions and resolve threads (which updates last_active_at)
        let (_, _thread_id) = manager.resolve_thread("user-active", "cli", None).await;
        let (s2, _thread_id) = manager.resolve_thread("user-stale", "cli", None).await;

        // Backdate the stale session's last_active_at AFTER thread creation
        {
            let mut sess = s2.lock().await;
            sess.last_active_at = chrono::Utc::now() - chrono::TimeDelta::seconds(86400 * 10); // 10 days ago
        }

        // Prune with 7-day timeout
        let pruned = manager
            .prune_stale_sessions(std::time::Duration::from_secs(86400 * 7))
            .await;
        assert_eq!(pruned, 1);

        // Active session should still exist
        let sessions = manager.sessions.read().await;
        assert!(sessions.contains_key(&scope_id("user-active")));
        assert!(!sessions.contains_key(&scope_id("user-stale")));
    }

    #[tokio::test]
    async fn test_prune_no_stale_sessions() {
        let manager = SessionManager::new();
        let _s1 = manager.get_or_create_session("user-1").await;

        // Nothing should be pruned when timeout is long
        let pruned = manager
            .prune_stale_sessions(std::time::Duration::from_secs(86400 * 365))
            .await;
        assert_eq!(pruned, 0);
    }

    #[tokio::test]
    async fn test_register_thread() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let thread_id = Uuid::new_v4();

        // Create a session with a hydrated thread
        let session = Arc::new(Mutex::new(Session::new("user-hydrate")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(thread_id, sess.id);
            sess.threads.insert(thread_id, thread);
            sess.active_thread = Some(thread_id);
        }

        // Register the thread
        manager
            .register_thread("user-hydrate", "gateway", thread_id, Arc::clone(&session))
            .await;

        // resolve_thread should find it (using the UUID as external_thread_id)
        let (resolved_session, resolved_tid) = manager
            .resolve_thread("user-hydrate", "gateway", Some(&thread_id.to_string()))
            .await;
        assert_eq!(resolved_tid, thread_id);

        // Should be the same session object
        let sess = resolved_session.lock().await;
        assert!(sess.threads.contains_key(&thread_id));
    }

    #[tokio::test]
    async fn test_resolve_thread_with_explicit_external_id() {
        let manager = SessionManager::new();

        // Two calls with the same explicit external thread ID should resolve
        // to the same internal thread.
        let (_, t1) = manager
            .resolve_thread("user-1", "gateway", Some("ext-abc"))
            .await;
        let (_, t2) = manager
            .resolve_thread("user-1", "gateway", Some("ext-abc"))
            .await;
        assert_eq!(t1, t2);

        // A different external ID on the same channel/user gets a new thread.
        let (_, t3) = manager
            .resolve_thread("user-1", "gateway", Some("ext-xyz"))
            .await;
        assert_ne!(t1, t3);
    }

    #[tokio::test]
    async fn test_resolve_thread_none_vs_some_external_id() {
        let manager = SessionManager::new();

        // None external_thread_id is a distinct key from Some("ext-1").
        let (_, t_none) = manager.resolve_thread("user-1", "cli", None).await;
        let (_, t_some) = manager.resolve_thread("user-1", "cli", Some("ext-1")).await;
        assert_ne!(t_none, t_some);
    }

    #[tokio::test]
    async fn test_resolve_thread_different_users_isolated() {
        let manager = SessionManager::new();

        let (_, t1) = manager
            .resolve_thread("user-a", "gateway", Some("same-ext"))
            .await;
        let (_, t2) = manager
            .resolve_thread("user-b", "gateway", Some("same-ext"))
            .await;

        // Same channel + same external ID but different users = different threads
        assert_ne!(t1, t2);
    }

    #[tokio::test]
    async fn test_resolve_thread_different_channels_share_direct_aliases() {
        let manager = SessionManager::new();

        let (_, t1) = manager
            .resolve_thread("user-1", "gateway", Some("thread-x"))
            .await;
        let (_, t2) = manager
            .resolve_thread("user-1", "telegram", Some("thread-x"))
            .await;

        // Direct aliases are channel-agnostic now, so this resolves to one thread.
        assert_eq!(t1, t2);
    }

    #[tokio::test]
    async fn test_resolve_thread_stale_mapping_creates_new_thread() {
        let manager = SessionManager::new();

        // Create a thread normally
        let (session, original_tid) = manager
            .resolve_thread("user-1", "gateway", Some("ext-1"))
            .await;

        // Simulate the thread being removed from the session (e.g. pruned)
        {
            let mut sess = session.lock().await;
            sess.threads.remove(&original_tid);
        }

        // Next resolve should detect the stale mapping and create a fresh thread
        let (_, new_tid) = manager
            .resolve_thread("user-1", "gateway", Some("ext-1"))
            .await;
        assert_ne!(original_tid, new_tid);

        // The new thread should actually exist in the session
        let sess = session.lock().await;
        assert!(sess.threads.contains_key(&new_tid));
    }

    #[tokio::test]
    async fn test_register_thread_preserves_uuid_on_resolve() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let known_uuid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("user-web")));
        let session_id = {
            let sess = session.lock().await;
            sess.id
        };

        // Simulate hydration: create thread with a known UUID
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(known_uuid, session_id);
            sess.threads.insert(known_uuid, thread);
        }

        // Register it
        manager
            .register_thread("user-web", "gateway", known_uuid, Arc::clone(&session))
            .await;

        // resolve_thread with UUID as external_thread_id MUST return the same UUID,
        // not mint a new one (this was the root cause of the "wrong conversation" bug)
        let (_, resolved) = manager
            .resolve_thread("user-web", "gateway", Some(&known_uuid.to_string()))
            .await;
        assert_eq!(resolved, known_uuid);
    }

    #[tokio::test]
    async fn test_register_thread_idempotent() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("user-idem")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }

        // Register twice
        manager
            .register_thread("user-idem", "gateway", tid, Arc::clone(&session))
            .await;
        manager
            .register_thread("user-idem", "gateway", tid, Arc::clone(&session))
            .await;

        // Should still resolve to the same thread
        let (_, resolved) = manager
            .resolve_thread("user-idem", "gateway", Some(&tid.to_string()))
            .await;
        assert_eq!(resolved, tid);
    }

    #[tokio::test]
    async fn test_register_thread_creates_undo_manager() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("user-undo")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }

        manager
            .register_thread("user-undo", "gateway", tid, Arc::clone(&session))
            .await;

        // Undo manager should exist for the registered thread
        let undo = manager.get_undo_manager(tid).await;
        let undo2 = manager.get_undo_manager(tid).await;
        assert!(Arc::ptr_eq(&undo, &undo2));
    }

    #[tokio::test]
    async fn test_register_thread_stores_session() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("user-new")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }

        // The user has no session yet in the manager
        {
            let sessions = manager.sessions.read().await;
            assert!(!sessions.contains_key(&scope_id("user-new")));
        }

        manager
            .register_thread("user-new", "gateway", tid, Arc::clone(&session))
            .await;

        // Now the session should be tracked
        {
            let sessions = manager.sessions.read().await;
            assert!(sessions.contains_key(&scope_id("user-new")));
        }
    }

    #[tokio::test]
    async fn test_multiple_threads_per_user() {
        let manager = SessionManager::new();

        let (_, t1) = manager
            .resolve_thread("user-1", "gateway", Some("thread-a"))
            .await;
        let (_, t2) = manager
            .resolve_thread("user-1", "gateway", Some("thread-b"))
            .await;
        let (session, t3) = manager
            .resolve_thread("user-1", "gateway", Some("thread-c"))
            .await;

        // All three should be distinct
        assert_ne!(t1, t2);
        assert_ne!(t2, t3);
        assert_ne!(t1, t3);

        // All three should exist in the same session
        let sess = session.lock().await;
        assert!(sess.threads.contains_key(&t1));
        assert!(sess.threads.contains_key(&t2));
        assert!(sess.threads.contains_key(&t3));
    }

    #[tokio::test]
    async fn test_prune_cleans_thread_map_and_undo_managers() {
        let manager = SessionManager::new();

        let (stale_session, stale_tid) = manager.resolve_thread("user-stale", "cli", None).await;

        // Backdate the session
        {
            let mut sess = stale_session.lock().await;
            sess.last_active_at = chrono::Utc::now() - chrono::TimeDelta::seconds(86400 * 30);
        }

        // Verify thread_map and undo_managers have entries
        {
            let tm = manager.thread_map.read().await;
            assert!(!tm.is_empty());
        }
        {
            let um = manager.undo_managers.read().await;
            assert!(um.contains_key(&stale_tid));
        }

        let pruned = manager
            .prune_stale_sessions(std::time::Duration::from_secs(86400 * 7))
            .await;
        assert_eq!(pruned, 1);

        // Thread map and undo managers should be cleaned up
        {
            let tm = manager.thread_map.read().await;
            assert!(tm.is_empty());
        }
        {
            let um = manager.undo_managers.read().await;
            assert!(!um.contains_key(&stale_tid));
        }
    }

    #[tokio::test]
    async fn test_resolve_thread_active_thread_set() {
        let manager = SessionManager::new();

        let (session, thread_id) = manager
            .resolve_thread("user-1", "gateway", Some("ext-1"))
            .await;

        // The resolved thread should be set as the active thread
        let sess = session.lock().await;
        assert_eq!(sess.active_thread, Some(thread_id));
    }

    #[tokio::test]
    async fn test_register_then_resolve_different_channel_reuses_thread() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("user-cross")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }

        // Register on "gateway" channel
        manager
            .register_thread("user-cross", "gateway", tid, Arc::clone(&session))
            .await;

        // Resolve on a different channel with the same UUID string should reuse
        // the same thread (cross-channel UUID aliasing for direct sessions).
        let (_, resolved) = manager
            .resolve_thread("user-cross", "telegram", Some(&tid.to_string()))
            .await;
        assert_eq!(resolved, tid);
    }

    #[tokio::test]
    async fn test_resolve_thread_finds_existing_session_thread_by_uuid() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        // Simulate chat_new_thread_handler: create thread directly in session
        // without registering it in thread_map
        let session = Arc::new(Mutex::new(Session::new("user-direct")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }
        {
            let mut sessions = manager.sessions.write().await;
            sessions.insert(scope_id("user-direct"), Arc::clone(&session));
        }

        // resolve_thread should find the existing thread by UUID
        // instead of creating a duplicate
        let (_, resolved) = manager
            .resolve_thread("user-direct", "gateway", Some(&tid.to_string()))
            .await;
        assert_eq!(
            resolved, tid,
            "should reuse existing thread, not create a new one"
        );

        // Verify no duplicate threads were created
        let sess = session.lock().await;
        assert_eq!(
            sess.threads.len(),
            1,
            "should have exactly 1 thread, not a duplicate"
        );
    }

    #[tokio::test]
    async fn test_resolve_thread_for_identity_direct_uses_principal_scope() {
        let manager = SessionManager::new();

        let identity_gateway = ResolvedIdentity {
            principal_id: "user-shared".to_string(),
            actor_id: "phone".to_string(),
            conversation_scope_id: scope_id_from_key("gateway://direct/user-shared/actor/phone"),
            conversation_kind: ConversationKind::Direct,
            raw_sender_id: "user-shared".to_string(),
            stable_external_conversation_key: "gateway://direct/user-shared/actor/phone"
                .to_string(),
        };
        let identity_cli = ResolvedIdentity {
            principal_id: "user-shared".to_string(),
            actor_id: "desktop".to_string(),
            conversation_scope_id: scope_id_from_key("cli:direct:user-shared"),
            conversation_kind: ConversationKind::Direct,
            raw_sender_id: "user-shared".to_string(),
            stable_external_conversation_key: "cli:direct:user-shared".to_string(),
        };

        let (session1, thread1) = manager
            .resolve_thread_for_identity(&identity_gateway, "gateway", None)
            .await;
        let (session2, thread2) = manager
            .resolve_thread_for_identity(&identity_cli, "cli", None)
            .await;

        assert!(Arc::ptr_eq(&session1, &session2));
        assert_eq!(thread1, thread2);
    }

    #[tokio::test]
    async fn test_resolve_thread_for_identity_group_scope_stays_isolated() {
        let manager = SessionManager::new();

        let identity_signal_group = ResolvedIdentity {
            principal_id: "user-group".to_string(),
            actor_id: "user-group".to_string(),
            conversation_scope_id: scope_id_from_key("signal:group:grp-1"),
            conversation_kind: ConversationKind::Group,
            raw_sender_id: "user-group".to_string(),
            stable_external_conversation_key: "signal:group:grp-1".to_string(),
        };
        let identity_telegram_group = ResolvedIdentity {
            principal_id: "user-group".to_string(),
            actor_id: "user-group".to_string(),
            conversation_scope_id: scope_id_from_key("telegram:group:grp-1"),
            conversation_kind: ConversationKind::Group,
            raw_sender_id: "user-group".to_string(),
            stable_external_conversation_key: "telegram:group:grp-1".to_string(),
        };

        let (_, thread1) = manager
            .resolve_thread_for_identity(&identity_signal_group, "signal", Some("grp-1"))
            .await;
        let (_, thread2) = manager
            .resolve_thread_for_identity(&identity_telegram_group, "telegram", Some("grp-1"))
            .await;

        assert_ne!(thread1, thread2);
    }
}
