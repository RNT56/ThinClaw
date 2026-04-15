//! Response cleaning and thinking-tag stripping for LLM outputs.
//!
//! LLMs may include reasoning/thinking tags (e.g. `<thinking>`, `<|think|>`)
//! in their responses that should be stripped before presenting to users.
//! This module handles:
//! - Regex-based thinking tag detection and removal
//! - Code region awareness (preserves tags inside code blocks)
//! - `<final>` tag extraction
//! - Response cleanup (collapse whitespace, strip artifacts)
//! - Tool call recovery from text content
//! - Success evaluation parsing

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::llm::{ToolCall, ToolDefinition};

/// Quick-check: bail early if no reasoning/final tags are present at all.
static QUICK_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<\s*/?\s*(?:think(?:ing)?|thought|thoughts|antthinking|reasoning|reflection|scratchpad|inner_monologue|final)\b").expect("QUICK_TAG_RE")
});

/// Matches thinking/reasoning open and close tags. Capture group 1 is "/" for close tags.
/// Whitespace-tolerant, case-insensitive, attribute-aware.
static THINKING_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<\s*(/?)\s*(?:think(?:ing)?|thought|thoughts|antthinking|reasoning|reflection|scratchpad|inner_monologue)\b[^<>]*>").expect("THINKING_TAG_RE")
});

/// Matches `<final>` / `</final>` tags. Capture group 1 is "/" for close tags.
static FINAL_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<\s*(/?)\s*final\b[^<>]*>").expect("FINAL_TAG_RE"));

/// Matches pipe-delimited reasoning tags: `<|think|>...<|/think|>` etc.
static PIPE_REASONING_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<\|(/?)\s*(?:think(?:ing)?|thought|thoughts|antthinking|reasoning|reflection|scratchpad|inner_monologue)\|>").expect("PIPE_REASONING_TAG_RE")
});

/// Result of success evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessEvaluation {
    pub success: bool,
    pub confidence: f64,
    pub reasoning: String,
    #[serde(default)]
    pub issues: Vec<String>,
    #[serde(default)]
    pub suggestions: Vec<String>,
}

/// Extract JSON from text that might contain other content.
pub fn extract_json(text: &str) -> Option<&str> {
    // Find the first { and last } to extract JSON
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if start < end {
        Some(&text[start..=end])
    } else {
        None
    }
}

/// A byte range in the source text that is inside a code region (fenced or inline).
#[derive(Debug, Clone, Copy)]
struct CodeRegion {
    start: usize,
    end: usize,
}

