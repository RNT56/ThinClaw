//! Unified Tauri command facade for IronClaw backend services.
//!
//! Provides the adapter functions that Scrappy's Tauri command stubs
//! should call. Each function maps directly to an `openclaw_*` command
//! from the §17.4 integration contract.
//!
//! # Usage from Scrappy `rpc.rs`
//!
//! ```rust,ignore
//! use thinclaw::tauri_commands;
//!
//! #[tauri::command]
//! fn openclaw_cost_summary(state: State<IronClawState>) -> Result<CostSummary, String> {
//!     tauri_commands::cost_summary(&state.cost_tracker)
//! }
//! ```

use crate::agent::routine::RoutineRun;
use crate::channels::gmail_wiring::GmailConfig;
use crate::cli::oauth_defaults::{self, GmailOAuthConfig};
use crate::extensions::clawhub::{CatalogCache, CatalogEntry};
use crate::extensions::lifecycle_hooks::{AuditLogHook, SerializedLifecycleEvent};
use crate::extensions::manifest_validator::{ManifestValidator, PluginInfoRef, ValidationResponse};
use crate::llm::cost_tracker::{CostSummary, CostTracker};
use crate::llm::response_cache_ext::{CacheStats, CachedResponseStore};
use crate::llm::routing_policy::{RoutingPolicy, RoutingRule, RoutingRuleSummary};

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

// ── 2b. openclaw_cost_reset ──────────────────────────────────────────

