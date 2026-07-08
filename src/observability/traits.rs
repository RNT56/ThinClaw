//! Core observer trait and event/metric types.

use std::sync::Arc;
use std::time::Duration;

use thinclaw_agent::loop_control::{LoopKind, LoopRunSummary, LoopStopReason};

/// Provider-agnostic observer for agent lifecycle events and metrics.
///
/// Implementations can log to tracing, export to OpenTelemetry, write to
/// Prometheus, or do nothing at all. The agent records events at key
/// lifecycle points and the observer decides what to do with them.
///
/// Thread-safe and cheaply cloneable behind `Arc<dyn Observer>`.
pub trait Observer: Send + Sync {
    /// Record a discrete lifecycle event.
    fn record_event(&self, event: &ObserverEvent);

    /// Record a numeric metric sample.
    fn record_metric(&self, metric: &ObserverMetric);

    /// Flush any buffered data (e.g. OTLP batch exporter). No-op by default.
    fn flush(&self) {}

    /// Human-readable backend name (e.g. "noop", "log", "otel").
    fn name(&self) -> &str;
}

/// Discrete lifecycle events the agent can emit.
#[derive(Debug, Clone)]
pub enum ObserverEvent {
    /// Agent started processing.
    AgentStart { provider: String, model: String },

    /// An LLM request was sent.
    LlmRequest {
        provider: String,
        model: String,
        message_count: usize,
    },

    /// An LLM response was received.
    LlmResponse {
        provider: String,
        model: String,
        duration: Duration,
        success: bool,
        error_message: Option<String>,
    },

    /// A tool call is about to start.
    ToolCallStart { tool: String },

    /// A tool call finished.
    ToolCallEnd {
        tool: String,
        duration: Duration,
        success: bool,
    },

    /// One reasoning turn completed.
    TurnComplete,

    /// A message was sent or received on a channel.
    ChannelMessage { channel: String, direction: String },

    /// The heartbeat system ran a tick.
    HeartbeatTick,

    /// Agent finished processing.
    AgentEnd {
        duration: Duration,
        tokens_used: Option<u64>,
    },

    /// An error occurred in a component.
    Error { component: String, message: String },
}

/// Numeric metric samples.
#[derive(Debug, Clone)]
pub enum ObserverMetric {
    /// Latency of a single request (histogram-style).
    RequestLatency(Duration),
    /// Cumulative tokens consumed.
    TokensUsed(u64),
    /// Current number of active jobs (gauge).
    ActiveJobs(u64),
    /// Current message queue depth (gauge).
    QueueDepth(u64),
    /// A long-running loop started.
    LoopStarted(LoopKind),
    /// A long-running loop stopped with a structured reason.
    LoopRun(LoopRunSummary),
    /// A named phase within a long-running loop completed.
    LoopPhaseRun(LoopPhaseRun),
}

#[derive(Debug, Clone)]
pub struct LoopPhaseRun {
    pub kind: LoopKind,
    pub phase: String,
    pub stop_reason: LoopStopReason,
    pub duration: Duration,
    pub iterations: usize,
    pub retries: u32,
}

impl LoopPhaseRun {
    pub fn new(
        kind: LoopKind,
        phase: impl Into<String>,
        stop_reason: LoopStopReason,
        duration: Duration,
        iterations: usize,
        retries: u32,
    ) -> Self {
        Self {
            kind,
            phase: phase.into(),
            stop_reason,
            duration,
            iterations,
            retries,
        }
    }
}

/// Synchronous guard for loops with many early-return paths.
///
/// The observer API is sync, so this can emit a final `LoopRun` metric from
/// `Drop` without changing async control flow. Loops should update iterations
/// and terminal reason as they make progress; unexpected `?` exits default to
/// `FatalError`, making missing classification visible in metrics.
pub struct LoopMetricGuard {
    observer: Arc<dyn Observer>,
    kind: LoopKind,
    iterations: usize,
    retries: u32,
    stop_reason: LoopStopReason,
}

impl LoopMetricGuard {
    pub fn start(observer: Arc<dyn Observer>, kind: LoopKind) -> Self {
        observer.record_metric(&ObserverMetric::LoopStarted(kind));
        Self {
            observer,
            kind,
            iterations: 0,
            retries: 0,
            stop_reason: LoopStopReason::FatalError,
        }
    }

    pub fn set_iterations(&mut self, iterations: usize) {
        self.iterations = iterations;
    }

    pub fn set_retries(&mut self, retries: u32) {
        self.retries = retries;
    }

    pub fn stop_with(&mut self, stop_reason: LoopStopReason) {
        self.stop_reason = stop_reason;
    }
}

impl Drop for LoopMetricGuard {
    fn drop(&mut self) {
        self.observer
            .record_metric(&ObserverMetric::LoopRun(LoopRunSummary::new(
                self.kind,
                self.stop_reason,
                self.iterations,
                self.retries,
            )));
    }
}

#[cfg(test)]
mod tests {
    use crate::observability::traits::*;

