#!/bin/bash
set -e

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${BLUE}=== OpenClaw Remote Deployment Tool ===${NC}"

# Check for Ansible and Auto-Install on the CONTROLLER machine (where this script runs)
if ! command -v ansible-playbook &> /dev/null; then
    echo "Ansible is not installed on this machine."
    
    if [[ "$OSTYPE" == "darwin"* ]]; then
        echo "Detected macOS Controller. Attempting install via Homebrew..."
        # We install Ansible locally so we can deploy TO the remote Linux server
        if command -v brew &> /dev/null; then
            brew install ansible
        else
            echo -e "${RED}Homebrew not found. Please install Homebrew or Ansible manually to proceed.${NC}"
            exit 1
        fi
    elif [[ -f /etc/debian_version ]]; then
         echo "Detected Debian/Ubuntu Controller."
         echo "Attempting to install Ansible via apt (requires sudo)..."
         if sudo -v; then
             sudo apt update && sudo apt install -y ansible
         else
             echo -e "${RED}Sudo privileges required to auto-install Ansible.${NC}"
             exit 1
         fi
    else
        echo -e "${RED}Could not auto-install Ansible on this OS.${NC}"
        echo "Please install 'ansible' manually and try again."
        exit 1
    fi
fi

# Parse Arguments if provided (Non-Interactive Mode)
TARGET_IP=""
SSH_USER=""

if [ "$#" -ge 2 ]; then
    TARGET_IP=$1
    SSH_USER=$2
fi

# Interactive Fallback
if [ -z "$TARGET_IP" ]; then
    read -p "Enter Target Server IP: " TARGET_IP
fi

if [ -z "$SSH_USER" ]; then
    read -p "Enter SSH User (e.g., root, ubuntu): " SSH_USER
fi

# Validate inputs
if [ -z "$TARGET_IP" ] || [ -z "$SSH_USER" ]; then
    echo -e "${RED}Error: IP and User are required.${NC}"
    exit 1
fi

DEPLOY_DIR=".deploy-cache"
REPO_URL="https://github.com/openclaw/openclaw-ansible.git"

# Create/Update cache
if [ -d "$DEPLOY_DIR" ]; then
    echo -e "${BLUE}Updating deployment scripts...${NC}"
    cd "$DEPLOY_DIR"
    git pull
    cd ..
else
    echo -e "${BLUE}Fetching deployment scripts...${NC}"
    git clone "$REPO_URL" "$DEPLOY_DIR"
fi

echo ""
echo -e "${GREEN}Deploying to $SSH_USER@$TARGET_IP${NC}"

# Run Ansible
# Using comma after IP to indicate list capable inventory
# ANSIBLE_HOST_KEY_CHECKING=False avoids prompt for new hosts
export ANSIBLE_HOST_KEY_CHECKING=False

# Patch playbook to allow remote execution (default is localhost)
if [ "$(uname)" == "Darwin" ]; then
    sed -i '' 's/hosts: localhost/hosts: all/g' "$DEPLOY_DIR/playbook.yml"
else
    sed -i 's/hosts: localhost/hosts: all/g' "$DEPLOY_DIR/playbook.yml"
fi

if ansible-playbook -i "$TARGET_IP," "$DEPLOY_DIR/playbook.yml" \
    -e "target_host=$TARGET_IP" \
    -e "tailscale_enabled=true" \
    -e "ansible_user=$SSH_USER"; then
    
    echo ""
    echo -e "${GREEN}Deployment Complete!${NC}"
    echo "Connect your Desktop App to: ws://<tailscale-ip>:18789"
else
    echo ""
    echo -e "${RED}Deployment Failed.${NC}"
    echo "Check the logs above for errors."
    exit 1
fi
