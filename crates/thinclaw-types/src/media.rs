//! Shared media attachment types.

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
            "docx" | "pptx" | "xlsx" | "txt" | "csv" | "json" | "xml" | "yaml" | "yml" | "toml"
            | "md" | "markdown" | "rs" | "py" | "js" | "ts" | "go" | "java" | "c" | "cpp" | "h"
            | "rb" | "sh" | "sql" | "html" | "css" | "ini" | "cfg" | "conf" | "log" => {
                Self::Document
            }
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

fn is_document_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime == "application/json"
        || mime == "application/xml"
        || mime == "application/javascript"
        || mime == "application/x-yaml"
        || mime == "application/toml"
        || mime == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        || mime == "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        || mime == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
}

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
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "txt" | "md" | "markdown" | "csv" | "log" => "text/plain",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "yaml" | "yml" => "application/x-yaml",
        "toml" | "ini" | "cfg" | "conf" => "text/plain",
        "rs" | "py" | "js" | "ts" | "go" | "java" | "c" | "cpp" | "h" | "rb" | "sh" | "sql"
        | "css" => "text/plain",
        _ => "application/octet-stream",
    }
    .to_string()
}
