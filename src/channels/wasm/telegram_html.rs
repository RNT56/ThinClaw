//! Markdown → Telegram HTML conversion for host-side streaming.
//!
//! This is a port of the WASM-side `markdown_to_telegram_html()` function
//! from `channels-src/telegram/src/lib.rs`. By having this on the host side,
//! the `send_draft()` streaming path can format messages identically to the
//! WASM `on_respond()` path, instead of sending raw markdown.
//!
//! The conversion handles standard LLM-emitted Markdown and converts it
//! to the subset of HTML that Telegram supports:
//! `<b>`, `<i>`, `<u>`, `<s>`, `<code>`, `<pre>`, `<a href>`, `<blockquote>`.

/// Sentinel used to protect unmatched `**` from the `*` handler.
const SENTINEL_STAR: &str = "\u{FFFE}\u{FFFE}";
/// Sentinel for unmatched `__`.
const SENTINEL_UNDER: &str = "\u{FFFF}\u{FFFF}";

/// Convert standard Markdown (as emitted by LLMs) to Telegram-safe HTML.
///
/// Handles:
/// - `**bold**` / `__bold__`  → `<b>bold</b>`
/// - `*italic*` / `_italic_` → `<i>italic</i>`
/// - `` `inline code` ``     → `<code>inline code</code>`
/// - ` ```lang\ncode``` `    → `<pre><code class="language-lang">code</code></pre>`
/// - `# Heading`             → `<b>Heading</b>`
/// - `[text](url)`           → `<a href="url">text</a>`
/// - `~~strikethrough~~`     → `<s>strikethrough</s>`
/// - `> blockquote`          → `<blockquote>text</blockquote>`
/// - HTML special chars      → escaped (`<`, `>`, `&`)
pub fn markdown_to_telegram_html(md: &str) -> String {
    let mut out = String::with_capacity(md.len() + md.len() / 4);
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // ── Fenced code blocks ──────────────────────────────────────
        if line.trim_start().starts_with("```") {
            let lang = line.trim_start().trim_start_matches('`').trim();
            let mut code_lines = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code_lines.push(lines[i]);
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }
            let code_text = escape_html(&code_lines.join("\n"));
            if lang.is_empty() {
                out.push_str(&format!("<pre><code>{code_text}</code></pre>\n"));
            } else {
                out.push_str(&format!(
                    "<pre><code class=\"language-{lang}\">{code_text}</code></pre>\n"
                ));
            }
            continue;
        }

        // ── Blockquotes ─────────────────────────────────────────────
        if line.starts_with("> ") || line == ">" {
            let mut bq_lines = Vec::new();
            while i < lines.len() && (lines[i].starts_with("> ") || lines[i] == ">") {
                let content = lines[i]
                    .strip_prefix("> ")
                    .unwrap_or(lines[i].strip_prefix(">").unwrap_or(lines[i]));
                bq_lines.push(content);
                i += 1;
            }
            let inner = bq_lines
                .iter()
                .map(|l| format_inline(&escape_html(l)))
                .collect::<Vec<_>>()
                .join("\n");
            out.push_str(&format!("<blockquote>{inner}</blockquote>\n"));
            continue;
        }

        // ── Headings (# ... ######) → bold line ─────────────────────
        if let Some(heading) = strip_heading(line) {
            let escaped = escape_html(heading);
            let formatted = format_inline(&escaped);
            out.push_str(&format!("<b>{formatted}</b>\n"));
            i += 1;
            continue;
        }

        // ── Regular line → inline formatting ────────────────────────
        let escaped = escape_html(line);
        let formatted = format_inline(&escaped);
        out.push_str(&formatted);
        out.push('\n');
        i += 1;
    }

    if out.ends_with('\n') {
        out.truncate(out.len() - 1);
    }
    out
}

/// Escape HTML special characters for Telegram.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Apply inline Markdown formatting to an already-HTML-escaped string.
fn format_inline(s: &str) -> String {
    let mut result = String::from(s);

    result = replace_inline_pairs(&result, "`", "<code>", "</code>", None);
    result = replace_inline_pairs(&result, "**", "<b>", "</b>", Some(SENTINEL_STAR));
    result = replace_inline_pairs(&result, "__", "<b>", "</b>", Some(SENTINEL_UNDER));
    result = replace_inline_pairs(&result, "~~", "<s>", "</s>", None);
    result = replace_inline_pairs(&result, "*", "<i>", "</i>", None);
    result = replace_inline_pairs(&result, "_", "<i>", "</i>", None);
    result = convert_links(&result);

    result = result.replace(SENTINEL_STAR, "**");
    result = result.replace(SENTINEL_UNDER, "__");

    result
}

