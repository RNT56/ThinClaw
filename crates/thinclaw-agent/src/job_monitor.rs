//! Background job monitor that forwards container coding agent output to the main agent loop.
//!
//! When the main agent kicks off a sandbox job (especially a container coding agent), this
//! monitor subscribes to the broadcast event channel and injects relevant
//! assistant messages back into the channel manager's stream. This lets the
//! main agent see what the sub-agent is producing and surface it to the user.
//!
//! ```text
//!   Container ──NDJSON──► Orchestrator ──broadcast──► JobMonitor
//!                                                        │
//!                                                  inject_tx (mpsc)
//!                                                        │
//!                                                        ▼
//!                                                   Agent Loop
//! ```

use std::time::Duration;

use thinclaw_channels_core::IncomingMessage;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use uuid::Uuid;

const JOB_MONITOR_INJECT_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(100)
} else {
    Duration::from_secs(5)
};
const JOB_MONITOR_MAX_LIFETIME: Duration = if cfg!(test) {
    Duration::from_millis(500)
} else {
    Duration::from_secs(24 * 60 * 60)
};

/// Portable job event consumed by the agent job monitor.
#[derive(Debug, Clone)]
pub enum JobMonitorEvent {
    Message { role: String, content: String },
    Result { status: String },
    SessionResult,
    Other,
}

/// Spawn a background task that watches for events from a specific job and
/// injects assistant messages into the agent loop.
///
/// The monitor forwards:
/// - `SseEvent::JobMessage` (assistant role): injected as incoming messages so
///   the main agent can read and relay to the user.
/// - `SseEvent::JobResult`: injected as a completion notice, then the task exits.
///
/// Tool use/result and status events are intentionally skipped (too noisy for
/// the main agent's context window).
pub fn spawn_job_monitor(
    job_id: Uuid,
    event_rx: broadcast::Receiver<(Uuid, JobMonitorEvent)>,
    inject_tx: mpsc::Sender<IncomingMessage>,
) -> JoinHandle<()> {
    tokio::spawn(run_job_monitor(
        job_id,
        event_rx,
        inject_tx,
        std::convert::identity,
    ))
}

