#!/usr/bin/env bash
# ============================================================================
# ThinClaw Remote Deployment Setup Script
# ============================================================================
#
# Bootstraps a Linux server for running ThinClaw natively or via Docker Compose.
#
# Core Docker features:
#   - Docker Engine + Docker Compose
#   - UFW Firewall (allows SSH + the ThinClaw gateway port)
#   - Fail2ban (SSH brute-force protection)
#   - ThinClaw Docker Compose stack
#
# Core native Pi OS Lite features:
#   - /usr/local/bin/thinclaw
#   - /var/lib/thinclaw/.thinclaw/.env
#   - system thinclaw.service running as the unprivileged thinclaw user
#
# Optional features (via flags):
#   --mode <auto|native|docker>
#   --binary <path>          Native install source binary
#   --image <image>          Docker image for Compose
#   --tailscale <auth-key>   Install Tailscale VPN and join the network
#   --systemd                Create a systemd service for ThinClaw
#
# Usage:
#   sudo bash setup.sh --token <gateway_token> [--mode auto|native|docker] [--tailscale <ts-key>] [--systemd]
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
MODE="auto"
BINARY_PATH=""
THINCLAW_IMAGE="${THINCLAW_IMAGE:-ghcr.io/rnt56/thinclaw:latest}"

while [[ "$#" -gt 0 ]]; do
    case $1 in
        --token) TOKEN="$2"; shift ;;
        --mode) MODE="$2"; shift ;;
        --binary) BINARY_PATH="$2"; shift ;;
        --image) THINCLAW_IMAGE="$2"; shift ;;
        --tailscale) TAILSCALE_KEY="$2"; shift ;;
        --systemd) ENABLE_SYSTEMD=true ;;
        --help|-h)
            echo "Usage: sudo bash setup.sh --token <token> [--mode auto|native|docker] [--binary <path>] [--image <image>] [--tailscale <auth-key>] [--systemd]"
            echo ""
            echo "  --token <token>         Gateway auth token (required)"
            echo "  --mode <mode>           Install mode: auto, native, or docker (default: auto)"
            echo "  --binary <path>         Native install source binary"
            echo "  --image <image>         Docker image for Compose (default: $THINCLAW_IMAGE)"
            echo "  --tailscale <auth-key>  Install Tailscale VPN and authenticate with this key"
            echo "  --systemd               In docker mode, create a systemd service for auto-start management"
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

if [[ "$MODE" != "auto" && "$MODE" != "native" && "$MODE" != "docker" ]]; then
    echo "ERROR: --mode must be one of: auto, native, docker"
    exit 1
fi

# ── Detect environment ──────────────────────────────────────────────────────

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
THINCLAW_PORT="${GATEWAY_PORT:-3000}"

echo "============================================================"
echo "  ThinClaw Remote Agent Setup"
echo "============================================================"
echo ""
echo "Deploy directory: $DEPLOY_DIR"
echo "Gateway port:     $THINCLAW_PORT"
echo "Requested mode:   $MODE"
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

is_pi_os_lite_64() {
    local arch=""
    arch="$(uname -m 2>/dev/null || true)"
    [[ "$arch" == "aarch64" || "$arch" == "arm64" ]] || return 1
    [[ -f /etc/os-release ]] || return 1
    grep -qi "bookworm" /etc/os-release || return 1
    if grep -qi "raspberry pi" /etc/os-release 2>/dev/null; then
        return 0
    fi
    if [[ -f /etc/rpi-issue ]] && grep -qi "raspberry pi" /etc/rpi-issue; then
        return 0
    fi
    return 1
}

generate_hex_32() {
    if command -v openssl >/dev/null 2>&1; then
        openssl rand -hex 32
    else
        od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
    fi
}

configure_firewall() {
    echo ""
    echo "==> Configuring UFW Firewall..."

    if ! command -v ufw &> /dev/null; then
        if [ "$PKG_MANAGER" = "apt" ]; then
            apt-get install -y -qq ufw
        elif [ "$PKG_MANAGER" = "yum" ] || [ "$PKG_MANAGER" = "dnf" ]; then
            $PKG_MANAGER install -y epel-release 2>/dev/null || true
            $PKG_MANAGER install -y ufw 2>/dev/null || true
        fi
    fi

    if command -v ufw &> /dev/null; then
        echo "y" | ufw reset 2>/dev/null || true
        ufw default deny incoming
        ufw default allow outgoing
        ufw allow ssh comment "SSH access"
        ufw allow "$THINCLAW_PORT/tcp" comment "ThinClaw Gateway"

        if [[ -n "$TAILSCALE_KEY" ]]; then
            ufw allow in on tailscale0 comment "Tailscale VPN"
        fi

        echo "y" | ufw enable
        echo "    UFW configured:"
        ufw status numbered 2>/dev/null || ufw status
    else
        echo "    WARNING: UFW could not be installed. Configure your firewall manually."
        echo "    Required ports: SSH (22), ThinClaw ($THINCLAW_PORT/tcp)"
    fi
}

