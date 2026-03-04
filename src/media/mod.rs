//! Media type detection, content extraction, and attachment handling.
//!
//! Provides a unified pipeline for processing binary attachments received
//! through messaging channels (images, PDFs, audio files, etc.) into
//! text or structured content that can be fed to the LLM.
//!
//! # Architecture
//!
//! ```text
//! IncomingMessage ─► MediaContent ─► MediaExtractor ─► extracted text
//!                    (raw bytes)     (type-specific)    (for LLM context)
//! ```

mod audio;
pub mod cache;
mod image;
pub mod limits;
mod pdf;
pub mod tts;
mod types;
pub mod video;

pub use audio::AudioExtractor;
pub use cache::{CacheConfig, CacheStats, MediaCache};
pub use image::ImageExtractor;
pub use limits::MediaLimits;
pub use pdf::PdfExtractor;
pub use tts::{TtsConfig, TtsError, TtsOutputFormat, TtsProvider, TtsSynthesizer, TtsVoice};
pub use types::{MediaContent, MediaExtractError, MediaExtractor, MediaPipeline, MediaType};
pub use video::{VideoAnalysis, VideoAnalysisConfig, VideoAnalyzer, VideoError, VideoMetadata};
