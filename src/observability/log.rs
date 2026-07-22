//! Tracing-based observer that emits structured log events.
//!
//! Uses the existing `tracing` infrastructure so events appear alongside
//! normal application logs, with no extra dependencies. Good for local
//! development and debugging.

use crate::observability::traits::{Observer, ObserverEvent, ObserverMetric};

/// Observer that logs events and metrics via `tracing`.
pub struct LogObserver;

impl Observer for LogObserver {
    fn record_event(&self, event: &ObserverEvent) {
        match event {
            ObserverEvent::AgentStart { provider, model } => {
                tracing::info!(provider, model, "observer: agent.start");
            }
            ObserverEvent::LlmRequest {
                provider,
                model,
                message_count,
            } => {
                tracing::info!(provider, model, message_count, "observer: llm.request");
            }
            ObserverEvent::LlmResponse {
                provider,
                model,
                duration,
                success,
                error_message,
            } => {
                tracing::info!(
                    provider,
                    model,
                    duration_ms = duration.as_millis() as u64,
                    success,
                    error_present = error_message.is_some(),
                    error_bytes = error_message.as_deref().map_or(0, str::len),
                    "observer: llm.response"
                );
            }
            ObserverEvent::ToolCallStart { tool } => {
                tracing::info!(tool, "observer: tool.start");
            }
            ObserverEvent::ToolCallEnd {
                tool,
                duration,
                success,
            } => {
                tracing::info!(
                    tool,
                    duration_ms = duration.as_millis() as u64,
                    success,
                    "observer: tool.end"
                );
            }
            ObserverEvent::TurnComplete => {
                tracing::info!("observer: turn.complete");
            }
            ObserverEvent::ChannelMessage { channel, direction } => {
                tracing::info!(channel, direction, "observer: channel.message");
            }
            ObserverEvent::HeartbeatTick => {
                tracing::debug!("observer: heartbeat.tick");
            }
            ObserverEvent::AgentEnd {
                duration,
                tokens_used,
            } => {
                tracing::info!(
                    duration_secs = duration.as_secs_f64(),
                    tokens_used = tokens_used.unwrap_or(0),
                    "observer: agent.end"
                );
            }
            ObserverEvent::Error { component, message } => {
                tracing::warn!(component, error_bytes = message.len(), "observer: error");
            }
        }
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        match metric {
            ObserverMetric::RequestLatency(d) => {
                tracing::debug!(
                    latency_ms = d.as_millis() as u64,
                    "observer: metric.request_latency"
                );
            }
            ObserverMetric::TokensUsed(n) => {
                tracing::debug!(tokens = n, "observer: metric.tokens_used");
            }
            ObserverMetric::ActiveJobs(n) => {
                tracing::debug!(active_jobs = n, "observer: metric.active_jobs");
            }
            ObserverMetric::QueueDepth(n) => {
                tracing::debug!(queue_depth = n, "observer: metric.queue_depth");
            }
            ObserverMetric::LoopStarted(kind) => {
                tracing::debug!(loop_kind = kind.as_str(), "observer: metric.loop_started");
            }
            ObserverMetric::LoopRun(summary) => {
                tracing::debug!(
                    loop_kind = summary.kind.as_str(),
                    stop_reason = summary.stop_reason.as_str(),
                    iterations = summary.iterations,
                    retries = summary.retries,
                    failed = summary.stop_reason.is_failure(),
                    "observer: metric.loop_run"
                );
            }
            ObserverMetric::LoopPhaseRun(phase) => {
                tracing::debug!(
                    loop_kind = phase.kind.as_str(),
                    phase = phase.phase.as_str(),
                    stop_reason = phase.stop_reason.as_str(),
                    duration_ms = phase.duration.as_millis() as u64,
                    iterations = phase.iterations,
                    retries = phase.retries,
                    failed = phase.stop_reason.is_failure(),
                    "observer: metric.loop_phase_run"
                );
            }
        }
    }

    fn name(&self) -> &str {
        "log"
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::observability::log::LogObserver;
    use crate::observability::traits::*;

    #[test]
    fn name_is_log() {
        assert_eq!(LogObserver.name(), "log");
    }

    #[test]
    fn record_event_does_not_panic() {
        let obs = LogObserver;
        obs.record_event(&ObserverEvent::AgentStart {
            provider: "nearai".into(),
            model: "test".into(),
        });
        obs.record_event(&ObserverEvent::LlmRequest {
            provider: "nearai".into(),
            model: "test".into(),
            message_count: 5,
        });
        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "nearai".into(),
            model: "test".into(),
            duration: Duration::from_millis(150),
            success: true,
            error_message: None,
        });
        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "nearai".into(),
            model: "test".into(),
            duration: Duration::from_millis(1500),
            success: false,
            error_message: Some("timeout".into()),
        });
        obs.record_event(&ObserverEvent::ToolCallStart {
            tool: "shell".into(),
        });
        obs.record_event(&ObserverEvent::ToolCallEnd {
            tool: "shell".into(),
            duration: Duration::from_millis(20),
            success: true,
        });
        obs.record_event(&ObserverEvent::TurnComplete);
        obs.record_event(&ObserverEvent::ChannelMessage {
            channel: "tui".into(),
            direction: "inbound".into(),
        });
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(30),
            tokens_used: Some(2500),
        });
        obs.record_event(&ObserverEvent::Error {
            component: "llm".into(),
            message: "connection refused".into(),
        });
    }

    #[test]
    fn record_metric_does_not_panic() {
        let obs = LogObserver;
        obs.record_metric(&ObserverMetric::RequestLatency(Duration::from_millis(200)));
        obs.record_metric(&ObserverMetric::TokensUsed(1000));
        obs.record_metric(&ObserverMetric::ActiveJobs(5));
        obs.record_metric(&ObserverMetric::QueueDepth(12));
        obs.record_metric(&ObserverMetric::LoopStarted(
            thinclaw_agent::loop_control::LoopKind::SelfRepair,
        ));
        obs.record_metric(&ObserverMetric::LoopRun(
            thinclaw_agent::loop_control::LoopRunSummary::new(
                thinclaw_agent::loop_control::LoopKind::SelfRepair,
                thinclaw_agent::loop_control::LoopStopReason::ExternalShutdown,
                1,
                0,
            ),
        ));
        obs.record_metric(&ObserverMetric::LoopPhaseRun(LoopPhaseRun::new(
            thinclaw_agent::loop_control::LoopKind::RepoProjectSupervisor,
            "reconcile",
            thinclaw_agent::loop_control::LoopStopReason::Completed,
            Duration::from_millis(20),
            1,
            0,
        )));
    }

    #[test]
    fn flush_does_not_panic() {
        LogObserver.flush();
    }
}
