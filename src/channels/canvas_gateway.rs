//! Canvas gateway — HTTP routes for serving agent-generated UI panels.
//!
//! Mounted on the unified webhook server at `/canvas/`. Provides:
//! - `GET /canvas/:panel_id` — Render a canvas panel as HTML
//! - `GET /canvas/api/panels` — List active panels (JSON)
//! - `GET /canvas/api/panels/:panel_id` — Get panel data (JSON)
//! - `POST /canvas/api/panels/:panel_id/action` — Handle form/button callbacks
//!
//! Panels are stored in memory with TTL expiry. The `CanvasTool` pushes
//! panels here; clients (Scrappy, browser, Telegram webapps) consume them.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Default TTL for canvas panels (15 minutes).
const DEFAULT_PANEL_TTL: Duration = Duration::from_secs(15 * 60);

/// Maximum number of active panels before LRU eviction.
const MAX_ACTIVE_PANELS: usize = 64;

/// A stored canvas panel.
#[derive(Debug, Clone)]
pub struct StoredPanel {
    /// Unique panel ID.
    pub panel_id: String,
    /// Panel title.
    pub title: String,
    /// UI component tree (serialized JSON from CanvasTool).
    pub components: serde_json::Value,
    /// When this panel was created/last updated.
    pub updated_at: Instant,
    /// Panel metadata (custom CSS, width hints, etc.).
    pub metadata: Option<serde_json::Value>,
}

impl Serialize for StoredPanel {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("StoredPanel", 5)?;
        s.serialize_field("panel_id", &self.panel_id)?;
        s.serialize_field("title", &self.title)?;
        s.serialize_field("components", &self.components)?;
        s.serialize_field("age_secs", &self.updated_at.elapsed().as_secs())?;
        s.serialize_field("metadata", &self.metadata)?;
        s.end()
    }
}

impl StoredPanel {
    /// Check if this panel has expired.
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.updated_at.elapsed() > ttl
    }
}

/// A queued action from a client callback.
#[derive(Debug, Clone, Serialize)]
pub struct QueuedAction {
    /// Which panel the action came from.
    pub panel_id: String,
    /// The action identifier (button name, form submit, etc.).
    pub action: String,
    /// Form field values (for form submissions).
    pub values: HashMap<String, serde_json::Value>,
}

/// In-memory store for active canvas panels.
#[derive(Debug, Clone)]
pub struct CanvasStore {
    panels: Arc<RwLock<HashMap<String, StoredPanel>>>,
    ttl: Duration,
    /// Channel for queuing action callbacks from clients.
    action_tx: tokio::sync::mpsc::Sender<QueuedAction>,
    action_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<QueuedAction>>>,
}

impl Default for CanvasStore {
    fn default() -> Self {
        Self::new(DEFAULT_PANEL_TTL)
    }
}

impl CanvasStore {
    /// Create a new canvas store with a custom TTL.
    pub fn new(ttl: Duration) -> Self {
        let (action_tx, action_rx) = tokio::sync::mpsc::channel(256);
        Self {
            panels: Arc::new(RwLock::new(HashMap::new())),
            ttl,
            action_tx,
            action_rx: Arc::new(tokio::sync::Mutex::new(action_rx)),
        }
    }

    /// Insert or update a panel.
    pub async fn upsert(
        &self,
        panel_id: String,
        title: String,
        components: serde_json::Value,
        metadata: Option<serde_json::Value>,
    ) {
        let mut panels = self.panels.write().await;

        // Evict expired panels first
        panels.retain(|_, p| !p.is_expired(self.ttl));

        // LRU eviction if at capacity
        if panels.len() >= MAX_ACTIVE_PANELS
            && !panels.contains_key(&panel_id)
            && let Some(oldest_key) = panels
                .iter()
                .min_by_key(|(_, p)| p.updated_at)
                .map(|(k, _)| k.clone())
        {
            panels.remove(&oldest_key);
        }

        panels.insert(
            panel_id.clone(),
            StoredPanel {
                panel_id,
                title,
                components,
                updated_at: Instant::now(),
                metadata,
            },
        );
    }

