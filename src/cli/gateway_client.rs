//! Hardened authenticated HTTP client shared by CLI gateway consumers.

use std::fmt;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;

use crate::settings::Settings;

pub const MAX_GATEWAY_CONTROL_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayRequestBudget {
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
    pub max_response_bytes: usize,
}

impl GatewayRequestBudget {
    pub const fn control_plane() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            total_timeout: Duration::from_secs(30),
            max_response_bytes: MAX_GATEWAY_CONTROL_RESPONSE_BYTES,
        }
    }
}

/// Opaque bearer value accepted only by the HTTP adapter.
pub struct GatewayAuthToken(SecretString);

impl GatewayAuthToken {
    pub fn new(value: String) -> Result<Self, GatewayClientError> {
        let value = value.trim().to_string();
        if value.is_empty() || value.len() > 16 * 1024 {
            return Err(GatewayClientError::InvalidToken);
        }
        Ok(Self(SecretString::from(value)))
    }
}

impl fmt::Debug for GatewayAuthToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GatewayAuthToken([REDACTED])")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayClientError {
    #[error("gateway origin is invalid: {0}")]
    InvalidOrigin(String),
    #[error("gateway authentication token is invalid")]
    InvalidToken,
    #[error("failed to construct the gateway HTTP client: {0}")]
    ClientBuild(String),
    #[error("gateway request failed: {0}")]
    Transport(String),
    #[error("gateway returned HTTP {status}: {message}")]
    Api { status: u16, message: String },
    #[error("gateway response exceeded the {limit}-byte limit")]
    ResponseTooLarge { limit: usize },
    #[error("gateway response was malformed: {0}")]
    MalformedResponse(String),
    #[error("gateway request path is invalid")]
    InvalidPath,
}

pub struct GatewayClient {
    origin: Url,
    token: Option<GatewayAuthToken>,
    client: reqwest::Client,
    budget: GatewayRequestBudget,
}

impl fmt::Debug for GatewayClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GatewayClient")
            .field("origin", &self.origin.as_str())
            .field("authenticated", &self.token.is_some())
            .field("budget", &self.budget)
            .finish()
    }
}

impl GatewayClient {
    pub fn resolve(
        explicit_origin: Option<&str>,
        explicit_token: Option<GatewayAuthToken>,
        settings: Option<&Settings>,
    ) -> Result<Self, GatewayClientError> {
        let origin = explicit_origin
            .map(str::to_string)
            .or_else(|| std::env::var("THINCLAW_GATEWAY_URL").ok())
            .unwrap_or_else(|| {
                let host = std::env::var("GATEWAY_HOST").ok().or_else(|| {
                    settings.and_then(|settings| settings.channels.gateway_host.clone())
                });
                let port = std::env::var("GATEWAY_PORT")
                    .ok()
                    .and_then(|value| value.parse::<u16>().ok())
                    .or_else(|| settings.and_then(|settings| settings.channels.gateway_port))
                    .unwrap_or(3000);
                format!(
                    "http://{}:{port}",
                    host.unwrap_or_else(|| "127.0.0.1".into())
                )
            });
        let origin = validate_origin(&origin)?;

        let token = match explicit_token {
            Some(token) => Some(token),
            None => std::env::var("GATEWAY_AUTH_TOKEN")
                .ok()
                .or_else(|| {
                    settings.and_then(|settings| settings.channels.gateway_auth_token.clone())
                })
                .map(GatewayAuthToken::new)
                .transpose()?,
        };
        Self::new(origin, token, GatewayRequestBudget::control_plane())
    }

    /// Resolve against the fully hydrated root configuration used by runtime
    /// startup. Environment values remain explicit compatibility overrides.
    pub fn resolve_from_config(
        explicit_origin: Option<&str>,
        explicit_token: Option<GatewayAuthToken>,
        config: &crate::config::Config,
    ) -> Result<Self, GatewayClientError> {
        let gateway = config.channels.gateway.as_ref();
        let origin = explicit_origin
            .map(str::to_string)
            .or_else(|| std::env::var("THINCLAW_GATEWAY_URL").ok())
            .unwrap_or_else(|| {
                let host = std::env::var("GATEWAY_HOST")
                    .ok()
                    .or_else(|| gateway.map(|gateway| gateway.host.clone()))
                    .unwrap_or_else(|| "127.0.0.1".into());
                let port = std::env::var("GATEWAY_PORT")
                    .ok()
                    .and_then(|value| value.parse::<u16>().ok())
                    .or_else(|| gateway.map(|gateway| gateway.port))
                    .unwrap_or(3000);
                format!("http://{host}:{port}")
            });
        let origin = validate_origin(&origin)?;
        let token = match explicit_token {
            Some(token) => Some(token),
            None => std::env::var("GATEWAY_AUTH_TOKEN")
                .ok()
                .or_else(|| gateway.and_then(|gateway| gateway.auth_token.clone()))
                .map(GatewayAuthToken::new)
                .transpose()?,
        };
        Self::new(origin, token, GatewayRequestBudget::control_plane())
    }

