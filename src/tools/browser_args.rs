//! Browser extraArgs configuration.
//!
//! Allows users to specify custom Chrome launch arguments
//! for the headless browser tool.

use serde::{Deserialize, Serialize};

/// Browser launch configuration with custom arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserArgsConfig {
    /// Extra arguments to pass to Chrome.
    pub extra_args: Vec<String>,
    /// Whether to use headless mode.
    pub headless: bool,
    /// Whether to disable extensions.
    pub disable_extensions: bool,
    /// Custom user data directory.
    pub user_data_dir: Option<String>,
    /// Proxy server.
    pub proxy_server: Option<String>,
    /// Window size.
    pub window_size: Option<(u32, u32)>,
    /// Disable sandbox (required in some containers).
    pub no_sandbox: bool,
}

impl Default for BrowserArgsConfig {
    fn default() -> Self {
        Self {
            extra_args: Vec::new(),
            headless: true,
            disable_extensions: true,
            user_data_dir: None,
            proxy_server: None,
            window_size: Some((1920, 1080)),
            no_sandbox: false,
        }
    }
}

impl BrowserArgsConfig {
    /// Create from environment.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(args) = std::env::var("BROWSER_EXTRA_ARGS") {
            config.extra_args = args.split_whitespace().map(|s| s.to_string()).collect();
        }
        if let Ok(size) = std::env::var("BROWSER_WINDOW_SIZE") {
            if let Some((w, h)) = parse_size(&size) {
                config.window_size = Some((w, h));
            }
        }
        if let Ok(proxy) = std::env::var("BROWSER_PROXY") {
            config.proxy_server = Some(proxy);
        }
        if std::env::var("BROWSER_NO_SANDBOX").is_ok() {
            config.no_sandbox = true;
        }

        config
    }

    /// Build the full Chrome argument list.
    pub fn build_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        if self.headless {
            args.push("--headless=new".to_string());
        }

        if self.disable_extensions {
            args.push("--disable-extensions".to_string());
        }

        if self.no_sandbox {
            args.push("--no-sandbox".to_string());
            args.push("--disable-setuid-sandbox".to_string());
        }

        if let Some((w, h)) = self.window_size {
            args.push(format!("--window-size={},{}", w, h));
        }

        if let Some(dir) = &self.user_data_dir {
            args.push(format!("--user-data-dir={}", dir));
        }

        if let Some(proxy) = &self.proxy_server {
            args.push(format!("--proxy-server={}", proxy));
        }

        args.extend(self.extra_args.clone());

        args
    }

    /// Merge with additional args (e.g., from a tool call).
    pub fn merge(&self, additional: &[String]) -> Vec<String> {
        let mut args = self.build_args();
        for arg in additional {
            if !args.contains(arg) {
                args.push(arg.clone());
            }
        }
        args
    }

    /// Add security hardening flags.
    pub fn with_hardening(mut self) -> Self {
        self.extra_args.extend([
            "--disable-web-security=false".to_string(),
            "--disable-background-networking".to_string(),
            "--disable-default-apps".to_string(),
            "--disable-sync".to_string(),
            "--disable-translate".to_string(),
            "--metrics-recording-only".to_string(),
            "--no-first-run".to_string(),
        ]);
        self
    }
}

/// Parse a "WxH" size string.
fn parse_size(s: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() == 2 {
        let w = parts[0].parse().ok()?;
        let h = parts[1].parse().ok()?;
        Some((w, h))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = BrowserArgsConfig::default();
        assert!(config.headless);
        assert_eq!(config.window_size, Some((1920, 1080)));
    }

    #[test]
    fn test_build_args_default() {
        let config = BrowserArgsConfig::default();
        let args = config.build_args();
        assert!(args.contains(&"--headless=new".to_string()));
        assert!(args.contains(&"--disable-extensions".to_string()));
        assert!(args.iter().any(|a| a.starts_with("--window-size")));
    }

    #[test]
    fn test_build_args_no_sandbox() {
        let config = BrowserArgsConfig {
            no_sandbox: true,
            ..Default::default()
        };
        let args = config.build_args();
        assert!(args.contains(&"--no-sandbox".to_string()));
    }

    #[test]
    fn test_build_args_proxy() {
        let config = BrowserArgsConfig {
            proxy_server: Some("http://proxy:8080".into()),
            ..Default::default()
        };
        let args = config.build_args();
        assert!(args.iter().any(|a| a.contains("proxy")));
    }

    #[test]
    fn test_extra_args() {
        let config = BrowserArgsConfig {
            extra_args: vec!["--custom-flag".into()],
            ..Default::default()
        };
        let args = config.build_args();
        assert!(args.contains(&"--custom-flag".to_string()));
    }

    #[test]
    fn test_merge_deduplication() {
        let config = BrowserArgsConfig::default();
        let merged = config.merge(&["--headless=new".into(), "--new-flag".into()]);
        let headless_count = merged.iter().filter(|a| *a == "--headless=new").count();
        assert_eq!(headless_count, 1);
        assert!(merged.contains(&"--new-flag".to_string()));
    }

    #[test]
    fn test_with_hardening() {
        let config = BrowserArgsConfig::default().with_hardening();
        let args = config.build_args();
        assert!(args.iter().any(|a| a.contains("no-first-run")));
    }

    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("1920x1080"), Some((1920, 1080)));
        assert_eq!(parse_size("invalid"), None);
    }
}
