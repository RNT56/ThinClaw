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
    println!("[web_search] Searching for: {}", query);

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!("https://duckduckgo.com/html/?q={}", query);
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    let html = resp.text().await.map_err(|e| e.to_string())?;

    let document = Html::parse_document(&html);
    let result_selector = Selector::parse(".result").unwrap();
    let title_selector = Selector::parse(".result__title").unwrap();
    let snippet_selector = Selector::parse(".result__snippet").unwrap();

    let link_selector = Selector::parse(".result__a").unwrap();

    let mut results = Vec::new();
    let mut context_strings = Vec::new();

    for (i, element) in document.select(&result_selector).take(5).enumerate() {
        let title = element
            .select(&title_selector)
            .next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default();

        let link = element
            .select(&link_selector)
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
            .select(&snippet_selector)
            .next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default();

        if !title.is_empty() {
            println!(
                "[web_search] Result {}: Title='{}', Link='{}', Snippet='{}'",
                i + 1,
                title.trim(),
                link,
                snippet.trim()
            );

            results.push(WebSearchResult {
                title: title.trim().to_string(),
                link: link.to_string(),
                snippet: snippet.trim().to_string(),
            });

            context_strings.push(format!(
                "Title: {}\nSource: {}\nSnippet: {}\n",
                title.trim(),
                link,
                snippet.trim()
            ));
        }
    }

    if results.is_empty() {
        println!("[web_search] No results found in HTML parsing.");
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

#[tauri::command]
#[specta::specta]
pub async fn check_web_search(query: String) -> Result<String, String> {
    perform_web_search(&query)
        .await
        .map(|(ctx, _)| ctx)
        .map_err(|e| e.to_string())
}
