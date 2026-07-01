//! GitHub App webhook and delivery helpers for repo project supervision.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use hmac::{Hmac, Mac};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

type HmacSha256 = Hmac<Sha256>;

const GITHUB_SIGNATURE_PREFIX: &str = "sha256=";
const GITHUB_API_VERSION: &str = "2022-11-28";
const GITHUB_USER_AGENT: &str = "ThinClaw-RepoProjectSupervisor";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubWebhookError {
    MissingSignature,
    InvalidSignatureFormat,
    InvalidSecret,
    SignatureMismatch,
    MissingDeliveryId,
    DuplicateDelivery,
}

#[derive(Debug)]
pub enum GitHubAppError {
    InvalidPrivateKey(String),
    Jwt(String),
    Http(String),
    InvalidHeader(String),
    InstallationToken(String),
}

impl std::fmt::Display for GitHubAppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPrivateKey(message) => {
                write!(f, "invalid GitHub App private key: {message}")
            }
            Self::Jwt(message) => write!(f, "failed to sign GitHub App JWT: {message}"),
            Self::Http(message) => write!(f, "GitHub App HTTP request failed: {message}"),
            Self::InvalidHeader(message) => write!(f, "invalid GitHub App HTTP header: {message}"),
            Self::InstallationToken(message) => {
                write!(f, "failed to obtain GitHub installation token: {message}")
            }
        }
    }
}

impl std::error::Error for GitHubAppError {}

#[derive(Debug)]
pub enum GitHubApiError {
    Auth(GitHubAppError),
    InvalidHeader(String),
    Http {
        method: String,
        url: String,
        source: reqwest::Error,
    },
    Api {
        status: StatusCode,
        method: String,
        url: String,
        message: Option<String>,
        documentation_url: Option<String>,
        errors: Option<serde_json::Value>,
        request_id: Option<String>,
        body: String,
    },
    Decode {
        status: StatusCode,
        method: String,
        url: String,
        body: String,
        source: serde_json::Error,
    },
}

impl std::fmt::Display for GitHubApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auth(error) => write!(f, "GitHub API authentication failed: {error}"),
            Self::InvalidHeader(message) => write!(f, "invalid GitHub API header: {message}"),
            Self::Http {
                method,
                url,
                source,
            } => {
                write!(
                    f,
                    "GitHub API HTTP request failed for {method} {url}: {source}"
                )
            }
            Self::Api {
                status,
                method,
                url,
                message,
                request_id,
                ..
            } => {
                write!(
                    f,
                    "GitHub API returned {status} for {method} {url}: {}",
                    message.as_deref().unwrap_or("no error message")
                )?;
                if let Some(request_id) = request_id {
                    write!(f, " (request id {request_id})")?;
                }
                Ok(())
            }
            Self::Decode {
                status,
                method,
                url,
                source,
                ..
            } => write!(
                f,
                "failed to decode GitHub API response for {method} {url} ({status}): {source}"
            ),
        }
    }
}

impl std::error::Error for GitHubApiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Auth(error) => Some(error),
            Self::Http { source, .. } => Some(source),
            Self::Decode { source, .. } => Some(source),
            Self::InvalidHeader(_) | Self::Api { .. } => None,
        }
    }
}

impl From<GitHubAppError> for GitHubApiError {
    fn from(error: GitHubAppError) -> Self {
        Self::Auth(error)
    }
}

#[derive(Debug, Clone)]
pub struct GitHubAppConfig {
    pub app_id: i64,
    pub private_key_pem: String,
    pub api_base_url: String,
}

