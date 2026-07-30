//! HuggingFace Hub dynamic model discovery and download.
//!
//! Provides capability-driven Direct Workbench commands for searching the Hub,
//! resolving immutable artifact plans, and downloading one validated selection.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};

const MAX_HF_API_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_HF_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_HF_TREE_ENTRIES: usize = 10_000;
const MAX_HF_TREE_PAGES: usize = 32;
const HF_TREE_PAGE_LIMIT: u16 = 1_000;
/// Total search pages fetched across every pipeline-tag route. Search pages
/// contain at most 100 cards, so this also bounds post-filtering work.
const MAX_HF_SEARCH_PAGES: usize = 8;
const MAX_HF_DOWNLOAD_FILES: usize = 4_096;
const MAX_HF_FILE_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const MAX_HF_DOWNLOAD_BYTES: u64 = 250 * 1024 * 1024 * 1024;
const HF_STAGING_PREFIX: &str = ".thinclaw-hf-";
const HF_STAGING_SUFFIX: &str = ".staging";
const HF_STAGING_MARKER_FILENAME: &str = ".thinclaw-hf-staging-owner";
const HF_STAGING_MARKER_CONTENT: &[u8] = b"thinclaw-hf-staging-v1\n";
const HF_PRODUCTION_BASE_URL: &str = "https://huggingface.co/";
const HF_HTTP_UNAUTHORIZED_MESSAGE: &str =
    "Hugging Face authentication failed (HTTP 401). Add or update your Hugging Face token in Settings > Secrets, then retry.";
const HF_HTTP_FORBIDDEN_MESSAGE: &str =
    "Hugging Face access denied (HTTP 403). Add or update a token with read access in Settings > Secrets and accept the model's gated license on Hugging Face, then retry.";
const HF_HTTP_RATE_LIMIT_MESSAGE: &str =
    "Hugging Face rate limit reached (HTTP 429). Wait before retrying, or add a Hugging Face token in Settings > Secrets to increase the limit.";
const HF_STAGING_STALE_AFTER: std::time::Duration =
    std::time::Duration::from_secs(7 * 24 * 60 * 60);
const HF_LEGACY_STAGING_STALE_AFTER: std::time::Duration =
    std::time::Duration::from_secs(30 * 24 * 60 * 60);
const HF_DOWNLOAD_CANCELLED: &str = "HuggingFace download cancelled";

mod transport;
pub(crate) use transport::{allowed_hf_redirect, cleanup_stale_hf_staging_dirs};
use transport::*;


// ---------------------------------------------------------------------------
// Types exposed to frontend via specta
// ---------------------------------------------------------------------------

/// A model card returned from HF Hub search.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfModelCard {
    pub id: String,     // "unsloth/Llama-3-8B-GGUF"
    pub author: String, // "unsloth"
    pub name: String,   // "Llama-3-8B-GGUF"
    pub downloads: f64,
    pub likes: u32,
    pub tags: Vec<String>,
    pub last_modified: String,
    pub gated: bool, // requires HF token for download
    /// Immutable repository commit returned by the Hub search API, when present.
    pub revision: Option<String>,
}

/// Search-only metadata used to decide whether a Hub card can be served by the
/// active runtime. Repository configs must never become part of the renderer
/// DTO: they are untrusted, potentially large Hub data and only the backend
/// compatibility gate needs them.
#[derive(Debug, Clone)]
struct HfModelSearchCandidate {
    card: HfModelCard,
    config: Option<serde_json::Value>,
}

/// Model tasks that ThinClaw has an actual local consumer for.
///
/// This intentionally does not contain a video task: ThinClaw currently has no
/// local video runtime, so presenting video repositories as installable would
/// be misleading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HfModelTask {
    Chat,
    Vision,
    Embedding,
    Stt,
    Diffusion,
    Tts,
}

impl HfModelTask {
    fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Vision => "vision",
            Self::Embedding => "embedding",
            Self::Stt => "stt",
            Self::Diffusion => "diffusion",
            Self::Tts => "tts",
        }
    }
}

fn task_requires_mmproj(engine_id: &str, task: HfModelTask) -> bool {
    engine_id == "llamacpp" && task == HfModelTask::Vision
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HfArtifactLayout {
    GgufVariants,
    Directory,
}

/// Backend-owned description of one supported engine/task combination.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfCapabilityProfileDto {
    pub engine_id: String,
    pub task: HfModelTask,
    pub category: String,
    pub pipeline_tags: Vec<String>,
    pub format_tag: String,
    pub layout: HfArtifactLayout,
    pub searchable: bool,
    pub compatibility_hint: Option<String>,
}

/// One immutable file in a selectable Hugging Face artifact.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfArtifactFile {
    pub path: String,
    #[specta(type = f64)]
    pub size: u64,
    pub size_display: String,
    pub sha256: Option<String>,
}

/// A complete loadable artifact. For sharded GGUF models, `files` contains
/// every required shard and `primary_file` is the first shard passed to
/// llama.cpp. Directory engines expose one artifact with no primary file.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfDownloadArtifact {
    pub id: String,
    pub download_id: String,
    pub label: String,
    pub layout: HfArtifactLayout,
    pub files: Vec<HfArtifactFile>,
    pub primary_file: Option<String>,
    pub quant_type: Option<String>,
    pub is_mmproj: bool,
    #[specta(type = f64)]
    pub total_size: u64,
    pub total_size_display: String,
}

/// Revision-pinned download choices for a repository.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfModelFilePlan {
    pub repo_id: String,
    pub revision: String,
    pub engine_id: String,
    pub task: HfModelTask,
    pub category: String,
    pub format_tag: String,
    pub layout: HfArtifactLayout,
    pub artifacts: Vec<HfDownloadArtifact>,
    pub companion_artifacts: Vec<HfDownloadArtifact>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct HfModelSearchResponse {
    pub engine_id: String,
    pub task: HfModelTask,
    pub models: Vec<HfModelCard>,
    /// More same-filter Hub pages were left unvisited, or this response omitted
    /// compatible cards beyond the requested limit.
    pub has_more: bool,
}

/// A request names a backend-produced artifact rather than supplying arbitrary
/// file paths. The backend rebuilds the pinned plan before downloading.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct HfDownloadSelectionRequest {
    pub repo_id: String,
    pub revision: String,
    pub task: HfModelTask,
    pub artifact_id: String,
    pub companion_artifact_id: Option<String>,
    pub destination_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct HfDownloadResult {
    pub download_id: String,
    pub repo_id: String,
    pub revision: String,
    pub engine_id: String,
    pub task: HfModelTask,
    pub category: String,
    pub artifact_id: String,
    pub companion_artifact_id: Option<String>,
    pub destination_dir: String,
    pub model_path: String,
    pub companion_path: Option<String>,
    pub downloaded_files: Vec<String>,
    #[specta(type = f64)]
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
struct HfCapabilityProfile {
    engine_id: &'static str,
    task: HfModelTask,
    category: &'static str,
    pipeline_tags: &'static [&'static str],
    format_tag: &'static str,
    layout: HfArtifactLayout,
    compatibility_hint: Option<&'static str>,
}

