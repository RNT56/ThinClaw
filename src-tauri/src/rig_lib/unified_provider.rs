use futures::StreamExt;
use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionResponse, ModelChoice,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ProviderKind {
    OpenAI,
    Anthropic,
    Gemini,
    Groq,  // Usually OpenAI compatible
    Local, // Usually OpenAI compatible
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

        let body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": 0.7,
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
                    }
                }
                Ok((total_chars / 4) as u32)
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
        match self.kind {
            ProviderKind::Local | ProviderKind::OpenAI | ProviderKind::Groq => {
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

        let stream = response.bytes_stream();
        let mut buffer = String::new();

        let s = stream
            .map(move |chunk_res| {
                let mut events = Vec::new();
                match chunk_res {
                    Ok(chunk) => {
                        let text = String::from_utf8_lossy(&chunk);
                        buffer.push_str(&text);

                        while let Some(pos) = buffer.find("\n\n") {
                            let block = buffer.drain(..pos + 2).collect::<String>();
                            for line in block.lines() {
                                if line.starts_with("data: ") {
                                    let data = &line[6..];
                                    if data == "[DONE]" || data.is_empty() {
                                        continue;
                                    }
                                    if let Ok(json) = serde_json::from_str::<Value>(data) {
                                        match json["type"].as_str() {
                                            Some("content_block_delta") => {
                                                if let Some(delta) = json["delta"]["text"].as_str()
                                                {
                                                    events.push(Ok(ProviderEvent::Content(
                                                        delta.to_string(),
                                                    )));
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
        let mut contents = Vec::new();
        for msg in messages {
            let role = if msg["role"] == "assistant" {
                "model"
            } else {
                "user"
            };
            contents.push(json!({
                "role": role,
                "parts": [{ "text": msg["content"] }]
            }));
        }

        let body = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": 0.7,
                "maxOutputTokens": 4096,
            }
        });

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

        let stream = response.bytes_stream();
        let mut buffer = String::new();

        let s = stream
            .map(move |chunk_res| {
                let mut events = Vec::new();
                match chunk_res {
                    Ok(chunk) => {
                        let text = String::from_utf8_lossy(&chunk);
                        buffer.push_str(&text);

                        while let Some(pos) = buffer.find("\n\n") {
                            let block = buffer.drain(..pos + 2).collect::<String>();
                            for line in block.lines() {
                                let trimmed = line.trim();
                                if trimmed.starts_with("data: ") {
                                    let data = &trimmed[6..];
                                    if data.is_empty() {
                                        continue;
                                    }
                                    if let Ok(json) = serde_json::from_str::<Value>(data) {
                                        if let Some(candidates) = json["candidates"].as_array() {
                                            if let Some(candidate) = candidates.first() {
                                                if let Some(parts) =
                                                    candidate["content"]["parts"].as_array()
                                                {
                                                    for part in parts {
                                                        if let Some(text) = part["text"].as_str() {
                                                            events.push(Ok(
                                                                ProviderEvent::Content(
                                                                    text.to_string(),
                                                                ),
                                                            ));
                                                        }
                                                    }
                                                }
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
