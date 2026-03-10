//! Browser automation tool using CDP (Chrome DevTools Protocol).
//!
//! Provides the agent with web browsing capabilities: navigating pages,
//! reading content via accessibility tree snapshots, clicking elements,
//! typing text, taking screenshots, and evaluating JavaScript.
//!
//! Uses `chromiumoxide` to connect to a Chrome/Chromium instance via CDP.

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
        && BLOCKED_PORTS.contains(&port) {
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
}

type SharedBrowser = Arc<RwLock<Option<BrowserInstance>>>;

/// Browser automation tool.
///
/// Exposes browser actions (navigate, snapshot, click, type, screenshot, eval)
/// as a single unified tool with an `action` parameter. The tool lazily
/// launches Chrome on first use and reuses the instance.
pub struct BrowserTool {
    instance: SharedBrowser,
    profile_dir: PathBuf,
}

impl BrowserTool {
    pub fn new(profile_dir: PathBuf) -> Self {
        Self {
            instance: Arc::new(RwLock::new(None)),
            profile_dir,
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
                *guard = None;
            } else {
                return Ok(());
            }
        }

        let chrome_path = Self::find_chrome().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "Chrome/Chromium not found. Install Google Chrome from https://www.google.com/chrome \
                 or Chromium from https://www.chromium.org. \
                 Expected at: /Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_string(),
            )
        })?;

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

        tracing::info!("Chrome launched for browser tool");

        *guard = Some(BrowserInstance {
            browser,
            pages: Vec::new(),
            current_page: None,
            _user_data_dir: self.profile_dir.clone(),
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
        let instance = guard.as_mut().unwrap();

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
                tracing::debug!("Page load timed out after {:?}, proceeding anyway", PAGE_LOAD_TIMEOUT);
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
        let instance = guard.as_mut().unwrap();
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
        let instance = guard.as_mut().unwrap();
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
        let instance = guard.as_mut().unwrap();
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
        let instance = guard.as_ref().unwrap();
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
        let screenshot_path = std::env::temp_dir().join("ironclaw_browser_screenshot.png");

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
        let instance = guard.as_ref().unwrap();
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
        let instance = guard.as_ref().unwrap();
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
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to extract page content: {e}")))?;

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
    async fn close_session(&self) -> Result<serde_json::Value, ToolError> {
        let mut guard = self.instance.write().await;
        if let Some(mut instance) = guard.take() {
            let tab_count = instance.pages.len();
            for state in instance.pages.drain(..) {
                let _ = state.page.close().await;
            }
            // Drop the browser, which kills the Chrome process.
            drop(instance);
            tracing::info!("Browser session closed ({tab_count} tabs)");
            Ok(serde_json::json!({
                "status": "session_closed",
                "tabs_closed": tab_count,
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
        let instance = guard.as_ref().unwrap();

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
        let instance = guard.as_mut().unwrap();

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
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Browse the web: navigate pages, read content via accessibility snapshots, \
         click elements, type text, take screenshots, and evaluate JavaScript. \
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
                    "enum": ["navigate", "snapshot", "click", "type", "screenshot", "evaluate", "get_text", "close", "tabs", "switch_tab"],
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
                    "Unknown action: '{action}'. Use: navigate, snapshot, click, type, screenshot, evaluate, get_text, close, tabs, switch_tab"
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
}
