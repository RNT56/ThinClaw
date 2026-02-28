# Networking Architecture: Tailscale-First Connectivity

This document defines the complete network topology for ThinClaw in all deployment modes. It answers the core questions: **How do components find each other? How is the connection secured? How does service discovery work?**

---

## 1. Guiding Principle: Tailscale as the Default Private Mesh

ThinClaw strongly recommends **Tailscale** as the private networking layer for any scenario where the Orchestrator runs on a different machine than the Tauri UI.

**Why Tailscale over raw WireGuard, port-forwarding, or VPN?**

| Requirement | Tailscale | Raw WireGuard | Port-Forward |
|---|---|---|---|
| NAT traversal (works behind home routers) | ✅ Automatic | ❌ Requires static IP | ❌ Requires port-forward |
| No public IP needed | ✅ | ❌ | ❌ |
| Encrypted by default | ✅ WireGuard underneath | ✅ | ❌ |
| Human-readable stable hostname | ✅ MagicDNS | ❌ | ❌ |
| Works on mobile (iOS, Android) | ✅ | Complex | ❌ |
| SSH key management | ✅ via `tssh` | ❌ | ❌ |
| Zero configuration for end users | ✅ | ❌ | ❌ |

Tailscale's **MagicDNS** gives every device a stable hostname like `thinclaw-vps.yourtailnet.ts.net`. This hostname works even when the VPS changes IP addresses and is the cornerstone of ThinClaw's service discovery.

> **Requirement:** Tailscale is a **first-class recommended dependency** for Remote Mode. It is not required for Local Mode (Orchestrator and Tauri share the same process). Users who cannot or will not use Tailscale can use manual connection strings as a fallback.

---

## 2. How OpenClaw Does It (Reference)

OpenClaw's companion apps discover the Gateway using a combination of:
1. **mDNS (Multicast DNS)** via the `@homebridge/ciao` library — the Node.js gateway broadcasts itself on the local LAN as e.g. `_openclaw._tcp.local`. The macOS companion app uses `NetServiceBrowser` to find it automatically on the same Wi-Fi network.
2. **Manual IP Entry** for remote (non-LAN) connections.
3. **TLS Certificate Pinning** — the gateway generates a self-signed TLS cert on first boot. The companion app trusts this cert's fingerprint after initial pairing.

OpenClaw does **not** support automatic internet-facing discovery. Users deploying to a remote VPS must manually input the server IP.

---

## 3. ThinClaw Discovery: Multiple Methods, Ranked by Preference

ThinClaw supports three connection methods. The Tauri UI walks the user through them in priority order:

### Method A: Tailscale MagicDNS (Recommended) 🟢

**Prerequisites:** Both the local MacBook (Tauri app) and the remote machine (Orchestrator) are logged into the same Tailscale account (or a shared Tailnet).

**Flow:**
1. The Remote Orchestrator registers itself on Tailscale at boot. Tailscale assigns it the hostname `thinclaw-server.yourtailnet.ts.net`.
2. The Rust Orchestrator registers its port via the **Tailscale local API** (`http://localhost:41112`) to advertise the service.
3. The Tauri app queries the Tailscale local API for all devices in the tailnet tagged as `thinclaw`.
4. The UI displays a simple dropdown: **"Found: thinclaw-server (Mac Mini at home)"**.
5. User clicks. The Tauri app connects to `ws://thinclaw-server.yourtailnet.ts.net:7878/ws`.

The Tailscale tunnel handles all encryption (WireGuard), so the WebSocket connection can be plain `ws://` (not `wss://`) without sacrificing security. Tailscale encrypts the entire link at the network layer.

**Tagging in Tailscale Policy (`tailscale.json`):**
```json
{
  "tagOwners": {
    "tag:thinclaw-server": ["autogroup:member"]
  },
  "acls": [
    {
      "action": "accept",
      "src": ["tag:thinclaw-server", "autogroup:member"],
      "dst": ["tag:thinclaw-server:7878"]
    }
  ]
}
```

### Method B: QR Code Pairing (No Tailscale) 🟡

For users who prefer not to use Tailscale. This replicates OpenClaw's pairing UX.

**Flow:**
1. The headless Orchestrator starts and exposes a one-time pairing endpoint on `https://SERVER_IP:7879/pair`.
2. It prints a QR code to stdout encoding a JSON pairing payload: `{"host": "https://SERVER_IP:7878", "cert_fingerprint": "sha256:abc123...", "pairing_token": "one-time-token"}`.
3. The Tauri app (or phone camera) scans the QR code.
4. The Tauri app uses the `pairing_token` to authenticate once and receive a long-lived `session_token`.
5. All subsequent connections use `wss://SERVER_IP:7878` with TLS verification against the pinned `cert_fingerprint`.

The server's self-signed TLS cert is generated on first boot (via `rcgen` crate) and its fingerprint is embedded in the QR code payload.

### Method C: Manual Entry 🔴 (Last Resort)

The user manually types a connection string: `ws://IP_OR_HOSTNAME:PORT`. The Tauri app prompts for the Orchestrator's auth token separately. This is primarily for advanced/developer use.

---

## 4. The WebSocket Protocol (Client ↔ Orchestrator)

All communication between the Tauri Thin Client and the Remote Orchestrator flows through a single persistent WebSocket connection.

