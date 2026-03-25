#!/usr/bin/env bash
# ============================================================================
# ThinClaw — One-Click Mac Mini Deployment Script
# ============================================================================
#
# Deploys ThinClaw as a standalone headless agent on a fresh macOS machine.
# Installs all prerequisites, builds the binary, runs the onboarding wizard,
# and optionally installs as a persistent launchd service.
#
# Usage:
#   # From a fresh machine (no repo needed):
#   curl -fsSL https://raw.githubusercontent.com/RNT56/ThinClaw/main/scripts/mac-deploy.sh | bash
#
#   # With options (fresh machine):
#   curl -fsSL https://raw.githubusercontent.com/RNT56/ThinClaw/main/scripts/mac-deploy.sh | bash -s -- --bundled
#
#   # From a local checkout:
#   ./scripts/mac-deploy.sh
#
# Options:
#   --bundled        Build with all WASM extensions embedded (air-gapped mode)
#   --skip-build     Skip compilation (use existing binary or download release)
#   --install-only   Install prerequisites only, don't build or run
#   --no-launch      Don't prompt to launch after build
#   --help           Show this help
#
# What this script does:
#   1. Checks macOS version compatibility
#   2. Installs Xcode CLI tools (if missing)
#   3. Installs Rust 1.92+ toolchain via rustup (if missing)
#   4. Adds wasm32-wasip2 target (required for WASM extension compilation)
#   5. Installs wasm-tools and cargo-component
#   6. Clones ThinClaw repository (or uses existing checkout)
#   7. Builds the release binary
#   8. Offers to launch the onboarding wizard
#   9. Offers to install as a launchd service for auto-start on boot
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

# ── Detect if running interactively or via pipe ────────────────────────────
# When run via `curl ... | bash`, stdin is the script, not the terminal.
# We need a TTY for interactive prompts.
IS_INTERACTIVE=false
if [[ -t 0 ]]; then
    IS_INTERACTIVE=true
elif [[ -e /dev/tty ]]; then
    # stdin is piped but /dev/tty is available — we can still prompt
    IS_INTERACTIVE=true
fi

# Safe read function that works both in pipe and interactive mode
safe_read() {
    if [[ "$IS_INTERACTIVE" == true ]] && [[ -e /dev/tty ]]; then
        read "$@" < /dev/tty
    elif [[ -t 0 ]]; then
        read "$@"
    else
        # Non-interactive, no TTY — return empty (default behavior)
        REPLY=""
        return 0
    fi
}

# ── Parse arguments ──────────────────────────────────────────────────────────
BUNDLED=false
SKIP_BUILD=false
INSTALL_ONLY=false
NO_LAUNCH=false
FEATURES="libsql"

while [[ "$#" -gt 0 ]]; do
    case $1 in
        --bundled)      BUNDLED=true ;;
        --skip-build)   SKIP_BUILD=true ;;
        --install-only) INSTALL_ONLY=true ;;
        --no-launch)    NO_LAUNCH=true ;;
        --help|-h)
            # Print the header comment block as help text (works in pipe mode too)
            cat << 'HELP'
ThinClaw — One-Click Mac Mini Deployment Script

Usage:
  curl -fsSL https://raw.githubusercontent.com/RNT56/ThinClaw/main/scripts/mac-deploy.sh | bash
  ./scripts/mac-deploy.sh              # from a local checkout

Options:
  --bundled        Build with all WASM extensions embedded (air-gapped mode)
  --skip-build     Skip compilation (use existing binary)
  --install-only   Install prerequisites only, don't build or run
  --no-launch      Don't prompt to launch after build
  --help           Show this help

Steps:
  1. Checks macOS version compatibility
  2. Installs Xcode CLI tools (if missing)
  3. Installs Rust 1.92+ toolchain via rustup (if missing)
  4. Adds wasm32-wasip2 target
  5. Installs wasm-tools and cargo-component
  6. Clones ThinClaw repository (or uses existing checkout)
  7. Builds the release binary
  8. Offers to launch onboarding wizard + install as launchd service
HELP
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

