> ⛔ **ARCHIVED** — This is a historical migration guide from the OpenClaw→IronClaw rewrite (early 2026). It does NOT reflect the current codebase. See [`../CLAUDE.md`](../CLAUDE.md) for current documentation.

---

# Browser Automation Tool: Playwright/CDP → Rust

The browser tool is one of OpenClaw's most complex subsystems (101 files). It provides the AI agent with full web browsing capabilities — navigating pages, filling forms, clicking elements, taking screenshots, and reading page content via accessibility tree snapshots.

---

## 1. What OpenClaw Does Today (Node.js)

OpenClaw's browser stack is built on **Playwright** + **Chrome DevTools Protocol (CDP)**:

- **`pw-session.ts`:** Manages a persistent Playwright browser connection via CDP WebSocket. Connects to an existing Chrome instance, tracks pages by target ID, caches console messages, network requests, and page errors per tab.
- **`pw-ai.ts`:** Exports ~50 browser actions: `navigateViaPlaywright`, `clickViaPlaywright`, `fillFormViaPlaywright`, `snapshotAiViaPlaywright`, `takeScreenshotViaPlaywright`, `evaluateViaPlaywright`, etc.
- **`pw-tools-core.ts`:** Core implementations — screenshot capture, accessibility tree snapshots, element interaction, downloads, storage management, tracing.
- **`pw-role-snapshot.ts`:** Generates accessibility-tree role snapshots that the LLM uses to "understand" the page structure. Each interactive element gets a short ref ID (e.g., `ref="a1"`) so the LLM can say "click ref a1" without using CSS selectors.
- **`chrome.ts`:** Chrome process lifecycle — finding Chrome executables, managing user data directories, profile decoration.
- **`cdp.ts`:** Raw CDP session management for direct DevTools protocol commands.
- **`navigation-guard.ts`:** Prevents navigation to forbidden URLs (file://, chrome://, localhost sensitive ports).
- **`extension-relay.ts`:** Chrome extension relay for injecting bridge scripts into pages.
- **`server-context.ts`:** Server-side browser context management — hot-reloading profiles, remote tab operations.

### Key Insight: Accessibility Tree Snapshots

The agent does **not** read raw HTML. Instead, `snapshotAiViaPlaywright` extracts the **accessibility tree** — a structural representation of the page showing roles (button, link, textbox), names, and states. Each interactive element gets a numbered `ref` so the LLM can reference it:

```
[page] https://example.com
  [heading] "Welcome"
  [textbox ref="t1" name="Email"]
  [textbox ref="t2" name="Password"]
  [button ref="b1" name="Sign In"]
```

The LLM responds: `{ "tool": "click", "ref": "b1" }`.

---

## 2. Rust Strategy: `chromiumoxide` + Accessibility Tree

### Why Not `playwright-rust`?

The Rust Playwright bindings (`playwright-rust` crate) are **abandoned** (last update 2022). They require a running Playwright Node.js server process, which defeats the purpose of a pure-Rust rewrite.

### Recommended Stack

| Component | Crate | Purpose |
|---|---|---|
| **CDP Connection** | `chromiumoxide` | Connect to Chrome via DevTools Protocol over WebSocket |
| **Chrome Launch** | `chromiumoxide::Browser::launch()` | Launch/manage Chrome process with custom profile |
| **Accessibility Tree** | CDP `Accessibility.getFullAXTree` command | Get the full AX tree, replicate role snapshots |
| **Screenshots** | CDP `Page.captureScreenshot` command | Generated via CDP, returned as PNG/JPEG bytes |
| **Navigation** | CDP `Page.navigate` command | Navigate with URL guard |
| **Form Interactions** | CDP `DOM.focus` + `Input.dispatchKeyEvent` | Type, click, select |
| **Downloads** | CDP `Browser.setDownloadBehavior` | Track and retrieve downloaded files |

### Core Rust Struct

```rust
use chromiumoxide::{Browser, BrowserConfig, Page};
use std::collections::HashMap;

pub struct BrowserTool {
    browser: Browser,
    /// Active pages tracked by target ID
    pages: HashMap<String, PageState>,
    /// Navigation guard: blocked URL patterns
    nav_guard: NavigationGuard,
    /// Chrome user data directory for profile persistence
    user_data_dir: PathBuf,
}

pub struct PageState {
    page: Page,
    /// Cached accessibility tree refs from last snapshot
    role_refs: HashMap<String, AXNodeRef>,
    /// Rolling buffer of console messages (max 500)
    console: Vec<ConsoleMessage>,
    /// Rolling buffer of network requests (max 500)
    network: Vec<NetworkRequest>,
}

impl BrowserTool {
    pub async fn new(profile_dir: Option<PathBuf>) -> Result<Self> {
        let user_data_dir = profile_dir.unwrap_or_else(|| {
            dirs::data_dir().unwrap().join("thinclaw/browser-profile")
        });

        let config = BrowserConfig::builder()
            .chrome_executable(find_chrome_executable()?)
            .user_data_dir(&user_data_dir)
            .no_sandbox()           // Required for headless on Linux
            .window_size(1280, 720)
            .build()?;

        let (browser, mut handler) = Browser::launch(config).await?;
        tokio::spawn(async move { while handler.next().await.is_some() {} });

        Ok(Self {
            browser,
            pages: HashMap::new(),
            nav_guard: NavigationGuard::default(),
            user_data_dir,
        })
    }
}
```

### Accessibility Tree Extraction in Rust

The critical innovation to replicate is **role ref snapshots**. In Rust:

```rust
use chromiumoxide::cdp::browser_protocol::accessibility::{
    GetFullAXTreeParams, AXNode,
};

impl BrowserTool {
    /// Generate a text-based accessibility snapshot with numbered refs
    pub async fn snapshot_ai(&mut self, target_id: &str) -> Result<String> {
        let page = self.pages.get_mut(target_id)
            .ok_or(BrowserError::PageNotFound)?;

        let tree = page.page
            .execute(GetFullAXTreeParams::default())
            .await?;

        let mut output = String::new();
        let mut ref_counter = 0u32;
        let mut refs = HashMap::new();

        for node in &tree.nodes {
            let role = node.role.as_deref().unwrap_or("generic");
            let name = node.name.as_deref().unwrap_or("");

            // Only interactive elements get refs
            let ref_label = if is_interactive_role(role) {
                ref_counter += 1;
                let label = format!("ref=\"e{}\"", ref_counter);
                refs.insert(format!("e{}", ref_counter), AXNodeRef {
                    backend_node_id: node.backend_dom_node_id,
                    role: role.to_string(),
                    name: name.to_string(),
                });
                format!(" {}", label)
            } else {
                String::new()
            };

            let indent = "  ".repeat(node.depth.unwrap_or(0) as usize);
            writeln!(output, "{}[{}{}] \"{}\"", indent, role, ref_label, name)?;
        }

        page.role_refs = refs;
        Ok(output)
    }
}

fn is_interactive_role(role: &str) -> bool {
    matches!(role,
        "button" | "link" | "textbox" | "checkbox" | "radio" |
        "combobox" | "menuitem" | "tab" | "slider" | "switch" |
        "searchbox" | "spinbutton"
    )
}
```

---

## 3. Navigation Guard

The agent must be prevented from accessing dangerous URLs:

```rust
pub struct NavigationGuard {
    blocked_schemes: Vec<String>,    // ["file", "chrome", "chrome-extension"]
    blocked_ports: Vec<u16>,          // [22, 3000, 5432, 6379, 8080] etc.
    blocked_hosts: Vec<String>,       // ["localhost", "127.0.0.1", "metadata.google"]
}

impl NavigationGuard {
    pub fn is_allowed(&self, url: &str) -> Result<(), NavigationError> {
        let parsed = url::Url::parse(url)?;

        if self.blocked_schemes.contains(&parsed.scheme().to_string()) {
            return Err(NavigationError::BlockedScheme(parsed.scheme().into()));
        }

        if let Some(host) = parsed.host_str() {
            if self.blocked_hosts.iter().any(|h| host == h || host.ends_with(h)) {
                return Err(NavigationError::BlockedHost(host.into()));
            }
        }

        if let Some(port) = parsed.port() {
            if self.blocked_ports.contains(&port) {
                return Err(NavigationError::BlockedPort(port));
            }
        }

        Ok(())
    }
}
```

---

## 4. Browser Profile Persistence

Each agent has its own Chrome profile directory so cookies, localStorage, and extensions persist between sessions:

```
~/.thinclaw/browser-profiles/
  └── agent_main/
      ├── Default/
      │   ├── Cookies
      │   ├── Local Storage/
      │   └── Preferences
      └── DevToolsActivePort
```

In Remote Mode, the browser runs on the remote Orchestrator's machine. The Tauri Thin Client doesn't need Chrome.

---

## 5. RIG Tool Integration

Each browser action becomes a RIG tool:

```rust
pub struct BrowserNavigateTool { browser: Arc<Mutex<BrowserTool>> }
pub struct BrowserClickTool    { browser: Arc<Mutex<BrowserTool>> }
pub struct BrowserSnapshotTool { browser: Arc<Mutex<BrowserTool>> }
pub struct BrowserScreenshotTool { browser: Arc<Mutex<BrowserTool>> }
pub struct BrowserTypeTool     { browser: Arc<Mutex<BrowserTool>> }

// All share the same Arc<Mutex<BrowserTool>> instance
```

The LLM's tool loop looks like:
1. `snapshot` → receives accessibility tree text
2. LLM decides what to click/type → calls `click` or `type` with a ref
3. `snapshot` again → sees the result
4. Repeat until the task is done

---

## 6. Crate Dependencies

```toml
[dependencies]
chromiumoxide = { version = "0.7", features = ["tokio-runtime"] }
url = "2"
```

---

## 7. Security Considerations

- The browser runs as an **Orchestrator-controlled resource**, not a tool the LLM has direct access to. The LLM issues JSON commands; the Orchestrator's `BrowserTool` implementation executes them.
- Navigation guard runs **before** every `Page.navigate` call.
- The browser's proxy settings can be locked to prevent the LLM from using it to exfiltrate data.
- In Local Mode with the "airgap" strategy (see `SANDBOX_RS.md`), the browser tool is disabled when a cloud model is active.
