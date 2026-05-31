#!/usr/bin/env bash
set -euo pipefail

REPO="${MMUX_REPO:-ilijaljubicic/mmux}"
VERSION="${VERSION:-latest}"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH_RAW="$(uname -m)"

case "$ARCH_RAW" in
  x86_64|amd64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="arm64" ;;
  *) echo "Unsupported architecture: $ARCH_RAW" >&2; exit 1 ;;
esac

case "$OS" in
  linux|darwin)
    EXT="tar.gz"
    PLATFORM="${OS}-${ARCH}"
    ;;
  *)
    echo "Unsupported OS: $OS" >&2
    exit 1
    ;;
esac

ARCHIVE="mmux-${PLATFORM}.${EXT}"

if [ "$VERSION" = "latest" ]; then
  VERSION="$(
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
      | sed -nE 's/.*"tag_name": *"([^"]+)".*/\1/p'
  )"
  if [ -z "$VERSION" ]; then
    echo "Could not resolve latest mmux release for ${REPO}" >&2
    exit 1
  fi
fi

URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo "Downloading ${ARCHIVE} (${VERSION}) from ${URL}"
curl -fsSL "$URL" -o "$TMPDIR/$ARCHIVE"
tar -xzf "$TMPDIR/$ARCHIVE" -C "$TMPDIR"

EXTRACTED="$(find "$TMPDIR" -maxdepth 1 -type f -name 'mmux*' ! -name "$ARCHIVE" | head -n 1)"
if [ -z "$EXTRACTED" ]; then
  echo "Archive did not contain an mmux binary" >&2
  exit 1
fi
chmod +x "$EXTRACTED"

if [ -z "${INSTALL_DIR:-}" ]; then
  if [ "$OS" = "darwin" ] && [ -d "/opt/homebrew/bin" ]; then
    INSTALL_DIR="/opt/homebrew/bin"
  else
    INSTALL_DIR="/usr/local/bin"
  fi
fi

mkdir -p "$INSTALL_DIR" 2>/dev/null || sudo mkdir -p "$INSTALL_DIR"
if [ -w "$INSTALL_DIR" ]; then
  mv "$EXTRACTED" "$INSTALL_DIR/mmux"
else
  sudo mv "$EXTRACTED" "$INSTALL_DIR/mmux"
fi

echo "Installed mmux ${VERSION} to ${INSTALL_DIR}/mmux"
