use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionResponse, ModelChoice,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone)]
pub struct LlamaProvider {
    base_url: String,
    api_key: String,
    model: String,
}

impl LlamaProvider {
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
        }
    }
}

#[derive(Serialize)]
struct LlamaChatRequest {
    messages: Vec<serde_json::Value>,
    model: String,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
struct LlamaChatResponse {
    choices: Vec<LlamaChoice>,
}

#[derive(Serialize, Deserialize)]
struct LlamaChoice {
    message: LlamaMessage,
}

#[derive(Serialize, Deserialize)]
struct LlamaMessage {
    content: Option<String>,
    tool_calls: Option<Vec<LlamaToolCall>>,
}

#[derive(Serialize, Deserialize)]
struct LlamaToolCall {
    #[serde(default)]
    id: String,
    #[serde(rename = "type")]
    type_: String,
    function: LlamaFunctionCall,
}

#[derive(Serialize, Deserialize)]
struct LlamaFunctionCall {
    name: String,
    arguments: String,
}

impl CompletionModel for LlamaProvider {
    type Response = Vec<ModelChoice>;

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Vec<ModelChoice>>, CompletionError> {
        // Construct messages
        let mut messages: Vec<serde_json::Value> = Vec::new();

        let mut push_msg = |role: &str, content: String| {
            // Try to parse content as JSON (multimodal)
            let content_val =
                if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
                    json!(parsed)
                } else {
                    json!(content)
                };

            if role == "user" {
                if let Some(last) = messages.last_mut() {
                    if last["role"] == "user" {
                        // Only merge if BOTH are strings
                        if let Some(last_str) = last["content"].as_str() {
                            if let Some(curr_str) = content_val.as_str() {
                                let new_content = format!("{}\n\n{}", last_str, curr_str);
                                *last = json!({ "role": "user", "content": new_content });
                                return;
                            }
                        }
                    }
                }
            }
            messages.push(json!({ "role": role, "content": content_val }));
        };

        if let Some(preamble) = &request.preamble {
            push_msg("system", preamble.clone());
        }

        for msg in &request.chat_history {
            match msg.role.as_str() {
                "system" => {
                    push_msg("system", msg.content.clone());
                }
                "tool" => {
                    // Turn 3: Fake Assistant acknowledgement
                    push_msg(
                        "assistant",
                        "I have gathered the necessary information from the web.".to_string(),
                    );
                    // Turn 3.5: User Context Injection
                    push_msg(
                        "user",
                        format!(
                            "<function_results>\n{}\n</function_results>\n\n[INSTRUCTION]: Based entirely on the results above, please answer the user's question. Do not cite the 'system' or 'tool' directly, just give the answer.",
                            msg.content
                        ),
                    );
                }
                _ => {
                    push_msg(msg.role.as_str(), msg.content.clone());
                }
            }
        }

        push_msg("user", request.prompt);

        // Map Tools
        let tools: Vec<serde_json::Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    }
                })
            })
            .collect();

        let body = LlamaChatRequest {
            messages,
            model: "default".to_string(),
            stream: false,
            temperature: None,
            top_p: None,
            tools,
            stop: Some(vec![
                "<|im_start|>".to_string(),
                "<|im_end|>".to_string(),
                "<|endoftext|>".to_string(),
                "<|user|>".to_string(),
                "<|assistant|>".to_string(),
                "user\n".to_string(),
                "assistant\n".to_string(),
                "&lt;|im_start|&gt;".to_string(),
                "&lt;|im_end|&gt;".to_string(),
            ]),
        };

        let client = reqwest::Client::new();
        let url = format!("{}/chat/completions", self.base_url);

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(CompletionError::ProviderError(format!(
                "Server returned {}",
                resp.status()
            )));
        }

        let llama_resp: LlamaChatResponse = resp
            .json()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        let choice_obj = llama_resp
            .choices
            .first()
            .ok_or_else(|| CompletionError::ProviderError("No choices returned".into()))?;

        let mut choices = Vec::new();

        // 1. Text Content
        if let Some(content) = &choice_obj.message.content {
            if !content.is_empty() {
                choices.push(ModelChoice::Message(content.clone()));
            }
        }

        // 2. Tool Calls
        if let Some(tool_calls) = &choice_obj.message.tool_calls {
            for tc in tool_calls {
                let args_val: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                    .map_err(|e| {
                        CompletionError::ProviderError(format!("Failed to parse tool args: {}", e))
                    })?;

                choices.push(ModelChoice::ToolCall(
                    tc.function.name.clone(),
                    tc.id.clone(),
                    args_val,
                ));
            }
        }

        // Return first choice as strict choice
        let first = if let Some(first) = choices.first() {
            match first {
                ModelChoice::Message(s) => ModelChoice::Message(s.clone()),
                ModelChoice::ToolCall(n, i, v) => {
                    ModelChoice::ToolCall(n.clone(), i.clone(), v.clone())
                }
            }
        } else {
            ModelChoice::Message("".into())
        };

        Ok(CompletionResponse {
            choice: first,
            raw_response: choices,
        })
    }
}