/// Detect fenced code blocks (``` and ~~~) and inline backtick spans.
/// Returns sorted `Vec<CodeRegion>` of byte ranges. Tags inside these ranges are
/// skipped during stripping so code examples mentioning `<thinking>` are preserved.
fn find_code_regions(text: &str) -> Vec<CodeRegion> {
    let mut regions = Vec::new();

    // Fenced code blocks: line starting with 3+ backticks or tildes
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        // Must be at start of line (i==0 or previous char is \n)
        if i > 0 && bytes[i - 1] != b'\n' {
            if let Some(nl) = text[i..].find('\n') {
                i += nl + 1;
            } else {
                break;
            }
            continue;
        }

        // Skip optional leading whitespace
        let line_start = i;
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }

        let fence_char = if i < bytes.len() && (bytes[i] == b'`' || bytes[i] == b'~') {
            bytes[i]
        } else {
            // Not a fence line, skip to next line
            if let Some(nl) = text[i..].find('\n') {
                i += nl + 1;
            } else {
                break;
            }
            continue;
        };

        // Count fence chars
        let fence_start = i;
        while i < bytes.len() && bytes[i] == fence_char {
            i += 1;
        }
        let fence_len = i - fence_start;
        if fence_len < 3 {
            // Not a real fence
            if let Some(nl) = text[i..].find('\n') {
                i += nl + 1;
            } else {
                break;
            }
            continue;
        }

        // Skip rest of opening fence line (info string)
        if let Some(nl) = text[i..].find('\n') {
            i += nl + 1;
        } else {
            // Fence at EOF with no content — region extends to end
            regions.push(CodeRegion {
                start: line_start,
                end: bytes.len(),
            });
            break;
        }

        // Find closing fence: line starting with >= fence_len of same char
        let content_start = i;
        let mut found_close = false;
        while i < bytes.len() {
            let cl_start = i;
            // Skip optional leading whitespace
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == fence_char {
                let close_fence_start = i;
                while i < bytes.len() && bytes[i] == fence_char {
                    i += 1;
                }
                let close_fence_len = i - close_fence_start;
                // Must be at least as long, and rest of line must be empty/whitespace
                if close_fence_len >= fence_len {
                    // Skip to end of line
                    while i < bytes.len() && bytes[i] != b'\n' {
                        if bytes[i] != b' ' && bytes[i] != b'\t' {
                            break;
                        }
                        i += 1;
                    }
                    if i >= bytes.len() || bytes[i] == b'\n' {
                        if i < bytes.len() {
                            i += 1; // skip the \n
                        }
                        regions.push(CodeRegion {
                            start: line_start,
                            end: i,
                        });
                        found_close = true;
                        break;
                    }
                }
            }
            // Not a closing fence, skip to next line
            if let Some(nl) = text[cl_start..].find('\n') {
                i = cl_start + nl + 1;
            } else {
                i = bytes.len();
                break;
            }
        }
        if !found_close {
            // Unclosed fence extends to EOF
            let _ = content_start; // suppress unused warning
            regions.push(CodeRegion {
                start: line_start,
                end: bytes.len(),
            });
        }
    }

    // Inline backtick spans (not inside fenced blocks)
    let mut j = 0;
    while j < bytes.len() {
        if bytes[j] != b'`' {
            j += 1;
            continue;
        }
        // Inside a fenced block? Skip
        if regions.iter().any(|r| j >= r.start && j < r.end) {
            j += 1;
            continue;
        }
        // Count opening backtick run
        let tick_start = j;
        while j < bytes.len() && bytes[j] == b'`' {
            j += 1;
        }
        let tick_len = j - tick_start;
        // Find matching closing run of exactly tick_len backticks
        let search_from = j;
        let mut found = false;
        let mut k = search_from;
        while k < bytes.len() {
            if bytes[k] != b'`' {
                k += 1;
                continue;
            }
            let close_start = k;
            while k < bytes.len() && bytes[k] == b'`' {
                k += 1;
            }
            if k - close_start == tick_len {
                regions.push(CodeRegion {
                    start: tick_start,
                    end: k,
                });
                j = k;
                found = true;
                break;
            }
        }
        if !found {
            j = tick_start + tick_len; // no match, move past
        }
    }

    regions.sort_by_key(|r| r.start);
    regions
}

/// Check if a byte position falls inside any code region.
fn is_inside_code(pos: usize, regions: &[CodeRegion]) -> bool {
    regions.iter().any(|r| pos >= r.start && pos < r.end)
}

/// Clean up LLM response by stripping model-internal tags and reasoning patterns.
///
/// Some models (GLM-4.7, etc.) emit XML-tagged internal state like
/// Try to extract tool calls from content text where the model emitted them
/// as XML tags instead of using the structured tool_calls field.
///
/// Handles these formats:
/// - `<tool_call>tool_name</tool_call>` (bare name)
/// - `<tool_call>{"name":"x","arguments":{}}</tool_call>` (JSON)
/// - `<|tool_call|>...<|/tool_call|>` (pipe-delimited variant)
/// - `<function_call>...</function_call>` (function_call variant)
///
/// Only returns calls whose name matches an available tool.
pub fn recover_tool_calls_from_content(
    content: &str,
    available_tools: &[ToolDefinition],
) -> Vec<ToolCall> {
    let tool_names: std::collections::HashSet<&str> =
        available_tools.iter().map(|t| t.name.as_str()).collect();
    let mut calls = Vec::new();

    for (open, close) in &[
        ("<tool_call>", "</tool_call>"),
        ("<|tool_call|>", "<|/tool_call|>"),
        ("<function_call>", "</function_call>"),
        ("<|function_call|>", "<|/function_call|>"),
    ] {
        let mut remaining = content;
        while let Some(start) = remaining.find(open) {
            let inner_start = start + open.len();
            let after = &remaining[inner_start..];
            let Some(end) = after.find(close) else {
                break;
            };
            let inner = after[..end].trim();
            remaining = &after[end + close.len()..];

            if inner.is_empty() {
                continue;
            }

            // Try JSON first: {"name":"x","arguments":{}}
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(inner)
                && let Some(name) = parsed.get("name").and_then(|v| v.as_str())
                && tool_names.contains(name)
            {
                let arguments = parsed
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                calls.push(ToolCall {
                    id: format!("recovered_{}", calls.len()),
                    name: name.to_string(),
                    arguments,
                });
                continue;
            }

            // Bare tool name (e.g. "<tool_call>tool_list</tool_call>")
            let name = inner.trim();
            if tool_names.contains(name) {
                calls.push(ToolCall {
                    id: format!("recovered_{}", calls.len()),
                    name: name.to_string(),
                    arguments: serde_json::Value::Object(Default::default()),
                });
            }
        }
    }

    calls
}

