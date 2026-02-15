# OpenClaw Remote Gateway & Agent Connection Guide

This comprehensive guide explains how to deploy a new remote OpenClaw Agent (Brain) onto a fresh server and connect your local desktop application securely.

The deployment process uses Ansible to automatically provision your server with:
1.  **Node.js & OpenClaw Engine**: The core runtime.
2.  **System Optimizations**: For AI/LLM workloads.
3.  **Tailscale VPN**: For secure, encrypted networking without exposing ports to the public internet.

---

## Part A: Preparing Access (One-Time Setup)

Before using the deployment wizard, your local machine must be able to SSH into the target server without a password prompt. This is required for the automation script to run smoothly.

### Step 1: Generate an SSH Key (If you haven't already)
On your local machine (Mac or Linux), open a terminal:

```bash
# Check if you already have a key
ls ~/.ssh/id_ed25519.pub

# If file not found, generate one:
ssh-keygen -t ed25519 -C "your_email@example.com"
# Press Enter to accept defaults (no passphrase recommended for automation)
```

### Step 2: Copy Key to Remote Server
You need the IP address of your fresh server (e.g., from DigitalOcean, AWS, or your home router) and the default user (usually `root` or `ubuntu`).

```bash
# Replace with your actual user and IP
ssh-copy-id root@192.168.1.50
```

**Verification:** Try logging in. You should **not** be asked for a password.
```bash
ssh root@192.168.1.50
# If you get in immediately, you are ready!
exit
```

---

## Part B: Deploying a New Remote Agent

Use the built-in wizard to turn your fresh server into an OpenClaw Brain.
*(Note: Supports Ubuntu/Debian Linux servers. macOS targets are not supported.)*

1.  Open **Settings** (Gear Icon) in the OpenClaw Desktop App.
2.  Go to the **Gateway** tab.
3.  Click the **"Deploy / Connect Agent"** button.
4.  Select **"Deploy New Agent"**.
5.  **Server IP Address**: Enter the public IP (e.g., `192.168.1.50`).
6.  **SSH User**: Enter the user you set up in Part A (e.g., `root`).
7.  Click **Start Deployment**.

### What happens during deployment?
The wizard runs an Ansible playbook that:
1.  Updates the system packages.
2.  Installs **Node.js 20+**, **npm**, and build tools.
3.  Installs **Tailscale** (Security Layer).
4.  Installs and starts the **OpenClaw Engine** service via PM2.

---

## Part C: Securing and Connecting

Once the deployment finishes successfully, the OpenClaw Engine is running, but you need to finalize the secure connection.

### Step 1: Authenticate Tailscale (Crucial for Security)
The deployment installed Tailscale, but it needs to be linked to your account to form a secure mesh network.

1.  **SSH into your server**:
    ```bash
    ssh root@<YOUR_SERVER_IP>
    ```
2.  **Run the authentication command**:
    ```bash
    sudo tailscale up
    ```
3.  Copy the URL printed in the terminal, open it in your browser, and log in to authorize the machine.
4.  Run `tailscale ip -4` to see the new secure IP address (starts with `100.x.y.z`).

### Step 2: Connect via Tailscale IP
Now, connect your Desktop App using the secure Tailscale IP. This ensures traffic is encrypted and the port `18789` doesn't need to be open to the whole internet.

1.  Back in **Settings** -> **Gateway**.
2.  Select **"Connect Existing"** (or use the main Gateway form).
3.  **Agent URL / IP**: Enter the **Tailscale IP** from the previous step.
    -   Example: `100.85.120.40`
    -   (The app will auto-append `:18789`)
4.  **Auth Token**: If you set one up in `openclaw_engine.json` (optional), enter it here.
5.  Click **Connect Agent**.

### Summary of Architecture
-   **Your Desktop**: Runs the UI. Connected to Tailscale.
-   **Remote Server**: Runs OpenClaw Engine + LLMs. Connected to Tailscale.
-   **Network**: Traffic flows over the encrypted WireGuard tunnel (Tailscale). Public ports on the server can remain closed (uFW deny incoming).

You now have a secure, remote AI Brain!
