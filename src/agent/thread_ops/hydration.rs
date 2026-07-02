//! Thread visibility, hydration from persisted history, and subagent resume.

use std::sync::Arc;

use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::mutate_thread_runtime;
use crate::agent::session::{PersistedSubagentState, ThreadRuntimeStateExt};
use crate::channels::IncomingMessage;
use crate::db::Database;
use crate::history::ConversationKind as HistoryConversationKind;
use crate::identity::ResolvedIdentity;
use thinclaw_agent::thread_ops::{
    ThreadVisibilityDecision, direct_conversation_candidate_is_primary,
    direct_conversation_metadata_updates, is_primary_direct_thread_metadata,
};

pub(in crate::agent) fn to_history_conversation_kind(
    kind: crate::identity::ConversationKind,
) -> HistoryConversationKind {
    match kind {
        crate::identity::ConversationKind::Direct => HistoryConversationKind::Direct,
        crate::identity::ConversationKind::Group => HistoryConversationKind::Group,
    }
}

impl Agent {
    async fn conversation_visible_to_identity(
        &self,
        store: &Arc<dyn Database>,
        conversation_id: Uuid,
        identity: &ResolvedIdentity,
    ) -> bool {
        let metadata = match store.get_conversation_metadata(conversation_id).await {
            Ok(metadata) => metadata,
            Err(err) => {
                tracing::warn!(
                    thread = %conversation_id,
                    error = %err,
                    "Failed to read conversation metadata while checking ownership"
                );
                return false;
            }
        };
        if metadata.is_none() {
            return true;
        }

        let belongs_to_actor = match store
            .conversation_belongs_to_actor(
                conversation_id,
                &identity.principal_id,
                &identity.actor_id,
            )
            .await
        {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(
                    thread = %conversation_id,
                    error = %err,
                    "Failed to verify actor ownership while hydrating thread"
                );
                return false;
            }
        };

