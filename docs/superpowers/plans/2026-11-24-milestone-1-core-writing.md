# Swift Note v2 — Milestone 1 (Core Writing Excellence) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the Milestone 1 cross-platform daily driver of Swift Note v2 (Windows + Linux Wayland/X11) with zero-shift wet/dry GPU ink, Xournal++-grade stylus stabilization, lossless clipboard image pasting, lazy multi-page PDF import, managed `.swnote` persistence, and opaque solid-matte UI chrome.

**Architecture:** Clean 3-crate unidirectional architecture: `swiftnote-core` (`#![forbid(unsafe_code)]`, OS/GPU-independent document, stroke stabilizer, analytical `StrokeContour` tessellator, R-Tree spatial index, erasers, undo/redo, `.swnote` ZIP library), `swiftnote-gpu` (unified `wgpu 30.0` MSAA stroke/page/mipmapped-image renderer + dedicated single-threaded `pdfium-render` worker), and `swiftnote-app` (`sdl3 0.22` `CategoryPen`/IME/clipboard + opaque solid-matte custom-skinned `egui 0.37` chrome emitting `UiAction` commands to `AppController`).

**Tech Stack:** Rust 2021 (`rust-version = "1.88"`, verified host `rustc 1.100.0`), `wgpu 30.0`, `egui 0.37`, `egui-wgpu 0.37`, `sdl3 0.22` (`raw-window-handle`, `build-from-source-static`), `pdfium-render 0.9`, `arboard 3.7`, `rstar 0.14`, `rfd 0.18`, `zip 9.8`, `blake3 1.8`, `png 0.18`, `image 0.25`, `resvg 0.50`, `directories 6.0`, `thiserror 2.0`, `anyhow 1.0`, `serde 1.0`, `serde_json 1.0`, `proptest 1.12`, `criterion 0.8`.

**Spec:** [2026-11-24-swiftnote-architecture-design.md](../../specs/2026-11-24-swiftnote-architecture-design.md)

## Global Constraints

- **License & Repo:** `GPL-3.0-or-later`, repository `https://github.com/nguyennguyenplus-source/swiftnote-ink`.
- **Rust & Safety:** Edition `2021`, `rust-version = "1.88"`; workspace Clippy denies `unwrap_used`, `expect_used`, `dbg_macro`; `unsafe_code = "warn"` (`#![forbid(unsafe_code)]` in `swiftnote-core`, `// SAFETY:` on every `unsafe` block); `thiserror` in library crates, `anyhow` at app boundary.
- **Hot-Path Discipline:** Zero heap allocations inside stylus motion events and per-frame rendering; pre-allocated scratch buffers and `rstar` R-Tree spatial indexing.
- **Zero Wet/Dry Renderer Mismatch:** Active wet ink and dry committed strokes execute the exact same `StrokeContour` tessellation and `wgpu` MSAA pipeline (`0.0 px` positional/blur/opacity shift on lift).
- **Zero Glassmorphism:** 100% opaque solid-matte UI surfaces with `1 px` hairline borders (`Light`: Desk `#ECECEE`, Surface `#FFFFFF`, Border `#DFDFE4`, Accent `#0066FF`; `Dark`: Desk `#0E0E10`, Surface `#18181B`, Border `#2B2B31`, Accent `#3B82F6`); zero backdrop blur or translucent frosted panels.
- **Canonical Page Units:** Typographic points (`1/72 inch`, A4 = `595.28 × 841.89 pt`, US Letter = `612.0 × 792.0 pt`).
- **Dual-OS Target:** Native execution and CI (`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`) on both `windows-latest` and `ubuntu-24.04` (Wayland `zwp_tablet_v2`, X11 XInput2, Windows Ink, full Vietnamese IME).

## Review Focus

