//! Authoritative-intent routing and compact tool-schema rendering.

use crate::llm::{ChatMessage, Role, ToolDefinition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AuthoritativeIntent {
    CurrentTime,
    TranscriptHistory,
    MemoryRecall,
    LocalState,
}

impl AuthoritativeIntent {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::CurrentTime => "current time/date",
            Self::TranscriptHistory => "conversation history",
            Self::MemoryRecall => "remembered context",
            Self::LocalState => "local/device state",
        }
    }

    pub(super) fn preferred_tools(self) -> &'static [&'static str] {
        match self {
            Self::CurrentTime => &["time"],
            Self::TranscriptHistory => &["session_search"],
            Self::MemoryRecall => &["memory_search", "memory_read", "external_memory_recall"],
            Self::LocalState => &["device_info", "homeassistant"],
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ToolRoutingDecision {
    pub(super) available_tools: Vec<ToolDefinition>,
    pub(super) tool_choice: &'static str,
    pub(super) unavailable_instruction: Option<String>,
}

fn last_user_message(messages: &[ChatMessage]) -> Option<&str> {
    messages
        .iter()
        .rev()
        .find(|message| matches!(message.role, Role::User))
        .map(|message| message.content.as_str())
}

pub(super) fn detect_authoritative_intent(messages: &[ChatMessage]) -> Option<AuthoritativeIntent> {
    let text = last_user_message(messages)?.to_ascii_lowercase();

    let current_time = [
        "what time",
        "current time",
        "what date",
        "current date",
        "what day is it",
        "today's date",
        "what day is today",
        "what date is today",
        "what day is tomorrow",
        "what date is tomorrow",
        "what day was yesterday",
        "what date was yesterday",
        // Keep "right now" anchored to a time word: a bare "right now"
        // needle hijacks unrelated requests like "deploy the app right now".
        "time right now",
        "date right now",
        "local time",
    ];
    if current_time.iter().any(|needle| text.contains(needle)) {
        return Some(AuthoritativeIntent::CurrentTime);
    }

    let transcript_history = [
        "earlier in this conversation",
        "earlier in the conversation",
        "earlier in this chat",
        "conversation history",
        "chat history",
        "what did i say",
        "what did we say",
        "previous message",
        "scroll back",
        "session history",
    ];
    if transcript_history
        .iter()
        .any(|needle| text.contains(needle))
    {
        return Some(AuthoritativeIntent::TranscriptHistory);
    }

    let memory_recall = [
        "what do you remember",
        "what do you know about me",
        "from memory",
        "did we decide",
        "what did we decide",
        "my preference",
        "my preferences",
        "remembered",
    ];
    if memory_recall.iter().any(|needle| text.contains(needle)) {
        return Some(AuthoritativeIntent::MemoryRecall);
    }

    let local_state = [
        "disk space",
        "device info",
        "disk usage",
        "memory usage",
        "cpu usage",
        "system uptime",
        "lights on",
        "thermostat",
        "temperature at home",
        "home assistant",
    ];
    if local_state.iter().any(|needle| text.contains(needle)) {
        return Some(AuthoritativeIntent::LocalState);
    }

    None
}

pub(super) fn authoritative_unavailable_instruction(intent: AuthoritativeIntent) -> String {
    format!(
        "The user is asking about {}. No authoritative tool for that intent is available in this turn. Do not guess or fabricate the answer; explain that the required tool is unavailable.",
        intent.label()
    )
}

pub(super) fn authoritative_shortlist_missing_instruction(intent: AuthoritativeIntent) -> String {
    format!(
        "The user is asking about {}. The preferred authoritative tool for that intent is not available in this turn. Use another available tool only if it can provide authoritative data; do not guess or fabricate the answer.",
        intent.label()
    )
}

fn schema_type_label(schema: &serde_json::Value) -> String {
    match schema.get("type") {
        Some(serde_json::Value::String(value)) => value.clone(),
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>()
            .join("|"),
        _ => "any".to_string(),
    }
}

fn schema_required_set(schema: &serde_json::Value) -> std::collections::HashSet<String> {
    schema
        .get("required")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn render_compact_schema_fields(schema: &serde_json::Value, depth: usize) -> Vec<String> {
    let Some(properties) = schema.get("properties").and_then(|value| value.as_object()) else {
        return Vec::new();
    };

    let required = schema_required_set(schema);
    let mut names = properties.keys().cloned().collect::<Vec<_>>();
    names.sort();

    names
        .into_iter()
        .filter_map(|name| {
            let property = properties.get(&name)?;
            let mut line = format!(
                "- {}{}: {}",
                name,
                if required.contains(&name) {
                    " (required)"
                } else {
                    ""
                },
                schema_type_label(property)
            );

            if let Some(enum_values) = property.get("enum").and_then(|value| value.as_array()) {
                let enum_preview = enum_values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .take(6)
                    .collect::<Vec<_>>();
                if !enum_preview.is_empty() {
                    line.push_str(&format!(" [{}]", enum_preview.join(", ")));
                }
            }

            if depth == 0 {
                if property.get("type").and_then(|value| value.as_str()) == Some("object") {
                    let nested = render_compact_schema_fields(property, depth + 1);
                    if !nested.is_empty() {
                        let nested_inline = nested
                            .into_iter()
                            .map(|value| value.trim_start_matches("- ").to_string())
                            .collect::<Vec<_>>()
                            .join("; ");
                        line.push_str(&format!(" {{ {} }}", nested_inline));
                    }
                } else if property.get("type").and_then(|value| value.as_str()) == Some("array")
                    && let Some(items) = property.get("items")
                {
                    let item_type = schema_type_label(items);
                    if item_type != "any" {
                        line.push_str(&format!(" of {}", item_type));
                    }
                    if items.get("type").and_then(|value| value.as_str()) == Some("object") {
                        let nested = render_compact_schema_fields(items, depth + 1);
                        if !nested.is_empty() {
                            let nested_inline = nested
                                .into_iter()
                                .map(|value| value.trim_start_matches("- ").to_string())
                                .collect::<Vec<_>>()
                                .join("; ");
                            line.push_str(&format!(" {{ {} }}", nested_inline));
                        }
                    }
                }
            }

            Some(line)
        })
        .collect()
}

pub(super) fn compact_tool_card(tool: &ToolDefinition) -> String {
    let mut required = schema_required_set(&tool.parameters)
        .into_iter()
        .collect::<Vec<_>>();
    required.sort();
    let required_line = if required.is_empty() {
        "none".to_string()
    } else {
        required.join(", ")
    };
    let fields = render_compact_schema_fields(&tool.parameters, 0);
    let fields_text = if fields.is_empty() {
        "- none".to_string()
    } else {
        fields.join("\n")
    };

    format!(
        "### {}\n{}\nRequired fields: {}\nFields:\n{}",
        tool.name, tool.description, required_line, fields_text
    )
}
