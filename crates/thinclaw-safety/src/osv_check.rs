//! OSV malware check for MCP extension packages.
//!
//! Queries the Google OSV API to check whether npm/PyPI packages have
//! known malware advisories before allowing MCP server launches.
//! Fail-open: network errors allow the package to proceed.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

/// Default OSV API endpoint.
const DEFAULT_OSV_ENDPOINT: &str = "https://api.osv.dev/v1/query";

/// Cache TTL for OSV check results (1 hour).
const CACHE_TTL: Duration = Duration::from_secs(3600);

/// Request timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OSV_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Result of an OSV malware check.
#[derive(Debug, Clone)]
pub enum OsvCheckResult {
    /// Package is clean (no malware advisories).
    Clean,
    /// Malware found — contains advisory IDs and summaries.
    MalwareFound(Vec<MalwareAdvisory>),
    /// Check failed (network error, timeout, etc.) — fail-open.
    CheckFailed(String),
    /// Checks are disabled via env var.
    Disabled,
}

impl OsvCheckResult {
    /// Whether this result blocks the package from loading.
    pub fn should_block(&self) -> bool {
        matches!(self, OsvCheckResult::MalwareFound(_))
    }
}

/// A malware advisory from OSV.
#[derive(Debug, Clone)]
pub struct MalwareAdvisory {
    /// Advisory ID (e.g., "MAL-2024-1234").
    pub id: String,
    /// Short summary.
    pub summary: String,
}

/// Cached check result.
struct CachedResult {
    result: OsvCheckResult,
    cached_at: Instant,
}

/// OSV malware checker with caching.
pub struct OsvChecker {
    endpoint: String,
    cache: Arc<RwLock<HashMap<String, CachedResult>>>,
    disabled: bool,
}

impl OsvChecker {
    /// Create a new checker, reading config from environment.
    pub fn new() -> Self {
        let endpoint =
            std::env::var("OSV_ENDPOINT").unwrap_or_else(|_| DEFAULT_OSV_ENDPOINT.to_string());

        let disabled = std::env::var("OSV_CHECK_DISABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        Self {
            endpoint,
            cache: Arc::new(RwLock::new(HashMap::new())),
            disabled,
        }
    }

    /// Check a package for malware advisories.
    ///
    /// Infers the ecosystem from the command used to launch the MCP server.
    pub async fn check_package(&self, command: &str, args: &[String]) -> OsvCheckResult {
        if self.disabled {
            return OsvCheckResult::Disabled;
        }

        // Infer ecosystem and package name
        let (ecosystem, package_name) = match infer_package(command, args) {
            Some(info) => info,
            None => {
                tracing::debug!(command, "OSV check: could not infer package from command");
                return OsvCheckResult::Clean; // Can't check, fail-open
            }
        };

        let cache_key = format!("{}:{}", ecosystem, package_name);

        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(&cache_key)
                && cached.cached_at.elapsed() < CACHE_TTL
            {
                tracing::debug!(
                    package = %package_name,
                    ecosystem = %ecosystem,
                    "OSV check: cache hit"
                );
                return cached.result.clone();
            }
        }

        // Query OSV API
        let result = self.query_osv(&ecosystem, &package_name).await;

        // Cache the result
        {
            let mut cache = self.cache.write().await;
            cache.insert(
                cache_key,
                CachedResult {
                    result: result.clone(),
                    cached_at: Instant::now(),
                },
            );
        }

        result
    }

    /// Query the OSV API for a specific package.
    async fn query_osv(&self, ecosystem: &str, package_name: &str) -> OsvCheckResult {
        let body = serde_json::json!({
            "package": {
                "name": package_name,
                "ecosystem": ecosystem,
            }
        });

        let guarded = match thinclaw_tools_core::validate_outbound_url_pinned_async(
            &self.endpoint,
            &thinclaw_tools_core::OutboundUrlGuardOptions {
                require_https: true,
                upgrade_http_to_https: false,
                allowlist: Vec::new(),
            },
        )
        .await
        {
            Ok(guarded) => guarded,
            Err(error) => return OsvCheckResult::CheckFailed(error.to_string()),
        };
        let Some(host) = guarded.url.host_str() else {
            return OsvCheckResult::CheckFailed("OSV endpoint has no host".to_string());
        };
        let mut builder = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        if !guarded.pinned_addrs.is_empty() {
            builder = builder.resolve_to_addrs(host, &guarded.pinned_addrs);
        }
        let client = match builder.build() {
            Ok(client) => client,
            Err(error) => return OsvCheckResult::CheckFailed(error.to_string()),
        };
        let response = match client.post(guarded.url).json(&body).send().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!(
                    package = %package_name,
                    error = %e,
                    "OSV check: request failed (fail-open)"
                );
                return OsvCheckResult::CheckFailed(e.to_string());
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            tracing::warn!(
                package = %package_name,
                status = %status,
                "OSV check: non-200 response (fail-open)"
            );
            return OsvCheckResult::CheckFailed(format!("HTTP {}", status));
        }

        let mut response = response;
        if response.content_length().is_some_and(|length| {
            usize::try_from(length).map_or(true, |length| length > MAX_OSV_RESPONSE_BYTES)
        }) {
            return OsvCheckResult::CheckFailed("OSV response is oversized".to_string());
        }
        let mut response_bytes = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if response_bytes.len().saturating_add(chunk.len()) > MAX_OSV_RESPONSE_BYTES {
                        return OsvCheckResult::CheckFailed(
                            "OSV response is oversized".to_string(),
                        );
                    }
                    response_bytes.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(error) => return OsvCheckResult::CheckFailed(error.to_string()),
            }
        }
        let data: serde_json::Value = match serde_json::from_slice(&response_bytes) {
            Ok(value) => value,
            Err(e) => {
                tracing::warn!(
                    package = %package_name,
                    error = %e,
                    "OSV check: response parse failed (fail-open)"
                );
                return OsvCheckResult::CheckFailed(e.to_string());
            }
        };

        // Check for malware advisories (MAL-* prefix)
        let vulns = data
            .get("vulns")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let malware: Vec<MalwareAdvisory> = vulns
            .iter()
            .filter_map(|vuln| {
                let id = vuln.get("id")?.as_str()?;
                // Only block on malware advisories (MAL-*), not regular CVEs
                if id.starts_with("MAL-") {
                    let summary = vuln
                        .get("summary")
                        .and_then(|s| s.as_str())
                        .unwrap_or("Malware advisory")
                        .to_string();
                    Some(MalwareAdvisory {
                        id: id.to_string(),
                        summary,
                    })
                } else {
                    None
                }
            })
            .collect();

        if malware.is_empty() {
            tracing::debug!(
                package = %package_name,
                ecosystem = %ecosystem,
                total_vulns = vulns.len(),
                "OSV check: clean (no malware)"
            );
            OsvCheckResult::Clean
        } else {
            tracing::warn!(
                package = %package_name,
                ecosystem = %ecosystem,
                malware_count = malware.len(),
                "OSV check: MALWARE FOUND"
            );
            OsvCheckResult::MalwareFound(malware)
        }
    }
}

