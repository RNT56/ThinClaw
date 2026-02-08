# Implementation Progress (Updated: 2026-01-28)

### Phase 1: Foundation ✅ COMPLETE
- `Cargo.toml` → Added tokio-tungstenite, tungstenite, futures-util
- `src-tauri/src/clawdbot/mod.rs` → Module structure
- `clawdbot/frames.rs` → WsFrame models, request builders
- `clawdbot/normalizer.rs` → UiEvent types, fallback parsing
- `clawdbot/config.rs` → MoltbotConfig, Slack/Telegram structs
- `clawdbot/ws_client.rs` → Async client, reconnection, actor pattern

### Phase 2: UI Scaffolding ✅ COMPLETE
- `SettingsSidebar.tsx` → Added Clawdbot section (Slack, Telegram, Gateway)
- `SlackTab.tsx` → Token inputs, manifest copy, validation, setup instructions
- `TelegramTab.tsx` → Bot token, DM policy, groups toggle, validation
- `GatewayTab.tsx` → Start/stop controls, status display, diagnostics export
- `SettingsPages.tsx` → Integrated all Clawdbot tabs

### Phase 3: Core Integration ✅ COMPLETE
- `clawdbot/commands.rs` → ClawdbotManager + 12 Tauri commands
- `lib.rs` → Registered commands in specta builder, ClawdbotManager state
- `src/lib/clawdbot.ts` → TypeScript API wrappers
- All 3 settings tabs wired to call Tauri commands

### Phase 4: Chat UI ✅ COMPLETE
- `ChatLayout.tsx` → 3-way mode switching (chat/clawdbot/settings)
- `ClawdbotSidebar.tsx` → Sessions list, gateway status, refresh
- `ClawdbotChatView.tsx` → Message history, send, subscribe
- Radio icon + gateway status indicator in sidebar

---

Below is a comprehensive implementation spec for adding a standalone Moltbot/Clawdbot mode inside your Scrappy Inference Tauri (Rust) desktop app, with a separate UI (clone of your existing chat UI) plus a settings tab for connectors/auth. It assumes macOS + Windows, Slack + Telegram first, and llama.cpp via local HTTP server.

I’m using “Moltbot” for the upstream project name (Clawdbot → Moltbot rebrand).  ￼

⸻

Scrappy Inference — Standalone “Clawdbot Mode” Spec (Moltbot Integration)

1) Summary

Add a new feature area in Scrappy Inference:
	•	Mode: “Clawdbot” (standalone Moltbot assistant)
	•	UI: separate chat UI (fork/clone of your existing one) + session list + activity/logs
	•	Settings page: Slack + Telegram setup, gateway status, security toggles, workspace management
	•	Runtime: Scrappy supervises a bundled Moltbot stack (Gateway + Agent loop) and points it at your local llama.cpp endpoint.

Moltbot is designed as an always-on Gateway (WS control plane + channel owners) plus an agent loop that turns inbound messages into actions/replies.  ￼

⸻

2) High-level architecture

2.1 Components (on the user’s machine)
	1.	Scrappy Inference (Tauri/Rust)
	•	Feature gating & UI
	•	Supervisor (starts/stops Moltbot + monitors health)
	•	Native WS client to Moltbot Gateway
	•	Settings & secrets management
	•	Log viewer
	2.	Moltbot Gateway (bundled)
	•	Owns Slack & Telegram connections
	•	Exposes WebSocket protocol (control plane) on one local port
	•	Multiplexes WS + HTTP on the same port  ￼
	•	Provides chat/session APIs (chat.history, chat.send, chat.subscribe, sessions.*, etc.)  ￼
	3.	Moltbot Agent runtime (bundled, same package)
	•	Runs the Moltbot agent loop (context assembly → inference → tool calls → persistence)  ￼
	•	Uses your llama.cpp HTTP endpoint as its model provider
	4.	llama.cpp server (existing, started by Scrappy today)
	•	Local inference
	•	Exposes OpenAI-compatible chat completions endpoint (your current setup)

2.2 Control-plane / data flows
	•	Slack/Telegram → Gateway → session routing → agent loop → tool calls → reply → Gateway → Slack/Telegram
	•	Scrappy UI → Gateway WS (operator role) to:
	•	display sessions and transcript (chat.history)
	•	send messages (chat.send)
	•	subscribe to live updates (chat.subscribe)
	•	display status/health (status, health events)
	•	handle approvals if enabled (exec.approval.requested)  ￼

⸻

3) Key requirements

3.1 Strict separation from Scrappy’s existing agent/tooling
	•	No shared memory, no shared tools, no shared prompt flow.
	•	Separate on-disk storage roots (see §6).
	•	Separate UI route, separate settings section.

3.2 Local-only, authenticated gateway
	•	Bind Gateway to loopback only (gateway.bind: "loopback").  ￼
	•	Enforce gateway auth token; Moltbot is fail-closed if not configured.  ￼
	•	Disable or set mDNS discovery to minimal/off inside your packaged config (recommended).  ￼

3.3 Slack + Telegram first
	•	Slack: Socket Mode with xapp-… + xoxb-… tokens and event subscriptions.  ￼
	•	Telegram: BotFather token, DM pairing by default.  ￼

