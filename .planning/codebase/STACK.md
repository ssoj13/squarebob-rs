# Technology Stack

**Analysis Date:** 2026-05-09

## Languages

**Primary:**
- Rust 1.95.0 (edition 2021) - All workspace crates and the root binary

**Secondary:**
- WGSL (WebGPU Shading Language) - GPU shaders for `wgpu`-based renderers (megakernel/wavefront path tracers, treemap GPU renderer). Shipped as `.wgsl` source loaded at runtime by the `render-3d`, `pt-megakernel`, `pt-wavefront`, and `bvh-gpu` crates.

## Runtime

**Environment:**
- Native desktop binary built via `cargo` for `x86_64-pc-windows-msvc` (primary), with conditional Unix support via `cfg(unix)` deps
- Rust toolchain pinned via `rust-toolchain.toml` to channel `1.95.0`
- `auto-allocator` v* with `secure` feature swaps in a high-performance allocator at startup (configured in root `Cargo.toml`)

**Package Manager:**
- `cargo` (bundled with toolchain 1.95.0)
- Lockfile: present at `Cargo.lock` (root, for the workspace)

## Frameworks

**Core (UI / Application Shell):**
- `eframe` 0.34 - egui application framework. Configured with `default-features = false` and explicit features `default_fonts`, `persistence`, `wgpu`, `x11`, `wayland` (root `Cargo.toml:24`).
- `egui` 0.34 - Immediate-mode GUI library used by `src/app/**`
- `egui-wgpu` 0.34 - wgpu render backend for egui
- `egui_dock` 0.19 (with `serde` feature) - Docking/tab layout used by `src/app/dock.rs`

**GPU / Graphics:**
- `wgpu` 29 - Cross-backend GPU API (Vulkan/DX12/Metal). Used by `render-core`, `render-3d`, `treemap` (optional), `bvh-gpu`, `pt-megakernel`, `pt-wavefront`.
- `pollster` 0.4 - Blocks on `wgpu` async calls (in `render-core`)
- `glam` 0.32 - SIMD math (vectors, matrices, quaternions) shared across `pt-core`, `render-3d`, `render-shared`, `pt-megakernel`
- `bytemuck` 1 (`derive` feature) - Zero-copy POD casts for GPU buffer uploads, used in every GPU-touching crate
- `half` 2.7.1 - `f16` support for HDR image / GPU texture interop (root binary, `render-3d`)

**Filesystem Scanning / Concurrency:**
- `jwalk` 0.8 - Parallel directory traversal (root binary scanner)
- `rayon` 1.12 - Data-parallel iterators (root binary, `treemap`)
- `crossbeam-channel` 0.5 - MPMC channels for scanner-to-UI events (`src/events.rs`, `src/scanner.rs`)
- `num_cpus` 1 - Worker count auto-tuning

**Testing:**
- Standard `cargo test` only. No `criterion`, `proptest`, `mockall`, or other test framework declared in any `Cargo.toml` across the workspace.

**Build / Dev:**
- `cargo` workspace build only. No `build.rs` referenced from manifests; no `xtask` crate; no procedural macro crates inside the workspace.

## Key Dependencies

**Critical (root binary `dirstat-rs`):**
- `eframe` 0.34 - App entry point and window/event loop
- `wgpu` 29 - GPU backend driving renderer crates and egui-wgpu
- `jwalk` 0.8 - Parallel filesystem scanner core
- `rayon` 1.12 - Parallel post-processing of scan results
- `glam` 0.32 - 3D math throughout renderer + treemap layout
- `serde` 1 (`derive`) + `serde_json` 1 + `bincode` 1 - Settings/preset (de)serialization (JSON for human-edited, bincode for cache)
- `sha2` 0.11 - Path hashing for cache keys (`src/path_key.rs`)
- `directories` 6 - OS-appropriate config/cache paths (`src/app/presets.rs`)
- `auto-allocator` (`secure` feature) - Allocator selection
- `sysinfo` 0.38 - System / process introspection (memory, CPU)

