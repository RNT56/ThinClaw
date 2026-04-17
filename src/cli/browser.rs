//! Browser automation CLI commands.
//!
//! Provides headless/visible browser automation for the agent, including:
//! - Opening URLs and taking screenshots
//! - Extracting page content (text, links, metadata)
//! - Running JavaScript snippets on pages
//! - Managing browser sessions
//!
//! Uses a lightweight approach — shells out to `chromium`/`chrome` with
//! `--headless` and `--dump-dom` flags for basic operations.

use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Subcommand;
use serde::{Deserialize, Serialize};

#[derive(Subcommand, Debug, Clone)]
pub enum BrowserCommand {
    /// Open a URL and extract page content
    Open {
        /// URL to open
        url: String,

        /// Output format: text (default), html, or json
        #[arg(long, default_value = "text")]
        format: String,

        /// Wait time in seconds before capturing (for JS-heavy pages)
        #[arg(long, default_value = "3")]
        wait: u64,

        /// Save screenshot to this path (PNG)
        #[arg(long)]
        screenshot: Option<String>,
    },

    /// Take a screenshot of a URL
    Screenshot {
        /// URL to screenshot
        url: String,

        /// Output file path (default: screenshot.png)
        #[arg(short, long, default_value = "screenshot.png")]
        output: String,

        /// Viewport width
        #[arg(long, default_value = "1280")]
        width: u32,

        /// Viewport height
        #[arg(long, default_value = "720")]
        height: u32,
    },

    /// Extract links from a page
    Links {
        /// URL to extract links from
        url: String,

        /// Only show external links
        #[arg(long)]
        external_only: bool,
    },

    /// Check if a browser binary is available
    Check,
}

/// Result of a browser operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserResult {
    /// URL that was visited.
    pub url: String,
    /// Page title.
    pub title: Option<String>,
    /// Extracted text content.
    pub content: Option<String>,
    /// Screenshot file path.
    pub screenshot_path: Option<String>,
    /// Extracted links.
    pub links: Vec<PageLink>,
    /// Whether the operation succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

/// A link found on a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageLink {
    pub href: String,
    pub text: String,
    pub is_external: bool,
}

/// Find a usable browser binary.
fn find_browser() -> Option<PathBuf> {
    let candidates = if cfg!(target_os = "windows") {
        vec![
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"C:\Program Files (x86)\BraveSoftware\Brave-Browser\Application\brave.exe",
            "chrome",
            "msedge",
            "brave",
        ]
    } else {
        vec![
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "chromium-browser",
            "chromium",
            "google-chrome",
            "google-chrome-stable",
            "chrome",
        ]
    };

    for candidate in candidates {
        let path = Path::new(candidate);
        if path.exists() {
            return Some(path.to_path_buf());
        }
        // Check in PATH
        let lookup = if cfg!(target_os = "windows") {
            Command::new("where").arg(candidate).output()
        } else {
            Command::new("which").arg(candidate).output()
        };
        if let Ok(output) = lookup
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let path_str = stdout
                .lines()
                .map(|line| line.trim())
                .find(|line| !line.is_empty());
            if let Some(path_str) = path_str {
                return Some(PathBuf::from(path_str));
            }
        }
    }

    None
}

