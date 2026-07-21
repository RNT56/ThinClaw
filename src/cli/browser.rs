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
use std::time::Duration;

use clap::Subcommand;
use regex::Regex;
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
    crate::platform::find_browser_executable()
}

/// Run browser headless and capture DOM content.
async fn headless_dom_dump(browser: &Path, url: &str, wait_secs: u64) -> anyhow::Result<String> {
    validate_browser_url(url)?;
    let wait_secs = wait_secs.min(120);
    let mut command = tokio::process::Command::new(browser);
    command.args([
        "--headless",
        "--disable-gpu",
        "--dump-dom",
        &format!("--virtual-time-budget={}", wait_secs.saturating_mul(1000)),
        "--",
        url,
    ]);
    let output = thinclaw_platform::bounded_command_output(
        &mut command,
        Duration::from_secs(wait_secs.saturating_add(30)),
        16 * 1024 * 1024,
        1024 * 1024,
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Browser failed: {}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run browser headless and take a screenshot.
async fn headless_screenshot(
    browser: &Path,
    url: &str,
    output_path: &str,
    width: u32,
    height: u32,
) -> anyhow::Result<()> {
    validate_browser_url(url)?;
    if width == 0
        || height == 0
        || width > 8_192
        || height > 8_192
        || u64::from(width) * u64::from(height) > 33_554_432
    {
        anyhow::bail!("screenshot dimensions must be 1..=8192 with at most 33,554,432 pixels");
    }
    let output_path = PathBuf::from(output_path);
    let parent = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        anyhow::bail!("screenshot parent is not a real directory");
    }
    let canonical_parent = std::fs::canonicalize(parent)?;
    let filename = output_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("screenshot output path has no file name"))?;
    let output_path = canonical_parent.join(filename);
    let stage_dir = tempfile::Builder::new()
        .prefix(".thinclaw-screenshot-")
        .tempdir_in(&canonical_parent)?;
    let staged_path = stage_dir.path().join("screenshot.png");
    let mut command = tokio::process::Command::new(browser);
    command.args([
        "--headless",
        "--disable-gpu",
        &format!("--screenshot={}", staged_path.display()),
        &format!("--window-size={width},{height}"),
        "--",
        url,
    ]);
    let result = thinclaw_platform::bounded_command_output(
        &mut command,
        Duration::from_secs(120),
        1024 * 1024,
        1024 * 1024,
    )
    .await;
    let output = result?;
    if !output.status.success() {
        anyhow::bail!(
            "screenshot capture failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    const MAX_SCREENSHOT_FILE_BYTES: u64 = 128 * 1024 * 1024;
    let bytes =
        thinclaw_platform::read_regular_file_bounded(&staged_path, MAX_SCREENSHOT_FILE_BYTES)?;
    if !valid_png_screenshot(&bytes) {
        anyhow::bail!("browser produced an invalid screenshot file");
    }
    thinclaw_platform::write_private_file_atomic(&output_path, &bytes, true)?;

    Ok(())
}

fn valid_png_screenshot(bytes: &[u8]) -> bool {
    if bytes.len() < 24
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || &bytes[8..12] != 13_u32.to_be_bytes().as_slice()
        || &bytes[12..16] != b"IHDR"
    {
        return false;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap_or_default());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap_or_default());
    width > 0
        && height > 0
        && width <= 8_192
        && height <= 8_192
        && u64::from(width) * u64::from(height) <= 33_554_432
}

fn validate_browser_url(raw: &str) -> anyhow::Result<url::Url> {
    let parsed = url::Url::parse(raw)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        anyhow::bail!("browser URL must be HTTP(S) without embedded credentials");
    }
    Ok(parsed)
}

/// Extract text content from HTML (very basic — strips tags).
fn extract_text_from_html(html: &str) -> String {
    let script_re =
        Regex::new(r"(?is)<script[^>]*>.*?</script>").expect("script regex is a tested constant");
    let style_re =
        Regex::new(r"(?is)<style[^>]*>.*?</style>").expect("style regex is a tested constant");
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("tag regex is a tested constant");

    let without_scripts = script_re.replace_all(html, " ");
    let without_styles = style_re.replace_all(&without_scripts, " ");
    let text = tag_re.replace_all(&without_styles, " ");
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
                    let mut command = tokio::process::Command::new(&path);
                    command.arg("--version");
                    if let Ok(output) = thinclaw_platform::bounded_command_output(
                        &mut command,
                        Duration::from_secs(10),
                        64 * 1024,
                        64 * 1024,
                    )
                    .await
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

            let html = headless_dom_dump(&browser, &url, wait).await?;

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
                headless_screenshot(&browser, &url, path, 1280, 720).await?;
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

            headless_screenshot(&browser, &url, &output, width, height).await?;
            println!("Screenshot saved to: {}", output);
        }

        BrowserCommand::Links { url, external_only } => {
            let browser = find_browser().ok_or_else(|| {
                anyhow::anyhow!("No browser found. Run `thinclaw browser check` for setup info.")
            })?;

            let html = headless_dom_dump(&browser, &url, 3).await?;
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

    #[test]
    fn png_screenshot_validation_checks_magic_and_dimensions() {
        let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        png.extend_from_slice(&1280_u32.to_be_bytes());
        png.extend_from_slice(&720_u32.to_be_bytes());
        assert!(valid_png_screenshot(&png));

        png[0] = 0;
        assert!(!valid_png_screenshot(&png));
    }
}
