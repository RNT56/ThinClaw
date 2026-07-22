//! InferenceRouter — the central routing struct.
//!
//! Holds one active backend per modality, loaded from `UserConfig`.
//! Cloud backends receive keys from `SecretStore` (not `ThinClawConfig`).

use crate::config::UserConfig;
use crate::inference::{InferenceError, InferenceResult};
use crate::secret_store::SecretStore;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::diffusion::DiffusionBackend;
use super::embedding::EmbeddingBackend;
use super::stt::SttBackend;
use super::tts::TtsBackend;
use super::{BackendInfo, Modality};

fn configured_cloud_embedding_model(config: &UserConfig, provider: &str) -> Option<String> {
    let models = config.inference_models.as_ref()?;
    let provider_key = format!("embedding_{provider}");
    if let Some(model) = models.get(&provider_key) {
        return Some(model.clone());
    }

    // Older configs used one `embedding` slot for both a local filesystem path
    // and a cloud model ID. Only accept that legacy value when it is a known
    // model for the selected provider; otherwise a local path could be sent to
    // a cloud API when the user switches modes.
    models
        .get("embedding")
        .filter(|model| {
            super::embedding::cloud_embedding_dimensions(provider, model.as_str()).is_some()
        })
        .cloned()
}

fn is_known_cloud_diffusion_model(provider: &str, model: &str) -> bool {
    match provider {
        "gemini" => matches!(
            model,
            "gemini-3.1-flash-image"
                | "gemini-3-pro-image"
                | "gemini-2.5-flash-image"
                | "gemini-3-pro-image-preview"
                | "nano-banana"
                | "nano-banana-pro"
        ),
        "fal" => matches!(model, "fal-ai/flux/dev" | "fal-ai/flux/schnell"),
        "together" => matches!(
            model,
            "black-forest-labs/FLUX.1-schnell-Free"
                | "black-forest-labs/FLUX.1-schnell"
                | "black-forest-labs/FLUX.1.1-pro"
        ),
        "openai" => model == "gpt-image-2",
        "stability" => model == "sd3.5-large",
        _ => false,
    }
}

fn configured_cloud_diffusion_model(config: &UserConfig, provider: &str) -> Option<String> {
    let models = config.inference_models.as_ref()?;
    let provider_key = format!("diffusion_{provider}");
    if let Some(model) = models.get(&provider_key) {
        return Some(model.clone());
    }

    // The legacy generic slot is also used for local filesystem model paths.
    // Never forward one of those paths to a cloud provider after a mode switch.
    models
        .get("diffusion")
        .filter(|model| is_known_cloud_diffusion_model(provider, model))
        .cloned()
}

/// Result of a `reconfigure()` call.
///
/// Callers can inspect this to determine if the embedding backend changed
/// dimensions, in which case vector indices may need to be rebuilt.
#[derive(Debug)]
pub struct ReconfigureResult {
    /// The embedding dimension of the **previous** backend (0 = none).
    pub old_embedding_dims: usize,
    /// The embedding dimension of the **new** backend (0 = none/local).
    pub new_embedding_dims: usize,
}

impl ReconfigureResult {
    /// Returns `true` if the embedding dimensions changed and both are non-zero
    /// (meaning existing indices are incompatible with the new backend).
    pub fn embedding_dims_changed(&self) -> bool {
        self.old_embedding_dims > 0
            && self.new_embedding_dims > 0
            && self.old_embedding_dims != self.new_embedding_dims
    }
}

/// Central inference routing state.
///
/// Managed as `app.manage(InferenceRouter::new(...))`.  Each modality has its
/// own active backend that can be hot-swapped at runtime.
pub struct InferenceRouter {
    embedding: RwLock<Option<Arc<dyn EmbeddingBackend>>>,
    tts: RwLock<Option<Arc<dyn TtsBackend>>>,
    stt: RwLock<Option<Arc<dyn SttBackend>>>,
    diffusion: RwLock<Option<Arc<dyn DiffusionBackend>>>,
    /// Reference to the app-wide secret store for live key reads.
    secret_store: Arc<SecretStore>,
    /// Managed output location for generated images.
    images_dir: PathBuf,
}

impl InferenceRouter {
    /// Create a new router.
    ///
    /// All backends start as `None`.  Call `reconfigure()` or
    /// `set_embedding_backend()` etc. to activate them.
    pub fn new(secret_store: Arc<SecretStore>, images_dir: PathBuf) -> Self {
        Self {
            embedding: RwLock::new(None),
            tts: RwLock::new(None),
            stt: RwLock::new(None),
            diffusion: RwLock::new(None),
            secret_store,
            images_dir,
        }
    }

