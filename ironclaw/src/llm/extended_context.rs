//! Anthropic extended context support (1M context beta).
//!
//! When enabled, adds the `anthropic-beta: max-model-context-window-2025-01-29`
//! header to Anthropic API requests, unlocking the full 1M token context window.
//!
//! Configuration:
//! - `ANTHROPIC_EXTENDED_CONTEXT` — "true" to enable 1M context (default: false)
//! - `ANTHROPIC_BETA_HEADER` — custom beta header value (overrides default)

/// Configuration for Anthropic extended context.
#[derive(Debug, Clone)]
pub struct ExtendedContextConfig {
    /// Whether extended context is enabled.
    pub enabled: bool,
    /// Beta header value to send.
    pub beta_header: String,
}

impl Default for ExtendedContextConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            beta_header: "max-model-context-window-2025-01-29".to_string(),
        }
    }
}

impl ExtendedContextConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("ANTHROPIC_EXTENDED_CONTEXT") {
            config.enabled = val == "1" || val.eq_ignore_ascii_case("true");
        }

        if let Ok(header) = std::env::var("ANTHROPIC_BETA_HEADER") {
            config.beta_header = header;
            config.enabled = true; // Providing a custom header implies enabling
        }

        config
    }

    /// Get headers to add to Anthropic API requests.
    pub fn extra_headers(&self) -> Vec<(String, String)> {
        if !self.enabled {
            return Vec::new();
        }

        vec![("anthropic-beta".to_string(), self.beta_header.clone())]
    }

    /// Apply extended context headers to a reqwest::RequestBuilder.
    pub fn apply_to_request(
        &self,
        mut builder: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        for (key, value) in self.extra_headers() {
            builder = builder.header(&key, &value);
        }
        builder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_by_default() {
        let config = ExtendedContextConfig::default();
        assert!(!config.enabled);
        assert!(config.extra_headers().is_empty());
    }

    #[test]
    fn test_enabled_headers() {
        let config = ExtendedContextConfig {
            enabled: true,
            ..Default::default()
        };

        let headers = config.extra_headers();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "anthropic-beta");
        assert!(headers[0].1.contains("max-model-context-window"));
    }

    #[test]
    fn test_custom_header() {
        let config = ExtendedContextConfig {
            enabled: true,
            beta_header: "custom-beta-feature".to_string(),
        };

        let headers = config.extra_headers();
        assert_eq!(headers[0].1, "custom-beta-feature");
    }
}
