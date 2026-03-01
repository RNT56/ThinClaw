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
    if id.starts_with("gpt-")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || id.starts_with("chatgpt")
    {
        ModelCategory::Chat
    } else if id.starts_with("text-embedding-") || id.contains("embedding") {
        ModelCategory::Embedding
    } else if id.starts_with("tts-") {
        ModelCategory::Tts
    } else if id.starts_with("whisper") {
        ModelCategory::Stt
    } else if id.starts_with("dall-e") {
        ModelCategory::Diffusion
    } else {
        ModelCategory::Other
    }
}

fn classify_gemini(id: &str) -> ModelCategory {
    if id.contains("embedding") || id.contains("text-embedding") {
        ModelCategory::Embedding
    } else if id.contains("imagen") {
        ModelCategory::Diffusion
    } else {
        // Default Gemini models are chat (gemini-1.5-pro, gemini-2.0-flash, etc.)
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
