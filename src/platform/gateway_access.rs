//! Shared Web Gateway access reporting.

use crate::settings::Settings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayAccessInfo {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub auth_token: Option<String>,
    pub cli_enabled: bool,
}

impl GatewayAccessInfo {
    pub fn from_env_and_settings(settings: Option<&Settings>) -> Self {
        let channels = settings.map(|settings| &settings.channels);
        let enabled = env_bool("GATEWAY_ENABLED")
            .or_else(|| channels.and_then(|channels| channels.gateway_enabled))
            .unwrap_or(true);
        let host = env_string("GATEWAY_HOST")
            .or_else(|| channels.and_then(|channels| channels.gateway_host.clone()))
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let port = env_string("GATEWAY_PORT")
            .and_then(|port| port.parse::<u16>().ok())
            .or_else(|| channels.and_then(|channels| channels.gateway_port))
            .unwrap_or(3000);
        let auth_token = env_string("GATEWAY_AUTH_TOKEN")
            .or_else(|| channels.and_then(|channels| channels.gateway_auth_token.clone()))
            .filter(|token| !token.trim().is_empty());
        let cli_enabled = env_bool("CLI_ENABLED")
            .or_else(|| channels.and_then(|channels| channels.cli_enabled))
            .unwrap_or(true);

        Self {
            enabled,
            host,
            port,
            auth_token,
            cli_enabled,
        }
    }

    pub fn is_loopback(&self) -> bool {
        is_loopback_host(&self.host)
    }

    pub fn bind_display(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn health_url(&self) -> String {
        format!(
            "http://{}:{}/api/health",
            self.local_request_host(),
            self.port
        )
    }

    pub fn local_url(&self) -> String {
        format!("http://{}:{}/", self.access_host(), self.port)
    }

    pub fn token_url(&self, show_token: bool) -> Option<String> {
        let token = self.auth_token.as_ref()?;
        let display_token = if show_token {
            urlencoding::encode(token).into_owned()
        } else {
            "<redacted>".to_string()
        };
        Some(format!(
            "http://{}:{}/?token={}",
            self.access_host(),
            self.port,
            display_token
        ))
    }

    pub fn ssh_tunnel_command(&self) -> String {
        format!("ssh -L {port}:127.0.0.1:{port} user@host", port = self.port)
    }

    pub fn auth_status(&self) -> &'static str {
        if self.auth_token.is_some() {
            "token configured"
        } else {
            "missing token"
        }
    }

    pub fn service_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if !self.enabled {
            warnings.push("Gateway is disabled; WebUI access will not be available.".to_string());
        }
        if self.auth_token.is_none() {
            warnings.push(
                "GATEWAY_AUTH_TOKEN is missing; configure a token before remote access."
                    .to_string(),
            );
        }
        if self.cli_enabled {
            warnings.push(
                "CLI_ENABLED is true; service/headless runs should set CLI_ENABLED=false to avoid stdin EOF shutdown.".to_string(),
            );
        }
        if !self.is_loopback() {
            warnings.push(
                "Gateway binds beyond loopback; keep firewall/TLS/proxy exposure intentional."
                    .to_string(),
            );
        }
        warnings
    }

    fn access_host(&self) -> &str {
        if self.host == "0.0.0.0" || self.host == "::" {
            "<host-or-tailnet-ip>"
        } else {
            &self.host
        }
    }

    fn local_request_host(&self) -> &str {
        if self.host == "0.0.0.0" || self.host == "::" {
            "127.0.0.1"
        } else {
            &self.host
        }
    }
}

pub fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn env_string(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_bool(key: &str) -> Option<bool> {
    env_string(key).map(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_url_redacts_by_default() {
        let info = GatewayAccessInfo {
            enabled: true,
            host: "127.0.0.1".to_string(),
            port: 3000,
            auth_token: Some("secret-token".to_string()),
            cli_enabled: false,
        };

        assert_eq!(
            info.token_url(false).as_deref(),
            Some("http://127.0.0.1:3000/?token=<redacted>")
        );
        assert_eq!(
            info.token_url(true).as_deref(),
            Some("http://127.0.0.1:3000/?token=secret-token")
        );
    }

    #[test]
    fn private_bind_uses_access_placeholder() {
        let info = GatewayAccessInfo {
            enabled: true,
            host: "0.0.0.0".to_string(),
            port: 3000,
            auth_token: Some("secret-token".to_string()),
            cli_enabled: false,
        };

        assert_eq!(
            info.token_url(false).as_deref(),
            Some("http://<host-or-tailnet-ip>:3000/?token=<redacted>")
        );
        assert_eq!(info.health_url(), "http://127.0.0.1:3000/api/health");
    }
}
