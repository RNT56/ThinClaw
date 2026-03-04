//! Unified Tauri command facade for IronClaw backend services.
//!
//! Provides the adapter functions that Scrappy's Tauri command stubs
//! should call. Each function maps directly to an `openclaw_*` command
//! from the §17.4 integration contract.
//!
//! # Usage from Scrappy `rpc.rs`
//!
//! ```rust,ignore
//! use ironclaw::tauri_commands;
//!
//! #[tauri::command]
//! fn openclaw_cost_summary(state: State<IronClawState>) -> Result<CostSummary, String> {
//!     tauri_commands::cost_summary(&state.cost_tracker)
//! }
//! ```

use crate::agent::routine_audit::{RoutineAuditLog, RoutineRun};
use crate::extensions::clawhub::{CatalogCache, CatalogEntry};
use crate::extensions::lifecycle_hooks::{AuditLogHook, SerializedLifecycleEvent};
use crate::extensions::manifest_validator::{ManifestValidator, PluginInfoRef, ValidationResponse};
use crate::llm::cost_tracker::{CostSummary, CostTracker};
use crate::llm::response_cache_ext::{CacheStats, CachedResponseStore};

// ── 1. openclaw_cost_summary ──────────────────────────────────────────

/// Build a cost summary.
///
/// Maps to: `openclaw_cost_summary`
/// Response: `CostSummary`
pub fn cost_summary(tracker: &CostTracker) -> Result<CostSummary, String> {
    let now = chrono::Utc::now();
    let today = now.format("%Y-%m-%d").to_string();
    let month = now.format("%Y-%m").to_string();
    Ok(tracker.summary(&today, &month))
}

// ── 2. openclaw_cost_export_csv ───────────────────────────────────────

/// Export cost entries as CSV.
///
/// Maps to: `openclaw_cost_export_csv`
/// Response: `String` (CSV text)
pub fn cost_export_csv(tracker: &CostTracker) -> Result<String, String> {
    Ok(tracker.export_csv())
}

// ── 3. openclaw_clawhub_search ────────────────────────────────────────

/// Search the ClawHub catalog cache.
///
/// Maps to: `openclaw_clawhub_search`
/// Params: `query: String`
/// Response: `Vec<CatalogEntry>`
pub fn clawhub_search(cache: &CatalogCache, query: &str) -> Result<Vec<CatalogEntry>, String> {
    Ok(cache.search(query).into_iter().cloned().collect())
}

// ── 4. openclaw_clawhub_install ───────────────────────────────────────

/// Install result from ClawHub.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstallResult {
    pub plugin_name: String,
    pub version: String,
    pub install_path: String,
    pub success: bool,
    pub message: String,
}

/// Install a plugin from ClawHub.
///
/// Maps to: `openclaw_clawhub_install`
/// Params: `plugin_id: String`
/// Response: `InstallResult`
///
/// This performs local validation and path resolution. The actual HTTP
/// fetch is done by the caller (Scrappy) since it has the reqwest client.
pub fn clawhub_prepare_install(
    cache: &CatalogCache,
    plugin_id: &str,
) -> Result<InstallResult, String> {
    // Look up in cache
    let entry = cache
        .entries()
        .iter()
        .find(|e| e.name == plugin_id)
        .cloned();

    match entry {
        Some(entry) => {
            let install_dir = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".ironclaw")
                .join("tools")
                .join(&entry.name);

            Ok(InstallResult {
                plugin_name: entry.name.clone(),
                version: entry.version.unwrap_or_else(|| "latest".to_string()),
                install_path: install_dir.to_string_lossy().to_string(),
                success: true,
                message: format!("Ready to install {}", entry.display_name),
            })
        }
        None => Err(format!("Plugin '{}' not found in catalog cache", plugin_id)),
    }
}

// ── 5. openclaw_routine_audit_list ────────────────────────────────────

/// Query routine audit log.
///
/// Maps to: `openclaw_routine_audit_list`
/// Params: `routine_key: String, limit: Option<u32>, outcome: Option<String>`
/// Response: `Vec<RoutineRun>`
pub fn routine_audit_list(
    log: &RoutineAuditLog,
    routine_key: &str,
    limit: Option<u32>,
    outcome_filter: Option<&str>,
) -> Result<Vec<RoutineRun>, String> {
    Ok(log
        .query_by_routine(routine_key, limit, outcome_filter)
        .into_iter()
        .cloned()
        .collect())
}