    fn get_secret(&self, secret_name: &str) -> Option<String> {
        thinclaw_runtime_contracts::descriptor_for_secret_name(secret_name)
            .and_then(|descriptor| self.secret_store.get_descriptor_secret(&descriptor))
            .or_else(|| self.secret_store.get(secret_name))
    }

    fn has_secret(&self, secret_name: &str) -> bool {
        thinclaw_runtime_contracts::descriptor_for_secret_name(secret_name)
            .map(|descriptor| self.secret_store.has_descriptor_secret(&descriptor))
            .unwrap_or_else(|| self.secret_store.has(secret_name))
    }

    // ─────────────────────────────────────────────────────────────────────
    // Accessors — return the active backend for each modality
    // ─────────────────────────────────────────────────────────────────────

    /// Get the active embedding backend.
    pub async fn embedding_backend(&self) -> Option<Arc<dyn EmbeddingBackend>> {
        self.embedding.read().await.clone()
    }

    /// Get the active TTS backend.
    pub async fn tts_backend(&self) -> Option<Arc<dyn TtsBackend>> {
        self.tts.read().await.clone()
    }

    /// Get the active STT backend.
    pub async fn stt_backend(&self) -> Option<Arc<dyn SttBackend>> {
        self.stt.read().await.clone()
    }

    /// Get the active diffusion backend.
    pub async fn diffusion_backend(&self) -> Option<Arc<dyn DiffusionBackend>> {
        self.diffusion.read().await.clone()
    }

    /// Resolve the backend explicitly selected by an image-generation request.
    /// This prevents an Imagine UI selection from accidentally being billed to
    /// a different, globally active diffusion provider.
    pub async fn diffusion_backend_for(
        &self,
        provider: &str,
        model_hint: Option<&str>,
    ) -> InferenceResult<Arc<dyn DiffusionBackend>> {
        let (provider, forced_model) = match provider {
            "nano-banana" => ("gemini", Some("gemini-3.1-flash-image".to_string())),
            "nano-banana-pro" => ("gemini", Some("gemini-3-pro-image".to_string())),
            provider => (provider, model_hint.map(str::to_string)),
        };
        let api_key = self.get_secret(provider).ok_or_else(|| {
            InferenceError::auth(format!(
                "No credential is configured for the {provider} image provider"
            ))
        })?;
        let backend: Arc<dyn DiffusionBackend> = match provider {
            "openai" => Arc::new(super::diffusion::cloud_dalle::DalleDiffusionBackend::new(
                api_key,
                self.images_dir.clone(),
            )),
            "gemini" => Arc::new(super::diffusion::cloud_imagen::ImagenDiffusionBackend::new(
                api_key,
                forced_model,
                self.images_dir.clone(),
            )),
            "stability" => Arc::new(
                super::diffusion::cloud_stability::StabilityDiffusionBackend::new(
                    api_key,
                    self.images_dir.clone(),
                ),
            ),
            "fal" => Arc::new(super::diffusion::cloud_fal::FalDiffusionBackend::new(
                api_key,
                forced_model,
                self.images_dir.clone(),
            )),
            "together" => Arc::new(
                super::diffusion::cloud_together::TogetherDiffusionBackend::new(
                    api_key,
                    forced_model,
                    self.images_dir.clone(),
                ),
            ),
            _ => {
                return Err(InferenceError::config(format!(
                    "Unsupported image provider '{provider}'"
                )));
            }
        };
        if !backend.info().available {
            return Err(InferenceError::config(format!(
                "The selected {provider} image model is not supported"
            )));
        }
        Ok(backend)
    }

    /// Get a reference to the secret store.
    pub fn secret_store(&self) -> &SecretStore {
        &self.secret_store
    }

    // ─────────────────────────────────────────────────────────────────────
    // Setters — swap backends at runtime
    // ─────────────────────────────────────────────────────────────────────

    /// Set the active embedding backend.
    pub async fn set_embedding_backend(&self, backend: Arc<dyn EmbeddingBackend>) {
        *self.embedding.write().await = Some(backend);
    }

    /// Set the active TTS backend.
    pub async fn set_tts_backend(&self, backend: Arc<dyn TtsBackend>) {
        *self.tts.write().await = Some(backend);
    }

    /// Set the active STT backend.
    pub async fn set_stt_backend(&self, backend: Arc<dyn SttBackend>) {
        *self.stt.write().await = Some(backend);
    }

    /// Set the active diffusion backend.
    pub async fn set_diffusion_backend(&self, backend: Arc<dyn DiffusionBackend>) {
        *self.diffusion.write().await = Some(backend);
    }