// Streaming Implementation
impl LlamaProvider {
    pub async fn stream_completion(
        &self,
        prompt: String,
        history: Vec<rig::completion::Message>,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = Result<String, String>> + Send>>, String>
    {
        use eventsource_stream::Eventsource;
        use futures::StreamExt;

        let client = reqwest::Client::new();
        let url = format!("{}/chat/completions", self.base_url);

        let mut messages: Vec<serde_json::Value> = Vec::new();

        let mut push_msg = |role: &str, content: String| {
            // Try to parse content as JSON (multimodal)
            let content_val =
                if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
                    json!(parsed)
                } else {
                    json!(content)
                };

            if role == "user" {
                if let Some(last) = messages.last_mut() {
                    if last["role"] == "user" {
                        // Only merge if BOTH are strings
                        if let Some(last_str) = last["content"].as_str() {
                            if let Some(curr_str) = content_val.as_str() {
                                let new_content = format!("{}\n\n{}", last_str, curr_str);
                                *last = json!({ "role": "user", "content": new_content });
                                return;
                            }
                        }
                    }
                }
            }
            messages.push(json!({ "role": role, "content": content_val }));
        };

        for msg in history {
            match msg.role.as_str() {
                "system" => {
                    push_msg("user", format!("[SYSTEM INSTRUCTIONS]\n{}", msg.content));
                }
                "tool" => {
                    // Turn 3: Fake Assistant acknowledgement
                    push_msg(
                        "assistant",
                        "I have gathered the necessary information from the web.".to_string(),
                    );
                    // Turn 3.5: User Context Injection
                    push_msg(
                        "user",
                        format!(
                            "<function_results>\n{}\n</function_results>\n\n[INSTRUCTION]: Based entirely on the results above, please answer the user's question. Do not cite the 'system' or 'tool' directly, just give the answer.",
                            msg.content
                        ),
                    );
                }
                _ => {
                    push_msg(msg.role.as_str(), msg.content);
                }
            }
        }
        push_msg("user", prompt);

        let body = LlamaChatRequest {
            messages,
            model: self.model.clone(),
            stream: true,
            temperature: None,
            top_p: None,
            tools: vec![],
            stop: Some(vec![
                "<|im_start|>".to_string(),
                "<|im_end|>".to_string(),
                "<|endoftext|>".to_string(),
                "<|user|>".to_string(),
                "<|assistant|>".to_string(),
                "user\n".to_string(),
                "assistant\n".to_string(),
                "&lt;|im_start|&gt;".to_string(),
                "&lt;|im_end|&gt;".to_string(),
            ]),
        };

        let stream = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .bytes_stream()
            .eventsource();

        Ok(stream
            .map(|event| {
                match event {
                    Ok(evt) => {
                        if evt.data == "[DONE]" {
                            return Ok("".to_string());
                        }
                        // Parse chunk
                        match serde_json::from_str::<serde_json::Value>(&evt.data) {
                            Ok(json) => {
                                if let Some(content) =
                                    json["choices"][0]["delta"]["content"].as_str()
                                {
                                    if !content.is_empty() {
                                        Ok(content.to_string())
                                    } else {
                                        Ok("".to_string())
                                    }
                                } else {
                                    Ok("".to_string())
                                }
                            }
                            Err(_) => Ok("".to_string()),
                        }
                    }
                    Err(e) => Err(e.to_string()),
                }
            })
            .boxed())
    }

