use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionResponse, ModelChoice,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;

#[derive(Clone)]
pub struct LlamaProvider {
    base_url: String,
    api_key: String,
    model: String,
    model_family: String,
    is_local: bool,
    client: Option<reqwest::Client>,
    streaming_client: Option<reqwest::Client>,
    configuration_error: Option<String>,
}

impl LlamaProvider {
    pub fn new(base_url: &str, api_key: &str, model: &str, model_family: &str) -> Self {
        let configuration = Self::validate_configuration(base_url, api_key, model);
        let (base_url, is_local, mut configuration_error) = match configuration {
            Ok(configuration) => (configuration.0, configuration.1, None),
            Err(error) => (String::new(), false, Some(error)),
        };
        let client = if configuration_error.is_none() {
            match crate::rig_lib::http::client(is_local, false) {
                Ok(client) => Some(client),
                Err(error) => {
                    configuration_error = Some(error);
                    None
                }
            }
        } else {
            None
        };
        let streaming_client = if configuration_error.is_none() {
            match crate::rig_lib::http::client(is_local, true) {
                Ok(client) => Some(client),
                Err(error) => {
                    configuration_error = Some(error);
                    None
                }
            }
        } else {
            None
        };
        Self {
            base_url,
            api_key: api_key.to_string(),
            model: model.to_string(),
            model_family: model_family.to_string(),
            is_local,
            client,
            streaming_client,
            configuration_error,
        }
    }

    fn validate_configuration(
        base_url: &str,
        api_key: &str,
        model: &str,
    ) -> Result<(String, bool), String> {
        if base_url.is_empty() || base_url.len() > 4_096 || base_url.chars().any(char::is_control) {
            return Err("The LLM endpoint is missing or invalid".to_string());
        }
        let url = reqwest::Url::parse(base_url)
            .map_err(|_| "The LLM endpoint is not a valid URL".to_string())?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "The LLM endpoint must not contain credentials, a query, or a fragment".into(),
            );
        }
        let host = url
            .host_str()
            .ok_or_else(|| "The LLM endpoint has no host".to_string())?;
        let is_local = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if (is_local && !matches!(url.scheme(), "http" | "https"))
            || (!is_local && url.scheme() != "https")
        {
            return Err(
                "Remote LLM endpoints must use HTTPS; local endpoints must use HTTP(S)".into(),
            );
        }
        if api_key.trim() != api_key
            || api_key.len() > 16 * 1024
            || api_key.chars().any(char::is_control)
            || (!is_local && api_key.is_empty())
        {
            return Err("The LLM credential is missing or invalid".to_string());
        }
        if model.is_empty() || model.len() > 512 || model.chars().any(char::is_control) {
            return Err("The LLM model identifier is missing or invalid".to_string());
        }
        Ok((base_url.trim_end_matches('/').to_string(), is_local))
    }

    fn client(&self, streaming: bool) -> Result<&reqwest::Client, String> {
        if let Some(error) = &self.configuration_error {
            return Err(error.clone());
        }
        if streaming {
            self.streaming_client
                .as_ref()
                .ok_or_else(|| "The streaming LLM client is unavailable".to_string())
        } else {
            self.client
                .as_ref()
                .ok_or_else(|| "The LLM client is unavailable".to_string())
        }
    }

    fn endpoint(&self, path: &str) -> Result<String, String> {
        self.client(false)?;
        Ok(format!(
            "{}/{}",
            self.base_url,
            path.trim_start_matches('/')
        ))
    }

    fn is_reasoning_model(&self) -> bool {
        let m = self.model.to_lowercase();
        // OpenAI o1/o3 and models with gpt-5 in the name (often used as placeholders for latest)
        // do not support temperature values other than 1.
        m.starts_with("o1-") || m.starts_with("o3-") || m == "o1" || m.contains("gpt-5")
    }
}

