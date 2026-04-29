use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};

#[derive(Debug, Deserialize)]
struct GenericWebhookConfig {
    channel_name: String,
    display_name: String,
    webhook_path: String,
    #[serde(default = "default_ack_status")]
    ack_status: u16,
    #[serde(default = "default_ack_body")]
    ack_body: String,
    #[serde(default)]
    mapping: EventMapping,
    #[serde(default)]
    template_values: BTreeMap<String, String>,
    #[serde(default)]
    response: Option<ResponseConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct EventMapping {
    #[serde(default)]
    text: Vec<String>,
    #[serde(default)]
    user_id: Vec<String>,
    #[serde(default)]
    user_name: Vec<String>,
    #[serde(default)]
    chat_id: Vec<String>,
    #[serde(default)]
    chat_type: Vec<String>,
    #[serde(default)]
    message_id: Vec<String>,
    #[serde(default)]
    event_id: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ResponseConfig {
    url: String,
    #[serde(default = "default_response_method")]
    method: String,
    #[serde(default = "default_body_format")]
    body_format: String,
    #[serde(default)]
    body: BTreeMap<String, String>,
}

struct GenericWebhookChannel;

const CONFIG_PATH: &str = "state/config.json";

impl Guest for GenericWebhookChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config = parse_config(&config_json)?;
        channel_host::workspace_write(CONFIG_PATH, &config_json)
            .map_err(|error| format!("Failed to persist channel config: {error}"))?;
        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("{} channel starting", config.display_name),
        );

        Ok(ChannelConfig {
            display_name: config.display_name,
            http_endpoints: vec![HttpEndpointConfig {
                path: config.webhook_path,
                methods: vec!["POST".to_string()],
                require_secret: true,
            }],
            poll: None,
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        let config = match load_config() {
            Ok(config) => config,
            Err(error) => return text_response(500, &error),
        };
        let payload = request_payload(&req);
        let Some(text) = first_string(&payload, &config.mapping.text) else {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("{} webhook ignored payload without message text", config.channel_name),
            );
            return text_response(config.ack_status, &config.ack_body);
        };

        let user_id = first_string(&payload, &config.mapping.user_id)
            .unwrap_or_else(|| "unknown-user".to_string());
        let user_name = first_string(&payload, &config.mapping.user_name);
        let chat_id = first_string(&payload, &config.mapping.chat_id)
            .unwrap_or_else(|| user_id.clone());
        let chat_type = first_string(&payload, &config.mapping.chat_type)
            .map(|kind| normalize_chat_type(&kind))
            .unwrap_or_else(|| "dm".to_string());
        let message_id = first_string(&payload, &config.mapping.message_id);
        let event_id = first_string(&payload, &config.mapping.event_id);

        let conversation_kind = if chat_type == "dm" { "direct" } else { "group" };
        let external_key = format!("{}://{}/{}", config.channel_name, chat_type, chat_id);

        let mut metadata = Map::new();
        metadata.insert("provider".to_string(), Value::String(config.channel_name.clone()));
        metadata.insert("chat_id".to_string(), Value::String(chat_id.clone()));
        metadata.insert("chat_type".to_string(), Value::String(chat_type.clone()));
        metadata.insert(
            "conversation_kind".to_string(),
            Value::String(conversation_kind.to_string()),
        );
        metadata.insert(
            "conversation_scope_id".to_string(),
            Value::String(format!("{}:{}:{}", config.channel_name, chat_type, chat_id)),
        );
        metadata.insert(
            "external_conversation_key".to_string(),
            Value::String(external_key),
        );
        if let Some(message_id) = message_id {
            metadata.insert("provider_message_id".to_string(), Value::String(message_id));
        }
        if let Some(event_id) = event_id {
            metadata.insert("provider_event_id".to_string(), Value::String(event_id));
        }

        channel_host::emit_message(&EmittedMessage {
            user_id,
            user_name,
            content: text,
            thread_id: Some(chat_id),
            metadata_json: Value::Object(metadata).to_string(),
            attachments: vec![],
        });

        text_response(config.ack_status, &config.ack_body)
    }

    fn on_poll() {}

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let config = load_config()?;
        let Some(response_config) = config.response else {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("{} response delivery not configured", config.channel_name),
            );
            return Ok(());
        };
        let metadata = serde_json::from_str::<Value>(&response.metadata_json).unwrap_or(Value::Null);
        let url = render_template(
            &response_config.url,
            &metadata,
            &response.content,
            &config.template_values,
        );
        let body = render_body(
            &response_config,
            &metadata,
            &response.content,
            &config.template_values,
        );
        let headers = match response_config.body_format.as_str() {
            "form" => serde_json::json!({"Content-Type": "application/x-www-form-urlencoded"}),
            _ => serde_json::json!({"Content-Type": "application/json"}),
        };

        let result = channel_host::http_request(
            &response_config.method,
            &url,
            &headers.to_string(),
            Some(&body),
            Some(30_000),
        );
        match result {
            Ok(http) if (200..300).contains(&http.status) => Ok(()),
            Ok(http) => Err(format!(
                "{} response delivery failed with HTTP {}",
                config.channel_name, http.status
            )),
            Err(error) => Err(format!("{} response delivery failed: {error}", config.channel_name)),
        }
    }

    fn on_status(update: StatusUpdate) {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("status: {}", update.message),
        );
    }

    fn on_shutdown() {}
}