1. **Short rapid math strokes (`'2'`, `'x'`, `'π'`, Vietnamese accents) & single-tap dots:** Cusp detection (`dot < 0`) must preserve crisp `>90°` corners, `finish()` must flush trailing points without retroactive shift, and single-point / coincident-point strokes must emit closed non-NaN triangle fans — pinned by Task 2 tests.
2. **Corrupted, `pressure == 0.0`, NaN/Inf, and `>100px` teleport stylus packets:** `InputGuard` must discard lift-artifact `pressure == 0.0` motion packets, non-finite coordinates, and impossible enter/leave teleports while smoothly clamping first-point pressure ramps — pinned by Task 2 & Task 6 tests.
3. **Segment eraser on wide variable-pressure strokes & Undo Z-order:** Segment eraser must account for `max_half_width()`, split surviving spans cleanly with consistent style/pressure, and Undo must binary-insert restored strokes by `StrokeId` to preserve exact draw Z-order — pinned by Task 3 tests.
4. **Windows `CF_DIB`/`CF_DIBV5` zero-alpha clipboard bitmaps & corrupt/zip-bomb `.swnote` archives:** Clipboard sanitization must promote all-zero alpha channels to `255` opaque while preserving real PNG alpha, and `.swnote` loader must abort cleanly above `MAX_ARCHIVE_UNCOMPRESSED_BYTES` (`256 MiB`) or malformed entries — pinned by Task 3 & Task 6 tests.
5. **Fast scrolling/zooming across 200+ page & encrypted PDFs:** `PdfWorker` must execute strictly sequentially, drop stale generation-ID render requests on fast scroll, bound LRU VRAM textures to `256 MiB`, never persist passwords to disk, and show locked placeholders until unlocked — pinned by Task 5 tests.

---

## File Structure Map

- Root: `Cargo.toml`, `LICENSE`, `.github/workflows/ci.yml`
- `crates/swiftnote-core/Cargo.toml` + `src/`:
  - `lib.rs` (crate root, `#![forbid(unsafe_code)]`, exports)
  - `error.rs` (`CoreError`, `MAX_ARCHIVE_UNCOMPRESSED_BYTES`)
  - `coords.rs` (`ScreenPoint`, `PhysicalPoint`, `CanvasPoint`, `PagePoint`, `Rect`, `ViewportTransform`)
  - `brush.rs` (`Color`, `BrushKind`, `BrushStyle`, `default_pen_slots`, `DEFAULT_PALETTE`)
  - `paper.rs` (`PaperStandard`, `PaperTemplate`, `PaperPolarity`, `PageBackground`)
  - `settings.rs` (`UiTheme`, `InputMode`, `StabilizerPreset`, `AppSettings`)
  - `stabilizer.rs` (`RawInputSample`, `InputGuard`, `StrokeStabilizer`, `decompose_pressure_step`)
  - `stroke.rs` (`StrokeId`, `StrokePoint`, `Stroke`)
  - `contour.rs` (`ContourVertex`, `ContourMesh`, `StrokeTessellator`)
  - `spatial.rs` (`StrokeIndex` over `rstar`)
  - `eraser.rs` (`EraserMode`, `EraserSize`, `EraseResult`, `erase_at`)
  - `document.rs` (`AssetId`, `ImageAsset`, `ImagePlacement`, `PdfAssetMeta`, `PageId`, `Page`, `Document`)
  - `history.rs` (`DirtyRegion`, `UndoCommand`, `UndoStack`)
  - `tool.rs` (`ActiveTool`, `ToolState`)
  - `storage.rs` (`save_swnote_atomic`, `load_swnote`)
  - `library.rs` (`LibraryEntry`, `LibraryIndex`, `NoteLibrary`)
  - `benches/core_bench.rs` + `tests/milestone1_workflow.rs`
