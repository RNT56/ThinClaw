//! Thread visibility, hydration from persisted history, and subagent resume.

use std::sync::Arc;

use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::mutate_thread_runtime;
use crate::agent::session::{PersistedSubagentState, ThreadRuntimeStateExt};
use crate::channels::IncomingMessage;
use crate::db::Database;
use crate::error::Error;
use crate::identity::ResolvedIdentity;
use thinclaw_agent::thread_ops::{
    direct_conversation_candidate_is_primary, direct_conversation_metadata_updates,
    is_primary_direct_thread_metadata,
};

fn leading_unreplayable_rows(messages: &[crate::history::ConversationMessage]) -> usize {
    messages
        .iter()
        .position(|message| {
            message.role == "user"
                || (message.role == "assistant"
                    && thinclaw_agent::session::message_is_startup_hook(&message.metadata))
        })
        .unwrap_or(messages.len())
}

impl Agent {
    async fn conversation_visible_to_identity(
        &self,
        store: &Arc<dyn Database>,
        conversation_id: Uuid,
        identity: &ResolvedIdentity,
    ) -> Result<bool, Error> {
        store
            .conversation_belongs_to_identity(
                conversation_id,
                &identity.principal_id,
                &identity.actor_id,
                identity.conversation_scope_id,
                crate::identity::to_history_conversation_kind(identity.conversation_kind),
            )
            .await
            .map_err(Error::from)
    }

