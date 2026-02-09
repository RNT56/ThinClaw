use eventsource_stream::Eventsource;
use futures::StreamExt;
use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionResponse, ModelChoice,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{error, info};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ProviderKind {
    OpenAI,
    Anthropic,
    Gemini,
    Groq,  // Usually OpenAI compatible
    Local, // Usually OpenAI compatible
    OpenRouter,
}

#[derive(Clone)]
pub struct UnifiedProvider {
    pub kind: ProviderKind,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl UnifiedProvider {
    pub fn new(kind: ProviderKind, base_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            kind,
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
        }
    }

    fn is_reasoning_model(&self) -> bool {
        let m = self.model.to_lowercase();
        m.starts_with("o1-") || m.starts_with("o3-") || m == "o1" || m.contains("gpt-5")
    }

    fn sanitize_temperature(&self, temp: f64) -> Option<f64> {
        if self.is_reasoning_model() {
            None
        } else {
            Some(temp)
        }
    }

    fn clone_model_choice(choice: &ModelChoice) -> ModelChoice {
        match choice {
            ModelChoice::Message(s) => ModelChoice::Message(s.clone()),
            ModelChoice::ToolCall(n, i, a) => {
                ModelChoice::ToolCall(n.clone(), i.clone(), a.clone())
            }
        }
    }

