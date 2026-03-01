//! InferenceRouter — the central routing struct.
//!
//! Holds one active backend per modality, loaded from `UserConfig`.
//! Cloud backends receive keys from `SecretStore` (not `OpenClawConfig`).

use crate::config::UserConfig;
use crate::secret_store::SecretStore;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::chat::ChatBackend;
use super::diffusion::DiffusionBackend;
use super::embedding::EmbeddingBackend;
use super::stt::SttBackend;
use super::tts::TtsBackend;
use super::{BackendInfo, Modality};

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
    chat: RwLock<Option<Arc<dyn ChatBackend>>>,
    embedding: RwLock<Option<Arc<dyn EmbeddingBackend>>>,
    tts: RwLock<Option<Arc<dyn TtsBackend>>>,
    stt: RwLock<Option<Arc<dyn SttBackend>>>,
    diffusion: RwLock<Option<Arc<dyn DiffusionBackend>>>,
    /// Reference to the app-wide secret store for live key reads.
    secret_store: Arc<SecretStore>,
}

impl InferenceRouter {
    /// Create a new router.
    ///
    /// All backends start as `None`.  Call `reconfigure()` or
    /// `set_chat_backend()` etc. to activate them.
    pub fn new(secret_store: Arc<SecretStore>) -> Self {
        Self {
            chat: RwLock::new(None),
            embedding: RwLock::new(None),
            tts: RwLock::new(None),
            stt: RwLock::new(None),
            diffusion: RwLock::new(None),
            secret_store,
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Accessors — return the active backend for each modality
    // ─────────────────────────────────────────────────────────────────────

    /// Get the active chat backend.
    pub async fn chat_backend(&self) -> Option<Arc<dyn ChatBackend>> {
        self.chat.read().await.clone()
    }

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

    /// Get a reference to the secret store.
    pub fn secret_store(&self) -> &SecretStore {
        &self.secret_store
    }

    // ─────────────────────────────────────────────────────────────────────
    // Setters — swap backends at runtime
    // ─────────────────────────────────────────────────────────────────────

    /// Set the active chat backend.
    pub async fn set_chat_backend(&self, backend: Arc<dyn ChatBackend>) {
        *self.chat.write().await = Some(backend);
    }

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
            Modality::Chat => *self.chat.write().await = None,
            Modality::Embedding => *self.embedding.write().await = None,
            Modality::Tts => *self.tts.write().await = None,
            Modality::Stt => *self.stt.write().await = None,
            Modality::Diffusion => *self.diffusion.write().await = None,
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Introspection
    // ─────────────────────────────────────────────────────────────────────

    /// Get info about the currently active backend for each modality.
    /// Returns a list of (modality, info) pairs.  `info` is `None` if
    /// no backend is active for that modality.
    pub async fn active_backends(&self) -> Vec<(Modality, Option<BackendInfo>)> {
        vec![
            (
                Modality::Chat,
                self.chat.read().await.as_ref().map(|b| b.info()),
            ),
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

        // Add cloud backends based on available keys
        let cloud = match modality {
            Modality::Chat => vec![
                ("anthropic", "Anthropic"),
                ("openai", "OpenAI"),
                ("gemini", "Google Gemini"),
                ("groq", "Groq"),
                ("openrouter", "OpenRouter"),
                ("mistral", "Mistral AI"),
                ("xai", "xAI (Grok)"),
                ("together", "Together AI"),
                ("venice", "Venice AI"),
                ("moonshot", "Moonshot (Kimi)"),
                ("minimax", "MiniMax"),
                ("nvidia", "NVIDIA NIM"),
                ("cohere", "Cohere"),
                ("xiaomi", "Xiaomi"),
            ],
            Modality::Embedding => vec![
                ("openai", "OpenAI Embeddings"),
                ("gemini", "Gemini Embeddings"),
                ("voyage", "Voyage AI"),
                ("cohere", "Cohere Embed"),
            ],
            Modality::Tts => vec![
                ("openai", "OpenAI TTS"),
                ("elevenlabs", "ElevenLabs"),
                ("gemini", "Gemini TTS"),
            ],
            Modality::Stt => vec![
                ("openai", "OpenAI Whisper"),
                ("gemini", "Gemini STT"),
                ("deepgram", "Deepgram"),
            ],
            Modality::Diffusion => vec![
                ("openai", "DALL·E 3"),
                ("gemini", "Imagen 3"),
                ("stability", "Stability AI"),
                ("fal", "fal.ai"),
                ("together", "Together AI"),
            ],
        };

        for (key_slug, display_name) in cloud {
            backends.push(BackendInfo {
                id: key_slug.to_string(),
                display_name: display_name.to_string(),
                is_local: false,
                model_id: None,
                available: self.secret_store.has(key_slug),
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
        let chat_id = config
            .chat_backend
            .as_deref()
            .or(config.selected_chat_provider.as_deref())
            .unwrap_or("local");
        tracing::info!("[inference_router] Chat backend: {}", chat_id);

        if chat_id != "local" {
            if let Some(ep) = super::provider_endpoints::endpoint_for(chat_id) {
                if let Some(api_key) = self.secret_store.get(chat_id) {
                    let model_override = config
                        .inference_models
                        .as_ref()
                        .and_then(|m| m.get("chat"))
                        .cloned();
                    let backend = super::chat::cloud::CloudChatBackend::from_endpoint(
                        chat_id,
                        ep,
                        api_key,
                        model_override,
                        config.selected_model_context_size,
                    );
                    *self.chat.write().await = Some(Arc::new(backend));
                    tracing::info!(
                        "[inference_router] Activated cloud chat backend: {}",
                        chat_id
                    );
                } else {
                    tracing::warn!(
                        "[inference_router] Chat backend '{}' selected but no API key found",
                        chat_id
                    );
                }
            }
        } else {
            // Local — will be set lazily when sidecar is started
            tracing::info!("[inference_router] Chat = local (deferred until sidecar starts)");
        }

        // ── Embedding backend ───────────────────────────────────────────
        let embed_id = config.embedding_backend.as_deref().unwrap_or("local");
        tracing::info!("[inference_router] Embedding backend: {}", embed_id);

        if embed_id != "local" {
            if let Some(api_key) = self.secret_store.get(embed_id) {
                let model_override = config
                    .inference_models
                    .as_ref()
                    .and_then(|m| m.get("embedding"))
                    .cloned();
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
                if let Some(backend) = maybe_backend {
                    tracing::info!(
                        "[inference_router] Activated embedding backend: {} ({}d)",
                        embed_id,
                        backend.dimensions()
                    );
                    *self.embedding.write().await = Some(backend);
                }
            }
        }

        // ── TTS backend ─────────────────────────────────────────────────
        let tts_id = config.tts_backend.as_deref().unwrap_or("local");
        tracing::info!("[inference_router] TTS backend: {}", tts_id);

        if tts_id != "local" {
            if let Some(api_key) = self.secret_store.get(tts_id) {
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
                if let Some(b) = backend {
                    *self.tts.write().await = Some(b);
                }
            }
        }

        // ── STT backend ─────────────────────────────────────────────────
        let stt_id = config.stt_backend.as_deref().unwrap_or("local");
        tracing::info!("[inference_router] STT backend: {}", stt_id);

        if stt_id != "local" {
            if let Some(api_key) = self.secret_store.get(stt_id) {
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
                if let Some(b) = backend {
                    *self.stt.write().await = Some(b);
                }
            }
        }

        // ── Diffusion backend ───────────────────────────────────────────
        let diffusion_id = config.diffusion_backend.as_deref().unwrap_or("local");
        tracing::info!("[inference_router] Diffusion backend: {}", diffusion_id);

        if diffusion_id != "local" {
            if let Some(api_key) = self.secret_store.get(diffusion_id) {
                let model_override = config
                    .inference_models
                    .as_ref()
                    .and_then(|m| m.get("diffusion"))
                    .cloned();
                let backend: Option<Arc<dyn DiffusionBackend>> = match diffusion_id {
                    "openai" => Some(Arc::new(
                        super::diffusion::cloud_dalle::DalleDiffusionBackend::new(api_key),
                    )),
                    "gemini" => Some(Arc::new(
                        super::diffusion::cloud_imagen::ImagenDiffusionBackend::new(
                            api_key,
                            model_override,
                        ),
                    )),
                    "stability" => Some(Arc::new(
                        super::diffusion::cloud_stability::StabilityDiffusionBackend::new(api_key),
                    )),
                    "fal" => Some(Arc::new(
                        super::diffusion::cloud_fal::FalDiffusionBackend::new(
                            api_key,
                            model_override,
                        ),
                    )),
                    "together" => Some(Arc::new(
                        super::diffusion::cloud_together::TogetherDiffusionBackend::new(
                            api_key,
                            model_override,
                        ),
                    )),
                    other => {
                        tracing::warn!("[inference_router] Unknown diffusion backend: {}", other);
                        None
                    }
                };
                if let Some(b) = backend {
                    *self.diffusion.write().await = Some(b);
                }
            }
        }

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
