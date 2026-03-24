//! Plugin lifecycle hooks.
//!
//! Fires structured events at plugin lifecycle transitions, enabling
//! cross-cutting concerns (audit logging, metrics, notifications).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Lifecycle event types.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LifecycleEvent {
    Installing {
        name: String,
        kind: String,
    },
    Installed {
        name: String,
    },
    Activating {
        name: String,
    },
    Activated {
        name: String,
        tools: Vec<String>,
    },
    Deactivating {
        name: String,
    },
    Deactivated {
        name: String,
    },
    Uninstalling {
        name: String,
    },
    Uninstalled {
        name: String,
    },
    Failed {
        name: String,
        event: String,
        reason: String,
    },
}

impl LifecycleEvent {
    /// Get the plugin name from any event variant.
    pub fn plugin_name(&self) -> &str {
        match self {
            Self::Installing { name, .. }
            | Self::Installed { name }
            | Self::Activating { name }
            | Self::Activated { name, .. }
            | Self::Deactivating { name }
            | Self::Deactivated { name }
            | Self::Uninstalling { name }
            | Self::Uninstalled { name }
            | Self::Failed { name, .. } => name,
        }
    }

    /// Get a label for the event type.
    pub fn label(&self) -> &str {
        match self {
            Self::Installing { .. } => "installing",
            Self::Installed { .. } => "installed",
            Self::Activating { .. } => "activating",
            Self::Activated { .. } => "activated",
            Self::Deactivating { .. } => "deactivating",
            Self::Deactivated { .. } => "deactivated",
            Self::Uninstalling { .. } => "uninstalling",
            Self::Uninstalled { .. } => "uninstalled",
            Self::Failed { .. } => "failed",
        }
    }
}

/// Trait for lifecycle hook implementations.
pub trait LifecycleHook: Send + Sync {
    fn on_event(&self, event: &LifecycleEvent);
    fn name(&self) -> &str;
}

/// Registry holding all registered hooks.
pub struct LifecycleHookRegistry {
    hooks: Vec<Box<dyn LifecycleHook>>,
}

impl LifecycleHookRegistry {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Register a hook.
    pub fn register(&mut self, hook: Box<dyn LifecycleHook>) {
        self.hooks.push(hook);
    }

    /// Fire an event to all hooks.
    pub fn fire(&self, event: &LifecycleEvent) {
        for hook in &self.hooks {
            hook.on_event(event);
        }
    }

    /// Number of registered hooks.
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }
}

impl Default for LifecycleHookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Built-in audit log hook: records all events.
pub struct AuditLogHook {
    events: Arc<Mutex<Vec<(String, LifecycleEvent)>>>,
}

impl Default for AuditLogHook {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditLogHook {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn events(&self) -> Vec<(String, LifecycleEvent)> {
        self.events
            .lock()
            .expect("lifecycle events mutex poisoned")
            .clone()
    }

    pub fn len(&self) -> usize {
        self.events
            .lock()
            .expect("lifecycle events mutex poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.events
            .lock()
            .expect("lifecycle events mutex poisoned")
            .is_empty()
    }

    /// Return events as serializable flat structs for Tauri command response.
    ///
    /// Matches the `openclaw_plugin_lifecycle_list` contract: a flat list
    /// of `{ timestamp, plugin, event_type, details }` objects.
    pub fn events_serialized(&self) -> Vec<SerializedLifecycleEvent> {
        self.events
            .lock()
            .expect("lifecycle events mutex poisoned")
            .iter()
            .map(|(ts, event)| SerializedLifecycleEvent {
                timestamp: ts.clone(),
                plugin: event.plugin_name().to_string(),
                event_type: event.label().to_string(),
                details: match event {
                    LifecycleEvent::Installing { kind, .. } => Some(format!("kind: {}", kind)),
                    LifecycleEvent::Activated { tools, .. } => {
                        Some(format!("tools: {}", tools.join(", ")))
                    }
                    LifecycleEvent::Failed { reason, event, .. } => {
                        Some(format!("event: {}, reason: {}", event, reason))
                    }
                    _ => None,
                },
            })
            .collect()
    }
}

impl LifecycleHook for AuditLogHook {
    fn on_event(&self, event: &LifecycleEvent) {
        let timestamp = chrono::Utc::now().to_rfc3339();
        self.events
            .lock()
            .expect("lifecycle events mutex poisoned")
            .push((timestamp, event.clone()));
    }

