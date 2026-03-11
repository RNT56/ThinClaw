# Implementation Plan: Full Remote Deployment via Scrappy Gateway UI

> How to make the "Remote" mode in Scrappy's Gateway Settings fully functional —
> from UI button click to a live remote IronClaw agent that Scrappy controls.

---

## Executive Summary

The Scrappy Gateway UI already has **extensive remote deployment UI** (deploy wizard,
connection profiles, mode switching). The backend also has scaffolding (deploy script
runner, profile CRUD, gateway settings persistence). But the critical middle layer
— **actually connecting Scrappy to a remote IronClaw instance** — is entirely missing.

This plan covers:
1. What should be deployed on the remote server (Docker vs binary)
2. How Scrappy connects to the remote agent as a client
3. Implementation steps, ordered by priority

---

## Part 1: What Gets Deployed — Docker vs. Binary

### Decision: **Docker Compose is the right choice**

| Factor | Docker | Direct Binary |
|---|---|---|
| Cross-platform | ✅ Any Linux server | ⚠️ Must compile per arch |
| Reproducibility | ✅ Exact same environment | ❌ Depends on system libs |
| Database included | ✅ PostgreSQL in compose | ❌ Must install separately |
| Update workflow | `docker-compose pull && up -d` | Rebuild from source |
| Resource isolation | ✅ Container limits | ❌ Shares system resources |
| Sandboxed jobs | ✅ Docker-in-Docker ready | ⚠️ Need Docker anyway |
| Complexity | Low (one `docker-compose up`) | Medium (cargo, deps, config) |

### What Docker Compose Deploys

```yaml
# ironclaw-remote/docker-compose.yml
version: '3.8'
services:
  ironclaw:
    build:
      context: .
      dockerfile: Dockerfile
    ports:
      - "18789:18789"  # Gateway API
    environment:
      - GATEWAY_HOST=0.0.0.0
      - GATEWAY_PORT=18789
      - GATEWAY_AUTH_TOKEN=${GATEWAY_AUTH_TOKEN}
      - DATABASE_BACKEND=postgres
      - DATABASE_URL=postgres://ironclaw:ironclaw@postgres:5432/ironclaw
      # LLM keys injected from Scrappy at connection time
    depends_on:
      - postgres
    restart: unless-stopped

  postgres:
    image: pgvector/pgvector:pg15
    environment:
      POSTGRES_DB: ironclaw
      POSTGRES_USER: ironclaw
      POSTGRES_PASSWORD: ironclaw
    volumes:
      - pgdata:/var/lib/postgresql/data

volumes:
  pgdata:
```

### Why Not Ansible?

Ansible is overkill for the default flow. It adds:
- Local Ansible installation requirement
- SSH key management complexity
- Playbook maintenance burden

Ansible should remain a **power-user option** documented separately, not the default UI flow.
The primary UI flow should be: **SSH → docker-compose up → connect**.

---

## Part 2: Architecture — How Scrappy Connects to Remote IronClaw

### Current Architecture (Local Only)

```
┌─────────────────────────────────────────┐
│              Scrappy (Tauri)            │
│                                         │
│  Frontend ←→ Tauri IPC ←→ ironclaw_bridge.rs
│                                │         │
│                         TauriChannel     │
│                                │         │
│                     IronClaw Agent       │
│                     (in-process)         │
└─────────────────────────────────────────┘
```

### Target Architecture (Remote Mode)

```
┌──────────────────────────────┐     HTTPS/WSS      ┌───────────────────────────┐
│       Scrappy (Tauri)        │  ←──────────────→  │   Remote Server           │
│                              │                     │                           │
│  Frontend ←→ Tauri IPC      │                     │  IronClaw Binary          │
│       ←→ RemoteGatewayProxy │                     │  ├── Gateway (Axum)       │
│                              │                     │  │   ├── /api/chat/send   │
│  ┌───────────────────────┐  │                     │  │   ├── /api/chat/events  │
│  │ RemoteGatewayProxy    │  │   HTTP POST/SSE     │  │   ├── /api/chat/ws      │
│  │ - Forwards invoke()   │──┼──────────────────→  │  │   └── /api/health       │
│  │   to HTTP API         │  │                     │  ├── Telegram Channel      │
│  │ - Subscribes SSE for  │←─┼──────────────────── │  ├── Discord Channel       │
│  │   events              │  │                     │  └── Database (PG)         │
│  └───────────────────────┘  │                     └───────────────────────────┘
└──────────────────────────────┘
```

