//! Session export in multiple formats.
//!
//! Supports Markdown, JSON, CSV, HTML, and plain text export
//! of chat session records.

/// Export format.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Markdown,
    Json,
    Csv,
    Html,
    Plain,
}

impl ExportFormat {
    /// Parse from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "json" => Some(Self::Json),
            "csv" => Some(Self::Csv),
            "html" => Some(Self::Html),
            "txt" | "plain" | "text" => Some(Self::Plain),
            _ => None,
        }
    }

    /// File extension.
    pub fn extension(&self) -> &str {
        match self {
            Self::Markdown => "md",
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Html => "html",
            Self::Plain => "txt",
        }
    }

    /// MIME type.
    pub fn mime_type(&self) -> &str {
        match self {
            Self::Markdown => "text/markdown",
            Self::Json => "application/json",
            Self::Csv => "text/csv",
            Self::Html => "text/html",
            Self::Plain => "text/plain",
        }
    }
}

/// A single export record.
pub struct ExportRecord {
    pub role: String,
    pub content: String,
    pub timestamp: Option<String>,
    pub model: Option<String>,
    pub tokens: Option<u32>,
}

/// Session exporter.
pub struct SessionExporter {
    pub format: ExportFormat,
    pub include_metadata: bool,
    pub include_timestamps: bool,
    pub max_records: Option<usize>,
}

impl SessionExporter {
    pub fn new(format: ExportFormat) -> Self {
        Self {
            format,
            include_metadata: true,
            include_timestamps: true,
            max_records: None,
        }
    }

    /// Export records to the configured format.
    pub fn export(&self, records: &[ExportRecord]) -> String {
        let records = if let Some(max) = self.max_records {
            &records[..records.len().min(max)]
        } else {
            records
        };

        match self.format {
            ExportFormat::Markdown => self.export_markdown(records),
            ExportFormat::Json => self.export_json(records),
            ExportFormat::Csv => self.export_csv(records),
            ExportFormat::Html => self.export_html(records),
            ExportFormat::Plain => self.export_plain(records),
        }
    }

    fn export_markdown(&self, records: &[ExportRecord]) -> String {
        let mut out = String::from("# Session Export\n\n");
        for r in records {
            let role = capitalize(&r.role);
            out.push_str(&format!("## {}\n", role));
            if self.include_timestamps {
                if let Some(ts) = &r.timestamp {
                    out.push_str(&format!("*{}*\n\n", ts));
                }
            }
            out.push_str(&r.content);
            out.push_str("\n\n");
        }
        out
    }

    fn export_json(&self, records: &[ExportRecord]) -> String {
        let mut items = Vec::new();
        for r in records {
            let ts = r.timestamp.as_deref().unwrap_or("");
            let model = r.model.as_deref().unwrap_or("");
            let tokens = r.tokens.unwrap_or(0);
            items.push(format!(
                r#"{{"role":"{}","content":"{}","timestamp":"{}","model":"{}","tokens":{}}}"#,
                escape_json(&r.role),
                escape_json(&r.content),
                escape_json(ts),
                escape_json(model),
                tokens,
            ));
        }
        format!(
            r#"{{"records":[{}],"count":{}}}"#,
            items.join(","),
            records.len()
        )
    }

    fn export_csv(&self, records: &[ExportRecord]) -> String {
        let mut out = String::from("role,timestamp,content,model,tokens\n");
        for r in records {
            out.push_str(&format!(
                "{},{},{},{},{}\n",
                csv_quote(&r.role),
                csv_quote(r.timestamp.as_deref().unwrap_or("")),
                csv_quote(&r.content),
                csv_quote(r.model.as_deref().unwrap_or("")),
                r.tokens.unwrap_or(0),
            ));
        }
        out
    }

    fn export_html(&self, records: &[ExportRecord]) -> String {
        let mut out = String::from(
            "<!DOCTYPE html>\n<html>\n<head><title>Session Export</title>\n\
             <style>body{font-family:sans-serif;max-width:800px;margin:0 auto;padding:20px}\
             .message{margin:10px 0;padding:10px;border-radius:8px}\
             .role-user{background:#e8f4fd}.role-assistant{background:#f0f0f0}</style>\n\
             </head>\n<body>\n<h1>Session Export</h1>\n",
        );
        for r in records {
            let class = format!("role-{}", r.role);
            out.push_str(&format!(
                "<div class=\"message {}\">\n<strong>{}</strong>\n",
                class,
                capitalize(&r.role)
            ));
            if self.include_timestamps {
                if let Some(ts) = &r.timestamp {
                    out.push_str(&format!("<small>{}</small>\n", ts));
                }
            }
            out.push_str(&format!("<p>{}</p>\n</div>\n", html_escape(&r.content)));
        }
        out.push_str("</body>\n</html>\n");
        out
    }

    fn export_plain(&self, records: &[ExportRecord]) -> String {
        let mut out = String::new();
        for r in records {
            out.push_str(&format!("[{}]: {}\n", r.role, r.content));
        }
        out
    }
}

/// Response shape for `openclaw_export_session` Tauri command.
///
/// Contains the exported content, format metadata, and MIME type
/// for use by Scrappy's export format picker.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionExportResponse {
    pub content: String,
    pub format: ExportFormat,
    pub mime_type: String,
    pub extension: String,
    pub record_count: usize,
}

