#!/usr/bin/env bash
# ============================================================================
# IronClaw Remote Deployment Setup Script
# ============================================================================
#
# Bootstraps a Linux server for running the IronClaw agent via Docker Compose.
#
# Core features (always installed):
#   - Docker Engine + Docker Compose
#   - UFW Firewall (allows SSH + port 18789)
#   - Fail2ban (SSH brute-force protection)
#   - IronClaw Docker Compose stack
#
# Optional features (via flags):
#   --tailscale <auth-key>   Install Tailscale VPN and join the network
#   --systemd                Create a systemd service for IronClaw
#
# Usage:
#   sudo bash setup.sh --token <gateway_token> [--tailscale <ts-key>] [--systemd]
#
# Examples:
#   # Minimal (Docker only):
#   sudo bash setup.sh --token abc123def456
#
#   # Full production setup:
#   sudo bash setup.sh --token abc123def456 --tailscale tskey-auth-xxx --systemd
#
# ============================================================================

set -euo pipefail

# ── Parse arguments ──────────────────────────────────────────────────────────

TOKEN=""
TAILSCALE_KEY=""
ENABLE_SYSTEMD=false

while [[ "$#" -gt 0 ]]; do
    case $1 in
        --token) TOKEN="$2"; shift ;;
        --tailscale) TAILSCALE_KEY="$2"; shift ;;
        --systemd) ENABLE_SYSTEMD=true ;;
        --help|-h)
            echo "Usage: sudo bash setup.sh --token <token> [--tailscale <auth-key>] [--systemd]"
            echo ""
            echo "  --token <token>         Gateway auth token (required)"
            echo "  --tailscale <auth-key>  Install Tailscale VPN and authenticate with this key"
            echo "  --systemd               Create a systemd service for auto-start management"
            echo ""
            exit 0
            ;;
        *) echo "Unknown parameter: $1. Use --help for usage."; exit 1 ;;
    esac
    shift
done

if [[ -z "$TOKEN" ]]; then
    echo "ERROR: --token parameter is required"
    echo "Usage: sudo bash setup.sh --token <gateway_token> [--tailscale <ts-key>] [--systemd]"
    exit 1
fi

# ── Detect environment ──────────────────────────────────────────────────────

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IRONCLAW_PORT=18789

echo "============================================================"
echo "  IronClaw Remote Agent Setup"
echo "============================================================"
echo ""
echo "Deploy directory: $DEPLOY_DIR"
echo "Gateway port:     $IRONCLAW_PORT"
echo "Tailscale:        $([ -n "$TAILSCALE_KEY" ] && echo 'Yes' || echo 'No')"
echo "Systemd service:  $([ "$ENABLE_SYSTEMD" = true ] && echo 'Yes' || echo 'No')"
echo ""

# Must run as root
if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root (use: sudo bash setup.sh ...)"
    exit 1
fi

# Detect package manager
if command -v apt-get &> /dev/null; then
    PKG_MANAGER="apt"
elif command -v yum &> /dev/null; then
    PKG_MANAGER="yum"
elif command -v dnf &> /dev/null; then
    PKG_MANAGER="dnf"
else
    echo "ERROR: No supported package manager found (apt, yum, dnf)."
    echo "Please install Docker, UFW, and Fail2ban manually."
    exit 1
fi

echo "==> Detected package manager: $PKG_MANAGER"

# ============================================================================
# 1. INSTALL DOCKER
# ============================================================================

if ! command -v docker &> /dev/null; then
    echo ""
    echo "==> [1/6] Installing Docker Engine..."

    if [ "$PKG_MANAGER" = "apt" ]; then
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -y -qq
        apt-get install -y -qq ca-certificates curl gnupg lsb-release

        # Add Docker's official GPG key
        install -m 0755 -d /etc/apt/keyrings
        if [ -f /etc/apt/keyrings/docker.asc ]; then
            rm /etc/apt/keyrings/docker.asc
        fi
        curl -fsSL "https://download.docker.com/linux/$(. /etc/os-release && echo "$ID")/gpg" \
            -o /etc/apt/keyrings/docker.asc
        chmod a+r /etc/apt/keyrings/docker.asc

        # Add Docker apt repository
        echo \
          "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] \
          https://download.docker.com/linux/$(. /etc/os-release && echo "$ID") \
          $(. /etc/os-release && echo "$VERSION_CODENAME") stable" | \
          tee /etc/apt/sources.list.d/docker.list > /dev/null

        apt-get update -y -qq
        apt-get install -y -qq docker-ce docker-ce-cli containerd.io \
            docker-buildx-plugin docker-compose-plugin

    elif [ "$PKG_MANAGER" = "yum" ] || [ "$PKG_MANAGER" = "dnf" ]; then
        $PKG_MANAGER install -y yum-utils
        yum-config-manager --add-repo https://download.docker.com/linux/centos/docker-ce.repo
        $PKG_MANAGER install -y docker-ce docker-ce-cli containerd.io \
            docker-buildx-plugin docker-compose-plugin
    fi

    systemctl enable docker
    systemctl start docker
    echo "    Docker installed successfully."
