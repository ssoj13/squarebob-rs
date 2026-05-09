# Architecture

**Analysis Date:** 2026-05-09

## Pattern Overview

**Overall:** Cargo workspace with binary-as-shell + layered library crates spanning two domains:
1. **dirstat domain** — filesystem scanner, tree model, treemap layout/render
2. **render/path-tracer domain** — wgpu GPU 3D renderer with megakernel and wavefront PT backends, GPU LBVH

The single binary `dirstat-rs` (defined by the root `Cargo.toml` package + `src/main.rs`) bundles both domains into one egui/eframe desktop app. All heavy lifting lives in 10 workspace member crates under `crates/` — the binary is a thin shell over them.

**Key Characteristics:**
- **Layered crate dependency graph** — strictly bottom-up: domain cores (`dirstat-core`, `pt-core`, `render-core`, `render-shared`) → GPU subsystems (`bvh-gpu`, `pt-mats`, `pt-megakernel`, `pt-wavefront`) → integrated renderer (`render-3d`) + visualization (`treemap`) → binary (`src/`)
- **Single canonical tree model** (`dirstat_core::DirEntry`) shared by scanner, treemap, picking, and PT geometry
- **Two PT backends behind one renderer** — `render-3d` selects megakernel vs wavefront via `Render3DOptions.pt_wavefront` flag; both share `pt-core` BVH/instance data
- **Background scan + main-thread render** — scanners send `ScanMsg` over a `crossbeam-channel` to the egui update loop
- **GPU compute everywhere on the PT side** — BVH build (LBVH on GPU), path tracing (compute shaders), accumulation, ReSTIR, path guiding (SVO) — all in WGSL
- **CPU readback bridge to egui** — current pipeline renders 3D/PT to a wgpu texture, reads back to RGBA, uploads as `egui::ColorImage` each frame (a known perf trade-off, documented in `AGENTS.md`)

## Layers

**Layer 1 — Domain cores (zero GPU, zero UI):**

- **`dirstat-core`** (`crates/dirstat-core/src/lib.rs`):
  - Purpose: canonical tree node model
  - Exports: `DirEntry { name, path, size, own_size, children, is_dir, ext, file_count, dir_count, modified_time, rect: Cell<[f32;4]>, lod_expand }`, `LodExpandInfo`, `LodKind`
  - Depends on: `serde`
  - Used by: every other crate that touches a directory tree

- **`pt-core`** (`crates/pt-core/src/lib.rs`):
  - Purpose: PT scene representation, CPU SAH BVH builder, GPU buffer layouts shared by both PT backends
  - Modules: `build` (instance BVH), `bvh` (`BvhNode`, `GpuAabb`, `GpuMaterial`, `Instance`), `gpu_data` (node + instance GPU layouts)
  - Used by: `pt-megakernel`, `pt-wavefront`, `render-3d`

- **`render-core`** (`crates/render-core/src/lib.rs`):
  - Purpose: minimal wgpu glue — `GpuContext { device, queue }`, `Viewport` (pan/zoom), `from_eframe` constructor
  - Used by: `render-3d`, `treemap` (wgpu feature), binary

- **`render-shared`** (`crates/render-shared/src/lib.rs`):
  - Purpose: cross-cutting render types/uniforms — `RenderBackend`, `RenderMode`, `Render3DOptions`, `OrbitCamera`, `CameraUniform`, `EnvParamsUniform`, `LightRigUniform`, `HoverParamsUniform`, `CubeHeightMode`, `ColorMode`, `HoverMode`, `HashTransformEffect`, `SpectralMode`, `hash_transform`, `name_hash`
  - Used by: `render-3d`, binary `src/renderer.rs`

**Layer 2 — GPU subsystems:**

- **`bvh-gpu`** (`crates/bvh-gpu/src/lib.rs` → `bvh_gpu` module):
  - Purpose: LBVH build on GPU — Morton codes, GPU radix sort, Karras-style topology, AABB reduction
  - WGSL: `morton.wgsl`, `radix_sort.wgsl`, `lbvh_build.wgsl`, `aabb_compute.wgsl`
  - Exports: `GpuBvhBuilder`, `GpuBvhConfig`
  - Used by: `pt-core`, `pt-megakernel`, `render-3d`