/// Replace matched pairs of a delimiter with open/close HTML tags.
fn replace_inline_pairs(
    s: &str,
    delim: &str,
    open_tag: &str,
    close_tag: &str,
    unmatched_sentinel: Option<&str>,
) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut open = true;

    while let Some(pos) = rest.find(delim) {
        let before = &rest[..pos];
        out.push_str(before);
        rest = &rest[pos + delim.len()..];

        if open {
            if rest.contains(delim) {
                out.push_str(open_tag);
                open = false;
            } else {
                out.push_str(unmatched_sentinel.unwrap_or(delim));
            }
        } else {
            out.push_str(close_tag);
            open = true;
        }
    }
    out.push_str(rest);
    out
}

/// Convert Markdown links `[text](url)` to `<a href="url">text</a>`.
fn convert_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;

    while let Some(bracket_open) = rest.find('[') {
        out.push_str(&rest[..bracket_open]);
        let after_open = &rest[bracket_open + 1..];

        if let Some(bracket_close) = after_open.find(']') {
            let link_text = &after_open[..bracket_close];
            let after_close = &after_open[bracket_close + 1..];

            if after_close.starts_with('(')
                && let Some(paren_close) = after_close.find(')')
            {
                let url = &after_close[1..paren_close];
                out.push_str(&format!("<a href=\"{url}\">{link_text}</a>"));
                rest = &after_close[paren_close + 1..];
                continue;
            }

            out.push('[');
            rest = after_open;
        } else {
            out.push('[');
            rest = after_open;
        }
    }
    out.push_str(rest);
    out
}

/// Strip a Markdown heading prefix (`# ` through `###### `) and return the text.
fn strip_heading(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
        if hashes <= 6 {
            let rest = &trimmed[hashes..];
            if let Some(stripped) = rest.strip_prefix(' ') {
                return Some(stripped.trim());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bold() {
        assert_eq!(
            markdown_to_telegram_html("Hello **world**!"),
            "Hello <b>world</b>!"
        );
    }

    #[test]
    fn test_italic() {
        assert_eq!(
            markdown_to_telegram_html("Hello *world*!"),
            "Hello <i>world</i>!"
        );
    }

    #[test]
    fn test_inline_code() {
        assert_eq!(
            markdown_to_telegram_html("Use `foo()` here"),
            "Use <code>foo()</code> here"
        );
    }

    #[test]
    fn test_code_block() {
        let input = "```rust\nfn main() {}\n```";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("<pre><code class=\"language-rust\">"));
        assert!(output.contains("fn main() {}"));
    }

    #[test]
    fn test_heading() {
        assert_eq!(markdown_to_telegram_html("## Status"), "<b>Status</b>");
    }

    #[test]
    fn test_link() {
        assert_eq!(
            markdown_to_telegram_html("[Google](https://google.com)"),
            "<a href=\"https://google.com\">Google</a>"
        );
    }

    #[test]
    fn test_blockquote() {
        assert_eq!(
            markdown_to_telegram_html("> Hello world"),
            "<blockquote>Hello world</blockquote>"
        );
    }

    #[test]
    fn test_html_escaping() {
        assert_eq!(
            markdown_to_telegram_html("x < y & z > w"),
            "x &lt; y &amp; z &gt; w"
        );
    }

    #[test]
    fn test_strikethrough() {
        assert_eq!(markdown_to_telegram_html("~~deleted~~"), "<s>deleted</s>");
    }

    #[test]
    fn test_unmatched_bold_preserved() {
        // Single ** without closing should be preserved
        assert_eq!(
            markdown_to_telegram_html("value is **true"),
            "value is **true"
        );
    }

    #[test]
    fn test_mixed_formatting() {
        assert_eq!(
            markdown_to_telegram_html("**bold** and *italic* and `code`"),
            "<b>bold</b> and <i>italic</i> and <code>code</code>"
        );
    }

    #[test]
    fn test_empty_input() {
        assert_eq!(markdown_to_telegram_html(""), "");
    }

    #[test]
    fn test_plain_text_passthrough() {
        assert_eq!(
            markdown_to_telegram_html("Just plain text"),
            "Just plain text"
        );
    }
}
