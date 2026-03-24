//! Google Gemini LLM provider adapter.
//!
//! Adapts Google's Gemini API (via AI Studio) to the OpenAI-compatible
//! format used by the rest of the provider chain.
//!
//! Configuration:
//! - `GOOGLE_AI_API_KEY` — API key from Google AI Studio
//! - `GEMINI_MODEL` — Model name (default: "gemini-3.1-flash")
//! - `GEMINI_BASE_URL` — Base URL (default: "https://generativelanguage.googleapis.com/v1beta/openai")

use serde::{Deserialize, Serialize};

/// Gemini adapter configuration.
#[derive(Debug, Clone)]
pub struct GeminiConfig {
    /// API key for Google AI Studio.
    pub api_key: Option<String>,
    /// Model name.
    pub model: String,
    /// Base URL for the Gemini API.
    pub base_url: String,
    /// Maximum output tokens.
    pub max_output_tokens: u32,
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model: "gemini-3.1-flash".to_string(),
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai".to_string(),
            max_output_tokens: 8192,
        }
    }
}

impl GeminiConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let api_key = std::env::var("GOOGLE_AI_API_KEY").ok();

        let mut config = Self {
            api_key,
            ..Self::default()
        };

        if let Ok(model) = std::env::var("GEMINI_MODEL") {
            config.model = model;
        }

        if let Ok(url) = std::env::var("GEMINI_BASE_URL") {
            config.base_url = url;
        }

        if let Ok(max) = std::env::var("GEMINI_MAX_OUTPUT_TOKENS")
            && let Ok(m) = max.parse()
        {
            config.max_output_tokens = m;
        }

        config
    }

    /// Build the full API URL for generateContent.
    pub fn generate_content_url(&self) -> String {
        format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url,
            self.model,
            self.api_key.as_deref().unwrap_or(""),
        )
    }

    /// Build the full API URL for streamGenerateContent.
    pub fn stream_url(&self) -> String {
        format!(
            "{}/models/{}:streamGenerateContent?key={}",
            self.base_url,
            self.model,
            self.api_key.as_deref().unwrap_or(""),
        )
    }

    /// Convert an OpenAI-format request to Gemini format.
    pub fn adapt_request(&self, openai_body: &serde_json::Value) -> GeminiRequest {
        let messages = openai_body
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Extract system instruction
        let system_instruction = messages
            .iter()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
            .and_then(|m| m.get("content").and_then(|c| c.as_str()))
            .map(|s| GeminiContent {
                role: None,
                parts: vec![GeminiPart::Text(s.to_string())],
            });

        // Convert messages
        let contents: Vec<GeminiContent> = messages
            .iter()
            .filter(|m| m.get("role").and_then(|r| r.as_str()) != Some("system"))
            .map(|m| {
                let role = match m.get("role").and_then(|r| r.as_str()) {
                    Some("assistant") => "model",
                    _ => "user",
                };
                let content = m
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();

                GeminiContent {
                    role: Some(role.to_string()),
                    parts: vec![GeminiPart::Text(content)],
                }
            })
            .collect();

        let temperature = openai_body.get("temperature").and_then(|v| v.as_f64());

        let max_tokens = openai_body
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.max_output_tokens as u64) as u32;

        GeminiRequest {
            contents,
            system_instruction,
            generation_config: Some(GeminiGenerationConfig {
                temperature,
                max_output_tokens: Some(max_tokens),
                top_p: openai_body.get("top_p").and_then(|v| v.as_f64()),
            }),
        }
    }

    /// Convert a Gemini response back to OpenAI format.
    pub fn adapt_response(&self, gemini_response: &GeminiResponse) -> serde_json::Value {
        let content = gemini_response
            .candidates
            .as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.content.parts.first())
            .map(|p| match p {
                GeminiPart::Text(t) => t.clone(),
            })
            .unwrap_or_default();

        let finish_reason = gemini_response
            .candidates
            .as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.finish_reason.as_deref())
            .unwrap_or("stop");

        let (prompt_tokens, completion_tokens) = gemini_response
            .usage_metadata
            .as_ref()
            .map(|u| {
                (
                    u.prompt_token_count.unwrap_or(0),
                    u.candidates_token_count.unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));

        serde_json::json!({
            "id": format!("gemini-{}", uuid::Uuid::new_v4()),
            "object": "chat.completion",
            "model": self.model,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content,
                },
                "finish_reason": finish_reason,
            }],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens,
            }
        })
    }
}