/// Run browser headless and capture DOM content.
fn headless_dom_dump(browser: &Path, url: &str, wait_secs: u64) -> anyhow::Result<String> {
    let output = Command::new(browser)
        .args([
            "--headless",
            "--disable-gpu",
            "--no-sandbox",
            "--dump-dom",
            &format!("--virtual-time-budget={}", wait_secs * 1000),
            url,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Browser failed: {}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run browser headless and take a screenshot.
fn headless_screenshot(
    browser: &Path,
    url: &str,
    output_path: &str,
    width: u32,
    height: u32,
) -> anyhow::Result<()> {
    let status = Command::new(browser)
        .args([
            "--headless",
            "--disable-gpu",
            "--no-sandbox",
            &format!("--screenshot={}", output_path),
            &format!("--window-size={},{}", width, height),
            url,
        ])
        .status()?;

    if !status.success() {
        anyhow::bail!("Screenshot capture failed");
    }

    Ok(())
}

/// Extract text content from HTML (very basic — strips tags).
fn extract_text_from_html(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;

    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                // Check for script/style opening
                let lower = html.to_lowercase();
                if let Some(pos) = lower.find("<script")
                    && pos == text.len()
                {
                    in_script = true;
                }
                if let Some(pos) = lower.find("<style")
                    && pos == text.len()
                {
                    in_style = true;
                }
            }
            '>' => {
                in_tag = false;
                // Check for script/style closing
                if in_script || in_style {
                    let recent: String = text.chars().rev().take(10).collect();
                    if recent.contains("tpircs/") || recent.contains("elyts/") {
                        in_script = false;
                        in_style = false;
                    }
                }
            }
            _ if !in_tag && !in_script && !in_style => {
                text.push(c);
            }
            _ => {}
        }
    }

    // Clean up whitespace
    text.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// Extract links from HTML.
fn extract_links(html: &str, base_url: &str) -> Vec<PageLink> {
    let mut links = Vec::new();
    let base_host = url::Url::parse(base_url)
        .ok()
        .and_then(|u| u.host_str().map(String::from));

    // Simple regex-like extraction for href attributes
    let mut search_from = 0;
    while let Some(href_pos) = html[search_from..].find("href=\"") {
        let start = search_from + href_pos + 6;
        if let Some(end) = html[start..].find('"') {
            let href = &html[start..start + end];

            // Extract link text (next > to <)
            let after_tag = &html[start + end..];
            let text = if let Some(gt) = after_tag.find('>') {
                let after_gt = &after_tag[gt + 1..];
                if let Some(lt) = after_gt.find('<') {
                    after_gt[..lt].trim().to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let is_external = if let Some(ref host) = base_host {
                url::Url::parse(href)
                    .ok()
                    .and_then(|u| u.host_str().map(String::from))
                    .is_some_and(|h| &h != host)
            } else {
                href.starts_with("http")
            };

            if href.starts_with("http") || href.starts_with("/") {
                links.push(PageLink {
                    href: href.to_string(),
                    text,
                    is_external,
                });
            }

            search_from = start + end + 1;
        } else {
            break;
        }
    }

    links
}

/// Run a browser CLI command.
pub async fn run_browser_command(cmd: BrowserCommand) -> anyhow::Result<()> {
    match cmd {
        BrowserCommand::Check => {
            match find_browser() {
                Some(path) => {
                    println!("✅ Browser found: {}", path.display());

                    // Try getting version
                    if let Ok(output) = Command::new(&path).arg("--version").output()
                        && output.status.success()
                    {
                        let version = String::from_utf8_lossy(&output.stdout);
                        println!("   Version: {}", version.trim());
                    }
                }
                None => {
                    println!("❌ No browser found.");
                    println!();
                    println!("Install one of:");
                    println!("  • Google Chrome");
                    if cfg!(target_os = "windows") {
                        println!("  • Microsoft Edge");
                        println!("  • Brave Browser");
                        println!();
                        println!(
                            "Windows: install Chrome, Edge, or Brave, or use Docker Desktop for the container fallback"
                        );
                    } else {
                        println!("  • Chromium");
                        println!("  • Brave Browser");
                        println!();
                        println!("macOS: brew install --cask google-chrome");
                        println!("Linux: apt install chromium-browser");
                    }
                }
            }
        }

        BrowserCommand::Open {
            url,
            format,
            wait,
            screenshot,
        } => {
            let browser = find_browser().ok_or_else(|| {
                anyhow::anyhow!("No browser found. Run `thinclaw browser check` for setup info.")
            })?;

            let html = headless_dom_dump(&browser, &url, wait)?;

            match format.as_str() {
                "html" => println!("{}", html),
                "json" => {
                    let result = BrowserResult {
                        url: url.clone(),
                        title: extract_title(&html),
                        content: Some(extract_text_from_html(&html)),
                        screenshot_path: screenshot.clone(),
                        links: extract_links(&html, &url),
                        success: true,
                        error: None,
                    };
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                _ => {
                    if let Some(title) = extract_title(&html) {
                        println!("Title: {}\n", title);
                    }
                    println!("{}", extract_text_from_html(&html));
                }
            }

            if let Some(ref path) = screenshot {
                headless_screenshot(&browser, &url, path, 1280, 720)?;
                println!("\nScreenshot saved to: {}", path);
            }
        }

        BrowserCommand::Screenshot {
            url,
            output,
            width,
            height,
        } => {
            let browser = find_browser().ok_or_else(|| {
                anyhow::anyhow!("No browser found. Run `thinclaw browser check` for setup info.")
            })?;

            headless_screenshot(&browser, &url, &output, width, height)?;
            println!("Screenshot saved to: {}", output);
        }

        BrowserCommand::Links { url, external_only } => {
            let browser = find_browser().ok_or_else(|| {
                anyhow::anyhow!("No browser found. Run `thinclaw browser check` for setup info.")
            })?;

            let html = headless_dom_dump(&browser, &url, 3)?;
            let links = extract_links(&html, &url);

            let filtered: Vec<&PageLink> = if external_only {
                links.iter().filter(|l| l.is_external).collect()
            } else {
                links.iter().collect()
            };

            for link in &filtered {
                let ext_marker = if link.is_external { " [ext]" } else { "" };
                if link.text.is_empty() {
                    println!("  {}{}", link.href, ext_marker);
                } else {
                    println!("  {} — {}{}", link.text, link.href, ext_marker);
                }
            }

            println!("\n{} link(s) found.", filtered.len());
        }
    }

    Ok(())
}

/// Extract <title> from HTML.
fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title>")?;
    let end = lower[start..].find("</title>")?;
    Some(html[start + 7..start + end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_browser_does_not_panic() {
        // May or may not find a browser, but should not panic
        let _ = find_browser();
    }

    #[test]
    fn test_extract_title() {
        let html = "<html><head><title>Hello World</title></head><body></body></html>";
        assert_eq!(extract_title(html), Some("Hello World".to_string()));
    }

    #[test]
    fn test_extract_title_missing() {
        let html = "<html><body>No title here</body></html>";
        assert_eq!(extract_title(html), None);
    }

    #[test]
    fn test_extract_text_from_html() {
        let html = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        let text = extract_text_from_html(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
    }

    #[test]
    fn test_extract_links() {
        let html = r#"<a href="https://example.com">Example</a> <a href="/about">About</a>"#;
        let links = extract_links(html, "https://mysite.com");
        assert_eq!(links.len(), 2);
        assert!(links[0].is_external);
        assert!(!links[1].is_external);
    }

    #[test]
    fn test_extract_links_empty() {
        let html = "<p>No links here</p>";
        let links = extract_links(html, "https://example.com");
        assert!(links.is_empty());
    }

    #[test]
    fn test_browser_result_serialization() {
        let result = BrowserResult {
            url: "https://example.com".to_string(),
            title: Some("Example".to_string()),
            content: Some("Hello world".to_string()),
            screenshot_path: None,
            links: vec![],
            success: true,
            error: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("example.com"));
    }
}
