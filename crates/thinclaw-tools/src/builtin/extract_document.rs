//! Document extraction tool for agent-initiated text extraction.
//!
//! Allows the agent to extract text from documents by URL or base64 data.
//! Supports PDF, DOCX, PPTX, XLSX, and plain text formats.

use async_trait::async_trait;

use thinclaw_media::document_extraction::extractors;
use thinclaw_media::document_extraction::{MAX_DOCUMENT_SIZE, MAX_EXTRACTED_TEXT_LEN};
use thinclaw_tools_core::{
    OutboundUrlGuardOptions, Tool, ToolDomain, ToolError, ToolOutput, validate_outbound_url,
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
    let guard_options = OutboundUrlGuardOptions {
        require_https: false,
        upgrade_http_to_https: false,
        allowlist: Vec::new(),
    };
    let guarded_url = validate_outbound_url(url, &guard_options)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if validate_outbound_url(attempt.url().as_str(), &guard_options).is_ok() {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|e| ToolError::ExecutionFailed(format!("HTTP client error: {e}")))?;

    let response = client
        .get(guarded_url.clone())
        .header("User-Agent", "ThinClaw/1.0")
        .send()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch document: {e}")))?;

    if !response.status().is_success() {
        return Err(ToolError::ExecutionFailed(format!(
            "HTTP {} fetching document",
            response.status()
        )));
    }

    // Get MIME type from Content-Type header
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.split(';').next().unwrap_or(ct).trim().to_string())
        .unwrap_or_else(|| guess_mime_from_url(guarded_url.as_str()));

    let bytes = response
        .bytes()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response body: {e}")))?;

    Ok((bytes.to_vec(), content_type))
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
