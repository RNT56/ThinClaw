//! HTTP client for worker-to-orchestrator communication.
//!
//! Every request includes a bearer token from `THINCLAW_WORKER_TOKEN` env var.
//! The orchestrator validates this token is scoped to the correct job.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

use crate::error::WorkerError;
use crate::llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};

const WORKER_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const WORKER_CONTROL_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const WORKER_LLM_REQUEST_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const WORKER_COMPLETION_REQUEST_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const MAX_WORKER_CONTROL_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_WORKER_LLM_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_WORKER_ERROR_RESPONSE_BYTES: usize = 8 * 1024;
const MAX_WORKER_ENDPOINT_BYTES: usize = 16 * 1024;
const MAX_WORKER_TOKEN_BYTES: usize = 64 * 1024;

/// HTTP client that a container worker uses to talk to the orchestrator.
pub struct WorkerHttpClient {
    client: reqwest::Client,
    orchestrator_url: String,
    job_id: Uuid,
    token: String,
}

/// Status update sent from worker to orchestrator.
#[derive(Debug, Serialize, Deserialize)]
pub struct StatusUpdate {
    pub state: String,
    pub message: Option<String>,
    pub iteration: u32,
}

/// Job description fetched from orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDescription {
    pub title: String,
    pub description: String,
    pub project_dir: Option<String>,
    #[serde(default)]
    pub principal_id: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub allowed_skills: Option<Vec<String>>,
    #[serde(default)]
    pub tool_profile: Option<String>,
    #[serde(default)]
    pub interactive: bool,
    #[serde(default)]
    pub idle_timeout_secs: Option<u64>,
}

