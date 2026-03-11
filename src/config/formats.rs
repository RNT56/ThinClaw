//! Alternative config format support: JSON5 and YAML.
//!
//! IronClaw's default config is JSON, but users may prefer:
//! - JSON5: supports comments, trailing commas, unquoted keys
//! - YAML: human-friendly, widely used for config files
//!
//! This module provides format detection and parsing.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Supported config formats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConfigFormat {
    Json,
    Json5,
    Yaml,
}

impl ConfigFormat {
    /// Detect format from file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "json5" | "jsonc" => Self::Json5,
            "yaml" | "yml" => Self::Yaml,
            _ => Self::Json,
        }
    }

    /// Detect format from file path.
    pub fn from_path(path: &str) -> Self {
        if let Some(ext) = path.rsplit('.').next() {
            Self::from_extension(ext)
        } else {
            Self::Json
        }
    }

    /// Detect format from content heuristics.
    pub fn detect(content: &str) -> Self {
        let trimmed = content.trim();

        // JSON always starts with { or [
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            // Check for JSON5 features
            if has_json5_features(trimmed) {
                return Self::Json5;
            }
            return Self::Json;
        }

        // YAML indicators
        if trimmed.starts_with("---") || trimmed.starts_with("# ") || trimmed.contains(": ") {
            return Self::Yaml;
        }

        Self::Json
    }

    /// File extension for this format.
    pub fn extension(&self) -> &str {
        match self {
            Self::Json => "json",
            Self::Json5 => "json5",
            Self::Yaml => "yaml",
        }
    }
}

/// Check if content has JSON5-specific features.
fn has_json5_features(content: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();
        // Single-line comments
        if trimmed.starts_with("//") {
            return true;
        }
        // Trailing commas before closing brackets
        if trimmed.ends_with(",}") || trimmed.ends_with(",]") {
            return true;
        }
    }
    // Block comments
    if content.contains("/*") {
        return true;
    }
    false
}

/// JSON5 preprocessing: strip comments and trailing commas.
pub fn preprocess_json5(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escape_next = false;

    while let Some(ch) = chars.next() {
        if escape_next {
            output.push(ch);
            escape_next = false;
            continue;
        }

        if ch == '\\' && in_string {
            output.push(ch);
            escape_next = true;
            continue;
        }

        if ch == '"' {
            in_string = !in_string;
            output.push(ch);
            continue;
        }

        if in_string {
            output.push(ch);
            continue;
        }

        // Single-line comment
        if ch == '/' && chars.peek() == Some(&'/') {
            // Skip until end of line
            for c in chars.by_ref() {
                if c == '\n' {
                    output.push('\n');
                    break;
                }
            }
            continue;
        }

        // Block comment
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next(); // consume '*'
            let mut prev = ' ';
            for c in chars.by_ref() {
                if prev == '*' && c == '/' {
                    break;
                }
                if c == '\n' {
                    output.push('\n');
                }
                prev = c;
            }
            continue;
        }

        output.push(ch);
    }

    // Remove trailing commas
    remove_trailing_commas(&output)
}

/// Remove trailing commas before } or ].
fn remove_trailing_commas(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == ',' {
            // Look ahead for } or ] (skipping whitespace)
            let mut j = i + 1;
            while j < len && chars[j].is_whitespace() {
                j += 1;
            }
            if j < len && (chars[j] == '}' || chars[j] == ']') {
                // Skip the trailing comma
                i += 1;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }

    result
}

/// YAML preprocessing: minimal YAML-to-JSON converter for simple configs.
pub fn preprocess_yaml(input: &str) -> Result<serde_json::Value, ConfigFormatError> {
    // Simple key: value line parsing for flat configs
    let mut map = serde_json::Map::new();

    for line in input.lines() {
        let trimmed = line.trim();

        // Skip comments and empty lines
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" {
            continue;
        }

        if let Some(colon_pos) = trimmed.find(": ") {
            let key = trimmed[..colon_pos].trim().to_string();
            let value_str = trimmed[colon_pos + 2..].trim();

            let value = parse_yaml_value(value_str);
            map.insert(key, value);
        } else if let Some(key) = trimmed.strip_suffix(':') {
            // Key with no value (nested object placeholder)
            map.insert(
                key.trim().to_string(),
                serde_json::Value::Object(serde_json::Map::new()),
            );
        }
    }

    Ok(serde_json::Value::Object(map))
}

