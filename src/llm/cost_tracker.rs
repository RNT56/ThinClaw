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
//! When the live entry buffer reaches `max_entries`, oldest entries are
//! evicted but their aggregates (daily/monthly totals, per-model/agent
//! breakdowns) are preserved in [`CompactedStats`] so that all-time
//! summaries remain accurate.
//!
//! The [`CostSummary`] struct provides the serializable response shape
//! for the `openclaw_cost_summary` Tauri command (see §17.4 integration contract).

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::llm::ProviderTokenCapture;

/// Where the billable token counts came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenCountSource {
    /// Token counts were reported by the provider usage fields.
    ProviderUsage,
    /// No token counts were available, usually because the request failed
    /// before a provider response was produced.
    None,
    /// Legacy entry loaded before source provenance was tracked.
    Unknown,
}

impl TokenCountSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProviderUsage => "provider_usage",
            Self::None => "none",
            Self::Unknown => "unknown",
        }
    }
}

fn default_token_count_source() -> TokenCountSource {
    TokenCountSource::Unknown
}

/// Where the dollar cost came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostSource {
    /// Provider response included an authoritative request cost.
    ProviderCost,
    /// Provider omitted dollar cost, so ThinClaw used local model pricing
    /// against provider-reported token counts.
    LocalPricingFallback,
    /// No cost was recorded, usually for a failed request.
    None,
    /// Legacy entry loaded before source provenance was tracked.
    Unknown,
}

impl CostSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProviderCost => "provider_cost",
            Self::LocalPricingFallback => "local_pricing_fallback",
            Self::None => "none",
            Self::Unknown => "unknown",
        }
    }
}

fn default_cost_source() -> CostSource {
    CostSource::Unknown
}

/// Compact summary of provider-native exact token/logprob capture.
///
/// The cost dashboard stores counts only, not raw token text, so it can surface
/// provenance without bloating persisted cost history.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenCaptureCostSummary {
    #[serde(default)]
    pub exact_tokens_supported: bool,
    #[serde(default)]
    pub logprobs_supported: bool,
    #[serde(default)]
    pub tokens: u32,
    #[serde(default)]
    pub token_ids: u32,
    #[serde(default)]
    pub logprobs: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl TokenCaptureCostSummary {
    pub fn from_capture(capture: &ProviderTokenCapture) -> Self {
        Self {
            exact_tokens_supported: capture.exact_tokens_supported,
            logprobs_supported: capture.logprobs_supported,
            tokens: capture.tokens.len() as u32,
            token_ids: capture.token_ids.len() as u32,
            logprobs: capture.logprobs.len() as u32,
            provider: capture.provider.clone(),
            model: capture.model.clone(),
        }
    }

    pub fn output_token_count(&self) -> u64 {
        u64::from(self.tokens.max(self.token_ids))
    }
}

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
    /// Provenance for `input_tokens` and `output_tokens`.
    #[serde(default = "default_token_count_source")]
    pub token_count_source: TokenCountSource,
    /// Provenance for `cost_usd`.
    #[serde(default = "default_cost_source")]
    pub cost_source: CostSource,
    /// Provider-native exact-token/logprob capture metadata, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_capture: Option<TokenCaptureCostSummary>,
}

impl CostEntry {
    fn captured_output_tokens(&self) -> u64 {
        self.token_capture
            .as_ref()
            .map(TokenCaptureCostSummary::output_token_count)
            .unwrap_or(0)
    }

    fn captured_token_ids(&self) -> u64 {
        self.token_capture
            .as_ref()
            .map(|capture| u64::from(capture.token_ids))
            .unwrap_or(0)
    }

    fn captured_logprobs(&self) -> u64 {
        self.token_capture
            .as_ref()
            .map(|capture| u64::from(capture.logprobs))
            .unwrap_or(0)
    }
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

/// Rolled-up statistics from evicted entries that exceed the live buffer.
///
/// Preserves daily/monthly cost totals, per-model token breakdowns, and
/// per-agent cost totals so that all-time summaries remain accurate even
/// after the oldest detailed entries have been evicted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompactedStats {
    pub daily: BTreeMap<String, f64>,
    pub monthly: BTreeMap<String, f64>,
    pub by_model: HashMap<String, CompactedModelEntry>,
    pub by_agent: HashMap<String, f64>,
    pub total_cost: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_requests: u64,
    #[serde(default)]
    pub token_count_sources: BTreeMap<String, u64>,
    #[serde(default)]
    pub cost_sources: BTreeMap<String, u64>,
    #[serde(default)]
    pub captured_output_tokens: u64,
    #[serde(default)]
    pub captured_token_ids: u64,
    #[serde(default)]
    pub captured_logprobs: u64,
    #[serde(default)]
    pub token_capture_requests: u64,
}

