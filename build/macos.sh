#!/usr/bin/env bash
# ============================================================================
# Ultraclaw — macOS Build Script
# ============================================================================
# Builds a .dmg disk image containing the Ultraclaw app bundle.
#
# Prerequisites:
#   cargo install cargo-bundle
#
# Usage:   ./build/macos.sh
# Output:  target/release/bundle/osx/Ultraclaw.app (app bundle)
#          ultraclaw.dmg (disk image)
# ============================================================================

set -euo pipefail

echo "╔══════════════════════════════════════════════════╗"
echo "║      ULTRACLAW — macOS .dmg Build                ║"
echo "╚══════════════════════════════════════════════════╝"

# Check for cargo-bundle
if ! command -v cargo-bundle &> /dev/null; then
    echo "[!] cargo-bundle not found. Installing..."
    cargo install cargo-bundle
fi

# Build the .app bundle
echo ""
echo "[1/3] Building release binary + .app bundle..."
cargo bundle --release

if [ $? -ne 0 ]; then
    echo "Build FAILED!"
    exit 1
fi

APP_PATH="target/release/bundle/osx/Ultraclaw.app"
if [ ! -d "$APP_PATH" ]; then
    echo "App bundle not found at $APP_PATH"
    exit 1
fi

echo "[2/3] App bundle created: $APP_PATH"

# Create .dmg from the .app bundle
echo "[3/3] Creating .dmg disk image..."
DMG_NAME="ultraclaw.dmg"

# Remove old .dmg if it exists
rm -f "$DMG_NAME"

# Create a temporary directory for the DMG contents
DMG_TMP=$(mktemp -d)
cp -r "$APP_PATH" "$DMG_TMP/"
ln -s /Applications "$DMG_TMP/Applications"

# Create the DMG
hdiutil create -volname "Ultraclaw" \
    -srcfolder "$DMG_TMP" \
    -ov -format UDZO \
    "$DMG_NAME"

# Clean up
rm -rf "$DMG_TMP"

DMG_SIZE=$(du -h "$DMG_NAME" | cut -f1)
echo ""
echo "Build complete!"
echo "DMG: $DMG_NAME ($DMG_SIZE)"
echo "Users can drag Ultraclaw.app to Applications to install."