    pub fn new(
        origin: Url,
        token: Option<GatewayAuthToken>,
        budget: GatewayRequestBudget,
    ) -> Result<Self, GatewayClientError> {
        let client = reqwest::Client::builder()
            .connect_timeout(budget.connect_timeout)
            .timeout(budget.total_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|error| GatewayClientError::ClientBuild(error.to_string()))?;
        Ok(Self {
            origin,
            token,
            client,
            budget,
        })
    }

    pub fn credential_free_origin(&self) -> &str {
        self.origin.as_str()
    }

    pub async fn post_json<Request, Response>(
        &self,
        path: &str,
        body: &Request,
    ) -> Result<Response, GatewayClientError>
    where
        Request: Serialize + ?Sized,
        Response: DeserializeOwned,
    {
        let url = self.join_path(path)?;
        let mut request = self.client.post(url).json(body);
        if let Some(token) = self.token.as_ref() {
            request = request.bearer_auth(token.0.expose_secret());
        }
        let response = request
            .send()
            .await
            .map_err(|error| GatewayClientError::Transport(error.without_url().to_string()))?;
        let status = response.status();
        let bytes = bounded_response_bytes(response, self.budget.max_response_bytes).await?;
        if !status.is_success() {
            return Err(GatewayClientError::Api {
                status: status.as_u16(),
                message: safe_api_message(&bytes),
            });
        }
        serde_json::from_slice(&bytes)
            .map_err(|error| GatewayClientError::MalformedResponse(error.to_string()))
    }

    /// POST a JSON mutation with the gateway's explicit confirmation header.
    pub async fn post_json_confirmed<Request, Response>(
        &self,
        path: &str,
        body: &Request,
    ) -> Result<Response, GatewayClientError>
    where
        Request: Serialize + ?Sized,
        Response: DeserializeOwned,
    {
        let url = self.join_path(path)?;
        let mut request = self
            .client
            .post(url)
            .header("x-confirm-action", "true")
            .json(body);
        if let Some(token) = self.token.as_ref() {
            request = request.bearer_auth(token.0.expose_secret());
        }
        self.decode_response(request).await
    }

    pub async fn put_json_confirmed<Request, Response>(
        &self,
        path: &str,
        body: &Request,
    ) -> Result<Response, GatewayClientError>
    where
        Request: Serialize + ?Sized,
        Response: DeserializeOwned,
    {
        let url = self.join_path(path)?;
        let mut request = self
            .client
            .put(url)
            .header("x-confirm-action", "true")
            .json(body);
        if let Some(token) = self.token.as_ref() {
            request = request.bearer_auth(token.0.expose_secret());
        }
        self.decode_response(request).await
    }

    pub async fn delete_json_confirmed<Response>(
        &self,
        path: &str,
    ) -> Result<Response, GatewayClientError>
    where
        Response: DeserializeOwned,
    {
        let url = self.join_path(path)?;
        let mut request = self.client.delete(url).header("x-confirm-action", "true");
        if let Some(token) = self.token.as_ref() {
            request = request.bearer_auth(token.0.expose_secret());
        }
        self.decode_response(request).await
    }

    pub async fn get_json<Query, Response>(
        &self,
        path: &str,
        query: &Query,
    ) -> Result<Response, GatewayClientError>
    where
        Query: Serialize + ?Sized,
        Response: DeserializeOwned,
    {
        let url = self.join_path(path)?;
        let mut request = self.client.get(url).query(query);
        if let Some(token) = self.token.as_ref() {
            request = request.bearer_auth(token.0.expose_secret());
        }
        let response = request
            .send()
            .await
            .map_err(|error| GatewayClientError::Transport(error.without_url().to_string()))?;
        let status = response.status();
        let bytes = bounded_response_bytes(response, self.budget.max_response_bytes).await?;
        if !status.is_success() {
            return Err(GatewayClientError::Api {
                status: status.as_u16(),
                message: safe_api_message(&bytes),
            });
        }
        serde_json::from_slice(&bytes)
            .map_err(|error| GatewayClientError::MalformedResponse(error.to_string()))
    }

