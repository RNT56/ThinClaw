//! Vision analysis tool.
//!
//! Allows the agent to proactively analyze an image file by path or URL
//! using the current multimodal LLM provider. This is distinct from the
//! passive `MediaPipeline` which processes incoming attachments.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::prelude::FromPrimitive;

use crate::context::JobContext;
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};
use crate::media::MediaContent;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};

/// Maximum image file size (10MB).
const MAX_IMAGE_SIZE: u64 = 10 * 1024 * 1024;

/// Supported image MIME types.
const SUPPORTED_MIMES: &[(&str, &[u8])] = &[
    ("image/jpeg", &[0xFF, 0xD8, 0xFF]),
    ("image/png", &[0x89, 0x50, 0x4E, 0x47]),
    ("image/gif", &[0x47, 0x49, 0x46]),
    ("image/webp", &[0x52, 0x49, 0x46, 0x46]),
];

/// Detect MIME type from magic bytes.
fn detect_mime(bytes: &[u8]) -> Option<&'static str> {
    for (mime, magic) in SUPPORTED_MIMES {
        if bytes.len() >= magic.len() && bytes[..magic.len()] == **magic {
            return Some(mime);
        }
    }
    // WebP has RIFF header then WEBP at offset 8
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// Tool for proactive image analysis using the multimodal LLM.
pub struct VisionAnalyzeTool {
    llm: Arc<dyn LlmProvider>,
}

impl VisionAnalyzeTool {
    pub fn new(llm: Arc<dyn LlmProvider>) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl Tool for VisionAnalyzeTool {
    fn name(&self) -> &str {
        "vision_analyze"
    }

    fn description(&self) -> &str {
        "Analyze an image using the multimodal LLM. Provide either a local file path \
         (image_path) or a URL (image_url). Optionally provide a custom analysis prompt. \
         Returns the LLM's textual analysis of the image."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "image_path": {
                    "type": "string",
                    "description": "Local file path to the image to analyze"
                },
                "image_url": {
                    "type": "string",
                    "description": "URL of the image to analyze (alternative to image_path)"
                },
                "prompt": {
                    "type": "string",
                    "description": "Analysis prompt (default: 'Describe this image in detail, including all visible text, objects, colors, and layout.')"
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let image_path = params.get("image_path").and_then(|v| v.as_str());
        let image_url = params.get("image_url").and_then(|v| v.as_str());
        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("Describe this image in detail, including all visible text, objects, colors, and layout.");

        if image_path.is_none() && image_url.is_none() {
            return Err(ToolError::InvalidParameters(
                "Provide either 'image_path' (local file) or 'image_url' (remote URL)".to_string(),
            ));
        }

        // Load image bytes
        let (image_bytes, source_desc) = if let Some(path_str) = image_path {
            let path = Path::new(path_str);
            if !path.exists() {
                return Err(ToolError::ExecutionFailed(format!(
                    "Image file not found: {}",
                    path_str
                )));
            }

            let metadata = tokio::fs::metadata(path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Cannot access file: {}", e)))?;

            if metadata.len() > MAX_IMAGE_SIZE {
                return Err(ToolError::ExecutionFailed(format!(
                    "Image too large ({} bytes, max {} bytes)",
                    metadata.len(),
                    MAX_IMAGE_SIZE
                )));
            }

            let bytes = tokio::fs::read(path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read image: {}", e)))?;

            (bytes, path_str.to_string())
        } else if let Some(url) = image_url {
            // Fetch from URL
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|e| ToolError::ExecutionFailed(format!("HTTP client error: {}", e)))?;

            let response =
                client.get(url).send().await.map_err(|e| {
                    ToolError::ExecutionFailed(format!("Failed to fetch image: {}", e))
                })?;

            if !response.status().is_success() {
                return Err(ToolError::ExecutionFailed(format!(
                    "HTTP {} fetching image from {}",
                    response.status(),
                    url
                )));
            }

            let bytes = response.bytes().await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to read response: {}", e))
            })?;

            if bytes.len() as u64 > MAX_IMAGE_SIZE {
                return Err(ToolError::ExecutionFailed(format!(
                    "Image too large ({} bytes)",
                    bytes.len()
                )));
            }

            (bytes.to_vec(), url.to_string())
        } else {
            unreachable!()
        };

        // Validate image format
        let mime = detect_mime(&image_bytes).ok_or_else(|| {
            ToolError::InvalidParameters(
                "Unsupported image format. Supported: JPEG, PNG, GIF, WebP".to_string(),
            )
        })?;

        let attachment = if let Some(path_str) = image_path {
            let filename = Path::new(path_str)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("image");
            MediaContent::new(image_bytes.clone(), mime).with_filename(filename)
        } else if let Some(url) = image_url {
            MediaContent::new(image_bytes.clone(), mime).with_source_url(url)
        } else {
            MediaContent::new(image_bytes.clone(), mime)
        };

        // Send to LLM for analysis
        let messages = vec![ChatMessage::user(prompt).with_attachments(vec![attachment])];

        let response = self
            .llm
            .complete(CompletionRequest::new(messages))
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("LLM analysis failed: {}", e)))?;

        // Extract text response
        let analysis_text = if response.content.trim().is_empty() {
            "No analysis produced".to_string()
        } else {
            response.content
        };

        let result = serde_json::json!({
            "analysis": analysis_text,
            "source": source_desc,
            "mime_type": mime,
            "image_size_bytes": image_bytes.len(),
        });

        let mut output = ToolOutput::success(result, start.elapsed());
        if let Some(cost) = response.cost_usd.and_then(rust_decimal::Decimal::from_f64) {
            output = output.with_cost(cost);
        }
        Ok(output)
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_mime_jpeg() {
        let bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_mime(&bytes), Some("image/jpeg"));
    }

    #[test]
    fn test_detect_mime_png() {
        let bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A];
        assert_eq!(detect_mime(&bytes), Some("image/png"));
    }

    #[test]
    fn test_detect_mime_gif() {
        let bytes = b"GIF89a".to_vec();
        assert_eq!(detect_mime(&bytes), Some("image/gif"));
    }

    #[test]
    fn test_detect_mime_webp() {
        let mut bytes = vec![0u8; 12];
        bytes[..4].copy_from_slice(b"RIFF");
        bytes[8..12].copy_from_slice(b"WEBP");
        assert_eq!(detect_mime(&bytes), Some("image/webp"));
    }

    #[test]
    fn test_detect_mime_unknown() {
        let bytes = vec![0x00, 0x01, 0x02, 0x03];
        assert_eq!(detect_mime(&bytes), None);
    }

    #[test]
    fn test_detect_mime_too_short() {
        let bytes = vec![0xFF];
        assert_eq!(detect_mime(&bytes), None);
    }
}