const HF_CAPABILITY_PROFILES: &[HfCapabilityProfile] = &[
    HfCapabilityProfile {
        engine_id: "llamacpp",
        task: HfModelTask::Chat,
        category: "LLM",
        pipeline_tags: &["text-generation"],
        format_tag: "gguf",
        layout: HfArtifactLayout::GgufVariants,
        compatibility_hint: None,
    },
    HfCapabilityProfile {
        engine_id: "llamacpp",
        task: HfModelTask::Vision,
        category: "LLM",
        pipeline_tags: &["image-text-to-text"],
        format_tag: "gguf",
        layout: HfArtifactLayout::GgufVariants,
        compatibility_hint: Some("Select a matching mmproj artifact for vision input."),
    },
    HfCapabilityProfile {
        engine_id: "llamacpp",
        task: HfModelTask::Embedding,
        category: "Embedding",
        pipeline_tags: &["feature-extraction", "sentence-similarity"],
        format_tag: "gguf",
        layout: HfArtifactLayout::GgufVariants,
        compatibility_hint: None,
    },
    HfCapabilityProfile {
        engine_id: "mlx",
        task: HfModelTask::Chat,
        category: "LLM",
        pipeline_tags: &["text-generation"],
        format_tag: "mlx",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: None,
    },
    HfCapabilityProfile {
        engine_id: "mlx",
        task: HfModelTask::Vision,
        category: "LLM",
        pipeline_tags: &["image-text-to-text"],
        format_tag: "mlx",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: None,
    },
    HfCapabilityProfile {
        engine_id: "mlx",
        task: HfModelTask::Embedding,
        category: "Embedding",
        pipeline_tags: &["feature-extraction", "sentence-similarity"],
        format_tag: "mlx",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: Some(
            "Pinned mlx-embeddings 0.0.5 text-vector families only: BERT, XLM-RoBERTa, Qwen3, Gemma3 Text, and safe ModernBERT configs.",
        ),
    },
    HfCapabilityProfile {
        engine_id: "mlx",
        task: HfModelTask::Stt,
        category: "STT",
        pipeline_tags: &["automatic-speech-recognition"],
        format_tag: "mlx",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: Some("ThinClaw's MLX speech runtime accepts Whisper-family models."),
    },
    HfCapabilityProfile {
        engine_id: "mlx",
        task: HfModelTask::Diffusion,
        category: "Diffusion",
        pipeline_tags: &["text-to-image", "image-to-image"],
        format_tag: "mflux",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: Some(
            "ThinClaw's MFlux image runtime accepts component-layout FLUX.1 schnell/dev models.",
        ),
    },
    HfCapabilityProfile {
        engine_id: "vllm",
        task: HfModelTask::Chat,
        category: "LLM",
        pipeline_tags: &["text-generation"],
        format_tag: "awq",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: None,
    },
    HfCapabilityProfile {
        engine_id: "vllm",
        task: HfModelTask::Vision,
        category: "LLM",
        pipeline_tags: &["image-text-to-text"],
        format_tag: "awq",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: None,
    },
];

impl HfCapabilityProfile {
    fn dto(self) -> HfCapabilityProfileDto {
        HfCapabilityProfileDto {
            engine_id: self.engine_id.to_string(),
            task: self.task,
            category: self.category.to_string(),
            pipeline_tags: self
                .pipeline_tags
                .iter()
                .map(|tag| (*tag).to_string())
                .collect(),
            format_tag: self.format_tag.to_string(),
            layout: self.layout,
            searchable: true,
            compatibility_hint: self.compatibility_hint.map(str::to_string),
        }
    }
}

fn capability_profile(engine_id: &str, task: HfModelTask) -> Option<HfCapabilityProfile> {
    HF_CAPABILITY_PROFILES
        .iter()
        .copied()
        .find(|profile| profile.engine_id == engine_id && profile.task == task)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct HfRepoMetadata {
    sha: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    pipeline_tag: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HfTreeEntryWire {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    oid: Option<String>,
    #[serde(default)]
    lfs: Option<HfLfsInfoWire>,
}

#[derive(Debug, Deserialize)]
struct HfLfsInfoWire {
    #[serde(default)]
    size: Option<u64>,
    #[serde(default, alias = "oid")]
    sha256: Option<String>,
}

#[derive(Debug, Clone)]
struct HfTreeFile {
    path: String,
    size: u64,
    #[allow(dead_code)]
    oid: Option<String>,
    sha256: Option<String>,
}

#[derive(Debug, Clone)]
struct PlannedDownloadFile {
    path: String,
    expected_size: Option<u64>,
    sha256: Option<String>,
}

#[derive(Debug)]
struct InternalDownloadResult {
    destination_dir: std::path::PathBuf,
    downloaded_files: Vec<String>,
    total_bytes: u64,
}

/// Build an HTTP client with optional HF token injection.
/// Reads the token from the app-wide SecretStore (populated once at startup
/// from the macOS Keychain).
async fn build_hf_client<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<reqwest::Client, String> {
    let token = app
        .try_state::<crate::secret_store::SecretStore>()
        .and_then(|store| store.huggingface_token());
    build_hf_client_with_token(token.as_deref())
}

fn build_hf_client_with_token(token: Option<&str>) -> Result<reqwest::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();

    if let Some(token) = token {
        if !token.trim().is_empty()
            && token.len() <= 16 * 1024
            && !token.chars().any(char::is_control)
        {
            let val = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| format!("Invalid HF token header: {e}"))?;
            headers.insert(reqwest::header::AUTHORIZATION, val);
        }
    }

    reqwest::Client::builder()
        .default_headers(headers)
        .user_agent(concat!("ThinClawDesktop/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

/// Format bytes as human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

fn model_matches_profile(candidate: &HfModelSearchCandidate, profile: HfCapabilityProfile) -> bool {
    if !candidate
        .card
        .tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case(profile.format_tag))
    {
        return false;
    }
    if !profile.pipeline_tags.iter().any(|pipeline| {
        candidate
            .card
            .tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case(pipeline))
    }) {
        return false;
    }
    let id = candidate.card.id.to_ascii_lowercase();
    match (profile.engine_id, profile.task) {
        ("mlx", HfModelTask::Vision) => candidate
            .config
            .as_ref()
            .is_some_and(crate::model_manager::is_supported_mlx_vision_config),
        ("mlx", HfModelTask::Embedding) => candidate
            .config
            .as_ref()
            .is_some_and(crate::model_manager::is_supported_mlx_embedding_config),
        ("mlx", HfModelTask::Stt) => id.contains("whisper"),
        ("mlx", HfModelTask::Diffusion) => is_supported_mflux_flux1_repo(&id),
        _ => true,
    }
}

fn is_supported_mflux_flux1_repo(repo_id: &str) -> bool {
    // Owners are arbitrary and often describe conversion work. Only the
    // repository-name segment identifies the model family and variant.
    let id = repo_id
        .rsplit('/')
        .next()
        .unwrap_or(repo_id)
        .to_ascii_lowercase();
    let is_flux_one = ["flux.1", "flux-1", "flux_1", "flux1"]
        .iter()
        .any(|marker| id.contains(marker));
    let is_schnell = id.contains("schnell");
    let is_dev = id.contains("dev");
    let has_unsupported_variant = [
        "flux.2",
        "flux-2",
        "flux_2",
        "flux2",
        "klein",
        "krea",
        "kontext",
        "fill",
        "redux",
        "depth",
        "controlnet",
        "canny",
    ]
    .iter()
    .any(|marker| id.contains(marker));
    is_flux_one && is_schnell != is_dev && !has_unsupported_variant
}

fn profile_requires_family_narrowing(profile: HfCapabilityProfile) -> bool {
    matches!(
        (profile.engine_id, profile.task),
        (
            "mlx",
            HfModelTask::Vision
                | HfModelTask::Embedding
                | HfModelTask::Stt
                | HfModelTask::Diffusion
        )
    )
}

fn profile_requires_search_config(profile: HfCapabilityProfile) -> bool {
    matches!(
        (profile.engine_id, profile.task, profile.format_tag),
        ("mlx", HfModelTask::Vision | HfModelTask::Embedding, "mlx")
    )
}

fn hf_search_candidate_limit(profile: HfCapabilityProfile, requested_limit: u32) -> u32 {
    if profile_requires_family_narrowing(profile) {
        100
    } else {
        requested_limit
    }
}

fn normalized_hf_search_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(20).clamp(1, 100)
}

