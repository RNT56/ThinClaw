//! Thread and session operations for the agent.
//!
//! Extracted from `agent_loop.rs` to isolate thread management (user input
//! processing, undo/redo, approval, auth, persistence) from the core loop.

use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::compaction::ContextCompactor;
use crate::agent::context_monitor::{ContextPressure, pressure_message, pressure_transition};
use crate::agent::dispatcher::{
    AgenticLoopResult, check_auth_required, execute_chat_tool_standalone, parse_auth_result,
};
use crate::agent::learning::{ImprovementClass, LearningEvent, LearningOrchestrator, RiskTier};
use crate::agent::session::{
    PendingApproval, PersistedSubagentState, Session, Thread, ThreadState,
};
use crate::agent::submission::SubmissionResult;
use crate::agent::{load_thread_runtime, mutate_thread_runtime};
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::context::JobContext;
use crate::db::Database;
use crate::error::Error;
use crate::history::ConversationKind as HistoryConversationKind;
use crate::identity::ResolvedIdentity;
use crate::llm::ChatMessage;

fn to_history_conversation_kind(
    kind: crate::identity::ConversationKind,
) -> HistoryConversationKind {
    match kind {
        crate::identity::ConversationKind::Direct => HistoryConversationKind::Direct,
        crate::identity::ConversationKind::Group => HistoryConversationKind::Group,
    }
}