- `crates/swiftnote-gpu/Cargo.toml` + `src/`:
  - `lib.rs`, `error.rs` (`GpuError`)
  - `shaders/stroke.wgsl`, `shaders/paper.wgsl`, `shaders/image.wgsl`, `shaders/blit.wgsl`
  - `tile_cache.rs` (`TileKey`, `GpuTileCache`, `256 MiB` LRU eviction)
  - `renderer.rs` (`UnifiedRenderer`, 4x MSAA page tiles + active stroke mask + mipmapped image upload)
  - `pdf_worker.rs` (`PdfWorker`, `PdfOpenSummary`, `PdfRenderRequest`, `PdfRenderTile`, generation cancellation)
- `crates/swiftnote-app/Cargo.toml` + `src/`:
  - `main.rs`, `clipboard.rs` (OS PNG/`CF_DIB` sanitization + internal vector clipboard)
  - `input.rs` (`SdlInputMapper` — `CategoryPen`, Palm Rejection, IME forwarding, wheel/pinch)
  - `theme.rs` (solid-matte Apple Pro `egui` style tokens + embedded SVG monochrome icons via `resvg`)
  - `ui.rs` (top 44px toolbar, pen/eraser/paper/save popovers, M1 `[≡ Gallery]` switcher, Pages sidebar, status pill, PDF password modal — emits `Vec<UiAction>`)
  - `controller.rs` (`AppController` unidirectional state mutation, autosave timer, dialogs via `rfd`)

---

### Task 1: Workspace Scaffold + Core Coordinate, Brush, Paper & Settings Models

**Files:**
- Create: `LICENSE`, `Cargo.toml`, `crates/swiftnote-core/Cargo.toml`
- Create: `crates/swiftnote-core/src/lib.rs`, `error.rs`, `coords.rs`, `brush.rs`, `paper.rs`, `settings.rs`

**Interfaces:**
- Produces: `CoreError`, `MAX_ARCHIVE_UNCOMPRESSED_BYTES`, `ScreenPoint`, `PhysicalPoint`, `CanvasPoint`, `PagePoint`, `Rect`, `ViewportTransform`, `Color`, `BrushKind`, `BrushStyle`, `default_pen_slots`, `DEFAULT_PALETTE`, `PaperStandard`, `PaperTemplate`, `PaperPolarity`, `PageBackground`, `UiTheme`, `InputMode`, `StabilizerPreset`, `AppSettings`.

- [ ] **Step 1: Write failing unit & property tests for `coords`, `brush`, `paper`, and `settings`**
  - In `coords.rs`: test `viewport_roundtrip_proptest` (`screen_to_canvas` / `canvas_to_screen` within `1e-6`), `zoom_preserves_anchor_proptest` (`set_zoom_at(anchor, z)` keeps world point under `anchor` invariant), and `zoom_clamps_to_bounds` (`0.1..=8.0`).
  - In `brush.rs`: test `blackboard_auto_invert_flips_black_and_white_only` (near-black `<=48` becomes chalk white, near-white `>=207` becomes black, chromatic `Color::BLUE` remains unchanged, and `auto_invert_monochrome == false` keeps black literal).
  - In `settings.rs`: test `AppSettings::default()` sets `auto_invert_monochrome_ink == true`, `stabilizer_preset == StabilizerPreset::Light`, `back_to_pen_after_erase == false`.

- [ ] **Step 2: Run test to verify it fails**
  - Run: `cargo test -p swiftnote-core`
  - Expected: FAIL (workspace/crate files not yet created).

- [ ] **Step 3: Scaffold `LICENSE`, root `Cargo.toml`, `crates/swiftnote-core/Cargo.toml`, and implement `error.rs`, `coords.rs`, `brush.rs`, `paper.rs`, `settings.rs`, `lib.rs`**
  - Enforce `#![forbid(unsafe_code)]` in `lib.rs` and workspace lints (`unwrap_used = "deny"`, `expect_used = "deny"`, `dbg_macro = "deny"`, `unsafe_code = "warn"`).
  - Model `PaperStandard` dimensions in canonical `1/72"` typographic points (`A4: 595.28 × 841.89`, `Letter: 612.0 × 792.0`, `Legal: 612.0 × 1008.0`, `Square: 612.0 × 612.0`).

