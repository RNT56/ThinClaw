use super::*;

impl BrowserTool {
    /// Navigate to a URL.
    ///
    /// Opens a new tab and sets it as the current page. When the tab count
    /// exceeds [`MAX_TABS`], the oldest tab is closed automatically.
    pub(super) async fn navigate(&self, url: &str) -> Result<serde_json::Value, ToolError> {
        is_url_allowed(url).map_err(ToolError::ExecutionFailed)?;
        is_network_url_allowed(url)
            .await
            .map_err(ToolError::ExecutionFailed)?;
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;

        let page = instance
            .browser
            .new_page("about:blank")
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Navigation failed: {e}")))?;
        let mut pending_page = PendingPageGuard::new(page.clone());
        if let Some(credentials) = instance.proxy_credentials.clone()
            && let Err(error) = page.authenticate(credentials).await
        {
            pending_page.disarm();
            let _ = page.close().await;
            return Err(ToolError::ExecutionFailed(format!(
                "Failed to configure browser proxy authentication: {error}"
            )));
        }
        let request_guard_handle = match Self::start_request_guard(&page).await {
            Ok(handle) => {
                pending_page.own_request_guard(&handle);
                handle
            }
            Err(error) => {
                pending_page.disarm();
                let _ = page.close().await;
                return Err(error);
            }
        };
        if let Err(error) = page.goto(url).await {
            pending_page.disarm();
            Self::close_page_state(PageState {
                page,
                request_guard_handle,
                role_refs: HashMap::new(),
            })
            .await;
            return Err(ToolError::ExecutionFailed(format!(
                "Navigation failed: {error}"
            )));
        }

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

        let page_url = bounded_browser_string(
            &page
                .url()
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| url.to_string()),
            MAX_BROWSER_URL_BYTES,
        );

        // Evict the oldest tab if we've hit the limit.
        if instance.pages.len() >= MAX_TABS {
            let evicted = instance.pages.remove(0);
            Self::close_page_state(evicted).await;
            tracing::debug!("Evicted oldest tab (limit={MAX_TABS})");
            // Adjust current_page index after removal.
            if let Some(ref mut cp) = instance.current_page {
                *cp = cp.saturating_sub(1);
            }
        }

        let idx = instance.pages.len();
        instance.pages.push(PageState {
            page,
            request_guard_handle,
            role_refs: HashMap::new(),
        });
        pending_page.disarm();
        instance.current_page = Some(idx);
        Self::touch_activity(instance);