### Connection URL
```
ws://thinclaw-server.yourtailnet.ts.net:7878/ws?token=SESSION_TOKEN
```

### Authentication
On connect, the Tauri client sends the `session_token` as a query parameter. The Orchestrator validates it. If invalid, it closes the connection with `4001 Unauthorized`.

### Message Envelope Format (JSON)
All messages share a common envelope:
```json
{
  "id":      "uuid-v4",
  "type":    "message.send | tool.rpc | config.set | model.list | ...",
  "payload": { ... }
}
```

**Message Types:**

| Type | Direction | Purpose |
|---|---|---|
| `message.send` | Client → Orch | User chat message |
| `message.delta` | Orch → Client | Streaming token from LLM |
| `message.done` | Orch → Client | End of LLM response |
| `model.list.request` | Client → Orch | Ask for available models |
| `model.list.response` | Orch → Client | Return list of models |
| `config.set` | Client → Orch | Change a config value (e.g., active model) |
| `secret.set` | Client → Orch | Securely transmit a new API key |
| `tool.rpc.request` | Orch → Client | Request hardware bridge action (screenshot, audio) |
| `tool.rpc.response` | Client → Orch | Return hardware bridge result |
| `status.heartbeat` | Orch → Client | Keep-alive + uptime info |
| `version.handshake` | Both | Exchange protocol versions on connect |

### Version Handshake (Q5: Version Sync)
Immediately after authentication, both sides exchange versions:
```json
// Client → Orchestrator
{ "id": "...", "type": "version.handshake", "payload": { "ui_version": "2026.2.27", "protocol_version": 3 } }

// Orchestrator → Client
{ "id": "...", "type": "version.handshake", "payload": { "orchestrator_version": "2026.2.27", "protocol_version": 3 } }
```
If `protocol_version` values mismatch, the Orchestrator closes the connection with `4002 Protocol Mismatch` and the Tauri UI shows a banner: **"Your ThinClaw app is outdated. Please update to version X."**

---

## 5. API Key Transmission (Q4: Adding Keys to Remote Orchestrator)

The user enters an API key in the Tauri UI Settings pane. This key must be securely transmitted to the Remote Orchestrator (which will store it in the headless server's keychain) **without** being stored locally:

```
User Input in Tauri → secret.set WebSocket message → Remote Orchestrator → headless Keychain
```

The explicit design decision: **The Tauri Thin Client NEVER stores API keys locally when in Remote Mode.** It only transmits them. The source of truth is the Orchestrator's keychain.

**Message Flow:**
```json
// Client → Orchestrator (over Tailscale-encrypted WS)
{
  "id": "...",
  "type": "secret.set",
  "payload": {
    "provider": "openai",
    "secret": "sk-proj-..."
  }
}

// Orchestrator stores via keyring crate, responds:
{
  "id": "...",
  "type": "secret.set.ack",
  "payload": { "success": true }
}
```

The key travels over a Tailscale (WireGuard) encrypted tunnel, providing transport-layer security. Even if the WS is `ws://` not `wss://`, the underlying Tailscale link is encrypted.

---

## 6. Auto-Update Strategy (Q5: Keeping Versions in Sync)

**Local Mode:** Tauri auto-updater (built-in Tauri feature, checks GitHub Releases via `tauri-plugin-updater`) updates both UI and Rust backend as a single bundle.

**Remote Mode (Orchestrator on VPS):**

The Remote Orchestrator checks for new releases on startup and every 24 hours using the GitHub Releases API:

```rust
async fn check_for_updates(current_version: &str) -> Option<String> {
    let resp = reqwest::get("https://api.github.com/repos/yourorg/thinclaw/releases/latest").await.ok()?;
    let json: serde_json::Value = resp.json().await.ok()?;
    let latest = json["tag_name"].as_str()?;
    if latest != current_version { Some(latest.to_string()) } else { None }
}
```

If a new version is found, the Orchestrator:
1. Downloads the new binary from GitHub Releases (using `self_update` crate).
2. Replaces itself in-place.
3. Sends a `status.restart_pending` WebSocket message to the Tauri Thin Client.
4. Restarts cleanly (systemd/launchd restarts the service automatically).

**The Tauri Thin Client is updated separately**, but the version handshake on reconnect catches any protocol mismatches. Both apps should be released simultaneously to minimize mismatch windows.

---

## 7. Local Mode (No Networking Required)

When the Orchestrator runs inside the Tauri app (same process), all communication is via Tauri IPC (Rust → Frontend via `tauri::AppHandle::emit_all`). No WebSocket, no Tailscale required. The networking layer described in this document is only relevant for Remote Mode.

---

## Summary: Connection Decision Flow

```
Is the user's Orchestrator on a DIFFERENT machine?
│
├── NO (Local Tauri Mode)
│    └── Use Tauri IPC directly. No networking config needed.
│
└── YES (Remote Mode)
     ├── Is Tailscale installed and configured? (RECOMMENDED)
     │    └── Method A: Auto-discover via MagicDNS + Tailscale local API.
     │        Connect to ws://thinclaw-server.yourtailnet.ts.net:7878/ws
     │
     ├── No Tailscale, but server has a public/static IP?
     │    └── Method B: QR Code pairing with cert pinning.
     │        Connect to wss://SERVER_IP:7878/ws
     │
     └── Advanced / Developer
          └── Method C: Manual connection string entry.
```
