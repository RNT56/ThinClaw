//! Document text extraction pipeline.
//!
//! Provides text extraction from document attachments so the LLM can reason
//! about uploaded files. Integrates with the media pipeline as a complement
//! to the built-in lightweight PDF parser in `media/pdf.rs`.
//!
//! Supported formats:
//! - **PDF** — via `pdf-extract` crate (full CMap/font support)
//! - **Office XML** (DOCX, PPTX, XLSX) — ZIP + XML text extraction
//! - **Plain text** (TXT, CSV, JSON, XML, Markdown, code) — UTF-8 decode
//!
//! This module is feature-gated behind `document-extraction`.

pub mod extractors;

/// Maximum extracted text length (100K chars ≈ ~25K tokens).
pub const MAX_EXTRACTED_TEXT_LEN: usize = 100_000;

/// Maximum document size (10 MB).
pub const MAX_DOCUMENT_SIZE: usize = 10 * 1024 * 1024;