**Key insight**: Scrappy does NOT embed a second IronClaw agent in remote mode.
Instead, `ironclaw_bridge.rs` creates a **proxy** that forwards Tauri IPC calls
to the remote gateway's HTTP API. The frontend is unchanged.

---

## Part 3: Implementation Plan

### Phase 0: Pre-requisites & Preparation

#### 0.1 Create a dedicated deploy bundle

Create `ironclaw/deploy/` containing:
- `Dockerfile` (copy of existing, verified working)
- `docker-compose.yml` (with PostgreSQL + IronClaw services)
- `.env.template` (minimal config — gateway token auto-generated)
- `setup.sh` (simple script: generate token, docker-compose up)

> **Files:** `ironclaw/deploy/Dockerfile`, `docker-compose.yml`, `.env.template`, `setup.sh`

#### 0.2 Verify the standalone gateway works end-to-end

Before wiring Scrappy, manually verify:
```bash
cd ironclaw/
GATEWAY_HOST=0.0.0.0 GATEWAY_PORT=18789 GATEWAY_AUTH_TOKEN=test123 \
ANTHROPIC_API_KEY=sk-... cargo run -- run
```
Then test: `curl -H "Authorization: Bearer test123" http://localhost:18789/api/health`

---

### Phase 1: Remote Deployment from Scrappy UI (SSH → Docker)

#### 1.1 Implement `deploy_via_ssh` backend command

Replace the Ansible-based `openclaw_deploy_remote` with a simpler SSH-based Docker deployment:

```rust
// backend/src/openclaw/deploy.rs

pub async fn openclaw_deploy_remote(
    app: AppHandle,
    ip: String,
    user: String,
) -> Result<(), String> {
    // 1. Generate a random GATEWAY_AUTH_TOKEN
    let token = uuid::Uuid::new_v4().to_string();
    
    // 2. SCP the deploy bundle to the server
    //    scp -r ironclaw/deploy/ user@ip:~/ironclaw-agent/
    
    // 3. SSH and run setup
    //    ssh user@ip 'cd ~/ironclaw-agent && GATEWAY_AUTH_TOKEN={token} docker-compose up -d --build'
    
    // 4. Emit events for each step (stdout/stderr → deploy-log)
    
    // 5. On success, emit the connection details (URL + token)
    
    // 6. Auto-create a profile with the generated token
}
```

**Complexity:** Medium. The trickiest part is SSH key handling (we rely on the user having
SSH keys configured, same as the existing Ansible approach).

**Files to modify:**
- `backend/src/openclaw/deploy.rs` — rewrite with Docker approach
- `ironclaw/deploy/` — create bundle directory

#### 1.2 Update RemoteDeployWizard UI

The existing wizard UI (`RemoteDeployWizard.tsx`) is already well-structured. Minor updates:

- Show the generated auth token after successful deploy
- Auto-populate the "Connect Existing" form with deployment results
- Add Docker status check step (verify port is accessible)

**Files:** `frontend/src/components/openclaw/RemoteDeployWizard.tsx`

---

### Phase 2: Connect to Remote Agent (Critical Path) 

This is the core missing piece — making Scrappy actually communicate with a remote agent.

#### 2.1 Create `RemoteGatewayProxy` in the backend

This is a new module that acts as a client to the remote IronClaw gateway:

