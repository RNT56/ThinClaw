use std::borrow::Cow;
use std::net::IpAddr;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

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
/// `pinned_addrs` is empty only when the host is an already-validated IP literal
/// (nothing to pin because `reqwest` connects to the literal directly).
#[derive(Debug, Clone)]
pub struct GuardedUrl {
    pub url: Url,
    pub pinned_addrs: Vec<SocketAddr>,
}

const DEFAULT_OUTBOUND_DNS_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_OUTBOUND_DNS_ADDRESSES: usize = 64;
const MAX_OUTBOUND_URL_BYTES: usize = 16 * 1024;

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

/// Validate URL syntax, scheme, credentials, allowlisting, and IP literals
/// without resolving a hostname.
///
/// This is appropriate for synchronous configuration parsing only. Code that
/// will connect to the URL must subsequently call
/// [`validate_outbound_url_pinned_async`] and pin the returned addresses.
pub fn validate_outbound_url_structure(
    url: &str,
    options: &OutboundUrlGuardOptions,
) -> Result<Url, ToolError> {
    validate_before_dns(url, options).map(|(parsed, _, _, _)| parsed)
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
    let (parsed, host, port, host_is_ip_literal) = validate_before_dns(url, options)?;
    if host_is_ip_literal {
        return Ok(GuardedUrl {
            url: parsed,
            pinned_addrs: Vec::new(),
        });
    }

    let socket_addr = format!("{host}:{port}");
    let addrs = socket_addr.to_socket_addrs().map_err(|error| {
        ToolError::ExternalService(format!(
            "failed to resolve outbound hostname '{host}': {error}"
        ))
    })?;
    let pinned_addrs = validate_resolved_addresses(&host, addrs)?;
    Ok(GuardedUrl {
        url: parsed,
        pinned_addrs,
    })
}

/// Async counterpart to [`validate_outbound_url_pinned`].
///
/// Hostname resolution is bounded by a hard deadline so an unresponsive or
/// adversarial resolver cannot block an async agent worker indefinitely.
pub async fn validate_outbound_url_pinned_async(
    url: &str,
    options: &OutboundUrlGuardOptions,
) -> Result<GuardedUrl, ToolError> {
    let (parsed, host, port, host_is_ip_literal) = validate_before_dns(url, options)?;
    if host_is_ip_literal {
        return Ok(GuardedUrl {
            url: parsed,
            pinned_addrs: Vec::new(),
        });
    }

    let resolved = tokio::time::timeout(
        DEFAULT_OUTBOUND_DNS_TIMEOUT,
        tokio::net::lookup_host((host.as_str(), port)),
    )
    .await
    .map_err(|_| {
        ToolError::ExternalService(format!(
            "outbound hostname '{host}' did not resolve within {DEFAULT_OUTBOUND_DNS_TIMEOUT:?}"
        ))
    })?
    .map_err(|error| {
        ToolError::ExternalService(format!(
            "failed to resolve outbound hostname '{host}': {error}"
        ))
    })?;
    let pinned_addrs = validate_resolved_addresses(&host, resolved)?;
    Ok(GuardedUrl {
        url: parsed,
        pinned_addrs,
    })
}

fn validate_before_dns(
    url: &str,
    options: &OutboundUrlGuardOptions,
) -> Result<(Url, String, u16, bool), ToolError> {
    if url.is_empty() || url.len() > MAX_OUTBOUND_URL_BYTES {
        return Err(ToolError::InvalidParameters(format!(
            "outbound URL is empty or exceeds the {MAX_OUTBOUND_URL_BYTES}-byte limit"
        )));
    }
    let normalized = if options.upgrade_http_to_https {
        if let Some(rest) = url.strip_prefix("http://") {
            tracing::debug!("[url_guard] Upgrading outbound URL from HTTP to HTTPS");
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
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ToolError::NotAuthorized(
            "outbound URLs cannot contain embedded credentials".to_string(),
        ));
    }
    if parsed.fragment().is_some() {
        return Err(ToolError::InvalidParameters(
            "outbound URLs cannot contain fragments".to_string(),
        ));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| ToolError::InvalidParameters("URL missing host".to_string()))?
        .to_string();
    let host_lower = host.trim_end_matches('.').to_lowercase();
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
        && !is_public_outbound_ip(ip)
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
    Ok((parsed, host, port, host_is_ip_literal))
}

