//! Document extraction tool for agent-initiated text extraction.
//!
//! Allows the agent to extract text from documents by URL or base64 data.
//! Supports PDF, DOCX, PPTX, XLSX, and plain text formats.

use async_trait::async_trait;
use futures::StreamExt as _;

use thinclaw_media::document_extraction::extractors;
use thinclaw_media::document_extraction::{MAX_DOCUMENT_SIZE, MAX_EXTRACTED_TEXT_LEN};
use thinclaw_tools_core::{
    OutboundUrlGuardOptions, Tool, ToolDomain, ToolError, ToolOutput,
    validate_outbound_url_pinned_async,
};
use thinclaw_types::JobContext;

/// Tool that lets the agent extract text from documents.
///
/// Accepts either a URL to fetch or base64-encoded document data.
/// Returns the extracted plain text content.
pub struct ExtractDocumentTool;

#[async_trait]
impl Tool for ExtractDocumentTool {
    fn name(&self) -> &str {
        "extract_document"
    }

    fn description(&self) -> &str {
        "Extract text from document files such as PDF, DOCX, PPTX, XLSX, and plain-text \
         formats. Use this when the user gives you a document and you need readable text \
         before summarizing, searching, or analyzing its contents."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL of the document to download and extract text from. Provide either 'url' OR both 'data' and 'mime_type'."
                },
                "data": {
                    "type": "string",
                    "description": "Base64-encoded document data (use when you already have the file bytes). Must be paired with 'mime_type'."
                },
                "mime_type": {
                    "type": "string",
                    "description": "MIME type of the document (e.g. 'application/pdf'). Required when using 'data'. Auto-detected for URLs."
                },
                "filename": {
                    "type": "string",
                    "description": "Original filename (used for type detection when MIME is ambiguous)"
                }
            }
        })
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let url = params.get("url").and_then(|v| v.as_str());
        let data_b64 = params.get("data").and_then(|v| v.as_str());
        let mime_type = params.get("mime_type").and_then(|v| v.as_str());
        let filename = params.get("filename").and_then(|v| v.as_str());

        // Get document bytes: either fetch from URL or decode base64
        let (bytes, resolved_mime) = if let Some(url) = url {
            fetch_document(url).await?
        } else if let Some(b64) = data_b64 {
            let mime = mime_type.ok_or_else(|| {
                ToolError::InvalidParameters("'mime_type' is required when using 'data'".into())
            })?;
            let bytes = decode_base64(b64)?;
            (bytes, mime.to_string())
        } else {
            return Err(ToolError::InvalidParameters(
                "Either 'url' or 'data' must be provided".into(),
            ));
        };

        // Size guard
        if bytes.len() > MAX_DOCUMENT_SIZE {
            return Err(ToolError::ExecutionFailed(format!(
                "Document too large: {} bytes (max: {} bytes / {} MB)",
                bytes.len(),
                MAX_DOCUMENT_SIZE,
                MAX_DOCUMENT_SIZE / (1024 * 1024)
            )));
        }

        // Extract text
        let mut text = extractors::extract_text(&bytes, &resolved_mime, filename)
            .map_err(ToolError::ExecutionFailed)?;

        // Truncate if needed
        if text.len() > MAX_EXTRACTED_TEXT_LEN {
            text.truncate(MAX_EXTRACTED_TEXT_LEN);
            text.push_str("\n\n[... text truncated ...]");
        }

        let label = filename.unwrap_or("document");
        let mime_display = &resolved_mime;
        let output = format!(
            "[Extracted from {} ({}, {} chars)]\n\n{}",
            label,
            mime_display,
            text.len(),
            text
        );

        Ok(ToolOutput::text(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true // External content should be sanitized
    }
}