```rust
// backend/src/openclaw/remote_proxy.rs

/// HTTP/SSE client that proxies Tauri IPC commands to a remote IronClaw gateway.
pub struct RemoteGatewayProxy {
    base_url: String,      // e.g. "http://192.168.1.50:18789"
    auth_token: String,    // Bearer token
    client: reqwest::Client,
    sse_handle: Option<tokio::task::JoinHandle<()>>,
}

impl RemoteGatewayProxy {
    pub fn new(url: &str, token: &str) -> Self { ... }
    
    /// Test connectivity: GET /api/health
    pub async fn health_check(&self) -> Result<bool, String> { ... }
    
    /// Send a chat message: POST /api/chat/send
    pub async fn send_message(&self, session: &str, text: &str) -> Result<Value, String> { ... }
    
    /// Get sessions: GET /api/chat/sessions
    pub async fn get_sessions(&self) -> Result<Vec<Session>, String> { ... }
    
    /// Get history: GET /api/chat/history/{session}
    pub async fn get_history(&self, session: &str, limit: u32) -> Result<Vec<Message>, String> { ... }
    
    /// Subscribe to SSE: GET /api/chat/events
    /// Forwards events as Tauri events (openclaw-event)
    pub async fn subscribe_sse(&mut self, app_handle: AppHandle) -> Result<(), String> { ... }
    
    /// Get diagnostics: GET /api/diagnostics
    pub async fn get_diagnostics(&self) -> Result<Value, String> { ... }
    
    /// Forward arbitrary API calls
    pub async fn proxy_get(&self, path: &str) -> Result<Value, String> { ... }
    pub async fn proxy_post(&self, path: &str, body: Value) -> Result<Value, String> { ... }
}
```

**This is the single most important piece.** Every Tauri command that currently talks to
the in-process `IronClawState` needs a code path that instead talks to this proxy.

**Files:** New `backend/src/openclaw/remote_proxy.rs`

#### 2.2 Add `RemoteGatewayProxy` to `IronClawState`

Modify the state to hold either a local engine OR a remote proxy:

```rust
// backend/src/openclaw/ironclaw_bridge.rs

pub struct IronClawState {
    /// Local engine — None when stopped or in remote mode
    inner: RwLock<Option<IronClawInner>>,
    /// Remote proxy — None when local or disconnected
    remote: RwLock<Option<RemoteGatewayProxy>>,
    /// Current mode
    mode: RwLock<GatewayMode>,  // Local | Remote
    // ...
}

enum GatewayMode { Local, Remote }
```

#### 2.3 Implement `openclaw_test_connection` backend command

```rust
#[tauri::command]
pub async fn openclaw_test_connection(
    url: String,
    token: Option<String>,
) -> Result<bool, String> {
    let client = reqwest::Client::new();
    let mut req = client.get(format!("{}/api/health", url));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    match req.send().await {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(e) => Err(format!("Connection failed: {}", e)),
    }
}
```

**Files:** `backend/src/openclaw/commands/gateway.rs`

#### 2.4 Modify `openclaw_start_gateway` to handle remote mode

```rust
pub async fn openclaw_start_gateway(
    state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
    // ...
) -> Result<(), String> {
    let config = state.get_config().await;
    let mode = config.as_ref().map(|c| c.gateway_mode.as_str()).unwrap_or("local");
    
    match mode {
        "local" => {
            // Existing code: start in-process engine
            ironclaw.start(secrets_store).await?;
        }
        "remote" => {
            let url = config.as_ref().and_then(|c| c.remote_url.clone())
                .ok_or("No remote URL configured")?;
            let token = config.as_ref().and_then(|c| c.remote_token.clone())
                .unwrap_or_default();
            
            // Create proxy and connect
            let mut proxy = RemoteGatewayProxy::new(&url, &token);
            proxy.health_check().await?;
            proxy.subscribe_sse(app_handle.clone()).await?;
            
            ironclaw.set_remote(proxy).await;
        }
        _ => return Err(format!("Unknown gateway mode: {}", mode)),
    }
    Ok(())
}
```