    async fn completion_openai(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Vec<ModelChoice>>, CompletionError> {
        let mut messages: Vec<Value> = Vec::new();
        if let Some(p) = request.preamble {
            messages.push(json!({ "role": "system", "content": p }));
        }
        for msg in request.chat_history {
            messages.push(json!({ "role": msg.role, "content": msg.content }));
        }
        messages.push(json!({ "role": "user", "content": request.prompt }));

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": false,
            "tools": request.tools.iter().map(|t| json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters
                }
            })).collect::<Vec<_>>()
        });

        if let Some(t) = self.sanitize_temperature(0.7) {
            body.as_object_mut()
                .unwrap()
                .insert("temperature".into(), json!(t));
        }

        let client = reqwest::Client::new();
        let url = format!("{}/chat/completions", self.base_url);

        let mut request_builder = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body);

        if matches!(self.kind, ProviderKind::OpenRouter) {
            request_builder = request_builder
                .header("HTTP-Referer", "https://github.com/scrappy-ai/scrappy")
                .header("X-Title", "Scrappy AI Desktop");
        }

        let resp = request_builder
            .send()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            return Err(CompletionError::ProviderError(format!(
                "OpenAI Error: {} - {}",
                status, err_text
            )));
        }

        let json: Value = resp
            .json()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        let choice = &json["choices"][0]["message"];
        let mut model_choices = Vec::new();

        if let Some(content) = choice["content"].as_str() {
            model_choices.push(ModelChoice::Message(content.to_string()));
        }

        if let Some(tool_calls) = choice["tool_calls"].as_array() {
            for tc in tool_calls {
                let name = tc["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let id = tc["id"].as_str().unwrap_or_default().to_string();
                let args =
                    serde_json::from_str(tc["function"]["arguments"].as_str().unwrap_or("{}"))
                        .unwrap_or(json!({}));
                model_choices.push(ModelChoice::ToolCall(name, id, args));
            }
        }

        let first = if let Some(first) = model_choices.first() {
            Self::clone_model_choice(first)
        } else {
            ModelChoice::Message("".into())
        };

        Ok(CompletionResponse {
            choice: first,
            raw_response: model_choices,
        })
    }

    async fn completion_anthropic(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Vec<ModelChoice>>, CompletionError> {
        let mut messages: Vec<Value> = Vec::new();
        let mut system = None;
        if let Some(p) = request.preamble {
            system = Some(p);
        }
        for msg in request.chat_history {
            if msg.role == "system" {
                system = Some(msg.content);
            } else {
                messages.push(json!({ "role": msg.role, "content": msg.content }));
            }
        }
        messages.push(json!({ "role": "user", "content": request.prompt }));

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": 4096,
        });

        if let Some(s) = system {
            body.as_object_mut()
                .unwrap()
                .insert("system".into(), json!(s));
        }

        if !request.tools.is_empty() {
            body.as_object_mut().unwrap().insert(
                "tools".into(),
                json!(request
                    .tools
                    .iter()
                    .map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.parameters
                    }))
                    .collect::<Vec<_>>()),
            );
        }

        let client = reqwest::Client::new();
        let url = format!("{}/messages", self.base_url);

        let resp = client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            return Err(CompletionError::ProviderError(format!(
                "Anthropic Error: {} - {}",
                status, err_text
            )));
        }

        let json: Value = resp
            .json()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        let mut model_choices = Vec::new();
        if let Some(content_array) = json["content"].as_array() {
            for item in content_array {
                match item["type"].as_str() {
                    Some("text") => {
                        model_choices.push(ModelChoice::Message(
                            item["text"].as_str().unwrap_or_default().to_string(),
                        ));
                    }
                    Some("tool_use") => {
                        let name = item["name"].as_str().unwrap_or_default().to_string();
                        let id = item["id"].as_str().unwrap_or_default().to_string();
                        let input = item["input"].clone();
                        model_choices.push(ModelChoice::ToolCall(name, id, input));
                    }
                    _ => {}
                }
            }
        }

        let first = if let Some(first) = model_choices.first() {
            Self::clone_model_choice(first)
        } else {
            ModelChoice::Message("".into())
        };

        Ok(CompletionResponse {
            choice: first,
            raw_response: model_choices,
        })
    }

    async fn completion_gemini(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Vec<ModelChoice>>, CompletionError> {
        let mut contents: Vec<Value> = Vec::new();

        if let Some(p) = request.preamble {
            contents.push(json!({ "role": "user", "parts": [{ "text": format!("System Instructions: {}\n\nUser Question: {}", p, request.prompt) }] }));
        } else {
            contents.push(json!({ "role": "user", "parts": [{ "text": request.prompt }] }));
        }

        let mut body = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": 0.7,
                "maxOutputTokens": 4096,
            }
        });

        if !request.tools.is_empty() {
            body.as_object_mut().unwrap().insert(
                "tools".into(),
                json!([{
                    "function_declarations": request.tools.iter().map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    })).collect::<Vec<_>>()
                }]),
            );
        }

        let client = reqwest::Client::new();
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            return Err(CompletionError::ProviderError(format!(
                "Gemini Error: {} - {}",
                status, err_text
            )));
        }

        let json: Value = resp
            .json()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        let mut model_choices = Vec::new();
        if let Some(candidates) = json["candidates"].as_array() {
            if let Some(candidate) = candidates.first() {
                if let Some(parts) = candidate["content"]["parts"].as_array() {
                    for part in parts {
                        if let Some(text) = part["text"].as_str() {
                            model_choices.push(ModelChoice::Message(text.to_string()));
                        } else if let Some(func_call) = part["functionCall"].as_object() {
                            let name = func_call["name"].as_str().unwrap_or_default().to_string();
                            let args = func_call["args"].clone();
                            model_choices.push(ModelChoice::ToolCall(
                                name,
                                "gemini-tool-id".to_string(),
                                args,
                            ));
                        }
                    }
                }
            }
        }

        let first = if let Some(first) = model_choices.first() {
            Self::clone_model_choice(first)
        } else {
            ModelChoice::Message("".into())
        };

        Ok(CompletionResponse {
            choice: first,
            raw_response: model_choices,
        })
    }
}

pub enum ProviderEvent {
    Content(String),
    Usage(crate::chat::TokenUsage),
    ContextUpdate(Vec<crate::chat::Message>),
}

impl CompletionModel for UnifiedProvider {
    type Response = Vec<ModelChoice>;

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Vec<ModelChoice>>, CompletionError> {
        match self.kind {
            ProviderKind::Anthropic => self.completion_anthropic(request).await,
            ProviderKind::Gemini => self.completion_gemini(request).await,
            _ => self.completion_openai(request).await,
        }
    }
}

impl UnifiedProvider {
    pub async fn count_tokens(&self, messages: Vec<Value>) -> Result<u32, String> {
        match self.kind {
            ProviderKind::Local => {
                let lp = crate::rig_lib::llama_provider::LlamaProvider::new(
                    &self.base_url,
                    &self.api_key,
                    &self.model,
                );
                lp.count_tokens(messages).await
            }
            _ => {
                let mut total_chars = 0;
                for msg in messages {
                    if let Some(content) = msg["content"].as_str() {
                        total_chars += content.len();
                    } else if let Some(content_array) = msg["content"].as_array() {
                        for part in content_array {
                            if let Some(text) = part["text"].as_str() {
                                total_chars += text.len();
                            }
                        }
                    }
                }
                Ok((total_chars / 3) as u32)
            }
        }
    }

