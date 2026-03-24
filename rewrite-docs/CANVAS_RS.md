> ⛔ **ARCHIVED** — This is a historical migration guide from the OpenClaw→IronClaw rewrite (early 2026). It does NOT reflect the current codebase. See [`../CLAUDE.md`](../CLAUDE.md) for current documentation.

---

# Canvas & a2UI: Agent-Generated Interactive UIs

Canvas is OpenClaw's system for allowing the agent to create, serve, and interact with **live web UIs** inside the chat interface. The agent writes HTML/JS, the Orchestrator serves it via a local HTTP server, and the user sees a live interactive widget embedded in the conversation.

---

## 1. What OpenClaw Does Today

### Canvas Host (`src/canvas-host/`)

A local HTTP server that serves agent-generated HTML/CSS/JS files:

- **`server.ts`:** Starts an HTTP server on a configurable port. Serves static files from a `canvas/` directory (default: `~/.openclaw/canvas/`). Supports WebSocket for live reload — when the agent writes new files, the UI auto-refreshes.
- **Default `index.html`:** If no canvas files exist, a default landing page is generated.
- **File Watcher:** Uses `fs.watch` to detect file changes and broadcast `reload` via WebSocket.
- **Security:** Files are resolved within the root directory only (`resolveFileWithinRoot`) — path traversal is blocked.

### a2UI (Agent-to-UI)

a2UI is the bridge that allows the agent's generated UI to **communicate back to the agent**:

- **`a2ui.ts`:** Serves the a2UI bundle (pre-built JS library) at `/__openclaw__/a2ui/`.
- **`injectCanvasLiveReload()`:** Before serving any HTML file, the system injects a `<script>` block that provides:
  - `OpenClaw.postMessage(payload)` — Send a JSON message from the UI to the agent
  - `OpenClaw.sendUserAction(action)` — Send a structured user interaction event
  - WebSocket connection for live reload
  - Cross-platform bridges: iOS (`webkit.messageHandlers`), Android (`window.openclawCanvasA2UIAction`)

### Canvas Capability in the Gateway

The gateway exposes canvas capabilities to the agent:
- **`canvas-capability.ts`:** Registers `canvas_create`, `canvas_update` as tools
- Agent writes HTML → Orchestrator saves to `canvas/` dir → Canvas Host serves it → UI renders it

---

## 2. Example Flow

```
User: "Create a task tracker for my project"

Agent (tool call):
{
  "tool": "canvas_create",
  "params": {
    "filename": "tracker.html",
    "content": "<html>...<script>
      document.getElementById('addBtn').onclick = () => {
        const task = document.getElementById('taskInput').value;
        OpenClaw.sendUserAction({ type: 'add_task', task: task });
      };
    </script></html>"
  }
}

→ Orchestrator saves tracker.html to ~/.thinclaw/canvas/
→ Canvas Host serves it at http://localhost:PORT/tracker.html
→ Tauri WebView or chat embed loads the URL
→ User clicks "Add Task" button in the embedded UI
→ OpenClaw.sendUserAction({ type: 'add_task', task: 'Fix bug #7' })
→ Message is delivered back to the agent as a user action event
→ Agent processes the action and updates state
```

---

## 3. Rust Implementation

### Canvas Host Server

```rust
use axum::{
    Router, routing::get,
    extract::{State, ws::{WebSocket, WebSocketUpgrade}},
    response::IntoResponse,
};
use notify::{Watcher, RecommendedWatcher, RecursiveMode};
use tokio::sync::broadcast;
use std::path::PathBuf;

pub struct CanvasHost {
    root_dir: PathBuf,
    port: u16,
    /// Broadcast channel for live reload notifications
    reload_tx: broadcast::Sender<()>,
}

impl CanvasHost {
    pub fn new(root_dir: PathBuf, port: u16) -> Self {
        let (reload_tx, _) = broadcast::channel(16);
        Self { root_dir, port, reload_tx }
    }

    pub async fn start(&self) -> Result<()> {
        // Ensure canvas directory exists
        tokio::fs::create_dir_all(&self.root_dir).await?;

        // Write default index.html if empty
        let index = self.root_dir.join("index.html");
        if !index.exists() {
            tokio::fs::write(&index, DEFAULT_CANVAS_HTML).await?;
        }

        // Start file watcher for live reload
        let reload_tx = self.reload_tx.clone();
        let watch_dir = self.root_dir.clone();
        tokio::spawn(async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel(32);
            let mut watcher = RecommendedWatcher::new(
                move |_| { let _ = tx.blocking_send(()); },
                notify::Config::default(),
            ).unwrap();
            watcher.watch(&watch_dir, RecursiveMode::Recursive).unwrap();

            while rx.recv().await.is_some() {
                let _ = reload_tx.send(());
            }
        });

        // Build HTTP router
        let state = Arc::new(self.clone());
        let app = Router::new()
            .route("/__thinclaw__/ws", get(Self::ws_handler))
            .route("/__thinclaw__/a2ui/*path", get(Self::a2ui_handler))
            .fallback(get(Self::serve_canvas_file))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind(
            format!("127.0.0.1:{}", self.port)
        ).await?;
        axum::serve(listener, app).await?;
        Ok(())
    }

    /// Serve canvas files with a2UI script injection
    async fn serve_canvas_file(
        State(host): State<Arc<CanvasHost>>,
        req: axum::extract::Request,
    ) -> impl IntoResponse {
        let path = req.uri().path().trim_start_matches('/');
        let file_path = host.root_dir.join(path);

        // Path traversal guard
        let canonical = file_path.canonicalize()?;
        if !canonical.starts_with(&host.root_dir) {
            return StatusCode::FORBIDDEN.into_response();
        }

        let content = tokio::fs::read_to_string(&canonical).await?;

        // Inject a2UI bridge script into HTML files
        if canonical.extension().map_or(false, |e| e == "html") {
            let injected = inject_a2ui_bridge(&content);
            return (
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                injected,
            ).into_response();
        }

        content.into_response()
    }

    /// WebSocket handler for live reload
    async fn ws_handler(
        State(host): State<Arc<CanvasHost>>,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        ws.on_upgrade(move |socket| async move {
            let mut rx = host.reload_tx.subscribe();
            let (mut sender, _) = socket.split();
            while rx.recv().await.is_ok() {
                let _ = sender.send(axum::extract::ws::Message::Text("reload".into())).await;
            }
        })
    }
}
```

