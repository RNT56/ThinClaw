//! Telegram message rendering for host-side streaming.
//!
//! This is a port of the WASM-side `markdown_to_telegram_html()` function
//! from `channels-src/telegram/src/lib.rs`. By having this on the host side,
//! the `send_draft()` streaming path can format messages identically to the
//! WASM `on_respond()` path, instead of sending raw markdown.
//!
//! Rendering supports two paths:
//! - raw Telegram HTML, sanitized to an allowlist of supported tags
//! - standard LLM-emitted Markdown, converted to Telegram HTML as a fallback

/// Sentinel used to protect unmatched `**` from the `*` handler.
const SENTINEL_STAR: &str = "\u{FFFE}\u{FFFE}";
/// Sentinel for unmatched `__`.
const SENTINEL_UNDER: &str = "\u{FFFF}\u{FFFF}";

/// Render agent output into Telegram-safe HTML.
///
/// If the message already contains supported Telegram HTML tags, those tags
/// are sanitized and preserved directly. Otherwise, standard Markdown is
/// converted into Telegram HTML.
pub fn markdown_to_telegram_html(input: &str) -> String {
    if contains_supported_html(input) {
        sanitize_telegram_html(input)
    } else {
        render_markdown_to_telegram_html(input)
    }
}

/// Convert standard Markdown (as emitted by LLMs) to Telegram-safe HTML.
fn render_markdown_to_telegram_html(md: &str) -> String {
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

#[derive(Debug, Clone)]
struct ParsedTelegramTag {
    name: String,
    rendered: String,
    is_closing: bool,
}

/// Return true if the input already contains at least one supported Telegram
/// HTML tag and should therefore be handled via the HTML sanitizer path.
fn contains_supported_html(input: &str) -> bool {
    let mut i = 0;
    while let Some(rel) = input[i..].find('<') {
        let start = i + rel;
        if parse_supported_tag(input, start).is_some() {
            return true;
        }
        i = start + 1;
    }
    false
}

/// Sanitize raw Telegram HTML, preserving only an allowlist of tags and attrs.
fn sanitize_telegram_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + input.len() / 8);
    let mut i = 0;
    let mut open_tags: Vec<String> = Vec::new();

    while i < input.len() {
        let Some(rel) = input[i..].find('<') else {
            out.push_str(&escape_html(&input[i..]));
            break;
        };
        let start = i + rel;
        out.push_str(&escape_html(&input[i..start]));

        if let Some((end, tag)) = parse_supported_tag(input, start) {
            if tag.is_closing {
                if open_tags.last().is_some_and(|name| name == &tag.name) {
                    out.push_str(&tag.rendered);
                    open_tags.pop();
                } else {
                    out.push_str(&escape_html(&input[start..end]));
                }
            } else {
                out.push_str(&tag.rendered);
                open_tags.push(tag.name);
            }
            i = end;
            continue;
        }

        if let Some(end) = find_tag_end(input, start) {
            out.push_str(&escape_html(&input[start..end]));
            i = end;
        } else {
            out.push_str("&lt;");
            i = start + 1;
        }
    }

    while let Some(tag) = open_tags.pop() {
        out.push_str("</");
        out.push_str(&tag);
        out.push('>');
    }

    out
}

fn parse_supported_tag(input: &str, start: usize) -> Option<(usize, ParsedTelegramTag)> {
    let end = find_tag_end(input, start)?;
    let raw = input.get(start + 1..end - 1)?.trim();
    if raw.is_empty() || raw.starts_with('!') || raw.starts_with('?') {
        return None;
    }

    let (is_closing, raw) = if let Some(rest) = raw.strip_prefix('/') {
        (true, rest.trim_start())
    } else {
        (false, raw)
    };

    if raw.is_empty() {
        return None;
    }

    if raw.ends_with('/') {
        return None;
    }
    let name_end = raw
        .find(|c: char| c.is_ascii_whitespace())
        .unwrap_or(raw.len());
    let name_raw = &raw[..name_end];
    let attrs_raw = raw[name_end..].trim();
    let name = canonical_tag_name(name_raw)?;

    if is_closing {
        if !attrs_raw.is_empty() {
            return None;
        }
        return Some((
            end,
            ParsedTelegramTag {
                name: name.to_string(),
                rendered: format!("</{name}>"),
                is_closing: true,
            },
        ));
    }

    let rendered = render_open_tag(name, attrs_raw)?;
    Some((
        end,
        ParsedTelegramTag {
            name: name.to_string(),
            rendered,
            is_closing: false,
        },
    ))
}

fn find_tag_end(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut i = start + 1;
    let mut quote: Option<u8> = None;

    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'>' {
            return Some(i + 1);
        }
        i += 1;
    }

    None
}

