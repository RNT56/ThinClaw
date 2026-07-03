//! agent_loop: startup-hook methods of `Agent`.

use super::*;

impl Agent {
    /// Execute startup hooks: BOOT.md after bootstrap completion, and
    /// BOOTSTRAP.md while bootstrap remains pending.
    ///
    /// Each hook is read from the workspace, processed as a synthetic user
    /// message, and the response is sent to the user's preferred notification
    /// channel. Errors are logged but never prevent the agent from starting.
    pub(crate) async fn run_startup_hooks(&self) {
        let workspace = match self.workspace() {
            Some(ws) => ws,
            None => {
                tracing::debug!("No workspace configured — skipping startup hooks");
                return;
            }
        };
        let workspace_user_id = workspace.user_id().to_string();

        let target_channel = self.config.notify_channel.as_deref().unwrap_or("web");

        // Resolve the notification recipient. For channels like Telegram,
        // this must be a numeric chat ID (e.g. the owner_id), not the
        // literal string "default" which Telegram::broadcast() silently drops.
        // We use the same resolution chain as heartbeat notifications.
        let notify_user = self
            .heartbeat_config
            .as_ref()
            .and_then(|hb| hb.notify_user.as_deref())
            .unwrap_or("default");
        let gateway_target = self.gateway_startup_hook_target(&workspace_user_id).await;

        let bootstrap_doc = match workspace.read(crate::workspace::paths::BOOTSTRAP).await {
            Ok(doc) => Some(doc),
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => None,
            Err(e) => {
                tracing::warn!(
                    "Failed to read BOOTSTRAP.md: {} — skipping bootstrap hook",
                    e
                );
                None
            }
        };
        let bootstrap_pending = bootstrap_doc
            .as_ref()
            .is_some_and(|doc| !crate::agent::heartbeat::is_effectively_empty(&doc.content));

        // ── 1. BOOT.md — runs on every startup after bootstrap completes ──
        if bootstrap_pending {
            tracing::debug!("BOOTSTRAP.md is still active — deferring BOOT.md startup hook");
        } else {
            match workspace.read(crate::workspace::paths::BOOT).await {
                Ok(doc) => {
                    if !crate::agent::heartbeat::is_effectively_empty(&doc.content) {
                        tracing::info!(
                            "Executing BOOT.md startup hook (target channel: {})",
                            target_channel,
                        );

                        // Pre-read workspace documents that BOOT.md references so the
                        // LLM always has this context, even if it skips tool calls.
                        let mut context_sections = Vec::new();

                        let today = workspace.local_today().format("%Y-%m-%d").to_string();
                        let ctx_docs = [
                            ("HEARTBEAT.md", "HEARTBEAT.md"),
                            ("MEMORY.md", "MEMORY.md"),
                            (
                                &format!("daily/{}.md", today),
                                &format!("daily/{}.md", today),
                            ),
                        ];
                        for (path, label) in &ctx_docs {
                            match workspace.read(path).await {
                                Ok(d) if !d.content.trim().is_empty() => {
                                    context_sections
                                        .push(format!("--- {} ---\n{}", label, d.content));
                                }
                                _ => {} // Missing or empty — skip silently
                            }
                        }

                        let enriched_content = if context_sections.is_empty() {
                            doc.content.clone()
                        } else {
                            format!(
                                "{}\n\n## Pre-loaded context\n\nThe following workspace documents were pre-read for you. \
                                 You do NOT need to call memory_read for these — the data is already here.\n\n{}",
                                doc.content,
                                context_sections.join("\n\n")
                            )
                        };

                        self.run_startup_hook(
                            "boot",
                            &enriched_content,
                            target_channel,
                            notify_user,
                            telegram_startup_thread_id("boot", target_channel, bootstrap_pending),
                            gateway_target.as_ref(),
                        )
                        .await;
                    } else {
                        tracing::debug!("BOOT.md is empty/template-only — skipping");
                    }
                }
                Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => {
                    tracing::debug!("No BOOT.md found — skipping boot hook");
                }
                Err(e) => {
                    tracing::warn!("Failed to read BOOT.md: {} — skipping boot hook", e);
                }
            }
        }

        // ── 2. BOOTSTRAP.md — runs while bootstrap is pending ──────────
        match bootstrap_doc {
            Some(doc) => {
                if bootstrap_pending {
                    tracing::info!(
                        "Executing BOOTSTRAP.md pending-bootstrap hook (target channel: {})",
                        target_channel,
                    );
                    self.run_startup_hook(
                        "bootstrap",
                        &doc.content,
                        target_channel,
                        notify_user,
                        telegram_startup_thread_id("bootstrap", target_channel, bootstrap_pending),
                        gateway_target.as_ref(),
                    )
                    .await;
                } else {
                    tracing::debug!("BOOTSTRAP.md is empty/template-only — skipping");
                }
            }
            None => {
                tracing::debug!(
                    "No BOOTSTRAP.md found — bootstrap completed, manually removed, or not configured"
                );
            }
        }
    }

