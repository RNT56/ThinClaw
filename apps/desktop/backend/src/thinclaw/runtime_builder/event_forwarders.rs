//! Runtime SSE and log forwarding into the typed Tauri event bus.

use std::sync::Arc;

use thinclaw_core::channels::web::log_layer::LogBroadcaster;
use thinclaw_core::channels::web::types::SseEvent;

use super::get_resolved_workspace_root;
use crate::thinclaw::ui_types::UiEvent;

pub(super) fn spawn_sse(
    app_handle: &tauri::AppHandle<tauri::Wry>,
    sse_tx: &tokio::sync::broadcast::Sender<SseEvent>,
) {
    // ── 8b. Spawn SSE → Tauri forwarder ─────────────────────────────────────────────────
    // Forward RoutineLifecycle events from the SSE channel to the frontend.
    {
        let mut sse_rx = sse_tx.subscribe();
        let fwd_handle = app_handle.clone();
        tokio::spawn(async move {
            use tauri::Emitter as _;
            loop {
                match sse_rx.recv().await {
                    Ok(event) => {
                        let ui_event: Option<UiEvent> = match &event {
                            SseEvent::RoutineLifecycle {
                                routine_name,
                                event,
                                run_id,
                                result_summary,
                            } => Some(UiEvent::RoutineLifecycle {
                                routine_name: routine_name.clone(),
                                event: event.clone(),
                                run_id: run_id.clone(),
                                result_summary: result_summary.clone(),
                            }),
                            SseEvent::ChannelStatusChange {
                                channel,
                                status,
                                message,
                            } => Some(UiEvent::ChannelStatus {
                                channel_id: channel.clone(),
                                state: status.clone(),
                                error: message.clone(),
                            }),
                            SseEvent::BootstrapCompleted => Some(UiEvent::BootstrapCompleted),
                            SseEvent::ToolResult { name, preview, .. } if name == "write_file" => {
                                // Parse the write_file result JSON to extract path & bytes
                                let val: serde_json::Value = serde_json::from_str(preview)
                                    .unwrap_or(serde_json::Value::Null);
                                if let (Some(path), Some(bytes)) = (
                                    val.get("path").and_then(|v| v.as_str()),
                                    val.get("bytes_written").and_then(|v| v.as_u64()),
                                ) {
                                    // Compute workspace-relative display path
                                    let workspace_root = get_resolved_workspace_root();
                                    let relative = if let Some(workspace_root) = workspace_root {
                                        path.strip_prefix(&workspace_root)
                                            .unwrap_or(path)
                                            .trim_start_matches('/')
                                            .to_string()
                                    } else {
                                        // Fall back to just the filename
                                        std::path::Path::new(path)
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or(path)
                                            .to_string()
                                    };
                                    tracing::info!(
                                        "[thinclaw-runtime] FileCreated: {} ({} bytes)",
                                        relative,
                                        bytes
                                    );
                                    Some(UiEvent::FileCreated {
                                        path: path.to_string(),
                                        relative_path: relative,
                                        bytes,
                                    })
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        if let Some(ev) = ui_event {
                            if let Err(e) = fwd_handle.emit("thinclaw-event", &ev) {
                                tracing::warn!("[sse-fwd] emit failed: {}", e);
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("[sse-fwd] dropped {} events", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
}

pub(super) fn spawn_logs(
    app_handle: &tauri::AppHandle<tauri::Wry>,
    log_broadcaster: &Arc<LogBroadcaster>,
) {
    // ── 8c. Live log push → Tauri frontend ──────────────────────────────────────────────
    // Subscribe to LogBroadcaster and forward each new entry as a
    // UiEvent::LogEntry so the UI Logs tab updates in real-time
    // instead of relying on the 2s polling interval.
    {
        let mut log_rx = log_broadcaster.subscribe();
        let log_fwd_handle = app_handle.clone();
        tokio::spawn(async move {
            use tauri::Emitter as _;
            loop {
                match log_rx.recv().await {
                    Ok(entry) => {
                        let ev = UiEvent::LogEntry {
                            timestamp: entry.timestamp,
                            level: entry.level,
                            target: entry.target,
                            message: entry.message,
                        };
                        // Fire-and-forget: if no UI is listening, drop the event.
                        let _ = log_fwd_handle.emit("thinclaw-event", &ev);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("[log-fwd] dropped {} log events (UI too slow)", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
}