else
    echo ""
    echo "==> [1/6] Docker is already installed. Ensuring service is running..."
    systemctl enable docker 2>/dev/null || true
    systemctl start docker 2>/dev/null || true
fi

# Verify docker compose is available
if ! docker compose version &> /dev/null; then
    echo "ERROR: 'docker compose' (V2) not found. Please install docker-compose-plugin."
    exit 1
fi

# ============================================================================
# 2. INSTALL & CONFIGURE UFW FIREWALL
# ============================================================================

echo ""
echo "==> [2/6] Configuring UFW Firewall..."

if ! command -v ufw &> /dev/null; then
    if [ "$PKG_MANAGER" = "apt" ]; then
        apt-get install -y -qq ufw
    elif [ "$PKG_MANAGER" = "yum" ] || [ "$PKG_MANAGER" = "dnf" ]; then
        # UFW isn't native on RHEL — install EPEL first
        $PKG_MANAGER install -y epel-release 2>/dev/null || true
        $PKG_MANAGER install -y ufw 2>/dev/null || true
    fi
fi

if command -v ufw &> /dev/null; then
    # Reset to clean state (non-interactive)
    echo "y" | ufw reset 2>/dev/null || true

    # Default policies: deny incoming, allow outgoing
    ufw default deny incoming
    ufw default allow outgoing

    # Allow SSH (critical — don't lock yourself out!)
    ufw allow ssh comment "SSH access"

    # Allow IronClaw gateway port
    ufw allow "$IRONCLAW_PORT/tcp" comment "IronClaw Gateway"

    # If Tailscale is being installed, allow Tailscale traffic
    if [[ -n "$TAILSCALE_KEY" ]]; then
        ufw allow in on tailscale0 comment "Tailscale VPN"
    fi

    # Enable firewall (non-interactive)
    echo "y" | ufw enable

    echo "    UFW configured:"
    ufw status numbered 2>/dev/null || ufw status
else
    echo "    WARNING: UFW could not be installed. Configure your firewall manually."
    echo "    Required ports: SSH (22), IronClaw ($IRONCLAW_PORT/tcp)"
fi

# ============================================================================
# 3. INSTALL & CONFIGURE FAIL2BAN
# ============================================================================

echo ""
echo "==> [3/6] Installing Fail2ban..."

if ! command -v fail2ban-client &> /dev/null; then
    if [ "$PKG_MANAGER" = "apt" ]; then
        apt-get install -y -qq fail2ban
    elif [ "$PKG_MANAGER" = "yum" ] || [ "$PKG_MANAGER" = "dnf" ]; then
        $PKG_MANAGER install -y epel-release 2>/dev/null || true
        $PKG_MANAGER install -y fail2ban
    fi
fi

if command -v fail2ban-client &> /dev/null; then
    # Create local config (overrides without touching defaults)
    cat > /etc/fail2ban/jail.local <<'FAIL2BAN_CONF'
[DEFAULT]
# Ban for 1 hour after 5 failed attempts within 10 minutes
bantime  = 3600
findtime = 600
maxretry = 5

# Email notifications (optional — configure sendmail if needed)
# destemail = admin@example.com
# action = %(action_mwl)s

[sshd]
enabled = true
port    = ssh
filter  = sshd
logpath = /var/log/auth.log
maxretry = 3
FAIL2BAN_CONF

    # Use journald for log backend if auth.log doesn't exist (systemd-based distros)
    if [ ! -f /var/log/auth.log ]; then
        sed -i 's|logpath = /var/log/auth.log|backend = systemd|' /etc/fail2ban/jail.local
    fi

    systemctl enable fail2ban
    systemctl restart fail2ban
    echo "    Fail2ban installed and configured."
    echo "    SSH jail: max 3 retries, 1 hour ban."
else
    echo "    WARNING: Fail2ban could not be installed."
fi

# ============================================================================
# 4. TAILSCALE VPN (Optional)
# ============================================================================

if [[ -n "$TAILSCALE_KEY" ]]; then
    echo ""
    echo "==> [4/6] Installing Tailscale VPN..."

    if ! command -v tailscale &> /dev/null; then
        # Official Tailscale install script
        curl -fsSL https://tailscale.com/install.sh | sh
    fi

    if command -v tailscale &> /dev/null; then
        # Authenticate and connect
        tailscale up --authkey="$TAILSCALE_KEY" --accept-routes --accept-dns=false

        # Get Tailscale IP
        TS_IP=$(tailscale ip -4 2>/dev/null || echo "unknown")
        echo "    Tailscale installed and connected."
        echo "    Tailscale IPv4: $TS_IP"
        echo ""
        echo "    ┌──────────────────────────────────────────────────────┐"
        echo "    │  SECURE CONNECTION:                                  │"
        echo "    │  Use http://$TS_IP:$IRONCLAW_PORT in Scrappy       │"
        echo "    │  (encrypted via Tailscale — no public port needed)  │"
        echo "    └──────────────────────────────────────────────────────┘"

        # Optionally restrict IronClaw to only Tailscale interface
        # This prevents public access even if port 18789 is open
        if command -v ufw &> /dev/null; then
            # Remove the public rule and only allow via Tailscale
            ufw delete allow "$IRONCLAW_PORT/tcp" 2>/dev/null || true
            ufw allow in on tailscale0 to any port "$IRONCLAW_PORT" proto tcp \
                comment "IronClaw via Tailscale only"
            echo "    UFW updated: port $IRONCLAW_PORT only accessible via Tailscale."
        fi
    else
        echo "    ERROR: Tailscale installation failed."
    fi
