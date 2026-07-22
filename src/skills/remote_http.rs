//! Hardened HTTP transport for community-controlled skill registries.

use std::time::Duration;

use reqwest::{Response, Url};
use thinclaw_tools_core::{OutboundUrlGuardOptions, validate_outbound_url_pinned_async};

const USER_AGENT: &str = concat!("thinclaw/", env!("CARGO_PKG_VERSION"));

async fn validate_public_https_url(
    value: &str,
) -> Result<thinclaw_tools_core::GuardedUrl, thinclaw_tools_core::ToolError> {
    validate_outbound_url_pinned_async(
        value,
        &OutboundUrlGuardOptions {
            require_https: true,
            upgrade_http_to_https: false,
            allowlist: Vec::new(),
        },
    )
    .await
}

/// Fetch an untrusted community-registry URL without exposing the local network.
///
/// Validation is repeated for every request because manifest URLs originate in
/// remote index data. The approved DNS answers are pinned into the client to
/// close the rebinding window, redirects are disabled, and ambient HTTP proxy
/// configuration is ignored so a proxy cannot re-resolve the untrusted host.
pub(super) async fn get_public_https(value: &str, timeout: Duration) -> anyhow::Result<Response> {
    let timeout = timeout
        .max(Duration::from_millis(1))
        .min(Duration::from_secs(5 * 60));
    let deadline = tokio::time::Instant::now() + timeout;
    let guarded = validate_public_https_url(value).await?;
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    anyhow::ensure!(
        !remaining.is_zero(),
        "skill registry request timed out during URL validation"
    );

    let host = guarded
        .url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("skill URL is missing a host"))?
        .to_string();
    let mut builder = reqwest::Client::builder()
        .timeout(remaining)
        .connect_timeout(remaining.min(Duration::from_secs(10)))
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if !guarded.pinned_addrs.is_empty() {
        builder = builder.resolve_to_addrs(&host, &guarded.pinned_addrs);
    }

    let response = builder
        .build()
        .map_err(|error| anyhow::anyhow!("could not build skill registry client: {error}"))?
        .get(guarded.url)
        .send()
        .await
        .map_err(|error| {
            anyhow::anyhow!("skill registry request failed: {}", error.without_url())
        })?;
    anyhow::ensure!(
        response.status().is_success(),
        "skill registry returned HTTP {}",
        response.status()
    );
    Ok(response)
}

pub(super) fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn public_url_guard_rejects_local_and_insecure_targets() {
        assert!(
            validate_public_https_url("http://example.com/skill")
                .await
                .is_err()
        );
        assert!(
            validate_public_https_url("https://127.0.0.1/skill")
                .await
                .is_err()
        );
        assert!(
            validate_public_https_url("https://[::1]/skill")
                .await
                .is_err()
        );
        assert!(
            validate_public_https_url("https://user:pass@example.com/skill")
                .await
                .is_err()
        );
    }

    #[test]
    fn origin_comparison_includes_effective_port() {
        let implicit = Url::parse("https://example.com/a").unwrap();
        let explicit = Url::parse("https://example.com:443/b").unwrap();
        let other_port = Url::parse("https://example.com:444/b").unwrap();
        assert!(same_origin(&implicit, &explicit));
        assert!(!same_origin(&implicit, &other_port));
    }
}
