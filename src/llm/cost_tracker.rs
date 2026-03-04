//! LLM cost tracker with windowed budgets.
//!
//! Accumulates per-request cost records, provides daily/monthly
//! aggregation, per-agent/model grouping, budget alerts, and CSV export.

use std::collections::{HashMap, VecDeque};

/// A single cost entry.
#[derive(Debug, Clone)]
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
}