- **`pt-mats`** (`crates/pt-mats/src/lib.rs`):
  - Purpose: deterministic material classifier — `MaterialClass`, `MaterialLibrary`, `MaterializeMode` (None / ByExtension / ByPath / BySize / ByAge / Random), `MaterialSource`, `MaterialDistribution`, `classify_path_filtered`
  - Used by: `render-3d` (PBR + PT classification), `pt-megakernel` / `pt-wavefront` shaders consume `GpuMaterial`

- **`pt-megakernel`** (`crates/pt-megakernel/src/lib.rs`):
  - Purpose: monolithic PT compute pipeline (single megakernel shader)
  - Modules: `compute` (`PathTraceCompute`, `PtCameraUniform`), `adaptive` (variance-based SPP), `pathguide` (SVO + sample/update WGSL), `restir` (DI/GI reservoirs, temporal/spatial WGSL)
  - WGSL passes: `blit.wgsl`, `pick.wgsl`, `pathguide/{sample,update}.wgsl`, `restir/{temporal,spatial}.wgsl`, `wavefront/gbuffer.wgsl`
  - Hot-path file: `crates/pt-megakernel/src/compute.rs`
  - Used by: `render-3d`

- **`pt-wavefront`** (`crates/pt-wavefront/src/lib.rs`):
  - Purpose: staged wavefront PT — separates raygen, intersect, shade, finalize into discrete dispatches; better divergence handling and tile scheduling
  - Exports: `WavefrontConfig`, `WavefrontPipeline`, `WfDims`, `WfHit`, `WfRay`
  - WGSL stages: `raygen.wgsl`, `intersect.wgsl`, `shade.wgsl`, `finalize.wgsl`, `count_swap.wgsl`
  - Used by: `render-3d` (selected via flag)

**Layer 3 — Integrated renderers:**

- **`render-3d`** (`crates/render-3d/src/lib.rs` — the hot-path file):
  - Purpose: 3D PBR raster pipeline (instanced cubes, hover/selection/outline/skybox) + PT integration that picks between megakernel and wavefront
  - Modules: `geometry` (cube mesh/instances), `pipelines` (`BindGroupLayouts`, `Pipelines`), `targets` (`RenderTargets`, `DynamicBindGroups`), `picking` (CPU/GPU object-id), `env_map` (HDR/EXR loader), `pt` (PT backend dispatcher with `wavefront.rs`)
  - WGSL: `shaders/cube_object_id.wgsl`, `shaders/outline.wgsl`, `shaders/skybox.wgsl`
  - Owns: `Renderer3D`, `PtState` (PT lifecycle), `MaterialCache`
  - Depends on: `dirstat-core`, `pt-core`, `pt-mats`, `pt-megakernel`, `pt-wavefront`, `bvh-gpu`, `render-core`, `render-shared`, `treemap`
  - Used by: binary `src/app/mod.rs::App::render_treemap`

- **`treemap`** (`crates/treemap/src/lib.rs`):
  - Purpose: squarified treemap layout (KDirStat/SequoiaView styles), CPU rayon cushion-shading rasterizer; optional `wgpu` feature exposes `GpuRenderer2D` (`crates/treemap/src/wgpu.rs`) for instanced GPU quads
  - Exports: `TreeMapOptions`, `LayoutStyle`, `DEFAULT_PALETTE`, `ext_color`, `hit_test`, layout entrypoints, `GpuRenderer2D` (feature-gated)
  - Depends on: `dirstat-core`, `rayon`, optionally `wgpu` + `render-core`
  - Used by: binary `src/app/treemap_view.rs`, `render-3d` (via `TreeMapOptions` + layout for cube placement)

**Layer 4 — Binary shell (`src/`):**

