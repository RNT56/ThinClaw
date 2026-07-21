//! ComfyUI REST/WebSocket client and workflow helpers.
//!
//! ComfyUI setup/lifecycle is intentionally separate from workflow execution:
//! the official `comfy` CLI manages installs and models, while generation uses
//! ComfyUI's HTTP and WebSocket APIs directly.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use url::Url;

const DEFAULT_CLIENT_ID_PREFIX: &str = "thinclaw";
const MAX_COMFY_URL_BYTES: usize = 4 * 1024;
const MAX_COMFY_API_KEY_BYTES: usize = 8 * 1024;
const MAX_COMFY_JSON_BYTES: usize = 16 * 1024 * 1024;
const MAX_COMFY_WORKFLOW_NODES: usize = 4_096;
const MAX_COMFY_JSON_VALUES: usize = 100_000;
const MAX_COMFY_JSON_DEPTH: usize = 128;
const MAX_COMFY_OUTPUTS: usize = 128;
const MAX_COMFY_DNS_ADDRESSES: usize = 64;
const COMFY_DNS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ComfyUiMode {
    #[default]
    LocalExisting,
    LocalManaged,
    Cloud,
}

impl ComfyUiMode {
    pub fn is_cloud(self) -> bool {
        matches!(self, Self::Cloud)
    }
}

#[derive(Clone)]
pub struct ComfyUiConfig {
    pub mode: ComfyUiMode,
    pub host: String,
    pub api_key: Option<String>,
    pub output_dir: PathBuf,
    pub request_timeout: Duration,
    pub max_output_bytes: u64,
}

impl std::fmt::Debug for ComfyUiConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComfyUiConfig")
            .field("mode", &self.mode)
            .field("host", &redacted_comfy_endpoint(&self.host))
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("output_dir", &self.output_dir)
            .field("request_timeout", &self.request_timeout)
            .field("max_output_bytes", &self.max_output_bytes)
            .finish()
    }
}