        match thinclaw_agent::thread_ops::thread_visibility_after_actor_membership(
            &identity.principal_id,
            &identity.actor_id,
            belongs_to_actor,
        ) {
            ThreadVisibilityDecision::Visible => true,
            ThreadVisibilityDecision::CheckPrincipalUser => store
                .conversation_belongs_to_user(conversation_id, &identity.principal_id)
                .await
                .unwrap_or(false),
            ThreadVisibilityDecision::Hidden => false,
        }
    }

    pub(in crate::agent) async fn ensure_persisted_conversation(
        &self,
        thread_id: Uuid,
        message: &IncomingMessage,
        identity: &ResolvedIdentity,
    ) -> Option<Arc<dyn Database>> {
        let store = self.store().map(Arc::clone)?;
        if let Err(err) = store
            .ensure_conversation(
                thread_id,
                &message.channel,
                &identity.principal_id,
                message.thread_id.as_deref(),
            )
            .await
        {
            tracing::warn!("Failed to ensure conversation {}: {}", thread_id, err);
            return None;
        }
        if let Err(err) = store
            .update_conversation_identity(
                thread_id,
                Some(&identity.principal_id),
                Some(&identity.actor_id),
                Some(identity.conversation_scope_id),
                to_history_conversation_kind(identity.conversation_kind),
                Some(&identity.stable_external_conversation_key),
            )
            .await
        {
            tracing::warn!(
                "Failed to persist conversation identity for {}: {}",
                thread_id,
                err
            );
            return None;
        }
        self.update_direct_conversation_metadata(&store, thread_id, message, identity)
            .await;
        if matches!(
            identity.conversation_kind,
            crate::identity::ConversationKind::Direct
        ) && let Some(workspace) = self.workspace().cloned()
        {
            let user_timezone = workspace.effective_timezone().name().to_string();
            if let Err(err) = crate::profile_evolution::upsert_profile_evolution_routine(
                &store,
                &workspace,
                &identity.principal_id,
                &identity.actor_id,
                Some(user_timezone.as_str()),
            )
            .await
            {
                tracing::debug!(
                    thread = %thread_id,
                    actor = %identity.actor_id,
                    error = %err,
                    "Failed to upsert actor profile evolution routine"
                );
            }
        }
        Some(store)
    }

    async fn update_direct_conversation_metadata(
        &self,
        store: &Arc<dyn Database>,
        thread_id: Uuid,
        message: &IncomingMessage,
        identity: &ResolvedIdentity,
    ) {
        if !matches!(
            identity.conversation_kind,
            crate::identity::ConversationKind::Direct
        ) {
            return;
        }

        let Ok(Some(metadata)) = store.get_conversation_metadata(thread_id).await else {
            return;
        };

        let updates = direct_conversation_metadata_updates(
            &metadata,
            &message.channel,
            message.thread_id.is_some(),
        );

        if updates.is_empty() {
            return;
        }

        for (key, value) in updates {
            if let Err(err) = store
                .update_conversation_metadata_field(thread_id, key, &value)
                .await
            {
                tracing::debug!(
                    thread = %thread_id,
                    key,
                    error = %err,
                    "Failed to update direct conversation metadata"
                );
            }
        }
    }

    async fn primary_direct_conversation_id(&self, identity: &ResolvedIdentity) -> Option<Uuid> {
        if !matches!(
            identity.conversation_kind,
            crate::identity::ConversationKind::Direct
        ) {
            return None;
        }

        let store = self.store().map(Arc::clone)?;
        let summaries = store
            .list_actor_conversations_for_recall(
                &identity.principal_id,
                &identity.actor_id,
                false,
                50,
            )
            .await
            .ok()?;

        if summaries.is_empty() {
            return None;
        }

        let mut fallback = None;
        for summary in summaries {
            fallback.get_or_insert(summary.id);
            let Ok(Some(metadata)) = store.get_conversation_metadata(summary.id).await else {
                continue;
            };
            if direct_conversation_candidate_is_primary(&metadata, summary.thread_type.as_deref()) {
                return Some(summary.id);
            }
        }

        fallback
    }

    pub(in crate::agent) async fn maybe_hydrate_primary_direct_thread(
        &self,
        message: &IncomingMessage,
    ) {
        if message.thread_id.is_some() {
            return;
        }

        let identity = message.resolved_identity();
        if !matches!(
            identity.conversation_kind,
            crate::identity::ConversationKind::Direct
        ) {
            return;
        }

        let Some(primary_thread_id) = self.primary_direct_conversation_id(&identity).await else {
            return;
        };

        self.maybe_hydrate_thread(message, &primary_thread_id.to_string())
            .await;

        if let Some(session) = self
            .session_manager
            .session_for_thread(primary_thread_id)
            .await
        {
            self.session_manager
                .register_direct_main_thread_for_scope(
                    crate::agent::session_manager::SessionManager::scope_id_for_user_id(
                        &identity.principal_id,
                    ),
                    primary_thread_id,
                    session,
                )
                .await;
        }
    }

    async fn resume_persisted_subagents(
        &self,
        message: &IncomingMessage,
        identity: &ResolvedIdentity,
        thread_id: Uuid,
        pending: &[PersistedSubagentState],
    ) {
        let Some(executor) = self.subagent_executor.as_ref() else {
            return;
        };
        let Some(store) = self.store().map(Arc::clone) else {
            return;
        };
        if pending.is_empty() {
            return;
        }

        let mut resumed = pending.to_vec();
        let mut changed = false;
        let mut spawn_metadata = message.metadata.clone();
        if !spawn_metadata.is_object() {
            spawn_metadata = serde_json::json!({});
        }
        if let Some(metadata) = spawn_metadata.as_object_mut() {
            metadata.insert(
                "thread_id".to_string(),
                serde_json::json!(thread_id.to_string()),
            );
            metadata.insert(
                "principal_id".to_string(),
                serde_json::json!(identity.principal_id.clone()),
            );
            metadata.insert(
                "actor_id".to_string(),
                serde_json::json!(identity.actor_id.clone()),
            );
            metadata.insert(
                "conversation_kind".to_string(),
                serde_json::json!(identity.conversation_kind.as_str()),
            );
        }

        for entry in &mut resumed {
            match executor
                .spawn(
                    entry.request.clone(),
                    &message.channel,
                    &spawn_metadata,
                    &message.user_id,
                    Some(identity),
                    Some(&thread_id.to_string()),
                )
                .await
            {
                Ok(result) => {
                    entry.agent_id = result.agent_id;
                    changed = true;
                }
                Err(err) => {
                    tracing::warn!(
                        thread = %thread_id,
                        task = %entry.request.name,
                        error = %err,
                        "Failed to resume persisted subagent after hydration"
                    );
                }
            }
        }

        if changed {
            let _ = mutate_thread_runtime(&store, thread_id, |runtime| {
                runtime.active_subagents = resumed;
            })
            .await;
        }
    }

    /// Hydrate a historical thread from DB into memory if not already present.
    ///
    /// Called before `resolve_thread` so that the session manager finds the
    /// thread on lookup instead of creating a new one.
    ///
    /// Creates an in-memory thread with the exact UUID the frontend sent,
    /// even when the conversation has zero messages (e.g. a brand-new
    /// assistant thread). Without this, `resolve_thread` would mint a
    /// fresh UUID and all messages would land in the wrong conversation.
    pub(in crate::agent) async fn maybe_hydrate_thread(
        &self,
        message: &IncomingMessage,
        external_thread_id: &str,
    ) {
        // Only hydrate UUID-shaped thread IDs (web gateway uses UUIDs)
        let thread_uuid = match Uuid::parse_str(external_thread_id) {
            Ok(id) => id,
            Err(_) => return,
        };

        let identity = message.resolved_identity();
        let store = self.store().map(Arc::clone);
        if let Some(ref store) = store
            && !self
                .conversation_visible_to_identity(store, thread_uuid, &identity)
                .await
        {
            tracing::warn!(
                thread = %thread_uuid,
                principal = %identity.principal_id,
                actor = %identity.actor_id,
                "Refusing to hydrate thread outside the caller's identity scope"
            );
            return;
        }

        // Check if already in memory
        let session = self
            .session_manager
            .get_or_create_session_for_identity(&identity)
            .await;
        {
            let sess = session.lock().await;
            if sess.threads.contains_key(&thread_uuid) {
                return;
            }
        }

        // Load history from DB (may be empty for a newly created thread).
        let msg_count;

        let conversation_metadata = if let Some(ref store) = store {
            store
                .get_conversation_metadata(thread_uuid)
                .await
                .ok()
                .flatten()
        } else {
            None
        };

        // Decode the runtime from the metadata already fetched above —
        // `load_thread_runtime` would refetch the identical row.
        let runtime: Option<crate::agent::ThreadRuntimeState> =
            conversation_metadata.as_ref().and_then(|metadata| {
                thinclaw_agent::thread_runtime::decode_thread_runtime(metadata).unwrap_or(None)
            });

        let db_messages = if let Some(ref store) = store {
            let mut db_messages = store
                .list_conversation_messages(thread_uuid)
                .await
                .unwrap_or_default();
            // Truncate resurrected history to the active-message watermark
            // recorded by `/undo`, `/redo`, `/clear`, or checkpoint resume.
            // Rows are returned oldest-first, so keeping the first N rows
            // matches what the user actually saw at that point — the DB
            // rows past the watermark stay intact as audit history, they
            // are just not replayed into the in-memory thread. `None`
            // (no watermark recorded, e.g. pre-existing threads) keeps
            // every row, preserving prior behavior.
            if let Some(watermark) = runtime
                .as_ref()
                .and_then(|runtime| runtime.active_message_row_count)
            {
                let keep = usize::try_from(watermark.max(0)).unwrap_or(usize::MAX);
                db_messages.truncate(keep);
            }
            msg_count = db_messages.len();
            Some(db_messages)
        } else {
            msg_count = 0;
            None
        };

        // Create thread with the historical ID and restore messages
        let session_id = {
            let sess = session.lock().await;
            sess.id
        };

        let mut thread = crate::agent::session::Thread::with_id(thread_uuid, session_id);
        if let Some(db_messages) = db_messages.as_ref()
            && !db_messages.is_empty()
        {
            thread.restore_from_conversation_messages(db_messages);
        }
        if let Some(runtime) = runtime.as_ref() {
            thread.restore_runtime_state(runtime.clone());
        }

        // Insert into session and register with session manager
        {
            let mut sess = session.lock().await;
            sess.threads.insert(thread_uuid, thread);
            sess.active_thread = Some(thread_uuid);
            sess.last_active_at = chrono::Utc::now();
        }

        let register_scope_id = match identity.conversation_kind {
            crate::identity::ConversationKind::Direct => {
                crate::agent::session_manager::SessionManager::scope_id_for_user_id(
                    &identity.principal_id,
                )
            }
            crate::identity::ConversationKind::Group => identity.conversation_scope_id,
        };
        self.session_manager
            .register_thread_for_scope(
                register_scope_id,
                identity.conversation_kind,
                &message.channel,
                thread_uuid,
                Arc::clone(&session),
            )
            .await;

        if matches!(
            identity.conversation_kind,
            crate::identity::ConversationKind::Direct
        ) && conversation_metadata
            .as_ref()
            .is_some_and(is_primary_direct_thread_metadata)
        {
            self.session_manager
                .register_direct_main_thread_for_scope(
                    register_scope_id,
                    thread_uuid,
                    Arc::clone(&session),
                )
                .await;
        }

        if let Some(runtime) = runtime {
            if let Some(owner) = runtime.owner_agent_id.clone() {
                let _ = self.agent_router.claim_thread(thread_uuid, &owner).await;
                let _ = self
                    .session_manager
                    .set_thread_owner(thread_uuid, &owner)
                    .await;
            }
            if let Some(model_override) = runtime.model_override.clone()
                && let Some(ref overrides) = self.deps.model_override
            {
                overrides
                    .set(format!("thread:{thread_uuid}"), model_override)
                    .await;
            }
            // Restore the capped undo-stack snapshot so `/undo` keeps
            // working after a restart instead of finding an empty,
            // freshly-created `UndoManager` (Problem B). Only restore when
            // there is something to restore: an empty persisted list can
            // legitimately mean "no undo history yet", and overwriting a
            // manager that already exists in memory (e.g. this thread was
            // already active) with an empty one would erase live undo state.
            if !runtime.undo_checkpoints.is_empty() {
                let mut undo = thinclaw_agent::undo::UndoManager::new();
                undo.restore_from_checkpoints(runtime.undo_checkpoints.clone());
                self.session_manager
                    .restore_undo_manager(thread_uuid, undo)
                    .await;
            }
            self.resume_persisted_subagents(
                message,
                &identity,
                thread_uuid,
                &runtime.active_subagents,
            )
            .await;
        }

        // Restore a persisted `/personality` session overlay so it survives
        // a process restart, the same way `model_override` does above. This
        // is stored under a dedicated conversation-metadata key rather than
        // on `ThreadRuntimeSnapshot` (see
        // `crate::agent::commands::PERSONALITY_OVERLAY_METADATA_KEY`), so it
        // is restored independently of `runtime`. Only applied when the
        // session doesn't already carry an active overlay, so an
        // already-active in-memory session (or a personality set after this
        // thread was hydrated) is never clobbered by stale persisted state.
        if let Some(ref store) = store
            && let Some(overlay) =
                crate::agent::commands::load_personality_overlay(store, thread_uuid).await
        {
            let mut sess = session.lock().await;
            if sess.active_personality.is_none() {
                sess.active_personality = Some(overlay);
            }
        }

        tracing::debug!(
            "Hydrated thread {} from DB ({} messages)",
            thread_uuid,
            msg_count
        );
    }
}