    fn join_path(&self, path: &str) -> Result<Url, GatewayClientError> {
        if !path.starts_with('/') || path.starts_with("//") || path.contains(['?', '#']) {
            return Err(GatewayClientError::InvalidPath);
        }
        let relative = path.trim_start_matches('/');
        if relative.split('/').any(|segment| segment == "..") {
            return Err(GatewayClientError::InvalidPath);
        }
        self.origin
            .join(relative)
            .map_err(|_| GatewayClientError::InvalidPath)
    }

    async fn decode_response<Response>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<Response, GatewayClientError>
    where
        Response: DeserializeOwned,
    {
        let response = request
            .send()
            .await
            .map_err(|error| GatewayClientError::Transport(error.without_url().to_string()))?;
        let status = response.status();
        let bytes = bounded_response_bytes(response, self.budget.max_response_bytes).await?;
        if !status.is_success() {
            return Err(GatewayClientError::Api {
                status: status.as_u16(),
                message: safe_api_message(&bytes),
            });
        }
        serde_json::from_slice(&bytes)
            .map_err(|error| GatewayClientError::MalformedResponse(error.to_string()))
    }
}

fn validate_origin(value: &str) -> Result<Url, GatewayClientError> {
    let mut origin =
        Url::parse(value).map_err(|error| GatewayClientError::InvalidOrigin(error.to_string()))?;
    if !matches!(origin.scheme(), "http" | "https")
        || origin.host_str().is_none()
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(GatewayClientError::InvalidOrigin(
            "expected an HTTP(S) origin without credentials, query, or fragment".to_string(),
        ));
    }
    if origin.scheme() == "http" {
        let host = origin.host_str().unwrap_or_default();
        let local = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if !local {
            return Err(GatewayClientError::InvalidOrigin(
                "unencrypted HTTP is restricted to loopback origins".to_string(),
            ));
        }
    }
    origin.set_path("/");
    Ok(origin)
}

async fn bounded_response_bytes(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, GatewayClientError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(GatewayClientError::ResponseTooLarge { limit });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| GatewayClientError::Transport(error.without_url().to_string()))?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(GatewayClientError::ResponseTooLarge { limit });
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn safe_api_message(bytes: &[u8]) -> String {
    const MAX_API_ERROR_CHARS: usize = 1024;
    let text = String::from_utf8_lossy(bytes);
    let sanitized: String = text
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(MAX_API_ERROR_CHARS)
        .collect();
    if sanitized.trim().is_empty() {
        "gateway request was rejected".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_rejects_credentials_and_remote_plaintext() {
        assert!(validate_origin("http://user:secret@127.0.0.1:3000").is_err());
        assert!(validate_origin("http://example.com").is_err());
        assert!(validate_origin("https://example.com?token=secret").is_err());
        assert_eq!(
            validate_origin("http://127.0.0.1:3000/base")
                .expect("loopback origin")
                .as_str(),
            "http://127.0.0.1:3000/"
        );
    }

    #[test]
    fn diagnostics_never_include_the_bearer() {
        let client = GatewayClient::new(
            validate_origin("http://127.0.0.1:3000").unwrap(),
            Some(GatewayAuthToken::new("known-secret-sentinel".into()).unwrap()),
            GatewayRequestBudget::control_plane(),
        )
        .unwrap();
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("known-secret-sentinel"));
        assert!(rendered.contains("authenticated: true"));
    }

    #[test]
    fn path_join_cannot_replace_the_origin() {
        let client = GatewayClient::new(
            validate_origin("https://example.com").unwrap(),
            None,
            GatewayRequestBudget::control_plane(),
        )
        .unwrap();
        assert!(client.join_path("//attacker.invalid/path").is_err());
        assert!(client.join_path("/../admin").is_err());
        assert_eq!(
            client.join_path("/api/chat/send").unwrap().as_str(),
            "https://example.com/api/chat/send"
        );
    }
}
