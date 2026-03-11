//! Post-compaction context injection.
//!
//! After a conversation is compacted (summarized to save tokens),
//! workspace-level context needs to be re-injected so the agent
//! retains critical project knowledge.
//!
//! Injection layers:
//! 1. Workspace rules (always appended to summaries)
//! 2. Active skill context (re-injected after compaction)
//! 3. Pinned facts (user-defined persistent context)
//!
//! Configuration:
//! - `POST_COMPACTION_INJECT` — enable context re-injection (default: true)
//! - `POST_COMPACTION_MAX_TOKENS` — max tokens for injected context (default: 2000)

use serde::{Deserialize, Serialize};

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
    /// Whether to include pinned facts.
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

        if let Ok(max) = std::env::var("POST_COMPACTION_MAX_TOKENS") {
            if let Ok(m) = max.parse() {
                config.max_tokens = m;
            }
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
        self.fragments.sort_by(|a, b| b.priority.cmp(&a.priority));

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
