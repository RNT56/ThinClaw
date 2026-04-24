//! Post-compaction context injection.
//!
//! After a conversation is compacted (summarized to save tokens),
//! workspace-level context needs to be re-injected so the agent
//! retains critical project knowledge.
//!
//! Supported injection layers:
//! 1. Workspace rules discovered from the current workspace
//! 2. Active skill context re-selected from the current query when available
//! 3. Durable pinned facts re-sourced from workspace/user memory documents
//!
//! Configuration:
//! - `POST_COMPACTION_INJECT` — enable context re-injection (default: true)
//! - `POST_COMPACTION_MAX_TOKENS` — max tokens for injected context (default: 2000)

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::profile::{PsychographicProfile, UserCohort};

/// Configuration for post-compaction context injection.
#[derive(Debug, Clone)]
pub struct PostCompactionConfig {
    /// Whether to inject context after compaction.
    pub enabled: bool,
    /// Maximum tokens to allocate for injected context.
    pub max_tokens: usize,
    /// Whether to include workspace rules.
    pub include_rules: bool,
    /// Whether to include active skill context.
    pub include_skills: bool,
    /// Whether to include caller-supplied pinned facts when provided.
    pub include_pinned: bool,
}

impl Default for PostCompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_tokens: 2000,
            include_rules: true,
            include_skills: true,
            include_pinned: true,
        }
    }
}

impl PostCompactionConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("POST_COMPACTION_INJECT") {
            config.enabled = val != "0" && !val.eq_ignore_ascii_case("false");
        }

        if let Ok(max) = std::env::var("POST_COMPACTION_MAX_TOKENS")
            && let Ok(m) = max.parse()
        {
            config.max_tokens = m;
        }

        config
    }
}

/// A piece of context to inject after compaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFragment {
    /// Label for the fragment (e.g., "workspace_rules", "skill:web-search").
    pub label: String,
    /// The context text to inject.
    pub content: String,
    /// Priority (higher = injected first, lower = potentially truncated).
    pub priority: u32,
    /// Estimated token count (chars / 4).
    pub estimated_tokens: usize,
}

impl ContextFragment {
    /// Create a new context fragment.
    pub fn new(label: impl Into<String>, content: impl Into<String>, priority: u32) -> Self {
        let content = content.into();
        let estimated_tokens = content.len() / 4;
        Self {
            label: label.into(),
            content,
            priority,
            estimated_tokens,
        }
    }
}

/// Build the injection payload from available context fragments.
pub struct ContextInjector {
    config: PostCompactionConfig,
    fragments: Vec<ContextFragment>,
}

impl ContextInjector {
    /// Create a new injector with the given config.
    pub fn new(config: PostCompactionConfig) -> Self {
        Self {
            config,
            fragments: Vec::new(),
        }
    }

    /// Add workspace rules.
    pub fn add_rules(&mut self, rules: &str) {
        if self.config.include_rules && !rules.is_empty() {
            self.fragments.push(ContextFragment::new(
                "workspace_rules",
                rules,
                100, // Highest priority
            ));
        }
    }

    /// Add active skill context.
    pub fn add_skill_context(&mut self, skill_name: &str, context: &str) {
        if self.config.include_skills && !context.is_empty() {
            self.fragments.push(ContextFragment::new(
                format!("skill:{}", skill_name),
                context,
                50,
            ));
        }
    }

    /// Add a pinned fact.
    pub fn add_pinned_fact(&mut self, fact: &str) {
        if self.config.include_pinned && !fact.is_empty() {
            self.fragments
                .push(ContextFragment::new("pinned_fact", fact, 75));
        }
    }

    /// Add a custom fragment.
    pub fn add_fragment(&mut self, fragment: ContextFragment) {
        self.fragments.push(fragment);
    }

    /// Build the final injection string, respecting token limits.
    pub fn build(&mut self) -> String {
        if !self.config.enabled || self.fragments.is_empty() {
            return String::new();
        }

        // Sort by priority (highest first)
        self.fragments
            .sort_by_key(|f| std::cmp::Reverse(f.priority));

        let mut result = Vec::new();
        let mut total_tokens = 0;

        for fragment in &self.fragments {
            if total_tokens + fragment.estimated_tokens > self.config.max_tokens {
                // Try to fit a truncated version
                let remaining = self.config.max_tokens.saturating_sub(total_tokens);
                if remaining > 5 {
                    let truncated_chars = remaining * 4;
                    let truncated: String =
                        fragment.content.chars().take(truncated_chars).collect();
                    result.push(format!("[{}] {}…", fragment.label, truncated));
                    total_tokens += remaining;
                }
                break;
            }

            result.push(format!("[{}] {}", fragment.label, fragment.content));
            total_tokens += fragment.estimated_tokens;
        }

        if result.is_empty() {
            return String::new();
        }

        format!(
            "[Post-compaction context — {} tokens used]\n{}",
            total_tokens,
            result.join("\n\n")
        )
    }

    /// Get the number of fragments.
    pub fn fragment_count(&self) -> usize {
        self.fragments.len()
    }
}

