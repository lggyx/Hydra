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

INSTALLED=false

if ! $BUILD_FROM_SOURCE; then
    # Try downloading from GitHub releases
    if [ "$VERSION" = "latest" ]; then
        BASE_URL="https://github.com/${REPO_OWNER}/${REPO_NAME}/releases/latest/download"
    else
        BASE_URL="https://github.com/${REPO_OWNER}/${REPO_NAME}/releases/download/${VERSION}"
    fi

    FILES=(
        "hydra-daemon|${BASE_URL}/hydra-daemon-${OS_NAME}-${ARCH_NAME}"
        "hydra|${BASE_URL}/hydra-${OS_NAME}-${ARCH_NAME}"
    )

    for FILE_SPEC in "${FILES[@]}"; do
        NAME="${FILE_SPEC%%|*}"
        URL="${FILE_SPEC##*|}"
        DEST="$BIN_DIR/$NAME"
        echo "Downloading $NAME..."
        if command -v curl &>/dev/null; then
            curl -fsSL "$URL" -o "$DEST" && chmod +x "$DEST" && INSTALLED=true || {
                echo "Download failed, will build from source..."
                break
            }
        elif command -v wget &>/dev/null; then
            wget -q "$URL" -O "$DEST" && chmod +x "$DEST" && INSTALLED=true || {
                echo "Download failed, will build from source..."
                break
            }
        fi
    done
fi

if ! $INSTALLED; then
    echo "Building from source..."

    # Install Rust if not present
    if ! command -v cargo &>/dev/null; then
        echo "Rust not found. Installing Rust toolchain..."
        if command -v curl &>/dev/null; then
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        elif command -v wget &>/dev/null; then
            wget -qO- https://sh.rustup.rs | sh -s -- -y
        else
            echo "Error: curl or wget required to install Rust"
            exit 1
        fi
        # Source cargo env
        if [ -f "$HOME/.cargo/env" ]; then
            . "$HOME/.cargo/env"
        fi
        echo "Rust installed: $(rustc --version)"
    fi

    if ! command -v git &>/dev/null; then
        echo "Installing git..."
        if command -v apt-get &>/dev/null; then
            apt-get update -qq && apt-get install -y -qq git
        elif command -v yum &>/dev/null; then
            yum install -y -q git
        elif command -v apk &>/dev/null; then
            apk add --no-cache git
        elif command -v brew &>/dev/null; then
            brew install git
        else
            echo "Warning: git not found, please install git manually"
        fi
    fi

    if [ -f "Cargo.toml" ] && grep -q "hydra" Cargo.toml 2>/dev/null; then
        cargo build --release -p hydra-daemon -p hydra
    else
        TMPDIR="$(mktemp -d)"
        git clone "https://github.com/${REPO_OWNER}/${REPO_NAME}.git" "$TMPDIR"
        cd "$TMPDIR"
        cargo build --release -p hydra-daemon -p hydra
    fi
    cp target/release/hydra-daemon "$BIN_DIR/"
    cp target/release/hydra "$BIN_DIR/"
    INSTALLED=true
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