#[derive(Debug)]
struct HfFilteredSearch {
    models: Vec<HfModelCard>,
    has_more: bool,
}

fn compatible_profile_model_count(
    models: &[HfModelSearchCandidate],
    profile: HfCapabilityProfile,
    stop_at: usize,
) -> usize {
    let mut seen = std::collections::HashSet::new();
    models
        .iter()
        .filter(|model| model_matches_profile(model, profile))
        .filter(|model| seen.insert(model.card.id.as_str()))
        .take(stop_at)
        .count()
}

fn finalize_profile_search(
    mut models: Vec<HfModelSearchCandidate>,
    profile: HfCapabilityProfile,
    requested_limit: u32,
    unvisited_pages: bool,
) -> HfFilteredSearch {
    let mut seen = std::collections::HashSet::new();
    models.retain(|model| model_matches_profile(model, profile));
    models.retain(|model| seen.insert(model.card.id.clone()));
    models.sort_by(|left, right| {
        right
            .card
            .downloads
            .total_cmp(&left.card.downloads)
            .then_with(|| left.card.id.cmp(&right.card.id))
    });
    let has_more = unvisited_pages || models.len() > requested_limit as usize;
    models.truncate(requested_limit as usize);
    HfFilteredSearch {
        models: models.into_iter().map(|candidate| candidate.card).collect(),
        has_more,
    }
}

fn should_fetch_another_search_round(
    family_narrowed: bool,
    compatible_count: usize,
    requested_limit: usize,
    unvisited_pages: bool,
    fetched_this_round: bool,
    pages_fetched: usize,
    page_budget: usize,
) -> bool {
    family_narrowed
        && compatible_count < requested_limit
        && unvisited_pages
        && fetched_this_round
        && pages_fetched < page_budget
}

fn metadata_matches_profile(
    metadata: &HfRepoMetadata,
    repo_id: &str,
    profile: HfCapabilityProfile,
) -> bool {
    let has_format = metadata
        .tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case(profile.format_tag));
    let has_pipeline = metadata.pipeline_tag.as_deref().is_some_and(|pipeline| {
        profile
            .pipeline_tags
            .iter()
            .any(|allowed| pipeline.eq_ignore_ascii_case(allowed))
    }) || profile.pipeline_tags.iter().any(|pipeline| {
        metadata
            .tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case(pipeline))
    });
    if !has_format || !has_pipeline {
        return false;
    }
    let id = repo_id.to_ascii_lowercase();
    match (profile.engine_id, profile.task) {
        ("mlx", HfModelTask::Stt) => id.contains("whisper"),
        ("mlx", HfModelTask::Diffusion) => is_supported_mflux_flux1_repo(&id),
        _ => true,
    }
}

async fn fetch_repo_metadata(
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    repo_id: &str,
    revision: Option<&str>,
) -> Result<HfRepoMetadata, String> {
    if let Some(revision) = revision {
        validate_hf_revision(revision, false)?;
    }
    let mut route = Vec::new();
    if let Some(revision) = revision {
        route.push("revision");
        route.push(revision);
    }
    let mut url = hf_model_api_url_at(base_url, repo_id, &route)?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("expand[]", "sha")
            .append_pair("expand[]", "tags")
            .append_pair("expand[]", "pipeline_tag");
    }
    let response = client.get(url).send().await.map_err(|error| {
        crate::rig_lib::http::transport_error("HuggingFace metadata request failed", error)
    })?;
    validate_hf_response_status(&response)?;
    let response =
        crate::rig_lib::http::checked_response(response, "HuggingFace model metadata").await?;
    let metadata: HfRepoMetadata =
        thinclaw_core::http_response::bounded_json(response, MAX_HF_API_RESPONSE_BYTES)
            .await
            .map_err(|error| format!("Invalid bounded HuggingFace metadata response: {error}"))?;
    validate_hf_revision(&metadata.sha, false)?;
    if revision.is_some_and(|requested| !metadata.sha.eq_ignore_ascii_case(requested)) {
        return Err("HuggingFace returned a different commit than the pinned revision".to_string());
    }
    Ok(metadata)
}

fn profile_requires_config_preflight(profile: HfCapabilityProfile) -> bool {
    matches!(
        (profile.engine_id, profile.task, profile.format_tag),
        ("mlx", HfModelTask::Vision | HfModelTask::Embedding, "mlx")
            | ("mlx", HfModelTask::Diffusion, "mflux")
            | ("vllm", _, "awq")
    )
}