else
    echo ""
    echo "==> [4/6] Tailscale: Skipped (no --tailscale flag provided)"
fi

# ============================================================================
# 5. CONFIGURE & START IRONCLAW
# ============================================================================

echo ""
echo "==> [5/6] Configuring IronClaw environment..."

cd "$DEPLOY_DIR"

# Backup existing config
if [[ -f .env ]]; then
    cp .env ".env.bak.$(date +%Y%m%d%H%M%S)"
    echo "    Backed up existing .env"
fi

# Create .env from template
cp .env.template .env

# Inject the gateway auth token
sed -i "s/^GATEWAY_AUTH_TOKEN=.*/GATEWAY_AUTH_TOKEN=${TOKEN}/" .env

echo "    .env configured with gateway token."

# Start Docker Compose
echo ""
echo "==> Starting IronClaw Docker Compose stack..."

docker compose down 2>/dev/null || true
docker compose up -d --build

echo "    Docker Compose stack started."

# ============================================================================
# 6. SYSTEMD SERVICE (Optional)
# ============================================================================

if [ "$ENABLE_SYSTEMD" = true ]; then
    echo ""
    echo "==> [6/6] Creating systemd service..."

    cat > /etc/systemd/system/ironclaw.service <<SYSTEMD_UNIT
[Unit]
Description=IronClaw AI Agent (Docker Compose)
Documentation=https://github.com/RNT56/ThinClaw
After=docker.service network-online.target
Requires=docker.service
Wants=network-online.target

[Service]
Type=oneshot
RemainAfterExit=yes
WorkingDirectory=${DEPLOY_DIR}
ExecStart=/usr/bin/docker compose up -d
ExecStop=/usr/bin/docker compose down
ExecReload=/usr/bin/docker compose restart
TimeoutStartSec=120

# Restart on failure (systemd will re-run ExecStart)
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
SYSTEMD_UNIT

    systemctl daemon-reload
    systemctl enable ironclaw.service
    # Don't start via systemd now — Docker Compose already started it above

    echo "    Systemd service created and enabled."
    echo "    Commands:"
    echo "      systemctl status ironclaw    # Check status"
    echo "      systemctl restart ironclaw   # Restart agent"
    echo "      systemctl stop ironclaw      # Stop agent"
    echo "      journalctl -u ironclaw -f    # View logs"
else
    echo ""
    echo "==> [6/6] Systemd service: Skipped (no --systemd flag provided)"
fi

# ============================================================================
# DONE — Summary
# ============================================================================

echo ""
echo "============================================================"
echo "  IronClaw Setup Complete!"
echo "============================================================"
echo ""

# Wait for container to be healthy
echo "  Waiting for health check..."
sleep 5

CONTAINER_STATUS=$(docker ps --filter "name=ironclaw-remote" --format "{{.Status}}" 2>/dev/null || echo "unknown")
echo "  Container status: $CONTAINER_STATUS"

# Determine connection URL
if [[ -n "$TAILSCALE_KEY" ]] && command -v tailscale &> /dev/null; then
    TS_IP=$(tailscale ip -4 2>/dev/null || echo "<tailscale-ip>")
    echo ""
    echo "  ┌────────────────────────────────────────────────────────┐"
    echo "  │  Connect via Tailscale (recommended, encrypted):       │"
    echo "  │    URL:   http://$TS_IP:$IRONCLAW_PORT                │"
    echo "  │    Token: $TOKEN                                       │"
    echo "  └────────────────────────────────────────────────────────┘"
fi

# Get public IP for fallback display
PUBLIC_IP=$(curl -s -4 ifconfig.me 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}' || echo "<server-ip>")
echo ""
echo "  ┌────────────────────────────────────────────────────────┐"
echo "  │  Connect via public IP:                                │"
echo "  │    URL:   http://$PUBLIC_IP:$IRONCLAW_PORT             │"
echo "  │    Token: $TOKEN                                       │"
echo "  └────────────────────────────────────────────────────────┘"
echo ""
echo "  Verify:"
echo "    curl http://localhost:$IRONCLAW_PORT/api/health"
echo "    docker logs ironclaw-remote"
echo ""
if [ "$ENABLE_SYSTEMD" = true ]; then
    echo "  Systemd:"
    echo "    systemctl status ironclaw"
    echo "    journalctl -u ironclaw -f"
    echo ""
fi
echo "============================================================"
