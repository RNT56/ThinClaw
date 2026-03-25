use std::collections::HashMap;
use std::time::Duration;

use crate::config::helpers::{optional_env, parse_bool_env, parse_option_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Per-model thinking override.
#[derive(Debug, Clone)]
pub struct ModelThinkingOverride {
    pub enabled: bool,
    pub budget_tokens: Option<u32>,
}

/// Agent behavior configuration.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub max_parallel_jobs: usize,
    pub job_timeout: Duration,
    pub stuck_threshold: Duration,
    pub repair_check_interval: Duration,
    pub max_repair_attempts: u32,
    /// Whether to use planning before tool execution.
    pub use_planning: bool,
    /// Session idle timeout. Sessions inactive longer than this are pruned.
    pub session_idle_timeout: Duration,
    /// Allow chat to use filesystem/shell tools directly (bypass sandbox).
    pub allow_local_tools: bool,
    /// Maximum daily LLM spend in cents (e.g. 10000 = $100). None = unlimited.
    pub max_cost_per_day_cents: Option<u64>,
    /// Maximum LLM/tool actions per hour. None = unlimited.
    pub max_actions_per_hour: Option<u64>,
    /// Maximum tool-call iterations per agentic loop invocation. Default 50.
    pub max_tool_iterations: usize,
    /// Hard cap on context messages sent to the LLM. Default 200.
    pub max_context_messages: usize,
    /// Enable extended thinking / chain-of-thought reasoning.
    pub thinking_enabled: bool,
    /// Token budget for extended thinking.
    pub thinking_budget_tokens: u32,
    /// When true, skip tool approval checks entirely. For benchmarks/CI.
    pub auto_approve_tools: bool,
    /// Per-model thinking overrides. Key is a model name (exact or prefix match).
    /// When a model matches, its override takes precedence over global thinking settings.
    /// Format of env var: `MODEL_THINKING_OVERRIDE=model1:true:16000,model2:false`
    pub model_thinking_overrides: HashMap<String, ModelThinkingOverride>,
    /// Workspace mode: "unrestricted", "sandboxed", or "project".
    /// - unrestricted: full filesystem access (Cursor-style)
    /// - sandboxed: file tools confined to workspace_root
    /// - project: shell cwd = workspace_root, but file tools can access anywhere
    pub workspace_mode: String,
    /// Root directory for sandboxed/project modes. None = user home.
    pub workspace_root: Option<std::path::PathBuf>,
    /// Preferred notification channel for proactive agent messages (boot, bootstrap).
    /// Resolved from: NOTIFY_CHANNEL env var > settings.notifications.preferred_channel.
    pub notify_channel: Option<String>,
}

