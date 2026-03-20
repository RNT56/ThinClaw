//! Post-compaction read audit.
//!
//! Layer 3 of the memory pipeline: after context has been compacted,
//! this module audits workspace rules and appends them to summaries,
//! ensuring the agent retains workspace-specific knowledge.

use serde::{Deserialize, Serialize};

/// Configuration for the read audit layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadAuditConfig {
    /// Whether read audit is enabled.
    pub enabled: bool,
    /// Maximum tokens for appended workspace rules.
    pub max_rule_tokens: u32,
    /// Paths to scan for workspace rules.
    pub rule_paths: Vec<String>,
    /// Whether to include agent-specific rules.
    pub include_agent_rules: bool,
    /// Whether to include global rules.
    pub include_global_rules: bool,
}

impl Default for ReadAuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_rule_tokens: 500,
            rule_paths: vec![
                ".ironclaw/rules.md".to_string(),
                ".ironclaw/workspace-rules.md".to_string(),
                "RULES.md".to_string(),
            ],
            include_agent_rules: true,
            include_global_rules: true,
        }
    }
}

/// A workspace rule discovered during audit.
#[derive(Debug, Clone)]
pub struct WorkspaceRule {
    /// Source file path.
    pub source: String,
    /// Rule content.
    pub content: String,
    /// Whether this is a global or agent-specific rule.
    pub scope: RuleScope,
    /// Estimated token count.
    pub estimated_tokens: u32,
}

/// Scope of a workspace rule.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleScope {
    /// Applies to all agents and sessions.
    Global,
    /// Applies to a specific agent.
    Agent(String),
    /// Applies to a specific workspace/directory.
    Workspace(String),
}

/// Read auditor that scans and appends workspace rules.
pub struct ReadAuditor {
    config: ReadAuditConfig,
    rules: Vec<WorkspaceRule>,
}

impl ReadAuditor {
    pub fn new(config: ReadAuditConfig) -> Self {
        Self {
            config,
            rules: Vec::new(),
        }
    }

    /// Add a rule to the auditor.
    pub fn add_rule(&mut self, rule: WorkspaceRule) {
        self.rules.push(rule);
    }

    /// Scan workspace paths for rules.
    pub fn scan_rules(&mut self, workspace_root: &str) {
        for rule_path in &self.config.rule_paths {
            let full_path = format!("{}/{}", workspace_root, rule_path);

            if let Ok(content) = std::fs::read_to_string(&full_path)
                && !content.trim().is_empty()
            {
                let estimated_tokens = estimate_tokens(&content);
                self.rules.push(WorkspaceRule {
                    source: rule_path.clone(),
                    content,
                    scope: RuleScope::Global,
                    estimated_tokens,
                });
            }
        }
    }

    /// Build the audit appendix for post-compaction context.
    pub fn build_appendix(&self) -> String {
        if !self.config.enabled || self.rules.is_empty() {
            return String::new();
        }

        let mut body = String::new();
        let mut total_tokens: u32 = 0;

        for rule in &self.rules {
            if !self.should_include(&rule.scope) {
                continue;
            }

            if total_tokens + rule.estimated_tokens > self.config.max_rule_tokens {
                // Truncate remaining rules
                let remaining = self.config.max_rule_tokens.saturating_sub(total_tokens);
                if remaining > 10 {
                    let truncated: String =
                        rule.content.chars().take(remaining as usize * 4).collect();
                    body.push_str(&format!("\n[{}]:\n{}", rule.source, truncated));
                    body.push_str("\n[... truncated]");
                }
                break;
            }

            body.push_str(&format!("\n[{}]:\n{}", rule.source, rule.content));
            total_tokens += rule.estimated_tokens;
        }

        if body.is_empty() {
            return String::new();
        }

        format!("\n--- Workspace Rules ---{}", body)
    }

    /// Check if a scope should be included.
    fn should_include(&self, scope: &RuleScope) -> bool {
        match scope {
            RuleScope::Global => self.config.include_global_rules,
            RuleScope::Agent(_) => self.config.include_agent_rules,
            RuleScope::Workspace(_) => true,
        }
    }

    /// Number of rules loaded.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Total estimated tokens across all rules.
    pub fn total_tokens(&self) -> u32 {
        self.rules.iter().map(|r| r.estimated_tokens).sum()
    }
}

/// Simple token estimation (~4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    (text.len() as u32 / 4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ReadAuditConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_rule_tokens, 500);
        assert!(!config.rule_paths.is_empty());
    }

    #[test]
    fn test_empty_appendix() {
        let auditor = ReadAuditor::new(ReadAuditConfig::default());
        assert!(auditor.build_appendix().is_empty());
    }

    #[test]
    fn test_disabled_appendix() {
        let mut auditor = ReadAuditor::new(ReadAuditConfig {
            enabled: false,
            ..Default::default()
        });
        auditor.add_rule(WorkspaceRule {
            source: "rules.md".to_string(),
            content: "Be helpful.".to_string(),
            scope: RuleScope::Global,
            estimated_tokens: 5,
        });
        assert!(auditor.build_appendix().is_empty());
    }

    #[test]
    fn test_build_appendix() {
        let mut auditor = ReadAuditor::new(ReadAuditConfig::default());
        auditor.add_rule(WorkspaceRule {
            source: "rules.md".to_string(),
            content: "Always be concise.".to_string(),
            scope: RuleScope::Global,
            estimated_tokens: 5,
        });

        let appendix = auditor.build_appendix();
        assert!(appendix.contains("Workspace Rules"));
        assert!(appendix.contains("Always be concise"));
    }

    #[test]
    fn test_agent_scope_filtering() {
        let mut auditor = ReadAuditor::new(ReadAuditConfig {
            include_agent_rules: false,
            ..Default::default()
        });
        auditor.add_rule(WorkspaceRule {
            source: "agent.md".to_string(),
            content: "Agent-only rule.".to_string(),
            scope: RuleScope::Agent("agent-1".to_string()),
            estimated_tokens: 5,
        });

        assert!(auditor.build_appendix().is_empty());
    }

    #[test]
    fn test_rule_count() {
        let mut auditor = ReadAuditor::new(ReadAuditConfig::default());
        auditor.add_rule(WorkspaceRule {
            source: "a.md".to_string(),
            content: "Rule A.".to_string(),
            scope: RuleScope::Global,
            estimated_tokens: 5,
        });
        assert_eq!(auditor.rule_count(), 1);
    }

    #[test]
    fn test_total_tokens() {
        let mut auditor = ReadAuditor::new(ReadAuditConfig::default());
        auditor.add_rule(WorkspaceRule {
            source: "a.md".to_string(),
            content: "x".to_string(),
            scope: RuleScope::Global,
            estimated_tokens: 10,
        });
        auditor.add_rule(WorkspaceRule {
            source: "b.md".to_string(),
            content: "y".to_string(),
            scope: RuleScope::Global,
            estimated_tokens: 20,
        });
        assert_eq!(auditor.total_tokens(), 30);
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens("hello world!"), 3);
        assert_eq!(estimate_tokens(""), 1);
    }
}
