# Swift Note (`swiftnote-ink`) v2 — Architecture & Product Design Specification

- **Date:** 2026-11-24
- **Status:** Approved (2026-11-24)
- **Repository:** `https://github.com/nguyennguyenplus-source/swiftnote-ink`
- **License:** `GPL-3.0-or-later` *(Design Call — preserves Xournal++-compatible open-source licensing)*
- **Target Platforms:** Linux (Wayland `zwp_tablet_v2` & X11 XInput2) and Windows 10/11 (Windows Ink)

---

## 1. Intent & Context ("The Why")

Swift Note is a cross-platform, GPU-accelerated handwriting, note-taking, and math/problem-solving desktop application built in **Rust**.

The user currently splits daily study and problem-solving work across two Windows applications before migrating to Linux:
- **Scrble Ink (`v6.8.6.0`):** Preferred for its calm, uncluttered canvas layout, intuitive pen-first ergonomics, and focused toolset, but suffers from wet-to-dry ink shift, blurry/broken clipboard image pasting, multi-page PDF instability, and Windows/UWP lock-in.
- **Xournal++ (`v1.3.8`):** Admired for its rock-solid stylus input isolation, crisp math-friendly stroke stabilization, zero-shift wet-to-dry blitting, and reliability — but its traditional GTK menu/multi-toolbar chrome feels cluttered and visually dated.
- **Prior Remake (`D:\Apps\Scrble Ink Remake`, `v0.1.0–v0.2.5`):** Built on GTK4 + Libadwaita + dual `gsk`/`tiny-skia` renderers. Failed due to:
  1. Inability to run/debug natively on Windows before Linux migration.
  2. GTK4 Wayland/X11 stylus bugs (dropped short strokes, axis index corruption, stray mouse interference, hover CPU spikes).
  3. Dual-renderer mismatch causing blurry or shifting strokes on pen lift and `TileCache` race conditions.
  4. `RefCell` callback re-entrancy panics and rigid GNOME Libadwaita aesthetics.

Swift Note v2 starts from a clean slate to combine **Xournal++-grade input fidelity and rendering precision** with **Scrble Ink's complete feature set and distraction-free canvas**, delivered in an **opaque, matte-precision Apple Pro-inspired aesthetic** with uncompromising performance.

---

## 2. Goals, Non-Goals & Anti-Goals

### 2.1 Goals
1. **Zero-Shift, Zero-Blur Pen Lift:** `0.0 px` geometric, width, and opacity delta between active wet ink and committed dry ink.
2. **Lossless Clipboard & Image Pipeline:** Instant `Ctrl+V` paste from OS clipboard (Windows & Linux Wayland/X11) preserving native resolution, alpha transparency, and crisp GPU mipmapped filtering at every zoom level.
3. **Instant, Crash-Proof Multi-Page PDF Engine:** Open 100–500+ page PDFs (including encrypted PDFs) in `< 50 ms` metadata time via lazy streaming, bounded VRAM LRU caching, and zero UI-thread blocking.
4. **Full Scrble Ink `v6.8.6.0` Feature Parity:** Deliver 100% of Scrble Ink's functional toolkit across 3 structured milestones without breaking simplicity on the writing canvas.
5. **Dual-OS Native Execution (`Windows` + `Linux`):** Compile, run, profile, and test directly on Windows today while targeting Linux (Wayland `zwp_tablet_v2` + X11 XInput2) as a first-class native platform, including full Vietnamese IME composition.
6. **Matte-Precision Minimalism & High Frame Pacing:** Event-driven 60–144+ FPS active drawing and scrolling, `< 1%` CPU / `0%` GPU at idle, and calm solid-matte surfaces.

### 2.2 Non-Goals (Out of Scope for v1.0)
- Cloud sync, user accounts, or proprietary SaaS subscriptions.
- Real-time multiplayer collaboration.
- Cloud AI OCR / cloud handwriting-to-LaTeX services.
- Lua/Python plugin scripting engines or external marketplace ecosystems.

### 2.3 Anti-Goals (Hard Rejections)
- **ZERO Glassmorphism / Frosted Glass / Liquid Glass:** Strictly forbidden across the entire UI. No backdrop blur shaders, no translucent frosted sidebars, no fake liquid glass highlights. Every chrome surface MUST be solid opaque matte with crisp `1 px` hairline borders and zero wasted blur GPU passes.
- **ZERO Dual Wet/Dry Renderer Mismatch:** Never render active strokes with one tessellator/rasterizer and dry strokes with another. Never silently re-fit, simplify, or distort a user's stroke on pen lift.
- **ZERO Lossy Clipboard Rasterization:** Never downsample or bake pasted images into fixed-DPI background page bitmaps.
- **ZERO Eager Full-PDF Loading:** Never decode every PDF page into RAM/VRAM on import.
- **ZERO GTK4/Libadwaita or WebView/Electron/Tauri.**

