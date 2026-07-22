//! Domain allowlist for the network proxy.
//!
//! Validates that HTTP requests only go to allowed domains.
//! Supports exact matches and wildcard patterns.

use std::fmt;

pub const MAX_ALLOWLIST_PATTERNS: usize = 256;
pub const MAX_ALLOWLIST_PATTERN_BYTES: usize = 255;

/// Validate allowlist patterns before they are installed in a live proxy.
/// Patterns support exact DNS names/IP literals and a single leading `*.`.
/// Rejecting malformed entries is preferable to silently accepting a typo in
/// what is intended to be a security boundary.
pub fn validate_domain_allowlist(domains: &[String]) -> Result<(), String> {
    if domains.len() > MAX_ALLOWLIST_PATTERNS {
        return Err(format!(
            "network allowlist has {} entries; maximum is {MAX_ALLOWLIST_PATTERNS}",
            domains.len()
        ));
    }

    let mut seen = std::collections::HashSet::new();
    for (index, pattern) in domains.iter().enumerate() {
        if pattern.is_empty()
            || pattern.len() > MAX_ALLOWLIST_PATTERN_BYTES
            || pattern.trim() != pattern
            || !pattern.is_ascii()
            || pattern.chars().any(char::is_control)
        {
            return Err(format!("network allowlist entry {index} is malformed"));
        }

        let (wildcard, base) = match pattern.strip_prefix("*.") {
            Some(base) => (true, base),
            None => (false, pattern.as_str()),
        };
        if base.is_empty() || base.ends_with('.') || base.contains('*') {
            return Err(format!("network allowlist entry {index} is malformed"));
        }

        if base.parse::<std::net::IpAddr>().is_err() {
            if base.len() > 253
                || base.split('.').any(|label| {
                    label.is_empty()
                        || label.len() > 63
                        || !label
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                        || !label
                            .as_bytes()
                            .first()
                            .is_some_and(u8::is_ascii_alphanumeric)
                        || !label
                            .as_bytes()
                            .last()
                            .is_some_and(u8::is_ascii_alphanumeric)
                })
            {
                return Err(format!("network allowlist entry {index} is malformed"));
            }
        } else if wildcard {
            return Err(format!(
                "network allowlist entry {index} cannot wildcard an IP address"
            ));
        }

        let canonical = pattern.to_ascii_lowercase();
        if !seen.insert(canonical) {
            return Err(format!("network allowlist entry {index} is duplicated"));
        }
    }
    Ok(())
}

/// Pattern for matching allowed domains.
#[derive(Debug, Clone)]
pub struct DomainPattern {
    /// The domain pattern (e.g., "api.example.com" or "*.example.com").
    pattern: String,
    /// Whether this is a wildcard pattern.
    is_wildcard: bool,
    /// The base domain for wildcard matching.
    base_domain: String,
    valid: bool,
}

impl DomainPattern {
    /// Create a new domain pattern.
    pub fn new(pattern: &str) -> Self {
        let valid = validate_domain_allowlist(&[pattern.to_string()]).is_ok();
        let is_wildcard = pattern.starts_with("*.");
        let base_domain = if is_wildcard {
            pattern[2..].to_lowercase()
        } else {
            pattern.to_lowercase()
        };

        Self {
            pattern: pattern.to_string(),
            is_wildcard,
            base_domain,
            valid,
        }
    }

    /// Check if a host matches this pattern.
    pub fn matches(&self, host: &str) -> bool {
        if !self.valid {
            return false;
        }
        let host_lower = host.to_lowercase();

        if self.is_wildcard {
            // *.example.com matches foo.example.com, bar.baz.example.com, example.com
            host_lower == self.base_domain
                || host_lower.ends_with(&format!(".{}", self.base_domain))
        } else {
            host_lower == self.base_domain
        }
    }

    /// Get the pattern string.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }
}

impl fmt::Display for DomainPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.pattern)
    }
}

/// Result of domain validation.
#[derive(Debug, Clone)]
pub enum DomainValidationResult {
    /// Domain is allowed.
    Allowed,
    /// Domain is denied with a reason.
    Denied(String),
}

impl DomainValidationResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, DomainValidationResult::Allowed)
    }
}

/// Validates domains against an allowlist.
#[derive(Debug, Clone)]
pub struct DomainAllowlist {
    patterns: Vec<DomainPattern>,
}

impl DomainAllowlist {
    /// Create a new allowlist from domain strings.
    pub fn new(domains: &[String]) -> Self {
        if let Err(error) = validate_domain_allowlist(domains) {
            tracing::error!(%error, "Rejected malformed sandbox network allowlist");
            return Self::empty();
        }
        Self {
            patterns: domains.iter().map(|d| DomainPattern::new(d)).collect(),
        }
    }

    /// Create an empty allowlist (denies everything).
    pub fn empty() -> Self {
        Self { patterns: vec![] }
    }

