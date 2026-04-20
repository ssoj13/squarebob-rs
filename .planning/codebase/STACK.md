# Technology Stack

**Analysis Date:** 2026-04-20

## Languages

**Primary:**
- **Rust** (edition 2021) — Entire application, libraries, and GPU/shader crates.

**Secondary:**
- **WGSL** — GPU compute/render pipelines in `crates/bvh-gpu`, `pt-*`, `render-3d`, `treemap` (where applicable).
- **Shell / Markdown** — Tooling and docs only.

## Runtime

**Environment:**
- **Native desktop binary** — No separate server runtime. GUI via **winit** (pulled in by `eframe`).
- **GPU:** **wgpu** 29 + **Vulkan/Metal/DX12** via backends (depends on OS/drivers).

**Package Manager:**
- **Cargo** (Rust toolchain) — Workspace root `Cargo.toml`; lockfile: `Cargo.lock` (tracked).

## Frameworks

**Core:**
- **eframe** 0.34 — Windowing, event loop, persistence, `Wgpu` rendering hook for custom 3D/treemap.
- **egui** 0.34 — Immediate-mode UI; **egui_dock** 0.19 — Dockable panels (serde feature).
- **egui-wgpu** 0.34 — Bridge between egui textures and wgpu.

**GPU / rendering:**
- **wgpu** 29 — Device/queue, pipelines, render passes for 2D treemap GPU path and full **render-3d** stack.
- **glam** — Math types in app/renderer integration.
- **bytemuck** — Pod casting for GPU buffers.
- **image** — PNG screenshots and texture I/O.

**Concurrency / I/O:**
- **rayon** / **jwalk** — Parallel directory scanning.
- **crossbeam-channel** — Scan progress and results to UI thread.
- **pollster** — Block on async wgpu operations where used (e.g. error scopes).

**Scan & platform:**
- **sysinfo** — Host memory stats for status bar.
- **directories** + **sha2** + **bincode** 1 — Cache and exclusion paths under user data dirs (`src/cache.rs`, `src/exclusions.rs`, `src/path_key.rs`).
- **rfd** — Native folder picker.
- **open**, **trash** — Open in Explorer / move to trash (`src/app/shell.rs`).
- **windows** 0.62 (Windows only) — NTFS fast path (`src/scanner_ntfs.rs`).

**Logging:**
- **log** + **env_logger** — CLI-controlled verbosity (`src/main.rs`).

## Key Dependencies

**Critical:**
- **dirstat-core** — `DirEntry` tree, serde, layout inputs (`crates/dirstat-core/`).
- **treemap** — Squarified layout + optional wgpu rendering (`crates/treemap/`).
- **render-3d** / **render-core** / **render-shared** — 3D scene, materials, integration with egui texture (`crates/render-3d/`, etc.).
- **pt-core**, **bvh-gpu**, **pt-megakernel**, **pt-wavefront**, **pt-mats** — Path tracing and BVH GPU pipeline (3D advanced mode).
- **auto-allocator** (`version = "*"`) — Global allocator override (review pinning for reproducibility).

**Infrastructure:**
- **serde** / **serde_json** — Presets and `eframe` persistence.

## Configuration

**Environment:**
- **`RUST_LOG`** — Log filtering when using `env_logger` patterns (`src/main.rs`).
- No mandatory `.env` for core app; CLI flags dominate.

**Build:**
- Workspace `Cargo.toml` — Members listed explicitly; `[profile.release]` `opt-level = 3`.
- Feature flags on `treemap` (`wgpu`) and `eframe` (e.g. `wgpu`, `persistence`, `default_fonts`, Linux backends).

## Platform Requirements

**Development:**
- **Rust stable** (2021 edition).
- **Windows:** Full feature set including optional NTFS MFT scanner (may require elevated rights for best results).
- **Linux:** `eframe` features include `x11`, `wayland`.

**Production:**
- Shipped as a **single executable** per target (`cargo build --release`); GPU and OS deps as per wgpu requirements.

---

*Stack analysis: 2026-04-20*  
*Update after major dependency changes*
