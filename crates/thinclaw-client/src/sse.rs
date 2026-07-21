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
    buffer: Vec<u8>,
    data_lines: Vec<Vec<u8>>,
    frame_bytes: usize,
}

const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;
const MAX_SSE_FRAME_BYTES: usize = 1024 * 1024;
const MAX_SSE_DATA_LINES: usize = 10_000;

impl SseDecoder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of bytes; returns any frames completed by this chunk as
    /// their joined `data` payloads.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, &'static str> {
        if self.buffer.len().saturating_add(chunk.len()) > MAX_SSE_LINE_BYTES
            && !chunk.contains(&b'\n')
        {
            return Err("SSE line exceeds the size limit");
        }
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();

        // Process complete lines; keep the trailing partial line in `buffer`.
        loop {
            let Some(nl) = self.buffer.iter().position(|byte| *byte == b'\n') else {
                break;
            };
            if nl > MAX_SSE_LINE_BYTES {
                return Err("SSE line exceeds the size limit");
            }
            let mut line = self.buffer.drain(..=nl).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }

            if line.is_empty() {
                // Frame boundary: emit the accumulated data if any.
                if !self.data_lines.is_empty() {
                    let mut payload = Vec::with_capacity(self.frame_bytes);
                    for (index, data) in self.data_lines.iter().enumerate() {
                        if index > 0 {
                            payload.push(b'\n');
                        }
                        payload.extend_from_slice(data);
                    }
                    frames.push(
                        String::from_utf8(payload)
                            .map_err(|_| "SSE data payload is not valid UTF-8")?,
                    );
                    self.data_lines.clear();
                    self.frame_bytes = 0;
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix(b"data:") {
                let rest = rest.strip_prefix(b" ").unwrap_or(rest);
                let added = rest.len() + usize::from(!self.data_lines.is_empty());
                if self.frame_bytes.saturating_add(added) > MAX_SSE_FRAME_BYTES
                    || self.data_lines.len() >= MAX_SSE_DATA_LINES
                {
                    return Err("SSE frame exceeds the size limit");
                }
                self.frame_bytes += added;
                self.data_lines.push(rest.to_vec());
            }
            // `event:`, `id:`, `retry:`, and `:` comments are ignored.
        }

        if self.buffer.len() > MAX_SSE_LINE_BYTES {
            return Err("SSE line exceeds the size limit");
        }
        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_frames_on_blank_line() {
        let mut d = SseDecoder::new();
        let frames = d.push(b"data: {\"a\":1}\n\ndata: {\"b\":2}\n\n").unwrap();
        assert_eq!(
            frames,
            vec!["{\"a\":1}".to_string(), "{\"b\":2}".to_string()]
        );
    }

    #[test]
    fn handles_split_across_chunks() {
        let mut d = SseDecoder::new();
        assert!(d.push(b"data: {\"a\"").unwrap().is_empty());
        assert!(d.push(b":1}").unwrap().is_empty());
        let frames = d.push(b"\n\n").unwrap();
        assert_eq!(frames, vec!["{\"a\":1}".to_string()]);
    }

    #[test]
    fn ignores_comments_and_event_lines() {
        let mut d = SseDecoder::new();
        let frames = d
            .push(b": keep-alive\nevent: message\ndata: hi\n\n")
            .unwrap();
        assert_eq!(frames, vec!["hi".to_string()]);
    }

    #[test]
    fn joins_multiline_data() {
        let mut d = SseDecoder::new();
        let frames = d.push(b"data: line1\ndata: line2\n\n").unwrap();
        assert_eq!(frames, vec!["line1\nline2".to_string()]);
    }

    #[test]
    fn rejects_unbounded_frame() {
        let mut decoder = SseDecoder::new();
        let chunk = vec![b'x'; MAX_SSE_LINE_BYTES + 1];
        assert!(decoder.push(&chunk).is_err());
    }
}
