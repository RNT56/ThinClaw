//! Zero-config web search tool.
//!
//! Searches the public web with **no API key and no operator setup**, so a
//! fresh ThinClaw install can answer "what's the latest on X" out of the box —
//! closing the gap versus assistants that ship a first-class web search.
//!
//! Backend: DuckDuckGo's keyless HTML endpoint (`https://html.duckduckgo.com/
//! html/`). Results (title, URL, snippet) are parsed from the returned markup.
//! Requests go through the same SSRF-guarded, size-capped fetch path as the
//! `http` tool. Operators who want a richer ranked API can still install the
//! Brave Search WASM extension; this tool is the always-available default.

use std::time::Duration;

use async_trait::async_trait;

use thinclaw_tools_core::{
    OutboundUrlGuardOptions, Tool, ToolError, ToolMetadata, ToolOutput, require_str,
    validate_outbound_url_pinned_async,
};
use thinclaw_types::JobContext;

/// DuckDuckGo keyless HTML search endpoint.
const DDG_HTML_ENDPOINT: &str = "https://html.duckduckgo.com/html/";
/// Cap on the search results HTML we will read (defensive against huge pages).
const MAX_RESPONSE_SIZE: usize = 2 * 1024 * 1024;
/// Default number of results to return when the caller does not specify.
const DEFAULT_RESULTS: usize = 5;
/// Hard ceiling on results, regardless of the requested count.
const MAX_RESULTS: usize = 10;
const MAX_QUERY_BYTES: usize = 8 * 1024;
const MAX_RESULT_URL_BYTES: usize = 16 * 1024;
const MAX_RESULT_TITLE_CHARS: usize = 512;
const MAX_RESULT_SNIPPET_CHARS: usize = 4 * 1024;

