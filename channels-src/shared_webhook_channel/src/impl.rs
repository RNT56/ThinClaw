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
    #[serde(default = "default_methods")]
    methods: Vec<String>,
    #[serde(default = "default_ack_status")]
    ack_status: u16,
    #[serde(default = "default_ack_body")]
    ack_body: String,
    #[serde(default)]
    events_path: Option<String>,
    #[serde(default)]
    challenge: Option<ChallengeConfig>,
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
    #[serde(default)]
    metadata: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ChallengeConfig {
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    match_path: Option<String>,
    #[serde(default)]
    match_value: Option<String>,
    response_path: String,
    #[serde(default = "default_challenge_format")]
    response_format: String,
}

#[derive(Debug, Deserialize)]
struct ResponseConfig {
    url: String,
    #[serde(default = "default_response_method")]
    method: String,
    #[serde(default = "default_body_format")]
    body_format: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: Value,
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
                methods: config.methods,
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
        let body_payload = request_payload(&req);
        let query_payload = query_payload(&req);

        if let Some(response) = maybe_challenge_response(&config, &req, &body_payload, &query_payload)
        {
            return response;
        }

        let mut emitted = 0usize;
        for payload in event_payloads(&body_payload, config.events_path.as_deref()) {
            if emit_payload(&config, payload) {
                emitted += 1;
            }
        }

        if emitted == 0 {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("{} webhook ignored payload without message text", config.channel_name),
            );
        }

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
        let mut headers = match response_config.body_format.as_str() {
            "form" => serde_json::json!({"Content-Type": "application/x-www-form-urlencoded"}),
            _ => serde_json::json!({"Content-Type": "application/json"}),
        };
        if let Some(object) = headers.as_object_mut() {
            for (key, value) in &response_config.headers {
                object.insert(
                    key.clone(),
                    Value::String(render_template(
                        value,
                        &metadata,
                        &response.content,
                        &config.template_values,
                    )),
                );
            }
        }

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

fn default_methods() -> Vec<String> {
    vec!["POST".to_string()]
}

fn default_response_method() -> String {
    "POST".to_string()
}

fn default_body_format() -> String {
    "json".to_string()
}

fn default_challenge_format() -> String {
    "text".to_string()
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
    let trimmed = body.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        serde_json::from_str(&body).unwrap_or(Value::Null)
    } else if trimmed.starts_with('<') {
        xml_body_to_json(trimmed)
    } else {
        form_body_to_json(&body)
    }
}

fn query_payload(req: &IncomingHttpRequest) -> Value {
    serde_json::from_str(&req.query_json).unwrap_or(Value::Null)
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

fn xml_body_to_json(body: &str) -> Value {
    let mut object = Map::new();
    let mut rest = body;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        if rest.starts_with('/') || rest.starts_with('?') || rest.starts_with('!') {
            continue;
        }
        let Some(tag_end) = rest.find('>') else {
            break;
        };
        let tag = rest[..tag_end]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if tag.is_empty() {
            rest = &rest[tag_end + 1..];
            continue;
        }
        let close = format!("</{tag}>");
        let after_open = &rest[tag_end + 1..];
        let Some(close_start) = after_open.find(&close) else {
            rest = after_open;
            continue;
        };
        let raw = after_open[..close_start].trim();
        let leaf = strip_cdata(raw);
        if !leaf.contains('<') {
            object.insert(tag, Value::String(xml_unescape(leaf)));
            rest = &after_open[close_start + close.len()..];
        } else {
            rest = after_open;
        }
    }
    Value::Object(object)
}

fn strip_cdata(value: &str) -> &str {
    value
        .strip_prefix("<![CDATA[")
        .and_then(|rest| rest.strip_suffix("]]>"))
        .unwrap_or(value)
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn event_payloads<'a>(payload: &'a Value, events_path: Option<&str>) -> Vec<&'a Value> {
    let Some(path) = events_path else {
        return vec![payload];
    };
    match value_at_json_path(payload, path) {
        Some(Value::Array(events)) => events.iter().collect(),
        Some(value) => vec![value],
        None => vec![payload],
    }
}

