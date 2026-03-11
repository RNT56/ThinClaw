//! Image handling: detection, base64 encoding, resize config, and LLM-ready formatting.

use super::types::{MediaContent, MediaExtractError, MediaExtractor, MediaType};

/// Extracts a text description from images.
///
/// For images that can't be sent natively to a multimodal LLM, this produces
/// a placeholder description. For multimodal-capable pipelines, use
/// `format_for_llm()` to get the image as a base64 content block.
pub struct ImageExtractor {
    /// Maximum image size in bytes (default: 20 MB).
    max_image_size: usize,
    /// Maximum width for resize hint (default: 2048).
    /// Used to set the `detail` parameter for vision models.
    max_width: u32,
    /// Maximum height for resize hint (default: 2048).
    max_height: u32,
}

impl ImageExtractor {
    /// Create a new image extractor with default settings.
    pub fn new() -> Self {
        Self {
            max_image_size: 20 * 1024 * 1024,
            max_width: 2048,
            max_height: 2048,
        }
    }

    /// Set the maximum image size.
    pub fn with_max_size(mut self, max_bytes: usize) -> Self {
        self.max_image_size = max_bytes;
        self
    }

    /// Set the maximum resize dimensions.
    ///
    /// Images larger than these dimensions will use the `low` detail level
    /// to reduce token consumption. Configurable per-agent via
    /// `IMAGE_MAX_WIDTH` / `IMAGE_MAX_HEIGHT` environment variables.
    pub fn with_max_dimensions(mut self, max_width: u32, max_height: u32) -> Self {
        self.max_width = max_width;
        self.max_height = max_height;
        self
    }

    /// Determine the OpenAI `detail` level based on image dimensions.
    ///
    /// - `high`: image fits within configured max dims
    /// - `low`: image exceeds max dims or dimensions unknown
    fn detail_level(&self, data: &[u8]) -> &'static str {
        match Self::detect_dimensions(data) {
            Some((w, h)) if w <= self.max_width && h <= self.max_height => "high",
            Some(_) => "low", // Image too large → use low detail to save tokens
            None => "auto",   // Unknown dims → let model decide
        }
    }

    /// Format an image as a base64 content block for multimodal LLMs.
    ///
    /// Returns a JSON-like structure that can be embedded in a multimodal
    /// message (OpenAI vision format). Includes a `detail` hint based on
    /// image dimensions vs configured max resize dims.
    pub fn format_for_llm(content: &MediaContent) -> serde_json::Value {
        serde_json::json!({
            "type": "image_url",
            "image_url": {
                "url": content.to_data_uri(),
            }
        })
    }

    /// Format an image with dimension-aware detail level.
    pub fn format_for_llm_with_detail(&self, content: &MediaContent) -> serde_json::Value {
        let detail = self.detail_level(&content.data);
        serde_json::json!({
            "type": "image_url",
            "image_url": {
                "url": content.to_data_uri(),
                "detail": detail,
            }
        })
    }

    /// Format multiple images as content blocks for a single multimodal message.
    ///
    /// Returns a JSON array of image content blocks that can be included alongside
    /// text in a single user message. This enables multi-image tool calls.
    pub fn format_multiple_for_llm(&self, contents: &[&MediaContent]) -> Vec<serde_json::Value> {
        contents
            .iter()
            .map(|content| self.format_for_llm_with_detail(content))
            .collect()
    }

    /// Detect image dimensions from raw bytes (JPEG, PNG, GIF, WebP).
    ///
    /// Returns `(width, height)` or `None` if the format is unrecognized.
    pub fn detect_dimensions(data: &[u8]) -> Option<(u32, u32)> {
        // PNG: bytes 16-23 contain width (4 bytes) and height (4 bytes) in IHDR
        if data.len() >= 24 && data[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
            let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
            let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
            return Some((w, h));
        }

        // GIF: bytes 6-9 contain width (2 LE) and height (2 LE)
        if data.len() >= 10 && (data[0..6] == *b"GIF87a" || data[0..6] == *b"GIF89a") {
            let w = u16::from_le_bytes([data[6], data[7]]) as u32;
            let h = u16::from_le_bytes([data[8], data[9]]) as u32;
            return Some((w, h));
        }

        // WebP: RIFF header + VP8 chunk
        if data.len() >= 30 && data[0..4] == *b"RIFF" && data[8..12] == *b"WEBP" {
            // VP8 (lossy): width/height at bytes 26-29
            if data[12..16] == *b"VP8 " && data.len() >= 30 {
                let w = (u16::from_le_bytes([data[26], data[27]]) & 0x3FFF) as u32;
                let h = (u16::from_le_bytes([data[28], data[29]]) & 0x3FFF) as u32;
                return Some((w, h));
            }
        }

        // JPEG: scan for SOF0/SOF2 markers
        if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 {
            let mut i = 2;
            while i + 8 < data.len() {
                if data[i] != 0xFF {
                    i += 1;
                    continue;
                }
                let marker = data[i + 1];
                // SOF0 or SOF2 (baseline/progressive)
                if marker == 0xC0 || marker == 0xC2 {
                    let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
                    let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
                    return Some((w, h));
                }
                // Skip other markers
                if i + 3 < data.len() {
                    let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
                    i += 2 + len;
                } else {
                    break;
                }
            }
        }

        None
    }
}

