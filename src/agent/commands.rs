//! System commands and job handlers for the agent.
//!
//! Extracted from `agent_loop.rs` to isolate the /help, /model, /status,
//! and other command processing from the core agent loop.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::checkpoint;
use crate::agent::command_catalog;
use crate::agent::personality::{available_personality_names, preview, resolve_personality};
use crate::agent::submission::SubmissionResult;
use crate::agent::{Agent, MessageIntent};
use crate::agent::{mutate_thread_runtime, session::Session};
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::error::Error;
use crate::llm::{ChatMessage, Reasoning};
use crate::tools::builtin::llm_tools::{ModelOverride, model_override_scope_key_from_metadata};
use crate::tui::skin::CliSkin;

/// Format a count with a suffix, using K/M abbreviations for large numbers.
fn format_count(n: u64, suffix: &str) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M {}", n as f64 / 1_000_000.0, suffix)
    } else if n >= 1_000 {
        format!("{:.1}K {}", n as f64 / 1_000.0, suffix)
    } else {
        format!("{} {}", n, suffix)
    }
}

fn format_checkpoint_age(timestamp: DateTime<Utc>) -> String {
    let age = Utc::now().signed_duration_since(timestamp);
    if age.num_seconds() < 60 {
        format!("{}s ago", age.num_seconds().max(0))
    } else if age.num_minutes() < 60 {
        format!("{}m ago", age.num_minutes())
    } else if age.num_hours() < 24 {
        format!("{}h ago", age.num_hours())
    } else {
        format!("{}d ago", age.num_days())
    }
}

fn rollback_usage() -> &'static str {
    "Usage:\n  /rollback list\n  /rollback diff <N>\n  /rollback <N> [file]"
}

fn agent_display_name(name: &str) -> &str {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "Assistant"
    } else {
        trimmed
    }
}

impl Agent {
    /// Handle job-related intents without turn tracking.
    pub(super) async fn handle_job_or_command(
        &self,
        intent: MessageIntent,
        message: &IncomingMessage,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        // Send thinking status for non-trivial operations
        if let MessageIntent::CreateJob { .. } = &intent {
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Thinking("Processing...".into()),
                    &message.metadata,
                )
                .await;
        }