impl SessionExportResponse {
    /// Create a response from an export operation.
    pub fn from_export(exporter: &SessionExporter, records: &[ExportRecord]) -> Self {
        Self {
            content: exporter.export(records),
            format: exporter.format.clone(),
            mime_type: exporter.format.mime_type().to_string(),
            extension: exporter.format.extension().to_string(),
            record_count: records.len(),
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn csv_quote(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_records() -> Vec<ExportRecord> {
        vec![
            ExportRecord {
                role: "user".into(),
                content: "Hello, world!".into(),
                timestamp: Some("2026-01-01T00:00:00Z".into()),
                model: None,
                tokens: Some(5),
            },
            ExportRecord {
                role: "assistant".into(),
                content: "Hi there!".into(),
                timestamp: Some("2026-01-01T00:00:01Z".into()),
                model: Some("gpt-5".into()),
                tokens: Some(10),
            },
        ]
    }

    #[test]
    fn test_markdown_has_roles() {
        let exporter = SessionExporter::new(ExportFormat::Markdown);
        let output = exporter.export(&sample_records());
        assert!(output.contains("## User"));
        assert!(output.contains("## Assistant"));
    }

    #[test]
    fn test_markdown_timestamps() {
        let exporter = SessionExporter::new(ExportFormat::Markdown);
        let output = exporter.export(&sample_records());
        assert!(output.contains("2026-01-01T00:00:00Z"));
    }

    #[test]
    fn test_json_valid() {
        let exporter = SessionExporter::new(ExportFormat::Json);
        let output = exporter.export(&sample_records());
        assert!(output.starts_with('{'));
        assert!(output.contains("\"records\""));
        assert!(output.contains("\"count\":2"));
    }

    #[test]
    fn test_csv_headers() {
        let exporter = SessionExporter::new(ExportFormat::Csv);
        let output = exporter.export(&sample_records());
        assert!(output.starts_with("role,timestamp,content,model,tokens"));
    }

    #[test]
    fn test_csv_quoting() {
        let exporter = SessionExporter::new(ExportFormat::Csv);
        let records = vec![ExportRecord {
            role: "user".into(),
            content: "Hello, world".into(),
            timestamp: None,
            model: None,
            tokens: None,
        }];
        let output = exporter.export(&records);
        assert!(output.contains("\"Hello, world\"")); // comma in content = quoted
    }

    #[test]
    fn test_html_has_structure() {
        let exporter = SessionExporter::new(ExportFormat::Html);
        let output = exporter.export(&sample_records());
        assert!(output.contains("<html>"));
        assert!(output.contains("<head>"));
        assert!(output.contains("<body>"));
        assert!(output.contains("</html>"));
    }

    #[test]
    fn test_plain_no_markup() {
        let exporter = SessionExporter::new(ExportFormat::Plain);
        let output = exporter.export(&sample_records());
        assert!(output.contains("[user]:"));
        assert!(!output.contains('<'));
        assert!(!output.contains('#'));
    }

    #[test]
    fn test_format_from_str() {
        assert_eq!(ExportFormat::from_str("md"), Some(ExportFormat::Markdown));
        assert_eq!(ExportFormat::from_str("json"), Some(ExportFormat::Json));
        assert_eq!(ExportFormat::from_str("csv"), Some(ExportFormat::Csv));
        assert_eq!(ExportFormat::from_str("html"), Some(ExportFormat::Html));
        assert_eq!(ExportFormat::from_str("txt"), Some(ExportFormat::Plain));
        assert_eq!(ExportFormat::from_str("nope"), None);
    }

    #[test]
    fn test_mime_types() {
        assert_eq!(ExportFormat::Markdown.mime_type(), "text/markdown");
        assert_eq!(ExportFormat::Json.mime_type(), "application/json");
        assert_eq!(ExportFormat::Csv.mime_type(), "text/csv");
        assert_eq!(ExportFormat::Html.mime_type(), "text/html");
        assert_eq!(ExportFormat::Plain.mime_type(), "text/plain");
    }

    #[test]
    fn test_max_records() {
        let mut exporter = SessionExporter::new(ExportFormat::Plain);
        exporter.max_records = Some(1);
        let output = exporter.export(&sample_records());
        assert!(output.contains("Hello, world!"));
        assert!(!output.contains("Hi there!"));
    }

    #[test]
    fn test_export_format_serializable() {
        let json = serde_json::to_string(&ExportFormat::Markdown).unwrap();
        assert_eq!(json, "\"markdown\"");
        let deser: ExportFormat = serde_json::from_str("\"csv\"").unwrap();
        assert_eq!(deser, ExportFormat::Csv);
    }

    #[test]
    fn test_session_export_response() {
        let exporter = SessionExporter::new(ExportFormat::Json);
        let response = SessionExportResponse::from_export(&exporter, &sample_records());
        assert_eq!(response.format, ExportFormat::Json);
        assert_eq!(response.mime_type, "application/json");
        assert_eq!(response.extension, "json");
        assert_eq!(response.record_count, 2);
        assert!(response.content.contains("\"records\""));
        // Verify serializable
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"format\":\"json\""));
    }
}