/// Completion result from the orchestrator (proxied from the real LLM).
#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyCompletionRequest {
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub context_documents: Vec<String>,
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stop_sequences: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyCompletionResponse {
    pub content: String,
    pub provider_model: Option<String>,
    pub cost_usd: Option<f64>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub finish_reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyToolCompletionRequest {
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub context_documents: Vec<String>,
    pub tools: Vec<ToolDefinition>,
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub tool_choice: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyToolCompletionResponse {
    pub content: Option<String>,
    pub provider_model: Option<String>,
    pub cost_usd: Option<f64>,
    pub tool_calls: Vec<ToolCall>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub finish_reason: String,
}

/// Completion result for the worker to report when done.
#[derive(Debug, Serialize, Deserialize)]
pub struct CompletionReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub success: bool,
    pub message: Option<String>,
    pub iterations: u32,
}

/// Payload sent to the orchestrator for each job event (shared by worker and Claude Code bridge).
#[derive(Debug, Serialize, Deserialize)]
pub struct JobEventPayload {
    pub event_type: String,
    pub data: serde_json::Value,
}

/// Response from the prompt polling endpoint.
#[derive(Debug, Deserialize)]
pub struct PromptResponse {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub done: bool,
}

/// A single credential delivered from the orchestrator to a container worker.
///
/// Shared between the orchestrator endpoint and the worker client.
#[derive(Debug, Serialize, Deserialize)]
pub struct CredentialResponse {
    pub env_var: String,
    pub value: String,
}

impl WorkerHttpClient {
    fn build_client() -> Result<reqwest::Client, WorkerError> {
        reqwest::Client::builder()
            .connect_timeout(WORKER_CONNECT_TIMEOUT)
            .timeout(WORKER_LLM_REQUEST_TIMEOUT)
            // The orchestrator is a local control-plane endpoint. Never send
            // its bearer token through an environment-configured proxy and
            // never follow a redirect to another origin.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| WorkerError::ExecutionFailed {
                reason: format!("failed to build orchestrator HTTP client: {error}"),
            })
    }

    /// Create a new client from environment.
    ///
    /// Reads `THINCLAW_WORKER_TOKEN` from the environment.
    pub fn from_env(orchestrator_url: String, job_id: Uuid) -> Result<Self, WorkerError> {
        let token =
            std::env::var("THINCLAW_WORKER_TOKEN").map_err(|_| WorkerError::MissingToken)?;
        Self::new(orchestrator_url, job_id, token)
    }

    /// Create with an explicit token (for testing).
    pub fn new(orchestrator_url: String, job_id: Uuid, token: String) -> Result<Self, WorkerError> {
        let orchestrator_url = validate_worker_endpoint(&orchestrator_url)?;
        if token.trim().is_empty()
            || token.len() > MAX_WORKER_TOKEN_BYTES
            || token.chars().any(char::is_control)
        {
            return Err(WorkerError::ExecutionFailed {
                reason: "worker token is empty, oversized, or malformed".to_string(),
            });
        }
        Ok(Self {
            client: Self::build_client()?,
            orchestrator_url,
            job_id,
            token,
        })
    }

    /// Get the base orchestrator URL.
    pub fn orchestrator_url(&self) -> &str {
        &self.orchestrator_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}/worker/{}/{}", self.orchestrator_url, self.job_id, path)
    }

    /// Send a GET request, check the status, and deserialize the JSON body.
    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        context: &str,
    ) -> Result<T, WorkerError> {
        let resp = self
            .client
            .get(self.url(path))
            .bearer_auth(&self.token)
            .timeout(WORKER_CONTROL_REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|e| WorkerError::ConnectionFailed {
                url: self.orchestrator_url.clone(),
                reason: e.without_url().to_string(),
            })?;

        if !resp.status().is_success() {
            return Err(WorkerError::OrchestratorRejected {
                job_id: self.job_id,
                reason: format!("{} returned {}", context, resp.status()),
            });
        }

        deserialize_limited_json(resp, MAX_WORKER_CONTROL_RESPONSE_BYTES, context).await
    }

    /// Send a POST request with a JSON body, check the status, and deserialize the response.
    async fn post_json<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
        context: &str,
        timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<T, WorkerError> {
        let resp = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.token)
            .json(body)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| WorkerError::LlmProxyFailed {
                reason: format!("{}: {}", context, e.without_url()),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = read_limited_body(resp, MAX_WORKER_ERROR_RESPONSE_BYTES)
                .await
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_else(|error| format!("<failed to read bounded error body: {error}>"));
            return Err(WorkerError::LlmProxyFailed {
                reason: format!("{}: orchestrator returned {}: {}", context, status, body),
            });
        }

        deserialize_limited_json(resp, max_response_bytes, context).await
    }

    /// Fetch the job description from the orchestrator.
    pub async fn get_job(&self) -> Result<JobDescription, WorkerError> {
        self.get_json("job", "GET /job").await
    }

    /// Proxy an LLM completion request through the orchestrator.
    pub async fn llm_complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, WorkerError> {
        let proxy_req = ProxyCompletionRequest {
            messages: request.messages.clone(),
            context_documents: request.context_documents.clone(),
            model: request.model.clone(),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            stop_sequences: request.stop_sequences.clone(),
        };

        let proxy_resp: ProxyCompletionResponse = self
            .post_json(
                "llm/complete",
                &proxy_req,
                "LLM complete",
                WORKER_LLM_REQUEST_TIMEOUT,
                MAX_WORKER_LLM_RESPONSE_BYTES,
            )
            .await?;

        Ok(CompletionResponse {
            content: proxy_resp.content,
            provider_model: proxy_resp.provider_model,
            cost_usd: proxy_resp.cost_usd,
            thinking_content: None, // Worker proxy doesn't forward thinking yet
            input_tokens: proxy_resp.input_tokens,
            output_tokens: proxy_resp.output_tokens,
            finish_reason: parse_finish_reason(&proxy_resp.finish_reason),
            token_capture: None,
        })
    }

    /// Proxy an LLM tool completion request through the orchestrator.
    pub async fn llm_complete_with_tools(
        &self,
        request: &ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, WorkerError> {
        let proxy_req = ProxyToolCompletionRequest {
            messages: request.messages.clone(),
            context_documents: request.context_documents.clone(),
            tools: request.tools.clone(),
            model: request.model.clone(),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            tool_choice: request.tool_choice.clone(),
        };

        let proxy_resp: ProxyToolCompletionResponse = self
            .post_json(
                "llm/complete_with_tools",
                &proxy_req,
                "LLM tool complete",
                WORKER_LLM_REQUEST_TIMEOUT,
                MAX_WORKER_LLM_RESPONSE_BYTES,
            )
            .await?;

        Ok(ToolCompletionResponse {
            content: proxy_resp.content,
            provider_model: proxy_resp.provider_model,
            cost_usd: proxy_resp.cost_usd,
            tool_calls: proxy_resp.tool_calls,
            thinking_content: None, // Worker proxy doesn't forward thinking yet
            input_tokens: proxy_resp.input_tokens,
            output_tokens: proxy_resp.output_tokens,
            finish_reason: parse_finish_reason(&proxy_resp.finish_reason),
            token_capture: None,
        })
    }

    /// Report status to the orchestrator.
    pub async fn report_status(&self, update: &StatusUpdate) -> Result<(), WorkerError> {
        let resp = self
            .client
            .post(self.url("status"))
            .bearer_auth(&self.token)
            .json(update)
            .timeout(WORKER_CONTROL_REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|e| WorkerError::ConnectionFailed {
                url: self.orchestrator_url.clone(),
                reason: e.without_url().to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = read_limited_body(resp, MAX_WORKER_ERROR_RESPONSE_BYTES)
                .await
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_else(|error| format!("<failed to read bounded error body: {error}>"));
            return Err(WorkerError::OrchestratorRejected {
                job_id: self.job_id,
                reason: format!("status endpoint returned {status}: {body}"),
            });
        }

        Ok(())
    }

    /// Post a job event to the orchestrator (fire-and-forget style, logs on failure).
    pub async fn post_event(&self, payload: &JobEventPayload) {
        let resp = self
            .client
            .post(self.url("event"))
            .bearer_auth(&self.token)
            .json(payload)
            .timeout(WORKER_CONTROL_REQUEST_TIMEOUT)
            .send()
            .await;

        match resp {
            Ok(r) if !r.status().is_success() => {
                tracing::debug!(
                    job_id = %self.job_id,
                    event_type = %payload.event_type,
                    status = %r.status(),
                    "Job event POST rejected"
                );
            }
            Err(e) => {
                tracing::debug!(
                    job_id = %self.job_id,
                    event_type = %payload.event_type,
                    "Job event POST failed: {}", e.without_url()
                );
            }
            _ => {}
        }
    }

    /// Poll the orchestrator for a follow-up prompt.
    ///
    /// Returns `None` if no prompt is available (204 No Content).
    pub async fn poll_prompt(&self) -> Result<Option<PromptResponse>, WorkerError> {
        let resp = self
            .client
            .get(self.url("prompt"))
            .bearer_auth(&self.token)
            .timeout(WORKER_CONTROL_REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|e| WorkerError::ConnectionFailed {
                url: self.orchestrator_url.clone(),
                reason: e.without_url().to_string(),
            })?;

        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }

        if !resp.status().is_success() {
            return Err(WorkerError::OrchestratorRejected {
                job_id: self.job_id,
                reason: format!("prompt endpoint returned {}", resp.status()),
            });
        }

        let prompt: PromptResponse =
            deserialize_limited_json(resp, MAX_WORKER_CONTROL_RESPONSE_BYTES, "prompt response")
                .await?;

        Ok(Some(prompt))
    }

    /// Fetch credentials granted to this job from the orchestrator.
    ///
    /// Returns an empty vec if no credentials are granted (204 No Content)
    /// or if the endpoint returns 404. The caller should set each credential
    /// as an environment variable before starting the execution loop.
    pub async fn fetch_credentials(&self) -> Result<Vec<CredentialResponse>, WorkerError> {
        let resp = self
            .client
            .get(self.url("credentials"))
            .bearer_auth(&self.token)
            .timeout(WORKER_CONTROL_REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|e| WorkerError::ConnectionFailed {
                url: self.orchestrator_url.clone(),
                reason: e.without_url().to_string(),
            })?;

        // 204 or 404 means no credentials granted, not an error
        if resp.status() == reqwest::StatusCode::NO_CONTENT
            || resp.status() == reqwest::StatusCode::NOT_FOUND
        {
            return Ok(vec![]);
        }

        if !resp.status().is_success() {
            return Err(WorkerError::SecretResolveFailed {
                secret_name: "(all)".to_string(),
                reason: format!("credentials endpoint returned {}", resp.status()),
            });
        }

        deserialize_limited_json(
            resp,
            MAX_WORKER_CONTROL_RESPONSE_BYTES,
            "credentials response",
        )
        .await
        .map_err(|e| WorkerError::SecretResolveFailed {
            secret_name: "(all)".to_string(),
            reason: e.to_string(),
        })
    }

    /// Signal job completion to the orchestrator.
    pub async fn report_complete(&self, report: &CompletionReport) -> Result<(), WorkerError> {
        let _: serde_json::Value = self
            .post_json(
                "complete",
                report,
                "report complete",
                WORKER_COMPLETION_REQUEST_TIMEOUT,
                MAX_WORKER_CONTROL_RESPONSE_BYTES,
            )
            .await?;
        Ok(())
    }
}

