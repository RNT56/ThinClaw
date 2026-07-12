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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn intent_detection_covers_every_authoritative_domain() {
        let cases = [
            (
                "What date is tomorrow?",
                AuthoritativeIntent::CurrentTime,
                "current time/date",
                &["time"][..],
            ),
            (
                "What did I say earlier in this chat?",
                AuthoritativeIntent::TranscriptHistory,
                "conversation history",
                &["session_search"][..],
            ),
            (
                "What do you remember about my preferences?",
                AuthoritativeIntent::MemoryRecall,
                "remembered context",
                &["memory_search", "memory_read", "external_memory_recall"][..],
            ),
            (
                "How much disk space is left on this device?",
                AuthoritativeIntent::LocalState,
                "local/device state",
                &["device_info", "homeassistant"][..],
            ),
        ];

        for (message, expected, label, tools) in cases {
            let detected = detect_authoritative_intent(&[ChatMessage::user(message)]);
            assert_eq!(detected, Some(expected));
            assert_eq!(expected.label(), label);
            assert_eq!(expected.preferred_tools(), tools);
        }
    }

    #[test]
    fn intent_detection_uses_latest_user_message_and_ignores_unrelated_text() {
        let messages = [
            ChatMessage::user("What time is it?"),
            ChatMessage::assistant("I need the time tool."),
            ChatMessage::user("Deploy the app right now."),
        ];
        assert_eq!(detect_authoritative_intent(&messages), None);
        assert_eq!(detect_authoritative_intent(&[]), None);
        assert_eq!(
            detect_authoritative_intent(&[ChatMessage::assistant("What time is it?")]),
            None
        );
    }

    #[test]
    fn unavailable_instructions_name_the_domain_and_forbid_guessing() {
        let unavailable =
            authoritative_unavailable_instruction(AuthoritativeIntent::TranscriptHistory);
        assert!(unavailable.contains("conversation history"));
        assert!(unavailable.contains("No authoritative tool"));
        assert!(unavailable.contains("Do not guess or fabricate"));

        let shortlist =
            authoritative_shortlist_missing_instruction(AuthoritativeIntent::MemoryRecall);
        assert!(shortlist.contains("remembered context"));
        assert!(shortlist.contains("preferred authoritative tool"));
        assert!(shortlist.contains("do not guess or fabricate"));
    }

    #[test]
    fn compact_tool_card_renders_nested_arrays_enums_and_required_fields() {
        let tool = ToolDefinition {
            name: "deploy".to_string(),
            description: "Deploy a service".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["config", "mode"],
                "properties": {
                    "anything": {},
                    "config": {
                        "type": "object",
                        "required": ["enabled"],
                        "properties": {"enabled": {"type": "boolean"}}
                    },
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {"name": {"type": "string"}}
                        }
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["a", "b", "c", "d", "e", "f", "g"]
                    },
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "variant": {"type": ["string", "null"]}
                }
            }),
        };

        let card = compact_tool_card(&tool);
        assert!(card.starts_with("### deploy\nDeploy a service"));
        assert!(card.contains("Required fields: config, mode"));
        assert!(card.contains("anything: any"));
        assert!(card.contains("config (required): object { enabled (required): boolean }"));
        assert!(card.contains("items: array of object { name: string }"));
        assert!(card.contains("mode (required): string [a, b, c, d, e, f]"));
        assert!(card.contains("tags: array of string"));
        assert!(card.contains("variant: string|null"));
        assert!(!card.contains(", g]"));
    }

    #[test]
    fn compact_tool_card_handles_schema_without_properties() {
        let card = compact_tool_card(&ToolDefinition {
            name: "ping".to_string(),
            description: "Check availability".to_string(),
            parameters: json!({"type": "object"}),
        });

        assert_eq!(
            card,
            "### ping\nCheck availability\nRequired fields: none\nFields:\n- none"
        );
    }
}
