# AGENTS.md — dirstat-rs agent context

Audience: humans and tooling resuming after context compaction.

## Crate layout

- **Binary** (`src/`): egui shell, scanners, filters, renderer glue, CLI.
- **dirstat-core**: canonical tree model (`DirEntry`, sizes, rects, serde, LoD hints).
- **treemap**: 2D layout + CPU/GPU (wgpu) treemap rasterization.
- **render-core**, **render-shared**, **render-3d**: GPU context and 3D path tracing stacks.
- **pt-***, **bvh-gpu**: path tracer pipelines (megakernel, wavefront, materials, etc.).

## ASCII dataflow — startup

```
main.rs
  ├─ parse_args() → CliOptions (optional overrides)
  ├─ env_logger filters (dirstat_rs, optional pt/wf/pg)
  └─ eframe::run_native → app::App::new(cc, cli)
           ├─ load PersistState from egui storage
           ├─ reconcile scan_path existence
           ├─ apply CLI mode/backend/render knobs
           ├─ register wgpu uncaptured-error hook (when eframe exposes device)
           └─ GpuContext::from_eframe(...) if POLYGON_MODE_LINE supported
```

## ASCII codepath — scan

```
start_scan(app/mod.rs)
  ├─ exclusions::load(scan_path)
  ├─ cache hit? → restore tree, rebuild display, optionally skip thread scan
  └─ miss → crossbeam channel + background thread:

      ScannerMode::Standard ──► scanner::scan_bg → jwalk walk → DirEntry tree → Done

      ScannerMode::Ntfs (Windows)
          ├─ is_ntfs_available? no  → scanner::scan_bg (jwalk)
          └─ yes → scanner_ntfs::scan_ntfs_bg
                     ├─ OK → Done
                     └─ Err → ScanMsg::NtfsFallback(reason)
                               → scanner::scan_dir_public (same thread)
                               → Done or Error
```

UI note: handling `NtfsFallback` sets `scanner_mode = Standard` (persists via normal save paths). Prefer explicit user confirmation if preserving **NTFS** preference matters.

## ASCII codepath — display / render tick

```
update()
  └─ poll_scan() → rebuild display tree when Done
  └─ handle_events() → Navigate/Select/LayoutDirty/RenderTick3D
  └─ rebuild_display_tree() when filters/size/exclusions change

render_treemap(app/mod.rs)
  ├─ ensure GpuContext when 3D or 2D-GPU
  ├─ Mode2D + Cpu → renderer::cpu::render
  ├─ Mode2D + Gpu → treemap::GpuRenderer2D (readback RGBA)
  └─ Mode3D → Renderer3D::render (readback RGBA); fallback CPU treemap if no GPU
       └─ ColorImage → ctx.load_texture (full upload each frame — known perf trade-off)
```

## Single sources of truth (SSOT)

| Concern              | Canonical location                                      |
|----------------------|---------------------------------------------------------|
| Tree node shape      | `crates/dirstat-core/src/lib.rs` (`DirEntry`)           |
| 3D options / camera  | `render_shared::Render3DOptions` (+ `App` persisted state slices) |
| Scan progress        | `app::state::ScanProgress`                              |
| Path-derived cache keys | `src/path_key.rs` (`scan_path_id_hex`)               |
| Ignore rules on disk | `src/exclusions.rs` + `.dirstat-exclusions.json`        |

## Open engineering items (tracked in code)

- **Zero-copy / shared device**: `src/app/mod.rs` ~L1035–L1074 — intentional CPU readback; eframe integration partial in `App::new` (~L427–437) vs legacy `GpuContext::new()` fallback in `render_treemap` (~L992–993).
- **Wide `#![allow(dead_code)]`** on several PT pipeline modules — indicates generated/experimental WGSL pathways; prune only after proving unused at link time across features.

## Maintenance commands

```text
cargo check --workspace
cargo clippy --workspace --all-targets
```

Last bughunt pass: cleared workspace clippy warnings (treemap GPU buffer reuse probe, redundant size calc in render-3d, UI clamp literals, serde test initializer).
