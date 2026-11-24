# Swift Note (`swiftnote-ink`)

High-performance cross-platform desktop handwriting and study notebook written in Rust for Windows (DX12) and Linux (Wayland/X11 + Vulkan/GL).

## Design Goals & Highlights

- **Zero-Shift Wet/Dry Ink Parity:** Both active stroke preview ("wet ink") and committed cached page tiles ("dry ink") execute the exact same CPU `StrokeTessellator` (`ContourMesh`) and `wgpu` 4x MSAA `stroke.wgsl` shader pipeline — eliminating stroke shifts when lifting the pen.
- **Digitizer Pen Isolation (`sdl3` `CategoryPen`):** Rejects palm/touch input and SDL virtual mouse synthesis while the stylus is in proximity, prevents out-of-range coordinate spikes, preserves sharp math cusps in the velocity-adaptive stabilizer, and clamps each stroke cleanly to the originating page.
- **Lossless Clipboard & Asset Insertion:** Pastes clipboard bitmaps at native pixel resolution in lossless PNG storage, normalizes Windows `CF_DIB` all-zero alpha frames without altering genuine transparent PNGs, and renders with GPU mipmaps.
- **Lazy Multi-Page PDF Engine:** Imports encrypted or standard multi-page PDFs through a generation-tagged background `PdfWorker` (`pdfium-render`), tiling visible pages into LRU-budgeted 2048 px segments so large PDFs load without UI freezes.
- **Solid-Matte Minimalist Chrome:** Single-bar writing workspace paired with opaque matte Light/Dark surfaces, crisp `1 px` hairline borders, monochrome vector SVG icons (`resvg`), and zero backdrop blur or translucency.
- **Independent Blackboard Contrast Toggle:** Light Paper vs. Blackboard page polarity is independent of the UI theme, with an optional **Auto-invert B/W ink on Blackboard** toggle in Paper & Preferences (`true` by default).

## Workspace Architecture

| Crate | Purpose |
| --- | --- |
| `crates/swiftnote-core` | Pure-Rust (`#![forbid(unsafe_code)]`) data model, R-tree spatial index, cusp-preserving stroke stabilizer, stroke/segment eraser, undo/redo history, `.swnote` ZIP archive storage, and local note library |
| `crates/swiftnote-gpu` | Unified `wgpu` DX12/Vulkan/GL renderer, 512x512 GPU LRU tile cache, procedural paper ruling shaders, WGSL 4x MSAA ink shaders, GPU mipmaps, and lazy `PdfWorker` |
| `crates/swiftnote-app` | Event-driven SDL3 windowing + `CategoryPen` isolation, OS clipboard integration, `AppController`, solid-matte `egui` chrome, and vector icon registry |

## Building from Source

### Prerequisites

- Rust toolchain **1.95+** with `rustfmt` and `clippy`.
- **Windows:** MSVC Build Tools (Visual Studio 2022) and CMake (required when building bundled SDL3 from source).
- **Linux (Ubuntu 24.04 / Debian):**

```bash
sudo apt-get update && sudo apt-get install -y --no-install-recommends \
  cmake ninja-build pkg-config \
  libwayland-dev libx11-dev libxext-dev libxrandr-dev \
  libxfixes-dev libxi-dev libxcursor-dev libxkbcommon-dev \
  libgl1-mesa-dev libdbus-1-dev libgtk-3-dev
```

### Build, Lint, and Test

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release --bin swiftnote
```

### Optional: Enable PDF Import via PDFium

PDF import uses dynamic loading (`pdfium.dll` on Windows, `libpdfium.so` on Linux) searched alongside the executable, in the current working directory, or in system library paths. Fetch the official `bblanchon/pdfium-binaries` shared library with the helper script for your OS:

**Windows (PowerShell):**

```powershell
.\scripts\fetch-pdfium.ps1
```

**Linux (Bash):**

```bash
./scripts/fetch-pdfium.sh
```

## Keyboard & Canvas Shortcuts

| Shortcut | Action |
| --- | --- |
| `1` – `5` | Select pen slot 1–5 (click active slot button for brush/width/color settings) |
| `E` | Select eraser (click again or use hardware eraser / barrel button) |
| `V` | Select & resize placed images |
| `Del` / `Backspace` | Delete selected image |
| `Ctrl+Z` / `Ctrl+Shift+Z` / `Ctrl+Y` | Undo / Redo |
| `Ctrl+V` | Paste clipboard image (or text when editing titles/inputs) |
| `Ctrl+N` / `Ctrl+O` / `Ctrl+S` | New note / Open `.swnote` / Export `.swnote` copy |
| `G` / `Tab` | Toggle Notes Gallery / Pages Sidebar |
| `Ctrl++` / `Ctrl+-` / `Ctrl+0` | Zoom in / Zoom out / Reset zoom to 100% |
| `Space + Drag` / Middle-Mouse / Two-Finger | Pan & pinch-zoom canvas |

## `.swnote` File Format

`.swnote` files are standard ZIP archives storing `document.json` (pages, pressure/tilt strokes, procedural backgrounds, image placements, PDF page bindings), deduplicated BLAKE3-addressed PNG images under `assets/images/`, and embedded PDFs under `assets/pdfs/`. Writes use atomic temp-file replacement so autosave never corrupts existing notebooks.

## License

Licensed under **GPL-3.0-or-later**.