/// Fetch a document from a URL.
async fn fetch_document(url: &str) -> Result<(Vec<u8>, String), ToolError> {
    const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    const MAX_REDIRECTS: usize = 10;
    const MAX_URL_BYTES: usize = 16 * 1024;

    if url.len() > MAX_URL_BYTES {
        return Err(ToolError::InvalidParameters(format!(
            "Document URL exceeds the {MAX_URL_BYTES}-byte limit"
        )));
    }
    let guard_options = OutboundUrlGuardOptions {
        require_https: true,
        upgrade_http_to_https: true,
        allowlist: Vec::new(),
    };
    let deadline = tokio::time::Instant::now() + FETCH_TIMEOUT;
    let mut current = url.to_string();
    for redirect_count in 0..=MAX_REDIRECTS {
        let guarded = tokio::time::timeout_at(
            deadline,
            validate_outbound_url_pinned_async(&current, &guard_options),
        )
        .await
        .map_err(|_| ToolError::Timeout(FETCH_TIMEOUT))??;
        let guarded_url = guarded.url;
        let host = guarded_url
            .host_str()
            .ok_or_else(|| ToolError::InvalidParameters("Document URL has no host".to_string()))?
            .to_string();
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .ok_or(ToolError::Timeout(FETCH_TIMEOUT))?;
        let mut builder = reqwest::Client::builder()
            .timeout(remaining)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        if !guarded.pinned_addrs.is_empty() {
            builder = builder.resolve_to_addrs(&host, &guarded.pinned_addrs);
        }
        let client = builder
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("HTTP client error: {e}")))?;
        let response = client
            .get(guarded_url.clone())
            .header("User-Agent", "ThinClaw/1.0")
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ToolError::Timeout(FETCH_TIMEOUT)
                } else {
                    ToolError::ExecutionFailed(format!(
                        "Failed to fetch document: {}",
                        e.without_url()
                    ))
                }
            })?;

        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err(ToolError::ExecutionFailed(format!(
                    "Document fetch exceeded {MAX_REDIRECTS} redirects"
                )));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    ToolError::ExecutionFailed(
                        "Document redirect has no valid Location header".to_string(),
                    )
                })?;
            let target = guarded_url.join(location).map_err(|error| {
                ToolError::ExecutionFailed(format!("Invalid document redirect: {error}"))
            })?;
            if target.as_str().len() > MAX_URL_BYTES {
                return Err(ToolError::ExecutionFailed(
                    "Document redirect URL is oversized".to_string(),
                ));
            }
            current = target.to_string();
            continue;
        }
        if !response.status().is_success() {
            return Err(ToolError::ExecutionFailed(format!(
                "HTTP {} fetching document",
                response.status()
            )));
        }
        if response.content_length().is_some_and(|length| {
            usize::try_from(length).map_or(true, |length| length > MAX_DOCUMENT_SIZE)
        }) {
            return Err(ToolError::ExecutionFailed(format!(
                "Document exceeds maximum size of {MAX_DOCUMENT_SIZE} bytes"
            )));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.split(';').next().unwrap_or(value).trim().to_string())
            .unwrap_or_else(|| guess_mime_from_url(guarded_url.as_str()));
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "Failed to read document body: {}",
                    error.without_url()
                ))
            })?;
            if bytes.len().saturating_add(chunk.len()) > MAX_DOCUMENT_SIZE {
                return Err(ToolError::ExecutionFailed(format!(
                    "Document exceeds maximum size of {MAX_DOCUMENT_SIZE} bytes"
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok((bytes, content_type));
    }

    Err(ToolError::ExecutionFailed(
        "Document redirect loop terminated unexpectedly".to_string(),
    ))
}

/// Decode base64-encoded data.
fn decode_base64(b64: &str) -> Result<Vec<u8>, ToolError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| ToolError::InvalidParameters(format!("Invalid base64 data: {e}")))
}

/// Guess MIME type from URL path.
fn guess_mime_from_url(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".pdf") {
        "application/pdf".to_string()
    } else if lower.ends_with(".docx") {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document".to_string()
    } else if lower.ends_with(".pptx") {
        "application/vnd.openxmlformats-officedocument.presentationml.presentation".to_string()
    } else if lower.ends_with(".xlsx") {
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string()
    } else if lower.ends_with(".txt") || lower.ends_with(".md") || lower.ends_with(".csv") {
        "text/plain".to_string()
    } else if lower.ends_with(".json") {
        "application/json".to_string()
    } else {
        "application/octet-stream".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guess_mime_from_url() {
        assert_eq!(
            guess_mime_from_url("https://example.com/report.pdf"),
            "application/pdf"
        );
        assert_eq!(
            guess_mime_from_url("https://example.com/doc.docx?token=abc"),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(
            guess_mime_from_url("https://example.com/data.json"),
            "application/json"
        );
        assert_eq!(
            guess_mime_from_url("https://example.com/unknown"),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_decode_base64_valid() {
        let encoded =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"Hello, World!");
        let decoded = decode_base64(&encoded).unwrap();
        assert_eq!(decoded, b"Hello, World!");
    }

    #[test]
    fn test_decode_base64_invalid() {
        assert!(decode_base64("not-valid-base64!!!").is_err());
    }

    #[tokio::test]
    async fn test_extract_document_missing_params() {
        let tool = ExtractDocumentTool;
        let ctx = JobContext::default();
        let result = tool.execute(serde_json::json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_extract_document_base64_text() {
        let tool = ExtractDocumentTool;
        let ctx = JobContext::default();
        let data = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            b"Hello from a text file",
        );
        let result = tool
            .execute(
                serde_json::json!({
                    "data": data,
                    "mime_type": "text/plain",
                    "filename": "test.txt"
                }),
                &ctx,
            )
            .await
            .unwrap();
        let text = result.result.as_str().unwrap_or("");
        assert!(text.contains("Hello from a text file"));
        assert!(text.contains("test.txt"));
    }

    #[tokio::test]
    async fn test_extract_document_base64_missing_mime() {
        let tool = ExtractDocumentTool;
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({
                    "data": "aGVsbG8="
                }),
                &ctx,
            )
            .await;
        assert!(result.is_err());
    }
}
