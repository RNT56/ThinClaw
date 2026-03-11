//! AWS Bedrock LLM provider adapter.
//!
//! Adapts AWS Bedrock to the OpenAI-compatible API format so it can
//! be used with the existing provider chain.
//!
//! Configuration:
//! - `AWS_REGION` — AWS region (default: "us-east-1")
//! - `AWS_ACCESS_KEY_ID` — AWS access key
//! - `AWS_SECRET_ACCESS_KEY` — AWS secret key
//! - `BEDROCK_MODEL_ID` — Model ID (default: "anthropic.claude-3-sonnet-20240229-v1:0")

use serde::{Deserialize, Serialize};

/// Bedrock adapter configuration.
#[derive(Debug, Clone)]
pub struct BedrockConfig {
    /// AWS region.
    pub region: String,
    /// Model ID in Bedrock format.
    pub model_id: String,
    /// Bedrock endpoint URL (computed from region).
    pub endpoint_url: String,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
}

impl Default for BedrockConfig {
    fn default() -> Self {
        let region = "us-east-1".to_string();
        Self {
            endpoint_url: format!("https://bedrock-runtime.{}.amazonaws.com", region),
            region,
            model_id: "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
            max_tokens: 4096,
        }
    }
}

impl BedrockConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(region) = std::env::var("AWS_REGION") {
            config.endpoint_url = format!("https://bedrock-runtime.{}.amazonaws.com", region);
            config.region = region;
        }

        if let Ok(model) = std::env::var("BEDROCK_MODEL_ID") {
            config.model_id = model;
        }

        if let Ok(max) = std::env::var("BEDROCK_MAX_TOKENS") {
            if let Ok(m) = max.parse() {
                config.max_tokens = m;
            }
        }

        config
    }

    /// Convert an OpenAI-format request body to Bedrock's InvokeModel format.
    pub fn adapt_request(&self, openai_body: &serde_json::Value) -> BedrockRequest {
        let messages = openai_body
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Extract system message
        let system = messages
            .iter()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
            .and_then(|m| m.get("content").and_then(|c| c.as_str()))
            .map(String::from);

        // Convert messages (exclude system)
        let converted_messages: Vec<BedrockMessage> = messages
            .iter()
            .filter(|m| m.get("role").and_then(|r| r.as_str()) != Some("system"))
            .map(|m| BedrockMessage {
                role: m
                    .get("role")
                    .and_then(|r| r.as_str())
                    .unwrap_or("user")
                    .to_string(),
                content: m
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
            .collect();

        let max_tokens = openai_body
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.max_tokens as u64) as u32;

        let temperature = openai_body.get("temperature").and_then(|v| v.as_f64());

        BedrockRequest {
            model_id: self.model_id.clone(),
            system,
            messages: converted_messages,
            max_tokens,
            temperature,
        }
    }

    /// Convert a Bedrock response back to OpenAI format.
    pub fn adapt_response(
        &self,
        bedrock_response: &BedrockResponse,
        model: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": format!("bedrock-{}", uuid::Uuid::new_v4()),
            "object": "chat.completion",
            "model": model,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": bedrock_response.output_text(),
                },
                "finish_reason": bedrock_response.stop_reason.as_deref().unwrap_or("stop"),
            }],
            "usage": {
                "prompt_tokens": bedrock_response.input_tokens.unwrap_or(0),
                "completion_tokens": bedrock_response.output_tokens.unwrap_or(0),
                "total_tokens": bedrock_response.input_tokens.unwrap_or(0)
                    + bedrock_response.output_tokens.unwrap_or(0),
            }
        })
    }
}

/// Bedrock API request format (Converse API).
#[derive(Debug, Clone, Serialize)]
pub struct BedrockRequest {
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<BedrockMessage>,
    #[serde(rename = "maxTokens")]
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

/// A message in Bedrock format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedrockMessage {
    pub role: String,
    pub content: String,
}

/// Bedrock API response format.
#[derive(Debug, Clone, Deserialize)]
pub struct BedrockResponse {
    pub output: Option<BedrockOutput>,
    #[serde(rename = "stopReason")]
    pub stop_reason: Option<String>,
    #[serde(rename = "inputTokens")]
    pub input_tokens: Option<u32>,
    #[serde(rename = "outputTokens")]
    pub output_tokens: Option<u32>,
}

impl BedrockResponse {
    /// Extract the text output from the response.
    pub fn output_text(&self) -> String {
        self.output
            .as_ref()
            .and_then(|o| o.message.as_ref())
            .and_then(|m| m.content.as_ref())
            .and_then(|c| c.first())
            .and_then(|b| b.get("text").and_then(|t| t.as_str()))
            .unwrap_or("")
            .to_string()
    }
}

/// Bedrock output container.
#[derive(Debug, Clone, Deserialize)]
pub struct BedrockOutput {
    pub message: Option<BedrockOutputMessage>,
}

/// Bedrock output message.
#[derive(Debug, Clone, Deserialize)]
pub struct BedrockOutputMessage {
    pub role: Option<String>,
    pub content: Option<Vec<serde_json::Value>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = BedrockConfig::default();
        assert_eq!(config.region, "us-east-1");
        assert!(config.endpoint_url.contains("us-east-1"));
    }

    #[test]
    fn test_adapt_request() {
        let config = BedrockConfig::default();
        let openai_body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello"},
            ],
            "max_tokens": 1000,
            "temperature": 0.7,
        });

        let req = config.adapt_request(&openai_body);
        assert_eq!(req.system, Some("You are helpful.".to_string()));
        assert_eq!(req.messages.len(), 1); // system excluded
        assert_eq!(req.messages[0].role, "user");
        assert_eq!(req.max_tokens, 1000);
    }

    #[test]
    fn test_adapt_response() {
        let config = BedrockConfig::default();
        let response = BedrockResponse {
            output: Some(BedrockOutput {
                message: Some(BedrockOutputMessage {
                    role: Some("assistant".to_string()),
                    content: Some(vec![serde_json::json!({"text": "Hello!"})]),
                }),
            }),
            stop_reason: Some("end_turn".to_string()),
            input_tokens: Some(10),
            output_tokens: Some(5),
        };

        let adapted = config.adapt_response(&response, "claude-3");
        assert_eq!(
            adapted["choices"][0]["message"]["content"].as_str(),
            Some("Hello!")
        );
        assert_eq!(adapted["usage"]["total_tokens"].as_u64(), Some(15));
    }

    #[test]
    fn test_output_text_empty() {
        let response = BedrockResponse {
            output: None,
            stop_reason: None,
            input_tokens: None,
            output_tokens: None,
        };
        assert_eq!(response.output_text(), "");
    }
}