fn emit_payload(config: &GenericWebhookConfig, payload: &Value) -> bool {
    let Some(text) = first_string(payload, &config.mapping.text) else {
        return false;
    };

    let user_id = first_string(payload, &config.mapping.user_id)
        .unwrap_or_else(|| "unknown-user".to_string());
    let user_name = first_string(payload, &config.mapping.user_name);
    let chat_id = first_string(payload, &config.mapping.chat_id).unwrap_or_else(|| user_id.clone());
    let chat_type = first_string(payload, &config.mapping.chat_type)
        .map(|kind| normalize_chat_type(&kind))
        .unwrap_or_else(|| "dm".to_string());
    let message_id = first_string(payload, &config.mapping.message_id);
    let event_id = first_string(payload, &config.mapping.event_id);

    let conversation_kind = if chat_type == "dm" { "direct" } else { "group" };
    let external_key = format!("{}://{}/{}", config.channel_name, chat_type, chat_id);

    let mut metadata = Map::new();
    metadata.insert("provider".to_string(), Value::String(config.channel_name.clone()));
    metadata.insert("chat_id".to_string(), Value::String(chat_id.clone()));
    metadata.insert("chat_type".to_string(), Value::String(chat_type.clone()));
    metadata.insert("user_id".to_string(), Value::String(user_id.clone()));
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
    for (name, paths) in &config.mapping.metadata {
        if let Some(value) = first_string(payload, paths) {
            metadata.insert(name.clone(), Value::String(value));
        }
    }

    channel_host::emit_message(&EmittedMessage {
        user_id,
        user_name,
        content: text,
        thread_id: Some(chat_id),
        metadata_json: Value::Object(metadata).to_string(),
        attachments: vec![],
    });
    true
}

fn maybe_challenge_response(
    config: &GenericWebhookConfig,
    req: &IncomingHttpRequest,
    body_payload: &Value,
    query_payload: &Value,
) -> Option<OutgoingHttpResponse> {
    let challenge = config.challenge.as_ref()?;
    if let Some(method) = &challenge.method {
        if !req.method.eq_ignore_ascii_case(method) {
            return None;
        }
    }
    if let Some(path) = &challenge.match_path {
        let value = request_value(body_payload, query_payload, path)?;
        if let Some(expected) = &challenge.match_value {
            if value != *expected {
                return None;
            }
        }
    }
    let response = request_value(body_payload, query_payload, &challenge.response_path)?;
    match challenge.response_format.as_str() {
        "json_challenge" => Some(json_response(
            200,
            serde_json::json!({ "challenge": response }),
        )),
        _ => Some(text_response(200, &response)),
    }
}

fn request_value(body_payload: &Value, query_payload: &Value, path: &str) -> Option<String> {
    if let Some(rest) = path.strip_prefix("query.") {
        return value_at_path(query_payload, rest);
    }
    if let Some(rest) = path.strip_prefix("body.") {
        return value_at_path(body_payload, rest);
    }
    value_at_path(body_payload, path).or_else(|| value_at_path(query_payload, path))
}

fn first_string(payload: &Value, paths: &[String]) -> Option<String> {
    paths.iter().find_map(|path| value_at_path(payload, path))
}

fn value_at_path(payload: &Value, path: &str) -> Option<String> {
    let current = value_at_json_path(payload, path)?;
    match current {
        Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn value_at_json_path<'a>(payload: &'a Value, path: &str) -> Option<&'a Value> {
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
    Some(current)
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
        "form" => {
            let Some(object) = config.body.as_object() else {
                return Vec::new();
            };
            object
                .iter()
                .map(|(key, value)| {
                    let rendered = render_template(
                        value.as_str().unwrap_or_default(),
                        metadata,
                        content,
                        template_values,
                    );
                    format!("{}={}", percent_encode(key), percent_encode(&rendered))
                })
                .collect::<Vec<_>>()
                .join("&")
                .into_bytes()
        }
        _ => {
            render_json_value(&config.body, metadata, content, template_values)
                .to_string()
                .into_bytes()
        }
    }
}

