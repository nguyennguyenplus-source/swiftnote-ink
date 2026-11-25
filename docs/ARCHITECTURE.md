# Swift note — Software Architecture Specification

This document specifies the internal architecture of **Swift note**, a hybrid **C++20/GTK3 + Rust** fork of Xournal++ (`1.3.8+dev`). It focuses on the subsystems engineered specifically for Swift note and how they integrate safely with the upstream Xournal++ engine.

---

## 1. System Topology & Module Map

Swift note preserves Xournal++'s core document model (`Document`, `PageRef`, `Stroke`, `Text`, `Image`), Poppler PDF backend, Lua plugin runtime, and geometry overlays while introducing a clean presentation shell and a Rust core static library (`libswiftnote_rs.a`).

```text
xournalpp/
├── rust/swiftnote-rs/                     # Memory-safe Rust core (staticlib)
│   ├── include/swiftnote_ffi.h            # Hand-audited C ABI contract
│   └── src/
│       ├── lib.rs                         # Panic-safe extern "C" entry points & memory lifecycle
│       ├── config.rs                      # Atomic JSON persistence, pen slots, shortcut table
│       ├── media.rs                       # Chroma-aware ink inversion & lossless PNG/DIB pipeline
│       ├── library.rs                     # WalkDir notebook scanner, Vietnamese folding, preview peek
│       └── error.rs                       # Typed internal error enum mapped to SnStatus
├── cmake/include/SwiftNoteRust.cmake      # Cargo build & staticlib link integration (Win32 + Linux)
├── src/core/gui/swiftnote/                # Swift note presentation & bridge layer (C++20/GTK3)
│   ├── SwiftBridge.{h,cpp}                # RAII C++ FFI wrapper + lock-free DisplayState
│   ├── SwiftShell.{h,cpp}                 # 3-layout shell, shortcuts, popovers, 4-tab Settings
│   ├── SwiftLibraryView.{h,cpp}           # In-app notebook browser & search UI
│   └── CloseUpWritingPanel.{h,cpp}        # Magnified auto-advance writing strip
├── src/core/view/                         # Display-layer hooks (Blackboard & Cairo safety)
│   ├── StrokeView.cpp / TextView.cpp      # Zero-overhead DisplayState::resolveInkColor()
│   ├── background/BackgroundView.cpp      # Non-destructive Blackboard surface substitution
│   └── Mask.cpp                           # Cairo surface/context status validation guard
├── ui/
│   ├── swiftnote.css                      # Calibrated Light/Dark UI design tokens
│   ├── fonts/PlusJakartaSans.ttf          # Bundled UI typography
│   └── pixmaps/swiftnote-logo-*.{svg,png} # Rounded vector identity assets
└── test/gtk_tests/
    └── SwiftShellRegressionTest.cpp       # Automated live-GTK3 regression suite
```

---

## 2. Hybrid C++20 ↔ Rust C-ABI Boundary

Integrating Rust into a C++20/GTK3 application requires strict invariants so Rust panics never unwind across FFI and heap allocations never cross allocator boundaries.

### Design Invariants

1. **Panic Firewall (`catch_unwind`):** Every `extern "C"` entrypoint in [`rust/swiftnote-rs/src/lib.rs`](../rust/swiftnote-rs/src/lib.rs) wraps execution in `std::panic::catch_unwind(AssertUnwindSafe(...))` and returns `SnStatus::ErrPanic (99)` on fault — guaranteeing zero Undefined Behavior across the C++ stack.
2. **Allocator-Matched Deallocation:** Heap-allocated return values (`*mut c_char`, `SnByteBuffer`, `SnNoteList`) are freed exclusively by their paired Rust destructors (`sn_string_free`, `sn_buffer_free`, `sn_note_list_free`) wrapped by RAII guards inside [`SwiftBridge.cpp`](../src/core/gui/swiftnote/SwiftBridge.cpp).
3. **UTF-8 Boundary Safety:** Fixed-size FFI string slots (`[c_char; 64]`, `[c_char; 512]`) use `write_fixed_cstr()` which steps backward using `str::is_char_boundary()` before truncation so multi-byte Vietnamese UTF-8 characters are never split mid-codepoint.
4. **Atomic Configuration Persistence:** `config::save_config()` serializes `SwiftConfig` (`schema_version = 2`) with `serde_json` and writes atomically via a temporary file rename so sudden power loss or crashes cannot corrupt pen slots or shortcut bindings.
5. **Single Static Artifact:** [`cmake/include/SwiftNoteRust.cmake`](../cmake/include/SwiftNoteRust.cmake) invokes Cargo during the CMake build and links `libswiftnote_rs.a` statically (with `ws2_32`, `userenv`, `bcrypt`, `ntdll` on Windows UCRT64 and `pthread`/`dl`/`m` on Linux), requiring zero extra runtime `.dll`/`.so` dependencies.

