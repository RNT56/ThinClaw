//! Observability subsystem: trait-based event and metric recording.
//!
//! Provides a pluggable [`Observer`] trait with runtime-selectable backends:
//!
//! | Backend | Description |
//! |---------|-------------|
//! | `noop`  | Zero overhead, discards everything (default) |
//! | `log`   | Emits structured events via `tracing` |
//!
//! The [`create_observer`] factory builds the right backend from
//! [`ObservabilityConfig`]. Future backends (OpenTelemetry, Prometheus)
//! can be added by implementing [`Observer`].

mod log;
mod multi;
mod noop;
pub mod traits;

pub use self::log::LogObserver;
pub use self::noop::NoopObserver;
pub use self::traits::{Observer, ObserverEvent, ObserverMetric};

/// Configuration for the observability backend.
#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    /// Backend name: "none", "noop", "log".
    pub backend: String,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            backend: "log".into(),
        }
    }
}

/// Create an observer from configuration.
///
/// Returns a [`NoopObserver`] for "none"/"noop" (or unknown values),
/// and a [`LogObserver`] for "log".
///
/// Returns an [`Arc`] so the runtime can store a single shared owner and hand
/// cheap clones to event-emitting sites (the [`Observer`] trait is documented
/// as cheaply cloneable behind `Arc<dyn Observer>`).
pub fn create_observer(config: &ObservabilityConfig) -> std::sync::Arc<dyn Observer> {
    match config.backend.as_str() {
        "log" => std::sync::Arc::new(LogObserver),
        _ => std::sync::Arc::new(NoopObserver),
    }
}

#[cfg(test)]
mod tests {
    use crate::observability::*;

    #[test]
    fn default_config_is_log() {
        let cfg = ObservabilityConfig::default();
        assert_eq!(cfg.backend, "log");
    }

    #[test]
    fn factory_returns_noop_for_none() {
        let cfg = ObservabilityConfig {
            backend: "none".into(),
        };
        let obs = create_observer(&cfg);
        assert_eq!(obs.name(), "noop");
    }

    #[test]
    fn factory_returns_noop_for_empty() {
        let cfg = ObservabilityConfig {
            backend: String::new(),
        };
        let obs = create_observer(&cfg);
        assert_eq!(obs.name(), "noop");
    }

    #[test]
    fn factory_returns_noop_for_unknown() {
        let cfg = ObservabilityConfig {
            backend: "prometheus".into(),
        };
        let obs = create_observer(&cfg);
        assert_eq!(obs.name(), "noop");
    }

    #[test]
    fn factory_returns_log_for_log() {
        let cfg = ObservabilityConfig {
            backend: "log".into(),
        };
        let obs = create_observer(&cfg);
        assert_eq!(obs.name(), "log");
    }

    #[test]
    fn factory_returns_noop_for_noop() {
        let cfg = ObservabilityConfig {
            backend: "noop".into(),
        };
        let obs = create_observer(&cfg);
        assert_eq!(obs.name(), "noop");
    }

    #[test]
    fn factory_observer_is_shareable_and_records_startup_event() {
        // Mirrors the AppBuilder wiring: a shared Arc observer that records a
        // startup AgentStart event. Must not panic for any backend.
        let cfg = ObservabilityConfig {
            backend: "log".into(),
        };
        let obs = create_observer(&cfg);
        let cloned = obs.clone();
        cloned.record_event(&ObserverEvent::AgentStart {
            provider: "openai_compatible".into(),
            model: "test-model".into(),
        });
        assert_eq!(obs.name(), "log");
    }
}
