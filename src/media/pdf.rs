//! PDF text extraction.
//!
//! Extracts plain text from PDF files. Uses a simple, dependency-light
//! approach that parses the PDF structure directly rather than requiring
//! heavy external dependencies.

use super::types::{MediaContent, MediaExtractError, MediaExtractor, MediaType};

/// Extracts text content from PDF files.
///
/// Uses a lightweight built-in approach:
/// 1. Validates PDF magic bytes
/// 2. Scans for text stream objects
/// 3. Extracts readable text content
///
/// For complex PDFs with embedded images or unusual encodings, consider
/// falling back to an external tool like `pdftotext`.
pub struct PdfExtractor {
    /// Maximum PDF size in bytes (default: 50 MB).
    max_pdf_size: usize,
    /// Maximum extracted text length (default: 500 KB).
    max_text_length: usize,
}

impl PdfExtractor {
    /// Create a new PDF extractor with default settings.
    pub fn new() -> Self {
        Self {
            max_pdf_size: 50 * 1024 * 1024,
            max_text_length: 500 * 1024,
        }
    }

    /// Set the maximum PDF file size.
    pub fn with_max_pdf_size(mut self, max_bytes: usize) -> Self {
        self.max_pdf_size = max_bytes;
        self
    }

    /// Set the maximum extracted text length.
    pub fn with_max_text_length(mut self, max_chars: usize) -> Self {
        self.max_text_length = max_chars;
        self
    }

    /// Extract text from PDF data using a simple stream-based approach.
    ///
    /// This scans for BT...ET text blocks and extracts Tj/TJ string operands.
    /// Works for most simple PDFs. Complex PDFs with CMap encodings or
    /// reordered pages may not extract perfectly.
    fn extract_text_from_pdf(data: &[u8]) -> Result<String, MediaExtractError> {
        // Validate PDF magic
        if data.len() < 5 || &data[0..5] != b"%PDF-" {
            return Err(MediaExtractError::ExtractionFailed {
                reason: "Not a valid PDF file (missing %PDF- header)".to_string(),
            });
        }

        let content = String::from_utf8_lossy(data);
        let mut extracted = String::new();
        let mut page_num = 0u32;

        // Scan for text blocks between BT (begin text) and ET (end text)
        let mut pos = 0;
        let bytes = content.as_bytes();

        while pos + 2 < bytes.len() {
            // Look for "BT" (begin text object)
            if bytes[pos] == b'B' && bytes[pos + 1] == b'T' && is_delimiter(bytes, pos + 2) {
                page_num += 1;
                pos += 2;

                // Scan until "ET" (end text object)
                while pos + 2 < bytes.len() {
                    if bytes[pos] == b'E' && bytes[pos + 1] == b'T' && is_delimiter(bytes, pos + 2)
                    {
                        pos += 2;
                        break;
                    }

                    // Extract text from Tj operator: (text) Tj
                    if bytes[pos] == b'(' {
                        if let Some((text, end)) = extract_parenthesized(&content, pos) {
                            if !text.is_empty() {
                                extracted.push_str(&text);
                            }
                            pos = end;
                            continue;
                        }
                    }

                    pos += 1;
                }

                // Add page separator
                if !extracted.is_empty() && !extracted.ends_with('\n') {
                    extracted.push('\n');
                }
            } else {
                pos += 1;
            }
        }

        if extracted.trim().is_empty() {
            // Fallback: try to extract any readable text sequences
            extracted = extract_readable_sequences(data);
        }

        if extracted.trim().is_empty() {
            return Err(MediaExtractError::ExtractionFailed {
                reason: format!(
                    "Could not extract text from PDF ({} bytes, {} pages detected). \
                     The PDF may contain only images or use unsupported encodings.",
                    data.len(),
                    page_num
                ),
            });
        }

        Ok(clean_extracted_text(&extracted))
    }
}

impl Default for PdfExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaExtractor for PdfExtractor {
    fn supported_types(&self) -> &[MediaType] {
        &[MediaType::Pdf]
    }

    fn extract_text(&self, content: &MediaContent) -> Result<String, MediaExtractError> {
        if content.size() > self.max_pdf_size {
            return Err(MediaExtractError::TooLarge {
                size: content.size(),
                max: self.max_pdf_size,
            });
        }

        let mut text = Self::extract_text_from_pdf(&content.data)?;

        // Truncate if too long
        if text.len() > self.max_text_length {
            text.truncate(self.max_text_length);
            text.push_str("\n\n[... text truncated ...]");
        }

        let filename = content.filename.as_deref().unwrap_or("document.pdf");

        Ok(format!(
            "[PDF: {} — {} chars extracted]\n\n{}",
            filename,
            text.len(),
            text
        ))
    }
}

