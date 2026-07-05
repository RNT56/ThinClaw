//! Minimal Server-Sent Events frame parser.
//!
//! We consume `reqwest`'s raw byte stream and split it into SSE frames without
//! pulling in an SSE dependency. A frame is terminated by a blank line; within
//! a frame we collect `data:` lines (joined with `\n` per the SSE spec) and
//! ignore other fields (`event:`, `id:`, `:comment`). Each completed frame's
//! joined data payload is yielded.

/// Incremental SSE frame accumulator.
#[derive(Default)]
pub(crate) struct SseDecoder {
    buffer: String,
    data_lines: Vec<String>,
}

impl SseDecoder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of bytes; returns any frames completed by this chunk as
    /// their joined `data` payloads.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        let mut frames = Vec::new();

        // Process complete lines; keep the trailing partial line in `buffer`.
        loop {
            let Some(nl) = self.buffer.find('\n') else {
                break;
            };
            let line = self.buffer[..nl].trim_end_matches('\r').to_string();
            self.buffer.drain(..=nl);

            if line.is_empty() {
                // Frame boundary: emit the accumulated data if any.
                if !self.data_lines.is_empty() {
                    frames.push(self.data_lines.join("\n"));
                    self.data_lines.clear();
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                self.data_lines
                    .push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
            }
            // `event:`, `id:`, `retry:`, and `:` comments are ignored.
        }

        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_frames_on_blank_line() {
        let mut d = SseDecoder::new();
        let frames = d.push(b"data: {\"a\":1}\n\ndata: {\"b\":2}\n\n");
        assert_eq!(
            frames,
            vec!["{\"a\":1}".to_string(), "{\"b\":2}".to_string()]
        );
    }

    #[test]
    fn handles_split_across_chunks() {
        let mut d = SseDecoder::new();
        assert!(d.push(b"data: {\"a\"").is_empty());
        assert!(d.push(b":1}").is_empty());
        let frames = d.push(b"\n\n");
        assert_eq!(frames, vec!["{\"a\":1}".to_string()]);
    }

    #[test]
    fn ignores_comments_and_event_lines() {
        let mut d = SseDecoder::new();
        let frames = d.push(b": keep-alive\nevent: message\ndata: hi\n\n");
        assert_eq!(frames, vec!["hi".to_string()]);
    }

    #[test]
    fn joins_multiline_data() {
        let mut d = SseDecoder::new();
        let frames = d.push(b"data: line1\ndata: line2\n\n");
        assert_eq!(frames, vec!["line1\nline2".to_string()]);
    }
}