/// Clear all cost tracking data.
///
/// Maps to: `openclaw_cost_reset`
/// Response: `()`
///
/// The caller is responsible for persisting the empty state to the DB
/// via `SettingsStore::set_setting("default", "cost_entries", &tracker.to_json())`.
pub fn cost_reset(tracker: &mut CostTracker) -> Result<(), String> {
    let count = tracker.len();
    tracker.clear();
    tracing::info!("[cost] Reset: cleared {} entries", count);
    Ok(())
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

/// Query routine run history from the database.
///
/// `RoutineEngine` persists every run via `store.create_routine_run()` /
/// `store.complete_routine_run()`. This command reads that data back.
///
/// Maps to: `openclaw_routine_audit_list`
/// Params: `routine_name: String, user_id: String, limit: Option<i64>`
/// Response: `Vec<RoutineRun>`
pub async fn routine_audit_list(
    store: &dyn crate::db::Database,
    routine_name: &str,
    user_id: &str,
    limit: Option<i64>,
) -> Result<Vec<RoutineRun>, String> {
    // Look up the routine by name to get its UUID.
    let routine = store
        .get_routine_by_name(user_id, routine_name)
        .await
        .map_err(|e| format!("DB error looking up routine '{}': {}", routine_name, e))?
        .ok_or_else(|| {
            format!(
                "Routine '{}' not found for user '{}'",
                routine_name, user_id
            )
        })?;

    let runs = store
        .list_routine_runs(routine.id, limit.unwrap_or(20))
        .await
        .map_err(|e| format!("DB error listing runs: {}", e))?;

    Ok(runs)
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

// ── 9. openclaw_routing_rules_list ────────────────────────────────────

/// List all routing rules with human-readable descriptions.
///
/// Maps to: `openclaw_routing_rules_list`
/// Response: `Vec<RoutingRuleSummary>`
pub fn routing_rules_list(policy: &RoutingPolicy) -> Result<Vec<RoutingRuleSummary>, String> {
    Ok(RoutingRuleSummary::from_policy(policy))
}

// ── 10. openclaw_routing_rules_add ────────────────────────────────────

/// Add a routing rule at the end (or at a specific position).
///
/// Maps to: `openclaw_routing_rules_add`
/// Params: `rule: RoutingRule, position: Option<usize>`
/// Response: `Vec<RoutingRuleSummary>` (full updated list)
pub fn routing_rules_add(
    policy: &mut RoutingPolicy,
    rule: RoutingRule,
    position: Option<usize>,
) -> Result<Vec<RoutingRuleSummary>, String> {
    // Validate the rule before adding
    match &rule {
        RoutingRule::RoundRobin { providers } if providers.is_empty() => {
            return Err("Round-robin rule requires at least one provider".into());
        }
        RoutingRule::Fallback { fallbacks, .. } if fallbacks.is_empty() => {
            return Err("Fallback rule requires at least one fallback provider".into());
        }
        RoutingRule::LargeContext { threshold, .. } if *threshold == 0 => {
            return Err("Large context threshold must be greater than 0".into());
        }
        _ => {}
    }

    if let Some(pos) = position {
        if pos > policy.rule_count() {
            return Err(format!(
                "Position {} out of bounds (have {} rules)",
                pos,
                policy.rule_count()
            ));
        }
        // Insert at position: add at end then reorder
        policy.add_rule(rule);
        let last = policy.rule_count() - 1;
        if pos < last {
            policy.reorder_rules(last, pos).map_err(|e| e.to_string())?;
        }
    } else {
        policy.add_rule(rule);
    }

    Ok(RoutingRuleSummary::from_policy(policy))
}

// ── 11. openclaw_routing_rules_remove ─────────────────────────────────

/// Remove a routing rule by index.
///
/// Maps to: `openclaw_routing_rules_remove`
/// Params: `index: usize`
/// Response: `Vec<RoutingRuleSummary>` (full updated list)
pub fn routing_rules_remove(
    policy: &mut RoutingPolicy,
    index: usize,
) -> Result<Vec<RoutingRuleSummary>, String> {
    policy.remove_rule(index)?;
    Ok(RoutingRuleSummary::from_policy(policy))
}

// ── 12. openclaw_routing_rules_reorder ────────────────────────────────

/// Reorder a routing rule (move from one position to another).
///
/// Maps to: `openclaw_routing_rules_reorder`
/// Params: `from: usize, to: usize`
/// Response: `Vec<RoutingRuleSummary>` (full updated list)
pub fn routing_rules_reorder(
    policy: &mut RoutingPolicy,
    from: usize,
    to: usize,
) -> Result<Vec<RoutingRuleSummary>, String> {
    policy.reorder_rules(from, to)?;
    Ok(RoutingRuleSummary::from_policy(policy))
}

// ── 13. openclaw_routing_status ───────────────────────────────────────

/// Get full routing policy status for UI display.
///
/// Maps to: `openclaw_routing_status`
/// Response: `RoutingStatusResponse`
#[derive(Debug, Clone, serde::Serialize)]
pub struct RoutingStatusResponse {
    pub enabled: bool,
    pub default_provider: String,
    pub rule_count: usize,
    pub rules: Vec<RoutingRuleSummary>,
    pub latency_data: Vec<LatencyEntry>,
}

/// Per-provider latency data for UI display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LatencyEntry {
    pub provider: String,
    pub avg_latency_ms: f64,
}

pub fn routing_status(policy: &RoutingPolicy) -> Result<RoutingStatusResponse, String> {
    let tracker = policy.latency_tracker();
    let mut latency_data = Vec::new();

    // Collect all providers with latency data.
    // We check known providers by iterating rules for provider names.
    let mut providers: Vec<String> = Vec::new();
    for rule in policy.rules() {
        match rule {
            RoutingRule::LargeContext { provider, .. }
            | RoutingRule::VisionContent { provider }
                if !providers.contains(provider) =>
            {
                providers.push(provider.clone());
            }
            RoutingRule::Fallback {
                primary, fallbacks, ..
            } => {
                if !providers.contains(primary) {
                    providers.push(primary.clone());
                }
                for p in fallbacks {
                    if !providers.contains(p) {
                        providers.push(p.clone());
                    }
                }
            }
            RoutingRule::RoundRobin {
                providers: rr_providers,
            } => {
                for p in rr_providers {
                    if !providers.contains(p) {
                        providers.push(p.clone());
                    }
                }
            }
            _ => {}
        }
    }
    // Also add default provider.
    if !providers.contains(&policy.default_provider().to_string()) {
        providers.push(policy.default_provider().to_string());
    }

    for provider in &providers {
        if let Some(latency) = tracker.get_latency(provider) {
            latency_data.push(LatencyEntry {
                provider: provider.clone(),
                avg_latency_ms: latency,
            });
        }
    }

    Ok(RoutingStatusResponse {
        enabled: policy.is_enabled(),
        default_provider: policy.default_provider().to_string(),
        rule_count: policy.rule_count(),
        rules: RoutingRuleSummary::from_policy(policy),
        latency_data,
    })
}

// ── 14. openclaw_gmail_status ─────────────────────────────────────────

/// Get Gmail channel configuration status.
///
/// Maps to: `openclaw_gmail_status`
/// Response: `GmailStatusResponse`
#[derive(Debug, Clone, serde::Serialize)]
pub struct GmailStatusResponse {
    pub enabled: bool,
    pub configured: bool,
    pub status: String,
    pub project_id: String,
    pub subscription_id: String,
    pub label_filters: Vec<String>,
    pub allowed_senders: Vec<String>,
    pub missing_fields: Vec<String>,
    pub oauth_configured: bool,
}

pub fn gmail_status(config: &GmailConfig) -> Result<GmailStatusResponse, String> {
    use crate::channels::gmail_wiring::GmailStatus;

    let status = config.status();
    let status_str = match &status {
        GmailStatus::Disabled => "disabled".to_string(),
        GmailStatus::Ready { subscription } => format!("ready ({})", subscription),
        GmailStatus::MissingCredentials { fields } => {
            format!("missing credentials: {}", fields.join(", "))
        }
        GmailStatus::Error(e) => format!("error: {}", e),
    };

    Ok(GmailStatusResponse {
        enabled: config.enabled,
        configured: config.is_configured(),
        status: status_str,
        project_id: config.project_id.clone(),
        subscription_id: config.subscription_id.clone(),
        label_filters: config.label_filters.clone(),
        allowed_senders: config.allowed_senders.clone(),
        missing_fields: config.validate(),
        oauth_configured: config.oauth_token.is_some(),
    })
}

// ── 15. openclaw_gmail_oauth_start ────────────────────────────────────

/// Response from the Gmail OAuth PKCE flow.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GmailOAuthResult {
    pub success: bool,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

/// Start the Gmail OAuth PKCE flow.
///
/// This opens the user's browser for Google consent, waits for the
/// callback, exchanges the auth code for tokens, and returns them.
///
/// Maps to: `openclaw_gmail_oauth_start`
/// Response: `GmailOAuthResult`
pub async fn gmail_oauth_start() -> Result<GmailOAuthResult, String> {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use sha2::{Digest, Sha256};

    // Check that we have the built-in Google credentials.
    let creds = oauth_defaults::builtin_credentials("gmail_oauth_token").ok_or_else(|| {
        "Gmail OAuth credentials not available. Rebuild with IRONCLAW_GOOGLE_CLIENT_ID set."
            .to_string()
    })?;

    // Generate PKCE verifier and challenge.
    let mut verifier_bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut verifier_bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

    // Build the authorization URL.
    let auth_url = GmailOAuthConfig::auth_url("gmail-tauri", &code_challenge);

    // Open the browser.
    if let Err(e) = open::that(&auth_url) {
        tracing::warn!(error = %e, "Could not open browser for Gmail OAuth");
        return Err(format!(
            "Could not open browser. Please open this URL manually: {}",
            auth_url
        ));
    }

    tracing::info!("Gmail OAuth flow started — waiting for browser callback");

    // Bind the callback listener.
    let listener = oauth_defaults::bind_callback_listener()
        .await
        .map_err(|e| format!("Failed to bind OAuth callback listener: {}", e))?;

    // Wait for the callback (5 minute timeout).
    let code = oauth_defaults::wait_for_callback(listener, "/callback", "code", "Gmail")
        .await
        .map_err(|e| format!("OAuth callback failed: {}", e))?;

    tracing::info!("Gmail OAuth code received — exchanging for tokens");

    // Exchange the code for tokens.
    let client = reqwest::Client::new();
    let token_params = [
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &GmailOAuthConfig::redirect_uri()),
        ("code_verifier", &code_verifier),
    ];

    let token_response = client
        .post(GmailOAuthConfig::TOKEN_URL)
        .basic_auth(creds.client_id, Some(&creds.client_secret))
        .form(&token_params)
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {}", e))?;

    if !token_response.status().is_success() {
        let status = token_response.status();
        let body = token_response.text().await.unwrap_or_default();
        tracing::error!(
            status = %status,
            body = %body,
            "Gmail OAuth token exchange failed"
        );
        return Ok(GmailOAuthResult {
            success: false,
            access_token: None,
            refresh_token: None,
            expires_in: None,
            scope: None,
            error: Some(format!("Token exchange failed: {} - {}", status, body)),
        });
    }

    let token_data: serde_json::Value = token_response
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    let access_token = token_data
        .get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let refresh_token = token_data
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let expires_in = token_data.get("expires_in").and_then(|v| v.as_u64());

    let scope = token_data
        .get("scope")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if access_token.is_none() {
        return Ok(GmailOAuthResult {
            success: false,
            access_token: None,
            refresh_token: None,
            expires_in: None,
            scope: None,
            error: Some("Token exchange succeeded but no access_token in response".into()),
        });
    }

    tracing::info!("Gmail OAuth completed successfully");

    Ok(GmailOAuthResult {
        success: true,
        access_token,
        refresh_token,
        expires_in,
        scope,
        error: None,
    })
}