fn redacted_comfy_endpoint(raw: &str) -> String {
    let Ok(url) = Url::parse(raw) else {
        return "<invalid-url>".to_string();
    };
    let Some(host) = url.host_str() else {
        return "<invalid-url>".to_string();
    };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

impl Default for ComfyUiConfig {
    fn default() -> Self {
        Self {
            mode: ComfyUiMode::LocalExisting,
            host: "http://127.0.0.1:8188".to_string(),
            api_key: None,
            output_dir: thinclaw_platform::resolve_data_dir("media_cache").join("generated"),
            request_timeout: Duration::from_secs(600),
            max_output_bytes: 100 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComfyGenerateRequest {
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub aspect_ratio: ComfyAspectRatio,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub seed: Option<i64>,
    pub steps: Option<u32>,
    pub cfg: Option<f64>,
    pub model: Option<String>,
    pub workflow: Value,
    pub workflow_name: String,
    pub input_image: Option<PathBuf>,
    pub mask_image: Option<PathBuf>,
    pub wait_for_completion: bool,
    pub use_websocket: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ComfyAspectRatio {
    #[default]
    Square,
    Landscape,
    Portrait,
    Wide,
    Tall,
}

impl ComfyAspectRatio {
    pub fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Square => (1024, 1024),
            Self::Landscape => (1216, 832),
            Self::Portrait => (832, 1216),
            Self::Wide => (1344, 768),
            Self::Tall => (768, 1344),
        }
    }
}

impl std::str::FromStr for ComfyAspectRatio {
    type Err = ComfyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "square" | "1:1" => Ok(Self::Square),
            "landscape" | "4:3" | "3:2" => Ok(Self::Landscape),
            "portrait" | "3:4" | "2:3" => Ok(Self::Portrait),
            "wide" | "16:9" => Ok(Self::Wide),
            "tall" | "9:16" => Ok(Self::Tall),
            other => Err(ComfyError::InvalidWorkflow(format!(
                "invalid aspect_ratio '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComfyGeneration {
    pub prompt_id: String,
    pub client_id: String,
    pub workflow_name: String,
    pub seed: i64,
    pub width: u32,
    pub height: u32,
    pub outputs: Vec<ComfySavedOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComfySavedOutput {
    pub file_path: PathBuf,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComfyDependencyReport {
    pub workflow_valid: bool,
    pub missing_nodes: Vec<String>,
    pub model_references: Vec<ComfyModelReference>,
    pub missing_models: Vec<ComfyModelReference>,
    pub embedding_references: Vec<String>,
}

impl ComfyDependencyReport {
    pub fn is_ready(&self) -> bool {
        self.workflow_valid && self.missing_nodes.is_empty() && self.missing_models.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ComfyModelReference {
    pub folder: String,
    pub name: String,
    pub node_id: String,
    pub class_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComfyHealth {
    pub reachable: bool,
    pub mode: ComfyUiMode,
    pub host: String,
    pub system_stats: Option<Value>,
    pub object_info_available: bool,
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ComfyError {
    #[error("ComfyUI HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("ComfyUI URL error: {0}")]
    Url(#[from] url::ParseError),

    #[error("ComfyUI I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ComfyUI API error: {0}")]
    Api(String),

    #[error("Invalid ComfyUI workflow: {0}")]
    InvalidWorkflow(String),

    #[error("ComfyUI job timed out after {0:?}")]
    Timeout(Duration),

    #[error("ComfyUI output rejected: {0}")]
    UnsafeOutput(String),
}

#[derive(Clone)]
pub struct ComfyUiClient {
    config: ComfyUiConfig,
}

impl ComfyUiClient {
    pub fn new(config: ComfyUiConfig) -> Result<Self, ComfyError> {
        if config.host.is_empty()
            || config.host.len() > MAX_COMFY_URL_BYTES
            || config.request_timeout.is_zero()
            || config.request_timeout > Duration::from_secs(24 * 60 * 60)
            || config.max_output_bytes == 0
            || config.max_output_bytes > 1024 * 1024 * 1024
        {
            return Err(ComfyError::Api(
                "ComfyUI timeout or output limit is outside the supported bounds".to_string(),
            ));
        }
        let base = Url::parse(&config.host)?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || (config.mode.is_cloud() && base.scheme() != "https")
        {
            return Err(ComfyError::Api(
                "ComfyUI host must be an HTTP(S) base URL without credentials, query, or fragment"
                    .to_string(),
            ));
        }
        if config.api_key.as_ref().is_some_and(|api_key| {
            api_key.is_empty()
                || api_key.len() > MAX_COMFY_API_KEY_BYTES
                || api_key.chars().any(char::is_control)
        }) {
            return Err(ComfyError::Api(
                "ComfyUI API key is empty, oversized, or contains control characters".to_string(),
            ));
        }
        Ok(Self { config })
    }

    pub fn config(&self) -> &ComfyUiConfig {
        &self.config
    }

    pub async fn health(&self) -> ComfyHealth {
        let system_stats = match self.get_json("/system_stats").await {
            Ok(value) => value,
            Err(error) => {
                return ComfyHealth {
                    reachable: false,
                    mode: self.config.mode,
                    host: self.config.host.clone(),
                    system_stats: None,
                    object_info_available: false,
                    error: Some(error.to_string()),
                };
            }
        };

        let object_info_available = self.get_json("/object_info").await.is_ok();
        ComfyHealth {
            reachable: true,
            mode: self.config.mode,
            host: self.config.host.clone(),
            system_stats: Some(system_stats),
            object_info_available,
            error: None,
        }
    }

    pub async fn object_info(&self) -> Result<Value, ComfyError> {
        self.get_json("/object_info").await
    }

    pub async fn check_dependencies(
        &self,
        workflow: &Value,
    ) -> Result<ComfyDependencyReport, ComfyError> {
        validate_api_workflow(workflow)?;
        let object_info = self.object_info().await.unwrap_or_else(|_| json!({}));
        let available_nodes = object_info
            .as_object()
            .map(|map| map.keys().cloned().collect::<BTreeSet<_>>())
            .unwrap_or_default();
        let workflow_nodes = workflow_nodes(workflow)?;
        let mut missing_nodes = BTreeSet::new();

        for node in workflow_nodes.values() {
            let class_type = node
                .get("class_type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !class_type.is_empty()
                && !available_nodes.is_empty()
                && !available_nodes.contains(class_type)
            {
                missing_nodes.insert(class_type.to_string());
            }
        }

        let refs = collect_model_references(workflow)?;
        let mut missing_models = Vec::new();
        for model_ref in &refs {
            let available = self
                .model_names(&model_ref.folder)
                .await
                .unwrap_or_default();
            if !available.is_empty() && !model_name_matches(&available, &model_ref.name) {
                missing_models.push(model_ref.clone());
            }
        }

        Ok(ComfyDependencyReport {
            workflow_valid: true,
            missing_nodes: missing_nodes.into_iter().collect(),
            model_references: refs,
            missing_models,
            embedding_references: collect_embedding_references(workflow)?,
        })
    }

    pub async fn generate(
        &self,
        request: ComfyGenerateRequest,
    ) -> Result<ComfyGeneration, ComfyError> {
        validate_api_workflow(&request.workflow)?;
        validate_generate_request(&request)?;
        let output_dir = ensure_real_output_directory(&self.config.output_dir).await?;

        let mut workflow = request.workflow.clone();
        let (default_width, default_height) = request.aspect_ratio.dimensions();
        let width = request.width.unwrap_or(default_width);
        let height = request.height.unwrap_or(default_height);
        let seed = request.seed.unwrap_or_else(random_seed);

        if let Some(path) = request.input_image.as_deref() {
            let uploaded = self.upload_image(path).await?;
            inject_image_reference(&mut workflow, &uploaded, false)?;
        }
        if let Some(path) = request.mask_image.as_deref() {
            let uploaded = self.upload_image(path).await?;
            inject_image_reference(&mut workflow, &uploaded, true)?;
        }

        inject_generation_params(
            &mut workflow,
            GenerationParams {
                prompt: &request.prompt,
                negative_prompt: request.negative_prompt.as_deref(),
                seed,
                width,
                height,
                steps: request.steps,
                cfg: request.cfg,
                model: request.model.as_deref(),
            },
        )?;
        validate_api_workflow(&workflow)?;

        let client_id = format!("{DEFAULT_CLIENT_ID_PREFIX}-{}", uuid::Uuid::new_v4());
        let prompt_id = self.queue_prompt(&workflow, &client_id).await?;

        if !request.wait_for_completion {
            return Ok(ComfyGeneration {
                prompt_id,
                client_id,
                workflow_name: request.workflow_name,
                seed,
                width,
                height,
                outputs: Vec::new(),
            });
        }

        if request.use_websocket {
            tracing::debug!(
                "ComfyUI websocket completion requested; using DNS-pinned HTTP polling"
            );
        }
        let history = self.wait_for_prompt_polling(&prompt_id).await?;
        let outputs = self
            .download_outputs(&prompt_id, &history, &output_dir)
            .await?;

        Ok(ComfyGeneration {
            prompt_id,
            client_id,
            workflow_name: request.workflow_name,
            seed,
            width,
            height,
            outputs,
        })
    }

    pub async fn queue_prompt(
        &self,
        workflow: &Value,
        client_id: &str,
    ) -> Result<String, ComfyError> {
        validate_api_workflow(workflow)?;
        validate_identifier("client_id", client_id)?;
        let response = self
            .post_json(
                "/prompt",
                &json!({ "prompt": workflow, "client_id": client_id }),
            )
            .await?;
        let prompt_id = response
            .get("prompt_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| ComfyError::Api("ComfyUI response omitted prompt_id".to_string()))?;
        validate_identifier("prompt_id", &prompt_id)?;
        Ok(prompt_id)
    }

    pub async fn interrupt(&self) -> Result<(), ComfyError> {
        let _ = self.post_json("/interrupt", &json!({})).await?;
        Ok(())
    }

    pub async fn upload_image(&self, path: &Path) -> Result<String, ComfyError> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                ComfyError::UnsafeOutput(format!("invalid input image path {}", path.display()))
            })?
            .to_string();
        if file_name.is_empty()
            || file_name.len() > 255
            || file_name.chars().any(char::is_control)
            || file_name.contains(['/', '\\'])
        {
            return Err(ComfyError::UnsafeOutput(
                "input image filename is malformed".to_string(),
            ));
        }
        let bytes = thinclaw_platform::read_regular_file_bounded_async(
            path.to_path_buf(),
            self.config.max_output_bytes,
        )
        .await?;
        let upload_name = format!("thinclaw_{}_{}", uuid::Uuid::new_v4().simple(), file_name);
        let part = reqwest::multipart::Part::bytes(bytes).file_name(upload_name.clone());
        let form = reqwest::multipart::Form::new()
            .part("image", part)
            .text("overwrite", "false");
        let response = self
            .request(reqwest::Method::POST, "/upload/image")
            .await?
            .multipart(form)
            .send()
            .await?
            .error_for_status()?;
        let value: Value = thinclaw_types::http_response::bounded_json(response, 4 * 1024 * 1024)
            .await
            .map_err(|error| ComfyError::Api(format!("invalid upload response: {error}")))?;
        let returned_name = value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&upload_name);
        if returned_name.is_empty()
            || returned_name.len() > 1024
            || returned_name.chars().any(char::is_control)
            || returned_name.contains(['/', '\\'])
        {
            return Err(ComfyError::UnsafeOutput(
                "ComfyUI returned an unsafe upload name".to_string(),
            ));
        }
        Ok(returned_name.to_string())
    }

    async fn wait_for_prompt_polling(&self, prompt_id: &str) -> Result<Value, ComfyError> {
        let start = Instant::now();
        loop {
            let Some(remaining) = self.config.request_timeout.checked_sub(start.elapsed()) else {
                return Err(ComfyError::Timeout(self.config.request_timeout));
            };
            let history = tokio::time::timeout(remaining, self.history(prompt_id))
                .await
                .map_err(|_| ComfyError::Timeout(self.config.request_timeout))??;
            if history_contains_prompt(&history, prompt_id) {
                return Ok(history);
            }
            let Some(remaining) = self.config.request_timeout.checked_sub(start.elapsed()) else {
                return Err(ComfyError::Timeout(self.config.request_timeout));
            };
            tokio::time::sleep(Duration::from_secs(2).min(remaining)).await;
        }
    }

    async fn history(&self, prompt_id: &str) -> Result<Value, ComfyError> {
        if self.config.mode.is_cloud() {
            self.get_json(&format!("/history_v2/{prompt_id}")).await
        } else {
            self.get_json(&format!("/history/{prompt_id}")).await
        }
    }

    async fn model_names(&self, folder: &str) -> Result<Vec<String>, ComfyError> {
        let path = if self.config.mode.is_cloud() {
            format!("/experiment/models/{folder}")
        } else {
            format!("/models/{folder}")
        };
        let value = self.get_json(&path).await?;
        Ok(normalize_model_list(&value))
    }

    async fn download_outputs(
        &self,
        prompt_id: &str,
        history: &Value,
        output_dir: &Path,
    ) -> Result<Vec<ComfySavedOutput>, ComfyError> {
        let output_entries = collect_output_entries(prompt_id, history)?;
        let mut saved = Vec::new();
        let mut total_bytes = 0_u64;

        for entry in output_entries {
            let remaining = self.config.max_output_bytes.saturating_sub(total_bytes);
            if remaining == 0 {
                return Err(ComfyError::UnsafeOutput(format!(
                    "ComfyUI outputs exceeded the {}-byte aggregate limit",
                    self.config.max_output_bytes
                )));
            }
            let bytes = self.download_view(&entry, remaining).await?;
            total_bytes = total_bytes.saturating_add(bytes.len() as u64);
            let filename = safe_output_filename(&entry.filename)?;
            let path = unique_output_path(output_dir, &filename);
            write_new_output_file(path.clone(), bytes).await?;
            let size_bytes = tokio::fs::metadata(&path).await?.len();
            let mime_type = mime_guess::from_path(&path)
                .first_or_octet_stream()
                .essence_str()
                .to_string();
            saved.push(ComfySavedOutput {
                file_path: path,
                filename,
                mime_type,
                size_bytes,
                media_type: entry.media_type,
            });
        }

        Ok(saved)
    }

    async fn download_view(
        &self,
        entry: &ComfyOutputEntry,
        max_bytes: u64,
    ) -> Result<Vec<u8>, ComfyError> {
        let mut url = self.url("/view")?;
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("filename", &entry.filename);
            qp.append_pair("subfolder", &entry.subfolder);
            qp.append_pair("type", &entry.output_type);
        }
        let response = self
            .request_url(reqwest::Method::GET, url)
            .await?
            .send()
            .await?;

        if response.status().is_redirection() {
            return Err(ComfyError::Api(
                "ComfyUI output redirects are not accepted".to_string(),
            ));
        }

        let limit = usize::try_from(max_bytes).map_err(|_| {
            ComfyError::UnsafeOutput("ComfyUI output limit does not fit this platform".to_string())
        })?;
        thinclaw_types::http_response::bounded_bytes(response.error_for_status()?, limit)
            .await
            .map_err(|error| ComfyError::UnsafeOutput(error.to_string()))
    }

    async fn get_json(&self, path: &str) -> Result<Value, ComfyError> {
        let response = self
            .request(reqwest::Method::GET, path)
            .await?
            .send()
            .await?
            .error_for_status()?;
        thinclaw_types::http_response::bounded_json(response, MAX_COMFY_JSON_BYTES)
            .await
            .map_err(|error| ComfyError::Api(format!("invalid ComfyUI JSON response: {error}")))
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<Value, ComfyError> {
        let response = self
            .request(reqwest::Method::POST, path)
            .await?
            .json(body)
            .send()
            .await?
            .error_for_status()?;
        thinclaw_types::http_response::bounded_json(response, MAX_COMFY_JSON_BYTES)
            .await
            .map_err(|error| ComfyError::Api(format!("invalid ComfyUI JSON response: {error}")))
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, ComfyError> {
        let url = self.url(path)?;
        self.request_url(method, url).await
    }

    async fn request_url(
        &self,
        method: reqwest::Method,
        url: Url,
    ) -> Result<reqwest::RequestBuilder, ComfyError> {
        let client = self.client_for_url(&url).await?;
        Ok(client.request(method, url).headers(self.auth_headers()?))
    }

    async fn client_for_url(&self, url: &Url) -> Result<reqwest::Client, ComfyError> {
        let (url, pinned_addrs) = if self.config.mode.is_cloud() {
            let guarded = thinclaw_tools_core::validate_outbound_url_pinned_async(
                url.as_str(),
                &thinclaw_tools_core::OutboundUrlGuardOptions {
                    require_https: true,
                    upgrade_http_to_https: false,
                    allowlist: Vec::new(),
                },
            )
            .await
            .map_err(|error| ComfyError::Api(format!("unsafe ComfyUI URL: {error}")))?;
            (guarded.url, guarded.pinned_addrs)
        } else {
            (url.clone(), resolve_configured_host(url).await?)
        };
        let host = url
            .host_str()
            .ok_or_else(|| ComfyError::Api("ComfyUI URL has no host".to_string()))?;
        let mut builder = reqwest::Client::builder()
            .timeout(self.config.request_timeout)
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        if !pinned_addrs.is_empty() {
            builder = builder.resolve_to_addrs(host, &pinned_addrs);
        }
        builder
            .build()
            .map_err(|error| ComfyError::Api(format!("failed to build ComfyUI client: {error}")))
    }

    fn auth_headers(&self) -> Result<HeaderMap, ComfyError> {
        let mut headers = HeaderMap::new();
        if self.config.mode.is_cloud()
            && let Some(api_key) = self.config.api_key.as_deref()
        {
            headers.insert(
                "X-API-Key",
                HeaderValue::from_str(api_key).map_err(|e| {
                    ComfyError::Api(format!("invalid Comfy Cloud API key header: {e}"))
                })?,
            );
        }
        Ok(headers)
    }

    fn url(&self, path: &str) -> Result<Url, ComfyError> {
        let base = self.config.host.trim_end_matches('/');
        let normalized = if self.config.mode.is_cloud() && !path.starts_with("/api/") {
            format!("/api{}", ensure_leading_slash(path))
        } else {
            ensure_leading_slash(path)
        };
        Ok(Url::parse(&format!("{base}{normalized}"))?)
    }
}

async fn resolve_configured_host(url: &Url) -> Result<Vec<SocketAddr>, ComfyError> {
    let host = url
        .host_str()
        .ok_or_else(|| ComfyError::Api("ComfyUI URL has no host".to_string()))?;
    if host.parse::<IpAddr>().is_ok() {
        return Ok(Vec::new());
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| ComfyError::Api("ComfyUI URL has no usable port".to_string()))?;
    let resolved = tokio::time::timeout(COMFY_DNS_TIMEOUT, tokio::net::lookup_host((host, port)))
        .await
        .map_err(|_| ComfyError::Api("ComfyUI hostname resolution timed out".to_string()))?
        .map_err(|error| ComfyError::Api(format!("failed to resolve ComfyUI hostname: {error}")))?;
    let mut addresses = Vec::new();
    for address in resolved {
        if addresses.len() >= MAX_COMFY_DNS_ADDRESSES {
            return Err(ComfyError::Api(format!(
                "ComfyUI hostname resolved to more than {MAX_COMFY_DNS_ADDRESSES} addresses"
            )));
        }
        addresses.push(address);
    }
    if addresses.is_empty() {
        return Err(ComfyError::Api(
            "ComfyUI hostname resolved to no addresses".to_string(),
        ));
    }
    addresses.sort_unstable();
    addresses.dedup();
    Ok(addresses)
}

async fn ensure_real_output_directory(path: &Path) -> Result<PathBuf, ComfyError> {
    tokio::fs::create_dir_all(path).await?;
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ComfyError::UnsafeOutput(
            "ComfyUI output directory is not a real directory".to_string(),
        ));
    }
    let canonical = tokio::fs::canonicalize(path).await?;
    let canonical_metadata = tokio::fs::symlink_metadata(&canonical).await?;
    if canonical_metadata.file_type().is_symlink() || !canonical_metadata.is_dir() {
        return Err(ComfyError::UnsafeOutput(
            "ComfyUI output directory did not resolve to a real directory".to_string(),
        ));
    }
    Ok(canonical)
}

async fn write_new_output_file(path: PathBuf, bytes: Vec<u8>) -> Result<(), ComfyError> {
    tokio::task::spawn_blocking(move || {
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("ComfyUI output has no parent directory"))?;
        let parent_metadata = std::fs::symlink_metadata(parent)?;
        if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
            return Err(std::io::Error::other(
                "ComfyUI output parent is not a real directory",
            ));
        }
        let expected_len = u64::try_from(bytes.len())
            .map_err(|_| std::io::Error::other("ComfyUI output is too large"))?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let result = (|| -> std::io::Result<()> {
            let mut file = options.open(&path)?;
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::other(
                    "ComfyUI output target is not a regular file",
                ));
            }
            file.write_all(&bytes)?;
            file.sync_all()?;
            if file.metadata()?.len() != expected_len {
                return Err(std::io::Error::other(
                    "ComfyUI output changed while it was written",
                ));
            }
            #[cfg(unix)]
            std::fs::File::open(parent)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&path);
        }
        result
    })
    .await
    .map_err(|error| {
        ComfyError::Io(std::io::Error::other(format!(
            "ComfyUI output writer panicked: {error}"
        )))
    })??;
    Ok(())
}

fn validate_generate_request(request: &ComfyGenerateRequest) -> Result<(), ComfyError> {
    let (default_width, default_height) = request.aspect_ratio.dimensions();
    let width = request.width.unwrap_or(default_width);
    let height = request.height.unwrap_or(default_height);
    if !(64..=8_192).contains(&width)
        || !(64..=8_192).contains(&height)
        || u64::from(width).saturating_mul(u64::from(height)) > 64 * 1024 * 1024
    {
        return Err(ComfyError::InvalidWorkflow(
            "generation dimensions are outside the supported bounds".to_string(),
        ));
    }
    if request.prompt.is_empty()
        || request.prompt.len() > 100_000
        || request
            .negative_prompt
            .as_ref()
            .is_some_and(|value| value.len() > 100_000)
        || request.workflow_name.is_empty()
        || request.workflow_name.len() > 256
        || request.workflow_name.chars().any(char::is_control)
        || request.model.as_ref().is_some_and(|value| {
            value.is_empty() || value.len() > 1_024 || value.chars().any(char::is_control)
        })
        || request
            .steps
            .is_some_and(|steps| !(1..=1_000).contains(&steps))
        || request
            .cfg
            .is_some_and(|cfg| !cfg.is_finite() || !(0.0..=100.0).contains(&cfg))
    {
        return Err(ComfyError::InvalidWorkflow(
            "generation parameters are empty, oversized, or outside supported bounds".to_string(),
        ));
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> Result<(), ComfyError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ComfyError::Api(format!("ComfyUI {label} is malformed")));
    }
    Ok(())
}

fn ensure_leading_slash(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

pub fn bundled_workflow(name: &str) -> Option<Value> {
    let raw = match name {
        "sdxl_txt2img" | "default" => SDXL_TXT2IMG,
        "sd15_txt2img" => SD15_TXT2IMG,
        "sdxl_img2img" => SDXL_IMG2IMG,
        "upscale_4x" => UPSCALE_4X,
        _ => return None,
    };
    serde_json::from_str(raw).ok()
}

pub fn bundled_workflow_names() -> &'static [&'static str] {
    &["sdxl_txt2img", "sd15_txt2img", "sdxl_img2img", "upscale_4x"]
}

pub fn validate_api_workflow(workflow: &Value) -> Result<(), ComfyError> {
    validate_json_complexity(workflow)?;
    let object = workflow.as_object().ok_or_else(|| {
        ComfyError::InvalidWorkflow(
            "workflow must be a JSON object of ComfyUI API nodes".to_string(),
        )
    })?;
    if object.contains_key("nodes") || object.contains_key("links") {
        return Err(ComfyError::InvalidWorkflow(
            "editor-format workflows are not executable; export/save in ComfyUI API format"
                .to_string(),
        ));
    }
    if object.is_empty() {
        return Err(ComfyError::InvalidWorkflow(
            "workflow has no nodes".to_string(),
        ));
    }
    if object.len() > MAX_COMFY_WORKFLOW_NODES {
        return Err(ComfyError::InvalidWorkflow(format!(
            "workflow contains more than {MAX_COMFY_WORKFLOW_NODES} nodes"
        )));
    }
    for (node_id, node) in object {
        if node_id.is_empty() || node_id.len() > 128 || node_id.chars().any(char::is_control) {
            return Err(ComfyError::InvalidWorkflow(
                "workflow contains a malformed node identifier".to_string(),
            ));
        }
        let class_type = node.get("class_type").and_then(Value::as_str);
        if class_type.is_none_or(|value| {
            value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
        }) {
            return Err(ComfyError::InvalidWorkflow(format!(
                "node {node_id} has a missing or malformed class_type"
            )));
        }
        if !node.get("inputs").is_some_and(Value::is_object) {
            return Err(ComfyError::InvalidWorkflow(format!(
                "node {node_id} is missing object inputs"
            )));
        }
    }
    Ok(())
}

fn validate_json_complexity(value: &Value) -> Result<(), ComfyError> {
    let mut stack = vec![(value, 0_usize)];
    let mut values = 0_usize;
    let mut text_bytes = 0_usize;
    while let Some((current, depth)) = stack.pop() {
        values = values.saturating_add(1);
        if values > MAX_COMFY_JSON_VALUES || depth > MAX_COMFY_JSON_DEPTH {
            return Err(ComfyError::InvalidWorkflow(
                "workflow JSON is too deep or complex".to_string(),
            ));
        }
        match current {
            Value::String(text) => text_bytes = text_bytes.saturating_add(text.len()),
            Value::Array(items) => {
                stack.extend(items.iter().map(|item| (item, depth.saturating_add(1))));
            }
            Value::Object(object) => {
                for (key, child) in object {
                    text_bytes = text_bytes.saturating_add(key.len());
                    stack.push((child, depth.saturating_add(1)));
                }
            }
            _ => {}
        }
        if text_bytes > MAX_COMFY_JSON_BYTES {
            return Err(ComfyError::InvalidWorkflow(format!(
                "workflow JSON exceeds the {MAX_COMFY_JSON_BYTES}-byte text limit"
            )));
        }
    }
    Ok(())
}

fn workflow_nodes(workflow: &Value) -> Result<&Map<String, Value>, ComfyError> {
    validate_api_workflow(workflow)?;
    workflow
        .as_object()
        .ok_or_else(|| ComfyError::InvalidWorkflow("workflow must be an object".to_string()))
}

struct GenerationParams<'a> {
    prompt: &'a str,
    negative_prompt: Option<&'a str>,
    seed: i64,
    width: u32,
    height: u32,
    steps: Option<u32>,
    cfg: Option<f64>,
    model: Option<&'a str>,
}

fn inject_generation_params(
    workflow: &mut Value,
    params: GenerationParams<'_>,
) -> Result<(), ComfyError> {
    let nodes = workflow.as_object_mut().ok_or_else(|| {
        ComfyError::InvalidWorkflow("workflow must be a mutable object".to_string())
    })?;
    let mut positive_set = false;
    let mut negative_set = params.negative_prompt.is_none();
    let mut dimensions_set = false;
    let mut seed_set = false;

    for node in nodes.values_mut() {
        let class_type = node
            .get("class_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) else {
            continue;
        };

        match class_type.as_str() {
            "CLIPTextEncode" => {
                if let Some(text_value) = inputs.get("text").and_then(Value::as_str) {
                    let lower = text_value.to_ascii_lowercase();
                    let is_negative = lower.contains("negative")
                        || lower.contains("low quality")
                        || lower.contains("bad anatomy");
                    if is_negative {
                        if let Some(negative) = params.negative_prompt {
                            inputs.insert("text".to_string(), Value::String(negative.to_string()));
                        }
                        negative_set = true;
                    } else if !positive_set {
                        inputs.insert("text".to_string(), Value::String(params.prompt.to_string()));
                        positive_set = true;
                    }
                }
            }
            "KSampler" | "KSamplerAdvanced" => {
                if inputs.contains_key("seed") || inputs.contains_key("noise_seed") {
                    let key = if inputs.contains_key("seed") {
                        "seed"
                    } else {
                        "noise_seed"
                    };
                    inputs.insert(key.to_string(), json!(params.seed));
                    seed_set = true;
                }
                if let Some(steps) = params.steps
                    && inputs.contains_key("steps")
                {
                    inputs.insert("steps".to_string(), json!(steps));
                }
                if let Some(cfg) = params.cfg
                    && inputs.contains_key("cfg")
                {
                    inputs.insert("cfg".to_string(), json!(cfg));
                }
            }
            "EmptyLatentImage" => {
                inputs.insert("width".to_string(), json!(params.width));
                inputs.insert("height".to_string(), json!(params.height));
                dimensions_set = true;
            }
            "CheckpointLoaderSimple" => {
                if let Some(model) = params.model
                    && inputs.contains_key("ckpt_name")
                {
                    inputs.insert("ckpt_name".to_string(), Value::String(model.to_string()));
                }
            }
            _ => {}
        }
    }

    if !positive_set {
        return Err(ComfyError::InvalidWorkflow(
            "workflow has no writable positive CLIPTextEncode text input".to_string(),
        ));
    }
    if !negative_set {
        return Err(ComfyError::InvalidWorkflow(
            "workflow has no writable negative CLIPTextEncode text input".to_string(),
        ));
    }
    if !dimensions_set {
        tracing::debug!(
            "ComfyUI workflow has no EmptyLatentImage node; dimensions were not injected"
        );
    }
    if !seed_set {
        tracing::debug!("ComfyUI workflow has no sampler seed input; seed was not injected");
    }
    Ok(())
}

fn inject_image_reference(
    workflow: &mut Value,
    uploaded_name: &str,
    mask: bool,
) -> Result<(), ComfyError> {
    let nodes = workflow.as_object_mut().ok_or_else(|| {
        ComfyError::InvalidWorkflow("workflow must be a mutable object".to_string())
    })?;
    let mut injected = false;

    for node in nodes.values_mut() {
        let class_type = node
            .get("class_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) else {
            continue;
        };
        let target = if mask {
            class_type.contains("Mask") || inputs.contains_key("mask")
        } else {
            class_type == "LoadImage" || inputs.contains_key("image")
        };
        if target {
            let key = if mask && inputs.contains_key("mask") {
                "mask"
            } else {
                "image"
            };
            inputs.insert(key.to_string(), Value::String(uploaded_name.to_string()));
            injected = true;
        }
    }

    if injected {
        Ok(())
    } else {
        Err(ComfyError::InvalidWorkflow(format!(
            "workflow has no writable {} input node",
            if mask { "mask" } else { "image" }
        )))
    }
}

fn collect_model_references(workflow: &Value) -> Result<Vec<ComfyModelReference>, ComfyError> {
    let mut refs = BTreeSet::new();
    for (node_id, node) in workflow_nodes(workflow)? {
        let class_type = node
            .get("class_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let inputs = node.get("inputs").and_then(Value::as_object);
        let Some(inputs) = inputs else {
            continue;
        };
        for &(field, folder) in model_loader_fields(class_type) {
            if let Some(name) = inputs.get(field).and_then(Value::as_str)
                && !name.trim().is_empty()
            {
                refs.insert(ComfyModelReference {
                    folder: folder.to_string(),
                    name: name.to_string(),
                    node_id: node_id.clone(),
                    class_type: class_type.to_string(),
                });
            }
        }
    }
    Ok(refs.into_iter().collect())
}

fn model_loader_fields(class_type: &str) -> &'static [(&'static str, &'static str)] {
    match class_type {
        "CheckpointLoaderSimple" | "CheckpointLoader" => &[("ckpt_name", "checkpoints")],
        "UNETLoader" => &[("unet_name", "diffusion_models")],
        "VAELoader" => &[("vae_name", "vae")],
        "CLIPLoader" | "DualCLIPLoader" => &[
            ("clip_name", "clip"),
            ("clip_name1", "clip"),
            ("clip_name2", "clip"),
        ],
        "LoraLoader" | "LoraLoaderModelOnly" => &[("lora_name", "loras")],
        "ControlNetLoader" => &[("control_net_name", "controlnet")],
        "UpscaleModelLoader" => &[("model_name", "upscale_models")],
        _ => &[],
    }
}

fn collect_embedding_references(workflow: &Value) -> Result<Vec<String>, ComfyError> {
    let mut refs = BTreeSet::new();
    for node in workflow_nodes(workflow)?.values() {
        if node.get("class_type").and_then(Value::as_str) != Some("CLIPTextEncode") {
            continue;
        }
        if let Some(text) = node
            .get("inputs")
            .and_then(Value::as_object)
            .and_then(|inputs| inputs.get("text"))
            .and_then(Value::as_str)
        {
            for word in text.split_whitespace() {
                if let Some(rest) = word.strip_prefix("embedding:") {
                    refs.insert(
                        rest.trim_matches(|c: char| c == ',' || c == ')' || c == '(')
                            .to_string(),
                    );
                }
            }
        }
    }
    Ok(refs.into_iter().collect())
}

fn normalize_model_list(value: &Value) -> Vec<String> {
    if let Some(list) = value.as_array() {
        return list
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .map(ToOwned::to_owned)
                    .or_else(|| {
                        item.get("name")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    })
                    .or_else(|| {
                        item.get("filename")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    })
            })
            .collect();
    }
    if let Some(models) = value.get("models").and_then(Value::as_array) {
        return normalize_model_list(&Value::Array(models.clone()));
    }
    Vec::new()
}

fn model_name_matches(available: &[String], wanted: &str) -> bool {
    let wanted_lower = wanted.to_ascii_lowercase();
    available.iter().any(|name| {
        let lower = name.to_ascii_lowercase();
        lower == wanted_lower || lower.ends_with(&format!("/{wanted_lower}"))
    })
}

fn random_seed() -> i64 {
    let uuid = uuid::Uuid::new_v4();
    let bytes = uuid.as_u128().to_le_bytes();
    let mut seed_bytes = [0_u8; 8];
    seed_bytes.copy_from_slice(&bytes[..8]);
    i64::from_le_bytes(seed_bytes) & i64::MAX
}

fn history_contains_prompt(history: &Value, prompt_id: &str) -> bool {
    if history.get(prompt_id).is_some() {
        return true;
    }
    history
        .get("prompt_id")
        .and_then(Value::as_str)
        .is_some_and(|id| id == prompt_id)
}

#[derive(Debug, Clone)]
struct ComfyOutputEntry {
    filename: String,
    subfolder: String,
    output_type: String,
    media_type: String,
}

fn collect_output_entries(
    prompt_id: &str,
    history: &Value,
) -> Result<Vec<ComfyOutputEntry>, ComfyError> {
    let prompt_history = history.get(prompt_id).unwrap_or(history);
    let outputs = prompt_history.get("outputs").unwrap_or(prompt_history);
    let mut entries = Vec::new();
    let Some(outputs) = outputs.as_object() else {
        return Ok(entries);
    };

    for node_output in outputs.values() {
        for (key, media_type) in [
            ("images", "image"),
            ("gifs", "video"),
            ("videos", "video"),
            ("audio", "audio"),
            ("files", "file"),
        ] {
            if let Some(list) = node_output.get(key).and_then(Value::as_array) {
                for item in list {
                    if let Some(filename) = item.get("filename").and_then(Value::as_str) {
                        if entries.len() >= MAX_COMFY_OUTPUTS {
                            return Err(ComfyError::UnsafeOutput(format!(
                                "ComfyUI returned more than {MAX_COMFY_OUTPUTS} outputs"
                            )));
                        }
                        let subfolder = item.get("subfolder").and_then(Value::as_str).unwrap_or("");
                        let output_type =
                            item.get("type").and_then(Value::as_str).unwrap_or("output");
                        validate_output_parameter("filename", filename, 1_024)?;
                        validate_output_parameter("subfolder", subfolder, 4_096)?;
                        validate_output_parameter("type", output_type, 128)?;
                        entries.push(ComfyOutputEntry {
                            filename: filename.to_string(),
                            subfolder: subfolder.to_string(),
                            output_type: output_type.to_string(),
                            media_type: media_type.to_string(),
                        });
                    }
                }
            }
        }
    }
    Ok(entries)
}

fn validate_output_parameter(label: &str, value: &str, max_bytes: usize) -> Result<(), ComfyError> {
    if value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(ComfyError::UnsafeOutput(format!(
            "ComfyUI returned a malformed output {label}"
        )));
    }
    Ok(())
}

fn safe_output_filename(filename: &str) -> Result<String, ComfyError> {
    let name = Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ComfyError::UnsafeOutput(format!("invalid output filename {filename}")))?;
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 255
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(char::is_control)
        || name != filename
    {
        return Err(ComfyError::UnsafeOutput(format!(
            "unsafe output filename {filename}"
        )));
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 16
                && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        });
    Ok(match extension {
        Some(extension) => format!("comfyui_{}.{}", uuid::Uuid::new_v4(), extension),
        None => format!("comfyui_{}", uuid::Uuid::new_v4()),
    })
}

