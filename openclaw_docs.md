# OpenClaw Implementation & Security Documentation

OpenClaw is an integrated agentic cockpit within Scrappy, powered by the **OpenClawEngine** ecosystem. It provides a secure, interactive interface for multi-channel communication, tool execution, and local/remote model orchestration.

---

## 1. Architectural Overview

OpenClaw utilizes a **Tauri Sidecar Pattern** to bridge the frontend UI with a specialized Node.js-based agent engine.

### Core Components:
- **Tauri Host (Rust)**: Manages the lifecycle of the sidecar process, handles binary execution, and provides a secure IPC bridge.
- **OpenClawEngine Sidecar (Node.js)**: A bundled runtime that executes the agent logic, manages WebSocket gateways, and interfaces with external APIs (Slack, Telegram, Anthropic).
- **ACP (Agent Client Protocol)**: The standard protocol used for event streaming, tool requests, and status updates between the UI and the agent.
- **WebSocket Gateway**: A lightweight server (running on loopback) that facilitates real-time communication.

---

## 2. Setup & Connection Details

### Initial Configuration
Upon first launch, Scrappy generates a unique **Identity** for the device (`identity.json`):
- **Device ID**: A persistent UUID used for cryptographic representation.
- **Auth Token**: A 32-character high-entropy token generated locally for secure Gateway access.
- **Gateway Port**: Dynamically allocated (defaults to `18789`) to avoid conflicts.

### Connection Modes
1.  **Local Mode (Default)**: Scrappy spawns its own internal gateway. Communication happens via `ws://127.0.0.1:[PORT]`.
2.  **Remote Mode**: Allows the Scrappy UI to connect to a OpenClawEngine instance running on a different server (useful for 24/7 bots). Requires a Remote URL and matching Remote Token.

### Session Lifecycle
- **New Session**: Generates a unique `session_key`. This key is used to route messages and maintain separate memory contexts for different tasks.
- **History Sync**: On session load, the UI fetches message history via the `chat.history` RPC call to the gateway.

---

## 3. Security Framework

OpenClaw is designed with a defense-in-depth model to ensure that the AI remains a tool, not a threat.

### Principle of Least Privilege (No Root)
- **User-Space Execution**: The sidecar process runs with the exact permissions of the logged-in OS user. It **never** requests or requires root/sudo access for standard operations.
- **Process Isolation**: The agent is isolated within its own runtime, and all host-level actions (shell execution, file writes) are mediated by the security policy.

### Network Isolation
- **Loopback Binding**: The gateway binds exclusively to `127.0.0.1`. This makes the agent's control channel physically inaccessible from the local network or the internet.
- **mDNS Disabled**: Network discovery is turned off by default to prevent device fingerprinting.

### Authentication & Authorization
- **Token-Based Auth**: Every WebSocket connection must provide the generated `auth_token`. Without this token, the gateway will reject the handshake.
- **Cryptographic Signing (Ed25519)**: Transmissions are signed using device-specific private keys, ensuring authenticity even in remote connection scenarios.

### ACP Approval Gate (Human-in-the-Loop)
Sensitive tools trigger a `RequiredApproval` flow:
1.  The agent requests a tool (e.g., `bash_exec`).
2.  The gateway pauses execution and sends an event to the Scrappy UI.
3.  The user is presented with the exact command/action.
4.  **Only** upon explicit user "Approve" does the command execute.

### Tool-Level Policies
Each session can be governed by an **Effective Tool Policy**:
- **Deny List**: High-risk tools (like `gateway_mode_switch` or `agents_list`) are blocked from subagents by default.
- **Allow List**: Specific capabilities (like `web_search` or `read_file`) can be granted per-session.

---

## 4. Integration Details

### External Channels
- **Slack**: Supports Socket Mode for real-time DM and channel interaction without public endpoints.
- **Telegram**: Uses the Telegram Bot API with group mention requirements enabled by default.

### Model Support
- **Cloud Models**: Native support for Anthropic (Claude 3.5/Opus).
- **Local LLMs**: Integration with the Scrappy local model sidecar (llama.cpp) via internal endpoints, providing a fully private experience.

### Filesystem & Persistence
- **State Directory**: Path: `~/Library/Application Support/scrappy cursor/OpenClaw/state`. Contains encrypted keys and configuration.
- **Workspace**: Each agent has a dedicated workspace directory for file manipulation, separate from your main project files.