---

## 3. Constraints & Engineering Standards

- **Language & Toolchain:** Stable Rust (Edition `2021`, `rust-version = "1.88"` compatible with host `rustc 1.100.0`).
- **Safety & Lints:**
  - Workspace Clippy denies: `unwrap_used = "deny"`, `expect_used = "deny"`, `dbg_macro = "deny"`, `unsafe_code = "warn"`.
  - Every `unsafe` block MUST carry a `// SAFETY:` comment proving all invariants.
  - Typed errors via `thiserror` in library crates; contextual propagation via `anyhow` at the app shell boundary.
- **Hot-Path Allocation Discipline:** Zero heap allocations (`Vec` growth, `String`, per-point cloning) inside stylus motion and per-frame render hot paths. Use pre-allocated reusable scratch buffers and `rstar` R-Tree spatial indexing for erasing, hit-testing, and lasso selection.
- **Strict Crate Separation:**
  - `swiftnote-core`: 100% OS- and GPU-independent document model, coordinate spaces, stabilizer, contour generator, spatial index, undo/redo, shape recognition, `.swnote` serialization.
  - `swiftnote-gpu`: `wgpu` unified stroke/page/image/PDF renderer, tile cache, offscreen wet-ink mask layer, `pdfium-render` worker, vector PDF & PNG exporters.
  - `swiftnote-app`: `SDL3` window/stylus/IME event loop, OS clipboard integration, solid-matte custom-skinned `egui` chrome, gallery & file dialogs.
- **CI & Verification:** Dual-OS GitHub Actions CI (`windows-latest` and `ubuntu-24.04`) running `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, headless `cargo test --workspace`, and release builds.

---

## 4. Approaches Considered & Selection Rationale

### 4.1 Eliminated Upfront
- **GTK4 + Libadwaita:** Directly responsible for the prior remake's failure — zero local Windows testability, GDK4 Wayland/XInput2 dropped stroke starts and corrupted axis indices, and rigid GNOME styling.
- **Tauri / WebKitGTK / Electron:** High memory overhead and well-documented Linux Wayland WebKitGTK regressions dropping stylus `PointerEvent` pressure to generic mouse input.

### 4.2 Evaluated Rust Architectures
| Option | Stack | Trade-offs | Verdict |
| :--- | :--- | :--- | :--- |
| **Approach 1 (Selected)** | `SDL3` (`CategoryPen` + IME + Window) + Unified `wgpu` GPU Engine + Custom Solid-Matte `egui` Chrome + `pdfium-render` | First-class native stylus on Wayland (`zwp_tablet_v2`), X11 (XInput2), and Windows Ink; zero-shift 100% shared GPU ink pipeline; requires bundling `SDL3` and `pdfium` shared libraries alongside binary. | **Selected** — eliminates all Wayland/Windows stylus fragmentation and gives full pixel control over both canvas and matte UI. |
| **Approach 2** | `iced` (Elm/`wgpu`) + Custom `wgpu` Canvas + `octotablet` Wayland bridge + `pdfium-render` | Pure Rust windowing/UI, but bridging `winit` coordinates, fractional scaling, and IME with `octotablet` on Wayland carries high desync risk. | Rejected |
| **Approach 3** | `SDL3` + `skia-safe` + `pdfium-render` | Mature Skia 2D & PDF backend, but imposes 40–50 MB C++ binary bloat and fragile C++/LLVM build requirements on Windows (`clang` not installed). | Rejected |

---

## 5. Detailed Technical Architecture

### 5.1 Crate Responsibilities & Unidirectional State Flow
To make re-entrant borrow panics (`RefCell`) structurally impossible:
- `swiftnote-app` owns plain Rust structs directly inside the single-threaded main event loop (`AppController`), eliminating `Rc<RefCell<...>>` across callbacks.
- UI widgets emit typed `UiAction` enum messages into a frame-local `Vec<UiAction>` queue during UI layout.
- After the UI pass completes, `AppController::apply(actions)` mutates `DocumentState` / `ToolState` / `Viewport` sequentially, appends undo entries, collects `DirtyRegion`s, and issues explicit GPU commands to `swiftnote-gpu`.

```
SDL3 Events (Pen/Mouse/Touch/Key/IME) ──▶ InputPipeline (Xournal++ state machine)
                                                │
                                                ▼
