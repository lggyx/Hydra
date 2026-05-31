#!/usr/bin/env bash
# Hydra Installer for Linux / macOS
# Installs Hydra CLI and Daemon for Ascend CANN operator development
# Usage: curl -fsSL <url>/install.sh | bash
#        or:  bash install.sh --version latest --install-dir ~/.hydra

set -euo pipefail

VERSION="${VERSION:-latest}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.hydra}"
BUILD_FROM_SOURCE=false
REPO_OWNER="lggyx"
REPO_NAME="Hydra"

# Parse args
while [[ $# -gt 0 ]]; do
    case $1 in
        --version) VERSION="$2"; shift 2 ;;
        --install-dir) INSTALL_DIR="$2"; shift 2 ;;
        --build-from-source) BUILD_FROM_SOURCE=true; shift ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

cat << 'BANNER'
  __  __           __
 / / / /_  ______/ /________
/ /_/ / / / / __  / ___/ __  /
/ __  / /_/ / /_/ / /  / /_/ /
/_/ /_/\__, /\__,_/_/   \__,_/
      /____/

  Ascend CANN Operator Development & Testing
BANNER

echo "Installing Hydra ${VERSION}..."

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS" in
    Linux)  OS_NAME="linux" ;;
    Darwin) OS_NAME="macos" ;;
    *)      echo "Unsupported OS: $OS"; exit 1 ;;
esac
case "$ARCH" in
    x86_64|amd64) ARCH_NAME="x86_64" ;;
    aarch64|arm64) ARCH_NAME="arm64" ;;
    *)           ARCH_NAME="x86_64" ;;
esac

# Create install directories
BIN_DIR="$INSTALL_DIR/bin"
mkdir -p "$BIN_DIR"

if $BUILD_FROM_SOURCE; then
    echo "Building from source..."
    if ! command -v cargo &>/dev/null; then
        echo "Error: Rust (cargo) is required. Install from https://rustup.rs/"
        exit 1
    fi
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    cd "$SCRIPT_DIR"
    cargo build --release -p hydra-daemon -p hydra
    cp target/release/hydra-daemon "$BIN_DIR/"
    cp target/release/hydra "$BIN_DIR/"
else
    # Download from GitHub releases
    if [ "$VERSION" = "latest" ]; then
        BASE_URL="https://github.com/${REPO_OWNER}/${REPO_NAME}/releases/latest/download"
    else
        BASE_URL="https://github.com/${REPO_OWNER}/${REPO_NAME}/releases/download/${VERSION}"
    fi

    FILES=(
        "hydra-daemon|${BASE_URL}/hydra-daemon-${OS_NAME}-${ARCH_NAME}"
        "hydra|${BASE_URL}/hydra-${OS_NAME}-${ARCH_NAME}"
    )

    DOWNLOAD_OK=true
    for FILE_SPEC in "${FILES[@]}"; do
        NAME="${FILE_SPEC%%|*}"
        URL="${FILE_SPEC##*|}"
        DEST="$BIN_DIR/$NAME"
        echo "Downloading $NAME..."
        if command -v curl &>/dev/null; then
            curl -fsSL "$URL" -o "$DEST" || DOWNLOAD_OK=false
        elif command -v wget &>/dev/null; then
            wget -q "$URL" -O "$DEST" || DOWNLOAD_OK=false
        else
            echo "Error: curl or wget required"
            exit 1
        fi
        if ! $DOWNLOAD_OK; then
            echo "Download failed: $URL"
            echo "No pre-built release found for $VERSION. Building from source..."
            BUILD_FROM_SOURCE=true
            break
        fi
        chmod +x "$DEST"
    done

    if $BUILD_FROM_SOURCE; then
        if ! command -v cargo &>/dev/null; then
            echo "Error: Rust (cargo) is required. Install from https://rustup.rs/"
            echo "Or wait for pre-built releases at https://github.com/${REPO_OWNER}/${REPO_NAME}/releases"
            exit 1
        fi
        echo "Building from source..."
        SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
        cd "$SCRIPT_DIR"
        cargo build --release -p hydra-daemon -p hydra
        cp target/release/hydra-daemon "$BIN_DIR/"
        cp target/release/hydra "$BIN_DIR/"
    fi
fi

# Add to PATH
SHELL_RC=""
case "$SHELL" in
    */bash) SHELL_RC="$HOME/.bashrc" ;;
    */zsh)  SHELL_RC="$HOME/.zshrc" ;;
    */fish) SHELL_RC="$HOME/.config/fish/config.fish" ;;
esac

if [ -n "$SHELL_RC" ]; then
    if ! grep -q "$BIN_DIR" "$SHELL_RC" 2>/dev/null; then
        echo "export PATH=\"$BIN_DIR:\$PATH\"" >> "$SHELL_RC"
        echo "Added $BIN_DIR to PATH in $SHELL_RC"
    fi
fi
export PATH="$BIN_DIR:$PATH"

# Set default env
HYDRA_ENV_FILE="$HOME/.hydra/env"
mkdir -p "$HOME/.hydra"
echo "HYDRA_DAEMON_PORT=13456" > "$HYDRA_ENV_FILE"
echo "export HYDRA_DAEMON_PORT=13456" >> "$HYDRA_ENV_FILE"

cat << EOF

Installation complete!

  hydra         - Launch TUI
  hydra-daemon  - Start API server

Quick start:
  1. Start daemon:  hydra-daemon
  2. In another terminal:  hydra
  3. In TUI:  /login  (free API quota)
  4. Create orchestrator:  /agents create --kind orchestrator
  5. Start operator dev:  /agents <id> start "implement Mul operator"

  Review layer: https://gitcode.com/cann/cannbot-skills
  Docs: https://github.com/${REPO_OWNER}/${REPO_NAME}

EOF