impl Default for ImageExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaExtractor for ImageExtractor {
    fn supported_types(&self) -> &[MediaType] {
        &[MediaType::Image]
    }

    fn extract_text(&self, content: &MediaContent) -> Result<String, MediaExtractError> {
        if content.size() > self.max_image_size {
            return Err(MediaExtractError::TooLarge {
                size: content.size(),
                max: self.max_image_size,
            });
        }

        let dims = Self::detect_dimensions(&content.data);
        let size_kb = content.size() / 1024;

        let desc = if let Some((w, h)) = dims {
            format!(
                "[Image: {} ({}×{}, {} KB)]",
                content.filename.as_deref().unwrap_or(&content.mime_type),
                w,
                h,
                size_kb,
            )
        } else {
            format!(
                "[Image: {} ({} KB)]",
                content.filename.as_deref().unwrap_or(&content.mime_type),
                size_kb,
            )
        };

        Ok(desc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_text_basic() {
        let mc = MediaContent::new(vec![0xFF, 0xD8, 0xFF], "image/jpeg")
            .with_filename("photo.jpg".to_string());
        let extractor = ImageExtractor::new();
        let text = extractor.extract_text(&mc).unwrap();
        assert!(text.contains("photo.jpg"));
        assert!(text.contains("Image"));
    }

    #[test]
    fn test_extract_text_too_large() {
        let extractor = ImageExtractor::new().with_max_size(10);
        let mc = MediaContent::new(vec![0; 100], "image/png");
        assert!(matches!(
            extractor.extract_text(&mc),
            Err(MediaExtractError::TooLarge { .. })
        ));
    }

    #[test]
    fn test_format_for_llm() {
        let mc = MediaContent::new(vec![1, 2, 3], "image/png");
        let block = ImageExtractor::format_for_llm(&mc);
        let url = block["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn test_detect_png_dimensions() {
        // Minimal valid PNG header with 100×200 dimensions
        let mut data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        // IHDR chunk: length (13), "IHDR", width, height, ...
        data.extend_from_slice(&[0, 0, 0, 13]); // chunk length
        data.extend_from_slice(b"IHDR");
        data.extend_from_slice(&100u32.to_be_bytes()); // width
        data.extend_from_slice(&200u32.to_be_bytes()); // height
        data.extend_from_slice(&[8, 6, 0, 0, 0]); // bit depth, color type, etc.

        assert_eq!(ImageExtractor::detect_dimensions(&data), Some((100, 200)));
    }

    #[test]
    fn test_detect_gif_dimensions() {
        let mut data = b"GIF89a".to_vec();
        data.extend_from_slice(&320u16.to_le_bytes()); // width
        data.extend_from_slice(&240u16.to_le_bytes()); // height
        assert_eq!(ImageExtractor::detect_dimensions(&data), Some((320, 240)));
    }

    #[test]
    fn test_detect_unknown_format() {
        assert_eq!(ImageExtractor::detect_dimensions(&[0, 0, 0, 0]), None);
    }

    #[test]
    fn test_supported_types() {
        let extractor = ImageExtractor::new();
        assert_eq!(extractor.supported_types(), &[MediaType::Image]);
    }

    #[test]
    fn test_detail_level_high_for_small_image() {
        let extractor = ImageExtractor::new().with_max_dimensions(1024, 1024);
        // 100×200 PNG fits within 1024×1024
        let mut data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        data.extend_from_slice(&[0, 0, 0, 13]);
        data.extend_from_slice(b"IHDR");
        data.extend_from_slice(&100u32.to_be_bytes());
        data.extend_from_slice(&200u32.to_be_bytes());
        data.extend_from_slice(&[8, 6, 0, 0, 0]);
        assert_eq!(extractor.detail_level(&data), "high");
    }

    #[test]
    fn test_detail_level_low_for_large_image() {
        let extractor = ImageExtractor::new().with_max_dimensions(512, 512);
        // 800×600 PNG exceeds 512×512
        let mut data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        data.extend_from_slice(&[0, 0, 0, 13]);
        data.extend_from_slice(b"IHDR");
        data.extend_from_slice(&800u32.to_be_bytes());
        data.extend_from_slice(&600u32.to_be_bytes());
        data.extend_from_slice(&[8, 6, 0, 0, 0]);
        assert_eq!(extractor.detail_level(&data), "low");
    }

    #[test]
    fn test_format_multiple_for_llm() {
        let mc1 = MediaContent::new(vec![1, 2, 3], "image/png");
        let mc2 = MediaContent::new(vec![4, 5, 6], "image/jpeg");
        let extractor = ImageExtractor::new();
        let blocks = extractor.format_multiple_for_llm(&[&mc1, &mc2]);
        assert_eq!(blocks.len(), 2);
        assert!(
            blocks[0]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert!(
            blocks[1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
    }

    #[test]
    fn test_format_with_detail_includes_detail_field() {
        let mc = MediaContent::new(vec![1, 2, 3], "image/png");
        let extractor = ImageExtractor::new();
        let block = extractor.format_for_llm_with_detail(&mc);
        assert!(block["image_url"]["detail"].is_string());
    }
}
