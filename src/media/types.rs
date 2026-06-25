//! Façade for core media extraction traits — see `thinclaw_media::extractor`.
//!
//! `MediaContent`/`MediaType` are owned by `thinclaw-types`; the
//! `MediaExtractor` trait and `MediaExtractError` are owned by
//! `thinclaw-media`. The `MediaPipeline` glue stays root-local in
//! `super` because its default extractor set includes `AudioExtractor`,
//! which depends on root `crate::config` (see `super::audio`).

pub use thinclaw_media::extractor::{MediaContent, MediaExtractError, MediaExtractor, MediaType};
