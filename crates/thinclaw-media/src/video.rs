//! Video media pipeline — metadata extraction and frame analysis.
//!
//! Provides video processing capabilities:
//! 1. Metadata extraction (duration, resolution, codec)
//! 2. Keyframe extraction for visual analysis via LLM
//! 3. Audio track extraction for transcription
//!
//! Video files are not embedded directly into LLM context; instead,
//! representative keyframes and audio transcripts are extracted
//! and provided as text/image content.
//!
//! ```text
//! Video file ──► VideoAnalyzer
//!                  ├── extract metadata (duration, resolution, fps)
//!                  ├── extract keyframes (every N seconds → images)
//!                  └── extract audio track (→ AudioExtractor for transcription)
//! ```

use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thinclaw_platform::{BoundedProcessOutput, bounded_command_output, executable_available};
use tokio::process::Command;

const DEFAULT_MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CONFIGURED_INPUT_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_PROBE_STDOUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_PROCESS_STDERR_BYTES: usize = 64 * 1024;
const MAX_KEYFRAME_BYTES: u64 = 16 * 1024 * 1024;

/// Video metadata extracted from a media file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoMetadata {
    /// Video duration in seconds.
    pub duration_secs: Option<f64>,
    /// Video width in pixels.
    pub width: Option<u32>,
    /// Video height in pixels.
    pub height: Option<u32>,
    /// Video codec (e.g., "h264", "vp9").
    pub codec: Option<String>,
    /// Container format (e.g., "mp4", "webm").
    pub container: Option<String>,
    /// Frames per second.
    pub fps: Option<f64>,
    /// Audio track present.
    pub has_audio: bool,
    /// File size in bytes.
    pub file_size: u64,
}

/// Configuration for video analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoAnalysisConfig {
    /// Maximum video duration to process (seconds). Longer videos are truncated.
    pub max_duration_secs: f64,
    /// Interval between keyframe extractions (seconds).
    pub keyframe_interval_secs: f64,
    /// Maximum number of keyframes to extract.
    pub max_keyframes: usize,
    /// Whether to extract the audio track for transcription.
    pub extract_audio: bool,
    /// Maximum keyframe dimensions (longest side).
    pub keyframe_max_dimension: u32,
    /// Maximum accepted input file size.
    pub max_input_bytes: u64,
}

impl Default for VideoAnalysisConfig {
    fn default() -> Self {
        Self {
            max_duration_secs: 300.0, // 5 minutes
            keyframe_interval_secs: 5.0,
            max_keyframes: 20,
            extract_audio: true,
            keyframe_max_dimension: 1024,
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
        }
    }
}

#[derive(Debug)]
struct ArtifactLease(tempfile::TempDir);

/// Result of video analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoAnalysis {
    /// Extracted metadata.
    pub metadata: VideoMetadata,
    /// Paths to extracted keyframe images.
    pub keyframe_paths: Vec<String>,
    /// Path to extracted audio data for downstream transcription (if available).
    pub audio_transcript_path: Option<String>,
    /// Deprecated alias maintained for one release cycle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_transcript: Option<String>,
    /// Summary suitable for LLM context.
    pub summary: String,
    /// Keeps temporary artifacts alive exactly as long as this analysis (and
    /// any clones) is alive. It is intentionally not serialized.
    #[serde(skip)]
    artifact_lease: Option<Arc<ArtifactLease>>,
}

impl VideoAnalysis {
    /// Directory containing extracted artifacts, if extraction produced any.
    pub fn artifact_dir(&self) -> Option<&Path> {
        self.artifact_lease.as_ref().map(|lease| lease.0.path())
    }
}

/// Video analyzer for extracting content from video files.
///
/// Uses `ffprobe` (part of the FFmpeg suite) for metadata and `ffmpeg`
/// for keyframe/audio extraction. When FFmpeg is not installed, falls
/// back to basic file-property inference.
pub struct VideoAnalyzer {
    config: VideoAnalysisConfig,
}

