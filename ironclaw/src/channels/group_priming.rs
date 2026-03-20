//! Group session priming — inject member roster into context.
//!
//! For group chats, it's useful for the agent to know who is in the
//! conversation. This module handles building a roster summary and
//! injecting it into the session context.
//!
//! Configuration:
//! - `GROUP_PRIMING_ENABLED` — enable group session priming (default: true)
//! - `GROUP_PRIMING_MAX_MEMBERS` — max members to include in roster (default: 50)

use serde::{Deserialize, Serialize};

/// Configuration for group session priming.
#[derive(Debug, Clone)]
pub struct GroupPrimingConfig {
    /// Whether group priming is enabled.
    pub enabled: bool,
    /// Maximum number of members to include in the roster.
    pub max_members: usize,
    /// Whether to include user IDs (some platforms may want privacy).
    pub include_ids: bool,
}

impl Default for GroupPrimingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_members: 50,
            include_ids: false,
        }
    }
}

impl GroupPrimingConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("GROUP_PRIMING_ENABLED") {
            config.enabled = val != "0" && !val.eq_ignore_ascii_case("false");
        }

        if let Ok(max) = std::env::var("GROUP_PRIMING_MAX_MEMBERS")
            && let Ok(m) = max.parse()
        {
            config.max_members = m;
        }

        if let Ok(val) = std::env::var("GROUP_PRIMING_INCLUDE_IDS") {
            config.include_ids = val == "1" || val.eq_ignore_ascii_case("true");
        }

        config
    }
}

/// A member of a group chat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMember {
    /// Platform-specific user ID.
    pub user_id: String,
    /// Display name.
    pub display_name: Option<String>,
    /// Username (e.g. @handle).
    pub username: Option<String>,
    /// Role in the group (admin, member, etc.).
    pub role: Option<String>,
    /// Whether this is the bot itself.
    pub is_bot: bool,
}

/// A group roster that can be injected into session context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupRoster {
    /// Group/chat name.
    pub group_name: Option<String>,
    /// Group ID.
    pub group_id: String,
    /// Channel type.
    pub channel: String,
    /// Members of the group.
    pub members: Vec<GroupMember>,
    /// Total member count (may be more than shown).
    pub total_count: usize,
}

impl GroupRoster {
    /// Create a new roster.
    pub fn new(group_id: impl Into<String>, channel: impl Into<String>) -> Self {
        Self {
            group_name: None,
            group_id: group_id.into(),
            channel: channel.into(),
            members: Vec::new(),
            total_count: 0,
        }
    }

    /// Add a member to the roster.
    pub fn add_member(&mut self, member: GroupMember) {
        self.members.push(member);
        self.total_count = self.total_count.max(self.members.len());
    }

    /// Format the roster as a context string for injection into the session.
    pub fn to_context_string(&self, config: &GroupPrimingConfig) -> String {
        if !config.enabled || self.members.is_empty() {
            return String::new();
        }

        let mut lines = Vec::new();
        lines.push(format!(
            "[Group Context] {} ({})",
            self.group_name.as_deref().unwrap_or("Unknown Group"),
            self.channel,
        ));

        let shown_members: Vec<&GroupMember> =
            self.members.iter().take(config.max_members).collect();

        for member in &shown_members {
            let name = member
                .display_name
                .as_deref()
                .or(member.username.as_deref())
                .unwrap_or("Unknown");

            let role_suffix = member
                .role
                .as_deref()
                .map(|r| format!(" ({})", r))
                .unwrap_or_default();

            let bot_suffix = if member.is_bot { " [bot]" } else { "" };

            if config.include_ids {
                lines.push(format!(
                    "  • {} [{}]{}{}",
                    name, member.user_id, role_suffix, bot_suffix
                ));
            } else {
                lines.push(format!("  • {}{}{}", name, role_suffix, bot_suffix));
            }
        }

        if self.total_count > shown_members.len() {
            lines.push(format!(
                "  ... and {} more members",
                self.total_count - shown_members.len()
            ));
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GroupPrimingConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_members, 50);
        assert!(!config.include_ids);
    }

    #[test]
    fn test_empty_roster() {
        let config = GroupPrimingConfig::default();
        let roster = GroupRoster::new("group-1", "telegram");
        assert!(roster.to_context_string(&config).is_empty());
    }

    #[test]
    fn test_roster_with_members() {
        let config = GroupPrimingConfig::default();
        let mut roster = GroupRoster::new("group-1", "telegram");
        roster.group_name = Some("Dev Team".to_string());

        roster.add_member(GroupMember {
            user_id: "u1".to_string(),
            display_name: Some("Alice".to_string()),
            username: Some("@alice".to_string()),
            role: Some("admin".to_string()),
            is_bot: false,
        });

        roster.add_member(GroupMember {
            user_id: "u2".to_string(),
            display_name: Some("Bob".to_string()),
            username: None,
            role: None,
            is_bot: false,
        });

        let ctx = roster.to_context_string(&config);
        assert!(ctx.contains("Dev Team"));
        assert!(ctx.contains("Alice"));
        assert!(ctx.contains("(admin)"));
        assert!(ctx.contains("Bob"));
        assert!(!ctx.contains("u1")); // IDs hidden by default
    }

    #[test]
    fn test_roster_with_ids() {
        let config = GroupPrimingConfig {
            include_ids: true,
            ..Default::default()
        };

        let mut roster = GroupRoster::new("g1", "discord");
        roster.add_member(GroupMember {
            user_id: "12345".to_string(),
            display_name: Some("Alice".to_string()),
            username: None,
            role: None,
            is_bot: false,
        });

        let ctx = roster.to_context_string(&config);
        assert!(ctx.contains("[12345]"));
    }

    #[test]
    fn test_roster_truncation() {
        let config = GroupPrimingConfig {
            max_members: 2,
            ..Default::default()
        };

        let mut roster = GroupRoster::new("g1", "tg");
        roster.total_count = 10;

        for i in 0..5 {
            roster.add_member(GroupMember {
                user_id: format!("u{}", i),
                display_name: Some(format!("User {}", i)),
                username: None,
                role: None,
                is_bot: false,
            });
        }
        roster.total_count = 10;

        let ctx = roster.to_context_string(&config);
        assert!(ctx.contains("User 0"));
        assert!(ctx.contains("User 1"));
        assert!(!ctx.contains("User 2")); // truncated
        assert!(ctx.contains("8 more members"));
    }

    #[test]
    fn test_bot_member() {
        let config = GroupPrimingConfig::default();
        let mut roster = GroupRoster::new("g1", "tg");
        roster.add_member(GroupMember {
            user_id: "bot".to_string(),
            display_name: Some("IronClaw".to_string()),
            username: None,
            role: None,
            is_bot: true,
        });

        let ctx = roster.to_context_string(&config);
        assert!(ctx.contains("[bot]"));
    }

    #[test]
    fn test_disabled_returns_empty() {
        let config = GroupPrimingConfig {
            enabled: false,
            ..Default::default()
        };

        let mut roster = GroupRoster::new("g1", "tg");
        roster.add_member(GroupMember {
            user_id: "u1".to_string(),
            display_name: Some("Alice".to_string()),
            username: None,
            role: None,
            is_bot: false,
        });

        assert!(roster.to_context_string(&config).is_empty());
    }
}