    /// Add a domain pattern to the allowlist.
    pub fn add(&mut self, pattern: &str) {
        let candidate = DomainPattern::new(pattern);
        if candidate.valid
            && !self
                .patterns
                .iter()
                .any(|existing| existing.pattern.eq_ignore_ascii_case(pattern))
            && self.patterns.len() < MAX_ALLOWLIST_PATTERNS
        {
            self.patterns.push(candidate);
        }
    }

    /// Check if a domain is allowed.
    pub fn is_allowed(&self, host: &str) -> DomainValidationResult {
        if self.patterns.is_empty() {
            return DomainValidationResult::Denied("empty allowlist".to_string());
        }

        for pattern in &self.patterns {
            if pattern.matches(host) {
                return DomainValidationResult::Allowed;
            }
        }

        DomainValidationResult::Denied(format!(
            "host '{}' not in allowlist: [{}]",
            host,
            self.patterns
                .iter()
                .map(|p| p.pattern())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    /// Get all patterns in the allowlist.
    pub fn patterns(&self) -> &[DomainPattern] {
        &self.patterns
    }

    /// Check if the allowlist is empty.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Get the number of patterns.
    pub fn len(&self) -> usize {
        self.patterns.len()
    }
}

impl Default for DomainAllowlist {
    fn default() -> Self {
        Self::new(&crate::sandbox::config::default_allowlist())
    }
}

/// Parse host from a URL string.
pub fn extract_host(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    parsed.host_str().map(|h| {
        h.strip_prefix('[')
            .and_then(|v| v.strip_suffix(']'))
            .unwrap_or(h)
            .to_lowercase()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let pattern = DomainPattern::new("api.example.com");
        assert!(pattern.matches("api.example.com"));
        assert!(pattern.matches("API.EXAMPLE.COM"));
        assert!(!pattern.matches("foo.api.example.com"));
        assert!(!pattern.matches("example.com"));
    }

    #[test]
    fn validates_allowlist_pattern_shape_and_limits() {
        assert!(
            validate_domain_allowlist(&[
                "example.com".to_string(),
                "*.api.example.com".to_string(),
                "203.0.113.10".to_string(),
            ])
            .is_ok()
        );
        for invalid in [
            "",
            " example.com",
            "https://example.com",
            "*.127.0.0.1",
            "*.*.example.com",
            "-bad.example",
            "bad-.example",
            "example.com.",
        ] {
            assert!(
                validate_domain_allowlist(&[invalid.to_string()]).is_err(),
                "{invalid:?} should be rejected"
            );
        }
        assert!(
            validate_domain_allowlist(&["Example.com".to_string(), "example.com".to_string()])
                .is_err()
        );
        assert!(
            validate_domain_allowlist(
                &(0..=MAX_ALLOWLIST_PATTERNS)
                    .map(|index| format!("host-{index}.example.com"))
                    .collect::<Vec<_>>()
            )
            .is_err()
        );

        let fail_closed = DomainAllowlist::new(&["*.".to_string()]);
        assert!(!fail_closed.is_allowed("evil.example.").is_allowed());
        let invalid_pattern = DomainPattern::new("*.");
        assert!(!invalid_pattern.matches("evil.example."));
    }

    #[test]
    fn test_wildcard_match() {
        let pattern = DomainPattern::new("*.example.com");
        assert!(pattern.matches("api.example.com"));
        assert!(pattern.matches("foo.bar.example.com"));
        assert!(pattern.matches("example.com")); // Base domain also matches
        assert!(!pattern.matches("exampleXcom"));
        assert!(!pattern.matches("other.com"));
    }

    #[test]
    fn test_allowlist_allows() {
        let allowlist =
            DomainAllowlist::new(&["crates.io".to_string(), "*.github.com".to_string()]);

        assert!(allowlist.is_allowed("crates.io").is_allowed());
        assert!(allowlist.is_allowed("api.github.com").is_allowed());
        assert!(
            !allowlist
                .is_allowed("raw.githubusercontent.com")
                .is_allowed()
        );
    }

    #[test]
    fn test_allowlist_denies() {
        let allowlist = DomainAllowlist::new(&["crates.io".to_string()]);

        let result = allowlist.is_allowed("evil.com");
        assert!(!result.is_allowed());
    }

    #[test]
    fn test_empty_allowlist() {
        let allowlist = DomainAllowlist::empty();
        assert!(!allowlist.is_allowed("anything.com").is_allowed());
    }

    #[test]
    fn test_extract_host() {
        assert_eq!(
            extract_host("https://api.example.com/v1/endpoint"),
            Some("api.example.com".to_string())
        );
        assert_eq!(
            extract_host("http://localhost:8080/api"),
            Some("localhost".to_string())
        );
        assert_eq!(
            extract_host("https://EXAMPLE.COM"),
            Some("example.com".to_string())
        );
        assert_eq!(
            extract_host("https://user:pass@api.example.com:443/path"),
            Some("api.example.com".to_string())
        );
        assert_eq!(
            extract_host("http://[::1]:8080/path"),
            Some("::1".to_string())
        );
        assert_eq!(extract_host("not-a-url"), None);
        assert_eq!(extract_host("ftp://example.com/file"), None);
    }
}