    pub(in crate::agent) async fn ensure_persisted_conversation(
        &self,
        thread_id: Uuid,
        message: &IncomingMessage,
        identity: &ResolvedIdentity,
    ) -> Result<Option<Arc<dyn Database>>, Error> {
        let Some(store) = self.store().map(Arc::clone) else {
            return Ok(None);
        };
        store
            .ensure_conversation(
                thread_id,
                &message.channel,
                &identity.principal_id,
                message.thread_id.as_deref(),
            )
            .await?;
        store
            .update_conversation_identity(
                thread_id,
                Some(&identity.principal_id),
                Some(&identity.actor_id),
                Some(identity.conversation_scope_id),
                crate::identity::to_history_conversation_kind(identity.conversation_kind),
                Some(&identity.stable_external_conversation_key),
            )
            .await?;
        self.update_direct_conversation_metadata(&store, thread_id, message, identity)
            .await;
        if matches!(
            identity.conversation_kind,
            crate::identity::ConversationKind::Direct
        ) && let Some(workspace) = self.workspace().cloned()
        {
            let user_timezone = workspace
                .effective_timezone_for_identity(identity)
                .await
                .name()
                .to_string();
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
        Ok(Some(store))
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

    async fn primary_direct_conversation_id(
        &self,
        identity: &ResolvedIdentity,
    ) -> Result<Option<Uuid>, Error> {
        if !matches!(
            identity.conversation_kind,
            crate::identity::ConversationKind::Direct
        ) {
            return Ok(None);
        }

        let Some(store) = self.store().map(Arc::clone) else {
            return Ok(None);
        };
        let summaries = store
            .list_actor_conversations_for_recall(
                &identity.principal_id,
                &identity.actor_id,
                false,
                50,
            )
            .await?;

        if summaries.is_empty() {
            return Ok(None);
        }

        let mut fallback = None;
        for summary in summaries {
            fallback.get_or_insert(summary.id);
            let Some(metadata) = store.get_conversation_metadata(summary.id).await? else {
                continue;
            };
            if direct_conversation_candidate_is_primary(&metadata, summary.thread_type.as_deref()) {
                return Ok(Some(summary.id));
            }
        }

        Ok(fallback)
    }

    pub(in crate::agent) async fn maybe_hydrate_primary_direct_thread(
        &self,
        message: &IncomingMessage,
    ) -> Result<(), Error> {
        if message.thread_id.is_some() {
            return Ok(());
        }

        let identity = message.resolved_identity();
        if !matches!(
            identity.conversation_kind,
            crate::identity::ConversationKind::Direct
        ) {
            return Ok(());
        }

        let Some(primary_thread_id) = self.primary_direct_conversation_id(&identity).await? else {
            return Ok(());
        };

        self.maybe_hydrate_thread(message, &primary_thread_id.to_string())
            .await?;

        if let Some(session) = self
            .session_manager
            .session_for_thread(primary_thread_id)
            .await
        {
            self.session_manager
                .register_direct_main_thread_for_scope(
                    crate::agent::session_manager::SessionManager::session_scope_for_identity(
                        &identity,
                    ),
                    primary_thread_id,
                    session,
                )
                .await;
        }
        Ok(())
    }

    /// Restore the durable thread addressed by the current ingress key.
    ///
    /// Web/Tauri surfaces normally carry ThinClaw UUIDs. Native channels use
    /// platform room/thread identifiers (or only a group scope), so parsing
    /// the external key as a UUID loses continuity after every restart. Resolve
    /// those keys through the identity-scoped conversation index, hydrate the
    /// durable UUID, then bind the native alias in the in-memory router.
    pub(in crate::agent) async fn maybe_hydrate_ingress_thread(
        &self,
        message: &IncomingMessage,
    ) -> Result<(), Error> {
        let identity = message.resolved_identity();
        let direct_main = identity.conversation_kind == crate::identity::ConversationKind::Direct
            && message.thread_id.as_deref().is_none_or(|thread_id| {
                thread_id
                    .eq_ignore_ascii_case(thinclaw_agent::session_manager::DIRECT_MAIN_THREAD_KEY)
            });
        if direct_main {
            return self.maybe_hydrate_primary_direct_thread(message).await;
        }

        let durable_thread_id = if let Some(thread_id) = message
            .thread_id
            .as_deref()
            .and_then(|thread_id| Uuid::parse_str(thread_id).ok())
        {
            Some(thread_id)
        } else if let Some(store) = self.store() {
            store
                .find_latest_conversation_for_ingress(
                    &identity.principal_id,
                    &identity.actor_id,
                    identity.conversation_scope_id,
                    crate::identity::to_history_conversation_kind(identity.conversation_kind),
                    &message.channel,
                    message.thread_id.as_deref(),
                )
                .await?
        } else {
            None
        };

        let Some(durable_thread_id) = durable_thread_id else {
            return Ok(());
        };
        self.maybe_hydrate_thread(message, &durable_thread_id.to_string())
            .await?;

        // `maybe_hydrate_thread` always registers the UUID alias. Native
        // ingress must additionally point its non-UUID key (or group scope
        // with no explicit thread id) at the restored thread.
        if let Some(session) = self
            .session_manager
            .session_for_thread(durable_thread_id)
            .await
        {
            self.session_manager
                .register_thread_alias_for_scope(
                    crate::agent::session_manager::SessionManager::session_scope_for_identity(
                        &identity,
                    ),
                    identity.conversation_kind,
                    &message.channel,
                    message.thread_id.as_deref(),
                    durable_thread_id,
                    session,
                )
                .await;
        }
        Ok(())
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
    ) -> Result<(), Error> {
        // Only hydrate UUID-shaped thread IDs (web gateway uses UUIDs)
        let thread_uuid = match Uuid::parse_str(external_thread_id) {
            Ok(id) => id,
            Err(_) => return Ok(()),
        };

        let identity = message.resolved_identity();
        // In-memory ownership is already scoped by the canonical identity.
        // Check it before consulting durable ACLs: a just-created empty thread
        // may not have reached persistence yet, and rejecting it here makes
        // `/new` immediately unusable on the next message.
        let session = self
            .session_manager
            .get_or_create_session_for_identity(&identity)
            .await;
        {
            let sess = session.lock().await;
            if sess.threads.contains_key(&thread_uuid) {
                return Ok(());
            }
        }

        let store = self.store().map(Arc::clone);
        if let Some(ref store) = store
            && !self
                .conversation_visible_to_identity(store, thread_uuid, &identity)
                .await?
        {
            tracing::warn!(
                thread = %thread_uuid,
                principal = %identity.principal_id,
                actor = %identity.actor_id,
                "Refusing to hydrate thread outside the caller's identity scope"
            );
            return Err(crate::error::JobError::NotFound { id: thread_uuid }.into());
        }

        // Load history from DB (may be empty for a newly created thread).
        let msg_count;

        let conversation_metadata = if let Some(ref store) = store {
            store.get_conversation_metadata(thread_uuid).await?
        } else {
            None
        };

        // Decode the runtime from the metadata already fetched above —
        // `load_thread_runtime` would refetch the identical row.
        let mut runtime: Option<crate::agent::ThreadRuntimeState> =
            match conversation_metadata.as_ref() {
                Some(metadata) => thinclaw_agent::thread_runtime::decode_thread_runtime(metadata)?,
                None => None,
            };

        let mut normalized_history_window = None;
        let db_messages = if let Some(ref store) = store {
            let hydration_limit =
                i64::try_from(self.config.max_context_messages.max(2)).unwrap_or(i64::MAX);
            let (load_start, load_count) = if let Some(active_count) = runtime
                .as_ref()
                .and_then(|runtime| runtime.active_message_row_count)
            {
                let active_count = active_count.max(0);
                let load_count = active_count.min(hydration_limit);
                let active_start = runtime
                    .as_ref()
                    .and_then(|runtime| runtime.active_message_start_row)
                    .unwrap_or(0)
                    .max(0);
                let load_start = active_start.saturating_add(active_count - load_count);
                (load_start, load_count)
            } else {
                // Legacy runtime has no explicit window. Load only the recent
                // tail so a very large historical thread cannot exhaust memory
                // during startup, then freeze that exact tail offset so the
                // next runtime snapshot cannot reinterpret it as old rows.
                let total = store.count_conversation_messages(thread_uuid).await?;
                let load_count = total.min(hydration_limit).max(0);
                let load_start = total.saturating_sub(load_count);
                (load_start, load_count)
            };

            let mut db_messages = store
                .list_conversation_messages_window(thread_uuid, load_start, load_count)
                .await?;

            // A bounded tail can start on an ordinary assistant row that cannot
            // be reconstructed without its user turn. Assistant-only startup
            // hook rows are independently replayable and must survive this
            // normalization. Move to the first replayable boundary so the next
            // snapshot cannot reinterpret a different database range.
            let leading_orphans = leading_unreplayable_rows(&db_messages);
            if leading_orphans > 0 {
                db_messages.drain(..leading_orphans);
            }

            let normalized_start = load_start.saturating_add(leading_orphans as i64);
            let normalized_count = i64::try_from(db_messages.len()).unwrap_or(i64::MAX);
            let existing_window = runtime.as_ref().and_then(|runtime| {
                Some((
                    runtime.active_message_start_row?,
                    runtime.active_message_row_count?,
                ))
            });
            if existing_window != Some((normalized_start, normalized_count)) {
                normalized_history_window = Some((normalized_start, normalized_count));
                if let Some(runtime) = runtime.as_mut() {
                    runtime.active_message_start_row = Some(normalized_start);
                    runtime.active_message_row_count = Some(normalized_count);
                    // Checkpoints can reference rows intentionally discarded by
                    // bounded hydration; restoring them would violate the new
                    // active-window invariant.
                    runtime.undo_checkpoints.clear();
                }
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

        // Persist a normalized window before publishing the hydrated thread to
        // live state. If this write fails, the caller receives an error without
        // leaving a partially admitted in-memory thread whose next request
        // would bypass the durable repair.
        if let (Some(store), Some((start_row, row_count))) =
            (store.as_ref(), normalized_history_window)
        {
            mutate_thread_runtime(store, thread_uuid, |runtime| {
                runtime.active_message_start_row = Some(start_row);
                runtime.active_message_row_count = Some(row_count);
                runtime.undo_checkpoints.clear();
            })
            .await?;
        }

        // Insert into session and register with session manager. Re-check after
        // the DB awaits so a concurrent hydrator can never be overwritten by
        // a stale snapshot.
        {
            let mut sess = session.lock().await;
            if sess.threads.contains_key(&thread_uuid) {
                return Ok(());
            }
            sess.threads.insert(thread_uuid, thread);
            sess.active_thread = Some(thread_uuid);
            sess.last_active_at = chrono::Utc::now();
        }

        let register_scope_id = match identity.conversation_kind {
            crate::identity::ConversationKind::Direct => {
                crate::agent::session_manager::SessionManager::session_scope_for_identity(&identity)
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::leading_unreplayable_rows;

    fn message(role: &str, metadata: serde_json::Value) -> crate::history::ConversationMessage {
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: role.to_string(),
            content: role.to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn bounded_hydration_keeps_replayable_assistant_only_startup_rows() {
        let rows = vec![
            message("assistant", serde_json::json!({})),
            message(
                "assistant",
                serde_json::json!({"synthetic_origin": "startup_hook"}),
            ),
            message("user", serde_json::json!({})),
        ];

        assert_eq!(leading_unreplayable_rows(&rows), 1);
    }

    #[test]
    fn bounded_hydration_drops_only_unreconstructable_prefix() {
        let rows = vec![
            message("assistant", serde_json::json!({})),
            message("tool", serde_json::json!({})),
            message("user", serde_json::json!({})),
        ];

        assert_eq!(leading_unreplayable_rows(&rows), 2);
    }
}