- Purpose: egui/eframe app, CLI parsing, scanner orchestration, scan caching, exclusions, settings persistence, panel layout
- Key modules:
  - `main.rs` — CLI parser (huge `CliOptions` struct), env_logger setup, eframe `NativeOptions` + `WgpuSetup` requesting `POLYGON_MODE_LINE`, `eframe::run_native`
  - `app/mod.rs` — `App::new` reconciles persisted state + CLI overrides, `start_scan`/`stop_scan`/`poll_scan`, `render_treemap` (the central per-frame dispatcher), event handling, screenshot capture
  - `app/state.rs` — `App`, `PersistState`, `ScannerMode`, `ScanProgress`
  - `app/dock.rs`, `app/tree_panel.rs`, `app/treemap_view.rs`, `app/ext_panel.rs`, `app/toolbar.rs`, `app/status_bar.rs`, `app/settings/*.rs`, `app/filters.rs`, `app/shell.rs`, `app/helpers.rs`, `app/presets.rs`
  - `scanner.rs` — `scan_bg(root, tx) -> Arc<AtomicBool>` jwalk-based parallel scan
  - `scanner_ntfs.rs` — Windows MFT direct-read fast path with NTFS fallback signal
  - `cache.rs` — bincode tree cache keyed by `path_key.rs::scan_path_id_hex` (sha256 of path)
  - `exclusions.rs` — `.dirstat-exclusions.json` per-root persistence
  - `events.rs` — typed event bus (`NavigateInto`, `NavigateUp`, `ZoomReset`, `SelectPath`, `LayoutDirty`, `RenderTick3D`, `SettingsChanged`)
  - `renderer.rs` — re-exports + binary-side `RenderMode`/`RenderBackend`/`CubeHeightMode`/`ColorMode`/`HashTransformEffect`/`HoverMode`/`SpectralMode` glue and CPU treemap entry
  - `cli_test.rs` — `dirstat-rs test ...` headless harness
  - `path_key.rs` — sha256-based stable cache keys

## Data Flow

### dirstat side: filesystem → tree → treemap render

```
[CLI/UI request scan]
    │
    ▼
src/app/mod.rs::App::start_scan
    │  ├─ exclusions::load(scan_path)
    │  ├─ cache::load_cache → DirEntry tree (instant) → rebuild_display_tree
    │  └─ spawn background scan thread
    │
    ├──► scanner::scan_bg (jwalk + rayon)               ──┐
    │                                                     │  ScanMsg::{Progress,Done,Error}
    │   OR                                                │  via crossbeam_channel
    └──► scanner_ntfs::scan_ntfs_bg (Windows MFT)        ──┤
            │                                             │
            └─ on err → ScanMsg::NtfsFallback ───────────►┤
                                                          │
                                                          ▼
            App::poll_scan (on each egui frame)
                ├─ Done(tree) → cache::write_cache_bytes (worker thread)
                │              compute_ext_stats, compute_size_range
                │              tree → App.tree
                │              rebuild_display_tree() (filters/exclusions/free_space)
                │              needs_layout = true
                ▼
            App::render_treemap
                ├─ Mode2D + Cpu → renderer::cpu::render (via treemap layout + cushion)
                ├─ Mode2D + Gpu → treemap::GpuRenderer2D::render (wgpu instanced quads, RGBA readback)
                └─ Mode3D       → Renderer3D::render (see render flow below)
                ▼
            egui::ColorImage → ctx.load_texture("treemap", …) → TextureHandle
                ▼
            egui Image widget displays it; treemap_view handles hit_test/zoom/pan
```

### render/PT side: scene → BVH → path-trace passes → image