/// Per-model aggregate from compacted (evicted) entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompactedModelEntry {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub requests: u64,
    #[serde(default)]
    pub provider_usage_requests: u64,
    #[serde(default)]
    pub unknown_token_count_requests: u64,
    #[serde(default)]
    pub provider_cost_requests: u64,
    #[serde(default)]
    pub local_pricing_fallback_requests: u64,
    #[serde(default)]
    pub captured_output_tokens: u64,
    #[serde(default)]
    pub captured_token_ids: u64,
    #[serde(default)]
    pub captured_logprobs: u64,
    #[serde(default)]
    pub token_capture_requests: u64,
}

impl CompactedModelEntry {
    fn add_sources(&mut self, entry: &CostEntry) {
        match entry.token_count_source {
            TokenCountSource::ProviderUsage => self.provider_usage_requests += 1,
            TokenCountSource::Unknown => self.unknown_token_count_requests += 1,
            TokenCountSource::None => {}
        }
        match entry.cost_source {
            CostSource::ProviderCost => self.provider_cost_requests += 1,
            CostSource::LocalPricingFallback => self.local_pricing_fallback_requests += 1,
            CostSource::None | CostSource::Unknown => {}
        }
        self.captured_output_tokens += entry.captured_output_tokens();
        self.captured_token_ids += entry.captured_token_ids();
        self.captured_logprobs += entry.captured_logprobs();
        if entry.token_capture.is_some() {
            self.token_capture_requests += 1;
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ModelBreakdownAccumulator {
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
    requests: u64,
    provider_usage_requests: u64,
    unknown_token_count_requests: u64,
    provider_cost_requests: u64,
    local_pricing_fallback_requests: u64,
    captured_output_tokens: u64,
    captured_token_ids: u64,
    captured_logprobs: u64,
    token_capture_requests: u64,
}

impl ModelBreakdownAccumulator {
    fn add_entry(&mut self, entry: &CostEntry) {
        self.input_tokens += entry.input_tokens as u64;
        self.output_tokens += entry.output_tokens as u64;
        self.cost_usd += entry.cost_usd;
        self.requests += 1;
        match entry.token_count_source {
            TokenCountSource::ProviderUsage => self.provider_usage_requests += 1,
            TokenCountSource::Unknown => self.unknown_token_count_requests += 1,
            TokenCountSource::None => {}
        }
        match entry.cost_source {
            CostSource::ProviderCost => self.provider_cost_requests += 1,
            CostSource::LocalPricingFallback => self.local_pricing_fallback_requests += 1,
            CostSource::None | CostSource::Unknown => {}
        }
        self.captured_output_tokens += entry.captured_output_tokens();
        self.captured_token_ids += entry.captured_token_ids();
        self.captured_logprobs += entry.captured_logprobs();
        if entry.token_capture.is_some() {
            self.token_capture_requests += 1;
        }
    }

    fn add_compacted(&mut self, entry: &CompactedModelEntry) {
        self.input_tokens += entry.input_tokens;
        self.output_tokens += entry.output_tokens;
        self.cost_usd += entry.cost_usd;
        self.requests += entry.requests;
        self.provider_usage_requests += entry.provider_usage_requests;
        self.unknown_token_count_requests += entry.unknown_token_count_requests;
        self.provider_cost_requests += entry.provider_cost_requests;
        self.local_pricing_fallback_requests += entry.local_pricing_fallback_requests;
        self.captured_output_tokens += entry.captured_output_tokens;
        self.captured_token_ids += entry.captured_token_ids;
        self.captured_logprobs += entry.captured_logprobs;
        self.token_capture_requests += entry.token_capture_requests;
    }

    fn into_breakdown(self, model: String) -> ModelBreakdown {
        ModelBreakdown {
            model,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cost_usd: self.cost_usd,
            requests: self.requests,
            provider_usage_requests: self.provider_usage_requests,
            unknown_token_count_requests: self.unknown_token_count_requests,
            provider_cost_requests: self.provider_cost_requests,
            local_pricing_fallback_requests: self.local_pricing_fallback_requests,
            captured_output_tokens: self.captured_output_tokens,
            captured_token_ids: self.captured_token_ids,
            captured_logprobs: self.captured_logprobs,
            token_capture_requests: self.token_capture_requests,
        }
    }
}

/// LLM cost tracker.
pub struct CostTracker {
    entries: VecDeque<CostEntry>,
    budget: BudgetConfig,
    max_entries: usize,
    compacted: CompactedStats,
}

impl CostTracker {
    pub fn new(budget: BudgetConfig) -> Self {
        Self {
            entries: VecDeque::new(),
            budget,
            max_entries: 50_000,
            compacted: CompactedStats::default(),
        }
    }

    /// Set the maximum number of live entries before compaction kicks in.
    pub fn with_max_entries(mut self, max: usize) -> Self {
        self.max_entries = max;
        self
    }

    /// Record a cost entry, compacting oldest if at capacity.
    pub fn record(&mut self, entry: CostEntry) {
        if self.entries.len() >= self.max_entries
            && let Some(evicted) = self.entries.pop_front()
        {
            self.compact_entry(&evicted);
        }
        self.entries.push_back(entry);
    }

    /// Roll an evicted entry's data into compacted aggregates.
    fn compact_entry(&mut self, entry: &CostEntry) {
        if let Some(date_key) = entry.timestamp.get(..10) {
            *self
                .compacted
                .daily
                .entry(date_key.to_string())
                .or_insert(0.0) += entry.cost_usd;
        }
        if let Some(month_key) = entry.timestamp.get(..7) {
            *self
                .compacted
                .monthly
                .entry(month_key.to_string())
                .or_insert(0.0) += entry.cost_usd;
        }
        let me = self
            .compacted
            .by_model
            .entry(entry.model.clone())
            .or_default();
        me.input_tokens += entry.input_tokens as u64;
        me.output_tokens += entry.output_tokens as u64;
        me.cost_usd += entry.cost_usd;
        me.requests += 1;
        me.add_sources(entry);
        let agent_key = entry.agent_id.clone().unwrap_or_else(|| "unknown".into());
        *self.compacted.by_agent.entry(agent_key).or_insert(0.0) += entry.cost_usd;
        self.compacted.total_cost += entry.cost_usd;
        self.compacted.total_input_tokens += entry.input_tokens as u64;
        self.compacted.total_output_tokens += entry.output_tokens as u64;
        self.compacted.total_requests += 1;
        *self
            .compacted
            .token_count_sources
            .entry(entry.token_count_source.as_str().to_string())
            .or_insert(0) += 1;
        *self
            .compacted
            .cost_sources
            .entry(entry.cost_source.as_str().to_string())
            .or_insert(0) += 1;
        self.compacted.captured_output_tokens += entry.captured_output_tokens();
        self.compacted.captured_token_ids += entry.captured_token_ids();
        self.compacted.captured_logprobs += entry.captured_logprobs();
        if entry.token_capture.is_some() {
            self.compacted.token_capture_requests += 1;
        }
    }

    /// Clear all entries and compacted aggregates (full reset).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.compacted = CompactedStats::default();
    }

    /// Number of live (non-compacted) entries.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Maximum live entries before compaction kicks in.
    pub fn max_entries(&self) -> usize {
        self.max_entries
    }

    /// Total cost across all entries (live + compacted).
    pub fn total_cost(&self) -> f64 {
        let live: f64 = self.entries.iter().map(|e| e.cost_usd).sum();
        live + self.compacted.total_cost
    }

    /// Cost for entries matching a date prefix (e.g., "2026-03-04").
    /// Includes both live entries and compacted daily aggregates.
    pub fn cost_for_date(&self, date_prefix: &str) -> f64 {
        let live: f64 = self
            .entries
            .iter()
            .filter(|e| e.timestamp.starts_with(date_prefix))
            .map(|e| e.cost_usd)
            .sum();
        let compacted = self
            .compacted
            .daily
            .get(date_prefix)
            .copied()
            .unwrap_or(0.0);
        live + compacted
    }

    /// Cost for entries matching a month prefix (e.g., "2026-03").
    /// Includes both live entries and compacted monthly aggregates.
    pub fn cost_for_month(&self, month_prefix: &str) -> f64 {
        let live: f64 = self
            .entries
            .iter()
            .filter(|e| e.timestamp.starts_with(month_prefix))
            .map(|e| e.cost_usd)
            .sum();
        let compacted = self
            .compacted
            .monthly
            .get(month_prefix)
            .copied()
            .unwrap_or(0.0);
        live + compacted
    }

    /// Group costs by agent (live + compacted).
    pub fn cost_by_agent(&self) -> HashMap<String, f64> {
        let mut map: HashMap<String, f64> = self.compacted.by_agent.clone();
        for entry in &self.entries {
            let key = entry.agent_id.clone().unwrap_or_else(|| "unknown".into());
            *map.entry(key).or_insert(0.0) += entry.cost_usd;
        }
        map
    }

    /// Group costs by model (live + compacted).
    pub fn cost_by_model(&self) -> HashMap<String, f64> {
        let mut map: HashMap<String, f64> = self
            .compacted
            .by_model
            .iter()
            .map(|(k, v)| (k.clone(), v.cost_usd))
            .collect();
        for entry in &self.entries {
            *map.entry(entry.model.clone()).or_insert(0.0) += entry.cost_usd;
        }
        map
    }

    /// Per-model breakdown with token counts and cost (live entries only).
    ///
    /// Optionally filtered by date prefix (e.g. `Some("2026-04-05")` for today
    /// or `Some("2026-04")` for the month).  Pass `None` for all-time.
    pub fn model_breakdown(&self, date_prefix: Option<&str>) -> Vec<ModelBreakdown> {
        self.collect_model_breakdown(|e| match date_prefix {
            Some(prefix) => e.timestamp.starts_with(prefix),
            None => true,
        })
    }

    /// Per-model breakdown for an explicit set of UTC date keys (`YYYY-MM-DD`).
    pub fn model_breakdown_for_dates(&self, date_keys: &HashSet<String>) -> Vec<ModelBreakdown> {
        self.collect_model_breakdown(|e| match e.timestamp.get(..10) {
            Some(date_key) => date_keys.contains(date_key),
            None => false,
        })
    }

    fn collect_model_breakdown<F>(&self, mut include: F) -> Vec<ModelBreakdown>
    where
        F: FnMut(&CostEntry) -> bool,
    {
        let mut map: HashMap<String, ModelBreakdownAccumulator> = HashMap::new();
        for e in &self.entries {
            if !include(e) {
                continue;
            }
            map.entry(e.model.clone()).or_default().add_entry(e);
        }
        let mut result: Vec<ModelBreakdown> = map
            .into_iter()
            .map(|(model, acc)| acc.into_breakdown(model))
            .collect();
        result.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        result
    }

    fn recent_date_keys(today: &str, days: usize) -> Option<HashSet<String>> {
        let today = NaiveDate::parse_from_str(today, "%Y-%m-%d").ok()?;
        let mut keys = HashSet::with_capacity(days);
        for offset in 0..days {
            let date = today - Duration::days(offset as i64);
            keys.insert(date.format("%Y-%m-%d").to_string());
        }
        Some(keys)
    }

    /// Count live entries that fall within the given trailing window.
    ///
    /// This is used to restore real-time `CostGuard` rate limits after a
    /// restart. Compacted entries are excluded because their exact timestamps
    /// are intentionally discarded.
    pub fn recent_action_count(&self, now: DateTime<Utc>, window: Duration) -> u64 {
        let cutoff = now - window;
        self.entries
            .iter()
            .filter_map(|entry| {
                chrono::DateTime::parse_from_rfc3339(&entry.timestamp)
                    .ok()
                    .map(|ts| ts.with_timezone(&Utc))
            })
            .filter(|ts| *ts >= cutoff)
            .count() as u64
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

    /// Export all live entries as CSV.
    ///
    /// All string fields are escaped per RFC 4180 to prevent CSV injection
    /// and malformed output from fields containing commas, quotes, or newlines.
    pub fn export_csv(&self) -> String {
        let mut out = String::from(
            "timestamp,agent_id,provider,model,input_tokens,output_tokens,cost_usd,request_id,token_count_source,cost_source,captured_output_tokens,captured_token_ids,captured_logprobs\n",
        );
        for e in &self.entries {
            out.push_str(&format!(
                "{},{},{},{},{},{},{:.6},{},{},{},{},{},{}\n",
                csv_escape(&e.timestamp),
                csv_escape(e.agent_id.as_deref().unwrap_or("")),
                csv_escape(&e.provider),
                csv_escape(&e.model),
                e.input_tokens,
                e.output_tokens,
                e.cost_usd,
                csv_escape(e.request_id.as_deref().unwrap_or("")),
                csv_escape(e.token_count_source.as_str()),
                csv_escape(e.cost_source.as_str()),
                e.captured_output_tokens(),
                e.captured_token_ids(),
                e.captured_logprobs(),
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

    /// Total input tokens across all entries (live + compacted).
    pub fn total_input_tokens(&self) -> u64 {
        let live: u64 = self.entries.iter().map(|e| e.input_tokens as u64).sum();
        live + self.compacted.total_input_tokens
    }

    /// Total output tokens across all entries (live + compacted).
    pub fn total_output_tokens(&self) -> u64 {
        let live: u64 = self.entries.iter().map(|e| e.output_tokens as u64).sum();
        live + self.compacted.total_output_tokens
    }

    /// Serialize entries + compacted aggregates to a JSON value for DB persistence.
    ///
    /// Store the result via `SettingsStore::set_setting("default", "cost_entries", &value)`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "entries": self.entries,
            "compacted": self.compacted,
        })
    }

    /// Restore entries from a JSON value loaded from the DB.
    ///
    /// Supports both the new format `{ "entries": [...], "compacted": {...} }`
    /// and the legacy format (plain array of entries). Invalid or missing data
    /// is silently ignored (starts fresh).
    pub fn from_json(&mut self, value: &serde_json::Value) {
        if let Some(obj) = value.as_object() {
            // New format: load compacted first so trim-compaction merges correctly.
            if let Some(compacted_val) = obj.get("compacted")
                && let Ok(compacted) =
                    serde_json::from_value::<CompactedStats>(compacted_val.clone())
            {
                self.compacted = compacted;
            }
            if let Some(entries_val) = obj.get("entries")
                && let Ok(entries) = serde_json::from_value::<Vec<CostEntry>>(entries_val.clone())
            {
                self.entries = VecDeque::from(entries);
                while self.entries.len() > self.max_entries {
                    if let Some(evicted) = self.entries.pop_front() {
                        self.compact_entry(&evicted);
                    }
                }
            }
            tracing::info!(
                "[cost] Restored {} entries + {} compacted requests from database",
                self.entries.len(),
                self.compacted.total_requests,
            );
        } else if let Ok(entries) = serde_json::from_value::<Vec<CostEntry>>(value.clone()) {
            // Legacy format: plain array of entries.
            self.entries = VecDeque::from(entries);
            while self.entries.len() > self.max_entries {
                if let Some(evicted) = self.entries.pop_front() {
                    self.compact_entry(&evicted);
                }
            }
            tracing::info!(
                "[cost] Restored {} entries from database (legacy format)",
                self.entries.len()
            );
        }
    }

    /// Build a serializable summary matching the `openclaw_cost_summary` response shape.
    ///
    /// Aggregates totals, daily/monthly breakdowns, per-model/per-agent groupings,
    /// and alert state into one response. All-time fields merge compacted + live data.
    pub fn summary(&self, today: &str, this_month: &str) -> CostSummary {
        // Start with compacted daily/monthly, then layer live entries on top.
        let mut daily: BTreeMap<String, f64> = self.compacted.daily.clone();
        let mut monthly: BTreeMap<String, f64> = self.compacted.monthly.clone();
        let mut token_count_sources = self.compacted.token_count_sources.clone();
        let mut cost_sources = self.compacted.cost_sources.clone();
        let mut captured_output_tokens = self.compacted.captured_output_tokens;
        let mut captured_token_ids = self.compacted.captured_token_ids;
        let mut captured_logprobs = self.compacted.captured_logprobs;
        let mut token_capture_requests = self.compacted.token_capture_requests;

        for e in &self.entries {
            if let Some(date_key) = e.timestamp.get(..10) {
                *daily.entry(date_key.to_string()).or_insert(0.0) += e.cost_usd;
            }
            if let Some(month_key) = e.timestamp.get(..7) {
                *monthly.entry(month_key.to_string()).or_insert(0.0) += e.cost_usd;
            }
            *token_count_sources
                .entry(e.token_count_source.as_str().to_string())
                .or_insert(0) += 1;
            *cost_sources
                .entry(e.cost_source.as_str().to_string())
                .or_insert(0) += 1;
            captured_output_tokens += e.captured_output_tokens();
            captured_token_ids += e.captured_token_ids();
            captured_logprobs += e.captured_logprobs();
            if e.token_capture.is_some() {
                token_capture_requests += 1;
            }
        }

        let total_cost = self.total_cost();
        let total_requests = self.entries.len() as u64 + self.compacted.total_requests;
        let last_7d_model_details = Self::recent_date_keys(today, 7)
            .map(|dates| self.model_breakdown_for_dates(&dates))
            .unwrap_or_default();
        let last_30d_model_details = Self::recent_date_keys(today, 30)
            .map(|dates| self.model_breakdown_for_dates(&dates))
            .unwrap_or_default();

        // All-time model breakdown: merge compacted aggregates with live entries.
        let mut all_time_map: HashMap<String, ModelBreakdownAccumulator> = HashMap::new();
        for entry in &self.entries {
            all_time_map
                .entry(entry.model.clone())
                .or_default()
                .add_entry(entry);
        }
        for (model, ce) in &self.compacted.by_model {
            all_time_map
                .entry(model.clone())
                .or_default()
                .add_compacted(ce);
        }
        let mut all_time_models: Vec<ModelBreakdown> = all_time_map
            .into_iter()
            .map(|(model, acc)| acc.into_breakdown(model))
            .collect();
        all_time_models.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        CostSummary {
            total_cost_usd: total_cost,
            total_input_tokens: self.total_input_tokens(),
            total_output_tokens: self.total_output_tokens(),
            total_requests,
            token_count_sources,
            cost_sources,
            captured_output_tokens,
            captured_token_ids,
            captured_logprobs,
            token_capture_requests,
            avg_cost_per_request: if total_requests > 0 {
                total_cost / total_requests as f64
            } else {
                0.0
            },
            daily,
            monthly,
            by_model: self.cost_by_model().into_iter().collect::<BTreeMap<_, _>>(),
            by_agent: self.cost_by_agent().into_iter().collect::<BTreeMap<_, _>>(),
            model_details: all_time_models,
            today_model_details: self.model_breakdown(Some(today)),
            last_7d_model_details,
            last_30d_model_details,
            alert_threshold_usd: self.budget.daily_limit_usd,
            alert_triggered: self.should_alert(today) || self.is_over_monthly_budget(this_month),
            monthly_limit_usd: self.budget.monthly_limit_usd,
            monthly_alert_triggered: self.is_over_monthly_budget(this_month),
            entries_at_capacity: self.entries.len() >= self.max_entries,
            max_entries: self.max_entries as u64,
        }
    }
}

/// Escape a CSV field per RFC 4180.
///
/// Wraps the value in double-quotes and escapes inner quotes when the value
/// contains commas, double-quotes, or newlines. Also defuses spreadsheet
/// formula injection by prefixing a leading `=`, `+`, `-`, or `@` with
/// a single-quote inside the quoted field.
fn csv_escape(value: &str) -> String {
    let needs_quoting =
        value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r');

    // Defuse formula injection: prefix dangerous leading chars
    let starts_with_formula = value.starts_with('=')
        || value.starts_with('+')
        || value.starts_with('-')
        || value.starts_with('@');

    if needs_quoting || starts_with_formula {
        let escaped = value.replace('"', "\"\"");
        if starts_with_formula {
            format!("\"'{escaped}\"")
        } else {
            format!("\"{escaped}\"")
        }
    } else {
        value.to_string()
    }
}

/// Per-model breakdown with token counts and cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelBreakdown {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub requests: u64,
    /// Requests whose billable token counts came from provider usage fields.
    #[serde(default)]
    pub provider_usage_requests: u64,
    /// Legacy requests without token-count provenance.
    #[serde(default)]
    pub unknown_token_count_requests: u64,
    /// Requests whose dollar cost came directly from the provider response.
    #[serde(default)]
    pub provider_cost_requests: u64,
    /// Requests whose dollar cost used local pricing against provider usage.
    #[serde(default)]
    pub local_pricing_fallback_requests: u64,
    /// Provider-native captured output token count, when exact capture exists.
    #[serde(default)]
    pub captured_output_tokens: u64,
    /// Provider-native captured token id count.
    #[serde(default)]
    pub captured_token_ids: u64,
    /// Provider-native captured logprob count.
    #[serde(default)]
    pub captured_logprobs: u64,
    /// Requests with provider-native token/logprob capture metadata.
    #[serde(default)]
    pub token_capture_requests: u64,
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
    /// Request counts by token-count provenance.
    #[serde(default)]
    pub token_count_sources: BTreeMap<String, u64>,
    /// Request counts by dollar-cost provenance.
    #[serde(default)]
    pub cost_sources: BTreeMap<String, u64>,
    /// Aggregate provider-native captured output tokens.
    #[serde(default)]
    pub captured_output_tokens: u64,
    /// Aggregate provider-native captured token ids.
    #[serde(default)]
    pub captured_token_ids: u64,
    /// Aggregate provider-native captured logprobs.
    #[serde(default)]
    pub captured_logprobs: u64,
    /// Number of requests that included provider-native token capture.
    #[serde(default)]
    pub token_capture_requests: u64,
    pub avg_cost_per_request: f64,
    pub daily: BTreeMap<String, f64>,
    pub monthly: BTreeMap<String, f64>,
    pub by_model: BTreeMap<String, f64>,
    pub by_agent: BTreeMap<String, f64>,
    /// Per-model token breakdown (all-time, includes compacted).
    pub model_details: Vec<ModelBreakdown>,
    /// Per-model token breakdown (today only).
    pub today_model_details: Vec<ModelBreakdown>,
    /// Per-model token breakdown for the trailing 7 UTC days, inclusive.
    pub last_7d_model_details: Vec<ModelBreakdown>,
    /// Per-model token breakdown for the trailing 30 UTC days, inclusive.
    pub last_30d_model_details: Vec<ModelBreakdown>,
    pub alert_threshold_usd: Option<f64>,
    pub alert_triggered: bool,
    /// Monthly budget limit (USD), if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly_limit_usd: Option<f64>,
    /// Whether monthly budget has been exceeded.
    #[serde(default)]
    pub monthly_alert_triggered: bool,
    /// Whether the live entry buffer has reached its maximum capacity.
    /// When true, oldest entries are being compacted into aggregates.
    #[serde(default)]
    pub entries_at_capacity: bool,
    /// Maximum number of live entries before compaction.
    #[serde(default)]
    pub max_entries: u64,
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
            token_count_source: TokenCountSource::ProviderUsage,
            cost_source: CostSource::ProviderCost,
            token_capture: None,
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
    fn test_summary_tracks_provider_usage_and_capture_provenance() {
        let mut tracker = CostTracker::new(BudgetConfig::default());
        let mut entry = make_entry(0.01, "2026-03-04T10:00:00Z", "gpt-4o");
        entry.cost_source = CostSource::LocalPricingFallback;
        entry.token_capture = Some(TokenCaptureCostSummary::from_capture(
            &ProviderTokenCapture {
                exact_tokens_supported: true,
                logprobs_supported: true,
                token_ids: vec![11, 12],
                tokens: vec!["he".into(), "llo".into()],
                logprobs: vec![-0.1, -0.2],
                provider: Some("openai".into()),
                model: Some("gpt-4o".into()),
            },
        ));
        tracker.record(entry);

        let summary = tracker.summary("2026-03-04", "2026-03");
        assert_eq!(summary.token_count_sources["provider_usage"], 1);
        assert_eq!(summary.cost_sources["local_pricing_fallback"], 1);
        assert_eq!(summary.captured_output_tokens, 2);
        assert_eq!(summary.captured_token_ids, 2);
        assert_eq!(summary.captured_logprobs, 2);
        assert_eq!(summary.token_capture_requests, 1);
        assert_eq!(summary.model_details[0].provider_usage_requests, 1);
        assert_eq!(summary.model_details[0].local_pricing_fallback_requests, 1);
        assert_eq!(summary.model_details[0].captured_output_tokens, 2);
        assert_eq!(summary.model_details[0].captured_token_ids, 2);
        assert_eq!(summary.model_details[0].captured_logprobs, 2);
        assert_eq!(summary.model_details[0].token_capture_requests, 1);
    }

    #[test]
    fn test_summary_recent_range_breakdowns() {
        let mut tracker = CostTracker::new(BudgetConfig::default());
        tracker.record(make_entry(1.0, "2026-03-28T10:00:00Z", "gpt-4o"));
        tracker.record(make_entry(2.0, "2026-04-02T10:00:00Z", "claude-sonnet"));
        tracker.record(make_entry(3.0, "2026-04-05T10:00:00Z", "gpt-4o-mini"));

        let summary = tracker.summary("2026-04-05", "2026-04");

        assert_eq!(summary.today_model_details.len(), 1);
        assert_eq!(summary.today_model_details[0].model, "gpt-4o-mini");

        let range_7d_models: Vec<_> = summary
            .last_7d_model_details
            .iter()
            .map(|entry| entry.model.as_str())
            .collect();
        assert!(range_7d_models.contains(&"claude-sonnet"));
        assert!(range_7d_models.contains(&"gpt-4o-mini"));
        assert!(!range_7d_models.contains(&"gpt-4o"));

        let range_30d_models: Vec<_> = summary
            .last_30d_model_details
            .iter()
            .map(|entry| entry.model.as_str())
            .collect();
        assert!(range_30d_models.contains(&"gpt-4o"));
        assert!(range_30d_models.contains(&"claude-sonnet"));
        assert!(range_30d_models.contains(&"gpt-4o-mini"));
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

    #[test]
    fn test_compaction_preserves_totals() {
        let mut tracker = CostTracker::new(BudgetConfig::default()).with_max_entries(3);
        tracker.record(make_entry(1.0, "2026-03-01T10:00:00Z", "gpt-4o"));
        tracker.record(make_entry(2.0, "2026-03-02T10:00:00Z", "claude"));
        tracker.record(make_entry(3.0, "2026-03-03T10:00:00Z", "gpt-4o"));
        // No eviction yet, 3 entries = max
        assert_eq!(tracker.entry_count(), 3);
        assert!((tracker.total_cost() - 6.0).abs() < 1e-6);

        // 4th entry triggers eviction of the 1st
        tracker.record(make_entry(4.0, "2026-03-04T10:00:00Z", "gpt-4o"));
        assert_eq!(tracker.entry_count(), 3); // still 3 live
        assert!((tracker.total_cost() - 10.0).abs() < 1e-6); // 2+3+4 live + 1 compacted

        // Check compacted data preserved the daily totals
        let summary = tracker.summary("2026-03-04", "2026-03");
        assert_eq!(summary.total_requests, 4);
        assert!((summary.daily["2026-03-01"] - 1.0).abs() < 1e-6); // compacted
        assert!((summary.daily["2026-03-04"] - 4.0).abs() < 1e-6); // live
        assert!((summary.total_cost_usd - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_json_roundtrip_with_compacted() {
        let mut tracker = CostTracker::new(BudgetConfig::default()).with_max_entries(2);
        tracker.record(make_entry(1.0, "2026-03-01T10:00:00Z", "gpt-4o"));
        tracker.record(make_entry(2.0, "2026-03-02T10:00:00Z", "claude"));
        tracker.record(make_entry(3.0, "2026-03-03T10:00:00Z", "gpt-4o"));
        // 1st entry compacted, 2 live
        let json = tracker.to_json();

        let mut restored = CostTracker::new(BudgetConfig::default()).with_max_entries(2);
        restored.from_json(&json);
        assert_eq!(restored.entry_count(), 2);
        assert!((restored.total_cost() - 6.0).abs() < 1e-6);
        assert_eq!(restored.compacted.total_requests, 1);
    }

    #[test]
    fn test_json_legacy_format_compat() {
        // Old format: plain array of entries
        let legacy = serde_json::json!([
            {
                "timestamp": "2026-03-01T10:00:00Z",
                "agent_id": "agent-1",
                "provider": "openai",
                "model": "gpt-4o",
                "input_tokens": 100,
                "output_tokens": 200,
                "cost_usd": 1.0,
                "request_id": null
            }
        ]);
        let mut tracker = CostTracker::new(BudgetConfig::default());
        tracker.from_json(&legacy);
        assert_eq!(tracker.entry_count(), 1);
        assert!((tracker.total_cost() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_recent_action_count_counts_only_windowed_live_entries() {
        let mut tracker = CostTracker::new(BudgetConfig::default());
        tracker.record(make_entry(1.0, "2026-03-01T10:10:00Z", "gpt-4o"));
        tracker.record(make_entry(1.0, "2026-03-01T11:25:00Z", "gpt-4o"));
        tracker.record(make_entry(1.0, "2026-03-01T11:50:00Z", "gpt-4o"));

        let now = DateTime::parse_from_rfc3339("2026-03-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(tracker.recent_action_count(now, Duration::hours(1)), 2);
    }
}