fn is_vllm_awq_config(config: &serde_json::Value) -> bool {
    config
        .get("quantization_config")
        .and_then(|value| value.get("quant_method"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|method| method.eq_ignore_ascii_case("awq"))
}

fn validate_profile_config(
    profile: HfCapabilityProfile,
    config: &serde_json::Value,
) -> Result<(), String> {
    match (profile.engine_id, profile.task, profile.format_tag) {
        ("mlx", HfModelTask::Vision, "mlx")
            if !crate::model_manager::is_supported_mlx_vision_config(config) =>
        {
            Err(
                "MLX vision config does not declare a supported multimodal architecture"
                    .to_string(),
            )
        }
        ("mlx", HfModelTask::Embedding, "mlx")
            if !crate::model_manager::is_supported_mlx_embedding_config(config) =>
        {
            Err(
                "MLX embedding config is not supported by ThinClaw's pinned mlx-embeddings 0.0.5 text-vector loader"
                    .to_string(),
            )
        }
        ("mlx", HfModelTask::Diffusion, "mflux")
            if !crate::model_manager::is_supported_mflux_config(config) =>
        {
            Err(
                "MFlux config does not declare a supported plain FLUX.1 schnell/dev model"
                    .to_string(),
            )
        }
        ("vllm", _, "awq") if !is_vllm_awq_config(config) => Err(
            "vLLM AWQ config must declare quantization_config.quant_method as 'awq'".to_string(),
        ),
        _ => Ok(()),
    }
}

/// Validate runtime-specific config semantics before a directory artifact is
/// offered for download. The raw file is fetched from the metadata commit, so
/// the preflight and eventual artifact plan refer to the same immutable tree.
async fn preflight_pinned_model_config(
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    repo_id: &str,
    revision: &str,
    profile: HfCapabilityProfile,
) -> Result<(), String> {
    if !profile_requires_config_preflight(profile) {
        return Ok(());
    }
    validate_hf_revision(revision, false)?;
    let url = hf_url_at(base_url, repo_id, &["raw", revision], Some("config.json"))?;
    let response = client.get(url).send().await.map_err(|error| {
        crate::rig_lib::http::transport_error(
            "HuggingFace model config preflight request failed",
            error,
        )
    })?;
    validate_hf_response_status(&response)?;
    let response =
        crate::rig_lib::http::checked_response(response, "HuggingFace model config preflight")
            .await?;
    let config: serde_json::Value =
        thinclaw_core::http_response::bounded_json(response, MAX_HF_CONFIG_BYTES)
            .await
            .map_err(|error| format!("Invalid bounded HuggingFace model config: {error}"))?;
    validate_profile_config(profile, &config)
}

fn parse_next_link(
    headers: &reqwest::header::HeaderMap,
    current_url: &reqwest::Url,
    expected_path: &str,
) -> Result<Option<reqwest::Url>, String> {
    for value in headers.get_all(reqwest::header::LINK) {
        let value = value
            .to_str()
            .map_err(|_| "HuggingFace pagination Link header is not valid text".to_string())?;
        for part in value.split(',') {
            let part = part.trim();
            let Some(target_start) = part.strip_prefix('<') else {
                continue;
            };
            let Some(end) = target_start.find('>') else {
                return Err("HuggingFace pagination Link header is malformed".to_string());
            };
            let target = &target_start[..end];
            let parameters = &target_start[end + 1..];
            let is_next = parameters.split(';').any(|parameter| {
                let parameter = parameter.trim();
                let Some((name, value)) = parameter.split_once('=') else {
                    return false;
                };
                name.trim().eq_ignore_ascii_case("rel")
                    && value
                        .trim()
                        .trim_matches('"')
                        .split_ascii_whitespace()
                        .any(|rel| rel.eq_ignore_ascii_case("next"))
            });
            if !is_next {
                continue;
            }
            let next = current_url
                .join(target)
                .map_err(|_| "HuggingFace pagination URL is invalid".to_string())?;
            let same_origin = next.scheme() == current_url.scheme()
                && next
                    .host_str()
                    .zip(current_url.host_str())
                    .is_some_and(|(next, current)| next.eq_ignore_ascii_case(current))
                && next.port_or_known_default() == current_url.port_or_known_default();
            if !same_origin
                || !next.username().is_empty()
                || next.password().is_some()
                || next.path() != expected_path
            {
                return Err("HuggingFace pagination URL escaped the expected API route".to_string());
            }
            return Ok(Some(next));
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HfSearchRouteIdentity {
    path: String,
    query: Vec<(String, String)>,
}

struct ParsedHfSearchRoute {
    identity: HfSearchRouteIdentity,
    has_cursor: bool,
}

fn parse_hf_search_route(url: &reqwest::Url) -> Result<ParsedHfSearchRoute, String> {
    let mut query = Vec::new();
    let mut has_cursor = false;
    for (key, value) in url.query_pairs() {
        if key == "cursor" {
            if has_cursor
                || value.is_empty()
                || value.len() > 8_192
                || value.chars().any(char::is_control)
            {
                return Err("HuggingFace search pagination cursor is invalid".to_string());
            }
            has_cursor = true;
            continue;
        }
        // The Hub currently normalizes `expand[]` to `expand` in Link
        // headers. Treat those spellings as the same immutable projection.
        let key = if key == "expand[]" {
            "expand".to_string()
        } else {
            key.into_owned()
        };
        query.push((key, value.into_owned()));
    }
    query.sort();
    Ok(ParsedHfSearchRoute {
        identity: HfSearchRouteIdentity {
            path: url.path().to_string(),
            query,
        },
        has_cursor,
    })
}

fn parse_search_next_link(
    headers: &reqwest::header::HeaderMap,
    current_url: &reqwest::Url,
    expected_route: &HfSearchRouteIdentity,
) -> Result<Option<reqwest::Url>, String> {
    let Some(next) = parse_next_link(headers, current_url, &expected_route.path)? else {
        return Ok(None);
    };
    let parsed = parse_hf_search_route(&next)?;
    if !parsed.has_cursor || parsed.identity != *expected_route {
        return Err("HuggingFace search pagination changed the expected search route".to_string());
    }
    Ok(Some(next))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HfTreeRouteIdentity {
    scheme: String,
    host: String,
    port: Option<u16>,
    path: String,
    query: Vec<(String, String)>,
}

struct ParsedHfTreeRoute {
    identity: HfTreeRouteIdentity,
    cursor: Option<String>,
}

fn parse_hf_tree_route(url: &reqwest::Url) -> Result<ParsedHfTreeRoute, String> {
    let mut query = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cursor = None;
    for (key, value) in url.query_pairs() {
        if !seen.insert(key.to_string()) {
            return Err("HuggingFace tree pagination contains duplicate parameters".to_string());
        }
        match key.as_ref() {
            "cursor" => {
                if value.is_empty() || value.len() > 8_192 || value.chars().any(char::is_control) {
                    return Err("HuggingFace tree pagination cursor is invalid".to_string());
                }
                cursor = Some(value.into_owned());
            }
            "recursive" | "expand" | "limit" => {
                if value.is_empty() || value.len() > 32 || value.chars().any(char::is_control) {
                    return Err("HuggingFace tree pagination parameter is invalid".to_string());
                }
                query.push((key.into_owned(), value.into_owned()));
            }
            _ => {
                return Err(
                    "HuggingFace tree pagination changed the expected tree route".to_string(),
                );
            }
        }
    }
    query.sort();
    let host = url
        .host_str()
        .ok_or_else(|| "HuggingFace tree pagination URL has no host".to_string())?
        .to_ascii_lowercase();
    Ok(ParsedHfTreeRoute {
        identity: HfTreeRouteIdentity {
            scheme: url.scheme().to_string(),
            host,
            port: url.port_or_known_default(),
            path: url.path().to_string(),
            query,
        },
        cursor,
    })
}

fn parse_tree_next_link(
    headers: &reqwest::header::HeaderMap,
    current_url: &reqwest::Url,
    expected_route: &HfTreeRouteIdentity,
) -> Result<Option<reqwest::Url>, String> {
    let Some(next) = parse_next_link(headers, current_url, &expected_route.path)? else {
        return Ok(None);
    };
    let parsed = parse_hf_tree_route(&next)?;
    if parsed.cursor.is_none() || parsed.identity != *expected_route {
        return Err("HuggingFace tree pagination changed the expected tree route".to_string());
    }
    Ok(Some(next))
}

async fn fetch_repo_tree(
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    repo_id: &str,
    revision: &str,
) -> Result<Vec<HfTreeFile>, String> {
    validate_hf_revision(revision, false)?;
    let mut next_url = hf_model_api_url_at(base_url, repo_id, &["tree", revision])?;
    {
        let mut query = next_url.query_pairs_mut();
        query
            .append_pair("recursive", "true")
            .append_pair("expand", "false")
            .append_pair("limit", &HF_TREE_PAGE_LIMIT.to_string());
    }
    let parsed_route = parse_hf_tree_route(&next_url)?;
    if parsed_route.cursor.is_some() {
        return Err("Initial HuggingFace tree URL unexpectedly contains a cursor".to_string());
    }
    let expected_route = parsed_route.identity;
    let mut seen_urls = std::collections::HashSet::new();
    let mut seen_cursors = std::collections::HashSet::new();
    let mut seen_paths = std::collections::HashSet::new();
    let mut files = Vec::new();
    let mut total_entries = 0_usize;

    for _ in 0..MAX_HF_TREE_PAGES {
        if !seen_urls.insert(next_url.as_str().to_string()) {
            return Err("HuggingFace tree pagination contained a cycle".to_string());
        }
        let cursor = parse_hf_tree_route(&next_url)?.cursor;
        if !seen_cursors.insert(cursor) {
            return Err("HuggingFace tree pagination contained a cursor cycle".to_string());
        }
        let response = client.get(next_url.clone()).send().await.map_err(|error| {
            crate::rig_lib::http::transport_error("HuggingFace tree request failed", error)
        })?;
        validate_hf_response_status(&response)?;
        let response =
            crate::rig_lib::http::checked_response(response, "HuggingFace repository tree").await?;
        let following = parse_tree_next_link(response.headers(), &next_url, &expected_route)?;
        let page: Vec<HfTreeEntryWire> =
            thinclaw_core::http_response::bounded_json(response, MAX_HF_API_RESPONSE_BYTES)
                .await
                .map_err(|error| format!("Invalid bounded HuggingFace tree response: {error}"))?;

        total_entries = total_entries.saturating_add(page.len());
        if total_entries > MAX_HF_TREE_ENTRIES {
            return Err(format!(
                "HuggingFace tree exceeds the {MAX_HF_TREE_ENTRIES}-entry limit"
            ));
        }
        for entry in page {
            validate_hf_file_path(&entry.path)?;
            if entry.kind != "file" {
                continue;
            }
            if !seen_paths.insert(entry.path.clone()) {
                return Err("HuggingFace tree contains duplicate file paths".to_string());
            }
            let size = entry
                .size
                .or_else(|| entry.lfs.as_ref().and_then(|lfs| lfs.size))
                .unwrap_or(0);
            let sha256 = entry
                .lfs
                .and_then(|lfs| lfs.sha256)
                .filter(|hash| {
                    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                .map(|hash| hash.to_ascii_lowercase());
            files.push(HfTreeFile {
                path: entry.path,
                size,
                oid: entry.oid,
                sha256,
            });
        }

        match following {
            Some(url) => next_url = url,
            None => return Ok(files),
        }
    }

    Err(format!(
        "HuggingFace tree exceeds the {MAX_HF_TREE_PAGES}-page limit"
    ))
}

fn artifact_id(kind: &str, repo_id: &str, revision: &str, group_key: &str) -> String {
    let identity = format!("hf://{repo_id}@{revision}/{kind}/{group_key}");
    format!(
        "{kind}-{}",
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, identity.as_bytes()).simple()
    )
}

fn artifact_download_id(repo_id: &str, revision: &str, artifact_id: &str) -> String {
    let identity = format!("hf-download://{repo_id}@{revision}/{artifact_id}");
    format!(
        "hf-{}",
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, identity.as_bytes()).simple()
    )
}

fn extract_quant_type(name: &str) -> Option<String> {
    let quant = regex::Regex::new(
        r"(?i)((?:UD-)?(?:IQ[0-9](?:_[A-Z0-9]+)+|Q[0-9](?:_[A-Z0-9]+)+|BF16|F16|F32))",
    )
    .expect("static GGUF quantization regex");
    quant
        .captures_iter(name)
        .last()
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_ascii_uppercase())
}

#[derive(Debug)]
struct PendingGgufGroup {
    group_key: String,
    is_mmproj: bool,
    expected_shards: usize,
    indexed_files: std::collections::BTreeMap<usize, HfTreeFile>,
}

type HfGgufArtifactPlan = (
    Vec<HfDownloadArtifact>,
    Vec<HfDownloadArtifact>,
    Vec<String>,
);

fn gguf_shard_identity(path: &str) -> Option<(String, usize, usize)> {
    let file_name = std::path::Path::new(path).file_name()?.to_str()?;
    let shard =
        regex::Regex::new(r"(?i)^(?P<stem>.+)-(?P<index>[0-9]{5})-of-(?P<total>[0-9]{5})\.gguf$")
            .expect("static GGUF shard regex");
    let captures = shard.captures(file_name)?;
    let index = captures.name("index")?.as_str().parse::<usize>().ok()?;
    let total = captures.name("total")?.as_str().parse::<usize>().ok()?;
    let stem = captures.name("stem")?.as_str();
    let parent = std::path::Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.to_string_lossy().to_string());
    let key = parent.map_or_else(|| stem.to_string(), |parent| format!("{parent}/{stem}"));
    Some((key, index, total))
}

fn build_gguf_artifacts(
    repo_id: &str,
    revision: &str,
    tree: &[HfTreeFile],
) -> Result<HfGgufArtifactPlan, String> {
    let mut groups: std::collections::BTreeMap<String, PendingGgufGroup> =
        std::collections::BTreeMap::new();
    let mut warnings = Vec::new();

    for file in tree {
        if !file.path.to_ascii_lowercase().ends_with(".gguf") {
            continue;
        }
        let is_mmproj = file.path.to_ascii_lowercase().contains("mmproj");
        let (group_key, index, expected_shards) =
            gguf_shard_identity(&file.path).unwrap_or_else(|| (file.path.clone(), 1, 1));
        let namespace = if is_mmproj { "mmproj" } else { "model" };
        let map_key = format!("{namespace}:{group_key}");
        let group = groups.entry(map_key).or_insert_with(|| PendingGgufGroup {
            group_key,
            is_mmproj,
            expected_shards,
            indexed_files: std::collections::BTreeMap::new(),
        });
        if group.expected_shards != expected_shards
            || index == 0
            || index > expected_shards
            || group.indexed_files.insert(index, file.clone()).is_some()
        {
            group.expected_shards = 0;
        }
    }

    let mut model_artifacts = Vec::new();
    let mut companion_artifacts = Vec::new();
    for (_, group) in groups {
        if group.expected_shards == 0
            || group.indexed_files.len() != group.expected_shards
            || !(1..=group.expected_shards).all(|index| group.indexed_files.contains_key(&index))
        {
            warnings.push(format!(
                "Ignored incomplete or inconsistent GGUF shard set '{}'.",
                group.group_key
            ));
            continue;
        }
        let files: Vec<HfTreeFile> = group.indexed_files.into_values().collect();
        if files.len() > MAX_HF_DOWNLOAD_FILES {
            warnings.push(format!(
                "Ignored GGUF artifact '{}' because it has too many shards.",
                group.group_key
            ));
            continue;
        }
        if files.iter().any(|file| file.size == 0) {
            warnings.push(format!(
                "Ignored GGUF artifact '{}' because one or more shard sizes are missing or zero.",
                group.group_key
            ));
            continue;
        }
        if files.iter().any(|file| file.size > MAX_HF_FILE_BYTES) {
            warnings.push(format!(
                "Ignored GGUF artifact '{}' because one shard exceeds the file-size limit.",
                group.group_key
            ));
            continue;
        }
        let Some(total_size) = files
            .iter()
            .try_fold(0_u64, |total, file| total.checked_add(file.size))
        else {
            warnings.push(format!(
                "Ignored GGUF artifact '{}' because its size overflowed.",
                group.group_key
            ));
            continue;
        };
        if total_size > MAX_HF_DOWNLOAD_BYTES {
            warnings.push(format!(
                "Ignored GGUF artifact '{}' because that selection exceeds the download limit.",
                group.group_key
            ));
            continue;
        }
        let kind = if group.is_mmproj { "mmproj" } else { "gguf" };
        let id = artifact_id(kind, repo_id, revision, &group.group_key);
        let quant_type = extract_quant_type(&group.group_key);
        let label = quant_type.clone().unwrap_or_else(|| {
            std::path::Path::new(&group.group_key)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| group.group_key.clone())
        });
        let primary_file = files.first().map(|file| file.path.clone());
        let artifact = HfDownloadArtifact {
            download_id: artifact_download_id(repo_id, revision, &id),
            id,
            label,
            layout: HfArtifactLayout::GgufVariants,
            files: files
                .into_iter()
                .map(|file| HfArtifactFile {
                    path: file.path,
                    size: file.size,
                    size_display: format_bytes(file.size),
                    sha256: file.sha256,
                })
                .collect(),
            primary_file,
            quant_type,
            is_mmproj: group.is_mmproj,
            total_size,
            total_size_display: format_bytes(total_size),
        };
        if group.is_mmproj {
            companion_artifacts.push(artifact);
        } else {
            model_artifacts.push(artifact);
        }
    }
    model_artifacts.sort_by_key(|artifact| artifact.total_size);
    companion_artifacts.sort_by_key(|artifact| artifact.total_size);
    if model_artifacts.is_empty() {
        return Err("No complete GGUF model artifacts were found in this repository".to_string());
    }
    Ok((model_artifacts, companion_artifacts, warnings))
}

fn is_directory_download_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower == crate::model_manager::MODEL_MANIFEST_FILENAME
        || matches!(lower.as_str(), ".ds_store" | ".gitkeep")
    {
        return false;
    }
    ![
        ".md",
        ".jpg",
        ".jpeg",
        ".png",
        ".gif",
        ".gitattributes",
        ".gitignore",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}

fn has_weight_extension(path: &str, extensions: &[&str]) -> bool {
    let lower = path.to_ascii_lowercase();
    extensions
        .iter()
        .any(|extension| lower.ends_with(extension))
}

fn validate_directory_layout(
    profile: HfCapabilityProfile,
    files: &[HfTreeFile],
) -> Result<(), String> {
    let has_root_config = files.iter().any(|file| file.path == "config.json");
    let has_tokenizer_assets = files.iter().any(|file| {
        let lower = file.path.to_ascii_lowercase();
        lower.starts_with("tokenizer/")
            || matches!(
                std::path::Path::new(&lower)
                    .file_name()
                    .and_then(|name| name.to_str()),
                Some(
                    "tokenizer.json"
                        | "tokenizer.model"
                        | "tokenizer_config.json"
                        | "spiece.model"
                        | "vocab.json"
                        | "vocab.txt"
                        | "merges.txt"
                )
            )
    });
    let has_top_level_weights = |extensions: &[&str]| {
        files
            .iter()
            .any(|file| !file.path.contains('/') && has_weight_extension(&file.path, extensions))
    };

    match (profile.engine_id, profile.task) {
        ("mlx", HfModelTask::Diffusion) => {
            let has_component_weights = |component: &str| {
                let prefix = format!("{component}/");
                files.iter().any(|file| {
                    file.path.starts_with(&prefix)
                        && has_weight_extension(&file.path, &[".safetensors", ".npz"])
                })
            };
            let has_component_assets = |component: &str| {
                let prefix = format!("{component}/");
                files.iter().any(|file| file.path.starts_with(&prefix))
            };
            let has_component_layout = has_root_config
                && has_component_weights("transformer")
                && has_component_weights("vae")
                && has_component_weights("text_encoder")
                && has_component_weights("text_encoder_2")
                && has_component_assets("tokenizer")
                && has_component_assets("tokenizer_2");
            if !has_component_layout {
                return Err(
                    "MFlux FLUX.1 repository requires root config.json plus transformer, VAE, both text encoders, and both tokenizer components"
                        .to_string(),
                );
            }
        }
        ("mlx", HfModelTask::Stt) => {
            let has_runtime_named_weights = files
                .iter()
                .any(|file| matches!(file.path.as_str(), "weights.npz" | "weights.safetensors"));
            if !has_root_config || !has_runtime_named_weights {
                return Err(
                    "MLX Whisper repository requires root config.json and weights.npz or weights.safetensors"
                        .to_string(),
                );
            }
        }
        ("mlx", _) => {
            if !has_root_config
                || !has_top_level_weights(&[".safetensors", ".npz"])
                || !has_tokenizer_assets
            {
                return Err(
                    "MLX model requires root config.json, tokenizer assets, and top-level MLX weights"
                        .to_string(),
                );
            }
        }
        ("vllm", _) => {
            if !has_root_config
                || !has_tokenizer_assets
                || !files.iter().any(|file| {
                    has_weight_extension(
                        &file.path,
                        &[".safetensors", ".bin", ".pt", ".ckpt", ".sft"],
                    )
                })
            {
                return Err(
                    "vLLM model requires root config.json, tokenizer assets, and supported weights"
                        .to_string(),
                );
            }
        }
        _ => {
            return Err("Unsupported directory-model capability profile".to_string());
        }
    }
    Ok(())
}

fn build_directory_artifact(
    repo_id: &str,
    revision: &str,
    profile: HfCapabilityProfile,
    tree: &[HfTreeFile],
) -> Result<HfDownloadArtifact, String> {
    let mut files: Vec<HfTreeFile> = tree
        .iter()
        .filter(|file| is_directory_download_file(&file.path))
        .cloned()
        .collect();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    if files.is_empty() {
        return Err("HuggingFace repository contains no downloadable model files".to_string());
    }
    if let Some(empty) = files.iter().find(|file| file.size == 0) {
        return Err(format!(
            "HuggingFace artifact contains an empty model file: {}",
            empty.path
        ));
    }
    if files.len() > MAX_HF_DOWNLOAD_FILES {
        return Err(format!(
            "HuggingFace artifact contains more than {MAX_HF_DOWNLOAD_FILES} files"
        ));
    }
    validate_directory_layout(profile, &files)?;
    if files.iter().any(|file| file.size > MAX_HF_FILE_BYTES) {
        return Err("HuggingFace artifact contains an oversized file".to_string());
    }
    let total_size = files
        .iter()
        .try_fold(0_u64, |total, file| total.checked_add(file.size))
        .ok_or_else(|| "HuggingFace artifact size overflow".to_string())?;
    if total_size > MAX_HF_DOWNLOAD_BYTES {
        return Err("HuggingFace artifact exceeds the total download size limit".to_string());
    }
    let id = artifact_id("directory", repo_id, revision, "complete");
    Ok(HfDownloadArtifact {
        download_id: artifact_download_id(repo_id, revision, &id),
        id,
        label: format!(
            "{} model directory",
            profile.format_tag.to_ascii_uppercase()
        ),
        layout: HfArtifactLayout::Directory,
        files: files
            .into_iter()
            .map(|file| HfArtifactFile {
                path: file.path,
                size: file.size,
                size_display: format_bytes(file.size),
                sha256: file.sha256,
            })
            .collect(),
        primary_file: None,
        quant_type: None,
        is_mmproj: false,
        total_size,
        total_size_display: format_bytes(total_size),
    })
}

async fn build_model_file_plan<R: tauri::Runtime>(
    app: &AppHandle<R>,
    repo_id: &str,
    engine_id: &str,
    task: HfModelTask,
    revision: Option<&str>,
) -> Result<HfModelFilePlan, String> {
    let client = build_hf_client(app).await?;
    build_model_file_plan_with_http(
        &client,
        &production_hf_base_url(),
        repo_id,
        engine_id,
        task,
        revision,
    )
    .await
}

async fn build_model_file_plan_with_http(
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    repo_id: &str,
    engine_id: &str,
    task: HfModelTask,
    revision: Option<&str>,
) -> Result<HfModelFilePlan, String> {
    validate_repo_id(repo_id)?;
    let profile = capability_profile(engine_id, task).ok_or_else(|| {
        format!("The {engine_id} runtime does not support HuggingFace discovery for {task:?}")
    })?;
    let metadata = fetch_repo_metadata(client, base_url, repo_id, revision).await?;
    if !metadata_matches_profile(&metadata, repo_id, profile) {
        return Err(format!(
            "Repository is not compatible with the {engine_id} {task:?} runtime"
        ));
    }
    preflight_pinned_model_config(client, base_url, repo_id, &metadata.sha, profile).await?;
    let tree = fetch_repo_tree(client, base_url, repo_id, &metadata.sha).await?;
    let (artifacts, companion_artifacts, warnings) = match profile.layout {
        HfArtifactLayout::GgufVariants => {
            let (artifacts, mut companions, warnings) =
                build_gguf_artifacts(repo_id, &metadata.sha, &tree)?;
            if task != HfModelTask::Vision {
                companions.clear();
            }
            (artifacts, companions, warnings)
        }
        HfArtifactLayout::Directory => (
            vec![build_directory_artifact(
                repo_id,
                &metadata.sha,
                profile,
                &tree,
            )?],
            Vec::new(),
            Vec::new(),
        ),
    };
    if task_requires_mmproj(engine_id, task) && companion_artifacts.is_empty() {
        return Err(
            "Repository does not contain a selectable mmproj artifact required for llama.cpp vision"
                .to_string(),
        );
    }
    Ok(HfModelFilePlan {
        repo_id: repo_id.to_string(),
        revision: metadata.sha,
        engine_id: engine_id.to_string(),
        task,
        category: profile.category.to_string(),
        format_tag: profile.format_tag.to_string(),
        layout: profile.layout,
        artifacts,
        companion_artifacts,
        warnings,
    })
}

/// Parse a raw JSON value from the HF models API into an HfModelCard.
fn parse_model_card(v: &serde_json::Value) -> Option<HfModelCard> {
    let id = v["id"].as_str()?;
    validate_repo_id(id).ok()?;
    let id = id.to_string();

    // Author is the part before the slash
    let author = id.split('/').next().unwrap_or("unknown").to_string();
    let name = id.split('/').nth(1).unwrap_or(&id).to_string();
    let valid_tag =
        |tag: &str| !tag.is_empty() && tag.len() <= 256 && !tag.chars().any(char::is_control);
    let mut tags = Vec::new();

    // `pipeline_tag` is a distinct field in expanded Hub responses and is not
    // guaranteed to be repeated in `tags`. Preserve it first so the bounded
    // tag list always carries the task signal used by compatibility filtering.
    if let Some(pipeline_tag) = v["pipeline_tag"].as_str().filter(|tag| valid_tag(tag)) {
        tags.push(pipeline_tag.to_string());
    }
    if let Some(card_tags) = v["tags"].as_array() {
        for tag in card_tags.iter().filter_map(serde_json::Value::as_str) {
            if tags.len() >= 256 {
                break;
            }
            if valid_tag(tag)
                && !tags
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(tag))
            {
                tags.push(tag.to_string());
            }
        }
    }

    Some(HfModelCard {
        id: id.clone(),
        author,
        name,
        downloads: v["downloads"].as_u64().unwrap_or(0) as f64,
        likes: v["likes"]
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(u32::MAX),
        tags,
        last_modified: v["lastModified"]
            .as_str()
            .filter(|value| value.len() <= 128 && !value.chars().any(char::is_control))
            .unwrap_or("")
            .to_string(),
        gated: v["gated"].as_bool().unwrap_or(false)
            || v["gated"].as_str().is_some_and(|s| s != "false"),
        revision: v["sha"]
            .as_str()
            .filter(|revision| validate_hf_revision(revision, false).is_ok())
            .map(str::to_string),
    })
}

fn parse_model_search_candidate(v: &serde_json::Value) -> Option<HfModelSearchCandidate> {
    let card = parse_model_card(v)?;
    let config = v.get("config").filter(|config| config.is_object()).cloned();
    Some(HfModelSearchCandidate { card, config })
}

/// Build one strict Hub search URL. Once any `expand[]` field is requested the
/// Hub stops returning several default fields, so keep the complete card
/// projection together here rather than letting individual callers drift.
#[cfg(test)]
fn hf_model_search_url(
    search: &str,
    format_tag: &str,
    limit: u32,
    pipeline_tag: Option<&str>,
    include_config: bool,
) -> Result<reqwest::Url, String> {
    hf_model_search_url_at(
        &production_hf_base_url(),
        search,
        format_tag,
        limit,
        pipeline_tag,
        include_config,
    )
}

fn hf_model_search_url_at(
    base_url: &reqwest::Url,
    search: &str,
    format_tag: &str,
    limit: u32,
    pipeline_tag: Option<&str>,
    include_config: bool,
) -> Result<reqwest::Url, String> {
    let mut url = base_url.clone();
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Could not construct HuggingFace search URL".to_string())?;
        segments.clear().push("api").push("models");
    }
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("search", search)
            .append_pair("filter", format_tag)
            .append_pair("sort", "downloads")
            .append_pair("direction", "-1")
            .append_pair("limit", &limit.to_string())
            .append_pair("expand[]", "sha")
            .append_pair("expand[]", "downloads")
            .append_pair("expand[]", "likes")
            .append_pair("expand[]", "tags")
            .append_pair("expand[]", "lastModified")
            .append_pair("expand[]", "gated")
            .append_pair("expand[]", "pipeline_tag");
        if include_config {
            query.append_pair("expand[]", "config");
        }
        if let Some(pipeline_tag) = pipeline_tag {
            query.append_pair("pipeline_tag", pipeline_tag);
        }
    }
    Ok(url)
}