// ── Canvas panel commands ─────────────────────────────────────────────

/// Summary of a canvas panel for listing.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CanvasPanelSummary {
    pub panel_id: String,
    pub title: String,
}

/// Full panel data returned by the get command.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CanvasPanelData {
    pub panel_id: String,
    pub title: String,
    pub components: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
}

/// List all active canvas panels.
///
/// Maps to: `openclaw_canvas_panels_list`
/// Response: `Vec<CanvasPanelSummary>`
pub async fn canvas_panels_list(
    store: &crate::channels::canvas_gateway::CanvasStore,
) -> Result<Vec<CanvasPanelSummary>, String> {
    let panels = store.list().await;
    Ok(panels
        .into_iter()
        .map(|p| CanvasPanelSummary {
            panel_id: p.panel_id,
            title: p.title,
        })
        .collect())
}

/// Get full data for a specific canvas panel.
///
/// Maps to: `openclaw_canvas_panel_get`
/// Response: `Option<CanvasPanelData>`
pub async fn canvas_panel_get(
    store: &crate::channels::canvas_gateway::CanvasStore,
    panel_id: &str,
) -> Result<Option<CanvasPanelData>, String> {
    Ok(store.get(panel_id).await.map(|p| CanvasPanelData {
        panel_id: p.panel_id,
        title: p.title,
        components: p.components,
        metadata: p.metadata,
    }))
}