⸻

4) UX specification (Scrappy)

4.1 Navigation and feature gating

Add top-level section (or sidebar entry): Clawdbot with subpages:
	1.	Chat
	•	Session list (left)
	•	Transcript (center)
	•	Composer (bottom)
	•	Connection indicator (top right)
	2.	Activity / Logs
	•	Gateway logs
	•	Channel logs (Slack/Telegram)
	•	Tool execution logs (optional)
	3.	Settings
	•	Gateway status and controls
	•	Model backend mapping (llama.cpp URL + model id mapping)
	•	Slack setup
	•	Telegram setup
	•	Security toggles
	•	Workspace path + reset controls

4.2 Chat UI behavior (native WebChat-like)

Moltbot’s WebChat model is: chat.history + chat.send + best-effort push updates via chat.subscribe events.  ￼

Required behaviors
	•	On app open: connect WS, subscribe to chat events.
	•	Session list:
	•	show main session and group/channel sessions
	•	show labels per Moltbot conventions (channel + token, etc.)  ￼
	•	Transcript:
	•	lazy load via chat.history (paging)
	•	render assistant/user/tool outputs with proper formatting
	•	Send:
	•	chat.send with idempotency key
	•	optimistic UI state → reconcile with event:"chat" updates
	•	Live updates:
	•	on event:"chat" apply incremental updates to the active transcript

4.3 Settings UI (connectors + auth)

Slack tab
	•	“Enable Slack”
	•	Inputs:
	•	App Token (xapp-...)
	•	Bot Token (xoxb-...)
	•	Optional User Token (xoxp-...) if you choose to support read-only extra scopes later (keep out of MVP)
	•	“Validate” button:
	•	writes config, restarts gateway, checks channel status
	•	Safety defaults:
	•	DMs enabled with pairing/allowlist policy
	•	Channels off by default unless allowlisted

Telegram tab
	•	“Enable Telegram”
	•	Input:
	•	Bot Token
	•	“Validate” button
	•	Safety defaults:
	•	dmPolicy: "pairing"  ￼
	•	groups off by default; if enabled, require mention by default  ￼

Gateway tab
	•	Start/Stop/Restart
	•	Port and bind mode display
	•	“Run security audit” shortcut (see §10)

⸻

5) Gateway WS integration specification (Scrappy client)

5.1 Transport and handshake
	•	WebSocket, JSON text frames.
	•	First server frame: connect.challenge event
	•	Client must respond with a connect request containing:
	•	protocol range (min/max)
	•	role: operator
	•	scopes: operator.read, operator.write (add operator.approvals if you implement approvals UI)
	•	auth.token
	•	device identity fields (persisted device token recommended)  ￼

5.2 Required methods for MVP UI

From Moltbot’s API categories:  ￼
	•	connect
	•	status / health (and their events)
	•	chat.history
	•	chat.send
	•	chat.abort (optional)
	•	chat.subscribe
	•	sessions.list (or equivalent sessions tool in your chosen schema version)
	•	sessions.patch / sessions.delete (optional; for “archive session” UX)

5.3 Event handling
	•	event:"tick" keepalive
	•	event:"presence" show connected clients/nodes (optional)
	•	event:"chat" push transcript changes (best-effort)  ￼
	•	event:"agent" show run progress (optional)
	•	event:"exec.approval.requested" if approvals enabled  ￼

5.4 Idempotency keys

Side-effecting methods require idempotency keys (Moltbot enforces this for safety).  ￼
Spec: every chat.send must include an idempotency key derived from (sessionKey + clientMsgId + timestamp).

⸻

6) Filesystem layout and isolation

Moltbot distinguishes:
	•	State dir (~/.clawdbot by default) holding config, secrets, sessions, extensions  ￼
	•	Workspace (~/clawd by default) holding the assistant’s working memory files, notes, artifacts  ￼

6.1 Scrappy-specific paths (must not use defaults)

Set Moltbot env vars so everything is contained inside Scrappy:

macOS
	•	Base: ~/Library/Application Support/Scrappy Inference/Clawdbot/
	•	state/
	•	workspace/
	•	logs/

Windows
	•	Base: %AppData%\Scrappy Inference\Clawdbot\
	•	state\
	•	workspace\
	•	logs\

6.2 Environment variables (set for Moltbot processes)
	•	CLAWDBOT_STATE_DIR=<.../state>
	•	CLAWDBOT_CONFIG_PATH=<.../state/moltbot.json>
	•	workspace set via config (agent.workspace) to <.../workspace>  ￼

6.3 Permissions and secrecy

Moltbot docs explicitly warn that state dirs contain sensitive data (tokens, sessions, transcripts).  ￼
Spec:
	•	Apply restrictive filesystem permissions (best-effort on Windows)
	•	Store raw Slack/Telegram tokens in OS keychain/credential store; write them into config only if unavoidable
	•	Prefer “tokenFile” approach when supported (Telegram supports tokenFile per setup docs; Slack is usually config/env)  ￼

⸻

7) Configuration generation (owned by Scrappy)