#[derive(Debug)]
struct HfModelSearchPage {
    cards: Vec<HfModelSearchCandidate>,
    next_url: Option<reqwest::Url>,
}

struct HfSearchRouteState {
    label: String,
    identity: HfSearchRouteIdentity,
    next_url: Option<reqwest::Url>,
    seen_urls: std::collections::HashSet<String>,
    pages_fetched: usize,
}

impl HfSearchRouteState {
    fn new(label: String, url: reqwest::Url) -> Result<Self, String> {
        let parsed = parse_hf_search_route(&url)?;
        if parsed.has_cursor {
            return Err(
                "Initial HuggingFace search URL unexpectedly contains a cursor".to_string(),
            );
        }
        Ok(Self {
            label,
            identity: parsed.identity,
            next_url: Some(url),
            seen_urls: std::collections::HashSet::new(),
            pages_fetched: 0,
        })
    }
}

/// Fetch and parse one Hub search page, retaining the next page only when its
/// host, path, and immutable search parameters match the initial route.
async fn fetch_hf_model_page(
    client: &reqwest::Client,
    url: &reqwest::Url,
    engine_tag: &str,
    expected_route: &HfSearchRouteIdentity,
) -> Result<HfModelSearchPage, String> {
    let response = client.get(url.clone()).send().await.map_err(|error| {
        crate::rig_lib::http::transport_error("HuggingFace API request failed", error)
    })?;

    validate_hf_response_status(&response)?;
    let response =
        crate::rig_lib::http::checked_response(response, "HuggingFace model search").await?;
    let next_url = parse_search_next_link(response.headers(), url, expected_route)?;
    let body: Vec<serde_json::Value> =
        thinclaw_core::http_response::bounded_json(response, MAX_HF_API_RESPONSE_BYTES)
            .await
            .map_err(|error| format!("Invalid bounded HuggingFace response: {error}"))?;
    if body.len() > 1_000 {
        return Err("HuggingFace model search returned too many entries".to_string());
    }

    // Post-filter: verify each result actually has the engine tag in its tags list.
    let cards = body
        .iter()
        .filter_map(parse_model_search_candidate)
        .filter(|candidate| {
            candidate
                .card
                .tags
                .iter()
                .any(|tag| tag.eq_ignore_ascii_case(engine_tag))
        })
        .collect();
    Ok(HfModelSearchPage { cards, next_url })
}

