## Summary
<!-- Briefly describe what this PR changes and why. -->

## Architectural Impact
- [ ] C++20 / GTK3 Core (`src/core/`)
- [ ] Rust Static Library (`rust/swiftnote-rs/`)
- [ ] UI Stylesheet / Assets (`ui/`, `assets/`)
- [ ] CI / Packaging (`.github/`, `windows-setup/`)

## Verification Checklist
- [ ] `cargo test --manifest-path rust/swiftnote-rs/Cargo.toml` passes
- [ ] `cargo clippy --manifest-path rust/swiftnote-rs/Cargo.toml --all-targets -- -D warnings` passes with 0 warnings
- [ ] `test-units` passes cleanly
- [ ] `SwiftShellRegressionTest` passes (no legacy statusbar/searchBar regressions, opaque `0xFF` color alpha, Setsquare/Compass & Text Box verified)
- [ ] UI changes strictly preserve **100% solid opaque surfaces (Zero Glassmorphism)** and canvas-first proportions
