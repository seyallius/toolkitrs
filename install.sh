#!/usr/bin/env bash
# script install.sh - One-line installer for toolkitrs on Unix-like systems (Linux/macOS).
# Fetches the latest (or specified) release from GitHub, extracts the binary,
# and installs it to a directory in the user's PATH.

set -e

# ----------------------- Configuration -----------------------

REPO="seyallius/toolkitrs"
BINARY_NAME="toolkitrs"
INSTALL_DIR="/usr/local/bin"

# ----------------------- OS & Arch Detection -----------------------

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux*)  OS_STR="unknown-linux-gnu"; EXT="tar.gz";;
    Darwin*) OS_STR="apple-darwin"; EXT="tar.gz";;
    *)       echo "❌ Unsupported OS: $OS"; exit 1;;
esac

case "$ARCH" in
    x86_64|amd64) ARCH_STR="x86_64";;
    arm64|aarch64) ARCH_STR="aarch64";;
    *)             echo "❌ Unsupported architecture: $ARCH"; exit 1;;
esac

TARGET="${ARCH_STR}-${OS_STR}"

# ----------------------- Version Resolution -----------------------

if [ -n "$1" ]; then
    VERSION="$1"
    echo "📦 Installing $BINARY_NAME version $VERSION for $TARGET..."
else
    echo "🔍 Fetching latest version..."
    VERSION=$(curl -s "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
    if [ -z "$VERSION" ]; then
        echo "❌ Failed to fetch latest version. Check your internet connection or repo name."
        exit 1
    fi
    echo "📦 Installing $BINARY_NAME version $VERSION for $TARGET..."
fi

# ----------------------- Download & Extract -----------------------

# 👇 FIXED: Include $VERSION in the asset name to match your CI output
ASSET_NAME="${BINARY_NAME}-${VERSION}-${TARGET}.${EXT}"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET_NAME}"

TMP_DIR=$(mktemp -d)
cd "$TMP_DIR"

echo "⬇️  Downloading $ASSET_NAME..."
if ! curl -fsSL "$DOWNLOAD_URL" -o "$ASSET_NAME"; then
    echo "❌ Failed to download asset. Does $ASSET_NAME exist in release $VERSION?"
    exit 1
fi

echo "📂 Extracting..."
if [[ "$ASSET_NAME" == *.tar.gz ]] || [[ "$ASSET_NAME" == *.tgz ]]; then
    tar -xzf "$ASSET_NAME"
elif [[ "$ASSET_NAME" == *.zip ]]; then
    unzip -q "$ASSET_NAME"
elif [[ "$ASSET_NAME" == *.tar.xz ]]; then
    tar -xJf "$ASSET_NAME"
else
    echo "❌ Unknown archive format: $ASSET_NAME"
    exit 1
fi

# Find the binary (it might be in the root or a subfolder depending on your CI)
BIN_PATH=$(find . -name "$BINARY_NAME" -type f | head -n 1)
if [ -z "$BIN_PATH" ]; then
    echo "❌ Error: Could not find binary '$BINARY_NAME' in the extracted archive."
    exit 1
fi

# ----------------------- Installation -----------------------

# Use /usr/local/bin if writable, otherwise fallback to ~/.local/bin
if [ -w "/usr/local/bin" ]; then
    INSTALL_DIR="/usr/local/bin"
else
    INSTALL_DIR="$HOME/.local/bin"
    mkdir -p "$INSTALL_DIR"

    # Add to PATH for current session
    export PATH="$INSTALL_DIR:$PATH"
    echo "🔗 Added $INSTALL_DIR to PATH for this session"

    # Suggest adding to profile for persistence
    echo ""
    echo "⚠️  To make $BINARY_NAME available in future terminals, add this to your shell profile:"
    echo ""

    # Detect shell and show appropriate command
    SHELL_NAME=$(basename "$SHELL")
    case "$SHELL_NAME" in
        bash)
            echo "   echo 'export PATH=\"\$PATH:$INSTALL_DIR\"' >> ~/.bashrc"
            echo "   source ~/.bashrc"
            ;;
        zsh)
            echo "   echo 'export PATH=\"\$PATH:$INSTALL_DIR\"' >> ~/.zshrc"
            echo "   source ~/.zshrc"
            ;;
        fish)
            echo "   echo 'set -Ux fish_user_paths $INSTALL_DIR \$fish_user_paths' >> ~/.config/fish/config.fish"
            echo "   source ~/.config/fish/config.fish"
            ;;
        *)
            echo "   export PATH=\"\$PATH:$INSTALL_DIR\""
            echo ""
            echo "   Then add the above line to your shell profile (~/.bashrc, ~/.zshrc, etc.)"
            ;;
    esac
    echo ""
fi

echo "🚀 Installing to $INSTALL_DIR..."
if [ -w "$INSTALL_DIR" ]; then
    mv "$BIN_PATH" "$INSTALL_DIR/$BINARY_NAME"
    chmod +x "$INSTALL_DIR/$BINARY_NAME"
else
    echo "🔒 Requires sudo privileges to install to $INSTALL_DIR"
    sudo mv "$BIN_PATH" "$INSTALL_DIR/$BINARY_NAME"
    sudo chmod +x "$INSTALL_DIR/$BINARY_NAME"
fi

# Cleanup
cd - > /dev/null
rm -rf "$TMP_DIR"

echo "✅ Successfully installed $BINARY_NAME to $INSTALL_DIR/$BINARY_NAME"
echo "✨ Run '$BINARY_NAME --help' to get started!"
