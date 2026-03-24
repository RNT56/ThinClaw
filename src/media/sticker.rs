//! Telegram sticker-to-image conversion.
//!
//! Telegram stickers come in WebP (static) or TGS (Lottie animated)
//! format. This module detects sticker types and converts them
//! to standard formats using external tools (ffmpeg, dwebp).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Sticker format detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StickerFormat {
    /// Static WebP sticker.
    WebP,
    /// Animated Lottie sticker (TGS).
    Tgs,
    /// Video sticker (WebM).
    WebM,
    /// Unknown format.
    Unknown,
}

impl StickerFormat {
    /// Detect format from MIME type.
    pub fn from_mime(mime: &str) -> Self {
        match mime {
            "image/webp" => Self::WebP,
            "application/x-tgsticker" | "application/gzip" => Self::Tgs,
            "video/webm" => Self::WebM,
            _ => Self::Unknown,
        }
    }

    /// Detect format from file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "webp" => Self::WebP,
            "tgs" => Self::Tgs,
            "webm" => Self::WebM,
            _ => Self::Unknown,
        }
    }

    /// Detect format from file magic bytes.
    pub fn from_magic(data: &[u8]) -> Self {
        if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
            return Self::WebP;
        }

        // TGS is gzip-compressed Lottie JSON
        if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
            return Self::Tgs;
        }

        // WebM starts with EBML header
        if data.len() >= 4 && data[0..4] == [0x1a, 0x45, 0xdf, 0xa3] {
            return Self::WebM;
        }

        Self::Unknown
    }

    /// Get the output format for conversion.
    pub fn output_format(&self) -> &str {
        match self {
            Self::WebP => "png",
            Self::Tgs => "gif",
            Self::WebM => "gif",
            Self::Unknown => "png",
        }
    }

    /// Get the output MIME type.
    pub fn output_mime(&self) -> &str {
        match self {
            Self::WebP => "image/png",
            Self::Tgs => "image/gif",
            Self::WebM => "image/gif",
            Self::Unknown => "image/png",
        }
    }
}

/// Configuration for sticker conversion.
#[derive(Debug, Clone)]
pub struct StickerConfig {
    /// Whether sticker conversion is enabled.
    pub enabled: bool,
    /// Maximum sticker file size in bytes.
    pub max_size: u64,
    /// Output image max dimension.
    pub max_dimension: u32,
    /// Temporary directory for conversion.
    pub temp_dir: PathBuf,
}

impl Default for StickerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_size: 5 * 1024 * 1024, // 5 MB
            max_dimension: 512,
            temp_dir: std::env::temp_dir().join("thinclaw-stickers"),
        }
    }
}

impl StickerConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(val) = std::env::var("STICKER_CONVERT_ENABLED") {
            config.enabled = val != "0" && !val.eq_ignore_ascii_case("false");
        }
        if let Ok(max) = std::env::var("STICKER_MAX_SIZE_MB")
            && let Ok(m) = max.parse::<u64>()
        {
            config.max_size = m * 1024 * 1024;
        }
        if let Ok(dim) = std::env::var("STICKER_MAX_DIMENSION")
            && let Ok(d) = dim.parse()
        {
            config.max_dimension = d;
        }
        config
    }
}

/// Result of a sticker conversion.
#[derive(Debug, Clone)]
pub struct ConvertedSticker {
    /// Output image data.
    pub data: Vec<u8>,
    /// Output format (png, gif).
    pub format: String,
    /// Output MIME type.
    pub mime_type: String,
    /// Original sticker format.
    pub original_format: StickerFormat,
}

/// Convert a sticker file using external tools.
///
/// Supported conversions:
/// - WebP → PNG via `dwebp` or `ffmpeg`
/// - TGS → GIF (requires `lottie_to_gif` or similar)
/// - WebM → GIF via `ffmpeg`
pub async fn convert_sticker(
    data: &[u8],
    format: StickerFormat,
    config: &StickerConfig,
) -> Result<ConvertedSticker, StickerError> {
    if !config.enabled {
        return Err(StickerError::Disabled);
    }

    if data.len() as u64 > config.max_size {
        return Err(StickerError::TooLarge {
            size: data.len() as u64,
            limit: config.max_size,
        });
    }

    match format {
        StickerFormat::WebP => convert_webp_via_ffmpeg(data, config).await,
        StickerFormat::Tgs => {
            // TGS/Lottie is complex; return the raw data with format hint
            Err(StickerError::ExternalToolRequired("lottie_to_gif"))
        }
        StickerFormat::WebM => convert_webm_via_ffmpeg(data, config).await,
        StickerFormat::Unknown => Err(StickerError::UnsupportedFormat),
    }
}

/// Convert WebP to PNG using ffmpeg.
async fn convert_webp_via_ffmpeg(
    data: &[u8],
    config: &StickerConfig,
) -> Result<ConvertedSticker, StickerError> {
    let _ = tokio::fs::create_dir_all(&config.temp_dir).await;

    let input_path = config.temp_dir.join("input.webp");
    let output_path = config.temp_dir.join("output.png");

    tokio::fs::write(&input_path, data)
        .await
        .map_err(|e| StickerError::IoError(e.to_string()))?;

    let status = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            input_path.to_str().unwrap_or("input.webp"),
            "-vf",
            &format!(
                "scale='min({0},iw)':'min({0},ih)':force_original_aspect_ratio=decrease",
                config.max_dimension
            ),
            output_path.to_str().unwrap_or("output.png"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| StickerError::ConversionFailed(format!("ffmpeg not found: {}", e)))?;

    if !status.success() {
        let _ = tokio::fs::remove_file(&input_path).await;
        return Err(StickerError::ConversionFailed(
            "ffmpeg conversion failed".to_string(),
        ));
    }

    let output_data = tokio::fs::read(&output_path)
        .await
        .map_err(|e| StickerError::IoError(e.to_string()))?;

    // Clean up
    let _ = tokio::fs::remove_file(&input_path).await;
    let _ = tokio::fs::remove_file(&output_path).await;

    Ok(ConvertedSticker {
        data: output_data,
        format: "png".to_string(),
        mime_type: "image/png".to_string(),
        original_format: StickerFormat::WebP,
    })
}

