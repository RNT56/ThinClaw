# Implementation Plan - Multi-Agent OpenClaw GUI

## Objective
Enable connection to multiple OpenClaw gateways simultaneously (local + multiple remotes) and provide a revamped GUI for orchestration and task management.

## 1. Backend Refactoring (Rust)

### A. Configuration Updates (`src-tauri/src/openclaw/config.rs`)
- Define `AgentProfile` struct:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
  pub struct AgentProfile {
      pub id: String,
      pub name: String, // e.g., "Local Core", "GPU Server 1"
      pub url: String,  // ws://...
      pub token: Option<String>,
      pub mode: String, // "local" | "remote"
      pub auto_connect: bool,
  }
  ```
- Add `profiles: Vec<AgentProfile>` to `OpenClawIdentity`.
- Migrate existing `remote_url`/`gateway_mode` to a default profile if `profiles` is empty.

### B. Connection Management (`src-tauri/src/openclaw/mod.rs` & `commands.rs`)
- Update `OpenClawManager` struct to hold multiple WebSocket clients:
  - `clients: Arc<RwLock<HashMap<String, WsClient>>>`
- Implement `connect_agent(profile_id)` and `disconnect_agent(profile_id)` commands.
- Update `openclaw_get_status` to return status for *all* configured profiles.

### C. Task Orchestration
- Create a `Task` struct/enum for dispatching commands (e.g., "Analyze this file", "Run shell command").
- Implement `dispatch_task(agent_id, task)` command.

## 2. Frontend Revamp (React/Tauri)

### A. New "Agents" Dashboard
- Create `src/pages/AgentsPage.tsx` (or similar main view).
- **Layout:**
  - **Sidebar:** List of all agents with live status indicators (Green/Red dots).
  - **Main Area:** Tabs for the selected agent:
    - **Overview:** System stats (CPU/RAM from `system_get_specs` equivalent on remote), consolidated logs.
    - **Terminal:** Remote shell access.
    - **Tasks:** Queue of active/past tasks.
    - **Config:** Agent-specific settings (allowed tools, etc.).

### B. "Add Agent" Workflow
- Integrate `RemoteDeployWizard` as a "Deploy New" option in the "Add Agent" modal.
- "Connect Existing" option for manual IP/Token entry.

### C. Multi-Agent Chat/Tasking
- Update Chat interface to allow selecting *which* agent executes a prompt, or "Auto" (router).
- Allow broadcasting commands (e.g., "Update all servers").

## 3. Migration Strategy
1. **Phase 1 (Data):** Update `OpenClawConfig` to support the profile list. Ensure backward compatibility by migrating the single `remote_url` to a profile.
2. **Phase 2 (UI):** Build the `AgentManager` UI side-by-side with current settings.
3. **Phase 3 (Logic):** Refactor the actual `WsClient` handling to support concurrency.

## 4. Execution Steps
1. Modify `config.rs`: Add `AgentProfile` and `profiles` list.
2. Create `migrations.rs` (or logic in `load_config`) to migrate old config.
3. Update `GatewayTab` to show the list of agents instead of just one toggle.
4. Implement `AgentManager` component.
5. Refactor `ws_client` management in Rust.
