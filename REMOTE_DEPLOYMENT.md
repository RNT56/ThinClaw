# 🌐 Remote OpenClaw Deployment Guide

This guide explains how to run the **IronClaw Agent** on a remote server and connect to it using the Scrappy Desktop App.

When Scrappy is in **Remote Mode**, the frontend UI doesn't run the heavy agent process locally. Instead, it connects to an external IronClaw gateway via HTTP/SSE. This allows you to run your agent on a powerful server while controlling it from your personal mac or laptop.

---

## 🎯 OS Compatibility

### Does Remote Deploy work with macOS or Windows?
**Yes!** The Remote Deploy architecture works on **any OS** (macOS, Windows, Linux).
However, the method you use to set it up will differ:

1. **"Deploy New Agent" Wizard in Scrappy UI:**
   This built-in wizard uses an SSH connection and Bash script to automatically install Docker and start the agent. **This wizard only works if the target server is a Linux machine** (e.g. Ubuntu/Debian).

2. **Manual Setup + "Connect Existing" Wizard:**
   If your remote machine is macOS or Windows (or a customized Linux server), you simply start the agent manually on that machine, and then use the **"Connect Existing"** tab in the Scrappy UI.

> **Note on Ansible:** The legacy "one-command Ansible script" (`deploy-remote.sh`) has been officially deprecated and removed. It relied on complex systemic dependencies (Node.js, auto-installing Tailscale, specific OS versions) and a separate repository structure. The new Docker Compose architecture handles everything in a much cleaner, self-contained way.

---

## 🚀 Option 1: Automated Deployment from Scrappy (Linux Targets Only)

If you have a fresh Ubuntu/Debian server (like a DigitalOcean droplet or AWS EC2), you can deploy directly from Scrappy in one click.

1. Go to **Settings > Gateway** in the Scrappy App.
2. Click **Deploy New Remote Agent**.
3. Enter your server's **SSH Host IP** and **SSH User** (e.g. `root`).
4. Click **Deploy via SSH**.
5. Scrappy will securely copy the deployment files, install Docker on the remote server, and spin up the IronClaw agent.
6. Once finished, click **Save & Connect**.

*Prerequisites: Your local machine must have SSH access to the target server via standard key pairs (e.g., `~/.ssh/id_rsa`). Port 18789 must be open on the server's firewall.*

---

## 🐳 Option 2: Manual Setup on ANY OS (No Scrappy UI Required)

If you want to run the agent on macOS, Windows, or if the auto-deploy wizard doesn't fit your needs, you can start the IronClaw gateway manually.

### Step 1: Transfer the Deployment Files
Copy the `ironclaw/deploy` folder from the Scrappy source code to your target machine.

### Step 2: Configure the Environment
Inside the `deploy` folder on your remote machine:
1. Rename `.env.template` to `.env`.
2. Open `.env` in a text editor.
3. Generate a secure random password and set it as `GATEWAY_AUTH_TOKEN`.
   *(Example: Run `openssl rand -hex 32` or just use a long strong password)*
4. Set at least one LLM API key (e.g., `ANTHROPIC_API_KEY=sk-ant...`).

### Step 3: Start the Agent
You can start the agent in two ways:

#### A. Using Docker (Recommended)
Ensure Docker Desktop (Mac/Win) or Docker Engine (Linux) is installed.
Run:
```bash
docker compose up -d --build
```
This starts the `ironclaw-remote` container, exposing port `18789`.

#### B. Native Rust (No Docker)
If you don't want to use Docker, ensure you have Rust installed (`rustup`).
From the root of the IronClaw source (`ironclaw/` directory):
```bash
cargo run --release
```
The gateway will start on `0.0.0.0:18789` and print logs to your terminal.

---

## 🔗 Connecting the Scrappy App to an Existing Agent

Once your remote IronClaw agent is running (via Option 1 or Option 2), you connect your Scrappy Desktop App to control it.

1. On your Scrappy Desktop App, go to **Settings > Gateway**.
2. Click **Add New Agent Profile** and select **Connect Existing**.
3. Fill in the connection details:
   - **Gateway URL:** `http://<your-remote-machine-ip>:18789` 
     *(e.g., `http://192.168.1.50:18789` or `http://my-vps.com:18789`)*
   - **Auth Token:** The value you set for `GATEWAY_AUTH_TOKEN` in Step 2.
4. Click **Test & Save**.
5. Scrappy will verify the connection and switch to Remote Mode.

### VPN / Security Recommendation (Tailscale)
Since the agent's port (`18789`) communicates over plain HTTP by default, **you should not expose it directly to the public internet** without a reverse proxy (like Nginx/Caddy with SSL).

The easiest and most secure way to connect is using a mesh VPN like **Tailscale**:
1. Install Tailscale on both your laptop (Scrappy UI) and your remote server (IronClaw Agent).
2. Configure your server's firewall to block port 18789 on the public interface, but allow it on the `tailscale0` interface.
3. In Scrappy "Connect Existing", use the server's **Tailscale IP**: `http://100.x.y.z:18789`

This provides end-to-end encryption and zero public open ports.

---

## 🔧 Post-Setup Configuration

Your Scrappy Desktop App acts as the **Control Plane**. Even in remote mode, you can manage the agent from the UI:

- **API Keys / Secrets:** Go to **Settings > Models & Secrets**. Toggling keys or entering new ones will securely push them to the remote agent over the encrypted HTTP proxy.
- **Routines:** Creating or triggering routines in the UI will execute them on the remote server exactly as expected.
- **File Access:** File paths shown in the UI will refer to the remote agent's filesystem (typically the Docker volume or `ironclaw` working directory).
