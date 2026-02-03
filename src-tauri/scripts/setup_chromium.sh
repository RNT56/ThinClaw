#!/bin/bash
set -e

# Configuration
CHROMIUM_REVISION="1313161"
TARGET_DIR="src-tauri/resources/chromium"
mkdir -p "$TARGET_DIR"

# Detect Architecture
ARCH=$(uname -m)
if [ "$ARCH" == "arm64" ]; then
    PLATFORM="Mac_Arm"
else
    PLATFORM="Mac"
fi

ZIP_NAME="chrome-mac.zip"
DOWNLOAD_URL="https://storage.googleapis.com/chromium-browser-snapshots/${PLATFORM}/${CHROMIUM_REVISION}/${ZIP_NAME}"

echo "Detected architecture: $ARCH ($PLATFORM)"
echo "Downloading Chromium r${CHROMIUM_REVISION}..."

# Download to temp file
curl -L "$DOWNLOAD_URL" -o "/tmp/$ZIP_NAME"

echo "Extracting..."
unzip -q -o "/tmp/$ZIP_NAME" -d "$TARGET_DIR"

# Mac specific cleanups
if [[ "$OSTYPE" == "darwin"* ]]; then
    APP_PATH="$TARGET_DIR/chrome-mac/Chromium.app"
    echo "Removing quarantine attributes from $APP_PATH..."
    xattr -cr "$APP_PATH"
fi

echo "Cleaning up..."
rm "/tmp/$ZIP_NAME"

echo "Chromium setup complete! Binary location: $TARGET_DIR/chrome-mac/Chromium.app"
