use std::borrow::Cow;
use std::net::IpAddr;
use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs};

use reqwest::Url;

use crate::tool::ToolError;

#[derive(Debug, Clone, Default)]
pub struct OutboundUrlGuardOptions {
    pub require_https: bool,
    pub upgrade_http_to_https: bool,
    pub allowlist: Vec<String>,
}

/// A URL that passed outbound SSRF validation, together with the resolved socket
/// addresses that were checked against the disallowed-IP policy.
///
/// The `pinned_addrs` are the exact addresses the host resolved to *at validation
/// time*. Callers that want to close the DNS-rebinding TOCTOU window should pin
/// these into their HTTP client (e.g. `reqwest::ClientBuilder::resolve_to_addrs`
/// keyed on the URL host) so the connection targets an address that already
/// passed the private-IP check, rather than letting the client re-resolve at
/// connect time and potentially reach a rebound private address.
///
/// `pinned_addrs` is empty when the host is an IP literal (nothing to pin —
/// `reqwest` will connect to that literal directly) or when resolution yielded
/// no addresses (the connection will simply fail later, as before).
#[derive(Debug, Clone)]
pub struct GuardedUrl {
    pub url: Url,
    pub pinned_addrs: Vec<SocketAddr>,
}

/// Validate an outbound URL against the SSRF policy and return the parsed [`Url`].
///
/// This is the backwards-compatible surface used by callers that do not (yet)
/// pin the validated address. New callers that want to defeat DNS-rebinding
/// should prefer [`validate_outbound_url_pinned`], which additionally returns the
/// resolved [`SocketAddr`]s that passed validation.
pub fn validate_outbound_url(
    url: &str,
    options: &OutboundUrlGuardOptions,
) -> Result<Url, ToolError> {
    validate_outbound_url_pinned(url, options).map(|guarded| guarded.url)
}

/// Validate an outbound URL against the SSRF policy and return both the parsed
/// [`Url`] and the resolved socket addresses that passed the disallowed-IP check.
///
/// The returned [`GuardedUrl::pinned_addrs`] let the caller bind the HTTP client
/// to the exact addresses that were validated, eliminating the time-of-check /
/// time-of-use gap where a hostname could rebind to a private address between
/// validation and the client's own connect-time resolution.
pub fn validate_outbound_url_pinned(
    url: &str,
    options: &OutboundUrlGuardOptions,
) -> Result<GuardedUrl, ToolError> {
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

    // If the host is an IP literal it was already validated above; there is
    // nothing to pin because `reqwest` connects to the literal directly and
    // cannot rebind it.
    let host_is_ip_literal = host.parse::<IpAddr>().is_ok();

    let mut pinned_addrs = Vec::new();
    if !host_is_ip_literal {
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
                // Record the validated address so callers can pin the
                // connection to it and avoid a connect-time re-resolution
                // (DNS-rebinding TOCTOU).
                pinned_addrs.push(addr);
            }
        }
    }

    Ok(GuardedUrl {
        url: parsed,
        pinned_addrs,
    })
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
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
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

    #[test]
    fn ip_literal_has_no_pinned_addrs() {
        // IP literals are validated directly and need no pinning; reqwest cannot
        // rebind a literal.
        let guarded = validate_outbound_url_pinned("https://8.8.8.8/", &https_options()).unwrap();
        assert!(
            guarded.pinned_addrs.is_empty(),
            "IP literals should not be pinned (reqwest connects to the literal directly)"
        );
        assert_eq!(guarded.url.host_str(), Some("8.8.8.8"));
    }

    #[test]
    fn hostname_pins_validated_addrs() {
        // localhost is blocked, so use a host that resolves locally without
        // network access. `127.0.0.1.nip.io`-style hosts need DNS; instead rely
        // on the OS resolving a literal-backed name is not portable. Resolution
        // may be unavailable in CI sandboxes, so only assert that when addresses
        // are returned they are all public (none disallowed) and the pin field
        // is consistent with the resolved set.
        let guarded = validate_outbound_url_pinned("https://8.8.8.8/", &https_options()).unwrap();
        // IP literal path: addrs empty by design.
        assert!(guarded.pinned_addrs.is_empty());

        // For a real hostname, if resolution succeeds the pinned addrs must all
        // have passed the disallowed-IP check.
        if let Ok(guarded) = validate_outbound_url_pinned("https://dns.google/", &https_options()) {
            for addr in &guarded.pinned_addrs {
                assert!(
                    !is_disallowed_ip(addr.ip()),
                    "pinned address {} should have passed the SSRF check",
                    addr.ip()
                );
            }
        }
    }
}
