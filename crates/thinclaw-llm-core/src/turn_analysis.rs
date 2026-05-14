use crate::provider::{ChatMessage, Role, ToolCall};
use crate::smart_routing::{SmartRoutingConfig, TaskComplexity, classify_message};

const PLANNING_KEYWORDS: &[&str] = &[
    "plan",
    "planning",
    "design",
    "review",
    "implementation analysis",
    "debugging strategy",
    "architecture",
    "architectural",
    "analyze implementation",
    "migration strategy",
    "tradeoff",
    "trade-off",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcomeDigest {
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantToolPlanDigest {
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnAwareness {
    pub estimated_tokens: u32,
    pub has_vision: bool,
    pub last_user_objective: Option<String>,
    pub recent_user_messages: Vec<String>,
    pub recent_assistant_messages: Vec<String>,
    pub recent_assistant_tool_plans: Vec<AssistantToolPlanDigest>,
    pub recent_tool_outcomes: Vec<ToolOutcomeDigest>,
}

impl TurnAwareness {
    pub fn from_messages(messages: &[ChatMessage]) -> Self {
        let estimated_tokens = messages
            .iter()
            .map(|message| (message.estimated_chars() / 4) as u32)
            .sum();
        let has_vision = messages.iter().any(|message| {
            message
                .attachments
                .iter()
                .any(|attachment| attachment.mime_type.starts_with("image/"))
        });
        let last_user_objective = messages
            .iter()
            .rev()
            .find(|message| message.role == Role::User)
            .map(|message| message.content.trim().to_string())
            .filter(|content| !content.is_empty());
        let recent_user_messages = messages
            .iter()
            .rev()
            .filter(|message| message.role == Role::User)
            .filter_map(|message| {
                let trimmed = message.content.trim();
                (!trimmed.is_empty()).then(|| trim_text(trimmed, 600))
            })
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let recent_assistant_messages = messages
            .iter()
            .rev()
            .filter(|message| message.role == Role::Assistant)
            .filter_map(|message| {
                let trimmed = message.content.trim();
                (!trimmed.is_empty()).then(|| trim_text(trimmed, 520))
            })
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let recent_assistant_tool_plans = messages
            .iter()
            .rev()
            .filter(|message| message.role == Role::Assistant)
            .filter_map(|message| {
                let tool_calls = message.tool_calls.as_ref()?;
                if tool_calls.is_empty() {
                    return None;
                }
                Some(AssistantToolPlanDigest {
                    content: summarize_tool_calls(tool_calls),
                })
            })
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let recent_tool_outcomes = messages
            .iter()
            .rev()
            .filter(|message| message.role == Role::Tool)
            .filter_map(|message| {
                let trimmed = message.content.trim();
                if trimmed.is_empty() {
                    return None;
                }
                Some(ToolOutcomeDigest {
                    name: message.name.clone().unwrap_or_else(|| "tool".to_string()),
                    content: trim_text(trimmed, 380),
                    is_error: looks_like_tool_error(trimmed),
                })
            })
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        Self {
            estimated_tokens,
            has_vision,
            last_user_objective,
            recent_user_messages,
            recent_assistant_messages,
            recent_assistant_tool_plans,
            recent_tool_outcomes,
        }
    }

    pub fn tool_result_count(&self) -> usize {
        self.recent_tool_outcomes.len()
    }

    pub fn failure_count(&self) -> usize {
        self.recent_tool_outcomes
            .iter()
            .filter(|outcome| outcome.is_error)
            .count()
    }

    pub fn complexity_probe_text(&self) -> String {
        let mut parts = Vec::new();
        if !self.recent_user_messages.is_empty() {
            parts.push(format!(
                "User context:\n{}",
                self.recent_user_messages.join("\n")
            ));
        }
        if !self.recent_assistant_messages.is_empty() {
            parts.push(format!(
                "Assistant reasoning:\n{}",
                self.recent_assistant_messages.join("\n")
            ));
        }
        if !self.recent_assistant_tool_plans.is_empty() {
            parts.push(format!(
                "Recent tool plans:\n{}",
                self.recent_assistant_tool_plans
                    .iter()
                    .map(|plan| plan.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !self.recent_tool_outcomes.is_empty() {
            parts.push(format!(
                "Recent tool outcomes:\n{}",
                self.recent_tool_outcomes
                    .iter()
                    .map(|outcome| format!("- {}: {}", outcome.name, outcome.content))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        parts.join("\n\n")
    }

    pub fn context_snapshot(&self, tool_results_seen: Option<u32>) -> String {
        let recorded_tool_results = tool_results_seen.unwrap_or(self.tool_result_count() as u32);
        let last_failure = self
            .recent_tool_outcomes
            .iter()
            .rev()
            .find(|outcome| outcome.is_error)
            .map(|outcome| format!("{} failed with {}", outcome.name, outcome.content))
            .unwrap_or_else(|| "none".to_string());
        format!(
            "Estimated tokens: {}. Vision input: {}. Tool results recorded: {}. Recent tool failures: {}. Recent assistant tool plans: {}. Latest recorded failure: {}.",
            self.estimated_tokens,
            if self.has_vision { "yes" } else { "no" },
            recorded_tool_results,
            self.failure_count(),
            self.recent_assistant_tool_plans.len(),
            last_failure
        )
    }

    pub fn is_complex_or_planning_turn(&self) -> bool {
        if self.estimated_tokens >= 12_000 {
            return true;
        }
        if self.has_vision && self.tool_result_count() > 0 {
            return true;
        }

        let probe = self.complexity_probe_text();
        if !probe.is_empty()
            && classify_message(&probe, &SmartRoutingConfig::default()) == TaskComplexity::Complex
        {
            return true;
        }

        let lowered_probe = probe.to_ascii_lowercase();
        if PLANNING_KEYWORDS
            .iter()
            .any(|needle| lowered_probe.contains(needle))
        {
            return true;
        }

        self.recent_assistant_tool_plans.len() >= 2
            || self.tool_result_count() >= 4
            || (self.failure_count() > 0 && self.tool_result_count() >= 2)
    }
}

fn summarize_tool_calls(tool_calls: &[ToolCall]) -> String {
    tool_calls
        .iter()
        .map(|tool_call| {
            let arg_keys = tool_call
                .arguments
                .as_object()
                .map(|object| {
                    let mut keys = object.keys().cloned().collect::<Vec<_>>();
                    keys.sort();
                    keys.join(", ")
                })
                .unwrap_or_default();
            if arg_keys.is_empty() {
                format!("- {}", tool_call.name)
            } else {
                format!("- {}({})", tool_call.name, trim_text(&arg_keys, 120))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn looks_like_tool_error(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("\"status\":\"error\"")
        || lowered.contains("\"success\":false")
        || lowered.starts_with("error:")
        || lowered.contains(" failed")
        || lowered.contains("failure")
        || lowered.contains("blocked by")
        || lowered.contains("not permitted")
        || lowered.contains("limit reached")
}

fn trim_text(text: &str, max_chars: usize) -> String {
    let mut trimmed = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            trimmed.push_str("...");
            return trimmed;
        }
        trimmed.push(ch);
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_awareness_flags_complex_turns_from_full_context() {
        let messages = vec![
            ChatMessage::user("Please help with the migration."),
            ChatMessage::assistant("I will inspect the current architecture and propose a plan."),
            ChatMessage::assistant_with_tool_calls(
                Some("I should inspect the repo and compare implementations.".to_string()),
                vec![
                    ToolCall {
                        id: "call_1".to_string(),
                        name: "read_file".to_string(),
                        arguments: serde_json::json!({"path":"src/main.rs"}),
                    },
                    ToolCall {
                        id: "call_2".to_string(),
                        name: "search_code".to_string(),
                        arguments: serde_json::json!({"query":"migration"}),
                    },
                ],
            ),
            ChatMessage::tool_result(
                "call_1",
                "read_file",
                "{\"status\":\"error\",\"message\":\"file missing\"}",
            ),
            ChatMessage::user("Continue and give me the architecture review."),
        ];

        let awareness = TurnAwareness::from_messages(&messages);

        assert!(awareness.is_complex_or_planning_turn());
        assert_eq!(awareness.failure_count(), 1);
        assert_eq!(awareness.recent_assistant_tool_plans.len(), 1);
    }
}
