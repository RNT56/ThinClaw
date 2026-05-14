//! Format-specific text extraction functions.
//!
//! Routes document data to the appropriate extractor based on MIME type,
//! with fallback heuristics from filename extension.

use std::io::Read;

use regex::Regex;

/// Extract text from document data based on MIME type.
///
/// Supports PDF, DOCX, PPTX, XLSX, and plain text formats.
/// Returns the extracted text or an error message.
pub fn extract_text(data: &[u8], mime: &str, filename: Option<&str>) -> Result<String, String> {
    match mime {
        "application/pdf" => extract_pdf(data),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            extract_office_xml(data, "word/document.xml")
        }
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            extract_pptx(data)
        }
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => extract_xlsx(data),
        m if is_text_mime(m) => extract_plaintext(data),
        _ => {
            // Try to guess from filename extension
            if let Some(name) = filename {
                let lower = name.to_ascii_lowercase();
                if lower.ends_with(".pdf") {
                    return extract_pdf(data);
                }
                if lower.ends_with(".docx") {
                    return extract_office_xml(data, "word/document.xml");
                }
                if lower.ends_with(".pptx") {
                    return extract_pptx(data);
                }
                if lower.ends_with(".xlsx") {
                    return extract_xlsx(data);
                }
                if is_text_extension(&lower) {
                    return extract_plaintext(data);
                }
            }
            Err(format!("Unsupported document type: {mime}"))
        }
    }
}

