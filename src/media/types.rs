//! Core media types and extraction trait.

use std::fmt;
use std::path::Path;

/// Broad media category inferred from MIME or extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaType {
    Image,
    Pdf,
    Audio,
    Video,
    /// Office documents (DOCX, PPTX, XLSX) and plain text files.
    Document,
    Unknown,
}

impl MediaType {
    /// Detect media type from a MIME string (e.g. `image/jpeg`).
    pub fn from_mime(mime: &str) -> Self {
        let mime = mime.to_ascii_lowercase();
        if mime.starts_with("image/") {
            Self::Image
        } else if mime == "application/pdf" {
            Self::Pdf
        } else if mime.starts_with("audio/") {
            Self::Audio
        } else if mime.starts_with("video/") {
            Self::Video
        } else if is_document_mime(&mime) {
            Self::Document
        } else {
            Self::Unknown
        }
    }

    /// Detect media type from a file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "tiff" | "ico" => Self::Image,
            "pdf" => Self::Pdf,
            "wav" | "mp3" | "m4a" | "ogg" | "flac" | "aac" | "wma" | "opus" => Self::Audio,
            "mp4" | "avi" | "mkv" | "mov" | "webm" | "flv" => Self::Video,
            "docx" | "pptx" | "xlsx" | "txt" | "csv" | "json" | "xml" | "yaml" | "yml"
            | "toml" | "md" | "markdown" | "rs" | "py" | "js" | "ts" | "go" | "java" | "c"
            | "cpp" | "h" | "rb" | "sh" | "sql" | "html" | "css" | "ini" | "cfg" | "conf"
            | "log" => Self::Document,
            _ => Self::Unknown,
        }
    }

    /// Detect media type from a filename (uses extension).
    pub fn from_filename(filename: &str) -> Self {
        Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .map(Self::from_extension)
            .unwrap_or(Self::Unknown)
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image => write!(f, "image"),
            Self::Pdf => write!(f, "pdf"),
            Self::Audio => write!(f, "audio"),
            Self::Video => write!(f, "video"),
            Self::Document => write!(f, "document"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// A media attachment received from a channel.
#[derive(Debug, Clone)]
pub struct MediaContent {
    /// Broad category.
    pub media_type: MediaType,
    /// Raw bytes of the content.
    pub data: Vec<u8>,
    /// MIME type (e.g. `image/png`).
    pub mime_type: String,
    /// Optional original filename.
    pub filename: Option<String>,
    /// Optional source URL.
    pub source_url: Option<String>,
}

impl MediaContent {
    /// Create a new media content from raw bytes with MIME detection.
    pub fn new(data: Vec<u8>, mime_type: impl Into<String>) -> Self {
        let mime = mime_type.into();
        let media_type = MediaType::from_mime(&mime);
        Self {
            media_type,
            data,
            mime_type: mime,
            filename: None,
            source_url: None,
        }
    }

    /// Create from raw bytes with filename-based type detection.
    pub fn from_file(data: Vec<u8>, filename: impl Into<String>) -> Self {
        let name = filename.into();
        let media_type = MediaType::from_filename(&name);
        let mime = guess_mime_from_extension(&name);
        Self {
            media_type,
            data,
            mime_type: mime,
            filename: Some(name),
            source_url: None,
        }
    }

    /// Set the source URL.
    pub fn with_source_url(mut self, url: impl Into<String>) -> Self {
        self.source_url = Some(url.into());
        self
    }

    /// Set the filename.
    pub fn with_filename(mut self, name: impl Into<String>) -> Self {
        self.filename = Some(name.into());
        self
    }

    /// Encode the data as base64.
    pub fn to_base64(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&self.data)
    }

    /// Create a data URI (e.g. `data:image/png;base64,...`).
    pub fn to_data_uri(&self) -> String {
        format!("data:{};base64,{}", self.mime_type, self.to_base64())
    }

    /// Size of the content in bytes.
    pub fn size(&self) -> usize {
        self.data.len()
    }
}

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
///
/// Each media type (image, PDF, audio) has its own extractor that
/// implements this trait.
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
        let mut extractors: Vec<Box<dyn MediaExtractor>> = vec![
            Box::new(super::image::ImageExtractor::new()),
            Box::new(super::pdf::PdfExtractor::new()),
            Box::new(super::audio::AudioExtractor::new()),
        ];

        #[cfg(feature = "document-extraction")]
        extractors.push(Box::new(super::document::DocumentExtractor::new()));

        Self {
            extractors,
            max_size: 50 * 1024 * 1024, // 50 MB
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
        // Size guard
        if content.size() > self.max_size {
            return Err(MediaExtractError::TooLarge {
                size: content.size(),
                max: self.max_size,
            });
        }

        // Find an extractor that supports this type
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

/// Check if a MIME type represents a document format extractable by the
/// document_extraction module.
fn is_document_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime == "application/json"
        || mime == "application/xml"
        || mime == "application/javascript"
        || mime == "application/x-yaml"
        || mime == "application/toml"
        || mime
            == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        || mime
            == "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        || mime
            == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
}

/// Guess MIME type from a filename's extension.
fn guess_mime_from_extension(filename: &str) -> String {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "tiff" => "image/tiff",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        "opus" => "audio/opus",
        "mp4" => "video/mp4",
        "avi" => "video/x-msvideo",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        // Document types
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "txt" | "md" | "markdown" | "csv" | "log" => "text/plain",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "yaml" | "yml" => "application/x-yaml",
        "toml" | "ini" | "cfg" | "conf" => "text/plain",
        "rs" | "py" | "js" | "ts" | "go" | "java" | "c" | "cpp" | "h" | "rb" | "sh"
        | "sql" | "css" => "text/plain",
        _ => "application/octet-stream",
    }
    .to_string()
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
        assert_eq!(MediaType::from_mime("application/json"), MediaType::Document);
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
    fn test_media_type_from_filename() {
        assert_eq!(MediaType::from_filename("photo.jpg"), MediaType::Image);
        assert_eq!(MediaType::from_filename("document.pdf"), MediaType::Pdf);
        assert_eq!(MediaType::from_filename("recording.m4a"), MediaType::Audio);
        assert_eq!(MediaType::from_filename("noext"), MediaType::Unknown);
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
    fn test_media_content_from_file() {
        let mc = MediaContent::from_file(vec![0xFF, 0xD8], "photo.jpg");
        assert_eq!(mc.media_type, MediaType::Image);
        assert_eq!(mc.mime_type, "image/jpeg");
        assert_eq!(mc.filename, Some("photo.jpg".to_string()));
    }

    #[test]
    fn test_media_content_data_uri() {
        let mc = MediaContent::new(vec![72, 101, 108, 108, 111], "text/plain");
        let uri = mc.to_data_uri();
        assert!(uri.starts_with("data:text/plain;base64,"));
    }

    #[test]
    fn test_media_content_base64() {
        let mc = MediaContent::new(vec![72, 101, 108, 108, 111], "text/plain");
        assert_eq!(mc.to_base64(), "SGVsbG8=");
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

    #[test]
    fn test_guess_mime() {
        assert_eq!(guess_mime_from_extension("test.jpg"), "image/jpeg");
        assert_eq!(guess_mime_from_extension("doc.pdf"), "application/pdf");
        assert_eq!(guess_mime_from_extension("song.mp3"), "audio/mpeg");
        assert_eq!(
            guess_mime_from_extension("unknown.xyz"),
            "application/octet-stream"
        );
    }
}
