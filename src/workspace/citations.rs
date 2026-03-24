//! Citation support for search results.
//!
//! Enriches search results with source citations linking back to
//! documents, pages, line ranges, and URLs.

use serde::{Deserialize, Serialize};

/// A citation reference for a search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    /// Title of the cited source.
    pub title: String,
    /// File path within the workspace.
    pub path: Option<String>,
    /// External URL (if applicable).
    pub url: Option<String>,
    /// Page number (for PDF/document sources).
    pub page: Option<u32>,
    /// Line range (start, end).
    pub line_range: Option<(u32, u32)>,
    /// Section heading within the document.
    pub section: Option<String>,
    /// Relevance score from the search (0.0-1.0).
    pub relevance: f32,
}

impl Citation {
    /// Create a simple path-based citation.
    pub fn from_path(path: impl Into<String>, relevance: f32) -> Self {
        let path_str: String = path.into();
        let title = path_str.rsplit('/').next().unwrap_or(&path_str).to_string();

        Self {
            title,
            path: Some(path_str),
            url: None,
            page: None,
            line_range: None,
            section: None,
            relevance,
        }
    }

    /// Create a URL-based citation.
    pub fn from_url(title: impl Into<String>, url: impl Into<String>, relevance: f32) -> Self {
        Self {
            title: title.into(),
            path: None,
            url: Some(url.into()),
            page: None,
            line_range: None,
            section: None,
            relevance,
        }
    }

    /// Add a line range.
    pub fn with_lines(mut self, start: u32, end: u32) -> Self {
        self.line_range = Some((start, end));
        self
    }

    /// Add a section heading.
    pub fn with_section(mut self, section: impl Into<String>) -> Self {
        self.section = Some(section.into());
        self
    }

    /// Add a page number.
    pub fn with_page(mut self, page: u32) -> Self {
        self.page = Some(page);
        self
    }
}

/// Format a list of citations as inline markdown references.
pub fn format_inline_citations(citations: &[Citation]) -> String {
    if citations.is_empty() {
        return String::new();
    }

    let mut parts = Vec::new();
    for (i, cite) in citations.iter().enumerate() {
        let num = i + 1;
        let location = format_location(cite);

        if let Some(url) = &cite.url {
            parts.push(format!("[{}] [{}]({}){}", num, cite.title, url, location));
        } else if let Some(path) = &cite.path {
            parts.push(format!("[{}] `{}`{}", num, path, location));
        } else {
            parts.push(format!("[{}] {}{}", num, cite.title, location));
        }
    }

    parts.join("\n")
}

/// Format a list of citations as a footnote block.
pub fn format_footnote_citations(citations: &[Citation]) -> String {
    if citations.is_empty() {
        return String::new();
    }

    let mut lines = vec!["---".to_string(), "**Sources:**".to_string()];

    for (i, cite) in citations.iter().enumerate() {
        let num = i + 1;
        let location = format_location(cite);
        let score = format!(" ({:.0}%)", cite.relevance * 100.0);

        if let Some(url) = &cite.url {
            lines.push(format!(
                "{}. [{}]({}){}{}",
                num, cite.title, url, location, score
            ));
        } else if let Some(path) = &cite.path {
            lines.push(format!("{}. `{}`{}{}", num, path, location, score));
        } else {
            lines.push(format!("{}. {}{}{}", num, cite.title, location, score));
        }
    }

    lines.join("\n")
}

/// Format location info (page, lines, section).
fn format_location(cite: &Citation) -> String {
    let mut parts = Vec::new();

    if let Some(section) = &cite.section {
        parts.push(format!(" § {}", section));
    }
    if let Some(page) = cite.page {
        parts.push(format!(" p.{}", page));
    }
    if let Some((start, end)) = cite.line_range {
        parts.push(format!(" L{}-{}", start, end));
    }

    parts.join(",")
}

/// Deduplication: merge citations pointing to the same source.
pub fn deduplicate_citations(citations: &[Citation]) -> Vec<Citation> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    for cite in citations {
        let key =
            cite.path.as_deref().unwrap_or("").to_string() + cite.url.as_deref().unwrap_or("");

        if seen.insert(key) {
            result.push(cite.clone());
        }
    }

    result
}

/// Sort citations by relevance (descending).
pub fn sort_by_relevance(citations: &mut [Citation]) {
    citations.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_path() {
        let cite = Citation::from_path("docs/README.md", 0.85);
        assert_eq!(cite.title, "README.md");
        assert_eq!(cite.path, Some("docs/README.md".to_string()));
        assert!((cite.relevance - 0.85).abs() < 0.001);
    }

    #[test]
    fn test_from_url() {
        let cite = Citation::from_url("Rust Book", "https://doc.rust-lang.org", 0.9);
        assert_eq!(cite.title, "Rust Book");
        assert!(cite.url.is_some());
    }

    #[test]
    fn test_with_lines() {
        let cite = Citation::from_path("main.rs", 0.8).with_lines(10, 20);
        assert_eq!(cite.line_range, Some((10, 20)));
    }

    #[test]
    fn test_with_section() {
        let cite = Citation::from_path("main.rs", 0.8).with_section("Introduction");
        assert_eq!(cite.section, Some("Introduction".to_string()));
    }

    #[test]
    fn test_format_inline_empty() {
        assert!(format_inline_citations(&[]).is_empty());
    }

    #[test]
    fn test_format_inline() {
        let cites = vec![
            Citation::from_path("foo.rs", 0.9),
            Citation::from_url("Docs", "https://example.com", 0.8),
        ];
        let result = format_inline_citations(&cites);
        assert!(result.contains("[1]"));
        assert!(result.contains("[2]"));
        assert!(result.contains("foo.rs"));
    }

    #[test]
    fn test_format_footnotes() {
        let cites = vec![Citation::from_path("test.rs", 0.75)];
        let result = format_footnote_citations(&cites);
        assert!(result.contains("Sources"));
        assert!(result.contains("75%"));
    }

    #[test]
    fn test_deduplicate() {
        let cites = vec![
            Citation::from_path("a.rs", 0.9),
            Citation::from_path("a.rs", 0.8),
            Citation::from_path("b.rs", 0.7),
        ];
        let deduped = deduplicate_citations(&cites);
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn test_sort_by_relevance() {
        let mut cites = vec![
            Citation::from_path("a.rs", 0.5),
            Citation::from_path("b.rs", 0.9),
            Citation::from_path("c.rs", 0.7),
        ];
        sort_by_relevance(&mut cites);
        assert_eq!(cites[0].title, "b.rs");
    }
}
