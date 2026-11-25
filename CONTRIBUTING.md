# Contributing to Swift note

Thank you for your interest in contributing to **Swift note**! **Swift note** is a GPL-2.0 hybrid **C++20/GTK3 + Rust** fork of [Xournal++](https://github.com/xournalpp/xournalpp).

---

## Development Principles

1. **Canvas-First UI Clarity:** Keep chrome, popovers, panels, and docks minimal, high-contrast, and cleanly styled using the Light/Dark theme tokens in [`ui/swiftnote.css`](ui/swiftnote.css) without cluttering the writing area.
2. **Panic-Safe FFI Boundary:** Every `extern "C"` function in [`rust/swiftnote-rs/src/lib.rs`](rust/swiftnote-rs/src/lib.rs) must be wrapped in `catch_unwind`, validate null pointers/UTF-8, and pair any heap allocation with an explicit Rust deallocator wrapped in RAII inside [`src/core/gui/swiftnote/SwiftBridge.cpp`](src/core/gui/swiftnote/SwiftBridge.cpp).
3. **Non-Destructive Document Model:** Display-layer features (such as Blackboard mode) must never mutate underlying `.xopp` stroke RGB values. Always preserve **100% compatibility** with standard `.xopp` files.
4. **Canvas-First Input Safety:** Never destroy a modal `GtkPopover` synchronously inside its own click handler; always pop down first, schedule deferred cleanup via `g_idle_add`, and restore focus to `XournalView`.

---

## Setting Up & Building

See the [Installation & Building from Source](README.md#installation--building-from-source) section in `README.md` for Windows (MSYS2 UCRT64) and Linux (Ubuntu/Debian) instructions.

---

## Mandatory Verification Before Opening a Pull Request

Every PR must pass the exact checks enforced by [`.github/workflows/ci-swiftnote.yml`](.github/workflows/ci-swiftnote.yml):

```bash
# 1. Rust unit tests and strict Clippy lint (zero warnings)
cargo test --manifest-path rust/swiftnote-rs/Cargo.toml
cargo clippy --manifest-path rust/swiftnote-rs/Cargo.toml --all-targets -- -D warnings

# 2. C++ unit tests
./build/test/test-units --gtest_brief=1

# 3. GTK3 SwiftShell regression test (use xvfb-run -a on headless Linux)
./build/test/test-gtk-integration --gtest_filter=SwiftShellRegressionTest.*
```

---

## Code Style & Commit Conventions

- **C++20:** Format modified files using the repository's `.clang-format` rules.
- **Rust:** Format with `cargo fmt` and satisfy `cargo clippy --all-targets -- -D warnings`.
- **Commits:** Use clear [Conventional Commits](https://www.conventionalcommits.org/) (`feat(shell): ...`, `fix(cairo): ...`, `docs(roadmap): ...`, `ci: ...`).
- **Pull Requests:** Fill out [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md) with verification output and screenshots for UI changes.

---

## Upstream Attribution

Improvements that apply cleanly to upstream Xournal++ (core rendering bugs, PDF export fixes, Poppler/QPDF issues) are encouraged to be contributed back to [xournalpp/xournalpp](https://github.com/xournalpp/xournalpp) as well.