/// `<tool_call>tool_list</tool_call>` or `<|tool_call|>` in the content field
/// instead of using the standard OpenAI tool_calls array. We strip all of
/// these before the response reaches channels/users.
///
/// Pipeline:
/// 1. Quick-check — bail if no reasoning/final tags
/// 2. Build code regions (fenced blocks + inline backticks)
/// 3. Strip thinking tags (regex, code-aware, strict mode for unclosed)
/// 4. If `<final>` tags present: extract only `<final>` content
///    Else: use the thinking-stripped text as-is
/// 5. Strip pipe-delimited reasoning tags (code-aware)
/// 6. Strip tool tags (string matching — no code-awareness needed)
/// 7. Collapse triple+ newlines, trim
pub fn clean_response(text: &str) -> String {
    // 1. Quick-check
    let mut result = if !QUICK_TAG_RE.is_match(text) {
        text.to_string()
    } else {
        // 2 + 3. Build code regions, strip thinking tags
        let code_regions = find_code_regions(text);
        let after_thinking = strip_thinking_tags_regex(text, &code_regions);

        // 4. If <final> tags present, extract only their content
        if FINAL_TAG_RE.is_match(&after_thinking) {
            let fresh_regions = find_code_regions(&after_thinking);
            extract_final_content(&after_thinking, &fresh_regions).unwrap_or(after_thinking)
        } else {
            after_thinking
        }
    };

    // 5. Strip pipe-delimited reasoning tags (code-aware)
    result = strip_pipe_reasoning_tags(&result);

    // 6. Strip tool tags (string matching, not code-aware)
    for tag in TOOL_TAGS {
        result = strip_xml_tag(&result, tag);
        result = strip_pipe_tag(&result, tag);
    }

    // 7. Collapse triple+ newlines, trim
    collapse_newlines(&result)
}

/// Tool-related tags stripped with simple string matching (no code-awareness needed).
const TOOL_TAGS: &[&str] = &["tool_call", "function_call", "tool_calls"];

/// Strip thinking/reasoning tags using regex, respecting code regions.
///
/// Strict mode: an unclosed opening tag discards all trailing text after it.
fn strip_thinking_tags_regex(text: &str, code_regions: &[CodeRegion]) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_index = 0;
    let mut in_thinking = false;

    for m in THINKING_TAG_RE.find_iter(text) {
        let idx = m.start();

        if is_inside_code(idx, code_regions) {
            continue;
        }

        // Check if this is a close tag by looking at capture group
        let caps = THINKING_TAG_RE.captures(&text[idx..]);
        let is_close = caps
            .and_then(|c| c.get(1))
            .is_some_and(|g| g.as_str() == "/");

        if !in_thinking {
            // Append text before this tag
            result.push_str(&text[last_index..idx]);
            if !is_close {
                in_thinking = true;
            }
        } else if is_close {
            in_thinking = false;
        }

        last_index = m.end();
    }

    // Strict mode: if still inside an unclosed thinking tag, discard trailing text
    if !in_thinking {
        result.push_str(&text[last_index..]);
    }

    result
}

/// Extract content inside `<final>` tags. Returns `None` if no non-code `<final>` tags found.
///
/// When `<final>` tags are present, ONLY content inside them reaches the user.
/// This discards any untagged reasoning that leaked outside `<think>` tags.
fn extract_final_content(text: &str, code_regions: &[CodeRegion]) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    let mut in_final = false;
    let mut last_index = 0;
    let mut found_any = false;

    for m in FINAL_TAG_RE.find_iter(text) {
        let idx = m.start();

        if is_inside_code(idx, code_regions) {
            continue;
        }

        let caps = FINAL_TAG_RE.captures(&text[idx..]);
        let is_close = caps
            .and_then(|c| c.get(1))
            .is_some_and(|g| g.as_str() == "/");

        if !in_final && !is_close {
            // Opening <final>
            in_final = true;
            found_any = true;
            last_index = m.end();
        } else if in_final && is_close {
            // Closing </final>
            parts.push(&text[last_index..idx]);
            in_final = false;
            last_index = m.end();
        }
    }

    if !found_any {
        return None;
    }

    // Unclosed <final> — include trailing content
    if in_final {
        parts.push(&text[last_index..]);
    }

    Some(parts.join(""))
}