#### 2.5 Route Tauri commands through proxy in remote mode

Every command handler needs a branching check. For the most critical ones:

```rust
// Example: openclaw_send_message
pub async fn openclaw_send_message(
    ironclaw: State<'_, IronClawState>,
    session_key: String,
    text: String,
    deliver: bool,
) -> Result<OpenClawRpcResponse, String> {
    if let Some(proxy) = ironclaw.remote().await {
        // Remote mode: HTTP POST to remote gateway
        proxy.send_message(&session_key, &text).await
    } else {
        // Local mode: existing in-process path
        let guard = ironclaw.inner.read().await;
        // ... existing code
    }
}
```

**Commands that need remote routing** (priority order):
1. `openclaw_send_message` — chat/send
2. `openclaw_get_sessions` — chat/sessions
3. `openclaw_get_history` — chat/history/{session}
4. `openclaw_subscribe_session` — SSE subscription
5. `openclaw_abort_chat` — chat/abort
6. `openclaw_cron_list` / `openclaw_cron_run` — routines
7. `openclaw_skills_list` — skills
8. `openclaw_get_file` / `openclaw_write_file` — workspace
9. `openclaw_logs_tail` — logs
10. `openclaw_system_presence` — presence/status

**Files:** All files in `backend/src/openclaw/commands/`

---

### Phase 3: API Key Forwarding (Security Bridge)

When in remote mode, the user's API keys (configured via Scrappy's UI) need to reach
the remote IronClaw agent.

#### 3.1 Add key injection endpoint to IronClaw gateway

```rust
// ironclaw/src/channels/web/server.rs — new route

POST /api/config/secrets
Body: { "anthropic_api_key": "sk-...", "openai_api_key": "sk-..." }
Auth: Bearer token required

// Injects keys into the running config overlay (env vars)
// Requires engine restart to take effect
```

#### 3.2 Scrappy sends keys on connect

After `RemoteGatewayProxy::subscribe_sse()` succeeds, iterate over all granted
secrets from the Keychain and POST them to the remote:

```rust
let keys = keychain_adapter.get_all_granted_keys();
proxy.proxy_post("/api/config/secrets", json!(keys)).await?;
```

> **Security note:** Keys travel over the network. For production, this requires:
> - HTTPS/WSS (TLS) between Scrappy and the remote gateway
> - Or a VPN tunnel (Tailscale)
> - The auth token itself must be strong (UUID v4 or better)

---

### Phase 4: Profile Management & Multi-Agent

#### 4.1 Implement `openclaw_switch_to_profile`

Currently frontend calls this but no backend exists:

```rust
#[tauri::command]
pub async fn openclaw_switch_to_profile(
    state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
    profile_id: String,
) -> Result<(), String> {
    let config = state.get_config().await.ok_or("Config not ready")?;
    let profile = config.profiles.iter()
        .find(|p| p.id == profile_id)
        .ok_or("Profile not found")?;
    
    // Stop current connection (local or remote)
    ironclaw.stop().await;
    
    // Update gateway settings to this profile
    state.update_gateway_settings(
        profile.mode.clone(),
        Some(profile.url.clone()),
        profile.token.clone(),
    ).await?;
    
    // Start with new settings
    openclaw_start_gateway(state, ironclaw, ...).await?;
    
    Ok(())
}
```

#### 4.2 Implement `openclaw_get_fleet_status`

For monitoring multiple remote agents:

```rust
#[tauri::command]
pub async fn openclaw_get_fleet_status(
    state: State<'_, OpenClawManager>,
) -> Result<Vec<AgentStatusSummary>, String> {
    let config = state.get_config().await.ok_or("Config not ready")?;
    
    let mut results = Vec::new();
    for profile in &config.profiles {
        if profile.mode == "remote" {
            let client = reqwest::Client::new();
            let health = client.get(format!("{}/api/health", profile.url))
                .bearer_auth(profile.token.as_deref().unwrap_or(""))
                .timeout(Duration::from_secs(3))
                .send()
                .await;
            
            results.push(AgentStatusSummary {
                id: profile.id.clone(),
                name: profile.name.clone(),
                url: profile.url.clone(),
                online: health.map(|r| r.status().is_success()).unwrap_or(false),
                // ... fill from /api/diagnostics if online
            });
        }
    }
    Ok(results)
}
```

