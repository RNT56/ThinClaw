use crate::rig_lib::unified_provider::UnifiedProvider;
use crate::rig_lib::tools::trusted_sources;
use futures::{stream, StreamExt};
use rig::completion::ToolDefinition;
use rig::completion::{CompletionModel, CompletionRequest};
use rig::tool::Tool;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use std::sync::atomic::Ordering;

use tauri::Manager;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("Request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("System error: {0}")]
    System(String),
}

#[derive(Deserialize)]
pub struct SearchArgs {
    pub query: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SearchResult {
    pub title: String,
    pub link: String,
    pub snippet: String,
    #[serde(default)]
    pub score: f32,
    #[serde(default)]
    pub reasoning: String,
}

#[derive(Serialize, Clone, specta::Type)]
pub struct ScrapingProgress {
    pub id: Option<String>,
    pub url: String,
    pub status: String, // "visiting", "scraped"
    pub title: Option<String>,
    pub content_preview: Option<String>,
}

use crate::config::ConfigManager;

pub struct DDGSearchTool {
    pub app: Option<tauri::AppHandle>,
    pub max_total_chars: usize,
    pub summarizer: Option<UnifiedProvider>,
    pub conversation_id: Option<String>,
}

impl Tool for DDGSearchTool {
    const NAME: &'static str = "web_search";

    type Error = SearchError;
    type Args = SearchArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "web_search".to_string(),
            description: "A research tool. Use this to find information. The output will be formatted Markdown text containing search results. Read this text and summarize the key findings for the user.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let cancellation_token = if let Some(app) = &self.app {
            app.try_state::<crate::sidecar::SidecarManager>()
                .map(|s| s.cancellation_token.clone())
        } else {
            None
        };

        if let Some(token) = &cancellation_token {
            if token.load(Ordering::Relaxed) {
                return Err(SearchError::System("Cancelled by user".into()));
            }
        }

        let url = format!(
            "https://duckduckgo.com/html/?q={}",
            urlencoding::encode(&args.query)
        );

        if let Some(app) = &self.app {
            use tauri::Emitter;
            #[derive(Serialize, Clone, specta::Type)]
            struct WebSearchStatus {
                id: Option<String>,
                step: String,
                message: String,
            }
            let _ = app.emit(
                "web_search_status",
                WebSearchStatus {
                    id: self.conversation_id.clone(),
                    step: "searching".into(),
                    message: format!("Agent searching for: {}", args.query),
                },
            );
        }

        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
            .build()?;

