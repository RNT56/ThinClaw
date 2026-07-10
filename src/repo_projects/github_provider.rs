//! Authenticated GitHub API client construction for the repo project supervisor.
//!
//! The supervisor needs a real, authenticated [`GitHubApiClient`] for each
//! enrolled repository so it can drive the PR / CI / merge pipeline. This module
//! isolates *credential resolution* (GitHub App installation token vs. the
//! `github_token` fallback) and *base-URL selection* (so integration tests can
//! point a real client at an in-process fake GitHub server) behind a single
//! trait. Nothing here touches task state — it only mints clients.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_repo_projects::{GitHubAuthMode, RepoProjectRepo};

use super::github::{GitHubApiClient, GitHubApiResilience, GitHubAppConfig, GitHubAppTokenCache};

/// Mints an authenticated [`GitHubApiClient`] for a given enrolled repository.
#[async_trait]
pub trait RepoGitHubClientProvider: Send + Sync {
    async fn client_for(&self, repo: &RepoProjectRepo) -> Result<GitHubApiClient, String>;

    /// Mint a client for installation/account-level *discovery* calls that are
    /// not scoped to a single enrolled repo — e.g. listing the repos an App
    /// installation can access so the connector UI can offer a repo picker.
    ///
    /// Returns the resolved [`GitHubAuthMode`] alongside the client so the caller
    /// knows which endpoint to hit: GitHub App auth lists
    /// `/installation/repositories`; a personal-access token lists `/user/repos`.
    async fn discovery_client(&self) -> Result<(GitHubApiClient, GitHubAuthMode), String>;
}

/// Production provider: resolves a GitHub App installation token (preferred) or
/// falls back to a personal-access `github_token` pulled from the secrets store.
pub struct SecretsRepoGitHubClientProvider {
    secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
    user_id: String,
    api_base_url: String,
    /// Shared App token cache, present only when an App id + private key are
    /// configured and the private key resolved successfully at startup.
    app_token_cache: Option<Arc<GitHubAppTokenCache>>,
    /// Installation id used when a repo does not pin its own.
    default_installation_id: Option<i64>,
    /// Name of the PAT secret used for the non-App fallback path.
    fallback_token_secret: String,
    /// Shared across short-lived clients so retries from consecutive
    /// reconciles participate in one circuit breaker.
    resilience: GitHubApiResilience,
}

impl SecretsRepoGitHubClientProvider {
    /// Build a provider, resolving the GitHub App private key from the secrets
    /// store when an App is configured. A missing/unreadable key is *not* fatal:
    /// the provider silently degrades to the `github_token` fallback path so a
    /// misconfigured App cannot take the whole supervisor offline.
    pub async fn build(
        secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
        user_id: impl Into<String>,
        api_base_url: impl Into<String>,
        app_id: Option<u64>,
        installation_id: Option<u64>,
        private_key_secret: Option<String>,
        fallback_token_secret: impl Into<String>,
    ) -> Self {
        let user_id = user_id.into();
        let api_base_url = api_base_url.into();
        let default_installation_id = installation_id.and_then(|id| i64::try_from(id).ok());

        let app_token_cache = match (app_id, private_key_secret.as_deref()) {
            (Some(app_id), Some(secret_name)) if !secret_name.trim().is_empty() => {
                match i64::try_from(app_id) {
                    Ok(app_id) => {
                        match resolve_secret(
                            secrets.as_ref(),
                            &user_id,
                            secret_name,
                            "load_app_private_key",
                        )
                        .await
                        {
                            Ok(pem) if !pem.trim().is_empty() => {
                                let config = GitHubAppConfig {
                                    app_id,
                                    private_key_pem: pem,
                                    api_base_url: api_base_url.clone(),
                                };
                                Some(Arc::new(GitHubAppTokenCache::new(config)))
                            }
                            Ok(_) => {
                                tracing::warn!(
                                    secret_name,
                                    "GitHub App private key secret is empty; \
                                     falling back to github_token auth"
                                );
                                None
                            }
                            Err(error) => {
                                tracing::warn!(
                                    secret_name,
                                    error = %error,
                                    "failed to load GitHub App private key; \
                                     falling back to github_token auth"
                                );
                                None
                            }
                        }
                    }
                    Err(_) => {
                        tracing::warn!(app_id, "GitHub App id does not fit in i64");
                        None
                    }
                }
            }
            _ => None,
        };

        Self {
            secrets,
            user_id,
            api_base_url,
            app_token_cache,
            default_installation_id,
            fallback_token_secret: fallback_token_secret.into(),
            resilience: GitHubApiResilience::default(),
        }
    }