/// Convert WebM to GIF using ffmpeg.
async fn convert_webm_via_ffmpeg(
    data: &[u8],
    config: &StickerConfig,
) -> Result<ConvertedSticker, StickerError> {
    let _ = tokio::fs::create_dir_all(&config.temp_dir).await;

    let input_path = config.temp_dir.join("input.webm");
    let output_path = config.temp_dir.join("output.gif");

    tokio::fs::write(&input_path, data)
        .await
        .map_err(|e| StickerError::IoError(e.to_string()))?;

    let status = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            input_path.to_str().unwrap_or("input.webm"),
            "-vf",
            &format!(
                "scale='min({0},iw)':'min({0},ih)':force_original_aspect_ratio=decrease,fps=15",
                config.max_dimension
            ),
            output_path.to_str().unwrap_or("output.gif"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| StickerError::ConversionFailed(format!("ffmpeg not found: {}", e)))?;

    if !status.success() {
        let _ = tokio::fs::remove_file(&input_path).await;
        return Err(StickerError::ConversionFailed(
            "ffmpeg conversion failed".to_string(),
        ));
    }

    let output_data = tokio::fs::read(&output_path)
        .await
        .map_err(|e| StickerError::IoError(e.to_string()))?;

    let _ = tokio::fs::remove_file(&input_path).await;
    let _ = tokio::fs::remove_file(&output_path).await;

    Ok(ConvertedSticker {
        data: output_data,
        format: "gif".to_string(),
        mime_type: "image/gif".to_string(),
        original_format: StickerFormat::WebM,
    })
}

/// Check if ffmpeg is available.
pub fn is_ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Sticker conversion errors.
#[derive(Debug, Clone)]
pub enum StickerError {
    Disabled,
    TooLarge { size: u64, limit: u64 },
    UnsupportedFormat,
    ConversionFailed(String),
    IoError(String),
    ExternalToolRequired(&'static str),
}

impl std::fmt::Display for StickerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "Sticker conversion disabled"),
            Self::TooLarge { size, limit } => {
                write!(f, "Sticker too large: {} bytes (limit: {})", size, limit)
            }
            Self::UnsupportedFormat => write!(f, "Unsupported sticker format"),
            Self::ConversionFailed(e) => write!(f, "Conversion failed: {}", e),
            Self::IoError(e) => write!(f, "IO error: {}", e),
            Self::ExternalToolRequired(tool) => {
                write!(f, "External tool required: {}", tool)
            }
        }
    }
}

impl std::error::Error for StickerError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_from_mime() {
        assert_eq!(StickerFormat::from_mime("image/webp"), StickerFormat::WebP);
        assert_eq!(
            StickerFormat::from_mime("application/x-tgsticker"),
            StickerFormat::Tgs
        );
        assert_eq!(StickerFormat::from_mime("video/webm"), StickerFormat::WebM);
        assert_eq!(
            StickerFormat::from_mime("text/plain"),
            StickerFormat::Unknown
        );
    }

    #[test]
    fn test_format_from_extension() {
        assert_eq!(StickerFormat::from_extension("webp"), StickerFormat::WebP);
        assert_eq!(StickerFormat::from_extension("TGS"), StickerFormat::Tgs);
        assert_eq!(StickerFormat::from_extension("webm"), StickerFormat::WebM);
    }

    #[test]
    fn test_format_from_magic_webp() {
        let data = b"RIFF\x00\x00\x00\x00WEBP";
        assert_eq!(StickerFormat::from_magic(data), StickerFormat::WebP);
    }

    #[test]
    fn test_format_from_magic_gzip() {
        let data = [0x1f, 0x8b, 0x08, 0x00];
        assert_eq!(StickerFormat::from_magic(&data), StickerFormat::Tgs);
    }

    #[test]
    fn test_format_from_magic_webm() {
        let data = [0x1a, 0x45, 0xdf, 0xa3];
        assert_eq!(StickerFormat::from_magic(&data), StickerFormat::WebM);
    }

    #[test]
    fn test_format_from_magic_unknown() {
        assert_eq!(
            StickerFormat::from_magic(&[0, 1, 2, 3]),
            StickerFormat::Unknown
        );
    }

    #[test]
    fn test_output_format() {
        assert_eq!(StickerFormat::WebP.output_format(), "png");
        assert_eq!(StickerFormat::Tgs.output_format(), "gif");
        assert_eq!(StickerFormat::WebM.output_format(), "gif");
    }

    #[test]
    fn test_output_mime() {
        assert_eq!(StickerFormat::WebP.output_mime(), "image/png");
        assert_eq!(StickerFormat::Tgs.output_mime(), "image/gif");
    }

    #[test]
    fn test_default_config() {
        let config = StickerConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_size, 5 * 1024 * 1024);
        assert_eq!(config.max_dimension, 512);
    }

    #[test]
    fn test_error_display() {
        let err = StickerError::TooLarge {
            size: 100,
            limit: 50,
        };
        assert!(format!("{}", err).contains("too large"));

        let err = StickerError::ExternalToolRequired("lottie_to_gif");
        assert!(format!("{}", err).contains("lottie_to_gif"));
    }
}
