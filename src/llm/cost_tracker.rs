//! LLM cost tracker with windowed budgets.
//!
//! Accumulates per-request cost records, provides daily/monthly
//! aggregation, per-agent/model grouping, budget alerts, and CSV export.
//!
//! **Persistence**: Call [`CostTracker::to_json()`] to get a serializable
//! snapshot and [`CostTracker::from_json()`] to restore entries.
//! The caller is responsible for persisting via `SettingsStore::set_setting()`
//! and loading via `SettingsStore::get_setting()` — this keeps the tracker
//! independent of any specific database backend.
//!
//! The [`CostSummary`] struct provides the serializable response shape
//! for the `openclaw_cost_summary` Tauri command (see §17.4 integration contract).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};

/// A single cost entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEntry {
    pub timestamp: String,
    pub agent_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
    pub request_id: Option<String>,
}

/// Budget configuration.
#[derive(Debug, Clone)]
pub struct BudgetConfig {
    pub daily_limit_usd: Option<f64>,
    pub monthly_limit_usd: Option<f64>,
    /// Alert when utilization exceeds this fraction (0.0-1.0).
    pub alert_threshold: f64,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            daily_limit_usd: None,
            monthly_limit_usd: None,
            alert_threshold: 0.9,
        }
    }
}

/// LLM cost tracker.
pub struct CostTracker {
    entries: VecDeque<CostEntry>,
    budget: BudgetConfig,
    max_entries: usize,
}

impl CostTracker {
    pub fn new(budget: BudgetConfig) -> Self {
        Self {
            entries: VecDeque::new(),
            budget,
            max_entries: 10_000,
        }
    }

