//! Model classifier — pattern-matches provider + model ID to determine modality.

use super::types::ModelCategory;

/// Classify a model ID into a modality category based on provider conventions.
///
/// Returns `None` if the model doesn't match any known pattern (will be
/// categorized as `ModelCategory::Other`).
pub fn classify_model(provider: &str, model_id: &str) -> ModelCategory {
    let id = model_id.to_lowercase();

    match provider {
        "openai" => classify_openai(&id),
        "anthropic" => ModelCategory::Chat, // All Anthropic models are chat
        "gemini" => classify_gemini(&id),
        "groq" => classify_groq(&id),
        "openrouter" => ModelCategory::Chat, // OpenRouter is a chat gateway
        "mistral" => classify_mistral(&id),
        "xai" => ModelCategory::Chat, // All xAI models are chat
        "together" => classify_together(&id),
        "cohere" => classify_cohere(&id),
        "elevenlabs" => ModelCategory::Tts, // All ElevenLabs models are TTS
        "stability" => ModelCategory::Diffusion, // All Stability models are diffusion
        "deepgram" => ModelCategory::Stt,   // All Deepgram models are STT
        "voyage" => ModelCategory::Embedding, // All Voyage models are embedding
        "fal" => ModelCategory::Diffusion,  // All fal.ai models are diffusion
        _ => ModelCategory::Other,
    }
}

fn classify_openai(id: &str) -> ModelCategory {
    // ── Non-chat modalities (check FIRST — some start with "gpt-") ─────
    // Image generation
    if id.starts_with("gpt-image-") || id.starts_with("chatgpt-image") || id.starts_with("dall-e") {
        return ModelCategory::Diffusion;
    }
    // Video generation
    if id.starts_with("sora") {
        return ModelCategory::Diffusion;
    }
    // Text-to-speech
    if id.starts_with("tts-") || id.contains("-tts") {
        return ModelCategory::Tts;
    }
    // Speech-to-text / transcription
    if id.starts_with("whisper") || id.contains("transcribe") {
        return ModelCategory::Stt;
    }
    // Embeddings
    if id.starts_with("text-embedding-") || id.contains("embedding") {
        return ModelCategory::Embedding;
    }
    // Realtime / audio-only models (not usable via Chat Completions API)
    if id.contains("realtime") || id.starts_with("gpt-audio") {
        return ModelCategory::Other;
    }
    // Moderation models
    if id.contains("moderation") || id.starts_with("omni-moderation") {
        return ModelCategory::Other;
    }
    // Computer use preview
    if id.starts_with("computer-use") {
        return ModelCategory::Other;
    }
    // Deprecated base models
    if id == "babbage-002" || id == "davinci-002" {
        return ModelCategory::Other;
    }

    // ── Chat models ────────────────────────────────────────────────────
    // GPT family: gpt-5.4, gpt-5.3-codex, gpt-5-mini, gpt-4.1, gpt-4o, etc.
    if id.starts_with("gpt-")
        || id.starts_with("chatgpt-")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || id.starts_with("gpt-oss-")
        || id.starts_with("codex-")
    {
        return ModelCategory::Chat;
    }

    ModelCategory::Other
}

fn classify_gemini(id: &str) -> ModelCategory {
    if id.contains("embedding") || id.contains("text-embedding") {
        ModelCategory::Embedding
    } else if id.contains("imagen") || id.contains("veo") {
        // Imagen = image gen, Veo = video gen
        ModelCategory::Diffusion
    } else if id.contains("lyria") || id.contains("chirp") {
        // Lyria = music gen, Chirp = voice
        ModelCategory::Other
    } else if id.contains("aqa") || id.contains("retrieval") {
        // Attributed QA / retrieval models
        ModelCategory::Other
    } else {
        // Default Gemini models are chat (gemini-2.5-flash, gemini-3-pro, etc.)
        ModelCategory::Chat
    }
}

fn classify_groq(id: &str) -> ModelCategory {
    if id.starts_with("whisper") || id.contains("whisper") {
        ModelCategory::Stt
    } else {
        ModelCategory::Chat
    }
}

fn classify_mistral(id: &str) -> ModelCategory {
    if id.contains("embed") {
        ModelCategory::Embedding
    } else {
        ModelCategory::Chat
    }
}

fn classify_together(id: &str) -> ModelCategory {
    if id.contains("flux") || id.contains("stable-diffusion") || id.contains("sdxl") {
        ModelCategory::Diffusion
    } else if id.contains("embed") || id.contains("bge-") {
        ModelCategory::Embedding
    } else {
        ModelCategory::Chat
    }
}

fn classify_cohere(id: &str) -> ModelCategory {
    if id.starts_with("embed-") || id.contains("embed") {
        ModelCategory::Embedding
    } else {
        ModelCategory::Chat
    }
}