fn run_bounded_sync(
    mut command: Command,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    operation: &'static str,
) -> Result<BoundedProcessOutput, VideoError> {
    let worker = std::thread::Builder::new()
        .name(format!("thinclaw-{operation}"))
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .map_err(|error| VideoError::IoError(error.to_string()))?;
            runtime
                .block_on(bounded_command_output(
                    &mut command,
                    timeout,
                    stdout_limit,
                    stderr_limit,
                ))
                .map_err(|error| VideoError::AnalysisFailed(format!("{operation}: {error}")))
        })
        .map_err(|error| VideoError::IoError(format!("failed to start {operation}: {error}")))?;
    worker
        .join()
        .map_err(|_| VideoError::AnalysisFailed(format!("{operation} worker panicked")))?
}

fn sanitize_process_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(1024)
        .collect()
}

impl VideoAnalyzer {
    /// Create a new video analyzer with the given configuration.
    pub fn new(config: VideoAnalysisConfig) -> Self {
        Self { config }
    }

    fn validate_config(&self) -> Result<(), VideoError> {
        if !self.config.max_duration_secs.is_finite()
            || !(0.1..=3600.0).contains(&self.config.max_duration_secs)
        {
            return Err(VideoError::InvalidConfiguration(
                "max_duration_secs must be finite and between 0.1 and 3600".to_string(),
            ));
        }
        if !self.config.keyframe_interval_secs.is_finite()
            || !(0.1..=self.config.max_duration_secs).contains(&self.config.keyframe_interval_secs)
        {
            return Err(VideoError::InvalidConfiguration(
                "keyframe_interval_secs must be finite, at least 0.1, and no greater than max_duration_secs"
                    .to_string(),
            ));
        }
        if !(1..=100).contains(&self.config.max_keyframes) {
            return Err(VideoError::InvalidConfiguration(
                "max_keyframes must be between 1 and 100".to_string(),
            ));
        }
        if !(16..=4096).contains(&self.config.keyframe_max_dimension) {
            return Err(VideoError::InvalidConfiguration(
                "keyframe_max_dimension must be between 16 and 4096".to_string(),
            ));
        }
        if !(1..=MAX_CONFIGURED_INPUT_BYTES).contains(&self.config.max_input_bytes) {
            return Err(VideoError::InvalidConfiguration(format!(
                "max_input_bytes must be between 1 and {MAX_CONFIGURED_INPUT_BYTES}"
            )));
        }
        Ok(())
    }