Scrappy generates moltbot.json and manages it (don’t ask the user to hand-edit). Base it on upstream schema (docs.molt.bot/gateway/configuration).  ￼

7.1 Minimal config skeleton (MVP)

{
  "gateway": {
    "bind": "loopback",
    "port": 18789,
    "auth": { "mode": "token", "token": "__GENERATED__" }
  },
  "discovery": { "mdns": { "mode": "off" } },
  "agent": { "workspace": "__SCRAPPY_WORKSPACE__" },
  "channels": {
    "slack": {
      "enabled": false,
      "botToken": "__SLACK_BOT_TOKEN__",
      "appToken": "__SLACK_APP_TOKEN__",
      "dm": { "enabled": true, "policy": "pairing" },
      "channels": {}
    },
    "telegram": {
      "enabled": false,
      "botToken": "__TELEGRAM_TOKEN__",
      "dmPolicy": "pairing",
      "groups": { "*": { "requireMention": true } }
    }
  }
}

Notes:
	•	gateway.bind: "loopback" and auth token are explicitly recommended; gateway refuses unauth connections by default.  ￼
	•	mDNS discovery can disclose info; disable or minimal.  ￼
	•	Telegram DM pairing default and group mention gating align with Telegram channel docs.  ￼
	•	Slack config shape is in gateway configuration docs and Slack channel guide.  ￼

7.2 Slack required Slack-side setup (show in UI)

From Moltbot Slack channel docs:
	•	Create Slack app
	•	Enable Socket Mode
	•	Create App token with connections:write
	•	Install to workspace, copy bot token
	•	Enable event subscriptions and subscribe to message/app mention/reactions/etc.  ￼

Your UI should provide:
	•	a “Slack Manifest” copy button (pre-filled with the scopes/events Moltbot expects)
	•	a checklist screen (so support tickets become trivial)

7.3 Telegram required Telegram-side setup (show in UI)

From Moltbot Telegram docs:
	•	Create bot with BotFather, copy token
	•	DM access is pairing by default (first DM triggers pairing code approval)  ￼

⸻

8) Process supervision & packaging

8.1 What “ship binaries” means here (practically)

Upstream Moltbot is distributed as an npm package requiring Node ≥ 22.  ￼
So “shipping binaries” should mean Scrappy ships:
	1.	a Node 22 runtime (per-OS bundled)
	2.	the Moltbot distribution (packaged JS bundle + node_modules, or a single compiled artifact)
	3.	launch scripts to run moltbot gateway and moltbot agent (or the unified entrypoint) as child processes

Moltbot supports npm/pnpm/bun in dev, and “preferred setup is moltbot onboard”.  ￼
Scrappy will not run the onboard wizard; it will generate config and start services directly.

8.2 Startup sequence
	1.	Ensure llama.cpp server is running (existing Scrappy behavior)
	2.	Start Moltbot Gateway (loopback, configured port/token)
	3.	Start Moltbot agent runtime (or moltbot agent-style runner as per upstream)
	4.	Scrappy UI connects to Gateway WS as operator

8.3 Crash & restart
	•	If gateway exits non-zero: restart gateway then agent
	•	If agent exits non-zero: restart agent
	•	Exponential backoff; surface error cause in UI

8.4 Logging
	•	Capture stdout/stderr of gateway and agent to <base>/logs/
	•	Provide “Copy diagnostics” button that bundles:
	•	versions
	•	config redacted
	•	last N log lines
	•	channel status output

⸻

9) Model backend integration (llama.cpp)

9.1 Provider configuration

Moltbot supports “any model”, and is configured via its model provider system (docs referenced from README).  ￼
Your implementation must:
	•	map Scrappy’s “selected model” into Moltbot’s model configuration (base URL + model id)
	•	ensure it points to your llama.cpp OpenAI-compatible endpoint

Spec requirements
	•	llama.cpp bind loopback only
	•	stable base URL for the Moltbot process (http://127.0.0.1:<port>/v1)
	•	if llama.cpp doesn’t support auth, keep it loopback and random-port to reduce local collision risk

⸻

10) Security & safety hardening (must ship in MVP)

Moltbot security docs highlight:
	•	bind exposure risks
	•	mandatory auth
	•	mDNS info disclosure
	•	secrets on disk (sessions/tokens/transcripts)  ￼

10.1 Mandatory defaults
	•	gateway.bind = loopback  ￼
	•	token auth enabled  ￼
	•	discovery mDNS off/minimal  ￼
	•	Slack channels disabled by default; DMs only
	•	Telegram groups disabled by default; require mention if enabled  ￼

10.2 Exec approvals UI (recommended even in MVP)

Moltbot protocol supports approvals:
	•	gateway emits exec.approval.requested
	•	operator resolves with exec.approval.resolve requiring operator.approvals scope  ￼

Spec
	•	Add a toggle: “Require approval for exec/process/file tools”
	•	Add an “Approvals” panel listing pending approvals with Approve/Deny
	•	Default ON for first release

10.3 Optional sandboxing (phase 2, but plan now)