# ── Check this is macOS ─────────────────────────────────────────────────────
if [[ "$(uname -s)" != "Darwin" ]]; then
    error "This script is for macOS only. For Linux, see docs/DEPLOYMENT.md."
    exit 1
fi

# ── Check macOS version ─────────────────────────────────────────────────────
MACOS_VERSION=$(sw_vers -productVersion 2>/dev/null || echo "unknown")
MACOS_MAJOR=$(echo "$MACOS_VERSION" | cut -d. -f1)

if [[ "$MACOS_MAJOR" != "unknown" ]] && [[ "$MACOS_MAJOR" -lt 11 ]]; then
    error "macOS $MACOS_VERSION is not supported. ThinClaw requires macOS 11 (Big Sur) or later."
    exit 1
fi
step "macOS $MACOS_VERSION detected"

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
    if [[ "$IS_INTERACTIVE" == true ]]; then
        echo "    Please complete the Xcode CLI tools installation dialog, then press Enter."
        safe_read -r
    else
        # Non-interactive: wait for xcode-select to become available (up to 10 min)
        step "Waiting for Xcode CLI tools installation (non-interactive mode)..."
        for i in $(seq 1 120); do
            if xcode-select -p &>/dev/null; then
                break
            fi
            sleep 5
        done
    fi
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

# Source cargo env if it exists (may have been installed in a previous run)
# shellcheck disable=SC1091
[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"

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
    # shellcheck disable=SC1091
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

# Method 1: Script was run from a local checkout (./scripts/mac-deploy.sh)
if [[ -n "${BASH_SOURCE[0]:-}" ]] && [[ "${BASH_SOURCE[0]}" != "bash" ]]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd || echo "")"
    if [[ -n "$SCRIPT_DIR" && -f "$SCRIPT_DIR/../Cargo.toml" ]]; then
        # Verify it's actually ThinClaw, not some other Rust project
        if grep -q 'name = "thinclaw"' "$SCRIPT_DIR/../Cargo.toml" 2>/dev/null; then
            THINCLAW_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
            success "Using existing checkout: $THINCLAW_DIR"
        fi
    fi
fi

# Method 2: We're already in a ThinClaw directory
if [[ -z "$THINCLAW_DIR" && -f "Cargo.toml" ]] && grep -q 'name = "thinclaw"' Cargo.toml 2>/dev/null; then
    THINCLAW_DIR="$(pwd)"
    success "Using current directory: $THINCLAW_DIR"
fi

# Method 3: Clone or update the repo
if [[ -z "$THINCLAW_DIR" ]]; then
    THINCLAW_DIR="$HOME/ThinClaw"
    if [[ -d "$THINCLAW_DIR/.git" ]]; then
        warn "Directory $THINCLAW_DIR already exists. Pulling latest..."
        # Use rebase to handle minor divergence; reset as last resort
        if ! git -C "$THINCLAW_DIR" pull --ff-only 2>/dev/null; then
            warn "Fast-forward failed. Trying rebase..."
            if ! git -C "$THINCLAW_DIR" pull --rebase 2>/dev/null; then
                warn "Rebase failed. Stashing local changes and resetting to origin..."
                git -C "$THINCLAW_DIR" stash 2>/dev/null || true
                git -C "$THINCLAW_DIR" fetch origin
                git -C "$THINCLAW_DIR" reset --hard origin/main
            fi
        fi
    elif [[ -d "$THINCLAW_DIR" ]]; then
        # Directory exists but isn't a git repo — don't destroy it
        error "$THINCLAW_DIR exists but is not a git repository."
        echo "    Please remove it or choose a different location:"
        echo "    rm -rf $THINCLAW_DIR"
        exit 1
    else
        step "Cloning ThinClaw repository..."
        git clone https://github.com/RNT56/ThinClaw.git "$THINCLAW_DIR"
    fi
    success "Repository ready: $THINCLAW_DIR"
fi

# ============================================================================
# 7. BUILD
# ============================================================================

if [ "$SKIP_BUILD" = true ]; then
    info "[7/7] Skipping build (--skip-build)"
    if [[ ! -f "$THINCLAW_DIR/target/release/thinclaw" ]]; then
        warn "No binary found at $THINCLAW_DIR/target/release/thinclaw"
        echo "  Run without --skip-build to compile, or download a pre-built binary."
    fi
