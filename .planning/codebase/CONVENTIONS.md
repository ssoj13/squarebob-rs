# Coding Conventions

**Analysis Date:** 2026-04-20

## Style & Tooling

- **fmt** — Rust standard (`rustfmt`); 2021 edition.
- **Lint** — `cargo clippy --workspace --all-targets` used in development; fix warnings for deprecations when upgrading egui/wgpu.
- **Logging** — `log` crate with `info!`, `warn!`, `error!`; verbosity from CLI/env (`src/main.rs`).

## Error Handling

- **`anyhow::Result`** — Common in scanners and file I/O where context chains help.
- **Fallible paths** — Cache load returns `Option` or soft-failure to force rescan rather than crash.
- **GPU paths** — `render-3d` uses many **`as_ref().unwrap()`** on paths assumed initialized after setup (see CONCERNS.md).

## Platform Conditionals

- **`#[cfg(windows)]`** — NTFS-only code (`src/scanner_ntfs.rs`), Windows shell helpers, optional UI arms in `src/app/mod.rs`.
- **`#[cfg(unix)]`** — Unix-only deps (`libc` in root `Cargo.toml`).

## Patterns

- **Immediate-mode GUI** — Per-frame `run_frame`; minimal retained state beyond `App` fields.
- **Channels** — Scan results and progress cross threads without async runtime in the hot path.
- **Serde** — `DirEntry`, dock state, presets, persistence JSON where applicable.
- **Documentation** — README lists features; inline comments for safety (`unsafe` around short-lived pointers in render paths).

## Unsafe

- **Small, localized** uses where tree pointers are passed into render callbacks with documented lifetime contracts (`src/app/mod.rs` capture / 3D render paths). Review changes carefully.

---

*Conventions analysis: 2026-04-20*