    fn validate_input(&self, path: &Path) -> Result<u64, VideoError> {
        self.validate_config()?;
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| VideoError::IoError(error.to_string()))?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(VideoError::AnalysisFailed(
                "video input must be a regular file, never a symlink".to_string(),
            ));
        }
        if metadata.len() > self.config.max_input_bytes {
            return Err(VideoError::TooLarge {
                size: metadata.len(),
                max_bytes: self.config.max_input_bytes,
            });
        }
        Ok(metadata.len())
    }

    fn processing_timeout(&self) -> Duration {
        Duration::from_secs(
            (self.config.max_duration_secs.ceil() as u64)
                .saturating_mul(2)
                .saturating_add(30)
                .min(900),
        )
    }

    /// Extract metadata from a video file.
    ///
    /// Uses `ffprobe -print_format json` when available, otherwise
    /// derives what it can from the filename and file size.
    pub fn extract_metadata(&self, path: &Path) -> Result<VideoMetadata, VideoError> {
        let file_size = self.validate_input(path)?;

        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown")
            .to_lowercase();
        let extension: String = extension
            .chars()
            .filter(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
            })
            .take(32)
            .collect();
        let extension = if extension.is_empty() {
            "unknown".to_string()
        } else {
            extension
        };

        let container = match extension.as_str() {
            "mp4" | "m4v" => Some("mp4".to_string()),
            "webm" => Some("webm".to_string()),
            "mkv" => Some("matroska".to_string()),
            "avi" => Some("avi".to_string()),
            "mov" => Some("quicktime".to_string()),
            _ => Some(extension.clone()),
        };

        // Try ffprobe for full metadata extraction.
        if Self::ffprobe_available() {
            match self.extract_metadata_ffprobe(path) {
                Ok(mut meta) => {
                    meta.file_size = file_size;
                    if meta.container.is_none() {
                        meta.container = container;
                    }
                    return Ok(meta);
                }
                Err(e) => {
                    tracing::debug!("ffprobe failed, falling back to file-based metadata: {}", e);
                }
            }
        }

        // Fallback: file-based metadata only.
        Ok(VideoMetadata {
            duration_secs: None,
            width: None,
            height: None,
            codec: None,
            container,
            fps: None,
            has_audio: true, // Assume true; ffprobe needed for accuracy
            file_size,
        })
    }

    /// Extract metadata using `ffprobe -print_format json`.
    fn extract_metadata_ffprobe(&self, path: &Path) -> Result<VideoMetadata, VideoError> {
        let mut command = Command::new("ffprobe");
        command
            .args([
                "-v",
                "error",
                "-protocol_whitelist",
                "file,pipe",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
                "-i",
            ])
            .arg(path);
        let output = run_bounded_sync(
            command,
            Duration::from_secs(15),
            MAX_PROBE_STDOUT_BYTES,
            MAX_PROCESS_STDERR_BYTES,
            "ffprobe",
        )?;

        if !output.status.success() {
            let stderr = sanitize_process_text(&output.stderr);
            return Err(VideoError::AnalysisFailed(format!(
                "ffprobe exited with {}: {}",
                output.status, stderr
            )));
        }

        let probe: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| VideoError::AnalysisFailed(format!("ffprobe JSON parse: {}", e)))?;

        // Extract from format section.
        let duration_secs = probe
            .pointer("/format/duration")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|duration| duration.is_finite() && *duration >= 0.0);

        let format_name = probe
            .pointer("/format/format_name")
            .and_then(|v| v.as_str())
            .map(|s| {
                s.split(',')
                    .next()
                    .unwrap_or(s)
                    .chars()
                    .filter(|character| !character.is_control())
                    .take(64)
                    .collect::<String>()
            })
            .filter(|value| !value.is_empty());

        // Extract from video stream (first one found).
        let streams = probe
            .get("streams")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let video_stream = streams
            .iter()
            .find(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("video"));

        let has_audio = streams
            .iter()
            .any(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("audio"));

        let (width, height, codec, fps) = if let Some(vs) = video_stream {
            let w = vs
                .get("width")
                .and_then(|v| v.as_u64())
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0);
            let h = vs
                .get("height")
                .and_then(|v| v.as_u64())
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0);
            let c = vs
                .get("codec_name")
                .and_then(|v| v.as_str())
                .map(|s| {
                    s.chars()
                        .filter(|character| !character.is_control())
                        .take(64)
                        .collect::<String>()
                })
                .filter(|value| !value.is_empty());
            // Parse fps from r_frame_rate (e.g. "30/1" or "30000/1001").
            let f = vs
                .get("r_frame_rate")
                .and_then(|v| v.as_str())
                .and_then(|s| {
                    let parts: Vec<&str> = s.split('/').collect();
                    if parts.len() == 2 {
                        let num = parts[0].parse::<f64>().ok()?;
                        let den = parts[1].parse::<f64>().ok()?;
                        if den > 0.0 { Some(num / den) } else { None }
                    } else {
                        s.parse::<f64>().ok()
                    }
                })
                .filter(|value| value.is_finite() && *value > 0.0 && *value <= 1000.0);
            (w, h, c, f)
        } else {
            (None, None, None, None)
        };

        Ok(VideoMetadata {
            duration_secs,
            width,
            height,
            codec,
            container: format_name,
            fps,
            has_audio,
            file_size: 0, // Filled by caller.
        })
    }

    /// Analyze a video file, extracting metadata, keyframes, and audio.
    ///
    /// When `ffmpeg` is available, keyframes are extracted at the configured
    /// interval and audio is extracted as a WAV file. Without ffmpeg, only
    /// basic metadata and a summary are returned.
    pub fn analyze(&self, path: &Path) -> Result<VideoAnalysis, VideoError> {
        let metadata = self.extract_metadata(path)?;

        // Check duration limits.
        if let Some(dur) = metadata.duration_secs
            && dur > self.config.max_duration_secs
        {
            return Err(VideoError::TooLong {
                duration_secs: dur,
                max_secs: self.config.max_duration_secs,
            });
        }

        let mut keyframe_paths = Vec::new();
        let mut audio_path: Option<String> = None;
        let mut artifact_lease = None;

        if Self::ffmpeg_available() {
            match tempfile::Builder::new().prefix("thinclaw-video-").tempdir() {
                Err(error) => tracing::warn!(%error, "Could not create video artifact directory"),
                Ok(out_dir) => {
                    // Extract keyframes using ffmpeg.
                    match self.extract_keyframes(path, out_dir.path()) {
                        Ok(frames) => keyframe_paths = frames,
                        Err(e) => tracing::warn!("Keyframe extraction failed: {}", e),
                    }

                    // Extract audio track if configured and present.
                    if self.config.extract_audio && metadata.has_audio {
                        match self.extract_audio(path, out_dir.path()) {
                            Ok(audio) => audio_path = Some(audio),
                            Err(e) => tracing::debug!("Audio extraction skipped: {}", e),
                        }
                    }
                    if !keyframe_paths.is_empty() || audio_path.is_some() {
                        artifact_lease = Some(Arc::new(ArtifactLease(out_dir)));
                    }
                }
            }
        }

        let summary = format_video_summary(&metadata, path);

        let audio_transcript_path = audio_path;
        let audio_transcript = audio_transcript_path.clone();

        Ok(VideoAnalysis {
            metadata,
            keyframe_paths,
            audio_transcript_path,
            audio_transcript,
            summary,
            artifact_lease,
        })
    }

    /// Extract keyframes at the configured interval using ffmpeg.
    fn extract_keyframes(&self, path: &Path, out_dir: &Path) -> Result<Vec<String>, VideoError> {
        let fps_filter = format!("fps=1/{}", self.config.keyframe_interval_secs);
        let scale_filter = format!(
            "scale='min({max_dim},iw)':'min({max_dim},ih)':force_original_aspect_ratio=decrease",
            max_dim = self.config.keyframe_max_dimension
        );
        let output_pattern = out_dir.join("frame_%04d.jpg");

        let filter = format!("{},{}", fps_filter, scale_filter);
        let frame_limit = self.config.max_keyframes.to_string();
        let duration_limit = self.config.max_duration_secs.to_string();
        let mut command = Command::new("ffmpeg");
        command
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-protocol_whitelist",
                "file,pipe",
                "-i",
            ])
            .arg(path)
            .args([
                "-t",
                &duration_limit,
                "-vf",
                &filter,
                "-frames:v",
                &frame_limit,
                "-q:v",
                "2",
                "-y",
            ])
            .arg(&output_pattern);
        let output = run_bounded_sync(
            command,
            self.processing_timeout(),
            0,
            MAX_PROCESS_STDERR_BYTES,
            "ffmpeg-keyframes",
        )?;

        if !output.status.success() {
            return Err(VideoError::AnalysisFailed(format!(
                "ffmpeg keyframe extraction exited with {}: {}",
                output.status,
                sanitize_process_text(&output.stderr)
            )));
        }

        // Collect extracted frames, sorted by name.
        let mut frames: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(out_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let valid_name = p.file_name().and_then(OsStr::to_str).is_some_and(|name| {
                    let bytes = name.as_bytes();
                    bytes.len() == b"frame_0000.jpg".len()
                        && &bytes[..6] == b"frame_"
                        && &bytes[10..] == b".jpg"
                        && bytes[6..10].iter().all(u8::is_ascii_digit)
                });
                if !valid_name {
                    continue;
                }
                let metadata = std::fs::symlink_metadata(&p)
                    .map_err(|error| VideoError::IoError(error.to_string()))?;
                if metadata.file_type().is_symlink()
                    || !metadata.file_type().is_file()
                    || metadata.len() == 0
                    || metadata.len() > MAX_KEYFRAME_BYTES
                {
                    return Err(VideoError::AnalysisFailed(
                        "ffmpeg produced an invalid keyframe artifact".to_string(),
                    ));
                }
                frames.push(p.to_string_lossy().into_owned());
            }
        }
        frames.sort();
        if frames.len() > self.config.max_keyframes {
            return Err(VideoError::AnalysisFailed(
                "ffmpeg produced more keyframes than requested".to_string(),
            ));
        }

        tracing::info!(count = frames.len(), "Extracted video keyframes");

        Ok(frames)
    }

    /// Extract the audio track as a WAV file for transcription.
    fn extract_audio(&self, path: &Path, out_dir: &Path) -> Result<String, VideoError> {
        let audio_out = out_dir.join("audio.wav");

        let duration_limit = self.config.max_duration_secs.to_string();
        let mut command = Command::new("ffmpeg");
        command
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-protocol_whitelist",
                "file,pipe",
                "-i",
            ])
            .arg(path)
            .args([
                "-t",
                &duration_limit,
                "-vn",
                "-acodec",
                "pcm_s16le",
                "-ar",
                "16000",
                "-ac",
                "1",
                "-y",
            ])
            .arg(&audio_out);
        let output = run_bounded_sync(
            command,
            self.processing_timeout(),
            0,
            MAX_PROCESS_STDERR_BYTES,
            "ffmpeg-audio",
        )?;

        if !output.status.success() {
            return Err(VideoError::AnalysisFailed(format!(
                "ffmpeg audio extraction failed: {}",
                sanitize_process_text(&output.stderr)
            )));
        }

        let metadata = std::fs::symlink_metadata(&audio_out)
            .map_err(|error| VideoError::IoError(error.to_string()))?;
        let maximum_audio_bytes = (self.config.max_duration_secs.ceil() as u64)
            .saturating_mul(32_000)
            .saturating_add(1024 * 1024);
        if metadata.file_type().is_symlink()
            || !metadata.file_type().is_file()
            || metadata.len() == 0
            || metadata.len() > maximum_audio_bytes
        {
            return Err(VideoError::AnalysisFailed(
                "ffmpeg produced an invalid audio artifact".to_string(),
            ));
        }

        Ok(audio_out.to_string_lossy().to_string())
    }

    /// Check if ffmpeg is available on the system.
    pub fn ffmpeg_available() -> bool {
        executable_available("ffmpeg")
    }

    /// Check if ffprobe is available on the system.
    pub fn ffprobe_available() -> bool {
        executable_available("ffprobe")
    }

    /// Get the current configuration.
    pub fn config(&self) -> &VideoAnalysisConfig {
        &self.config
    }

    /// List supported video file extensions.
    pub fn supported_extensions() -> &'static [&'static str] {
        &["mp4", "webm", "mkv", "avi", "mov", "m4v", "flv", "wmv"]
    }

    /// Check if a file extension is a supported video format.
    pub fn is_supported(path: &std::path::Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| Self::supported_extensions().contains(&ext.to_lowercase().as_str()))
            .unwrap_or(false)
    }
}

