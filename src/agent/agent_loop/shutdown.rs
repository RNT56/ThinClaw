use thinclaw_agent::loop_control::LoopStopReason;

pub(super) async fn drain_conversation_tasks(
    tasks: &mut tokio::task::JoinSet<()>,
    graceful_timeout: std::time::Duration,
    abort_timeout: std::time::Duration,
) -> LoopStopReason {
    let graceful_drain = async {
        while let Some(joined) = tasks.join_next().await {
            if let Err(join_error) = joined
                && join_error.is_panic()
            {
                tracing::error!("A conversation worker panicked: {}", join_error);
            }
        }
    };
    if tokio::time::timeout(graceful_timeout, graceful_drain)
        .await
        .is_ok()
    {
        return LoopStopReason::Completed;
    }

    tracing::warn!(
        timeout_secs = graceful_timeout.as_secs(),
        "Conversation workers did not drain before shutdown timeout; aborting in-flight turns"
    );
    if tokio::time::timeout(abort_timeout, tasks.shutdown())
        .await
        .is_err()
    {
        tracing::error!(
            timeout_secs = abort_timeout.as_secs(),
            "Conversation workers did not join promptly after abort"
        );
        tasks.abort_all();
    }
    LoopStopReason::Cancelled
}
