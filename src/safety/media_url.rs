//! Media URL validation.
//!
//! Validates media URLs before fetching to prevent SSRF and other
//! security issues.

use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Media URL validation config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaUrlConfig {
    /// Whether URL validation is enabled.
    pub enabled: bool,
    /// Allow private/internal IP addresses.
    pub allow_private_ips: bool,
    /// Allow loopback addresses.
    pub allow_loopback: bool,
    /// Maximum URL length.
    pub max_url_length: usize,
    /// Allowed schemes.
    pub allowed_schemes: Vec<String>,
    /// Blocked hostname patterns.
    pub blocked_hosts: Vec<String>,
    /// Maximum redirect count.
    pub max_redirects: u32,
    /// Maximum file size to fetch (bytes).
    pub max_fetch_size: u64,
}

impl Default for MediaUrlConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_private_ips: false,
            allow_loopback: false,
            max_url_length: 2048,
            allowed_schemes: vec!["https".to_string(), "http".to_string()],
            blocked_hosts: vec![
                "metadata.google.internal".to_string(),
                "169.254.169.254".to_string(), // Cloud metadata
            ],
            max_redirects: 5,
            max_fetch_size: 100 * 1024 * 1024, // 100 MB
        }
    }
}

/// Validation result.
#[derive(Debug, Clone, PartialEq)]
pub enum UrlValidation {
    Valid,
    Invalid(String),
}

impl MediaUrlConfig {
    /// Validate a URL.
    pub fn validate(&self, url: &str) -> UrlValidation {
        if !self.enabled {
            return UrlValidation::Valid;
        }

        // Check length
        if url.len() > self.max_url_length {
            return UrlValidation::Invalid(format!(
                "URL too long: {} > {}",
                url.len(),
                self.max_url_length
            ));
        }

        // Parse the URL
        let parsed = match url::Url::parse(url) {
            Ok(u) => u,
            Err(e) => return UrlValidation::Invalid(format!("Invalid URL: {}", e)),
        };

        // Check scheme
        if !self.allowed_schemes.iter().any(|s| s == parsed.scheme()) {
            return UrlValidation::Invalid(format!("Blocked scheme: {}", parsed.scheme()));
        }

        let host = match parsed.host() {
            Some(host) => host,
            None => return UrlValidation::Invalid("No host in URL".to_string()),
        };

        let host_display = host.to_string();

        // Blocked hosts
        for blocked in &self.blocked_hosts {
            if host_display == *blocked || host_display.ends_with(&format!(".{}", blocked)) {
                return UrlValidation::Invalid(format!("Blocked host: {}", host_display));
            }
        }

        // IP address checks
        let ip = match host {
            url::Host::Ipv4(ip) => Some(IpAddr::V4(ip)),
            url::Host::Ipv6(ip) => Some(IpAddr::V6(ip)),
            url::Host::Domain(_) => None,
        };
        if let Some(ip) = ip {
            if !self.allow_loopback && ip.is_loopback() {
                return UrlValidation::Invalid("Loopback address not allowed".to_string());
            }
            if !self.allow_private_ips && is_private_ip(&ip) {
                return UrlValidation::Invalid("Private IP address not allowed".to_string());
            }
        }

        UrlValidation::Valid
    }
}

/// Check if an IP address is private/internal.
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.octets()[0] == 100 && v4.octets()[1] >= 64 && v4.octets()[1] <= 127 // CGN
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // fc00::/7 unique local addresses
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // fe80::/10 link-local addresses
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Sanitize a URL for logging (mask query params that might contain tokens).
pub fn sanitize_url_for_log(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            if parsed.query().is_some() {
                parsed.set_query(Some("***"));
            }
            parsed.to_string()
        }
        Err(_) => "[invalid URL]".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_https() {
        let config = MediaUrlConfig::default();
        assert_eq!(
            config.validate("https://example.com/image.png"),
            UrlValidation::Valid
        );
    }

    #[test]
    fn test_blocked_scheme() {
        let config = MediaUrlConfig::default();
        assert!(matches!(
            config.validate("ftp://example.com/file"),
            UrlValidation::Invalid(_)
        ));
    }

    #[test]
    fn test_blocked_host() {
        let config = MediaUrlConfig::default();
        assert!(matches!(
            config.validate("http://169.254.169.254/metadata"),
            UrlValidation::Invalid(_)
        ));
    }

    #[test]
    fn test_private_ip_blocked() {
        let config = MediaUrlConfig::default();
        assert!(matches!(
            config.validate("http://192.168.1.1/image.png"),
            UrlValidation::Invalid(_)
        ));
    }

    #[test]
    fn test_private_ip_allowed() {
        let config = MediaUrlConfig {
            allow_private_ips: true,
            ..Default::default()
        };
        assert_eq!(
            config.validate("http://192.168.1.1/image.png"),
            UrlValidation::Valid
        );
    }

    #[test]
    fn test_loopback_blocked() {
        let config = MediaUrlConfig::default();
        assert!(matches!(
            config.validate("http://127.0.0.1/secret"),
            UrlValidation::Invalid(_)
        ));
    }

    #[test]
    fn test_ipv6_ula_blocked() {
        let config = MediaUrlConfig::default();
        assert!(matches!(
            config.validate("http://[fc00::1]/image.png"),
            UrlValidation::Invalid(_)
        ));
    }

    #[test]
    fn test_ipv6_unspecified_blocked() {
        let config = MediaUrlConfig::default();
        assert!(matches!(
            config.validate("http://[::]/image.png"),
            UrlValidation::Invalid(_)
        ));
    }

    #[test]
    fn test_ipv6_multicast_blocked() {
        let config = MediaUrlConfig::default();
        assert!(matches!(
            config.validate("http://[ff02::1]/image.png"),
            UrlValidation::Invalid(_)
        ));
    }

    #[test]
    fn test_too_long() {
        let config = MediaUrlConfig {
            max_url_length: 10,
            ..Default::default()
        };
        assert!(matches!(
            config.validate("https://example.com/very/long/path"),
            UrlValidation::Invalid(_)
        ));
    }

    #[test]
    fn test_disabled() {
        let config = MediaUrlConfig {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(config.validate("ftp://evil.com"), UrlValidation::Valid);
    }

    #[test]
    fn test_sanitize_url() {
        let result = sanitize_url_for_log("https://api.example.com/v1?token=secret");
        assert!(!result.contains("secret"));
        assert!(result.contains("***"));
    }
}