impl AgentConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            name: parse_optional_env("AGENT_NAME", settings.agent.name.clone())?,
            max_parallel_jobs: parse_optional_env(
                "AGENT_MAX_PARALLEL_JOBS",
                settings.agent.max_parallel_jobs as usize,
            )?,
            job_timeout: Duration::from_secs(parse_optional_env(
                "AGENT_JOB_TIMEOUT_SECS",
                settings.agent.job_timeout_secs,
            )?),
            stuck_threshold: Duration::from_secs(parse_optional_env(
                "AGENT_STUCK_THRESHOLD_SECS",
                settings.agent.stuck_threshold_secs,
            )?),
            repair_check_interval: Duration::from_secs(parse_optional_env(
                "SELF_REPAIR_CHECK_INTERVAL_SECS",
                settings.agent.repair_check_interval_secs,
            )?),
            max_repair_attempts: parse_optional_env(
                "SELF_REPAIR_MAX_ATTEMPTS",
                settings.agent.max_repair_attempts,
            )?,
            use_planning: parse_bool_env("AGENT_USE_PLANNING", settings.agent.use_planning)?,
            session_idle_timeout: Duration::from_secs(parse_optional_env(
                "SESSION_IDLE_TIMEOUT_SECS",
                settings.agent.session_idle_timeout_secs,
            )?),
            allow_local_tools: parse_bool_env(
                "ALLOW_LOCAL_TOOLS",
                settings.agent.allow_local_tools,
            )?,
            max_cost_per_day_cents: parse_option_env("MAX_COST_PER_DAY_CENTS")?,
            max_actions_per_hour: parse_option_env("MAX_ACTIONS_PER_HOUR")?,
            max_tool_iterations: parse_optional_env(
                "AGENT_MAX_TOOL_ITERATIONS",
                settings.agent.max_tool_iterations,
            )?,
            max_context_messages: parse_optional_env(
                "AGENT_MAX_CONTEXT_MESSAGES",
                settings.agent.max_context_messages,
            )?,
            thinking_enabled: parse_bool_env(
                "AGENT_THINKING_ENABLED",
                settings.agent.thinking_enabled,
            )?,
            thinking_budget_tokens: parse_optional_env(
                "AGENT_THINKING_BUDGET_TOKENS",
                settings.agent.thinking_budget_tokens,
            )?,
            auto_approve_tools: parse_bool_env(
                "AGENT_AUTO_APPROVE_TOOLS",
                settings.agent.auto_approve_tools,
            )?,
            model_thinking_overrides: parse_model_thinking_overrides()?,
            workspace_mode: optional_env("WORKSPACE_MODE")?
                .unwrap_or_else(|| "sandboxed".to_string()),
            workspace_root: optional_env("WORKSPACE_ROOT")?.map(std::path::PathBuf::from),
            notify_channel: optional_env("NOTIFY_CHANNEL")?
                .or_else(|| settings.notifications.preferred_channel.clone()),
        })
    }

    /// Resolve thinking config for a specific model.
    ///
    /// Checks `model_thinking_overrides` first (exact match, then prefix match),
    /// falling back to global `thinking_enabled` / `thinking_budget_tokens`.
    pub fn resolve_thinking_for_model(&self, model_name: &str) -> (bool, u32) {
        // Exact match first
        if let Some(ovr) = self.model_thinking_overrides.get(model_name) {
            return (
                ovr.enabled,
                ovr.budget_tokens.unwrap_or(self.thinking_budget_tokens),
            );
        }
        // Prefix match (e.g. "claude-sonnet" matches "claude-sonnet-4-20250514")
        for (pattern, ovr) in &self.model_thinking_overrides {
            if model_name.starts_with(pattern.as_str()) {
                return (
                    ovr.enabled,
                    ovr.budget_tokens.unwrap_or(self.thinking_budget_tokens),
                );
            }
        }
        // Global default
        (self.thinking_enabled, self.thinking_budget_tokens)
    }
}

/// Parse `MODEL_THINKING_OVERRIDE` env var into a map.
///
/// Format: `model1:true:16000,model2:false` (budget_tokens is optional)
fn parse_model_thinking_overrides() -> Result<HashMap<String, ModelThinkingOverride>, ConfigError> {
    let val = match optional_env("MODEL_THINKING_OVERRIDE")? {
        Some(v) if !v.is_empty() => v,
        _ => return Ok(HashMap::new()),
    };

    let mut map = HashMap::new();
    for entry in val.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let parts: Vec<&str> = entry.splitn(3, ':').collect();
        if parts.len() < 2 {
            return Err(ConfigError::InvalidValue {
                key: "MODEL_THINKING_OVERRIDE".to_string(),
                message: format!("malformed entry '{}', expected model:bool[:budget]", entry),
            });
        }
        let model = parts[0].trim().to_string();
        let enabled = parts[1]
            .trim()
            .parse::<bool>()
            .map_err(|_| ConfigError::InvalidValue {
                key: "MODEL_THINKING_OVERRIDE".to_string(),
                message: format!("invalid bool '{}' in entry '{}'", parts[1], entry),
            })?;
        let budget_tokens = if parts.len() == 3 {
            Some(
                parts[2]
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| ConfigError::InvalidValue {
                        key: "MODEL_THINKING_OVERRIDE".to_string(),
                        message: format!("invalid budget '{}' in entry '{}'", parts[2], entry),
                    })?,
            )
        } else {
            None
        };
        map.insert(
            model,
            ModelThinkingOverride {
                enabled,
                budget_tokens,
            },
        );
    }
    Ok(map)
}