fn ordered_list_body(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let rest = trimmed.get(digits..)?.trim_start();
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    Some(rest.trim_start())
}

fn normalize_fact_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed == "---"
        || trimmed.starts_with('#')
        || trimmed.starts_with("<!--")
    {
        return None;
    }

    let trimmed = trimmed
        .strip_prefix("- [ ]")
        .or_else(|| trimmed.strip_prefix("- [x]"))
        .or_else(|| trimmed.strip_prefix("- "))
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("• "))
        .unwrap_or(trimmed)
        .trim_start_matches(['-', '*', '•'])
        .trim();
    let trimmed = ordered_list_body(trimmed).unwrap_or(trimmed).trim();

    if trimmed.is_empty() || trimmed == "_" || trimmed.starts_with("_(") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn push_fact(
    facts: &mut Vec<String>,
    seen: &mut HashSet<String>,
    candidate: impl Into<String>,
    max_facts: usize,
) {
    if facts.len() >= max_facts {
        return;
    }
    let candidate = candidate.into();
    let key = candidate.trim().to_ascii_lowercase();
    if !key.is_empty() && seen.insert(key) {
        facts.push(candidate);
    }
}

/// Extract filled markdown fields such as `**Name:** Alex` or
/// `- **Feedback style:** direct`.
pub fn extract_markdown_field_facts(content: &str, max_facts: usize) -> Vec<String> {
    if max_facts == 0 {
        return Vec::new();
    }

    let mut facts = Vec::new();
    let mut seen = HashSet::new();

    for line in content.lines() {
        let trimmed = line.trim();
        let is_field =
            (trimmed.starts_with("**") || trimmed.starts_with("- **")) && trimmed.contains(':');
        if !is_field {
            continue;
        }
        let after_colon = trimmed
            .split_once(':')
            .map(|(_, rest)| rest.trim())
            .unwrap_or_default();
        if after_colon.is_empty()
            || after_colon == "_"
            || after_colon == "**"
            || after_colon.starts_with("_(")
        {
            continue;
        }
        if let Some(fact) = normalize_fact_line(trimmed) {
            push_fact(&mut facts, &mut seen, fact, max_facts);
        }
    }

    facts
}

/// Extract durable pinned facts from markdown memory documents.
///
/// Priority order:
/// 1. Explicit `PIN:` / `Pinned:` style markers
/// 2. Lines inside sections like `## Pinned` / `## Key Facts`
/// 3. Concise checklist or bullet entries as a conservative fallback
pub fn extract_pinned_facts_from_markdown(content: &str, max_facts: usize) -> Vec<String> {
    if max_facts == 0 {
        return Vec::new();
    }

    const EXPLICIT_PREFIXES: &[&str] = &[
        "PIN:",
        "Pinned:",
        "PINNED:",
        "- PIN:",
        "- Pinned:",
        "- PINNED:",
        "* PIN:",
        "* Pinned:",
        "* PINNED:",
    ];
    const PINNED_SECTION_HINTS: &[&str] = &[
        "pinned",
        "key fact",
        "key facts",
        "always remember",
        "durable fact",
        "durable facts",
        "important",
    ];

    let mut facts = Vec::new();
    let mut seen = HashSet::new();
    let mut in_pinned_section = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let heading = trimmed.trim_start_matches('#').trim().to_ascii_lowercase();
            in_pinned_section = PINNED_SECTION_HINTS
                .iter()
                .any(|hint| heading.contains(hint));
            continue;
        }

        if let Some(explicit) = EXPLICIT_PREFIXES
            .iter()
            .find_map(|prefix| trimmed.strip_prefix(prefix))
            .and_then(normalize_fact_line)
        {
            push_fact(&mut facts, &mut seen, explicit, max_facts);
            continue;
        }

        if in_pinned_section && let Some(fact) = normalize_fact_line(trimmed) {
            push_fact(&mut facts, &mut seen, fact, max_facts);
        }
    }

    if !facts.is_empty() {
        return facts;
    }

    for line in content.lines() {
        let trimmed = line.trim();
        let looks_like_entry = trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("- [")
            || ordered_list_body(trimmed).is_some();
        if !looks_like_entry {
            continue;
        }
        if let Some(fact) = normalize_fact_line(trimmed) {
            push_fact(&mut facts, &mut seen, fact, max_facts);
        }
    }

    facts
}