/// Run a job monitor over an arbitrary broadcast event type.
///
/// The mapper lets gateway adapters translate their native event without
/// spawning a second forwarding task and channel.
pub async fn run_job_monitor<E, F>(
    job_id: Uuid,
    mut event_rx: broadcast::Receiver<(Uuid, E)>,
    inject_tx: mpsc::Sender<IncomingMessage>,
    map_event: F,
) where
    E: Clone + Send + 'static,
    F: Fn(E) -> JobMonitorEvent + Send + 'static,
{
    let short_id = job_id.to_string()[..8].to_string();

    tracing::info!(job_id = %short_id, "Job monitor started successfully");

    let lifetime = tokio::time::sleep(JOB_MONITOR_MAX_LIFETIME);
    tokio::pin!(lifetime);

    loop {
        let received = tokio::select! {
            result = event_rx.recv() => result,
            _ = &mut lifetime => {
                tracing::warn!(
                    job_id = %short_id,
                    max_lifetime_secs = JOB_MONITOR_MAX_LIFETIME.as_secs(),
                    "Job monitor reached its maximum lifetime and is stopping"
                );
                break;
            }
        };
        match received {
            Ok((ev_job_id, event)) => {
                if ev_job_id != job_id {
                    continue;
                }

                match map_event(event) {
                    // IC-025: Only forward assistant messages — system/tool messages
                    // are too noisy for the parent agent's context window.
                    JobMonitorEvent::Message { role, content } if role == "assistant" => {
                        let msg = IncomingMessage::new(
                            "job_monitor",
                            "system",
                            format!("[Job {}] Container agent: {}", short_id, content),
                        );
                        if !send_monitor_message(&inject_tx, msg, &short_id).await {
                            tracing::debug!(
                                job_id = %short_id,
                                "Inject channel closed, stopping monitor"
                            );
                            break;
                        }
                    }
                    JobMonitorEvent::Result { status } => {
                        let msg = IncomingMessage::new(
                            "job_monitor",
                            "system",
                            format!("[Job {}] Container finished (status: {})", short_id, status),
                        );
                        let _ = send_monitor_message(&inject_tx, msg, &short_id).await;
                        tracing::debug!(
                            job_id = %short_id,
                            status = %status,
                            "Job monitor exiting (job finished)"
                        );
                        break;
                    }
                    JobMonitorEvent::SessionResult => {}
                    _ => {}
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(
                    job_id = %short_id,
                    skipped = n,
                    "Job monitor lagged, some events were dropped"
                );
            }
            Err(broadcast::error::RecvError::Closed) => {
                tracing::debug!(
                    job_id = %short_id,
                    "Broadcast channel closed, stopping monitor"
                );
                break;
            }
        }
    }
}

async fn send_monitor_message(
    inject_tx: &mpsc::Sender<IncomingMessage>,
    message: IncomingMessage,
    short_id: &str,
) -> bool {
    match tokio::time::timeout(JOB_MONITOR_INJECT_TIMEOUT, inject_tx.send(message)).await {
        Ok(Ok(())) => true,
        Ok(Err(_)) => false,
        Err(_) => {
            tracing::warn!(
                job_id = short_id,
                timeout_ms = JOB_MONITOR_INJECT_TIMEOUT.as_millis() as u64,
                "Job monitor injection timed out; dropping backpressured update"
            );
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_monitor_forwards_assistant_messages() {
        let (event_tx, _) = broadcast::channel::<(Uuid, JobMonitorEvent)>(16);
        let (inject_tx, mut inject_rx) = mpsc::channel::<IncomingMessage>(16);

        let job_id = Uuid::new_v4();
        let _handle = spawn_job_monitor(job_id, event_tx.subscribe(), inject_tx);

        // Send an assistant message
        event_tx
            .send((
                job_id,
                JobMonitorEvent::Message {
                    role: "assistant".to_string(),
                    content: "I found a bug".to_string(),
                },
            ))
            .unwrap();

        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), inject_rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(msg.channel, "job_monitor");
        assert_eq!(msg.user_id, "system");
        assert!(msg.content.contains("I found a bug"));
    }

    #[tokio::test]
    async fn test_monitor_ignores_other_jobs() {
        let (event_tx, _) = broadcast::channel::<(Uuid, JobMonitorEvent)>(16);
        let (inject_tx, mut inject_rx) = mpsc::channel::<IncomingMessage>(16);

        let job_id = Uuid::new_v4();
        let other_job_id = Uuid::new_v4();
        let _handle = spawn_job_monitor(job_id, event_tx.subscribe(), inject_tx);

        // Send a message for a different job
        event_tx
            .send((
                other_job_id,
                JobMonitorEvent::Message {
                    role: "assistant".to_string(),
                    content: "wrong job".to_string(),
                },
            ))
            .unwrap();

        // Should not receive anything
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(100), inject_rx.recv()).await;
        assert!(
            result.is_err(),
            "should have timed out, no message expected"
        );
    }

    #[tokio::test]
    async fn test_monitor_exits_on_job_result() {
        let (event_tx, _) = broadcast::channel::<(Uuid, JobMonitorEvent)>(16);
        let (inject_tx, mut inject_rx) = mpsc::channel::<IncomingMessage>(16);

        let job_id = Uuid::new_v4();
        let handle = spawn_job_monitor(job_id, event_tx.subscribe(), inject_tx);

        // Send a completion event
        event_tx
            .send((
                job_id,
                JobMonitorEvent::Result {
                    status: "completed".to_string(),
                },
            ))
            .unwrap();

        // Should receive the completion message
        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), inject_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(msg.content.contains("finished"));

        // The monitor task should exit
        tokio::time::timeout(std::time::Duration::from_secs(1), handle)
            .await
            .expect("monitor should have exited")
            .expect("monitor task should not panic");
    }

    #[tokio::test]
    async fn test_monitor_exits_on_terminal_result_when_inject_queue_is_full() {
        let (event_tx, _) = broadcast::channel::<(Uuid, JobMonitorEvent)>(16);
        let (inject_tx, _inject_rx) = mpsc::channel::<IncomingMessage>(1);
        inject_tx
            .send(IncomingMessage::new("test", "system", "occupy queue"))
            .await
            .unwrap();

        let job_id = Uuid::new_v4();
        let handle = spawn_job_monitor(job_id, event_tx.subscribe(), inject_tx);
        event_tx
            .send((
                job_id,
                JobMonitorEvent::Result {
                    status: "completed".to_string(),
                },
            ))
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("terminal monitor should not wait forever on a full inject queue")
            .expect("monitor task should not panic");
        assert_eq!(event_tx.receiver_count(), 0);
    }

    #[tokio::test]
    async fn test_monitor_skips_tool_events() {
        let (event_tx, _) = broadcast::channel::<(Uuid, JobMonitorEvent)>(16);
        let (inject_tx, mut inject_rx) = mpsc::channel::<IncomingMessage>(16);

        let job_id = Uuid::new_v4();
        let _handle = spawn_job_monitor(job_id, event_tx.subscribe(), inject_tx);

        // Send tool use event (should be skipped)
        event_tx.send((job_id, JobMonitorEvent::Other)).unwrap();

        // Send user message (should be skipped)
        event_tx
            .send((
                job_id,
                JobMonitorEvent::Message {
                    role: "user".to_string(),
                    content: "user prompt".to_string(),
                },
            ))
            .unwrap();

        // Should not receive anything for tool events or user messages
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(100), inject_rx.recv()).await;
        assert!(
            result.is_err(),
            "should have timed out, no message expected"
        );
    }

    #[tokio::test]
    async fn test_monitor_has_bounded_lifetime_without_terminal_event() {
        let (event_tx, _) = broadcast::channel::<(Uuid, JobMonitorEvent)>(4);
        let (inject_tx, _inject_rx) = mpsc::channel::<IncomingMessage>(4);
        let job_id = Uuid::new_v4();
        let handle = spawn_job_monitor(job_id, event_tx.subscribe(), inject_tx);

        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("monitor should stop at its maximum lifetime")
            .expect("monitor task should not panic");
        assert_eq!(event_tx.receiver_count(), 0);
    }
}
