use super::*;

#[derive(Default)]
pub(super) struct CatalogPagination {
    pages: usize,
    aggregate_bytes: usize,
    seen_cursors: HashSet<String>,
}

impl CatalogPagination {
    pub(super) fn begin_page(&mut self, catalog: &str) -> Result<(), ToolError> {
        self.pages = self.pages.checked_add(1).ok_or_else(|| {
            ToolError::ExternalService(format!("MCP {catalog} page count overflow"))
        })?;
        if self.pages > MAX_MCP_CATALOG_PAGES {
            return Err(ToolError::ExternalService(format!(
                "MCP {catalog} catalog exceeds the {MAX_MCP_CATALOG_PAGES} page limit"
            )));
        }
        Ok(())
    }

    pub(super) fn accept_items<T: serde::Serialize>(
        &mut self,
        existing_items: usize,
        items: &[T],
        catalog: &str,
    ) -> Result<(), ToolError> {
        let total_items = existing_items.checked_add(items.len()).ok_or_else(|| {
            ToolError::ExternalService(format!("MCP {catalog} item count overflow"))
        })?;
        if total_items > MAX_MCP_CATALOG_ITEMS {
            return Err(ToolError::ExternalService(format!(
                "MCP {catalog} catalog exceeds the {MAX_MCP_CATALOG_ITEMS} item limit"
            )));
        }

        let page_bytes = serde_json::to_vec(items)
            .map_err(|error| {
                ToolError::ExternalService(format!(
                    "Failed to measure MCP {catalog} catalog page: {error}"
                ))
            })?
            .len();
        self.aggregate_bytes = self
            .aggregate_bytes
            .checked_add(page_bytes)
            .ok_or_else(|| {
                ToolError::ExternalService(format!("MCP {catalog} catalog size overflow"))
            })?;
        if self.aggregate_bytes > MAX_MCP_CATALOG_BYTES {
            return Err(ToolError::ExternalService(format!(
                "MCP {catalog} catalog exceeds the {MAX_MCP_CATALOG_BYTES} byte aggregate limit"
            )));
        }
        Ok(())
    }

    pub(super) fn accept_cursor(
        &mut self,
        cursor: Option<String>,
        catalog: &str,
    ) -> Result<Option<String>, ToolError> {
        let Some(cursor) = cursor else {
            return Ok(None);
        };
        if cursor.is_empty() || cursor.len() > MAX_MCP_CURSOR_BYTES {
            return Err(ToolError::ExternalService(format!(
                "MCP {catalog} cursor must be non-empty and at most {MAX_MCP_CURSOR_BYTES} bytes"
            )));
        }
        if !self.seen_cursors.insert(cursor.clone()) {
            return Err(ToolError::ExternalService(format!(
                "MCP {catalog} server returned a repeated pagination cursor"
            )));
        }
        Ok(Some(cursor))
    }
}

pub(super) fn serialize_http_payload<T: serde::Serialize>(
    payload: &T,
) -> Result<Vec<u8>, ToolError> {
    let bytes = serde_json::to_vec(payload).map_err(|error| {
        ToolError::ExternalService(format!("Failed to serialize MCP HTTP payload: {error}"))
    })?;
    if bytes.len() > MAX_MCP_HTTP_MESSAGE_BYTES {
        return Err(ToolError::InvalidParameters(format!(
            "MCP HTTP payload exceeds the {MAX_MCP_HTTP_MESSAGE_BYTES} byte limit"
        )));
    }
    Ok(bytes)
}