/// Sanitize system prompt content for local models.
/// Some local models (especially abliterated/uncensored variants) produce very short
/// or empty outputs when the system prompt contains:
/// - HTML-like tags (e.g. `<think>`, `<tool_result>`) which clash with Gemma's `<start_of_turn>` markers
/// - Complex negative instructions ("Do NOT do X") which overwhelm smaller models
/// - Overly long/complex multi-part instructions
///
/// This function cleans up the system prompt while preserving its core meaning.
fn sanitize_system_prompt_for_local(content: &str) -> String {
    let mut result = content.to_string();

    // 1. Strip angle-bracketed tags that could confuse the model's template parser.
    //    Models like Gemma use <start_of_turn>/<end_of_turn> as special tokens,
    //    so seeing other <...> patterns in content can cause premature EOS.
    //    We preserve the text content but remove the angle brackets.
    static TAG_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let tag_re = TAG_RE
        .get_or_init(|| regex::Regex::new(r"<(/?\w[\w_-]*)>").expect("valid static tag regex"));
    result = tag_re.replace_all(&result, "`$1`").to_string();

    // 2. Simplify negative instructions that cause small models to "shut down".
    //    "Do NOT output X" gets interpreted by degraded models as "do not output".
    //    Replace with positive framing that achieves the same goal.
    result = result.replace(
        "Do NOT output internal thoughts, `think` tags, or simulate tool usage.",
        "Respond directly and concisely.",
    );
    result = result.replace(
        "Do NOT output internal thoughts, <think> tags, or simulate tool usage.",
        "Respond directly and concisely.",
    );

    // 3. Clean up double spaces and excessive punctuation from replacements
    static MULTI_SPACE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let multi_space_re =
        MULTI_SPACE_RE.get_or_init(|| regex::Regex::new(r"  +").expect("valid static space regex"));
    result = multi_space_re.replace_all(&result, " ").to_string();
    static MULTI_PERIOD_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let multi_period_re = MULTI_PERIOD_RE
        .get_or_init(|| regex::Regex::new(r"\.(\s*\.)+").expect("valid static period regex"));
    result = multi_period_re.replace_all(&result, ".").to_string();

    result.trim().to_string()
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
        if request
            .temperature
            .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
        {
            return Err(CompletionError::ProviderError(
                "LLM temperature is outside the supported range".into(),
            ));
        }
        let temperature = if self.is_reasoning_model() {
            None
        } else {
            request.temperature
        };
        // Construct messages
        let mut messages: Vec<serde_json::Value> = Vec::new();
        let is_local = self.is_local;

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
            model: self.model.clone(),
            stream: false,
            temperature,
            top_p: None,
            tools,
            // Per-model-family stop tokens: prevents ChatML models (Ministral, Qwen)
            // from hallucinating turns, while leaving Gemma and others to use native EOS.
            stop: if is_local {
                Some(crate::gguf::stop_tokens_for_family(&self.model_family))
            } else {
                None
            },
        };

        let client = self.client(false).map_err(CompletionError::ProviderError)?;
        let url = self
            .endpoint("chat/completions")
            .map_err(CompletionError::ProviderError)?;
        let body = crate::rig_lib::http::bounded_json_body(&body)
            .map_err(CompletionError::ProviderError)?;

        let mut request_builder = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);

        if self.base_url.contains("openrouter.ai") {
            request_builder = request_builder
                .header("HTTP-Referer", "https://github.com/RNT56/ThinClaw")
                .header("X-Title", "ThinClaw Desktop");
        }

        let resp = request_builder.send().await.map_err(|error| {
            CompletionError::ProviderError(crate::rig_lib::http::transport_error(
                "LLM request failed",
                error,
            ))
        })?;
        let resp = crate::rig_lib::http::checked_response(resp, "LLM")
            .await
            .map_err(CompletionError::ProviderError)?;
        let llama_resp: LlamaChatResponse = crate::rig_lib::http::bounded_json(resp, "LLM")
            .await
            .map_err(CompletionError::ProviderError)?;

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
            if tool_calls.len() > 128 {
                return Err(CompletionError::ProviderError(
                    "LLM returned too many tool calls".into(),
                ));
            }
            for tc in tool_calls {
                if tc.type_ != "function"
                    || tc.function.name.is_empty()
                    || tc.function.name.len() > 256
                    || tc.function.name.chars().any(char::is_control)
                    || tc.id.is_empty()
                    || tc.id.len() > 512
                    || tc.id.chars().any(char::is_control)
                {
                    return Err(CompletionError::ProviderError(
                        "LLM returned a malformed tool call".into(),
                    ));
                }
                let args_val: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                    .map_err(|_| {
                        CompletionError::ProviderError(
                            "LLM returned malformed tool arguments".into(),
                        )
                    })?;
                if !args_val.is_object() {
                    return Err(CompletionError::ProviderError(
                        "LLM tool arguments must be a JSON object".into(),
                    ));
                }

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
            return Err(CompletionError::ProviderError(
                "LLM returned an empty response".into(),
            ));
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

        let client = self.client(true)?;
        let url = self.endpoint("chat/completions")?;

        let mut messages: Vec<serde_json::Value> = Vec::new();
        let is_local = self.is_local;

        let mut push_msg = |role: &str, content: String| {
            // For gemma family, sanitize system prompt content to avoid issues
            // with abliterated models that produce empty/short outputs.
            let effective_content = if is_local && role == "system" && self.model_family == "gemma"
            {
                sanitize_system_prompt_for_local(&content)
            } else {
                content
            };

            // Try to parse content as JSON (multimodal)
            let content_val = if let Ok(parsed) =
                serde_json::from_str::<Vec<serde_json::Value>>(&effective_content)
            {
                json!(parsed)
            } else {
                json!(effective_content)
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
                    // Pass system messages through as-is.
                    // Modern models (Gemma 3, Qwen, DeepSeek) support system role natively
                    // via their GGUF templates. Remapping to user created broken double-user turns.
                    push_msg("system", msg.content);
                }
                "tool" => {
                    // Turn 3: Fake Assistant acknowledgement
                    push_msg(
                        "assistant",
                        "I have gathered the necessary information from the web.".to_string(),
                    );
                    // Turn 3.5: User Context Injection
                    // Use backtick-delimited markers instead of angle-bracket tags.
                    // Angle brackets like <function_results> confuse models that use
                    // angle-bracket special tokens (e.g. Gemma's <start_of_turn>).
                    push_msg(
                        "user",
                        format!(
                            "```function_results\n{}\n```\n\nBased entirely on the results above, please answer the user's question directly.",
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
            messages: messages.clone(),
            model: self.model.clone(),
            stream: true,
            temperature: None,
            top_p: None,
            tools: vec![],
            // Per-model-family stop tokens: prevents ChatML models (Ministral, Qwen)
            // from hallucinating turns, while leaving Gemma and others to use native EOS.
            stop: if is_local {
                Some(crate::gguf::stop_tokens_for_family(&self.model_family))
            } else {
                None
            },
        };

        let body = crate::rig_lib::http::bounded_json_body(&body)?;
        let mut request_builder = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);

        if self.base_url.contains("openrouter.ai") {
            request_builder = request_builder
                .header("HTTP-Referer", "https://github.com/RNT56/ThinClaw")
                .header("X-Title", "ThinClaw Desktop");
        }

        let response = request_builder.send().await.map_err(|error| {
            crate::rig_lib::http::transport_error("LLM connection failed", error)
        })?;
        let response = crate::rig_lib::http::checked_response(response, "LLM").await?;

        let stream = crate::rig_lib::http::bounded_sse_bytes(response).eventsource();

        Ok(stream
            .map(|event| {
                match event {
                    Ok(evt) => {
                        if evt.data == "[DONE]" {
                            return Ok("".to_string());
                        }
                        if evt.data.len() > 2 * 1024 * 1024 {
                            return Err("LLM event exceeds the supported size".to_string());
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
                            Err(_) => Err("LLM stream returned invalid JSON".to_string()),
                        }
                    }
                    Err(e) => Err(e.to_string()),
                }
            })
            .boxed())
    }

    pub async fn count_tokens(&self, messages: Vec<serde_json::Value>) -> Result<u32, String> {
        if messages.len() > 512 {
            return Err("Tokenization input contains too many messages".to_string());
        }
        let client = self.client(false)?;

        let url = if self.base_url.ends_with("/v1") {
            format!("{}/tokenize", self.base_url.trim_end_matches("/v1"))
        } else {
            format!("{}/tokenize", self.base_url)
        };

        let mut total_tokens = 0_u32;
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
            if content_str.len() > 2 * 1024 * 1024 {
                return Err("Tokenization input message exceeds the size limit".to_string());
            }
            total_tokens = total_tokens.saturating_add(4);

            if !content_str.is_empty() {
                // Approximate for cloud providers (no /tokenize)
                if !self.is_local {
                    let estimate =
                        u32::try_from(content_str.len().saturating_add(2) / 3).unwrap_or(u32::MAX);
                    total_tokens = total_tokens.saturating_add(estimate);
                    continue;
                }

                let body =
                    crate::rig_lib::http::bounded_json_body(&json!({ "content": content_str }))?;
                let res = client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body)
                    .timeout(std::time::Duration::from_secs(3))
                    .send()
                    .await;

                match res {
                    Ok(r) if r.status().is_success() => {
                        let token_count =
                            crate::rig_lib::http::bounded_json::<serde_json::Value>(r, "tokenizer")
                                .await
                                .ok()
                                .and_then(|body| body["tokens"].as_array().map(Vec::len))
                                .and_then(|count| u32::try_from(count).ok());
                        let count = token_count.unwrap_or_else(|| {
                            u32::try_from(content_str.len().saturating_add(2) / 3)
                                .unwrap_or(u32::MAX)
                        });
                        total_tokens = total_tokens.saturating_add(count);
                    }
                    _ => {
                        // Fallback to char count on error/timeout
                        let estimate = u32::try_from(content_str.len().saturating_add(2) / 3)
                            .unwrap_or(u32::MAX);
                        total_tokens = total_tokens.saturating_add(estimate);
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

        let client = self.client(true)?;
        let url = self.endpoint("chat/completions")?;

        let mut final_messages = Vec::new();
        let is_local = self.is_local;

        for msg in messages {
            let mut m = msg.clone();

            // For gemma family, sanitize system prompt content to avoid issues
            // with abliterated models that produce empty/short outputs.
            if is_local && self.model_family == "gemma" {
                if let Some(role) = m["role"].as_str() {
                    if role == "system" {
                        if let Some(content_str) = m["content"].as_str() {
                            m["content"] = serde_json::Value::String(
                                sanitize_system_prompt_for_local(content_str),
                            );
                        }
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

        // Enforce strict user/assistant alternation by merging consecutive same-role
        // messages. Models like Mistral have chat templates that throw exceptions on
        // consecutive user or assistant messages. This is a safety net that handles
        // any upstream conversation structure issues (e.g. tool_result as user +
        // synthesis instruction as user, or history ending with user + effective_query).
        let mut merged_messages: Vec<serde_json::Value> = Vec::new();
        for msg in final_messages {
            let role = msg["role"].as_str().unwrap_or("").to_string();
            if role != "system" {
                if let Some(last) = merged_messages.last_mut() {
                    if last["role"].as_str() == Some(&role) {
                        // Merge: append content to previous message of same role
                        if let (Some(existing), Some(new)) =
                            (last["content"].as_str(), msg["content"].as_str())
                        {
                            last["content"] =
                                serde_json::Value::String(format!("{}\n\n{}", existing, new));
                            info!(
                                "[llama_provider] Merged consecutive '{}' messages to enforce alternation",
                                role
                            );
                            continue;
                        }
                    }
                }
            }
            merged_messages.push(msg);
        }
        let final_messages = merged_messages;

        if temperature.is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value)) {
            return Err("LLM temperature is outside the supported range".to_string());
        }
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
            // Per-model-family stop tokens: prevents ChatML models (Ministral, Qwen)
            // from hallucinating turns, while leaving Gemma and others to use native EOS.
            stop: if is_local {
                Some(crate::gguf::stop_tokens_for_family(&self.model_family))
            } else {
                None
            },
        };

        info!(
            model = %self.model,
            message_count = body.messages.len(),
            "[llama_provider] sending bounded request"
        );
        let body = crate::rig_lib::http::bounded_json_body(&body)?;
        let mut request_builder = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);

        if self.base_url.contains("openrouter.ai") {
            request_builder = request_builder
                .header("HTTP-Referer", "https://github.com/RNT56/ThinClaw")
                .header("X-Title", "ThinClaw Desktop");
        }

        let response = request_builder
            .send()
            .await
            .map_err(|error| crate::rig_lib::http::transport_error("LLM request failed", error))?;
        let response = crate::rig_lib::http::checked_response(response, "LLM").await?;

        let stream = crate::rig_lib::http::bounded_sse_bytes(response).eventsource();

        // Track how many events we've seen for debugging
        let event_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let event_count_clone = event_count.clone();

        Ok(stream
            .map(move |event| {
                let ev_num = event_count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                match event {
                    Ok(evt) => {
                        if evt.data == "[DONE]" {
                            tracing::debug!(events = ev_num, "[llama_provider] stream completed");
                            return Ok(ProviderEvent::Content("".into()));
                        }
                        if evt.data.len() > 2 * 1024 * 1024 {
                            return Err("LLM event exceeds the supported size".to_string());
                        }
                        match serde_json::from_str::<serde_json::Value>(&evt.data) {
                            Ok(json) => {
                                // Check for server error objects (MLX/vLLM may return these)
                                if let Some(err) = json.get("error").or(json.get("detail")) {
                                    let error_kind =
                                        if err.is_string() { "message" } else { "object" };
                                    return Err(format!(
                                        "LLM server returned an error {error_kind}"
                                    ));
                                }

                                // Check for usage
                                if let Some(usage) = json.get("usage") {
                                    if let (Some(p), Some(c), Some(t)) = (
                                        usage["prompt_tokens"].as_u64(),
                                        usage["completion_tokens"].as_u64(),
                                        usage["total_tokens"].as_u64(),
                                    ) {
                                        return Ok(ProviderEvent::Usage(crate::chat::TokenUsage {
                                            prompt_tokens: u32::try_from(p).unwrap_or(u32::MAX),
                                            completion_tokens: u32::try_from(c).unwrap_or(u32::MAX),
                                            total_tokens: u32::try_from(t).unwrap_or(u32::MAX),
                                        }));
                                    }
                                }

                                if let Some(content) =
                                    json["choices"][0]["delta"]["content"].as_str()
                                {
                                    if !content.is_empty() {
                                        Ok(ProviderEvent::Content(content.to_string()))
                                    } else {
                                        Ok(ProviderEvent::Content("".to_string()))
                                    }
                                } else {
                                    Ok(ProviderEvent::Content("".to_string()))
                                }
                            }
                            Err(_) => Err("LLM stream returned invalid JSON".to_string()),
                        }
                    }
                    Err(e) => Err(e.to_string()),
                }
            })
            .boxed())
    }
}

use crate::rig_lib::unified_provider::ProviderEvent;