    /// Dismiss (remove) a panel.
    pub async fn dismiss(&self, panel_id: &str) -> bool {
        self.panels.write().await.remove(panel_id).is_some()
    }

    /// Get a panel by ID.
    pub async fn get(&self, panel_id: &str) -> Option<StoredPanel> {
        let panels = self.panels.read().await;
        panels
            .get(panel_id)
            .filter(|p| !p.is_expired(self.ttl))
            .cloned()
    }

    /// List all active (non-expired) panels.
    pub async fn list(&self) -> Vec<StoredPanel> {
        let panels = self.panels.read().await;
        panels
            .values()
            .filter(|p| !p.is_expired(self.ttl))
            .cloned()
            .collect()
    }

    /// Prune expired panels. Returns the count of pruned panels.
    pub async fn prune(&self) -> usize {
        let mut panels = self.panels.write().await;
        let before = panels.len();
        panels.retain(|_, p| !p.is_expired(self.ttl));
        before - panels.len()
    }

    /// Queue an action callback from a client (called by the HTTP handler).
    pub async fn push_action(&self, action: QueuedAction) -> bool {
        self.action_tx.send(action).await.is_ok()
    }

    /// Drain all pending action callbacks (called by the agent loop).
    ///
    /// Returns a vector of queued actions. Non-blocking: returns an empty
    /// vector if no actions are pending.
    pub async fn drain_actions(&self) -> Vec<QueuedAction> {
        let mut rx = self.action_rx.lock().await;
        let mut actions = Vec::new();
        while let Ok(action) = rx.try_recv() {
            actions.push(action);
        }
        actions
    }
}

/// Form/button action callback from the client.
#[derive(Debug, Deserialize)]
pub struct ActionPayload {
    /// The action identifier (button name, form submit, etc.).
    pub action: String,
    /// Form field values (for form submissions).
    #[serde(default)]
    pub values: HashMap<String, serde_json::Value>,
}

/// Build an axum Router for the canvas gateway.
pub fn canvas_routes(store: CanvasStore) -> Router {
    Router::new()
        .route("/canvas/api/panels", get(list_panels))
        .route("/canvas/api/panels/{panel_id}", get(get_panel_api))
        .route("/canvas/api/panels/{panel_id}/action", post(handle_action))
        .route("/canvas/{panel_id}", get(render_panel))
        .with_state(store)
}

// ── Route handlers ────────────────────────────────────────────

/// GET /canvas/api/panels — List all active panels.
async fn list_panels(State(store): State<CanvasStore>) -> Json<Vec<PanelSummary>> {
    let panels = store.list().await;
    let summaries: Vec<PanelSummary> = panels
        .into_iter()
        .map(|p| PanelSummary {
            panel_id: p.panel_id,
            title: p.title,
        })
        .collect();
    Json(summaries)
}

#[derive(Serialize)]
struct PanelSummary {
    panel_id: String,
    title: String,
}

