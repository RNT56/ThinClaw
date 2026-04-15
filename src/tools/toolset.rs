//! Composable toolset system.
//!
//! Provides named collections of tools ("toolsets") that can be assigned
//! to agents, subagents, and capability scopes. Toolsets can include
//! other toolsets for composition.
//!
//! Built-in toolsets: `web`, `dev`, `memory`, `safe`, `full`,
//! `communication`, `automation`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// A named set of tools, optionally including other toolsets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Toolset {
    /// Unique name of this toolset.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Direct tool names in this toolset.
    pub tools: Vec<String>,
    /// Other toolsets to include (recursive).
    #[serde(default)]
    pub includes: Vec<String>,
}

/// Registry of named toolsets with resolution support.
#[derive(Debug, Clone, Default)]
pub struct ToolsetRegistry {
    toolsets: HashMap<String, Toolset>,
}

impl ToolsetRegistry {
    /// Create a new registry with built-in toolsets pre-loaded.
    pub fn new() -> Self {
        let mut reg = Self::default();
        reg.register_builtins();
        reg
    }

    /// Register all built-in toolsets.
    fn register_builtins(&mut self) {
        // Web tools
        self.register(Toolset {
            name: "web".to_string(),
            description: "Web search and HTTP tools".to_string(),
            tools: vec!["http".to_string(), "browser".to_string()],
            includes: vec![],
        });

        // Development tools
        self.register(Toolset {
            name: "dev".to_string(),
            description: "Software development tools (shell, file ops, code editing)".to_string(),
            tools: vec![
                "shell".to_string(),
                "read_file".to_string(),
                "write_file".to_string(),
                "list_dir".to_string(),
                "apply_patch".to_string(),
                "grep".to_string(),
                "process".to_string(),
            ],
            includes: vec![],
        });

        // Memory tools
        self.register(Toolset {
            name: "memory".to_string(),
            description: "Memory and knowledge management tools".to_string(),
            tools: vec![
                "memory_search".to_string(),
                "memory_write".to_string(),
                "memory_read".to_string(),
                "memory_tree".to_string(),
                "memory_delete".to_string(),
                "session_search".to_string(),
            ],
            includes: vec![],
        });

        // Safe tools (no shell/file access)
        self.register(Toolset {
            name: "safe".to_string(),
            description: "Safe tools with no filesystem or shell access".to_string(),
            tools: vec![
                "echo".to_string(),
                "time".to_string(),
                "json".to_string(),
                "device_info".to_string(),
                "todo".to_string(),
                "clarify".to_string(),
                "agent_think".to_string(),
                "emit_user_message".to_string(),
            ],
            includes: vec!["web".to_string(), "memory".to_string()],
        });

        // Communication tools
        self.register(Toolset {
            name: "communication".to_string(),
            description: "Messaging and communication tools".to_string(),
            tools: vec![
                "send_message".to_string(),
                "telegram_actions".to_string(),
                "discord_actions".to_string(),
                "slack_actions".to_string(),
                "apple_mail".to_string(),
                "tts".to_string(),
            ],
            includes: vec![],
        });

        // Automation tools
        self.register(Toolset {
            name: "automation".to_string(),
            description: "Job scheduling, routines, and automation tools".to_string(),
            tools: vec![
                "create_job".to_string(),
                "list_jobs".to_string(),
                "job_status".to_string(),
                "cancel_job".to_string(),
                "routine_create".to_string(),
                "routine_list".to_string(),
                "routine_update".to_string(),
                "routine_delete".to_string(),
                "routine_history".to_string(),
            ],
            includes: vec![],
        });

        // Agent management
        self.register(Toolset {
            name: "agents".to_string(),
            description: "Multi-agent management tools".to_string(),
            tools: vec![
                "create_agent".to_string(),
                "list_agents".to_string(),
                "update_agent".to_string(),
                "remove_agent".to_string(),
                "message_agent".to_string(),
                "spawn_subagent".to_string(),
                "list_subagents".to_string(),
                "cancel_subagent".to_string(),
            ],
            includes: vec![],
        });

        // Full access — everything
        self.register(Toolset {
            name: "full".to_string(),
            description: "Full access to all tools".to_string(),
            tools: vec![],
            includes: vec![
                "safe".to_string(),
                "dev".to_string(),
                "communication".to_string(),
                "automation".to_string(),
                "agents".to_string(),
            ],
        });
    }

    /// Register a toolset.
    pub fn register(&mut self, toolset: Toolset) {
        self.toolsets.insert(toolset.name.clone(), toolset);
    }

    /// Remove a toolset.
    pub fn unregister(&mut self, name: &str) -> Option<Toolset> {
        self.toolsets.remove(name)
    }

    /// Get a toolset by name.
    pub fn get(&self, name: &str) -> Option<&Toolset> {
        self.toolsets.get(name)
    }

    /// List all registered toolsets.
    pub fn list(&self) -> Vec<&Toolset> {
        let mut sets: Vec<&Toolset> = self.toolsets.values().collect();
        sets.sort_by_key(|t| &t.name);
        sets
    }

    /// Resolve a toolset name to a flat list of tool names.
    ///
    /// Recursively resolves includes, with cycle detection.
    pub fn resolve(&self, name: &str) -> Result<Vec<String>, String> {
        let mut resolved = Vec::new();
        let mut visited = HashSet::new();
        let mut stack = HashSet::new();
        self.resolve_inner(name, &mut resolved, &mut visited, &mut stack)?;
        Ok(resolved)
    }