    pub async fn stream_raw_completion(
        &self,
        messages: Vec<Value>,
        temperature: Option<f64>,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<ProviderEvent, String>> + Send>>,
        String,
    > {
        info!(
            "[unified_provider] stream_raw_completion for model: {}",
            self.model
        );
        match self.kind {
            ProviderKind::Local
            | ProviderKind::OpenAI
            | ProviderKind::Groq
            | ProviderKind::OpenRouter => {
                let lp = crate::rig_lib::llama_provider::LlamaProvider::new(
                    &self.base_url,
                    &self.api_key,
                    &self.model,
                );
                lp.stream_raw_completion(messages, temperature).await
            }
            ProviderKind::Anthropic => self.stream_anthropic(messages, temperature).await,
            ProviderKind::Gemini => self.stream_gemini(messages, temperature).await,
        }
    }

    async fn stream_anthropic(
        &self,
        messages: Vec<Value>,
        _temperature: Option<f64>,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<ProviderEvent, String>> + Send>>,
        String,
    > {
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": 4096,
            "stream": true,
        });

        if let Some(t) = _temperature.and_then(|temp| self.sanitize_temperature(temp)) {
            body.as_object_mut()
                .unwrap()
                .insert("temperature".into(), json!(t));
        }

        // Handle system message if present in the messages list (Anthropic expects it as a top-level field)
        let mut filtered_messages = Vec::new();
        for msg in messages {
            if msg["role"] == "system" {
                body.as_object_mut()
                    .unwrap()
                    .insert("system".into(), msg["content"].clone());
            } else {
                filtered_messages.push(msg);
            }
        }
        body["messages"] = json!(filtered_messages);

        let client = reqwest::Client::new();
        let url = format!("{}/messages", self.base_url);

        let response = client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            error!(
                "[unified_provider] Anthropic API Error ({}): {}",
                status, err_text
            );
            return Err(format!("Anthropic API Error ({}): {}", status, err_text));
        }

        let stream = response.bytes_stream().eventsource();
        let s = stream
            .map(|event_res| {
                let mut events = Vec::new();
                match event_res {
                    Ok(event) => {
                        let data = event.data;
                        if data == "[DONE]" {
                            return futures::stream::iter(events);
                        }

                        if let Ok(json) = serde_json::from_str::<Value>(&data) {
                            match json["type"].as_str() {
                                Some("content_block_delta") => {
                                    if let Some(delta) = json["delta"]["text"].as_str() {
                                        events.push(Ok(ProviderEvent::Content(delta.to_string())));
                                    }
                                }
                                Some("message_delta") => {
                                    if let Some(usage) = json["usage"].as_object() {
                                        events.push(Ok(ProviderEvent::Usage(
                                            crate::chat::TokenUsage {
                                                prompt_tokens: 0,
                                                completion_tokens: usage
                                                    .get("output_tokens")
                                                    .and_then(|v| v.as_u64())
                                                    .unwrap_or(0)
                                                    as u32,
                                                total_tokens: 0,
                                            },
                                        )));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        events.push(Err(e.to_string()));
                    }
                }
                futures::stream::iter(events)
            })
            .flatten();

        Ok(Box::pin(s))
    }

    async fn stream_gemini(
        &self,
        messages: Vec<Value>,
        _temperature: Option<f64>,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<ProviderEvent, String>> + Send>>,
        String,
    > {
        info!(
            "[unified_provider] Starting Gemini stream for model: {}",
            self.model
        );
        let mut contents: Vec<Value> = Vec::new();
        let mut system_instruction = None;

        for msg in messages {
            let role = msg["role"].as_str().unwrap_or("user");
            if role == "system" {
                system_instruction = Some(json!({ "parts": [{ "text": msg["content"] }] }));
                continue;
            }

            let gemini_role = if role == "assistant" { "model" } else { "user" };

            // Ensure we don't send consecutive identical roles (Gemini requirement)
            if let Some(last) = contents.last() {
                if last["role"] == gemini_role {
                    // Merge content if roles match
                    if let Some(last_parts) =
                        contents.last_mut().and_then(|m| m["parts"].as_array_mut())
                    {
                        last_parts.push(json!({ "text": format!("\n\n{}", msg["content"].as_str().unwrap_or_default()) }));
                        continue;
                    }
                }
            }

            contents.push(json!({
                "role": gemini_role,
                "parts": [{ "text": msg["content"] }]
            }));
        }

        // If history is empty but we have a user msg (not possible here due to how its called, but for safety)
        if contents.is_empty() {
            contents.push(json!({ "role": "user", "parts": [{ "text": "Hello" }] }));
        }

        let mut body = json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": 8192,
            }
        });

        if let Some(si) = system_instruction {
            body.as_object_mut()
                .unwrap()
                .insert("system_instruction".into(), si);
        }

        if let Some(t) = self.sanitize_temperature(0.7) {
            if let Some(obj) = body["generationConfig"].as_object_mut() {
                obj.insert("temperature".into(), json!(t));
            }
        }

        let client = reqwest::Client::new();
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            self.model, self.api_key
        );

        let response = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown Error".into());
            error!(
                "[unified_provider] Gemini API Error ({}): {}",
                status, err_text
            );
            return Err(format!("Gemini API Error ({}): {}", status, err_text));
        }