install_fail2ban() {
    echo ""
    echo "==> Installing Fail2ban..."

    if ! command -v fail2ban-client &> /dev/null; then
        if [ "$PKG_MANAGER" = "apt" ]; then
            apt-get install -y -qq fail2ban
        elif [ "$PKG_MANAGER" = "yum" ] || [ "$PKG_MANAGER" = "dnf" ]; then
            $PKG_MANAGER install -y epel-release 2>/dev/null || true
            $PKG_MANAGER install -y fail2ban
        fi
    fi

    if command -v fail2ban-client &> /dev/null; then
        cat > /etc/fail2ban/jail.local <<'FAIL2BAN_CONF'
[DEFAULT]
bantime  = 3600
findtime = 600
maxretry = 5

[sshd]
enabled = true
port    = ssh
filter  = sshd
logpath = /var/log/auth.log
maxretry = 3
FAIL2BAN_CONF

        if [ ! -f /var/log/auth.log ]; then
            sed -i 's|logpath = /var/log/auth.log|backend = systemd|' /etc/fail2ban/jail.local
        fi

        systemctl enable fail2ban
        systemctl restart fail2ban
        echo "    Fail2ban installed and configured."
    else
        echo "    WARNING: Fail2ban could not be installed."
    fi
}

install_tailscale_if_requested() {
    if [[ -z "$TAILSCALE_KEY" ]]; then
        echo ""
        echo "==> Tailscale: Skipped (no --tailscale flag provided)"
        return 0
    fi

    echo ""
    echo "==> Installing Tailscale VPN..."
    if ! command -v tailscale &> /dev/null; then
        curl -fsSL https://tailscale.com/install.sh | sh
    fi

    if command -v tailscale &> /dev/null; then
        tailscale up --authkey="$TAILSCALE_KEY" --accept-routes --accept-dns=false
        TS_IP=$(tailscale ip -4 2>/dev/null || echo "unknown")
        echo "    Tailscale installed and connected."
        echo "    Tailscale IPv4: $TS_IP"
        if command -v ufw &> /dev/null; then
            ufw delete allow "$THINCLAW_PORT/tcp" 2>/dev/null || true
            ufw allow in on tailscale0 to any port "$THINCLAW_PORT" proto tcp \
                comment "ThinClaw via Tailscale only"
            echo "    UFW updated: port $THINCLAW_PORT only accessible via Tailscale."
        fi
    else
        echo "    ERROR: Tailscale installation failed."
    fi
}

resolve_native_binary() {
    if [[ -n "$BINARY_PATH" && -x "$BINARY_PATH" ]]; then
        echo "$BINARY_PATH"
        return 0
    fi
    if [[ -x "$DEPLOY_DIR/../target/release/thinclaw" ]]; then
        echo "$DEPLOY_DIR/../target/release/thinclaw"
        return 0
    fi
    if [[ -x "$DEPLOY_DIR/../thinclaw" ]]; then
        echo "$DEPLOY_DIR/../thinclaw"
        return 0
    fi
    if command -v thinclaw >/dev/null 2>&1; then
        command -v thinclaw
        return 0
    fi
    return 1
}