    #[test]
    fn event_variants_are_constructible() {
        let _ = ObserverEvent::AgentStart {
            provider: "nearai".into(),
            model: "test".into(),
        };
        let _ = ObserverEvent::LlmRequest {
            provider: "nearai".into(),
            model: "test".into(),
            message_count: 3,
        };
        let _ = ObserverEvent::LlmResponse {
            provider: "nearai".into(),
            model: "test".into(),
            duration: Duration::from_millis(100),
            success: true,
            error_message: None,
        };
        let _ = ObserverEvent::ToolCallStart {
            tool: "echo".into(),
        };
        let _ = ObserverEvent::ToolCallEnd {
            tool: "echo".into(),
            duration: Duration::from_millis(5),
            success: true,
        };
        let _ = ObserverEvent::TurnComplete;
        let _ = ObserverEvent::ChannelMessage {
            channel: "tui".into(),
            direction: "inbound".into(),
        };
        let _ = ObserverEvent::HeartbeatTick;
        let _ = ObserverEvent::AgentEnd {
            duration: Duration::from_secs(10),
            tokens_used: Some(1500),
        };
        let _ = ObserverEvent::Error {
            component: "llm".into(),
            message: "timeout".into(),
        };
    }

    #[test]
    fn metric_variants_are_constructible() {
        use thinclaw_agent::loop_control::LoopStopReason;

        let _ = ObserverMetric::RequestLatency(Duration::from_millis(200));
        let _ = ObserverMetric::TokensUsed(500);
        let _ = ObserverMetric::ActiveJobs(3);
        let _ = ObserverMetric::QueueDepth(10);
        let _ = ObserverMetric::LoopStarted(LoopKind::RoutineCron);
        let _ = ObserverMetric::LoopRun(LoopRunSummary::new(
            LoopKind::RoutineCron,
            LoopStopReason::ExternalShutdown,
            2,
            0,
        ));
        let _ = ObserverMetric::LoopPhaseRun(LoopPhaseRun::new(
            LoopKind::RepoProjectSupervisor,
            "reconcile",
            LoopStopReason::Completed,
            Duration::from_millis(50),
            3,
            0,
        ));
    }

    #[test]
    fn loop_metric_guard_records_start_and_drop_summary() {
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct RecordingObserver {
            metrics: Mutex<Vec<ObserverMetric>>,
        }

        impl Observer for RecordingObserver {
            fn record_event(&self, _event: &ObserverEvent) {}

            fn record_metric(&self, metric: &ObserverMetric) {
                self.metrics
                    .lock()
                    .expect("metrics lock")
                    .push(metric.clone());
            }

            fn name(&self) -> &str {
                "recording"
            }
        }

        let observer = Arc::new(RecordingObserver::default());
        {
            let mut guard = LoopMetricGuard::start(observer.clone(), LoopKind::AgentDispatcher);
            guard.set_iterations(7);
            guard.set_retries(2);
            guard.stop_with(LoopStopReason::IterationBudgetExceeded);
        }

        let metrics = observer.metrics.lock().expect("metrics lock");
        assert!(matches!(
            metrics.first(),
            Some(ObserverMetric::LoopStarted(LoopKind::AgentDispatcher))
        ));
        let Some(ObserverMetric::LoopRun(summary)) = metrics.get(1) else {
            panic!("expected loop run summary metric");
        };
        assert_eq!(summary.kind, LoopKind::AgentDispatcher);
        assert_eq!(summary.stop_reason, LoopStopReason::IterationBudgetExceeded);
        assert_eq!(summary.iterations, 7);
        assert_eq!(summary.retries, 2);
    }

    /// Guardrail (backlog B2): every `ObserverEvent` variant must have a real
    /// production emit site, not merely a definition. We scan the crate's `src/`
    /// tree for `ObserverEvent::<Variant>` references, excluding the observability
    /// module itself — which holds the enum definition, the observer impls that
    /// *match* on variants, and this test. Any remaining reference is an emit call,
    /// so a missing variant here means we defined an event nobody ever records.
    #[test]
    fn all_event_variants_have_production_emit_sites() {
        use std::path::{Path, PathBuf};

        const VARIANTS: &[&str] = &[
            "AgentStart",
            "LlmRequest",
            "LlmResponse",
            "ToolCallStart",
            "ToolCallEnd",
            "TurnComplete",
            "ChannelMessage",
            "HeartbeatTick",
            "AgentEnd",
            "Error",
        ];

        let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let observability_dir = src_root.join("observability");

        let mut haystack = String::new();
        let mut stack: Vec<PathBuf> = vec![src_root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read src dir") {
                let path = entry.expect("dir entry").path();
                // Skip the observability module (definition + observer impls + this test).
                if path.starts_with(&observability_dir) {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    haystack.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
                    haystack.push('\n');
                }
            }
        }

        let missing: Vec<&str> = VARIANTS
            .iter()
            .copied()
            .filter(|v| !haystack.contains(&format!("ObserverEvent::{}", v)))
            .collect();

        assert!(
            missing.is_empty(),
            "ObserverEvent variants with no production emit site outside src/observability/: {:?}. \
             Every variant must be recorded somewhere (backlog B2).",
            missing
        );
    }
}
