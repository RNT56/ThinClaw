//! Core media extraction trait and error type.

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