// ---------------------------------------------------------------------------
// Tauri Commands
// ---------------------------------------------------------------------------

/// Return only the Hugging Face workflows that the compiled local runtime can
/// actually consume. Ollama and cloud-only builds intentionally return none:
/// downloading a raw Hub file does not import it into Ollama.
#[tauri::command]
#[specta::specta]
pub fn direct_runtime_get_hf_capabilities() -> Vec<HfCapabilityProfileDto> {
    let engine = crate::engine::direct_runtime_get_active_engine_info();
    HF_CAPABILITY_PROFILES
        .iter()
        .copied()
        .filter(|profile| profile.engine_id == engine.id)
        .map(HfCapabilityProfile::dto)
        .collect()
}

/// Backend-authoritative Hugging Face search. The caller chooses a ThinClaw
/// task, while Rust derives the active engine, allowed Hub format, pipeline
/// tags, and any runtime-family narrowing.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_discover_hf_models_v2(
    app: AppHandle,
    query: String,
    task: HfModelTask,
    limit: Option<u32>,
) -> Result<HfModelSearchResponse, crate::thinclaw::bridge::BridgeError> {
    let engine = crate::engine::direct_runtime_get_active_engine_info();
    let profile = capability_profile(&engine.id, task).ok_or_else(|| {
        crate::thinclaw::bridge::BridgeError::Runtime {
            message: format!(
                "{} does not support HuggingFace discovery for {task:?}",
                engine.display_name
            ),
        }
    })?;
    let requested_limit = normalized_hf_search_limit(limit);
    let search = direct_runtime_discover_hf_models(app, query, profile, requested_limit).await?;
    Ok(HfModelSearchResponse {
        engine_id: engine.id,
        task,
        models: search.models,
        has_more: search.has_more,
    })
}

