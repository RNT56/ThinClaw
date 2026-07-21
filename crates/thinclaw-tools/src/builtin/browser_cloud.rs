//! Cloud browser provider adapters for the CDP-based browser tool.
//!
//! Browserbase provisions explicit cloud sessions over a REST API and returns a
//! connect URL for CDP. Browser Use exposes a direct WebSocket CDP URL where
//! session parameters are passed as query parameters and the browser stops when
//! the connection closes.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;
use url::Url;

use thinclaw_tools_core::ToolError;

pub const DEFAULT_CLOUD_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAX_CLOUD_CONNECT_URL_BYTES: usize = 8 * 1024;
const MAX_CLOUD_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_CLOUD_IDENTIFIER_BYTES: usize = 256;
const MAX_CLOUD_SECRET_BYTES: usize = 4 * 1024;

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

#[derive(Clone)]
pub struct CloudBrowserSession {
    pub id: Option<String>,
    pub connect_url: String,
    pub provider: CloudBrowserProviderKind,
    pub label: String,
}

impl CloudBrowserSession {
    pub fn endpoint_label(&self) -> String {
        redacted_endpoint(&self.connect_url)
    }
}

impl std::fmt::Debug for CloudBrowserSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CloudBrowserSession")
            .field("id", &self.id.as_ref().map(|_| "[REDACTED]"))
            .field("connect_url", &self.endpoint_label())
            .field("provider", &self.provider)
            .field("label", &self.label)
            .finish()
    }
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
) -> Result<Option<Arc<dyn CloudBrowserProvider>>, ToolError> {
    let requested = configured
        .and_then(CloudBrowserProviderKind::from_name)
        .or_else(infer_provider_from_env);

    match requested {
        Some(CloudBrowserProviderKind::Browserbase) => Ok(Some(Arc::new(
            BrowserbaseProvider::from_env().map_err(ToolError::ExecutionFailed)?,
        ))),
        Some(CloudBrowserProviderKind::BrowserUse) => Ok(Some(Arc::new(
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
            if value.trim().is_empty()
                || value.len() > MAX_CLOUD_SECRET_BYTES
                || value.chars().any(char::is_control)
            {
                Err(format!("{} is empty, oversized, or malformed", key))
            } else {
                Ok(value)
            }
        })
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CLOUD_IDENTIFIER_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn redacted_endpoint(raw: &str) -> String {
    let Ok(url) = Url::parse(raw) else {
        return "[invalid cloud browser endpoint]".to_string();
    };
    let host = url.host_str().unwrap_or("unknown");
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

fn validate_cloud_connect_url(
    raw: &str,
    provider: CloudBrowserProviderKind,
) -> Result<(), ToolError> {
    if raw.is_empty() || raw.len() > MAX_CLOUD_CONNECT_URL_BYTES {
        return Err(ToolError::ExecutionFailed(
            "Cloud browser returned an empty or oversized CDP URL".to_string(),
        ));
    }
    let url = Url::parse(raw).map_err(|_| {
        ToolError::ExecutionFailed("Cloud browser returned an invalid CDP URL".to_string())
    })?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let allowed_host = match provider {
        CloudBrowserProviderKind::Browserbase => {
            host == "browserbase.com" || host.ends_with(".browserbase.com")
        }
        CloudBrowserProviderKind::BrowserUse => host == "connect.browser-use.com",
    };
    if url.scheme() != "wss"
        || !allowed_host
        || url.port().is_some_and(|port| port != 443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(ToolError::ExecutionFailed(
            "Cloud browser returned a CDP URL outside its trusted WSS origin".to_string(),
        ));
    }
    Ok(())
}

async fn bounded_response_body(response: reqwest::Response) -> Result<Vec<u8>, ToolError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            ToolError::ExecutionFailed(format!(
                "Failed reading cloud browser response: {}",
                error.without_url()
            ))
        })?;
        if body.len().saturating_add(chunk.len()) > MAX_CLOUD_RESPONSE_BYTES {
            return Err(ToolError::ExecutionFailed(format!(
                "Cloud browser response exceeded {MAX_CLOUD_RESPONSE_BYTES} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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
        if !valid_identifier(&project_id) {
            return Err("BROWSERBASE_PROJECT_ID is malformed".to_string());
        }
        let region = std::env::var("BROWSERBASE_REGION").ok();
        if region.as_deref().is_some_and(|region| {
            !matches!(
                region,
                "us-west-2" | "us-east-1" | "eu-central-1" | "ap-southeast-1"
            )
        }) {
            return Err("BROWSERBASE_REGION is unsupported".to_string());
        }
        let proxy_country_code = std::env::var("BROWSERBASE_PROXY_COUNTRY_CODE").ok();
        if proxy_country_code.as_deref().is_some_and(|country| {
            country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_alphabetic())
        }) {
            return Err("BROWSERBASE_PROXY_COUNTRY_CODE is malformed".to_string());
        }

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
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|error| format!("Failed to build Browserbase client: {error}"))?;

        Ok(Self {
            client,
            project_id,
            region,
            proxy_country_code,
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
                ToolError::ExecutionFailed(format!(
                    "Failed to create Browserbase session: {}",
                    error.without_url()
                ))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(ToolError::ExecutionFailed(format!(
                "Browserbase session creation failed with HTTP {status}"
            )));
        }

        let body = bounded_response_body(response).await?;
        let session: BrowserbaseSessionResponse =
            serde_json::from_slice(&body).map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "Failed to decode Browserbase session response: {error}"
                ))
            })?;
        if !valid_identifier(&session.id) {
            return Err(ToolError::ExecutionFailed(
                "Browserbase returned an invalid session identifier".to_string(),
            ));
        }
        validate_cloud_connect_url(&session.connect_url, self.kind())?;

        Ok(CloudBrowserSession {
            id: Some(session.id.clone()),
            connect_url: session.connect_url,
            provider: self.kind(),
            label: "Browserbase cloud browser".to_string(),
        })
    }

    async fn close_session(&self, session: &CloudBrowserSession) -> Result<(), ToolError> {
        let Some(ref session_id) = session.id else {
            return Ok(());
        };
        if !valid_identifier(session_id) {
            return Err(ToolError::ExecutionFailed(
                "Browserbase session identifier is invalid".to_string(),
            ));
        }

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
                    "Failed to close Browserbase session: {}",
                    error.without_url()
                ))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(ToolError::ExecutionFailed(format!(
                "Browserbase session close failed with HTTP {status}"
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
        let timeout_minutes = std::env::var("BROWSER_USE_TIMEOUT").ok();
        if timeout_minutes.as_deref().is_some_and(|timeout| {
            timeout
                .parse::<u16>()
                .map_or(true, |minutes| !(1..=240).contains(&minutes))
        }) {
            return Err("BROWSER_USE_TIMEOUT must be between 1 and 240 minutes".to_string());
        }
        let profile_id = std::env::var("BROWSER_USE_PROFILE_ID").ok();
        if profile_id
            .as_deref()
            .is_some_and(|profile| !valid_identifier(profile))
        {
            return Err("BROWSER_USE_PROFILE_ID is malformed".to_string());
        }
        let proxy_country_code = std::env::var("BROWSER_USE_PROXY_COUNTRY_CODE").ok();
        if proxy_country_code.as_deref().is_some_and(|country| {
            country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_alphabetic())
        }) {
            return Err("BROWSER_USE_PROXY_COUNTRY_CODE is malformed".to_string());
        }
        Ok(Self {
            api_key: env_required("BROWSER_USE_API_KEY")?,
            timeout_minutes,
            profile_id,
            proxy_country_code,
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

        let connect_url = url.to_string();
        validate_cloud_connect_url(&connect_url, self.kind())?;
        Ok(CloudBrowserSession {
            id: None,
            connect_url,
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

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let session = runtime.block_on(provider.create_session()).unwrap();
        assert!(session.connect_url.contains("apiKey=test-key"));
        assert!(session.connect_url.contains("timeout=15"));
        assert!(session.connect_url.contains("profileId=profile_123"));
        assert!(session.connect_url.contains("proxyCountryCode=de"));
        let debug = format!("{session:?}");
        assert!(!debug.contains("test-key"));
        assert!(!debug.contains("apiKey"));
        assert_eq!(session.endpoint_label(), "wss://connect.browser-use.com");
    }

    #[test]
    fn cloud_session_debug_redacts_query_secrets() {
        let session = CloudBrowserSession {
            id: Some("session-1".to_string()),
            connect_url:
                "wss://connect.browserbase.com/devtools?token=do-not-log-this&project=secret"
                    .to_string(),
            provider: CloudBrowserProviderKind::Browserbase,
            label: "test session".to_string(),
        };

        let debug = format!("{session:?}");
        assert!(debug.contains("wss://connect.browserbase.com"));
        assert!(!debug.contains("do-not-log-this"));
        assert!(!debug.contains("project=secret"));
    }

    #[test]
    fn cloud_connect_urls_are_restricted_to_provider_wss_origins() {
        assert!(
            validate_cloud_connect_url(
                "wss://connect.browserbase.com/devtools?token=secret",
                CloudBrowserProviderKind::Browserbase,
            )
            .is_ok()
        );
        assert!(
            validate_cloud_connect_url(
                "wss://connect.browser-use.com?apiKey=secret",
                CloudBrowserProviderKind::BrowserUse,
            )
            .is_ok()
        );

        for (invalid, provider) in [
            (
                "ws://connect.browser-use.com?apiKey=secret",
                CloudBrowserProviderKind::BrowserUse,
            ),
            (
                "wss://connect.browser-use.com.evil.test?apiKey=secret",
                CloudBrowserProviderKind::BrowserUse,
            ),
            (
                "wss://connect.browserbase.com:444/devtools",
                CloudBrowserProviderKind::Browserbase,
            ),
            (
                "wss://user:password@connect.browserbase.com/devtools",
                CloudBrowserProviderKind::Browserbase,
            ),
            (
                "wss://connect.browserbase.com/devtools#secret",
                CloudBrowserProviderKind::Browserbase,
            ),
            (
                "wss://browserbase.com.evil.test/devtools",
                CloudBrowserProviderKind::Browserbase,
            ),
        ] {
            assert!(
                validate_cloud_connect_url(invalid, provider).is_err(),
                "unexpectedly trusted {invalid}"
            );
        }
    }

    #[test]
    fn builder_returns_none_without_configuration() {
        let provider = build_provider(Some("definitely-not-real")).unwrap();
        assert!(provider.is_none());
    }
}
