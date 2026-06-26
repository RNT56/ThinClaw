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
//!
//! Most of this subsystem lives in the `thinclaw-media` crate. This module is
//! a thin façade that re-exports the crate's public API and keeps the two
//! root-local pieces that depend on root services:
//!
//! - [`audio::AudioExtractor`] resolves its Whisper endpoint through root
//!   `crate::config`, so it cannot move into `thinclaw-media` without creating
//!   a `media → config → channels → media` dependency cycle.
//! - [`pipeline::MediaPipeline`] registers `AudioExtractor` in its default
//!   extractor set, so it stays root-local alongside `audio`.

mod audio;
pub mod cache;
#[cfg(feature = "document-extraction")]
mod document;
mod image;
pub mod limits;
mod pdf;
mod pipeline;
pub mod sticker;
pub mod tts;
pub mod tts_streaming;
mod types;
pub mod video;

pub use audio::AudioExtractor;
pub use cache::{CacheConfig, CacheStats, MediaCache};
#[cfg(feature = "document-extraction")]
pub use document::DocumentExtractor;
pub use image::ImageExtractor;
pub use limits::MediaLimits;
pub use pdf::PdfExtractor;
pub use pipeline::MediaPipeline;
pub use tts::{TtsConfig, TtsError, TtsOutputFormat, TtsProvider, TtsSynthesizer, TtsVoice};
pub use tts_streaming::{
    IncrementalTtsConfig, SentenceChunker, TtsChunk, TtsChunkFormat, TtsPlaybackProgress,
};
pub use types::{MediaContent, MediaExtractError, MediaExtractor, MediaType};
pub use video::{VideoAnalysis, VideoAnalysisConfig, VideoAnalyzer, VideoError, VideoMetadata};