- [ ] **Step 4: Verify formatting, Clippy, and tests pass**
  - Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
  - Expected: PASS with 0 warnings.

- [ ] **Step 5: Commit**
  - `git add LICENSE Cargo.toml crates/swiftnote-core/Cargo.toml crates/swiftnote-core/src/{lib,error,coords,brush,paper,settings}.rs`
  - `git commit -m "feat(core): scaffold workspace and core coordinate, brush, paper, and settings models"`

---

### Task 2: Xournal++ InputGuard, Cusp-Deadzone Gaussian Stabilizer & Analytical `StrokeContour` Tessellator

**Files:**
- Create: `crates/swiftnote-core/src/stabilizer.rs`, `stroke.rs`, `contour.rs`
- Create: `crates/swiftnote-core/benches/core_bench.rs`
- Modify: `crates/swiftnote-core/src/lib.rs`, `crates/swiftnote-core/Cargo.toml` (add `[[bench]] name = "core_bench" harness = false`)

**Interfaces:**
- Consumes: `PagePoint`, `Rect`, `BrushStyle`, `BrushKind`, `StabilizerPreset`, `Color`.
- Produces: `RawInputSample`, `InputGuard`, `StrokeStabilizer`, `StrokeId`, `StrokePoint`, `Stroke`, `ContourVertex`, `ContourMesh`, `StrokeTessellator`.

- [ ] **Step 1: Write failing tests covering Review Focus #1 & #2 and the Zero-Shift Invariant**
  - `input_guard_drops_nan_pressure_zero_and_teleport_spikes`: verifies `NaN`/`inf`, `pressure == 0.0` motion packets, and `> 100.0` logical px jumps are discarded while first-point pressure thickening updates smoothly.
  - `stabilizer_preserves_sharp_math_cusp_and_flushes_tail`: feeds sharp `>90°` corner (`'2'` / `'x'`) and confirms cusp vertex is preserved within `1.5 pt` while straight runs smooth jitter; confirms `finish()` appends tail to final coordinate without mutating previously emitted points.
  - `contour_single_dot_and_coincident_points_have_no_nan`: single-point and identical-point strokes produce valid finite `ContourMesh` triangles (`vertices.len() >= 12`, zero NaN).
  - `contour_incremental_matches_committed_stroke_bit_for_bit`: **Zero-Shift Gate** — tessellating incrementally as points arrive produces identical `vertices` and `indices` (`<= 1e-6` / bit-exact) to tessellating the committed `Stroke`.
  - `contour_self_crossing_stroke_has_consistent_ccw_winding`: checks no degenerate zero-area cap reversals or winding holes across `Pen`, `Pencil`, `Calligraphy`, `HighlighterRound`, and `HighlighterSquare`.

- [ ] **Step 2: Run tests to verify they fail**
  - Run: `cargo test -p swiftnote-core`
  - Expected: FAIL (`stabilizer`, `stroke`, `contour` modules missing).

- [ ] **Step 3: Implement `stabilizer.rs`, `stroke.rs`, `contour.rs`, and `benches/core_bench.rs`**
  - Implement Xournal++ `Deadzone` + `Cusp Detection` (`dot < 0.0`) + `VelocityGaussian` (`exp(-(sum_v)^2 / (2*sigma^2))`) + `MAX_WIDTH_VARIATION` sub-segment decomposition.
  - Implement Xournal++ `StrokeContour` analytical offset envelopes with `asin` circle-tangent couplings and semicircular round caps (or oriented calligraphy nib / square highlighter caps) using reusable scratch `Vec` buffers inside `StrokeTessellator`.