fn canonical_tag_name(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "b" | "strong" => Some("b"),
        "i" | "em" => Some("i"),
        "u" | "ins" => Some("u"),
        "s" | "strike" | "del" => Some("s"),
        "code" => Some("code"),
        "pre" => Some("pre"),
        "a" => Some("a"),
        "blockquote" => Some("blockquote"),
        _ => None,
    }
}

fn render_open_tag(name: &str, attrs_raw: &str) -> Option<String> {
    match name {
        "b" | "i" | "u" | "s" | "pre" => {
            if attrs_raw.is_empty() {
                Some(format!("<{name}>"))
            } else {
                None
            }
        }
        "code" => {
            let attrs = parse_attributes(attrs_raw)?;
            if attrs.is_empty() {
                return Some("<code>".to_string());
            }
            if attrs.len() != 1 {
                return None;
            }
            let (attr_name, attr_value) = &attrs[0];
            if attr_name != "class" {
                return None;
            }
            let class = attr_value.as_deref()?;
            if !is_safe_code_class(class) {
                return None;
            }
            Some(format!("<code class=\"{}\">", escape_html_attribute(class)))
        }
        "a" => {
            let attrs = parse_attributes(attrs_raw)?;
            if attrs.len() != 1 {
                return None;
            }
            let (attr_name, attr_value) = &attrs[0];
            if attr_name != "href" {
                return None;
            }
            let href = sanitize_href(attr_value.as_deref()?)?;
            Some(format!("<a href=\"{href}\">"))
        }
        "blockquote" => {
            let attrs = parse_attributes(attrs_raw)?;
            if attrs.is_empty() {
                return Some("<blockquote>".to_string());
            }
            if attrs.len() != 1 {
                return None;
            }
            let (attr_name, attr_value) = &attrs[0];
            if attr_name != "expandable" {
                return None;
            }
            if attr_value.is_none()
                || attr_value
                    .as_deref()
                    .is_some_and(|value| value.is_empty() || value.eq_ignore_ascii_case("true"))
            {
                return Some("<blockquote expandable>".to_string());
            }
            None
        }
        _ => None,
    }
}

fn parse_attributes(attrs: &str) -> Option<Vec<(String, Option<String>)>> {
    let mut out = Vec::new();
    let bytes = attrs.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        let name_start = i;
        while i < bytes.len() && is_attr_name_char(bytes[i]) {
            i += 1;
        }
        if name_start == i {
            return None;
        }
        let name = attrs[name_start..i].to_ascii_lowercase();

        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }

        let value = if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                return None;
            }

            if bytes[i] == b'"' || bytes[i] == b'\'' {
                let quote = bytes[i];
                i += 1;
                let value_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                if i >= bytes.len() {
                    return None;
                }
                let value = attrs[value_start..i].to_string();
                i += 1;
                Some(value)
            } else {
                let value_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                Some(attrs[value_start..i].to_string())
            }
        } else {
            None
        };

        out.push((name, value));
    }

    Some(out)
}

fn is_attr_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')
}

fn is_safe_code_class(class: &str) -> bool {
    let Some(lang) = class.strip_prefix("language-") else {
        return false;
    };
    !lang.is_empty()
        && lang
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'+'))
}

fn sanitize_href(href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.chars().any(char::is_control) {
        return None;
    }

    let lower = href.to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("vbscript:")
        || lower.starts_with("data:")
    {
        return None;
    }

    Some(escape_html_attribute(href))
}

/// Escape HTML special characters for Telegram.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_html_attribute(s: &str) -> String {
    escape_html(s).replace('"', "&quot;").replace('\'', "&#39;")
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

    #[test]
    fn test_raw_html_passthrough() {
        assert_eq!(
            markdown_to_telegram_html("Hello <b>world</b>!"),
            "Hello <b>world</b>!"
        );
    }

    #[test]
    fn test_raw_html_alias_tags_are_canonicalized() {
        assert_eq!(
            markdown_to_telegram_html("<strong>Hello</strong>"),
            "<b>Hello</b>"
        );
    }

    #[test]
    fn test_raw_html_invalid_tags_are_escaped() {
        assert_eq!(
            markdown_to_telegram_html("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
        );
    }

    #[test]
    fn test_raw_html_link_attrs_are_sanitized() {
        assert_eq!(
            markdown_to_telegram_html("<a href=\"https://example.com?a=1&b=2\">Example</a>"),
            "<a href=\"https://example.com?a=1&amp;b=2\">Example</a>"
        );
    }

    #[test]
    fn test_raw_html_rejects_unsafe_links() {
        assert_eq!(
            markdown_to_telegram_html("<a href=\"javascript:alert(1)\">bad</a>"),
            "&lt;a href=\"javascript:alert(1)\"&gt;bad&lt;/a&gt;"
        );
    }

    #[test]
    fn test_raw_html_auto_closes_open_tags() {
        assert_eq!(markdown_to_telegram_html("<b>Hello"), "<b>Hello</b>");
    }
}