async fn read_limited_body(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, WorkerError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(WorkerError::LlmProxyFailed {
            reason: format!("orchestrator response exceeds the {max_bytes} byte limit"),
        });
    }

    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| WorkerError::LlmProxyFailed {
            reason: format!(
                "failed to read orchestrator response: {}",
                error.without_url()
            ),
        })?
    {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(WorkerError::LlmProxyFailed {
                reason: format!("orchestrator response exceeds the {max_bytes} byte limit"),
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn validate_worker_endpoint(raw: &str) -> Result<String, WorkerError> {
    if raw.trim().is_empty()
        || raw.len() > MAX_WORKER_ENDPOINT_BYTES
        || raw.chars().any(char::is_control)
    {
        return Err(WorkerError::ExecutionFailed {
            reason: "orchestrator URL is empty, oversized, or malformed".to_string(),
        });
    }
    let mut url = reqwest::Url::parse(raw).map_err(|_| WorkerError::ExecutionFailed {
        reason: "orchestrator URL is invalid".to_string(),
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(WorkerError::ExecutionFailed {
            reason:
                "orchestrator URL must be an HTTP(S) URL without credentials, query, or fragment"
                    .to_string(),
        });
    }
    let trimmed_path = url.path().trim_end_matches('/').to_string();
    url.set_path(if trimmed_path.is_empty() {
        "/"
    } else {
        &trimmed_path
    });
    Ok(url.as_str().trim_end_matches('/').to_string())
}

async fn deserialize_limited_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    max_bytes: usize,
    context: &str,
) -> Result<T, WorkerError> {
    let body = read_limited_body(response, max_bytes).await?;
    serde_json::from_slice(&body).map_err(|error| WorkerError::LlmProxyFailed {
        reason: format!("{context}: failed to parse response: {error}"),
    })
}

fn parse_finish_reason(s: &str) -> FinishReason {
    match s {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "tool_use" | "tool_calls" => FinishReason::ToolUse,
        "content_filter" => FinishReason::ContentFilter,
        _ => FinishReason::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_construction() {
        let client = WorkerHttpClient::new(
            "http://host.docker.internal:50051".to_string(),
            Uuid::nil(),
            "test-token".to_string(),
        )
        .expect("test client should build");

        assert_eq!(
            client.url("llm/complete"),
            format!(
                "http://host.docker.internal:50051/worker/{}/llm/complete",
                Uuid::nil()
            )
        );
    }

    #[test]
    fn test_parse_finish_reason() {
        assert_eq!(parse_finish_reason("stop"), FinishReason::Stop);
        assert_eq!(parse_finish_reason("tool_use"), FinishReason::ToolUse);
        assert_eq!(parse_finish_reason("unknown"), FinishReason::Unknown);
    }

    #[test]
    fn test_credentials_url_construction() {
        let client = WorkerHttpClient::new(
            "http://host.docker.internal:50051".to_string(),
            Uuid::nil(),
            "test-token".to_string(),
        )
        .expect("test client should build");

        assert_eq!(
            client.url("credentials"),
            format!(
                "http://host.docker.internal:50051/worker/{}/credentials",
                Uuid::nil()
            )
        );
    }

    #[test]
    fn test_job_description_deserialization() {
        let json = r#"{"title":"Test","description":"desc","project_dir":null}"#;
        let job: JobDescription = serde_json::from_str(json).unwrap();
        assert_eq!(job.title, "Test");
        assert_eq!(job.description, "desc");
        assert!(job.project_dir.is_none());
    }
}
