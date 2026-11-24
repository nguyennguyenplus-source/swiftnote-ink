# Changelog

All notable changes to **Swift Note** will be documented in this file.

## [0.1.0] - Milestone 1

### Added
- **Core Notebook Engine (`swiftnote-core`):**
  - Hierarchical notebook model (`Document`, `Page`, `Stroke`, `ImagePlacement`, `PdfAssetMeta`) with R-tree spatial indexing and atomic `.swnote` ZIP persistence + local recent-notes library.
  - Cusp-preserving velocity-adaptive stroke stabilizer and out-of-range/NaN input guard.
  - Exact identical wet/dry `StrokeTessellator` (`ContourMesh`) and stroke/segment eraser with z-order-preserving undo/redo history.
  - Procedural paper rulings (`Blank`, `Graph`, `Isometric`, `Ruled 10/15/20/30/40`, `Math 10/15/20/30/40`, `Dotted 10/20`) across Light Paper and Blackboard polarities with optional `auto_invert_monochrome_ink` toggle.
- **Unified GPU Canvas (`swiftnote-gpu`):**
  - `wgpu` renderer (DX12 on Windows, Vulkan/GL on Linux) with 4x MSAA stroke rendering, 512x512 LRU page tile cache, multiply highlighter blending, and GPU mipmap chain generation.
  - Asynchronous `PdfWorker` (`pdfium-render`) with generation cancellation, password prompt support, and 2048 px tile chunking for large PDFs.
- **Minimalist Desktop App (`swiftnote-app`):**
  - SDL3 `CategoryPen` stylus isolation (palm/touch rejection while pen in proximity, hardware eraser & barrel button support), trackpad/touch pan & pinch-zoom, IME plumbing, and OS clipboard RGBA/DIB sanitization.
  - Solid-matte Light and Dark UI themes (`egui` + `egui-wgpu`) with zero blur or translucent chrome, monochrome vector SVG icons, single-row top bar, Notes Gallery sidebar, Pages sidebar, contextual popovers, status pill, and event-driven wake loop.
- **Tooling & CI:**
  - Dual-OS GitHub Actions CI (`windows-latest` and `ubuntu-24.04`) and `scripts/fetch-pdfium.{ps1,sh}` helper scripts.
