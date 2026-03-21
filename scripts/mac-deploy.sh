#!/usr/bin/env bash
# ============================================================================
# ThinClaw — One-Click Mac Mini Deployment Script
# ============================================================================
#
# Deploys ThinClaw as a standalone headless agent on a fresh macOS machine.
# Installs all prerequisites, builds the binary, and launches the onboarding
# wizard. After onboarding, ThinClaw runs as a persistent launchd service.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/RNT56/ThinClaw/main/scripts/mac-deploy.sh | bash
#
#   # Or from a local checkout:
#   ./scripts/mac-deploy.sh
#
# Options:
#   --bundled        Build with all WASM extensions embedded (air-gapped mode)
#   --skip-build     Skip compilation (use existing binary or download release)
#   --install-only   Install prerequisites only, don't build or run
#   --help           Show this help
#
# What this script does:
#   1. Installs Xcode CLI tools (if missing)
#   2. Installs Rust toolchain via rustup (if missing)
#   3. Adds wasm32-wasip2 target (required for WASM extension compilation)
#   4. Installs wasm-tools (required for WASM component model conversion)
#   5. Installs cargo-component (required for building WASM extensions from source)
#   6. Clones ThinClaw repository (or uses existing checkout)
#   7. Builds the binary
#   8. Launches the onboarding wizard (database, LLM, channels, extensions)
#   9. Optionally installs as a launchd service for auto-start on boot
#
# ============================================================================

set -euo pipefail

# ── Color output ─────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m' # No Color

info()    { echo -e "${BLUE}==>${NC} ${BOLD}$1${NC}"; }
success() { echo -e "${GREEN}  ✓${NC} $1"; }
warn()    { echo -e "${YELLOW}  ⚠${NC} $1"; }
error()   { echo -e "${RED}  ✗${NC} $1"; }
step()    { echo -e "${BLUE}  →${NC} $1"; }

# ── Parse arguments ──────────────────────────────────────────────────────────
BUNDLED=false
SKIP_BUILD=false
INSTALL_ONLY=false
FEATURES="libsql"

while [[ "$#" -gt 0 ]]; do
    case $1 in
        --bundled)      BUNDLED=true ;;
        --skip-build)   SKIP_BUILD=true ;;
        --install-only) INSTALL_ONLY=true ;;
        --help|-h)
            head -35 "$0" | tail -30
            exit 0
            ;;
        *) echo "Unknown option: $1. Use --help for usage."; exit 1 ;;
    esac
    shift
done

if [ "$BUNDLED" = true ]; then
    FEATURES="libsql,bundled-wasm"
fi

# ── Banner ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}╭──────────────────────────────────────────────────────╮${NC}"
echo -e "${BOLD}│         🦀 ThinClaw — Mac Deployment Setup          │${NC}"
echo -e "${BOLD}╰──────────────────────────────────────────────────────╯${NC}"
echo ""
echo "  Mode: $([ "$BUNDLED" = true ] && echo 'Air-gapped (all WASM embedded)' || echo 'Standard (extensions downloaded on demand)')"
echo "  Features: $FEATURES"
echo ""

# ── Detect architecture ─────────────────────────────────────────────────────
ARCH=$(uname -m)
if [[ "$ARCH" == "arm64" ]]; then
    step "Detected Apple Silicon (arm64)"
elif [[ "$ARCH" == "x86_64" ]]; then
    step "Detected Intel Mac (x86_64)"
else
    error "Unsupported architecture: $ARCH"
    exit 1
fi

# ============================================================================
# 1. XCODE CLI TOOLS
# ============================================================================
info "[1/7] Checking Xcode Command Line Tools..."

if xcode-select -p &>/dev/null; then
    success "Xcode CLI tools already installed"
else
    step "Installing Xcode Command Line Tools (this may take a few minutes)..."
    xcode-select --install 2>/dev/null || true
    # Wait for installation to complete
    echo "    Please complete the Xcode CLI tools installation dialog, then press Enter."
    read -r
    if ! xcode-select -p &>/dev/null; then
        error "Xcode CLI tools installation failed. Please install manually:"
        echo "    xcode-select --install"
        exit 1
    fi
    success "Xcode CLI tools installed"
fi

# ============================================================================
# 2. RUST TOOLCHAIN
# ============================================================================
info "[2/7] Checking Rust toolchain..."

if command -v rustup &>/dev/null; then
    RUST_VERSION=$(rustc --version 2>/dev/null | awk '{print $2}' || echo "unknown")
    success "Rust $RUST_VERSION already installed"

    # Ensure we're on a recent enough version (1.92+)
    MIN_MAJOR=1
    MIN_MINOR=92
    MAJOR=$(echo "$RUST_VERSION" | cut -d. -f1)
    MINOR=$(echo "$RUST_VERSION" | cut -d. -f2)
    if [[ "$MAJOR" -lt "$MIN_MAJOR" ]] || [[ "$MAJOR" -eq "$MIN_MAJOR" && "$MINOR" -lt "$MIN_MINOR" ]]; then
        warn "Rust $RUST_VERSION is below minimum 1.92. Updating..."
        rustup update stable
    fi
else
    step "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    source "$HOME/.cargo/env"
    success "Rust $(rustc --version | awk '{print $2}') installed"
fi

# Ensure cargo is in PATH for subsequent commands
export PATH="$HOME/.cargo/bin:$PATH"