UI Chrome (Matte egui, emits UiAction) ──▶ AppController (sequential mutation)
                                                │
           ┌────────────────────────────────────┴──────────────────────────────┐
           ▼                                                                   ▼
   swiftnote-core                                                        swiftnote-gpu
 (StrokeContour, Stabilizer,                                       (Unified wgpu Pipeline:
  R-Tree Index, Document, .swnote)                                  Page Tile Cache + Active Mask +
                                                                    Image Mipmaps + PDFium Worker)
```

### 5.2 Coordinate Spaces (`swiftnote-core`)
All coordinates are strongly typed newtypes to prevent space-mixing bugs:
- `ScreenPoint` (`f32` / `f64` logical window pixels) and `PhysicalPoint` (`ScreenPoint * display_scale`).
- `CanvasPoint` (infinite vertical canvas space in typographic points, `72 pt = 1 inch`, pages arranged vertically with uniform gap).
- `PagePoint` (page-local origin `(0, 0)` to `(width_pt, height_pt)`, matching PDF `1/72"` point coordinates exactly — e.g. A4 = `595.28 × 841.89 pt`, US Letter = `612.0 × 792.0 pt`).
- `ViewportTransform` supports anchor-preserving zoom (`10%` to `800%`) and pixel-snapped page positioning.

### 5.3 Input Pipeline (Xournal++ Discipline over `SDL3`)
Powered by SDL3's native `CategoryPen` + `SDL_GetWindowSurface` / `raw-window-handle` integration:
- **Backend Mapping:** Windows Ink (`Windows`), `zwp_tablet_v2` (`Linux Wayland`), `XInput2` (`Linux X11`), with full `SDL_TEXTINPUT` / `SDL_TEXTEDITING` IME support for Vietnamese typing inside Text Boxes and Rename/Search fields.
- **Device Isolation State Machine:**
  - `InputMode::DigitizerPen`: Stylus draws/erases with pressure & tilt; active stylus proximity/hover activates strict **Palm Rejection** ignoring touch; single-finger touch pans/scrolls when stylus is away; 2-finger pinch zooms & pans (`InputMode::DigitizerPenNoPressure` ignores pressure axis and uses fixed stroke width; `InputMode::FingerCapacitivePen` allows 1-finger drawing alongside 2-finger zoom/pan).
  - **Barrel Buttons & Hardware Eraser:** `SDL_PEN_INPUT_ERASER_TIP` overrides active tool to Eraser while inverted and restores previous tool on lift; barrel buttons map cleanly to temporary Eraser or Lasso/Pan.
  - **Mouse Ergonomics:** Left-click draws (when enabled) or interacts with adorners/UI; Right-click drag or Middle-click drag or `Space + Left-drag` pans smoothly anywhere; `Ctrl + Wheel` (or plain `Wheel` scroll) zooms/scrolls without mode switching.
- **Hardware Noise Guards (Derived from Xournal++ `StylusInputHandler.cpp` & `StrokeHandler.cpp`):**
  1. Ignore `PenMotion` events with `pressure == 0.0` while tip is flagged down (common driver lift artifact).
  2. Ignore impossible coordinate teleports (`> 100 logical px` spike on proximity enter/leave).
  3. Cleanly terminate active strokes on window focus-loss (`SDL_EVENT_WINDOW_FOCUS_LOST`) or device loss — never leaving stuck strokes or stuck `Space`/`Shift` modifiers.
  4. Apply Xournal++-style **First-Point Pressure Thickening**: if point #1 begins with faint contact pressure and point #2 ramps sharply before exceeding the motion threshold, update point #1's width smoothly.
  5. Distance threshold filtering (`PIXEL_MOTION_THRESHOLD` scaled cleanly in screen pixels) + sub-segment width decomposition (`MAX_WIDTH_VARIATION`) so sudden pressure spikes never produce boxy steps.

### 5.4 Mathematical Stroke Engine (`swiftnote-core`)
1. **Dual-Stage Stroke Stabilizer (Xournal++ `StrokeStabilizer.cpp`):**
   - **Stage 1 — Deadzone Preprocessor with Cusp Detection:** Filters micro-jitter inside a small radius while tracking entry-to-exit direction vectors; when directional reversal / angle exceeds `~90°` (e.g. writing `'2'`, `'x'`, `'π'`, `'√'`), immediately flushes the cusp vertex untouched so sharp math corners stay razor-crisp.
   - **Stage 2 — Velocity-Integral Gaussian Averaging:** Weights recent buffered points by `exp(-(sum_v)^2 / (2 * sigma^2))` with 4 presets: `Off`, `Light` (default), `Medium`, `Strong`.
   - **Pen-Lift Finalization:** Flushes residual stabilizer lag with a quadratic spline tail toward the exact final physical pen coordinate so fast strokes never stop short.