        Ok(serde_json::json!({
            "status": "navigated",
            "url": page_url,
            "tab_index": idx,
            "open_tabs": instance.pages.len(),
        }))
    }

    /// Get the current page state mutably.
    pub(super) fn current_page_mut(
        instance: &mut BrowserInstance,
    ) -> Result<&mut PageState, ToolError> {
        let idx = instance
            .current_page
            .ok_or_else(|| ToolError::ExecutionFailed("No page open. Navigate first.".into()))?;

        instance
            .pages
            .get_mut(idx)
            .ok_or_else(|| ToolError::ExecutionFailed("Page not found".into()))
    }

    /// Generate an accessibility tree snapshot with numbered refs.
    pub(super) async fn snapshot(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;
        let state = Self::current_page_mut(instance)?;

        let page_url = bounded_browser_string(
            &state.page.url().await.ok().flatten().unwrap_or_default(),
            MAX_BROWSER_URL_BYTES,
        );

        // Get the full accessibility tree via CDP
        let tree_result = state
            .page
            .execute(GetFullAxTreeParams::default())
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to get accessibility tree: {e}"))
            })?;

        let nodes: &[AxNode] = &tree_result.result.nodes;

        let mut output = String::with_capacity(MAX_SNAPSHOT_BYTES.min(8 * 1024));
        let _ = writeln!(output, "[page] {page_url}");

        let mut ref_counter = 0u32;
        let mut refs = HashMap::new();
        let mut processed_nodes = 0usize;
        let mut truncated = nodes.len() > MAX_SNAPSHOT_NODES;

        for node in nodes.iter().take(MAX_SNAPSHOT_NODES) {
            processed_nodes += 1;
            let raw_role = node
                .role
                .as_ref()
                .and_then(|r| r.value.as_ref())
                .and_then(|v| v.as_str())
                .unwrap_or("generic");

            let raw_name = node
                .name
                .as_ref()
                .and_then(|n| n.value.as_ref())
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let (role, role_truncated) = truncate_chars(raw_role, MAX_AX_ROLE_CHARS);
            let (name, name_truncated) = truncate_chars(raw_name, MAX_AX_NAME_CHARS);
            truncated |= role_truncated || name_truncated;

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

            let is_interactive = is_interactive_role(&role);
            let ref_label = if is_interactive && refs.len() < MAX_SNAPSHOT_REFS {
                ref_counter += 1;
                let label = format!("e{ref_counter}");
                format!(" ref=\"{label}\"")
            } else {
                truncated |= is_interactive;
                String::new()
            };

            let indent = "  ";
            let line = if !name.is_empty() {
                format!("{indent}[{role}{ref_label}] \"{name}\"\n")
            } else {
                format!("{indent}[{role}{ref_label}]\n")
            };
            if output.len().saturating_add(line.len()) > MAX_SNAPSHOT_BYTES {
                truncated = true;
                break;
            }
            output.push_str(&line);
            if !ref_label.is_empty() {
                refs.insert(
                    format!("e{ref_counter}"),
                    AXNodeRef {
                        backend_node_id: node.backend_dom_node_id,
                        role,
                        name,
                    },
                );
            }
        }

        truncated |= processed_nodes < nodes.len();
        if truncated {
            let _ = push_bounded_text(
                &mut output,
                "[… accessibility snapshot truncated …]\n",
                MAX_SNAPSHOT_BYTES,
            );
        }

        state.role_refs = refs;
        Self::touch_activity(instance);

        Ok(serde_json::json!({
            "snapshot": output,
            "element_count": ref_counter,
            "url": page_url,
            "truncated": truncated,
        }))
    }

    /// Click an element by its ref ID.
    pub(super) async fn click_ref(&self, ref_id: &str) -> Result<serde_json::Value, ToolError> {
        validate_ref_id(ref_id)?;
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;
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
        Self::touch_activity(instance);

        Ok(serde_json::json!({
            "status": "clicked",
            "ref": ref_id,
            "role": node_ref.role,
            "name": node_ref.name,
        }))
    }

    /// Type text into an element by ref ID.
    pub(super) async fn type_text(
        &self,
        ref_id: &str,
        text: &str,
    ) -> Result<serde_json::Value, ToolError> {
        validate_ref_id(ref_id)?;
        if text.len() > MAX_TYPE_TEXT_BYTES {
            return Err(ToolError::InvalidParameters(format!(
                "Text to type exceeds the {MAX_TYPE_TEXT_BYTES}-byte browser limit"
            )));
        }
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;
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
        Self::touch_activity(instance);

        Ok(serde_json::json!({
            "status": "typed",
            "ref": ref_id,
            "text_length": text.len(),
        }))
    }

    /// Take a screenshot of the current page.
    pub(super) async fn screenshot(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;
        let state = Self::current_page_mut(instance)?;

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
        if screenshot_bytes.len() > MAX_SCREENSHOT_BYTES {
            return Err(ToolError::ExecutionFailed(format!(
                "Screenshot exceeded the {MAX_SCREENSHOT_BYTES}-byte browser limit"
            )));
        }
        if !screenshot_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(ToolError::ExecutionFailed(
                "Browser returned malformed PNG screenshot data".to_string(),
            ));
        }

        // A deterministic path allowed a local symlink to redirect screenshot
        // writes into an arbitrary file. Create a fresh file atomically, then
        // remove the previous capture owned by this browser instance.
        let screenshot_path = std::env::temp_dir().join(format!(
            "thinclaw-browser-{}.png",
            uuid::Uuid::new_v4().simple()
        ));
        // Declare the cleanup owner before the file handle so cancellation
        // drops the handle first (important for deletion on Windows).
        let owned_screenshot = OwnedScreenshotPath(screenshot_path.clone());
        let mut options = tokio::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        let mut file = options.open(&screenshot_path).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to create screenshot file: {e}"))
        })?;
        if let Err(error) = file.write_all(&screenshot_bytes).await {
            drop(file);
            return Err(ToolError::ExecutionFailed(format!(
                "Failed to save screenshot: {error}"
            )));
        }
        if let Err(error) = file.flush().await {
            drop(file);
            return Err(ToolError::ExecutionFailed(format!(
                "Failed to flush screenshot: {error}"
            )));
        }
        drop(file);
        drop(instance.screenshot_path.replace(owned_screenshot));
        Self::touch_activity(instance);

        Ok(serde_json::json!({
            "status": "screenshot_taken",
            "path": screenshot_path.to_string_lossy(),
            "size_bytes": screenshot_bytes.len(),
        }))
    }

    /// Evaluate JavaScript on the current page.
    pub(super) async fn evaluate(&self, expression: &str) -> Result<serde_json::Value, ToolError> {
        if expression.is_empty() || expression.len() > MAX_EVALUATE_EXPRESSION_BYTES {
            return Err(ToolError::InvalidParameters(format!(
                "JavaScript expression must be non-empty and at most {MAX_EVALUATE_EXPRESSION_BYTES} bytes"
            )));
        }
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;
        let state = Self::current_page_mut(instance)?;

        let eval_result = state
            .page
            .evaluate_expression(expression)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("JS evaluation failed: {e}")))?;

        let value = eval_result
            .into_value::<serde_json::Value>()
            .unwrap_or(serde_json::Value::Null);
        let (value, truncated) = bound_json_value(value);
        Self::touch_activity(instance);

        Ok(serde_json::json!({
            "result": value,
            "truncated": truncated,
        }))
    }

    /// Get the page's content as clean Markdown (preserves structure for LLM comprehension).
    pub(super) async fn get_text(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;
        let state = Self::current_page_mut(instance)?;

        let page_url = bounded_browser_string(
            &state.page.url().await.ok().flatten().unwrap_or_default(),
            MAX_BROWSER_URL_BYTES,
        );

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
const text=w(m,0).replace(/\n{3,}/g,'\n\n').trim();
return {content:text.slice(0,16000),total_chars:text.length,truncated:text.length>16000};
})()"#;

        let eval_result = state
            .page
            .evaluate_expression(extractor_js)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to extract page content: {e}"))
            })?;

        let extracted = eval_result
            .into_value::<serde_json::Value>()
            .unwrap_or_else(|_| serde_json::json!({}));
        let text = extracted
            .get("content")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let total_chars = extracted
            .get("total_chars")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| text.chars().count() as u64);
        let page_truncated = extracted
            .get("truncated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let (safe, rust_truncated) = truncate_chars(text, TEXT_CHAR_LIMIT);
        let was_truncated = page_truncated || rust_truncated;
        let content = if was_truncated {
            format!(
                "{safe}\n\n[… truncated — approximately {total_chars} chars total. Use snapshot + scroll for more.]\n"
            )
        } else {
            safe
        };
        Self::touch_activity(instance);

        Ok(serde_json::json!({
            "url": page_url,
            "content": content,
            "format": "markdown",
            "length": content.len(),
            "total_chars": total_chars,
            "truncated": was_truncated,
        }))
    }

    /// Close all open tabs and release the browser instance, reclaiming
    /// resources. The next `execute` call will re-launch Chrome as needed.
    ///
    /// If the browser was running in a Docker container, the container is
    /// stopped and removed.
    pub(super) async fn close_session(&self) -> Result<serde_json::Value, ToolError> {
        let mut guard = self.instance.write().await;
        if let Some(instance) = guard.take() {
            let tab_count = instance.pages.len();
            let was_docker = instance.is_docker;
            let cloud_provider = instance
                .cloud_session
                .as_ref()
                .map(|session| format!("{:?}", session.provider).to_ascii_lowercase());
            self.shutdown_browser_instance(instance).await;
            tracing::info!(
                tabs = tab_count,
                docker = was_docker,
                ?cloud_provider,
                "Browser session closed"
            );
            Ok(serde_json::json!({
                "status": "session_closed",
                "tabs_closed": tab_count,
                "was_docker": was_docker,
                "cloud_provider": cloud_provider,
            }))
        } else {
            if let Some(runtime) = self.docker_config.as_ref() {
                let _ = runtime.stop().await;
            }
            if let Some(runtime) = self.egress_runtime.as_ref() {
                let _ = runtime.stop().await;
            }
            Ok(serde_json::json!({
                "status": "no_session",
                "message": "No browser session was active",
            }))
        }
    }

    /// Release any local, Docker, or managed browser session owned by this
    /// tool. This is used by the registry's runtime shutdown path even when an
    /// agent never issued the explicit `close` action.
    pub async fn shutdown(&self) {
        let _operation_guard = self.operation_lock.lock().await;
        let instance = self.instance.write().await.take();
        let stopped_docker_instance = instance.as_ref().is_some_and(|instance| instance.is_docker);
        let stopped_local_egress = instance
            .as_ref()
            .is_some_and(|instance| instance.uses_local_egress);
        if let Some(instance) = instance {
            self.shutdown_browser_instance(instance).await;
        }
        // A cancelled or failed Docker start can own proxy/relay/container
        // resources before a BrowserInstance is published. Ask the runtime to
        // clean that partial state during registry shutdown as well.
        if !stopped_docker_instance
            && let Some(runtime) = self.docker_config.as_ref()
            && let Err(error) = runtime.stop().await
        {
            tracing::warn!(%error, "Failed to clean partial Docker browser state");
        }
        if !stopped_local_egress
            && let Some(runtime) = self.egress_runtime.as_ref()
            && let Err(error) = runtime.stop().await
        {
            tracing::warn!(%error, "Failed to clean partial local browser proxy state");
        }
        *self
            .active_scope
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}
