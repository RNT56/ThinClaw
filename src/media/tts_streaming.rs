//! Incremental TTS playback — chunked, streaming text-to-speech.
//!
//! Breaks long text into sentence-aligned chunks and synthesizes them
//! incrementally, so the client can begin playback of chunk 0 while
//! chunk 1 is still being generated. This eliminates the "wait for
//! full response" latency that makes TTS feel sluggish.
//!
//! # Architecture
//!
//! ```text
//! LLM tokens ──► SentenceChunker ──► TtsChunkSynthesizer ──► audio chunks
//!                 (buffer until       (parallel synthesis)     (progressive
//!                  sentence end)                                playback)
//! ```
//!
//! # SSE Format
//!
//! Audio chunks are delivered via Server-Sent Events:
//! ```text
//! event: tts_chunk
//! data: {"chunk_index":0,"audio_base64":"...","format":"mp3","is_final":false}
//!
//! event: tts_chunk
//! data: {"chunk_index":1,"audio_base64":"...","format":"mp3","is_final":true}
//! ```

use serde::{Deserialize, Serialize};

/// Configuration for incremental TTS playback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncrementalTtsConfig {
    /// Minimum characters to buffer before synthesizing a chunk.
    /// Prevents too-short audio fragments.
    pub min_chunk_chars: usize,

    /// Maximum characters per chunk.
    /// Prevents chunks from growing unbounded in case of missing punctuation.
    pub max_chunk_chars: usize,

    /// Whether to pre-buffer the first N characters before starting synthesis.
    /// Helps avoid stuttery starts from single-word chunks.
    pub pre_buffer_chars: usize,

    /// Audio format for the chunks.
    pub format: TtsChunkFormat,

    /// Whether to send silence padding between chunks.
    /// Helps smooth playback transitions.
    pub inter_chunk_silence_ms: u32,

    /// Maximum number of chunks to hold in the output buffer.
    /// Old chunks are dropped if the client isn't consuming fast enough.
    pub max_buffered_chunks: usize,
}

impl Default for IncrementalTtsConfig {
    fn default() -> Self {
        Self {
            min_chunk_chars: 20,
            max_chunk_chars: 200,
            pre_buffer_chars: 40,
            format: TtsChunkFormat::Mp3,
            inter_chunk_silence_ms: 0,
            max_buffered_chunks: 10,
        }
    }
}

/// Audio format for streaming TTS chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TtsChunkFormat {
    Mp3,
    Opus,
    Pcm,
}

impl std::fmt::Display for TtsChunkFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mp3 => write!(f, "mp3"),
            Self::Opus => write!(f, "opus"),
            Self::Pcm => write!(f, "pcm"),
        }
    }
}

/// A single synthesized audio chunk ready for streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsChunk {
    /// Zero-based index of this chunk in the stream.
    pub chunk_index: u32,

    /// Audio data as base64-encoded bytes.
    pub audio_base64: String,

    /// Source text that was synthesized.
    pub source_text: String,

    /// Audio format.
    pub format: TtsChunkFormat,

    /// Whether this is the final chunk in the stream.
    pub is_final: bool,

    /// Duration estimate in milliseconds (based on text length heuristic).
    pub estimated_duration_ms: u32,
}

/// Breaks streaming text into sentence-aligned chunks.
///
/// Feed tokens as they arrive from the LLM; the chunker buffers
/// until a sentence boundary is detected, then emits a chunk.
pub struct SentenceChunker {
    config: IncrementalTtsConfig,
    buffer: String,
    chunk_index: u32,
    finished: bool,
}

impl SentenceChunker {
    pub fn new(config: IncrementalTtsConfig) -> Self {
        Self {
            config,
            buffer: String::new(),
            chunk_index: 0,
            finished: false,
        }
    }

    /// Feed text (e.g., a token from the LLM stream).
    ///
    /// Returns a chunk if the buffer has accumulated a complete sentence
    /// that meets the minimum length threshold.
    pub fn feed(&mut self, text: &str) -> Option<String> {
        if self.finished {
            return None;
        }

        self.buffer.push_str(text);

        // Check if we've hit a sentence boundary with sufficient text
        if self.buffer.len() >= self.config.min_chunk_chars
            && let Some(pos) = self.find_sentence_boundary()
        {
            let chunk = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos..].trim_start().to_string();
            self.chunk_index += 1;
            return Some(chunk);
        }

        // Force-emit if we've exceeded max chunk size
        if self.buffer.len() >= self.config.max_chunk_chars {
            // Try to find a word boundary
            let split_at = self
                .find_word_boundary_near(self.config.max_chunk_chars)
                .unwrap_or(self.config.max_chunk_chars);
            let chunk = self.buffer[..split_at].to_string();
            self.buffer = self.buffer[split_at..].trim_start().to_string();
            self.chunk_index += 1;
            return Some(chunk);
        }