install_native_pi() {
    echo ""
    echo "==> Native Raspberry Pi OS Lite install"

    if [ "$PKG_MANAGER" != "apt" ]; then
        echo "ERROR: Native Pi OS Lite mode expects apt."
        exit 1
    fi

    export DEBIAN_FRONTEND=noninteractive
    apt-get update -y -qq
    apt-get install -y -qq ca-certificates curl openssl

    local source_binary=""
    if ! source_binary="$(resolve_native_binary)"; then
        echo "ERROR: Could not find a ThinClaw binary to install."
        echo "Provide one with --binary /path/to/thinclaw, or install the aarch64-unknown-linux-gnu release artifact first."
        exit 1
    fi

    if [[ "$(readlink -f "$source_binary" 2>/dev/null || echo "$source_binary")" != "/usr/local/bin/thinclaw" ]]; then
        install -m 0755 "$source_binary" /usr/local/bin/thinclaw
    else
        chmod 0755 /usr/local/bin/thinclaw
    fi

    if ! id thinclaw >/dev/null 2>&1; then
        useradd --system --create-home --home-dir /var/lib/thinclaw --shell /usr/sbin/nologin thinclaw
    fi

    install -d -m 0750 -o thinclaw -g thinclaw /var/lib/thinclaw/.thinclaw
    install -d -m 0750 -o thinclaw -g thinclaw /var/lib/thinclaw/.thinclaw/logs

    local master_key=""
    master_key="$(generate_hex_32)"
    cat > /var/lib/thinclaw/.thinclaw/.env <<ENV
ONBOARD_COMPLETED=true
THINCLAW_HOME=/var/lib/thinclaw/.thinclaw
DATABASE_BACKEND=libsql
LIBSQL_PATH=/var/lib/thinclaw/.thinclaw/thinclaw.db
GATEWAY_ENABLED=true
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=${THINCLAW_PORT}
GATEWAY_AUTH_TOKEN=${TOKEN}
THINCLAW_ALLOW_ENV_MASTER_KEY=1
SECRETS_MASTER_KEY=${master_key}
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
OPENROUTER_API_KEY=CHANGE_ME
EMBEDDING_ENABLED=false
CLI_ENABLED=false
HEARTBEAT_ENABLED=false
SANDBOX_ENABLED=false
ROUTINES_ENABLED=true
BROWSER_DOCKER=auto
SCREEN_CAPTURE_ENABLED=false
CAMERA_CAPTURE_ENABLED=false
TALK_MODE_ENABLED=false
LOCATION_ENABLED=false
LOCATION_ALLOW_IP_FALLBACK=false
DESKTOP_AUTONOMY_ENABLED=false
ENV
    chown thinclaw:thinclaw /var/lib/thinclaw/.thinclaw/.env
    chmod 0600 /var/lib/thinclaw/.thinclaw/.env

    cat > /etc/systemd/system/thinclaw.service <<'SYSTEMD_UNIT'
[Unit]
Description=ThinClaw AI Agent
Documentation=https://github.com/RNT56/ThinClaw
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=thinclaw
Group=thinclaw
WorkingDirectory=/var/lib/thinclaw
Environment=THINCLAW_HOME=/var/lib/thinclaw/.thinclaw
Environment=HOME=/var/lib/thinclaw
ExecStart=/usr/local/bin/thinclaw run --no-onboard
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
SYSTEMD_UNIT

    systemctl daemon-reload
    systemctl enable thinclaw.service
    systemctl restart thinclaw.service || true

    configure_firewall
    install_fail2ban
    install_tailscale_if_requested

    echo ""
    echo "============================================================"
    echo "  ThinClaw Native Pi OS Lite Setup Complete!"
    echo "============================================================"
    echo "  Binary:       /usr/local/bin/thinclaw"
    echo "  Config:       /var/lib/thinclaw/.thinclaw/.env"
    echo "  Service:      systemctl status thinclaw"
    echo "  Gateway URL:  http://$(hostname -I 2>/dev/null | awk '{print $1}' || echo '<pi-ip>'):$THINCLAW_PORT"
    echo "  Token:        $TOKEN"
    echo ""
    echo "  Next:"
    echo "    1. Edit /var/lib/thinclaw/.thinclaw/.env and replace OPENROUTER_API_KEY=CHANGE_ME or configure another LLM."
    echo "    2. Run: sudo systemctl restart thinclaw"
    echo "    3. Verify: curl http://localhost:$THINCLAW_PORT/api/health"
    echo "============================================================"
}

if [[ "$MODE" == "auto" ]]; then
    if is_pi_os_lite_64; then
        MODE="native"
    else
        MODE="docker"
    fi
    echo "==> Auto-selected install mode: $MODE"
fi

if [[ "$MODE" == "native" ]]; then
    install_native_pi
    exit 0
fi

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

    # Allow ThinClaw gateway port
    ufw allow "$THINCLAW_PORT/tcp" comment "ThinClaw Gateway"

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
    echo "    Required ports: SSH (22), ThinClaw ($THINCLAW_PORT/tcp)"
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
        echo "    │  Use http://$TS_IP:$THINCLAW_PORT in Scrappy        │"
        echo "    │  (encrypted via Tailscale — no public port needed)  │"
        echo "    └──────────────────────────────────────────────────────┘"

        # Restrict ThinClaw to Tailscale interface only
        if command -v ufw &> /dev/null; then
            ufw delete allow "$THINCLAW_PORT/tcp" 2>/dev/null || true
            ufw allow in on tailscale0 to any port "$THINCLAW_PORT" proto tcp \
                comment "ThinClaw via Tailscale only"
            echo "    UFW updated: port $THINCLAW_PORT only accessible via Tailscale."
        fi
    else
        echo "    ERROR: Tailscale installation failed."
    fi