### a2UI Bridge Script Injection

```rust
fn inject_a2ui_bridge(html: &str) -> String {
    let bridge_script = r#"
<script>
(() => {
  // ThinClaw Canvas Bridge
  function postToOrchestrator(payload) {
    const raw = typeof payload === 'string' ? payload : JSON.stringify(payload);

    // Tauri WebView bridge
    if (window.__TAURI__) {
      window.__TAURI__.event.emit('canvas-action', raw);
      return true;
    }

    // iOS WKWebView bridge
    if (window.webkit?.messageHandlers?.thinclawCanvas) {
      window.webkit.messageHandlers.thinclawCanvas.postMessage(raw);
      return true;
    }

    // Fallback: POST to orchestrator endpoint
    fetch('/__thinclaw__/action', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: raw,
    }).catch(() => {});
    return true;
  }

  function sendUserAction(action) {
    const id = action?.id || crypto.randomUUID();
    return postToOrchestrator({ userAction: { ...action, id } });
  }

  globalThis.ThinClaw = globalThis.ThinClaw ?? {};
  globalThis.ThinClaw.postMessage = postToOrchestrator;
  globalThis.ThinClaw.sendUserAction = sendUserAction;

  // Live reload WebSocket
  try {
    const ws = new WebSocket(
      `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/__thinclaw__/ws`
    );
    ws.onmessage = (ev) => { if (ev.data === 'reload') location.reload(); };
  } catch {}
})();
</script>"#;

    // Inject before </body> if present, otherwise append
    if let Some(idx) = html.to_lowercase().rfind("</body>") {
        format!("{}\n{}\n{}", &html[..idx], bridge_script, &html[idx..])
    } else {
        format!("{}\n{}", html, bridge_script)
    }
}
```

---

## 4. Canvas Tools for the Agent

The agent interacts with Canvas via tool calls:

```rust
pub struct CanvasCreateTool {
    canvas_dir: PathBuf,
}

#[async_trait]
impl Tool for CanvasCreateTool {
    const NAME: &'static str = "canvas_create";

    async fn call(&self, params: CanvasCreateParams) -> Result<String> {
        let target = self.canvas_dir.join(&params.filename);

        // Security: prevent path traversal
        if !target.starts_with(&self.canvas_dir) {
            return Err(ToolError::PathTraversal);
        }

        tokio::fs::write(&target, &params.content).await?;

        Ok(format!(
            "Canvas created: http://localhost:{}/{}",
            CANVAS_PORT, params.filename
        ))
    }
}

pub struct CanvasUpdateTool { canvas_dir: PathBuf }
pub struct CanvasDeleteTool { canvas_dir: PathBuf }
pub struct CanvasListTool   { canvas_dir: PathBuf }
```

---

## 5. User Action Flow (a2UI → Agent)

When the user interacts with a canvas UI (clicks a button, submits a form), the a2UI bridge sends a `userAction` event:

```json
{ "userAction": { "id": "uuid", "type": "add_task", "task": "Fix bug #7" } }
```

This is received by:
- **Tauri (Local Mode):** The `__TAURI__.event.emit('canvas-action', payload)` call is caught by a Tauri event listener in the Rust backend, which injects it into the agent's message queue as a user message.
- **Remote Mode:** The HTTP POST to `/__thinclaw__/action` is handled by the Canvas Host's action endpoint, which forwards it to the Orchestrator's agent session.

The agent then receives: `"[Canvas Action] User clicked 'Add Task' with data: { task: 'Fix bug #7' }"` and can respond or update the canvas.

---

## 6. Tauri Integration

In Local Mode, the canvas is rendered inside a **Tauri WebView window**:

```rust
// In the Tauri app, open a canvas window
tauri::WebviewWindowBuilder::new(
    app,
    "canvas",
    tauri::WebviewUrl::External(
        format!("http://localhost:{}/tracker.html", CANVAS_PORT).parse().unwrap()
    ),
)
.title("ThinClaw Canvas")
.inner_size(800.0, 600.0)
.build()?;
```

In Remote Mode, the canvas URL is sent to the user in the chat, and they open it in their browser.

---

## 7. Security Boundaries

| Rule | Detail |
|---|---|
| **Path traversal blocked** | `canonicalize()` + `starts_with(root_dir)` check |
| **Localhost only** | Canvas Host binds to `127.0.0.1`, not `0.0.0.0` |
| **No external requests** | Canvas pages served with CSP `default-src 'self'` |
| **a2UI bridge is read-only** | The bridge can only *send* events to the Orchestrator, not read agent state |
| **Canvas file size limit** | Max 1MB per canvas file to prevent abuse |