/// Extract text from XLSX by resolving shared strings, inline strings,
/// and plain worksheet cell values.
fn extract_xlsx(data: &[u8]) -> Result<String, String> {
    let cursor = std::io::Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Not a valid ZIP/XLSX file: {e}"))?;

    let shared_strings = read_xlsx_shared_strings(&mut archive);

    let mut sheet_names = Vec::new();
    for i in 0..archive.len() {
        if let Ok(file) = archive.by_index(i) {
            let name = file.name().to_string();
            if name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml") {
                sheet_names.push(name);
            }
        }
    }
    sheet_names.sort();

    let cell_re = Regex::new(r#"(?s)<c\b([^>]*)>(.*?)</c>"#)
        .map_err(|e| format!("Invalid cell regex: {e}"))?;
    let value_re =
        Regex::new(r#"(?s)<v[^>]*>(.*?)</v>"#).map_err(|e| format!("Invalid value regex: {e}"))?;
    let inline_re = Regex::new(r#"(?s)<is[^>]*>(.*?)</is>"#)
        .map_err(|e| format!("Invalid inline regex: {e}"))?;

    let mut all_values = Vec::new();
    for sheet_name in sheet_names {
        let mut xml = String::new();
        if let Ok(mut file) = archive.by_name(&sheet_name)
            && file.read_to_string(&mut xml).is_ok()
        {
            for cell in cell_re.captures_iter(&xml) {
                let attrs = cell.get(1).map(|m| m.as_str()).unwrap_or_default();
                let body = cell.get(2).map(|m| m.as_str()).unwrap_or_default();
                let cell_type = attrs
                    .split_whitespace()
                    .find_map(|part| part.strip_prefix(r#"t=""#))
                    .map(|value| value.trim_end_matches('"'));

                let value = match cell_type {
                    Some("s") => value_re
                        .captures(body)
                        .and_then(|caps| caps.get(1))
                        .and_then(|m| m.as_str().trim().parse::<usize>().ok())
                        .and_then(|index| shared_strings.get(index).cloned())
                        .unwrap_or_default(),
                    Some("inlineStr") => inline_re
                        .captures(body)
                        .and_then(|caps| caps.get(1))
                        .map(|m| strip_xml_tags(m.as_str()))
                        .unwrap_or_default(),
                    _ => value_re
                        .captures(body)
                        .and_then(|caps| caps.get(1))
                        .map(|m| strip_xml_tags(m.as_str()))
                        .unwrap_or_default(),
                };

                if !value.trim().is_empty() {
                    all_values.push(value);
                }
            }
        }
    }

    if all_values.is_empty() {
        return Err("No text content found in XLSX worksheets".to_string());
    }

    Ok(all_values.join("\n"))
}

fn read_xlsx_shared_strings<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Vec<String> {
    let mut xml = String::new();
    let Ok(mut file) = archive.by_name("xl/sharedStrings.xml") else {
        return Vec::new();
    };
    if file.read_to_string(&mut xml).is_err() {
        return Vec::new();
    }

    let Ok(shared_re) = Regex::new(r#"(?s)<si[^>]*>(.*?)</si>"#) else {
        return Vec::new();
    };

    shared_re
        .captures_iter(&xml)
        .filter_map(|caps| caps.get(1))
        .map(|m| strip_xml_tags(m.as_str()))
        .filter(|value| !value.trim().is_empty())
        .collect()
}

/// Extract text from a PDF using `pdf-extract` (full parser with CMap support).
pub fn extract_pdf(data: &[u8]) -> Result<String, String> {
    pdf_extract::extract_text_from_mem(data).map_err(|e| format!("PDF extraction failed: {e}"))
}

/// Extract text from an Office XML document (DOCX, XLSX) by reading
/// the specified XML file inside the ZIP archive and stripping XML tags.
fn extract_office_xml(data: &[u8], xml_path: &str) -> Result<String, String> {
    let cursor = std::io::Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Not a valid ZIP/Office file: {e}"))?;

    let mut xml_content = String::new();
    match archive.by_name(xml_path) {
        Ok(mut file) => {
            file.read_to_string(&mut xml_content)
                .map_err(|e| format!("Failed to read {xml_path}: {e}"))?;
        }
        Err(_) => {
            return Err(format!(
                "File {xml_path} not found in archive (not a valid Office document)"
            ));
        }
    }

    Ok(strip_xml_tags(&xml_content))
}

/// Extract text from PPTX by reading all slide XML files.
fn extract_pptx(data: &[u8]) -> Result<String, String> {
    let cursor = std::io::Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Not a valid ZIP/PPTX file: {e}"))?;

    let mut all_text = String::new();
    let mut slide_names: Vec<String> = Vec::new();

    for i in 0..archive.len() {
        if let Ok(file) = archive.by_index(i) {
            let name = file.name().to_string();
            if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
                slide_names.push(name);
            }
        }
    }

    slide_names.sort();

    for name in slide_names {
        if let Ok(mut file) = archive.by_name(&name) {
            let mut content = String::new();
            if file.read_to_string(&mut content).is_ok() {
                let text = strip_xml_tags(&content);
                if !text.is_empty() {
                    if !all_text.is_empty() {
                        all_text.push('\n');
                    }
                    all_text.push_str(&text);
                }
            }
        }
    }

    if all_text.is_empty() {
        return Err("No text content found in PPTX slides".to_string());
    }

    Ok(all_text)
}

/// Extract plain text from UTF-8 data.
fn extract_plaintext(data: &[u8]) -> Result<String, String> {
    String::from_utf8(data.to_vec()).map_err(|e| format!("Not valid UTF-8 text: {e}"))
}

/// Strip XML tags from a string, keeping only text content.
fn strip_xml_tags(xml: &str) -> String {
    let mut result = String::with_capacity(xml.len() / 2);
    let mut in_tag = false;
    let mut prev_was_space = false;

    for ch in xml.chars() {
        match ch {
            '<' => {
                in_tag = true;
            }
            '>' => {
                in_tag = false;
                // Add space after closing tags to separate words
                if !prev_was_space && !result.is_empty() {
                    result.push(' ');
                    prev_was_space = true;
                }
            }
            _ if !in_tag => {
                if ch.is_whitespace() {
                    if !prev_was_space {
                        result.push(' ');
                        prev_was_space = true;
                    }
                } else {
                    result.push(ch);
                    prev_was_space = false;
                }
            }
            _ => {}
        }
    }

    result.trim().to_string()
}

/// Check if a MIME type represents plain text.
fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime == "application/json"
        || mime == "application/xml"
        || mime == "application/javascript"
        || mime == "application/x-yaml"
        || mime == "application/toml"
}

/// Check if a filename extension is a plain text format.
fn is_text_extension(filename: &str) -> bool {
    let text_exts = [
        ".txt",
        ".csv",
        ".json",
        ".xml",
        ".yaml",
        ".yml",
        ".toml",
        ".md",
        ".markdown",
        ".rs",
        ".py",
        ".js",
        ".ts",
        ".go",
        ".java",
        ".c",
        ".cpp",
        ".h",
        ".rb",
        ".sh",
        ".bash",
        ".zsh",
        ".sql",
        ".html",
        ".css",
        ".ini",
        ".cfg",
        ".conf",
        ".log",
    ];
    text_exts.iter().any(|ext| filename.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn test_extract_plaintext() {
        let data = b"Hello, world!";
        let result = extract_text(data, "text/plain", None).unwrap();
        assert_eq!(result, "Hello, world!");
    }

    #[test]
    fn test_extract_json() {
        let data = br#"{"key": "value"}"#;
        let result = extract_text(data, "application/json", None).unwrap();
        assert!(result.contains("key"));
    }

    #[test]
    fn test_extract_csv() {
        let data = b"name,age\nAlice,30";
        let result = extract_text(data, "text/csv", None).unwrap();
        assert_eq!(result, "name,age\nAlice,30");
    }

    #[test]
    fn test_strip_xml_tags() {
        let xml = "<root><p>Hello</p><p>World</p></root>";
        let result = strip_xml_tags(xml);
        assert!(result.contains("Hello"));
        assert!(result.contains("World"));
    }

    #[test]
    fn test_extract_xlsx_reads_shared_and_inline_strings() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("xl/sharedStrings.xml", options).unwrap();
            zip.write_all(
                br#"<sst><si><t>Shared Value</t></si><si><r><t>Rich Text</t></r></si></sst>"#,
            )
            .unwrap();
            zip.start_file("xl/worksheets/sheet1.xml", options).unwrap();
            zip.write_all(
                br#"<worksheet><sheetData><row>
                    <c r="A1" t="s"><v>0</v></c>
                    <c r="A2" t="inlineStr"><is><t>Inline Value</t></is></c>
                    <c r="A3"><v>42</v></c>
                </row></sheetData></worksheet>"#,
            )
            .unwrap();
            zip.finish().unwrap();
        }

        let text = extract_xlsx(cursor.get_ref()).unwrap();
        assert!(text.contains("Shared Value"));
        assert!(text.contains("Inline Value"));
        assert!(text.contains("42"));
    }

    #[test]
    fn test_unsupported_type() {
        let result = extract_text(b"data", "application/octet-stream", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported"));
    }

    #[test]
    fn test_extension_fallback_txt() {
        let data = b"Content from file";
        let result = extract_text(data, "application/octet-stream", Some("notes.txt")).unwrap();
        assert_eq!(result, "Content from file");
    }

    #[test]
    fn test_is_text_mime() {
        assert!(is_text_mime("text/plain"));
        assert!(is_text_mime("text/csv"));
        assert!(is_text_mime("application/json"));
        assert!(!is_text_mime("application/pdf"));
        assert!(!is_text_mime("image/png"));
    }

    #[test]
    fn test_is_text_extension() {
        assert!(is_text_extension("readme.md"));
        assert!(is_text_extension("main.rs"));
        assert!(is_text_extension("data.csv"));
        assert!(!is_text_extension("photo.jpg"));
        assert!(!is_text_extension("doc.pdf"));
    }
}
