//! Canvas gateway — HTTP routes for serving agent-generated UI panels.
//!
//! Mounted on the unified webhook server at `/canvas/`. Provides:
//! - `GET /canvas/:panel_id` — Render a canvas panel as HTML
//! - `GET /canvas/api/panels` — List active panels (JSON)
//! - `GET /canvas/api/panels/:panel_id` — Get panel data (JSON)
//! - `POST /canvas/api/panels/:panel_id/action` — Handle form/button callbacks
//!
//! Panels are stored in memory with TTL expiry. The `CanvasTool` pushes
//! panels here; clients (ThinClaw Desktop, browser, Telegram webapps) consume them.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use thinclaw_channels_core::IncomingMessage;
use thinclaw_identity::scope_id_from_key;
use tokio::sync::RwLock;

/// Default TTL for canvas panels (15 minutes).
const DEFAULT_PANEL_TTL: Duration = Duration::from_secs(15 * 60);

/// Maximum number of active panels before LRU eviction.
const MAX_ACTIVE_PANELS: usize = 64;
const MAX_ACTION_BYTES: usize = 256;
const MAX_ACTION_FIELDS: usize = 64;
const MAX_ACTION_VALUES_BYTES: usize = 64 * 1024;

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
    /// Model-facing panel ID within the owning conversation.
    pub source_panel_id: String,
    /// Canonical ingress envelope used to route callbacks back to the exact
    /// actor/conversation that created the panel. Deliberately not serialized.
    pub source_message: IncomingMessage,
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

/// A canvas action accepted from a client callback.
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
    /// Submission path consumed by the standalone channel loop or a desktop
    /// action worker. Actions are pushed immediately; they are never drained by
    /// whichever unrelated agent turn happens to run next.
    submission_tx: Arc<RwLock<Option<tokio::sync::mpsc::Sender<IncomingMessage>>>>,
}

impl Default for CanvasStore {
    fn default() -> Self {
        Self::new(DEFAULT_PANEL_TTL)
    }
}

impl CanvasStore {
    /// Create a new canvas store with a custom TTL.
    pub fn new(ttl: Duration) -> Self {
        Self {
            panels: Arc::new(RwLock::new(HashMap::new())),
            ttl,
            submission_tx: Arc::new(RwLock::new(None)),
        }
    }

    /// Attach the bounded ingress queue that should receive authenticated
    /// canvas callbacks.
    pub async fn set_submission_sender(&self, sender: tokio::sync::mpsc::Sender<IncomingMessage>) {
        *self.submission_tx.write().await = Some(sender);
    }

    /// Insert or update a panel.
    pub async fn upsert(
        &self,
        panel_id: String,
        title: String,
        components: serde_json::Value,
        metadata: Option<serde_json::Value>,
    ) {
        let source_message = IncomingMessage::new("canvas", "local_user", "");
        self.upsert_with_handle(
            panel_id.clone(),
            panel_id,
            title,
            components,
            metadata,
            source_message,
        )
        .await;
    }

    /// Insert/update a panel under a collision-resistant handle derived from
    /// its actor, conversation, channel, thread, and model-facing panel ID.
    /// Returns the public handle sent to UI clients.
    pub async fn upsert_for_message(
        &self,
        source_panel_id: String,
        title: String,
        components: serde_json::Value,
        metadata: Option<serde_json::Value>,
        source_message: IncomingMessage,
    ) -> String {
        let handle = scoped_panel_handle(&source_message, &source_panel_id);
        self.upsert_with_handle(
            handle.clone(),
            source_panel_id,
            title,
            components,
            metadata,
            source_message,
        )
        .await;
        handle
    }

    /// Return the deterministic public handle for a model-facing panel ID in
    /// this exact ingress scope. This lets update/dismiss events address the
    /// same UI panel without exposing globally reusable model-chosen IDs.
    pub fn public_handle_for_message(
        &self,
        source_panel_id: &str,
        source_message: &IncomingMessage,
    ) -> String {
        scoped_panel_handle(source_message, source_panel_id)
    }

