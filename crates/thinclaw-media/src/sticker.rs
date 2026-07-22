//! Telegram sticker-to-image conversion.
//!
//! Telegram stickers come in WebP (static) or TGS (Lottie animated)
//! format. This module detects sticker types and converts them
//! to standard formats using external tools (ffmpeg, dwebp).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use thinclaw_platform::{bounded_command_output, executable_available};
use tokio::process::Command;

const MAX_CONFIGURED_STICKER_BYTES: u64 = 50 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FFMPEG_STDERR_BYTES: usize = 64 * 1024;

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
            && (1..=50).contains(&m)
            && let Some(bytes) = m.checked_mul(1024 * 1024)
        {
            config.max_size = bytes;
        }
        if let Ok(dim) = std::env::var("STICKER_MAX_DIMENSION")
            && let Ok(d) = dim.parse()
            && (16..=4096).contains(&d)
        {
            config.max_dimension = d;
        }
        config
    }

    fn validate(&self) -> Result<(), StickerError> {
        if !(1..=MAX_CONFIGURED_STICKER_BYTES).contains(&self.max_size) {
            return Err(StickerError::InvalidConfiguration(format!(
                "max_size must be between 1 and {MAX_CONFIGURED_STICKER_BYTES}"
            )));
        }
        if !(16..=4096).contains(&self.max_dimension) {
            return Err(StickerError::InvalidConfiguration(
                "max_dimension must be between 16 and 4096".to_string(),
            ));
        }
        Ok(())
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

    config.validate()?;

    if data.len() as u64 > config.max_size {
        return Err(StickerError::TooLarge {
            size: data.len() as u64,
            limit: config.max_size,
        });
    }

    if StickerFormat::from_magic(data) != format {
        return Err(StickerError::FormatMismatch);
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
    convert_via_ffmpeg(
        data,
        config,
        "webp",
        "png",
        format!(
            "scale='min({0},iw)':'min({0},ih)':force_original_aspect_ratio=decrease",
            config.max_dimension
        ),
        true,
        StickerFormat::WebP,
        "image/png",
    )
    .await
}

/// Convert WebM to GIF using ffmpeg.
async fn convert_webm_via_ffmpeg(
    data: &[u8],
    config: &StickerConfig,
) -> Result<ConvertedSticker, StickerError> {
    convert_via_ffmpeg(
        data,
        config,
        "webm",
        "gif",
        format!(
            "scale='min({0},iw)':'min({0},ih)':force_original_aspect_ratio=decrease,fps=15",
            config.max_dimension
        ),
        false,
        StickerFormat::WebM,
        "image/gif",
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn convert_via_ffmpeg(
    data: &[u8],
    config: &StickerConfig,
    input_extension: &str,
    output_extension: &str,
    video_filter: String,
    single_frame: bool,
    original_format: StickerFormat,
    output_mime: &str,
) -> Result<ConvertedSticker, StickerError> {
    tokio::fs::create_dir_all(&config.temp_dir)
        .await
        .map_err(|error| StickerError::IoError(error.to_string()))?;
    let root_metadata = tokio::fs::symlink_metadata(&config.temp_dir)
        .await
        .map_err(|error| StickerError::IoError(error.to_string()))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(StickerError::InvalidConfiguration(
            "temp_dir must be a directory, never a symlink".to_string(),
        ));
    }
    let temp_root = tokio::fs::canonicalize(&config.temp_dir)
        .await
        .map_err(|error| StickerError::IoError(error.to_string()))?;
    let work_dir = tempfile::Builder::new()
        .prefix("conversion-")
        .tempdir_in(&temp_root)
        .map_err(|error| StickerError::IoError(error.to_string()))?;
    let input_path = work_dir.path().join(format!("input.{input_extension}"));
    let output_path = work_dir.path().join(format!("output.{output_extension}"));

    thinclaw_platform::write_private_file_atomic_async(input_path.clone(), data.to_vec(), false)
        .await
        .map_err(|error| StickerError::IoError(error.to_string()))?;

    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-y",
            "-i",
        ])
        .arg(&input_path)
        .args(["-t", "30", "-vf", &video_filter]);
    if single_frame {
        command.args(["-frames:v", "1"]);
    }
    command.arg(&output_path);

    let output = bounded_command_output(
        &mut command,
        Duration::from_secs(60),
        0,
        MAX_FFMPEG_STDERR_BYTES,
    )
    .await
    .map_err(|error| StickerError::ConversionFailed(format!("ffmpeg: {error}")))?;
    if !output.status.success() {
        return Err(StickerError::ConversionFailed(format!(
            "ffmpeg exited with {}: {}",
            output.status,
            sanitize_process_text(&output.stderr)
        )));
    }

    let max_output = config.max_size.saturating_mul(8).clamp(1, MAX_OUTPUT_BYTES);
    let output_data =
        thinclaw_platform::read_regular_file_bounded_single_link_async(output_path, max_output)
            .await
            .map_err(|error| StickerError::IoError(error.to_string()))?;
    if output_data.is_empty() {
        return Err(StickerError::ConversionFailed(
            "ffmpeg produced an invalid or oversized artifact".to_string(),
        ));
    }
    Ok(ConvertedSticker {
        data: output_data,
        format: output_extension.to_string(),
        mime_type: output_mime.to_string(),
        original_format,
    })
}

fn sanitize_process_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(1024)
        .collect()
}

/// Check if ffmpeg is available.
pub fn is_ffmpeg_available() -> bool {
    executable_available("ffmpeg")
}

/// Sticker conversion errors.
#[derive(Debug, Clone)]
pub enum StickerError {
    Disabled,
    TooLarge { size: u64, limit: u64 },
    UnsupportedFormat,
    FormatMismatch,
    InvalidConfiguration(String),
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
            Self::FormatMismatch => write!(f, "Sticker bytes do not match the declared format"),
            Self::InvalidConfiguration(error) => {
                write!(f, "Invalid sticker configuration: {error}")
            }
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

    #[tokio::test]
    async fn declared_format_must_match_magic_before_ffmpeg_runs() {
        let error = convert_sticker(
            b"not a webp",
            StickerFormat::WebP,
            &StickerConfig::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, StickerError::FormatMismatch));
    }

    #[tokio::test]
    async fn invalid_limits_are_rejected_before_files_are_written() {
        let config = StickerConfig {
            max_dimension: 0,
            ..StickerConfig::default()
        };
        let mut webp = vec![0_u8; 12];
        webp[..4].copy_from_slice(b"RIFF");
        webp[8..].copy_from_slice(b"WEBP");
        let error = convert_sticker(&webp, StickerFormat::WebP, &config)
            .await
            .unwrap_err();
        assert!(matches!(error, StickerError::InvalidConfiguration(_)));
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