# ============================================================================
# 3. WASM TARGET
# ============================================================================
info "[3/7] Adding wasm32-wasip2 target..."

if rustup target list --installed | grep -q wasm32-wasip2; then
    success "wasm32-wasip2 target already installed"
else
    rustup target add wasm32-wasip2
    success "wasm32-wasip2 target added"
fi

# ============================================================================
# 4. WASM TOOLS
# ============================================================================
info "[4/7] Checking wasm-tools..."

if command -v wasm-tools &>/dev/null; then
    success "wasm-tools already installed: $(wasm-tools --version)"
else
    step "Installing wasm-tools (WASM component model conversion)..."
    cargo install wasm-tools --locked
    success "wasm-tools installed"
fi

# ============================================================================
# 5. CARGO-COMPONENT
# ============================================================================
info "[5/7] Checking cargo-component..."

if cargo component --version &>/dev/null 2>&1; then
    success "cargo-component already installed"
else
    step "Installing cargo-component (WASM extension builder)..."
    cargo install cargo-component --locked
    success "cargo-component installed"
fi

if [ "$INSTALL_ONLY" = true ]; then
    echo ""
    info "Prerequisites installed. Skipping build (--install-only)."
    echo ""
    echo "  Next steps:"
    echo "    git clone https://github.com/RNT56/ThinClaw.git && cd ThinClaw"
    echo "    cargo build --release --features $FEATURES"
    echo "    ./target/release/thinclaw"
    exit 0
fi

# ============================================================================
# 6. CLONE OR LOCATE REPOSITORY
# ============================================================================
info "[6/7] Locating ThinClaw source..."

# Detect if we're already in a ThinClaw checkout
THINCLAW_DIR=""
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd || echo "")"

if [[ -n "$SCRIPT_DIR" && -f "$SCRIPT_DIR/../Cargo.toml" ]]; then
    THINCLAW_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
    success "Using existing checkout: $THINCLAW_DIR"
elif [[ -f "Cargo.toml" ]] && grep -q 'name = "thinclaw"' Cargo.toml 2>/dev/null; then
    THINCLAW_DIR="$(pwd)"
    success "Using current directory: $THINCLAW_DIR"
else
    step "Cloning ThinClaw repository..."
    THINCLAW_DIR="$HOME/ThinClaw"
    if [[ -d "$THINCLAW_DIR" ]]; then
        warn "Directory $THINCLAW_DIR already exists. Pulling latest..."
        git -C "$THINCLAW_DIR" pull --ff-only
    else
        git clone https://github.com/RNT56/ThinClaw.git "$THINCLAW_DIR"
    fi
    success "Repository ready: $THINCLAW_DIR"
fi

# ============================================================================
# 7. BUILD
# ============================================================================

if [ "$SKIP_BUILD" = true ]; then
    info "[7/7] Skipping build (--skip-build)"
else
    info "[7/7] Building ThinClaw (this may take 3-10 minutes on first build)..."
    echo "  Features: $FEATURES"

    cd "$THINCLAW_DIR"

    if [ "$BUNDLED" = true ]; then
        step "Building with bundled WASM extensions..."
        cargo build --release --features "$FEATURES"
    else
        step "Building standard binary..."
        # Build bundled channels (telegram) first
        if [[ -f "scripts/build-all.sh" ]]; then
            bash scripts/build-all.sh
        else
            cargo build --release --features "$FEATURES"
        fi
    fi

    success "Build complete: $THINCLAW_DIR/target/release/thinclaw"
fi

# ============================================================================
# SUMMARY & NEXT STEPS
# ============================================================================

BINARY="$THINCLAW_DIR/target/release/thinclaw"

echo ""
echo -e "${BOLD}╭──────────────────────────────────────────────────────╮${NC}"
echo -e "${BOLD}│         ✅ ThinClaw Deployment Ready!                │${NC}"
echo -e "${BOLD}╰──────────────────────────────────────────────────────╯${NC}"
echo ""
echo "  Binary:  $BINARY"
echo "  Config:  ~/.thinclaw/.env"
echo "  Data:    ~/.thinclaw/"
echo ""
echo -e "${BOLD}  Next steps:${NC}"
echo ""
echo "  1. Run the onboarding wizard (first-time setup):"
echo "     $BINARY"
echo ""
echo "  2. (Optional) Install as a launchd service for auto-start on boot:"
echo "     $BINARY service install"
echo ""
echo "  3. Connect Scrappy desktop app:"
echo "     Settings → Gateway → Connect Existing"
echo "     URL: http://<this-machine-ip>:<gateway-port>"
echo ""

if [ "$BUNDLED" = true ]; then
    echo -e "  ${GREEN}Air-gapped mode:${NC} All WASM extensions are embedded in the binary."
    echo "  Extensions are extracted to ~/.thinclaw/ on first install — no network needed."
else
    echo -e "  ${BLUE}Standard mode:${NC} WASM extensions will be downloaded from GitHub Releases"
    echo "  on first install (requires internet access)."
fi
echo ""

# Offer to launch immediately
if [[ -f "$BINARY" ]]; then
    echo -e "  ${BOLD}Launch ThinClaw now? [Y/n]${NC}"
    read -r -n 1 LAUNCH
    echo ""
    if [[ ! "$LAUNCH" =~ ^[Nn]$ ]]; then
        exec "$BINARY"
    fi
fi
