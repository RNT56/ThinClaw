//! Constrained credential-validation requests for extension setup.
//!
//! Capability metadata is extension-controlled. Validation therefore stays
//! inside the extension's declared HTTP grant, rejects private/rebinding DNS,
//! disables redirects, and never places credentials in a URL or error body.

use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};
use thinclaw_tools_core::{OutboundUrlGuardOptions, ToolError, validate_outbound_url_pinned_async};

const MAX_VALIDATION_URL_BYTES: usize = 16 * 1024;
const MAX_VALIDATION_SECRET_BYTES: usize = 64 * 1024;
const MAX_VALIDATION_GRANTS: usize = 256;

#[derive(Debug, Clone)]
pub(crate) struct ValidationEndpointGrant {
    pub host: String,
    pub path_prefix: Option<String>,
    pub methods: Vec<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum ValidationCredential<'a> {
    None,
    Bearer(&'a str),
    Basic {
        username: &'a str,
        password: &'a str,
    },
    Header {
        name: &'a str,
        prefix: Option<&'a str>,
        value: &'a str,
    },
}

impl std::fmt::Debug for ValidationCredential<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::Bearer(_) => formatter.write_str("Bearer([REDACTED])"),
            Self::Basic { username, .. } => formatter
                .debug_struct("Basic")
                .field("username", username)
                .field("password", &"[REDACTED]")
                .finish(),
            Self::Header { name, prefix, .. } => formatter
                .debug_struct("Header")
                .field("name", name)
                .field("prefix", prefix)
                .field("value", &"[REDACTED]")
                .finish(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ValidationRequestError {
    #[error("invalid validation request: {0}")]
    Invalid(String),
    #[error("validation endpoint is outside the extension HTTP grant")]
    NotGranted,
    #[error("validation request failed: {0}")]
    Request(String),
    #[error("validation returned HTTP {actual}; expected {expected}")]
    Status { actual: u16, expected: u16 },
}

fn canonical_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

fn host_matches(pattern: &str, host: &str) -> bool {
    let pattern = canonical_host(pattern);
    let host = canonical_host(host);
    if pattern == "*" {
        return false;
    }
    if pattern == host {
        return true;
    }
    pattern
        .strip_prefix("*.")
        .is_some_and(|suffix| host.len() > suffix.len() && host.ends_with(&format!(".{suffix}")))
}

fn grant_matches(grant: &ValidationEndpointGrant, host: &str, path: &str, method: &str) -> bool {
    host_matches(&grant.host, host)
        && grant
            .path_prefix
            .as_deref()
            .is_none_or(|prefix| path.starts_with(prefix))
        && (grant.methods.is_empty()
            || grant
                .methods
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(method)))
}

pub(crate) fn ensure_endpoint_is_granted(
    url: &str,
    method: &str,
    grants: &[ValidationEndpointGrant],
) -> Result<(), ValidationRequestError> {
    let parsed = url::Url::parse(url).map_err(|error| {
        ValidationRequestError::Invalid(format!("endpoint is not a valid URL: {error}"))
    })?;
    let host = parsed
        .host_str()
        .ok_or_else(|| ValidationRequestError::Invalid("endpoint has no hostname".to_string()))?;
    if !grants
        .iter()
        .any(|grant| grant_matches(grant, host, parsed.path(), method))
    {
        return Err(ValidationRequestError::NotGranted);
    }
    Ok(())
}

fn validate_secret(value: &str) -> Result<(), ValidationRequestError> {
    if value.is_empty()
        || value.len() > MAX_VALIDATION_SECRET_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(ValidationRequestError::Invalid(
            "credential is empty, malformed, or oversized".to_string(),
        ));
    }
    Ok(())
}

fn map_guard_error(error: ToolError) -> ValidationRequestError {
    ValidationRequestError::Invalid(error.to_string())
}

