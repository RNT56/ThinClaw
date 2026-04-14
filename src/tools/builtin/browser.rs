//! Browser automation tool using CDP (Chrome DevTools Protocol).
//!
//! Provides the agent with web browsing capabilities: navigating pages,
//! reading content via accessibility tree snapshots, clicking elements,
//! typing text, taking screenshots, and evaluating JavaScript.
//!
//! Uses `chromiumoxide` to connect to a Chrome/Chromium instance via CDP.
//!
//! **Chrome resolution order:**
//! 1. Local Chrome/Chromium binary (macOS, Linux, Windows)
//! 2. Docker container with Chromium + Xvfb (automatic fallback when no
//!    local binary is found and Docker is available, or forced via
//!    `BROWSER_DOCKER=always` env var)

use std::collections::HashMap;
use std::fmt::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chromiumoxide::cdp::browser_protocol::accessibility::{AxNode, GetFullAxTreeParams};
use chromiumoxide::cdp::browser_protocol::dom::{BackendNodeId, FocusParams, ResolveNodeParams};
use chromiumoxide::cdp::browser_protocol::input::{DispatchKeyEventParams, DispatchKeyEventType};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::cdp::js_protocol::runtime::CallFunctionOnParams;
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use tokio::sync::RwLock;

use crate::context::JobContext;
use crate::sandbox::docker_chromium::DockerChromiumConfig;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

// ── Limits ───────────────────────────────────────────────────────────

/// Maximum number of open tabs before the oldest is auto-closed.
const MAX_TABS: usize = 8;

/// Character limit for `get_text` content returned to the LLM.
const TEXT_CHAR_LIMIT: usize = 16_000;

/// Navigation page-load timeout.
const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(30);

// ── Navigation guard ─────────────────────────────────────────────────

/// Blocked URL schemes.
const BLOCKED_SCHEMES: &[&str] = &["file", "chrome", "chrome-extension", "about", "data"];

/// Blocked hosts (internal/metadata services).
const BLOCKED_HOSTS: &[&str] = &[
    "localhost",
    "127.0.0.1",
    "0.0.0.0",
    "::1",
    "metadata.google.internal",
    "169.254.169.254",
];

/// Blocked ports (common service ports).
const BLOCKED_PORTS: &[u16] = &[22, 5432, 6379, 3306, 27017, 9200, 2375, 2376];

fn is_url_allowed(raw_url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(raw_url).map_err(|e| format!("Invalid URL: {e}"))?;

    if BLOCKED_SCHEMES.contains(&parsed.scheme()) {
        return Err(format!("Blocked scheme: {}", parsed.scheme()));
    }

    if let Some(host) = parsed.host_str() {
        for blocked in BLOCKED_HOSTS {
            if host == *blocked || host.ends_with(blocked) {
                return Err(format!("Blocked host: {host}"));
            }
        }
    }

    if let Some(port) = parsed.port()
        && BLOCKED_PORTS.contains(&port)
    {
        return Err(format!("Blocked port: {port}"));
    }

    Ok(())
}

// ── Interactive role detection ───────────────────────────────────────

fn is_interactive_role(role: &str) -> bool {
    matches!(
        role,
        "button"
            | "link"
            | "textbox"
            | "checkbox"
            | "radio"
            | "combobox"
            | "menuitem"
            | "tab"
            | "slider"
            | "switch"
            | "searchbox"
            | "spinbutton"
            | "option"
            | "menuitemcheckbox"
            | "menuitemradio"
    )
}

// ── AX node ref tracking ─────────────────────────────────────────────

/// Reference to an accessibility tree node, mapped from a short ref ID.
#[derive(Debug, Clone)]
struct AXNodeRef {
    /// CDP backend node ID for DOM interaction.
    backend_node_id: Option<BackendNodeId>,
    /// The accessibility role.
    role: String,
    /// The accessible name.
    name: String,
}

/// State for a single browser page/tab.
struct PageState {
    page: Page,
    /// Cached accessibility tree refs from last snapshot.
    role_refs: HashMap<String, AXNodeRef>,
}

/// Shared browser instance with page tracking.
struct BrowserInstance {
    browser: Browser,
    /// Active pages by tab index.
    pages: Vec<PageState>,
    /// The "current" page index.
    current_page: Option<usize>,
    /// Chrome user data directory.
    _user_data_dir: PathBuf,
    /// Whether this instance is connected to a Docker container.
    is_docker: bool,
}

type SharedBrowser = Arc<RwLock<Option<BrowserInstance>>>;

/// Browser automation tool.
///
/// Exposes browser actions (navigate, snapshot, click, type, screenshot, eval)
/// as a single unified tool with an `action` parameter. The tool lazily
/// launches Chrome on first use and reuses the instance.
///
/// When no local Chrome binary is found, the tool automatically falls back to
/// running Chromium inside a Docker container (if Docker is available).
/// Set `BROWSER_DOCKER=always` to force Docker mode even when local Chrome
/// exists.
pub struct BrowserTool {
    instance: SharedBrowser,
    profile_dir: PathBuf,
    /// Docker config for Chromium fallback (or forced mode).
    docker_config: Option<DockerChromiumConfig>,
}

