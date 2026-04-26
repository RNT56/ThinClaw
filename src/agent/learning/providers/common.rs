use super::*;
pub(super) fn provider_base_url(
    config: &std::collections::HashMap<String, String>,
) -> Option<String> {
    config
        .get("base_url")
        .or_else(|| config.get("url"))
        .cloned()
        .filter(|v| !v.trim().is_empty())
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

pub(super) fn provider_config_value(
    config: &std::collections::HashMap<String, String>,
    key: &str,
) -> Option<String> {
    config.get(key).cloned().filter(|v| !v.trim().is_empty())
}

pub(super) fn provider_base_url_or(
    config: &std::collections::HashMap<String, String>,
    default: &str,
) -> String {
    provider_base_url(config).unwrap_or_else(|| default.to_string())
}

pub(super) fn provider_bool(config: &std::collections::HashMap<String, String>, key: &str) -> bool {
    config
        .get(key)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}

pub(super) fn provider_scoped_user_id(
    config: &std::collections::HashMap<String, String>,
    user_id: &str,
) -> String {
    provider_config_value(config, "user_id").unwrap_or_else(|| user_id.to_string())
}

pub(super) fn provider_agent_id(config: &std::collections::HashMap<String, String>) -> String {
    provider_config_value(config, "agent_id").unwrap_or_else(|| "thinclaw".to_string())
}

pub(super) fn provider_join_url(base_url: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

pub(super) fn provider_path(
    config: &std::collections::HashMap<String, String>,
    key: &str,
    default: &str,
) -> String {
    provider_config_value(config, key).unwrap_or_else(|| default.to_string())
}

pub(super) fn provider_path_with_vars(
    config: &std::collections::HashMap<String, String>,
    key: &str,
    default: &str,
) -> String {
    let mut path = provider_path(config, key, default);
    for (name, value) in config {
        path = path.replace(&format!("{{{name}}}"), value);
    }
    path
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

pub(super) fn provider_required_status(
    provider_name: &str,
    enabled: bool,
    missing: &[String],
) -> Option<ProviderHealthStatus> {
    if !enabled {
        return Some(ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled,
            healthy: false,
            readiness: ProviderReadiness::Disabled,
            latency_ms: None,
            error: None,
            capabilities: Vec::new(),
            metadata: serde_json::json!({"state": "disabled"}),
        });
    }
    if missing.is_empty() {
        return None;
    }
    Some(ProviderHealthStatus {
        provider: provider_name.to_string(),
        active: false,
        enabled,
        healthy: false,
        readiness: ProviderReadiness::NotConfigured,
        latency_ms: None,
        error: Some(format!("missing {}", missing.join(", "))),
        capabilities: Vec::new(),
        metadata: serde_json::json!({
            "state": "not_configured",
            "missing": missing,
        }),
    })
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
        return ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled: provider.enabled,
            healthy: true,
            readiness: ProviderReadiness::Ready,
            latency_ms: None,
            error: None,
            capabilities: Vec::new(),
            metadata: serde_json::json!({
                "state": "configured",
                "health_check": "skipped",
            }),
        };
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
        return ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled: provider.enabled,
            healthy: false,
            readiness: ProviderReadiness::Unhealthy,
            latency_ms: None,
            error: Some("failed to initialize HTTP client".to_string()),
            capabilities: Vec::new(),
            metadata: serde_json::json!({}),
        };
    };

    let started = std::time::Instant::now();
    let request = apply_provider_auth(
        client.get(&health_url),
        &provider.config,
        default_auth_scheme,
    );
    match request.send().await {
        Ok(response) => ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled: provider.enabled,
            healthy: response.status().is_success(),
            readiness: if response.status().is_success() {
                ProviderReadiness::Ready
            } else {
                ProviderReadiness::Unhealthy
            },
            latency_ms: Some(started.elapsed().as_millis() as u64),
            error: if response.status().is_success() {
                None
            } else {
                Some(format!("HTTP {}", response.status()))
            },
            capabilities: Vec::new(),
            metadata: serde_json::json!({
                "status": response.status().as_u16(),
                "health_url": health_url,
            }),
        },
        Err(err) => ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled: provider.enabled,
            healthy: false,
            readiness: ProviderReadiness::Unhealthy,
            latency_ms: Some(started.elapsed().as_millis() as u64),
            error: Some(err.to_string()),
            capabilities: Vec::new(),
            metadata: serde_json::json!({"health_url": health_url}),
        },
    }
}

pub(super) async fn provider_health_request(
    provider_name: &str,
    enabled: bool,
    base_url: Option<String>,
    token: Option<String>,
) -> ProviderHealthStatus {
    if !enabled {
        return ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled,
            healthy: false,
            readiness: ProviderReadiness::Disabled,
            latency_ms: None,
            error: None,
            capabilities: Vec::new(),
            metadata: serde_json::json!({"state": "disabled"}),
        };
    }

    let Some(base_url) = base_url else {
        return ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled,
            healthy: false,
            readiness: ProviderReadiness::NotConfigured,
            latency_ms: None,
            error: Some("missing base_url".to_string()),
            capabilities: Vec::new(),
            metadata: serde_json::json!({}),
        };
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let Ok(client) = client else {
        return ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled,
            healthy: false,
            readiness: ProviderReadiness::Unhealthy,
            latency_ms: None,
            error: Some("failed to initialize HTTP client".to_string()),
            capabilities: Vec::new(),
            metadata: serde_json::json!({}),
        };
    };

    let started = std::time::Instant::now();
    let mut req = client.get(format!("{}/health", base_url.trim_end_matches('/')));
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }

    match req.send().await {
        Ok(response) => ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled,
            healthy: response.status().is_success(),
            readiness: if response.status().is_success() {
                ProviderReadiness::Ready
            } else {
                ProviderReadiness::Unhealthy
            },
            latency_ms: Some(started.elapsed().as_millis() as u64),
            error: if response.status().is_success() {
                None
            } else {
                Some(format!("HTTP {}", response.status()))
            },
            capabilities: Vec::new(),
            metadata: serde_json::json!({"status": response.status().as_u16()}),
        },
        Err(err) => ProviderHealthStatus {
            provider: provider_name.to_string(),
            active: false,
            enabled,
            healthy: false,
            readiness: ProviderReadiness::Unhealthy,
            latency_ms: Some(started.elapsed().as_millis() as u64),
            error: Some(err.to_string()),
            capabilities: Vec::new(),
            metadata: serde_json::json!({}),
        },
    }
}