    /// True when an authenticated GitHub App token cache is available.
    pub fn has_github_app(&self) -> bool {
        self.app_token_cache.is_some()
    }

    fn installation_for(&self, repo: &RepoProjectRepo) -> Option<i64> {
        repo.installation_id.or(self.default_installation_id)
    }
}

#[async_trait]
impl RepoGitHubClientProvider for SecretsRepoGitHubClientProvider {
    async fn client_for(&self, repo: &RepoProjectRepo) -> Result<GitHubApiClient, String> {
        // Prefer GitHub App installation auth when the repo opts into it and an
        // installation id is resolvable.
        if repo.auth_mode == GitHubAuthMode::GitHubApp
            && let Some(cache) = self.app_token_cache.as_ref()
            && let Some(installation_id) = self.installation_for(repo)
        {
            return Ok(
                GitHubApiClient::with_app_installation(Arc::clone(cache), installation_id)
                    .with_resilience(self.resilience.clone()),
            );
        }

        // Fallback: short-lived bearer token from the secrets store.
        let token = resolve_secret(
            self.secrets.as_ref(),
            &self.user_id,
            &self.fallback_token_secret,
            "build_repo_supervisor_client",
        )
        .await
        .map_err(|error| {
            format!(
                "no GitHub credentials for {}/{}: {error}",
                repo.owner, repo.repo
            )
        })?;
        if token.trim().is_empty() {
            return Err(format!(
                "GitHub token secret '{}' is empty",
                self.fallback_token_secret
            ));
        }
        Ok(
            GitHubApiClient::with_base_url_and_token(self.api_base_url.clone(), token)
                .with_resilience(self.resilience.clone()),
        )
    }

    async fn discovery_client(&self) -> Result<(GitHubApiClient, GitHubAuthMode), String> {
        // Prefer App installation auth: this is what unlocks
        // `/installation/repositories`, the native "select all or specific
        // repos" source. Requires both an App key and a default installation id.
        if let Some(cache) = self.app_token_cache.as_ref()
            && let Some(installation_id) = self.default_installation_id
        {
            return Ok((
                GitHubApiClient::with_app_installation(Arc::clone(cache), installation_id)
                    .with_resilience(self.resilience.clone()),
                GitHubAuthMode::GitHubApp,
            ));
        }

        // Fallback: a personal-access token enumerates the owner's repos via
        // `/user/repos`.
        let token = resolve_secret(
            self.secrets.as_ref(),
            &self.user_id,
            &self.fallback_token_secret,
            "discover_repos",
        )
        .await
        .map_err(|error| format!("no GitHub credentials for repo discovery: {error}"))?;
        if token.trim().is_empty() {
            return Err(format!(
                "GitHub token secret '{}' is empty",
                self.fallback_token_secret
            ));
        }
        Ok((
            GitHubApiClient::with_base_url_and_token(self.api_base_url.clone(), token)
                .with_resilience(self.resilience.clone()),
            GitHubAuthMode::UserToken,
        ))
    }
}

async fn resolve_secret(
    secrets: &(dyn crate::secrets::SecretsStore + Send + Sync),
    user_id: &str,
    name: &str,
    purpose: &str,
) -> Result<String, String> {
    let secret = secrets
        .get_for_injection(
            user_id,
            name,
            crate::secrets::SecretAccessContext::new("repo_projects.supervisor", purpose)
                .target("api.github.com", "/"),
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(secret.expose().to_string())
}

/// Fixed-endpoint provider used by integration tests to point the real
/// [`GitHubApiClient`] at an in-process fake GitHub server.
#[derive(Debug, Clone)]
pub struct FixedTokenGitHubClientProvider {
    api_base_url: String,
    token: String,
    resilience: GitHubApiResilience,
}

impl FixedTokenGitHubClientProvider {
    pub fn new(api_base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            api_base_url: api_base_url.into(),
            token: token.into(),
            resilience: GitHubApiResilience::default(),
        }
    }
}

#[async_trait]
impl RepoGitHubClientProvider for FixedTokenGitHubClientProvider {
    async fn client_for(&self, _repo: &RepoProjectRepo) -> Result<GitHubApiClient, String> {
        Ok(
            GitHubApiClient::with_base_url_and_token(self.api_base_url.clone(), self.token.clone())
                .with_resilience(self.resilience.clone()),
        )
    }

    async fn discovery_client(&self) -> Result<(GitHubApiClient, GitHubAuthMode), String> {
        // Tests drive the App-installation discovery path against the fake
        // server, so report GitHubApp mode here.
        Ok((
            GitHubApiClient::with_base_url_and_token(self.api_base_url.clone(), self.token.clone())
                .with_resilience(self.resilience.clone()),
            GitHubAuthMode::GitHubApp,
        ))
    }
}
