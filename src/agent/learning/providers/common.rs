use super::*;

pub(super) use thinclaw_agent::learning_provider_policy::{
    payload_text, provider_agent_id, provider_base_url, provider_base_url_or, provider_bool,
    provider_config_value, provider_export_messages, provider_join_url, provider_path,
    provider_path_with_vars, provider_scoped_user_id,
};

/// Shared `reqwest::Client` reused by every memory-provider HTTP call.
///
/// Provider adapters used to build a brand-new `reqwest::Client` per request
/// (per health probe, per recall, per export...), which pays TCP/TLS
/// handshake cost on every call instead of reusing pooled connections. A
/// single process-wide client (no default request timeout — callers set a
/// per-request timeout via `RequestBuilder::timeout` to preserve each call
/// site's existing budget) lets `reqwest`/hyper pool connections across
/// requests to the same provider host.
pub(super) fn shared_http_client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: std::sync::LazyLock<Result<reqwest::Client, String>> =
        std::sync::LazyLock::new(|| {
            reqwest::Client::builder()
                // Authenticated provider requests must never replay custom or
                // bearer headers to a redirect-selected origin.
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(5))
                .no_proxy()
                .build()
                .map_err(|error| {
                    format!("failed to initialize memory-provider HTTP client: {error}")
                })
        });
    CLIENT.as_ref().map_err(Clone::clone)
}

pub(super) async fn provider_json_response(
    response: reqwest::Response,
) -> Result<serde_json::Value, String> {
    const MAX_PROVIDER_JSON_BYTES: usize = 8 * 1024 * 1024;
    const MAX_PROVIDER_ERROR_BYTES: usize = 64 * 1024;
    let status = response.status();
    if !status.is_success() {
        // Drain only a bounded error body. Do not reflect peer-controlled text
        // into agent prompts, logs, or UI errors.
        let _ = crate::http_response::bounded_bytes(response, MAX_PROVIDER_ERROR_BYTES).await;
        return Err(format!("HTTP {status}"));
    }
    let body = crate::http_response::bounded_bytes(response, MAX_PROVIDER_JSON_BYTES)
        .await
        .map_err(|error| error.to_string())?;
    if body.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_slice(&body).map_err(|error| error.to_string())
}

pub(super) async fn provider_status_response(response: reqwest::Response) -> Result<(), String> {
    const MAX_PROVIDER_STATUS_BYTES: usize = 1024 * 1024;
    const MAX_PROVIDER_ERROR_BYTES: usize = 64 * 1024;
    let status = response.status();
    let limit = if status.is_success() {
        MAX_PROVIDER_STATUS_BYTES
    } else {
        MAX_PROVIDER_ERROR_BYTES
    };
    crate::http_response::bounded_bytes(response, limit)
        .await
        .map_err(|error| error.to_string())?;
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {status}"))
    }
}