- [ ] **Step 4: Verify formatting, Clippy, tests, and benchmark compilation**
  - Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo bench --no-run`
  - Expected: PASS with 0 warnings.

- [ ] **Step 5: Commit**
  - `git add crates/swiftnote-core/`
  - `git commit -m "feat(core): add Xournal++ stroke stabilizer, input guard, and analytical contour tessellator"`

---

### Task 3: R-Tree Spatial Index, Erasers, Document Model, Undo/Redo, ToolState & `.swnote` Library

**Files:**
- Create: `crates/swiftnote-core/src/spatial.rs`, `eraser.rs`, `document.rs`, `history.rs`, `tool.rs`, `storage.rs`, `library.rs`
- Create: `crates/swiftnote-core/tests/milestone1_workflow.rs`
- Modify: `crates/swiftnote-core/src/lib.rs`

**Interfaces:**
- Consumes: `CoreError`, `MAX_ARCHIVE_UNCOMPRESSED_BYTES`, `PagePoint`, `CanvasPoint`, `Rect`, `BrushStyle`, `PageBackground`, `Stroke`, `StrokeId`, `StrokePoint`.
- Produces: `StrokeIndex`, `EraserMode`, `EraserSize`, `EraseResult`, `erase_at`, `AssetId`, `ImageAsset`, `ImagePlacement`, `PdfAssetMeta`, `PageId`, `Page`, `Document`, `DirtyRegion`, `UndoCommand`, `UndoStack`, `ActiveTool`, `ToolState`, `save_swnote_atomic`, `load_swnote`, `NoteLibrary`, `LibraryEntry`.

- [ ] **Step 1: Write failing unit, proptest, and integration tests covering Review Focus #3 & #4**
  - In `spatial.rs`: `rtree_matches_bruteforce_oracle_proptest` across random strokes and query rects.
  - In `eraser.rs`: `segment_eraser_accounts_for_stroke_half_width_and_splits_cleanly` (wide stroke `16 pt` grazed at `8 pt` edge splits properly into non-empty sub-strokes preserving pressure/tilt/color).
  - In `history.rs`: `undo_restores_erased_strokes_in_exact_stroke_id_z_order`.
  - In `tool.rs`: `hardware_eraser_and_back_to_pen_restore_previous_slot`.
  - In `storage.rs` & `tests/milestone1_workflow.rs`: `.swnote` atomic save/load roundtrip deduplicating shared PNG assets; zip-bomb rejection test when decompressed manifest/entry exceeds `MAX_ARCHIVE_UNCOMPRESSED_BYTES`; `NoteLibrary` create/save/rename/list-recent/export-copy test using `tempdir`.

- [ ] **Step 2: Run tests to verify they fail**
  - Run: `cargo test -p swiftnote-core`
  - Expected: FAIL (`spatial`, `eraser`, `document`, `history`, `tool`, `storage`, `library` missing).

- [ ] **Step 3: Implement `spatial.rs`, `eraser.rs`, `document.rs`, `history.rs`, `tool.rs`, `storage.rs`, `library.rs`, and wire `lib.rs`**
  - Keep `Page::strokes` strictly sorted by `StrokeId` for binary-search Z-order restoration on Undo.
  - Deduplicate images/PDFs by BLAKE3 `AssetId` inside `Document` and ZIP `assets/`. Never persist PDF passwords in `manifest.json` or assets.

- [ ] **Step 4: Verify formatting, Clippy, full tests, and bench compilation**
  - Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo bench --no-run`
  - Expected: PASS with 0 warnings.

- [ ] **Step 5: Commit**
  - `git add crates/swiftnote-core/`
  - `git commit -m "feat(core): add document model, R-tree spatial index, erasers, undo stack, and .swnote library"`

---

### Task 4: `swiftnote-gpu` — Unified Zero-Shift GPU Stroke, Paper & Mipmapped Image Renderer

**Files:**
- Create: `crates/swiftnote-gpu/Cargo.toml`, `src/lib.rs`, `src/error.rs`, `src/tile_cache.rs`, `src/renderer.rs`
- Create: `crates/swiftnote-gpu/src/shaders/stroke.wgsl`, `paper.wgsl`, `image.wgsl`, `blit.wgsl`
- Modify: `Cargo.toml` (add `crates/swiftnote-gpu` to workspace members)