/// Parse a simple YAML value.
fn parse_yaml_value(s: &str) -> serde_json::Value {
    // Boolean
    match s.to_lowercase().as_str() {
        "true" | "yes" | "on" => return serde_json::Value::Bool(true),
        "false" | "no" | "off" => return serde_json::Value::Bool(false),
        "null" | "~" => return serde_json::Value::Null,
        _ => {}
    }

    // Number
    if let Ok(n) = s.parse::<i64>() {
        return serde_json::Value::Number(n.into());
    }
    if let Ok(n) = s.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(n) {
            return serde_json::Value::Number(num);
        }
    }

    // Quoted string
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        return serde_json::Value::String(s[1..s.len() - 1].to_string());
    }

    // Array shorthand: [a, b, c]
    if s.starts_with('[') && s.ends_with(']') {
        let inner = s[1..s.len() - 1].trim();
        let items: Vec<serde_json::Value> = inner
            .split(',')
            .map(|item| parse_yaml_value(item.trim()))
            .collect();
        return serde_json::Value::Array(items);
    }

    // Plain string
    serde_json::Value::String(s.to_string())
}

/// Parse a config string in any supported format.
pub fn parse_config<T: DeserializeOwned>(
    content: &str,
    format: ConfigFormat,
) -> Result<T, ConfigFormatError> {
    match format {
        ConfigFormat::Json => serde_json::from_str(content)
            .map_err(|e| ConfigFormatError::ParseError(format!("JSON: {}", e))),
        ConfigFormat::Json5 => {
            let preprocessed = preprocess_json5(content);
            serde_json::from_str(&preprocessed)
                .map_err(|e| ConfigFormatError::ParseError(format!("JSON5: {}", e)))
        }
        ConfigFormat::Yaml => {
            let value = preprocess_yaml(content)?;
            serde_json::from_value(value)
                .map_err(|e| ConfigFormatError::ParseError(format!("YAML: {}", e)))
        }
    }
}

/// Serialize a config value to the specified format.
pub fn serialize_config<T: Serialize>(
    value: &T,
    format: ConfigFormat,
) -> Result<String, ConfigFormatError> {
    match format {
        ConfigFormat::Json | ConfigFormat::Json5 => serde_json::to_string_pretty(value)
            .map_err(|e| ConfigFormatError::SerializeError(e.to_string())),
        ConfigFormat::Yaml => {
            // Simple JSON-to-YAML serialization
            let json_value = serde_json::to_value(value)
                .map_err(|e| ConfigFormatError::SerializeError(e.to_string()))?;
            Ok(json_to_yaml(&json_value, 0))
        }
    }
}

/// Simple JSON value to YAML string conversion.
fn json_to_yaml(value: &serde_json::Value, indent: usize) -> String {
    let prefix = "  ".repeat(indent);
    match value {
        serde_json::Value::Object(map) => {
            let mut lines = Vec::new();
            for (key, val) in map {
                match val {
                    serde_json::Value::Object(_) => {
                        lines.push(format!("{}{}:", prefix, key));
                        lines.push(json_to_yaml(val, indent + 1));
                    }
                    _ => {
                        lines.push(format!("{}{}: {}", prefix, key, yaml_value(val)));
                    }
                }
            }
            lines.join("\n")
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr
                .iter()
                .map(|v| format!("{}- {}", prefix, yaml_value(v)))
                .collect();
            items.join("\n")
        }
        _ => format!("{}{}", prefix, yaml_value(value)),
    }
}

/// Format a JSON value as YAML scalar.
fn yaml_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            if s.contains(':') || s.contains('#') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.clone()
            }
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(yaml_value).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(_) => "{...}".to_string(),
    }
}

/// Config format errors.
#[derive(Debug, Clone)]
pub enum ConfigFormatError {
    ParseError(String),
    SerializeError(String),
    UnsupportedFormat(String),
}