// ── 6. openclaw_cache_stats ───────────────────────────────────────────

/// Get response cache statistics.
///
/// Maps to: `openclaw_cache_stats`
/// Response: `CacheStats { hits, misses, evictions, size, hit_rate }`
pub fn cache_stats(store: &CachedResponseStore) -> Result<CacheStats, String> {
    Ok(store.stats())
}

// ── 7. openclaw_plugin_lifecycle_list ──────────────────────────────────

/// List plugin lifecycle events.
///
/// Maps to: `openclaw_plugin_lifecycle_list`
/// Response: `Vec<SerializedLifecycleEvent>`
pub fn plugin_lifecycle_list(hook: &AuditLogHook) -> Result<Vec<SerializedLifecycleEvent>, String> {
    Ok(hook.events_serialized())
}

// ── 8. openclaw_manifest_validate ─────────────────────────────────────

/// Validate a plugin manifest.
///
/// Maps to: `openclaw_manifest_validate`
/// Params: `plugin_id: String` (used to look up the plugin info)
/// Response: `ValidationResponse { errors: Vec<String>, warnings: Vec<String> }`
pub fn manifest_validate(
    validator: &ManifestValidator,
    info: &PluginInfoRef,
) -> Result<ValidationResponse, String> {
    let result = validator.validate(info);
    Ok(result.to_response())
}

// ── Convenience: list all available command names ──────────────────────