fn default_ack_status() -> u16 {
    200
}

fn default_ack_body() -> String {
    "ok".to_string()
}

fn default_response_method() -> String {
    "POST".to_string()
}

fn default_body_format() -> String {
    "json".to_string()
}

fn parse_config(config_json: &str) -> Result<GenericWebhookConfig, String> {
    serde_json::from_str(config_json).map_err(|error| format!("Failed to parse config: {error}"))
}

fn load_config() -> Result<GenericWebhookConfig, String> {
    let config_json = channel_host::workspace_read(CONFIG_PATH)
        .ok_or_else(|| "Channel config has not been initialized".to_string())?;
    parse_config(&config_json)
}

fn request_payload(req: &IncomingHttpRequest) -> Value {
    let body = String::from_utf8_lossy(&req.body);
    if body.trim().starts_with('{') || body.trim().starts_with('[') {
        serde_json::from_str(&body).unwrap_or(Value::Null)
    } else {
        form_body_to_json(&body)
    }
}

fn form_body_to_json(body: &str) -> Value {
    let mut object = Map::new();
    for part in body.split('&') {
        if part.is_empty() {
            continue;
        }
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        object.insert(percent_decode(key), Value::String(percent_decode(value)));
    }
    Value::Object(object)
}

fn first_string(payload: &Value, paths: &[String]) -> Option<String> {
    paths.iter().find_map(|path| value_at_path(payload, path))
}

fn value_at_path(payload: &Value, path: &str) -> Option<String> {
    let mut current = payload;
    for part in path.split('.') {
        if part.is_empty() {
            continue;
        }
        current = if let Some((name, index)) = parse_indexed_part(part) {
            current.get(name)?.get(index)?
        } else {
            current.get(part)?
        };
    }
    match current {
        Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn parse_indexed_part(part: &str) -> Option<(&str, usize)> {
    let (name, rest) = part.split_once('[')?;
    let index = rest.strip_suffix(']')?.parse().ok()?;
    Some((name, index))
}

fn normalize_chat_type(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "direct" | "private" | "dm" | "im" | "sms" | "user" => "dm".to_string(),
        "group" | "room" | "channel" | "team" | "guild" | "space" | "chat" => {
            "group".to_string()
        }
        other if other.is_empty() => "dm".to_string(),
        other => other.to_string(),
    }
}

fn render_body(
    config: &ResponseConfig,
    metadata: &Value,
    content: &str,
    template_values: &BTreeMap<String, String>,
) -> Vec<u8> {
    match config.body_format.as_str() {
        "form" => config
            .body
            .iter()
            .map(|(key, value)| {
                format!(
                    "{}={}",
                    percent_encode(key),
                    percent_encode(&render_template(value, metadata, content, template_values))
                )
            })
            .collect::<Vec<_>>()
            .join("&")
            .into_bytes(),
        _ => {
            let mut object = Map::new();
            for (key, value) in &config.body {
                object.insert(
                    key.clone(),
                    Value::String(render_template(value, metadata, content, template_values)),
                );
            }
            Value::Object(object).to_string().into_bytes()
        }
    }
}

fn render_template(
    template: &str,
    metadata: &Value,
    content: &str,
    template_values: &BTreeMap<String, String>,
) -> String {
    let mut rendered = template.replace("{content}", content);
    for (key, value) in template_values {
        rendered = rendered.replace(&format!("{{{key}}}"), value);
    }
    for key in ["chat_id", "channel_id", "conversation_id", "user_id", "provider_message_id"] {
        if let Some(value) = value_at_path(metadata, key) {
            rendered = rendered.replace(&format!("{{{key}}}"), &value);
        }
    }
    rendered
}

fn text_response(status: u16, body: &str) -> OutgoingHttpResponse {
    OutgoingHttpResponse {
        status,
        headers_json: serde_json::json!({"Content-Type": "text/plain"}).to_string(),
        body: body.as_bytes().to_vec(),
    }
}

fn percent_decode(value: &str) -> String {
    let mut bytes = Vec::with_capacity(value.len());
    let mut chars = value.as_bytes().iter().copied();
    while let Some(byte) = chars.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let hi = chars.next();
                let lo = chars.next();
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    if let (Some(hi), Some(lo)) = (hex_value(hi), hex_value(lo)) {
                        bytes.push((hi << 4) | lo);
                    }
                }
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            b' ' => encoded.push('+'),
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