        let response = match intent {
            MessageIntent::CreateJob {
                title,
                description,
                category,
            } => {
                self.handle_create_job(message, title, description, category)
                    .await?
            }
            MessageIntent::CheckJobStatus { job_id } => {
                self.handle_check_status(&message.user_id, job_id).await?
            }
            MessageIntent::CancelJob { job_id } => {
                self.handle_cancel_job(&message.user_id, &job_id).await?
            }
            MessageIntent::ListJobs { filter } => {
                self.handle_list_jobs(&message.user_id, filter).await?
            }
            MessageIntent::HelpJob { job_id } => {
                self.handle_help_job(&message.user_id, &job_id).await?
            }
            MessageIntent::Command { command, args } => {
                match self
                    .handle_command(message, thread_id, &command, &args)
                    .await?
                {
                    Some(s) => s,
                    None => return Ok(SubmissionResult::Ok { message: None }), // Shutdown signal
                }
            }
            _ => "Unknown intent".to_string(),
        };
        Ok(SubmissionResult::response(response))
    }

    async fn handle_create_job(
        &self,
        message: &IncomingMessage,
        title: String,
        description: String,
        category: Option<String>,
    ) -> Result<String, Error> {
        let identity = message.resolved_identity();
        let job_id = self
            .scheduler
            .dispatch_job_for_identity(
                &identity.principal_id,
                &identity.actor_id,
                &title,
                &description,
                None,
            )
            .await?;

        // Set the dedicated category field (not stored in metadata)
        if let Some(cat) = category
            && let Err(e) = self
                .context_manager
                .update_context(job_id, |ctx| {
                    ctx.category = Some(cat);
                })
                .await
        {
            tracing::warn!(job_id = %job_id, "Failed to set job category: {}", e);
        }

        Ok(format!(
            "Created job: {}\nID: {}\n\nThe job has been scheduled and is now running.",
            title, job_id
        ))
    }

    async fn handle_check_status(
        &self,
        user_id: &str,
        job_id: Option<String>,
    ) -> Result<String, Error> {
        match job_id {
            Some(id) => {
                let uuid = Uuid::parse_str(&id)
                    .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

                let ctx = self.context_manager.get_context(uuid).await?;
                if ctx.user_id != user_id {
                    return Err(crate::error::JobError::NotFound { id: uuid }.into());
                }

                Ok(format!(
                    "Job: {}\nStatus: {:?}\nCreated: {}\nStarted: {}\nActual cost: {}",
                    ctx.title,
                    ctx.state,
                    ctx.created_at.format("%Y-%m-%d %H:%M:%S"),
                    ctx.started_at
                        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "Not started".to_string()),
                    ctx.actual_cost
                ))
            }
            None => {
                // Show summary of all jobs
                let summary = self.context_manager.summary_for(user_id).await;
                Ok(format!(
                    "Jobs summary:\n  Total: {}\n  In Progress: {}\n  Completed: {}\n  Failed: {}\n  Stuck: {}",
                    summary.total,
                    summary.in_progress,
                    summary.completed,
                    summary.failed,
                    summary.stuck
                ))
            }
        }
    }

    async fn handle_cancel_job(&self, user_id: &str, job_id: &str) -> Result<String, Error> {
        let uuid = Uuid::parse_str(job_id)
            .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

        let ctx = self.context_manager.get_context(uuid).await?;
        if ctx.user_id != user_id {
            return Err(crate::error::JobError::NotFound { id: uuid }.into());
        }

        self.scheduler.stop(uuid).await?;

        Ok(format!("Job {} has been cancelled.", job_id))
    }

    async fn handle_list_jobs(
        &self,
        user_id: &str,
        _filter: Option<String>,
    ) -> Result<String, Error> {
        let jobs = self.context_manager.all_jobs_for(user_id).await;

        if jobs.is_empty() {
            return Ok("No jobs found.".to_string());
        }

        let mut output = String::from("Jobs:\n");
        for job_id in jobs {
            if let Ok(ctx) = self.context_manager.get_context(job_id).await
                && ctx.user_id == user_id
            {
                output.push_str(&format!("  {} - {} ({:?})\n", job_id, ctx.title, ctx.state));
            }
        }

        Ok(output)
    }

    async fn handle_help_job(&self, user_id: &str, job_id: &str) -> Result<String, Error> {
        let uuid = Uuid::parse_str(job_id)
            .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

        let ctx = self.context_manager.get_context(uuid).await?;
        if ctx.user_id != user_id {
            return Err(crate::error::JobError::NotFound { id: uuid }.into());
        }

        if ctx.state == crate::context::JobState::Stuck {
            // Attempt recovery
            self.context_manager
                .update_context(uuid, |ctx| ctx.attempt_recovery())
                .await?
                .map_err(|s| crate::error::JobError::ContextError {
                    id: uuid,
                    reason: s,
                })?;

            // Reschedule
            self.scheduler.schedule(uuid).await?;

            Ok(format!(
                "Job {} was stuck. Attempting recovery (attempt #{}).",
                job_id,
                ctx.repair_attempts + 1
            ))
        } else {
            Ok(format!(
                "Job {} is not stuck (current state: {:?}). No help needed.",
                job_id, ctx.state
            ))
        }
    }

    /// Trigger a manual heartbeat check.
    pub(super) async fn process_heartbeat(&self) -> Result<SubmissionResult, Error> {
        let Some(workspace) = self.workspace() else {
            return Ok(SubmissionResult::error(
                "Heartbeat requires a workspace (database must be connected).",
            ));
        };

        let heartbeat_cfg = self.heartbeat_config.clone().unwrap_or_default();
        let runtime_heartbeat = crate::agent::HeartbeatConfig {
            interval: std::time::Duration::from_secs(heartbeat_cfg.interval_secs),
            enabled: heartbeat_cfg.enabled,
            max_failures: 3,
            notify_user_id: heartbeat_cfg.notify_user.clone(),
            notify_channel: heartbeat_cfg.notify_channel.clone(),
        };
        let hygiene_cfg = self
            .hygiene_config
            .clone()
            .unwrap_or_default()
            .to_workspace_config();

        let mut runner = crate::agent::HeartbeatRunner::new(
            runtime_heartbeat,
            hygiene_cfg,
            workspace.clone(),
            self.llm().clone(),
            self.safety().clone(),
        );
        if let Some(ref tracker) = self.deps.cost_tracker {
            runner = runner.with_cost_tracker(std::sync::Arc::clone(tracker));
        }

        match runner.check_heartbeat().await {
            crate::agent::HeartbeatResult::Ok => Ok(SubmissionResult::ok_with_message(
                "Heartbeat: all clear, nothing needs attention.",
            )),
            crate::agent::HeartbeatResult::NeedsAttention(msg) => Ok(SubmissionResult::response(
                format!("Heartbeat findings:\n\n{}", msg),
            )),
            crate::agent::HeartbeatResult::Skipped => Ok(SubmissionResult::ok_with_message(
                "Heartbeat skipped: no HEARTBEAT.md checklist found in workspace.",
            )),
            crate::agent::HeartbeatResult::Failed(err) => Ok(SubmissionResult::error(format!(
                "Heartbeat failed: {}",
                err
            ))),
        }
    }

    /// Summarize the current thread's conversation.
    pub(super) async fn process_summarize(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let messages = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.messages()
        };

        if messages.is_empty() {
            return Ok(SubmissionResult::ok_with_message(
                "Nothing to summarize (empty thread).",
            ));
        }

        // Build a summary prompt with the conversation
        let mut context = Vec::new();
        context.push(ChatMessage::system(
            "Summarize the conversation so far in 3-5 concise bullet points. \
             Focus on decisions made, actions taken, and key outcomes. \
             Be brief and factual.",
        ));
        // Include the conversation messages (truncate to last 20 to avoid context overflow)
        let start = if messages.len() > 20 {
            messages.len() - 20
        } else {
            0
        };
        context.extend_from_slice(&messages[start..]);
        context.push(ChatMessage::user("Summarize this conversation."));

        let request = crate::llm::CompletionRequest::new(context)
            .with_max_tokens(512)
            .with_temperature(0.3);

        let mut reasoning = Reasoning::new(self.llm().clone(), self.safety().clone());
        if let Some(ref tracker) = self.deps.cost_tracker {
            reasoning = reasoning.with_cost_tracker(std::sync::Arc::clone(tracker));
        }
        match reasoning.complete(request).await {
            Ok((text, _usage)) => Ok(SubmissionResult::response(format!(
                "Thread Summary:\n\n{}",
                text.trim()
            ))),
            Err(e) => Ok(SubmissionResult::error(format!("Summarize failed: {}", e))),
        }
    }

    /// Suggest next steps based on the current thread.
    pub(super) async fn process_suggest(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let messages = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.messages()
        };

        if messages.is_empty() {
            return Ok(SubmissionResult::ok_with_message(
                "Nothing to suggest from (empty thread).",
            ));
        }

        let mut context = Vec::new();
        context.push(ChatMessage::system(
            "Based on the conversation so far, suggest 2-4 concrete next steps the user could take. \
             Be actionable and specific. Format as a numbered list.",
        ));
        let start = if messages.len() > 20 {
            messages.len() - 20
        } else {
            0
        };
        context.extend_from_slice(&messages[start..]);
        context.push(ChatMessage::user("What should I do next?"));

        let request = crate::llm::CompletionRequest::new(context)
            .with_max_tokens(512)
            .with_temperature(0.5);

        let mut reasoning = Reasoning::new(self.llm().clone(), self.safety().clone());
        if let Some(ref tracker) = self.deps.cost_tracker {
            reasoning = reasoning.with_cost_tracker(std::sync::Arc::clone(tracker));
        }
        match reasoning.complete(request).await {
            Ok((text, _usage)) => Ok(SubmissionResult::response(format!(
                "Suggested Next Steps:\n\n{}",
                text.trim()
            ))),
            Err(e) => Ok(SubmissionResult::error(format!("Suggest failed: {}", e))),
        }
    }

    /// Handle system commands that bypass thread-state checks entirely.
    pub(super) async fn handle_system_command(
        &self,
        message: &IncomingMessage,
        thread_id: Uuid,
        command: &str,
        args: &[String],
    ) -> Result<SubmissionResult, Error> {
        match command {
            "help" => Ok(SubmissionResult::response(
                command_catalog::agent_help_text(),
            )),

            "ping" => Ok(SubmissionResult::response("pong!")),

            "version" => Ok(SubmissionResult::response(format!(
                "{} v{}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            ))),

            "rollback" => Ok(SubmissionResult::response(
                self.handle_rollback_command(thread_id, args).await,
            )),

            "identity" => {
                let Some(session) = self.session_manager.session_for_thread(thread_id).await else {
                    return Ok(SubmissionResult::error(
                        "Could not find the active session for this thread.",
                    ));
                };
                let session = session.lock().await;
                let session_personality = session
                    .active_personality
                    .as_ref()
                    .map(|personality| personality.name.as_str())
                    .unwrap_or("base identity");
                let (soul_pack, soul_schema, soul_summary) =
                    match crate::identity::soul_store::read_home_soul() {
                        Ok(content) => (
                            crate::identity::soul::canonical_seed_pack(&content)
                                .unwrap_or_else(|| self.config.personality_pack.clone()),
                            crate::identity::soul::canonical_schema_version(&content).to_string(),
                            crate::identity::soul::summarize_canonical_soul(&content),
                        ),
                        Err(_) => (
                            self.config.personality_pack.clone(),
                            "missing".to_string(),
                            "Canonical home soul not found yet".to_string(),
                        ),
                    };
                let local_overlay = if let Some(workspace) = self.workspace() {
                    workspace
                        .exists(crate::workspace::paths::SOUL_LOCAL)
                        .await
                        .ok()
                } else {
                    None
                };
                let soul_mode = if local_overlay == Some(true) {
                    "global + local overlay"
                } else {
                    "global only"
                };
                Ok(SubmissionResult::response(format!(
                    "Identity\n\nName: {}\nBase personality pack: {}\nCanonical soul path: {}\nSoul schema: {}\nSoul summary: {}\nWorkspace soul mode: {}\nWorkspace overlay: {}\nSession personality: {}\nConfigured CLI/Web skin: {}\n\nUse /personality <name> for a temporary overlay.\nAvailable overlays: {}",
                    agent_display_name(&self.config.name),
                    soul_pack,
                    crate::identity::soul_store::canonical_soul_path().display(),
                    soul_schema,
                    soul_summary,
                    soul_mode,
                    if local_overlay == Some(true) {
                        "SOUL.local.md present"
                    } else {
                        "Using global soul"
                    },
                    session_personality,
                    self.config.cli_skin,
                    available_personality_names().collect::<Vec<_>>().join(", ")
                )))
            }

            "personality" | "vibe" => {
                let Some(session) = self.session_manager.session_for_thread(thread_id).await else {
                    return Ok(SubmissionResult::error(
                        "Could not find the active session for this thread.",
                    ));
                };
                let mut session = session.lock().await;
                if args.is_empty() {
                    return Ok(SubmissionResult::response(
                        match session.active_personality.as_ref() {
                            Some(personality) => {
                                format!(
                                    "Current session personality: {}\n\n{}",
                                    personality.name,
                                    preview(personality)
                                )
                            }
                            None => format!(
                                "Current session personality: base identity\n\nAvailable personalities: {}",
                                available_personality_names().collect::<Vec<_>>().join(", ")
                            ),
                        },
                    ));
                }

                if args.len() == 1 && args[0].eq_ignore_ascii_case("reset") {
                    session.active_personality = None;
                    return Ok(SubmissionResult::ok_with_message(
                        "Session personality cleared. Back to your base identity.",
                    ));
                }

                let requested = args.join(" ");
                let personality = resolve_personality(&requested);
                let preview_text = preview(&personality).into_owned();
                let personality_name = personality.name.clone();
                session.active_personality = Some(personality);
                Ok(SubmissionResult::response(format!(
                    "Session personality set: {}\n\n{}",
                    personality_name, preview_text
                )))
            }

            "memory" => Ok(SubmissionResult::response(format!(
                "Memory & Growth\n\nWorkspace memory: {}\nCore tools: memory_search, memory_read, memory_write, memory_tree, session_search\nLearning tools: learning_status, learning_outcomes, learning_history, learning_feedback, learning_proposal_review, prompt_manage\nShared commands: /compress, /summarize, /skills, /heartbeat\nWebUI surfaces: Memory & Growth, Skills, Learning Ledger\n\nUse /skills to inspect installed skills and the WebUI tabs to browse durable memory and learning history.",
                if self.workspace().is_some() {
                    "available"
                } else {
                    "unavailable until a workspace/database is attached"
                }
            ))),

            "skin" => {
                let available = CliSkin::available_names().join(", ");
                if args.is_empty() || args[0].eq_ignore_ascii_case("current") {
                    Ok(SubmissionResult::response(format!(
                        "Current CLI skin: {}\nAvailable skins: {}\n\nUse /skin <name> in your local CLI client to switch immediately.",
                        self.config.cli_skin, available
                    )))
                } else if args[0].eq_ignore_ascii_case("list") {
                    Ok(SubmissionResult::response(format!(
                        "Available skins: {}\n\nUse /skin <name> in your local CLI client to switch immediately.",
                        available
                    )))
                } else if args[0].eq_ignore_ascii_case("reset") {
                    Ok(SubmissionResult::response(format!(
                        "Local clients can reset to their configured default skin. This agent is currently configured for '{}'.",
                        self.config.cli_skin
                    )))
                } else {
                    let requested = args.join(" ");
                    Ok(SubmissionResult::response(format!(
                        "Skin '{}' is available as a local client preset. Current configured skin: {}\nAvailable skins: {}",
                        requested, self.config.cli_skin, available
                    )))
                }
            }

            "tools" => {
                let tools = self.tools().list().await;
                Ok(SubmissionResult::response(format!(
                    "Available tools: {}",
                    tools.join(", ")
                )))
            }

            "debug" => {
                // Toggle debug mode on the originating channel.
                // For WASM channels (Telegram, Slack, etc.), this controls
                // whether tool-level status events are forwarded as messages.
                let channel_name = &message.channel;
                let new_state = self.channels.toggle_debug_mode(channel_name).await;
                let label = if new_state { "on" } else { "off" };
                Ok(SubmissionResult::ok_with_message(format!(
                    "Debug mode {label}. Tool call details will {}be shown.",
                    if new_state { "" } else { "not " }
                )))
            }

            "skills" => {
                if args.first().map(|s| s.as_str()) == Some("search") {
                    let query = args[1..].join(" ");
                    if query.is_empty() {
                        return Ok(SubmissionResult::error("Usage: /skills search <query>"));
                    }
                    self.handle_skills_search(&query).await
                } else if args.is_empty() {
                    self.handle_skills_list().await
                } else {
                    Ok(SubmissionResult::error(
                        "Usage: /skills or /skills search <query>",
                    ))
                }
            }

            "model" => {
                let current = self.llm().active_model_name();

                if args.is_empty() {
                    // Show current model and list available models
                    let mut out = format!("Active model: {}\n", current);
                    match self.llm().list_models().await {
                        Ok(models) if !models.is_empty() => {
                            out.push_str("\nAvailable models:\n");
                            for m in &models {
                                let marker = if *m == current { " (active)" } else { "" };
                                out.push_str(&format!("  {}{}\n", m, marker));
                            }
                            out.push_str("\nUse /model <name> to switch.");
                        }
                        Ok(_) => {
                            out.push_str(
                                "\nCould not fetch model list. Use /model <name> to switch.",
                            );
                        }
                        Err(e) => {
                            out.push_str(&format!(
                                "\nCould not fetch models: {}. Use /model <name> to switch.",
                                e
                            ));
                        }
                    }
                    Ok(SubmissionResult::response(out))
                } else {
                    let requested = &args[0];
                    let identity = message.resolved_identity();
                    let scope_key = model_override_scope_key_from_metadata(
                        &message.metadata,
                        Some(identity.principal_id.as_str()),
                        Some(identity.actor_id.as_str()),
                    );

                    if requested.eq_ignore_ascii_case("reset")
                        && let Some(ref override_lock) = self.deps.model_override
                    {
                        override_lock.clear(&scope_key).await;
                        if let Some(store) = self.store() {
                            let _ = mutate_thread_runtime(store, thread_id, |runtime| {
                                runtime.model_override = None;
                            })
                            .await;
                        }
                        return Ok(SubmissionResult::response(
                            "Switched back to the default routed model.".to_string(),
                        ));
                    }

                    // Validate the model exists
                    match self.llm().list_models().await {
                        Ok(models) if !models.is_empty() => {
                            if !models.iter().any(|m| m == requested) {
                                return Ok(SubmissionResult::error(format!(
                                    "Unknown model: {}. Available models:\n  {}",
                                    requested,
                                    models.join("\n  ")
                                )));
                            }
                        }
                        Ok(_) => {
                            // Empty model list, can't validate but try anyway
                        }
                        Err(e) => {
                            tracing::warn!("Could not fetch model list for validation: {}", e);
                        }
                    }

                    if !requested.contains('/') {
                        return Ok(SubmissionResult::error(
                            "Use /model <provider/model> or /model reset. Example: /model openai/gpt-4o"
                                .to_string(),
                        ));
                    }

                    if let Some(ref override_lock) = self.deps.model_override {
                        let override_value = ModelOverride {
                            model_spec: requested.to_string(),
                            reason: Some("manual /model command".to_string()),
                        };
                        override_lock.set(scope_key, override_value.clone()).await;
                        if let Some(store) = self.store() {
                            let _ = mutate_thread_runtime(store, thread_id, |runtime| {
                                runtime.model_override = Some(override_value.clone());
                            })
                            .await;
                        }
                        Ok(SubmissionResult::response(format!(
                            "Switched model for this conversation to: {}",
                            requested
                        )))
                    } else {
                        match self.llm().set_model(requested) {
                            Ok(()) => Ok(SubmissionResult::response(format!(
                                "Switched model to: {}",
                                requested
                            ))),
                            Err(e) => Ok(SubmissionResult::error(format!(
                                "Failed to switch model: {}",
                                e
                            ))),
                        }
                    }
                }
            }

            "status" => {
                let model = self.llm().active_model_name();
                let workspace_mode = &self.config.workspace_mode;
                Ok(SubmissionResult::response(format!(
                    "Agent status\n\
                     ──────────────────────\n\
                     ✅ Reachable\n\
                     Model:     {model}\n\
                     Workspace: {workspace_mode}",
                )))
            }

            "context" => {
                let detail = args.first().map(|s| s.as_str()) == Some("detail");
                let ws = self.workspace();

                let mut sections = Vec::new();

                // Always-present sections
                sections.push(("Safety guardrails", true, String::new()));
                sections.push(("Tool list", true, {
                    let tools = self.tools().list().await;
                    format!("{} tools: {}", tools.len(), tools.join(", "))
                }));

                // Workspace sections (identity files)
                if let Some(workspace) = ws {
                    let paths = [
                        ("SOUL.md (home)", "SOUL.md (home)"),
                        (crate::workspace::paths::AGENTS, "AGENTS.md"),
                        (crate::workspace::paths::SOUL_LOCAL, "SOUL.local.md"),
                        (crate::workspace::paths::USER, "USER.md"),
                        (crate::workspace::paths::IDENTITY, "IDENTITY.md"),
                        (crate::workspace::paths::MEMORY, "MEMORY.md"),
                        (crate::workspace::paths::HEARTBEAT, "HEARTBEAT.md"),
                        (crate::workspace::paths::BOOT, "BOOT.md"),
                    ];
                    for (path, label) in paths {
                        if path == "SOUL.md (home)" {
                            match crate::identity::soul_store::read_home_soul() {
                                Ok(content) if !content.is_empty() => {
                                    let preview = if detail {
                                        content
                                    } else {
                                        let first_line = content.lines().next().unwrap_or("");
                                        format!("{} ({} chars)", first_line, content.len())
                                    };
                                    sections.push((label, true, preview));
                                }
                                Ok(_) => sections.push((label, false, "(empty)".to_string())),
                                Err(_) => sections.push((label, false, "(not found)".to_string())),
                            }
                            continue;
                        }
                        match workspace.read(path).await {
                            Ok(doc) if !doc.content.is_empty() => {
                                let preview = if detail {
                                    doc.content.clone()
                                } else {
                                    let first_line = doc.content.lines().next().unwrap_or("");
                                    format!("{} ({} chars)", first_line, doc.content.len())
                                };
                                sections.push((label, true, preview));
                            }
                            Ok(_) => sections.push((label, false, "(empty)".to_string())),
                            Err(_) => sections.push((label, false, "(not found)".to_string())),
                        }
                    }
                } else {
                    sections.push(("Workspace", false, "(no workspace connected)".to_string()));
                }

                let mut out = String::from("Context sources\n──────────────────────\n");
                for (label, active, preview) in sections {
                    let icon = if active { "✅" } else { "❌" };
                    if detail && !preview.is_empty() {
                        out.push_str(&format!("\n{} {}\n{}\n", icon, label, preview));
                    } else {
                        out.push_str(&format!("{} {}  {}\n", icon, label, preview));
                    }
                }
                Ok(SubmissionResult::response(out))
            }

            _ => Ok(SubmissionResult::error(format!(
                "Unknown command: {}. Try /help",
                command
            ))),
        }
    }

    /// List installed skills.
    async fn handle_skills_list(&self) -> Result<SubmissionResult, Error> {
        let Some(registry) = self.skill_registry() else {
            return Ok(SubmissionResult::error("Skills system not enabled."));
        };

        let guard = registry.read().await;

        let skills = guard.skills();
        if skills.is_empty() {
            return Ok(SubmissionResult::response(
                "No skills installed.\n\nUse /skills search <query> to find skills on ClawHub.",
            ));
        }

        let mut out = String::from("Installed skills:\n\n");
        for s in skills {
            let desc = if s.manifest.description.chars().count() > 60 {
                let truncated: String = s.manifest.description.chars().take(57).collect();
                format!("{}...", truncated)
            } else {
                s.manifest.description.clone()
            };
            out.push_str(&format!(
                "  {:<24} v{:<10} [{}]  {}\n",
                s.manifest.name, s.manifest.version, s.trust, desc,
            ));
        }
        out.push_str("\nUse /skills search <query> to find more on ClawHub.");

        Ok(SubmissionResult::response(out))
    }

    /// Search ClawHub for skills.
    async fn handle_skills_search(&self, query: &str) -> Result<SubmissionResult, Error> {
        let catalog = match self.skill_catalog() {
            Some(c) => c,
            None => {
                return Ok(SubmissionResult::error("Skill catalog not available."));
            }
        };

        let outcome = catalog.search(query).await;

        // Enrich top results with detail data (stars, downloads, owner)
        let mut entries = outcome.results;
        catalog.enrich_search_results(&mut entries, 5).await;

        let mut out = format!("ClawHub results for \"{}\":\n\n", query);

        if entries.is_empty() {
            if let Some(ref err) = outcome.error {
                out.push_str(&format!("  (registry error: {})\n", err));
            } else {
                out.push_str("  No results found.\n");
            }
        } else {
            for entry in &entries {
                let owner_str = entry
                    .owner
                    .as_deref()
                    .map(|o| format!("  by {}", o))
                    .unwrap_or_default();

                let stats_parts: Vec<String> = [
                    entry.stars.map(|s| format!("{} stars", s)),
                    entry.downloads.map(|d| format_count(d, "downloads")),
                ]
                .into_iter()
                .flatten()
                .collect();
                let stats_str = if stats_parts.is_empty() {
                    String::new()
                } else {
                    format!("  {}", stats_parts.join("  "))
                };

                out.push_str(&format!(
                    "  {:<24} v{:<10}{}{}\n",
                    entry.name, entry.version, owner_str, stats_str,
                ));
                if !entry.description.is_empty() {
                    out.push_str(&format!("    {}\n\n", entry.description));
                }
            }
        }

        // Show matching installed skills
        if let Some(registry) = self.skill_registry() {
            let guard = registry.read().await;
            let query_lower = query.to_lowercase();
            let matches: Vec<_> = guard
                .skills()
                .iter()
                .filter(|s| {
                    s.manifest.name.to_lowercase().contains(&query_lower)
                        || s.manifest.description.to_lowercase().contains(&query_lower)
                })
                .collect();

            if !matches.is_empty() {
                out.push_str(&format!("Installed skills matching \"{}\":\n", query));
                for s in &matches {
                    out.push_str(&format!(
                        "  {:<24} v{:<10} [{}]\n",
                        s.manifest.name, s.manifest.version, s.trust,
                    ));
                }
            }
        }

        Ok(SubmissionResult::response(out))
    }

    async fn handle_rollback_command(&self, thread_id: Uuid, args: &[String]) -> String {
        if !self.config.checkpoints_enabled {
            return "Filesystem checkpoints are disabled in settings.".to_string();
        }

        let fallback_root = self
            .config
            .workspace_root
            .clone()
            .or_else(|| std::env::current_dir().ok());
        let Some(project_root) =
            checkpoint::resolve_thread_root(&thread_id.to_string(), fallback_root.as_deref())
        else {
            return "Could not resolve the current project root for rollback.".to_string();
        };

        let thread_scope = thread_id.to_string();

        if args.is_empty() || args[0].eq_ignore_ascii_case("help") {
            return format!(
                "{}\n\nActive project: {}",
                rollback_usage(),
                project_root.display()
            );
        }

        match args[0].as_str() {
            "list" => {
                let entries = match checkpoint::list_checkpoints(&project_root).await {
                    Ok(entries) => entries,
                    Err(e) => return format!("Error listing checkpoints: {}", e),
                };
                if entries.is_empty() {
                    return format!(
                        "No filesystem checkpoints found for {}.",
                        project_root.display()
                    );
                }

                let mut out = format!("Filesystem checkpoints for {}:\n", project_root.display());
                for (idx, entry) in entries.iter().enumerate() {
                    out.push_str(&format!(
                        "  {}. {}  {}  {}\n",
                        idx + 1,
                        &entry.commit_hash[..entry.commit_hash.len().min(12)],
                        format_checkpoint_age(entry.timestamp),
                        entry.summary
                    ));
                }
                out
            }
            "diff" => {
                let Some(raw_index) = args.get(1) else {
                    return rollback_usage().to_string();
                };
                if args.len() != 2 {
                    return format!(
                        "{}\n\n`/rollback diff <N>` does not take a file path.",
                        rollback_usage()
                    );
                }
                let index = match raw_index.parse::<usize>().ok().filter(|n| *n > 0) {
                    Some(index) => index,
                    None => {
                        return "Rollback index must be a positive integer.".to_string();
                    }
                };
                let entries = match checkpoint::list_checkpoints(&project_root).await {
                    Ok(entries) => entries,
                    Err(e) => return format!("Error listing checkpoints: {}", e),
                };
                let Some(entry) = entries.get(index - 1) else {
                    return format!(
                        "Checkpoint {} not found. Run `/rollback list` to inspect available checkpoints.",
                        index
                    );
                };
                let diff = match checkpoint::diff(&project_root, &entry.commit_hash).await {
                    Ok(diff) => diff,
                    Err(e) => return format!("Error computing diff: {}", e),
                };
                if diff.trim().is_empty() {
                    format!(
                        "No differences between checkpoint {} and the current project state.",
                        index
                    )
                } else {
                    format!(
                        "Diff for checkpoint {} ({})\n\n{}",
                        index,
                        &entry.commit_hash[..entry.commit_hash.len().min(12)],
                        diff.trim_end()
                    )
                }
            }
            _ => {
                let index = match args[0].parse::<usize>().ok().filter(|n| *n > 0) {
                    Some(index) => index,
                    None => return "Rollback index must be a positive integer.".to_string(),
                };

                let file = if args.len() > 1 {
                    Some(args[1..].join(" "))
                } else {
                    None
                };

                let entries = match checkpoint::list_checkpoints(&project_root).await {
                    Ok(entries) => entries,
                    Err(e) => return format!("Error listing checkpoints: {}", e),
                };
                let Some(entry) = entries.get(index - 1) else {
                    return format!(
                        "Checkpoint {} not found. Run `/rollback list` to inspect available checkpoints.",
                        index
                    );
                };

                if let Err(e) = checkpoint::restore_with_scope(
                    &thread_scope,
                    &project_root,
                    &entry.commit_hash,
                    file.as_deref(),
                )
                .await
                {
                    return format!("Error restoring checkpoint: {}", e);
                }

                match file {
                    Some(file) => format!(
                        "Restored {} from checkpoint {} ({})",
                        file,
                        index,
                        &entry.commit_hash[..entry.commit_hash.len().min(12)]
                    ),
                    None => format!(
                        "Restored project state from checkpoint {} ({})",
                        index,
                        &entry.commit_hash[..entry.commit_hash.len().min(12)]
                    ),
                }
            }
        }
    }

    /// Handle legacy command routing from the Router (job commands that go through
    /// process_user_input -> router -> handle_job_or_command -> here).
    pub(super) async fn handle_command(
        &self,
        message: &IncomingMessage,
        thread_id: Uuid,
        command: &str,
        args: &[String],
    ) -> Result<Option<String>, Error> {
        // System commands are now handled directly via Submission::SystemCommand,
        // but the router may still send us unknown /commands.
        match self
            .handle_system_command(message, thread_id, command, args)
            .await?
        {
            SubmissionResult::Response { content } => Ok(Some(content)),
            SubmissionResult::Ok { message } => Ok(message),
            SubmissionResult::Error { message } => Ok(Some(format!("Error: {}", message))),
            _ => Ok(None),
        }
    }
}