    /// Clear a backend (set to None).
    pub async fn clear_backend(&self, modality: Modality) {
        match modality {
            // Chat is not router-managed; the active chat path reads
            // `config.chat_backend` directly (see `chat::resolve_provider`).
            Modality::Chat => {}
            Modality::Embedding => *self.embedding.write().await = None,
            Modality::Tts => *self.tts.write().await = None,
            Modality::Stt => *self.stt.write().await = None,
            Modality::Diffusion => *self.diffusion.write().await = None,
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Introspection
    // ─────────────────────────────────────────────────────────────────────

    /// Synthesize the active chat-backend badge from config.
    ///
    /// Chat is not a router-managed modality — the live chat path
    /// (`chat::resolve_provider`) builds its provider directly from
    /// `config.chat_backend`. This derives the settings-UI "active" badge
    /// from that same config field so the badge stays accurate.
    fn active_chat_backend(&self, config: &UserConfig) -> Option<BackendInfo> {
        let chat_id = config
            .chat_backend
            .as_deref()
            .or(config.selected_chat_provider.as_deref())
            .unwrap_or("local");

        if chat_id == "local" {
            return Some(BackendInfo {
                id: "local".to_string(),
                display_name: "Local (llama.cpp / MLX)".to_string(),
                is_local: true,
                model_id: config
                    .inference_models
                    .as_ref()
                    .and_then(|m| m.get("chat"))
                    .cloned(),
                available: true,
            });
        }

        let endpoint = thinclaw_config::provider_catalog::endpoint_for(chat_id)?;
        let model_id = config
            .inference_models
            .as_ref()
            .and_then(|m| m.get("chat"))
            .cloned()
            .unwrap_or_else(|| endpoint.default_model.to_string());

        Some(BackendInfo {
            id: chat_id.to_string(),
            display_name: endpoint.display_name.to_string(),
            is_local: false,
            model_id: Some(model_id),
            available: self.has_secret(&endpoint.secret_name),
        })
    }

    /// Get info about the currently active backend for each modality.
    /// Returns a list of (modality, info) pairs.  `info` is `None` if
    /// no backend is active for that modality.
    ///
    /// The chat badge is synthesized from `config` (see
    /// [`Self::active_chat_backend`]) because chat is not router-managed.
    pub async fn active_backends(
        &self,
        config: &UserConfig,
    ) -> Vec<(Modality, Option<BackendInfo>)> {
        vec![
            (Modality::Chat, self.active_chat_backend(config)),
            (
                Modality::Embedding,
                self.embedding.read().await.as_ref().map(|b| b.info()),
            ),
            (
                Modality::Tts,
                self.tts.read().await.as_ref().map(|b| b.info()),
            ),
            (
                Modality::Stt,
                self.stt.read().await.as_ref().map(|b| b.info()),
            ),
            (
                Modality::Diffusion,
                self.diffusion.read().await.as_ref().map(|b| b.info()),
            ),
        ]
    }

    /// List all available backends (active + those that COULD be activated)
    /// for a given modality, based on available API keys.
    pub fn available_backends_for(&self, modality: Modality) -> Vec<BackendInfo> {
        let mut backends = vec![];

        // Local is always "available" (might not be running, but it's an option)
        backends.push(BackendInfo {
            id: "local".to_string(),
            display_name: match modality {
                Modality::Chat => "Local (llama.cpp / MLX)".to_string(),
                Modality::Embedding => "Local (llama-server / mlx-embed)".to_string(),
                Modality::Tts => "Local (Piper)".to_string(),
                Modality::Stt => "Local (Whisper)".to_string(),
                Modality::Diffusion => "Local (sd.cpp / mflux)".to_string(),
            },
            is_local: true,
            model_id: None,
            available: true,
        });

        // Add cloud backends based on the shared provider catalog and modality
        // specific direct-workbench support.
        let cloud = match modality {
            Modality::Chat => thinclaw_config::provider_catalog::catalog()
                .values()
                .map(|endpoint| {
                    (
                        endpoint.slug.as_str(),
                        endpoint.display_name.as_str(),
                        endpoint.secret_name.as_str(),
                    )
                })
                .collect(),
            Modality::Embedding => vec![
                ("openai", "OpenAI Embeddings", "llm_openai_api_key"),
                ("gemini", "Gemini Embeddings", "gemini"),
                ("voyage", "Voyage AI", "voyage"),
                ("cohere", "Cohere Embed", "cohere"),
            ],
            Modality::Tts => vec![
                ("openai", "OpenAI TTS", "llm_openai_api_key"),
                ("elevenlabs", "ElevenLabs", "elevenlabs"),
                ("gemini", "Gemini TTS", "gemini"),
            ],
            Modality::Stt => vec![
                ("openai", "OpenAI Whisper", "llm_openai_api_key"),
                ("gemini", "Gemini STT", "gemini"),
                ("deepgram", "Deepgram", "deepgram"),
            ],
            Modality::Diffusion => vec![
                ("openai", "OpenAI GPT Image 2", "llm_openai_api_key"),
                ("gemini", "Gemini Nano Banana", "gemini"),
                ("stability", "Stability AI", "stability"),
                ("fal", "fal.ai", "fal"),
                ("together", "Together AI", "together"),
            ],
        };

        for (key_slug, display_name, secret_name) in cloud {
            backends.push(BackendInfo {
                id: key_slug.to_string(),
                display_name: display_name.to_string(),
                is_local: false,
                model_id: None,
                available: self.has_secret(secret_name),
            });
        }

        backends
    }

    /// Reconfigure all backends from the given `UserConfig`.
    ///
    /// This reads the per-modality backend settings from the config and
    /// allocates the appropriate backend implementations.  Called on
    /// startup and when the user changes settings.
    ///
    /// **Cloud backends** are constructed eagerly — they only need an API key.
    /// **Local backends** are NOT constructed here — they require Tauri state
    /// (SidecarManager, EngineManager) that isn't available to the router.
    /// Local backends are set lazily via `set_*_backend()` when the first
    /// request arrives and the sidecar is started.
    pub async fn reconfigure(&self, config: &UserConfig) -> ReconfigureResult {
        tracing::info!("[inference_router] Reconfiguring backends from UserConfig");

        // Track old embedding dims for the dimension guard
        let old_embedding_dims = {
            let guard = self.embedding.read().await;
            guard.as_ref().map(|b| b.dimensions()).unwrap_or(0)
        };

        // ── Chat backend ────────────────────────────────────────────────
        // Chat is not router-managed: the live chat path
        // (`chat::resolve_provider`) builds its provider directly from
        // `config.chat_backend`. Nothing to construct here.

        // ── Embedding backend ───────────────────────────────────────────
        let embed_id = config.embedding_backend.as_deref().unwrap_or("local");
        tracing::info!("[inference_router] Embedding backend: {}", embed_id);

        let next_embedding: Option<Arc<dyn EmbeddingBackend>> = if embed_id != "local" {
            if let Some(api_key) = self.get_secret(embed_id) {
                let model_override = configured_cloud_embedding_model(config, embed_id);
                let maybe_backend: Option<Arc<dyn EmbeddingBackend>> = match embed_id {
                    "openai" => Some(Arc::new(
                        super::embedding::cloud_openai::OpenAiEmbeddingBackend::new(
                            api_key,
                            model_override,
                        ),
                    )),
                    "gemini" => Some(Arc::new(
                        super::embedding::cloud_gemini::GeminiEmbeddingBackend::new(
                            api_key,
                            model_override,
                        ),
                    )),
                    "voyage" => Some(Arc::new(
                        super::embedding::cloud_voyage::VoyageEmbeddingBackend::new(
                            api_key,
                            model_override,
                        ),
                    )),
                    "cohere" => Some(Arc::new(
                        super::embedding::cloud_cohere::CohereEmbeddingBackend::new(
                            api_key,
                            model_override,
                        ),
                    )),
                    other => {
                        tracing::warn!("[inference_router] Unknown embedding backend: {}", other);
                        None
                    }
                };
                if let Some(backend) = maybe_backend.filter(|backend| backend.dimensions() > 0) {
                    tracing::info!(
                        "[inference_router] Activated embedding backend: {} ({}d)",
                        embed_id,
                        backend.dimensions()
                    );
                    Some(backend)
                } else {
                    tracing::warn!(
                        "[inference_router] Embedding backend configuration was rejected"
                    );
                    None
                }
            } else {
                tracing::warn!("[inference_router] Embedding backend credential is unavailable");
                None
            }
        } else {
            None
        };
        *self.embedding.write().await = next_embedding;

        // ── TTS backend ─────────────────────────────────────────────────
        let tts_id = config.tts_backend.as_deref().unwrap_or("local");
        tracing::info!("[inference_router] TTS backend: {}", tts_id);

        let next_tts: Option<Arc<dyn TtsBackend>> = if tts_id != "local" {
            if let Some(api_key) = self.get_secret(tts_id) {
                let backend: Option<Arc<dyn TtsBackend>> = match tts_id {
                    "openai" => Some(Arc::new(super::tts::cloud_openai::OpenAiTtsBackend::new(
                        api_key,
                    ))),
                    "elevenlabs" => Some(Arc::new(
                        super::tts::cloud_elevenlabs::ElevenLabsTtsBackend::new(api_key),
                    )),
                    "gemini" => Some(Arc::new(super::tts::cloud_gemini::GeminiTtsBackend::new(
                        api_key,
                    ))),
                    other => {
                        tracing::warn!("[inference_router] Unknown TTS backend: {}", other);
                        None
                    }
                };
                backend
            } else {
                tracing::warn!("[inference_router] TTS backend credential is unavailable");
                None
            }
        } else {
            None
        };
        *self.tts.write().await = next_tts;

        // ── STT backend ─────────────────────────────────────────────────
        let stt_id = config.stt_backend.as_deref().unwrap_or("local");
        tracing::info!("[inference_router] STT backend: {}", stt_id);

        let next_stt: Option<Arc<dyn SttBackend>> = if stt_id != "local" {
            if let Some(api_key) = self.get_secret(stt_id) {
                let backend: Option<Arc<dyn SttBackend>> = match stt_id {
                    "openai" => Some(Arc::new(super::stt::cloud_openai::OpenAiSttBackend::new(
                        api_key,
                    ))),
                    "gemini" => Some(Arc::new(super::stt::cloud_gemini::GeminiSttBackend::new(
                        api_key,
                    ))),
                    "deepgram" => Some(Arc::new(
                        super::stt::cloud_deepgram::DeepgramSttBackend::new(api_key),
                    )),
                    other => {
                        tracing::warn!("[inference_router] Unknown STT backend: {}", other);
                        None
                    }
                };
                backend
            } else {
                tracing::warn!("[inference_router] STT backend credential is unavailable");
                None
            }
        } else {
            None
        };
        *self.stt.write().await = next_stt;

        // ── Diffusion backend ───────────────────────────────────────────
        let diffusion_id = config.diffusion_backend.as_deref().unwrap_or("local");
        tracing::info!("[inference_router] Diffusion backend: {}", diffusion_id);

        let next_diffusion: Option<Arc<dyn DiffusionBackend>> = if diffusion_id != "local" {
            if let Some(api_key) = self.get_secret(diffusion_id) {
                let model_override = configured_cloud_diffusion_model(config, diffusion_id);
                let backend: Option<Arc<dyn DiffusionBackend>> = match diffusion_id {
                    "openai" => Some(Arc::new(
                        super::diffusion::cloud_dalle::DalleDiffusionBackend::new(
                            api_key,
                            self.images_dir.clone(),
                        ),
                    )),
                    "gemini" => Some(Arc::new(
                        super::diffusion::cloud_imagen::ImagenDiffusionBackend::new(
                            api_key,
                            model_override,
                            self.images_dir.clone(),
                        ),
                    )),
                    "stability" => Some(Arc::new(
                        super::diffusion::cloud_stability::StabilityDiffusionBackend::new(
                            api_key,
                            self.images_dir.clone(),
                        ),
                    )),
                    "fal" => Some(Arc::new(
                        super::diffusion::cloud_fal::FalDiffusionBackend::new(
                            api_key,
                            model_override,
                            self.images_dir.clone(),
                        ),
                    )),
                    "together" => Some(Arc::new(
                        super::diffusion::cloud_together::TogetherDiffusionBackend::new(
                            api_key,
                            model_override,
                            self.images_dir.clone(),
                        ),
                    )),
                    other => {
                        tracing::warn!("[inference_router] Unknown diffusion backend: {}", other);
                        None
                    }
                };
                backend
            } else {
                tracing::warn!("[inference_router] Diffusion backend credential is unavailable");
                None
            }
        } else {
            None
        };
        *self.diffusion.write().await = next_diffusion;

        // ── Build result ────────────────────────────────────────────────
        let new_embedding_dims = {
            let guard = self.embedding.read().await;
            guard.as_ref().map(|b| b.dimensions()).unwrap_or(0)
        };

        if old_embedding_dims > 0
            && new_embedding_dims > 0
            && old_embedding_dims != new_embedding_dims
        {
            tracing::warn!(
                "[inference_router] ⚠️ Embedding dimensions changed: {} → {}. Vector indices must be rebuilt!",
                old_embedding_dims,
                new_embedding_dims
            );
        }

        tracing::info!("[inference_router] Reconfiguration complete");

        ReconfigureResult {
            old_embedding_dims,
            new_embedding_dims,
        }
    }
}
