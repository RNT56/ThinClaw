//! Root-independent session manager for multi-user, multi-thread conversations.
//!
//! Direct sessions are principal+actor scoped and share the default direct
//! thread across channels for that actor. Group sessions remain isolated by
//! conversation scope.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeDelta, Utc};
use thinclaw_identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::session::{PendingApproval, Session};
use crate::undo::UndoManager;

pub const SESSION_COUNT_WARNING_THRESHOLD: usize = 1000;
pub const DIRECT_MAIN_THREAD_KEY: &str = "__direct_main__";
const SESSION_LIFECYCLE_HOOK_TIMEOUT: Duration = Duration::from_secs(30);

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
    /// Serializes all stateful submissions within one conversation scope.
    /// Interrupts deliberately bypass this lock at the root adapter.
    execution_locks: RwLock<HashMap<Uuid, Arc<Mutex<()>>>>,
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
            execution_locks: RwLock::new(HashMap::new()),
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

    pub async fn execution_lock_for_identity(&self, identity: &ResolvedIdentity) -> Arc<Mutex<()>> {
        let scope_id = Self::session_scope_for_identity(identity);
        {
            let locks = self.execution_locks.read().await;
            if let Some(lock) = locks.get(&scope_id) {
                return Arc::clone(lock);
            }
        }

        let mut locks = self.execution_locks.write().await;
        locks
            .entry(scope_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
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

    /// Locate an in-memory approval by its globally unique request ID.
    ///
    /// Privileged local hosts (notably the desktop UI) receive approval cards
    /// asynchronously and cannot safely guess which thread currently owns the
    /// request. Returning the original pending envelope lets the host route the
    /// decision through the normal actor-bound message pipeline.
    pub async fn find_pending_approval(
        &self,
        request_id: Uuid,
    ) -> Option<(Arc<Mutex<Session>>, Uuid, PendingApproval)> {
        let sessions = self
            .sessions
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();

        for session in sessions {
            let found = {
                let guard = session.lock().await;
                guard.threads.iter().find_map(|(thread_id, thread)| {
                    thread
                        .pending_approval
                        .as_ref()
                        .filter(|pending| pending.request_id == request_id)
                        .cloned()
                        .map(|pending| (*thread_id, pending))
                })
            };
            if let Some((thread_id, pending)) = found {
                return Some((session, thread_id, pending));
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

        let hooks = self.hooks.clone();
        drop(sessions);

        if let Some(hooks) = hooks {
            let user_id = principal_id.to_string();
            if tokio::time::timeout(
                SESSION_LIFECYCLE_HOOK_TIMEOUT,
                hooks.session_started(user_id, session_id),
            )
            .await
            .is_err()
            {
                tracing::warn!(
                    scope = %scope_id,
                    timeout_ms = SESSION_LIFECYCLE_HOOK_TIMEOUT.as_millis() as u64,
                    "Session-start hook timed out"
                );
            }
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

    /// Resolve an already-loaded thread without creating a session, thread, or
    /// mapping. Control-plane operations such as cancellation must be lookup
    /// only: a typo or stale UI key must not manufacture a ghost conversation.
    pub async fn lookup_thread_for_identity(
        &self,
        identity: &ResolvedIdentity,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> Option<(Arc<Mutex<Session>>, Uuid)> {
        let scope_id = Self::session_scope_for_identity(identity);
        let normalized =
            normalize_external_thread_key(identity.conversation_kind, channel, external_thread_id);
        let mapped_thread = self
            .thread_map
            .read()
            .await
            .get(&ThreadKey {
                scope_id,
                external_thread_id: normalized.clone(),
            })
            .copied();
        let session = self.sessions.read().await.get(&scope_id).cloned()?;

        if let Some(thread_id) = mapped_thread {
            let contains_thread = session.lock().await.threads.contains_key(&thread_id);
            if contains_thread {
                return Some((session, thread_id));
            }
        }

        let external_uuid = normalized
            .as_deref()
            .and_then(|value| Uuid::parse_str(value).ok())?;
        let contains_thread = session.lock().await.threads.contains_key(&external_uuid);
        if contains_thread {
            Some((session, external_uuid))
        } else {
            None
        }
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

        // Hold the map's write lock through lookup and creation.  External
        // surfaces can submit concurrently, and the former read/check/create/
        // insert sequence allowed two first messages for the same key to
        // create different threads and orphan one of them.
        let mut thread_map = self.thread_map.write().await;
        if let Some(&thread_id) = thread_map.get(&key) {
            let sess = session.lock().await;
            if sess.threads.contains_key(&thread_id) {
                return (Arc::clone(&session), thread_id);
            }
            // Stale mappings are repaired below while the admission lock is
            // still held, so no competing resolver can observe the gap.
            thread_map.remove(&key);
        }

        if let Some(ext_tid) = external_thread_id.as_deref()
            && let Ok(ext_uuid) = Uuid::parse_str(ext_tid)
        {
            let sess = session.lock().await;
            if sess.threads.contains_key(&ext_uuid) {
                drop(sess);

                thread_map.insert(key, ext_uuid);
                drop(thread_map);
                self.ensure_undo_manager(ext_uuid).await;
                return (session, ext_uuid);
            }
        }

        let thread_id = {
            let mut sess = session.lock().await;
            sess.create_thread().id
        };

        thread_map.insert(key, thread_id);
        drop(thread_map);
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

    /// Point a concrete ingress thread key at an existing in-memory thread.
    ///
    /// Lifecycle commands such as `/new` and `/thread` change which internal
    /// thread subsequent messages on the *same external conversation key*
    /// should use. Merely changing `Session::active_thread` is insufficient:
    /// normal ingress resolves through `thread_map` first and would otherwise
    /// continue routing to the previous thread.
    pub async fn register_thread_alias_for_scope(
        &self,
        scope_id: Uuid,
        conversation_kind: ConversationKind,
        channel: &str,
        external_thread_id: Option<&str>,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        let key = ThreadKey {
            scope_id,
            external_thread_id: normalize_external_thread_key(
                conversation_kind,
                channel,
                external_thread_id,
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
        let stale_candidates: Vec<Uuid> = {
            let sessions = self.sessions.read().await;
            sessions
                .iter()
                .filter_map(|(scope_id, session)| {
                    let sess = session.try_lock().ok()?;
                    if sess.last_active_at < cutoff {
                        Some(*scope_id)
                    } else {
                        None
                    }
                })
                .collect()
        };

        if stale_candidates.is_empty() {
            return 0;
        }

        // Freeze admission-lock lookup while revalidating and removing. Every
        // stateful root submission clones its scope lock before it resolves a
        // session; a strong count above one therefore means a turn is active or
        // queued even if `last_active_at` has not yet reached its first
        // persistence boundary. A new submitter either obtains the old lock
        // before this write guard (and makes us skip it), or waits and observes
        // the fresh session/lock after removal.
        let mut execution_locks = self.execution_locks.write().await;
        let removed: Vec<(Uuid, String, String, Vec<Uuid>)> = {
            let mut sessions = self.sessions.write().await;
            let mut removed = Vec::new();
            for scope_id in stale_candidates {
                if execution_locks
                    .get(&scope_id)
                    .is_some_and(|lock| Arc::strong_count(lock) > 1)
                {
                    continue;
                }

                let Some(session) = sessions.get(&scope_id).cloned() else {
                    continue;
                };
                let Ok(sess) = session.try_lock() else {
                    continue;
                };
                if sess.last_active_at >= cutoff {
                    continue;
                }
                let removed_entry = (
                    scope_id,
                    sess.principal_id.clone(),
                    sess.id.to_string(),
                    sess.threads.keys().copied().collect(),
                );
                drop(sess);
                sessions.remove(&scope_id);
                execution_locks.remove(&scope_id);
                removed.push(removed_entry);
            }
            removed
        };

        if removed.is_empty() {
            return 0;
        }

        let count = removed.len();
        let stale_scopes: Vec<_> = removed
            .iter()
            .map(|(scope_id, _, _, _)| *scope_id)
            .collect();
        let stale_principals: Vec<_> = removed
            .iter()
            .map(|(_, principal, _, _)| principal.clone())
            .collect();
        let stale_sessions: Vec<_> = removed
            .iter()
            .map(|(_, principal, session_id, _)| (principal.clone(), session_id.clone()))
            .collect();
        let stale_thread_ids: Vec<_> = removed
            .into_iter()
            .flat_map(|(_, _, _, thread_ids)| thread_ids)
            .collect();

        // Workspace locks are principal-scoped because sibling actors share
        // one household workspace. Pruning one stale actor session must not
        // remove the lock while another actor under the same principal is
        // still active, or subsequent operations could acquire two distinct
        // locks and mutate that workspace concurrently.
        let remaining_sessions = self
            .sessions
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut remaining_principals = HashSet::new();
        let mut inspected_all_remaining_sessions = true;
        for session in remaining_sessions {
            if let Ok(session) = session.try_lock() {
                remaining_principals.insert(session.principal_id.clone());
            } else {
                // An in-use session is conservatively treated as potentially
                // sharing any stale principal. Workspace-lock cleanup can wait
                // for the next pruning pass; admission must not be held across
                // an unbounded session-mutex wait.
                inspected_all_remaining_sessions = false;
                break;
            }
        }

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
            if inspected_all_remaining_sessions {
                for user_id in &stale_principals {
                    if !remaining_principals.contains(user_id) {
                        locks.remove(user_id);
                    }
                }
            }
        }

        // Keep admission frozen through dependent-map cleanup. Releasing it
        // immediately after removing `sessions` would let a new submitter
        // recreate this scope, only for stale dependent-map cleanup to delete
        // its fresh mapping.
        drop(execution_locks);

        if let Some(hooks) = self.hooks.clone() {
            let hook_calls = stale_sessions.into_iter().map(|(user_id, session_id)| {
                let hooks = Arc::clone(&hooks);
                async move {
                    hooks.session_ended(user_id, session_id).await;
                }
            });
            if tokio::time::timeout(
                SESSION_LIFECYCLE_HOOK_TIMEOUT,
                futures::future::join_all(hook_calls),
            )
            .await
            .is_err()
            {
                tracing::warn!(
                    count,
                    timeout_ms = SESSION_LIFECYCLE_HOOK_TIMEOUT.as_millis() as u64,
                    "Session-end hooks timed out"
                );
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

/// Stable direct-session scope for one actor within a principal/household.
///
/// Preserve the historical principal-only UUID when actor and principal are
/// identical so existing single-user deployments keep their active session
/// and direct-main-thread mapping.  Family-member actors get an unambiguous
/// length-prefixed key to avoid delimiter collisions.
pub fn direct_scope_id_for_actor(principal_id: &str, actor_id: &str) -> Uuid {
    thinclaw_identity::direct_scope_id(principal_id, actor_id)
}

pub fn session_scope_for_identity(identity: &ResolvedIdentity) -> Uuid {
    match identity.conversation_kind {
        ConversationKind::Direct => {
            direct_scope_id_for_actor(&identity.principal_id, &identity.actor_id)
        }
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
    fn direct_scope_uses_principal_and_actor_without_changing_legacy_owner_scope() {
        let identity = identity(ConversationKind::Direct);
        assert_ne!(identity.principal_id, identity.actor_id);
        assert_eq!(
            session_scope_for_identity(&identity),
            direct_scope_id_for_actor("principal-1", "actor-1")
        );
        assert_ne!(
            session_scope_for_identity(&identity),
            scope_id_for_user_id("principal-1")
        );

        let mut owner_identity = identity;
        owner_identity.actor_id = owner_identity.principal_id.clone();
        assert_eq!(
            session_scope_for_identity(&owner_identity),
            scope_id_for_user_id("principal-1")
        );
    }

    #[tokio::test]
    async fn direct_family_actors_never_share_sessions_or_default_threads() {
        let manager = SessionManager::new();
        let actor_one = identity(ConversationKind::Direct);
        let mut actor_two = actor_one.clone();
        actor_two.actor_id = "actor-2".to_string();
        actor_two.raw_sender_id = "sender-2".to_string();

        let (session_one, thread_one) = manager
            .resolve_thread_for_identity(&actor_one, "gateway", None)
            .await;
        let (session_two, thread_two) = manager
            .resolve_thread_for_identity(&actor_two, "gateway", None)
            .await;

        assert!(!Arc::ptr_eq(&session_one, &session_two));
        assert_ne!(thread_one, thread_two);
        assert_eq!(session_one.lock().await.actor_id, "actor-1");
        assert_eq!(session_two.lock().await.actor_id, "actor-2");
    }

    #[tokio::test]
    async fn concurrent_first_resolution_creates_exactly_one_thread() {
        let manager = Arc::new(SessionManager::new());
        let identity = identity(ConversationKind::Direct);
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let manager = Arc::clone(&manager);
            let identity = identity.clone();
            tasks.push(tokio::spawn(async move {
                manager
                    .resolve_thread_for_identity(&identity, "gateway", Some("external-thread"))
                    .await
                    .1
            }));
        }

        let mut resolved = Vec::new();
        for task in tasks {
            resolved.push(task.await.expect("resolver task"));
        }
        assert!(resolved.iter().all(|id| *id == resolved[0]));

        let session = manager.get_or_create_session_for_identity(&identity).await;
        assert_eq!(session.lock().await.threads.len(), 1);
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
    async fn registering_ingress_alias_retargets_existing_external_key() {
        let manager = SessionManager::new();
        let identity = identity(ConversationKind::Direct);
        let (session, old_thread) = manager
            .resolve_thread_for_identity(&identity, "tauri", Some("agent:main"))
            .await;
        let new_thread = {
            let mut session = session.lock().await;
            session.create_thread().id
        };
        manager
            .register_thread_alias_for_scope(
                SessionManager::session_scope_for_identity(&identity),
                ConversationKind::Direct,
                "tauri",
                Some("agent:main"),
                new_thread,
                Arc::clone(&session),
            )
            .await;

        let (_, resolved) = manager
            .resolve_thread_for_identity(&identity, "tauri", Some("agent:main"))
            .await;
        assert_ne!(old_thread, new_thread);
        assert_eq!(resolved, new_thread);
    }

    #[tokio::test]
    async fn lookup_only_resolution_finds_alias_without_creating_ghost_state() {
        let manager = SessionManager::new();
        let identity = identity(ConversationKind::Direct);

        assert!(
            manager
                .lookup_thread_for_identity(&identity, "tauri", Some("missing"))
                .await
                .is_none()
        );
        assert!(manager.sessions.read().await.is_empty());
        assert!(manager.thread_map.read().await.is_empty());

        let (_, thread_id) = manager
            .resolve_thread_for_identity(&identity, "tauri", Some("agent:main"))
            .await;
        let (_, looked_up) = manager
            .lookup_thread_for_identity(&identity, "tauri", Some("agent:main"))
            .await
            .expect("loaded alias");
        assert_eq!(looked_up, thread_id);

        let mut other_actor = identity.clone();
        other_actor.actor_id = "other-actor".to_string();
        assert!(
            manager
                .lookup_thread_for_identity(&other_actor, "tauri", Some("agent:main"))
                .await
                .is_none()
        );
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

    #[tokio::test]
    async fn prune_never_removes_scope_with_active_or_queued_submission_lock() {
        let manager = SessionManager::new();
        let identity = identity(ConversationKind::Direct);
        let (session, _) = manager
            .resolve_thread_for_identity(&identity, "gateway", None)
            .await;
        session.lock().await.last_active_at = Utc::now() - TimeDelta::days(30);
        let execution_lock = manager.execution_lock_for_identity(&identity).await;
        let execution_guard = execution_lock.lock().await;

        assert_eq!(
            manager
                .prune_stale_sessions(Duration::from_secs(7 * 86_400))
                .await,
            0
        );
        assert!(
            manager
                .sessions
                .read()
                .await
                .contains_key(&SessionManager::session_scope_for_identity(&identity))
        );

        drop(execution_guard);
        drop(execution_lock);
        assert_eq!(
            manager
                .prune_stale_sessions(Duration::from_secs(7 * 86_400))
                .await,
            1
        );
    }

    #[tokio::test]
    async fn pruning_one_actor_keeps_shared_principal_workspace_lock() {
        let manager = SessionManager::new();
        let actor_one = identity(ConversationKind::Direct);
        let mut actor_two = actor_one.clone();
        actor_two.actor_id = "actor-2".to_string();
        actor_two.raw_sender_id = "sender-2".to_string();

        let stale = manager.get_or_create_session_for_identity(&actor_one).await;
        let active = manager.get_or_create_session_for_identity(&actor_two).await;
        stale.lock().await.last_active_at = Utc::now() - TimeDelta::days(30);
        active.lock().await.last_active_at = Utc::now();
        manager.workspace_lock(&actor_one.principal_id).await;

        assert_eq!(
            manager
                .prune_stale_sessions(Duration::from_secs(7 * 86_400))
                .await,
            1
        );
        assert!(
            manager
                .workspace_locks
                .read()
                .await
                .contains_key(&actor_one.principal_id)
        );
    }
}