pub(super) async fn read_limited_response(
    mut response: reqwest::Response,
    max_bytes: usize,
    context: &str,
) -> Result<Vec<u8>, ToolError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(ToolError::ExternalService(format!(
            "{context} exceeds the {max_bytes} byte limit"
        )));
    }

    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(0)
        .min(max_bytes);
    let mut body = Vec::with_capacity(initial_capacity);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| ToolError::ExternalService(format!("Failed to read {context}: {error}")))?
    {
        let new_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| ToolError::ExternalService(format!("{context} size overflow")))?;
        if new_len > max_bytes {
            return Err(ToolError::ExternalService(format!(
                "{context} exceeds the {max_bytes} byte limit"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(super) async fn bounded_error_body(response: reqwest::Response) -> String {
    match read_limited_response(
        response,
        MAX_MCP_HTTP_ERROR_BYTES,
        "MCP error response body",
    )
    .await
    {
        Ok(body) => String::from_utf8_lossy(&body)
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect(),
        Err(error) => format!("<body omitted: {error}>"),
    }
}

pub(super) fn append_sse_line_fragment(
    line: &mut Vec<u8>,
    fragment: &[u8],
) -> Result<(), ToolError> {
    let new_len = line
        .len()
        .checked_add(fragment.len())
        .ok_or_else(|| ToolError::ExternalService("MCP SSE line size overflow".to_string()))?;
    let max_line_bytes = MAX_MCP_HTTP_MESSAGE_BYTES + b"data:".len() + 1;
    if new_len > max_line_bytes {
        return Err(ToolError::ExternalService(format!(
            "MCP SSE line exceeds the {max_line_bytes} byte limit"
        )));
    }
    line.extend_from_slice(fragment);
    Ok(())
}

/// Extract a server name from a URL for logging/display purposes.
pub(super) fn extract_server_name(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
        .replace('.', "_")
}

/// Encode a server or tool identifier component for ThinClaw tool names.
pub(super) fn encode_tool_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        let ch = byte as char;
        if ch.is_ascii_alphanumeric() || ch == '_' {
            encoded.push(ch.to_ascii_lowercase());
        } else {
            encoded.push('_');
            encoded.push_str(&format!("{byte:02x}"));
        }
    }
    encoded
}

pub(super) fn normalize_root_uri(root: &str) -> String {
    if root.contains("://") {
        return root.to_string();
    }

    url::Url::from_file_path(root)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| root.to_string())
}

pub(super) fn root_name(root: &str) -> String {
    Path::new(root)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(root)
        .to_string()
}

pub(super) fn describe_pending_interaction(
    kind: &McpInteractionKind,
    params: &serde_json::Value,
) -> (String, String, Option<serde_json::Value>) {
    match kind {
        McpInteractionKind::Sampling => {
            let parsed =
                serde_json::from_value::<SamplingCreateMessageRequest>(params.clone()).ok();
            let message_count = parsed
                .as_ref()
                .map(|request| request.messages.len())
                .unwrap_or(0);
            let system_prompt = parsed
                .as_ref()
                .and_then(|request| request.system_prompt.as_deref())
                .filter(|prompt| !prompt.trim().is_empty())
                .map(str::to_string);
            let title = if let Some(system_prompt) = system_prompt.as_deref() {
                format!("Sampling request: {}", truncate_label(system_prompt, 48))
            } else {
                "Sampling request".to_string()
            };
            let description = if message_count > 0 {
                format!(
                    "Server requested an assistant message from {} input messages.",
                    message_count
                )
            } else {
                "Server requested an assistant message from the client.".to_string()
            };
            (title, description, None)
        }
        McpInteractionKind::Elicitation => {
            let parsed = serde_json::from_value::<ElicitationCreateRequest>(params.clone()).ok();
            let title = parsed
                .as_ref()
                .and_then(|request| request.title.as_deref())
                .filter(|title| !title.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| "Form input requested".to_string());
            let description = parsed
                .as_ref()
                .and_then(|request| {
                    request
                        .instructions
                        .as_deref()
                        .or(request.message.as_deref())
                        .filter(|text| !text.trim().is_empty())
                })
                .map(str::to_string)
                .unwrap_or_else(|| "Server requested structured user input.".to_string());
            let schema = parsed.and_then(|request| request.requested_schema);
            (title, description, schema)
        }
    }
}

pub(super) fn truncate_label(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    let cutoff = value
        .char_indices()
        .nth(max_chars.saturating_sub(1))
        .map(|(idx, _)| idx)
        .unwrap_or(value.len());
    format!("{}...", &value[..cutoff])
}