impl Default for OsvChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Infer ecosystem and package name from the MCP server launch command.
///
/// Returns `(ecosystem, package_name)` or None if we can't infer.
fn infer_package(command: &str, args: &[String]) -> Option<(String, String)> {
    let cmd_basename = command.rsplit('/').next().unwrap_or(command);

    match cmd_basename {
        "npx" | "npx.cmd" | "bunx" => {
            // npx -y @scope/package-name
            // npx package-name
            let package = args
                .iter()
                .find(|a| !a.starts_with('-') && !a.starts_with("--"))
                .or_else(|| args.last())?;

            // Strip version specifier: @scope/pkg@1.2.3 → @scope/pkg
            let name = strip_version(package);
            Some(("npm".to_string(), name))
        }
        "uvx" | "pipx" | "pip" => {
            // uvx package-name
            // pipx run package-name
            let package = args
                .iter()
                .find(|a| !a.starts_with('-') && *a != "run" && *a != "install")
                .or_else(|| args.last())?;

            let name = strip_version(package);
            Some(("PyPI".to_string(), name))
        }
        _ => None,
    }
}

/// Strip version specifier from a package name.
/// "package@1.2.3" → "package", "@scope/pkg@2.0.0" → "@scope/pkg"
fn strip_version(name: &str) -> String {
    if name.starts_with('@') {
        // Scoped package: @scope/name@version
        if let Some(slash_pos) = name.find('/') {
            let after_slash = &name[slash_pos + 1..];
            if let Some(at_pos) = after_slash.find('@') {
                return format!("{}/{}", &name[..slash_pos], &after_slash[..at_pos]);
            }
        }
        name.to_string()
    } else if let Some(at_pos) = name.find('@') {
        name[..at_pos].to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn lock_env() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn test_infer_npm_package() {
        let result = infer_package("npx", &["-y".to_string(), "some-tool".to_string()]);
        assert_eq!(result, Some(("npm".to_string(), "some-tool".to_string())));
    }

    #[test]
    fn test_infer_npm_scoped() {
        let result = infer_package(
            "npx",
            &[
                "-y".to_string(),
                "@modelcontextprotocol/server-fetch".to_string(),
            ],
        );
        assert_eq!(
            result,
            Some((
                "npm".to_string(),
                "@modelcontextprotocol/server-fetch".to_string()
            ))
        );
    }

    #[test]
    fn test_infer_npm_with_version() {
        let result = infer_package("npx", &["-y".to_string(), "@scope/pkg@1.2.3".to_string()]);
        assert_eq!(result, Some(("npm".to_string(), "@scope/pkg".to_string())));
    }

    #[test]
    fn test_infer_pypi_package() {
        let result = infer_package("uvx", &["mcp-server-sqlite".to_string()]);
        assert_eq!(
            result,
            Some(("PyPI".to_string(), "mcp-server-sqlite".to_string()))
        );
    }

    #[test]
    fn test_infer_unknown_command() {
        let result = infer_package("python", &["server.py".to_string()]);
        assert!(result.is_none());
    }

    #[test]
    fn test_strip_version_simple() {
        assert_eq!(strip_version("package@1.2.3"), "package");
    }

    #[test]
    fn test_strip_version_scoped() {
        assert_eq!(strip_version("@scope/name@2.0.0"), "@scope/name");
    }

    #[test]
    fn test_strip_version_no_version() {
        assert_eq!(strip_version("package"), "package");
    }

    #[test]
    fn test_strip_version_scoped_no_version() {
        assert_eq!(strip_version("@scope/name"), "@scope/name");
    }

    #[test]
    fn test_osv_check_result_should_block() {
        assert!(!OsvCheckResult::Clean.should_block());
        assert!(!OsvCheckResult::Disabled.should_block());
        assert!(!OsvCheckResult::CheckFailed("err".into()).should_block());
        assert!(
            OsvCheckResult::MalwareFound(vec![MalwareAdvisory {
                id: "MAL-2024-001".into(),
                summary: "test".into(),
            }])
            .should_block()
        );
    }

    #[test]
    fn test_checker_disabled() {
        let _env_guard = lock_env();
        unsafe {
            std::env::set_var("OSV_CHECK_DISABLED", "true");
        }
        let checker = OsvChecker::new();
        assert!(checker.disabled);
        unsafe {
            std::env::remove_var("OSV_CHECK_DISABLED");
        }
    }
}