Moltbot supports running tool execution in Docker to reduce blast radius.  ￼
Plan
	•	Windows/macOS: detect Docker Desktop presence
	•	Offer “Sandbox non-main sessions” mode (“non-main” is especially relevant because group/channel sessions are non-main).  ￼

⸻

11) Dependencies & upstream links (what you need)

11.1 Upstream repositories
	•	Core assistant repo (Moltbot): moltbot/moltbot  ￼
	•	Website / installer scripts: moltbot/molt.bot  ￼
	•	Docs: docs.molt.bot (protocol, security, channels, configuration)  ￼
	•	Org repos (extras like Lobster workflows): moltbot org page lists related repos  ￼
	•	Optional community skill index: “awesome moltbot skills” (for future extension UI)  ￼

11.2 Runtime dependencies (bundled with Scrappy)
	•	Node.js ≥ 22 (required by Moltbot)  ￼
	•	Moltbot JS distribution (your packaging choice):
	•	ship as node + node_modules bundle, or
	•	ship a single compiled artifact if you build one internally from the repo

11.3 Scrappy (Rust) dependencies (recommended crates)
	•	WS client:
	•	tokio-tungstenite (or fastwebsockets)
	•	tokio, futures
	•	JSON + schema:
	•	serde, serde_json
	•	Storage:
	•	directories (platform paths)
	•	keychain:
	•	macOS: security-framework
	•	Windows: windows crate + Credential Manager wrappers
	•	Process supervision:
	•	tokio::process
	•	log capture + rotation (your logging stack)
	•	UI event plumbing:
	•	Tauri events for pushing WS updates into the webview

(These are implementation suggestions; the critical “dependency” is the upstream Moltbot WS protocol and config schema.)  ￼

11.4 Slack and Telegram external dependencies
	•	Slack requires Socket Mode (no public webhook URL), which is ideal for local apps.  ￼
	•	Telegram uses Bot API token from BotFather.  ￼

⸻

12) Testing plan

12.1 Unit tests (Rust)
	•	WS protocol framing:
	•	connect handshake
	•	request/response correlation
	•	event routing
	•	Chat UI state:
	•	history paging
	•	event-driven updates
	•	idempotent send retry

12.2 Integration tests (desktop)
	•	Launch gateway + agent
	•	Connect operator WS
	•	Slack DM → reply
	•	Telegram DM → reply
	•	Restart Scrappy, confirm sessions persist (Moltbot stores sessions under state dir)  ￼

12.3 Security tests
	•	Verify gateway not reachable via LAN
	•	Verify WS connect fails without token
	•	Verify mDNS is off/minimal in your shipped config  ￼

⸻

13) Delivery milestones (practical)

Milestone A — “Standalone boots + local chat”
	•	Bundle Node + Moltbot
	•	Generate config (loopback + token auth + disabled connectors)
	•	Start gateway + agent
	•	Implement WS connect + sessions + chat history/send in cloned UI

Milestone B — “Telegram DM MVP”
	•	Add Telegram settings UI
	•	Enable Telegram channel via config
	•	Pairing flow surfaced (show pending pairing / instructions)
	•	Confirm DM replies

Milestone C — “Slack DM MVP”
	•	Add Slack settings UI + manifest/checklist
	•	Enable Slack socket mode via config
	•	Confirm DM replies

Milestone D — “Hardening”
	•	Approvals UI (exec approvals events)  ￼
	•	Diagnostic bundle/export
	•	Optional: sandbox toggle (Docker-based)  ￼

⸻

14) What you should not do (common pitfalls)
	•	Don’t expose gateway on 0.0.0.0 (even temporarily).  ￼
	•	Don’t rely on Moltbot onboarding wizard in production packaging; generate config directly.
	•	Don’t share Scrappy’s tool permissions or filesystem access with Clawdbot mode—keep it sandboxable and auditable.
	•	Don’t implement your own Slack/Telegram connectors; let Moltbot own the surfaces (that’s the whole point).  ￼


Below is a Rust-first WS client blueprint you can drop into Scrappy Inference for the standalone Clawdbot mode:
	•	serde models for the Gateway WS envelope + core methods
	•	a normalization layer that converts upstream payloads → your stable UI event contract
	•	a tokio state machine outline (connect → subscribe → history/send → event loop with reconnect + idempotent retry)

This is designed to be resilient even if Moltbot evolves, by parsing most payloads as serde_json::Value and normalizing.

⸻

0) Crate choices (recommended)

# Cargo.toml
[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "sync", "process"] }
tokio-tungstenite = { version = "0.23", features = ["native-tls"] }
tungstenite = "0.23"
futures-util = "0.3"

serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4", "serde"] }
thiserror = "1"
tracing = "0.1"


⸻

1) Core WS envelope models (minimal but correct shape)

These types model the outer framing robustly.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsFrame {
    #[serde(rename = "req")]
    Req {
        id: String,
        method: String,
        #[serde(default)]
        params: Value,
    },

    #[serde(rename = "res")]
    Res {
        id: String,
        ok: bool,
        #[serde(default)]
        payload: Value,
        #[serde(default)]
        error: Option<WsError>,
    },

    #[serde(rename = "event")]
    Event {
        event: String,
        #[serde(default)]
        payload: Value,
        #[serde(default)]
        seq: Option<u64>,
        #[serde(default)]
        stateVersion: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Value,
}


⸻

2) Request builders clearly typed at the “params” layer

