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

use serde::{Deserialize, Serialize};

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
    pub file_size: usize,
}

/// Configuration for video analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl Default for VideoAnalysisConfig {
    fn default() -> Self {
        Self {
            max_duration_secs: 300.0, // 5 minutes
            keyframe_interval_secs: 5.0,
            max_keyframes: 20,
            extract_audio: true,
            keyframe_max_dimension: 1024,
        }
    }
}

/// Result of video analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoAnalysis {
    /// Extracted metadata.
    pub metadata: VideoMetadata,
    /// Paths to extracted keyframe images.
    pub keyframe_paths: Vec<String>,
    /// Extracted audio transcript (if available).
    pub audio_transcript: Option<String>,
    /// Summary suitable for LLM context.
    pub summary: String,
}

/// Video analyzer for extracting content from video files.
///
/// Uses `ffprobe` (part of the FFmpeg suite) for metadata and `ffmpeg`
/// for keyframe/audio extraction. When FFmpeg is not installed, falls
/// back to basic file-property inference.
pub struct VideoAnalyzer {
    config: VideoAnalysisConfig,
}

impl VideoAnalyzer {
    /// Create a new video analyzer with the given configuration.
    pub fn new(config: VideoAnalysisConfig) -> Self {
        Self { config }
    }

    /// Extract metadata from a video file.
    ///
    /// Uses `ffprobe -print_format json` when available, otherwise
    /// derives what it can from the filename and file size.
    pub fn extract_metadata(&self, path: &std::path::Path) -> Result<VideoMetadata, VideoError> {
        let file_size = std::fs::metadata(path)
            .map_err(|e| VideoError::IoError(e.to_string()))?
            .len() as usize;

        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown")
            .to_lowercase();

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
    fn extract_metadata_ffprobe(
        &self,
        path: &std::path::Path,
    ) -> Result<VideoMetadata, VideoError> {
        let output = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "quiet",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
            ])
            .arg(path)
            .output()
            .map_err(|e| VideoError::IoError(format!("ffprobe exec: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
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
            .and_then(|s| s.parse::<f64>().ok());

        let format_name = probe
            .pointer("/format/format_name")
            .and_then(|v| v.as_str())
            .map(|s| s.split(',').next().unwrap_or(s).to_string());

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
            let w = vs.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
            let h = vs.get("height").and_then(|v| v.as_u64()).map(|v| v as u32);
            let c = vs
                .get("codec_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
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
                });
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
    pub fn analyze(&self, path: &std::path::Path) -> Result<VideoAnalysis, VideoError> {
        let metadata = self.extract_metadata(path)?;

        // Check duration limits.
        if let Some(dur) = metadata.duration_secs {
            if dur > self.config.max_duration_secs {
                return Err(VideoError::TooLong {
                    duration_secs: dur,
                    max_secs: self.config.max_duration_secs,
                });
            }
        }

        let mut keyframe_paths = Vec::new();
        let mut audio_path: Option<String> = None;

        if Self::ffmpeg_available() {
            // Create a temporary output directory for keyframes.
            let out_dir = std::env::temp_dir().join(format!(
                "ironclaw_video_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            ));
            if let Err(e) = std::fs::create_dir_all(&out_dir) {
                tracing::warn!("Could not create keyframe output dir: {}", e);
            } else {
                // Extract keyframes using ffmpeg.
                match self.extract_keyframes(path, &out_dir) {
                    Ok(frames) => keyframe_paths = frames,
                    Err(e) => tracing::warn!("Keyframe extraction failed: {}", e),
                }

                // Extract audio track if configured and present.
                if self.config.extract_audio && metadata.has_audio {
                    match self.extract_audio(path, &out_dir) {
                        Ok(audio) => audio_path = Some(audio),
                        Err(e) => tracing::debug!("Audio extraction skipped: {}", e),
                    }
                }
            }
        }

        let summary = format_video_summary(&metadata, path);

        Ok(VideoAnalysis {
            metadata,
            keyframe_paths,
            audio_transcript: audio_path, // Path to extracted WAV for downstream transcription.
            summary,
        })
    }

    /// Extract keyframes at the configured interval using ffmpeg.
    fn extract_keyframes(
        &self,
        path: &std::path::Path,
        out_dir: &std::path::Path,
    ) -> Result<Vec<String>, VideoError> {
        let fps_filter = format!("fps=1/{}", self.config.keyframe_interval_secs);
        let scale_filter = format!(
            "scale='min({max_dim},iw)':min'({max_dim},ih)':force_original_aspect_ratio=decrease",
            max_dim = self.config.keyframe_max_dimension
        );
        let output_pattern = out_dir.join("frame_%04d.jpg");

        let status = std::process::Command::new("ffmpeg")
            .args([
                "-i",
                &path.to_string_lossy(),
                "-vf",
                &format!("{},{}", fps_filter, scale_filter),
                "-frames:v",
                &self.config.max_keyframes.to_string(),
                "-q:v",
                "2",  // High JPEG quality.
                "-y", // Overwrite.
            ])
            .arg(&output_pattern)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| VideoError::IoError(format!("ffmpeg exec: {}", e)))?;

        if !status.success() {
            return Err(VideoError::AnalysisFailed(format!(
                "ffmpeg keyframe extraction exited with {}",
                status
            )));
        }

        // Collect extracted frames, sorted by name.
        let mut frames: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(out_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().is_some_and(|e| e == "jpg") {
                    frames.push(p.to_string_lossy().to_string());
                }
            }
        }
        frames.sort();

        tracing::info!(
            count = frames.len(),
            "Extracted keyframes from {}",
            path.display()
        );

        Ok(frames)
    }

    /// Extract the audio track as a WAV file for transcription.
    fn extract_audio(
        &self,
        path: &std::path::Path,
        out_dir: &std::path::Path,
    ) -> Result<String, VideoError> {
        let audio_out = out_dir.join("audio.wav");

        let status = std::process::Command::new("ffmpeg")
            .args([
                "-i",
                &path.to_string_lossy(),
                "-vn", // No video.
                "-acodec",
                "pcm_s16le", // 16-bit PCM WAV (Whisper-compatible).
                "-ar",
                "16000", // 16kHz sample rate (Whisper default).
                "-ac",
                "1", // Mono.
                "-y",
            ])
            .arg(&audio_out)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| VideoError::IoError(format!("ffmpeg audio exec: {}", e)))?;

        if !status.success() {
            return Err(VideoError::AnalysisFailed(
                "ffmpeg audio extraction failed".to_string(),
            ));
        }

        Ok(audio_out.to_string_lossy().to_string())
    }

    /// Check if ffmpeg is available on the system.
    pub fn ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Check if ffprobe is available on the system.
    pub fn ffprobe_available() -> bool {
        std::process::Command::new("ffprobe")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
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
fn format_video_summary(metadata: &VideoMetadata, path: &std::path::Path) -> String {
    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("video");

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
}
