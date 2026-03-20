# Implementation Plan: Integrating Chromium Oxide for Enhanced Web Scraping in Scrappy

This plan defines how to implement the `chromiumoxide` Rust crate (version 0.7 or latest) into your Scrappy app (Tauri/Rust backend, React frontend, llama.cpp sidecar, and upcoming Rig integration). The purpose is to upgrade your web search capabilities—specifically the Page Scraper Tool—to handle JavaScript-rendered content, addressing the limitation of only getting titles/snippets from static fetches. This enables full DOM extraction after JS execution (e.g., for SPAs like React apps or lazy-loaded sites), bringing it closer to Playwright's functionality but in pure Rust.

`chromiumoxide` uses the Chrome DevTools Protocol for headless browsing, navigation, element interaction, and content retrieval. It's async, lightweight, and fits your API-free approach—no external services needed, just a local Chrome binary.

**Assumptions**: You have Chrome/Chromium installed (required for the crate). If not, the plan includes setup. Build on your existing TODO plan (add to Phase 3: Tool Building). Estimated effort: 2-4 days for integration and testing.

## Phase 1: Preparation (1 Day)
- **Add Dependencies**:
  - Update `src-tauri/Cargo.toml`: Add `chromiumoxide = "0.7"` (check crates.io for latest; it's actively maintained as of 2026).
  - If needed for advanced parsing: Ensure `scraper` is already added (from previous plan).
  - Run `cargo build` to verify. Note: Requires Rust 1.70+ for async stability.
- **Install Browser Binary**:
  - Ensure Chrome/Chromium is installed on your dev machine and target OS (e.g., via apt on Linux: `sudo apt install chromium-browser`; or download from google.com/chrome).
  - For Tauri distribution: Bundle or detect Chrome at runtime (e.g., use `which chromium` in code). If absent, prompt user to install (via Tauri dialog).
- **Review Ethical/Legal**:
  - Update app docs: Emphasize personal use only; respect robots.txt (fetch and parse before browsing).
  - Add config flag for headless mode (default: true) to avoid visible windows.
- **Milestone**: Crate compiles; manually test a standalone browser launch in a test file (copy from GitHub examples).

## Phase 2: Core Implementation in Rig Tool (1-2 Days)
- **Enhance Page Scraper Tool**:
  - In your Rig tool module (e.g., `tools.rs`), modify the Page Scraper to optionally use chromiumoxide for JS rendering.
  - Code Structure (async fn with fallback to static reqwest):
    ```rust
    use chromiumoxide::{Browser, BrowserConfig, Page};
    use std::error::Error;
    use scraper::{Html, Selector};  // For parsing rendered HTML

    #[tool_macro]
    async fn scrape_page(url: String, use_browser: bool) -> Result<String, Box<dyn Error>> {
        if !use_browser {
            // Fallback: Static fetch with reqwest (existing code)
            let response = reqwest::get(&url).await?.text().await?;
            // Parse and extract (e.g., body text)
            let document = Html::parse_document(&response);
            let selector = Selector::parse("body").unwrap();
            return Ok(document.select(&selector).next().unwrap().inner_html());
        } else {
            // Chromiumoxide: Launch headless browser
            let config = BrowserConfig::builder().with_head(false).build()?;
            let (browser, mut handler) = Browser::launch(config).await?;
            
            // Spawn handler task (required for event polling)
            let handle = tokio::spawn(async move {
                while let Some(_) = handler.next().await {}
            });
            
            // Create page and navigate
            let page: Page = browser.new_page(&url).await?;
            page.wait_for_navigation().await?;  // Wait for JS load
            
            // Get rendered content
            let content = page.content().await?;
            
            // Parse if needed (e.g., extract specific elements)
            let document = Html::parse_document(&content);
            let selector = Selector::parse("article, main, body").unwrap();  // Target content areas
            let extracted = document.select(&selector).map(|el| el.inner_html()).collect::<Vec<_>>().join("\n");
            
            // Cleanup
            browser.close().await?;
            handle.await?;
            
            Ok(extracted)
        }
    }
    ```
  - Inputs: Add `use_browser: bool` (default: false) to tool schema for agent control (e.g., LLM decides based on query or initial static fail).
- **Integrate with Rig Agent**:
  - Attach to agent: `agent.tool(scrape_page)`.
  - In agent preamble: Instruct LLM to use browser mode for "dynamic sites" or if static yields empty content.
- **Error Handling**:
  - Wrap in `anyhow` or `thiserror`: Handle timeouts (e.g., 10s via `tokio::time::timeout`), navigation fails, or no Chrome (fallback to static with warning).
- **Milestone**: Test tool standalone: Scrape a JS site (e.g., https://react.dev) vs. static; verify full content extraction.

## Phase 3: Tauri/Frontend Integration (1 Day)
- **Expose in Tauri**:
  - If not using Rig yet, add a command: `#[command] async fn scrape_url(url: String, use_browser: bool) -> Result<String, String> { scrape_page(url, use_browser).await.map_err(|e| e.to_string()) }`.
  - From React: Call via `invoke('scrape_url', { url, use_browser: true })` in your search UI.
- **UI Enhancements**:
  - Add toggle/checkbox for "Deep Scrape (JS)" in frontend form.
  - Show progress: Use Tauri events to emit "scraping..." updates during browser ops.
- **Hybrid with Existing Tools**:
  - In Web Search Tool: After fetching links, chain to Scrape Page with browser if needed.
  - For deep research: Agent iterates (e.g., search → evaluate if JS needed → scrape with browser).
- **Milestone**: End-to-end: Frontend query triggers browser scrape; displays full content.

## Phase 4: Testing and Optimization (Ongoing)
- **Testing**:
  - Unit: Mock with static HTML; use GitHub examples for browser sim.
  - E2E: Test on JS-heavy sites (e.g., single-page apps, infinite scroll—add page.scroll_down() if needed).
  - Edge: No Chrome (error gracefully), timeouts, anti-bot (add stealth: e.g., randomize user-agent via config).
- **Performance**:
  - Headless mode: Reduces overhead; aim for 5-10s per page.
  - Reuse Browser: For multi-scrapes, launch once per session (store in app state).
  - Resource Use: Monitor CPU/RAM; limit concurrent tabs (e.g., one page at a time).
- **Deployment**:
  - Tauri Build: Ensure Chrome dependency is documented in app installer/README.
  - Cross-OS: Test on Windows/Mac/Linux (Chromium paths vary; use env vars).
- **Alternatives if Issues**:
  - If flaky (as noted in Reddit), fallback to `thirtyfour` (Selenium-like, but heavier) or `fantoccini` (another DevTools crate).
- **Milestone**: Stable scraping of dynamic content; integrate into full search flow.

This plan is feasible on local hardware (mid-range CPU/GPU suffices; ~100-500MB RAM per browser instance). Start with the code sketch—it's based on official examples. If Chrome setup blocks, use a managed binary downloader (e.g., via another crate like `chrome_driver_rs`). Let me know for code tweaks.