/// List all available Tauri command names from this facade.
pub fn available_commands() -> Vec<&'static str> {
    vec![
        "openclaw_cost_summary",
        "openclaw_cost_export_csv",
        "openclaw_clawhub_search",
        "openclaw_clawhub_install",
        "openclaw_routine_audit_list",
        "openclaw_cache_stats",
        "openclaw_plugin_lifecycle_list",
        "openclaw_manifest_validate",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::routine_audit::{RoutineOutcome, RoutineRun, TriggerKind};
    use crate::extensions::lifecycle_hooks::{LifecycleEvent, LifecycleHook};
    use crate::llm::cost_tracker::{BudgetConfig, CostEntry};
    use crate::llm::response_cache_ext::CacheConfig;

    #[test]
    fn test_cost_summary() {
        let mut tracker = CostTracker::new(BudgetConfig::default());
        tracker.record(CostEntry {
            cost_usd: 0.05,
            model: "gpt-4o".into(),
            agent_id: Some("default".into()),
            provider: "openai".into(),
            timestamp: "2026-03-04T12:00:00Z".into(),
            input_tokens: 100,
            output_tokens: 50,
            request_id: None,
        });
        let summary = cost_summary(&tracker).unwrap();
        assert!((summary.total_cost_usd - 0.05).abs() < 0.001);
        assert!(!summary.by_model.is_empty());
    }

    #[test]
    fn test_cost_export_csv() {
        let mut tracker = CostTracker::new(BudgetConfig::default());
        tracker.record(CostEntry {
            cost_usd: 0.10,
            model: "claude".into(),
            agent_id: Some("main".into()),
            provider: "anthropic".into(),
            timestamp: "2026-03-04T10:00:00Z".into(),
            input_tokens: 200,
            output_tokens: 100,
            request_id: None,
        });
        let csv = cost_export_csv(&tracker).unwrap();
        assert!(csv.contains("claude"));
        assert!(csv.contains("0.1"));
    }

    #[test]
    fn test_clawhub_search() {
        let mut cache = CatalogCache::new(3600);
        cache.update(vec![
            CatalogEntry {
                name: "slack-bot".into(),
                display_name: "Slack Bot".into(),
                kind: "channel".into(),
                description: "Slack integration".into(),
                keywords: vec!["chat".into()],
                version: Some("1.0.0".into()),
            },
            CatalogEntry {
                name: "weather".into(),
                display_name: "Weather Tool".into(),
                kind: "tool".into(),
                description: "Get weather data".into(),
                keywords: vec!["api".into()],
                version: None,
            },
        ]);

        let results = clawhub_search(&cache, "slack").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "slack-bot");

        let results = clawhub_search(&cache, "weather").unwrap();
        assert_eq!(results.len(), 1);

        let results = clawhub_search(&cache, "nonexistent").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_clawhub_prepare_install_found() {
        let mut cache = CatalogCache::new(3600);
        cache.update(vec![CatalogEntry {
            name: "my-plugin".into(),
            display_name: "My Plugin".into(),
            kind: "tool".into(),
            description: "Test".into(),
            keywords: vec![],
            version: Some("2.0.0".into()),
        }]);
        let result = clawhub_prepare_install(&cache, "my-plugin").unwrap();
        assert!(result.success);
        assert_eq!(result.plugin_name, "my-plugin");
        assert_eq!(result.version, "2.0.0");
        assert!(result.install_path.contains("my-plugin"));
    }

    #[test]
    fn test_clawhub_prepare_install_not_found() {
        let cache = CatalogCache::new(3600);
        let result = clawhub_prepare_install(&cache, "unknown");
        assert!(result.is_err());
    }

    #[test]
    fn test_routine_audit_list() {
        let mut log = RoutineAuditLog::new(100);
        log.push(RoutineRun {
            id: "run-1".into(),
            routine_name: "daily-backup".into(),
            started_at: "2026-03-04T01:00:00Z".into(),
            outcome: RoutineOutcome::Success { duration_ms: 500 },
            triggered_by: TriggerKind::Cron,
            job_id: None,
            agent_id: None,
        });
        log.push(RoutineRun {
            id: "run-2".into(),
            routine_name: "daily-backup".into(),
            started_at: "2026-03-04T02:00:00Z".into(),
            outcome: RoutineOutcome::Failed {
                error: "timeout".into(),
                duration_ms: 30000,
            },
            triggered_by: TriggerKind::Cron,
            job_id: None,
            agent_id: None,
        });

        let runs = routine_audit_list(&log, "daily-backup", None, None).unwrap();
        assert_eq!(runs.len(), 2);

        let successes = routine_audit_list(&log, "daily-backup", None, Some("success")).unwrap();
        assert_eq!(successes.len(), 1);
    }

    #[test]
    fn test_cache_stats_facade() {
        let mut store = CachedResponseStore::new(CacheConfig::default());
        store.set("k1", "r1".into(), "gpt-4o");
        store.get("k1"); // hit
        store.get("miss"); // miss

        let stats = cache_stats(&store).unwrap();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.size, 1);
    }

    #[test]
    fn test_plugin_lifecycle_list() {
        let hook = AuditLogHook::new();
        hook.on_event(&LifecycleEvent::Installed {
            name: "test-plugin".into(),
        });
        let events = plugin_lifecycle_list(&hook).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].plugin, "test-plugin");
        assert_eq!(events[0].event_type, "installed");
    }

    #[test]
    fn test_manifest_validate_valid() {
        let validator = ManifestValidator::new();
        let info = PluginInfoRef {
            name: "my-plugin".into(),
            version: Some("1.0.0".into()),
            description: Some("A test plugin".into()),
            permissions: vec!["network".into()],
            keywords: vec![],
            homepage_url: None,
        };
        let response = manifest_validate(&validator, &info).unwrap();
        assert!(response.errors.is_empty());
    }

    #[test]
    fn test_manifest_validate_invalid() {
        let validator = ManifestValidator::new();
        let info = PluginInfoRef {
            name: "".into(), // Empty name = error
            version: Some("not-semver".into()),
            description: None,
            permissions: vec!["unknown_perm".into()],
            keywords: vec![],
            homepage_url: None,
        };
        let response = manifest_validate(&validator, &info).unwrap();
        assert!(!response.errors.is_empty());
    }

    #[test]
    fn test_available_commands() {
        let cmds = available_commands();
        assert_eq!(cmds.len(), 8);
        assert!(cmds.contains(&"openclaw_cost_summary"));
        assert!(cmds.contains(&"openclaw_manifest_validate"));
    }

    #[test]
    fn test_install_result_serializable() {
        let result = InstallResult {
            plugin_name: "test".into(),
            version: "1.0.0".into(),
            install_path: "/path/to/plugin".into(),
            success: true,
            message: "OK".into(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"success\":true"));
        let deser: InstallResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.plugin_name, "test");
    }
}
