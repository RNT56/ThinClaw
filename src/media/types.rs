//! Core media extraction traits and pipeline.

pub use thinclaw_types::media::{MediaContent, MediaType};

/// Error type for media extraction.
#[derive(Debug, thiserror::Error)]
pub enum MediaExtractError {
    #[error("Unsupported media type: {media_type}")]
    UnsupportedType { media_type: String },

    #[error("Extraction failed: {reason}")]
    ExtractionFailed { reason: String },

    #[error("HTTP fetch failed: {reason}")]
    FetchFailed { reason: String },

    #[error("Content too large: {size} bytes (max: {max})")]
    TooLarge { size: usize, max: usize },
}

/// Trait for extracting text from media content.
pub trait MediaExtractor: Send + Sync {
    /// What media types this extractor handles.
    fn supported_types(&self) -> &[MediaType];

    /// Extract text/description from the media content.
    fn extract_text(&self, content: &MediaContent) -> Result<String, MediaExtractError>;
}

/// Pipeline that routes media content to the appropriate extractor.
pub struct MediaPipeline {
    extractors: Vec<Box<dyn MediaExtractor>>,
    /// Maximum content size in bytes (default: 50 MB).
    max_size: usize,
}

impl MediaPipeline {
    /// Create a new pipeline with default extractors.
    pub fn new() -> Self {
        #[cfg(not(feature = "document-extraction"))]
        let extractors: Vec<Box<dyn MediaExtractor>> = vec![
            Box::new(super::image::ImageExtractor::new()),
            Box::new(super::pdf::PdfExtractor::new()),
            Box::new(super::audio::AudioExtractor::new()),
        ];

        #[cfg(feature = "document-extraction")]
        let mut extractors: Vec<Box<dyn MediaExtractor>> = vec![
            Box::new(super::image::ImageExtractor::new()),
            Box::new(super::pdf::PdfExtractor::new()),
            Box::new(super::audio::AudioExtractor::new()),
        ];

        #[cfg(feature = "document-extraction")]
        extractors.push(Box::new(super::document::DocumentExtractor::new()));

        Self {
            extractors,
            max_size: 50 * 1024 * 1024,
        }
    }

    /// Set the maximum allowed content size.
    pub fn with_max_size(mut self, max_bytes: usize) -> Self {
        self.max_size = max_bytes;
        self
    }

    /// Add a custom extractor.
    pub fn with_extractor(mut self, extractor: Box<dyn MediaExtractor>) -> Self {
        self.extractors.push(extractor);
        self
    }

    /// Process a media attachment and extract text.
    pub fn extract(&self, content: &MediaContent) -> Result<String, MediaExtractError> {
        if content.size() > self.max_size {
            return Err(MediaExtractError::TooLarge {
                size: content.size(),
                max: self.max_size,
            });
        }

        for extractor in &self.extractors {
            if extractor.supported_types().contains(&content.media_type) {
                return extractor.extract_text(content);
            }
        }

        Err(MediaExtractError::UnsupportedType {
            media_type: content.media_type.to_string(),
        })
    }

    /// Process multiple attachments and return all extracted text.
    pub fn extract_all(
        &self,
        attachments: &[MediaContent],
    ) -> Vec<Result<String, MediaExtractError>> {
        attachments.iter().map(|a| self.extract(a)).collect()
    }
}

impl Default for MediaPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_type_from_mime() {
        assert_eq!(MediaType::from_mime("image/jpeg"), MediaType::Image);
        assert_eq!(MediaType::from_mime("image/PNG"), MediaType::Image);
        assert_eq!(MediaType::from_mime("application/pdf"), MediaType::Pdf);
        assert_eq!(MediaType::from_mime("audio/mp3"), MediaType::Audio);
        assert_eq!(MediaType::from_mime("video/mp4"), MediaType::Video);
        assert_eq!(
            MediaType::from_mime("application/json"),
            MediaType::Document
        );
        assert_eq!(MediaType::from_mime("text/plain"), MediaType::Document);
    }

    #[test]
    fn test_media_type_from_extension() {
        assert_eq!(MediaType::from_extension("jpg"), MediaType::Image);
        assert_eq!(MediaType::from_extension("PDF"), MediaType::Pdf);
        assert_eq!(MediaType::from_extension("mp3"), MediaType::Audio);
        assert_eq!(MediaType::from_extension("mp4"), MediaType::Video);
        assert_eq!(MediaType::from_extension("txt"), MediaType::Document);
        assert_eq!(MediaType::from_extension("docx"), MediaType::Document);
        assert_eq!(MediaType::from_extension("csv"), MediaType::Document);
    }

    #[test]
    fn test_media_content_creation() {
        let mc = MediaContent::new(vec![1, 2, 3], "image/png");
        assert_eq!(mc.media_type, MediaType::Image);
        assert_eq!(mc.mime_type, "image/png");
        assert_eq!(mc.size(), 3);
        assert!(mc.filename.is_none());
    }

    #[test]
    fn test_pipeline_too_large() {
        let pipeline = MediaPipeline::new().with_max_size(10);
        let mc = MediaContent::new(vec![0; 20], "image/png");
        let result = pipeline.extract(&mc);
        assert!(matches!(result, Err(MediaExtractError::TooLarge { .. })));
    }

    #[test]
    fn test_pipeline_unsupported_type() {
        let pipeline = MediaPipeline::new();
        let mc = MediaContent::new(vec![1, 2, 3], "video/mp4");
        let result = pipeline.extract(&mc);
        assert!(matches!(
            result,
            Err(MediaExtractError::UnsupportedType { .. })
        ));
    }
}
