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
mod image;
mod pdf;
mod types;

pub use audio::AudioExtractor;
pub use image::ImageExtractor;
pub use pdf::PdfExtractor;
pub use types::{MediaContent, MediaExtractError, MediaExtractor, MediaPipeline, MediaType};
