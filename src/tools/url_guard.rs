use std::borrow::Cow;
use std::net::IpAddr;
use std::net::{Ipv4Addr, ToSocketAddrs};

use reqwest::Url;

use crate::tools::tool::ToolError;

#[derive(Debug, Clone, Default)]
pub struct OutboundUrlGuardOptions {
    pub require_https: bool,
    pub upgrade_http_to_https: bool,
    pub allowlist: Vec<String>,
}

pub fn validate_outbound_url(
    url: &str,
    options: &OutboundUrlGuardOptions,
) -> Result<Url, ToolError> {
    let normalized = if options.upgrade_http_to_https {
        if let Some(rest) = url.strip_prefix("http://") {
            tracing::debug!("[url_guard] Upgrading http:// to https:// for {}", rest);
            Cow::Owned(format!("https://{}", rest))
        } else {
            Cow::Borrowed(url)
        }
    } else {
        Cow::Borrowed(url)
    };

    let parsed = Url::parse(normalized.as_ref())
        .map_err(|e| ToolError::InvalidParameters(format!("invalid URL: {e}")))?;
    let scheme = parsed.scheme();
    if !matches!(scheme, "http" | "https") {
        return Err(ToolError::NotAuthorized(format!(
            "only http:// and https:// URLs are allowed (got '{scheme}')"
        )));
    }
    if options.require_https && scheme != "https" {
        return Err(ToolError::NotAuthorized(format!(
            "only https:// URLs are allowed (got '{scheme}')"
        )));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| ToolError::InvalidParameters("URL missing host".to_string()))?;
    let host_lower = host.to_lowercase();
    if host_lower == "localhost" || host_lower.ends_with(".localhost") {
        return Err(ToolError::NotAuthorized(
            "localhost is not allowed".to_string(),
        ));
    }

    if !options.allowlist.is_empty() {
        let allowed = options
            .allowlist
            .iter()
            .any(|pattern| host_matches_allowlist(&host_lower, pattern));
        if !allowed {
            return Err(ToolError::NotAuthorized(format!(
                "host '{}' is not in the URL allowlist",
                host
            )));
        }
    }

    if let Ok(ip) = host.parse::<IpAddr>()
        && is_disallowed_ip(ip)
    {
        return Err(ToolError::NotAuthorized(format!(
            "address '{}' is not allowed",
            host
        )));
    }

    let port = parsed.port_or_known_default().unwrap_or(match scheme {
        "https" => 443,
        _ => 80,
    });
    let socket_addr = format!("{host}:{port}");
    if let Ok(addrs) = socket_addr.to_socket_addrs() {
        for addr in addrs {
            if is_disallowed_ip(addr.ip()) {
                return Err(ToolError::NotAuthorized(format!(
                    "hostname '{}' resolves to disallowed IP {}",
                    host,
                    addr.ip()
                )));
            }
        }
    }

    Ok(parsed)
}

fn host_matches_allowlist(host: &str, pattern: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        host == pattern
    }
}

fn is_disallowed_ip(ip: IpAddr) -> bool {
    match normalize_ip(ip) {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4 == Ipv4Addr::new(169, 254, 169, 254)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unicast_link_local()
                || v6.is_unique_local()
                || v6.is_multicast()
        }
    }
}

fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn https_options() -> OutboundUrlGuardOptions {
        OutboundUrlGuardOptions {
            require_https: true,
            upgrade_http_to_https: true,
            allowlist: Vec::new(),
        }
    }

    #[test]
    fn upgrades_http_when_requested() {
        let parsed = validate_outbound_url("http://example.com", &https_options()).unwrap();
        assert_eq!(parsed.as_str(), "https://example.com/");
    }

    #[test]
    fn blocks_ipv4_mapped_loopback() {
        let parsed = validate_outbound_url("https://[::ffff:127.0.0.1]/", &https_options());
        assert!(parsed.is_err());
    }

    #[test]
    fn blocks_ipv6_unique_local() {
        let parsed = validate_outbound_url("https://[fd00::1]/", &https_options());
        assert!(parsed.is_err());
    }
}
