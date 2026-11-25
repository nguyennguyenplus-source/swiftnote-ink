#!/usr/bin/env bash
set -euo pipefail

# Swift Note Windows Portable Packager (MSYS2 UCRT64)
# Usage: ./windows-setup/package-swiftnote-portable.sh <build-dir> <output-dist-dir>

BUILD_DIR="$(cd "${1:-build-ucrt64}" && pwd)"
DIST_DIR="${2:-$BUILD_DIR/dist-swiftnote}"
PREFIX="${MSYSTEM_PREFIX:-/ucrt64}"

echo "=== Packaging Swift Note Portable ==="
echo "Build dir : $BUILD_DIR"
echo "Output dir: $DIST_DIR"
echo "UCRT64    : $PREFIX"

rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR/bin" "$DIST_DIR/lib" "$DIST_DIR/share"

echo "[1/6] Running CMake install..."
(cd "$BUILD_DIR" && cmake --install . --prefix "$DIST_DIR" >/dev/null)

echo "[2/6] Copying Swift Note executable & runtime DLLs..."
cp "$DIST_DIR/bin/xournalpp.exe" "$DIST_DIR/bin/SwiftNote.exe"
ldd "$BUILD_DIR/xournalpp.exe" | grep -o "${PREFIX}/bin/[^ ]*\.dll" | sort -u | while IFS= read -r dll; do
    cp -u "$dll" "$DIST_DIR/bin/"
done
cp -u "$PREFIX"/bin/libqpdf*.dll "$DIST_DIR/bin/" 2>/dev/null || true
cp -u "$PREFIX"/bin/gspawn-win64-helper*.exe "$DIST_DIR/bin/" 2>/dev/null || true
cp -u "$PREFIX"/bin/gdbus.exe "$DIST_DIR/bin/" 2>/dev/null || true

echo "[3/6] Copying GDK-Pixbuf loaders & transitive DLLs (PNG/SVG/JPEG)..."
cp -r "$PREFIX/lib/gdk-pixbuf-2.0" "$DIST_DIR/lib/"
declare -A copied_deps
mapfile -d '' pixbuf_libs < <(find "$PREFIX/lib/gdk-pixbuf-2.0" -type f -name "*.dll" -print0)
for ((i = 0; i < ${#pixbuf_libs[@]}; ++i)); do
    while IFS= read -r dep; do
        if [[ -n ${copied_deps[$dep]+x} ]]; then
            continue
        fi
        copied_deps[$dep]=1
        if [[ -f "$PREFIX/bin/$dep" ]]; then
            cp -u "$PREFIX/bin/$dep" "$DIST_DIR/bin/"
            pixbuf_libs+=("$PREFIX/bin/$dep")
        fi
    done < <(strings -a "${pixbuf_libs[$i]}" | grep -Ei '^[[:alnum:]_+.-]+\.dll$' | sort -u)
done

echo "[4/6] Copying GTK3 schemas, icons, poppler-data, fonts config, and GtkSourceView..."
cp -r "$PREFIX/share/icons" "$DIST_DIR/share/"
cp -r "$PREFIX/share/glib-2.0" "$DIST_DIR/share/"
cp -r "$PREFIX/share/poppler" "$DIST_DIR/share/"
cp -r "$PREFIX/share/gtksourceview-4" "$DIST_DIR/share/" 2>/dev/null || true
mkdir -p "$DIST_DIR/etc"
cp -r "$PREFIX/etc/fonts" "$DIST_DIR/etc/" 2>/dev/null || true

echo "[5/6] Ensuring Swift note stylesheet, Plus Jakarta Sans font, and A2 logo assets are present..."
cp -f "$BUILD_DIR/../ui/swiftnote.css" "$DIST_DIR/share/xournalpp/ui/swiftnote.css"
mkdir -p "$DIST_DIR/share/xournalpp/ui/fonts"
cp -f "$BUILD_DIR/../ui/fonts/"*.ttf "$DIST_DIR/share/xournalpp/ui/fonts/" 2>/dev/null || true
cp -f "$BUILD_DIR/../ui/pixmaps/swiftnote-logo-"*.svg "$DIST_DIR/share/xournalpp/ui/pixmaps/" 2>/dev/null || true
cp -f "$BUILD_DIR/../ui/pixmaps/swiftnote-logo-"*.png "$DIST_DIR/share/xournalpp/ui/pixmaps/" 2>/dev/null || true
cp -f "$BUILD_DIR/../ui/pixmaps/com.github.xournalpp.xournalpp.svg" "$DIST_DIR/share/xournalpp/ui/pixmaps/com.github.xournalpp.xournalpp.svg" 2>/dev/null || true

echo "[6/6] Writing top-level launcher..."
cat > "$DIST_DIR/SwiftNote.cmd" << 'EOF'
@echo off
start "" "%~dp0bin\SwiftNote.exe" %*
EOF

echo "=== Done! Portable Swift Note ready at: $DIST_DIR ==="
