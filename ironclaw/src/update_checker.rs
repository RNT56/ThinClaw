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
                .user_agent(format!("thinclaw/{}", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_default(),
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
        let url = format!(
            "https://api.github.com/repos/{}/releases",
            self.config.github_repo
        );

        let resp = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await
            .map_err(|e| format!("GitHub API: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("GitHub API returned {}", resp.status()));
        }

        let releases: Vec<ReleaseInfo> = resp
            .json()
            .await
            .map_err(|e| format!("Parse releases: {e}"))?;

        // Find the latest non-draft release (optionally including pre-releases)
        let latest = releases
            .iter()
            .find(|r| !r.draft && (self.config.include_prereleases || !r.prerelease));

        let current = env!("CARGO_PKG_VERSION");
        let now = chrono::Utc::now().to_rfc3339();

        let status = match latest {
            Some(release) => {
                let tag = release
                    .tag_name
                    .strip_prefix('v')
                    .unwrap_or(&release.tag_name);

                let update_available = is_newer(tag, current);

                UpdateStatus {
                    current_version: current.to_string(),
                    latest_version: Some(tag.to_string()),
                    update_available,
                    release_url: Some(release.html_url.clone()),
                    release_notes: release.body.clone(),
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
        let interval = self.config.check_interval;

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

/// Simple semver comparison: returns true if `remote` > `local`.
fn is_newer(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> (u64, u64, u64) {
        let mut parts = s.split('.');
        let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts
            .next()
            .and_then(|p| {
                // Handle pre-release suffixes like "1.2.3-beta"
                p.split('-').next().and_then(|v| v.parse().ok())
            })
            .unwrap_or(0);
        (major, minor, patch)
    };

    let r = parse(remote);
    let l = parse(local);
    r > l
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
