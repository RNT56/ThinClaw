//! Prometheus metrics observer.
//!
//! Implements [`Observer`] over a `prometheus_client` registry so the agent's
//! existing lifecycle events/metrics become scrapeable counters, histograms,
//! and gauges. The same instance is shared with the gateway, which exposes the
//! registry at `GET /metrics` in Prometheus text-exposition format.
//!
//! Pull-based only: `flush()` is a no-op. OTLP push export is intentionally
//! deferred (its tonic/gRPC exporter would conflict with the tonic version
//! libSQL already pins) and can be added later behind its own feature flag.

use std::sync::atomic::{AtomicU64, Ordering};

use prometheus_client::encoding::{EncodeLabelSet, text::encode};
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::Histogram;
use prometheus_client::registry::Registry;

use crate::observability::traits::{Observer, ObserverEvent, ObserverMetric};

/// Latency histogram buckets in seconds (sub-100ms to 60s).
const LATENCY_BUCKETS: [f64; 12] = [
    0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 20.0, 30.0, 45.0, 60.0,
];

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct ProviderModelLabels {
    provider: String,
    model: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct ToolLabels {
    tool: String,
    success: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct ToolNameLabels {
    tool: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct ComponentLabels {
    component: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct ChannelLabels {
    channel: String,
    direction: String,
}

/// Prometheus-backed observer. Cloneable metric families share atomic state, so
/// the single registered registry always reflects live values at scrape time.
pub struct PrometheusObserver {
    registry: Registry,

    llm_requests: Family<ProviderModelLabels, Counter>,
    llm_errors: Family<ProviderModelLabels, Counter>,
    llm_response_seconds: Family<ProviderModelLabels, Histogram>,

    tool_calls: Family<ToolLabels, Counter>,
    tool_call_seconds: Family<ToolNameLabels, Histogram>,

    agent_turns: Counter,
    agent_errors: Family<ComponentLabels, Counter>,
    channel_messages: Family<ChannelLabels, Counter>,
    heartbeat_ticks: Counter,
    tokens_used: Counter,
    request_latency_seconds: Histogram,

    active_jobs: Gauge,
    queue_depth: Gauge,
    /// Cumulative cost in whole cents, refreshed from CostTracker at scrape time.
    cost_cents: Gauge,

    /// Process start time (unix seconds) for an uptime gauge computed on scrape.
    start_unix_secs: AtomicU64,
}

impl PrometheusObserver {
    /// Build the observer, registering every metric into a fresh registry.
    pub fn new() -> Self {
        let mut registry = Registry::with_prefix("thinclaw");

        let llm_requests = Family::<ProviderModelLabels, Counter>::default();
        registry.register(
            "llm_requests",
            "Total LLM requests issued",
            llm_requests.clone(),
        );

        let llm_errors = Family::<ProviderModelLabels, Counter>::default();
        registry.register(
            "llm_errors",
            "Total failed LLM responses",
            llm_errors.clone(),
        );

        let llm_response_seconds =
            Family::<ProviderModelLabels, Histogram>::new_with_constructor(|| {
                Histogram::new(LATENCY_BUCKETS)
            });
        registry.register(
            "llm_response_seconds",
            "LLM response latency in seconds",
            llm_response_seconds.clone(),
        );

        let tool_calls = Family::<ToolLabels, Counter>::default();
        registry.register("tool_calls", "Total tool invocations", tool_calls.clone());

        let tool_call_seconds = Family::<ToolNameLabels, Histogram>::new_with_constructor(|| {
            Histogram::new(LATENCY_BUCKETS)
        });
        registry.register(
            "tool_call_seconds",
            "Tool execution latency in seconds",
            tool_call_seconds.clone(),
        );

        let agent_turns = Counter::default();
        registry.register(
            "agent_turns",
            "Total completed reasoning turns",
            agent_turns.clone(),
        );

        let agent_errors = Family::<ComponentLabels, Counter>::default();
        registry.register(
            "agent_errors",
            "Total component errors",
            agent_errors.clone(),
        );

        let channel_messages = Family::<ChannelLabels, Counter>::default();
        registry.register(
            "channel_messages",
            "Total channel messages by direction",
            channel_messages.clone(),
        );

        let heartbeat_ticks = Counter::default();
        registry.register(
            "heartbeat_ticks",
            "Total heartbeat ticks",
            heartbeat_ticks.clone(),
        );

        let tokens_used = Counter::default();
        registry.register(
            "tokens_used",
            "Cumulative model tokens consumed",
            tokens_used.clone(),
        );

        let request_latency_seconds = Histogram::new(LATENCY_BUCKETS);
        registry.register(
            "request_latency_seconds",
            "Generic request latency in seconds",
            request_latency_seconds.clone(),
        );

        let active_jobs = Gauge::default();
        registry.register("active_jobs", "Currently active jobs", active_jobs.clone());

        let queue_depth = Gauge::default();
        registry.register(
            "queue_depth",
            "Current message queue depth",
            queue_depth.clone(),
        );

        let cost_cents = Gauge::default();
        registry.register(
            "cost_cents",
            "Cumulative model spend in whole cents",
            cost_cents.clone(),
        );

        Self {
            registry,
            llm_requests,
            llm_errors,
            llm_response_seconds,
            tool_calls,
            tool_call_seconds,
            agent_turns,
            agent_errors,
            channel_messages,
            heartbeat_ticks,
            tokens_used,
            request_latency_seconds,
            active_jobs,
            queue_depth,
            cost_cents,
            start_unix_secs: AtomicU64::new(0),
        }
    }

    /// Record the process start time (unix seconds) for the uptime gauge.
    pub fn set_start_time(&self, unix_secs: u64) {
        self.start_unix_secs.store(unix_secs, Ordering::Relaxed);
    }

    /// Update the cost gauge from a CostTracker snapshot (called at scrape time
    /// so the hot LLM path never touches the observer for cost).
    pub fn set_cost_cents(&self, cents: i64) {
        self.cost_cents.set(cents);
    }

    /// Encode the registry as Prometheus text-exposition format.
    pub fn encode(&self) -> String {
        let mut buffer = String::new();
        // Registry encode is infallible for String sinks in practice; on the
        // off chance of a formatting error, surface a diagnostic body rather
        // than panicking a scrape.
        if let Err(e) = encode(&mut buffer, &self.registry) {
            return format!("# encode error: {e}\n");
        }
        buffer
    }
}

impl Default for PrometheusObserver {
    fn default() -> Self {
        Self::new()
    }
}

impl Observer for PrometheusObserver {
    fn record_event(&self, event: &ObserverEvent) {
        match event {
            ObserverEvent::AgentStart { .. } => {}
            ObserverEvent::LlmRequest {
                provider, model, ..
            } => {
                self.llm_requests
                    .get_or_create(&ProviderModelLabels {
                        provider: provider.clone(),
                        model: model.clone(),
                    })
                    .inc();
            }
            ObserverEvent::LlmResponse {
                provider,
                model,
                duration,
                success,
                ..
            } => {
                let labels = ProviderModelLabels {
                    provider: provider.clone(),
                    model: model.clone(),
                };
                self.llm_response_seconds
                    .get_or_create(&labels)
                    .observe(duration.as_secs_f64());
                if !success {
                    self.llm_errors.get_or_create(&labels).inc();
                }
            }
            ObserverEvent::ToolCallStart { .. } => {}
            ObserverEvent::ToolCallEnd {
                tool,
                duration,
                success,
            } => {
                self.tool_calls
                    .get_or_create(&ToolLabels {
                        tool: tool.clone(),
                        success: success.to_string(),
                    })
                    .inc();
                self.tool_call_seconds
                    .get_or_create(&ToolNameLabels { tool: tool.clone() })
                    .observe(duration.as_secs_f64());
            }
            ObserverEvent::TurnComplete => {
                self.agent_turns.inc();
            }
            ObserverEvent::ChannelMessage { channel, direction } => {
                self.channel_messages
                    .get_or_create(&ChannelLabels {
                        channel: channel.clone(),
                        direction: direction.clone(),
                    })
                    .inc();
            }
            ObserverEvent::HeartbeatTick => {
                self.heartbeat_ticks.inc();
            }
            ObserverEvent::AgentEnd { tokens_used, .. } => {
                if let Some(tokens) = tokens_used {
                    self.tokens_used.inc_by(*tokens);
                }
            }
            ObserverEvent::Error { component, .. } => {
                self.agent_errors
                    .get_or_create(&ComponentLabels {
                        component: component.clone(),
                    })
                    .inc();
            }
        }
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        match metric {
            ObserverMetric::RequestLatency(d) => {
                self.request_latency_seconds.observe(d.as_secs_f64());
            }
            ObserverMetric::TokensUsed(n) => {
                self.tokens_used.inc_by(*n);
            }
            ObserverMetric::ActiveJobs(n) => {
                self.active_jobs.set(*n as i64);
            }
            ObserverMetric::QueueDepth(n) => {
                self.queue_depth.set(*n as i64);
            }
        }
    }

    fn name(&self) -> &str {
        "prometheus"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn name_is_prometheus() {
        assert_eq!(PrometheusObserver::new().name(), "prometheus");
    }

    #[test]
    fn records_and_encodes_all_variants() {
        let obs = PrometheusObserver::new();
        obs.record_event(&ObserverEvent::LlmRequest {
            provider: "openai".into(),
            model: "gpt-x".into(),
            message_count: 3,
        });
        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "openai".into(),
            model: "gpt-x".into(),
            duration: Duration::from_millis(1200),
            success: true,
            error_message: None,
        });
        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "openai".into(),
            model: "gpt-x".into(),
            duration: Duration::from_millis(400),
            success: false,
            error_message: Some("boom".into()),
        });
        obs.record_event(&ObserverEvent::ToolCallEnd {
            tool: "shell".into(),
            duration: Duration::from_millis(20),
            success: true,
        });
        obs.record_event(&ObserverEvent::TurnComplete);
        obs.record_event(&ObserverEvent::ChannelMessage {
            channel: "signal".into(),
            direction: "inbound".into(),
        });
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(5),
            tokens_used: Some(1500),
        });
        obs.record_event(&ObserverEvent::Error {
            component: "llm".into(),
            message: "oops".into(),
        });
        obs.record_metric(&ObserverMetric::ActiveJobs(4));
        obs.record_metric(&ObserverMetric::QueueDepth(9));
        obs.set_cost_cents(1234);

        let text = obs.encode();
        // Prefix + a sampling of registered series must be present.
        assert!(text.contains("thinclaw_llm_requests_total"));
        assert!(text.contains("thinclaw_llm_errors_total"));
        assert!(text.contains("thinclaw_tool_calls_total"));
        assert!(text.contains("thinclaw_agent_turns_total"));
        assert!(text.contains("thinclaw_tokens_used_total"));
        assert!(text.contains("thinclaw_active_jobs 4"));
        assert!(text.contains("thinclaw_queue_depth 9"));
        assert!(text.contains("thinclaw_cost_cents 1234"));
        // Label rendering.
        assert!(text.contains("provider=\"openai\""));
        assert!(text.contains("tool=\"shell\""));
        // Exposition ends with the EOF marker.
        assert!(text.contains("# EOF"));
    }

    #[test]
    fn counters_accumulate() {
        let obs = PrometheusObserver::new();
        for _ in 0..3 {
            obs.record_event(&ObserverEvent::TurnComplete);
        }
        assert!(obs.encode().contains("thinclaw_agent_turns_total 3"));
    }
}
