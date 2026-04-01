//! Document text extraction for the media pipeline.
//!
//! Bridges the `document_extraction` module into the `MediaPipeline` extractor
//! system so that document attachments (DOCX, PPTX, XLSX, text files) are
//! automatically extracted when received through channels.

use super::types::{MediaContent, MediaExtractError, MediaExtractor, MediaType};

/// Extracts text from document attachments (DOCX, PPTX, XLSX, text files).
///
/// Delegates to the `document_extraction::extractors` module for the actual
/// content parsing. Registered in `MediaPipeline::new()` alongside the
/// image, PDF, and audio extractors.
pub struct DocumentExtractor {
    /// Maximum document size in bytes (default: 10 MB).
    max_size: usize,
    /// Maximum extracted text length (default: 100K chars).
    max_text_length: usize,
}

impl DocumentExtractor {
    /// Create a new document extractor with defaults from the document_extraction module.
    pub fn new() -> Self {
        Self {
            max_size: crate::document_extraction::MAX_DOCUMENT_SIZE,
            max_text_length: crate::document_extraction::MAX_EXTRACTED_TEXT_LEN,
        }
    }
}

impl Default for DocumentExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaExtractor for DocumentExtractor {
    fn supported_types(&self) -> &[MediaType] {
        &[MediaType::Document]
    }

    fn extract_text(&self, content: &MediaContent) -> Result<String, MediaExtractError> {
        if content.size() > self.max_size {
            return Err(MediaExtractError::TooLarge {
                size: content.size(),
                max: self.max_size,
            });
        }

        let filename = content.filename.as_deref();

        let mut text = crate::document_extraction::extractors::extract_text(
            &content.data,
            &content.mime_type,
            filename,
        )
        .map_err(|e| MediaExtractError::ExtractionFailed { reason: e })?;

        // Truncate if needed
        if text.len() > self.max_text_length {
            text.truncate(self.max_text_length);
            text.push_str("\n\n[... text truncated ...]");
        }

        let label = filename.unwrap_or("document");

        Ok(format!(
            "[Document: {} — {} chars extracted]\n\n{}",
            label,
            text.len(),
            text
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supported_types() {
        let extractor = DocumentExtractor::new();
        assert_eq!(extractor.supported_types(), &[MediaType::Document]);
    }

    #[test]
    fn test_extract_text_file() {
        let content = MediaContent::new(b"Hello from text".to_vec(), "text/plain")
            .with_filename("notes.txt".to_string());
        let extractor = DocumentExtractor::new();
        let result = extractor.extract_text(&content).unwrap();
        assert!(result.contains("Hello from text"));
        assert!(result.contains("notes.txt"));
    }

    #[test]
    fn test_extract_json_file() {
        let content = MediaContent::new(
            br#"{"key": "value"}"#.to_vec(),
            "application/json",
        )
        .with_filename("data.json".to_string());
        let extractor = DocumentExtractor::new();
        let result = extractor.extract_text(&content).unwrap();
        assert!(result.contains("key"));
        assert!(result.contains("value"));
    }

    #[test]
    fn test_too_large() {
        let extractor = DocumentExtractor {
            max_size: 10,
            max_text_length: 100,
        };
        let content = MediaContent::new(vec![0; 100], "text/plain");
        assert!(matches!(
            extractor.extract_text(&content),
            Err(MediaExtractError::TooLarge { .. })
        ));
    }
}