fn unique_output_path(output_dir: &Path, filename: &str) -> PathBuf {
    output_dir.join(filename)
}

const SDXL_TXT2IMG: &str = r#"{
  "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "sd_xl_base_1.0.safetensors"}},
  "5": {"class_type": "CLIPTextEncode", "inputs": {"text": "positive prompt", "clip": ["4", 1]}},
  "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "negative prompt, low quality, blurry", "clip": ["4", 1]}},
  "7": {"class_type": "EmptyLatentImage", "inputs": {"width": 1024, "height": 1024, "batch_size": 1}},
  "3": {"class_type": "KSampler", "inputs": {"seed": 1, "steps": 25, "cfg": 7.0, "sampler_name": "euler", "scheduler": "normal", "denoise": 1.0, "model": ["4", 0], "positive": ["5", 0], "negative": ["6", 0], "latent_image": ["7", 0]}},
  "8": {"class_type": "VAEDecode", "inputs": {"samples": ["3", 0], "vae": ["4", 2]}},
  "9": {"class_type": "SaveImage", "inputs": {"filename_prefix": "thinclaw", "images": ["8", 0]}}
}"#;

const SD15_TXT2IMG: &str = r#"{
  "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "v1-5-pruned-emaonly.safetensors"}},
  "5": {"class_type": "CLIPTextEncode", "inputs": {"text": "positive prompt", "clip": ["4", 1]}},
  "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "negative prompt, low quality, blurry", "clip": ["4", 1]}},
  "7": {"class_type": "EmptyLatentImage", "inputs": {"width": 768, "height": 768, "batch_size": 1}},
  "3": {"class_type": "KSampler", "inputs": {"seed": 1, "steps": 25, "cfg": 7.0, "sampler_name": "euler", "scheduler": "normal", "denoise": 1.0, "model": ["4", 0], "positive": ["5", 0], "negative": ["6", 0], "latent_image": ["7", 0]}},
  "8": {"class_type": "VAEDecode", "inputs": {"samples": ["3", 0], "vae": ["4", 2]}},
  "9": {"class_type": "SaveImage", "inputs": {"filename_prefix": "thinclaw", "images": ["8", 0]}}
}"#;