2. **Analytical Variable-Width `StrokeContour` (`StrokeContour.cpp` Geometry):**
   - Computes exact left/right offset envelopes around each polyline vertex using tangent circle-to-circle coupling angles (`asin((r1 - r2) / d)`) and analytical semicircular end caps (`start_arc` and `end_arc`) as **one closed, consistently wound polygon**, triangulated into a clean indexed triangle strip/list.
   - **Guarantees:** Zero inner winding holes, zero overlapping endpoint circles (eliminating dark double-alpha blobs on translucent/highlighter strokes), and `1:1` identical geometry for wet and dry rendering.
3. **Brush Catalog (Full Scrble Ink Parity):**
   - `Pen`: Solid ink with crisp variable pressure width (`0.3x`–`1.3x` curve or custom pressure toggle).
   - `Pencil`: Soft organic graphite edge + screen-aligned procedural graphite grain texture modulated in the fragment shader + optional tilt shading (`Xtilt`/`Ytilt`).
   - `Calligraphy`: Fixed-angle elliptical nib (`nib_angle`, default `45°`) modulated by pressure.
   - `HighlighterRound` & `HighlighterSquare`: Fixed translucency (`alpha ≈ 0.45`), square or round end caps, rendered via single-stroke coverage buffer + `Multiply` blend on light paper (`SrcOver` translucent blend on Blackboard) so crossing over an existing part of the *same* stroke never darkens it.

### 5.5 Zero-Shift Unified GPU Rendering Pipeline (`swiftnote-gpu`)
This architecture enforces the core invariant: **wet ink and dry ink never diverge.**

1. **Single Mesh & Shader Pipeline:**
   - Both active wet strokes and committed dry strokes run through the **exact same `StrokeContour` CPU tessellator → `wgpu` vertex/index buffer → `wgpu` MSAA 4x render pipeline** in `PagePoint` coordinates projected by the exact camera matrix.
2. **Active Stroke Layer (Wet Ink):**
   - When `PenDown` starts a stroke, `swiftnote-gpu` allocates/reuses a scratch GPU mesh and renders into a viewport-sized `ActiveStrokeMask` (`Rgba8Unorm` / `R8Unorm` 4x MSAA resolved texture aligned 1:1 to physical swapchain pixels).
   - Each `PenMotion` sample tessellates only the dirty tail segments `(N-2 .. N)` in `O(1)` time (`< 50 µs`) and appends vertices to the active GPU buffer.
   - **Highlighter & Pencil Self-Overlap Safety:** Rendering the active stroke with `MAX` coverage / `Stencil` single-pass rule into the `ActiveStrokeMask`, then compositing `ActiveStrokeMask` once over the canvas guarantees zero dark double-alpha seams while drawing.
3. **Instant Zero-Shift Commit (Pen Up):**
   - On `PenUp`, `swiftnote-core` finalizes ONLY the unrendered tail points (without refitting or altering already-drawn vertices), appends the final triangles to the exact same stroke mesh, inserts the immutable `Stroke` into `Page` + `R-Tree`, and immediately schedules the tile-cache delta blit on the **same GPU command encoder** (or renders the stroke directly into affected `512×512` page tiles in `< 0.3 ms` on GPU before retiring the `ActiveStrokeOverlay` once all touched tiles confirm ready).
   - **Result:** `0.0 px` positional shift, `0.0` blur change, `0.0` opacity flicker on pen lift.
4. **Physical-Pixel Page Tile Cache:**
   - Page tiles are `512 × 512` **physical pixels** rendered on the GPU (`wgpu` render-to-texture with 4x MSAA) at exact `zoom * display_scale` for settled views — eliminating the `floor/ceil` 1px stretch seams and blurry `2^(1/4)` supersampling mismatches of the prior remake.
   - During active continuous zoom (`GestureZoom` / `Ctrl+Wheel`), existing tiles scale smoothly on GPU; after `80 ms` zoom settle (or `> 20%` zoom delta), visible tiles re-render cleanly to sharp physical pixel resolution via LRU budget (`256 MiB` default VRAM cap).
   - Erase / Undo / Redo invalidates ONLY the bounding-box tiles intersecting the modified `Rect`, re-rendering those tiles directly on GPU in `< 1 ms`.
5. **Frame Pacing & Power Efficiency:**
   - Event-driven SDL3 loop (`SDL_WaitEvent` when idle, waking immediately on input, animation, laser trail decay, or background PDF completion) → `0%` GPU/CPU at rest.

