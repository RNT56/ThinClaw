use super::*;

pub(super) use thinclaw_agent::learning_provider_policy::{
    payload_text, provider_agent_id, provider_base_url, provider_base_url_or, provider_bool,
    provider_config_value, provider_export_messages, provider_join_url, provider_path,
    provider_path_with_vars, provider_scoped_user_id,
};

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

    let base_url = base_url.expect("checked above");
    let health_path = provider_config_value(&provider.config, "health_url")
        .or_else(|| provider_config_value(&provider.config, "health_path"))
        .unwrap_or_else(|| default_health_path.unwrap_or("/health").to_string());
    let health_url = provider_join_url(&base_url, &health_path);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let Ok(client) = client else {
        return provider_http_client_error_status(provider_name, provider.enabled);
    };

    let started = std::time::Instant::now();
    let request = apply_provider_auth(
        client.get(&health_url),
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
            err.to_string(),
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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let Ok(client) = client else {
        return provider_http_client_error_status(provider_name, enabled);
    };

    let started = std::time::Instant::now();
    let mut req = client.get(format!("{}/health", base_url.trim_end_matches('/')));
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
            err.to_string(),
            started.elapsed().as_millis() as u64,
            None,
        ),
    }
}
