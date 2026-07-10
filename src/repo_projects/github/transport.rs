use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::header::HeaderMap;
use reqwest::{Method, StatusCode};
use thinclaw_agent::loop_control::LoopRetryPolicy;

use super::{GitHubApiClient, GitHubApiError};

const GITHUB_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const GITHUB_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const GITHUB_MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

pub(super) fn github_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(GITHUB_CONNECT_TIMEOUT)
        .timeout(GITHUB_REQUEST_TIMEOUT)
        .build()
        .expect("static GitHub HTTP client configuration should be valid")
}

#[derive(Debug, Clone, Copy)]
pub(super) struct GitHubRequestPolicy {
    pub(super) retry: LoopRetryPolicy,
    pub(super) circuit_failure_threshold: u32,
    pub(super) circuit_open_duration: Duration,
}

impl Default for GitHubRequestPolicy {
    fn default() -> Self {
        Self {
            retry: LoopRetryPolicy::bounded(2, Duration::from_millis(200), Duration::from_secs(3)),
            circuit_failure_threshold: 5,
            circuit_open_duration: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Default)]
struct GitHubCircuitState {
    consecutive_transient_failures: u32,
    open_until: Option<Instant>,
}

#[derive(Debug, Clone, Default)]
pub struct GitHubApiResilience {
    circuit: Arc<Mutex<GitHubCircuitState>>,
}

impl GitHubApiClient {
    #[cfg(test)]
    pub(super) fn with_request_policy(mut self, request_policy: GitHubRequestPolicy) -> Self {
        self.request_policy = request_policy;
        self
    }

    pub(super) async fn send_with_retry<F>(
        &self,
        method: &Method,
        url: &str,
        mut build_request: F,
    ) -> Result<reqwest::Response, GitHubApiError>
    where
        F: FnMut(HeaderMap) -> reqwest::RequestBuilder,
    {
        let retry_safe = github_retry_safe_method(method);
        let mut retries_used = 0u32;

        loop {
            if let Some(error) = self.circuit_rejection() {
                return Err(error);
            }
            let headers = self.github_api_headers().await?;
            match build_request(headers).send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        self.record_github_request_success();
                        return Ok(response);
                    }

                    let retry_after = github_retry_after(response.headers());
                    let transient = github_transient_status(status, response.headers());
                    if transient {
                        self.record_github_transient_failure(status.as_str());
                    } else {
                        self.record_github_request_success();
                    }

                    if transient
                        && retry_safe
                        && let Some(delay) = self.github_retry_delay(retries_used, retry_after)
                    {
                        retries_used += 1;
                        tracing::warn!(
                            method = method.as_str(),
                            url,
                            status = status.as_u16(),
                            retry = retries_used,
                            backoff_ms = delay.as_millis() as u64,
                            "retrying transient GitHub API response"
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Ok(response);
                }
                Err(source) => {
                    let transient = source.is_connect() || source.is_timeout();
                    if transient {
                        self.record_github_transient_failure(if source.is_timeout() {
                            "timeout"
                        } else {
                            "connect"
                        });
                    } else {
                        self.record_github_request_success();
                    }

                    if transient
                        && retry_safe
                        && let Some(delay) = self.github_retry_delay(retries_used, None)
                    {
                        retries_used += 1;
                        tracing::warn!(
                            method = method.as_str(),
                            url,
                            retry = retries_used,
                            backoff_ms = delay.as_millis() as u64,
                            error = %source,
                            "retrying transient GitHub transport failure"
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Err(GitHubApiError::Http {
                        method: method.to_string(),
                        url: url.to_string(),
                        source,
                    });
                }
            }
        }
    }

    fn github_retry_delay(
        &self,
        retries_used: u32,
        retry_after: Option<Duration>,
    ) -> Option<Duration> {
        self.request_policy
            .retry
            .delay_for_retry(retries_used)
            .map(github_jittered_delay)
            .map(|delay| retry_after.map_or(delay, |server_delay| server_delay.max(delay)))
    }

    fn circuit_rejection(&self) -> Option<GitHubApiError> {
        let mut circuit = self
            .resilience
            .circuit
            .lock()
            .expect("GitHub circuit mutex poisoned");
        let open_until = circuit.open_until?;
        let now = Instant::now();
        if now < open_until {
            return Some(GitHubApiError::CircuitOpen {
                retry_after: open_until.saturating_duration_since(now),
            });
        }
        circuit.open_until = None;
        circuit.consecutive_transient_failures = 0;
        None
    }

    fn record_github_request_success(&self) {
        let mut circuit = self
            .resilience
            .circuit
            .lock()
            .expect("GitHub circuit mutex poisoned");
        circuit.consecutive_transient_failures = 0;
        circuit.open_until = None;
    }

    fn record_github_transient_failure(&self, failure: &str) {
        let mut circuit = self
            .resilience
            .circuit
            .lock()
            .expect("GitHub circuit mutex poisoned");
        circuit.consecutive_transient_failures =
            circuit.consecutive_transient_failures.saturating_add(1);
        if circuit.consecutive_transient_failures
            >= self.request_policy.circuit_failure_threshold.max(1)
        {
            circuit.open_until = Some(Instant::now() + self.request_policy.circuit_open_duration);
            tracing::warn!(
                failure,
                failures = circuit.consecutive_transient_failures,
                open_ms = self.request_policy.circuit_open_duration.as_millis() as u64,
                "GitHub API circuit opened"
            );
        }
    }
}

fn github_retry_safe_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::PUT | Method::DELETE | Method::OPTIONS
    )
}

fn github_transient_status(status: StatusCode, headers: &HeaderMap) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
        || (status == StatusCode::FORBIDDEN
            && (github_retry_after(headers).is_some()
                || headers
                    .get("x-ratelimit-remaining")
                    .and_then(|value| value.to_str().ok())
                    == Some("0")))
}

fn github_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let retry_after = headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs);
    let rate_limit_reset = headers
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|reset_at| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            Duration::from_secs(reset_at.saturating_sub(now))
        });
    retry_after
        .or(rate_limit_reset)
        .map(|delay| delay.min(GITHUB_MAX_RETRY_AFTER))
}

fn github_jittered_delay(delay: Duration) -> Duration {
    let millis = delay.as_millis().min(u64::MAX as u128) as u64;
    if millis == 0 {
        return delay;
    }
    let spread = (millis / 5).max(1);
    let sample = rand::random::<u64>() % (spread.saturating_mul(2).saturating_add(1));
    Duration::from_millis(millis.saturating_sub(spread).saturating_add(sample))
}
