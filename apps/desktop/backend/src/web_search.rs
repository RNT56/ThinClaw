use scraper::{Html, Selector};
use serde::{Deserialize, Serialize}; // Added missing imports
                                     // removed unused imports

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)] // Added Type
pub struct WebSearchResult {
    pub title: String,
    pub link: String,
    pub snippet: String,
}

pub async fn perform_web_search(query: &str) -> Result<(String, Vec<WebSearchResult>), String> {
    if query.trim().is_empty() || query.len() > 4_096 || query.contains('\0') {
        return Err("Web search query is empty, too large, or contains NUL".to_string());
    }

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get("https://duckduckgo.com/html/")
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|error| crate::rig_lib::http::transport_error("Web search failed", error))?;
    let resp = crate::rig_lib::http::checked_response(resp, "DuckDuckGo").await?;
    let html = thinclaw_core::http_response::bounded_text(resp, 4 * 1024 * 1024)
        .await
        .map_err(|error| format!("Invalid bounded web search response: {error}"))?;

    let document = Html::parse_document(&html);
    // Compile the (constant) CSS selectors once, not on every search call.
    static RESULT_SEL: std::sync::OnceLock<Selector> = std::sync::OnceLock::new();
    static TITLE_SEL: std::sync::OnceLock<Selector> = std::sync::OnceLock::new();
    static SNIPPET_SEL: std::sync::OnceLock<Selector> = std::sync::OnceLock::new();
    static LINK_SEL: std::sync::OnceLock<Selector> = std::sync::OnceLock::new();
    let result_selector =
        RESULT_SEL.get_or_init(|| Selector::parse(".result").expect("valid static selector"));
    let title_selector =
        TITLE_SEL.get_or_init(|| Selector::parse(".result__title").expect("valid static selector"));
    let snippet_selector = SNIPPET_SEL
        .get_or_init(|| Selector::parse(".result__snippet").expect("valid static selector"));
    let link_selector =
        LINK_SEL.get_or_init(|| Selector::parse(".result__a").expect("valid static selector"));

    let mut results = Vec::new();
    let mut context_strings = Vec::new();

    for element in document.select(result_selector).take(5) {
        let title = element
            .select(title_selector)
            .next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default();

        let link = element
            .select(link_selector)
            .next()
            .and_then(|e| e.value().attr("href"))
            .map(|href| {
                if href.starts_with("//") {
                    format!("https:{}", href)
                } else if href.starts_with("/") {
                    format!("https://duckduckgo.com{}", href)
                } else {
                    href.to_string()
                }
            })
            .unwrap_or_default();

        let snippet = element
            .select(snippet_selector)
            .next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default();

        let title = title.trim();
        let snippet = snippet.trim();
        let link_is_safe = reqwest::Url::parse(&link).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        });
        if !title.is_empty()
            && title.len() <= 1_024
            && snippet.len() <= 8 * 1024
            && link.len() <= 4_096
            && link_is_safe
            && !title.chars().any(char::is_control)
            && !snippet
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        {
            results.push(WebSearchResult {
                title: title.to_string(),
                link: link.clone(),
                snippet: snippet.to_string(),
            });

            context_strings.push(format!(
                "Title: {}\nSource: {}\nSnippet: {}\n",
                title, link, snippet
            ));
        }
    }

    if results.is_empty() {
        return Ok(("No relevant search results found.".to_string(), vec![]));
    }

    // Inject Date
    let current_date = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let final_context = format!(
        "Current Date: {}\n\n{}",
        current_date,
        context_strings.join("\n---\n")
    );

    Ok((final_context, results))
}