    fn name(&self) -> &str {
        "audit_log"
    }
}

/// Built-in metrics hook: counts events by type.
pub struct MetricsHook {
    pub installs: Arc<AtomicU64>,
    pub activations: Arc<AtomicU64>,
    pub deactivations: Arc<AtomicU64>,
    pub failures: Arc<AtomicU64>,
}

impl Default for MetricsHook {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsHook {
    pub fn new() -> Self {
        Self {
            installs: Arc::new(AtomicU64::new(0)),
            activations: Arc::new(AtomicU64::new(0)),
            deactivations: Arc::new(AtomicU64::new(0)),
            failures: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl LifecycleHook for MetricsHook {
    fn on_event(&self, event: &LifecycleEvent) {
        match event {
            LifecycleEvent::Installed { .. } => {
                self.installs.fetch_add(1, Ordering::Relaxed);
            }
            LifecycleEvent::Activated { .. } => {
                self.activations.fetch_add(1, Ordering::Relaxed);
            }
            LifecycleEvent::Deactivated { .. } => {
                self.deactivations.fetch_add(1, Ordering::Relaxed);
            }
            LifecycleEvent::Failed { .. } => {
                self.failures.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    fn name(&self) -> &str {
        "metrics"
    }
}

/// Flattened, serializable lifecycle event for Tauri command responses.
///
/// Matches the `openclaw_plugin_lifecycle_list` response shape.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SerializedLifecycleEvent {
    pub timestamp: String,
    pub plugin: String,
    pub event_type: String,
    pub details: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fire_installs() {
        let hook = AuditLogHook::new();
        hook.on_event(&LifecycleEvent::Installed {
            name: "slack".into(),
        });
        assert_eq!(hook.len(), 1);
    }

    #[test]
    fn test_fire_activates() {
        let hook = AuditLogHook::new();
        hook.on_event(&LifecycleEvent::Activated {
            name: "slack".into(),
            tools: vec!["send_message".into()],
        });
        assert_eq!(hook.len(), 1);
    }

    #[test]
    fn test_fire_failed() {
        let hook = AuditLogHook::new();
        hook.on_event(&LifecycleEvent::Failed {
            name: "bad".into(),
            event: "install".into(),
            reason: "network error".into(),
        });
        assert_eq!(hook.len(), 1);
    }

    #[test]
    fn test_multiple_hooks() {
        let mut registry = LifecycleHookRegistry::new();
        let audit = Arc::new(AuditLogHook::new());
        let metrics = Arc::new(MetricsHook::new());

        // Wrap in newtype to share
        struct AuditWrap(Arc<AuditLogHook>);
        impl LifecycleHook for AuditWrap {
            fn on_event(&self, event: &LifecycleEvent) {
                self.0.on_event(event);
            }
            fn name(&self) -> &str {
                "audit"
            }
        }
        struct MetricsWrap(Arc<MetricsHook>);
        impl LifecycleHook for MetricsWrap {
            fn on_event(&self, event: &LifecycleEvent) {
                self.0.on_event(event);
            }
            fn name(&self) -> &str {
                "metrics"
            }
        }

        registry.register(Box::new(AuditWrap(audit.clone())));
        registry.register(Box::new(MetricsWrap(metrics.clone())));

        registry.fire(&LifecycleEvent::Installed {
            name: "test".into(),
        });
        assert_eq!(audit.len(), 1);
        assert_eq!(metrics.installs.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_audit_log_records() {
        let hook = AuditLogHook::new();
        hook.on_event(&LifecycleEvent::Installing {
            name: "notion".into(),
            kind: "mcp".into(),
        });
        let events = hook.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1.plugin_name(), "notion");
    }

    #[test]
    fn test_metrics_hook_counts() {
        let hook = MetricsHook::new();
        hook.on_event(&LifecycleEvent::Installed { name: "a".into() });
        hook.on_event(&LifecycleEvent::Installed { name: "b".into() });
        hook.on_event(&LifecycleEvent::Failed {
            name: "c".into(),
            event: "install".into(),
            reason: "err".into(),
        });
        assert_eq!(hook.installs.load(Ordering::Relaxed), 2);
        assert_eq!(hook.failures.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_event_variants() {
        let events = vec![
            LifecycleEvent::Installing {
                name: "a".into(),
                kind: "mcp".into(),
            },
            LifecycleEvent::Installed { name: "a".into() },
            LifecycleEvent::Activating { name: "a".into() },
            LifecycleEvent::Activated {
                name: "a".into(),
                tools: vec![],
            },
            LifecycleEvent::Deactivating { name: "a".into() },
            LifecycleEvent::Deactivated { name: "a".into() },
            LifecycleEvent::Uninstalling { name: "a".into() },
            LifecycleEvent::Uninstalled { name: "a".into() },
            LifecycleEvent::Failed {
                name: "a".into(),
                event: "x".into(),
                reason: "y".into(),
            },
        ];
        for e in &events {
            assert_eq!(e.plugin_name(), "a");
        }
        assert_eq!(events.len(), 9);
    }

    #[test]
    fn test_hook_count() {
        let mut registry = LifecycleHookRegistry::new();
        assert_eq!(registry.hook_count(), 0);
        registry.register(Box::new(AuditLogHook::new()));
        assert_eq!(registry.hook_count(), 1);
    }

    #[test]
    fn test_events_serialized() {
        let hook = AuditLogHook::new();
        hook.on_event(&LifecycleEvent::Installing {
            name: "notion".into(),
            kind: "mcp".into(),
        });
        hook.on_event(&LifecycleEvent::Activated {
            name: "notion".into(),
            tools: vec!["search".into(), "add_page".into()],
        });
        let serialized = hook.events_serialized();
        assert_eq!(serialized.len(), 2);
        assert_eq!(serialized[0].plugin, "notion");
        assert_eq!(serialized[0].event_type, "installing");
        assert!(serialized[0].details.as_ref().unwrap().contains("mcp"));
        assert_eq!(serialized[1].event_type, "activated");
        assert!(serialized[1].details.as_ref().unwrap().contains("search"));
    }

    #[test]
    fn test_events_serialized_json() {
        let hook = AuditLogHook::new();
        hook.on_event(&LifecycleEvent::Failed {
            name: "bad_plugin".into(),
            event: "install".into(),
            reason: "network error".into(),
        });
        let serialized = hook.events_serialized();
        let json = serde_json::to_string(&serialized[0]).unwrap();
        assert!(json.contains("\"event_type\":\"failed\""));
        assert!(json.contains("\"plugin\":\"bad_plugin\""));
        assert!(json.contains("network error"));
    }

    #[test]
    fn test_lifecycle_event_serializable() {
        let event = LifecycleEvent::Activated {
            name: "slack".into(),
            tools: vec!["send".into()],
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("slack"));
        let deser: LifecycleEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deser, event);
    }
}