        None
    }

    /// Flush remaining buffer as the final chunk.
    ///
    /// Call this when the LLM stream ends.
    pub fn flush(&mut self) -> Option<String> {
        if self.finished {
            return None;
        }
        self.finished = true;

        let remaining = self.buffer.trim().to_string();
        self.buffer.clear();

        if remaining.is_empty() {
            None
        } else {
            self.chunk_index += 1;
            Some(remaining)
        }
    }

    /// Current chunk index.
    pub fn chunk_index(&self) -> u32 {
        self.chunk_index
    }

    /// Whether the chunker has finished (flush was called).
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Number of characters currently buffered.
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    /// Find the first sentence boundary in the buffer that meets min_chunk_chars.
    fn find_sentence_boundary(&self) -> Option<usize> {
        // Look for sentence-ending punctuation followed by a space or end
        let bytes = self.buffer.as_bytes();

        for (i, &b) in bytes.iter().enumerate() {
            if (b == b'.' || b == b'!' || b == b'?' || b == b';') && i > 0 {
                // Check it's followed by whitespace or end of buffer
                if i + 1 >= bytes.len() || bytes[i + 1] == b' ' || bytes[i + 1] == b'\n' {
                    let pos = i + 1;
                    if pos >= self.config.min_chunk_chars {
                        return Some(pos);
                    }
                }
            }
        }

        None
    }

    /// Find a word boundary near the target position.
    fn find_word_boundary_near(&self, target: usize) -> Option<usize> {
        let bytes = self.buffer.as_bytes();
        let max = bytes.len().min(target);

        // Search backwards from target for a space
        for i in (0..max).rev() {
            if bytes[i] == b' ' || bytes[i] == b'\n' {
                return Some(i + 1);
            }
        }

        None
    }
}

/// Progress update for incremental TTS playback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsPlaybackProgress {
    /// Number of chunks synthesized so far.
    pub chunks_synthesized: u32,
    /// Number of chunks played to the client.
    pub chunks_played: u32,
    /// Total characters of source text processed.
    pub chars_processed: usize,
    /// Whether all chunks have been synthesized.
    pub synthesis_complete: bool,
    /// Whether all chunks have been played.
    pub playback_complete: bool,
    /// Estimated percentage complete (0.0 - 1.0).
    pub progress: f32,
}