### 5.6 Lossless Clipboard & Image Pipeline
Solves Scrble Ink's blurry/broken paste bug permanently:
1. **Cross-Platform Clipboard Ingestion (`swiftnote-app`):**
   - Primary clipboard via `arboard` + platform-verified binary extraction prioritizing lossless `image/png` MIME before raw bitmap fallbacks.
   - **Windows `CF_DIB` / `CF_DIBV5` Sanitization:** Handles bottom-up vs top-down row order, 32-bit `BI_RGB` zero-alpha bugs (where screenshots from Snipping Tool / browsers supply `alpha = 0` for opaque pixels → promoted cleanly to `alpha = 255` when all alpha bytes are 0), and premultiplied vs straight alpha.
   - **Linux Wayland/X11:** Reads `image/png` directly from compositor clipboard (`wl-clipboard` / `arboard` Wayland data-control support) with graceful fallback.
   - **Internal Copy/Paste:** Copying selected strokes/shapes/text/images writes both an internal structured vector clipboard entry AND an optional rendered PNG to the OS clipboard so pasting back into Swift Note stays 100% vector-editable while pasting into external apps works seamlessly.
2. **Immutable Content-Addressed Asset Storage (`swiftnote-core`):**
   - Pasted or imported images are stored as deduplicated lossless PNG bytes keyed by BLAKE3/SHA-256 `AssetId` inside `Document::assets`, retaining exact original `(pixel_width, pixel_height)`.
   - Default placement sizes the image cleanly onto the current viewport center (capped to ~65% of page width while preserving exact aspect ratio) without ever touching or resampling the underlying asset bytes.
3. **Sharp Mipmapped GPU Rendering (`swiftnote-gpu`):**
   - Uploads full-resolution RGBA8 textures to `wgpu` with full mipmap generation (`log2(max(w, h)) + 1` levels via GPU linear blit downsample) and samples with `FilterMode::Linear` + `MipmapFilterMode::Linear` + `anisotropy_clamp` (up to `8x` where supported).
   - **Result:** Large 4K screenshots zoomed out to fit an A4 page show zero shimmering/blur, and zooming in to `300–400%` reveals every crisp native pixel.
   - Supports non-destructive normalized crop rectangle (`crop: Rect`), aspect-ratio lock / free resize, arbitrary rotation, Z-order front/back, and Image File import (`.png`, `.jpg`, `.webp`, `.bmp`).

### 5.7 Lazy Streaming Multi-Page PDF Engine (`pdfium-render`)
Solves multi-page PDF crashes, RAM spikes, and freezes permanently:
1. **Runtime Binding:** Dynamically loads bundled `pdfium.dll` (Windows x64) or `libpdfium.so` (Linux x64) adjacent to the binary, surfacing a crisp inline dialog if missing.
2. **Dedicated Single-Threaded `PdfWorker`:** Because C++ PDFium is not thread-safe for concurrent access on a single `FPDF_DOCUMENT`, all `pdfium-render` calls execute strictly sequentially on a dedicated `PdfWorker` thread via bounded request/response channels (with generation-ID cancellation when the user scrolls rapidly past pages).
3. **Instant `< 50 ms` Open & Encrypted PDF Support:**
   - Import supports **Open as New Note** and **Append to Current Note** (matching Scrble Ink).
   - Initial open reads ONLY `page_count`, per-page `(width_pt, height_pt)`, and optional encryption check (prompting cleanly via an in-app Password dialog if locked).
   - Each PDF page becomes a lightweight `PageBackground::Pdf { asset_id, page_index }` without rasterizing anything upfront.
   - Large PDF bytes are kept memory-mapped / file-backed or stored once as an immutable `PdfAsset` blob (never duplicated per page!).
   - **Encrypted PDF Persistence `(Design Call)`:** `.swnote` stores the **original encrypted PDF bytes** in `assets/<hash>.pdf` and NEVER writes passwords to disk. Once unlocked, the session password/handle stays strictly in RAM while the note is open; closed/reopened notes re-prompt cleanly and Gallery previews show a clean lock placeholder until unlocked. Vector PDF export skips copying restricted source pages unless unlocked cleanly in session.
4. **Lazy Viewport-Driven Rasterization & Caching:**
   - Only visible pages + `±1` adjacent pages are queued for rasterization at the exact needed physical tile resolution (`zoom * dpi_scale`, automatically split into `<= 2048px` tiles at deep zoom so `> 4K` zoom never exceeds GPU `max_texture_dimension_2d` or exhausts RAM).
   - Stale queued requests for scrolled-away pages are dropped before PDFium renders them.
   - Decoded PDF page textures live in a dedicated bounded LRU GPU cache (`128–256 MiB` configurable ceiling), keeping memory flat even on 500-page textbooks.
