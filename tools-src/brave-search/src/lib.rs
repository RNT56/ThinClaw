//! Brave Search WASM Tool for ThinClaw.
//!
//! Provides web search using the Brave Search API, returning structured
//! results including titles, URLs, descriptions, and optional age/freshness.
//!
//! # Authentication
//!
//! Requires a Brave Search API key stored in the secrets store:
//!   `thinclaw tool auth brave-search`
//!
//! Get a free key (2000 queries/month) at:
//!   https://brave.com/search/api/
//!
//! The key is injected by the host into every HTTP request automatically.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const MAX_RESULTS: u32 = 20;
const DEFAULT_RESULTS: u32 = 5;

/// Top-level action enum for this tool.
#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
enum BraveSearchAction {
    #[serde(rename = "web_search")]
    WebSearch {
        /// The search query.
        query: String,
        /// Number of results to return (1-20, default 5).
        count: Option<u32>,
        /// Optional country code for results (e.g. "US", "DE"). Default: US.
        country: Option<String>,
        /// Optional language code (e.g. "en", "de"). Default: en.
        search_lang: Option<String>,
        /// Whether to include news results. Default: false.
        include_news: Option<bool>,
    },
    #[serde(rename = "news_search")]
    NewsSearch {
        /// The search query.
        query: String,
        /// Number of results to return (1-20, default 5).
        count: Option<u32>,
        /// Optional country code.
        country: Option<String>,
    },
}

struct BraveSearchTool;

impl exports::near::agent::tool::Guest for BraveSearchTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Search the web using Brave Search. Provides high-quality, privacy-respecting \
         web and news search results. Use 'web_search' for general queries and \
         'news_search' for recent news. Requires a free Brave Search API key stored \
         as 'brave_search_api_key' in the secrets store."
            .to_string()
    }
}

fn check_api_key() -> Result<(), String> {
    if near::agent::host::secret_exists("brave_search_api_key") {
        Ok(())
    } else {
        Err(
            "Brave Search API key not configured. \
             Get a free key at https://brave.com/search/api/ \
             then store it with: thinclaw tool auth brave-search"
                .to_string(),
        )
    }
}

/// Percent-encode a query string value.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
            }
        }
    }
    out
}

fn execute_inner(params: &str) -> Result<String, String> {
    let action: BraveSearchAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    check_api_key()?;

    match action {
        BraveSearchAction::WebSearch {
            query,
            count,
            country,
            search_lang,
            include_news,
        } => web_search(&query, count, country.as_deref(), search_lang.as_deref(), include_news.unwrap_or(false)),
        BraveSearchAction::NewsSearch {
            query,
            count,
            country,
        } => news_search(&query, count, country.as_deref()),
    }
}

/// Perform a web search.
fn web_search(
    query: &str,
    count: Option<u32>,
    country: Option<&str>,
    search_lang: Option<&str>,
    include_news: bool,
) -> Result<String, String> {
    if query.trim().is_empty() {
        return Err("Query cannot be empty".to_string());
    }
    if query.len() > 400 {
        return Err("Query too long (max 400 characters)".to_string());
    }

    let count = count.unwrap_or(DEFAULT_RESULTS).min(MAX_RESULTS).max(1);
    let country = country.unwrap_or("US");
    let search_lang = search_lang.unwrap_or("en");

    let mut url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}&country={}&search_lang={}&text_decorations=false&spellcheck=true",
        url_encode(query),
        count,
        url_encode(country),
        url_encode(search_lang),
    );

    if include_news {
        url.push_str("&result_filter=web,news");
    } else {
        url.push_str("&result_filter=web");
    }

    let headers = serde_json::json!({
        "Accept": "application/json",
        "Accept-Encoding": "gzip",
        "User-Agent": "ThinClaw-BraveSearch/1.0"
        // X-Subscription-Token is injected by the host via capabilities.json
    });

    let response = near::agent::host::http_request(
        "GET",
        &url,
        &headers.to_string(),
        None,
        Some(10_000), // 10s timeout
    )
    .map_err(|e| format!("HTTP request failed: {e}"))?;

    if response.status == 401 || response.status == 403 {
        return Err(
            "Invalid Brave Search API key. Check your key at https://brave.com/search/api/"
                .to_string(),
        );
    }
    if response.status == 429 {
        return Err("Brave Search rate limit exceeded. Please wait before retrying.".to_string());
    }
    if response.status != 200 {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!("Brave Search API error {}: {}", response.status, body));
    }

    let body: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    format_web_results(&body, query, count)
}

