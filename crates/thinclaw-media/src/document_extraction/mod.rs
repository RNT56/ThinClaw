//! Document text extraction pipeline.
//!
//! Provides text extraction from document attachments so the LLM can reason
//! about uploaded files. Supports PDF, Office XML, and plain text formats.

pub mod extractors;

/// Maximum extracted text length (100K chars ≈ ~25K tokens).
pub const MAX_EXTRACTED_TEXT_LEN: usize = 100_000;

/// Maximum document size (10 MB).
pub const MAX_DOCUMENT_SIZE: usize = 10 * 1024 * 1024;
