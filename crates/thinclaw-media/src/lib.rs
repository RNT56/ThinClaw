//! Media domain crate.

pub mod cache;
pub mod comfyui;
#[cfg(feature = "document-extraction")]
pub mod document_extraction;
pub mod limits;
pub mod sticker;
pub mod tts;
pub mod tts_streaming;
pub mod video;

pub use cache::{CacheConfig, CacheStats, MediaCache};
pub use comfyui::*;
pub use limits::MediaLimits;
pub use sticker::{
    ConvertedSticker, StickerConfig, StickerError, StickerFormat, convert_sticker,
    is_ffmpeg_available,
};
pub use thinclaw_types::{MediaContent, MediaType};
pub use tts::{TtsConfig, TtsError, TtsOutputFormat, TtsProvider, TtsSynthesizer, TtsVoice};
pub use tts_streaming::{
    IncrementalTtsConfig, SentenceChunker, TtsChunk, TtsChunkFormat, TtsPlaybackProgress,
};
pub use video::{VideoAnalysis, VideoAnalysisConfig, VideoAnalyzer, VideoError, VideoMetadata};