impl BrowserTool {
    /// Create a BrowserTool that uses local Chrome.
    pub fn new(profile_dir: PathBuf) -> Self {
        Self {
            instance: Arc::new(RwLock::new(None)),
            profile_dir,
            docker_config: None,
        }
    }

    /// Create a BrowserTool with Docker Chromium fallback (or forced mode).
    pub fn new_with_docker(profile_dir: PathBuf, docker_config: DockerChromiumConfig) -> Self {
        Self {
            instance: Arc::new(RwLock::new(None)),
            profile_dir,
            docker_config: Some(docker_config),
        }
    }

    /// Find Chrome/Chromium executable on the system.
    fn find_chrome() -> Option<PathBuf> {
        let candidates = if cfg!(target_os = "macos") {
            vec![
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
                "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
                "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            ]
        } else if cfg!(target_os = "linux") {
            vec![
                "/usr/bin/google-chrome",
                "/usr/bin/google-chrome-stable",
                "/usr/bin/chromium",
                "/usr/bin/chromium-browser",
                "/snap/bin/chromium",
            ]
        } else {
            vec![
                r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            ]
        };

        for path in candidates {
            let p = PathBuf::from(path);
            if p.exists() {
                return Some(p);
            }
        }
        None
    }

    /// Get or launch the browser instance.
    ///
    /// Resolution order:
    /// 1. If `BROWSER_DOCKER=always`, skip local Chrome and use Docker.
    /// 2. Try to find and launch a local Chrome/Chromium binary.
    /// 3. If no local binary found and Docker is available with a
    ///    `DockerChromiumConfig`, start a container and connect via CDP.
    ///
    /// If Chrome was previously launched but has since crashed (CDP connection
    /// lost), the dead instance is dropped and a fresh one is started.
    async fn ensure_browser(&self) -> Result<(), ToolError> {
        let mut guard = self.instance.write().await;

        // If we have an instance, verify Chrome is still alive by pinging CDP.
        if let Some(ref instance) = *guard {
            // `browser.pages()` makes a CDP call — if the process died this
            // will return an error, signalling we must re-launch.
            if instance.browser.pages().await.is_err() {
                tracing::warn!("Chrome process appears dead, re-launching");
                // If it was a Docker instance, try to stop the dead container.
                if instance.is_docker
                    && let Some(ref dc) = self.docker_config
                {
                    let _ = dc.stop_container();
                }
                *guard = None;
            } else {
                return Ok(());
            }
        }

        // Check if Docker mode is forced via env var.
        let force_docker = std::env::var("BROWSER_DOCKER")
            .map(|v| v.eq_ignore_ascii_case("always"))
            .unwrap_or(false);

        // Try local Chrome first (unless Docker is forced).
        if !force_docker && let Some(chrome_path) = Self::find_chrome() {
            return self.launch_local_chrome(&mut guard, chrome_path).await;
        }

        // Fall back to Docker Chromium.
        if let Some(ref docker_config) = self.docker_config {
            if DockerChromiumConfig::is_docker_available() {
                return self.connect_docker_chrome(&mut guard, docker_config).await;
            }
            tracing::warn!("Docker not available for browser fallback");
        }

        // Neither local Chrome nor Docker available.
        Err(ToolError::ExecutionFailed(
            "Chrome/Chromium not found. Either install Google Chrome \
             (https://www.google.com/chrome), or install Docker and set \
             BROWSER_DOCKER=always. On macOS: brew install --cask google-chrome. \
             On Linux: apt install chromium-browser."
                .to_string(),
        ))
    }

    /// Launch a local Chrome binary and store the instance.
    async fn launch_local_chrome(
        &self,
        guard: &mut tokio::sync::RwLockWriteGuard<'_, Option<BrowserInstance>>,
        chrome_path: PathBuf,
    ) -> Result<(), ToolError> {
        std::fs::create_dir_all(&self.profile_dir).map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to create profile dir: {e}"))
        })?;

