//! Cloud browser provider adapters for the CDP-based browser tool.
//!
//! Browserbase provisions explicit cloud sessions over a REST API and returns a
//! connect URL for CDP. Browser Use exposes a direct WebSocket CDP URL where
//! session parameters are passed as query parameters and the browser stops when
//! the connection closes.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;
use url::Url;

use crate::tools::tool::ToolError;

pub const DEFAULT_CLOUD_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudBrowserProviderKind {
    Browserbase,
    BrowserUse,
}

impl CloudBrowserProviderKind {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "browserbase" => Some(Self::Browserbase),
            "browser-use" | "browser_use" | "browseruse" => Some(Self::BrowserUse),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CloudBrowserSession {
    pub id: Option<String>,
    pub connect_url: String,
    pub provider: CloudBrowserProviderKind,
    pub label: String,
}

#[async_trait]
pub trait CloudBrowserProvider: Send + Sync {
    fn kind(&self) -> CloudBrowserProviderKind;
    fn label(&self) -> &'static str;

    async fn create_session(&self) -> Result<CloudBrowserSession, ToolError>;

    async fn close_session(&self, session: &CloudBrowserSession) -> Result<(), ToolError>;
}

pub fn build_provider(
    configured: Option<&str>,
) -> Result<Option<Box<dyn CloudBrowserProvider>>, ToolError> {
    let requested = configured
        .and_then(CloudBrowserProviderKind::from_name)
        .or_else(infer_provider_from_env);

    match requested {
        Some(CloudBrowserProviderKind::Browserbase) => Ok(Some(Box::new(
            BrowserbaseProvider::from_env().map_err(ToolError::ExecutionFailed)?,
        ))),
        Some(CloudBrowserProviderKind::BrowserUse) => Ok(Some(Box::new(
            BrowserUseProvider::from_env().map_err(ToolError::ExecutionFailed)?,
        ))),
        None => Ok(None),
    }
}

fn infer_provider_from_env() -> Option<CloudBrowserProviderKind> {
    if env_present("BROWSERBASE_API_KEY") && env_present("BROWSERBASE_PROJECT_ID") {
        Some(CloudBrowserProviderKind::Browserbase)
    } else if env_present("BROWSER_USE_API_KEY") {
        Some(CloudBrowserProviderKind::BrowserUse)
    } else {
        None
    }
}

fn env_present(key: &str) -> bool {
    std::env::var(key).is_ok_and(|value| !value.trim().is_empty())
}

fn env_required(key: &str) -> Result<String, String> {
    std::env::var(key)
        .map_err(|_| format!("{} is not set", key))
        .and_then(|value| {
            if value.trim().is_empty() {
                Err(format!("{} is empty", key))
            } else {
                Ok(value)
            }
        })
}

struct BrowserbaseProvider {
    client: reqwest::Client,
    project_id: String,
    region: Option<String>,
    proxy_country_code: Option<String>,
}

impl BrowserbaseProvider {
    fn from_env() -> Result<Self, String> {
        let api_key = env_required("BROWSERBASE_API_KEY")?;
        let project_id = env_required("BROWSERBASE_PROJECT_ID")?;

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-bb-api-key",
            HeaderValue::from_str(&api_key)
                .map_err(|error| format!("Invalid BROWSERBASE_API_KEY header: {error}"))?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| format!("Failed to build Browserbase client: {error}"))?;

        Ok(Self {
            client,
            project_id,
            region: std::env::var("BROWSERBASE_REGION").ok(),
            proxy_country_code: std::env::var("BROWSERBASE_PROXY_COUNTRY_CODE").ok(),
        })
    }
}

#[derive(Debug, Deserialize)]
struct BrowserbaseSessionResponse {
    id: String,
    #[serde(rename = "connectUrl")]
    connect_url: String,
}

#[async_trait]
impl CloudBrowserProvider for BrowserbaseProvider {
    fn kind(&self) -> CloudBrowserProviderKind {
        CloudBrowserProviderKind::Browserbase
    }