pub(super) fn validated_provider_request_url(url: &str) -> Result<reqwest::Url, String> {
    if url.is_empty() || url.len() > 4096 || url.chars().any(char::is_control) {
        return Err("provider URL is empty, malformed, or oversized".to_string());
    }
    let parsed =
        reqwest::Url::parse(url).map_err(|error| format!("invalid provider URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err("provider URL must be HTTP(S) without credentials or a fragment".to_string());
    }
    Ok(parsed)
}

pub(super) fn provider_token(config: &std::collections::HashMap<String, String>) -> Option<String> {
    if let Some(token) = config.get("api_key").cloned().filter(|v| !v.is_empty()) {
        return Some(token);
    }
    if let Some(env_name) = config
        .get("api_key_env")
        .cloned()
        .filter(|v| !v.trim().is_empty())
    {
        return std::env::var(env_name)
            .ok()
            .filter(|v| !v.trim().is_empty());
    }
    None
}

pub(super) fn apply_provider_auth(
    request: reqwest::RequestBuilder,
    config: &std::collections::HashMap<String, String>,
    default_scheme: &str,
) -> reqwest::RequestBuilder {
    let Some(token) = provider_token(config) else {
        return request;
    };

    if let Some(header) = provider_config_value(config, "auth_header") {
        return request.header(header, token);
    }

    let scheme = provider_config_value(config, "auth_scheme")
        .unwrap_or_else(|| default_scheme.to_string())
        .to_ascii_lowercase();
    match scheme.as_str() {
        "none" | "disabled" => request,
        "token" => request.header(reqwest::header::AUTHORIZATION, format!("Token {token}")),
        "api-key" | "api_key" => request.header("api-key", token),
        "x-api-key" | "x_api_key" => request.header("X-API-Key", token),
        "x-chroma-token" | "x_chroma_token" => request.header("x-chroma-token", token),
        _ => request.bearer_auth(token),
    }
}

pub(super) async fn configured_provider_health(
    provider_name: &str,
    provider: Option<&crate::settings::LearningProviderSettings>,
    default_base_url: Option<&str>,
    default_health_path: Option<&str>,
    default_auth_scheme: &str,
    required_keys: &[&str],
) -> ProviderHealthStatus {
    let Some(provider) = provider else {
        return provider_health_request(provider_name, false, None, None).await;
    };

    let mut missing = Vec::new();
    let base_url =
        provider_base_url(&provider.config).or_else(|| default_base_url.map(str::to_string));
    if base_url.is_none() {
        missing.push("base_url".to_string());
    }
    for key in required_keys {
        if *key == "api_key" {
            if provider_token(&provider.config).is_none() {
                missing.push("api_key or api_key_env".to_string());
            }
        } else if provider_config_value(&provider.config, key).is_none() {
            missing.push((*key).to_string());
        }
    }
    if provider_bool(&provider.config, "require_api_key")
        && provider_token(&provider.config).is_none()
    {
        missing.push("api_key or api_key_env".to_string());
    }
    if let Some(status) = provider_required_status(provider_name, provider.enabled, &missing) {
        return status;
    }

    if provider_bool(&provider.config, "skip_health_check") || default_health_path.is_none() {
        return provider_configured_skipped_health_status(provider_name, provider.enabled);
    }

    let Some(base_url) = base_url else {
        return provider_missing_base_url_status(provider_name, provider.enabled);
    };
    let Some(default_health_path) = default_health_path else {
        return provider_configured_skipped_health_status(provider_name, provider.enabled);
    };
    let health_path = provider_config_value(&provider.config, "health_url")
        .or_else(|| provider_config_value(&provider.config, "health_path"))
        .unwrap_or_else(|| default_health_path.to_string());
    let health_url = match provider_join_url(&base_url, &health_path) {
        Ok(url) => url,
        Err(error) => {
            return provider_http_request_error_status(
                provider_name,
                provider.enabled,
                error,
                0,
                None,
            );
        }
    };

    let started = std::time::Instant::now();
    let client = match shared_http_client() {
        Ok(client) => client,
        Err(error) => {
            return provider_http_request_error_status(
                provider_name,
                provider.enabled,
                error,
                0,
                Some(&health_url),
            );
        }
    };
    let request = apply_provider_auth(
        client
            .get(&health_url)
            .timeout(std::time::Duration::from_secs(5)),
        &provider.config,
        default_auth_scheme,
    );
    match request.send().await {
        Ok(response) => provider_http_response_status(
            provider_name,
            provider.enabled,
            response.status().as_u16(),
            started.elapsed().as_millis() as u64,
            Some(&health_url),
        ),
        Err(err) => provider_http_request_error_status(
            provider_name,
            provider.enabled,
            err.without_url().to_string(),
            started.elapsed().as_millis() as u64,
            Some(&health_url),
        ),
    }
}

pub(super) async fn provider_health_request(
    provider_name: &str,
    enabled: bool,
    base_url: Option<String>,
    token: Option<String>,
) -> ProviderHealthStatus {
    if !enabled {
        return provider_disabled_status(provider_name, enabled);
    }

    let Some(base_url) = base_url else {
        return provider_missing_base_url_status(provider_name, enabled);
    };

    let health_url = match provider_join_url(&base_url, "/health") {
        Ok(url) => url,
        Err(error) => {
            return provider_http_request_error_status(provider_name, enabled, error, 0, None);
        }
    };
    let started = std::time::Instant::now();
    let client = match shared_http_client() {
        Ok(client) => client,
        Err(error) => {
            return provider_http_request_error_status(provider_name, enabled, error, 0, None);
        }
    };
    let mut req = client
        .get(health_url)
        .timeout(std::time::Duration::from_secs(5));
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }

    match req.send().await {
        Ok(response) => provider_http_response_status(
            provider_name,
            enabled,
            response.status().as_u16(),
            started.elapsed().as_millis() as u64,
            None,
        ),
        Err(err) => provider_http_request_error_status(
            provider_name,
            enabled,
            err.without_url().to_string(),
            started.elapsed().as_millis() as u64,
            None,
        ),
    }
}
