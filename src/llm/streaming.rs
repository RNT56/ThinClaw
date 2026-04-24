//! Shared LLM streaming helpers.

use std::collections::{HashMap, HashSet};

use crate::error::LlmError;
use crate::llm::provider::{FinishReason, StreamChunk, StreamChunkStream, ToolCall};

/// Simulate a streaming response by word-chunking a completed response.
///
/// This is deliberately separated from provider-native streaming so callers can
/// tell whether they are receiving real upstream deltas or a compatibility
/// fallback.
pub fn simulate_stream_from_response(
    content: String,
    provider_model: Option<String>,
    cost_usd: Option<f64>,
    thinking_content: Option<String>,
    tool_calls: Vec<ToolCall>,
    input_tokens: u32,
    output_tokens: u32,
    finish_reason: FinishReason,
) -> StreamChunkStream {
    Box::pin(futures::stream::unfold(
        SimState::new(
            content,
            provider_model,
            cost_usd,
            thinking_content,
            tool_calls,
            input_tokens,
            output_tokens,
            finish_reason,
        ),
        |mut state| async move {
            if let Some(ref mut thinking) = state.thinking {
                if !thinking.is_empty() {
                    let chunk = std::mem::take(thinking);
                    state.thinking = None;
                    return Some((Ok(StreamChunk::ReasoningDelta(chunk)), state));
                }
                state.thinking = None;
            }

            if !state.words.is_empty() {
                let word = state.words.remove(0);
                return Some((Ok(StreamChunk::Text(word)), state));
            }

            if !state.tool_calls.is_empty() {
                let tc = state.tool_calls.remove(0);
                return Some((Ok(StreamChunk::ToolCall(tc)), state));
            }

            if !state.done {
                state.done = true;
                return Some((
                    Ok(StreamChunk::Done {
                        provider_model: state.provider_model.clone(),
                        cost_usd: state.cost_usd,
                        input_tokens: state.input_tokens,
                        output_tokens: state.output_tokens,
                        finish_reason: state.finish_reason,
                    }),
                    state,
                ));
            }

            None
        },
    ))
}

struct SimState {
    provider_model: Option<String>,
    cost_usd: Option<f64>,
    thinking: Option<String>,
    words: Vec<String>,
    tool_calls: Vec<ToolCall>,
    input_tokens: u32,
    output_tokens: u32,
    finish_reason: FinishReason,
    done: bool,
}

impl SimState {
    fn new(
        content: String,
        provider_model: Option<String>,
        cost_usd: Option<f64>,
        thinking: Option<String>,
        tool_calls: Vec<ToolCall>,
        input_tokens: u32,
        output_tokens: u32,
        finish_reason: FinishReason,
    ) -> Self {
        let mut words = Vec::new();
        let mut buf = String::new();
        for word in content.split_inclusive(char::is_whitespace) {
            buf.push_str(word);
            if buf.len() >= 20 {
                words.push(std::mem::take(&mut buf));
            }
        }
        if !buf.is_empty() {
            words.push(buf);
        }

        Self {
            provider_model,
            cost_usd,
            thinking,
            words,
            tool_calls,
            input_tokens,
            output_tokens,
            finish_reason,
            done: false,
        }
    }
}

/// Accumulate and deduplicate provider streaming tool-call chunks.
pub fn merge_streamed_tool_calls(
    mut tool_calls: Vec<ToolCall>,
    partial_tool_calls: HashMap<u32, (String, String, String)>,
) -> Vec<ToolCall> {
    for (_idx, (id, name, args)) in partial_tool_calls {
        if name.is_empty() {
            continue;
        }

        let arguments: serde_json::Value =
            serde_json::from_str(&args).unwrap_or(serde_json::Value::Null);

        let safe_id = if id.trim().is_empty() {
            format!("call_{}", uuid::Uuid::new_v4().simple())
        } else {
            id
        };

        if let Some(existing) = tool_calls.iter_mut().find(|tc| tc.id == safe_id) {
            if existing.name.is_empty() {
                existing.name = name;
            }
            if existing.arguments.is_null() && !arguments.is_null() {
                existing.arguments = arguments;
            }
            continue;
        }

        tool_calls.push(ToolCall {
            id: safe_id,
            name,
            arguments,
        });
    }

    let mut seen_ids = HashSet::new();
    tool_calls.retain(|tc| seen_ids.insert(tc.id.clone()));
    tool_calls
}

/// Normalize a tool call name returned by an OpenAI-compatible provider.
pub fn normalize_tool_name(name: &str, known_tools: &HashSet<String>) -> String {
    if known_tools.contains(name) {
        return name.to_string();
    }

    if let Some(stripped) = name.strip_prefix("proxy_")
        && known_tools.contains(stripped)
    {
        return stripped.to_string();
    }

    name.to_string()
}

pub fn native_required_error(provider: impl Into<String>, model: impl Into<String>) -> LlmError {
    LlmError::StreamingUnsupported {
        provider: provider.into(),
        model: model.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_streamed_tool_calls_accumulates_and_deduplicates_deltas() {
        let complete = ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "rust"}),
        };
        let partials = HashMap::from([
            (
                0,
                (
                    "call_1".to_string(),
                    "search".to_string(),
                    r#"{"q":"ignored"}"#.to_string(),
                ),
            ),
            (
                1,
                (
                    "call_2".to_string(),
                    "fetch".to_string(),
                    r#"{"url":"https://example.com"}"#.to_string(),
                ),
            ),
        ]);

        let merged = merge_streamed_tool_calls(vec![complete], partials);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "call_1");
        assert_eq!(merged[0].arguments, serde_json::json!({"q": "rust"}));
        assert_eq!(merged[1].id, "call_2");
        assert_eq!(
            merged[1].arguments,
            serde_json::json!({"url": "https://example.com"})
        );
    }

    #[test]
    fn merge_streamed_tool_calls_uses_null_for_malformed_arguments() {
        let partials = HashMap::from([(
            0,
            (
                "call_bad".to_string(),
                "broken".to_string(),
                "{not-json".to_string(),
            ),
        )]);

        let merged = merge_streamed_tool_calls(Vec::new(), partials);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "call_bad");
        assert!(merged[0].arguments.is_null());
    }
}
