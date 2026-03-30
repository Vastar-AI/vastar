#!/bin/sh
# Vastar installer — download pre-built binary for your platform.
# Usage: curl -sSf https://raw.githubusercontent.com/Vastar-AI/vastar/main/install.sh | sh
set -e

REPO="Vastar-AI/vastar"
INSTALL_DIR="${VASTAR_INSTALL_DIR:-$HOME/.local/bin}"

# Detect OS
OS="$(uname -s)"
case "$OS" in
    Linux)  OS_TAG="unknown-linux-gnu" ;;
    Darwin) OS_TAG="apple-darwin" ;;
    *)      echo "Error: unsupported OS: $OS"; exit 1 ;;
esac

# Detect architecture
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64|amd64)  ARCH_TAG="x86_64" ;;
    aarch64|arm64) ARCH_TAG="aarch64" ;;
    *)             echo "Error: unsupported architecture: $ARCH"; exit 1 ;;
esac

TARGET="${ARCH_TAG}-${OS_TAG}"

# Get latest release tag
echo "Detecting latest vastar release..."
TAG=$(curl -sI "https://github.com/$REPO/releases/latest" | grep -i "^location:" | sed 's|.*/||' | tr -d '\r\n')

if [ -z "$TAG" ]; then
    echo "Error: could not detect latest release. Check https://github.com/$REPO/releases"
    exit 1
fi

FILENAME="vastar-${TAG}-${TARGET}.tar.gz"
URL="https://github.com/$REPO/releases/download/${TAG}/${FILENAME}"

echo "  Version:  $TAG"
echo "  Platform: $TARGET"
echo "  URL:      $URL"
echo ""

# Download
TMPDIR=$(mktemp -d)
echo "Downloading..."
curl -sL "$URL" -o "$TMPDIR/$FILENAME"

if [ ! -s "$TMPDIR/$FILENAME" ]; then
    echo "Error: download failed or empty file."
    echo "Check: $URL"
    rm -rf "$TMPDIR"
    exit 1
fi

# Extract
cd "$TMPDIR"
tar xzf "$FILENAME"

if [ ! -f vastar ]; then
    echo "Error: vastar binary not found in archive."
    rm -rf "$TMPDIR"
    exit 1
fi

# Install
mkdir -p "$INSTALL_DIR"
mv vastar "$INSTALL_DIR/vastar"
chmod +x "$INSTALL_DIR/vastar"
rm -rf "$TMPDIR"

echo ""
echo "Installed: $INSTALL_DIR/vastar ($TAG)"
echo ""

# Check PATH
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        echo "Note: $INSTALL_DIR is not in your PATH."
        echo "Add this to your shell profile:"
        echo ""
        echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
        echo ""
        ;;
esac

echo "Run: vastar --version"
"$INSTALL_DIR/vastar" --version 2>/dev/null || true