    fn resolve_inner(
        &self,
        name: &str,
        resolved: &mut Vec<String>,
        visited: &mut HashSet<String>,
        stack: &mut HashSet<String>,
    ) -> Result<(), String> {
        if stack.contains(name) {
            return Err(format!("Cycle detected in toolset resolution: {}", name));
        }
        if !visited.insert(name.to_string()) {
            return Ok(());
        }
        stack.insert(name.to_string());

        let toolset = self
            .toolsets
            .get(name)
            .ok_or_else(|| format!("Unknown toolset: {}", name))?;

        // Add direct tools
        for tool in &toolset.tools {
            if !resolved.contains(tool) {
                resolved.push(tool.clone());
            }
        }

        // Resolve includes
        for include in &toolset.includes {
            self.resolve_inner(include, resolved, visited, stack)?;
        }

        stack.remove(name);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_toolsets_exist() {
        let reg = ToolsetRegistry::new();
        assert!(reg.get("web").is_some());
        assert!(reg.get("dev").is_some());
        assert!(reg.get("memory").is_some());
        assert!(reg.get("safe").is_some());
        assert!(reg.get("full").is_some());
        assert!(reg.get("communication").is_some());
        assert!(reg.get("automation").is_some());
        assert!(reg.get("agents").is_some());
    }

    #[test]
    fn test_resolve_simple() {
        let reg = ToolsetRegistry::new();
        let tools = reg.resolve("web").unwrap();
        assert!(tools.contains(&"http".to_string()));
        assert!(tools.contains(&"browser".to_string()));
    }

    #[test]
    fn test_resolve_with_includes() {
        let reg = ToolsetRegistry::new();
        let tools = reg.resolve("safe").unwrap();
        // Should include direct tools and web+memory includes
        assert!(tools.contains(&"echo".to_string()));
        assert!(tools.contains(&"http".to_string())); // from web include
        assert!(tools.contains(&"memory_search".to_string())); // from memory include
    }

    #[test]
    fn test_resolve_full() {
        let reg = ToolsetRegistry::new();
        let tools = reg.resolve("full").unwrap();
        // Full should include everything
        assert!(tools.contains(&"shell".to_string()));
        assert!(tools.contains(&"http".to_string()));
        assert!(tools.contains(&"memory_search".to_string()));
        assert!(tools.contains(&"send_message".to_string()));
        assert!(tools.contains(&"create_job".to_string()));
        assert!(tools.contains(&"create_agent".to_string()));
    }

    #[test]
    fn test_resolve_no_duplicates() {
        let reg = ToolsetRegistry::new();
        let tools = reg.resolve("full").unwrap();
        let unique: HashSet<&String> = tools.iter().collect();
        assert_eq!(tools.len(), unique.len());
    }

    #[test]
    fn test_resolve_unknown() {
        let reg = ToolsetRegistry::new();
        let err = reg.resolve("nonexistent").unwrap_err();
        assert!(err.contains("Unknown toolset"));
    }

    #[test]
    fn test_cycle_detection() {
        let mut reg = ToolsetRegistry::new();
        reg.register(Toolset {
            name: "a".to_string(),
            description: "cycle test".to_string(),
            tools: vec![],
            includes: vec!["b".to_string()],
        });
        reg.register(Toolset {
            name: "b".to_string(),
            description: "cycle test".to_string(),
            tools: vec![],
            includes: vec!["a".to_string()],
        });
        let err = reg.resolve("a").unwrap_err();
        assert!(err.contains("Cycle"));
    }

    #[test]
    fn test_custom_toolset() {
        let mut reg = ToolsetRegistry::new();
        reg.register(Toolset {
            name: "custom".to_string(),
            description: "My custom set".to_string(),
            tools: vec!["shell".to_string(), "read_file".to_string()],
            includes: vec!["web".to_string()],
        });

        let tools = reg.resolve("custom").unwrap();
        assert!(tools.contains(&"shell".to_string()));
        assert!(tools.contains(&"read_file".to_string()));
        assert!(tools.contains(&"http".to_string())); // from web
    }

    #[test]
    fn test_shared_include_dag_is_allowed() {
        let mut reg = ToolsetRegistry::new();
        reg.register(Toolset {
            name: "shared".to_string(),
            description: "Shared dependency".to_string(),
            tools: vec!["json".to_string()],
            includes: vec![],
        });
        reg.register(Toolset {
            name: "left".to_string(),
            description: "Left branch".to_string(),
            tools: vec!["echo".to_string()],
            includes: vec!["shared".to_string()],
        });
        reg.register(Toolset {
            name: "right".to_string(),
            description: "Right branch".to_string(),
            tools: vec!["time".to_string()],
            includes: vec!["shared".to_string()],
        });
        reg.register(Toolset {
            name: "root".to_string(),
            description: "Root".to_string(),
            tools: vec![],
            includes: vec!["left".to_string(), "right".to_string()],
        });

        let tools = reg.resolve("root").unwrap();
        assert!(tools.contains(&"echo".to_string()));
        assert!(tools.contains(&"time".to_string()));
        assert!(tools.contains(&"json".to_string()));
    }

    #[test]
    fn test_list_sorted() {
        let reg = ToolsetRegistry::new();
        let list = reg.list();
        let names: Vec<&str> = list.iter().map(|t| t.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }
}
