//! Compatibility facade for the extracted agent job monitor.

use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::channels::IncomingMessage;
use crate::channels::web::types::SseEvent;

pub use thinclaw_agent::job_monitor::JobMonitorEvent;

/// Spawn a background task that adapts gateway SSE job events into the
/// extracted agent job monitor event stream.
pub fn spawn_job_monitor(
    job_id: Uuid,
    event_rx: broadcast::Receiver<(Uuid, SseEvent)>,
    inject_tx: mpsc::Sender<IncomingMessage>,
) -> JoinHandle<()> {
    tokio::spawn(thinclaw_agent::job_monitor::run_job_monitor(
        job_id,
        event_rx,
        inject_tx,
        |event| match event {
            SseEvent::JobMessage { role, content, .. } => {
                JobMonitorEvent::Message { role, content }
            }
            SseEvent::JobResult { status, .. } => JobMonitorEvent::Result { status },
            SseEvent::JobSessionResult { .. } => JobMonitorEvent::SessionResult,
            _ => JobMonitorEvent::Other,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn gateway_monitor_releases_subscription_after_terminal_result() {
        let (event_tx, _) = broadcast::channel(8);
        let (inject_tx, mut inject_rx) = mpsc::channel(8);
        let job_id = Uuid::new_v4();
        let handle = spawn_job_monitor(job_id, event_tx.subscribe(), inject_tx);

        event_tx
            .send((
                job_id,
                SseEvent::JobResult {
                    job_id: job_id.to_string(),
                    status: "completed".to_string(),
                    session_id: None,
                    success: Some(true),
                    message: None,
                },
            ))
            .expect("monitor should be subscribed");

        let message = tokio::time::timeout(std::time::Duration::from_secs(1), inject_rx.recv())
            .await
            .expect("completion message should arrive")
            .expect("inject channel should remain open");
        assert!(message.content.contains("finished"));

        tokio::time::timeout(std::time::Duration::from_secs(1), handle)
            .await
            .expect("monitor should stop after terminal result")
            .expect("monitor should not panic");
        assert_eq!(event_tx.receiver_count(), 0);
    }
}