impl std::fmt::Display for ConfigFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParseError(e) => write!(f, "Parse error: {}", e),
            Self::SerializeError(e) => write!(f, "Serialize error: {}", e),
            Self::UnsupportedFormat(e) => write!(f, "Unsupported format: {}", e),
        }
    }
}

impl std::error::Error for ConfigFormatError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_from_extension() {
        assert_eq!(ConfigFormat::from_extension("json"), ConfigFormat::Json);
        assert_eq!(ConfigFormat::from_extension("json5"), ConfigFormat::Json5);
        assert_eq!(ConfigFormat::from_extension("yaml"), ConfigFormat::Yaml);
        assert_eq!(ConfigFormat::from_extension("yml"), ConfigFormat::Yaml);
    }

    #[test]
    fn test_format_from_path() {
        assert_eq!(ConfigFormat::from_path("config.json5"), ConfigFormat::Json5);
        assert_eq!(ConfigFormat::from_path("settings.yaml"), ConfigFormat::Yaml);
    }

    #[test]
    fn test_detect_json() {
        assert_eq!(
            ConfigFormat::detect("{\"key\": \"value\"}"),
            ConfigFormat::Json
        );
    }

    #[test]
    fn test_detect_json5() {
        let content = r#"{
            // This is a comment
            "key": "value",
        }"#;
        assert_eq!(ConfigFormat::detect(content), ConfigFormat::Json5);
    }

    #[test]
    fn test_detect_yaml() {
        assert_eq!(
            ConfigFormat::detect("key: value\nother: 42"),
            ConfigFormat::Yaml
        );
    }

    #[test]
    fn test_preprocess_json5_comments() {
        let input = r#"{
  // comment
  "key": "value"
}"#;
        let result = preprocess_json5(input);
        assert!(!result.contains("//"));
        assert!(result.contains("\"key\""));
    }

    #[test]
    fn test_preprocess_json5_block_comments() {
        let input = r#"{ /* block comment */ "key": "value" }"#;
        let result = preprocess_json5(input);
        assert!(!result.contains("/*"));
        assert!(result.contains("\"key\""));
    }

    #[test]
    fn test_preprocess_json5_trailing_commas() {
        let input = r#"{"a": 1, "b": 2, }"#;
        let result = preprocess_json5(input);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["a"], 1);
        assert_eq!(parsed["b"], 2);
    }

    #[test]
    fn test_parse_yaml_values() {
        assert_eq!(parse_yaml_value("true"), serde_json::Value::Bool(true));
        assert_eq!(parse_yaml_value("42"), serde_json::json!(42));
        assert_eq!(parse_yaml_value("null"), serde_json::Value::Null);
        assert_eq!(parse_yaml_value("hello"), serde_json::json!("hello"));
    }

    #[test]
    fn test_preprocess_yaml() {
        let input = "# Config\nname: IronClaw\nversion: 1\nenabled: true";
        let value = preprocess_yaml(input).unwrap();
        assert_eq!(value["name"], "IronClaw");
        assert_eq!(value["version"], 1);
        assert_eq!(value["enabled"], true);
    }

    #[test]
    fn test_parse_config_json() {
        let input = r#"{"name": "test", "count": 42}"#;
        let value: serde_json::Value = parse_config(input, ConfigFormat::Json).unwrap();
        assert_eq!(value["name"], "test");
    }

    #[test]
    fn test_parse_config_json5() {
        let input = r#"{
            // JSON5 with comments
            "name": "test",
        }"#;
        let value: serde_json::Value = parse_config(input, ConfigFormat::Json5).unwrap();
        assert_eq!(value["name"], "test");
    }

    #[test]
    fn test_serialize_yaml() {
        let value = serde_json::json!({"name": "IronClaw", "version": 1});
        let yaml = serialize_config(&value, ConfigFormat::Yaml).unwrap();
        assert!(yaml.contains("name: IronClaw"));
    }

    #[test]
    fn test_error_display() {
        let err = ConfigFormatError::ParseError("bad input".to_string());
        assert!(format!("{}", err).contains("bad input"));
    }
}
