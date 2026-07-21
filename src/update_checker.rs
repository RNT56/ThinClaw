//! Auto-update checker.
//!
//! Periodically checks GitHub Releases for new versions and reports
//! availability to the user. Does NOT auto-install — just notifies.
//!
//! Design:
//! - Background `tokio` task polling every 24h
//! - Uses the GitHub REST API to check for newer releases
//! - Compares semver against `CARGO_PKG_VERSION`
//! - Sends update notification through the event system

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

const MAX_RELEASE_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_RELEASES: usize = 100;
const MAX_RELEASE_NOTES_BYTES: usize = 128 * 1024;

/// GitHub release info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub name: Option<String>,
    pub html_url: String,
    pub published_at: Option<String>,
    pub body: Option<String>,
    pub prerelease: bool,
    pub draft: bool,
}

/// Update check result.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateStatus {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub release_url: Option<String>,
    pub release_notes: Option<String>,
    pub last_checked: Option<String>,
}

/// Configuration for the auto-update checker.
#[derive(Debug, Clone)]
pub struct UpdateCheckerConfig {
    /// GitHub owner/repo (e.g., "RNT56/ThinClaw")
    pub github_repo: String,
    /// Check interval (default: 24 hours)
    pub check_interval: Duration,
    /// Whether to include pre-releases
    pub include_prereleases: bool,
}

impl Default for UpdateCheckerConfig {
    fn default() -> Self {
        Self {
            github_repo: "RNT56/ThinClaw".to_string(),
            check_interval: Duration::from_secs(24 * 60 * 60), // 24 hours
            include_prereleases: false,
        }
    }
}

/// Auto-update checker that runs as a background task.
pub struct UpdateChecker {
    config: UpdateCheckerConfig,
    client: Client,
    status_tx: watch::Sender<UpdateStatus>,
    status_rx: watch::Receiver<UpdateStatus>,
}

impl UpdateChecker {
    /// Create a new update checker.
    pub fn new(config: UpdateCheckerConfig) -> Self {
        let initial_status = UpdateStatus {
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            latest_version: None,
            update_available: false,
            release_url: None,
            release_notes: None,
            last_checked: None,
        };
        let (status_tx, status_rx) = watch::channel(initial_status);

        Self {
            config,
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(10))
                .user_agent(format!("thinclaw/{}", env!("CARGO_PKG_VERSION")))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()
                .expect("static update-checker HTTP client configuration"),
            status_tx,
            status_rx,
        }
    }

    /// Get the current update status.
    pub fn status(&self) -> UpdateStatus {
        self.status_rx.borrow().clone()
    }

    /// Get a receiver to watch for status changes.
    pub fn subscribe(&self) -> watch::Receiver<UpdateStatus> {
        self.status_rx.clone()
    }

    /// Perform a single update check.
    pub async fn check_now(&self) -> Result<UpdateStatus, String> {
        let (owner, repo) = parse_github_repo(&self.config.github_repo)
            .ok_or_else(|| "GitHub repository must be an owner/repo pair".to_string())?;
        let mut url = reqwest::Url::parse("https://api.github.com/repos")
            .map_err(|error| format!("Invalid GitHub API base URL: {error}"))?;
        url.path_segments_mut()
            .map_err(|_| "Invalid GitHub API base URL".to_string())?
            .extend([owner, repo, "releases"]);
        url.query_pairs_mut()
            .append_pair("per_page", &MAX_RELEASES.to_string());

        let resp = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await
            .map_err(|e| format!("GitHub API: {}", e.without_url()))?;

        if !resp.status().is_success() {
            return Err(format!("GitHub API returned {}", resp.status()));
        }

        let releases: Vec<ReleaseInfo> =
            crate::http_response::bounded_json(resp, MAX_RELEASE_RESPONSE_BYTES)
                .await
                .map_err(|e| format!("Parse releases: {e}"))?;

        // Find the latest non-draft release (optionally including pre-releases)
        let latest = releases.into_iter().take(MAX_RELEASES).find_map(|release| {
            if release.draft
                || (!self.config.include_prereleases && release.prerelease)
                || release.tag_name.len() > 128
            {
                return None;
            }
            let version = semver::Version::parse(release.tag_name.trim_start_matches('v')).ok()?;
            let release_url = valid_release_page_url(&release.html_url, owner, repo)
                .then_some(release.html_url.clone());
            Some((release, version, release_url))
        });

        let current = env!("CARGO_PKG_VERSION");
        let now = chrono::Utc::now().to_rfc3339();

        let status = match latest {
            Some((release, version, release_url)) => {
                let tag = version.to_string();
                let update_available = is_newer(&tag, current);

                UpdateStatus {
                    current_version: current.to_string(),
                    latest_version: Some(tag),
                    update_available,
                    release_url,
                    release_notes: release
                        .body
                        .as_deref()
                        .map(|notes| truncate_utf8(notes, MAX_RELEASE_NOTES_BYTES)),
                    last_checked: Some(now),
                }
            }
            None => UpdateStatus {
                current_version: current.to_string(),
                latest_version: None,
                update_available: false,
                release_url: None,
                release_notes: None,
                last_checked: Some(now),
            },
        };

        let _ = self.status_tx.send(status.clone());
        Ok(status)
    }

    /// Start the background check loop. Returns a `JoinHandle` for the task.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        let interval = self.config.check_interval.max(Duration::from_secs(60));

        tokio::spawn(async move {
            loop {
                match self.check_now().await {
                    Ok(status) => {
                        if status.update_available {
                            tracing::info!(
                                "🆕 Update available: {} → {} — {}",
                                status.current_version,
                                status.latest_version.as_deref().unwrap_or("?"),
                                status.release_url.as_deref().unwrap_or(""),
                            );
                        } else {
                            tracing::debug!(
                                "Update check: current {} is latest",
                                status.current_version,
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Update check failed: {e}");
                    }
                }

                tokio::time::sleep(interval).await;
            }
        })
    }
}