```
Renderer3D (crates/render-3d/src/lib.rs)
    │
    ├─ build CubeInstance[] from DirEntry tree using treemap layout
    │   (placement, height_mode, hash_transform offsets, color_mode classification)
    │
    ├─ assign per-instance materials via pt_mats::classify_path_filtered
    │   (MaterialCache hashes Render3DOptions to skip reclassification)
    │
    ├─ raster path (PBR):
    │   instances → vertex/fragment pipeline → render targets → skybox/outline/hover composite
    │
    └─ PT path (when Render3DOptions.path_tracing):
            │
            ├─ scene upload → Vec<Instance>, Vec<GpuMaterial> in pt-core layout
            │
            ├─ BVH build:
            │   pt_gpu_bvh ON  → bvh_gpu::GpuBvhBuilder (Morton → radix sort → LBVH → AABB reduce)
            │   pt_gpu_bvh OFF → pt_core::build::build_instance_bvh (CPU SAH)
            │
            ├─ optional pt_bvh_refit between frames (no full rebuild)
            │
            ├─ backend dispatch (PtState.pt_backend_kind):
            │   Megakernel → pt_megakernel::PathTraceCompute::dispatch
            │       ├─ adaptive sampling (variance-based SPP per tile)
            │       ├─ ReSTIR DI/GI (reservoir.rs + temporal.wgsl + spatial.wgsl)
            │       ├─ path guiding (SVO update + sample WGSL)
            │       └─ accumulate into PT history texture
            │   Wavefront  → pt_wavefront::WavefrontPipeline (via render-3d/src/pt/wavefront.rs)
            │       ├─ raygen.wgsl     → ray queue
            │       ├─ count_swap.wgsl → compaction
            │       ├─ intersect.wgsl  → BVH traversal + WfHit queue
            │       ├─ shade.wgsl      → BSDF eval, next-event estimation
            │       └─ finalize.wgsl   → accumulate
            │
            ├─ camera-snap freeze (pt_camera_snap): freezes inv_view/inv_proj across SPP accumulation
            │
            └─ blit.wgsl → final RGBA texture
                ▼
            CPU readback (wgpu::Buffer::map_async + pollster) → Vec<u8>
                ▼
            (back to render_treemap → egui texture)
```

## Key Abstractions

**`DirEntry` (single source of truth for tree shape):**
- Location: `crates/dirstat-core/src/lib.rs`
- Contains `Cell<[f32;4]>` rect for interior-mutable layout writes (treemap sets rects without `&mut`, so the same shared tree feeds 2D treemap, 3D cube placement, and PT picking simultaneously)

**`Render3DOptions` (canonical render settings bag):**
- Location: `crates/render-shared/src/lib.rs`
- ~80 fields: PT params (bounces, spp, gpu_bvh, bvh_refit, russian_roulette, dof, restir_di/gi, path_guiding, svo_resolution), camera (orbit, inertia), materials (materialize mode + probabilities), env map (path/intensity/rotation/animate), hash effects, slice plane, LoD
- Persisted to disk via egui storage; mirrored by CLI overrides in `src/main.rs::CliOptions`

**`OrbitCamera`:**
- Location: `crates/render-shared/src/lib.rs`
- Houdini-style orbit (LMB) / pan (MMB) / zoom (RMB / scroll) with inertia, animation snapping, fit-all/fit-selection helpers

**`GpuContext`:**
- Location: `crates/render-core/src/lib.rs`, struct `GpuContext { device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue> }`
- `from_eframe(device, queue)` shares eframe's wgpu device when `POLYGON_MODE_LINE` is supported (zero-copy intent); otherwise `GpuContext::new()` creates an isolated device

**`PathTraceCompute` / `WavefrontPipeline`:**
- Twin abstractions for the two PT backends. `render-3d/src/pt/mod.rs` (`PtBackendKind` enum) selects which one is owned by `PtState` based on `Render3DOptions.pt_wavefront`

**`MaterialClass` + `MaterialLibrary`:**
- Location: `crates/pt-mats/src/lib.rs`
- Bridges PBR raster (per-instance albedo) and PT (per-instance `GpuMaterial` index into a packed buffer); deterministic classification by extension/path/size/age/random with separate light/glass probability gates

**Event bus (`src/events.rs`):**
- Type-erased queue with `downcast::<EventType>(&event)`; emits decoupled UI ↔ render signals: `NavigateIntoEvent`, `NavigateUpEvent`, `ZoomResetEvent`, `SelectPathEvent`, `LayoutDirtyEvent`, `RenderTick3DEvent`, `SettingsChangedEvent`

## Entry Points

**Process entry:**
- `src/main.rs::main()` — parses CLI, sets up `env_logger` with module-specific filters (dirstat_rs, pt_megakernel, pt_core, bvh_gpu, render_3d, pt_wavefront, pt_megakernel::pathguide), configures `eframe::egui_wgpu::WgpuSetupCreateNew` requesting `wgpu::Features::POLYGON_MODE_LINE`, calls `eframe::run_native`

**App lifecycle entry:**
- `src/app/mod.rs::App::new(cc, cli)` — restores `PersistState` from egui storage, applies CLI overrides, registers wgpu uncaptured-error hook, builds `GpuContext::from_eframe` if device features allow