/// Strip pipe-delimited reasoning tags, respecting code regions.
fn strip_pipe_reasoning_tags(text: &str) -> String {
    if !PIPE_REASONING_TAG_RE.is_match(text) {
        return text.to_string();
    }

    let code_regions = find_code_regions(text);
    let mut result = String::with_capacity(text.len());
    let mut last_index = 0;
    let mut in_tag = false;

    for m in PIPE_REASONING_TAG_RE.find_iter(text) {
        let idx = m.start();

        if is_inside_code(idx, &code_regions) {
            continue;
        }

        let caps = PIPE_REASONING_TAG_RE.captures(&text[idx..]);
        let is_close = caps
            .and_then(|c| c.get(1))
            .is_some_and(|g| g.as_str() == "/");

        if !in_tag {
            result.push_str(&text[last_index..idx]);
            if !is_close {
                in_tag = true;
            }
        } else if is_close {
            in_tag = false;
        }

        last_index = m.end();
    }

    if !in_tag {
        result.push_str(&text[last_index..]);
    }

    result
}

/// Strip `<tag>...</tag>` and `<tag ...>...</tag>` blocks from text.
/// Used for tool tags only (no code-awareness needed).
fn strip_xml_tag(text: &str, tag: &str) -> String {
    let open_exact = format!("<{}>", tag);
    let open_prefix = format!("<{} ", tag); // for <tag attr="...">
    let close = format!("</{}>", tag);

    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    loop {
        // Find the next opening tag (exact or with attributes)
        let exact_pos = remaining.find(&open_exact);
        let prefix_pos = remaining.find(&open_prefix);
        let start = match (exact_pos, prefix_pos) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => break,
        };

        // Add everything before the tag
        result.push_str(&remaining[..start]);

        // Find the end of the opening tag (the closing >)
        let after_open = &remaining[start..];
        let open_end = match after_open.find('>') {
            Some(pos) => start + pos + 1,
            None => break, // malformed, stop
        };

        // Find the closing tag
        if let Some(close_offset) = remaining[open_end..].find(&close) {
            let end = open_end + close_offset + close.len();
            remaining = &remaining[end..];
        } else {
            // No closing tag, discard from here (malformed)
            remaining = "";
            break;
        }
    }

    result.push_str(remaining);
    result
}

/// Strip `<|tag|>...<|/tag|>` pipe-delimited blocks from text.
/// Used for tool tags only (no code-awareness needed).
fn strip_pipe_tag(text: &str, tag: &str) -> String {
    let open = format!("<|{}|>", tag);
    let close = format!("<|/{}|>", tag);

    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(start) = remaining.find(&open) {
        result.push_str(&remaining[..start]);

        if let Some(close_offset) = remaining[start..].find(&close) {
            let end = start + close_offset + close.len();
            remaining = &remaining[end..];
        } else {
            remaining = "";
            break;
        }
    }

    result.push_str(remaining);
    result
}