/// Search HuggingFace Hub for models compatible with the active engine.
///
/// Uses the HF `/api/models` endpoint filtered by engine-specific tag,
/// sorted by download count (most popular first).
///
/// A capability profile may declare multiple HF pipeline tags (for example,
/// text generation and image-to-text). One route is searched per tag, then
/// results are merged, deduplicated by repo ID, and re-sorted by downloads.
/// Family-narrowed profiles continue through bounded, route-locked Link pages
/// until enough compatible cards are available or every route is exhausted.
async fn direct_runtime_discover_hf_models(
    app: AppHandle,
    query: String,
    profile: HfCapabilityProfile,
    requested_limit: u32,
) -> Result<HfFilteredSearch, crate::thinclaw::bridge::BridgeError> {
    let client = build_hf_client(&app).await?;
    Ok(discover_hf_models_with_http(
        &client,
        &production_hf_base_url(),
        &query,
        profile,
        requested_limit,
    )
    .await?)
}

async fn discover_hf_models_with_http(
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    query: &str,
    profile: HfCapabilityProfile,
    requested_limit: u32,
) -> Result<HfFilteredSearch, String> {
    if query.len() > 1_024 || query.chars().any(char::is_control) {
        return Err("HuggingFace search query is invalid or too large".to_string());
    }

    let page_limit = hf_search_candidate_limit(profile, requested_limit.max(1)).min(100);
    let include_config = profile_requires_search_config(profile);
    let mut routes = Vec::new();
    if profile.pipeline_tags.is_empty() {
        let url = hf_model_search_url_at(
            base_url,
            query,
            profile.format_tag,
            page_limit,
            None,
            include_config,
        )?;
        routes.push(HfSearchRouteState::new("unfiltered".to_string(), url)?);
    } else {
        for pipeline_tag in profile.pipeline_tags {
            let url = hf_model_search_url_at(
                base_url,
                query,
                profile.format_tag,
                page_limit,
                Some(pipeline_tag),
                include_config,
            )?;
            routes.push(HfSearchRouteState::new((*pipeline_tag).to_string(), url)?);
        }
    }

    let family_narrowed = profile_requires_family_narrowing(profile);
    let page_budget = if family_narrowed {
        MAX_HF_SEARCH_PAGES
    } else {
        routes.len()
    };
    let mut pages_fetched = 0_usize;
    let mut all_cards = Vec::new();

    loop {
        let mut fetched_this_round = false;
        for route in &mut routes {
            if pages_fetched >= page_budget {
                break;
            }
            let Some(url) = route.next_url.take() else {
                continue;
            };
            if !route.seen_urls.insert(url.as_str().to_string()) {
                return Err(format!(
                    "HuggingFace search pagination for '{}' contained a cycle",
                    route.label
                ));
            }

            let page_number = route.pages_fetched.saturating_add(1);
            let page = fetch_hf_model_page(client, &url, profile.format_tag, &route.identity)
                .await
                .map_err(|error| {
                    format!(
                        "HuggingFace search page {page_number} for '{}' failed: {error}",
                        route.label
                    )
                })?;
            route.pages_fetched = page_number;
            pages_fetched = pages_fetched.saturating_add(1);
            fetched_this_round = true;
            if page
                .next_url
                .as_ref()
                .is_some_and(|next| route.seen_urls.contains(next.as_str()))
            {
                return Err(format!(
                    "HuggingFace search pagination for '{}' contained a cycle",
                    route.label
                ));
            }
            route.next_url = page.next_url;
            all_cards.extend(page.cards);
        }

        let unvisited_pages = routes.iter().any(|route| route.next_url.is_some());
        let compatible_count =
            compatible_profile_model_count(&all_cards, profile, requested_limit as usize);
        if !should_fetch_another_search_round(
            family_narrowed,
            compatible_count,
            requested_limit as usize,
            unvisited_pages,
            fetched_this_round,
            pages_fetched,
            page_budget,
        ) {
            break;
        }
    }

    let unvisited_pages = routes.iter().any(|route| route.next_url.is_some());
    Ok(finalize_profile_search(
        all_cards,
        profile,
        requested_limit,
        unvisited_pages,
    ))
}

pub(crate) mod download;
pub use download::{
    direct_runtime_discover_embedding_dimension, direct_runtime_download_hf_selection,
    direct_runtime_get_model_files_v2,
};


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "hf_hub/tests/mod.rs"]
mod tests;
