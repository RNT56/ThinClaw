#!/usr/bin/env sh
set -eu

REPO="${THINCLAW_REPO:-RNT56/ThinClaw}"
PROFILE="full"
VERSION="${THINCLAW_VERSION:-latest}"
PREFIX=""
SYSTEM="false"
STATIC="false"
DRY_RUN="false"

usage() {
  cat <<'EOF'
ThinClaw installer

Usage:
  thinclaw-installer.sh [--profile full|edge] [--version <tag>] [--prefix <dir>] [--system] [--static] [--dry-run]

Defaults:
  --profile full
  install path ~/.local/bin, or /usr/local/bin when run as root/--system
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --profile) PROFILE="$2"; shift ;;
    --version) VERSION="$2"; shift ;;
    --prefix) PREFIX="$2"; shift ;;
    --system) SYSTEM="true" ;;
    --static) STATIC="true" ;;
    --dry-run) DRY_RUN="true" ;;
    --help|-h) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

case "$PROFILE" in
  full|edge) ;;
  *) echo "unsupported profile: $PROFILE" >&2; exit 2 ;;
esac

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "required command not found: $1" >&2
    exit 1
  fi
}

need curl
need tar

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS:$ARCH" in
  Darwin:arm64) TARGET="aarch64-apple-darwin" ;;
  Darwin:x86_64) TARGET="x86_64-apple-darwin" ;;
  Linux:aarch64|Linux:arm64)
    if [ "$STATIC" = "true" ]; then
      TARGET="aarch64-unknown-linux-musl"
    else
      TARGET="aarch64-unknown-linux-gnu"
    fi
    ;;
  Linux:x86_64|Linux:amd64)
    if [ "$STATIC" = "true" ]; then
      TARGET="x86_64-unknown-linux-musl"
    else
      TARGET="x86_64-unknown-linux-gnu"
    fi
    ;;
  *) echo "unsupported OS/arch: $OS/$ARCH" >&2; exit 1 ;;
esac

if [ -z "$PREFIX" ]; then
  if [ "$SYSTEM" = "true" ] || [ "$(id -u 2>/dev/null || echo 1)" = "0" ]; then
    PREFIX="/usr/local/bin"
  else
    PREFIX="$HOME/.local/bin"
  fi
fi

if [ "$PROFILE" = "edge" ]; then
  ARCHIVE="thinclaw-edge-${TARGET}.tar.gz"
else
  ARCHIVE="thinclaw-${TARGET}.tar.gz"
fi

if [ "$VERSION" = "latest" ]; then
  BASE_URL="https://github.com/${REPO}/releases/latest/download"
else
  BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
fi

TMPDIR="$(mktemp -d "${TMPDIR:-/tmp}/thinclaw-install.XXXXXX")"
cleanup() {
  rm -rf "$TMPDIR"
}
trap cleanup EXIT INT TERM

ARCHIVE_URL="${BASE_URL}/${ARCHIVE}"
if [ "$PROFILE" = "edge" ]; then
  CHECKSUM_NAME="checksums-edge.txt"
else
  CHECKSUM_NAME="checksums.txt"
fi
CHECKSUM_URL="${BASE_URL}/${CHECKSUM_NAME}"

echo "ThinClaw installer"
echo "  profile: $PROFILE"
echo "  target:  $TARGET"
echo "  archive: $ARCHIVE_URL"
echo "  install: $PREFIX/thinclaw"

if [ "$DRY_RUN" = "true" ]; then
  exit 0
fi

mkdir -p "$PREFIX"

curl --proto '=https' --tlsv1.2 -fsSL "$ARCHIVE_URL" -o "$TMPDIR/$ARCHIVE"
curl --proto '=https' --tlsv1.2 -fsSL "$CHECKSUM_URL" -o "$TMPDIR/checksums.txt"

EXPECTED="$(awk -v f="$ARCHIVE" '$2 == f || $2 == "*" f { print $1; exit }' "$TMPDIR/checksums.txt")"
if [ -z "$EXPECTED" ]; then
  echo "checksum for $ARCHIVE not found in checksums.txt" >&2
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "$TMPDIR/$ARCHIVE" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  ACTUAL="$(shasum -a 256 "$TMPDIR/$ARCHIVE" | awk '{print $1}')"
else
  echo "required command not found: sha256sum or shasum" >&2
  exit 1
fi

if [ "$EXPECTED" != "$ACTUAL" ]; then
  echo "checksum mismatch for $ARCHIVE" >&2
  echo "expected: $EXPECTED" >&2
  echo "actual:   $ACTUAL" >&2
  exit 1
fi

tar -xzf "$TMPDIR/$ARCHIVE" -C "$TMPDIR"
BIN="$(find "$TMPDIR" -type f -name thinclaw -perm -111 | head -n 1)"
if [ -z "$BIN" ]; then
  BIN="$(find "$TMPDIR" -type f -name thinclaw | head -n 1)"
fi
if [ -z "$BIN" ]; then
  echo "archive did not contain a thinclaw binary" >&2
  exit 1
fi

install -m 0755 "$BIN" "$PREFIX/thinclaw"

case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *) echo "warning: $PREFIX is not on PATH" >&2 ;;
esac

echo "installed: $PREFIX/thinclaw"