5. **High-Fidelity Export & Clean-Slate `swNK` v2 Schema `(Design Call)`:**
   - **Export to PDF:** Writes true vector PDF files via `printpdf` / `pdf-writer` (preserving original PDF background pages cleanly when PDFium/QPDF or `lopdf` merging is used, emitting native PDF vector Bézier/polygon ink strokes, crisp text, and lossless embedded images — never blurry flattened page screenshots unless user explicitly chooses flattened raster PDF).
   - **Export to PNG:** Exports single page or folder of high-DPI PNG pages (`150 / 300 / 600 DPI`). Editable PNG export uses a **clean-slate `swNK` v2 chunk schema** (zero backward compatibility with the discarded `Scrble Ink Remake` `v0.2.x` format); each PNG embeds only that single exported page's vector payload and can be imported back into Swift Note as an editable note/page.

### 5.8 Native File Format (`.swnote`) & TextBox Specification
- **`.swnote` Container `(Design Call)`:** Standard ZIP container storing:
  - `manifest.json`: schema version, `note_id`, title, created/modified timestamps, paper defaults, page order & metadata.
  - `pages/p_NNNN.json` (zstd/deflate-compressed): page size (`width_pt`, `height_pt`), background descriptor, vector strokes, geometric shapes, text boxes, image placements (referencing `assets/<hash>`).
  - `assets/<hash>.png` & `assets/<hash>.pdf`: deduplicated immutable binary assets stored with zero re-compression (`Store` or fast `Deflate`).
- **Atomic Saves & Autosave:** Writes to a temporary sibling `*.swnote.tmp` → `fsync` → atomic rename, triggered asynchronously off the UI thread on manual save, debounced autosave (`10 s` after idle edit), and safe recovery snapshot on crash.
- **`TextBox` Model `(Design Call)`:** Matches Scrble Ink `v6.8.6.0`'s `CanvasTextBoxTools` — box-level uniform `TextStyle` (`font_size_pt`, `bold`, `italic`, `color`) rather than mixed inline rich-text spans, rendered with a bundled Vietnamese-complete sans-serif + monospace font family (`Noto Sans` / `Inter` + `JetBrains Mono`) so layout, caret hit-testing, IME composition, and Vector PDF Export font subsetting are 100% deterministic across Windows and Linux.

### 5.9 Visual Design System & Layout (Matte Precision Minimalism — NO Glassmorphism)
Zero backdrop blur, zero translucent frosted glass. Inspired by Apple Pro creative tools (Pages / Freeform / Xcode flat toolbars) and Teenage Engineering precision:

1. **Color & Surface Tokens:**
   - **Light Mode:** Desk `#ECECEE` · Paper `#FFFFFF` · Toolbar / Popover `#FFFFFF` · Hover / Secondary Fill `#F3F3F5` · Active Selection Fill `#E8F0FE` · `1 px` Hairline Border `#DFDFE4` · Primary Text `#111113` · Secondary Text `#686870` · Cobalt Accent `#0066FF`.
   - **Dark Mode:** Desk `#0E0E10` · Paper (Light `#FFFFFF` or Blackboard `#16171B`) · Toolbar / Popover `#18181B` · Hover / Secondary Fill `#232328` · Active Selection Fill `#1B2B48` · `1 px` Hairline Border `#2B2B31` · Primary Text `#F4F4F6` · Secondary Text `#94949E` · Accent `#3B82F6`.
2. **Surface & Typography Rules:**
   - Solid opaque matte surfaces everywhere; subtle crisp `1 px` hairline dividers; detached popovers use a clean tight drop shadow (`0 8px 24px rgba(0,0,0,0.12)`) on an opaque background.
   - Custom monochrome SVG icons (`18×18 px`, `1.5 px` uniform stroke), clean geometric sans UI typography + tabular numeric counters (`Page 1 / 12`, `100%`), `8 px` spacing grid.
   - Snappy immediate state transitions (`0–100 ms` linear/ease-out micro-transitions only — no sluggish bouncy springs).
3. **Distraction-Free Editor Layout (Scrble Ink Ergonomics):**
```
┌──────────────────────────────────────────────────────────────────────────────┐
│ [≡] Note Title  [↶] [↷]   │  (●)(●)(●)(●)(●)  [✎] [⬡] [◇] [T] [📏] [↕] [◉] │ [📄] [↓] [▤] │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│                                                                              │
│                      ┌──────────────────────────────┐                        │
│                      │                              │                        │
│                      │     Crisp Paper / PDF        │                        │
│                      │     Unified GPU Canvas       │                        │
│                      │                              │                        │
│                      │                              │                        │
│                      └──────────────────────────────┘                        │
│                                                                              │
│                                                    ┌───────────────────────┐ │
│                                                    │  Page 1 / 12 │ 100% ⊡ │ │
└────────────────────────────────────────────────────┴───────────────────────┴─┘
```
- **Top Single-Row Toolbar (`44 px` height):**
  - **Left:** `[≡ Gallery]` · `Note Title (click to rename)` · `[↶ Undo]` `[↷ Redo]`
  - **Center:** `[5 Circular Pen Swatch Slots (click active slot opens Brush/Color/Width popover)]` · `[Eraser + Popover]` · `[Lasso / Box Select]` · `[Ink-to-Shape Toggle/Mode]` · `[Text & Shapes Popover]` · `[Ruler & Protractor]` · `[Vertical Spacer]` · `[View / Laser Pointer]`
  - **Right:** `[Paper / Input / Stabilizer Popover]` · `[Save / Import / Export Popover]` · `[Pages Sidebar Toggle]`