/// Dismiss (remove) a canvas panel.
///
/// Maps to: `openclaw_canvas_panel_dismiss`
/// Response: `bool` (true if panel existed)
pub async fn canvas_panel_dismiss(
    store: &crate::channels::canvas_gateway::CanvasStore,
    panel_id: &str,
) -> Result<bool, String> {
    Ok(store.dismiss(panel_id).await)
}

// ── 19. openclaw_channel_status_list ──────────────────────────────────

use crate::channels::ChannelManager;
use crate::channels::status_view::ChannelStatusEntry;

/// Get live channel status entries (message counters, uptime, state).
///
/// Maps to: `openclaw_channel_status_list`
/// Response: `Vec<ChannelStatusEntry>`
pub async fn channel_status_list(
    manager: &ChannelManager,
) -> Result<Vec<ChannelStatusEntry>, String> {
    Ok(manager.status_entries().await)
}

// ── 20. openclaw_routine_create ────────────────────────────────────────

use crate::agent::routine::{NotifyConfig, Routine, RoutineAction, RoutineGuardrails, Trigger};

/// Parameters for creating a new routine (Scrappy sends this from the UI).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoutineCreateParams {
    pub name: String,
    pub description: String,
    pub user_id: String,
    pub trigger: Trigger,
    pub action: RoutineAction,
    #[serde(default)]
    pub notify: Option<NotifyConfig>,
    /// Optional directory to write the routine file.
    /// Defaults to `~/.ironclaw/routines/`.
    pub routines_dir: Option<std::path::PathBuf>,
}