/// GET /canvas/api/panels/:panel_id — Get panel data as JSON.
async fn get_panel_api(
    State(store): State<CanvasStore>,
    Path(panel_id): Path<String>,
) -> Result<Json<StoredPanel>, StatusCode> {
    store
        .get(&panel_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// POST /canvas/api/panels/:panel_id/action — Handle a client action callback.
async fn handle_action(
    State(store): State<CanvasStore>,
    Path(panel_id): Path<String>,
    Json(payload): Json<ActionPayload>,
) -> impl IntoResponse {
    // Verify the panel exists
    if store.get(&panel_id).await.is_none() {
        return (StatusCode::NOT_FOUND, "Panel not found").into_response();
    }

    tracing::info!(
        panel_id = %panel_id,
        action = %payload.action,
        "Canvas action callback received"
    );

    // Queue the action for the agent loop to process on its next turn.
    let queued = QueuedAction {
        panel_id,
        action: payload.action,
        values: payload.values,
    };
    if store.push_action(queued).await {
        (StatusCode::ACCEPTED, "Action queued").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "Action queue full").into_response()
    }
}

/// GET /canvas/:panel_id — Render a panel as a standalone HTML page.
async fn render_panel(
    State(store): State<CanvasStore>,
    Path(panel_id): Path<String>,
) -> Result<Html<String>, StatusCode> {
    let panel = store.get(&panel_id).await.ok_or(StatusCode::NOT_FOUND)?;

    let components_json =
        serde_json::to_string(&panel.components).unwrap_or_else(|_| "[]".to_string());

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>{title} — ThinClaw Canvas</title>
    <style>
        :root {{
            --bg: #0f0f14; --surface: #1a1a24; --border: #2a2a3a;
            --text: #e4e4ef; --text-muted: #8888a0; --accent: #7c6df0;
            --accent-hover: #9b8ff5; --danger: #e05252; --success: #4caf50;
            --radius: 10px; --font: 'Inter', -apple-system, sans-serif;
        }}
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{
            font-family: var(--font); background: var(--bg); color: var(--text);
            min-height: 100vh; padding: 2rem;
        }}
        .panel {{
            max-width: 720px; margin: 0 auto; background: var(--surface);
            border: 1px solid var(--border); border-radius: var(--radius);
            padding: 1.5rem; box-shadow: 0 8px 32px rgba(0,0,0,0.3);
        }}
        .panel-title {{
            font-size: 1.25rem; font-weight: 600; margin-bottom: 1rem;
            padding-bottom: 0.75rem; border-bottom: 1px solid var(--border);
        }}
        .component {{ margin-bottom: 1rem; }}
        .component:last-child {{ margin-bottom: 0; }}
        table {{ width: 100%; border-collapse: collapse; }}
        th, td {{ padding: 0.5rem 0.75rem; text-align: left; border-bottom: 1px solid var(--border); }}
        th {{ color: var(--text-muted); font-weight: 500; font-size: 0.85rem; text-transform: uppercase; }}
        .kv-list {{ display: grid; grid-template-columns: auto 1fr; gap: 0.25rem 1rem; }}
        .kv-key {{ color: var(--text-muted); }}
        .btn {{
            display: inline-block; padding: 0.5rem 1rem; border-radius: 6px;
            border: none; cursor: pointer; font-size: 0.9rem; font-weight: 500;
            background: var(--accent); color: white; transition: background 0.15s;
        }}
        .btn:hover {{ background: var(--accent-hover); }}
        .btn-danger {{ background: var(--danger); }}
        .btn-secondary {{ background: var(--border); color: var(--text); }}
        pre {{ background: var(--bg); padding: 1rem; border-radius: 6px; overflow-x: auto; font-size: 0.85rem; }}
        input, select, textarea {{
            width: 100%; padding: 0.5rem 0.75rem; border-radius: 6px;
            border: 1px solid var(--border); background: var(--bg); color: var(--text);
            font-family: var(--font); font-size: 0.9rem;
        }}
        label {{ display: block; margin-bottom: 0.25rem; font-size: 0.85rem; color: var(--text-muted); }}
        .form-group {{ margin-bottom: 0.75rem; }}
    </style>
</head>
<body>
    <div class="panel">
        <div class="panel-title">{title}</div>
        <div id="components"></div>
    </div>
    <script>
        const components = {components_json};
        const container = document.getElementById('components');
        const panelId = '{panel_id}';

        function renderComponent(comp) {{
            const div = document.createElement('div');
            div.className = 'component';
            switch (comp.type) {{
                case 'text':
                    div.innerHTML = comp.content || '';
                    break;
                case 'heading':
                    const h = document.createElement('h' + (comp.level || 2));
                    h.textContent = comp.text || '';
                    div.appendChild(h);
                    break;
                case 'table':
                    const table = document.createElement('table');
                    if (comp.headers) {{
                        const thead = table.createTHead();
                        const tr = thead.insertRow();
                        comp.headers.forEach(h => {{ const th = document.createElement('th'); th.textContent = h; tr.appendChild(th); }});
                    }}
                    const tbody = table.createTBody();
                    (comp.rows || []).forEach(row => {{
                        const tr = tbody.insertRow();
                        row.forEach(cell => {{ const td = tr.insertCell(); td.textContent = cell; }});
                    }});
                    div.appendChild(table);
                    break;
                case 'key_value':
                    const kvDiv = document.createElement('div');
                    kvDiv.className = 'kv-list';
                    (comp.items || []).forEach(item => {{
                        const k = document.createElement('span'); k.className = 'kv-key'; k.textContent = item.key;
                        const v = document.createElement('span'); v.textContent = item.value;
                        kvDiv.appendChild(k); kvDiv.appendChild(v);
                    }});
                    div.appendChild(kvDiv);
                    break;
                case 'code':
                    const pre = document.createElement('pre');
                    const code = document.createElement('code');
                    code.textContent = comp.source || '';
                    pre.appendChild(code);
                    div.appendChild(pre);
                    break;
                case 'button':
                    const btn = document.createElement('button');
                    btn.className = 'btn' + (comp.style === 'danger' ? ' btn-danger' : comp.style === 'secondary' ? ' btn-secondary' : '');
                    btn.textContent = comp.label || 'Click';
                    btn.onclick = () => fetch('/canvas/api/panels/' + panelId + '/action', {{
                        method: 'POST', headers: {{ 'Content-Type': 'application/json' }},
                        body: JSON.stringify({{ action: comp.action || comp.label }})
                    }});
                    div.appendChild(btn);
                    break;
                case 'json':
                    const jpre = document.createElement('pre');
                    jpre.textContent = JSON.stringify(comp.data, null, 2);
                    div.appendChild(jpre);
                    break;
                default:
                    div.textContent = JSON.stringify(comp);
            }}
            return div;
        }}

        components.forEach(c => container.appendChild(renderComponent(c)));
    </script>
</body>
</html>"#,
        title = html_escape(&panel.title),
        components_json = components_json,
        panel_id = html_escape(&panel.panel_id),
    );

    Ok(Html(html))
}

