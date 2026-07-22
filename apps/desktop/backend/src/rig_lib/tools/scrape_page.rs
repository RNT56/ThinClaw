use futures::StreamExt;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScrapeError {
    #[error("Serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Other: {0}")]
    Other(String),
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
            description: "Fetch and extract readable text from a public web page. Use this when you need detailed information from a specific URL.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL of the page to scrape"
                    },
                    "javascript_enabled": {
                        "type": "boolean",
                        "description": "Retained for compatibility. Fetching is isolated and does not execute page JavaScript."
                    }
                },
                "required": ["url"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let _javascript_requested = args.javascript_enabled.unwrap_or(true);
        let display_url = Self::display_url(&args.url);

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
                        message: format!("Visiting page: {display_url}"),
                    },
                );
            }
        }

        let html = Self::fetch_public_page(&args.url).await?;
        Self::extract_smart_text(&html)
    }
}

impl ScrapePageTool {
    const MAX_PAGE_BYTES: u64 = 4 * 1024 * 1024;
    const MAX_EXTRACTED_BYTES: usize = 2 * 1024 * 1024;
    const MAX_REDIRECTS: usize = 5;

    fn display_url(raw: &str) -> String {
        reqwest::Url::parse(raw)
            .ok()
            .and_then(|mut url| {
                url.set_query(None);
                url.set_fragment(None);
                if !url.username().is_empty() || url.password().is_some() {
                    return None;
                }
                Some(url.to_string())
            })
            .unwrap_or_else(|| "invalid URL".to_string())
    }

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

        let mut text = html2text::from_read(target_html.as_bytes(), 120)
            .map_err(|_| ScrapeError::Other("Could not extract page text".to_string()))?;
        if text.len() > Self::MAX_EXTRACTED_BYTES {
            let mut end = Self::MAX_EXTRACTED_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        Ok(text)
    }

    pub async fn fetch_public_page(raw_url: &str) -> Result<String, ScrapeError> {
        let options = thinclaw_tools_core::OutboundUrlGuardOptions {
            require_https: true,
            upgrade_http_to_https: true,
            allowlist: Vec::new(),
        };
        let mut current = raw_url.to_string();
        for redirect_count in 0..=Self::MAX_REDIRECTS {
            let guarded =
                thinclaw_tools_core::validate_outbound_url_pinned_async(&current, &options)
                    .await
                    .map_err(|_| {
                        ScrapeError::Other("Page URL is not a public HTTPS destination".into())
                    })?;
            let host = guarded
                .url
                .host_str()
                .ok_or_else(|| ScrapeError::Other("Page URL has no host".into()))?
                .to_string();
            let mut builder = reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(std::time::Duration::from_secs(10))
                .read_timeout(std::time::Duration::from_secs(15))
                .timeout(std::time::Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("ThinClawDesktop/0.14");
            if !guarded.pinned_addrs.is_empty() {
                builder = builder.resolve_to_addrs(&host, &guarded.pinned_addrs);
            }
            let client = builder
                .build()
                .map_err(|_| ScrapeError::Other("Could not create page client".into()))?;
            let response = client
                .get(guarded.url.clone())
                .send()
                .await
                .map_err(|error| {
                    ScrapeError::Other(crate::rig_lib::http::transport_error(
                        "Page request failed",
                        error,
                    ))
                })?;
            if response.status().is_redirection() {
                if redirect_count == Self::MAX_REDIRECTS {
                    return Err(ScrapeError::Other("Page redirected too many times".into()));
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| ScrapeError::Other("Page redirect was malformed".into()))?;
                current = guarded
                    .url
                    .join(location)
                    .map_err(|_| ScrapeError::Other("Page redirect URL was invalid".into()))?
                    .to_string();
                continue;
            }
            if !response.status().is_success() {
                return Err(ScrapeError::Other(format!(
                    "Page request failed with HTTP {}",
                    response.status()
                )));
            }
            if response
                .content_length()
                .is_some_and(|length| length > Self::MAX_PAGE_BYTES)
            {
                return Err(ScrapeError::Other("Page exceeds the download limit".into()));
            }
            if response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| {
                    let mime = value.split(';').next().unwrap_or_default().trim();
                    !matches!(mime, "text/html" | "application/xhtml+xml" | "text/plain")
                })
            {
                return Err(ScrapeError::Other(
                    "Page response is not readable text or HTML".into(),
                ));
            }
            let mut bytes = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| {
                    ScrapeError::Other(crate::rig_lib::http::transport_error(
                        "Page response failed",
                        error,
                    ))
                })?;
                if bytes.len().saturating_add(chunk.len())
                    > usize::try_from(Self::MAX_PAGE_BYTES).unwrap_or(usize::MAX)
                {
                    return Err(ScrapeError::Other("Page exceeds the download limit".into()));
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok(String::from_utf8_lossy(&bytes).into_owned());
        }
        Err(ScrapeError::Other("Page fetch did not complete".into()))
    }
}
