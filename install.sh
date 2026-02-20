#!/bin/sh
# Install script for virgil-cli (macOS / Linux)
# Usage: curl -fsSL https://raw.githubusercontent.com/Delanyo32/virgil-cli/master/install.sh | sh
set -eu

REPO="Delanyo32/virgil-cli"
BINARY="virgil-cli"
INSTALL_DIR="${INSTALL_DIR:-${HOME}/.local/bin}"

usage() {
    cat <<EOF
Install virgil-cli

Usage: install.sh [OPTIONS]

Options:
  -b DIR      Install directory (default: ~/.local/bin)
  -v VERSION  Install a specific version (e.g. v0.1.0)
  -h          Show this help
EOF
}

# Parse flags
VERSION=""
while [ $# -gt 0 ]; do
    case "$1" in
        -b) INSTALL_DIR="$2"; shift 2 ;;
        -v) VERSION="$2"; shift 2 ;;
        -h) usage; exit 0 ;;
        *)  echo "Unknown option: $1"; usage; exit 1 ;;
    esac
done

# Detect OS
OS="$(uname -s)"
case "$OS" in
    Linux)  OS_TARGET="unknown-linux-gnu" ;;
    Darwin) OS_TARGET="apple-darwin" ;;
    *)      echo "Error: unsupported OS: $OS"; exit 1 ;;
esac

# Detect architecture
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64|amd64)  ARCH_TARGET="x86_64" ;;
    aarch64|arm64) ARCH_TARGET="aarch64" ;;
    *)             echo "Error: unsupported architecture: $ARCH"; exit 1 ;;
esac

TARGET="${ARCH_TARGET}-${OS_TARGET}"

# Resolve version
if [ -z "$VERSION" ]; then
    echo "Fetching latest release..."
    if command -v curl >/dev/null 2>&1; then
        VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"tag_name":\s*"([^"]+)".*/\1/')"
    elif command -v wget >/dev/null 2>&1; then
        VERSION="$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"tag_name":\s*"([^"]+)".*/\1/')"
    else
        echo "Error: curl or wget is required"; exit 1
    fi
    if [ -z "$VERSION" ]; then
        echo "Error: could not determine latest version"; exit 1
    fi
fi

ARCHIVE="${BINARY}-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"

echo "Installing ${BINARY} ${VERSION} for ${TARGET}..."

# Download
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$URL" -o "${TMPDIR}/${ARCHIVE}"
elif command -v wget >/dev/null 2>&1; then
    wget -qO "${TMPDIR}/${ARCHIVE}" "$URL"
fi

# Extract
tar xzf "${TMPDIR}/${ARCHIVE}" -C "$TMPDIR"

# Install
mkdir -p "$INSTALL_DIR"
mv "${TMPDIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
chmod +x "${INSTALL_DIR}/${BINARY}"

echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"

# PATH check
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        echo ""
        echo "Add ${INSTALL_DIR} to your PATH:"
        echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
        echo ""
        echo "To make it permanent, add the line above to your ~/.bashrc, ~/.zshrc, or equivalent."
        ;;
esac

# Verify
if "${INSTALL_DIR}/${BINARY}" --version >/dev/null 2>&1; then
    echo "Verification: $("${INSTALL_DIR}/${BINARY}" --version)"
else
    echo "Warning: could not verify installation"
fi