fn parse_github_repo(value: &str) -> Option<(&str, &str)> {
    let mut parts = value.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    let valid = |part: &str| {
        !part.is_empty()
            && part.len() <= 100
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    (parts.next().is_none() && valid(owner) && valid(repo)).then_some((owner, repo))
}

fn valid_release_page_url(value: &str, owner: &str, repo: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let Some(parts) = url.path_segments().map(|parts| parts.collect::<Vec<_>>()) else {
        return false;
    };
    parts.len() >= 5
        && parts[0].eq_ignore_ascii_case(owner)
        && parts[1].eq_ignore_ascii_case(repo)
        && parts[2] == "releases"
        && parts[3] == "tag"
        && !parts[4].is_empty()
}

fn truncate_utf8(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut boundary = limit;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_string()
}

/// SemVer comparison: returns true if `remote` > `local`.
fn is_newer(remote: &str, local: &str) -> bool {
    let Ok(remote) = semver::Version::parse(remote.trim_start_matches('v')) else {
        return false;
    };
    let Ok(local) = semver::Version::parse(local.trim_start_matches('v')) else {
        return false;
    };
    remote > local
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("1.1.0", "1.0.0"));
        assert!(is_newer("2.0.0", "1.99.99"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.1"));
    }

    #[test]
    fn test_is_newer_with_prefix() {
        // The v prefix is stripped before calling is_newer
        assert!(is_newer("1.1.0", "1.0.0"));
    }

    #[test]
    fn test_is_newer_with_prerelease() {
        assert!(is_newer("1.1.0-beta", "1.0.0"));
        assert!(!is_newer("1.0.0-beta", "1.0.0"));
        assert!(is_newer("1.0.0", "1.0.0-beta.1"));
        assert!(!is_newer("not-semver", "1.0.0"));
    }

    #[test]
    fn github_repo_and_release_url_are_strictly_scoped() {
        assert_eq!(
            parse_github_repo("RNT56/ThinClaw"),
            Some(("RNT56", "ThinClaw"))
        );
        assert!(parse_github_repo("RNT56/ThinClaw/../../admin").is_none());
        assert!(parse_github_repo("RNT56/%2fadmin").is_none());
        assert!(valid_release_page_url(
            "https://github.com/RNT56/ThinClaw/releases/tag/v1.0.0",
            "RNT56",
            "ThinClaw"
        ));
        assert!(!valid_release_page_url(
            "https://github.com/attacker/ThinClaw/releases/tag/v1.0.0",
            "RNT56",
            "ThinClaw"
        ));
    }

    #[test]
    fn test_default_config() {
        let config = UpdateCheckerConfig::default();
        assert_eq!(config.github_repo, "RNT56/ThinClaw");
        assert_eq!(config.check_interval, Duration::from_secs(86400));
        assert!(!config.include_prereleases);
    }

    #[test]
    fn test_checker_initial_status() {
        let checker = UpdateChecker::new(UpdateCheckerConfig::default());
        let status = checker.status();
        assert_eq!(status.current_version, env!("CARGO_PKG_VERSION"));
        assert!(!status.update_available);
        assert!(status.latest_version.is_none());
    }

    #[test]
    fn test_update_status_serialization() {
        let status = UpdateStatus {
            current_version: "1.0.0".to_string(),
            latest_version: Some("1.1.0".to_string()),
            update_available: true,
            release_url: Some("https://example.com".to_string()),
            release_notes: Some("Bug fixes".to_string()),
            last_checked: Some("2026-01-01T00:00:00Z".to_string()),
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["update_available"], true);
        assert_eq!(json["latest_version"], "1.1.0");
    }
}