    fn label(&self) -> &'static str {
        "browserbase"
    }

    async fn create_session(&self) -> Result<CloudBrowserSession, ToolError> {
        let mut payload = serde_json::json!({
            "projectId": self.project_id,
            "keepAlive": true,
        });

        if let Some(ref region) = self.region
            && let Some(map) = payload.as_object_mut()
        {
            map.insert(
                "region".to_string(),
                serde_json::Value::String(region.clone()),
            );
        }

        if let Some(ref proxy_country_code) = self.proxy_country_code
            && let Some(map) = payload.as_object_mut()
        {
            map.insert(
                "proxyCountryCode".to_string(),
                serde_json::Value::String(proxy_country_code.clone()),
            );
        }

        let response = self
            .client
            .post("https://api.browserbase.com/v1/sessions")
            .json(&payload)
            .send()
            .await
            .map_err(|error| {
                ToolError::ExecutionFailed(format!("Failed to create Browserbase session: {error}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!(
                "Browserbase session creation failed with HTTP {}: {}",
                status,
                body.trim()
            )));
        }

        let session: BrowserbaseSessionResponse = response.json().await.map_err(|error| {
            ToolError::ExecutionFailed(format!(
                "Failed to decode Browserbase session response: {error}"
            ))
        })?;

        Ok(CloudBrowserSession {
            id: Some(session.id.clone()),
            connect_url: session.connect_url,
            provider: self.kind(),
            label: format!("Browserbase session {}", session.id),
        })
    }

    async fn close_session(&self, session: &CloudBrowserSession) -> Result<(), ToolError> {
        let Some(ref session_id) = session.id else {
            return Ok(());
        };

        let response = self
            .client
            .post(format!(
                "https://api.browserbase.com/v1/sessions/{session_id}"
            ))
            .json(&serde_json::json!({
                "projectId": self.project_id,
                "status": "REQUEST_RELEASE",
            }))
            .send()
            .await
            .map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "Failed to close Browserbase session {session_id}: {error}"
                ))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!(
                "Browserbase session close failed with HTTP {}: {}",
                status,
                body.trim()
            )));
        }

        Ok(())
    }
}

struct BrowserUseProvider {
    api_key: String,
    timeout_minutes: Option<String>,
    profile_id: Option<String>,
    proxy_country_code: Option<String>,
}

impl BrowserUseProvider {
    fn from_env() -> Result<Self, String> {
        Ok(Self {
            api_key: env_required("BROWSER_USE_API_KEY")?,
            timeout_minutes: std::env::var("BROWSER_USE_TIMEOUT").ok(),
            profile_id: std::env::var("BROWSER_USE_PROFILE_ID").ok(),
            proxy_country_code: std::env::var("BROWSER_USE_PROXY_COUNTRY_CODE").ok(),
        })
    }
}

#[async_trait]
impl CloudBrowserProvider for BrowserUseProvider {
    fn kind(&self) -> CloudBrowserProviderKind {
        CloudBrowserProviderKind::BrowserUse
    }

    fn label(&self) -> &'static str {
        "browser_use"
    }

    async fn create_session(&self) -> Result<CloudBrowserSession, ToolError> {
        let mut url = Url::parse("wss://connect.browser-use.com").map_err(|error| {
            ToolError::ExecutionFailed(format!("Invalid Browser Use endpoint: {error}"))
        })?;

        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("apiKey", &self.api_key);
            if let Some(ref timeout) = self.timeout_minutes {
                pairs.append_pair("timeout", timeout);
            }
            if let Some(ref profile_id) = self.profile_id {
                pairs.append_pair("profileId", profile_id);
            }
            if let Some(ref proxy_country_code) = self.proxy_country_code {
                pairs.append_pair("proxyCountryCode", proxy_country_code);
            }
        }

        Ok(CloudBrowserSession {
            id: None,
            connect_url: url.to_string(),
            provider: self.kind(),
            label: "Browser Use cloud browser".to_string(),
        })
    }

    async fn close_session(&self, _session: &CloudBrowserSession) -> Result<(), ToolError> {
        // Browser Use cloud sessions stop when the WebSocket disconnects.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_name_detection_handles_aliases() {
        assert_eq!(
            CloudBrowserProviderKind::from_name("browser_use"),
            Some(CloudBrowserProviderKind::BrowserUse)
        );
        assert_eq!(
            CloudBrowserProviderKind::from_name("browserbase"),
            Some(CloudBrowserProviderKind::Browserbase)
        );
        assert_eq!(CloudBrowserProviderKind::from_name("unknown"), None);
    }

    #[test]
    fn browser_use_session_url_includes_query_params() {
        let provider = BrowserUseProvider {
            api_key: "test-key".to_string(),
            timeout_minutes: Some("15".to_string()),
            profile_id: Some("profile_123".to_string()),
            proxy_country_code: Some("de".to_string()),
        };

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let session = runtime.block_on(provider.create_session()).unwrap();
        assert!(session.connect_url.contains("apiKey=test-key"));
        assert!(session.connect_url.contains("timeout=15"));
        assert!(session.connect_url.contains("profileId=profile_123"));
        assert!(session.connect_url.contains("proxyCountryCode=de"));
    }

    #[test]
    fn builder_returns_none_without_configuration() {
        let provider = build_provider(Some("definitely-not-real")).unwrap();
        assert!(provider.is_none());
    }
}