fn detect_user_correction_signal(role: &str, content: &str) -> u32 {
    if !role.eq_ignore_ascii_case("user") {
        return 0;
    }
    let normalized = content.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return 0;
    }

    let correction_prefixes = [
        "actually",
        "correction:",
        "to clarify",
        "that's incorrect",
        "that is incorrect",
        "not quite",
        "no,",
        "no.",
        "use this instead",
        "please use",
        "instead:",
    ];
    if correction_prefixes
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
    {
        return 1;
    }

    let correction_markers = [
        "you should have",
        "please do not",
        "this is wrong",
        "the correct way is",
    ];
    if correction_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return 1;
    }

    0
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

        match store
            .conversation_belongs_to_actor(
                conversation_id,
                &identity.principal_id,
                &identity.actor_id,
            )
            .await
        {
            Ok(true) => true,
            Ok(false) if identity.actor_id == identity.principal_id => store
                .conversation_belongs_to_user(conversation_id, &identity.principal_id)
                .await
                .unwrap_or(false),
            Ok(false) => false,
            Err(err) => {
                tracing::warn!(
                    thread = %conversation_id,
                    error = %err,
                    "Failed to verify actor ownership while hydrating thread"
                );
                false
            }
        }
    }

    async fn ensure_persisted_conversation(
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
        Some(store)
    }

    fn compact_text_preview(text: &str) -> String {
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let preview: String = collapsed.chars().take(120).collect();
        if collapsed.chars().count() > 120 {
            format!("{}…", preview)
        } else {
            preview
        }
    }

    async fn best_effort_record_learning_event(
        &self,
        store: &Arc<dyn Database>,
        thread_id: Uuid,
        message: &IncomingMessage,
        identity: &ResolvedIdentity,
        role: &str,
        content: &str,
        persisted_message_id: Option<Uuid>,
    ) {
        let correction_count = detect_user_correction_signal(role, content);
        let class = if correction_count > 0 {
            ImprovementClass::Skill
        } else {
            ImprovementClass::Memory
        };
        let risk_tier = if correction_count > 0 {
            RiskTier::Medium
        } else {
            RiskTier::Low
        };
        let summary = if correction_count > 0 {
            "Persisted explicit user correction to conversation history".to_string()
        } else {
            format!("Persisted {} message to conversation history", role)
        };
        let target = if correction_count > 0 {
            "workflow_correction"
        } else {
            "conversation_history"
        };

        let job_id = message
            .metadata
            .get("job_id")
            .and_then(|v| v.as_str())
            .and_then(|value| Uuid::parse_str(value).ok());

        let learning_event = LearningEvent::new(
            format!("thread_ops::persist_{}_message", role),
            class,
            risk_tier,
            summary,
        )
        .with_target(target)
        .with_metadata(json!({
            "thread_id": thread_id.to_string(),
            "channel": message.channel.clone(),
            "role": role,
            "principal_id": identity.principal_id.clone(),
            "actor_id": identity.actor_id.clone(),
            "conversation_kind": identity.conversation_kind.as_str(),
            "message_id": message.id.to_string(),
            "content_length": content.len(),
            "content_preview": Self::compact_text_preview(content),
            "received_at": message.received_at.to_rfc3339(),
            "correction_count": correction_count,
            "repeated_failures": correction_count,
            "success": !(role.eq_ignore_ascii_case("user") && correction_count > 0),
        }));

        let persisted_event = learning_event.into_persisted(
            identity.principal_id.clone(),
            Some(identity.actor_id.clone()),
            Some(message.channel.clone()),
            Some(thread_id.to_string()),
            Some(thread_id),
            persisted_message_id,
            job_id,
        );

        let mut outcome_payload = serde_json::json!({});
        match store.insert_learning_event(&persisted_event).await {
            Ok(event_id) => {
                let orchestrator = LearningOrchestrator::new(
                    Arc::clone(store),
                    self.workspace().cloned(),
                    self.skill_registry().cloned(),
                );
                match orchestrator
                    .handle_event(
                        if role.eq_ignore_ascii_case("assistant") {
                            "assistant_turn_complete"
                        } else {
                            "user_turn_input"
                        },
                        &persisted_event,
                    )
                    .await
                {
                    Ok(outcome) => {
                        outcome_payload = serde_json::json!(outcome);
                    }
                    Err(err) => {
                        tracing::debug!(
                            thread = %thread_id,
                            event_id = %event_id,
                            error = %err,
                            "Learning orchestrator skipped event"
                        );
                    }
                }

                orchestrator
                    .export_turn_to_providers(
                        &identity.principal_id,
                        &serde_json::json!({
                            "event_id": event_id,
                            "conversation_id": thread_id,
                            "message_id": persisted_message_id,
                            "role": role,
                            "channel": message.channel.clone(),
                            "content_preview": Self::compact_text_preview(content),
                            "content_length": content.len(),
                            "received_at": message.received_at.to_rfc3339(),
                            "actor_id": identity.actor_id.clone(),
                            "conversation_kind": identity.conversation_kind.as_str(),
                        }),
                    )
                    .await;
            }
            Err(err) => {
                tracing::debug!(
                    thread = %thread_id,
                    error = %err,
                    "Best-effort learning event insert failed"
                );
            }
        }

        let payload = serde_json::to_value(&persisted_event).unwrap_or_else(|_| {
            json!({
                "id": persisted_event.id.to_string(),
                "source": persisted_event.source,
                "event_type": persisted_event.event_type,
                "payload": persisted_event.payload,
                "metadata": persisted_event.metadata,
                "created_at": persisted_event.created_at.to_rfc3339(),
            })
        });

        if let Some(job_id) = job_id
            && let Err(err) = store
                .save_job_event(job_id, "learning_event", &payload)
                .await
        {
            tracing::debug!(
                thread = %thread_id,
                job_id = %job_id,
                error = %err,
                "Best-effort learning event job write failed"
            );
        }

        let summary_payload = serde_json::json!({
            "event": payload,
            "outcome": outcome_payload,
        });
        if let Err(err) = store
            .update_conversation_metadata_field(thread_id, "learning_last_event", &summary_payload)
            .await
        {
            tracing::debug!(
                thread = %thread_id,
                error = %err,
                "Best-effort learning event conversation metadata write failed"
            );
        }
    }

    pub(super) async fn persist_thread_runtime_snapshot(
        &self,
        message: &IncomingMessage,
        session: &Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) {
        let (thread, auto_approved_tools) = {
            let sess = session.lock().await;
            (
                sess.threads.get(&thread_id).cloned(),
                Some(sess.auto_approved_tools.iter().cloned().collect::<Vec<_>>()),
            )
        };
        let Some(thread) = thread else {
            return;
        };
        self.persist_thread_runtime_with_thread(message, thread_id, &thread, auto_approved_tools)
            .await;
    }

    async fn persist_thread_runtime_with_thread(
        &self,
        message: &IncomingMessage,
        thread_id: Uuid,
        thread: &Thread,
        auto_approved_tools: Option<Vec<String>>,
    ) {
        let identity = message.resolved_identity();
        let Some(store) = self
            .ensure_persisted_conversation(thread_id, message, &identity)
            .await
        else {
            return;
        };

        let owner_agent_id = match self.session_manager.get_thread_owner(thread_id).await {
            Some(owner) => Some(owner),
            None => self.agent_router.get_thread_owner(thread_id).await,
        };
        let model_override = if let Some(ref overrides) = self.deps.model_override {
            overrides.get(&format!("thread:{thread_id}")).await
        } else {
            None
        };
        let existing_runtime = match load_thread_runtime(&store, thread_id).await {
            Ok(runtime) => runtime,
            Err(err) => {
                tracing::debug!(
                    thread = %thread_id,
                    error = %err,
                    "Failed to load thread runtime before snapshot; preserving defaults"
                );
                None
            }
        };

        if let Err(err) = mutate_thread_runtime(&store, thread_id, |runtime| {
            let active_subagents = runtime.active_subagents.clone();
            let preserved_auto_approved = runtime.auto_approved_tools.clone();
            *runtime = thread.runtime_state(
                owner_agent_id.clone(),
                model_override.clone(),
                auto_approved_tools
                    .clone()
                    .unwrap_or(preserved_auto_approved),
                active_subagents,
                existing_runtime
                    .as_ref()
                    .and_then(|runtime| runtime.last_context_pressure),
            );
        })
        .await
        {
            tracing::warn!(
                thread = %thread_id,
                error = %err,
                "Failed to persist thread runtime snapshot"
            );
        }
    }

    async fn record_context_pressure_state(
        &self,
        thread_id: Uuid,
        usage_percent: f64,
    ) -> Option<ContextPressure> {
        let current_pressure = self.context_monitor.check_pressure(usage_percent as f32);
        let store = self.store().map(Arc::clone)?;

        let previous_pressure = match load_thread_runtime(&store, thread_id).await {
            Ok(Some(runtime)) => runtime.last_context_pressure,
            Ok(None) => None,
            Err(err) => {
                tracing::debug!(
                    thread = %thread_id,
                    error = %err,
                    "Failed to load thread runtime for context pressure tracking"
                );
                None
            }
        };

        if previous_pressure == Some(current_pressure) {
            return Some(current_pressure);
        }

        let persisted_pressure = if current_pressure == ContextPressure::None {
            None
        } else {
            Some(current_pressure)
        };
        if let Err(err) = mutate_thread_runtime(&store, thread_id, |runtime| {
            runtime.last_context_pressure = persisted_pressure;
        })
        .await
        {
            tracing::debug!(
                thread = %thread_id,
                error = %err,
                "Failed to persist context pressure state"
            );
        }

        Some(current_pressure)
    }

    async fn sync_context_pressure_warning(
        &self,
        message: &IncomingMessage,
        thread_id: Uuid,
        usage_percent: f64,
    ) {
        let current_pressure = self.context_monitor.check_pressure(usage_percent as f32);
        let Some(store) = self.store().map(Arc::clone) else {
            return;
        };

        let previous_pressure = match load_thread_runtime(&store, thread_id).await {
            Ok(Some(runtime)) => runtime.last_context_pressure,
            Ok(None) => None,
            Err(err) => {
                tracing::debug!(
                    thread = %thread_id,
                    error = %err,
                    "Failed to load thread runtime for context pressure warning"
                );
                None
            }
        };

        let warning_level = pressure_transition(previous_pressure, current_pressure);
        if let Some(level) = warning_level
            && let Some(status) = pressure_message(level)
        {
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status(status),
                    &message.metadata,
                )
                .await;
        }

        let _ = self
            .record_context_pressure_state(thread_id, usage_percent)
            .await;
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
    pub(super) async fn maybe_hydrate_thread(
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
        let mut chat_messages: Vec<ChatMessage> = Vec::new();
        let msg_count;

        let runtime = if let Some(ref store) = store {
            let db_messages = store
                .list_conversation_messages(thread_uuid)
                .await
                .unwrap_or_default();
            msg_count = db_messages.len();
            chat_messages = db_messages
                .iter()
                .filter_map(|m| match m.role.as_str() {
                    "user" => Some(ChatMessage::user(&m.content)),
                    "assistant" => Some(ChatMessage::assistant(&m.content)),
                    _ => None,
                })
                .collect();
            load_thread_runtime(store, thread_uuid)
                .await
                .unwrap_or(None)
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
        if !chat_messages.is_empty() {
            thread.restore_from_messages(chat_messages);
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
            self.resume_persisted_subagents(
                message,
                &identity,
                thread_uuid,
                &runtime.active_subagents,
            )
            .await;
        }

        tracing::debug!(
            "Hydrated thread {} from DB ({} messages)",
            thread_uuid,
            msg_count
        );
    }

    pub(super) async fn process_user_input(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        content: &str,
    ) -> Result<SubmissionResult, Error> {
        // ── Media attachment handling ────────────────────────────────
        // Images/audio/video → multimodal: attached to ChatMessage for vision/audio LLMs
        // PDFs/documents/unknown → text extraction: prepended to the user content
        let (multimodal_attachments, text_extract_attachments): (Vec<_>, Vec<_>) =
            message.attachments.iter().cloned().partition(|a| {
                matches!(
                    a.media_type,
                    crate::media::MediaType::Image
                        | crate::media::MediaType::Audio
                        | crate::media::MediaType::Video
                )
            });

        let content = if !text_extract_attachments.is_empty() {
            let pipeline = crate::media::MediaPipeline::new();
            let mut media_context = String::new();
            for (idx, attachment) in text_extract_attachments.iter().enumerate() {
                match pipeline.extract(attachment) {
                    Ok(extracted) => {
                        if !media_context.is_empty() {
                            media_context.push_str("\n\n");
                        }
                        media_context.push_str(&extracted);
                        tracing::debug!(
                            attachment = idx,
                            media_type = %attachment.media_type,
                            size = attachment.size(),
                            "Extracted text from media attachment"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            attachment = idx,
                            media_type = %attachment.media_type,
                            error = %e,
                            "Failed to extract text from media attachment"
                        );
                    }
                }
            }

            if media_context.is_empty() {
                content.to_string()
            } else {
                format!("{}\n\n{}", media_context, content)
            }
        } else {
            content.to_string()
        };
        let content = content.as_str();

        if !multimodal_attachments.is_empty() {
            tracing::info!(
                attachment_count = multimodal_attachments.len(),
                total_bytes = multimodal_attachments.iter().map(|a| a.size()).sum::<usize>(),
                types = ?multimodal_attachments.iter().map(|a| a.media_type.to_string()).collect::<Vec<_>>(),
                "Routing media attachments to multimodal LLM processing"
            );
        }

        // First check thread state without holding lock during I/O
        let thread_state = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.state
        };

        // Check thread state
        match thread_state {
            ThreadState::Processing => {
                return Ok(SubmissionResult::error(
                    "Turn in progress. Use /interrupt to cancel.",
                ));
            }
            ThreadState::AwaitingApproval => {
                return Ok(SubmissionResult::error(
                    "Waiting for approval. Use /interrupt to cancel.",
                ));
            }
            ThreadState::Completed => {
                return Ok(SubmissionResult::error(
                    "Thread completed. Use /thread new.",
                ));
            }
            ThreadState::Idle | ThreadState::Interrupted => {
                // Can proceed
            }
        }

        // Safety validation for user input
        let validation = self.safety().validate_input(content);
        if !validation.is_valid {
            let details = validation
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Ok(SubmissionResult::error(format!(
                "Input rejected by safety validation: {}",
                details
            )));
        }

        let violations = self.safety().check_policy(content);
        if violations
            .iter()
            .any(|rule| rule.action == crate::safety::PolicyAction::Block)
        {
            return Ok(SubmissionResult::error("Input rejected by safety policy."));
        }

        // Handle explicit commands (starting with /) directly
        // Everything else goes through the normal agentic loop with tools
        let temp_message = IncomingMessage {
            content: content.to_string(),
            ..message.clone()
        };

        if let Some(intent) = self.router.route_command(&temp_message) {
            // Explicit command like /status, /job, /list - handle directly
            return self.handle_job_or_command(intent, message, thread_id).await;
        }

        // Reset the file checkpoint dedup bucket for this thread's new turn.
        crate::agent::checkpoint::new_turn(thread_id.to_string());

        // Natural language goes through the agentic loop
        // Job tools (create_job, list_jobs, etc.) are in the tool registry

        // Auto-compact if needed BEFORE adding new turn
        {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            let messages = thread.messages();
            if let Some(strategy) = self.context_monitor.suggest_compaction(&messages) {
                let pct = self.context_monitor.usage_percent(&messages);
                tracing::info!("Context at {:.1}% capacity, auto-compacting", pct);

                if let Some(store) = self.store().map(Arc::clone) {
                    let identity = message.resolved_identity();
                    let event = LearningEvent::new(
                        "thread_ops::pre_compaction_nudge",
                        ImprovementClass::Memory,
                        RiskTier::Low,
                        "Context nearing limit; compaction nudge emitted before turn",
                    )
                    .with_target("context_compaction")
                    .with_metadata(json!({
                        "thread_id": thread_id.to_string(),
                        "channel": message.channel,
                        "usage_percent": pct,
                        "strategy": format!("{:?}", strategy),
                    }))
                    .into_persisted(
                        identity.principal_id.clone(),
                        Some(identity.actor_id.clone()),
                        Some(message.channel.clone()),
                        Some(thread_id.to_string()),
                        Some(thread_id),
                        None,
                        None,
                    );

                    if store.insert_learning_event(&event).await.is_ok() {
                        let orchestrator = LearningOrchestrator::new(
                            store,
                            self.workspace().cloned(),
                            self.skill_registry().cloned(),
                        );
                        let _ = orchestrator
                            .handle_event("pre_compaction_memory_nudge", &event)
                            .await;
                    }
                }

                // Notify the user that compaction is happening
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status(format!(
                            "Context at {:.0}% capacity, compacting...",
                            pct
                        )),
                        &message.metadata,
                    )
                    .await;

                let mut compactor =
                    ContextCompactor::new(self.llm().clone(), self.safety().clone());
                if let Some(ref tracker) = self.deps.cost_tracker {
                    compactor = compactor.with_cost_tracker(std::sync::Arc::clone(tracker));
                }
                if let Err(e) = compactor
                    .compact(thread, strategy, self.workspace().map(|w| w.as_ref()))
                    .await
                {
                    tracing::warn!("Auto-compaction failed: {}", e);
                }
            }
        }

        // Create checkpoint before turn
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            let mut mgr = undo_mgr.lock().await;
            mgr.checkpoint(
                thread.turn_number(),
                thread.messages(),
                format!("Before turn {}", thread.turn_number()),
            );
        }

        // Start the turn and get messages
        let mut turn_messages = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.start_turn(content);
            thread.messages()
        };

        // Attach multimodal media to the last user message for LLM processing.
        // The rig adapter converts these to provider-native base64 content blocks.
        if !multimodal_attachments.is_empty()
            && let Some(last_user) = turn_messages
                .iter_mut()
                .rev()
                .find(|m| m.role == crate::llm::Role::User)
        {
            last_user.attachments = multimodal_attachments;
        }

        // Persist user message to DB immediately so it survives crashes
        self.persist_user_message(thread_id, message, content).await;
        self.persist_thread_runtime_snapshot(message, &session, thread_id)
            .await;

        // ── Lifecycle: start ─────────────────────────────────────────
        // Emit immediately — before compaction or LLM call — so the
        // frontend can show a thinking indicator right away.
        let run_id = uuid::Uuid::new_v4().to_string();
        let _ = self
            .channels
            .send_status(
                &message.channel,
                crate::channels::StatusUpdate::LifecycleStart {
                    run_id: run_id.clone(),
                },
                &message.metadata,
            )
            .await;

        // Send thinking status
        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::Thinking("Processing...".into()),
                &message.metadata,
            )
            .await;

        // Run the agentic tool execution loop
        let result = self
            .run_agentic_loop(message, session.clone(), thread_id, turn_messages)
            .await;

        // Re-acquire lock and check if interrupted
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        if thread.state == ThreadState::Interrupted {
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status("Interrupted".into()),
                    &message.metadata,
                )
                .await;
            // Lifecycle end: interrupted
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    crate::channels::StatusUpdate::LifecycleEnd {
                        run_id: run_id.clone(),
                        phase: "interrupted".to_string(),
                    },
                    &message.metadata,
                )
                .await;
            return Ok(SubmissionResult::Interrupted);
        }

        // Complete, fail, or request approval
        let was_streamed = matches!(&result, Ok(AgenticLoopResult::Streamed(_)));
        match result {
            Ok(AgenticLoopResult::Response(response))
            | Ok(AgenticLoopResult::Streamed(response)) => {
                // Hook: TransformResponse — allow hooks to modify or reject the final response
                let response = {
                    let event = crate::hooks::HookEvent::ResponseTransform {
                        user_id: message.user_id.clone(),
                        thread_id: thread_id.to_string(),
                        response: response.clone(),
                    };
                    match self.hooks().run(&event).await {
                        Err(crate::hooks::HookError::Rejected { reason }) => {
                            format!("[Response filtered: {}]", reason)
                        }
                        Err(err) => {
                            format!("[Response blocked by hook policy: {}]", err)
                        }
                        Ok(crate::hooks::HookOutcome::Continue {
                            modified: Some(new_response),
                        }) => new_response,
                        _ => response, // fail-open: use original
                    }
                };

                thread.complete_turn(&response);
                let usage_percent = self.context_monitor.usage_percent(&thread.messages());
                let thread_snapshot = thread.clone();
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Done".into()),
                        &message.metadata,
                    )
                    .await;

                // Persist assistant response (user message already persisted at turn start)
                self.persist_assistant_response(thread_id, message, &response)
                    .await;
                drop(sess);
                let _ = thread_snapshot;
                self.sync_context_pressure_warning(message, thread_id, usage_percent)
                    .await;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;

                // Lifecycle end: response
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        crate::channels::StatusUpdate::LifecycleEnd {
                            run_id,
                            phase: "response".to_string(),
                        },
                        &message.metadata,
                    )
                    .await;

                if was_streamed {
                    Ok(SubmissionResult::Streamed(response))
                } else {
                    Ok(SubmissionResult::response(response))
                }
            }
            Ok(AgenticLoopResult::NeedApproval { pending }) => {
                // Store pending approval in thread and update state
                let request_id = pending.request_id;
                let tool_name = pending.tool_name.clone();
                let description = pending.description.clone();
                let parameters = pending.parameters.clone();
                thread.await_approval(pending);
                let usage_percent = self.context_monitor.usage_percent(&thread.messages());
                let thread_snapshot = thread.clone();
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Awaiting approval".into()),
                        &message.metadata,
                    )
                    .await;
                drop(sess);
                let _ = thread_snapshot;
                self.sync_context_pressure_warning(message, thread_id, usage_percent)
                    .await;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
                Ok(SubmissionResult::NeedApproval {
                    request_id,
                    tool_name,
                    description,
                    parameters,
                })
            }
            Err(e) => {
                thread.fail_turn(e.to_string());
                let usage_percent = self.context_monitor.usage_percent(&thread.messages());
                let thread_snapshot = thread.clone();
                // User message already persisted at turn start; nothing else to save
                // Lifecycle end: error
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        crate::channels::StatusUpdate::LifecycleEnd {
                            run_id,
                            phase: "error".to_string(),
                        },
                        &message.metadata,
                    )
                    .await;
                drop(sess);
                let _ = thread_snapshot;
                self.sync_context_pressure_warning(message, thread_id, usage_percent)
                    .await;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
                Ok(SubmissionResult::error(e.to_string()))
            }
        }
    }

    /// Persist the user message to the DB at turn start (before the agentic loop).
    ///
    /// This ensures the user message is durable even if the process crashes
    /// mid-response. Call this right after `thread.start_turn()`.
    pub(super) async fn persist_user_message(
        &self,
        thread_id: Uuid,
        message: &IncomingMessage,
        user_input: &str,
    ) {
        let identity = message.resolved_identity();
        let Some(store) = self
            .ensure_persisted_conversation(thread_id, message, &identity)
            .await
        else {
            return;
        };

        let persisted_message_id = match store
            .add_conversation_message_with_attribution(
                thread_id,
                "user",
                user_input,
                Some(&identity.actor_id),
                message.user_name.as_deref(),
                Some(&identity.raw_sender_id),
                Some(&message.metadata),
            )
            .await
        {
            Ok(message_id) => Some(message_id),
            Err(e) => {
                tracing::warn!("Failed to persist user message: {}", e);
                return;
            }
        };

        self.best_effort_record_learning_event(
            &store,
            thread_id,
            message,
            &identity,
            "user",
            user_input,
            persisted_message_id,
        )
        .await;
    }

    /// Persist the assistant response to the DB after the agentic loop completes.
    ///
    /// Re-ensures the conversation row exists so that assistant responses are
    /// still persisted even if `persist_user_message` failed transiently at
    /// turn start (e.g. a brief DB blip that resolved before response time).
    pub(super) async fn persist_assistant_response(
        &self,
        thread_id: Uuid,
        message: &IncomingMessage,
        response: &str,
    ) {
        let identity = message.resolved_identity();
        let Some(store) = self
            .ensure_persisted_conversation(thread_id, message, &identity)
            .await
        else {
            return;
        };

        let persisted_message_id = match store
            .add_conversation_message_with_attribution(
                thread_id,
                "assistant",
                response,
                None,
                None,
                None,
                Some(&message.metadata),
            )
            .await
        {
            Ok(message_id) => Some(message_id),
            Err(e) => {
                tracing::warn!("Failed to persist assistant message: {}", e);
                return;
            }
        };

        self.best_effort_record_learning_event(
            &store,
            thread_id,
            message,
            &identity,
            "assistant",
            response,
            persisted_message_id,
        )
        .await;
    }

    pub(super) async fn process_undo(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut mgr = undo_mgr.lock().await;

        if !mgr.can_undo() {
            return Ok(SubmissionResult::ok_with_message("Nothing to undo."));
        }

        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        // Save current state to redo, get previous checkpoint
        let current_messages = thread.messages();
        let current_turn = thread.turn_number();

        if let Some(checkpoint) = mgr.undo(current_turn, current_messages) {
            // Extract values before consuming the reference
            let turn_number = checkpoint.turn_number;
            let messages = checkpoint.messages.clone();
            let undo_count = mgr.undo_count();
            // Restore thread from checkpoint
            thread.restore_from_messages(messages);
            let usage_percent = self.context_monitor.usage_percent(&thread.messages());
            drop(mgr);
            drop(sess);
            self.record_context_pressure_state(thread_id, usage_percent)
                .await;
            Ok(SubmissionResult::ok_with_message(format!(
                "Undone to turn {}. {} undo(s) remaining.",
                turn_number, undo_count
            )))
        } else {
            Ok(SubmissionResult::error("Undo failed."))
        }
    }

    pub(super) async fn process_redo(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut mgr = undo_mgr.lock().await;

        if !mgr.can_redo() {
            return Ok(SubmissionResult::ok_with_message("Nothing to redo."));
        }

        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        let current_messages = thread.messages();
        let current_turn = thread.turn_number();

        if let Some(checkpoint) = mgr.redo(current_turn, current_messages) {
            thread.restore_from_messages(checkpoint.messages);
            let usage_percent = self.context_monitor.usage_percent(&thread.messages());
            drop(mgr);
            drop(sess);
            self.record_context_pressure_state(thread_id, usage_percent)
                .await;
            Ok(SubmissionResult::ok_with_message(format!(
                "Redone to turn {}.",
                checkpoint.turn_number
            )))
        } else {
            Ok(SubmissionResult::error("Redo failed."))
        }
    }

    pub(super) async fn process_interrupt(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        match thread.state {
            ThreadState::Processing | ThreadState::AwaitingApproval => {
                thread.interrupt();
                let thread_snapshot = thread.clone();
                drop(sess);
                let _ = thread_snapshot;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
                Ok(SubmissionResult::ok_with_message("Interrupted."))
            }
            _ => Ok(SubmissionResult::ok_with_message("Nothing to interrupt.")),
        }
    }

    pub(super) async fn process_compact(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        let messages = thread.messages();
        let usage = self.context_monitor.usage_percent(&messages);
        let strategy = self
            .context_monitor
            .suggest_compaction(&messages)
            .unwrap_or(
                crate::agent::context_monitor::CompactionStrategy::Summarize { keep_recent: 5 },
            );

        let mut compactor = ContextCompactor::new(self.llm().clone(), self.safety().clone());
        if let Some(ref tracker) = self.deps.cost_tracker {
            compactor = compactor.with_cost_tracker(std::sync::Arc::clone(tracker));
        }
        match compactor
            .compact(thread, strategy, self.workspace().map(|w| w.as_ref()))
            .await
        {
            Ok(result) => {
                let usage_after = self.context_monitor.usage_percent(&thread.messages());
                let mut msg = format!(
                    "Compacted: {} turns removed, {} → {} tokens (was {:.1}% full)",
                    result.turns_removed, result.tokens_before, result.tokens_after, usage
                );
                if result.summary_written {
                    msg.push_str(", summary saved to workspace");
                }
                drop(sess);
                self.record_context_pressure_state(thread_id, usage_after)
                    .await;
                Ok(SubmissionResult::ok_with_message(msg))
            }
            Err(e) => Ok(SubmissionResult::error(format!("Compaction failed: {}", e))),
        }
    }

    pub(super) async fn process_clear(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
        thread.turns.clear();
        thread.state = ThreadState::Idle;
        let usage_percent = self.context_monitor.usage_percent(&thread.messages());

        // Clear undo history too
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        undo_mgr.lock().await.clear();
        drop(sess);
        self.record_context_pressure_state(thread_id, usage_percent)
            .await;

        Ok(SubmissionResult::ok_with_message("Thread cleared."))
    }

    /// Process an approval or rejection of a pending tool execution.
    pub(super) async fn process_approval(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        request_id: Option<Uuid>,
        approved: bool,
        always: bool,
    ) -> Result<SubmissionResult, Error> {
        // Get pending approval for this thread
        let pending = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            if thread.state != ThreadState::AwaitingApproval {
                return Ok(SubmissionResult::error("No pending approval request."));
            }

            thread.take_pending_approval()
        };

        let pending = match pending {
            Some(p) => p,
            None => return Ok(SubmissionResult::error("No pending approval request.")),
        };

        // Verify request ID if provided
        if let Some(req_id) = request_id
            && req_id != pending.request_id
        {
            // Put it back and return error
            let thread_snapshot = {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    thread.await_approval(pending);
                    Some(thread.clone())
                } else {
                    None
                }
            };
            if let Some(thread_snapshot) = thread_snapshot {
                let _ = thread_snapshot;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
            }
            return Ok(SubmissionResult::error(
                "Request ID mismatch. Use the correct request ID.",
            ));
        }

        if approved {
            // If always, add to auto-approved set
            if always {
                let mut sess = session.lock().await;
                sess.auto_approve_tool_for_channel(&message.channel, &pending.tool_name);
                tracing::info!(
                    "Auto-approved tool '{}' for session {}",
                    pending.tool_name,
                    sess.id
                );
            }

            // Reset thread state to processing
            let processing_snapshot = {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    thread.state = ThreadState::Processing;
                    Some(thread.clone())
                } else {
                    None
                }
            };
            if let Some(thread_snapshot) = processing_snapshot {
                let _ = thread_snapshot;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
            }

            // Execute the approved tool and continue the loop
            let identity = message.resolved_identity();
            let mut job_ctx = JobContext::with_identity(
                identity.principal_id.clone(),
                identity.actor_id.clone(),
                "chat",
                "Interactive chat session",
            );
            job_ctx.metadata = message.metadata.clone();
            if !job_ctx.metadata.is_object() {
                job_ctx.metadata = serde_json::json!({});
            }
            if let Some(metadata) = job_ctx.metadata.as_object_mut() {
                metadata.insert(
                    "thread_id".to_string(),
                    serde_json::json!(thread_id.to_string()),
                );
                metadata.insert(
                    "conversation_kind".to_string(),
                    serde_json::json!(identity.conversation_kind.as_str()),
                );
                metadata.insert(
                    "conversation_scope_id".to_string(),
                    serde_json::json!(identity.conversation_scope_id.to_string()),
                );
                metadata.insert(
                    "principal_id".to_string(),
                    serde_json::json!(identity.principal_id.clone()),
                );
                metadata.insert(
                    "actor_id".to_string(),
                    serde_json::json!(identity.actor_id.clone()),
                );
                if let Some(owner) = self.agent_router.get_thread_owner(thread_id).await {
                    metadata.insert("agent_id".to_string(), serde_json::json!(owner.clone()));
                    if let Some(agent) = self.agent_router.get_agent(&owner).await {
                        if let Some(workspace_id) = agent.workspace_id {
                            metadata.insert(
                                "agent_workspace_id".to_string(),
                                serde_json::json!(workspace_id.to_string()),
                            );
                        }
                        if let Some(allowed_tools) = agent.allowed_tools.as_ref() {
                            metadata.insert(
                                "allowed_tools".to_string(),
                                serde_json::json!(allowed_tools),
                            );
                        }
                        if let Some(allowed_skills) = agent.allowed_skills.as_ref() {
                            metadata.insert(
                                "allowed_skills".to_string(),
                                serde_json::json!(allowed_skills),
                            );
                        }
                    }
                }
            }

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::ToolStarted {
                        name: pending.tool_name.clone(),
                        parameters: Some(pending.parameters.clone()),
                    },
                    &message.metadata,
                )
                .await;

            let tool_result = self
                .execute_chat_tool(&pending.tool_name, &pending.parameters, &job_ctx)
                .await;

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::ToolCompleted {
                        name: pending.tool_name.clone(),
                        success: tool_result.is_ok(),
                        result_preview: tool_result
                            .as_ref()
                            .ok()
                            .map(|s| crate::agent::dispatcher::truncate_preview(s, 500)),
                    },
                    &message.metadata,
                )
                .await;

            if let Ok(ref output) = tool_result
                && !output.is_empty()
            {
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::ToolResult {
                            name: pending.tool_name.clone(),
                            preview: output.clone(),
                        },
                        &message.metadata,
                    )
                    .await;
            }

            // Build context including the tool result
            let mut context_messages = pending.context_messages;
            let deferred_tool_calls = pending.deferred_tool_calls;

            // Sanitize the restored snapshot before appending new results.
            // The snapshot was captured at approval time; if the hard history
            // cap had fired in that same iteration and orphaned any Tool
            // messages, those orphans would be frozen into the snapshot.
            // Sanitizing here ensures the context is clean before we append
            // the approved tool result and resume the agentic loop.
            crate::llm::sanitize_tool_messages(&mut context_messages);

            // Record result in thread
            {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id)
                    && let Some(turn) = thread.last_turn_mut()
                {
                    match &tool_result {
                        Ok(output) => {
                            turn.record_tool_result(serde_json::json!(output));
                        }
                        Err(e) => {
                            turn.record_tool_error(e.to_string());
                        }
                    }
                }
            }

            // If tool_auth returned awaiting_token, enter auth mode and
            // return instructions directly (skip agentic loop continuation).
            if let Some((ext_name, instructions)) =
                check_auth_required(&pending.tool_name, &tool_result)
            {
                self.handle_auth_intercept(
                    &session,
                    thread_id,
                    message,
                    &tool_result,
                    ext_name,
                    instructions.clone(),
                )
                .await;
                return Ok(SubmissionResult::response(instructions));
            }

            // Add tool result to context
            let result_content = match tool_result {
                Ok(output) => {
                    let sanitized = self
                        .safety()
                        .sanitize_tool_output(&pending.tool_name, &output);
                    self.safety().wrap_for_llm(
                        &pending.tool_name,
                        &sanitized.content,
                        sanitized.was_modified,
                    )
                }
                Err(e) => format!("Error: {}", e),
            };

            context_messages.push(ChatMessage::tool_result(
                &pending.tool_call_id,
                &pending.tool_name,
                result_content,
            ));

            // Replay deferred tool calls from the same assistant message so
            // every tool_use ID gets a matching tool_result before the next
            // LLM call.
            if !deferred_tool_calls.is_empty() {
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Thinking(format!(
                            "Executing {} deferred tool(s)...",
                            deferred_tool_calls.len()
                        )),
                        &message.metadata,
                    )
                    .await;
            }

            // === Phase 1: Preflight (sequential) ===
            // Walk deferred tools checking approval. Collect runnable
            // tools; stop at the first that needs approval.
            let mut runnable: Vec<crate::llm::ToolCall> = Vec::new();
            let mut approval_needed: Option<(
                usize,
                crate::llm::ToolCall,
                Arc<dyn crate::tools::Tool>,
            )> = None;

            for (idx, tc) in deferred_tool_calls.iter().enumerate() {
                if let Some(tool) = self.tools().get(&tc.name).await {
                    use crate::tools::ApprovalRequirement;
                    let needs_approval = match tool.requires_approval(&tc.arguments) {
                        ApprovalRequirement::Never => false,
                        ApprovalRequirement::UnlessAutoApproved => {
                            let sess = session.lock().await;
                            !sess.is_tool_auto_approved_for_channel(&message.channel, &tc.name)
                        }
                        ApprovalRequirement::Always => true,
                    };

                    if needs_approval {
                        approval_needed = Some((idx, tc.clone(), tool));
                        break; // remaining tools stay deferred
                    }
                }

                runnable.push(tc.clone());
            }

            // === Phase 2: Parallel execution ===
            let exec_results: Vec<(crate::llm::ToolCall, Result<String, Error>)> = if runnable.len()
                <= 1
            {
                // Single tool (or none): execute inline
                let mut results = Vec::new();
                for tc in &runnable {
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::ToolStarted {
                                name: tc.name.clone(),
                                parameters: Some(tc.arguments.clone()),
                            },
                            &message.metadata,
                        )
                        .await;

                    let result = self
                        .execute_chat_tool(&tc.name, &tc.arguments, &job_ctx)
                        .await;

                    let _ =
                        self.channels
                            .send_status(
                                &message.channel,
                                StatusUpdate::ToolCompleted {
                                    name: tc.name.clone(),
                                    success: result.is_ok(),
                                    result_preview: result.as_ref().ok().map(|s| {
                                        crate::agent::dispatcher::truncate_preview(s, 500)
                                    }),
                                },
                                &message.metadata,
                            )
                            .await;

                    results.push((tc.clone(), result));
                }
                results
            } else {
                // Multiple tools: execute in parallel via JoinSet
                let mut join_set = JoinSet::new();
                let runnable_count = runnable.len();

                for (spawn_idx, tc) in runnable.iter().enumerate() {
                    let tools = self.tools().clone();
                    let safety = self.safety().clone();
                    let channels = self.channels.clone();
                    let job_ctx = job_ctx.clone();
                    let tc = tc.clone();
                    let channel = message.channel.clone();
                    let metadata = message.metadata.clone();

                    join_set.spawn(async move {
                        let _ = channels
                            .send_status(
                                &channel,
                                StatusUpdate::ToolStarted {
                                    name: tc.name.clone(),
                                    parameters: Some(tc.arguments.clone()),
                                },
                                &metadata,
                            )
                            .await;

                        let result = execute_chat_tool_standalone(
                            &tools,
                            &safety,
                            &tc.name,
                            &tc.arguments,
                            &job_ctx,
                        )
                        .await;

                        let _ = channels
                            .send_status(
                                &channel,
                                StatusUpdate::ToolCompleted {
                                    name: tc.name.clone(),
                                    success: result.is_ok(),
                                    result_preview: result.as_ref().ok().map(|s| {
                                        crate::agent::dispatcher::truncate_preview(s, 500)
                                    }),
                                },
                                &metadata,
                            )
                            .await;

                        (spawn_idx, tc, result)
                    });
                }

                // Collect and reorder by original index
                let mut ordered: Vec<Option<(crate::llm::ToolCall, Result<String, Error>)>> =
                    (0..runnable_count).map(|_| None).collect();
                while let Some(join_result) = join_set.join_next().await {
                    match join_result {
                        Ok((idx, tc, result)) => {
                            ordered[idx] = Some((tc, result));
                        }
                        Err(e) => {
                            if e.is_panic() {
                                tracing::error!("Deferred tool execution task panicked: {}", e);
                            } else {
                                tracing::error!("Deferred tool execution task cancelled: {}", e);
                            }
                        }
                    }
                }

                // Fill panicked slots with error results
                ordered
                    .into_iter()
                    .enumerate()
                    .map(|(i, opt)| {
                        opt.unwrap_or_else(|| {
                            let tc = runnable[i].clone();
                            let err: Error = crate::error::ToolError::ExecutionFailed {
                                name: tc.name.clone(),
                                reason: "Task failed during execution".to_string(),
                            }
                            .into();
                            (tc, Err(err))
                        })
                    })
                    .collect()
            };

            // === Phase 3: Post-flight (sequential, in original order) ===
            // Process all results before any conditional return so every
            // tool result is recorded in the session audit trail.
            let mut deferred_auth: Option<String> = None;

            for (tc, deferred_result) in exec_results {
                if let Ok(ref output) = deferred_result
                    && !output.is_empty()
                {
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::ToolResult {
                                name: tc.name.clone(),
                                preview: output.clone(),
                            },
                            &message.metadata,
                        )
                        .await;
                }

                // Record in thread
                {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id)
                        && let Some(turn) = thread.last_turn_mut()
                    {
                        match &deferred_result {
                            Ok(output) => turn.record_tool_result(serde_json::json!(output)),
                            Err(e) => turn.record_tool_error(e.to_string()),
                        }
                    }
                }

                // Auth detection — defer return until all results are recorded
                if deferred_auth.is_none()
                    && let Some((ext_name, instructions)) =
                        check_auth_required(&tc.name, &deferred_result)
                {
                    self.handle_auth_intercept(
                        &session,
                        thread_id,
                        message,
                        &deferred_result,
                        ext_name,
                        instructions.clone(),
                    )
                    .await;
                    deferred_auth = Some(instructions);
                }

                let deferred_content = match deferred_result {
                    Ok(output) => {
                        let sanitized = self.safety().sanitize_tool_output(&tc.name, &output);
                        self.safety().wrap_for_llm(
                            &tc.name,
                            &sanitized.content,
                            sanitized.was_modified,
                        )
                    }
                    Err(e) => format!("Error: {}", e),
                };

                context_messages.push(ChatMessage::tool_result(&tc.id, &tc.name, deferred_content));
            }

            // Return auth response after all results are recorded
            if let Some(instructions) = deferred_auth {
                return Ok(SubmissionResult::response(instructions));
            }

            // Handle approval if a tool needed it
            if let Some((approval_idx, tc, tool)) = approval_needed {
                let new_pending = PendingApproval {
                    request_id: Uuid::new_v4(),
                    tool_name: tc.name.clone(),
                    parameters: tc.arguments.clone(),
                    description: tool.description().to_string(),
                    tool_call_id: tc.id.clone(),
                    context_messages: context_messages.clone(),
                    deferred_tool_calls: deferred_tool_calls[approval_idx + 1..].to_vec(),
                };

                let request_id = new_pending.request_id;
                let tool_name = new_pending.tool_name.clone();
                let description = new_pending.description.clone();
                let parameters = new_pending.parameters.clone();

                {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thread.await_approval(new_pending);
                    }
                }
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;

                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Awaiting approval".into()),
                        &message.metadata,
                    )
                    .await;

                return Ok(SubmissionResult::NeedApproval {
                    request_id,
                    tool_name,
                    description,
                    parameters,
                });
            }

            // Continue the agentic loop (a tool was already executed this turn)
            let result = self
                .run_agentic_loop(message, session.clone(), thread_id, context_messages)
                .await;

            // Handle the result
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            let was_streamed = matches!(&result, Ok(AgenticLoopResult::Streamed(_)));
            match result {
                Ok(AgenticLoopResult::Response(response))
                | Ok(AgenticLoopResult::Streamed(response)) => {
                    thread.complete_turn(&response);
                    let usage_percent = self.context_monitor.usage_percent(&thread.messages());
                    let thread_snapshot = thread.clone();
                    // User message already persisted at turn start; save assistant response
                    self.persist_assistant_response(thread_id, message, &response)
                        .await;
                    drop(sess);
                    let _ = thread_snapshot;
                    self.sync_context_pressure_warning(message, thread_id, usage_percent)
                        .await;
                    self.persist_thread_runtime_snapshot(message, &session, thread_id)
                        .await;
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::Status("Done".into()),
                            &message.metadata,
                        )
                        .await;
                    if was_streamed {
                        Ok(SubmissionResult::Streamed(response))
                    } else {
                        Ok(SubmissionResult::response(response))
                    }
                }
                Ok(AgenticLoopResult::NeedApproval {
                    pending: new_pending,
                }) => {
                    let request_id = new_pending.request_id;
                    let tool_name = new_pending.tool_name.clone();
                    let description = new_pending.description.clone();
                    let parameters = new_pending.parameters.clone();
                    thread.await_approval(new_pending);
                    let usage_percent = self.context_monitor.usage_percent(&thread.messages());
                    let thread_snapshot = thread.clone();
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::Status("Awaiting approval".into()),
                            &message.metadata,
                        )
                        .await;
                    drop(sess);
                    let _ = thread_snapshot;
                    self.sync_context_pressure_warning(message, thread_id, usage_percent)
                        .await;
                    self.persist_thread_runtime_snapshot(message, &session, thread_id)
                        .await;
                    Ok(SubmissionResult::NeedApproval {
                        request_id,
                        tool_name,
                        description,
                        parameters,
                    })
                }
                Err(e) => {
                    thread.fail_turn(e.to_string());
                    let usage_percent = self.context_monitor.usage_percent(&thread.messages());
                    let thread_snapshot = thread.clone();
                    // User message already persisted at turn start
                    drop(sess);
                    let _ = thread_snapshot;
                    self.sync_context_pressure_warning(message, thread_id, usage_percent)
                        .await;
                    self.persist_thread_runtime_snapshot(message, &session, thread_id)
                        .await;
                    Ok(SubmissionResult::error(e.to_string()))
                }
            }
        } else {
            // Rejected - complete the turn with a rejection message and persist
            let rejection = format!(
                "Tool '{}' was rejected. The agent will not execute this tool.\n\n\
                 You can continue the conversation or try a different approach.",
                pending.tool_name
            );
            {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    thread.clear_pending_approval();
                    thread.complete_turn(&rejection);
                    let usage_percent = self.context_monitor.usage_percent(&thread.messages());
                    let thread_snapshot = thread.clone();
                    // User message already persisted at turn start; save rejection response
                    self.persist_assistant_response(thread_id, message, &rejection)
                        .await;
                    drop(sess);
                    let _ = thread_snapshot;
                    self.sync_context_pressure_warning(message, thread_id, usage_percent)
                        .await;
                    self.persist_thread_runtime_snapshot(message, &session, thread_id)
                        .await;
                }
            }

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status("Rejected".into()),
                    &message.metadata,
                )
                .await;

            Ok(SubmissionResult::response(rejection))
        }
    }

    /// Handle an auth-required result from a tool execution.
    ///
    /// Enters auth mode on the thread, completes + persists the turn,
    /// and sends the AuthRequired status to the channel.
    /// Returns the instructions string for the caller to wrap in a response.
    async fn handle_auth_intercept(
        &self,
        session: &Arc<Mutex<Session>>,
        thread_id: Uuid,
        message: &IncomingMessage,
        tool_result: &Result<String, Error>,
        ext_name: String,
        instructions: String,
    ) {
        let auth_data = parse_auth_result(tool_result);
        let thread_snapshot = {
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                thread.enter_auth_mode(ext_name.clone());
                thread.complete_turn(&instructions);
                // User message already persisted at turn start; save auth instructions
                self.persist_assistant_response(thread_id, message, &instructions)
                    .await;
                Some(thread.clone())
            } else {
                None
            }
        };
        if let Some(thread_snapshot) = thread_snapshot {
            let _ = thread_snapshot;
            self.persist_thread_runtime_snapshot(message, session, thread_id)
                .await;
        }
        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::AuthRequired {
                    extension_name: ext_name,
                    instructions: Some(instructions.clone()),
                    auth_url: auth_data.auth_url,
                    setup_url: auth_data.setup_url,
                },
                &message.metadata,
            )
            .await;
    }

    /// Handle an auth token submitted while the thread is in auth mode.
    ///
    /// The token goes directly to the extension manager's credential store,
    /// completely bypassing logging, turn creation, history, and compaction.
    pub(super) async fn process_auth_token(
        &self,
        message: &IncomingMessage,
        pending: &crate::agent::session::PendingAuth,
        token: &str,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<Option<String>, Error> {
        let token = token.trim();

        // Clear auth mode regardless of outcome
        let cleared_snapshot = {
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                thread.pending_auth = None;
                Some(thread.clone())
            } else {
                None
            }
        };
        if let Some(thread_snapshot) = cleared_snapshot {
            let _ = thread_snapshot;
            self.persist_thread_runtime_snapshot(message, &session, thread_id)
                .await;
        }

        let ext_mgr = match self.deps.extension_manager.as_ref() {
            Some(mgr) => mgr,
            None => return Ok(Some("Extension manager not available.".to_string())),
        };

        match ext_mgr.auth(&pending.extension_name, Some(token)).await {
            Ok(result) if result.status == "authenticated" => {
                tracing::info!(
                    "Extension '{}' authenticated via auth mode",
                    pending.extension_name
                );

                // Auto-activate so tools are available immediately after auth
                match ext_mgr.activate(&pending.extension_name).await {
                    Ok(activate_result) => {
                        let tool_count = activate_result.tools_loaded.len();
                        let tool_list = if activate_result.tools_loaded.is_empty() {
                            String::new()
                        } else {
                            format!("\n\nTools: {}", activate_result.tools_loaded.join(", "))
                        };
                        let msg = format!(
                            "{} authenticated and activated ({} tools loaded).{}",
                            pending.extension_name, tool_count, tool_list
                        );
                        let _ = self
                            .channels
                            .send_status(
                                &message.channel,
                                StatusUpdate::AuthCompleted {
                                    extension_name: pending.extension_name.clone(),
                                    success: true,
                                    message: msg.clone(),
                                },
                                &message.metadata,
                            )
                            .await;
                        Ok(Some(msg))
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Extension '{}' authenticated but activation failed: {}",
                            pending.extension_name,
                            e
                        );
                        let msg = format!(
                            "{} authenticated successfully, but activation failed: {}. \
                             Try activating manually.",
                            pending.extension_name, e
                        );
                        let _ = self
                            .channels
                            .send_status(
                                &message.channel,
                                StatusUpdate::AuthCompleted {
                                    extension_name: pending.extension_name.clone(),
                                    success: true,
                                    message: msg.clone(),
                                },
                                &message.metadata,
                            )
                            .await;
                        Ok(Some(msg))
                    }
                }
            }
            Ok(result) => {
                // Invalid token, re-enter auth mode
                {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thread.enter_auth_mode(pending.extension_name.clone());
                    }
                }
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
                let msg = result
                    .instructions
                    .clone()
                    .unwrap_or_else(|| "Invalid token. Please try again.".to_string());
                // Re-emit AuthRequired so web UI re-shows the card
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::AuthRequired {
                            extension_name: pending.extension_name.clone(),
                            instructions: Some(msg.clone()),
                            auth_url: result.auth_url,
                            setup_url: result.setup_url,
                        },
                        &message.metadata,
                    )
                    .await;
                Ok(Some(msg))
            }
            Err(e) => {
                let msg = format!(
                    "Authentication failed for {}: {}",
                    pending.extension_name, e
                );
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::AuthCompleted {
                            extension_name: pending.extension_name.clone(),
                            success: false,
                            message: msg.clone(),
                        },
                        &message.metadata,
                    )
                    .await;
                Ok(Some(msg))
            }
        }
    }

    pub(super) async fn process_new_thread(
        &self,
        message: &IncomingMessage,
    ) -> Result<SubmissionResult, Error> {
        let identity = message.resolved_identity();
        let session = self
            .session_manager
            .get_or_create_session_for_identity(&identity)
            .await;
        let mut sess = session.lock().await;
        let thread = sess.create_thread();
        let thread_id = thread.id;
        Ok(SubmissionResult::ok_with_message(format!(
            "New thread: {}",
            thread_id
        )))
    }

    pub(super) async fn process_switch_thread(
        &self,
        message: &IncomingMessage,
        target_thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let identity = message.resolved_identity();
        let session = self
            .session_manager
            .get_or_create_session_for_identity(&identity)
            .await;
        let mut sess = session.lock().await;

        if sess.switch_thread(target_thread_id) {
            Ok(SubmissionResult::ok_with_message(format!(
                "Switched to thread {}",
                target_thread_id
            )))
        } else {
            Ok(SubmissionResult::error("Thread not found."))
        }
    }

    pub(super) async fn process_resume(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        checkpoint_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut mgr = undo_mgr.lock().await;

        if let Some(checkpoint) = mgr.restore(checkpoint_id) {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.restore_from_messages(checkpoint.messages);
            Ok(SubmissionResult::ok_with_message(format!(
                "Resumed from checkpoint: {}",
                checkpoint.description
            )))
        } else {
            Ok(SubmissionResult::error("Checkpoint not found."))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::detect_user_correction_signal;

    #[test]
    fn detects_correction_prefixes() {
        assert_eq!(
            detect_user_correction_signal("user", "Actually, please use this endpoint."),
            1
        );
        assert_eq!(
            detect_user_correction_signal("user", "No, that's incorrect."),
            1
        );
    }

    #[test]
    fn ignores_non_correction_messages() {
        assert_eq!(
            detect_user_correction_signal("user", "Can you summarize this for me?"),
            0
        );
        assert_eq!(
            detect_user_correction_signal("assistant", "Actually this is fine."),
            0
        );
    }
}