impl GitHubAppConfig {
    pub fn new(app_id: i64, private_key_pem: impl Into<String>) -> Self {
        Self {
            app_id,
            private_key_pem: private_key_pem.into(),
            api_base_url: "https://api.github.com".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubAppJwtClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubInstallationToken {
    pub installation_id: i64,
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

impl GitHubInstallationToken {
    pub fn is_valid_for(&self, min_ttl: ChronoDuration) -> bool {
        self.expires_at - Utc::now() > min_ttl
    }
}

#[derive(Debug, Deserialize)]
struct GitHubInstallationTokenResponse {
    token: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct GitHubAppTokenCache {
    config: GitHubAppConfig,
    client: reqwest::Client,
    tokens: tokio::sync::Mutex<HashMap<i64, GitHubInstallationToken>>,
}

impl GitHubAppTokenCache {
    pub fn new(config: GitHubAppConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            tokens: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn with_client(config: GitHubAppConfig, client: reqwest::Client) -> Self {
        Self {
            config,
            client,
            tokens: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn app_jwt(&self) -> Result<String, GitHubAppError> {
        create_github_app_jwt(self.config.app_id, &self.config.private_key_pem)
    }

    pub async fn installation_token(
        &self,
        installation_id: i64,
    ) -> Result<GitHubInstallationToken, GitHubAppError> {
        {
            let tokens = self.tokens.lock().await;
            if let Some(token) = tokens.get(&installation_id)
                && token.is_valid_for(ChronoDuration::minutes(5))
            {
                return Ok(token.clone());
            }
        }

        let jwt = self.app_jwt()?;
        let url = format!(
            "{}/app/installations/{}/access_tokens",
            self.config.api_base_url.trim_end_matches('/'),
            installation_id
        );
        let response = self
            .client
            .post(url)
            .headers(github_app_headers(&jwt)?)
            .send()
            .await
            .map_err(|error| GitHubAppError::Http(error.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(GitHubAppError::InstallationToken(format!(
                "GitHub returned {status}: {body}"
            )));
        }
        let body: GitHubInstallationTokenResponse = response
            .json()
            .await
            .map_err(|error| GitHubAppError::InstallationToken(error.to_string()))?;
        let token = GitHubInstallationToken {
            installation_id,
            token: body.token,
            expires_at: body.expires_at,
        };
        self.tokens
            .lock()
            .await
            .insert(installation_id, token.clone());
        Ok(token)
    }
}

#[derive(Clone)]
pub enum GitHubApiAuth {
    BearerToken(String),
    Installation {
        installation_id: i64,
        token_cache: Arc<GitHubAppTokenCache>,
    },
}

impl std::fmt::Debug for GitHubApiAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BearerToken(_) => f.write_str("BearerToken([REDACTED])"),
            Self::Installation {
                installation_id, ..
            } => f
                .debug_struct("Installation")
                .field("installation_id", installation_id)
                .field("token_cache", &"[REDACTED]")
                .finish(),
        }
    }
}

#[derive(Clone)]
pub struct GitHubApiClient {
    api_base_url: String,
    client: reqwest::Client,
    auth: GitHubApiAuth,
}

impl std::fmt::Debug for GitHubApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubApiClient")
            .field("api_base_url", &self.api_base_url)
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}

impl GitHubApiClient {
    pub fn with_token(token: impl Into<String>) -> Self {
        Self::with_base_url_and_token("https://api.github.com", token)
    }

    pub fn with_base_url_and_token(
        api_base_url: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        Self::with_client_and_auth(
            api_base_url,
            reqwest::Client::new(),
            GitHubApiAuth::BearerToken(token.into()),
        )
    }

    pub fn with_app_installation(
        token_cache: Arc<GitHubAppTokenCache>,
        installation_id: i64,
    ) -> Self {
        let api_base_url = token_cache.config.api_base_url.clone();
        Self::with_client_and_auth(
            api_base_url,
            reqwest::Client::new(),
            GitHubApiAuth::Installation {
                installation_id,
                token_cache,
            },
        )
    }

    pub fn with_app_config(config: GitHubAppConfig, installation_id: i64) -> Self {
        let token_cache = Arc::new(GitHubAppTokenCache::new(config));
        Self::with_app_installation(token_cache, installation_id)
    }

    pub fn with_client_and_auth(
        api_base_url: impl Into<String>,
        client: reqwest::Client,
        auth: GitHubApiAuth,
    ) -> Self {
        Self {
            api_base_url: api_base_url.into(),
            client,
            auth,
        }
    }

    pub async fn get_repository(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<GitHubRepository, GitHubApiError> {
        self.get(repo_path(owner, repo, &[])).await
    }

    pub async fn ensure_repository_permission(
        &self,
        owner: &str,
        repo: &str,
        required: GitHubRepoPermission,
    ) -> Result<GitHubRepositoryPermissionCheck, GitHubApiError> {
        let repository = self.get_repository(owner, repo).await?;
        let permissions = repository.permissions.unwrap_or_default();
        Ok(GitHubRepositoryPermissionCheck {
            owner: owner.to_string(),
            repo: repo.to_string(),
            required,
            allowed: permissions.allows(required),
            permissions,
        })
    }

    /// List the repositories accessible to the authenticated App installation.
    /// This is the "which repos can the agent act on" source that powers the
    /// connector repo picker. Requires installation (GitHub App) auth.
    pub async fn list_installation_repositories(
        &self,
        query: &GitHubListQuery,
    ) -> Result<GitHubInstallationRepositoriesResponse, GitHubApiError> {
        self.get_query("installation/repositories".to_string(), query)
            .await
    }

    /// List repositories visible to the authenticated user. Used as the
    /// personal-access-token fallback when no GitHub App installation is
    /// configured.
    pub async fn list_user_repositories(
        &self,
        query: &GitHubUserReposQuery,
    ) -> Result<Vec<GitHubRepository>, GitHubApiError> {
        self.get_query("user/repos".to_string(), query).await
    }

    pub async fn get_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<GitHubBranch, GitHubApiError> {
        self.get(repo_path(owner, repo, &["branches", branch]))
            .await
    }

    pub async fn get_branch_ref(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<GitHubGitRef, GitHubApiError> {
        self.get(git_ref_path(owner, repo, branch)).await
    }

    pub async fn create_branch_ref(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        sha: &str,
    ) -> Result<GitHubGitRef, GitHubApiError> {
        self.post(
            repo_path(owner, repo, &["git", "refs"]),
            &GitHubCreateRefRequest {
                git_ref: format!("refs/heads/{branch}"),
                sha: sha.to_string(),
            },
        )
        .await
    }

    pub async fn update_branch_ref(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        sha: &str,
        force: bool,
    ) -> Result<GitHubGitRef, GitHubApiError> {
        self.patch(
            git_ref_path(owner, repo, branch),
            &GitHubUpdateRefRequest {
                sha: sha.to_string(),
                force,
            },
        )
        .await
    }

    /// Compare two refs (`base...head`) to determine how far ahead/behind the
    /// head branch is. Used by the merge gate to enforce "branch up to date".
    pub async fn compare_commits(
        &self,
        owner: &str,
        repo: &str,
        base: &str,
        head: &str,
    ) -> Result<GitHubCommitComparison, GitHubApiError> {
        self.get(repo_path(
            owner,
            repo,
            &["compare", &format!("{base}...{head}")],
        ))
        .await
    }

    /// Delete a `heads/<branch>` ref. Used to clean up the task branch after a
    /// successful auto-merge when branch deletion is permitted.
    pub async fn delete_branch_ref(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<(), GitHubApiError> {
        self.delete_no_content(repo_path(
            owner,
            repo,
            &["git", "refs", &format!("heads/{branch}")],
        ))
        .await
    }

    pub async fn list_pull_requests(
        &self,
        owner: &str,
        repo: &str,
        query: &GitHubPullRequestListQuery,
    ) -> Result<Vec<GitHubPullRequest>, GitHubApiError> {
        self.get_query(repo_path(owner, repo, &["pulls"]), query)
            .await
    }

    pub async fn get_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<GitHubPullRequest, GitHubApiError> {
        self.get(repo_path(owner, repo, &["pulls", &number.to_string()]))
            .await
    }

    pub async fn create_pull_request(
        &self,
        owner: &str,
        repo: &str,
        input: &GitHubCreatePullRequestRequest,
    ) -> Result<GitHubPullRequest, GitHubApiError> {
        self.post(repo_path(owner, repo, &["pulls"]), input).await
    }

    pub async fn update_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        input: &GitHubUpdatePullRequestRequest,
    ) -> Result<GitHubPullRequest, GitHubApiError> {
        self.patch(
            repo_path(owner, repo, &["pulls", &number.to_string()]),
            input,
        )
        .await
    }

    pub async fn create_pull_request_comment(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        body: impl Into<String>,
    ) -> Result<GitHubIssueComment, GitHubApiError> {
        self.post(
            repo_path(owner, repo, &["issues", &number.to_string(), "comments"]),
            &GitHubCreateIssueCommentRequest { body: body.into() },
        )
        .await
    }

    pub async fn list_pull_request_reviews(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        query: &GitHubListQuery,
    ) -> Result<Vec<GitHubPullRequestReview>, GitHubApiError> {
        self.get_query(
            repo_path(owner, repo, &["pulls", &number.to_string(), "reviews"]),
            query,
        )
        .await
    }

    pub async fn list_pull_request_review_comments(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        query: &GitHubListQuery,
    ) -> Result<Vec<GitHubReviewComment>, GitHubApiError> {
        self.get_query(
            repo_path(owner, repo, &["pulls", &number.to_string(), "comments"]),
            query,
        )
        .await
    }

    pub async fn create_pull_request_review_comment(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        input: &GitHubCreateReviewCommentRequest,
    ) -> Result<GitHubReviewComment, GitHubApiError> {
        self.post(
            repo_path(owner, repo, &["pulls", &number.to_string(), "comments"]),
            input,
        )
        .await
    }

    pub async fn merge_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        input: &GitHubMergePullRequestRequest,
    ) -> Result<GitHubMergePullRequestResponse, GitHubApiError> {
        self.put(
            repo_path(owner, repo, &["pulls", &number.to_string(), "merge"]),
            input,
        )
        .await
    }

    pub async fn list_check_runs_for_ref(
        &self,
        owner: &str,
        repo: &str,
        git_ref: &str,
        query: &GitHubCheckRunsQuery,
    ) -> Result<GitHubCheckRunsResponse, GitHubApiError> {
        self.get_query(
            repo_path(owner, repo, &["commits", git_ref, "check-runs"]),
            query,
        )
        .await
    }

    pub async fn list_workflow_runs(
        &self,
        owner: &str,
        repo: &str,
        query: &GitHubWorkflowRunsQuery,
    ) -> Result<GitHubWorkflowRunsResponse, GitHubApiError> {
        self.get_query(repo_path(owner, repo, &["actions", "runs"]), query)
            .await
    }

    pub async fn get_workflow_run(
        &self,
        owner: &str,
        repo: &str,
        run_id: u64,
    ) -> Result<GitHubWorkflowRun, GitHubApiError> {
        self.get(repo_path(
            owner,
            repo,
            &["actions", "runs", &run_id.to_string()],
        ))
        .await
    }

    pub async fn list_workflow_run_jobs(
        &self,
        owner: &str,
        repo: &str,
        run_id: u64,
        query: &GitHubWorkflowJobsQuery,
    ) -> Result<GitHubWorkflowJobsResponse, GitHubApiError> {
        self.get_query(
            repo_path(
                owner,
                repo,
                &["actions", "runs", &run_id.to_string(), "jobs"],
            ),
            query,
        )
        .await
    }

    pub async fn download_workflow_job_logs(
        &self,
        owner: &str,
        repo: &str,
        job_id: u64,
    ) -> Result<GitHubResponseBytes, GitHubApiError> {
        self.get_bytes(repo_path(
            owner,
            repo,
            &["actions", "jobs", &job_id.to_string(), "logs"],
        ))
        .await
    }

    pub async fn list_labels(
        &self,
        owner: &str,
        repo: &str,
        query: &GitHubListQuery,
    ) -> Result<Vec<GitHubLabel>, GitHubApiError> {
        self.get_query(repo_path(owner, repo, &["labels"]), query)
            .await
    }

    pub async fn create_label(
        &self,
        owner: &str,
        repo: &str,
        input: &GitHubCreateLabelRequest,
    ) -> Result<GitHubLabel, GitHubApiError> {
        self.post(repo_path(owner, repo, &["labels"]), input).await
    }

    pub async fn get_issue(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<GitHubIssue, GitHubApiError> {
        self.get(repo_path(owner, repo, &["issues", &number.to_string()]))
            .await
    }

    pub async fn list_issues(
        &self,
        owner: &str,
        repo: &str,
        query: &GitHubIssuesQuery,
    ) -> Result<Vec<GitHubIssue>, GitHubApiError> {
        self.get_query(repo_path(owner, repo, &["issues"]), query)
            .await
    }

    pub async fn create_issue(
        &self,
        owner: &str,
        repo: &str,
        input: &GitHubCreateIssueRequest,
    ) -> Result<GitHubIssue, GitHubApiError> {
        self.post(repo_path(owner, repo, &["issues"]), input).await
    }

    pub async fn update_issue(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        input: &GitHubUpdateIssueRequest,
    ) -> Result<GitHubIssue, GitHubApiError> {
        self.patch(
            repo_path(owner, repo, &["issues", &number.to_string()]),
            input,
        )
        .await
    }

    pub async fn add_labels_to_issue(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        labels: Vec<String>,
    ) -> Result<Vec<GitHubLabel>, GitHubApiError> {
        self.post(
            repo_path(owner, repo, &["issues", &number.to_string(), "labels"]),
            &GitHubIssueLabelsRequest { labels },
        )
        .await
    }

    async fn get<T>(&self, path: String) -> Result<T, GitHubApiError>
    where
        T: DeserializeOwned,
    {
        self.request::<T, (), ()>(Method::GET, path, None, None)
            .await
    }

    async fn get_query<T, Q>(&self, path: String, query: &Q) -> Result<T, GitHubApiError>
    where
        T: DeserializeOwned,
        Q: Serialize + ?Sized,
    {
        self.request::<T, Q, ()>(Method::GET, path, Some(query), None)
            .await
    }

    async fn post<T, B>(&self, path: String, body: &B) -> Result<T, GitHubApiError>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        self.request::<T, (), B>(Method::POST, path, None, Some(body))
            .await
    }

    async fn patch<T, B>(&self, path: String, body: &B) -> Result<T, GitHubApiError>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        self.request::<T, (), B>(Method::PATCH, path, None, Some(body))
            .await
    }

    async fn put<T, B>(&self, path: String, body: &B) -> Result<T, GitHubApiError>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        self.request::<T, (), B>(Method::PUT, path, None, Some(body))
            .await
    }

    async fn delete_no_content(&self, path: String) -> Result<(), GitHubApiError> {
        let method = Method::DELETE;
        let url = self.api_url(&path);
        let response = self
            .client
            .request(method.clone(), &url)
            .headers(self.github_api_headers().await?)
            .send()
            .await
            .map_err(|source| GitHubApiError::Http {
                method: method.to_string(),
                url: url.clone(),
                source,
            })?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let request_id = github_request_id(response.headers());
        let body = response.text().await.unwrap_or_default();
        Err(github_api_error_from_body(
            status,
            method.as_str(),
            &url,
            request_id,
            body,
        ))
    }

    async fn get_bytes(&self, path: String) -> Result<GitHubResponseBytes, GitHubApiError> {
        let method = Method::GET;
        let url = self.api_url(&path);
        let response = self
            .client
            .request(method.clone(), &url)
            .headers(self.github_api_headers().await?)
            .send()
            .await
            .map_err(|source| GitHubApiError::Http {
                method: method.to_string(),
                url: url.clone(),
                source,
            })?;
        decode_bytes_response(method.as_str(), &url, response).await
    }

    async fn request<T, Q, B>(
        &self,
        method: Method,
        path: String,
        query: Option<&Q>,
        body: Option<&B>,
    ) -> Result<T, GitHubApiError>
    where
        T: DeserializeOwned,
        Q: Serialize + ?Sized,
        B: Serialize + ?Sized,
    {
        let url = self.api_url(&path);
        let mut request = self
            .client
            .request(method.clone(), &url)
            .headers(self.github_api_headers().await?);
        if let Some(query) = query {
            request = request.query(query);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|source| GitHubApiError::Http {
                method: method.to_string(),
                url: url.clone(),
                source,
            })?;
        decode_json_response(method.as_str(), &url, response).await
    }

    async fn github_api_headers(&self) -> Result<HeaderMap, GitHubApiError> {
        let token = match &self.auth {
            GitHubApiAuth::BearerToken(token) => token.clone(),
            GitHubApiAuth::Installation {
                installation_id,
                token_cache,
            } => {
                token_cache
                    .installation_token(*installation_id)
                    .await?
                    .token
            }
        };
        github_bearer_headers(&token).map_err(|error| match error {
            GitHubAppError::InvalidHeader(message) => GitHubApiError::InvalidHeader(message),
            error => GitHubApiError::Auth(error),
        })
    }

    fn api_url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.api_base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubResponseBytes {
    pub status: StatusCode,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubRepositoryPermissionCheck {
    pub owner: String,
    pub repo: String,
    pub required: GitHubRepoPermission,
    pub allowed: bool,
    pub permissions: GitHubRepositoryPermissions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubRepoPermission {
    Pull,
    Triage,
    Push,
    Maintain,
    Admin,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubRepositoryPermissions {
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub maintain: bool,
    #[serde(default)]
    pub push: bool,
    #[serde(default)]
    pub triage: bool,
    #[serde(default)]
    pub pull: bool,
}

impl GitHubRepositoryPermissions {
    pub fn allows(&self, required: GitHubRepoPermission) -> bool {
        match required {
            GitHubRepoPermission::Pull => {
                self.pull || self.triage || self.push || self.maintain || self.admin
            }
            GitHubRepoPermission::Triage => self.triage || self.push || self.maintain || self.admin,
            GitHubRepoPermission::Push => self.push || self.maintain || self.admin,
            GitHubRepoPermission::Maintain => self.maintain || self.admin,
            GitHubRepoPermission::Admin => self.admin,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubUser {
    pub login: String,
    pub id: i64,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default, rename = "type")]
    pub user_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubRepository {
    pub id: i64,
    #[serde(default)]
    pub node_id: Option<String>,
    pub name: String,
    pub full_name: String,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub clone_url: Option<String>,
    #[serde(default)]
    pub ssh_url: Option<String>,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub fork: bool,
    #[serde(default)]
    pub permissions: Option<GitHubRepositoryPermissions>,
    #[serde(default)]
    pub owner: Option<GitHubUser>,
}

impl GitHubRepository {
    pub fn has_permission(&self, required: GitHubRepoPermission) -> bool {
        self.permissions
            .as_ref()
            .is_some_and(|permissions| permissions.allows(required))
    }
}

/// Response shape for `GET /installation/repositories`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubInstallationRepositoriesResponse {
    #[serde(default)]
    pub total_count: u64,
    #[serde(default)]
    pub repositories: Vec<GitHubRepository>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubBranch {
    pub name: String,
    pub commit: GitHubBranchCommit,
    #[serde(default)]
    pub protected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubBranchCommit {
    pub sha: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubCommitComparison {
    /// One of `diverged`, `ahead`, `behind`, `identical`.
    pub status: String,
    #[serde(default)]
    pub ahead_by: i64,
    #[serde(default)]
    pub behind_by: i64,
    #[serde(default)]
    pub total_commits: i64,
}

impl GitHubCommitComparison {
    /// True when the head ref already contains every commit on the base ref,
    /// i.e. the branch is not behind its base.
    pub fn is_up_to_date(&self) -> bool {
        self.behind_by <= 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubGitRef {
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub node_id: String,
    pub url: String,
    pub object: GitHubGitObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubGitObject {
    #[serde(rename = "type")]
    pub object_type: String,
    pub sha: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct GitHubCreateRefRequest {
    #[serde(rename = "ref")]
    git_ref: String,
    sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct GitHubUpdateRefRequest {
    sha: String,
    force: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubListQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u32>,
}

/// Query for `GET /user/repos` (PAT fallback repo discovery). Defaults to the
/// repos the token owner can administer/push to, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubUserReposQuery {
    /// One of `all`, `owner`, `public`, `private`, `member`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affiliation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubPullRequestListQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubPullRequest {
    pub id: i64,
    pub number: u64,
    pub state: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub diff_url: Option<String>,
    #[serde(default)]
    pub patch_url: Option<String>,
    pub head: GitHubPullRequestRef,
    pub base: GitHubPullRequestRef,
    #[serde(default)]
    pub user: Option<GitHubUser>,
    #[serde(default)]
    pub labels: Vec<GitHubLabel>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub mergeable: Option<bool>,
    #[serde(default)]
    pub merged: Option<bool>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub closed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub merged_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubPullRequestRef {
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub sha: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub repo: Option<GitHubRepository>,
    #[serde(default)]
    pub user: Option<GitHubUser>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubCreatePullRequestRequest {
    pub title: String,
    pub head: String,
    pub base: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maintainer_can_modify: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubUpdatePullRequestRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maintainer_can_modify: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitHubCreateIssueCommentRequest {
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubIssueComment {
    pub id: i64,
    #[serde(default)]
    pub node_id: Option<String>,
    pub body: String,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub user: Option<GitHubUser>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubPullRequestReview {
    pub id: i64,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub user: Option<GitHubUser>,
    #[serde(default)]
    pub body: Option<String>,
    pub state: String,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub submitted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubReviewComment {
    pub id: i64,
    #[serde(default)]
    pub pull_request_review_id: Option<i64>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default)]
    pub line: Option<i64>,
    #[serde(default)]
    pub side: Option<String>,
    pub body: String,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub user: Option<GitHubUser>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubCreateReviewCommentRequest {
    pub body: String,
    pub commit_id: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_side: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitHubMergeMethod {
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubMergePullRequestRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_method: Option<GitHubMergeMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubMergePullRequestResponse {
    pub sha: String,
    pub merged: bool,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubCheckRunsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubCheckRunsResponse {
    pub total_count: u64,
    #[serde(default)]
    pub check_runs: Vec<GitHubCheckRun>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubCheckRun {
    pub id: i64,
    pub name: String,
    pub head_sha: String,
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub details_url: Option<String>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub output: Option<GitHubCheckRunOutput>,
    #[serde(default)]
    pub check_suite: Option<GitHubCheckSuiteRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubCheckRunOutput {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubCheckSuiteRef {
    pub id: i64,
    #[serde(default)]
    pub head_sha: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub conclusion: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubWorkflowRunsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubWorkflowRunsResponse {
    pub total_count: u64,
    #[serde(default)]
    pub workflow_runs: Vec<GitHubWorkflowRun>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubWorkflowRun {
    pub id: u64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub head_branch: Option<String>,
    pub head_sha: String,
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    pub event: String,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub run_number: Option<u64>,
    #[serde(default)]
    pub run_attempt: Option<u64>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub run_started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub pull_requests: Vec<GitHubWorkflowRunPullRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubWorkflowRunPullRequest {
    pub id: i64,
    pub number: u64,
    #[serde(default)]
    pub head: Option<GitHubWorkflowRunPullRequestRef>,
    #[serde(default)]
    pub base: Option<GitHubWorkflowRunPullRequestRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubWorkflowRunPullRequestRef {
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub sha: String,
    #[serde(default)]
    pub repo: Option<GitHubWorkflowRunRepoRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubWorkflowRunRepoRef {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubWorkflowJobsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubWorkflowJobsResponse {
    pub total_count: u64,
    #[serde(default)]
    pub jobs: Vec<GitHubWorkflowJob>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubWorkflowJob {
    pub id: u64,
    pub run_id: u64,
    #[serde(default)]
    pub run_url: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    pub head_sha: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    pub name: String,
    #[serde(default)]
    pub check_run_url: Option<String>,
    #[serde(default)]
    pub steps: Vec<GitHubWorkflowJobStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubWorkflowJobStep {
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    pub number: u64,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubLabel {
    pub id: i64,
    pub node_id: String,
    pub url: String,
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitHubIssue {
    pub id: i64,
    pub number: u64,
    pub state: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub user: Option<GitHubUser>,
    #[serde(default)]
    pub labels: Vec<GitHubLabel>,
    #[serde(default)]
    pub pull_request: Option<serde_json::Value>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubIssuesQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mentioned: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubCreateIssueRequest {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assignees: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitHubUpdateIssueRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignees: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitHubCreateLabelRequest {
    pub name: String,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct GitHubIssueLabelsRequest {
    labels: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubApiErrorBody {
    message: Option<String>,
    documentation_url: Option<String>,
    errors: Option<serde_json::Value>,
}

async fn decode_json_response<T>(
    method: &str,
    url: &str,
    response: reqwest::Response,
) -> Result<T, GitHubApiError>
where
    T: DeserializeOwned,
{
    let status = response.status();
    let request_id = github_request_id(response.headers());
    let body = response
        .text()
        .await
        .map_err(|source| GitHubApiError::Http {
            method: method.to_string(),
            url: url.to_string(),
            source,
        })?;
    if !status.is_success() {
        return Err(github_api_error_from_body(
            status, method, url, request_id, body,
        ));
    }

    serde_json::from_str(&body).map_err(|source| GitHubApiError::Decode {
        status,
        method: method.to_string(),
        url: url.to_string(),
        body,
        source,
    })
}

async fn decode_bytes_response(
    method: &str,
    url: &str,
    response: reqwest::Response,
) -> Result<GitHubResponseBytes, GitHubApiError> {
    let status = response.status();
    let request_id = github_request_id(response.headers());
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(github_api_error_from_body(
            status, method, url, request_id, body,
        ));
    }
    let body = response
        .bytes()
        .await
        .map_err(|source| GitHubApiError::Http {
            method: method.to_string(),
            url: url.to_string(),
            source,
        })?
        .to_vec();
    Ok(GitHubResponseBytes {
        status,
        content_type,
        body,
    })
}

fn github_api_error_from_body(
    status: StatusCode,
    method: &str,
    url: &str,
    request_id: Option<String>,
    body: String,
) -> GitHubApiError {
    let parsed = parse_github_api_error_body(&body);
    GitHubApiError::Api {
        status,
        method: method.to_string(),
        url: url.to_string(),
        message: parsed.as_ref().and_then(|body| body.message.clone()),
        documentation_url: parsed
            .as_ref()
            .and_then(|body| body.documentation_url.clone()),
        errors: parsed.and_then(|body| body.errors),
        request_id,
        body,
    }
}

fn parse_github_api_error_body(body: &str) -> Option<GitHubApiErrorBody> {
    serde_json::from_str(body).ok()
}

fn github_request_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-github-request-id")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

pub fn create_github_app_jwt(app_id: i64, private_key_pem: &str) -> Result<String, GitHubAppError> {
    let now = Utc::now();
    let claims = GitHubAppJwtClaims {
        iat: (now - ChronoDuration::seconds(60)).timestamp(),
        exp: (now + ChronoDuration::minutes(9)).timestamp(),
        iss: app_id.to_string(),
    };
    let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
        .map_err(|error| GitHubAppError::InvalidPrivateKey(error.to_string()))?;
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key)
        .map_err(|error| GitHubAppError::Jwt(error.to_string()))
}

fn github_app_headers(jwt: &str) -> Result<HeaderMap, GitHubAppError> {
    github_auth_headers(&format!("Bearer {jwt}"))
}

fn github_bearer_headers(token: &str) -> Result<HeaderMap, GitHubAppError> {
    github_auth_headers(&format!("Bearer {token}"))
}

fn github_auth_headers(authorization: &str) -> Result<HeaderMap, GitHubAppError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(USER_AGENT, HeaderValue::from_static(GITHUB_USER_AGENT));
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static(GITHUB_API_VERSION),
    );
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(authorization)
            .map_err(|error| GitHubAppError::InvalidHeader(error.to_string()))?,
    );
    Ok(headers)
}

fn repo_path(owner: &str, repo: &str, suffix: &[&str]) -> String {
    let mut segments = vec!["repos", owner, repo];
    segments.extend_from_slice(suffix);
    github_api_path(&segments)
}

fn git_ref_path(owner: &str, repo: &str, branch: &str) -> String {
    repo_path(owner, repo, &["git", "ref", &format!("heads/{branch}")])
}

fn github_api_path(segments: &[&str]) -> String {
    segments
        .iter()
        .map(|segment| urlencoding::encode(segment).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

impl std::fmt::Display for GitHubWebhookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSignature => write!(f, "missing X-Hub-Signature-256 header"),
            Self::InvalidSignatureFormat => write!(f, "invalid GitHub webhook signature format"),
            Self::InvalidSecret => write!(f, "invalid GitHub webhook secret"),
            Self::SignatureMismatch => write!(f, "GitHub webhook signature mismatch"),
            Self::MissingDeliveryId => write!(f, "missing X-GitHub-Delivery header"),
            Self::DuplicateDelivery => write!(f, "duplicate GitHub webhook delivery"),
        }
    }
}

impl std::error::Error for GitHubWebhookError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubWebhookEnvelope {
    pub event: String,
    pub delivery_id: String,
    pub installation_id: Option<i64>,
    pub repository_full_name: Option<String>,
    pub action: Option<String>,
    pub payload: serde_json::Value,
}

pub fn verify_github_webhook_signature(
    secret: &str,
    body: &[u8],
    signature_header: Option<&str>,
) -> Result<(), GitHubWebhookError> {
    let signature = signature_header.ok_or(GitHubWebhookError::MissingSignature)?;
    let signature = signature
        .strip_prefix(GITHUB_SIGNATURE_PREFIX)
        .ok_or(GitHubWebhookError::InvalidSignatureFormat)?;
    let expected =
        hex::decode(signature).map_err(|_| GitHubWebhookError::InvalidSignatureFormat)?;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| GitHubWebhookError::InvalidSecret)?;
    mac.update(body);
    mac.verify_slice(&expected)
        .map_err(|_| GitHubWebhookError::SignatureMismatch)
}

pub fn parse_github_webhook_envelope(
    event: &str,
    delivery_id: Option<&str>,
    body: &[u8],
) -> Result<GitHubWebhookEnvelope, GitHubWebhookError> {
    let delivery_id = delivery_id
        .filter(|id| !id.trim().is_empty())
        .ok_or(GitHubWebhookError::MissingDeliveryId)?
        .to_string();
    let payload: serde_json::Value =
        serde_json::from_slice(body).unwrap_or_else(|_| serde_json::json!({}));
    let installation_id = payload
        .get("installation")
        .and_then(|installation| installation.get("id"))
        .and_then(|value| value.as_i64());
    let repository_full_name = payload
        .get("repository")
        .and_then(|repo| repo.get("full_name"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let action = payload
        .get("action")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);

    Ok(GitHubWebhookEnvelope {
        event: event.to_string(),
        delivery_id,
        installation_id,
        repository_full_name,
        action,
        payload,
    })
}

#[derive(Debug)]
pub struct GitHubDeliveryDeduper {
    ttl: Duration,
    seen: std::sync::Mutex<Vec<(String, Instant)>>,
}

impl GitHubDeliveryDeduper {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            seen: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn accept(&self, delivery_id: &str) -> Result<(), GitHubWebhookError> {
        if delivery_id.trim().is_empty() {
            return Err(GitHubWebhookError::MissingDeliveryId);
        }

        let mut seen = self
            .seen
            .lock()
            .expect("GitHub delivery deduper mutex poisoned");
        let now = Instant::now();
        seen.retain(|(_, inserted_at)| now.duration_since(*inserted_at) <= self.ttl);

        let existing: HashSet<&str> = seen.iter().map(|(id, _)| id.as_str()).collect();
        if existing.contains(delivery_id) {
            return Err(GitHubWebhookError::DuplicateDelivery);
        }

        seen.push((delivery_id.to_string(), now));
        Ok(())
    }
}

#[cfg(test)]
mod tests;