- **Collapsible Pages Preview Sidebar:** Live crisp page thumbnails, add page, duplicate, reorder drag-and-drop, delete, jump to page.
- **Close-up Writing Area Overlay (Milestone 3):** Anchored bottom or top magnified writing strip with Top/Bottom Palm Guard, auto-advance carriage return (LTR & RTL), matching Scrble Ink's signature feature.

### 5.10 Complete Scrble Ink `v6.8.6.0` Feature Parity Matrix (3 Milestones)

| Milestone | Scope | Features Delivered |
| :--- | :--- | :--- |
| **M1: Core Writing Excellence** *(Daily Driver)* | Foundation, Zero-Shift Ink, Clipboard Images, Multi-Page PDFs | • `SDL3` + `wgpu` + Matte `egui` shell on Windows & Linux (Wayland `zwp_tablet_v2` + X11 + IME)<br>• Xournal++ Input Isolation (Palm Rejection, pressure-0 filter, hardware eraser, barrel buttons, Space/Right/Middle pan, Zoom `10–800%` + double-tap zoom toggle)<br>• Xournal++ Cusp-Deadzone + Velocity-Gaussian Stabilizer (`Off/Light/Medium/Strong`)<br>• Analytical `StrokeContour` + Unified Wet/Dry GPU Pipeline (`0 px` shift on lift)<br>• 5 Pen Slots (`Pen`, `Pencil`, `Calligraphy`, `HighlighterRound`, `HighlighterSquare`), 16-swatch palette + HSV Custom Color Picker + Width Slider<br>• `Stroke Eraser`, `Segment Eraser` (`Small/Medium/Large`), `Back to Pen after erase` toggle<br>• Procedural Crisp Papers: `Blank`, `Ruled` (10/15/20/30/40), `Math` (10/15/20/30/40), `Graph`, `Dotted` (10/20), `Isometric`, `Blackboard` (auto-contrast)<br>• Lossless Clipboard Paste (`Ctrl+V`) + File Image Insert (PNG/JPG/WebP) + basic Move/Resize<br>• Lazy `pdfium-render` Multi-Page PDF Import (`Open as New` & `Append`, Password Dialog, bounded LRU cache)<br>• Native `.swnote` Save/Open + Atomic Autosave + Undo/Redo + Pages Preview Sidebar (add/delete/jump) |
| **M2: Complete Editing Toolkit** *(Scrble Ink Core Tools)* | Selection, Shapes, Text, Ruler, Spacer, Laser | • Lasso & Rectangular Selection (`SelectionLayer`) + Interactive `AdornerLayer` (Move, Corner/Edge Resize, Rotation handle, non-destructive Image Crop, Align Shapes, Recolor strokes, Copy/Cut/Paste/Duplicate/Delete)<br>• Ink-to-Shape (`ToShape` & `ToShapeAndLine` with `Shift`-hold straight line preview)<br>• Geometric Shapes Tool (`Line`, `Arrow`, `Circle`, `Ellipse`, `Rectangle`, `Polygon 3–10`, `Star` — outline & filled variants)<br>• Native `TextBox` Tool with full Vietnamese IME, font size, bold, italic, color, and live reflow<br>• Interactive Ruler & Protractor (`RulerCanvas`) with angle readout (`0–360°`), drag/rotate gestures, and direct pen stroke snapping<br>• Vertical Spacer (`OnePageSpacer` and `TwoPagesSpacer` / cross-page shifting) with full Undo/Redo<br>• `View & Laser Pointer` mode (`V`) with fading crisp red trail |
| **M3: Ecosystem, Polish & Release** *(Full Parity & Packaging)* | Writing Area, Full Paper Pack, Gallery, Vector Export, Installers | • Close-up Writing Area (`WritingPane`) with Top & Bottom Palm Guard, margin indicators, and LTR/RTL Auto-Advance<br>• Complete Scrble Ink Paper Pack: `Music` staves, `Todo`, Planners (`Daily`, `5-Days`, `7-Days`, `Monthly`, `Yearly`, `Habit`, `Homework`, `Fitness`, `Menu`, `Project` in Portrait/Landscape/Square) + Custom Image Paper<br>• Project & Folder Gallery (`ProjectTreeControl` folders, Large/Medium/List views, search, rename, move note, merge notes, live thumbnails)<br>• True Vector PDF Export (`Export PDF`), Multi-Page High-DPI PNG Export (`Export Folder` + `swNK` editable PNG), and OS Printing<br>• Native Packaging & Installers: Windows `.zip`/installer + Linux `AppImage` / `.deb` / `Flatpak`-ready desktop entry & icons<br>• Automated Latency, Memory Ceiling, & 500-Page PDF Stress Verification Suite |