const SDXL_IMG2IMG: &str = r#"{
  "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "sd_xl_base_1.0.safetensors"}},
  "10": {"class_type": "LoadImage", "inputs": {"image": "input.png"}},
  "11": {"class_type": "VAEEncode", "inputs": {"pixels": ["10", 0], "vae": ["4", 2]}},
  "5": {"class_type": "CLIPTextEncode", "inputs": {"text": "positive prompt", "clip": ["4", 1]}},
  "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "negative prompt, low quality, blurry", "clip": ["4", 1]}},
  "3": {"class_type": "KSampler", "inputs": {"seed": 1, "steps": 25, "cfg": 7.0, "sampler_name": "euler", "scheduler": "normal", "denoise": 0.75, "model": ["4", 0], "positive": ["5", 0], "negative": ["6", 0], "latent_image": ["11", 0]}},
  "8": {"class_type": "VAEDecode", "inputs": {"samples": ["3", 0], "vae": ["4", 2]}},
  "9": {"class_type": "SaveImage", "inputs": {"filename_prefix": "thinclaw_img2img", "images": ["8", 0]}}
}"#;
const UPSCALE_4X: &str = r#"{
  "1": {"class_type": "LoadImage", "inputs": {"image": "input.png"}},
  "2": {"class_type": "UpscaleModelLoader", "inputs": {"model_name": "RealESRGAN_x4plus.pth"}},
  "3": {"class_type": "ImageUpscaleWithModel", "inputs": {"upscale_model": ["2", 0], "image": ["1", 0]}},
  "4": {"class_type": "SaveImage", "inputs": {"filename_prefix": "thinclaw_upscale", "images": ["3", 0]}}
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_workflow_is_api_format() {
        let workflow = bundled_workflow("sdxl_txt2img").unwrap();
        validate_api_workflow(&workflow).unwrap();
    }

    #[test]
    fn rejects_editor_format() {
        let workflow = json!({"nodes": [], "links": []});
        let err = validate_api_workflow(&workflow).unwrap_err();
        assert!(err.to_string().contains("editor-format"));
    }

    #[test]
    fn injects_basic_generation_params() {
        let mut workflow = bundled_workflow("sdxl_txt2img").unwrap();
        inject_generation_params(
            &mut workflow,
            GenerationParams {
                prompt: "a castle",
                negative_prompt: Some("blurry"),
                seed: 42,
                width: 832,
                height: 1216,
                steps: Some(12),
                cfg: Some(5.5),
                model: Some("custom.safetensors"),
            },
        )
        .unwrap();
        assert_eq!(workflow["5"]["inputs"]["text"], "a castle");
        assert_eq!(workflow["6"]["inputs"]["text"], "blurry");
        assert_eq!(workflow["7"]["inputs"]["width"], 832);
        assert_eq!(workflow["7"]["inputs"]["height"], 1216);
        assert_eq!(workflow["3"]["inputs"]["seed"], 42);
        assert_eq!(workflow["4"]["inputs"]["ckpt_name"], "custom.safetensors");
    }

    #[test]
    fn extracts_model_dependencies() {
        let workflow = bundled_workflow("sdxl_txt2img").unwrap();
        let refs = collect_model_references(&workflow).unwrap();
        assert!(refs.iter().any(|r| r.folder == "checkpoints"));
    }

    #[test]
    fn cloud_mode_requires_https_without_credentials() {
        let config = ComfyUiConfig {
            mode: ComfyUiMode::Cloud,
            host: "http://user:password@example.com".to_string(),
            ..ComfyUiConfig::default()
        };
        assert!(ComfyUiClient::new(config).is_err());
    }

    #[test]
    fn workflow_depth_is_bounded() {
        let mut nested = Value::Null;
        for _ in 0..=MAX_COMFY_JSON_DEPTH {
            nested = json!([nested]);
        }
        let workflow = json!({
            "1": {"class_type": "Node", "inputs": {"nested": nested}}
        });
        assert!(validate_api_workflow(&workflow).is_err());
    }

    #[test]
    fn output_filename_cannot_contain_a_path() {
        assert!(safe_output_filename("../escape.png").is_err());
        assert!(safe_output_filename("nested/escape.png").is_err());
        assert!(safe_output_filename("image.png").is_ok());
    }
}
