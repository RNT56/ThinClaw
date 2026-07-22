use eventsource_stream::Eventsource;
use futures::StreamExt;
use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionResponse, ModelChoice,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::info;

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
    pub model_family: Option<String>,
    client: Option<reqwest::Client>,
    streaming_client: Option<reqwest::Client>,
    configuration_error: Option<String>,
}

impl UnifiedProvider {
    pub fn new(
        kind: ProviderKind,
        base_url: &str,
        api_key: &str,
        model: &str,
        model_family: Option<String>,
    ) -> Self {
        let configuration = Self::validate_configuration(&kind, base_url, api_key, model);
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
            kind,
            base_url,
            api_key: api_key.to_string(),
            model: model.to_string(),
            model_family,
            client,
            streaming_client,
            configuration_error,
        }
    }

    fn validate_configuration(
        kind: &ProviderKind,
        base_url: &str,
        api_key: &str,
        model: &str,
    ) -> Result<(String, bool), String> {
        if base_url.is_empty() || base_url.len() > 4_096 || base_url.chars().any(char::is_control) {
            return Err("The provider endpoint is missing or invalid".to_string());
        }
        let url = reqwest::Url::parse(base_url)
            .map_err(|_| "The provider endpoint is not a valid URL".to_string())?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "The provider endpoint must not contain credentials, a query, or a fragment"
                    .to_string(),
            );
        }
        let host = url
            .host_str()
            .ok_or_else(|| "The provider endpoint has no host".to_string())?;
        let host_is_loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        let is_local = matches!(kind, ProviderKind::Local);
        if is_local && !host_is_loopback {
            return Err("The local provider endpoint must use a loopback host".to_string());
        }
        if (is_local && !matches!(url.scheme(), "http" | "https"))
            || (!is_local && url.scheme() != "https")
        {
            return Err(
                "Remote provider endpoints must use HTTPS; local endpoints must use HTTP(S)"
                    .to_string(),
            );
        }
        if api_key.trim() != api_key
            || api_key.len() > 16 * 1024
            || api_key.chars().any(char::is_control)
            || (!is_local && api_key.is_empty())
        {
            return Err("The provider credential is missing or invalid".to_string());
        }
        if model.is_empty() || model.len() > 512 || model.chars().any(char::is_control) {
            return Err("The provider model identifier is missing or invalid".to_string());
        }
        if matches!(kind, ProviderKind::Gemini)
            && !model
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err("The Gemini model identifier is invalid".to_string());
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
                .ok_or_else(|| "The streaming provider client is unavailable".to_string())
        } else {
            self.client
                .as_ref()
                .ok_or_else(|| "The provider client is unavailable".to_string())
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
        m.starts_with("o1-") || m.starts_with("o3-") || m == "o1" || m.contains("gpt-5")
    }

    fn sanitize_temperature(&self, temp: f64) -> Option<f64> {
        if self.is_reasoning_model() || !temp.is_finite() || !(0.0..=2.0).contains(&temp) {
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

    fn tool_choice(
        provider: &str,
        name: Option<&str>,
        id: Option<&str>,
        arguments: Value,
    ) -> Result<ModelChoice, CompletionError> {
        let name = name.filter(|value| {
            !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
        });
        let id = id.filter(|value| {
            !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
        });
        let (Some(name), Some(id)) = (name, id) else {
            return Err(CompletionError::ProviderError(format!(
                "{provider} returned a malformed tool call"
            )));
        };
        if !arguments.is_object() {
            return Err(CompletionError::ProviderError(format!(
                "{provider} returned a malformed tool call"
            )));
        }
        Ok(ModelChoice::ToolCall(
            name.to_string(),
            id.to_string(),
            arguments,
        ))
    }

    fn required_first(
        provider: &str,
        choices: &[ModelChoice],
    ) -> Result<ModelChoice, CompletionError> {
        choices
            .first()
            .map(Self::clone_model_choice)
            .ok_or_else(|| {
                CompletionError::ProviderError(format!("{provider} returned an empty response"))
            })
    }

    async fn completion_openai(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Vec<ModelChoice>>, CompletionError> {
        let temperature = request.temperature.unwrap_or(0.7);
        if !temperature.is_finite() || !(0.0..=2.0).contains(&temperature) {
            return Err(CompletionError::ProviderError(
                "OpenAI-compatible temperature is outside the supported range".into(),
            ));
        }
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

        if let Some(t) = self.sanitize_temperature(temperature) {
            body["temperature"] = json!(t);
        }

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

        if matches!(self.kind, ProviderKind::OpenRouter) {
            request_builder = request_builder
                .header("HTTP-Referer", "https://github.com/RNT56/ThinClaw")
                .header("X-Title", "ThinClaw Desktop");
        }

        let resp = request_builder.send().await.map_err(|error| {
            CompletionError::ProviderError(crate::rig_lib::http::transport_error(
                "OpenAI-compatible request failed",
                error,
            ))
        })?;
        let resp = crate::rig_lib::http::checked_response(resp, "OpenAI-compatible provider")
            .await
            .map_err(CompletionError::ProviderError)?;
        let json: Value = crate::rig_lib::http::bounded_json(resp, "OpenAI-compatible provider")
            .await
            .map_err(CompletionError::ProviderError)?;

        let choice = json
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(Value::as_object)
            .ok_or_else(|| {
                CompletionError::ProviderError(
                    "OpenAI-compatible provider returned no message choice".into(),
                )
            })?;
        let mut model_choices = Vec::new();

        if let Some(content) = choice.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                model_choices.push(ModelChoice::Message(content.to_string()));
            }
        } else if choice.get("content").is_some_and(|value| !value.is_null()) {
            return Err(CompletionError::ProviderError(
                "OpenAI-compatible provider returned malformed message content".into(),
            ));
        }

        if let Some(tool_calls) = choice.get("tool_calls").and_then(Value::as_array) {
            if tool_calls.len() > 128 {
                return Err(CompletionError::ProviderError(
                    "OpenAI-compatible provider returned too many tool calls".into(),
                ));
            }
            for tc in tool_calls {
                if tc.get("type").and_then(Value::as_str) != Some("function") {
                    return Err(CompletionError::ProviderError(
                        "OpenAI-compatible provider returned a malformed tool call".into(),
                    ));
                }
                let args = tc
                    .get("function")
                    .and_then(|function| function.get("arguments"))
                    .and_then(Value::as_str)
                    .and_then(|arguments| serde_json::from_str(arguments).ok())
                    .ok_or_else(|| {
                        CompletionError::ProviderError(
                            "OpenAI-compatible provider returned malformed tool arguments".into(),
                        )
                    })?;
                model_choices.push(Self::tool_choice(
                    "OpenAI-compatible provider",
                    tc.get("function")
                        .and_then(|function| function.get("name"))
                        .and_then(Value::as_str),
                    tc.get("id").and_then(Value::as_str),
                    args,
                )?);
            }
        } else if choice
            .get("tool_calls")
            .is_some_and(|value| !value.is_null())
        {
            return Err(CompletionError::ProviderError(
                "OpenAI-compatible provider returned malformed tool calls".into(),
            ));
        }

        let first = Self::required_first("OpenAI-compatible provider", &model_choices)?;

        Ok(CompletionResponse {
            choice: first,
            raw_response: model_choices,
        })
    }

    async fn completion_anthropic(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Vec<ModelChoice>>, CompletionError> {
        let temperature = request.temperature.unwrap_or(0.7);
        if !temperature.is_finite() || !(0.0..=1.0).contains(&temperature) {
            return Err(CompletionError::ProviderError(
                "Anthropic temperature is outside the supported range".into(),
            ));
        }
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
            body["system"] = json!(s);
        }

        if let Some(temperature) = self.sanitize_temperature(temperature) {
            body["temperature"] = json!(temperature);
        }

        if !request.tools.is_empty() {
            body["tools"] = json!(request
                .tools
                .iter()
                .map(|t| json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters
                }))
                .collect::<Vec<_>>());
        }

        let client = self.client(false).map_err(CompletionError::ProviderError)?;
        let url = self
            .endpoint("messages")
            .map_err(CompletionError::ProviderError)?;
        let body = crate::rig_lib::http::bounded_json_body(&body)
            .map_err(CompletionError::ProviderError)?;

        let resp = client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|error| {
                CompletionError::ProviderError(crate::rig_lib::http::transport_error(
                    "Anthropic request failed",
                    error,
                ))
            })?;
        let resp = crate::rig_lib::http::checked_response(resp, "Anthropic")
            .await
            .map_err(CompletionError::ProviderError)?;
        let json: Value = crate::rig_lib::http::bounded_json(resp, "Anthropic")
            .await
            .map_err(CompletionError::ProviderError)?;

        let mut model_choices = Vec::new();
        let content_array = json
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                CompletionError::ProviderError("Anthropic returned malformed content".into())
            })?;
        if content_array.len() > 128 {
            return Err(CompletionError::ProviderError(
                "Anthropic returned too many content blocks".into(),
            ));
        }
        for item in content_array {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let text = item.get("text").and_then(Value::as_str).ok_or_else(|| {
                        CompletionError::ProviderError(
                            "Anthropic returned malformed text content".into(),
                        )
                    })?;
                    if !text.is_empty() {
                        model_choices.push(ModelChoice::Message(text.to_string()));
                    }
                }
                Some("tool_use") => {
                    model_choices.push(Self::tool_choice(
                        "Anthropic",
                        item.get("name").and_then(Value::as_str),
                        item.get("id").and_then(Value::as_str),
                        item.get("input").cloned().unwrap_or(Value::Null),
                    )?);
                }
                _ => {}
            }
        }

        let first = Self::required_first("Anthropic", &model_choices)?;

        Ok(CompletionResponse {
            choice: first,
            raw_response: model_choices,
        })
    }

    async fn completion_gemini(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Vec<ModelChoice>>, CompletionError> {
        if request
            .temperature
            .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
        {
            return Err(CompletionError::ProviderError(
                "Gemini temperature is outside the supported range".into(),
            ));
        }
        let mut contents: Vec<Value> = Vec::new();
        let mut system_instruction = None;

        if let Some(p) = request.preamble {
            system_instruction = Some(json!({ "parts": [{ "text": p }] }));
        }

        // Include chat history
        for msg in request.chat_history {
            let gemini_role = if msg.role == "assistant" {
                "model"
            } else {
                "user"
            };
            contents.push(json!({ "role": gemini_role, "parts": [{ "text": msg.content }] }));
        }

        contents.push(json!({ "role": "user", "parts": [{ "text": request.prompt }] }));

        let mut body = json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": 4096,
            }
        });

        if let Some(si) = system_instruction {
            body["system_instruction"] = si;
        }

        if let Some(t) = request
            .temperature
            .and_then(|temp| self.sanitize_temperature(temp))
        {
            if let Some(obj) = body["generationConfig"].as_object_mut() {
                obj.insert("temperature".into(), json!(t));
            }
        }

        if !request.tools.is_empty() {
            body["tools"] = json!([{
                "function_declarations": request.tools.iter().map(|t| json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters
                })).collect::<Vec<_>>()
            }]);
        }

        let client = self.client(false).map_err(CompletionError::ProviderError)?;
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        );
        let body = crate::rig_lib::http::bounded_json_body(&body)
            .map_err(CompletionError::ProviderError)?;

        let resp = client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|error| {
                CompletionError::ProviderError(crate::rig_lib::http::transport_error(
                    "Gemini request failed",
                    error,
                ))
            })?;
        let resp = crate::rig_lib::http::checked_response(resp, "Gemini")
            .await
            .map_err(CompletionError::ProviderError)?;
        let json: Value = crate::rig_lib::http::bounded_json(resp, "Gemini")
            .await
            .map_err(CompletionError::ProviderError)?;

        let mut model_choices = Vec::new();
        let parts = json
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|candidates| candidates.first())
            .and_then(|candidate| candidate.get("content"))
            .and_then(|content| content.get("parts"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                CompletionError::ProviderError("Gemini returned no candidate content".into())
            })?;
        if parts.len() > 128 {
            return Err(CompletionError::ProviderError(
                "Gemini returned too many content parts".into(),
            ));
        }
        for (index, part) in parts.iter().enumerate() {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                if !text.is_empty() {
                    model_choices.push(ModelChoice::Message(text.to_string()));
                }
            } else if let Some(func_call) = part.get("functionCall").and_then(Value::as_object) {
                let id = format!("gemini-tool-{index}");
                model_choices.push(Self::tool_choice(
                    "Gemini",
                    func_call.get("name").and_then(Value::as_str),
                    Some(&id),
                    func_call.get("args").cloned().unwrap_or(Value::Null),
                )?);
            }
        }

        let first = Self::required_first("Gemini", &model_choices)?;

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
                    self.model_family.as_deref().unwrap_or("chatml"),
                );
                lp.count_tokens(messages).await
            }
            _ => {
                let mut total_chars = 0_usize;
                for msg in messages {
                    if let Some(content) = msg["content"].as_str() {
                        total_chars = total_chars.saturating_add(content.len());
                    } else if let Some(content_array) = msg["content"].as_array() {
                        for part in content_array {
                            if let Some(text) = part["text"].as_str() {
                                total_chars = total_chars.saturating_add(text.len());
                            }
                        }
                    }
                }
                Ok(u32::try_from(total_chars.saturating_add(2) / 3).unwrap_or(u32::MAX))
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
                    self.model_family.as_deref().unwrap_or("chatml"),
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
        temperature: Option<f64>,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<ProviderEvent, String>> + Send>>,
        String,
    > {
        if temperature.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
            return Err("Anthropic temperature is outside the supported range".to_string());
        }
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": 4096,
            "stream": true,
        });

        if let Some(t) = temperature.and_then(|temp| self.sanitize_temperature(temp)) {
            body["temperature"] = json!(t);
        }

        // Handle system message if present in the messages list (Anthropic expects it as a top-level field)
        let mut filtered_messages = Vec::new();
        for msg in messages {
            if msg["role"] == "system" {
                body["system"] = msg["content"].clone();
            } else {
                filtered_messages.push(msg);
            }
        }
        body["messages"] = json!(filtered_messages);

        let client = self.client(true)?;
        let url = self.endpoint("messages")?;
        let body = crate::rig_lib::http::bounded_json_body(&body)?;

        let response = client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|error| {
                crate::rig_lib::http::transport_error("Anthropic request failed", error)
            })?;
        let response = crate::rig_lib::http::checked_response(response, "Anthropic").await?;

        let stream = crate::rig_lib::http::bounded_sse_bytes(response).eventsource();
        let s = stream
            .map(|event_res| {
                let mut events = Vec::new();
                match event_res {
                    Ok(event) => {
                        let data = event.data;
                        if data == "[DONE]" {
                            return futures::stream::iter(events);
                        }
                        if data.len() > 2 * 1024 * 1024 {
                            events.push(Err("Anthropic event exceeds the supported size".into()));
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
                                                    .map(|value| {
                                                        u32::try_from(value).unwrap_or(u32::MAX)
                                                    })
                                                    .unwrap_or(0),
                                                total_tokens: usage
                                                    .get("output_tokens")
                                                    .and_then(|v| v.as_u64())
                                                    .map(|value| {
                                                        u32::try_from(value).unwrap_or(u32::MAX)
                                                    })
                                                    .unwrap_or(0),
                                            },
                                        )));
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            events.push(Err("Anthropic stream returned invalid JSON".into()));
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
        temperature: Option<f64>,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<ProviderEvent, String>> + Send>>,
        String,
    > {
        if temperature.is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value)) {
            return Err("Gemini temperature is outside the supported range".to_string());
        }
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
            body["system_instruction"] = si;
        }

        if let Some(t) = temperature
            .and_then(|temp| self.sanitize_temperature(temp))
            .or_else(|| self.sanitize_temperature(0.7))
        {
            if let Some(obj) = body["generationConfig"].as_object_mut() {
                obj.insert("temperature".into(), json!(t));
            }
        }

        let client = self.client(true)?;
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
            self.model
        );
        let body = crate::rig_lib::http::bounded_json_body(&body)?;

        let response = client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|error| {
                crate::rig_lib::http::transport_error("Gemini request failed", error)
            })?;
        let response = crate::rig_lib::http::checked_response(response, "Gemini").await?;

        let stream = crate::rig_lib::http::bounded_sse_bytes(response).eventsource();
        let in_thought = std::sync::Arc::new(std::sync::Mutex::new(false));

        let s = stream
            .map(move |event_res| {
                let mut events = Vec::new();
                match event_res {
                    Ok(event) => {
                        let data = event.data;
                        if data.len() > 2 * 1024 * 1024 {
                            events.push(Err("Gemini event exceeds the supported size".into()));
                            return futures::stream::iter(events);
                        }
                        if let Ok(json) = serde_json::from_str::<Value>(&data) {
                            if let Some(candidates) = json["candidates"].as_array() {
                                if let Some(candidate) = candidates.first() {
                                    if let Some(parts) = candidate["content"]["parts"].as_array() {
                                        for part in parts {
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
                                                let mut thought_guard = in_thought
                                                    .lock()
                                                    .unwrap_or_else(|e| e.into_inner());

                                                if is_thought && !*thought_guard {
                                                    final_text = format!("<think>\n{}", final_text);
                                                    *thought_guard = true;
                                                } else if !is_thought && *thought_guard {
                                                    final_text =
                                                        format!("</think>\n\n{}", final_text);
                                                    *thought_guard = false;
                                                }
                                                drop(thought_guard);

                                                events.push(Ok(ProviderEvent::Content(final_text)));
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            events.push(Err("Gemini stream returned invalid JSON".into()));
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
                    self.model_family.as_deref().unwrap_or("chatml"),
                );
                lp.stream_completion(prompt, history).await
            }
        }
    }
}