    /// Execute a single startup hook by creating a synthetic message and
    /// routing the response to the target channel.
    pub(crate) async fn run_startup_hook(
        &self,
        hook_name: &str,
        content: &str,
        target_channel: &str,
        notify_user: &str,
        broadcast_thread_id: Option<&str>,
        gateway_target: Option<&GatewayStartupThreadTarget>,
    ) {
        // Build a synthetic IncomingMessage from the hook content.
        // The channel is set to the hook name (e.g. "boot", "bootstrap")
        // so handle_message can identify the source. The user_id is the
        // resolved notification recipient (e.g. Telegram owner_id).
        let message = IncomingMessage::new(hook_name, notify_user, content).with_metadata(
            serde_json::json!({
                "synthetic_origin": "startup_hook",
                "startup_hook": hook_name,
                "hide_user_input_from_webui_chat": true,
            }),
        );

        match self.handle_message(&message, None).await {
            Ok(Some(response)) if !response.is_empty() => {
                let web_thread_synced = if let Some(target) = gateway_target {
                    self.sync_startup_hook_to_gateway_assistant(
                        target,
                        hook_name,
                        content,
                        &response.content,
                    )
                    .await
                } else {
                    false
                };

                // Send the response to the user's preferred notification channel.
                let out = match broadcast_thread_id {
                    Some(thread_id) => OutgoingResponse::text(&response.content)
                        .with_attachments(response.attachments.clone())
                        .in_thread(thread_id),
                    None => OutgoingResponse::text(&response.content)
                        .with_attachments(response.attachments.clone()),
                };
                if target_channel == "web" {
                    if !web_thread_synced {
                        let _ = self
                            .channels
                            .broadcast("web", notify_user, out.clone())
                            .await;
                    }
                } else if let Err(e) = self
                    .channels
                    .broadcast(target_channel, notify_user, out.clone())
                    .await
                {
                    tracing::warn!(
                        "Failed to send {} hook response to '{}': {}{}",
                        hook_name,
                        target_channel,
                        e,
                        if web_thread_synced {
                            " — WebUI assistant thread already synced"
                        } else {
                            " — falling back to web"
                        }
                    );
                    if !web_thread_synced {
                        let _ = self.channels.broadcast("web", notify_user, out).await;
                    }
                } else {
                    tracing::info!("Sent {} hook response to '{}'", hook_name, target_channel,);
                }
            }
            Ok(Some(_empty)) => {
                tracing::debug!(
                    "{} hook returned empty response — nothing to send",
                    hook_name
                );
            }
            Ok(None) => {
                tracing::debug!("{} hook returned None — nothing to send", hook_name);
            }
            Err(e) => {
                tracing::error!(
                    "Error executing {} startup hook: {} — agent will continue normally",
                    hook_name,
                    e
                );
            }
        }
    }

    pub(crate) async fn gateway_startup_hook_target(
        &self,
        fallback_user_id: &str,
    ) -> Option<GatewayStartupThreadTarget> {
        let store = self.store().map(Arc::clone)?;
        let gateway_diagnostics = self.channels.channel_diagnostics("gateway").await;
        let (principal_id, actor_id) = heartbeat_routine_owner_for_gateway(
            &store,
            gateway_diagnostics.as_ref(),
            fallback_user_id,
        )
        .await;
        let thread_id =
            crate::channels::web::identity_helpers::get_or_create_gateway_assistant_conversation(
                store.as_ref(),
                &principal_id,
                &actor_id,
            )
            .await
            .ok()?;

        Some(GatewayStartupThreadTarget {
            principal_id,
            actor_id,
            thread_id,
        })
    }