        let mut initial_results = Vec::new();
        {
            let res = client.get(&url).send().await?.text().await?;
            let document = Html::parse_document(&res);

            let result_selector = Selector::parse(".result").unwrap();
            let title_selector = Selector::parse(".result__title .result__a").unwrap();
            let snippet_selector = Selector::parse(".result__snippet").unwrap();

            for element in document.select(&result_selector).take(20) {
                let title = element
                    .select(&title_selector)
                    .next()
                    .map(|e| e.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();

                let link = element
                    .select(&title_selector)
                    .next()
                    .and_then(|e| e.value().attr("href"))
                    .map(|s| s.to_string())
                    .unwrap_or_default();

                let snippet = element
                    .select(&snippet_selector)
                    .next()
                    .map(|e| e.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();

                if !title.is_empty() && !link.is_empty() {
                    if link.contains("duckduckgo.com") || link.contains("y.js") {
                        continue;
                    }
                    initial_results.push(SearchResult {
                        title,
                        link,
                        snippet,
                        score: 0.0,
                        reasoning: String::new(),
                    });
                }
            }
        }

        // Rank candidates: Prioritize trusted sources
        initial_results.sort_by(|a, b| {
            let a_trusted = trusted_sources::is_trusted(&a.link);
            let b_trusted = trusted_sources::is_trusted(&b.link);
            b_trusted.cmp(&a_trusted) // true > false
        });

        // Limit to 5 results after filtering and ranking
        initial_results.truncate(5);

        // Emit initial results for UI cards
        if let Some(app) = &self.app {
            use tauri::Emitter;
            let _ = app.emit("web_search_results", json!({
                "id": self.conversation_id,
                "results": &initial_results
            }));
        }

        if let Some(token) = &cancellation_token {
            if token.load(Ordering::Relaxed) {
                return Err(SearchError::System("Cancelled by user".into()));
            }
        }

        let initial_count = initial_results.len();
        if let Some(app) = &self.app {
            use tauri::Emitter;
            #[derive(Serialize, Clone, specta::Type)]
            struct WebSearchStatus {
                id: Option<String>,
                step: String,
                message: String,
            }
            let _ = app.emit(
                "web_search_status",
                WebSearchStatus {
                    id: self.conversation_id.clone(),
                    step: "scraping".into(),
                    message: format!("Deep scraping and analyzing {} results...", initial_count),
                },
            );
        }

        // Get config
        let (scrape_limit, analysis_limit, chunk_size, max_total) = if let Some(app) = &self.app {
            let state = app.state::<ConfigManager>();
            let conf = state.get_config();
            (
                conf.scrape_concurrency_limit,
                conf.search_concurrency_limit,
                conf.summarization_chunk_size,
                conf.max_scrape_chars,
            )
        } else {
            (2, 2, 4000, 15000)
        };

        // Calculate limits (fallback if config was default or 0)
        let max_total = if max_total > 0 { max_total } else { 15000 };
        let chars_per_slot = max_total / 5; // Target size per final result

        // Use config chunk size or fallback to dynamic
        let chunk_size: u32 = if chunk_size > 0 {
            chunk_size
        } else {
            let estimated_context = (max_total as f64 / 0.6) as u32;
            (estimated_context as f64 * 0.5) as u32
        };

        // Launch cleanup thread and scraping in an isolated runtime to handle !Sync browser
        let initial_results_clone = initial_results.clone();
        let app_handle = self.app.clone();
        let conversation_id_top = self.conversation_id.clone();
        let token_for_spawn = cancellation_token.clone();

        let mut scraping_result = tokio::task::spawn_blocking(move || {
            let conversation_id_clone = conversation_id_top;
            let token_for_inner = token_for_spawn;
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| SearchError::System(format!("Failed to build runtime: {}", e)))?;

            rt.block_on(async {
                use crate::rig_lib::tools::scrape_page::ScrapePageTool;
                let scraper = ScrapePageTool {
                    app: std::sync::Mutex::new(app_handle),
                };

                // Launch browser once (shared for all parallel tabs)

                let (mut browser, handler) = match scraper.launch_browser().await {
                    Ok(b) => b,
                    Err(e) => {
                        println!("Failed to launch browser: {}", e);
                        return Ok(initial_results_clone);
                    }
                };

                // Parallel Scraping
                let results = stream::iter(initial_results_clone)
                    .map(|mut result| {
                        let scraper = &scraper;
                        let browser = &browser;
                        let conversation_id = conversation_id_clone.clone();
                        let token_ref = token_for_inner.clone();
                        async move {
                            if let Some(token) = token_ref {
                                if token.load(Ordering::Relaxed) {
                                    return result;
                                }
                            }

                            // Emit "visiting"
                            if let Ok(guard) = scraper.app.lock() {
                                if let Some(app) = guard.as_ref() {
                                    use tauri::Emitter;
                                    let _ = app.emit(
                                        "scraping_progress",
                                        ScrapingProgress {
                                            id: conversation_id.clone(),
                                            url: result.link.clone(),
                                            status: "visiting".into(),
                                            title: Some(result.title.clone()),
                                            content_preview: None,
                                        },
                                    );
                                }
                            }

                            match scraper.scrape_url(browser, &result.link).await {
                                Ok(content) => {
                                    // Emit "scraped" with preview
                                    if let Ok(guard) = scraper.app.lock() {
                                        if let Some(app) = guard.as_ref() {
                                            use tauri::Emitter;
                                            let preview =
                                                content.chars().take(500).collect::<String>();
                                            let _ = app.emit(
                                                "scraping_progress",
                                                ScrapingProgress {
                                                    id: conversation_id.clone(),
                                                    url: result.link.clone(),
                                                    status: "scraped".into(),
                                                    title: Some(result.title.clone()),
                                                    content_preview: Some(preview),
                                                },
                                            );
                                        }
                                    }
                                    result.snippet = content;
                                }
                                Err(e) => {
                                    println!("Scraping failed for {}: {}", result.link, e);
                                }
                            }
                            result
                        }
                    })
                    .buffer_unordered(scrape_limit as usize)
                    .collect::<Vec<_>>()
                    .await;

                // Systematic Cleanup
                let _ = browser.close().await;
                let _ = handler.await;
                // Small sleep to ensure the OS handles the socket cleanup before dropping the runtime
                tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

                Ok::<Vec<SearchResult>, SearchError>(results)
            })
        })
        .await
        .map_err(|e| SearchError::System(format!("Task join error: {}", e)))??;

        // Post-Processing: Map-Reduce Summarization
        if let Some(summarizer) = &self.summarizer {
            if let Some(token) = &cancellation_token {
                if token.load(Ordering::Relaxed) {
                    return Err(SearchError::System("Cancelled by user".into()));
                }
            }
            for result in scraping_result.iter_mut() {
                let content_len = result.snippet.len();
                let original_content = result.snippet.clone();

                // Optimization: Skip validation for short content
                if content_len < 2000 {
                    result.score = 8.0; // Assume high relevance if it was a top hit and short enough to read fully
                    result.snippet = original_content;
                    continue;
                }

                // If content is significantly larger than its slot (1.2x), we summarize
                if content_len > (chars_per_slot as f64 * 1.2) as usize {
                    // ...
                    // Chunking: Divide content into chunks of chunk_size with 10% overlap
                    let overlap = (chunk_size as f64 * 0.1) as usize;
                    let mut chunks = Vec::new();
                    let content_chars: Vec<char> = result.snippet.chars().collect();
                    let mut start = 0;

                    while start < content_chars.len() {
                        let end = std::cmp::min(start + chunk_size as usize, content_chars.len());
                        chunks.push(content_chars[start..end].iter().collect::<String>());
                        start += chunk_size as usize - overlap;
                    }

                    // Aggregation/Analysis
                    let chunks_len = chunks.len();
                    let summaries_conv_id = self.conversation_id.clone();
                    let query_clone = args.query.clone();
                    let app_top = self.app.clone();
                    let app_top_for_closure = app_top.clone();
                    let summaries_conv_id_for_closure = summaries_conv_id.clone();
                    let token_for_stream = cancellation_token.clone();

                    let summaries: Vec<(f32, String)> = stream::iter(chunks)
                        .enumerate()
                        .map(move |(i, chunk)| {
                            let summarizer = summarizer.clone();
                            let query = query_clone.clone();
                            let app = app_top_for_closure.clone();
                            let conversation_id_inner = summaries_conv_id_for_closure.clone();
                            let token_ref = token_for_stream.clone();

                            async move {
                                if let Some(token) = token_ref {
                                    if token.load(Ordering::Relaxed) {
                                        return None;
                                    }
                                }

                                if let Some(app) = &app {
                                    use tauri::Emitter;
                                    #[derive(Serialize, Clone, specta::Type)]
                                    struct WebSearchStatus {
                                        id: Option<String>,
                                        step: String,
                                        message: String,
                                    }
                                    let _ = app.emit(
                                        "web_search_status",
                                        WebSearchStatus {
                                            id: conversation_id_inner.clone(),
                                            step: "analyzing".into(),
                                            message: format!("Analyzing chunk {}/{}...", i + 1, chunks_len),
                                        },
                                    );
                                }

                                let prompt = format!(
                                    "You are a strict Validator. Your goal is to ensure this text actually ANSWERS the user's question, not just contains keywords.\n\
                                    Query: '{}'\n\n\
                                    Instructions:\n\
                                    1. Check if the text contains specific facts, numbers, or direct answers to the query.\n\
                                    2. Ignore generic intro text, navigation, or clickbait.\n\
                                    3. Score from 0.0 (irrelevant) to 10.0 (perfect direct answer).\n\
                                    4. Provide a brief reasoning.\n\n\
                                    Return JSON:\n\
                                    {{\n\
                                      \"score\": 8.5,\n\
                                      \"reasoning\": \"Contains exact date and figures requested...\",\n\
                                      \"summary\": \"The text states...\"\n\
                                    }}\n\n\
                                    Text Chunk:\n{}",
                                    query,
                                    chunk
                                );

                                let req = CompletionRequest {
                                    prompt,
                                    chat_history: vec![],
                                    preamble: Some("You are a strict data analyst. Output only JSON.".into()),
                                    documents: vec![],
                                    tools: vec![],
                                    temperature: Some(0.1),
                                    max_tokens: Some(1024),
                                    additional_params: None,
                                };

                                if let Ok(resp) = summarizer.completion(req).await {
                                    let text = match resp.choice {
                                        rig::completion::ModelChoice::Message(msg) => msg,
                                        _ => String::new(),
                                    };

                                    let clean_text = text.trim();
                                    let clean_text = if clean_text.starts_with("```json") {
                                        clean_text.trim_start_matches("```json").trim_end_matches("```").trim()
                                    } else if clean_text.starts_with("```") {
                                        clean_text.trim_start_matches("```").trim_end_matches("```").trim()
                                    } else {
                                        clean_text
                                    };

                                    #[derive(Deserialize)]
                                    struct SummaryResponse {
                                        score: f32,
                                        reasoning: String,
                                        summary: String,
                                    }

                                    if let Ok(parsed) = serde_json::from_str::<SummaryResponse>(clean_text) {
                                        if parsed.score >= 4.0 {
                                            // Return tuple with score for aggregation
return Some((parsed.score, format!("Score: {}\nReasoning: {}\nSummary: {}", parsed.score, parsed.reasoning, parsed.summary)));
                                        }
                                    }
                                }
                                None
                            }
                        })
                        .buffer_unordered(analysis_limit as usize)
                        .filter_map(|x| async { x })
                        .collect()
                        .await;

                    // Aggregate scores and combine summaries
                    let count = summaries.len();
                    let (avg_score, combined_summary) = if count > 0 {
                        let total_score: f32 = summaries.iter().map(|(s, _)| s).sum();
                        let combined = summaries.into_iter().map(|(_, text)| text).collect::<Vec<String>>().join("\n\n");
                        (total_score / count as f32, combined)
                    } else {
                        (0.0, String::new())
                    };

                    result.score = avg_score;

                    // Recursive Reduce if still too big
                    if combined_summary.len() > chars_per_slot as usize {
                        if let Some(app) = &app_top {
                             use tauri::Emitter;
                             #[derive(Serialize, Clone, specta::Type)]
                             struct WebSearchStatus {
                                 id: Option<String>,
                                 step: String,
                                 message: String,
                             }
                             let _ = app.emit(
                                 "web_search_status",
                                 WebSearchStatus {
                                     id: summaries_conv_id.clone(),
                                     step: "summarizing".into(),
                                     message: "Summarizing findings...".into(),
                                 },
                             );
                        }

                        let prompt = format!(
                            "Compress the following summaries into a single coherent text relevant to '{}'. Max length {} chars.\n\n{}",
                            args.query, chars_per_slot, combined_summary
                        );
                        let req = CompletionRequest {
                            prompt,
                            chat_history: vec![],
                            preamble: None,
                            documents: vec![],
                            tools: vec![],
                            temperature: None,
                            max_tokens: None,
                            additional_params: None,
                        };
                        if let Ok(resp) = summarizer.completion(req).await {
                            let content = match resp.choice {
                                rig::completion::ModelChoice::Message(msg) => msg,
                                _ => combined_summary.clone(),
                            };
                            result.snippet = format!("Analyzed Context:\n{}", content);
                        } else {
                            result.snippet =
                                format!("Extracted Highlights:\n{}", combined_summary);
                        }
                    } else {
                        result.snippet = format!("Extracted Highlights:\n{}", combined_summary);
                    }

                    // Fallback if summary failed/empty
                    if result.snippet.trim().is_empty() || result.snippet.len() < 50 {
                        // Just truncate original
                        if content_len > chars_per_slot as usize {
                            let mut safe_end = chars_per_slot as usize;
                            while !original_content.is_char_boundary(safe_end) {
                                safe_end -= 1;
                            }
                            result.snippet = format!("{}...", &original_content[..safe_end]);
                        } else {
                            result.snippet = original_content.clone();
                        };
                    }

                } else {
                    // Fits in slot, keep as is (subject to generic truncation logic if we want)
                    if result.snippet.len() > chars_per_slot as usize {
                        let mut safe_end = chars_per_slot as usize;
                        while !result.snippet.is_char_boundary(safe_end) {
                            safe_end -= 1;
                        }
                        result.snippet.truncate(safe_end);
                        result.snippet.push_str("...");
                    }
                }
            }
        } else {
            // No summarizer, just truncate hard
            for result in scraping_result.iter_mut() {
                if result.snippet.len() > chars_per_slot as usize {
                    let mut safe_end = chars_per_slot as usize;
                    while !result.snippet.is_char_boundary(safe_end) {
                        safe_end -= 1;
                    }
                    result.snippet.truncate(safe_end);
                    result.snippet.push_str("...");
                }
                }
        }


        // Sort results by their aggregate score (descending)
        scraping_result.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Emit generating status (instead of done)
        if let Some(app) = &self.app {
            use tauri::Emitter;
            #[derive(Serialize, Clone, specta::Type)]
            struct WebSearchStatus {
                id: Option<String>,
                step: String,
                message: String,
            }
            let _ = app.emit(
                "web_search_status",
                WebSearchStatus {
                    id: self.conversation_id.clone(),
                    step: "generating".into(),
                    message: "Formulating response...".into(),
                },
            );
            // Emit final results with summaries for UI and Persistence
            let _ = app.emit("web_search_results", json!({
                "id": self.conversation_id,
                "results": &scraping_result
            }));
        }

        Ok(generate_tool_result_json(&args.query, &scraping_result))
    }
}