/// Extract confidence-gated profile facts suitable for post-compaction context.
pub fn extract_profile_facts(content: &str, max_facts: usize) -> Vec<String> {
    if max_facts == 0 {
        return Vec::new();
    }

    let Ok(profile) = serde_json::from_str::<PsychographicProfile>(content) else {
        return Vec::new();
    };
    if !profile.is_populated() || profile.confidence < 0.3 {
        return Vec::new();
    }

    let mut facts = Vec::new();
    if !profile.preferred_name.is_empty() {
        facts.push(format!("**Name**: {}", profile.preferred_name));
    }
    facts.push(format!(
        "**Communication**: {} tone, {} detail, {} formality",
        profile.communication.tone,
        profile.communication.detail_level,
        profile.communication.formality
    ));
    if profile.cohort.cohort != UserCohort::Other && profile.cohort.confidence > 0 {
        facts.push(format!(
            "**User type**: {} ({}% confidence)",
            profile.cohort.cohort, profile.cohort.confidence
        ));
    }

    if profile.confidence >= 0.6 {
        facts.extend(
            extract_markdown_field_facts(&profile.to_user_md(), max_facts)
                .into_iter()
                .filter(|fact| {
                    !fact.starts_with("**Name**:")
                        && !fact.starts_with("**Communication**:")
                        && !fact.starts_with("**User type**:")
                }),
        );
    }

    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for fact in facts {
        push_fact(&mut deduped, &mut seen, fact, max_facts);
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::PsychographicProfile;

    #[test]
    fn test_default_config() {
        let config = PostCompactionConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_tokens, 2000);
    }

    #[test]
    fn test_disabled_returns_empty() {
        let config = PostCompactionConfig {
            enabled: false,
            ..Default::default()
        };
        let mut injector = ContextInjector::new(config);
        injector.add_rules("some rules");
        assert!(injector.build().is_empty());
    }

    #[test]
    fn test_basic_injection() {
        let config = PostCompactionConfig::default();
        let mut injector = ContextInjector::new(config);
        injector.add_rules("Always respond in English.");
        injector.add_pinned_fact("The project uses Rust.");

        let result = injector.build();
        assert!(result.contains("[workspace_rules]"));
        assert!(result.contains("Always respond in English"));
        assert!(result.contains("[pinned_fact]"));
    }

    #[test]
    fn test_priority_ordering() {
        let config = PostCompactionConfig::default();
        let mut injector = ContextInjector::new(config);

        injector.add_fragment(ContextFragment::new("low", "low priority", 10));
        injector.add_fragment(ContextFragment::new("high", "high priority", 90));

        let result = injector.build();
        let high_pos = result.find("[high]").unwrap();
        let low_pos = result.find("[low]").unwrap();
        assert!(high_pos < low_pos);
    }

    #[test]
    fn test_token_limit_truncation() {
        let config = PostCompactionConfig {
            max_tokens: 20, // Small limit
            ..Default::default()
        };
        let mut injector = ContextInjector::new(config);
        injector.add_rules(&"x".repeat(1000));

        let result = injector.build();
        // Should be truncated but not empty
        assert!(!result.is_empty());
        assert!(result.len() < 500);
    }

    #[test]
    fn test_empty_fragments_skipped() {
        let config = PostCompactionConfig::default();
        let mut injector = ContextInjector::new(config);
        injector.add_rules("");
        injector.add_pinned_fact("");
        assert_eq!(injector.fragment_count(), 0);
    }

    #[test]
    fn test_skill_context() {
        let config = PostCompactionConfig::default();
        let mut injector = ContextInjector::new(config);
        injector.add_skill_context("web-search", "Use DuckDuckGo API.");

        let result = injector.build();
        assert!(result.contains("[skill:web-search]"));
    }

    #[test]
    fn test_extract_markdown_field_facts() {
        let facts = extract_markdown_field_facts(
            "# User Profile\n\n**Name**: Alex\n- **Timezone:** Europe/Berlin\n- **Notes:** Loves Rust",
            4,
        );
        assert_eq!(facts.len(), 3);
        assert!(facts.iter().any(|fact| fact.contains("Alex")));
        assert!(facts.iter().any(|fact| fact.contains("Europe/Berlin")));
    }

    #[test]
    fn test_extract_pinned_facts_prefers_explicit_markers() {
        let facts = extract_pinned_facts_from_markdown(
            "# Memory\n\nPIN: Prefers short morning check-ins\n- Pinned: Working on ThinClaw\n- Not pinned\n",
            4,
        );
        assert_eq!(facts.len(), 2);
        assert!(
            facts
                .iter()
                .any(|fact| fact.contains("short morning check-ins"))
        );
        assert!(
            facts
                .iter()
                .any(|fact| fact.contains("Working on ThinClaw"))
        );
    }

    #[test]
    fn test_extract_profile_facts_respects_confidence_gate() {
        let mut profile = PsychographicProfile::default();
        profile.preferred_name = "Sam".into();
        profile.communication.tone = "warm".into();
        profile.communication.detail_level = "balanced".into();
        profile.communication.formality = "casual".into();
        profile.confidence = 0.25;
        let low_confidence = serde_json::to_string(&profile).expect("serialize low confidence");
        assert!(extract_profile_facts(&low_confidence, 4).is_empty());

        profile.confidence = 0.7;
        profile.assistance.goals = vec!["Ship ThinClaw".into()];
        let high_confidence = serde_json::to_string(&profile).expect("serialize high confidence");
        let facts = extract_profile_facts(&high_confidence, 4);
        assert!(facts.iter().any(|fact| fact.contains("Sam")));
        assert!(facts.iter().any(|fact| fact.contains("Communication")));
    }
}
