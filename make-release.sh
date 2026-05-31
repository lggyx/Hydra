#!/usr/bin/env bash
# Package Hydra release binaries for the current platform
# Usage: bash make-release.sh [--all] [--output ./dist]

set -euo pipefail

OUTPUT_DIR="${OUTPUT_DIR:-./dist}"
BUILD_ALL=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --all) BUILD_ALL=true; shift ;;
        --output) OUTPUT_DIR="$2"; shift 2 ;;
        *) echo "Usage: $0 [--all] [--output ./dist]"; exit 1 ;;
    esac
done

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS" in
    Linux)  OS_NAME="linux" ;;
    Darwin) OS_NAME="macos" ;;
    MINGW*|MSYS*|CYGWIN*) OS_NAME="windows" ;;
    *)      echo "Unknown OS: $OS"; OS_NAME="unknown" ;;
esac
case "$ARCH" in
    x86_64|amd64) ARCH_NAME="x86_64" ;;
    aarch64|arm64) ARCH_NAME="arm64" ;;
    *)           ARCH_NAME="x86_64" ;;
esac

PLATFORM="${OS_NAME}-${ARCH_NAME}"
echo "Platform: $PLATFORM"
echo "Building release binaries..."

# Build
cargo build --release -p hydra-daemon -p hydra

# Package for current platform
mkdir -p "$OUTPUT_DIR"

if [ "$OS_NAME" = "windows" ]; then
    DAEMON_SRC="target/release/hydra-daemon.exe"
    CLI_SRC="target/release/hydra.exe"
    DAEMON_DST="hydra-daemon-${PLATFORM}.exe"
    CLI_DST="hydra-${PLATFORM}.exe"
else
    DAEMON_SRC="target/release/hydra-daemon"
    CLI_SRC="target/release/hydra"
    DAEMON_DST="hydra-daemon-${PLATFORM}"
    CLI_DST="hydra-${PLATFORM}"
fi

cp "$DAEMON_SRC" "$OUTPUT_DIR/$DAEMON_DST"
cp "$CLI_SRC" "$OUTPUT_DIR/$CLI_DST"

echo ""
echo "Release binaries ready in $OUTPUT_DIR/:"
ls -lh "$OUTPUT_DIR/"
echo ""
echo "Upload these to GitHub Releases:"
echo "  https://github.com/lggyx/Hydra/releases/new"
echo ""
echo "  Tag: v$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')"
