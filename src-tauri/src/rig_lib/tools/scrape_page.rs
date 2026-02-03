use chromiumoxide::{Browser, BrowserConfig};
use futures::StreamExt;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScrapeError {
    #[error("Chromium error: {0}")]
    Chromium(String), // map chromiumoxide error to string deeply if needed, or implement From
    #[error("Request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Other: {0}")]
    Other(String),
}

// Implement From for chromiumoxide::CdpError
impl From<chromiumoxide::error::CdpError> for ScrapeError {
    fn from(e: chromiumoxide::error::CdpError) -> Self {
        ScrapeError::Chromium(e.to_string())
    }
}

#[derive(Deserialize)]
pub struct ScrapePageArgs {
    pub url: String,
    pub javascript_enabled: Option<bool>,
}

pub struct ScrapePageTool {
    pub app: std::sync::Mutex<Option<tauri::AppHandle>>,
}

impl Tool for ScrapePageTool {
    const NAME: &'static str = "scrape_page";

    type Error = ScrapeError;
    type Args = ScrapePageArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "scrape_page".to_string(),
            description: "A tool to scrape the content of a web page. Use this when you need detailed information from a specific URL.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL of the page to scrape"
                    },
                    "javascript_enabled": {
                        "type": "boolean",
                        "description": "Whether to use a headless browser to render JavaScript (default: true). Set to false for simple static pages for faster speed."
                    }
                },
                "required": ["url"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let use_js = args.javascript_enabled.unwrap_or(true);

        if let Ok(app_guard) = self.app.lock() {
            if let Some(app) = app_guard.as_ref() {
                use tauri::Emitter;
                #[derive(Serialize, Clone, specta::Type)]
                struct WebSearchStatus {
                    step: String,
                    message: String,
                }
                let _ = app.emit(
                    "web_search_status",
                    WebSearchStatus {
                        step: "browsing".into(),
                        message: format!("Visiting page: {}", args.url),
                    },
                );
            }
        }

        if use_js {
            // scrape_with_browser already returns clean text via scrape_url
            self.scrape_with_browser(&args.url).await
        } else {
            let html = reqwest::get(&args.url).await?.text().await?;
            Self::extract_smart_text(&html)
        }
    }
}

impl ScrapePageTool {
    pub fn extract_smart_text(html: &str) -> Result<String, ScrapeError> {
        let document = Html::parse_document(html);
        let selectors = vec!["main", "article", "#content", "#main", ".main-content"];

        let mut target_html = html;
        let selected_html_string;

        for sel in selectors {
            if let Ok(selector) = Selector::parse(sel) {
                if let Some(element) = document.select(&selector).next() {
                    selected_html_string = element.inner_html();
                    target_html = &selected_html_string;
                    break;
                }
            }
        }

        html2text::from_read(target_html.as_bytes(), 120)
            .map_err(|e| ScrapeError::Other(e.to_string()))
    }

    pub async fn launch_browser(
        &self,
    ) -> Result<(Browser, tokio::task::JoinHandle<()>), ScrapeError> {
        let app_handle = self.app.lock().map(|g| g.clone()).unwrap_or(None);

        use crate::rig_lib::chromium_resolver::ensure_chromium;
        // Ensure proper browser binary is available
        let exec_path = ensure_chromium(app_handle.as_ref())
            .await
            .map_err(ScrapeError::Other)?;

        // Config with unique user data dir implicitly handled by chromiumoxide usually,
        // but we can trust a single instance to be fine.
        let config = BrowserConfig::builder()
            .chrome_executable(exec_path)
            .arg("--disable-dev-shm-usage")
            .arg("--disable-extensions")
            .arg("--no-sandbox")
            .arg("--disable-gpu")
            .arg("--disable-setuid-sandbox")
            .arg("--password-store=basic")
            .viewport(None)
            .build()
            .map_err(ScrapeError::Other)?;

        let (browser, mut handler) = Browser::launch(config).await.map_err(ScrapeError::from)?;

        let handler_handle = tokio::task::spawn(async move {
            while let Some(h) = handler.next().await {
                if let Err(e) = h {
                    eprintln!("Chromium handler error: {}", e);
                    // Do not break on deserialization errors, just continue
                }
            }
        });

        Ok((browser, handler_handle))
    }

    pub async fn scrape_url(&self, browser: &Browser, url: &str) -> Result<String, ScrapeError> {
        let scrape_logic = async {
            let page = browser.new_page(url).await.map_err(ScrapeError::from)?;

            page.wait_for_navigation()
                .await
                .map_err(ScrapeError::from)?;

            let content = page.content().await.map_err(ScrapeError::from)?;
            page.close().await.map_err(ScrapeError::from)?;

            Self::extract_smart_text(&content)
        };

        match tokio::time::timeout(std::time::Duration::from_secs(8), scrape_logic).await {
            Ok(res) => res,
            Err(_) => {
                println!("[scrape] Timeout for url: {}", url);
                Err(ScrapeError::Other("Timeout".to_string()))
            }
        }
    }

    pub async fn scrape_with_browser(&self, url: &str) -> Result<String, ScrapeError> {
        // Hybrid Fetching: Try simple GET first
        if let Ok(resp) = reqwest::get(url).await {
            if let Ok(text) = resp.text().await {
                // Heuristic: If text is substantial and doesn't look like a JS loader
                if text.len() > 500
                    && !text.contains("You need to enable JavaScript")
                    && !text.contains("Please enable JS")
                {
                    if let Ok(clean) = Self::extract_smart_text(&text) {
                        if clean.len() > 200 {
                            println!("[scrape] Fast path success for: {}", url);
                            return Ok(clean);
                        }
                    }
                }
            }
        }

        println!(
            "[scrape] Fast path failed/insufficient, using browser for: {}",
            url
        );

        let app_handle = self.app.lock().map(|g| g.clone()).unwrap_or(None);
        let url = url.to_string();

        let result = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| ScrapeError::Other(e.to_string()))?;

            rt.block_on(async {
                let scraper = ScrapePageTool {
                    app: std::sync::Mutex::new(app_handle),
                };
                let (mut browser, handler) = scraper.launch_browser().await?;
                let res = scraper.scrape_url(&browser, &url).await;
                let _ = browser.close().await;
                let _ = handler.await;
                res
            })
        })
        .await
        .map_err(|e| ScrapeError::Other(e.to_string()))??;

        Ok(result)
    }
}