impl Default for VideoAnalyzer {
    fn default() -> Self {
        Self::new(VideoAnalysisConfig::default())
    }
}

/// Format a human-readable summary of video metadata.
fn format_video_summary(metadata: &VideoMetadata, path: &Path) -> String {
    let filename: String = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("video")
        .chars()
        .filter(|character| !character.is_control())
        .take(255)
        .collect();
    let filename = if filename.is_empty() {
        "video"
    } else {
        &filename
    };

    let mut parts = vec![format!("Video: {}", filename)];

    if let Some(d) = metadata.duration_secs {
        let mins = (d / 60.0).floor() as u32;
        let secs = (d % 60.0).floor() as u32;
        parts.push(format!("Duration: {}:{:02}", mins, secs));
    }

    if let (Some(w), Some(h)) = (metadata.width, metadata.height) {
        parts.push(format!("Resolution: {}x{}", w, h));
    }

    if let Some(ref codec) = metadata.codec {
        parts.push(format!("Codec: {}", codec));
    }

    if let Some(ref container) = metadata.container {
        parts.push(format!("Format: {}", container));
    }

    let size_mb = metadata.file_size as f64 / (1024.0 * 1024.0);
    parts.push(format!("Size: {:.1} MB", size_mb));

    parts.join(" | ")
}