/// A single parsed search result.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Zero-config web search over DuckDuckGo's keyless endpoint.
pub struct WebSearchTool;

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self
    }

    async fn fetch(&self, query: &str) -> Result<String, ToolError> {
        let guarded = validate_outbound_url_pinned_async(
            DDG_HTML_ENDPOINT,
            &OutboundUrlGuardOptions {
                require_https: true,
                upgrade_http_to_https: true,
                allowlist: vec!["html.duckduckgo.com".to_string()],
            },
        )
        .await?;
        let host = guarded
            .url
            .host_str()
            .ok_or_else(|| ToolError::ExternalService("search URL has no host".to_string()))?;
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            // A browser-like UA; the HTML endpoint returns an empty page to some
            // automated agents otherwise.
            .user_agent("Mozilla/5.0 (compatible; ThinClaw/1.0; +https://thinclaw.dev) web_search");
        if !guarded.pinned_addrs.is_empty() {
            builder = builder.resolve_to_addrs(host, &guarded.pinned_addrs);
        }
        let client = builder.build().map_err(|error| {
            ToolError::ExternalService(format!("failed to build search client: {error}"))
        })?;

        let resp = client
            .post(guarded.url)
            .form(&[("q", query), ("kl", "wt-wt")])
            .send()
            .await
            .map_err(|e| {
                ToolError::ExternalService(format!(
                    "web search request failed: {}",
                    e.without_url()
                ))
            })?;

        if !resp.status().is_success() {
            return Err(ToolError::ExternalService(format!(
                "web search returned HTTP {}",
                resp.status()
            )));
        }

        // Reject an oversized body up front by its declared length...
        if let Some(len) = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
            && len > MAX_RESPONSE_SIZE
        {
            return Err(ToolError::ExternalService(format!(
                "web search response too large ({len} bytes > {MAX_RESPONSE_SIZE})"
            )));
        }

        // ...and stream with a hard cap so a server that omits/understates
        // Content-Length can't exhaust memory (mirrors the `http` tool).
        let mut body: Vec<u8> = Vec::with_capacity(64 * 1024);
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = futures::StreamExt::next(&mut stream).await {
            let chunk = chunk.map_err(|e| {
                ToolError::ExternalService(format!(
                    "failed to read search body: {}",
                    e.without_url()
                ))
            })?;
            if body.len() + chunk.len() > MAX_RESPONSE_SIZE {
                return Err(ToolError::ExternalService(format!(
                    "web search response exceeds {MAX_RESPONSE_SIZE} bytes"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }
}

/// Parse DuckDuckGo HTML results into structured `(title, url, snippet)` items.
///
/// The keyless HTML endpoint renders each result as an anchor with
/// `class="result__a"` (title + href) followed by a `class="result__snippet"`
/// block. We scan for those markers rather than pulling in an HTML-parser
/// dependency; the markup is stable and simple. DuckDuckGo wraps outbound links
/// in a redirect (`//duckduckgo.com/l/?uddg=<encoded target>`), which we unwrap.
pub fn parse_ddg_results(html: &str, limit: usize) -> Vec<WebSearchResult> {
    let mut results = Vec::new();

    for anchor_start in find_all(html, "result__a") {
        if results.len() >= limit {
            break;
        }
        // Find the href="..." to the left/right of the class marker within the
        // enclosing <a ...> tag.
        let tag_open = html[..anchor_start].rfind('<').unwrap_or(anchor_start);
        let tag_end = match html[anchor_start..].find('>') {
            Some(rel) => anchor_start + rel,
            None => continue,
        };
        let tag = &html[tag_open..tag_end];
        let Some(href) = extract_attr(tag, "href") else {
            continue;
        };
        let url = normalize_ddg_href(&href);
        if url.is_empty() {
            continue;
        }

        // The visible title is the text between '>' and the closing </a>.
        let after_tag = &html[tag_end + 1..];
        let title = match after_tag.find("</a>") {
            Some(end) => {
                bounded_result_text(&strip_html(&after_tag[..end]), MAX_RESULT_TITLE_CHARS)
            }
            None => continue,
        };
        if title.is_empty() {
            continue;
        }

        // Snippet: the next result__snippet block after this anchor.
        let snippet = find_after(after_tag, "result__snippet")
            .and_then(|idx| after_tag[idx..].find('>').map(|rel| idx + rel + 1))
            .and_then(|start| {
                after_tag[start..]
                    .find("</a>")
                    .or_else(|| after_tag[start..].find("</div>"))
                    .map(|end| {
                        bounded_result_text(
                            &strip_html(&after_tag[start..start + end]),
                            MAX_RESULT_SNIPPET_CHARS,
                        )
                    })
            })
            .unwrap_or_default();

        results.push(WebSearchResult {
            title,
            url,
            snippet,
        });
    }

    results
}

/// Unwrap DuckDuckGo's `/l/?uddg=<url-encoded target>` redirect wrapper.
fn normalize_ddg_href(href: &str) -> String {
    let candidate = if let Some(rest) = href.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        href.to_string()
    };
    if let Some(idx) = candidate.find("uddg=") {
        let encoded: String = candidate[idx + 5..]
            .chars()
            .take_while(|&c| c != '&')
            .collect();
        if let Some(decoded) = percent_decode(&encoded) {
            return validated_result_url(&decoded).unwrap_or_default();
        }
    }
    validated_result_url(&candidate).unwrap_or_default()
}

fn validated_result_url(candidate: &str) -> Option<String> {
    if candidate.is_empty()
        || candidate.len() > MAX_RESULT_URL_BYTES
        || candidate.chars().any(char::is_control)
    {
        return None;
    }
    let url = reqwest::Url::parse(candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(url.to_string())
}

fn bounded_result_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .take(max_chars)
        .collect()
}

/// Minimal application/x-www-form-urlencoded percent-decoder (no extra deps).
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16)?;
                let lo = (bytes[i + 2] as char).to_digit(16)?;
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Extract the value of an HTML attribute (double- or single-quoted).
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=");
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let inner = &rest[1..];
    let end = inner.find(quote)?;
    Some(inner[..end].to_string())
}

/// Strip HTML tags and collapse whitespace, decoding a few common entities.
fn strip_html(fragment: &str) -> String {
    let mut text = String::with_capacity(fragment.len());
    let mut in_tag = false;
    for ch in fragment.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn find_all(haystack: &str, needle: &str) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(needle) {
        let abs = from + rel;
        hits.push(abs);
        from = abs + needle.len();
    }
    hits
}

fn find_after(haystack: &str, needle: &str) -> Option<usize> {
    haystack.find(needle)
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the public web and return ranked results (title, URL, snippet). \
         Use this for current events, facts you are unsure about, documentation \
         lookups, or anything that may have changed since your training cutoff. \
         No API key or setup required. Follow up with the `http` tool to fetch a \
         result's full page when you need more than the snippet."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query."
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results to return (1-10, default 5).",
                    "minimum": 1,
                    "maximum": MAX_RESULTS
                }
            },
            "required": ["query"]
        })
    }

    fn metadata(&self) -> ToolMetadata {
        // Live external read: authoritative for current info, safe to run in
        // parallel, no side effects.
        ToolMetadata {
            live_data: true,
            ..ToolMetadata::read_only()
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let query = require_str(&params, "query")?.trim().to_string();
        if query.is_empty() || query.len() > MAX_QUERY_BYTES {
            return Err(ToolError::InvalidParameters(format!(
                "query must contain 1..={MAX_QUERY_BYTES} bytes"
            )));
        }
        let count = params
            .get("count")
            .and_then(|v| v.as_u64())
            .map(|n| {
                usize::try_from(n)
                    .unwrap_or(MAX_RESULTS)
                    .clamp(1, MAX_RESULTS)
            })
            .unwrap_or(DEFAULT_RESULTS);

        let html = self.fetch(&query).await?;
        let results = parse_ddg_results(&html, count);

        let payload = serde_json::json!({
            "query": query,
            "count": results.len(),
            "results": results,
        });
        Ok(ToolOutput::success(payload, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        // Results are attacker-influenced web content.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
      <div class="result">
        <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Frust&amp;rut=abc">The Rust <b>Programming</b> Language</a>
        <a class="result__snippet" href="//dead">A language empowering everyone &amp; building reliable software.</a>
      </div>
      <div class="result">
        <a class="result__a" href="https://docs.rs/tokio">Tokio Docs</a>
        <a class="result__snippet">An async runtime.</a>
      </div>
    "#;

    #[test]
    fn parses_title_url_and_snippet() {
        let results = parse_ddg_results(SAMPLE, 10);
        assert_eq!(results.len(), 2);

        assert_eq!(results[0].title, "The Rust Programming Language");
        // Redirect wrapper unwrapped + percent-decoded.
        assert_eq!(results[0].url, "https://example.com/rust");
        assert_eq!(
            results[0].snippet,
            "A language empowering everyone & building reliable software."
        );

        assert_eq!(results[1].title, "Tokio Docs");
        assert_eq!(results[1].url, "https://docs.rs/tokio");
    }

    #[test]
    fn respects_result_limit() {
        let results = parse_ddg_results(SAMPLE, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "The Rust Programming Language");
    }

    #[test]
    fn empty_html_yields_no_results() {
        assert!(parse_ddg_results("<html><body>no results</body></html>", 5).is_empty());
    }

    #[test]
    fn percent_decode_handles_encoded_urls() {
        assert_eq!(
            percent_decode("https%3A%2F%2Fa.com%2Fb%3Fc%3Dd").as_deref(),
            Some("https://a.com/b?c=d")
        );
        assert_eq!(percent_decode("a+b").as_deref(), Some("a b"));
    }

    #[test]
    fn strip_html_collapses_and_decodes() {
        assert_eq!(
            strip_html("  <b>Hello</b>&amp;   <i>world</i> "),
            "Hello& world"
        );
    }

    #[test]
    fn result_links_reject_unsafe_schemes_and_credentials() {
        assert!(normalize_ddg_href("javascript:alert(1)").is_empty());
        assert!(normalize_ddg_href("https://user:secret@example.com/").is_empty());
        assert_eq!(
            normalize_ddg_href("https://example.com/path"),
            "https://example.com/path"
        );
    }

    #[test]
    fn result_text_is_bounded_and_strips_controls() {
        let text = format!("a\u{0}{}", "b".repeat(MAX_RESULT_TITLE_CHARS + 10));
        let bounded = bounded_result_text(&text, MAX_RESULT_TITLE_CHARS);
        assert_eq!(bounded.chars().count(), MAX_RESULT_TITLE_CHARS);
        assert!(!bounded.contains('\u{0}'));
    }
}
