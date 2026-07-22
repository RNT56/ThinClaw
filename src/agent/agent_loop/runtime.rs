use super::*;

impl Agent {
    pub(super) async fn drain_tail_tasks(&self) {
        let mut tasks = self.tail_tasks.lock().await;
        let graceful = async {
            while let Some(joined) = tasks.join_next().await {
                if let Err(error) = joined {
                    tracing::warn!(%error, "Post-turn task failed during shutdown");
                }
            }
        };
        if tokio::time::timeout(Self::SHUTDOWN_DRAIN_TIMEOUT, graceful)
            .await
            .is_ok()
        {
            return;
        }
        tracing::warn!(
            timeout_secs = Self::SHUTDOWN_DRAIN_TIMEOUT.as_secs(),
            "Post-turn tasks did not drain before shutdown timeout; aborting"
        );
        if tokio::time::timeout(Self::BACKGROUND_TASK_SHUTDOWN_TIMEOUT, tasks.shutdown())
            .await
            .is_err()
        {
            tracing::error!("Post-turn tasks did not join promptly after abort");
            tasks.abort_all();
        }
    }

    pub(super) async fn drain_external_submission_tasks(&self) {
        let mut tasks = self.external_submission_tasks.lock().await;
        let graceful = async {
            while let Some(joined) = tasks.join_next().await {
                if let Err(error) = joined {
                    tracing::warn!(%error, "External submission failed during shutdown");
                }
            }
        };
        if tokio::time::timeout(Self::SHUTDOWN_DRAIN_TIMEOUT, graceful)
            .await
            .is_ok()
        {
            return;
        }
        tracing::warn!(
            timeout_secs = Self::SHUTDOWN_DRAIN_TIMEOUT.as_secs(),
            "External submissions did not drain before shutdown timeout; aborting"
        );
        if tokio::time::timeout(Self::BACKGROUND_TASK_SHUTDOWN_TIMEOUT, tasks.shutdown())
            .await
            .is_err()
        {
            tracing::error!("External submissions did not join promptly after abort");
            tasks.abort_all();
        }
    }

    pub(super) async fn drain_or_abort_background_task(
        name: &'static str,
        mut handle: tokio::task::JoinHandle<()>,
        timeout: std::time::Duration,
        drained_reason: LoopStopReason,
    ) -> LoopStopReason {
        let sleep = tokio::time::sleep(timeout);
        tokio::pin!(sleep);

        tokio::select! {
            joined = &mut handle => {
                match joined {
                    Ok(()) => {
                        tracing::debug!(task = name, "background task drained on shutdown");
                        drained_reason
                    }
                    Err(error) if error.is_cancelled() => {
                        tracing::debug!(task = name, "background task was already cancelled");
                        LoopStopReason::Cancelled
                    }
                    Err(error) => {
                        tracing::warn!(task = name, error = %error, "background task failed while draining");
                        LoopStopReason::FatalError
                    }
                }
            }
            _ = &mut sleep => {
                tracing::warn!(
                    task = name,
                    timeout_secs = timeout.as_secs(),
                    "background task did not drain before timeout; aborting"
                );
                handle.abort();
                if let Err(error) = handle.await
                    && error.is_panic()
                {
                    tracing::error!(task = name, error = %error, "background task panicked during abort");
                    return LoopStopReason::FatalError;
                }
                LoopStopReason::Cancelled
            }
        }
    }

    /// Route one incoming message to its conversation's ordered worker.
    ///
    /// Messages within a conversation scope stay strictly ordered (one
    /// worker per scope, processing serially); different conversations run
    /// concurrently up to `MAIN_LOOP_MAX_CONCURRENT_TURNS`. Control
    /// submissions (/interrupt, /quit, /restart) bypass the queue entirely
    /// — an interrupt must reach a conversation whose worker is mid-turn,
    /// and quit must work while every worker is busy.
    pub(super) async fn dispatch_incoming_message(
        agent: &Arc<Agent>,
        workers: &Arc<
            Mutex<std::collections::HashMap<Uuid, tokio::sync::mpsc::Sender<IncomingMessage>>>,
        >,
        worker_tasks: &Arc<Mutex<tokio::task::JoinSet<()>>>,
        turn_permits: &Arc<tokio::sync::Semaphore>,
        shutdown_tx: &tokio::sync::mpsc::Sender<()>,
        routine_engine: Option<Arc<RoutineEngine>>,
        message: IncomingMessage,
    ) {
        // Resolve identity before deriving the worker key. Otherwise untrusted
        // adapter metadata can manufacture arbitrary principal/actor IDs and
        // shard one real conversation across workers (or force unrelated
        // conversations into the same queue) before the canonical ingress
        // resolver gets a chance to strip those claims.
        let mut message = message;
        if let Err(error) = agent.resolve_ingress_identity(&mut message).await {
            tracing::error!(
                channel = %message.channel,
                error = %error,
                "Rejecting message whose ingress identity could not be resolved"
            );
            if let Err(send_error) = agent
                .channels
                .respond(&message, OutgoingResponse::text(format!("Error: {error}")))
                .await
            {
                tracing::error!(
                    channel = %message.channel,
                    error = %send_error,
                    "Failed to send ingress identity error response"
                );
            }
            return;
        }

        let preview = SubmissionParser::parse(&message.content);
        if matches!(
            preview,
            Submission::Interrupt | Submission::Quit | Submission::Restart
        ) {
            let agent = Arc::clone(agent);
            let shutdown_tx = shutdown_tx.clone();
            Self::spawn_tracked(worker_tasks, async move {
                if agent
                    .handle_and_respond(&message, Some(preview), routine_engine.as_ref())
                    .await
                {
                    let _ = shutdown_tx.try_send(());
                }
            })
            .await;
            return;
        }

        let key = thinclaw_agent::session_manager::SessionManager::session_scope_for_identity(
            &message.resolved_identity(),
        );
        let mut pending = message;
        loop {
            // Fast path: hand to the existing worker for this conversation.
            {
                let senders = workers.lock().await;
                if let Some(tx) = senders.get(&key) {
                    let tx = tx.clone();
                    drop(senders);
                    match tx.send(pending).await {
                        Ok(()) => return,
                        Err(tokio::sync::mpsc::error::SendError(msg)) => {
                            // Worker exited between lookup and send; retry
                            // against a fresh worker.
                            pending = msg;
                        }
                    }
                }
            }

            // Slow path: install a worker for this conversation, then loop
            // back to the fast path to enqueue.
            let mut senders = workers.lock().await;
            if let std::collections::hash_map::Entry::Vacant(entry) = senders.entry(key) {
                let (tx, rx) = tokio::sync::mpsc::channel(Self::CONVERSATION_WORKER_QUEUE_DEPTH);
                entry.insert(tx);
                Self::spawn_conversation_worker(
                    Arc::clone(agent),
                    Arc::clone(workers),
                    Arc::clone(turn_permits),
                    shutdown_tx.clone(),
                    routine_engine.clone(),
                    key,
                    rx,
                    worker_tasks,
                )
                .await;
            }
        }
    }
}