2.1 connect

Keep connect mostly typed (because you own it), but allow unknown fields.

use serde_json::json;

pub fn build_connect_req(
    id: String,
    token: &str,
    device_id: &str,
    nonce: Option<&str>,
    scopes: &[&str],
) -> WsFrame {
    WsFrame::Req {
        id,
        method: "connect".to_string(),
        params: json!({
            "minProtocol": 3,
            "maxProtocol": 3,
            "client": {
                "id": "scrappy-inference",
                "version": env!("CARGO_PKG_VERSION"),
                "platform": std::env::consts::OS,
                "mode": "operator"
            },
            "role": "operator",
            "scopes": scopes,
            "caps": [],
            "commands": [],
            "permissions": {},
            "auth": { "token": token },
            "locale": "en-US",
            "userAgent": format!("scrappy-inference/{}", env!("CARGO_PKG_VERSION")),
            "device": {
                "id": device_id,
                // If you later implement signed nonces / remote access, add:
                // "publicKey": "...",
                // "signature": "...",
                // "signedAt": <ms>,
                "nonce": nonce
            }
        }),
    }
}

2.2 chat.history, chat.subscribe, chat.send, chat.abort, chat.inject

use serde_json::json;

pub fn build_chat_history_req(id: String, session_key: &str, limit: u32, before: Option<&str>) -> WsFrame {
    WsFrame::Req {
        id,
        method: "chat.history".to_string(),
        params: json!({
            "sessionKey": session_key,
            "limit": limit,
            "before": before,
            "includeToolMessages": true
        }),
    }
}

pub fn build_chat_subscribe_req(id: String, session_key: &str) -> WsFrame {
    WsFrame::Req {
        id,
        method: "chat.subscribe".to_string(),
        params: json!({ "sessionKey": session_key }),
    }
}

pub fn build_chat_send_req(
    id: String,
    session_key: &str,
    idempotency_key: &str,
    text: &str,
    deliver: bool,
) -> WsFrame {
    WsFrame::Req {
        id,
        method: "chat.send".to_string(),
        params: json!({
            "sessionKey": session_key,
            "idempotencyKey": idempotency_key,
            "message": { "role": "user", "text": text, "attachments": [] },
            "options": { "deliver": deliver }
        }),
    }
}

pub fn build_chat_abort_req(id: String, session_key: &str, run_id: Option<&str>) -> WsFrame {
    WsFrame::Req {
        id,
        method: "chat.abort".to_string(),
        params: json!({
            "sessionKey": session_key,
            "runId": run_id
        }),
    }
}

pub fn build_chat_inject_req(id: String, session_key: &str, role: &str, text: &str) -> WsFrame {
    WsFrame::Req {
        id,
        method: "chat.inject".to_string(),
        params: json!({
            "sessionKey": session_key,
            "message": { "role": role, "text": text }
        }),
    }
}


⸻

3) Your stable UI contract (normalize upstream → this)