/// Create and persist a new routine as a JSON file.
///
/// Routines in IronClaw are file-backed (not database-backed).
/// Each routine is saved as `{id}.json` under the routines directory.
///
/// Maps to: `openclaw_routine_create`
/// Params: `RoutineCreateParams`
/// Response: `Routine` (the saved object with a generated ID)
pub fn routine_create(params: RoutineCreateParams) -> Result<Routine, String> {
    let now = chrono::Utc::now();
    let routine = Routine {
        id: uuid::Uuid::new_v4(),
        name: params.name,
        description: params.description,
        user_id: params.user_id,
        enabled: true,
        trigger: params.trigger,
        action: params.action,
        guardrails: RoutineGuardrails::default(),
        notify: params.notify.unwrap_or_default(),
        last_run_at: None,
        next_fire_at: None,
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::Value::Null,
        created_at: now,
        updated_at: now,
    };

    // Determine the routines directory.
    let dir = params.routines_dir.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".ironclaw")
            .join("routines")
    });

    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create routines directory: {}", e))?;

    let path = dir.join(format!("{}.json", routine.id));
    let json = serde_json::to_string_pretty(&routine)
        .map_err(|e| format!("Failed to serialize routine: {}", e))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write routine file {:?}: {}", path, e))?;

    tracing::info!(
        routine_id = %routine.id,
        routine_name = %routine.name,
        path = ?path,
        "Routine created"
    );

    Ok(routine)
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
        "openclaw_routing_rules_list",
        "openclaw_routing_rules_add",
        "openclaw_routing_rules_remove",
        "openclaw_routing_rules_reorder",
        "openclaw_routing_status",
        "openclaw_gmail_status",
        "openclaw_gmail_oauth_start",
        "openclaw_canvas_panels_list",
        "openclaw_canvas_panel_get",
        "openclaw_canvas_panel_dismiss",
        "openclaw_channel_status_list",
        "openclaw_routine_create",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    // RoutineRun tests removed — routine_audit_list now queries the DB (integration test needed)
    use crate::extensions::lifecycle_hooks::{LifecycleEvent, LifecycleHook};
    use crate::llm::cost_tracker::{BudgetConfig, CostEntry};
    use crate::llm::response_cache_ext::CacheConfig;
    use crate::llm::routing_policy::RoutingRule;

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

    // NOTE: test_routine_audit_list removed — the function now queries
    // the Database (async + requires a real or mock DB). The DB-backed
    // routine_run persistence is tested via routine_engine integration tests.

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
        assert_eq!(cmds.len(), 20);
        assert!(cmds.contains(&"openclaw_cost_summary"));
        assert!(cmds.contains(&"openclaw_manifest_validate"));
        assert!(cmds.contains(&"openclaw_routing_rules_list"));
        assert!(cmds.contains(&"openclaw_routing_status"));
        assert!(cmds.contains(&"openclaw_gmail_status"));
        assert!(cmds.contains(&"openclaw_gmail_oauth_start"));
        assert!(cmds.contains(&"openclaw_channel_status_list"));
        assert!(cmds.contains(&"openclaw_routine_create"));
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

    // ── Routing rule CRUD tests ───────────────────────────────────────

    #[test]
    fn test_routing_rules_list_empty() {
        let policy = RoutingPolicy::new("openai");
        let rules = routing_rules_list(&policy).unwrap();
        assert!(rules.is_empty());
    }

    #[test]
    fn test_routing_rules_list_with_rules() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::VisionContent {
            provider: "gemini".into(),
        });
        policy.add_rule(RoutingRule::LargeContext {
            threshold: 100_000,
            provider: "claude".into(),
        });
        let rules = routing_rules_list(&policy).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].rule_type, "vision");
        assert_eq!(rules[1].rule_type, "large_context");
        assert!(rules[1].description.contains("100000"));
    }

    #[test]
    fn test_routing_rules_add_at_end() {
        let mut policy = RoutingPolicy::new("openai");
        let rules = routing_rules_add(&mut policy, RoutingRule::LowestLatency, None).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].rule_type, "lowest_latency");
    }

    #[test]
    fn test_routing_rules_add_at_position() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::LowestLatency);
        policy.add_rule(RoutingRule::VisionContent {
            provider: "gemini".into(),
        });

        // Insert at position 0
        let rules = routing_rules_add(
            &mut policy,
            RoutingRule::LargeContext {
                threshold: 50000,
                provider: "claude".into(),
            },
            Some(0),
        )
        .unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].rule_type, "large_context");
        assert_eq!(rules[1].rule_type, "lowest_latency");
    }

    #[test]
    fn test_routing_rules_add_validation_empty_round_robin() {
        let mut policy = RoutingPolicy::new("openai");
        let result = routing_rules_add(
            &mut policy,
            RoutingRule::RoundRobin { providers: vec![] },
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least one provider"));
    }

    #[test]
    fn test_routing_rules_add_validation_zero_threshold() {
        let mut policy = RoutingPolicy::new("openai");
        let result = routing_rules_add(
            &mut policy,
            RoutingRule::LargeContext {
                threshold: 0,
                provider: "claude".into(),
            },
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("greater than 0"));
    }

    #[test]
    fn test_routing_rules_remove() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::LowestLatency);
        policy.add_rule(RoutingRule::VisionContent {
            provider: "gemini".into(),
        });
        let rules = routing_rules_remove(&mut policy, 0).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].rule_type, "vision");
    }

    #[test]
    fn test_routing_rules_remove_out_of_bounds() {
        let mut policy = RoutingPolicy::new("openai");
        let result = routing_rules_remove(&mut policy, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_routing_rules_reorder() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::LowestLatency);
        policy.add_rule(RoutingRule::VisionContent {
            provider: "gemini".into(),
        });
        policy.add_rule(RoutingRule::LargeContext {
            threshold: 100_000,
            provider: "claude".into(),
        });

        // Move last to first
        let rules = routing_rules_reorder(&mut policy, 2, 0).unwrap();
        assert_eq!(rules[0].rule_type, "large_context");
        assert_eq!(rules[1].rule_type, "lowest_latency");
        assert_eq!(rules[2].rule_type, "vision");
    }

    #[test]
    fn test_routing_status() {
        let mut policy = RoutingPolicy::new("openai");
        policy.add_rule(RoutingRule::VisionContent {
            provider: "gemini".into(),
        });
        policy.record_latency("openai", 200.0);
        policy.record_latency("gemini", 100.0);

        let status = routing_status(&policy).unwrap();
        assert!(status.enabled);
        assert_eq!(status.default_provider, "openai");
        assert_eq!(status.rule_count, 1);
        assert_eq!(status.rules.len(), 1);
        assert!(!status.latency_data.is_empty());
    }

    #[test]
    fn test_routing_status_serializable() {
        let policy = RoutingPolicy::new("openai");
        let status = routing_status(&policy).unwrap();
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"default_provider\":\"openai\""));
    }

    // ── Gmail status tests ────────────────────────────────────────────

    #[test]
    fn test_gmail_status_disabled() {
        let config = GmailConfig::default();
        let status = gmail_status(&config).unwrap();
        assert!(!status.enabled);
        assert!(!status.configured);
        assert_eq!(status.status, "disabled");
    }

    #[test]
    fn test_gmail_status_missing_creds() {
        let config = GmailConfig {
            enabled: true,
            ..Default::default()
        };
        let status = gmail_status(&config).unwrap();
        assert!(status.enabled);
        assert!(!status.configured);
        assert!(status.status.contains("missing credentials"));
        assert!(!status.missing_fields.is_empty());
    }

    #[test]
    fn test_gmail_status_ready() {
        let config = GmailConfig {
            enabled: true,
            project_id: "my-project".into(),
            subscription_id: "my-sub".into(),
            topic_id: "my-topic".into(),
            ..Default::default()
        };
        let status = gmail_status(&config).unwrap();
        assert!(status.enabled);
        assert!(status.configured);
        assert!(status.status.contains("ready"));
        assert!(status.missing_fields.is_empty());
    }

    #[test]
    fn test_gmail_status_serializable() {
        let config = GmailConfig::default();
        let status = gmail_status(&config).unwrap();
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"enabled\":false"));
        assert!(json.contains("\"configured\":false"));
    }
}