/// Basic HTML escaping.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_store_upsert_and_get() {
        let store = CanvasStore::default();
        store
            .upsert(
                "panel-1".to_string(),
                "Test Panel".to_string(),
                serde_json::json!([{"type": "text", "content": "Hello"}]),
                None,
            )
            .await;

        let panel = store.get("panel-1").await;
        assert!(panel.is_some());
        assert_eq!(panel.unwrap().title, "Test Panel");
    }

    #[tokio::test]
    async fn test_store_dismiss() {
        let store = CanvasStore::default();
        store
            .upsert(
                "panel-1".to_string(),
                "Test".to_string(),
                serde_json::json!([]),
                None,
            )
            .await;

        assert!(store.dismiss("panel-1").await);
        assert!(store.get("panel-1").await.is_none());
        assert!(!store.dismiss("nonexistent").await);
    }

    #[tokio::test]
    async fn test_store_list() {
        let store = CanvasStore::default();
        for i in 0..3 {
            store
                .upsert(
                    format!("panel-{}", i),
                    format!("Panel {}", i),
                    serde_json::json!([]),
                    None,
                )
                .await;
        }

        let panels = store.list().await;
        assert_eq!(panels.len(), 3);
    }

    #[tokio::test]
    async fn test_store_expiry() {
        let store = CanvasStore::new(Duration::from_millis(1));
        store
            .upsert(
                "temp".to_string(),
                "Temp".to_string(),
                serde_json::json!([]),
                None,
            )
            .await;

        // Wait for expiry
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(store.get("temp").await.is_none());
    }

    #[tokio::test]
    async fn test_store_prune() {
        let store = CanvasStore::new(Duration::from_millis(1));
        store
            .upsert(
                "old".to_string(),
                "Old".to_string(),
                serde_json::json!([]),
                None,
            )
            .await;

        tokio::time::sleep(Duration::from_millis(10)).await;
        let pruned = store.prune().await;
        assert_eq!(pruned, 1);
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("\"quotes\""), "&quot;quotes&quot;");
    }
}