This is what your cloned chat UI should consume, regardless of upstream payload drift.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum UiEvent {
    Connected { protocol: u32 },
    Disconnected { reason: String },

    SessionList { sessions: Vec<UiSession> },

    History {
        session_key: String,
        messages: Vec<UiMessage>,
        has_more: bool,
        before: Option<String>,
    },

    AssistantDelta {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        delta: String,
    },

    AssistantFinal {
        session_key: String,
        run_id: Option<String>,
        message_id: String,
        text: String,
        usage: Option<UiUsage>,
    },

    ToolUpdate {
        session_key: String,
        run_id: Option<String>,
        tool_name: String,
        status: String,      // started|stream|ok|error
        input: serde_json::Value,
        output: serde_json::Value,
    },

    RunStatus {
        session_key: String,
        run_id: Option<String>,
        status: String,      // started|in_flight|ok|error|aborted
        error: Option<String>,
    },

    Error {
        code: String,
        message: String,
        details: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSession {
    pub session_key: String,
    pub title: Option<String>,
    pub updated_at_ms: Option<u64>,
    pub source: Option<String>, // slack|telegram|webchat|...
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiMessage {
    pub id: String,
    pub role: String, // user|assistant|tool|system
    pub ts_ms: u64,
    pub text: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}


⸻

4) Normalizer: parse gateway events into UiEvent

You’ll get WsFrame::Event { event: "chat", payload: … } and others. Because upstream payload details can vary, normalize defensively.

use serde_json::Value;

pub fn normalize_event(frame: &WsFrame) -> Option<UiEvent> {
    let WsFrame::Event { event, payload, .. } = frame else { return None };

    match event.as_str() {
        "connect.challenge" => {
            // Usually you handle this in the connection state machine (store nonce).
            None
        }

        "chat" => normalize_chat_event(payload),

        "health" | "status" => {
            // Optional: map into UiEvent::RunStatus or a dedicated health UI event
            None
        }

        // If you later implement approvals:
        // "exec.approval.requested" => ...
        _ => None,
    }
}

/// Tries multiple shapes to support upstream evolution.
/// You SHOULD refine this once you inspect real payloads from your gateway build.
fn normalize_chat_event(payload: &Value) -> Option<UiEvent> {
    // Most useful fields if present:
    let session_key = payload.get("sessionKey").and_then(Value::as_str).unwrap_or("").to_string();
    let run_id = payload.get("runId").and_then(Value::as_str).map(|s| s.to_string());

    // If upstream already includes a kind/type:
    if let Some(kind) = payload.get("kind").and_then(Value::as_str) {
        match kind {
            "assistant.delta" => {
                return Some(UiEvent::AssistantDelta {
                    session_key,
                    run_id,
                    message_id: payload.get("messageId").and_then(Value::as_str).unwrap_or("assistant").to_string(),
                    delta: payload.get("delta").and_then(Value::as_str).unwrap_or("").to_string(),
                });
            }
            "assistant.final" => {
                return Some(UiEvent::AssistantFinal {
                    session_key,
                    run_id,
                    message_id: payload.get("messageId").and_then(Value::as_str).unwrap_or("assistant").to_string(),
                    text: payload.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                    usage: payload.get("usage").and_then(|u| {
                        Some(UiUsage {
                            input_tokens: u.get("inputTokens")?.as_u64()?,
                            output_tokens: u.get("outputTokens")?.as_u64()?,
                            total_tokens: u.get("totalTokens")?.as_u64()?,
                        })
                    }),
                });
            }
            "tool" => {
                let tool = payload.get("tool").cloned().unwrap_or(Value::Null);
                return Some(UiEvent::ToolUpdate {
                    session_key,
                    run_id,
                    tool_name: tool.get("name").and_then(Value::as_str).unwrap_or("tool").to_string(),
                    status: tool.get("status").and_then(Value::as_str).unwrap_or("unknown").to_string(),
                    input: tool.get("input").cloned().unwrap_or(Value::Null),
                    output: tool.get("output").cloned().unwrap_or(Value::Null),
                });
            }
            "run.status" => {
                return Some(UiEvent::RunStatus {
                    session_key,
                    run_id,
                    status: payload.get("status").and_then(Value::as_str).unwrap_or("unknown").to_string(),
                    error: payload.get("error").and_then(Value::as_str).map(|s| s.to_string()),
                });
            }
            _ => {}
        }
    }

    // Fallback heuristics:
    // 1) If payload looks like { delta: "..." } assume assistant delta
    if payload.get("delta").and_then(Value::as_str).is_some() {
        return Some(UiEvent::AssistantDelta {
            session_key,
            run_id,
            message_id: payload.get("messageId").and_then(Value::as_str).unwrap_or("assistant").to_string(),
            delta: payload.get("delta").and_then(Value::as_str).unwrap_or("").to_string(),
        });
    }

    // 2) If payload has full text
    if payload.get("text").and_then(Value::as_str).is_some() {
        return Some(UiEvent::AssistantFinal {
            session_key,
            run_id,
            message_id: payload.get("messageId").and_then(Value::as_str).unwrap_or("assistant").to_string(),
            text: payload.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
            usage: None,
        });
    }

    None
}


⸻

5) A practical operator client state machine (tokio)

This is the shape you want in a standalone module, e.g. clawdbot_ws.rs.

5.1 Error type

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("WebSocket error: {0}")]
    Ws(#[from] tungstenite::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Gateway protocol error: {0}")]
    Protocol(String),

    #[error("Timeout")]
    Timeout,
}

5.2 The client struct

use std::{collections::HashMap, time::Duration};
use tokio::sync::{mpsc, oneshot};
use futures_util::{SinkExt, StreamExt};

pub struct ClawdbotWsClient {
    pub gateway_url: String,
    pub token: String,
    pub device_id: String,

    // UI events out leading to Tauri
    ui_tx: mpsc::Sender<UiEvent>,

    // pending RPCs
    pending: HashMap<String, oneshot::Sender<Result<serde_json::Value, WsError>>>,

    // For idempotent sends
    in_flight: HashMap<String, String>, // idempotencyKey -> runId
}

impl ClawdbotWsClient {
    pub fn new(gateway_url: String, token: String, device_id: String, ui_tx: mpsc::Sender<UiEvent>) -> Self {
        Self {
            gateway_url,
            token,
            device_id,
            ui_tx,
            pending: HashMap::new(),
            in_flight: HashMap::new(),
        }
    }
}

5.3 Run loop outline (connect + reconnect)

impl ClawdbotWsClient {
    pub async fn run_forever(mut self) {
        let mut backoff = Duration::from_millis(250);
        let max_backoff = Duration::from_secs(10);

        loop {
            match self.run_once().await {
                Ok(_) => backoff = Duration::from_millis(250),
                Err(e) => {
                    let _ = self.ui_tx.send(UiEvent::Disconnected { reason: e.to_string() }).await;
                    tokio::time::sleep(backoff).await;
                    backoff = std::cmp::min(max_backoff, backoff * 2);
                }
            }
        }
    }