---

## 6. Design Decisions Settled Upfront `(Design Call)`

The following implementation-adjacent product calls are explicitly recorded so the user can review or override them:
1. **License:** `GPL-3.0-or-later` `(Design Call)`.
2. **Primary Storage Format:** `.swnote` ZIP container (JSON/compressed pages + deduplicated lossless `assets/`) instead of overloading PNG `swNK` chunks for multi-page PDFs & 4K images, while keeping PNG + Vector PDF as clean exports `(Design Call)`.
3. **UI Layering:** Custom opaque solid-matte `egui` + `egui-wgpu` for chrome/dialogs/gallery ONLY; 100% custom `wgpu` renderer for canvas, ink, paper, images, PDFs, rulers, and adorners `(Design Call)`.
4. **Canonical Page Unit:** Typographic points (`1/72 inch`, matching PDFium and Xournal++ natively — e.g. A4 = `595.28 × 841.89 pt`) `(Design Call)`.
5. **Library Bundling:** Bundle `SDL3` (`SDL3.dll` / `libSDL3.so.0`) and `pdfium` (`pdfium.dll` / `libpdfium.so`) directly in build/release artifacts so out-of-the-box execution requires zero user setup `(Design Call)`.
6. **Encrypted PDF Security:** Store original encrypted PDF bytes in `assets/`, never write passwords to disk, keep session unlock strictly in RAM, and show a lock placeholder in Gallery thumbnails until unlocked `(Design Call)`.
7. **`TextBox` Granularity & Fonts:** Uniform box-level `TextStyle` (Font Size, Bold, Italic, Color — matching Scrble Ink `v6.8.6.0`'s `CanvasTextBox`) using bundled Vietnamese-complete sans-serif + monospace fonts `(Design Call)`.
8. **Clean-Slate `swNK` PNG Schema:** `swNK` v2 single-page editable PNG export/import with zero backward compatibility with the discarded `Scrble Ink Remake` `v0.2.x` format `(Design Call)`.
9. **Managed Note Library `(User Approved)`:** Auto-persisted managed workspace (`~/.local/share/swiftnote/library/` on Linux, `%APPDATA%\SwiftNote\library\` on Windows) with `NoteId` identity, M1 `[≡ Gallery]` recent-notes switcher + New Note, plus standalone `.swnote` Import/Save Copy anywhere on disk.
10. **Page Boundaries & Vertical Spacer `(User Approved)`:** Strict single-page element ownership; `OnePageSpacer` expands non-PDF `Page.height_pt` if shifted past bottom; `TwoPagesSpacer` / document spacer cascades elements into subsequent pages and inserts a blank paper page before fixed PDF sheets or at end of document.
11. **Blackboard Auto-Contrast & Settings Toggle `(User Approved)`:** Per-page/note independent of UI Light/Dark chrome; adapts paper grid, switches highlighter blend (`Multiply` → `SrcOver`), swaps default pen black ↔ chalk white, and dynamically inverts monochrome black ↔ white strokes controlled by an explicit **Auto-Invert Monochrome Ink on Blackboard** toggle in Swift Note Settings (default: ON), plus optional light-paper export toggle for printing.

---

## 7. What's Left to the Builder

The builder is explicitly responsible for settling the following details during planning and implementation without re-confirming with the user:
- Exact crate versions compatible with `rustc 1.100.0` (`wgpu`, `egui`, `egui-wgpu`, `sdl3`/`sdl3-sys`, `pdfium-render`, `rstar`, `arboard`, `zip`, `printpdf`/`pdf-writer`, `resvg`/`usvg`).
- Exact WGSL shader source for stroke anti-aliasing, procedural graphite `Pencil` grain, highlighter single-pass mask composition, procedural grid/ruled/isometric shaders, and GPU mipmap generation.
- Exact `SDL3` build/link configuration across Windows MSVC/MinGW and Ubuntu 24.04 CI (`cmake` vs prebuilt dev artifact caching in GitHub Actions).
- Exact internal binary/JSON schema versioning inside `.swnote`, `UndoAction` delta payloads, and `proptest` / `criterion` benchmark organization.
