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
    mut event_rx: broadcast::Receiver<(Uuid, SseEvent)>,
    inject_tx: mpsc::Sender<IncomingMessage>,
) -> JoinHandle<()> {
    let (adapter_tx, adapter_rx) = broadcast::channel(64);
    let adapter_job_id = job_id;
    tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok((event_job_id, event)) => {
                    let monitor_event = match event {
                        SseEvent::JobMessage { role, content, .. } => {
                            JobMonitorEvent::Message { role, content }
                        }
                        SseEvent::JobResult { status, .. } => JobMonitorEvent::Result { status },
                        SseEvent::JobSessionResult { .. } => JobMonitorEvent::SessionResult,
                        _ => JobMonitorEvent::Other,
                    };
                    if adapter_tx.send((event_job_id, monitor_event)).is_err()
                        && event_job_id == adapter_job_id
                    {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        job_id = %adapter_job_id,
                        skipped = n,
                        "Job monitor adapter lagged, some events were dropped"
                    );
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    thinclaw_agent::job_monitor::spawn_job_monitor(job_id, adapter_rx, inject_tx)
}
