#!/bin/bash
set -e

# Default to a known good tag (b4406 is known to be static/complete for macOS)
TAG_NAME="${1:-b4406}" 

OS=$(uname -s)
ARCH=$(uname -m)

echo "Detected OS: $OS"
echo "Detected Arch: $ARCH"

# Determine Asset Name
if [ "$OS" = "Darwin" ]; then
    if [ "$ARCH" = "arm64" ]; then
        ASSET_SUFFIX="bin-macos-arm64"
        TARGET_NAME="llama-server-aarch64-apple-darwin"
    elif [ "$ARCH" = "x86_64" ]; then
        ASSET_SUFFIX="bin-macos-x64"
        TARGET_NAME="llama-server-x86_64-apple-darwin"
    else
        echo "Unsupported Architecture: $ARCH"
        exit 1
    fi
elif [ "$OS" = "Linux" ]; then
    if [ "$ARCH" = "x86_64" ]; then
        ASSET_SUFFIX="bin-ubuntu-x64" # Common Linux build, usually Ubuntu based
        TARGET_NAME="llama-server-x86_64-unknown-linux-gnu"
    else
        echo "Unsupported Linux Architecture: $ARCH"
        exit 1
    fi
# Windows usually runs this script via Git Bash or WSL, reporting MINGW or similar
elif [[ "$OS" == *"MINGW"* ]] || [[ "$OS" == *"CYGWIN"* ]] || [[ "$OS" == *"MSYS"* ]]; then
    echo "Detected Windows environment."
    # Default to AVX2 build for compatibility. Users with CUDA should manually setup or we can add args.
    ASSET_SUFFIX="bin-win-avx2-x64"
    TARGET_NAME="llama-server-x86_64-pc-windows-msvc.exe"
else
    echo "Unsupported OS: $OS"
    exit 1
fi

echo "Using release: $TAG_NAME"
echo "Target asset: llama-${TAG_NAME}-${ASSET_SUFFIX}.zip"

ASSET_NAME="llama-${TAG_NAME}-${ASSET_SUFFIX}.zip"
DOWNLOAD_URL="https://github.com/ggerganov/llama.cpp/releases/download/${TAG_NAME}/${ASSET_NAME}"

mkdir -p src-tauri/bin

echo "Downloading $DOWNLOAD_URL ..."
curl -L -o "src-tauri/bin/$ASSET_NAME" "$DOWNLOAD_URL"

echo "Extracting..."
mkdir -p "src-tauri/bin/temp"
unzip -o "src-tauri/bin/$ASSET_NAME" -d "src-tauri/bin/temp"

# Find the binary
if [[ "$OS" == *"MINGW"* ]] || [[ "$OS" == *"CYGWIN"* ]] || [[ "$OS" == *"MSYS"* ]]; then
    FOUND_BIN=$(find src-tauri/bin/temp -name "llama-server.exe" -type f | head -n 1)
else
    FOUND_BIN=$(find src-tauri/bin/temp -name "llama-server" -type f | head -n 1)
fi

if [ -z "$FOUND_BIN" ]; then
    echo "Error: llama-server binary not found in zip."
    exit 1
fi

echo "Moving $FOUND_BIN to src-tauri/bin/$TARGET_NAME"
mv "$FOUND_BIN" "src-tauri/bin/$TARGET_NAME"

# Checks for associated libraries (dylibs)
if [ "$OS" = "Darwin" ]; then
    FOUND_LIB=$(find src-tauri/bin/temp -name "libllama.dylib" -type f | head -n 1)
    if [ ! -z "$FOUND_LIB" ]; then
        echo "Found libllama.dylib, moving to src-tauri/bin/"
        cp "$FOUND_LIB" "src-tauri/bin/libllama.dylib"
        
        # Try to fix rpath using install_name_tool
        if command -v install_name_tool &> /dev/null; then
            echo "Fixing rpath for libllama.dylib..."
            install_name_tool -add_rpath "@executable_path" "src-tauri/bin/$TARGET_NAME" || true
            install_name_tool -change "@rpath/libllama.dylib" "@executable_path/libllama.dylib" "src-tauri/bin/$TARGET_NAME" || true
        fi
    fi
    # Also look for ggml libraries if split
    find src-tauri/bin/temp -name "libggml*.dylib" -type f -exec cp {} "src-tauri/bin/" \;
fi

if [[ "$OS" != *"MINGW"* ]] && [[ "$OS" != *"CYGWIN"* ]] && [[ "$OS" != *"MSYS"* ]]; then
    chmod +x "src-tauri/bin/$TARGET_NAME"
fi

# Cleanup
rm "src-tauri/bin/$ASSET_NAME"
rm -rf "src-tauri/bin/temp"
rm -rf "src-tauri/bin/__MACOSX"

echo "Done! Binary setup at src-tauri/bin/$TARGET_NAME"