/// Perform a news-specific search.
fn news_search(query: &str, count: Option<u32>, country: Option<&str>) -> Result<String, String> {
    if query.trim().is_empty() {
        return Err("Query cannot be empty".to_string());
    }
    if query.len() > 400 {
        return Err("Query too long (max 400 characters)".to_string());
    }

    let count = count.unwrap_or(DEFAULT_RESULTS).min(MAX_RESULTS).max(1);
    let country = country.unwrap_or("US");

    let url = format!(
        "https://api.search.brave.com/res/v1/news/search?q={}&count={}&country={}&text_decorations=false",
        url_encode(query),
        count,
        url_encode(country),
    );

    let headers = serde_json::json!({
        "Accept": "application/json",
        "Accept-Encoding": "gzip",
        "User-Agent": "ThinClaw-BraveSearch/1.0"
    });

    let response = near::agent::host::http_request(
        "GET",
        &url,
        &headers.to_string(),
        None,
        Some(10_000),
    )
    .map_err(|e| format!("HTTP request failed: {e}"))?;

    if response.status == 401 || response.status == 403 {
        return Err(
            "Invalid Brave Search API key. Check your key at https://brave.com/search/api/"
                .to_string(),
        );
    }
    if response.status == 429 {
        return Err("Brave Search rate limit exceeded. Please wait before retrying.".to_string());
    }
    if response.status != 200 {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!("Brave News API error {}: {}", response.status, body));
    }

    let body: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    format_news_results(&body, query)
}

/// Format web search results into a clean, LLM-friendly string.
fn format_web_results(body: &serde_json::Value, query: &str, count: u32) -> Result<String, String> {
    let mut out = format!("# Brave Web Search Results\n**Query:** {}\n\n", query);

    // Web results
    let results = body
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array());

    if let Some(results) = results {
        if results.is_empty() {
            out.push_str("No web results found.\n");
        } else {
            out.push_str(&format!("## Web Results (top {} of {})\n\n", results.len().min(count as usize), results.len()));
            for (i, result) in results.iter().enumerate().take(count as usize) {
                let title = result.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
                let url = result.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let description = result
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No description available.");
                let age = result.get("age").and_then(|v| v.as_str()).unwrap_or("");

                out.push_str(&format!("### {}. {}\n", i + 1, title));
                out.push_str(&format!("**URL:** {}\n", url));
                if !age.is_empty() {
                    out.push_str(&format!("**Published:** {}\n", age));
                }
                out.push_str(&format!("{}\n\n", description));
            }
        }
    } else {
        out.push_str("No web results returned.\n");
    }

    // News results (if requested / present)
    let news_results = body
        .get("news")
        .and_then(|n| n.get("results"))
        .and_then(|r| r.as_array());

    if let Some(news) = news_results {
        if !news.is_empty() {
            out.push_str("\n## Recent News\n\n");
            for (i, article) in news.iter().enumerate().take(3) {
                let title = article.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
                let url = article.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let description = article.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let age = article.get("age").and_then(|v| v.as_str()).unwrap_or("");

                out.push_str(&format!("{}. **{}**\n", i + 1, title));
                if !age.is_empty() {
                    out.push_str(&format!("   *{}* — ", age));
                }
                out.push_str(&format!("{}\n", url));
                if !description.is_empty() {
                    out.push_str(&format!("   {}\n", description));
                }
                out.push('\n');
            }
        }
    }

    Ok(out)
}

/// Format news search results into a clean, LLM-friendly string.
fn format_news_results(body: &serde_json::Value, query: &str) -> Result<String, String> {
    let mut out = format!("# Brave News Search Results\n**Query:** {}\n\n", query);

    let results = body
        .get("results")
        .and_then(|r| r.as_array());

    if let Some(results) = results {
        if results.is_empty() {
            out.push_str("No news results found.\n");
        } else {
            for (i, article) in results.iter().enumerate() {
                let title = article.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
                let url = article.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let description = article.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let age = article.get("age").and_then(|v| v.as_str()).unwrap_or("");
                let source = article
                    .get("meta_url")
                    .and_then(|m| m.get("hostname"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                out.push_str(&format!("### {}. {}\n", i + 1, title));
                out.push_str(&format!("**URL:** {}\n", url));
                if !source.is_empty() {
                    out.push_str(&format!("**Source:** {}\n", source));
                }
                if !age.is_empty() {
                    out.push_str(&format!("**Published:** {}\n", age));
                }
                if !description.is_empty() {
                    out.push_str(&format!("{}\n", description));
                }
                out.push('\n');
            }
        }
    } else {
        out.push_str("No results returned.\n");
    }

    Ok(out)
}

const SCHEMA: &str = r#"{
    "type": "object",
    "required": ["action"],
    "oneOf": [
        {
            "properties": {
                "action": { "const": "web_search" },
                "query": {
                    "type": "string",
                    "description": "The search query (max 400 characters)"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results to return (1-20, default 5)",
                    "minimum": 1,
                    "maximum": 20,
                    "default": 5
                },
                "country": {
                    "type": "string",
                    "description": "Country code for results (e.g. 'US', 'DE', 'GB'). Default: US"
                },
                "search_lang": {
                    "type": "string",
                    "description": "Language code (e.g. 'en', 'de', 'fr'). Default: en"
                },
                "include_news": {
                    "type": "boolean",
                    "description": "Whether to also include news results. Default: false"
                }
            },
            "required": ["action", "query"]
        },
        {
            "properties": {
                "action": { "const": "news_search" },
                "query": {
                    "type": "string",
                    "description": "The news search query (max 400 characters)"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of news articles to return (1-20, default 5)",
                    "minimum": 1,
                    "maximum": 20,
                    "default": 5
                },
                "country": {
                    "type": "string",
                    "description": "Country code for news results (e.g. 'US', 'DE'). Default: US"
                }
            },
            "required": ["action", "query"]
        }
    ]
}"#;

export!(BraveSearchTool);