else
    info "[7/7] Building ThinClaw (this may take 3-10 minutes on first build)..."
    echo "  Features: $FEATURES"

    cd "$THINCLAW_DIR"

    # Always use the direct cargo build — build-all.sh is for developers
    # who have channel sources checked out. The standard build downloads
    # WASM extensions from GitHub Releases on first install.
    step "Building binary..."
    cargo build --release --features "$FEATURES"

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
echo "     $BINARY service start"
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

# ── Make `thinclaw` available on PATH ────────────────────────────────────────
BINARY_DIR="$(dirname "$BINARY")"
SYMLINK_TARGET="/usr/local/bin/thinclaw"

if command -v thinclaw &>/dev/null; then
    EXISTING="$(command -v thinclaw)"
    success "thinclaw is already on PATH: $EXISTING"
elif [[ -d "/usr/local/bin" ]] && [[ -f "$BINARY" ]]; then
    # /usr/local/bin exists (created by Xcode CLI tools) and is in PATH on macOS
    step "Adding thinclaw to PATH via symlink..."
    if ln -sf "$BINARY" "$SYMLINK_TARGET" 2>/dev/null; then
        success "Symlinked: $SYMLINK_TARGET → $BINARY"
        echo "  You can now use 'thinclaw' from any terminal."
    else
        # Symlink failed (permissions?) — try with sudo
        warn "Symlink to $SYMLINK_TARGET requires elevated permissions."
        if [[ "$IS_INTERACTIVE" == true ]]; then
            echo -e "  ${BOLD}Create symlink with sudo? [Y/n]${NC}"
            safe_read -r -n 1 DO_SUDO
            echo ""
            if [[ ! "${DO_SUDO:-Y}" =~ ^[Nn]$ ]]; then
                if sudo ln -sf "$BINARY" "$SYMLINK_TARGET"; then
                    success "Symlinked: $SYMLINK_TARGET → $BINARY"
                    echo "  You can now use 'thinclaw' from any terminal."
                else
                    warn "Symlink failed. Add manually:"
                    echo "    echo 'export PATH=\"$BINARY_DIR:\$PATH\"' >> ~/.zshrc && source ~/.zshrc"
                fi
            else
                echo -e "  ${YELLOW}Tip:${NC} Add ThinClaw to your PATH manually:"
                echo "    echo 'export PATH=\"$BINARY_DIR:\$PATH\"' >> ~/.zshrc && source ~/.zshrc"
            fi
        else
            echo -e "  ${YELLOW}Tip:${NC} Add ThinClaw to your PATH:"
            echo "    sudo ln -sf $BINARY $SYMLINK_TARGET"
            echo "  Or:"
            echo "    echo 'export PATH=\"$BINARY_DIR:\$PATH\"' >> ~/.zshrc && source ~/.zshrc"
        fi
    fi
    echo ""
else
    echo -e "  ${YELLOW}Tip:${NC} Add ThinClaw to your PATH for convenience:"
    echo "    echo 'export PATH=\"$BINARY_DIR:\$PATH\"' >> ~/.zshrc && source ~/.zshrc"
    echo ""
fi

# ── Offer to launch ─────────────────────────────────────────────────────────
if [[ -f "$BINARY" ]] && [[ "$NO_LAUNCH" != true ]] && [[ "$IS_INTERACTIVE" == true ]]; then
    echo -e "  ${BOLD}Launch ThinClaw now? [Y/n]${NC}"
    safe_read -r -n 1 LAUNCH
    echo ""
    if [[ ! "${LAUNCH:-Y}" =~ ^[Nn]$ ]]; then
        echo ""
        info "Launching ThinClaw (first run starts the onboarding wizard)..."
        echo ""
        exec "$BINARY"
    fi
elif [[ -f "$BINARY" ]] && [[ "$IS_INTERACTIVE" != true ]]; then
    echo -e "  ${YELLOW}Non-interactive mode:${NC} Skipping launch prompt."
    echo "  Run '$BINARY' to start the onboarding wizard."
fi