        let config = BrowserConfig::builder()
            .chrome_executable(chrome_path)
            .user_data_dir(&self.profile_dir)
            .window_size(1280, 900)
            // Use the new headless mode via the proper builder method.
            .new_headless_mode()
            .no_sandbox()
            // Only pass args NOT already in chromiumoxide's DEFAULT_ARGS.
            .arg("disable-gpu")
            .arg("no-default-browser-check")
            // Realistic user-agent for better site compatibility.
            .arg("user-agent=Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36")
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("BrowserConfig error: {e}")))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to launch Chrome: {e}")))?;

        // Spawn the CDP handler loop (Handler implements futures::Stream)
        tokio::spawn(async move { while handler.next().await.is_some() {} });

        tracing::info!("Chrome launched locally for browser tool");

        **guard = Some(BrowserInstance {
            browser,
            pages: Vec::new(),
            current_page: None,
            _user_data_dir: self.profile_dir.clone(),
            is_docker: false,
        });

        Ok(())
    }

    /// Start a Docker container running Chromium and connect via CDP.
    async fn connect_docker_chrome(
        &self,
        guard: &mut tokio::sync::RwLockWriteGuard<'_, Option<BrowserInstance>>,
        docker_config: &DockerChromiumConfig,
    ) -> Result<(), ToolError> {
        // Start the container (idempotent — re-uses running container).
        docker_config.start_container().map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to start Docker Chromium: {e}"))
        })?;

        // Wait for Chrome inside the container to be ready.
        docker_config
            .wait_for_ready(Duration::from_secs(30))
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "Docker Chromium not ready: {e}. Is the image `{}` available?",
                    docker_config.image
                ))
            })?;

        // Connect to Chrome via its HTTP endpoint. `chromiumoxide` will
        // automatically discover the WebSocket URL from /json/version.
        let endpoint = docker_config.http_endpoint();
        let (browser, mut handler) = Browser::connect(&endpoint).await.map_err(|e| {
            ToolError::ExecutionFailed(format!(
                "Failed to connect to Docker Chromium at {endpoint}: {e}"
            ))
        })?;

        // Spawn the CDP handler loop.
        tokio::spawn(async move { while handler.next().await.is_some() {} });

        tracing::info!(
            endpoint = %endpoint,
            "Connected to Docker Chromium for browser tool"
        );

        **guard = Some(BrowserInstance {
            browser,
            pages: Vec::new(),
            current_page: None,
            _user_data_dir: self.profile_dir.clone(),
            is_docker: true,
        });

        Ok(())
    }

    /// Navigate to a URL.
    ///
    /// Opens a new tab and sets it as the current page. When the tab count
    /// exceeds [`MAX_TABS`], the oldest tab is closed automatically.
    async fn navigate(&self, url: &str) -> Result<serde_json::Value, ToolError> {
        is_url_allowed(url).map_err(ToolError::ExecutionFailed)?;
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().expect("browser instance ensured");

        let page = instance
            .browser
            .new_page(url)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Navigation failed: {e}")))?;

        // Wait for the page to load (with timeout).
        match tokio::time::timeout(PAGE_LOAD_TIMEOUT, page.wait_for_navigation()).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                tracing::debug!("Navigation wait error (non-fatal): {e}");
            }
            Err(_) => {
                tracing::debug!(
                    "Page load timed out after {:?}, proceeding anyway",
                    PAGE_LOAD_TIMEOUT
                );
            }
        }

        let page_url = page
            .url()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| url.to_string());

        // Evict the oldest tab if we've hit the limit.
        if instance.pages.len() >= MAX_TABS {
            let evicted = instance.pages.remove(0);
            let _ = evicted.page.close().await;
            tracing::debug!("Evicted oldest tab (limit={MAX_TABS})");
            // Adjust current_page index after removal.
            if let Some(ref mut cp) = instance.current_page {
                *cp = cp.saturating_sub(1);
            }
        }

        let idx = instance.pages.len();
        instance.pages.push(PageState {
            page,
            role_refs: HashMap::new(),
        });
        instance.current_page = Some(idx);

        Ok(serde_json::json!({
            "status": "navigated",
            "url": page_url,
            "tab_index": idx,
            "open_tabs": instance.pages.len(),
        }))
    }

    /// Get the current page state mutably.
    fn current_page_mut(instance: &mut BrowserInstance) -> Result<&mut PageState, ToolError> {
        let idx = instance
            .current_page
            .ok_or_else(|| ToolError::ExecutionFailed("No page open. Navigate first.".into()))?;

        instance
            .pages
            .get_mut(idx)
            .ok_or_else(|| ToolError::ExecutionFailed("Page not found".into()))
    }

    /// Get the current page state immutably.
    fn current_page(instance: &BrowserInstance) -> Result<&PageState, ToolError> {
        let idx = instance
            .current_page
            .ok_or_else(|| ToolError::ExecutionFailed("No page open. Navigate first.".into()))?;

        instance
            .pages
            .get(idx)
            .ok_or_else(|| ToolError::ExecutionFailed("Page not found".into()))
    }

    /// Generate an accessibility tree snapshot with numbered refs.
    async fn snapshot(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().expect("browser instance ensured");
        let state = Self::current_page_mut(instance)?;

        let page_url = state.page.url().await.ok().flatten().unwrap_or_default();

        // Get the full accessibility tree via CDP
        let tree_result = state
            .page
            .execute(GetFullAxTreeParams::default())
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to get accessibility tree: {e}"))
            })?;

        let nodes: &[AxNode] = &tree_result.result.nodes;

        let mut output = String::new();
        let _ = writeln!(output, "[page] {page_url}");

        let mut ref_counter = 0u32;
        let mut refs = HashMap::new();

        for node in nodes {
            let role = node
                .role
                .as_ref()
                .and_then(|r| r.value.as_ref())
                .and_then(|v| v.as_str())
                .unwrap_or("generic");

            let name = node
                .name
                .as_ref()
                .and_then(|n| n.value.as_ref())
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Skip ignored nodes
            if node.ignored {
                continue;
            }

            // Skip generic/none roles with no name
            if (role == "generic" || role == "none" || role == "GenericContainer")
                && name.is_empty()
            {
                continue;
            }

            let ref_label = if is_interactive_role(role) {
                ref_counter += 1;
                let label = format!("e{ref_counter}");
                refs.insert(
                    label.clone(),
                    AXNodeRef {
                        backend_node_id: node.backend_dom_node_id,
                        role: role.to_string(),
                        name: name.to_string(),
                    },
                );
                format!(" ref=\"{label}\"")
            } else {
                String::new()
            };

            let indent = "  ";
            if !name.is_empty() {
                let _ = writeln!(output, "{indent}[{role}{ref_label}] \"{name}\"");
            } else {
                let _ = writeln!(output, "{indent}[{role}{ref_label}]");
            }
        }

        state.role_refs = refs;

        Ok(serde_json::json!({
            "snapshot": output,
            "element_count": ref_counter,
            "url": page_url,
        }))
    }

    /// Click an element by its ref ID.
    async fn click_ref(&self, ref_id: &str) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().expect("browser instance ensured");
        let state = Self::current_page_mut(instance)?;

        let node_ref = state
            .role_refs
            .get(ref_id)
            .ok_or_else(|| {
                ToolError::ExecutionFailed(format!(
                    "Ref '{}' not found. Run snapshot first to get valid refs.",
                    ref_id
                ))
            })?
            .clone();

        let backend_node_id = node_ref.backend_node_id.ok_or_else(|| {
            ToolError::ExecutionFailed(format!(
                "Ref '{}' has no DOM node ID — may not be clickable",
                ref_id
            ))
        })?;

        // Resolve the backend node to a JS object, then call .click()
        let resolve_params = ResolveNodeParams::builder()
            .backend_node_id(backend_node_id)
            .build();

        let resolved = state
            .page
            .execute(resolve_params)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to resolve node: {e}")))?;

        let object_id = resolved
            .result
            .object
            .object_id
            .ok_or_else(|| ToolError::ExecutionFailed("Could not get object ID".into()))?;

        let click_fn = CallFunctionOnParams::builder()
            .function_declaration("function() { this.click(); }")
            .object_id(object_id)
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("Build CallFunctionOnParams: {e}")))?;

        state
            .page
            .execute(click_fn)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Click failed: {e}")))?;

        // Wait briefly for navigation or DOM update
        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok(serde_json::json!({
            "status": "clicked",
            "ref": ref_id,
            "role": node_ref.role,
            "name": node_ref.name,
        }))
    }

    /// Type text into an element by ref ID.
    async fn type_text(&self, ref_id: &str, text: &str) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().expect("browser instance ensured");
        let state = Self::current_page_mut(instance)?;

        let node_ref = state
            .role_refs
            .get(ref_id)
            .ok_or_else(|| ToolError::ExecutionFailed(format!("Ref '{}' not found", ref_id)))?
            .clone();

        let backend_node_id = node_ref.backend_node_id.ok_or_else(|| {
            ToolError::ExecutionFailed(format!("Ref '{}' has no DOM node ID", ref_id))
        })?;

        // Focus the element
        let focus_params = FocusParams::builder()
            .backend_node_id(backend_node_id)
            .build();

        state
            .page
            .execute(focus_params)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to focus element: {e}")))?;

        // Clear existing content
        let resolve_params = ResolveNodeParams::builder()
            .backend_node_id(backend_node_id)
            .build();

        let resolved = state
            .page
            .execute(resolve_params)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to resolve node: {e}")))?;

        if let Some(object_id) = resolved.result.object.object_id {
            let clear_fn = CallFunctionOnParams::builder()
                .function_declaration("function() { this.value = ''; }")
                .object_id(object_id)
                .build()
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("Build CallFunctionOnParams: {e}"))
                })?;
            let _ = state.page.execute(clear_fn).await;
        }

        // Type each character via CDP Input.dispatchKeyEvent
        for ch in text.chars() {
            let key_down = DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyDown)
                .text(ch.to_string())
                .build()
                .map_err(|e| ToolError::ExecutionFailed(format!("KeyEvent build error: {e}")))?;

            let _ = state.page.execute(key_down).await;

            let key_up = DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyUp)
                .text(ch.to_string())
                .build()
                .map_err(|e| ToolError::ExecutionFailed(format!("KeyEvent build error: {e}")))?;

            let _ = state.page.execute(key_up).await;
        }

        Ok(serde_json::json!({
            "status": "typed",
            "ref": ref_id,
            "text_length": text.len(),
        }))
    }

    /// Take a screenshot of the current page.
    async fn screenshot(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let guard = self.instance.read().await;
        let instance = guard.as_ref().expect("browser instance ensured");
        let state = Self::current_page(instance)?;

        let screenshot_bytes = state
            .page
            .screenshot(
                chromiumoxide::page::ScreenshotParams::builder()
                    .format(CaptureScreenshotFormat::Png)
                    .full_page(false)
                    .build(),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Screenshot failed: {e}")))?;

        // Save to temp file — use a deterministic name so repeated screenshots
        // from the same session overwrite rather than accumulating.
        let screenshot_path = std::env::temp_dir().join("thinclaw_browser_screenshot.png");

        tokio::fs::write(&screenshot_path, &screenshot_bytes)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to save screenshot: {e}")))?;

        Ok(serde_json::json!({
            "status": "screenshot_taken",
            "path": screenshot_path.to_string_lossy(),
            "size_bytes": screenshot_bytes.len(),
        }))
    }

    /// Evaluate JavaScript on the current page.
    async fn evaluate(&self, expression: &str) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let guard = self.instance.read().await;
        let instance = guard.as_ref().expect("browser instance ensured");
        let state = Self::current_page(instance)?;

        let eval_result = state
            .page
            .evaluate_expression(expression)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("JS evaluation failed: {e}")))?;

        let value = eval_result
            .value()
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        Ok(serde_json::json!({
            "result": value,
        }))
    }

    /// Get the page's content as clean Markdown (preserves structure for LLM comprehension).
    async fn get_text(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let guard = self.instance.read().await;
        let instance = guard.as_ref().expect("browser instance ensured");
        let state = Self::current_page(instance)?;

        let page_url = state.page.url().await.ok().flatten().unwrap_or_default();

        // Extract a structured Markdown representation of the page.
        // Preserves headings, links, lists, and paragraph structure — far more
        // useful than flat innerText for LLM comprehension.
        let extractor_js = r#"(function(){
function w(el,d){if(!el||el.nodeType===8)return '';if(el.nodeType===3)return el.textContent||'';
const tag=el.tagName?el.tagName.toLowerCase():'';
const st=el.nodeType===1?window.getComputedStyle(el):null;
if(st&&(st.display==='none'||st.visibility==='hidden'))return '';
if(['script','style','noscript','nav','footer','aside','header'].includes(tag))return '';
const ch=Array.from(el.childNodes).map(c=>w(c,d+1)).join('');
switch(tag){
case 'h1':return '\n# '+ch.trim()+'\n';case 'h2':return '\n## '+ch.trim()+'\n';
case 'h3':return '\n### '+ch.trim()+'\n';case 'h4':return '\n#### '+ch.trim()+'\n';
case 'p':return '\n'+ch.trim()+'\n';case 'li':return '\n- '+ch.trim();
case 'ul':case 'ol':return ch+'\n';case 'br':return '\n';
case 'a':const href=el.getAttribute('href')||'';const t=ch.trim();
if(!t||href.startsWith('javascript:'))return t;if(href.startsWith('#'))return t;
return '['+t+']('+href+')';
case 'strong':case 'b':return '**'+ch+'**';case 'em':case 'i':return '*'+ch+'*';
case 'code':return '`'+ch+'`';case 'pre':return '\n```\n'+el.innerText+'\n```\n';
case 'table':return '\n'+Array.from(el.querySelectorAll('tr')).map(r=>
Array.from(r.querySelectorAll('td,th')).map(c=>c.innerText.trim()).join(' | ')).join('\n')+'\n';
default:const bl=['div','section','article','main','figure'].includes(tag);
return bl?'\n'+ch+'\n':ch;}}
const m=document.querySelector('main,[role="main"],article,.content,.main-content,#content,#main')||document.body;
return w(m,0).replace(/\n{3,}/g,'\n\n').trim();
})()"#;

        let eval_result = state
            .page
            .evaluate_expression(extractor_js)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to extract page content: {e}"))
            })?;

        let text = eval_result.into_value::<String>().unwrap_or_default();

        // Truncate on a char boundary (not byte boundary) to avoid panics on
        // multi-byte UTF-8 text.
        let truncated = if text.chars().count() > TEXT_CHAR_LIMIT {
            let safe: String = text.chars().take(TEXT_CHAR_LIMIT).collect();
            format!(
                "{safe}\n\n[… truncated — {} chars total. Use snapshot + scroll for more.]\n",
                text.chars().count()
            )
        } else {
            text
        };

        Ok(serde_json::json!({
            "url": page_url,
            "content": truncated,
            "format": "markdown",
            "length": truncated.len(),
        }))
    }

    /// Close all open tabs and release the browser instance, reclaiming
    /// resources. The next `execute` call will re-launch Chrome as needed.
    ///
    /// If the browser was running in a Docker container, the container is
    /// stopped and removed.
    async fn close_session(&self) -> Result<serde_json::Value, ToolError> {
        let mut guard = self.instance.write().await;
        if let Some(mut instance) = guard.take() {
            let tab_count = instance.pages.len();
            let was_docker = instance.is_docker;
            for state in instance.pages.drain(..) {
                let _ = state.page.close().await;
            }
            // Drop the browser, which kills the Chrome process (local) or
            // closes the WebSocket connection (Docker).
            drop(instance);

            // Stop the Docker container if applicable.
            if was_docker && let Some(ref dc) = self.docker_config {
                let _ = dc.stop_container();
            }

            tracing::info!(
                tabs = tab_count,
                docker = was_docker,
                "Browser session closed"
            );
            Ok(serde_json::json!({
                "status": "session_closed",
                "tabs_closed": tab_count,
                "was_docker": was_docker,
            }))
        } else {
            Ok(serde_json::json!({
                "status": "no_session",
                "message": "No browser session was active",
            }))
        }
    }

    /// List open tabs.
    async fn list_tabs(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let guard = self.instance.read().await;
        let instance = guard.as_ref().expect("browser instance ensured");

        let mut tabs = Vec::new();
        for (i, state) in instance.pages.iter().enumerate() {
            let url = state.page.url().await.ok().flatten().unwrap_or_default();
            let is_current = instance.current_page == Some(i);
            tabs.push(serde_json::json!({
                "index": i,
                "url": url,
                "current": is_current,
            }));
        }

        Ok(serde_json::json!({
            "tabs": tabs,
            "count": instance.pages.len(),
            "max_tabs": MAX_TABS,
        }))
    }

    /// Switch to a tab by index.
    async fn switch_tab(&self, tab_index: usize) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().expect("browser instance ensured");

        if tab_index >= instance.pages.len() {
            return Err(ToolError::InvalidParameters(format!(
                "Tab index {} out of range (0..{})",
                tab_index,
                instance.pages.len()
            )));
        }

        instance.current_page = Some(tab_index);
        let url = instance.pages[tab_index]
            .page
            .url()
            .await
            .ok()
            .flatten()
            .unwrap_or_default();

        Ok(serde_json::json!({
            "status": "switched",
            "tab_index": tab_index,
            "url": url,
        }))
    }

    /// Scroll the current page.
    async fn scroll(&self, direction: &str, amount: Option<i64>) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let guard = self.instance.read().await;
        let instance = guard.as_ref().expect("browser instance ensured");
        let state = Self::current_page(instance)?;

        let pixels = amount.unwrap_or(500);
        let js = match direction {
            "up" => format!("window.scrollBy(0, -{})", pixels),
            "down" => format!("window.scrollBy(0, {})", pixels),
            "top" => "window.scrollTo(0, 0)".to_string(),
            "bottom" => "window.scrollTo(0, document.body.scrollHeight)".to_string(),
            _ => format!("window.scrollBy(0, {})", pixels), // default: down
        };

        state.page.evaluate_expression(&js).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Scroll failed: {e}"))
        })?;

        // Allow time for lazy-loaded content
        tokio::time::sleep(Duration::from_millis(300)).await;

        Ok(serde_json::json!({
            "status": "scrolled",
            "direction": direction,
            "pixels": pixels,
        }))
    }

    /// Press a keyboard key (Enter, Tab, Escape, etc.) on the current page.
    async fn press_key(&self, key: &str) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().expect("browser instance ensured");
        let state = Self::current_page_mut(instance)?;

        // Map user-friendly key names to CDP key identifiers
        let (key_code, text) = match key.to_lowercase().as_str() {
            "enter" | "return" => ("Enter", Some("\r")),
            "tab" => ("Tab", Some("\t")),
            "escape" | "esc" => ("Escape", None),
            "backspace" => ("Backspace", None),
            "delete" => ("Delete", None),
            "arrowup" | "up" => ("ArrowUp", None),
            "arrowdown" | "down" => ("ArrowDown", None),
            "arrowleft" | "left" => ("ArrowLeft", None),
            "arrowright" | "right" => ("ArrowRight", None),
            "space" => ("Space", Some(" ")),
            "home" => ("Home", None),
            "end" => ("End", None),
            "pageup" => ("PageUp", None),
            "pagedown" => ("PageDown", None),
            _ => (key, None),
        };

        let mut key_down_builder = DispatchKeyEventParams::builder()
            .r#type(DispatchKeyEventType::KeyDown)
            .key(key_code);

        if let Some(t) = text {
            key_down_builder = key_down_builder.text(t);
        }

        let key_down = key_down_builder.build()
            .map_err(|e| ToolError::ExecutionFailed(format!("KeyEvent build error: {e}")))?;

        state.page.execute(key_down).await
            .map_err(|e| ToolError::ExecutionFailed(format!("Key press failed: {e}")))?;

        let key_up = DispatchKeyEventParams::builder()
            .r#type(DispatchKeyEventType::KeyUp)
            .key(key_code)
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("KeyEvent build error: {e}")))?;

        state.page.execute(key_up).await
            .map_err(|e| ToolError::ExecutionFailed(format!("Key release failed: {e}")))?;

        // Brief pause for any triggered navigation/updates
        tokio::time::sleep(Duration::from_millis(200)).await;

        Ok(serde_json::json!({
            "status": "key_pressed",
            "key": key_code,
        }))
    }

    /// Go back in browser history.
    async fn go_back(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let guard = self.instance.read().await;
        let instance = guard.as_ref().expect("browser instance ensured");
        let state = Self::current_page(instance)?;

        state.page.evaluate_expression("window.history.back()").await
            .map_err(|e| ToolError::ExecutionFailed(format!("Back navigation failed: {e}")))?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        let url = state.page.url().await.ok().flatten().unwrap_or_default();
        Ok(serde_json::json!({
            "status": "went_back",
            "url": url,
        }))
    }

    /// Go forward in browser history.
    async fn go_forward(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let guard = self.instance.read().await;
        let instance = guard.as_ref().expect("browser instance ensured");
        let state = Self::current_page(instance)?;

        state.page.evaluate_expression("window.history.forward()").await
            .map_err(|e| ToolError::ExecutionFailed(format!("Forward navigation failed: {e}")))?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        let url = state.page.url().await.ok().flatten().unwrap_or_default();
        Ok(serde_json::json!({
            "status": "went_forward",
            "url": url,
        }))
    }

    /// Get all images on the current page with their src and alt text.
    async fn get_images(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let guard = self.instance.read().await;
        let instance = guard.as_ref().expect("browser instance ensured");
        let state = Self::current_page(instance)?;

        let js = r#"
        (function() {
            const imgs = Array.from(document.querySelectorAll('img'));
            return imgs.slice(0, 50).map(img => ({
                src: img.src || '',
                alt: img.alt || '',
                width: img.naturalWidth || img.width || 0,
                height: img.naturalHeight || img.height || 0,
            }));
        })()
        "#;

        let eval_result = state.page.evaluate_expression(js).await
            .map_err(|e| ToolError::ExecutionFailed(format!("get_images failed: {e}")))?;

        let images = eval_result.value().cloned().unwrap_or(serde_json::json!([]));

        Ok(serde_json::json!({
            "images": images,
            "count": images.as_array().map(|a| a.len()).unwrap_or(0),
        }))
    }

    /// Get browser console messages from the current page.
    async fn get_console(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let guard = self.instance.read().await;
        let instance = guard.as_ref().expect("browser instance ensured");
        let state = Self::current_page(instance)?;

        // Inject a console capture script if not already present
        let js = r#"
        (function() {
            if (!window.__tc_console) {
                window.__tc_console = [];
                const orig = {
                    log: console.log,
                    warn: console.warn,
                    error: console.error,
                    info: console.info,
                };
                ['log', 'warn', 'error', 'info'].forEach(level => {
                    console[level] = function(...args) {
                        window.__tc_console.push({
                            level: level,
                            message: args.map(a => typeof a === 'object' ? JSON.stringify(a) : String(a)).join(' '),
                            time: Date.now()
                        });
                        if (window.__tc_console.length > 100) window.__tc_console.shift();
                        orig[level].apply(console, args);
                    };
                });
            }
            const msgs = window.__tc_console.slice(-50);
            return msgs;
        })()
        "#;

        let eval_result = state.page.evaluate_expression(js).await
            .map_err(|e| ToolError::ExecutionFailed(format!("get_console failed: {e}")))?;

        let messages = eval_result.value().cloned().unwrap_or(serde_json::json!([]));

        Ok(serde_json::json!({
            "console_messages": messages,
            "count": messages.as_array().map(|a| a.len()).unwrap_or(0),
        }))
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Browse the web: navigate pages, read content via accessibility snapshots, \
         click elements, type text, press keys, scroll, go back/forward, take \
         screenshots, extract images, read console output, and evaluate JavaScript. \
         Use 'snapshot' after navigation to see what's on the page — it returns an \
         accessibility tree with numbered refs (e.g., ref=\"e1\") that you can use \
         with 'click' and 'type' actions. Use 'close' when finished browsing."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "snapshot", "click", "type", "press_key", "scroll", "screenshot", "evaluate", "get_text", "get_images", "console", "back", "forward", "close", "tabs", "switch_tab"],
                    "description": "The browser action to perform"
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to (for 'navigate' action)"
                },
                "ref": {
                    "type": "string",
                    "description": "Element ref ID from snapshot (for 'click' and 'type' actions, e.g. 'e1')"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type (for 'type' action)"
                },
                "expression": {
                    "type": "string",
                    "description": "JavaScript expression to evaluate (for 'evaluate' action)"
                },
                "tab_index": {
                    "type": "integer",
                    "description": "Tab index to switch to (for 'switch_tab' action)"
                },
                "key": {
                    "type": "string",
                    "description": "Key to press (for 'press_key' action, e.g. 'Enter', 'Tab', 'Escape', 'ArrowDown')"
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "top", "bottom"],
                    "description": "Scroll direction (for 'scroll' action, default 'down')"
                },
                "amount": {
                    "type": "integer",
                    "description": "Scroll amount in pixels (for 'scroll' action, default 500)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'action' parameter".into()))?;

        let result = match action {
            "navigate" => {
                let url = params.get("url").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("'navigate' requires 'url' parameter".into())
                })?;
                self.navigate(url).await?
            }
            "snapshot" => self.snapshot().await?,
            "click" => {
                let ref_id = params.get("ref").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("'click' requires 'ref' parameter".into())
                })?;
                self.click_ref(ref_id).await?
            }
            "type" => {
                let ref_id = params.get("ref").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("'type' requires 'ref' parameter".into())
                })?;
                let text = params.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("'type' requires 'text' parameter".into())
                })?;
                self.type_text(ref_id, text).await?
            }
            "screenshot" => self.screenshot().await?,
            "evaluate" => {
                let expression = params
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters(
                            "'evaluate' requires 'expression' parameter".into(),
                        )
                    })?;
                self.evaluate(expression).await?
            }
            "get_text" => self.get_text().await?,
            "get_images" => self.get_images().await?,
            "console" => self.get_console().await?,
            "back" => self.go_back().await?,
            "forward" => self.go_forward().await?,
            "scroll" => {
                let direction = params.get("direction").and_then(|v| v.as_str()).unwrap_or("down");
                let amount = params.get("amount").and_then(|v| v.as_i64());
                self.scroll(direction, amount).await?
            }
            "press_key" => {
                let key = params.get("key").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("'press_key' requires 'key' parameter".into())
                })?;
                self.press_key(key).await?
            }
            "close" => self.close_session().await?,
            "tabs" => self.list_tabs().await?,
            "switch_tab" => {
                let tab_index = params
                    .get("tab_index")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters(
                            "'switch_tab' requires 'tab_index' parameter".into(),
                        )
                    })? as usize;
                self.switch_tab(tab_index).await?
            }
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "Unknown action: '{action}'. Use: navigate, snapshot, click, type, press_key, scroll, screenshot, evaluate, get_text, get_images, console, back, forward, close, tabs, switch_tab"
                )));
            }
        };

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(
        &self,
        _params: &serde_json::Value,
    ) -> crate::tools::tool::ApprovalRequirement {
        crate::tools::tool::ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        true // External web content must be sanitized
    }

    /// Browser operations (especially navigation + JS eval) can take longer
    /// than the default 60s timeout.
    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_navigation_guard() {
        assert!(is_url_allowed("https://example.com").is_ok());
        assert!(is_url_allowed("https://google.com/search?q=test").is_ok());
        assert!(is_url_allowed("file:///etc/passwd").is_err());
        assert!(is_url_allowed("chrome://settings").is_err());
        assert!(is_url_allowed("http://localhost:3000").is_err());
        assert!(is_url_allowed("http://127.0.0.1:8080").is_err());
        assert!(is_url_allowed("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(is_url_allowed("http://example.com:22").is_err());
    }

    #[test]
    fn test_interactive_roles() {
        assert!(is_interactive_role("button"));
        assert!(is_interactive_role("textbox"));
        assert!(is_interactive_role("link"));
        assert!(!is_interactive_role("generic"));
        assert!(!is_interactive_role("heading"));
        assert!(!is_interactive_role("paragraph"));
    }

    #[test]
    fn test_browser_tool_schema() {
        let tool = BrowserTool::new(PathBuf::from("/tmp/test-browser"));
        assert_eq!(tool.name(), "browser");

        let schema = tool.parameters_schema();
        let action = schema["properties"]["action"].clone();
        assert!(action["enum"].as_array().unwrap().len() >= 7);
    }

    #[test]
    fn test_execution_timeout_override() {
        let tool = BrowserTool::new(PathBuf::from("/tmp/test-browser"));
        assert_eq!(tool.execution_timeout(), Duration::from_secs(120));
    }

    #[test]
    fn test_new_with_docker() {
        let docker_config = DockerChromiumConfig::default();
        let tool =
            BrowserTool::new_with_docker(PathBuf::from("/tmp/test-browser"), docker_config.clone());
        assert_eq!(tool.name(), "browser");
        assert!(tool.docker_config.is_some());
        assert_eq!(
            tool.docker_config.unwrap().debug_port,
            docker_config.debug_port
        );
    }

    #[test]
    fn test_new_without_docker() {
        let tool = BrowserTool::new(PathBuf::from("/tmp/test-browser"));
        assert!(tool.docker_config.is_none());
    }
}