    /// Mirror a startup hook turn into the pinned WebUI Assistant thread.
    ///
    /// The startup hook still runs as a background synthetic message, but we
    /// also persist the hidden prompt + assistant reply into the gateway
    /// assistant conversation, keep any loaded in-memory thread in sync, and
    /// emit a thread-scoped SSE response so open browser tabs update live.
    pub(crate) async fn sync_startup_hook_to_gateway_assistant(
        &self,
        target: &GatewayStartupThreadTarget,
        hook_name: &str,
        prompt: &str,
        response: &str,
    ) -> bool {
        let Some(store) = self.store().map(Arc::clone) else {
            return false;
        };

        let thread_id = target.thread_id;
        let thread_id_string = thread_id.to_string();
        let prompt_metadata = serde_json::json!({
            "synthetic_origin": "startup_hook",
            "startup_hook": hook_name,
            "hide_user_input_from_webui_chat": true,
        });
        let response_metadata = serde_json::json!({
            "synthetic_origin": "startup_hook",
            "startup_hook": hook_name,
        });

        if let Err(error) = store
            .add_conversation_message_with_attribution(
                thread_id,
                "user",
                prompt,
                None,
                None,
                None,
                Some(&prompt_metadata),
            )
            .await
        {
            tracing::warn!(
                thread = %thread_id,
                hook = hook_name,
                %error,
                "Failed to persist hidden startup hook prompt to gateway assistant thread"
            );
        }

        if let Err(error) = store
            .add_conversation_message_with_attribution(
                thread_id,
                "assistant",
                response,
                None,
                None,
                None,
                Some(&response_metadata),
            )
            .await
        {
            tracing::warn!(
                thread = %thread_id,
                hook = hook_name,
                %error,
                "Failed to persist startup hook response to gateway assistant thread"
            );
        }

        let identity = crate::channels::web::identity_helpers::gateway_identity(
            &target.principal_id,
            &target.actor_id,
            Some(&thread_id_string),
        );
        let sync_message = IncomingMessage::new("gateway", &target.principal_id, prompt)
            .with_thread(thread_id_string.clone())
            .with_identity(identity.clone())
            .with_metadata(serde_json::json!({
                "thread_id": thread_id_string,
                "synthetic_origin": "startup_hook",
                "startup_hook": hook_name,
                "hide_user_input_from_webui_chat": true,
            }));
        let session = self
            .session_manager
            .get_or_create_session_for_identity(&identity)
            .await;
        let (had_thread_loaded, previous_active_thread) = {
            let sess = session.lock().await;
            (sess.threads.contains_key(&thread_id), sess.active_thread)
        };

        if had_thread_loaded {
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                thread.start_turn_with_visibility(prompt, true);
                thread.complete_turn(response);
            }
        } else {
            self.maybe_hydrate_thread(&sync_message, &thread_id.to_string())
                .await;

            let mut sess = session.lock().await;
            if !sess.threads.contains_key(&thread_id) {
                let session_id = sess.id;
                let mut thread = crate::agent::session::Thread::with_id(thread_id, session_id);
                thread.start_turn_with_visibility(prompt, true);
                thread.complete_turn(response);
                sess.threads.insert(thread_id, thread);
            }
            if let Some(previous_active_thread) = previous_active_thread
                && previous_active_thread != thread_id
                && sess.threads.contains_key(&previous_active_thread)
            {
                sess.active_thread = Some(previous_active_thread);
            }
        }

        self.session_manager
            .register_direct_main_thread_for_scope(
                SessionManager::scope_id_for_user_id(&target.principal_id),
                thread_id,
                Arc::clone(&session),
            )
            .await;

        let web_response = OutgoingResponse::text(response).in_thread(thread_id.to_string());
        match self
            .channels
            .broadcast("web", &target.principal_id, web_response)
            .await
        {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    thread = %thread_id,
                    principal = %target.principal_id,
                    actor = %target.actor_id,
                    %error,
                    "Failed to broadcast startup hook response to WebUI assistant thread"
                );
                false
            }
        }
    }
}