    pub async fn count_tokens(&self, messages: Vec<serde_json::Value>) -> Result<u32, String> {
        let client = reqwest::Client::new();
        // Handle llama-server quirk: /tokenize is at root, but base_url usually ends in /v1 for chat
        let url = if self.base_url.ends_with("/v1") {
            format!("{}/tokenize", self.base_url.trim_end_matches("/v1"))
        } else {
            format!("{}/tokenize", self.base_url)
        };

        // Pre-parse content strings for potential JSON/Image arrays
        // Note: The /tokenize endpoint might expect raw text or a specific format.
        // Usually, llama.cpp server /tokenize expects {"content": "text"}.
        // But for chat history, we might need to serialize the whole thing.
        // Actually, the standard llama.cpp server endpoint /tokenize expects `content`.
        // If we want to count tokens for a whole chat, we rely on the fact that /tokenize handles one string.
        // We will approximate by joining messages or assume strict format.
        // Optimization: For now, we serialize messages to a string as a close-enough approximation for check
        // OR better: iterate and sum.
        // But the best is if the server supports `/extras/tokenize` or similar.
        // Standard OAI compat doesn't have a count endpoint.
        // llama.cpp has POST /tokenize with json body { content: "..." }.

        let mut total_tokens = 0;
        for msg in messages {
            let content_str = match msg["content"].clone() {
                serde_json::Value::String(s) => s,
                serde_json::Value::Array(arr) => {
                    // Extract text parts
                    arr.iter()
                        .filter_map(|v| v["text"].as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                }
                _ => "".to_string(),
            };

            // Add some overhead for role wrappers
            total_tokens += 4;

            if !content_str.is_empty() {
                let res = client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .json(&json!({ "content": content_str }))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;

                if res.status().is_success() {
                    let body: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
                    if let Some(tokens) = body["tokens"].as_array() {
                        total_tokens += tokens.len() as u32;
                    }
                }
            }
        }

        Ok(total_tokens)
    }

    pub async fn stream_raw_completion(
        &self,
        messages: Vec<serde_json::Value>,
        temperature: Option<f64>,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<ProviderEvent, String>> + Send>>,
        String,
    > {
        use eventsource_stream::Eventsource;
        use futures::StreamExt;

        let client = reqwest::Client::new();
        let url = format!("{}/chat/completions", self.base_url);

        let mut final_messages = Vec::new();
        for msg in messages {
            let mut m = msg.clone();
            if let Some(content_str) = m["content"].as_str() {
                if content_str.trim().starts_with('[') {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content_str) {
                        if parsed.is_array() {
                            m["content"] = parsed;
                        }
                    }
                }
            }
            final_messages.push(m);
        }

        let body = LlamaChatRequest {
            messages: final_messages,
            model: self.model.clone(),
            stream: true,
            temperature,
            top_p: None,
            tools: vec![],
            stop: Some(vec![
                "<|im_start|>".to_string(),
                "<|im_end|>".to_string(),
                "<|endoftext|>".to_string(),
                "<|user|>".to_string(),
                "<|assistant|>".to_string(),
                "<|end_of_text|>".to_string(),
                "<|eot_id|>".to_string(),
                "user\n".to_string(),
                "assistant\n".to_string(),
                "&lt;|im_start|&gt;".to_string(),
                "&lt;|im_end|&gt;".to_string(),
                "&lt;|user|&gt;".to_string(),
                "&lt;|assistant|&gt;".to_string(),
            ]),
        };

        let stream = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .bytes_stream()
            .eventsource();

        Ok(stream
            .map(|event| match event {
                Ok(evt) => {
                    if evt.data == "[DONE]" {
                        // End of stream
                        return Ok(ProviderEvent::Content("".into()));
                    }
                    match serde_json::from_str::<serde_json::Value>(&evt.data) {
                        Ok(json) => {
                            // Check for usage
                            if let Some(usage) = json.get("usage") {
                                if let (Some(p), Some(c), Some(t)) = (
                                    usage["prompt_tokens"].as_u64(),
                                    usage["completion_tokens"].as_u64(),
                                    usage["total_tokens"].as_u64(),
                                ) {
                                    return Ok(ProviderEvent::Usage(crate::chat::TokenUsage {
                                        prompt_tokens: p as u32,
                                        completion_tokens: c as u32,
                                        total_tokens: t as u32,
                                    }));
                                }
                            }

                            if let Some(content) = json["choices"][0]["delta"]["content"].as_str() {
                                if !content.is_empty() {
                                    Ok(ProviderEvent::Content(content.to_string()))
                                } else {
                                    Ok(ProviderEvent::Content("".to_string()))
                                }
                            } else {
                                Ok(ProviderEvent::Content("".to_string()))
                            }
                        }
                        Err(_) => Ok(ProviderEvent::Content("".to_string())),
                    }
                }
                Err(e) => Err(e.to_string()),
            })
            .boxed())
    }
}

use crate::rig_lib::unified_provider::ProviderEvent;
