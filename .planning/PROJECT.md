# dirstat-rs

## What This Is

Desktop disk-usage visualizer (Rust): squarified treemap, 2D/3D + path tracing (wgpu), parallel scan, NTFS MFT on Windows, scan cache.

## Core Value

Fast, accurate visualization of **where space goes** on huge trees (millions of files) without dropping features.

## Context

- Toolchain: Rust 1.95+ (`rust-toolchain.toml`).
- LoD по размеру (merge вне [Min, Max], раскрытие по double-click) — см. `.planning/phases/01-lod.md`.

## Constraints

- **Tech**: egui, wgpu, workspace crates (`treemap`, `render-3d`, `pt-*`).
- **Compatibility**: сохранять существующие режимы рендера и UX.

---
*Last updated: 2026-04-19 — planning bootstrap for GSD*