**Interfaces:**
- Consumes: `swiftnote_core::{Document, Page, PageId, PageBackground, PaperTemplate, PaperPolarity, Stroke, StrokeTessellator, ContourMesh, AssetId, ImageAsset, ImagePlacement, ViewportTransform, DirtyRegion, Rect, AppSettings}`.
- Produces: `GpuError`, `TileKey`, `GpuTileCache`, `UnifiedRenderer` (`new`, `resize`, `sync_dirty`, `update_active_stroke`, `commit_active_stroke`, `upload_image_asset_mipmapped`, `render_frame`).

- [ ] **Step 1: Write failing unit & headless GPU tests**
  - In `tile_cache.rs`: `lru_cache_enforces_256mib_budget_and_generation_invalidation` (stale background results cannot clear newer tile `dirty_gen`).
  - In `renderer.rs` (using `wgpu` headless/software adapter when available or shader/pipeline validation test): `wgsl_shaders_compile_and_wet_matches_dry_pipeline` (verifies identical pipeline descriptors, blend states, and exact physical-pixel tile rect math without `floor/ceil` 1px stretch seams).
  - Verify `upload_image_asset_mipmapped` allocates full `mip_level_count` chain down to `1×1` without downsampling original PNG bytes.

- [ ] **Step 2: Run tests to verify they fail**
  - Run: `cargo test -p swiftnote-gpu`
  - Expected: FAIL (`swiftnote-gpu` crate does not exist).

- [ ] **Step 3: Implement `swiftnote-gpu` shaders, `GpuTileCache`, and `UnifiedRenderer`**
  - Implement `stroke.wgsl` (solid `Pen`/`Calligraphy`, procedural screen-space graphite grain for `Pencil`, single-pass coverage + `Multiply`/`SrcOver` for `Highlighter`), `paper.wgsl` (crisp pixel-snapped `Blank`, `Ruled`, `Math`, `Graph`, `Dotted`, `Isometric`, and `Blackboard`), `image.wgsl` (trilinear mipmapped anisotropic sampling + crop UV transform), and `blit.wgsl`.
  - Use exact 4x MSAA tile render targets sized to physical pixels (`512×512`) and an `ActiveStrokeMask` layer composited identically to dry page tiles.

- [ ] **Step 4: Verify formatting, Clippy, and tests pass**
  - Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
  - Expected: PASS.

- [ ] **Step 5: Commit**
  - `git add Cargo.toml crates/swiftnote-gpu/`
  - `git commit -m "feat(gpu): add unified zero-shift wgpu stroke, paper, and mipmapped image renderer"`

---

### Task 5: `swiftnote-gpu` — Lazy Streaming Multi-Page `pdfium-render` Worker

**Files:**
- Create: `crates/swiftnote-gpu/src/pdf_worker.rs`
- Modify: `crates/swiftnote-gpu/src/lib.rs`, `renderer.rs`, `Cargo.toml` (add `pdfium-render = "0.9"`)

**Interfaces:**
- Consumes: `PdfAssetMeta`, `AssetId`, `GpuError`.
- Produces: `PdfWorker` (`spawn`, `open_document`, `unlock_document`, `request_tile`, `poll_completed`, `cancel_stale_generation`), `PdfOpenSummary`, `PdfTileRequest`, `PdfTileResult`.

- [ ] **Step 1: Write failing tests covering Review Focus #5**
  - `pdf_worker_reports_missing_library_gracefully` (never panics if `pdfium` shared library is absent in test env).
  - `pdf_worker_drops_stale_generation_requests_on_fast_scroll` (queuing generation 1..50 pages and advancing generation cancels obsolete renders).
  - `pdf_worker_password_unlock_stays_in_ram_only` (encrypted PDF reports `locked = true` without crashing; unlocked password never serializes to `PdfAssetMeta`).
  - `pdf_worker_splits_deep_zoom_into_max_2048_tiles`.

