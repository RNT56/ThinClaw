# 🌐 Remote OpenClaw Deployment Guide (Updated)

This guide explains how to deploy the **OpenClaw Agent** on a remote server. We offer two methods:
1. **[Recommended] Automated Deployment Script:** A one-command setup using official Ansible playbooks.
2. **Docker Manual Setup:** A simpler, container-focused method for quick testing.

---

## 🚀 Option 1: Automated Deployment Script (Recommended)
**Best for:** Production servers (VPS, Dedicated), Home Labs, Secure remote access.

We provide a helper script that downloads and runs the official `openclaw-ansible` playbook.
It handles:
- **System Hardening:** Installs UFW (Firewall), Fail2ban.
- **VPN:** Installs Tailscale for secure, private access without exposing ports.
- **Dependencies:** Installs Docker, Node.js, and Systemd services.
- **Auto-Start:** Ensures the agent runs on boot.

### 1. Prerequisites
- A fresh Ubuntu/Debian server or a Mac.
- SSH access to the server.
- [Ansible](https://docs.ansible.com/ansible/latest/installation_guide/intro_installation.html) installed locally (`brew install ansible` on Mac).

### 2. Run the Script
From the project root:

```bash
./src-tauri/openclaw-engine/deploy-remote.sh
```

Follow the prompts:
- **Target Server IP:** The public IP of your server.
- **SSH User:** Typically `root` or `ubuntu`.

The script will configure your server and output the connection details.

### 3. Connect via VPN
Once installed, your server will join your Tailscale network.
1. Install Tailscale on your **Desktop**.
2. Go to **OpenClaw Desktop > Settings > Gateway**.
3. Set **Gateway Mode** to **Remote Bridge**.
4. Use the **Tailscale IP** of your server:
   `ws://<tailscale-ip>:18789`
   *(This is secure and encrypted over the VPN).*

---

## 🐳 Option 2: Docker Manual Setup
**Best for:** Quick tests, existing Docker environments, or if you don't want system-level changes.

### 1. Transfer Files
Copy `src-tauri/openclaw-engine` to your server:
```bash
scp -r src-tauri/openclaw-engine user@your-server-ip:~/openclaw-agent
```

### 2. Start the Container
On the server:
```bash
cd ~/openclaw-agent
docker-compose up -d --build
```
*Note: This exposes port 18789 to the public internet unless you configure a firewall yourself.*

### 3. Connect
Go to **OpenClaw Desktop > Settings > Gateway** and connect to:
`ws://<public-ip>:18789`

---

## 🔗 Connecting & Configuring (Both Methods)

Once your agent is running (via Ansible or Docker), connect your Desktop App to control it.

### 1. Remote Configuration
Your Desktop App is the **Control Plane**.
- **API Keys:** Go to **Settings > Secrets**. Toggling keys (OpenAI, Anthropic) sends them securely to the remote agent.
- **Agent Settings:** Changing "System Prompt" or "Model" updates the remote agent's config.

### 2. Verification
1. Open the Chat.
2. Send a message: "Hello, what host are you running on?"
3. The agent should reply with the hostname of your remote server (e.g., "I am running on 'linux-vps-01'").

---

## ⚠️ Troubleshooting

### 🔴 Connection Refused
- **Ansible Users:** Ensure you are connected to **Tailscale**. Are you using the Tailscale IP (100.x.y.z)?
- **Docker Users:** Did you open port 18789 in your cloud provider's firewall (AWS Security Group / DigitalOcean Firewall)?

### 🔴 "Authentication Failed"
- If you set a `OPENCLAW_GATEWAY_TOKEN` env var on the server, ensure it matches the **Gateway Token** in your Desktop Settings.
- Ansible setup might generate a random token; check `/etc/openclaw/env` or the playbook logs.

### 🔴 Updates
- **Ansible:** Run the `deploy-remote.sh` script again to pull latest changes and restart services.
- **Docker:** Copy new files and run `docker-compose up -d --build`.