/// Estimate audio duration from text length.
///
/// Average speaking rate: ~150 words per minute = ~2.5 words/sec
/// Average word: ~5 characters
/// So: ~12.5 chars/sec → ~80ms per character
pub fn estimate_duration_ms(text: &str) -> u32 {
    let char_count = text.chars().count();
    (char_count as u32) * 80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = IncrementalTtsConfig::default();
        assert_eq!(config.min_chunk_chars, 20);
        assert_eq!(config.max_chunk_chars, 200);
        assert_eq!(config.format, TtsChunkFormat::Mp3);
    }

    #[test]
    fn test_chunk_format_display() {
        assert_eq!(TtsChunkFormat::Mp3.to_string(), "mp3");
        assert_eq!(TtsChunkFormat::Opus.to_string(), "opus");
        assert_eq!(TtsChunkFormat::Pcm.to_string(), "pcm");
    }

    #[test]
    fn test_chunker_short_text_no_emit() {
        let mut chunker = SentenceChunker::new(IncrementalTtsConfig::default());
        assert!(chunker.feed("Hi").is_none());
        assert_eq!(chunker.buffered_len(), 2);
    }

    #[test]
    fn test_chunker_sentence_boundary() {
        let config = IncrementalTtsConfig {
            min_chunk_chars: 5,
            max_chunk_chars: 200,
            ..Default::default()
        };
        let mut chunker = SentenceChunker::new(config);

        // "Hello there. " = 13 chars; boundary at 12 (after '.'), 12 >= 5
        // So it DOES emit
        let result = chunker.feed("Hello there. ");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Hello there.");
    }

    #[test]
    fn test_chunker_emits_on_sentence() {
        let config = IncrementalTtsConfig {
            min_chunk_chars: 5,
            max_chunk_chars: 200,
            ..Default::default()
        };
        let mut chunker = SentenceChunker::new(config);

        let result = chunker.feed("Hello world. How are you?");
        // Buffer = "Hello world. How are you?" (25 chars)
        // Sentence boundary at index 12 (after '.')
        // 12 >= 5, so emit "Hello world."
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Hello world.");
    }

    #[test]
    fn test_chunker_max_chunk_force_emit() {
        let config = IncrementalTtsConfig {
            min_chunk_chars: 5,
            max_chunk_chars: 20,
            ..Default::default()
        };
        let mut chunker = SentenceChunker::new(config);

        // No sentence boundary, but exceeds max
        let result = chunker.feed("This is a long text without any punctuation at all");
        assert!(result.is_some());
        let chunk = result.unwrap();
        assert!(chunk.len() <= 20);
    }

    #[test]
    fn test_chunker_flush() {
        let config = IncrementalTtsConfig {
            min_chunk_chars: 100, // High threshold so nothing emits during feed
            ..Default::default()
        };
        let mut chunker = SentenceChunker::new(config);

        chunker.feed("Remaining text");
        assert_eq!(chunker.buffered_len(), 14);

        let flushed = chunker.flush();
        assert!(flushed.is_some());
        assert_eq!(flushed.unwrap(), "Remaining text");
        assert!(chunker.is_finished());
    }

    #[test]
    fn test_chunker_flush_empty() {
        let mut chunker = SentenceChunker::new(IncrementalTtsConfig::default());
        let flushed = chunker.flush();
        assert!(flushed.is_none());
    }

    #[test]
    fn test_chunker_no_feed_after_flush() {
        let mut chunker = SentenceChunker::new(IncrementalTtsConfig::default());
        chunker.flush();
        assert!(chunker.feed("more text").is_none());
    }

    #[test]
    fn test_chunker_chunk_index() {
        let config = IncrementalTtsConfig {
            min_chunk_chars: 5,
            max_chunk_chars: 200,
            ..Default::default()
        };
        let mut chunker = SentenceChunker::new(config);
        assert_eq!(chunker.chunk_index(), 0);

        chunker.feed("First sentence. ");
        assert_eq!(chunker.chunk_index(), 1);
    }

    #[test]
    fn test_estimate_duration() {
        let ms = estimate_duration_ms("Hello, world!");
        // 13 chars * 80ms = 1040ms
        assert_eq!(ms, 1040);
    }

    #[test]
    fn test_estimate_duration_empty() {
        assert_eq!(estimate_duration_ms(""), 0);
    }

    #[test]
    fn test_tts_chunk_serializable() {
        let chunk = TtsChunk {
            chunk_index: 0,
            audio_base64: "dGVzdA==".to_string(),
            source_text: "Hello".to_string(),
            format: TtsChunkFormat::Mp3,
            is_final: false,
            estimated_duration_ms: 400,
        };
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains("\"chunk_index\":0"));
        assert!(json.contains("\"is_final\":false"));
        assert!(json.contains("\"format\":\"mp3\""));

        let deser: TtsChunk = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.chunk_index, 0);
        assert_eq!(deser.source_text, "Hello");
    }

    #[test]
    fn test_playback_progress_serializable() {
        let progress = TtsPlaybackProgress {
            chunks_synthesized: 3,
            chunks_played: 2,
            chars_processed: 150,
            synthesis_complete: false,
            playback_complete: false,
            progress: 0.66,
        };
        let json = serde_json::to_string(&progress).unwrap();
        assert!(json.contains("\"chunks_synthesized\":3"));
        assert!(json.contains("\"playback_complete\":false"));
    }

    #[test]
    fn test_config_serializable() {
        let config = IncrementalTtsConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"min_chunk_chars\":20"));
        let deser: IncrementalTtsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.min_chunk_chars, 20);
    }

    #[test]
    fn test_multiple_sentences() {
        let config = IncrementalTtsConfig {
            min_chunk_chars: 5,
            max_chunk_chars: 200,
            ..Default::default()
        };
        let mut chunker = SentenceChunker::new(config);

        // Feed multiple sentences
        let c1 = chunker.feed("First. ");
        assert!(c1.is_some());
        assert_eq!(c1.unwrap(), "First.");

        let c2 = chunker.feed("Second sentence. ");
        assert!(c2.is_some());
        assert_eq!(c2.unwrap(), "Second sentence.");

        let c3 = chunker.flush();
        assert!(c3.is_none()); // Nothing left
    }

    #[test]
    fn test_question_mark_boundary() {
        let config = IncrementalTtsConfig {
            min_chunk_chars: 5,
            max_chunk_chars: 200,
            ..Default::default()
        };
        let mut chunker = SentenceChunker::new(config);

        let result = chunker.feed("How are you? I am fine.");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "How are you?");
    }

    #[test]
    fn test_exclamation_boundary() {
        let config = IncrementalTtsConfig {
            min_chunk_chars: 5,
            max_chunk_chars: 200,
            ..Default::default()
        };
        let mut chunker = SentenceChunker::new(config);

        let result = chunker.feed("Wow, great! Thanks.");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Wow, great!");
    }
}