pub(crate) async fn validate_extension_credential(
    url: &str,
    method: &str,
    expected_status: u16,
    grants: &[ValidationEndpointGrant],
    credential: ValidationCredential<'_>,
) -> Result<(), ValidationRequestError> {
    if url.is_empty()
        || url.len() > MAX_VALIDATION_URL_BYTES
        || url.contains('{')
        || url.contains('}')
        || grants.is_empty()
        || grants.len() > MAX_VALIDATION_GRANTS
    {
        return Err(ValidationRequestError::Invalid(
            "endpoint is empty, templated, oversized, or has no bounded HTTP grant".to_string(),
        ));
    }
    let method = method.to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "POST") {
        return Err(ValidationRequestError::Invalid(
            "only GET and POST validation requests are supported".to_string(),
        ));
    }
    if !(100..=599).contains(&expected_status) {
        return Err(ValidationRequestError::Invalid(
            "expected status is outside the HTTP status range".to_string(),
        ));
    }

    let options = OutboundUrlGuardOptions {
        require_https: true,
        upgrade_http_to_https: false,
        allowlist: Vec::new(),
    };
    let guarded = validate_outbound_url_pinned_async(url, &options)
        .await
        .map_err(map_guard_error)?;
    if !guarded.url.username().is_empty()
        || guarded.url.password().is_some()
        || guarded.url.fragment().is_some()
    {
        return Err(ValidationRequestError::Invalid(
            "endpoint contains credentials or a fragment".to_string(),
        ));
    }
    let host = guarded
        .url
        .host_str()
        .ok_or_else(|| ValidationRequestError::Invalid("endpoint has no hostname".to_string()))?;
    ensure_endpoint_is_granted(guarded.url.as_str(), &method, grants)?;

    let mut client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if !guarded.pinned_addrs.is_empty() {
        let addresses = guarded.pinned_addrs.clone();
        client = client.resolve_to_addrs(host, &addresses);
    }
    let client = client.build().map_err(|error| {
        ValidationRequestError::Request(format!("could not build HTTP client: {error}"))
    })?;
    let mut request = match method.as_str() {
        "GET" => client.get(guarded.url),
        "POST" => client.post(guarded.url).body(Vec::new()),
        _ => unreachable!("method was validated above"),
    };
    request = match credential {
        ValidationCredential::None => request,
        ValidationCredential::Bearer(value) => {
            validate_secret(value)?;
            request.bearer_auth(value)
        }
        ValidationCredential::Basic { username, password } => {
            validate_secret(username)?;
            validate_secret(password)?;
            request.basic_auth(username, Some(password))
        }
        ValidationCredential::Header {
            name,
            prefix,
            value,
        } => {
            validate_secret(value)?;
            if prefix
                .is_some_and(|prefix| prefix.len() > 256 || prefix.chars().any(char::is_control))
            {
                return Err(ValidationRequestError::Invalid(
                    "credential header prefix is malformed or oversized".to_string(),
                ));
            }
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                ValidationRequestError::Invalid("credential header name is invalid".to_string())
            })?;
            if matches!(
                name.as_str(),
                "host"
                    | "content-length"
                    | "connection"
                    | "transfer-encoding"
                    | "upgrade"
                    | "proxy-authorization"
                    | "proxy-authenticate"
            ) {
                return Err(ValidationRequestError::Invalid(
                    "credential cannot use a transport-controlled header".to_string(),
                ));
            }
            let combined = format!("{}{}", prefix.unwrap_or_default(), value);
            let value = HeaderValue::from_str(&combined).map_err(|_| {
                ValidationRequestError::Invalid("credential header value is invalid".to_string())
            })?;
            request.header(name, value)
        }
    };

    let response = request.send().await.map_err(|error| {
        ValidationRequestError::Request(format!("endpoint request failed: {}", error.without_url()))
    })?;
    let actual = response.status().as_u16();
    if actual != expected_status {
        return Err(ValidationRequestError::Status {
            actual,
            expected: expected_status,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_grants_require_domain_boundaries_and_reject_universal_wildcards() {
        assert!(host_matches("api.example.com", "api.example.com"));
        assert!(host_matches("*.example.com", "api.example.com"));
        assert!(!host_matches("*.example.com", "evil-example.com"));
        assert!(!host_matches("*", "api.example.com"));
    }

    #[tokio::test]
    async fn validation_rejects_templates_and_private_endpoints_before_network() {
        let grant = ValidationEndpointGrant {
            host: "localhost".to_string(),
            path_prefix: None,
            methods: vec!["GET".to_string()],
        };
        assert!(
            validate_extension_credential(
                "https://example.com/{secret}",
                "GET",
                200,
                std::slice::from_ref(&grant),
                ValidationCredential::None,
            )
            .await
            .is_err()
        );
        assert!(
            validate_extension_credential(
                "http://127.0.0.1:1234/check",
                "GET",
                200,
                &[grant],
                ValidationCredential::None,
            )
            .await
            .is_err()
        );
    }
}