/// Collapse triple+ newlines to double, then trim.
fn collapse_newlines(text: &str) -> String {
    let mut result = text.to_string();
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ChatMessage, ReasoningContext, ToolDefinition};

    // ---- Utility / structural tests ----

    #[test]
    fn test_extract_json() {
        let text = r#"Here's the plan:
{"goal": "test", "actions": []}
That's my plan."#;
        let json = extract_json(text).unwrap();
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
    }

    #[test]
    fn test_reasoning_context_builder() {
        let context = ReasoningContext::new()
            .with_message(ChatMessage::user("Hello"))
            .with_job("Test job");
        assert_eq!(context.messages.len(), 1);
        assert!(context.job_description.is_some());
    }

    // ---- Basic thinking tag stripping ----

    #[test]
    fn test_strip_thinking_tags_basic() {
        let input = "<thinking>Let me think about this...</thinking>Hello, user!";
        assert_eq!(clean_response(input), "Hello, user!");
    }

    #[test]
    fn test_strip_thinking_tags_multiple() {
        let input =
            "<thinking>First thought</thinking>Hello<thinking>Second thought</thinking> world!";
        assert_eq!(clean_response(input), "Hello world!");
    }

    #[test]
    fn test_strip_thinking_tags_multiline() {
        let input = "<thinking>\nI need to consider:\n1. What the user wants\n2. How to respond\n</thinking>\nHere is my response to your question.";
        assert_eq!(
            clean_response(input),
            "Here is my response to your question."
        );
    }

    #[test]
    fn test_strip_thinking_tags_no_tags() {
        let input = "Just a normal response without thinking tags.";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_strip_thinking_tags_unclosed() {
        // Strict mode: unclosed tag discards trailing text
        let input = "Hello <thinking>this never closes";
        assert_eq!(clean_response(input), "Hello");
    }

    // ---- Different tag names ----

    #[test]
    fn test_strip_think_tags() {
        let input = "<think>Let me reason about this...</think>The answer is 42.";
        assert_eq!(clean_response(input), "The answer is 42.");
    }

    #[test]
    fn test_strip_thought_tags() {
        let input = "<thought>The user wants X.</thought>Sure, here you go.";
        assert_eq!(clean_response(input), "Sure, here you go.");
    }

    #[test]
    fn test_strip_thoughts_tags() {
        let input = "<thoughts>Multiple thoughts...</thoughts>Result.";
        assert_eq!(clean_response(input), "Result.");
    }

    #[test]
    fn test_strip_reasoning_tags() {
        let input = "<reasoning>Analyzing the request...</reasoning>\n\nHere's what I found.";
        assert_eq!(clean_response(input), "Here's what I found.");
    }

    #[test]
    fn test_strip_reflection_tags() {
        let input = "<reflection>Am I answering correctly? Yes.</reflection>The capital is Paris.";
        assert_eq!(clean_response(input), "The capital is Paris.");
    }

    #[test]
    fn test_strip_scratchpad_tags() {
        let input =
            "<scratchpad>Step 1: check memory\nStep 2: respond</scratchpad>\n\nI found the answer.";
        assert_eq!(clean_response(input), "I found the answer.");
    }

    #[test]
    fn test_strip_inner_monologue_tags() {
        let input = "<inner_monologue>Processing query...</inner_monologue>Done!";
        assert_eq!(clean_response(input), "Done!");
    }

    #[test]
    fn test_strip_antthinking_tags() {
        let input = "<antthinking>Claude reasoning here</antthinking>Visible answer.";
        assert_eq!(clean_response(input), "Visible answer.");
    }

    // ---- Regex flexibility: whitespace, case, attributes ----

    #[test]
    fn test_whitespace_in_tags() {
        let input = "< think >reasoning</ think >Answer.";
        assert_eq!(clean_response(input), "Answer.");
    }

    #[test]
    fn test_case_insensitive_tags() {
        let input = "<THINKING>Upper case reasoning</THINKING>Visible.";
        assert_eq!(clean_response(input), "Visible.");
    }

    #[test]
    fn test_mixed_case_tags() {
        let input = "<Think>Mixed case</Think>Output.";
        assert_eq!(clean_response(input), "Output.");
    }

    #[test]
    fn test_tags_with_attributes() {
        let input = "<thinking type=\"deep\" level=\"3\">reasoning</thinking>Answer.";
        assert_eq!(clean_response(input), "Answer.");
    }

    // ---- Tool call tags ----

    #[test]
    fn test_strip_tool_call_tags() {
        let input = "<tool_call>tool_list</tool_call>";
        assert_eq!(clean_response(input), "");
    }

    #[test]
    fn test_strip_tool_call_with_surrounding_text() {
        let input = "Here is my answer.\n\n<tool_call>\n{\"name\": \"search\", \"arguments\": {}}\n</tool_call>";
        assert_eq!(clean_response(input), "Here is my answer.");
    }

    #[test]
    fn test_strip_function_call_tags() {
        let input = "Response text<function_call>{\"name\": \"foo\"}</function_call>";
        assert_eq!(clean_response(input), "Response text");
    }

    #[test]
    fn test_strip_tool_calls_plural() {
        let input = "<tool_calls>[{\"id\": \"1\"}]</tool_calls>Actual response.";
        assert_eq!(clean_response(input), "Actual response.");
    }

    #[test]
    fn test_strip_xml_tag_with_attributes() {
        let input = "<tool_call type=\"function\">search()</tool_call>Done.";
        assert_eq!(clean_response(input), "Done.");
    }

    // ---- Pipe-delimited tags ----

    #[test]
    fn test_strip_pipe_delimited_tags() {
        let input = "<|tool_call|>{\"name\": \"search\"}<|/tool_call|>Hello!";
        assert_eq!(clean_response(input), "Hello!");
    }

    #[test]
    fn test_strip_pipe_delimited_thinking() {
        let input = "<|thinking|>reasoning here<|/thinking|>The answer is 42.";
        assert_eq!(clean_response(input), "The answer is 42.");
    }

    #[test]
    fn test_strip_pipe_delimited_think() {
        let input = "<|think|>reasoning here<|/think|>The answer is 42.";
        assert_eq!(clean_response(input), "The answer is 42.");
    }

    // ---- Mixed tags ----

    #[test]
    fn test_strip_multiple_internal_tags() {
        let input = "<thinking>Let me think</thinking>Hello!\n<tool_call>some_tool</tool_call>";
        assert_eq!(clean_response(input), "Hello!");
    }

    #[test]
    fn test_strip_multiple_reasoning_tag_types() {
        let input = "<think>Initial analysis</think>Intermediate.\n<reflection>Double-check</reflection>Final answer.";
        assert_eq!(clean_response(input), "Intermediate.\nFinal answer.");
    }

    #[test]
    fn test_clean_response_preserves_normal_content() {
        let input = "The function tool_call_handler works great. No tags here!";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_clean_response_thinking_tags_with_trailing_text() {
        let input = "<thinking>Internal thought</thinking>Some text.\n\nHere's the answer.";
        assert_eq!(clean_response(input), "Some text.\n\nHere's the answer.");
    }

    #[test]
    fn test_clean_response_thinking_tags_reasoning_properly_tagged() {
        let input = "<thinking>The user is asking about my name.</thinking>\n\nI'm ThinClaw, a secure personal agent.";
        assert_eq!(
            clean_response(input),
            "I'm ThinClaw, a secure personal agent."
        );
    }

    // ---- Code-awareness: tags inside code blocks are preserved ----

    #[test]
    fn test_tags_in_fenced_code_block_preserved() {
        let input =
            "Here is an example:\n\n```\n<thinking>This is inside code</thinking>\n```\n\nDone.";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_tags_in_tilde_fenced_block_preserved() {
        let input = "Example:\n\n~~~\n<think>code example</think>\n~~~\n\nEnd.";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_tags_in_inline_backticks_preserved() {
        let input = "Use the `<thinking>` tag for reasoning.";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_mixed_real_and_code_tags() {
        let input = "<thinking>real reasoning</thinking>Use `<thinking>` tags.\n\n```\n<thinking>code example</thinking>\n```";
        let expected = "Use `<thinking>` tags.\n\n```\n<thinking>code example</thinking>\n```";
        assert_eq!(clean_response(input), expected);
    }

    #[test]
    fn test_code_block_with_info_string() {
        let input = "```xml\n<thinking>xml example</thinking>\n```\nVisible.";
        assert_eq!(clean_response(input), input);
    }

    // ---- <final> tag extraction ----

    #[test]
    fn test_final_tag_basic() {
        let input = "<think>reasoning</think><final>answer</final>";
        assert_eq!(clean_response(input), "answer");
    }

    #[test]
    fn test_final_tag_strips_untagged_reasoning() {
        let input = "Untagged reasoning.\n<final>answer</final>";
        assert_eq!(clean_response(input), "answer");
    }

    #[test]
    fn test_final_tag_multiple_blocks() {
        let input =
            "<think>part 1</think><final>Hello </final><think>part 2</think><final>world!</final>";
        assert_eq!(clean_response(input), "Hello world!");
    }

    #[test]
    fn test_no_final_tag_fallthrough() {
        // Without <final>, thinking-stripped text returned as-is
        let input = "<think>reasoning</think>Just the answer.";
        assert_eq!(clean_response(input), "Just the answer.");
    }

    #[test]
    fn test_no_tags_at_all() {
        let input = "Just a normal response";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_final_tag_in_code_preserved() {
        // <final> inside code block should not trigger extraction
        let input = "Use `<final>` to mark output.\n\nHello.";
        assert_eq!(clean_response(input), input);
    }

    #[test]
    fn test_final_tag_unclosed_includes_trailing() {
        let input = "<think>reasoning</think><final>answer continues";
        assert_eq!(clean_response(input), "answer continues");
    }

    // ---- Unicode content ----

    #[test]
    fn test_unicode_content_preserved() {
        let input = "<thinking>日本語の推論</thinking>こんにちは世界！";
        assert_eq!(clean_response(input), "こんにちは世界！");
    }

    #[test]
    fn test_unicode_in_final() {
        let input = "<think>推論</think><final>答え：42</final>";
        assert_eq!(clean_response(input), "答え：42");
    }

    // ---- Newline collapsing ----

    #[test]
    fn test_collapse_triple_newlines() {
        let input = "<thinking>removed</thinking>\n\n\nVisible.";
        assert_eq!(clean_response(input), "Visible.");
    }

    #[test]
    fn test_trims_whitespace() {
        let input = "  <thinking>removed</thinking>  Hello, user!  \n";
        assert_eq!(clean_response(input), "Hello, user!");
    }

    // ---- Code region detection ----

    #[test]
    fn test_find_code_regions_fenced() {
        let text = "before\n```\ncode\n```\nafter";
        let regions = find_code_regions(text);
        assert_eq!(regions.len(), 1);
        assert!(text[regions[0].start..regions[0].end].contains("code"));
    }

    #[test]
    fn test_find_code_regions_inline() {
        let text = "Use `<thinking>` tag.";
        let regions = find_code_regions(text);
        assert_eq!(regions.len(), 1);
        assert!(text[regions[0].start..regions[0].end].contains("<thinking>"));
    }

    #[test]
    fn test_find_code_regions_unclosed_fence() {
        let text = "before\n```\ncode goes on\nno closing fence";
        let regions = find_code_regions(text);
        assert_eq!(regions.len(), 1);
        // Unclosed fence extends to EOF
        assert_eq!(regions[0].end, text.len());
    }

    // ---- recover_tool_calls_from_content tests ----

    fn make_tools(names: &[&str]) -> Vec<ToolDefinition> {
        names
            .iter()
            .map(|n| ToolDefinition {
                name: n.to_string(),
                description: String::new(),
                parameters: serde_json::json!({}),
            })
            .collect()
    }

    #[test]
    fn test_recover_bare_tool_name() {
        let tools = make_tools(&["tool_list", "tool_auth"]);
        let content = "<tool_call>tool_list</tool_call>";
        let calls = recover_tool_calls_from_content(content, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tool_list");
        assert_eq!(calls[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn test_recover_json_tool_call() {
        let tools = make_tools(&["memory_search"]);
        let content =
            r#"<tool_call>{"name": "memory_search", "arguments": {"query": "test"}}</tool_call>"#;
        let calls = recover_tool_calls_from_content(content, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "memory_search");
        assert_eq!(calls[0].arguments, serde_json::json!({"query": "test"}));
    }

    #[test]
    fn test_recover_pipe_delimited() {
        let tools = make_tools(&["tool_list"]);
        let content = "<|tool_call|>tool_list<|/tool_call|>";
        let calls = recover_tool_calls_from_content(content, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tool_list");
    }

    #[test]
    fn test_recover_unknown_tool_ignored() {
        let tools = make_tools(&["tool_list"]);
        let content = "<tool_call>nonexistent_tool</tool_call>";
        let calls = recover_tool_calls_from_content(content, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_no_tags() {
        let tools = make_tools(&["tool_list"]);
        let content = "Just a normal response.";
        let calls = recover_tool_calls_from_content(content, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_multiple_tool_calls() {
        let tools = make_tools(&["tool_list", "tool_auth"]);
        let content = "<tool_call>tool_list</tool_call>\n<tool_call>tool_auth</tool_call>";
        let calls = recover_tool_calls_from_content(content, &tools);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "tool_list");
        assert_eq!(calls[1].name, "tool_auth");
    }

    #[test]
    fn test_recover_function_call_variant() {
        let tools = make_tools(&["shell"]);
        let content =
            r#"<function_call>{"name": "shell", "arguments": {"cmd": "ls"}}</function_call>"#;
        let calls = recover_tool_calls_from_content(content, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
    }

    #[test]
    fn test_recover_with_surrounding_text() {
        let tools = make_tools(&["tool_list"]);
        let content = "Let me check.\n\n<tool_call>tool_list</tool_call>\n\nDone.";
        let calls = recover_tool_calls_from_content(content, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tool_list");
    }
}