    async fn run_once(&mut self) -> Result<(), ClientError> {
        let (ws_stream, _resp) = tokio_tungstenite::connect_async(&self.gateway_url).await?;
        let (mut write, mut read) = ws_stream.split();

        // Wait for connect.challenge
        let nonce = self.wait_for_challenge(&mut read).await?;

        // Send connect
        let connect_id = uuid::Uuid::new_v4().to_string();
        let connect = build_connect_req(
            connect_id.clone(),
            &self.token,
            &self.device_id,
            nonce.as_deref(),
            &["operator.read", "operator.write"],
        );

        write.send(tungstenite::Message::Text(serde_json::to_string(&connect)?)).await?;

        // Wait for connect response
        let protocol = self.wait_for_ok_response(&mut read, &connect_id).await?;
        let _ = self.ui_tx.send(UiEvent::Connected { protocol }).await;

        // After connect: start a ping/tick watchdog (optional)
        // After connect: subscribe to main session if you want immediate UI
        // (or subscribe when user selects a session)
        //
        // You may also request a sessions list here, if the gateway supports it.

        // Main event loop: multiplex inbound frames and outbound RPC requests
        loop {
            let msg = read.next().await.ok_or_else(|| ClientError::Protocol("WS closed".into()))??;
            if let tungstenite::Message::Text(txt) = msg {
                let frame: WsFrame = serde_json::from_str(&txt)?;
                self.handle_incoming_frame(&mut write, frame).await?;
            }
        }
    }

    async fn wait_for_challenge<S>(&self, read: &mut S) -> Result<Option<String>, ClientError>
    where
        S: StreamExt<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin,
    {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let msg = tokio::time::timeout(remaining, read.next()).await.map_err(|_| ClientError::Timeout)?;
            let msg = msg.ok_or_else(|| ClientError::Protocol("WS closed before challenge".into()))??;
            if let tungstenite::Message::Text(txt) = msg {
                let frame: WsFrame = serde_json::from_str(&txt)?;
                if let WsFrame::Event { event, payload, .. } = frame {
                    if event == "connect.challenge" {
                        let nonce = payload.get("nonce").and_then(|v| v.as_str()).map(|s| s.to_string());
                        return Ok(nonce);
                    }
                }
            }
        }
    }

    async fn wait_for_ok_response<S>(&mut self, read: &mut S, id: &str) -> Result<u32, ClientError>
    where
        S: StreamExt<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin,
    {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let msg = tokio::time::timeout(remaining, read.next()).await.map_err(|_| ClientError::Timeout)?;
            let msg = msg.ok_or_else(|| ClientError::Protocol("WS closed".into()))??;

            if let tungstenite::Message::Text(txt) = msg {
                let frame: WsFrame = serde_json::from_str(&txt)?;
                match frame {
                    WsFrame::Res { id: rid, ok, payload, error } if rid == id => {
                        if ok {
                            // The payload has protocol and other fields; parse what you need:
                            let protocol = payload.get("protocol").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
                            return Ok(protocol);
                        } else {
                            let e = error.unwrap_or(WsError {
                                code: "UNKNOWN".into(),
                                message: "connect failed".into(),
                                details: Value::Null,
                            });
                            return Err(ClientError::Protocol(format!("connect failed: {} {}", e.code, e.message)));
                        }
                    }
                    // ignore other frames until connect response
                    _ => {}
                }
            }
        }
    }

    async fn handle_incoming_frame<W>(&mut self, _write: &mut W, frame: WsFrame) -> Result<(), ClientError>
    where
        W: SinkExt<tungstenite::Message> + Unpin,
        W::Error: std::fmt::Debug,
    {
        match &frame {
            WsFrame::Res { id, ok, payload, error } => {
                if let Some(tx) = self.pending.remove(id) {
                    if *ok {
                        let _ = tx.send(Ok(payload.clone()));
                    } else {
                        let _ = tx.send(Err(error.clone().unwrap_or(WsError {
                            code: "UNKNOWN".into(),
                            message: "unknown error".into(),
                            details: Value::Null,
                        })));
                    }
                }
            }
            WsFrame::Event { .. } => {
                if let Some(ui) = normalize_event(&frame) {
                    let _ = self.ui_tx.send(ui).await;
                }
            }
            WsFrame::Req { .. } => {
                // Normally gateway won't send reqs to operator client.
            }
        }
        Ok(())
    }
}


⸻

6) RPC helper: send request and await response

This gives you a clean API for your Tauri commands (get_history, send_message, etc.).

impl ClawdbotWsClient {
    pub async fn rpc<W>(
        &mut self,
        write: &mut W,
        method: &str,
        params: serde_json::Value,
        timeout: std::time::Duration,
    ) -> Result<serde_json::Value, ClientError>
    where
        W: futures_util::Sink<tungstenite::Message> + Unpin,
        W::Error: std::fmt::Debug,
    {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.insert(id.clone(), tx);

        let req = WsFrame::Req { id: id.clone(), method: method.to_string(), params };
        write.send(tungstenite::Message::Text(serde_json::to_string(&req)?)).await?;

        let res = tokio::time::timeout(timeout, rx).await.map_err(|_| ClientError::Timeout)??;
        match res {
            Ok(payload) => Ok(payload),
            Err(e) => Err(ClientError::Protocol(format!("rpc {} failed: {} {}", method, e.code, e.message))),
        }
    }
}