---

## Phase Summary & Timeline

| Phase | Scope | Effort | Dependencies |
|---|---|---|---|
| **Phase 0** | Deploy bundle + verification | 1-2 days | None |
| **Phase 1** | SSH deploy from UI (Docker) | 2-3 days | Phase 0 |
| **Phase 2** | RemoteGatewayProxy + wiring | 5-7 days | Phase 0 |
| **Phase 3** | API key forwarding | 1-2 days | Phase 2 |
| **Phase 4** | Profile switching + fleet | 2-3 days | Phase 2 |

**Total estimate: ~2-3 weeks**

**Phase 2 is the critical path.** Without the `RemoteGatewayProxy`, everything else
(deploy, connect, profiles) is just storing config values that nothing reads.

---

## Implementation Order (Recommended)

```
Phase 0 → Phase 2 (core proxy) → Phase 1 (deploy) → Phase 3 (keys) → Phase 4 (profiles)
```

Rationale: Build the proxy first so you can test with a manually-deployed remote agent.
Then add the automated deploy. Then layer on key forwarding and profile management.

---

## Gateway API Mapping

Complete mapping of Tauri commands → remote gateway API endpoints:

| Tauri Command | Remote Gateway Endpoint | Method |
|---|---|---|
| `openclaw_get_status` | `/api/health` + `/api/diagnostics` | GET |
| `openclaw_start_gateway` | N/A (proxy connects) | — |
| `openclaw_stop_gateway` | N/A (proxy disconnects) | — |
| `openclaw_send_message` | `/api/chat/send` | POST |
| `openclaw_get_sessions` | `/api/chat/sessions` | GET |
| `openclaw_get_history` | `/api/chat/history/{session}` | GET |
| `openclaw_subscribe_session` | `/api/chat/events` (SSE) | GET |
| `openclaw_abort_chat` | `/api/chat/abort` | POST |
| `openclaw_cron_list` | `/api/routines` | GET |
| `openclaw_cron_run` | `/api/routines/{key}/run` | POST |
| `openclaw_skills_list` | `/api/skills` | GET |
| `openclaw_get_file` | `/api/memory/{path}` | GET |
| `openclaw_write_file` | `/api/memory/{path}` | PUT |
| `openclaw_logs_tail` | `/api/logs/tail` | GET |
| `openclaw_system_presence` | `/api/diagnostics` | GET |
| `openclaw_test_connection` | `/api/health` | GET |
| `openclaw_tools_list` | `/api/tools` | GET |
| `openclaw_extensions_list` | `/api/extensions` | GET |
| `openclaw_hooks_list` | `/api/hooks` | GET |

---

## Open Questions

1. **TLS/HTTPS**: Should the default deploy bundle include self-signed certs or
   require the user to set up a reverse proxy (nginx/caddy)? Recommendation:
   Include a Caddy sidecar for automatic HTTPS with Let's Encrypt.

2. **WebSocket vs SSE**: The remote proxy needs real-time streaming. SSE is simpler
   (one-way), WebSocket is more powerful (bi-directional). The gateway already
   supports both. Recommendation: Start with SSE for simplicity; add WS later
   for features like live typing indicators.

3. **Secrets storage on remote**: Currently `.env` file. Should we add an endpoint
   for Scrappy to push secrets to the remote's database encrypted store?
   Recommendation: Yes, using the existing `SecretsStore` trait.

4. **Multiple Scrappy clients**: Can two Scrappy instances connect to the same remote
   agent? The gateway already supports this (session-based isolation). The main
   concern is auth token management. Recommendation: Allow it — each client
   uses the same token but different session keys.