/// Check if the character at position is a whitespace delimiter.
fn is_delimiter(bytes: &[u8], pos: usize) -> bool {
    if pos >= bytes.len() {
        return true;
    }
    matches!(
        bytes[pos],
        b' ' | b'\n' | b'\r' | b'\t' | b'[' | b']' | b'(' | b')'
    )
}

/// Extract text from a parenthesized PDF string: `(text)`.
///
/// Handles escape sequences like `\\`, `\(`, `\)`, `\n`.
fn extract_parenthesized(content: &str, start: usize) -> Option<(String, usize)> {
    let bytes = content.as_bytes();
    if start >= bytes.len() || bytes[start] != b'(' {
        return None;
    }

    let mut result = String::new();
    let mut depth = 1;
    let mut i = start + 1;

    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => {
                match bytes[i + 1] {
                    b'n' => result.push('\n'),
                    b'r' => result.push('\r'),
                    b't' => result.push('\t'),
                    b'(' => result.push('('),
                    b')' => result.push(')'),
                    b'\\' => result.push('\\'),
                    _ => {
                        // Octal escape or unknown — skip
                    }
                }
                i += 2;
            }
            b'(' => {
                depth += 1;
                result.push('(');
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth > 0 {
                    result.push(')');
                }
                i += 1;
            }
            b if b.is_ascii_graphic() || b == b' ' => {
                result.push(b as char);
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    Some((result, i))
}

/// Extract readable ASCII sequences from binary data as a fallback.
fn extract_readable_sequences(data: &[u8]) -> String {
    let mut result = String::new();
    let mut current_seq = String::new();

    for &byte in data {
        if byte.is_ascii_graphic() || byte == b' ' {
            current_seq.push(byte as char);
        } else {
            if current_seq.len() >= 20 {
                // Only keep sequences of reasonable length
                result.push_str(current_seq.trim());
                result.push(' ');
            }
            current_seq.clear();
        }
    }
    if current_seq.len() >= 20 {
        result.push_str(current_seq.trim());
    }

    result
}

/// Clean up extracted text: normalize whitespace, remove control chars.
fn clean_extracted_text(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    let mut prev_was_space = false;

    for ch in text.chars() {
        if ch == '\n' {
            if !prev_was_space {
                cleaned.push('\n');
            }
            prev_was_space = true;
        } else if ch.is_whitespace() {
            if !prev_was_space {
                cleaned.push(' ');
                prev_was_space = true;
            }
        } else if ch.is_control() {
            // Skip control characters
        } else {
            cleaned.push(ch);
            prev_was_space = false;
        }
    }

    cleaned.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_a_pdf() {
        let mc = MediaContent::new(b"not a pdf".to_vec(), "application/pdf");
        let extractor = PdfExtractor::new();
        let result = extractor.extract_text(&mc);
        assert!(matches!(
            result,
            Err(MediaExtractError::ExtractionFailed { .. })
        ));
    }

    #[test]
    fn test_pdf_too_large() {
        let extractor = PdfExtractor::new().with_max_pdf_size(10);
        let mc = MediaContent::new(vec![0; 100], "application/pdf");
        assert!(matches!(
            extractor.extract_text(&mc),
            Err(MediaExtractError::TooLarge { .. })
        ));
    }

    #[test]
    fn test_simple_pdf_extraction() {
        // Minimal PDF with text object
        let pdf_content = b"%PDF-1.4\nBT (Hello World) Tj ET\n";
        let mc = MediaContent::new(pdf_content.to_vec(), "application/pdf")
            .with_filename("test.pdf".to_string());
        let extractor = PdfExtractor::new();
        let result = extractor.extract_text(&mc).unwrap();
        assert!(result.contains("Hello World"), "Got: {}", result);
        assert!(result.contains("test.pdf"));
    }

    #[test]
    fn test_extract_parenthesized() {
        let content = "(Hello World)";
        let (text, end) = extract_parenthesized(content, 0).unwrap();
        assert_eq!(text, "Hello World");
        assert_eq!(end, content.len());
    }

    #[test]
    fn test_extract_parenthesized_with_escapes() {
        let content = r"(Hello\nWorld)";
        let (text, _) = extract_parenthesized(content, 0).unwrap();
        assert_eq!(text, "Hello\nWorld");
    }

    #[test]
    fn test_extract_parenthesized_nested() {
        let content = "(Hello (nested) World)";
        let (text, _) = extract_parenthesized(content, 0).unwrap();
        assert_eq!(text, "Hello (nested) World");
    }

    #[test]
    fn test_clean_extracted_text() {
        let text = "  Hello   \n\n\n  World  \t\t test  ";
        let cleaned = clean_extracted_text(text);
        assert_eq!(cleaned, "Hello\nWorld test");
    }

    #[test]
    fn test_supported_types() {
        let extractor = PdfExtractor::new();
        assert_eq!(extractor.supported_types(), &[MediaType::Pdf]);
    }
}
