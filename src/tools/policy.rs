//! Per-group tool access policies.
//!
//! Allows restricting which tools are available in specific channels,
//! groups, or conversations. Useful for multi-tenant deployments where
//! different groups should have different tool access levels.
//!
//! ## Policy evaluation order
//!
//! 1. Check group-level overrides (most specific)
//! 2. Check channel-level overrides
//! 3. Fall back to global default (allow all)
//!
//! ## Policy modes
//!
//! - **AllowAll**: All tools are available (default)
//! - **AllowList**: Only explicitly listed tools are available
//! - **DenyList**: All tools except explicitly listed ones are available

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Policy controlling which tools are accessible.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ToolAccessPolicy {
    /// All tools are available.
    AllowAll,
    /// Only the listed tools are available.
    AllowList {
        /// Tool names to allow.
        tools: HashSet<String>,
    },
    /// All tools except the listed ones are available.
    DenyList {
        /// Tool names to deny.
        tools: HashSet<String>,
    },
}

impl Default for ToolAccessPolicy {
    fn default() -> Self {
        Self::AllowAll
    }
}

impl ToolAccessPolicy {
    /// Check if a tool is allowed by this policy.
    pub fn allows(&self, tool_name: &str) -> bool {
        match self {
            Self::AllowAll => true,
            Self::AllowList { tools } => tools.contains(tool_name),
            Self::DenyList { tools } => !tools.contains(tool_name),
        }
    }

    /// Create an allow-list policy from a list of tool names.
    pub fn allow_only(tools: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::AllowList {
            tools: tools.into_iter().map(Into::into).collect(),
        }
    }

    /// Create a deny-list policy from a list of tool names.
    pub fn deny(tools: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::DenyList {
            tools: tools.into_iter().map(Into::into).collect(),
        }
    }
}

/// Manages per-group and per-channel tool access policies.
///
/// Evaluation order: group override → channel override → global default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPolicyManager {
    /// Global default policy (applies when no override matches).
    pub default_policy: ToolAccessPolicy,
    /// Per-channel policies (keyed by channel name, e.g., "signal", "telegram").
    pub channel_policies: HashMap<String, ToolAccessPolicy>,
    /// Per-group policies (keyed by `channel:group_id`, e.g., "signal:+1234567890").
    pub group_policies: HashMap<String, ToolAccessPolicy>,
}

impl ToolPolicyManager {
    /// Create a new policy manager with the default (allow all) policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the global default policy.
    pub fn set_default(&mut self, policy: ToolAccessPolicy) {
        self.default_policy = policy;
    }

    /// Set a policy for a specific channel.
    pub fn set_channel_policy(&mut self, channel: impl Into<String>, policy: ToolAccessPolicy) {
        self.channel_policies.insert(channel.into(), policy);
    }

    /// Set a policy for a specific group within a channel.
    ///
    /// The `group_key` should be in the format `channel:group_id`, e.g.,
    /// `"signal:+1234567890"` or `"telegram:chat_123"`.
    pub fn set_group_policy(&mut self, group_key: impl Into<String>, policy: ToolAccessPolicy) {
        self.group_policies.insert(group_key.into(), policy);
    }

    /// Check if a tool is allowed for a given context.
    ///
    /// - `tool_name`: Name of the tool to check.
    /// - `channel`: Optional channel name (e.g., "signal", "telegram").
    /// - `group_id`: Optional group/conversation ID within the channel.
    ///
    /// Evaluation order: group override → channel override → global default.
    pub fn is_allowed(
        &self,
        tool_name: &str,
        channel: Option<&str>,
        group_id: Option<&str>,
    ) -> bool {
        // 1. Check group-level override (most specific).
        if let (Some(ch), Some(gid)) = (channel, group_id) {
            let group_key = format!("{}:{}", ch, gid);
            if let Some(policy) = self.group_policies.get(&group_key) {
                return policy.allows(tool_name);
            }
        }

        // 2. Check channel-level override.
        if let Some(ch) = channel {
            if let Some(policy) = self.channel_policies.get(ch) {
                return policy.allows(tool_name);
            }
        }

        // 3. Fall back to global default.
        self.default_policy.allows(tool_name)
    }