    /// Record a cost entry, evicting oldest if at capacity.
    pub fn record(&mut self, entry: CostEntry) {
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Clear all entries (reset).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Total cost across all entries.
    pub fn total_cost(&self) -> f64 {
        self.entries.iter().map(|e| e.cost_usd).sum()
    }

    /// Cost for entries matching a date prefix (e.g., "2026-03-04").
    pub fn cost_for_date(&self, date_prefix: &str) -> f64 {
        self.entries
            .iter()
            .filter(|e| e.timestamp.starts_with(date_prefix))
            .map(|e| e.cost_usd)
            .sum()
    }

    /// Cost for entries matching a month prefix (e.g., "2026-03").
    pub fn cost_for_month(&self, month_prefix: &str) -> f64 {
        self.entries
            .iter()
            .filter(|e| e.timestamp.starts_with(month_prefix))
            .map(|e| e.cost_usd)
            .sum()
    }

    /// Group costs by agent.
    pub fn cost_by_agent(&self) -> HashMap<String, f64> {
        let mut map: HashMap<String, f64> = HashMap::new();
        for entry in &self.entries {
            let key = entry.agent_id.clone().unwrap_or_else(|| "unknown".into());
            *map.entry(key).or_insert(0.0) += entry.cost_usd;
        }
        map
    }

    /// Group costs by model.
    pub fn cost_by_model(&self) -> HashMap<String, f64> {
        let mut map: HashMap<String, f64> = HashMap::new();
        for entry in &self.entries {
            *map.entry(entry.model.clone()).or_insert(0.0) += entry.cost_usd;
        }
        map
    }

    /// Check if daily budget exceeded.
    pub fn is_over_daily_budget(&self, date: &str) -> bool {
        match self.budget.daily_limit_usd {
            Some(limit) => self.cost_for_date(date) > limit,
            None => false,
        }
    }

    /// Check if monthly budget exceeded.
    pub fn is_over_monthly_budget(&self, month: &str) -> bool {
        match self.budget.monthly_limit_usd {
            Some(limit) => self.cost_for_month(month) > limit,
            None => false,
        }
    }

    /// Daily budget utilization (0.0-1.0).
    pub fn budget_utilization(&self, date: &str) -> Option<f64> {
        self.budget
            .daily_limit_usd
            .map(|limit| self.cost_for_date(date) / limit)
    }

    /// Whether an alert should fire at the current daily utilization.
    pub fn should_alert(&self, date: &str) -> bool {
        match self.budget_utilization(date) {
            Some(util) => util >= self.budget.alert_threshold,
            None => false,
        }
    }

    /// Export all entries as CSV.
    pub fn export_csv(&self) -> String {
        let mut out = String::from(
            "timestamp,agent_id,provider,model,input_tokens,output_tokens,cost_usd,request_id\n",
        );
        for e in &self.entries {
            out.push_str(&format!(
                "{},{},{},{},{},{},{:.6},{}\n",
                e.timestamp,
                e.agent_id.as_deref().unwrap_or(""),
                e.provider,
                e.model,
                e.input_tokens,
                e.output_tokens,
                e.cost_usd,
                e.request_id.as_deref().unwrap_or(""),
            ));
        }
        out
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total input tokens across all entries.
    pub fn total_input_tokens(&self) -> u64 {
        self.entries.iter().map(|e| e.input_tokens as u64).sum()
    }

    /// Total output tokens across all entries.
    pub fn total_output_tokens(&self) -> u64 {
        self.entries.iter().map(|e| e.output_tokens as u64).sum()
    }

    /// Serialize all entries to a JSON value for DB persistence.
    ///
    /// Store the result via `SettingsStore::set_setting("default", "cost_entries", &value)`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.entries).unwrap_or_default()
    }

    /// Restore entries from a JSON value loaded from the DB.
    ///
    /// Typically called with the result of `SettingsStore::get_setting("default", "cost_entries")`.
    /// Invalid or missing data is silently ignored (starts fresh).
    pub fn from_json(&mut self, value: &serde_json::Value) {
        if let Ok(entries) = serde_json::from_value::<Vec<CostEntry>>(value.clone()) {
            self.entries = VecDeque::from(entries);
            // Trim to max_entries
            while self.entries.len() > self.max_entries {
                self.entries.pop_front();
            }
            tracing::info!(
                "[cost] Restored {} entries from database",
                self.entries.len()
            );
        }
    }

    /// Build a serializable summary matching the `openclaw_cost_summary` response shape.
    ///
    /// Aggregates totals, daily/monthly breakdowns, per-model/per-agent groupings,
    /// and alert state into one response.
    pub fn summary(&self, today: &str, _this_month: &str) -> CostSummary {
        let mut daily: BTreeMap<String, f64> = BTreeMap::new();
        let mut monthly: BTreeMap<String, f64> = BTreeMap::new();

        for e in &self.entries {
            // date key: first 10 chars "2026-03-04"
            if e.timestamp.len() >= 10 {
                let date_key = &e.timestamp[..10];
                *daily.entry(date_key.to_string()).or_insert(0.0) += e.cost_usd;
            }
            // month key: first 7 chars "2026-03"
            if e.timestamp.len() >= 7 {
                let month_key = &e.timestamp[..7];
                *monthly.entry(month_key.to_string()).or_insert(0.0) += e.cost_usd;
            }
        }

        let total_cost = self.total_cost();
        let total_requests = self.entries.len() as u64;

        CostSummary {
            total_cost_usd: total_cost,
            total_input_tokens: self.total_input_tokens(),
            total_output_tokens: self.total_output_tokens(),
            total_requests,
            avg_cost_per_request: if total_requests > 0 {
                total_cost / total_requests as f64
            } else {
                0.0
            },
            daily,
            monthly,
            by_model: self.cost_by_model().into_iter().collect::<BTreeMap<_, _>>(),
            by_agent: self.cost_by_agent().into_iter().collect::<BTreeMap<_, _>>(),
            alert_threshold_usd: self.budget.daily_limit_usd,
            alert_triggered: self.should_alert(today),
        }
    }
}

/// Serializable cost summary for the `openclaw_cost_summary` Tauri command.
///
/// Matches the response shape agreed in §17.4 integration contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostSummary {
    pub total_cost_usd: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_requests: u64,
    pub avg_cost_per_request: f64,
    pub daily: BTreeMap<String, f64>,
    pub monthly: BTreeMap<String, f64>,
    pub by_model: BTreeMap<String, f64>,
    pub by_agent: BTreeMap<String, f64>,
    pub alert_threshold_usd: Option<f64>,
    pub alert_triggered: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(cost: f64, date: &str, model: &str) -> CostEntry {
        CostEntry {
            timestamp: date.into(),
            agent_id: Some("agent-1".into()),
            provider: "openai".into(),
            model: model.into(),
            input_tokens: 100,
            output_tokens: 200,
            cost_usd: cost,
            request_id: None,
        }
    }

    #[test]
    fn test_record_and_total() {
        let mut tracker = CostTracker::new(BudgetConfig::default());
        tracker.record(make_entry(0.01, "2026-03-04T10:00:00Z", "gpt-4o"));
        tracker.record(make_entry(0.02, "2026-03-04T10:01:00Z", "gpt-4o"));
        assert!((tracker.total_cost() - 0.03).abs() < 1e-6);
    }

    #[test]
    fn test_cost_for_date() {
        let mut tracker = CostTracker::new(BudgetConfig::default());
        tracker.record(make_entry(0.01, "2026-03-04T10:00:00Z", "m"));
        tracker.record(make_entry(0.05, "2026-03-05T10:00:00Z", "m"));
        assert!((tracker.cost_for_date("2026-03-04") - 0.01).abs() < 1e-6);
    }

    #[test]
    fn test_cost_by_agent() {
        let mut tracker = CostTracker::new(BudgetConfig::default());
        tracker.record(make_entry(0.01, "t1", "m"));
        let by_agent = tracker.cost_by_agent();
        assert!(by_agent.contains_key("agent-1"));
    }