/// Gemini API request.
#[derive(Debug, Clone, Serialize)]
pub struct GeminiRequest {
    pub contents: Vec<GeminiContent>,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<GeminiContent>,
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GeminiGenerationConfig>,
}

/// Gemini generation configuration.
#[derive(Debug, Clone, Serialize)]
pub struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(rename = "topP", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
}

/// Content block in Gemini format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub parts: Vec<GeminiPart>,
}

/// A part of a content block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GeminiPart {
    Text(String),
}

/// Gemini API response.
#[derive(Debug, Clone, Deserialize)]
pub struct GeminiResponse {
    pub candidates: Option<Vec<GeminiCandidate>>,
    #[serde(rename = "usageMetadata")]
    pub usage_metadata: Option<GeminiUsageMetadata>,
}

/// A candidate response from Gemini.
#[derive(Debug, Clone, Deserialize)]
pub struct GeminiCandidate {
    pub content: GeminiContent,
    #[serde(rename = "finishReason")]
    pub finish_reason: Option<String>,
}

/// Usage metadata from Gemini.
#[derive(Debug, Clone, Deserialize)]
pub struct GeminiUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    pub prompt_token_count: Option<u32>,
    #[serde(rename = "candidatesTokenCount")]
    pub candidates_token_count: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GeminiConfig::default();
        assert_eq!(config.model, "gemini-3.1-flash");
        assert!(config.base_url.contains("googleapis.com"));
    }

    #[test]
    fn test_adapt_request() {
        let config = GeminiConfig::default();
        let body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "Be helpful"},
                {"role": "user", "content": "Hi"}
            ],
            "temperature": 0.5,
        });

        let req = config.adapt_request(&body);
        assert!(req.system_instruction.is_some());
        assert_eq!(req.contents.len(), 1); // system excluded
        assert_eq!(req.contents[0].role.as_deref(), Some("user"));
    }

    #[test]
    fn test_adapt_request_no_system() {
        let config = GeminiConfig::default();
        let body = serde_json::json!({
            "messages": [
                {"role": "user", "content": "Hi"},
                {"role": "assistant", "content": "Hello"},
                {"role": "user", "content": "Thanks"},
            ]
        });

        let req = config.adapt_request(&body);
        assert!(req.system_instruction.is_none());
        assert_eq!(req.contents.len(), 3);
        assert_eq!(req.contents[1].role.as_deref(), Some("model")); // assistant → model
    }

    #[test]
    fn test_generate_content_url() {
        let config = GeminiConfig {
            api_key: Some("test-key".to_string()),
            ..Default::default()
        };
        let url = config.generate_content_url();
        assert!(url.contains("gemini-3.1-flash"));
        assert!(url.contains("test-key"));
    }

    #[test]
    fn test_adapt_response() {
        let config = GeminiConfig::default();
        let response = GeminiResponse {
            candidates: Some(vec![GeminiCandidate {
                content: GeminiContent {
                    role: Some("model".to_string()),
                    parts: vec![GeminiPart::Text("Hello there!".to_string())],
                },
                finish_reason: Some("STOP".to_string()),
            }]),
            usage_metadata: Some(GeminiUsageMetadata {
                prompt_token_count: Some(20),
                candidates_token_count: Some(10),
            }),
        };

        let adapted = config.adapt_response(&response);
        assert_eq!(
            adapted["choices"][0]["message"]["content"].as_str(),
            Some("Hello there!")
        );
        assert_eq!(adapted["usage"]["total_tokens"].as_u64(), Some(30));
    }
}