**Per-frame entry:**
- `src/app/mod.rs::App::run_frame(ui, frame)` — called from `eframe::App::ui` impl; drives `poll_scan`, `handle_events`, keyboard shortcuts, animation tick, dock layout

**Render dispatch entry:**
- `src/app/mod.rs::App::render_treemap(ctx, (w,h))` (~L975) — picks 2D/CPU vs 2D/GPU vs 3D, owns lazy creation of `Renderer3D` and `GpuRenderer2D`

**Library roots (each crate's top-level public API):**
- `crates/dirstat-core/src/lib.rs` — `DirEntry`, LoD types
- `crates/pt-core/src/lib.rs` — re-exports `build_instance_bvh`, `BvhNode`, `GpuAabb`, `GpuMaterial`, `Instance`, `build_gpu_data_from_nodes`, `build_instance_gpu_data`
- `crates/render-core/src/lib.rs` — `Viewport`, `gpu::GpuContext`
- `crates/render-shared/src/lib.rs` — render-side enums, options, camera, uniforms, hash helpers
- `crates/render-3d/src/lib.rs` — `Renderer3D` and modules
- `crates/treemap/src/lib.rs` — layout, palette, CPU render; feature-gated `wgpu::GpuRenderer2D`
- `crates/bvh-gpu/src/lib.rs` — `GpuBvhBuilder`, `GpuBvhConfig`
- `crates/pt-megakernel/src/lib.rs` — `PathTraceCompute`, `PtCameraUniform`
- `crates/pt-wavefront/src/lib.rs` — `WavefrontPipeline`, `WavefrontConfig`, `WfDims`, `WfHit`, `WfRay`
- `crates/pt-mats/src/lib.rs` — `MaterialClass`, `MaterialLibrary`, `MaterializeMode`, `MaterializeSettings`, `classify_path_filtered`

## How dirstat side connects to render side

The bridge is **`DirEntry` plus the treemap layout pass**:

1. Scanner produces a `DirEntry` tree (shape, sizes, extensions, modified times).
2. `App::rebuild_display_tree` applies filters/exclusions/free-space wrapping to produce a display root.
3. For 2D: `treemap::layout` writes rects into the shared `DirEntry.rect` (interior mutability via `Cell`); CPU/GPU treemap renderers walk leaves and emit pixels.
4. For 3D: `Renderer3D::render` performs the same layout pass to get rect placement, then derives `CubeInstance` arrays — height from `CubeHeightMode` (file size, own size, file count, dir count, age, depth, depth^2, constant), color from `ColorMode`, position offsets from `HashTransformEffect`, materials from `pt-mats` classification of each leaf's path.
5. PT consumes the same `CubeInstance` set as `Vec<pt_core::Instance>` for BVH build.

Picking goes the other way: `render-3d/src/picking.rs` reads back GPU object-IDs (or CPU-walks the layout for 2D), maps to `DirEntry.path`, then `App` updates `selected_path` and `expanded` sets.

## Threading model

- **Filesystem scan** — runs on a dedicated `std::thread::Builder::new().name("scanner")` thread; jwalk uses an internal rayon thread pool to parallelize directory walks
- **NTFS MFT scan** — separate worker thread (`scanner_ntfs::scan_ntfs_bg`) that reads MFT records; on failure emits `ScanMsg::NtfsFallback` and continues with `scanner::scan_dir_public` on the same thread
- **Channel** — `crossbeam-channel::unbounded` `Sender<ScanMsg>` from worker → `Receiver` polled by `App::poll_scan` on the egui main thread (no blocking; `try_iter`)
- **Cancel** — `Arc<AtomicBool>` returned by `scan_bg` and stored in `App.scan_cancel`; checked inside the walk loop
- **Cache write** — serialized on the main thread (avoids tree clone), then a fire-and-forget `std::thread::spawn` writes the bytes to disk
- **CPU treemap render** — rayon `par_iter` inside `treemap::cpu` cushion shader (`crates/treemap/src/lib.rs`)
- **GPU work** — wgpu device/queue lives on the main thread; PT uses `pollster::block_on` for buffer mapping during readback (synchronous from the caller's perspective)
- **wgpu error hook** — `device.on_uncaptured_error(...)` stores into `App.wgpu_error_flag: Arc<AtomicBool>`; the next `run_frame` resets renderers and re-creates render targets

## GPU pipeline architecture (megakernel vs wavefront)

**Shared substrate:**
- Scene upload: `pt_core::Instance` array + `GpuMaterial` array, BVH nodes from CPU SAH or GPU LBVH
- BVH build path: CPU `pt_core::build_instance_bvh` (SAH) OR GPU `bvh_gpu::GpuBvhBuilder` (LBVH: `morton.wgsl` → `radix_sort.wgsl` → `lbvh_build.wgsl` → `aabb_compute.wgsl`)
- Both backends share `pt-mats`-produced materials and consume the same env map / camera uniforms (`PtCameraUniform`)

**Megakernel (`crates/pt-megakernel`):**
- One large compute shader walks rays through full bounce loop (raygen → intersect → BSDF shade → russian roulette → loop) inside a single workgroup invocation
- Side passes: `pathguide/{sample,update}.wgsl` (SVO sample/update), `restir/{temporal,spatial}.wgsl` (reservoir reuse), `wavefront/gbuffer.wgsl` (G-buffer prep), `pick.wgsl` (object-id lookup), `blit.wgsl` (final composite)
- Adaptive sampling (`adaptive/mod.rs`): variance-driven per-tile SPP scaling
- Best for: small bounce counts, high coherence; minimal launch overhead

**Wavefront (`crates/pt-wavefront`):**
- Stages execute as separate compute dispatches against ray queues:
  - `raygen.wgsl` — generate primary/continuation rays
  - `count_swap.wgsl` — compact active rays
  - `intersect.wgsl` — BVH traversal, fill `WfHit` queue
  - `shade.wgsl` — material eval, NEE, write next-bounce rays
  - `finalize.wgsl` — accumulate radiance
- Tile-based dispatch driven by `Render3DOptions.pt_wavefront_tile_size`
- Best for: deep paths, complex shading (less divergence per dispatch), enables per-stage profiling

**Selection point:** `Renderer3D::PtState.pt_backend_kind: pt::PtBackendKind` flips between `Megakernel` and `Wavefront` based on `Render3DOptions.pt_wavefront`; switching invalidates `path_tracer` so the next frame rebuilds pipelines

## Error Handling

**Strategy:** mixed — `anyhow::Result` for binary/scanner code, panic-on-pipeline-creation in renderers (validated WGSL), `Option`/log-and-continue for non-fatal renderer ops

**Patterns:**
- Scanner errors → `ScanMsg::Error(String)` → `App.progress.error` (shown in status bar)
- NTFS failure → `ScanMsg::NtfsFallback(reason)` → forces `scanner_mode = Standard` and re-runs jwalk inline
- wgpu uncaptured errors → `Arc<AtomicBool>` flag → next frame tears down `renderer_3d` / `renderer_2d_gpu` / cached textures
- Cache load/serialization failures → `log::warn!` and continue (cache is best-effort)
- Missing GPU features (e.g., `POLYGON_MODE_LINE`) → fallback to creating a separate wgpu device

## Cross-Cutting Concerns

**Logging:** `log` + `env_logger`, configured per-module in `src/main.rs::main()` — `dirstat_rs`, `pt_megakernel`, `pt_core`, `bvh_gpu`, `render_3d`, `pt_wavefront`, `pt_megakernel::pathguide`. CLI flags `--log-pt`, `--log-wf`, `--log-pg`, `--log-modules pt,wf,pg` toggle per-subsystem TRACE. Noisy crates (`naga`, `wgpu`, `eframe`, `egui`) are pinned to `Warn`. Optional `--log [FILE]` redirects formatted output to disk.

**Persistence:** egui storage → `dirstat_state` JSON via `serde_json`; on-disk scan cache via `bincode` (`src/cache.rs`); per-root exclusions via `serde_json` (`src/exclusions.rs`); preset autosave via `app::presets`.

**Validation/clamping:** `Render3DOptions` clamps applied at CLI parse and at settings-panel write sites; SVO resolution clamped 16..512 in `src/main.rs`.

**Authentication:** N/A (local desktop app).

**Allocator:** `auto_allocator` crate selects best allocator at startup (`src/main.rs::main()` logs the choice).

---

*Architecture analysis: 2026-05-09*