**User-facing OS Integrations:**
- `rfd` 0.17 - Native file/folder open dialogs
- `trash` 5 - Move-to-trash for delete operations
- `open` 5 - "Reveal in file manager" / launch with default app
- `image` 0.25 - PNG/JPEG/EXR image I/O for screenshots and texture loading

**Logging / Errors:**
- `log` 0.4 - Logging facade (used by every crate that logs)
- `env_logger` 0.11 - Logger implementation (root binary only)
- `anyhow` 1 - Error wrapping in `render-3d` and root binary

## Workspace Members

Defined in root `Cargo.toml:7-19`:

| Crate | Path | Role |
|-------|------|------|
| `dirstat-core` | `crates/dirstat-core/` | Pure data model for directory entries (serde only, zero deps beyond serde) |
| `treemap` | `crates/treemap/` | Treemap layout algorithms; CPU default, optional `wgpu`/`cuda` features. Pulls `dirstat-core` + `rayon`. |
| `render-core` | `crates/render-core/` | Thin wgpu device/queue setup utilities (`wgpu` + `pollster`) |
| `render-shared` | `crates/render-shared/` | Shared GPU-facing types (vertex layouts, material handles); depends on `pt-mats` |
| `render-3d` | `crates/render-3d/` | High-level 3D scene renderer; aggregates `render-core`, `render-shared`, `pt-core`, `pt-megakernel`, `pt-mats`, `treemap` |
| `pt-core` | `crates/pt-core/` | Path-tracer primitives (rays, hit records); depends only on `glam` + `bytemuck` |
| `pt-mats` | `crates/pt-mats/` | Material definitions (serde-serializable); depends on `pt-core` |
| `bvh-gpu` | `crates/bvh-gpu/` | GPU-resident BVH builder/traverser using `wgpu` + `pt-core` |
| `pt-megakernel` | `crates/pt-megakernel/` | Single-kernel GPU path tracer (depends on `pt-core`, `bvh-gpu`, `pt-wavefront`) |
| `pt-wavefront` | `crates/pt-wavefront/` | Multi-pass "wavefront" GPU path tracer (`wgpu` + `bytemuck`) |

The root crate `dirstat-rs` (binary at `src/main.rs`) consumes all of the above via path dependencies.

## Configuration

**Toolchain:**
- `rust-toolchain.toml:1-2` pins channel `1.95.0`. No `components` or `targets` declared, so rustup installs the default profile.

**Application configuration:**
- Runtime settings persisted via `eframe`'s `persistence` feature (egui memory store) and via `directories::ProjectDirs` in `src/app/presets.rs` for user presets.
- Cache files keyed by SHA-256 of canonical path (`src/path_key.rs`, `src/cache.rs`).

**Environment variables:**
- `RUST_LOG` consumed by `env_logger`
- No `.env` files are present in the repo; no other env-driven configuration.

**Build:**
- Root `Cargo.toml:73-74` enables `[profile.release]` with `opt-level = 3`. No custom `dev`, `bench`, or `test` profiles. No workspace-level lint table.

## Platform Requirements

**Development:**
- Rust 1.95 (auto-installed by `rust-toolchain.toml`)
- A wgpu-compatible GPU + driver (Vulkan on Linux, DX12/Vulkan on Windows, Metal on macOS)
- Windows MSVC toolchain when building on Windows (for `windows` crate FFI)

**Production / Target-specific dependencies:**
- `cfg(windows)` (root `Cargo.toml:64-71`): `windows` 0.62 with features `Win32_Storage_FileSystem`, `Win32_System_IO`, `Win32_System_Ioctl`, `Win32_Foundation`, `Win32_Security` - powers NTFS USN journal scanning in `src/scanner_ntfs.rs`.
- `cfg(unix)` (root `Cargo.toml:61-62`): `libc` 0.2 - used for POSIX file metadata calls.
- Linux runtime requires X11 or Wayland (eframe features `x11`, `wayland` both enabled).

---

*Stack analysis: 2026-05-09*