    /// Filter a list of tool names, returning only those allowed for the context.
    pub fn filter_tools<'a>(
        &self,
        tool_names: &'a [String],
        channel: Option<&str>,
        group_id: Option<&str>,
    ) -> Vec<&'a String> {
        tool_names
            .iter()
            .filter(|name| self.is_allowed(name, channel, group_id))
            .collect()
    }

    /// Get the effective policy for a given context.
    pub fn effective_policy(
        &self,
        channel: Option<&str>,
        group_id: Option<&str>,
    ) -> &ToolAccessPolicy {
        // Group-level overrides.
        if let (Some(ch), Some(gid)) = (channel, group_id) {
            let group_key = format!("{}:{}", ch, gid);
            if let Some(policy) = self.group_policies.get(&group_key) {
                return policy;
            }
        }

        // Channel-level overrides.
        if let Some(ch) = channel {
            if let Some(policy) = self.channel_policies.get(ch) {
                return policy;
            }
        }

        &self.default_policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allow_all_default() {
        let policy = ToolAccessPolicy::default();
        assert!(policy.allows("shell"));
        assert!(policy.allows("http"));
        assert!(policy.allows("anything"));
    }

    #[test]
    fn test_allow_list() {
        let policy = ToolAccessPolicy::allow_only(["shell", "calculator"]);
        assert!(policy.allows("shell"));
        assert!(policy.allows("calculator"));
        assert!(!policy.allows("http"));
        assert!(!policy.allows("unknown"));
    }

    #[test]
    fn test_deny_list() {
        let policy = ToolAccessPolicy::deny(["shell", "http"]);
        assert!(!policy.allows("shell"));
        assert!(!policy.allows("http"));
        assert!(policy.allows("calculator"));
        assert!(policy.allows("search"));
    }

    #[test]
    fn test_manager_global_default() {
        let manager = ToolPolicyManager::new();
        assert!(manager.is_allowed("shell", None, None));
        assert!(manager.is_allowed("shell", Some("signal"), None));
    }

    #[test]
    fn test_manager_channel_override() {
        let mut manager = ToolPolicyManager::new();
        manager.set_channel_policy("telegram", ToolAccessPolicy::deny(["shell"]));

        // Signal uses global default (allow all).
        assert!(manager.is_allowed("shell", Some("signal"), None));

        // Telegram denies shell.
        assert!(!manager.is_allowed("shell", Some("telegram"), None));
        assert!(manager.is_allowed("calculator", Some("telegram"), None));
    }

    #[test]
    fn test_manager_group_override_takes_precedence() {
        let mut manager = ToolPolicyManager::new();
        manager.set_channel_policy("signal", ToolAccessPolicy::deny(["shell"]));
        manager.set_group_policy(
            "signal:+1234567890",
            ToolAccessPolicy::allow_only(["shell", "calculator"]),
        );

        // Channel policy denies shell, but group policy overrides it.
        assert!(manager.is_allowed("shell", Some("signal"), Some("+1234567890")));
        assert!(manager.is_allowed("calculator", Some("signal"), Some("+1234567890")));
        assert!(!manager.is_allowed("http", Some("signal"), Some("+1234567890")));

        // Other groups in the signal channel still use channel policy.
        assert!(!manager.is_allowed("shell", Some("signal"), Some("+9876543210")));
    }

    #[test]
    fn test_filter_tools() {
        let mut manager = ToolPolicyManager::new();
        manager.set_channel_policy(
            "telegram",
            ToolAccessPolicy::allow_only(["calculator", "search"]),
        );

        let tools: Vec<String> = vec![
            "shell".into(),
            "calculator".into(),
            "http".into(),
            "search".into(),
        ];

        let filtered = manager.filter_tools(&tools, Some("telegram"), None);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.contains(&&"calculator".to_string()));
        assert!(filtered.contains(&&"search".to_string()));
    }

    #[test]
    fn test_effective_policy() {
        let mut manager = ToolPolicyManager::new();
        manager.set_channel_policy("signal", ToolAccessPolicy::deny(["shell"]));

        let policy = manager.effective_policy(Some("signal"), None);
        assert_eq!(
            *policy,
            ToolAccessPolicy::DenyList {
                tools: HashSet::from(["shell".to_string()])
            }
        );

        // Unknown channel falls back to default.
        let policy = manager.effective_policy(Some("discord"), None);
        assert_eq!(*policy, ToolAccessPolicy::AllowAll);
    }

    #[test]
    fn test_policy_serialization() {
        let policy = ToolAccessPolicy::allow_only(["shell", "http"]);
        let json = serde_json::to_string(&policy).unwrap();
        let deserialized: ToolAccessPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, deserialized);
    }

    #[test]
    fn test_manager_serialization() {
        let mut manager = ToolPolicyManager::new();
        manager.set_channel_policy("signal", ToolAccessPolicy::deny(["shell"]));
        manager.set_group_policy(
            "signal:group1",
            ToolAccessPolicy::allow_only(["calculator"]),
        );

        let json = serde_json::to_string(&manager).unwrap();
        let deserialized: ToolPolicyManager = serde_json::from_str(&json).unwrap();

        assert!(!deserialized.is_allowed("shell", Some("signal"), None));
        assert!(deserialized.is_allowed("calculator", Some("signal"), Some("group1")));
        assert!(!deserialized.is_allowed("shell", Some("signal"), Some("group1")));
    }
}
