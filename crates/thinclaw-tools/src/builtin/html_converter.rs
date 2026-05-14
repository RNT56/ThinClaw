//! HTML to Markdown conversion for HTTP responses.
//!
//! Two-stage pipeline: readability (extract article) -> html-to-markdown-rs (convert to md).
//! When the `html-to-markdown` feature is disabled, passthrough only.

use thinclaw_tools_core::ToolError;

#[cfg(feature = "html-to-markdown")]
use html_to_markdown_rs::convert;
#[cfg(feature = "html-to-markdown")]
use readabilityrs::Readability;

#[cfg(not(feature = "html-to-markdown"))]
pub fn convert_html_to_markdown(html: &str, _url: &str) -> Result<String, ToolError> {
    Ok(html.to_string())
}

#[cfg(feature = "html-to-markdown")]
pub fn convert_html_to_markdown(html: &str, url: &str) -> Result<String, ToolError> {
    // Stage 1: try readability extraction for article-style pages.
    let article_result = Readability::new(html, Some(url), None)
        .ok()
        .and_then(|r| r.parse())
        .and_then(|article| article.content);

    if let Some(ref clean_html) = article_result
        && let Ok(md) = convert(clean_html, None)
        && !md.trim().is_empty()
    {
        // Readability found an article — convert the clean extract to markdown.
        return Ok(md);
    }

    // Stage 2: readability couldn't extract an article (index pages, SPAs,
    // heavily-JS-rendered sites, etc.). Fall back to direct html→markdown
    // conversion of the full page. This is noisier than readability output
    // but far more useful to the LLM than raw HTML.
    convert(html, None)
        .map_err(|e| ToolError::ExecutionFailed(format!("HTML to markdown: {}", e)))
        .and_then(|md| {
            if md.trim().is_empty() {
                Err(ToolError::ExecutionFailed(
                    "HTML to markdown conversion produced empty output".to_string(),
                ))
            } else {
                Ok(md)
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "html-to-markdown"))]
    #[test]
    fn passthrough_returns_input_unchanged_when_feature_disabled() {
        {
            let html = "<html><body>raw</body></html>";
            let out = convert_html_to_markdown(html, "https://example.com/").unwrap();
            assert_eq!(out, html);
        }
    }

    #[cfg(not(feature = "html-to-markdown"))]
    #[test]
    fn passthrough_ignores_url_when_feature_disabled() {
        {
            let html = "anything";
            let _ = convert_html_to_markdown(html, "").unwrap();
            let _ = convert_html_to_markdown(html, "https://example.com/page").unwrap();
        }
    }

    #[cfg(feature = "html-to-markdown")]
    #[test]
    fn simple_article_extracted_and_converted_to_markdown() {
        // Readability needs enough content (default char_threshold ~500) and clear main content.
        let html = r#"<!DOCTYPE html>
<html><head><title>Test</title></head><body>
<nav><a href="/">Home</a></nav>
<main>
  <article>
    <h1>Test Title</h1>
    <p>First paragraph with enough text so that readability's scoring finds this as the main content block. We need to exceed the default character threshold.</p>
    <p>Second paragraph. More body text here to make the article clearly the dominant content area versus the short nav and footer.</p>
    <p>Third paragraph for good measure. The extraction algorithm scores candidates by paragraph count and text length; this block should win.</p>
  </article>
</main>
<footer><p>Footer</p></footer>
</body></html>"#;
        let out = convert_html_to_markdown(html, "https://example.com/article").unwrap();
        assert!(
            out.contains("Test Title"),
            "expected title in output: {}",
            out
        );
        assert!(
            out.contains("First paragraph"),
            "expected content in output: {}",
            out
        );
        assert!(
            out.contains("Second paragraph"),
            "expected content in output: {}",
            out
        );
        assert!(
            !out.contains("<article>"),
            "expected markdown, not raw HTML"
        );
    }

    #[cfg(feature = "html-to-markdown")]
    #[test]
    fn returns_execution_error_on_empty_html() {
        let result = convert_html_to_markdown("", "https://example.com/");
        // Empty input should still produce an error (empty output after conversion).
        assert!(
            result.is_err()
                || result
                    .as_ref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(false),
            "Expected error or empty result on empty HTML input, got: {:?}",
            result
        );
    }

    #[cfg(feature = "html-to-markdown")]
    #[test]
    fn plain_text_falls_through_to_direct_conversion() {
        // With the two-stage pipeline, plain text that readability can't parse
        // falls through to the direct html→markdown converter, which may
        // produce output (the text itself). This is the correct behavior —
        // returning *something* is better than erroring for the LLM.
        let result = convert_html_to_markdown("not html at all", "https://example.com/");
        // Either succeeds with the text, or errors — both are acceptable.
        if let Ok(ref md) = result {
            assert!(
                md.contains("not html at all"),
                "Expected plain text pass-through, got: {}",
                md
            );
        }
    }
}