    async fn upsert_with_handle(
        &self,
        panel_id: String,
        source_panel_id: String,
        title: String,
        components: serde_json::Value,
        metadata: Option<serde_json::Value>,
        mut source_message: IncomingMessage,
    ) {
        // Callbacks need the canonical routing envelope, not the originating
        // prompt or media. Avoid retaining sensitive/large user content for
        // the entire panel TTL.
        source_message.content.clear();
        source_message.attachments.clear();
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
                source_panel_id,
                source_message,
            },
        );
    }

    pub async fn get_for_message(
        &self,
        source_panel_id: &str,
        source_message: &IncomingMessage,
    ) -> Option<StoredPanel> {
        self.get(&scoped_panel_handle(source_message, source_panel_id))
            .await
    }

    pub async fn dismiss_for_message(
        &self,
        source_panel_id: &str,
        source_message: &IncomingMessage,
    ) -> bool {
        self.dismiss(&scoped_panel_handle(source_message, source_panel_id))
            .await
    }

    /// Dismiss (remove) a panel.
    pub async fn dismiss(&self, panel_id: &str) -> bool {
        self.panels.write().await.remove(panel_id).is_some()
    }

    /// Get a panel by ID.
    pub async fn get(&self, panel_id: &str) -> Option<StoredPanel> {
        let mut panels = self.panels.write().await;
        if panels
            .get(panel_id)
            .is_some_and(|panel| panel.is_expired(self.ttl))
        {
            panels.remove(panel_id);
            return None;
        }
        panels.get(panel_id).cloned()
    }

    /// List all active (non-expired) panels.
    pub async fn list(&self) -> Vec<StoredPanel> {
        let mut panels = self.panels.write().await;
        panels.retain(|_, panel| !panel.is_expired(self.ttl));
        panels.values().cloned().collect()
    }

    /// Prune expired panels. Returns the count of pruned panels.
    pub async fn prune(&self) -> usize {
        let mut panels = self.panels.write().await;
        let before = panels.len();
        panels.retain(|_, p| !p.is_expired(self.ttl));
        before - panels.len()
    }

    /// Submit an action immediately to the panel's owning conversation.
    pub async fn dispatch_action(
        &self,
        panel: &StoredPanel,
        action: String,
        values: HashMap<String, serde_json::Value>,
    ) -> Result<(), CanvasDispatchError> {
        let action = action.trim().to_string();
        if action.is_empty() || action.len() > MAX_ACTION_BYTES {
            return Err(CanvasDispatchError::Invalid(
                "action must contain 1 to 256 bytes".to_string(),
            ));
        }
        if values.len() > MAX_ACTION_FIELDS {
            return Err(CanvasDispatchError::Invalid(
                "too many canvas form fields".to_string(),
            ));
        }
        if values
            .keys()
            .any(|key| key.is_empty() || key.len() > MAX_ACTION_BYTES)
        {
            return Err(CanvasDispatchError::Invalid(
                "canvas form field names must contain 1 to 256 bytes".to_string(),
            ));
        }
        let values_json = serde_json::to_string(&values).map_err(|error| {
            CanvasDispatchError::Invalid(format!("invalid canvas values: {error}"))
        })?;
        if values_json.len() > MAX_ACTION_VALUES_BYTES {
            return Err(CanvasDispatchError::Invalid(
                "canvas values exceed the 64 KiB limit".to_string(),
            ));
        }

        let sender = self
            .submission_tx
            .read()
            .await
            .clone()
            .ok_or(CanvasDispatchError::Unavailable)?;
        let mut message = panel.source_message.clone();
        message.id = uuid::Uuid::new_v4();
        message.received_at = chrono::Utc::now();
        message.content = format!(
            "[Canvas Interaction]\n{}",
            serde_json::json!({
                "panel_id": &panel.source_panel_id,
                "action": &action,
                "values": values,
            })
        );
        message.attachments.clear();
        if !message.metadata.is_object() {
            message.metadata = serde_json::json!({});
        }
        if let Some(metadata) = message.metadata.as_object_mut() {
            metadata.insert("canvas_interaction".to_string(), serde_json::json!(true));
            metadata.insert(
                "canvas_panel_id".to_string(),
                serde_json::json!(panel.source_panel_id.clone()),
            );
            metadata.insert("canvas_action".to_string(), serde_json::json!(action));
        }
        sender.try_send(message).map_err(|error| match error {
            tokio::sync::mpsc::error::TrySendError::Full(_) => CanvasDispatchError::Full,
            tokio::sync::mpsc::error::TrySendError::Closed(_) => CanvasDispatchError::Unavailable,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasDispatchError {
    Full,
    Unavailable,
    Invalid(String),
}

fn scoped_panel_handle(message: &IncomingMessage, source_panel_id: &str) -> String {
    let identity = message.resolved_identity();
    let thread_id = message.thread_id.as_deref().unwrap_or_default();
    scope_id_from_key(&format!(
        "canvas:scope:{}:principal:{}:{}:actor:{}:{}:channel:{}:{}:thread:{}:{}:panel:{}:{}",
        identity.conversation_scope_id,
        identity.principal_id.len(),
        identity.principal_id,
        identity.actor_id.len(),
        identity.actor_id,
        message.channel.len(),
        message.channel,
        thread_id.len(),
        thread_id,
        source_panel_id.len(),
        source_panel_id,
    ))
    .to_string()
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

#[derive(Clone)]
struct CanvasHttpState {
    store: CanvasStore,
    auth_token: Arc<str>,
}

#[derive(Default, Deserialize)]
struct CanvasAuthQuery {
    token: Option<String>,
}

impl std::fmt::Debug for CanvasAuthQuery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanvasAuthQuery")
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

/// Build authenticated axum routes for the canvas gateway.
pub fn canvas_routes(store: CanvasStore, auth_token: impl Into<String>) -> Router {
    let state = CanvasHttpState {
        store,
        auth_token: Arc::from(auth_token.into()),
    };
    Router::new()
        .route("/canvas/api/panels", get(list_panels))
        .route("/canvas/api/panels/{panel_id}", get(get_panel_api))
        .route("/canvas/api/panels/{panel_id}/action", post(handle_action))
        .route("/canvas/{panel_id}", get(render_panel))
        .layer(DefaultBodyLimit::max(128 * 1024))
        .with_state(state)
}

// ── Route handlers ────────────────────────────────────────────

/// GET /canvas/api/panels — List all active panels.
async fn list_panels(
    State(state): State<CanvasHttpState>,
    headers: HeaderMap,
    Query(auth): Query<CanvasAuthQuery>,
) -> Result<Json<Vec<PanelSummary>>, StatusCode> {
    authorize_canvas_request(&state, &headers, &auth)?;
    let panels = state.store.list().await;
    let summaries: Vec<PanelSummary> = panels
        .into_iter()
        .map(|p| PanelSummary {
            panel_id: p.panel_id,
            title: p.title,
        })
        .collect();
    Ok(Json(summaries))
}

#[derive(Serialize)]
struct PanelSummary {
    panel_id: String,
    title: String,
}

/// GET /canvas/api/panels/:panel_id — Get panel data as JSON.
async fn get_panel_api(
    State(state): State<CanvasHttpState>,
    headers: HeaderMap,
    Query(auth): Query<CanvasAuthQuery>,
    Path(panel_id): Path<String>,
) -> Result<Json<StoredPanel>, StatusCode> {
    authorize_canvas_request(&state, &headers, &auth)?;
    state
        .store
        .get(&panel_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// POST /canvas/api/panels/:panel_id/action — Handle a client action callback.
async fn handle_action(
    State(state): State<CanvasHttpState>,
    headers: HeaderMap,
    Query(auth): Query<CanvasAuthQuery>,
    Path(panel_id): Path<String>,
    Json(payload): Json<ActionPayload>,
) -> impl IntoResponse {
    if let Err(status) = authorize_canvas_request(&state, &headers, &auth) {
        return status.into_response();
    }
    // Verify the panel exists
    let Some(panel) = state.store.get(&panel_id).await else {
        return (StatusCode::NOT_FOUND, "Panel not found").into_response();
    };

    tracing::info!(
        panel_id = %panel_id,
        action = %payload.action,
        "Canvas action callback received"
    );

    match state
        .store
        .dispatch_action(&panel, payload.action, payload.values)
        .await
    {
        Ok(()) => (StatusCode::ACCEPTED, "Action submitted").into_response(),
        Err(CanvasDispatchError::Full) => {
            (StatusCode::TOO_MANY_REQUESTS, "Agent ingress queue full").into_response()
        }
        Err(CanvasDispatchError::Unavailable) => {
            (StatusCode::SERVICE_UNAVAILABLE, "Agent ingress unavailable").into_response()
        }
        Err(CanvasDispatchError::Invalid(message)) => {
            (StatusCode::BAD_REQUEST, message).into_response()
        }
    }
}

/// GET /canvas/:panel_id — Render a panel as a standalone HTML page.
async fn render_panel(
    State(state): State<CanvasHttpState>,
    headers: HeaderMap,
    Query(auth): Query<CanvasAuthQuery>,
    Path(panel_id): Path<String>,
) -> Result<Response, StatusCode> {
    authorize_canvas_request(&state, &headers, &auth)?;
    let panel = state
        .store
        .get(&panel_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    // Escape for inline <script> embedding: panel content can carry untrusted
    // data the agent fetched, and a raw `</script>` in a component string would
    // otherwise break out of the script block.
    let components_json = escape_json_for_script(
        &serde_json::to_string(&panel.components).unwrap_or_else(|_| "[]".to_string()),
    );
    let auth_token_json = escape_json_for_script(
        &serde_json::to_string(state.auth_token.as_ref()).unwrap_or_else(|_| "\"\"".to_string()),
    );
    let panel_id_json = escape_json_for_script(
        &serde_json::to_string(&panel.panel_id).unwrap_or_else(|_| "\"\"".to_string()),
    );
    let csp_nonce = uuid::Uuid::new_v4().simple().to_string();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>{title} — ThinClaw Canvas</title>
    <style nonce="{csp_nonce}">
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
        img {{ max-width: 100%; height: auto; border-radius: 6px; }}
        hr {{ border: 0; border-top: 1px solid var(--border); }}
        progress {{ width: 100%; accent-color: var(--accent); }}
        input, select, textarea {{
            width: 100%; padding: 0.5rem 0.75rem; border-radius: 6px;
            border: 1px solid var(--border); background: var(--bg); color: var(--text);
            font-family: var(--font); font-size: 0.9rem;
        }}
        label {{ display: block; margin-bottom: 0.25rem; font-size: 0.85rem; color: var(--text-muted); }}
        .form-group {{ margin-bottom: 0.75rem; }}
        .action-error {{ margin-top: 0.75rem; color: var(--danger); font-size: 0.85rem; }}
    </style>
</head>
<body>
    <div class="panel">
        <div class="panel-title">{title}</div>
        <div id="components"></div>
    </div>
    <script nonce="{csp_nonce}">
        const components = {components_json};
        const container = document.getElementById('components');
        const panelId = {panel_id_json};
        const canvasToken = {auth_token_json};

        async function submitAction(action, values = {{}}) {{
            const response = await fetch('/canvas/api/panels/' + encodeURIComponent(panelId) + '/action', {{
                method: 'POST',
                headers: {{
                    'Authorization': 'Bearer ' + canvasToken,
                    'Content-Type': 'application/json'
                }},
                body: JSON.stringify({{ action, values }})
            }});
            if (!response.ok) throw new Error(await response.text() || ('HTTP ' + response.status));
        }}

        function reportActionError(container, error) {{
            let notice = container.querySelector(':scope > .action-error');
            if (!notice) {{
                notice = document.createElement('div');
                notice.className = 'action-error';
                notice.setAttribute('role', 'alert');
                container.appendChild(notice);
            }}
            notice.textContent = error instanceof Error ? error.message : 'Action failed';
        }}

        function safeImageSource(value) {{
            if (typeof value !== 'string') return null;
            if (/^data:image\/(?:png|gif|jpeg|webp|avif);base64,[a-z0-9+/=]+$/i.test(value)) return value;
            try {{
                const parsed = new URL(value, window.location.href);
                return parsed.protocol === 'https:' || parsed.protocol === 'http:' ? parsed.href : null;
            }} catch (_) {{
                return null;
            }}
        }}

        function renderComponent(comp) {{
            const div = document.createElement('div');
            div.className = 'component';
            switch (comp.type) {{
                case 'text':
                    div.textContent = comp.content || '';
                    break;
                case 'heading':
                    const headingLevel = Math.round(Math.min(6, Math.max(1, Number.isFinite(Number(comp.level)) ? Number(comp.level) : 2)));
                    const h = document.createElement('h' + headingLevel);
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
                    code.textContent = comp.content || '';
                    pre.appendChild(code);
                    div.appendChild(pre);
                    break;
                case 'button':
                    const btn = document.createElement('button');
                    btn.className = 'btn' + (comp.style === 'danger' ? ' btn-danger' : comp.style === 'secondary' ? ' btn-secondary' : '');
                    btn.textContent = comp.label || 'Click';
                    btn.onclick = async () => {{
                        btn.disabled = true;
                        try {{ await submitAction(comp.action || comp.label || 'click'); }}
                        catch (error) {{ reportActionError(div, error); }}
                        finally {{ btn.disabled = false; }}
                    }};
                    div.appendChild(btn);
                    break;
                case 'image':
                    const imageSource = safeImageSource(comp.src);
                    if (!imageSource) {{
                        div.textContent = 'Blocked unsafe image source';
                        break;
                    }}
                    const img = document.createElement('img');
                    img.src = imageSource;
                    img.alt = comp.alt || '';
                    if (Number.isFinite(Number(comp.width)) && Number(comp.width) > 0) {{
                        img.style.maxWidth = Math.min(4096, Number(comp.width)) + 'px';
                    }}
                    div.appendChild(img);
                    break;
                case 'progress':
                    const rawMax = Number(comp.max);
                    const progressMax = Number.isFinite(rawMax) && rawMax > 0 ? rawMax : 100;
                    const rawValue = Number(comp.value);
                    const progressValue = Number.isFinite(rawValue) ? Math.min(progressMax, Math.max(0, rawValue)) : 0;
                    if (comp.label) {{
                        const progressLabel = document.createElement('label');
                        progressLabel.textContent = comp.label;
                        div.appendChild(progressLabel);
                    }}
                    const progress = document.createElement('progress');
                    progress.max = progressMax;
                    progress.value = progressValue;
                    div.appendChild(progress);
                    break;
                case 'divider':
                    div.appendChild(document.createElement('hr'));
                    break;
                case 'form':
                    const form = document.createElement('form');
                    (Array.isArray(comp.fields) ? comp.fields : []).forEach(field => {{
                        if (!field || typeof field.name !== 'string' || !field.name) return;
                        const group = document.createElement('div');
                        group.className = 'form-group';
                        const label = document.createElement('label');
                        label.textContent = field.label || field.name;
                        group.appendChild(label);
                        let control;
                        if (field.type === 'textarea') {{
                            control = document.createElement('textarea');
                            control.rows = Math.min(20, Math.max(1, Number(field.rows) || 3));
                        }} else if (field.type === 'select') {{
                            control = document.createElement('select');
                            (Array.isArray(field.options) ? field.options : []).forEach(optionValue => {{
                                const option = document.createElement('option');
                                option.value = String(optionValue);
                                option.textContent = String(optionValue);
                                control.appendChild(option);
                            }});
                        }} else {{
                            control = document.createElement('input');
                            control.type = field.type === 'number' ? 'number' : field.type === 'checkbox' ? 'checkbox' : 'text';
                            if (control.type === 'number') {{
                                if (Number.isFinite(Number(field.min))) control.min = String(field.min);
                                if (Number.isFinite(Number(field.max))) control.max = String(field.max);
                            }}
                            if (control.type === 'checkbox') control.checked = Boolean(field.checked);
                        }}
                        control.name = field.name;
                        if (field.placeholder) control.placeholder = field.placeholder;
                        if (field.required) control.required = true;
                        group.appendChild(control);
                        form.appendChild(group);
                    }});
                    const submit = document.createElement('button');
                    submit.type = 'submit';
                    submit.className = 'btn';
                    submit.textContent = comp.submit_label || 'Submit';
                    form.appendChild(submit);
                    form.onsubmit = async event => {{
                        event.preventDefault();
                        const values = Object.create(null);
                        for (const control of form.elements) {{
                            if (!control.name) continue;
                            values[control.name] = control.type === 'checkbox'
                                ? control.checked
                                : control.type === 'number' && control.value !== ''
                                    ? Number(control.value)
                                    : control.value;
                        }}
                        submit.disabled = true;
                        try {{ await submitAction('form_submit:' + (comp.form_id || 'form'), values); }}
                        catch (error) {{ reportActionError(div, error); }}
                        finally {{ submit.disabled = false; }}
                    }};
                    div.appendChild(form);
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

        (Array.isArray(components) ? components : []).forEach(c => container.appendChild(renderComponent(c)));
    </script>
</body>
</html>"#,
        title = html_escape(&panel.title),
        components_json = components_json,
        auth_token_json = auth_token_json,
        panel_id_json = panel_id_json,
        csp_nonce = csp_nonce,
    );

    let mut response = Html(html).into_response();
    let csp = format!(
        "default-src 'none'; script-src 'nonce-{csp_nonce}'; style-src 'nonce-{csp_nonce}'; \
         img-src https: http: data:; connect-src 'self'; form-action 'none'; \
         frame-ancestors 'self'; base-uri 'none'; object-src 'none'"
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&csp).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    Ok(response)
}

fn authorize_canvas_request(
    state: &CanvasHttpState,
    headers: &HeaderMap,
    query: &CanvasAuthQuery,
) -> Result<(), StatusCode> {
    let supplied = query.token.as_deref().or_else(|| {
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
    });
    let Some(supplied) = supplied else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    if state.auth_token.is_empty()
        || supplied
            .as_bytes()
            .ct_eq(state.auth_token.as_bytes())
            .unwrap_u8()
            != 1
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

/// Basic HTML escaping.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Escape an already-serialized JSON string for safe embedding inside an inline
/// `<script>` element. `<`, `>`, and `&` become their `\uXXXX` forms (so a
/// `</script>`, `<!--`, or `]]>` in the data cannot break out of the block), and
/// the U+2028/U+2029 line separators are escaped (they are legal in JSON but
/// terminate JavaScript string literals). The result remains valid JSON/JS.
fn escape_json_for_script(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use thinclaw_identity::{ConversationKind, ResolvedIdentity};
    use tower::ServiceExt;

    fn message_for(actor: &str, thread: &str) -> IncomingMessage {
        IncomingMessage::new("test", actor, "source")
            .with_thread(thread)
            .with_metadata(serde_json::json!({"thread_id": thread}))
            .with_identity(ResolvedIdentity {
                principal_id: "owner".to_string(),
                actor_id: actor.to_string(),
                conversation_scope_id: scope_id_from_key(&format!("scope:{thread}")),
                conversation_kind: ConversationKind::Direct,
                raw_sender_id: actor.to_string(),
                stable_external_conversation_key: format!("test:{thread}"),
            })
    }

    #[test]
    fn escape_json_for_script_neutralizes_script_breakout() {
        // A component string containing `</script>` must not be able to close
        // the inline script block.
        let json = serde_json::to_string(&serde_json::json!([
            {"text": "</script><img src=x onerror=alert(1)>"}
        ]))
        .unwrap();
        let escaped = escape_json_for_script(&json);
        assert!(
            !escaped.contains("</script>"),
            "raw </script> survived: {escaped}"
        );
        assert!(!escaped.contains('<'), "unescaped '<' survived: {escaped}");
        // Round-trips back to the original once a JS/JSON parser decodes it.
        let decoded: serde_json::Value = serde_json::from_str(&escaped).unwrap();
        assert_eq!(decoded[0]["text"], "</script><img src=x onerror=alert(1)>");
    }

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

    #[tokio::test]
    async fn scoped_handles_isolate_actors_and_conversations() {
        let store = CanvasStore::default();
        let alice = message_for("alice", "one");
        let bob = message_for("bob", "one");
        let other_thread = message_for("alice", "two");

        let alice_handle = store
            .upsert_for_message(
                "dashboard".to_string(),
                "Alice".to_string(),
                serde_json::json!([]),
                None,
                alice.clone(),
            )
            .await;
        let bob_handle = store
            .upsert_for_message(
                "dashboard".to_string(),
                "Bob".to_string(),
                serde_json::json!([]),
                None,
                bob,
            )
            .await;

        assert_ne!(alice_handle, bob_handle);
        assert_ne!(
            alice_handle,
            store.public_handle_for_message("dashboard", &other_thread)
        );
        assert_eq!(store.list().await.len(), 2);
        assert!(store.get_for_message("dashboard", &alice).await.is_some());
    }

    #[tokio::test]
    async fn action_dispatch_preserves_the_owning_ingress_scope() {
        let store = CanvasStore::default();
        let source = message_for("alice", "thread-a");
        let handle = store
            .upsert_for_message(
                "form".to_string(),
                "Form".to_string(),
                serde_json::json!([]),
                None,
                source.clone(),
            )
            .await;
        let panel = store.get(&handle).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        store.set_submission_sender(tx).await;

        store
            .dispatch_action(
                &panel,
                "submit".to_string(),
                HashMap::from([("answer".to_string(), serde_json::json!(42))]),
            )
            .await
            .unwrap();

        let dispatched = rx.recv().await.unwrap();
        assert_eq!(dispatched.channel, source.channel);
        assert_eq!(dispatched.thread_id, source.thread_id);
        assert_eq!(dispatched.identity, source.identity);
        assert_eq!(dispatched.metadata["canvas_panel_id"], "form");
        assert_eq!(dispatched.metadata["canvas_action"], "submit");
        assert!(dispatched.content.contains("\"answer\":42"));
        assert!(panel.source_message.content.is_empty());
        assert!(panel.source_message.attachments.is_empty());
    }

    #[tokio::test]
    async fn action_dispatch_rejects_oversized_or_empty_input_before_enqueue() {
        let store = CanvasStore::default();
        let source = message_for("alice", "thread-a");
        let handle = store
            .upsert_for_message(
                "form".to_string(),
                "Form".to_string(),
                serde_json::json!([]),
                None,
                source,
            )
            .await;
        let panel = store.get(&handle).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        store.set_submission_sender(tx).await;

        assert!(matches!(
            store
                .dispatch_action(&panel, "   ".to_string(), HashMap::new())
                .await,
            Err(CanvasDispatchError::Invalid(_))
        ));
        assert!(matches!(
            store
                .dispatch_action(&panel, "x".repeat(MAX_ACTION_BYTES + 1), HashMap::new())
                .await,
            Err(CanvasDispatchError::Invalid(_))
        ));
        assert!(matches!(
            store
                .dispatch_action(
                    &panel,
                    "submit".to_string(),
                    HashMap::from([(
                        "value".to_string(),
                        serde_json::json!("x".repeat(MAX_ACTION_VALUES_BYTES)),
                    )]),
                )
                .await,
            Err(CanvasDispatchError::Invalid(_))
        ));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn canvas_http_routes_require_authentication() {
        let store = CanvasStore::default();
        let app = canvas_routes(store, "test-secret");

        let unauthenticated = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/canvas/api/panels")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

        let authenticated = app
            .oneshot(
                Request::builder()
                    .uri("/canvas/api/panels")
                    .header("authorization", "Bearer test-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authenticated.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn canvas_http_rejects_invalid_actions_and_hardens_rendered_html() {
        let store = CanvasStore::default();
        let source = message_for("alice", "thread-a");
        let handle = store
            .upsert_for_message(
                "panel".to_string(),
                "<unsafe>".to_string(),
                serde_json::json!([{
                    "type": "text",
                    "content": "</script><img src=x onerror=alert(1)>"
                }]),
                None,
                source,
            )
            .await;
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        store.set_submission_sender(tx).await;
        let app = canvas_routes(store, "test-secret");

        let invalid = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/canvas/api/panels/{handle}/action"))
                    .header("authorization", "Bearer test-secret")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"action":"   "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let rendered = app
            .oneshot(
                Request::builder()
                    .uri(format!("/canvas/{handle}?token=test-secret"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rendered.status(), StatusCode::OK);
        assert_eq!(
            rendered.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert!(
            rendered
                .headers()
                .get(header::CONTENT_SECURITY_POLICY)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("script-src 'nonce-")
        );
        let body = axum::body::to_bytes(rendered.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(!body.contains("</script><img"));
        assert!(body.contains("&lt;unsafe&gt;"));
        assert!(body.contains("\\u003c/script\\u003e"));
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("\"quotes\""), "&quot;quotes&quot;");
    }
}