    #[test]
    fn test_cost_by_model() {
        let mut tracker = CostTracker::new(BudgetConfig::default());
        tracker.record(make_entry(0.01, "t1", "gpt-4o"));
        tracker.record(make_entry(0.02, "t2", "claude"));
        let by_model = tracker.cost_by_model();
        assert_eq!(by_model.len(), 2);
    }

    #[test]
    fn test_daily_budget_exceeded() {
        let budget = BudgetConfig {
            daily_limit_usd: Some(0.05),
            ..Default::default()
        };
        let mut tracker = CostTracker::new(budget);
        tracker.record(make_entry(0.06, "2026-03-04T10:00:00Z", "m"));
        assert!(tracker.is_over_daily_budget("2026-03-04"));
    }

    #[test]
    fn test_monthly_budget_exceeded() {
        let budget = BudgetConfig {
            monthly_limit_usd: Some(1.0),
            ..Default::default()
        };
        let mut tracker = CostTracker::new(budget);
        tracker.record(make_entry(1.5, "2026-03-04T10:00:00Z", "m"));
        assert!(tracker.is_over_monthly_budget("2026-03"));
    }

    #[test]
    fn test_budget_utilization() {
        let budget = BudgetConfig {
            daily_limit_usd: Some(1.0),
            ..Default::default()
        };
        let mut tracker = CostTracker::new(budget);
        tracker.record(make_entry(0.5, "2026-03-04T10:00:00Z", "m"));
        let util = tracker.budget_utilization("2026-03-04").unwrap();
        assert!((util - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_should_alert_at_threshold() {
        let budget = BudgetConfig {
            daily_limit_usd: Some(1.0),
            alert_threshold: 0.8,
            ..Default::default()
        };
        let mut tracker = CostTracker::new(budget);
        tracker.record(make_entry(0.85, "2026-03-04T10:00:00Z", "m"));
        assert!(tracker.should_alert("2026-03-04"));
    }

    #[test]
    fn test_export_csv() {
        let mut tracker = CostTracker::new(BudgetConfig::default());
        tracker.record(make_entry(0.01, "2026-03-04T10:00:00Z", "gpt-4o"));
        let csv = tracker.export_csv();
        assert!(csv.starts_with("timestamp,"));
        assert!(csv.contains("gpt-4o"));
    }

    #[test]
    fn test_no_budget() {
        let tracker = CostTracker::new(BudgetConfig::default());
        assert!(!tracker.is_over_daily_budget("2026-03-04"));
        assert!(!tracker.is_over_monthly_budget("2026-03"));
        assert!(!tracker.should_alert("2026-03-04"));
    }

    #[test]
    fn test_summary_basic() {
        let budget = BudgetConfig {
            daily_limit_usd: Some(10.0),
            alert_threshold: 0.8,
            ..Default::default()
        };
        let mut tracker = CostTracker::new(budget);
        tracker.record(make_entry(1.0, "2026-03-04T10:00:00Z", "gpt-4o"));
        tracker.record(make_entry(2.0, "2026-03-04T11:00:00Z", "claude"));
        tracker.record(make_entry(3.0, "2026-03-05T10:00:00Z", "gpt-4o"));

        let summary = tracker.summary("2026-03-04", "2026-03");
        assert!((summary.total_cost_usd - 6.0).abs() < 1e-6);
        assert_eq!(summary.daily.len(), 2);
        assert!((summary.daily["2026-03-04"] - 3.0).abs() < 1e-6);
        assert!((summary.daily["2026-03-05"] - 3.0).abs() < 1e-6);
        assert_eq!(summary.monthly.len(), 1);
        assert!((summary.monthly["2026-03"] - 6.0).abs() < 1e-6);
        assert_eq!(summary.by_model.len(), 2);
        assert!((summary.by_model["gpt-4o"] - 4.0).abs() < 1e-6);
        assert!((summary.by_model["claude"] - 2.0).abs() < 1e-6);
        assert_eq!(summary.alert_threshold_usd, Some(10.0));
        assert!(!summary.alert_triggered);
    }

    #[test]
    fn test_summary_alert_triggered() {
        let budget = BudgetConfig {
            daily_limit_usd: Some(1.0),
            alert_threshold: 0.8,
            ..Default::default()
        };
        let mut tracker = CostTracker::new(budget);
        tracker.record(make_entry(0.9, "2026-03-04T10:00:00Z", "m"));
        let summary = tracker.summary("2026-03-04", "2026-03");
        assert!(summary.alert_triggered);
    }

    #[test]
    fn test_summary_serializable() {
        let tracker = CostTracker::new(BudgetConfig::default());
        let summary = tracker.summary("2026-03-04", "2026-03");
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("total_cost_usd"));
        assert!(json.contains("alert_triggered"));
    }
}