else
    echo ""
    echo "==> [4/6] Tailscale: Skipped (no --tailscale flag provided)"
fi

# ============================================================================
# 5. CONFIGURE & START THINCLAW
# ============================================================================

echo ""
echo "==> [5/6] Configuring ThinClaw environment..."

cd "$DEPLOY_DIR"

# Backup existing config
if [[ -f .env ]]; then
    cp .env ".env.bak.$(date +%Y%m%d%H%M%S)"
    echo "    Backed up existing .env"
fi

# Create .env from template
cp env.example .env

# Inject the gateway auth token
sed -i "s/^GATEWAY_AUTH_TOKEN=.*/GATEWAY_AUTH_TOKEN=${TOKEN}/" .env
sed -i "s/^GATEWAY_PORT=.*/GATEWAY_PORT=${THINCLAW_PORT}/" .env
sed -i "s|^THINCLAW_IMAGE=.*|THINCLAW_IMAGE=${THINCLAW_IMAGE}|" .env

echo "    .env configured with gateway token."

# Start Docker Compose
echo ""
echo "==> Starting ThinClaw Docker Compose stack..."

docker compose down 2>/dev/null || true
docker compose pull thinclaw 2>/dev/null || true
docker compose up -d

echo "    Docker Compose stack started."

# ============================================================================
# 6. SYSTEMD SERVICE (Optional)
# ============================================================================

if [ "$ENABLE_SYSTEMD" = true ]; then
    echo ""
    echo "==> [6/6] Creating systemd service..."

    cat > /etc/systemd/system/thinclaw.service <<SYSTEMD_UNIT
[Unit]
Description=ThinClaw AI Agent (Docker Compose)
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
    systemctl enable thinclaw.service
    # Don't start via systemd now — Docker Compose already started it above

    echo "    Systemd service created and enabled."
    echo "    Commands:"
    echo "      systemctl status thinclaw    # Check status"
    echo "      systemctl restart thinclaw   # Restart agent"
    echo "      systemctl stop thinclaw      # Stop agent"
    echo "      journalctl -u thinclaw -f    # View logs"
else
    echo ""
    echo "==> [6/6] Systemd service: Skipped (no --systemd flag provided)"
fi

# ============================================================================
# DONE — Summary
# ============================================================================

echo ""
echo "============================================================"
echo "  ThinClaw Setup Complete!"
echo "============================================================"
echo ""

# Wait for container to be healthy
echo "  Waiting for health check..."
sleep 5

CONTAINER_STATUS=$(docker ps --filter "name=thinclaw-remote" --format "{{.Status}}" 2>/dev/null || echo "unknown")
echo "  Container status: $CONTAINER_STATUS"
if curl -fsS "http://localhost:$THINCLAW_PORT/api/health" >/dev/null 2>&1; then
    echo "  Health endpoint:  http://localhost:$THINCLAW_PORT/api/health OK"
else
    echo "  WARNING: Health endpoint did not respond on http://localhost:$THINCLAW_PORT/api/health"
    echo "           Check: docker compose ps && docker compose logs thinclaw"
fi

# Determine connection URL
if [[ -n "$TAILSCALE_KEY" ]] && command -v tailscale &> /dev/null; then
    TS_IP=$(tailscale ip -4 2>/dev/null || echo "<tailscale-ip>")
    echo ""
    echo "  ┌────────────────────────────────────────────────────────┐"
    echo "  │  Connect via Tailscale (recommended, encrypted):       │"
    echo "  │    URL:   http://$TS_IP:$THINCLAW_PORT                │"
    echo "  │    Token: $TOKEN                                       │"
    echo "  └────────────────────────────────────────────────────────┘"
fi

# Get public IP for fallback display
PUBLIC_IP=$(curl -s -4 ifconfig.me 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}' || echo "<server-ip>")
echo ""
echo "  ┌────────────────────────────────────────────────────────┐"
echo "  │  Connect via public IP:                                │"
echo "  │    URL:   http://$PUBLIC_IP:$THINCLAW_PORT             │"
echo "  │    Token: $TOKEN                                       │"
echo "  └────────────────────────────────────────────────────────┘"
echo ""
echo "  Verify:"
echo "    curl http://localhost:$THINCLAW_PORT/api/health"
echo "    docker logs thinclaw-remote"
echo ""
if [ "$ENABLE_SYSTEMD" = true ]; then
    echo "  Systemd:"
    echo "    systemctl status thinclaw"
    echo "    journalctl -u thinclaw -f"
    echo ""
fi
echo "============================================================"
