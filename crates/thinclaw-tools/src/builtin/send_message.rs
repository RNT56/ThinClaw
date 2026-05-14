//! Unified cross-platform send message tool.
//!
//! Provides a single tool for the agent to send messages across any
//! configured platform (Telegram, Discord, Slack, etc.) without needing
//! to know which platform-specific action tool to use.

use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use thinclaw_media::{MediaContent, MediaLimits, MediaType};
use thinclaw_tools_core::{
    ApprovalRequirement, Tool, ToolError, ToolOutput, ToolRateLimitConfig, require_str,
};
use thinclaw_types::JobContext;

/// Callback for dispatching messages to the gateway's channel infrastructure.
///
/// This is injected at registration time. The callback receives:
/// (platform, recipient, text, thread_id, attachments) and returns either a message ID or error.
pub type SendMessageFn = Arc<
    dyn Fn(
            String,
            String,
            String,
            Option<String>,
            Vec<MediaContent>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

#[cfg(test)]
static TEST_GENERATED_ROOTS: LazyLock<Mutex<Vec<PathBuf>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// Unified cross-platform messaging tool.
pub struct SendMessageTool {
    send_fn: Option<SendMessageFn>,
}

impl SendMessageTool {
    /// Create a new SendMessageTool without a send function (will error on use).
    pub fn new() -> Self {
        Self { send_fn: None }
    }

    /// Create with a gateway send function.
    pub fn with_send_fn(mut self, send_fn: SendMessageFn) -> Self {
        self.send_fn = Some(send_fn);
        self
    }
}

impl Default for SendMessageTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send a message to a user on any connected platform. \
         Specify the platform/channel and recipient. \
         The message will be delivered through the gateway. \
         Optional attachments may reference generated media files by file_path. \
         For Nostr, this tool sends encrypted DMs only; use nostr_actions for public posting or social interaction. \
         For platform-specific features (reactions, polls, etc.), use the \
         dedicated platform action tools instead."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "platform": {
                    "type": "string",
                    "description": "Target platform/channel name (telegram, discord, slack, email, whatsapp, signal, web, etc.)"
                },
                "recipient": {
                    "type": "string",
                    "description": "Recipient identifier (chat_id, channel_id, email address, Nostr pubkey, etc.)"
                },
                "text": {
                    "type": "string",
                    "description": "Message content"
                },
                "thread_id": {
                    "type": "string",
                    "description": "Optional thread/topic ID to reply in"
                },
                "attachments": {
                    "type": "array",
                    "description": "Optional generated media files to attach.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file_path": {"type": "string", "description": "Local generated media file path."},
                            "filename": {"type": "string", "description": "Optional outbound filename override."},
                            "mime_type": {"type": "string", "description": "Optional MIME type override."}
                        },
                        "required": ["file_path"]
                    }
                }
            },
            "required": ["platform", "recipient", "text"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let platform = require_str(&params, "platform")?;
        let recipient = require_str(&params, "recipient")?;
        let text = require_str(&params, "text")?;
        let thread_id = params.get("thread_id").and_then(|v| v.as_str());
        let attachments = parse_attachments(&params).await?;

        if platform.trim().is_empty() {
            return Err(ToolError::InvalidParameters(
                "platform cannot be empty".to_string(),
            ));
        }

        // Check that send function is available (gateway running)
        let send_fn = self.send_fn.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "Message gateway not available. \
                 Ensure the gateway is running and the target channel is configured."
                    .to_string(),
            )
        })?;

        // Dispatch the message
        let message_id = send_fn(
            platform.to_string(),
            recipient.to_string(),
            text.to_string(),
            thread_id.map(String::from),
            attachments,
        )
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to send message: {}", e)))?;

        let result = serde_json::json!({
            "success": true,
            "platform": platform,
            "recipient": recipient,
            "message_id": message_id,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(10, 60))
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

async fn parse_attachments(params: &serde_json::Value) -> Result<Vec<MediaContent>, ToolError> {
    let Some(items) = params.get("attachments") else {
        return Ok(Vec::new());
    };
    let items = items
        .as_array()
        .ok_or_else(|| ToolError::InvalidParameters("attachments must be an array".to_string()))?;
    let mut attachments = Vec::new();
    let max_bytes = MediaLimits::from_env().default_max_bytes;
    for item in items {
        let path = item
            .get("file_path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("attachments[].file_path is required".to_string())
            })?;
        let canonical = canonical_generated_media_path(path).await?;
        let metadata = tokio::fs::metadata(&canonical).await.map_err(|e| {
            ToolError::InvalidParameters(format!(
                "cannot stat attachment {}: {e}",
                canonical.display()
            ))
        })?;
        if !metadata.is_file() {
            return Err(ToolError::InvalidParameters(format!(
                "attachment path is not a file: {}",
                canonical.display()
            )));
        }
        if metadata.len() > max_bytes {
            return Err(ToolError::InvalidParameters(format!(
                "attachment {} exceeds {} bytes",
                canonical.display(),
                max_bytes
            )));
        }
        let data = tokio::fs::read(&canonical).await.map_err(|e| {
            ToolError::InvalidParameters(format!(
                "cannot read attachment {}: {e}",
                canonical.display()
            ))
        })?;
        let filename = item
            .get("filename")
            .and_then(|value| value.as_str())
            .and_then(safe_basename)
            .or_else(|| {
                canonical
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "attachment".to_string());
        let mime_type = item
            .get("mime_type")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                mime_guess::from_path(&canonical)
                    .first_or_octet_stream()
                    .essence_str()
                    .to_string()
            });
        if MediaType::from_mime(&mime_type) == MediaType::Unknown {
            return Err(ToolError::InvalidParameters(format!(
                "unsupported attachment MIME type '{}'",
                mime_type
            )));
        }
        attachments.push(
            MediaContent::new(data, mime_type)
                .with_filename(filename)
                .with_source_url(canonical.to_string_lossy().to_string()),
        );
    }
    Ok(attachments)
}