- [ ] **Step 2: Run tests to verify they fail**
  - Run: `cargo test -p swiftnote-gpu pdf_worker`
  - Expected: FAIL (`pdf_worker` missing).

- [ ] **Step 3: Implement sequential single-threaded `PdfWorker` over bounded channels**
  - Isolate all `pdfium-render` calls to the single `PdfWorker` thread; dynamically search executable dir then OS library path for `pdfium.dll` / `libpdfium.so`; wire `PdfTileResult` upload into `GpuTileCache` (`256 MiB` bounded LRU).

- [ ] **Step 4: Verify formatting, Clippy, and tests pass**
  - Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
  - Expected: PASS.

- [ ] **Step 5: Commit**
  - `git add crates/swiftnote-gpu/`
  - `git commit -m "feat(gpu): add lazy single-threaded pdfium-render worker and LRU tile streaming"`

---

### Task 6: `swiftnote-app` — SDL3 Windowing, `CategoryPen` Input Pipeline, IME & Lossless Clipboard

**Files:**
- Create: `crates/swiftnote-app/Cargo.toml`, `src/clipboard.rs`, `src/input.rs`
- Modify: `Cargo.toml` (add `crates/swiftnote-app` to workspace members)

**Interfaces:**
- Consumes: `swiftnote_core::{InputMode, RawInputSample, InputGuard, ViewportTransform, ActiveTool}`.
- Produces: `ClipboardContent` (`ImagePng { png_bytes, width_px, height_px }`, `InternalVector`, `Text`), `read_os_clipboard`, `sanitize_dib_alpha`, `SdlInputMapper`, `CanvasInputEvent`.

- [ ] **Step 1: Write failing unit tests covering Review Focus #2 & #4**
  - `sanitize_dib_alpha_promotes_all_zero_alpha_to_opaque_255`: confirms Windows `CF_DIB` screenshots with `a == 0` everywhere become `a == 255` opaque while preserving genuine partial transparency when any pixel has `a > 0`.
  - `sdl_input_mapper_isolates_palm_touch_when_stylus_in_proximity`: touch down/motion is ignored while stylus proximity is active; 2-finger pinch emits zoom/pan deltas when stylus is away.
  - `sdl_input_mapper_restores_tool_after_hardware_eraser_and_focus_loss`: verifies `SDL_PEN_INPUT_ERASER_TIP` overrides to eraser and cleanly cancels/commits active strokes on `WindowFocusLost`.

- [ ] **Step 2: Run tests to verify they fail**
  - Run: `cargo test -p swiftnote-app`
  - Expected: FAIL (`swiftnote-app` crate missing).

- [ ] **Step 3: Implement `clipboard.rs` (prioritizing PNG before DIB + `arboard`) and `input.rs` (`SDL3 CategoryPen` + `SDL_TEXTINPUT`/`SDL_TEXTEDITING` IME)**

- [ ] **Step 4: Verify formatting, Clippy, and tests pass**
  - Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
  - Expected: PASS.

- [ ] **Step 5: Commit**
  - `git add Cargo.toml crates/swiftnote-app/`
  - `git commit -m "feat(app): add SDL3 CategoryPen input isolation, IME plumbing, and lossless clipboard"`

---

### Task 7: `swiftnote-app` — Opaque Solid-Matte `egui` Chrome, Unidirectional `AppController` & Main Loop

**Files:**
- Create: `crates/swiftnote-app/src/theme.rs`, `src/ui.rs`, `src/controller.rs`, `src/main.rs`
- Create: `crates/swiftnote-app/assets/icons/*.svg` (monochrome `18×18`, `1.5 px` stroke icons)

**Interfaces:**
- Consumes: `swiftnote_core::{Document, NoteLibrary, ToolState, UndoStack, AppSettings}`, `swiftnote_gpu::{UnifiedRenderer, PdfWorker}`, `clipboard`, `input`.
- Produces: `UiAction`, `MatteTheme`, `render_chrome`, `AppController`, `swiftnote` binary entry point.

