# AGENTS.md

## Bug-Hunt Operating Notes

This repository is a Rust workspace for a desktop disk-usage visualizer with CPU and GPU rendering paths. The current bug-hunt pass was run on 2026-05-16 from the repository root on branch `main`.

Primary constraints for future agents:

- Keep scan data ownership on the main UI side. `DirEntry.rect` uses `Cell`, so the owned tree is intentionally passed through channels and caches rather than shared by `Arc`.
- Treat `render_core::gpu::GpuContext::new` as the single source of truth for wgpu device setup. `main.rs` passes that same instance/device/queue to eframe and app renderers.
- Prefer central readback helpers over ad hoc `map_async` blocks. `wgpu::BufferSlice::map_async` returns a callback `Result`; never unwrap channel receive or map errors in UI render paths.
- Keep 2D/3D zero-copy paths on the eframe-backed device. CPU readback paths are legacy/fallback paths and must return recoverable errors instead of panicking.
- Do not remove `#[allow(dead_code)]` items without tracing intended feature toggles and platform/API parity stubs.

## Inventory Snapshot

- Root package: `squarebob-rs` binary `squarebob`.
- Workspace members: `squarebob-core`, `pt-core`, `bvh-gpu`, `pt-megakernel`, `pt-wavefront`, `pt-mats`, `render-core`, `render-shared`, `render-3d`, `media-encoder`, `xtask`, `treemap`, `pt-denoise-oidn`, `gpu-mem`.
- Files scanned excluding `target/**` and `.git/**`: 223.
- Existing historical bug-hunt artifact: `.bughunt/plan1.md`.
- This pass created `.bughunt/plan2.md` and `BUG_HUNT_REPORT.md`.

## High-Level Dataflow

```text
CLI args
  |
  v
main.rs
  |-- parse CLI / test mode
  |-- create shared render_core::gpu::GpuContext
  |-- pass WgpuSetup::Existing to eframe
  v
App::new
  |
  v
App::start_scan
  |-- load cache if available
  |-- choose scanner: jwalk or NTFS MFT on Windows
  |-- spawn background scanner
  v
ScanMsg channel
  |-- Progress -> App::poll_scan updates UI counters
  |-- Done(DirEntry) -> cache serialize + display tree rebuild
  |-- Error/NtfsFallback -> UI progress state
  v
App::ui_treemap
  |-- Mode2D CPU -> treemap::render -> egui texture
  |-- Mode2D GPU -> treemap::GpuRenderer2D -> eframe texture
  |-- Mode3D raster/PT -> render_3d::Renderer3D -> eframe texture or CPU readback
```

## GPU Readback Codepath

```text
2D legacy render
  crates/treemap/src/wgpu.rs:680 -> render_core::gpu::readback_texture
  crates/treemap/src/wgpu.rs:688 -> render_core::gpu::map_readback

3D raster legacy render
  crates/render-3d/src/lib.rs:1331 -> render_core::gpu::readback_texture
  crates/render-3d/src/lib.rs:1348 -> render_core::gpu::map_readback

PT megakernel readback render
  crates/render-3d/src/pt/megakernel/render.rs:465 -> render_core::gpu::readback_texture
  crates/render-3d/src/pt/megakernel/render.rs:481 -> render_core::gpu::map_readback

Shared failure point
  crates/render-core/src/lib.rs:227 -> BufferSlice::map_async callback
  crates/render-core/src/lib.rs:228 -> tx.send(result).unwrap()
  crates/render-core/src/lib.rs:232 -> rx.recv().unwrap().unwrap()
```

## Scan / Cache Codepath

```text
App::start_scan
  |
  |-- cache::load_cache(scan_path)
  |     |-- cache_path(scan_path)
  |     |-- bincode::deserialize_from
  |     `-- cached DirEntry tree returned to App
  |
  `-- scanner::scan_bg or scanner_ntfs::scan_ntfs_bg
        |
        |-- jwalk WalkDir / NTFS MFT enumeration
        |-- DirEntry::new_file / DirEntry::new_dir
        |-- sort_by_size
        `-- tx.send(ScanMsg::Done(tree))

App::poll_scan
  |
  |-- compute_ext_stats / compute_size_range
  |-- cache::serialize_cache
  |-- cache::write_cache_bytes on background thread
  `-- rebuild_display_tree + needs_layout
```

## Rendering Codepath

```text
App::ui_treemap
  |
  |-- callback path when wgpu_render_state and gpu_context exist
  |     |-- Mode2D + GPU: render_2d_callback
  |     |     |-- GpuRenderer2D::render_to_texture
  |     |     `-- egui_wgpu texture registration/update
  |     |
  |     `-- Mode3D: render_3d_callback
  |           |-- Renderer3D::render_to_view
  |           |-- object-id picking readback
  |           `-- egui_wgpu texture registration/update
  |
  `-- legacy path
        |-- render_treemap / Renderer3D::render
        `-- CPU pixel Vec uploaded to egui texture
```

## Current Bug-Hunt Focus Areas

- Shared GPU readback helper still panics on callback/map failures.
- Readback size arithmetic is still done in `u32` before casting to `u64` / `usize`.
- Some unsafe blocks lack `// SAFETY:` comments.
- Megakernel path-tracer initialization is duplicated between readback and no-readback render paths.
- A few `expect` invariants remain after `.bughunt/plan1.md` fixes; most are mechanically safe but should be centralized to keep the panic surface small.