---

## 3. Non-Destructive Blackboard & Dual Export Pipeline

A classic pitfall in note-taking apps is mutating stroke RGB values when switching between white paper and dark chalkboard modes, which permanently corrupts shared `.xopp` files and ruins printed PDFs.

Swift note solves this strictly at the **display layer**:

```text
  Document Model (Immutable RGB)
         │
         ▼
  DisplayState::resolveInkColor(original)   ◄── Lock-free std::atomic<bool> hot path
         │                                      (0 mutexes, 0 FFI calls when inactive)
         ├── Blackboard OFF ──► Return original RGB untouched (0xFF alpha preserved)
         └── Blackboard ON  ──► Chroma = max(R,G,B) - min(R,G,B)
                                ├── Chroma ≤ 28 (black/gray/white) ──► v' = 255 - v
                                └── Chroma > 28 (blue/red/green/yellow) ──► Preserve hue
```

- **Zero Hot-Path Cost:** [`DisplayState`](../src/core/gui/swiftnote/SwiftBridge.h) stores Blackboard flags and surface color in `std::atomic` fields read directly by `StrokeView`, `TextView`, and `BackgroundView`.
- **Dual PDF Export via RAII:** [`PdfExportJob`](../src/core/control/jobs/PdfExportJob.cpp) instantiates `DisplayState::ScopedExportOverride(keepBlackboard, keepInkInvert)` for the duration of the export job. Users working in Blackboard mode can export a crisp **white-paper printable PDF** or a **Blackboard dark PDF** in one click, with previous display state automatically restored on scope exit (even on exception or cancellation).

---

## 4. Lossless Clipboard Image Pipeline

On Windows, pasting screenshots from Snipping Tool, browsers, or Office applications via `CF_DIB` / `GdkPixbuf` frequently produces 32-bit BGRA buffers where every pixel has `alpha == 0` while RGB channels hold valid pixel data — causing GTK/Cairo to render pasted images as completely invisible.

[`media.rs`](../rust/swiftnote-rs/src/media.rs) resolves this deterministically in [`ClipboardHandler.cpp`](../src/core/control/ClipboardHandler.cpp):

1. **PNG Path (`sn_normalize_clipboard_png`):** Decodes clipboard PNGs with `EXPAND | STRIP_16`, expands Grayscale/RGB/GrayscaleAlpha to canonical RGBA8, and re-encodes as lossless PNG preserving exact dimensions and alpha.
2. **Raw DIB / Pixbuf Path (`sn_pixels_to_lossless_png`):** Validates integer overflow on `width * 4` and `stride * height`, converts BGRA → RGBA when needed, and performs a single linear scan when `fix_all_zero_alpha = true`. **Only if 100% of pixels have `alpha == 0` AND at least one RGB channel is non-zero** does it repair alpha to `255` — guaranteeing genuine transparent graphics are never flattened.

---

## 5. Zero-Duplication 3-Layout GTK3 Shell

[`SwiftShell`](../src/core/gui/swiftnote/SwiftShell.cpp) mounts atop Xournal++'s `MainWindow` without duplicating tool state or breaking upstream actions:

- **Live Widget Reparenting:** The three layouts — `PureTopBar`, `CanvasPill`, and `ZenBottomDock` — reuse the exact same GTK widget subtrees and signal handlers, reparenting containers cleanly on layout switch without leaking callbacks or losing active tool highlights.
- **Legacy Chrome Suppression:** `SwiftShell::mount()` avoids `gtk_widget_show_all()` on the outer stack and runs `hideLegacyChrome()` to keep upstream `statusbar`, `searchBar`, and `searchToolbar` hidden (`gtk_widget_set_no_show_all(TRUE)`), dedicating maximum vertical space to the writing canvas.
- **Calibrated Solid-Surface CSS Token System:** [`ui/swiftnote.css`](../ui/swiftnote.css) defines clean Light (`#FFFFFF` / `#F5F6F8`) and Dark (`#16181D` / `#1E2028`) surface tokens with subtle 1px structural hairlines — ensuring consistent, artifact-free rendering across Windows and X11/Wayland compositors.
- **Vietnamese Diacritic-Insensitive Library:** [`library.rs`](../rust/swiftnote-rs/src/library.rs) traverses the workspace directory (`WalkDir`, max depth 6, hidden entries skipped), folds Vietnamese diacritics (`đ/Đ → d`, `ă/â/á/à/ả/ã/ạ → a`, etc.) for both query and filename/folder haystacks, and performs a bounded streaming gzip peek to count `<page` tags and extract embedded base64 `<preview>` thumbnails without loading full documents into RAM.

---

## 6. Reliability & Input Safety Hardening

During live desktop testing across Windows 10/11 and Linux Xvfb, several subtle GTK3/Cairo failure modes were identified and permanently fixed:

| Failure Mode | Root Cause | Engineered Fix |
| :--- | :--- | :--- |
| **Solid gray page corruption (`#DCDAD5`)** | An invalid/zero-area Cairo `Mask` left a poisoned `cairo_status_t` that propagated to subsequent `PageView` draws | Hardened [`Mask::isInitialized()`](../src/core/view/Mask.cpp) to validate both `cairo_status()` and `cairo_surface_status()`; wrapped overlay rendering in `CairoSaveGuard` in [`PageView.cpp`](../src/core/gui/PageView.cpp) |
| **Invisible shapes / black-zero alpha** | `Color` values constructed from 24-bit `0xRRGGBB` left the alpha byte at `0x00`, turning `cairo_set_source_argb` fully transparent | Normalized all `SwiftBridge`/`SwiftShell` `Color` conversions to force `0xFF` alpha (`0xFF000000U \| rgb`) |
| **Stuck input after closing tool popovers** | Destroying a modal `GtkPopover` synchronously inside its own button `clicked` handler triggered re-entrant teardown and left a dangling grab | Switched to `gtk_popover_popdown()` + one-shot `g_idle_add()` destruction + explicit `XournalView::requestFocus()` |
| **Setsquare / Compass not rendering immediately** | Toggling geometry overlays from custom toolbar buttons did not force a canvas redraw or track live active state | Added `Control::hasSetsquare()` / `Control::hasCompass()` + immediate `gtk_widget_queue_draw()`, routed cleanly through `Action::SETSQUARE` / `Action::COMPASS` |
| **Text Box losing focus / shortcut clashes** | Single-key shortcuts intercepted typing when `TextEditor` or a popover was active | Added `requestFocus()` in `XojPageView::startText()` and guarded `SwiftShell::onKeyPress()` whenever a `TextEditor` or popover holds focus |

---

## 7. Automated Verification Pyramid

Every commit is gated by three layers of automated tests in [`.github/workflows/ci-swiftnote.yml`](../.github/workflows/ci-swiftnote.yml):

1. **Rust Unit & Lint Layer (`cargo test` + `clippy -D warnings`):** Validates config round-trips, atomic writes, chroma-preserving Blackboard inversion, `CF_DIB` zero-alpha repair, Vietnamese diacritic folding, and `.xopp` preview extraction.
2. **C++ Core Unit Layer (`test-units`, 119 cases):** Verifies Xournal++ document serialization, undo/redo stacks, geometry math, stroke splitting, and PDF metadata parsing.
3. **End-to-End GTK3 Regression Layer ([`SwiftShellRegressionTest.cpp`](../test/gtk_tests/SwiftShellRegressionTest.cpp)):** Boots a real `MainWindow` + `SwiftShell` (natively on Windows and under `xvfb-run` on Ubuntu 24.04) and asserts:
   - Legacy `statusbar` and `searchBar` remain hidden.
   - Pen slot colors preserve `alpha == 255`.
   - Setsquare and Compass overlays activate, render without Cairo error states, and deactivate cleanly.
   - Text Box creation initializes cleanly and returns focus without stuck modal grabs.
