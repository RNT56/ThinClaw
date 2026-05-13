//! Root-independent dispatcher helper functions.

use thinclaw_llm_core::{ChatMessage, Role, sanitize_tool_messages};

use crate::session::PendingAuthMode;

/// Parsed auth result fields for emitting auth-required status updates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedAuthData {
    pub auth_url: Option<String>,
    pub setup_url: Option<String>,
    pub auth_mode: Option<String>,
    pub auth_status: Option<String>,
    pub shared_auth_provider: Option<String>,
    pub missing_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAuthRequest {
    pub extension_name: String,
    pub instructions: String,
    pub auth_mode: PendingAuthMode,
    pub auth_status: String,
}

/// Extract auth fields from a tool_auth/tool_activate JSON result.
pub fn parse_auth_result_json(output: Option<&str>) -> ParsedAuthData {
    let parsed = output.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
    ParsedAuthData {
        auth_url: parsed
            .as_ref()
            .and_then(|v| v.get("auth_url"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        setup_url: parsed
            .as_ref()
            .and_then(|v| v.get("setup_url"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        auth_mode: parsed
            .as_ref()
            .and_then(|v| v.get("auth_mode"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        auth_status: parsed
            .as_ref()
            .and_then(|v| v.get("auth_status").or_else(|| v.get("status")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        shared_auth_provider: parsed
            .as_ref()
            .and_then(|v| v.get("shared_auth_provider"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        missing_scopes: parsed
            .as_ref()
            .and_then(|v| v.get("missing_scopes"))
            .and_then(|v| v.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    }
}

/// Check whether a tool auth/activation result requires more authentication.
pub fn check_auth_required_json(
    tool_name: &str,
    output: Option<&str>,
) -> Option<PendingAuthRequest> {
    if tool_name != "tool_auth" && tool_name != "tool_activate" {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(output?).ok()?;
    let auth_status = parsed
        .get("auth_status")
        .or_else(|| parsed.get("status"))
        .and_then(|v| v.as_str())?;
    if !matches!(
        auth_status,
        "awaiting_token" | "awaiting_authorization" | "needs_reauth" | "insufficient_scope"
    ) {
        return None;
    }
    let name = parsed.get("name")?.as_str()?.to_string();
    let auth_mode = parsed.get("auth_mode").and_then(|v| v.as_str()).unwrap_or(
        if auth_status == "awaiting_token" {
            "manual_token"
        } else {
            "oauth"
        },
    );
    let instructions = parsed
        .get("instructions")
        .and_then(|v| v.as_str())
        .unwrap_or(if auth_mode == "oauth" {
            "Open the browser authentication flow to continue."
        } else {
            "Please provide your API token/key."
        })
        .to_string();
    Some(PendingAuthRequest {
        extension_name: name,
        instructions,
        auth_mode: if auth_mode == "manual_token" && auth_status == "awaiting_token" {
            PendingAuthMode::ManualToken
        } else {
            PendingAuthMode::ExternalOAuth
        },
        auth_status: auth_status.to_string(),
    })
}

/// Truncate a string to `max_chars`, appending an ellipsis when truncated.
pub fn truncate_preview(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        let mut end = max_chars;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

/// Collapse output into a single-line preview for status display.
pub fn truncate_for_preview(output: &str, max_chars: usize) -> String {
    let collapsed: String = output
        .chars()
        .take(max_chars + 50)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() > max_chars {
        let byte_offset = collapsed
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(collapsed.len());
        format!("{}...", &collapsed[..byte_offset])
    } else {
        collapsed
    }
}

/// Compact messages for retry after a context-length-exceeded error.
pub fn compact_messages_for_retry(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut compacted = Vec::new();
    let last_user_idx = messages.iter().rposition(|m| m.role == Role::User);

    if let Some(idx) = last_user_idx {
        for msg in &messages[..idx] {
            if msg.role == Role::System {
                compacted.push(msg.clone());
            }
        }
        if idx > 0 {
            compacted.push(ChatMessage::system(
                "[Note: Earlier conversation history was automatically compacted \
                 to fit within the context window. The most recent exchange is preserved below.]",
            ));
        }
        compacted.extend_from_slice(&messages[idx..]);
    } else {
        for msg in messages {
            if msg.role == Role::System {
                compacted.push(msg.clone());
            }
        }
        for msg in messages {
            if msg.role != Role::System {
                compacted.push(msg.clone());
            }
        }
    }

    sanitize_tool_messages(&mut compacted);
    compacted
}

#[cfg(test)]
mod tests {
    use super::truncate_for_preview;

    #[test]
    fn test_truncate_short_input() {
        assert_eq!(truncate_for_preview("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_empty_input() {
        assert_eq!(truncate_for_preview("", 10), "");
    }

    #[test]
    fn test_truncate_exact_length() {
        assert_eq!(truncate_for_preview("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_over_limit() {
        let result = truncate_for_preview("hello world, this is long", 10);
        assert!(result.ends_with("..."));
        assert_eq!(result, "hello worl...");
    }

    #[test]
    fn test_truncate_collapses_newlines() {
        let result = truncate_for_preview("line1\nline2\nline3", 100);
        assert!(!result.contains('\n'));
        assert_eq!(result, "line1 line2 line3");
    }

    #[test]
    fn test_truncate_collapses_whitespace() {
        let result = truncate_for_preview("hello   world", 100);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_truncate_multibyte_utf8() {
        let input = "😀😁😂🤣😃😄😅😆😉😊";
        let result = truncate_for_preview(input, 5);
        assert!(result.ends_with("..."));
        assert_eq!(result, "😀😁😂🤣😃...");
    }

    #[test]
    fn test_truncate_cjk_characters() {
        let input = "你好世界测试数据很长的字符串";
        let result = truncate_for_preview(input, 4);
        assert_eq!(result, "你好世界...");
    }

    #[test]
    fn test_truncate_mixed_multibyte_and_ascii() {
        let input = "hello 世界 foo";
        let result = truncate_for_preview(input, 8);
        assert_eq!(result, "hello 世界...");
    }
}