/// Errors for video operations.
#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error("I/O error: {0}")]
    IoError(String),

    #[error("ffmpeg not available; install ffmpeg for full video analysis")]
    FfmpegNotAvailable,

    #[error("Video too long ({duration_secs:.0}s, max {max_secs:.0}s)")]
    TooLong { duration_secs: f64, max_secs: f64 },

    #[error("Video is too large ({size} bytes, max {max_bytes} bytes)")]
    TooLarge { size: u64, max_bytes: u64 },

    #[error("Invalid video analysis configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("Analysis failed: {0}")]
    AnalysisFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = VideoAnalysisConfig::default();
        assert!((config.max_duration_secs - 300.0).abs() < 0.1);
        assert_eq!(config.max_keyframes, 20);
        assert!(config.extract_audio);
        assert_eq!(config.max_input_bytes, DEFAULT_MAX_INPUT_BYTES);
    }

    #[test]
    fn test_supported_extensions() {
        assert!(VideoAnalyzer::supported_extensions().contains(&"mp4"));
        assert!(VideoAnalyzer::supported_extensions().contains(&"webm"));
        assert!(!VideoAnalyzer::supported_extensions().contains(&"jpg"));
    }

    #[test]
    fn test_is_supported() {
        use std::path::Path;
        assert!(VideoAnalyzer::is_supported(Path::new("test.mp4")));
        assert!(VideoAnalyzer::is_supported(Path::new("test.webm")));
        assert!(!VideoAnalyzer::is_supported(Path::new("test.jpg")));
        assert!(!VideoAnalyzer::is_supported(Path::new("test")));
    }

    #[test]
    fn test_format_summary() {
        let metadata = VideoMetadata {
            duration_secs: Some(125.5),
            width: Some(1920),
            height: Some(1080),
            codec: Some("h264".to_string()),
            container: Some("mp4".to_string()),
            fps: Some(30.0),
            has_audio: true,
            file_size: 10_000_000,
        };

        let summary = format_video_summary(&metadata, std::path::Path::new("test.mp4"));
        assert!(summary.contains("test.mp4"));
        assert!(summary.contains("2:05"));
        assert!(summary.contains("1920x1080"));
        assert!(summary.contains("h264"));
    }

    #[test]
    fn test_format_summary_minimal() {
        let metadata = VideoMetadata {
            duration_secs: None,
            width: None,
            height: None,
            codec: None,
            container: Some("mp4".to_string()),
            fps: None,
            has_audio: false,
            file_size: 1024,
        };

        let summary = format_video_summary(&metadata, std::path::Path::new("clip.mp4"));
        assert!(summary.contains("clip.mp4"));
        assert!(summary.contains("mp4"));
    }

    #[test]
    fn invalid_config_is_rejected_before_external_tools_run() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let config = VideoAnalysisConfig {
            keyframe_interval_secs: f64::NAN,
            ..VideoAnalysisConfig::default()
        };
        let error = VideoAnalyzer::new(config)
            .extract_metadata(temp.path())
            .unwrap_err();
        assert!(matches!(error, VideoError::InvalidConfiguration(_)));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_input_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.mp4");
        let link = directory.path().join("link.mp4");
        std::fs::write(&target, b"not a video").unwrap();
        symlink(&target, &link).unwrap();
        let error = VideoAnalyzer::default()
            .extract_metadata(&link)
            .unwrap_err();
        assert!(matches!(error, VideoError::AnalysisFailed(_)));
    }
}