fn validate_resolved_addresses(
    host: &str,
    addrs: impl IntoIterator<Item = SocketAddr>,
) -> Result<Vec<SocketAddr>, ToolError> {
    let mut pinned_addrs = Vec::new();
    for addr in addrs {
        if pinned_addrs.len() >= MAX_OUTBOUND_DNS_ADDRESSES {
            return Err(ToolError::ExternalService(format!(
                "outbound hostname '{host}' resolved to more than {MAX_OUTBOUND_DNS_ADDRESSES} addresses"
            )));
        }
        if !is_public_outbound_ip(addr.ip()) {
            return Err(ToolError::NotAuthorized(format!(
                "hostname '{}' resolves to disallowed IP {}",
                host,
                addr.ip()
            )));
        }
        pinned_addrs.push(addr);
    }
    if pinned_addrs.is_empty() {
        return Err(ToolError::ExternalService(format!(
            "outbound hostname '{host}' resolved to no addresses"
        )));
    }
    pinned_addrs.sort_unstable();
    pinned_addrs.dedup();
    Ok(pinned_addrs)
}

fn host_matches_allowlist(host: &str, pattern: &str) -> bool {
    let pattern = pattern.trim().trim_end_matches('.').to_ascii_lowercase();
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        host == pattern
    }
}

/// Whether an address is globally routable enough for untrusted outbound HTTP.
/// This deliberately rejects special-use, benchmarking, documentation,
/// translation, and transition ranges in addition to ordinary private space.
pub fn is_public_outbound_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || ip.is_multicast()
                || octets[0] == 0
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
                || (octets[0] == 192 && octets[1] == 88 && octets[2] == 99)
                || (octets[0] == 198 && (18..=19).contains(&octets[1]))
                || octets[0] >= 240)
        }
        IpAddr::V6(ip) => {
            if let Some(ipv4) = ip.to_ipv4_mapped() {
                return is_public_outbound_ip(IpAddr::V4(ipv4));
            }
            let segments = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || (segments[0] & 0xffc0) == 0xfec0
                || segments[..6] == [0, 0, 0, 0, 0, 0]
                || (segments[0] == 0x0064 && segments[1] == 0xff9b)
                || (segments[0] == 0x2001 && segments[1] <= 0x01ff)
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
                || segments[0] == 0x2002)
        }
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

    #[tokio::test]
    async fn async_guard_validates_ip_literals_without_dns() {
        let guarded = validate_outbound_url_pinned_async("https://8.8.8.8/", &https_options())
            .await
            .unwrap();
        assert!(guarded.pinned_addrs.is_empty());
        assert_eq!(guarded.url.host_str(), Some("8.8.8.8"));
    }

    #[test]
    fn resolved_address_count_is_bounded() {
        let address: SocketAddr = "8.8.8.8:443".parse().unwrap();
        let addresses = vec![address; MAX_OUTBOUND_DNS_ADDRESSES + 1];
        let error = validate_resolved_addresses("example.com", addresses).unwrap_err();
        assert!(error.to_string().contains("more than"));
    }

    #[test]
    fn allowlist_matching_is_case_insensitive() {
        assert!(host_matches_allowlist("api.example.com", "*.EXAMPLE.COM."));
        assert!(!host_matches_allowlist("evil-example.com", "*.example.com"));
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
                    is_public_outbound_ip(addr.ip()),
                    "pinned address {} should have passed the SSRF check",
                    addr.ip()
                );
            }
        }
    }
}