fn generate_tool_result_json(query: &str, results: &[SearchResult]) -> String {
    use crate::rig_lib::tools::models::{Citation, ToolResult};

    if results.is_empty() {
        return serde_json::to_string(&ToolResult::error("No results found".into())).unwrap_or_default();
    }

    let citations: Vec<Citation> = results.iter().map(|r| Citation {
        source_id: r.link.clone(),
        title: r.title.clone(),
        loc: Some(r.link.clone()),
        confidence: r.score,
    }).collect();

    let summary = format!("Found {} results for '{}'", results.len(), query);

    let result = ToolResult {
        ok: true,
        summary,
        data: serde_json::json!(results),
        citations,
        artifacts: vec![],
        timings_ms: None,
    };

    serde_json::to_string(&result).unwrap_or_else(|_| "{\"error\":\"Serialization failed\"}".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ddg_search_with_scraping() {
        let tool = DDGSearchTool {
            app: None,
            max_total_chars: 1000,
            summarizer: None,
            conversation_id: None,
        };
        let args = SearchArgs {
            query: "rust lang".to_string(),
        };
        // This call should trigger the scraping loop
        let result = tool.call(args).await;
        match result {
            Ok(res) => {
                println!("Full Output Length: {}", res.len());
                // We verify that the "Deep Scraped" header is present (indicating the new logic ran)
                assert!(res.contains("## Search Results (Deep Scraped)"));

                // We cannot strictly assert content scraping in CI/Test without a browser,
                // but we can verify the fallback mechanism worked if scraping failed.
                if !res.contains("**Scraped Content**:") {
                    println!("WARNING: Scraping markers not found. This is expected if no Chromium binary is available in the test environment.");
                } else {
                    println!("Scraping verified successfully.");
                }
            }
            Err(e) => panic!("Search failed: {}", e),
        }
    }
}
