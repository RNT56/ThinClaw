//! Snapshot tests for core data structures using `insta`.
//!
//! These tests capture the serialization format of key types so that
//! accidental changes to their shape (field renames, default changes,
//! new required fields) are caught during `cargo test`.
//!
//! Run `cargo insta review` to interactively accept/reject changes.

#[cfg(test)]
mod tests {
    use insta::{assert_json_snapshot, assert_yaml_snapshot};

    // ── Settings ────────────────────────────────────────────────────────

    #[test]
    fn test_settings_default_snapshot() {
        let settings = thinclaw::settings::Settings::default();
        assert_json_snapshot!("settings_default", settings);
    }

    // ── Safety types ────────────────────────────────────────────────────

    #[test]
    fn test_validation_result_valid() {
        let result = thinclaw::safety::ValidationResult {
            is_valid: true,
            errors: vec![],
            warnings: vec![],
        };
        assert_yaml_snapshot!("validation_result_valid", result);
    }

    #[test]
    fn test_validation_result_invalid() {
        let result = thinclaw::safety::ValidationResult {
            is_valid: false,
            errors: vec![thinclaw::safety::ValidationError {
                field: "content".to_string(),
                message: "Input too long".to_string(),
                code: thinclaw::safety::ValidationErrorCode::TooLong,
            }],
            warnings: vec![],
        };
        assert_yaml_snapshot!("validation_result_invalid", result);
    }

    // ── MediaType ───────────────────────────────────────────────────────

    #[test]
    fn test_media_type_display() {
        use thinclaw::media::MediaType;
        let types = vec![
            MediaType::Image,
            MediaType::Pdf,
            MediaType::Audio,
            MediaType::Video,
            MediaType::Document,
            MediaType::Unknown,
        ];
        let display: Vec<String> = types.iter().map(|t| t.to_string()).collect();
        assert_yaml_snapshot!("media_type_display", display);
    }

    // ── Timezone ────────────────────────────────────────────────────────

    #[test]
    fn test_timezone_resolve_defaults() {
        let tz = thinclaw::timezone::resolve_timezone(None, None, "UTC");
        assert_yaml_snapshot!("timezone_default", tz.to_string());
    }

    // ── Document extraction tool schema ─────────────────────────────────

    #[cfg(feature = "document-extraction")]
    #[test]
    fn test_extract_document_tool_schema() {
        use thinclaw::tools::Tool;
        let tool = thinclaw::tools::builtin::ExtractDocumentTool;
        let schema = tool.parameters_schema();
        assert_json_snapshot!("extract_document_schema", schema);
    }

    // ── Error types ────────────────────────────────────────────────────

    #[test]
    fn test_database_error_display() {
        use thinclaw::error::DatabaseError;
        let errors: Vec<DatabaseError> = vec![
            DatabaseError::Pool("connection refused".to_string()),
            DatabaseError::Query("syntax error".to_string()),
            DatabaseError::Migration("version mismatch".to_string()),
            DatabaseError::Serialization("invalid json".to_string()),
        ];
        let display: Vec<String> = errors
            .iter()
            .map(|e: &DatabaseError| e.to_string())
            .collect();
        assert_yaml_snapshot!("database_error_display", display);
    }

    #[test]
    fn test_channel_error_display() {
        use thinclaw::error::ChannelError;
        let errors: Vec<ChannelError> = vec![
            ChannelError::StartupFailed {
                name: "telegram".to_string(),
                reason: "timeout".to_string(),
            },
            ChannelError::SendFailed {
                name: "slack".to_string(),
                reason: "rate limited".to_string(),
            },
            ChannelError::MessageTooLong {
                channel: "telegram".to_string(),
                length: 5000,
                max: 3800,
            },
        ];
        let display: Vec<String> = errors
            .iter()
            .map(|e: &ChannelError| e.to_string())
            .collect();
        assert_yaml_snapshot!("channel_error_display", display);
    }
}
