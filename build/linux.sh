#!/usr/bin/env bash
# ============================================================================
# Ultraclaw — Linux (Debian) Build Script
# ============================================================================
# Builds a .deb package for Debian/Ubuntu distributions.
#
# Prerequisites:
#   cargo install cargo-deb
#
# Usage:   ./build/linux.sh
# Output:  target/debian/ultraclaw_*.deb
# ============================================================================

set -euo pipefail

echo "╔══════════════════════════════════════════════════╗"
echo "║      ULTRACLAW — Linux .deb Build                ║"
echo "╚══════════════════════════════════════════════════╝"

# Check for cargo-deb
if ! command -v cargo-deb &> /dev/null; then
    echo "[!] cargo-deb not found. Installing..."
    cargo install cargo-deb
fi

# Build the .deb package
echo ""
echo "[1/2] Building release binary + .deb package..."
cargo deb --no-strip

if [ $? -ne 0 ]; then
    echo "Build FAILED!"
    exit 1
fi

# Find the output .deb file
DEB_FILE=$(find target/debian -name "ultraclaw_*.deb" -type f | head -n1)

if [ -n "$DEB_FILE" ]; then
    DEB_SIZE=$(du -h "$DEB_FILE" | cut -f1)
    echo "[2/2] Package built: $DEB_FILE ($DEB_SIZE)"
    echo ""
    echo "Install with:"
    echo "  sudo dpkg -i $DEB_FILE"
    echo ""
    echo "Or add to APT repository for distribution."
else
    echo "No .deb file found in target/debian/"
    exit 1
fi

echo "Build complete!"
