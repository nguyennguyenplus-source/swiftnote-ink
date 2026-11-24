#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-latest}"
ARCH="${2:-auto}"

if [ "$ARCH" = "auto" ]; then
    case "$(uname -m)" in
        x86_64|amd64) ARCH="x64" ;;
        aarch64|arm64) ARCH="arm64" ;;
        *)
            echo "Unsupported architecture: $(uname -m). Pass 'x64' or 'arm64' explicitly." >&2
            exit 1
            ;;
    esac
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ARCHIVE_NAME="pdfium-linux-$ARCH.tgz"

if [ "$VERSION" = "latest" ]; then
    DOWNLOAD_URL="https://github.com/bblanchon/pdfium-binaries/releases/latest/download/$ARCHIVE_NAME"
else
    DOWNLOAD_URL="https://github.com/bblanchon/pdfium-binaries/releases/download/$VERSION/$ARCHIVE_NAME"
fi

TMP_DIR="$(mktemp -d)"
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

ARCHIVE_PATH="$TMP_DIR/$ARCHIVE_NAME"
EXTRACT_DIR="$TMP_DIR/extract"
mkdir -p "$EXTRACT_DIR"

echo "Downloading $DOWNLOAD_URL ..."
if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$DOWNLOAD_URL" -o "$ARCHIVE_PATH"
elif command -v wget >/dev/null 2>&1; then
    wget -qO "$ARCHIVE_PATH" "$DOWNLOAD_URL"
else
    echo "Neither curl nor wget is installed." >&2
    exit 1
fi

echo "Extracting $ARCHIVE_NAME ..."
tar -xzf "$ARCHIVE_PATH" -C "$EXTRACT_DIR"

LIB_FILE="$(find "$EXTRACT_DIR" -name 'libpdfium.so*' -type f | head -n 1)"
if [ -z "$LIB_FILE" ]; then
    echo "libpdfium.so not found inside extracted archive." >&2
    exit 1
fi

cp -f "$LIB_FILE" "$REPO_ROOT/libpdfium.so"
chmod 755 "$REPO_ROOT/libpdfium.so"
echo "Installed $REPO_ROOT/libpdfium.so"

for profile in debug release; do
    if [ -d "$REPO_ROOT/target/$profile" ]; then
        cp -f "$LIB_FILE" "$REPO_ROOT/target/$profile/libpdfium.so"
        chmod 755 "$REPO_ROOT/target/$profile/libpdfium.so"
        echo "Copied $REPO_ROOT/target/$profile/libpdfium.so"
    fi
done
