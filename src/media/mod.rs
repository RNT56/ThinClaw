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
pub mod media_cache_config;
#[cfg(feature = "document-extraction")]
mod document;
mod pdf;
pub mod sticker;
pub mod tts;
pub mod tts_streaming;
mod types;
pub mod video;

pub use audio::AudioExtractor;
pub use cache::{CacheConfig, CacheStats, MediaCache};
pub use image::ImageExtractor;
pub use limits::MediaLimits;
#[cfg(feature = "document-extraction")]
pub use document::DocumentExtractor;
pub use pdf::PdfExtractor;
pub use tts::{TtsConfig, TtsError, TtsOutputFormat, TtsProvider, TtsSynthesizer, TtsVoice};
pub use tts_streaming::{
    IncrementalTtsConfig, SentenceChunker, TtsChunk, TtsChunkFormat, TtsPlaybackProgress,
};
pub use types::{MediaContent, MediaExtractError, MediaExtractor, MediaPipeline, MediaType};
pub use video::{VideoAnalysis, VideoAnalysisConfig, VideoAnalyzer, VideoError, VideoMetadata};
