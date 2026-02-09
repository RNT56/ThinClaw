use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionResponse, ModelChoice,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

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

    fn is_reasoning_model(&self) -> bool {
        let m = self.model.to_lowercase();
        // OpenAI o1/o3 and models with gpt-5 in the name (often used as placeholders for latest)
        // do not support temperature values other than 1.
        m.starts_with("o1-") || m.starts_with("o3-") || m == "o1" || m.contains("gpt-5")
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

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;
        let url = format!("{}/chat/completions", self.base_url);

        let mut request_builder = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body);

        if self.base_url.contains("openrouter.ai") {
            request_builder = request_builder
                .header("HTTP-Referer", "https://github.com/scrappy-ai/scrappy")
                .header("X-Title", "Scrappy AI Desktop");
        }

        let resp = request_builder.send().await.map_err(|e| {
            CompletionError::ProviderError(format!(
                "Network Error: {}. Check your connection and API key.",
                e
            ))
        })?;

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

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
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
                    // Use "system" role if not local
                    let target_role = if self.base_url.contains("127.0.0.1")
                        || self.base_url.contains("localhost")
                    {
                        "user"
                    } else {
                        "system"
                    };

                    if target_role == "user" {
                        push_msg("user", format!("[SYSTEM INSTRUCTIONS]\n{}", msg.content));
                    } else {
                        push_msg("system", msg.content);
                    }
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

        // For Cloud models (OpenAI/Anthropic compat), we should be careful with stop sequences
        let is_local = self.base_url.contains("127.0.0.1") || self.base_url.contains("localhost");

        let body = LlamaChatRequest {
            messages: messages.clone(),
            model: self.model.clone(),
            stream: true,
            temperature: None,
            top_p: None,
            tools: vec![],
            stop: if is_local {
                Some(vec![
                    "<|im_start|>".to_string(),
                    "<|im_end|>".to_string(),
                    "<|endoftext|>".to_string(),
                    "<|user|>".to_string(),
                    "<|assistant|>".to_string(),
                    "user\n".to_string(),
                    "assistant\n".to_string(),
                    "&lt;|im_start|&gt;".to_string(),
                    "&lt;|im_end|&gt;".to_string(),
                ])
            } else {
                None
            },
        };

        let mut request_builder = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body);

        if self.base_url.contains("openrouter.ai") {
            request_builder = request_builder
                .header("HTTP-Referer", "https://github.com/scrappy-ai/scrappy")
                .header("X-Title", "Scrappy AI Desktop");
        }

        let response = request_builder
            .send()
            .await
            .map_err(|e| format!("Connection Failed: {}. Check your internet or API key.", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            return Err(format!("API Error {}: {}", status, err_text));
        }

        let stream = response.bytes_stream().eventsource();

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
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3)) // Very short timeout for tokenization
            .build()
            .map_err(|e| e.to_string())?;

        let url = if self.base_url.ends_with("/v1") {
            format!("{}/tokenize", self.base_url.trim_end_matches("/v1"))
        } else {
            format!("{}/tokenize", self.base_url)
        };

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
                // Approximate for cloud providers (no /tokenize)
                if !self.base_url.contains("127.0.0.1") && !self.base_url.contains("localhost") {
                    total_tokens += (content_str.len() / 3) as u32;
                    continue;
                }

                let res = client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .json(&json!({ "content": content_str }))
                    .send()
                    .await;

                match res {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(body) = r.json::<serde_json::Value>().await {
                            if let Some(tokens) = body["tokens"].as_array() {
                                total_tokens += tokens.len() as u32;
                            }
                        }
                    }
                    _ => {
                        // Fallback to char count on error/timeout
                        total_tokens += (content_str.len() / 3) as u32;
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

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(45))
            .build()
            .map_err(|e| e.to_string())?;
        let url = format!("{}/chat/completions", self.base_url);

        let mut final_messages = Vec::new();
        for msg in messages {
            let mut m = msg.clone();
            // Map "system" role if cloud
            if !self.base_url.contains("127.0.0.1") && !self.base_url.contains("localhost") {
                if m["role"] == "system" {
                    m["role"] = json!("system");
                }
            } else {
                if m["role"] == "system" {
                    m["role"] = json!("user");
                    if let Some(c) = m["content"].as_str() {
                        m["content"] = json!(format!("[SYSTEM INSTRUCTIONS]\n{}", c));
                    }
                }
            }

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

        let is_local = self.base_url.contains("127.0.0.1") || self.base_url.contains("localhost");

        let effective_temp = if self.is_reasoning_model() {
            None
        } else {
            temperature
        };

        let body = LlamaChatRequest {
            messages: final_messages,
            model: self.model.clone(),
            stream: true,
            temperature: effective_temp,
            top_p: None,
            tools: vec![],
            stop: if is_local {
                Some(vec![
                    "<|im_start|>".to_string(),
                    "<|im_end|>".to_string(),
                    "<|endoftext|>".to_string(),
                    "<|user|>".to_string(),
                    "<|assistant|>".to_string(),
                    "<|end_of_text|>".to_string(),
                    "<|eot_id|>".to_string(),
                    "user\n".to_string(),
                    "assistant\n".to_string(),
                ])
            } else {
                None
            },
        };

        info!(
            "[llama_provider] Sending request to model: {} at url: {}",
            self.model, url
        );
        let mut request_builder = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body);

        if self.base_url.contains("openrouter.ai") {
            request_builder = request_builder
                .header("HTTP-Referer", "https://github.com/scrappy-ai/scrappy")
                .header("X-Title", "Scrappy AI Desktop");
        }

        let response = request_builder.send().await.map_err(|e| {
            error!("[llama_provider] Network error: {}", e);
            format!(
                "Request failed: {}. Check model name '{}' and API key.",
                e, self.model
            )
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            error!("[llama_provider] API Error ({}): {}", status, err_text);
            return Err(format!("OpenAI API Error ({}): {}", status, err_text));
        }

        let stream = response.bytes_stream().eventsource();

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