        let stream = response.bytes_stream().eventsource();
        let mut in_thought = false;

        let s = stream
            .map(move |event_res| {
                let mut events = Vec::new();
                match event_res {
                    Ok(event) => {
                        let data = event.data;
                        if let Ok(json) = serde_json::from_str::<Value>(&data) {
                            info!("[unified_provider] Gemini chunk received.");
                            if let Some(candidates) = json["candidates"].as_array() {
                                if let Some(candidate) = candidates.first() {
                                    if let Some(parts) = candidate["content"]["parts"].as_array() {
                                        for part in parts {
                                            info!("[unified_provider] Gemini part: {:?}", part);
                                            // Handle Thinking Process (Gemini 3)
                                            let is_thought =
                                                part["thought"].as_bool().unwrap_or(false)
                                                    || part
                                                        .get("text")
                                                        .and_then(|v| v.as_str())
                                                        .map(|s| s.contains("<thought>"))
                                                        .unwrap_or(false);

                                            if let Some(text) = part["text"].as_str() {
                                                if text.is_empty() {
                                                    continue;
                                                }

                                                let mut final_text = text.to_string();

                                                if is_thought && !in_thought {
                                                    final_text = format!("<think>\n{}", final_text);
                                                    in_thought = true;
                                                } else if !is_thought && in_thought {
                                                    final_text =
                                                        format!("</think>\n\n{}", final_text);
                                                    in_thought = false;
                                                }

                                                events.push(Ok(ProviderEvent::Content(final_text)));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        events.push(Err(e.to_string()));
                    }
                }
                futures::stream::iter(events)
            })
            .flatten();

        Ok(Box::pin(s))
    }

    pub async fn stream_completion(
        &self,
        prompt: String,
        history: Vec<rig::completion::Message>,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = Result<String, String>> + Send>>, String>
    {
        match self.kind {
            ProviderKind::Anthropic => {
                let mut messages = Vec::new();
                for msg in history {
                    messages.push(json!({ "role": msg.role, "content": msg.content }));
                }
                messages.push(json!({ "role": "user", "content": prompt }));
                self.stream_anthropic(messages, None).await.map(|stream| {
                    Box::pin(stream.map(|res| match res {
                        Ok(ProviderEvent::Content(c)) => Ok(c),
                        Ok(_) => Ok("".into()),
                        Err(e) => Err(e),
                    }))
                        as std::pin::Pin<
                            Box<dyn futures::Stream<Item = Result<String, String>> + Send>,
                        >
                })
            }
            ProviderKind::Gemini => {
                let mut messages = Vec::new();
                for msg in history {
                    messages.push(json!({ "role": msg.role, "content": msg.content }));
                }
                messages.push(json!({ "role": "user", "content": prompt }));
                self.stream_gemini(messages, None).await.map(|stream| {
                    Box::pin(stream.map(|res| match res {
                        Ok(ProviderEvent::Content(c)) => Ok(c),
                        Ok(_) => Ok("".into()),
                        Err(e) => Err(e),
                    }))
                        as std::pin::Pin<
                            Box<dyn futures::Stream<Item = Result<String, String>> + Send>,
                        >
                })
            }
            _ => {
                let lp = crate::rig_lib::llama_provider::LlamaProvider::new(
                    &self.base_url,
                    &self.api_key,
                    &self.model,
                );
                lp.stream_completion(prompt, history).await
            }
        }
    }
}
