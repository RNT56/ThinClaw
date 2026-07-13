use crate::util::floor_char_boundary;
use thinclaw_types::error::ChannelError;

/// Normalize a server URL (add http:// if missing, strip trailing slash).
pub(super) fn normalize_server_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let with_scheme = if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        format!("http://{trimmed}")
    } else {
        trimmed.to_string()
    };
    with_scheme.trim_end_matches('/').to_string()
}

/// Extract the message record from a BlueBubbles webhook payload.
pub(super) fn extract_record(payload: &serde_json::Value) -> Option<serde_json::Value> {
    if let Some(data) = payload.get("data") {
        if data.is_object() {
            return Some(data.clone());
        }
        if let Some(arr) = data.as_array()
            && let Some(first) = arr.first()
            && first.is_object()
        {
            return Some(first.clone());
        }
    }
    if let Some(msg) = payload.get("message")
        && msg.is_object()
    {
        return Some(msg.clone());
    }
    payload.is_object().then(|| payload.clone())
}

/// Get the first non-empty string from a slice of optional JSON values.
pub(super) fn first_string(candidates: &[Option<&serde_json::Value>]) -> Option<String> {
    for candidate in candidates {
        if let Some(val) = candidate
            && let Some(value) = val.as_str().map(str::trim)
            && !value.is_empty()
        {
            return Some(value.to_string());
        }
    }
    None
}

/// Split a long message into chunks at newline boundaries.
pub(super) fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }
        let safe_end = floor_char_boundary(remaining, max_len);
        let split_at = remaining[..safe_end].rfind('\n').unwrap_or(safe_end);
        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start_matches('\n');
    }
    chunks
}

/// Redact a URL for logging (hide password, port, etc.).
pub(super) fn redact_url(url: &str) -> String {
    if let Some(idx) = url.find("://") {
        let after_scheme = &url[idx + 3..];
        if let Some(slash) = after_scheme.find('/') {
            return format!("{}://{}/***", &url[..idx], &after_scheme[..slash]);
        }
        return format!("{}://{}", &url[..idx], after_scheme);
    }
    "[redacted]".to_string()
}

/// Redact phone numbers and email addresses from text.
pub(super) fn redact_pii(text: &str) -> String {
    use std::sync::OnceLock;
    static PHONE_RE: OnceLock<regex::Regex> = OnceLock::new();
    static EMAIL_RE: OnceLock<regex::Regex> = OnceLock::new();

    let phone = PHONE_RE
        .get_or_init(|| regex::Regex::new(r"\+?\d{7,15}").expect("static phone regex is valid"));
    let email = EMAIL_RE.get_or_init(|| {
        regex::Regex::new(r"[\w.+-]+@[\w-]+\.[\w.]+").expect("static email regex is valid")
    });
    let redacted = phone.replace_all(text, "[REDACTED]");
    email.replace_all(&redacted, "[REDACTED]").to_string()
}

/// Decode a base64-encoded string to bytes.
pub(super) fn base64_decode(input: &str) -> Result<Vec<u8>, ChannelError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|error| ChannelError::SendFailed {
            name: super::NAME.to_string(),
            reason: format!("base64 decode error: {error}"),
        })
}