async fn canonical_generated_media_path(path: &str) -> Result<PathBuf, ToolError> {
    let canonical = tokio::fs::canonicalize(path).await.map_err(|e| {
        ToolError::InvalidParameters(format!("invalid attachment file_path '{}': {e}", path))
    })?;
    for root in approved_generated_roots() {
        if let Ok(canonical_root) = tokio::fs::canonicalize(root).await
            && canonical.starts_with(&canonical_root)
        {
            return Ok(canonical);
        }
    }

    Err(ToolError::InvalidParameters(
        "attachment path must be under an approved generated media directory".to_string(),
    ))
}

fn safe_basename(value: &str) -> Option<String> {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .map(str::to_string)
}

fn approved_generated_roots() -> Vec<PathBuf> {
    let mut roots = vec![thinclaw_platform::resolve_data_dir("media_cache").join("generated")];

    if let Ok(extra_roots) = std::env::var("THINCLAW_GENERATED_MEDIA_ROOTS") {
        roots.extend(
            extra_roots
                .split(',')
                .map(str::trim)
                .filter(|root| !root.is_empty())
                .map(PathBuf::from),
        );
    }

    #[cfg(test)]
    roots.extend(
        TEST_GENERATED_ROOTS
            .lock()
            .expect("test roots lock")
            .clone(),
    );

    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_send_message_no_gateway() {
        let tool = SendMessageTool::new();
        let ctx = JobContext::default();

        let err = tool
            .execute(
                serde_json::json!({
                    "platform": "telegram",
                    "recipient": "12345",
                    "text": "hello"
                }),
                &ctx,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("gateway not available"));
    }

    #[tokio::test]
    async fn test_send_message_empty_platform() {
        let tool = SendMessageTool::new();
        let ctx = JobContext::default();

        let err = tool
            .execute(
                serde_json::json!({
                    "platform": "",
                    "recipient": "user",
                    "text": "hello"
                }),
                &ctx,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("platform cannot be empty"));
    }

    #[tokio::test]
    async fn test_send_message_missing_params() {
        let tool = SendMessageTool::new();
        let ctx = JobContext::default();

        let err = tool
            .execute(serde_json::json!({"platform": "telegram"}), &ctx)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("missing"));
    }

    #[tokio::test]
    async fn test_send_message_with_mock() {
        let send_fn: SendMessageFn =
            Arc::new(|platform, recipient, _text, _thread, _attachments| {
                Box::pin(async move { Ok(format!("msg_{}_{}", platform, recipient)) })
            });

        let tool = SendMessageTool::new().with_send_fn(send_fn);
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "platform": "telegram",
                    "recipient": "12345",
                    "text": "Hello from unified tool!"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
        assert_eq!(
            result.result.get("message_id").unwrap().as_str().unwrap(),
            "msg_telegram_12345"
        );
    }

    #[tokio::test]
    async fn test_send_message_with_thread() {
        let send_fn: SendMessageFn =
            Arc::new(|_platform, _recipient, _text, thread, _attachments| {
                Box::pin(async move {
                    assert!(thread.is_some());
                    Ok("msg_threaded".to_string())
                })
            });

        let tool = SendMessageTool::new().with_send_fn(send_fn);
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "platform": "discord",
                    "recipient": "channel_123",
                    "text": "thread reply",
                    "thread_id": "thread_456"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_send_message_with_generated_attachment() {
        let generated_root = tempfile::tempdir().expect("generated root");
        let image_path = generated_root.path().join("image.png");
        tokio::fs::write(&image_path, b"image-bytes").await.unwrap();

        TEST_GENERATED_ROOTS
            .lock()
            .expect("test roots lock")
            .push(generated_root.path().to_path_buf());

        let send_fn: SendMessageFn =
            Arc::new(|_platform, _recipient, _text, _thread, attachments| {
                Box::pin(async move {
                    assert_eq!(attachments.len(), 1);
                    assert_eq!(attachments[0].data, b"image-bytes");
                    assert_eq!(attachments[0].filename.as_deref(), Some("rendered.png"));
                    assert_eq!(attachments[0].mime_type, "image/png");
                    Ok("msg_with_attachment".to_string())
                })
            });

        let tool = SendMessageTool::new().with_send_fn(send_fn);
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "platform": "discord",
                    "recipient": "channel_123",
                    "text": "generated",
                    "attachments": [
                        {
                            "file_path": image_path,
                            "filename": "../rendered.png",
                            "mime_type": "image/png"
                        }
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_send_message_rejects_unapproved_attachment_path() {
        let generated_root = tempfile::tempdir().expect("generated root");
        let outside = tempfile::NamedTempFile::new().expect("outside file");
        tokio::fs::write(outside.path(), b"outside").await.unwrap();

        TEST_GENERATED_ROOTS
            .lock()
            .expect("test roots lock")
            .push(generated_root.path().to_path_buf());

        let send_fn: SendMessageFn =
            Arc::new(|_platform, _recipient, _text, _thread, _attachments| {
                Box::pin(async move { Ok("unexpected".to_string()) })
            });

        let tool = SendMessageTool::new().with_send_fn(send_fn);
        let ctx = JobContext::default();
        let err = tool
            .execute(
                serde_json::json!({
                    "platform": "discord",
                    "recipient": "channel_123",
                    "text": "generated",
                    "attachments": [
                        {
                            "file_path": outside.path()
                        }
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("approved generated media directory")
        );
    }

    #[tokio::test]
    async fn test_send_message_rejects_unknown_attachment_mime() {
        let generated_root = tempfile::tempdir().expect("generated root");
        let file_path = generated_root.path().join("image.unknown");
        tokio::fs::write(&file_path, b"image-bytes").await.unwrap();
        TEST_GENERATED_ROOTS
            .lock()
            .expect("test roots lock")
            .push(generated_root.path().to_path_buf());

        let send_fn: SendMessageFn =
            Arc::new(|_platform, _recipient, _text, _thread, _attachments| {
                Box::pin(async move { Ok("unexpected".to_string()) })
            });

        let tool = SendMessageTool::new().with_send_fn(send_fn);
        let ctx = JobContext::default();
        let err = tool
            .execute(
                serde_json::json!({
                    "platform": "discord",
                    "recipient": "channel_123",
                    "text": "generated",
                    "attachments": [
                        {
                            "file_path": file_path,
                            "mime_type": "application/x-unknown"
                        }
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("unsupported attachment MIME"));
    }

    #[test]
    fn test_approval_required() {
        let tool = SendMessageTool::new();
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }
}