⸻

7) Idempotent chat.send wrapper (important)

Because chat.send is async and retries return in_flight/ok, you should treat send as “submit and then follow events”.

pub fn make_idempotency_key(session_key: &str, client_msg_id: &str) -> String {
    // stable across retries of the same UI message
    format!("scrappy:{}:{}:{}", session_key, client_msg_id, uuid::Uuid::new_v4())
}

In your UI flow:
	•	generate client_msg_id per message
	•	generate idempotency key once
	•	store it in your local UI state
	•	if WS reconnect happens mid-run, you can re-issue chat.send with the same idempotencyKey
	•	gateway will tell you it’s in flight or already completed, and you’ll continue receiving events.

⸻

8) Tauri integration pattern (clean separation)

Background task
	•	spawn one ClawdbotWsClient::run_forever() task
	•	ui_tx is an mpsc::Sender<UiEvent>
	•	a separate forwarder pushes events into Tauri webview via app_handle.emit_all("clawdbot:event", ui_event_json)

Commands

Expose Tauri commands like:
	•	clawdbot_list_sessions()
	•	clawdbot_chat_history(session_key, before?)
	•	clawdbot_send(session_key, text, deliver=false)
	•	clawdbot_abort(session_key, run_id?)

These commands should send requests into the WS task via an internal channel (don’t let multiple tasks write to the WS sink).

Implementation tip: model WS task as an actor:
	•	enum Command { Rpc{...}, Subscribe{...}, … }
	•	actor owns the sink and pending map
	•	UI commands await oneshot responses

⸻

9) Anything else missing?

Yes—if you want to be “correct per docs” and future-proof:
	1.	Schema strategy: upstream protocol is defined in TypeBox and codegen exists. Long-term, generate Rust types or validate dynamically against schema. (You can ship MVP with the dynamic approach above, then harden.)
	2.	Approvals support: if you enable powerful tools, implement exec.approval.requested UI and resolve calls (operator.approvals scope).
	3.	Sessions API: you’ll want a proper sessions.list / session metadata fetch so your session sidebar works like the Control UI. (Wire method names can vary by protocol version; keep them dynamic and map to your UiSession model.)
	4.	Pairing / allowlists: for Telegram pairing and Slack channel allowlists, expose toggles and show “pending pairing” state (even if the gateway handles most of it).
	5.	Diagnostics export: bundle logs + redacted config + versions (huge for connector support tickets).

Here’s a ready-to-paste Slack App Manifest for Socket Mode that’s appropriate for a local-first Clawdbot/Moltbot-style assistant in Scrappy Inference (Slack + Telegram first). It includes the typical bot scopes and event subscriptions you’ll want for DMs + channel mentions + reactions, plus a /clawd slash command.

{
  "display_information": {
    "name": "Scrappy Inference (Clawdbot)",
    "description": "Local-first Clawdbot mode inside Scrappy Inference",
    "background_color": "#111111"
  },
  "features": {
    "bot_user": {
      "display_name": "ScrappyBot",
      "always_online": false
    },
    "app_home": {
      "home_tab_enabled": true,
      "messages_tab_enabled": true,
      "messages_tab_read_only_enabled": false
    },
    "slash_commands": [
      {
        "command": "/clawd",
        "description": "Send a message to Scrappy Clawdbot",
        "should_escape": false
      }
    ]
  },
  "oauth_config": {
    "scopes": {
      "bot": [
        "app_mentions:read",
        "channels:history",
        "channels:read",
        "chat:write",
        "commands",
        "emoji:read",
        "files:read",
        "files:write",
        "groups:history",
        "groups:read",
        "groups:write",
        "im:history",
        "im:read",
        "im:write",
        "mpim:history",
        "mpim:read",
        "mpim:write",
        "pins:read",
        "pins:write",
        "reactions:read",
        "reactions:write",
        "users:read"
      ],
      "user": [
        "channels:history",
        "channels:read",
        "emoji:read",
        "groups:history",
        "groups:read",
        "im:history",
        "im:read",
        "mpim:history",
        "mpim:read",
        "pins:read",
        "reactions:read",
        "search:read",
        "users:read"
      ]
    }
  },
  "settings": {
    "socket_mode_enabled": true,
    "event_subscriptions": {
      "bot_events": [
        "app_mention",
        "message.channels",
        "message.groups",
        "message.im",
        "message.mpim",
        "reaction_added",
        "reaction_removed",
        "pin_added",
        "pin_removed"
      ]
    }
  }
}

Notes for your Settings UI (so users don’t get stuck)
	•	Socket Mode requires an App-Level Token (xapp-…) with the scope connections:write (users create this under Basic Information → App-Level Tokens).
	•	You also need a Bot Token (xoxb-…) after installing the app to the workspace.
	•	The optional User Token (xoxp-…) is only needed if you want richer history/search behaviors; you can omit it for a DM-only MVP.