fn render_json_value(
    value: &Value,
    metadata: &Value,
    content: &str,
    template_values: &BTreeMap<String, String>,
) -> Value {
    match value {
        Value::String(template) => {
            if let Some(json_template) = template.strip_prefix("$json_string:") {
                Value::String(
                    render_json_value(
                        &serde_json::from_str(json_template)
                            .unwrap_or_else(|_| Value::String(json_template.to_string())),
                        metadata,
                        content,
                        template_values,
                    )
                    .to_string(),
                )
            } else {
                Value::String(render_template(template, metadata, content, template_values))
            }
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| render_json_value(item, metadata, content, template_values))
                .collect(),
        ),
        Value::Object(object) => {
            let mut rendered = Map::new();
            for (key, value) in object {
                rendered.insert(
                    key.clone(),
                    render_json_value(value, metadata, content, template_values),
                );
            }
            Value::Object(rendered)
        }
        other => other.clone(),
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
    replace_metadata_placeholders(&mut rendered, metadata);
    rendered
}

fn replace_metadata_placeholders(rendered: &mut String, metadata: &Value) {
    if let Some(object) = metadata.as_object() {
        for (key, value) in object {
            match value {
                Value::String(text) => {
                    *rendered = rendered.replace(&format!("{{{key}}}"), text);
                }
                Value::Number(number) => {
                    *rendered = rendered.replace(&format!("{{{key}}}"), &number.to_string());
                }
                Value::Bool(flag) => {
                    *rendered = rendered.replace(&format!("{{{key}}}"), &flag.to_string());
                }
                _ => {}
            }
        }
    }
}

fn json_response(status: u16, value: Value) -> OutgoingHttpResponse {
    OutgoingHttpResponse {
        status,
        headers_json: serde_json::json!({"Content-Type": "application/json"}).to_string(),
        body: value.to_string().into_bytes(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_json_body_supports_nested_arrays_and_objects() {
        let config = ResponseConfig {
            url: "https://example.test".to_string(),
            method: "POST".to_string(),
            body_format: "json".to_string(),
            headers: BTreeMap::new(),
            body: serde_json::json!({
                "to": "{chat_id}",
                "messages": [{"type": "text", "text": "{content}"}]
            }),
        };
        let metadata = serde_json::json!({"chat_id": "room-1"});
        let body = String::from_utf8(render_body(
            &config,
            &metadata,
            "hello",
            &BTreeMap::new(),
        ))
        .unwrap();

        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            serde_json::json!({
                "to": "room-1",
                "messages": [{"type": "text", "text": "hello"}]
            })
        );
    }

    #[test]
    fn render_json_string_field_escapes_content() {
        let config = ResponseConfig {
            url: "https://example.test".to_string(),
            method: "POST".to_string(),
            body_format: "json".to_string(),
            headers: BTreeMap::new(),
            body: serde_json::json!({
                "content": "$json_string:{\"text\":\"{content}\"}"
            }),
        };
        let body = String::from_utf8(render_body(
            &config,
            &Value::Null,
            "hello \"team\"",
            &BTreeMap::new(),
        ))
        .unwrap();
        let parsed = serde_json::from_str::<Value>(&body).unwrap();
        let content = parsed.get("content").and_then(Value::as_str).unwrap();

        assert_eq!(
            serde_json::from_str::<Value>(content).unwrap(),
            serde_json::json!({"text": "hello \"team\""})
        );
    }

    #[test]
    fn xml_body_parser_extracts_text_leaf_nodes() {
        let parsed = xml_body_to_json(
            "<xml><FromUserName><![CDATA[user-1]]></FromUserName><Content>hello &amp; hi</Content></xml>",
        );

        assert_eq!(value_at_path(&parsed, "FromUserName").as_deref(), Some("user-1"));
        assert_eq!(
            value_at_path(&parsed, "Content").as_deref(),
            Some("hello & hi")
        );
    }

    #[test]
    fn events_path_expands_batched_payloads() {
        let payload = serde_json::json!({"events": [{"id": "a"}, {"id": "b"}]});
        let ids = event_payloads(&payload, Some("events"))
            .into_iter()
            .filter_map(|value| value_at_path(value, "id"))
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    }
}