- [ ] **Step 1: Write failing unit tests for unidirectional `AppController` & `MatteTheme` invariants**
  - `matte_theme_has_zero_translucent_panels_or_blur`: asserts Light & Dark `egui::Visuals` panel/window fills have alpha `255`, `1.0 px` hairline stroke widths, and exact hex palette (`#ECECEE`/`#FFFFFF`/`#0066FF` Light; `#0E0E10`/`#18181B`/`#3B82F6` Dark).
  - `app_controller_applies_ui_actions_sequentially_without_reentrancy`: simulates rapid Pen/Eraser/HSV Color/Width/Paper/Blackboard-Toggle/Undo/Redo/PDF-Import/Paste actions cleanly via `UiAction` queue without panics.
  - `ctrl_v_paste_places_image_centered_without_resampling_asset`: asserts pasted PNG keeps exact original pixel dimensions in `ImageAsset`.

- [ ] **Step 2: Run tests to verify they fail**
  - Run: `cargo test -p swiftnote-app`
  - Expected: FAIL (`theme`, `ui`, `controller` missing).

- [ ] **Step 3: Implement `theme.rs`, `ui.rs`, `controller.rs`, and `main.rs`**
  - Build single-row 44px top matte toolbar (`[≡ Gallery] · Title · [↶][↷] | 5 Circular Pen Slots | Eraser | Paper/Settings | Save/PDF/Image | Pages Sidebar`), M1 `[≡ Gallery]` recent-notes drawer + New Note, 16-color + HSV color picker popover, PDF password prompt modal, bottom-right solid status pill (`Page X / Y | 100% | Fit`), and event-driven `SDL_WaitEvent` idle loop (`0%` GPU/CPU at rest).

- [ ] **Step 4: Verify formatting, Clippy, tests, and Windows debug/release binary build**
  - Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --bin swiftnote`
  - Expected: PASS and produces runnable `swiftnote.exe`.

- [ ] **Step 5: Commit**
  - `git add crates/swiftnote-app/`
  - `git commit -m "feat(app): implement solid-matte egui chrome, AppController, and event-driven SDL3+wgpu loop"`

---

### Task 8: Dual-OS GitHub Actions CI, Bundled Library Scripts & Milestone 1 Release Verification

**Files:**
- Create: `.github/workflows/ci.yml`, `scripts/fetch-deps.ps1`, `scripts/fetch-deps.sh`, `CHANGELOG.md`, `README.md`

**Interfaces:**
- Produces: Dual-OS CI (`windows-latest` + `ubuntu-24.04`) running `fmt`, `clippy -D warnings`, `cargo test --workspace`, and bundled artifact build (`SDL3` + `pdfium`).

- [ ] **Step 1: Add `.github/workflows/ci.yml`, `scripts/fetch-deps.{ps1,sh}`, `README.md`, and `CHANGELOG.md` (`0.1.0`)**
  - Configure Ubuntu 24.04 apt dependencies (`cmake`, `ninja-build`, `pkg-config`, `libx11-dev`, `libwayland-dev`, `libxkbcommon-dev`, `libvulkan-dev`, `libegl-dev`, `libdbus-1-dev`, `libgtk-3-dev` for `rfd` portal) and Windows `windows-latest` MSVC build + automated `pdfium` shared library fetch scripts.

- [ ] **Step 2: Run full local verification on Windows**
  - Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo bench --no-run`
  - Run: `scripts/fetch-deps.ps1` and launch `swiftnote.exe` smoke check (verify crisp matte window render, zero console panics).

- [ ] **Step 3: Commit, push `main`, verify both GitHub Actions CI jobs pass green, then tag `v0.1.0`**
  - `git push origin main` → verify `windows-latest` and `ubuntu-24.04` CI pass → `git tag -a v0.1.0 -m "Release v0.1.0: Milestone 1 Core Writing Excellence"` → `git push origin main --follow-tags`.
