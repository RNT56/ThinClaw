//! llms.txt discovery.
//!
//! Auto-discovers site metadata from `/.well-known/llms.txt` files,
//! following the proposed standard for LLM-friendly site descriptions.
//!
//! See: https://llmstxt.org/

use serde::{Deserialize, Serialize};

/// Parsed llms.txt content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmsTxt {
    /// Site name.
    pub name: Option<String>,
    /// Site description.
    pub description: Option<String>,
    /// Full URL where this was discovered.
    pub source_url: String,
    /// Sections from the llms.txt document.
    pub sections: Vec<LlmsTxtSection>,
    /// Raw content.
    pub raw: String,
}

/// A section in an llms.txt file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmsTxtSection {
    /// Section heading.
    pub heading: String,
    /// Links in this section.
    pub links: Vec<LlmsTxtLink>,
    /// Text content (non-link lines).
    pub text: String,
}

/// A link from an llms.txt file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmsTxtLink {
    pub title: String,
    pub url: String,
    pub description: Option<String>,
}

/// Discover llms.txt from a domain.
pub async fn discover(domain: &str) -> Result<LlmsTxt, LlmsDiscoveryError> {
    let urls = [
        format!("https://{}/.well-known/llms.txt", domain),
        format!("https://{}/llms.txt", domain),
    ];

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| LlmsDiscoveryError::Network(e.to_string()))?;

    for url in &urls {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(text) = resp.text().await {
                    return Ok(parse_llms_txt(&text, url));
                }
            }
            _ => continue,
        }
    }

    Err(LlmsDiscoveryError::NotFound(domain.to_string()))
}

/// Parse llms.txt content.
pub fn parse_llms_txt(content: &str, source_url: &str) -> LlmsTxt {
    let mut name = None;
    let mut description = None;
    let mut sections = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_links = Vec::new();
    let mut current_text = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Title (first H1)
        if trimmed.starts_with("# ") && name.is_none() {
            name = Some(trimmed[2..].trim().to_string());
            continue;
        }

        // Description (first non-heading, non-empty line after title)
        if name.is_some()
            && description.is_none()
            && !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && !trimmed.starts_with('-')
            && !trimmed.starts_with('[')
        {
            description = Some(trimmed.to_string());
            continue;
        }

        // Section headings
        if trimmed.starts_with("## ") {
            // Save previous section
            if let Some(heading) = current_heading.take() {
                sections.push(LlmsTxtSection {
                    heading,
                    links: std::mem::take(&mut current_links),
                    text: std::mem::take(&mut current_text).trim().to_string(),
                });
            }
            current_heading = Some(trimmed[3..].trim().to_string());
            continue;
        }

        // Links: [title](url): description
        // or - [title](url): description
        let link_line = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        if link_line.starts_with('[') {
            if let Some(link) = parse_link(link_line) {
                current_links.push(link);
                continue;
            }
        }

        // Regular text
        if !trimmed.is_empty() {
            if !current_text.is_empty() {
                current_text.push(' ');
            }
            current_text.push_str(trimmed);
        }
    }

    // Save final section
    if let Some(heading) = current_heading {
        sections.push(LlmsTxtSection {
            heading,
            links: current_links,
            text: current_text.trim().to_string(),
        });
    }

    LlmsTxt {
        name,
        description,
        source_url: source_url.to_string(),
        sections,
        raw: content.to_string(),
    }
}

/// Parse a markdown-style link line.
fn parse_link(line: &str) -> Option<LlmsTxtLink> {
    let title_end = line.find(']')?;
    let title = line[1..title_end].to_string();

    let url_start = line[title_end..].find('(')? + title_end + 1;
    let url_end = line[url_start..].find(')')? + url_start;
    let url = line[url_start..url_end].to_string();

    let description = line[url_end + 1..]
        .trim()
        .strip_prefix(':')
        .or_else(|| line[url_end + 1..].trim().strip_prefix('-'))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Some(LlmsTxtLink {
        title,
        url,
        description,
    })
}

/// Errors during llms.txt discovery.
#[derive(Debug, Clone)]
pub enum LlmsDiscoveryError {
    NotFound(String),
    Network(String),
    ParseError(String),
}

impl std::fmt::Display for LlmsDiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(domain) => write!(f, "No llms.txt found for {}", domain),
            Self::Network(e) => write!(f, "Network error: {}", e),
            Self::ParseError(e) => write!(f, "Parse error: {}", e),
        }
    }
}

impl std::error::Error for LlmsDiscoveryError {}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_LLMS_TXT: &str = r#"# Example Corp

> The best example company

## Documentation

- [Getting Started](https://example.com/docs/start): Introduction guide
- [API Reference](https://example.com/docs/api): Full API docs

## Resources

- [Blog](https://example.com/blog)
- [GitHub](https://github.com/example)
"#;

    #[test]
    fn test_parse_title() {
        let parsed = parse_llms_txt(SAMPLE_LLMS_TXT, "https://example.com/llms.txt");
        assert_eq!(parsed.name, Some("Example Corp".to_string()));
    }

    #[test]
    fn test_parse_description() {
        let parsed = parse_llms_txt(SAMPLE_LLMS_TXT, "https://example.com/llms.txt");
        assert_eq!(
            parsed.description,
            Some("> The best example company".to_string())
        );
    }

    #[test]
    fn test_parse_sections() {
        let parsed = parse_llms_txt(SAMPLE_LLMS_TXT, "https://example.com/llms.txt");
        assert_eq!(parsed.sections.len(), 2);
        assert_eq!(parsed.sections[0].heading, "Documentation");
        assert_eq!(parsed.sections[1].heading, "Resources");
    }

    #[test]
    fn test_parse_links() {
        let parsed = parse_llms_txt(SAMPLE_LLMS_TXT, "https://example.com/llms.txt");
        let doc_section = &parsed.sections[0];
        assert_eq!(doc_section.links.len(), 2);
        assert_eq!(doc_section.links[0].title, "Getting Started");
        assert_eq!(doc_section.links[0].url, "https://example.com/docs/start");
        assert_eq!(
            doc_section.links[0].description,
            Some("Introduction guide".to_string())
        );
    }

    #[test]
    fn test_parse_link_no_description() {
        let parsed = parse_llms_txt(SAMPLE_LLMS_TXT, "https://example.com/llms.txt");
        let resources = &parsed.sections[1];
        assert_eq!(resources.links.len(), 2);
        assert_eq!(resources.links[0].description, None);
    }

    #[test]
    fn test_parse_single_link() {
        let link = parse_link("[Title](https://example.com): A description");
        assert!(link.is_some());
        let link = link.unwrap();
        assert_eq!(link.title, "Title");
        assert_eq!(link.url, "https://example.com");
        assert_eq!(link.description, Some("A description".to_string()));
    }

    #[test]
    fn test_parse_link_no_desc() {
        let link = parse_link("[Title](https://example.com)");
        assert!(link.is_some());
        assert_eq!(link.unwrap().description, None);
    }

    #[test]
    fn test_empty_content() {
        let parsed = parse_llms_txt("", "https://example.com/llms.txt");
        assert!(parsed.name.is_none());
        assert!(parsed.sections.is_empty());
    }

    #[test]
    fn test_error_display() {
        let err = LlmsDiscoveryError::NotFound("example.com".to_string());
        assert!(format!("{}", err).contains("example.com"));
    }
}
