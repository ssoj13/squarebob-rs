# Codebase Structure

**Original analysis:** 2026-05-09
**Updated:** 2026-05-09 (post-sprint-2)

> **Note:** the directory listings below have been updated to match
> the post-sprint-2 layout. Stage B refactors split two god-objects
> (`app/mod.rs` and `render-3d/src/lib.rs`); new submodules
> (`app/{scan_orchestration,render_loop,screenshot,cli_apply}.rs` and
> `render-3d/src/renderer3d/{material_cache,instance_collect}.rs`)
> are reflected here. CONCERNS.md is the source of truth for "what's
> done vs open"; this file is layout-only.

## Directory Layout

```
dirstat-rs/
├── Cargo.toml                # Workspace root + binary package "dirstat-rs"
├── Cargo.lock
├── rust-toolchain.toml       # Pins Rust 1.95+
├── README.md                 # User-facing overview, features, shortcuts, build
├── AGENTS.md                 # Agent-resume context: SSOT table, ASCII dataflow, open items
├── DIAGRAMS.md               # Mermaid diagrams (app layer, NTFS fallback, display pipeline)
├── TODO4.md                  # Active validated roadmap (supersedes TODO/TODO2/TODO3/plan1/task)
├── CHANGELOG.md              # Sprint summaries, behaviour-affecting changes
├── LICENSE                   # MIT
├── .github/workflows/ci.yml  # GitHub Actions CI (Linux + Windows + cargo-audit)
│
├── src/                      # Binary crate "dirstat-rs" (egui shell)
│   ├── main.rs               # CLI parser (CliOptions ~110 fields), env_logger, eframe::run_native
│   ├── renderer.rs           # Re-exports + binary-side render enums + CPU treemap entry
│   ├── scanner.rs            # jwalk parallel scan → ScanMsg over crossbeam-channel
│   ├── scanner_ntfs.rs       # Windows MFT direct-read scanner (cfg(windows))
│   ├── cache.rs              # bincode tree cache (load/serialize/write)
│   ├── exclusions.rs         # .dirstat-exclusions.json per-root persistence
│   ├── events.rs             # Type-erased event bus (NavigateInto/Up, ZoomReset, etc.)
│   ├── path_key.rs           # sha256-based stable cache keys (scan_path_id_hex)
│   ├── cli_test.rs           # `dirstat-rs test ...` headless harness
│   └── app/                  # egui application module
│       ├── mod.rs            # App::new, render_treemap, event handling, kb shortcuts (~716 LOC after Stage B.3)
│       ├── state.rs          # App, PersistState, ScannerMode, ScanProgress, SavedOpts
│       ├── scan_orchestration.rs  # start_scan, stop_scan, poll_scan, scan_engine_label_for_mode (Stage B.3)
│       ├── render_loop.rs    # run_frame, handle_events, sync_dock_tabs_visibility (Stage B.3)
│       ├── screenshot.rs     # handle_screenshot, capture_viewport, save_png (Stage B.3)
│       ├── cli_apply.rs      # apply_cli_overrides(&mut Render3DOptions, &CliOptions) + tests (Stage B.4)
│       ├── dock.rs           # egui_dock layout, DockTab, DockTabs
│       ├── toolbar.rs        # Top toolbar + path bar
│       ├── status_bar.rs     # Bottom status bar (FPS, samples/sec, scan progress)
│       ├── tree_panel.rs     # Left virtual file tree
│       ├── treemap_view.rs   # Central treemap/3D viewport + interactions
│       ├── ext_panel.rs      # Right extension stats panel
│       ├── filters.rs        # Tree filter/mask/glob/size-range/exclusion logic + LoD-merge tests
│       ├── helpers.rs        # compute_ext_stats, compute_size_range, find_node_by_path, fmt_size, disk_free_total
│       ├── shell.rs          # OS shell ops + shell_open() helper (logs open::that failures)
│       ├── presets.rs        # Settings preset save/load + autosave
│       └── settings/         # Settings panel (modular tabs)
│           ├── mod.rs
│           ├── appearance.rs
│           ├── exclusions.rs
│           ├── scanner.rs
│           ├── view.rs
│           └── renderer.rs
│
├── crates/                   # 10 workspace member library crates
│   ├── dirstat-core/         # Shared tree model (DirEntry, LoD types)
│   │   ├── Cargo.toml
│   │   ├── README.md
│   │   └── src/lib.rs
│   │
│   ├── pt-core/              # PT scene, CPU SAH BVH, GPU buffer layouts
│   │   ├── Cargo.toml
│   │   ├── README.md
│   │   └── src/
│   │       ├── lib.rs        # Module declarations + re-exports
│   │       ├── build.rs      # build_instance_bvh (CPU SAH)
│   │       ├── bvh.rs        # BvhNode, GpuAabb, GpuMaterial, Instance
│   │       └── gpu_data.rs   # GPU buffer construction
│   │
│   ├── render-core/          # GpuContext (device/queue), Viewport (pan/zoom)
│   │   ├── Cargo.toml
│   │   ├── README.md
│   │   └── src/lib.rs
│   │
│   ├── render-shared/        # Render enums + Render3DOptions + OrbitCamera + uniforms
│   │   ├── Cargo.toml
│   │   └── src/lib.rs        # RenderBackend, RenderMode, CubeHeightMode, ColorMode,
│   │                         # HoverMode, HashTransformEffect, SpectralMode,
│   │                         # CameraUniform, EnvParamsUniform, LightRigUniform,
│   │                         # HoverParamsUniform, hash_transform, name_hash
│   │
│   ├── bvh-gpu/              # GPU LBVH build (Morton + radix sort + Karras topology)
│   │   ├── Cargo.toml
│   │   ├── README.md
│   │   └── src/
│   │       ├── lib.rs
│   │       └── bvh_gpu/
│   │           ├── morton.wgsl
│   │           ├── radix_sort.wgsl
│   │           ├── lbvh_build.wgsl
│   │           └── aabb_compute.wgsl
│   │
│   ├── pt-mats/              # Material classifier + GpuMaterial library
│   │   ├── Cargo.toml
│   │   ├── README.md
│   │   └── src/lib.rs        # MaterialClass, MaterialLibrary, MaterializeMode,
│   │                         # MaterialSource, MaterialDistribution, classify_path_filtered
│   │
│   ├── pt-megakernel/        # Monolithic PT compute pipeline (HOT-PATH for PT)
│   │   ├── Cargo.toml
│   │   ├── README.md
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── compute.rs    # PathTraceCompute, PtCameraUniform
│   │       ├── blit.wgsl
│   │       ├── pick.wgsl
│   │       ├── adaptive/mod.rs       # Variance-driven per-tile SPP
│   │       ├── pathguide/
│   │       │   ├── config.rs
│   │       │   ├── svo.rs
│   │       │   ├── sample.wgsl
│   │       │   └── update.wgsl
│   │       ├── restir/
│   │       │   ├── config.rs
│   │       │   ├── reservoir.rs
│   │       │   ├── temporal.wgsl
│   │       │   └── spatial.wgsl
│   │       └── wavefront/
│   │           └── gbuffer.wgsl
│   │
│   ├── pt-wavefront/         # Staged wavefront PT (raygen→intersect→shade→finalize)
│   │   ├── Cargo.toml
│   │   ├── README.md
│   │   └── src/
│   │       ├── lib.rs        # Re-exports WavefrontConfig, WavefrontPipeline, WfDims, WfHit, WfRay
│   │       └── wavefront/
│   │           ├── raygen.wgsl
│   │           ├── count_swap.wgsl
│   │           ├── intersect.wgsl
│   │           ├── shade.wgsl
│   │           └── finalize.wgsl
│   │
│   ├── render-3d/            # Integrated 3D renderer (PBR raster + PT dispatcher) — HOT PATH
│   │   ├── Cargo.toml
│   │   ├── README.md
│   │   ├── src/
│   │   │   ├── lib.rs        # Renderer3D, PtState, MatGlobalUniform import, instance_rebuild_count() (~1937 LOC after Stage B.1)
│   │   │   ├── geometry.rs   # CubeInstance (with material_id slot 9), CUBE_INDICES
│   │   │   ├── pipelines.rs  # BindGroupLayouts, Pipelines
│   │   │   ├── targets.rs    # RenderTargets, DynamicBindGroups
│   │   │   ├── picking.rs    # GPU object-id readback + path mapping
│   │   │   ├── env_map.rs    # HDR/EXR environment map loader
│   │   │   ├── renderer3d/   # Stage B.1 extraction
│   │   │   │   ├── mod.rs    # pub(crate) submodule declarations
│   │   │   │   ├── material_cache.rs  # MaterialCache, MatGlobalUniform, mat_settings_hash, settings_from_opts
│   │   │   │   └── instance_collect.rs # impl Renderer3D::collect_cubes + collect_recursive
│   │   │   └── pt/
│   │   │       ├── mod.rs       # PtBackendKind enum
│   │   │       ├── megakernel.rs # Megakernel backend dispatch
│   │   │       ├── wavefront.rs  # Wavefront backend dispatch
│   │   │       └── spectral.rs   # Spectral mode helpers
│   │   └── shaders/
│   │       ├── cube_pbr.wgsl     # Per-instance materials via material_id, mat_global UBO (Stage A)
│   │       ├── cube_object_id.wgsl
│   │       ├── outline.wgsl
│   │       └── skybox.wgsl
│   │
│   └── treemap/              # Squarified layout + CPU/GPU treemap rasterizer
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs        # TreeMapOptions, LayoutStyle, palette, CPU cushion render
│           └── wgpu.rs       # GpuRenderer2D (feature = "wgpu")
│
├── data/                     # Bundled assets
│   ├── uffizi-large.hdr      # Default HDR environment map
│   ├── LICENSE               # Asset license
│   └── (screenshots referenced by README.md)
│
└── docs/                     # Reference papers
    ├── Fast_BVH_construction_on_gpus.pdf
    ├── jakob2021_optimizing_lbvh.pdf
    └── karras2012_maximizing_parallelism_bvh.pdf
```

## Directory Purposes

**`src/`:**
- Purpose: Binary crate — egui/eframe app shell, CLI, scanners, persistence, panels
- Contains: `main.rs` + flat module files for cross-cutting infra + nested `app/` module for UI
- Key files: `src/main.rs`, `src/app/mod.rs`, `src/scanner.rs`, `src/scanner_ntfs.rs`

**`src/app/`:**
- Purpose: All egui UI code — panels, dock layout, settings, treemap interactions
- Contains: One file per panel/concern (toolbar, status bar, tree, ext, treemap view, settings/*)
- Key files: `src/app/mod.rs` (lifecycle + render dispatch), `src/app/state.rs` (App struct + PersistState)

**`src/app/settings/`:**
- Purpose: Modular settings panel — one file per tab
- Contains: `appearance.rs`, `exclusions.rs`, `scanner.rs`, `view.rs`, `renderer.rs`

**`crates/`:**
- Purpose: All reusable library code, organized by concern
- Contains: 10 workspace members; each is a standalone library with its own README.md
- Naming convention: lowercase-with-dashes; domain prefix groups related crates (see below)

**`crates/{name}/src/`:**
- Each crate uses Rust standard layout — `lib.rs` is the public root, sibling modules for concerns
- WGSL shaders live alongside their owning module file (e.g. `pt-megakernel/src/restir/temporal.wgsl`)
- For larger shader sets (`render-3d`), shaders live in a sibling `shaders/` directory

**`data/`:**
- Purpose: Runtime-bundled assets (default HDR env map)
- Contains: `uffizi-large.hdr`, screenshots used in README
- Generated: No
- Committed: Yes

**`docs/`:**
- Purpose: Reference PDFs for GPU BVH algorithms backing `bvh-gpu`
- Contains: Karras 2012 (parallelism), Jakob 2021 (LBVH optimization), Fast BVH on GPUs
- Generated: No
- Committed: Yes

## Key File Locations

**Entry Points:**
- `src/main.rs` — process entry, CLI parser, eframe bootstrap
- `src/app/mod.rs` — `App::new`, `App::run_frame`, `App::render_treemap` (render dispatch)
- Library roots: `crates/{dirstat-core,pt-core,render-core,render-shared,render-3d,treemap,bvh-gpu,pt-megakernel,pt-wavefront,pt-mats}/src/lib.rs`

**Configuration:**
- `Cargo.toml` — workspace + binary deps
- `rust-toolchain.toml` — pins Rust 1.95+
- `crates/*/Cargo.toml` — per-crate manifests
- Runtime config persisted via egui storage (`dirstat_state` key); per-root `.dirstat-exclusions.json`

**Core Logic — dirstat side:**
- `src/scanner.rs` — jwalk parallel walk
- `src/scanner_ntfs.rs` — Windows MFT fast path
- `src/cache.rs` — bincode scan cache
- `crates/dirstat-core/src/lib.rs` — `DirEntry` model
- `crates/treemap/src/lib.rs` — squarified layout + cushion shading
- `crates/treemap/src/wgpu.rs` — GPU instanced-quads 2D treemap

**Core Logic — render/PT side (HOT PATHS):**
- `crates/render-3d/src/lib.rs` — `Renderer3D` orchestrator (PBR + PT integration); the largest single render file
- `crates/pt-megakernel/src/compute.rs` (with WGSL siblings) — megakernel PT pipeline
- `crates/pt-wavefront/src/wavefront/*.wgsl` + `lib.rs` — wavefront stages
- `crates/bvh-gpu/src/bvh_gpu/*.wgsl` — GPU LBVH build pipeline
- `crates/pt-core/src/{build,bvh,gpu_data}.rs` — shared PT data model + CPU SAH BVH

**Testing:**
- `src/cli_test.rs` — `dirstat-rs test ...` headless harness (no `tests/` directory at workspace root; check for `#[cfg(test)]` modules inside each crate)

**Documentation:**
- `README.md` — user-facing
- `AGENTS.md` — SSOT table, ASCII dataflow for scan/render, open engineering items
- `DIAGRAMS.md` — Mermaid diagrams of app layer, NTFS fallback, display pipeline
- `TODO.md`, `TODO2.md`, `task.md`, `plan1.md` — work tracking
- `crates/*/README.md` — purpose + dependents for each crate

## Naming Conventions

**Crate naming (groups by domain prefix):**
- `pt-*` — path-tracer family: `pt-core`, `pt-mats`, `pt-megakernel`, `pt-wavefront`
- `render-*` — renderer family: `render-core`, `render-shared`, `render-3d`
- `bvh-gpu` — standalone GPU BVH build (no prefix because it serves multiple PT crates)
- `treemap` — standalone 2D visualization
- `dirstat-core` — domain root (matches binary name prefix)

**Files:**
- Rust modules: `snake_case.rs` (e.g. `path_key.rs`, `scanner_ntfs.rs`, `tree_panel.rs`)
- WGSL shaders: `snake_case.wgsl`, colocated with owning Rust module
- Markdown: `UPPERCASE.md` for top-level project docs (`README.md`, `AGENTS.md`, `DIAGRAMS.md`, `TODO.md`, `LICENSE`); lowercase for working files (`plan1.md`, `task.md`)

**Directories:**
- `lowercase` or `kebab-case` (`render-3d`, `pt-megakernel`)
- Module subdirectories under a Rust crate match Rust module names: `app/settings/`, `pt-megakernel/src/restir/`

**Types:**
- `PascalCase` for structs/enums (`DirEntry`, `Render3DOptions`, `PathTraceCompute`)
- `SCREAMING_SNAKE_CASE` for consts (`DEFAULT_PALETTE`, `CUBE_INDICES`, `NUM_INDICES`, `DEFAULT_SCENE_LAYOUT_SIZE`)
- `snake_case` for functions and fields

## Where to Add New Code

**New scanner backend (e.g. APFS, ext4):**
- Implementation: new file `src/scanner_<name>.rs` mirroring `scanner.rs` interface (`scan_bg(root, tx) -> Arc<AtomicBool>`, `ScanMsg` enum)
- Wire-up: extend `ScannerMode` enum in `src/app/state.rs`, add branch in `App::start_scan` (`src/app/mod.rs`), add UI toggle in `src/app/settings/scanner.rs`

**New 3D render feature (e.g. new hover style, new height mode):**
- Add variant to enum in `crates/render-shared/src/lib.rs` (`HoverMode`, `CubeHeightMode`, `ColorMode`, `HashTransformEffect`)
- Implement in `crates/render-3d/src/lib.rs` (raster path) and pipeline files (`pipelines.rs`/`targets.rs`)
- Add CLI parser in `src/main.rs` (parse fn + `CliOptions` field + match arm)
- Add settings UI in `src/app/settings/renderer.rs`
- Persist via `Render3DOptions` (already serde)

**New PT feature (e.g. new sampling strategy):**
- If shared between backends: add to `crates/pt-core/src/` and re-export
- Megakernel: add WGSL pass + module under `crates/pt-megakernel/src/{feature}/` (mirror `restir/`, `pathguide/` pattern), wire into `compute.rs`
- Wavefront: add WGSL stage + Rust dispatcher under `crates/pt-wavefront/src/wavefront/`, wire into `WavefrontPipeline`
- Toggle in `Render3DOptions` (`crates/render-shared/src/lib.rs`) + CLI in `src/main.rs`

**New treemap layout style:**
- Add variant to `LayoutStyle` enum in `crates/treemap/src/lib.rs`
- Implement squarification logic in same file (alongside `KDirStat`/`SequoiaView` impls)
- Migration in `App::new` (`src/app/mod.rs` ~L107) deserialization match

**New material classification mode:**
- Add variant to `MaterialSource` / `MaterializeMode` in `crates/pt-mats/src/lib.rs`
- Implement in `classify_path_filtered`
- CLI parse in `src/main.rs::parse_materialize_mode`
- UI in `src/app/settings/renderer.rs`

**New UI panel:**
- New file `src/app/<panel_name>.rs` exposing `pub fn ui_<name>(&mut self, ui: &mut egui::Ui)`
- Register in `src/app/mod.rs` `mod` declarations
- Add `DockTab` variant in `src/app/dock.rs` and route in `DockTabs::ui`

**New event:**
- Define struct in `src/events.rs`
- Emit via `self.events.emit(MyEvent { ... })`
- Handle in `App::handle_events` (`src/app/mod.rs`) with `downcast::<MyEvent>(&event)`

**New shared GPU utility:**
- If render-only: `crates/render-core/src/lib.rs`
- If PT-only: `crates/pt-core/src/`
- If shared types/uniforms: `crates/render-shared/src/lib.rs`

**Utilities:**
- Tree-shape helpers (filtering, finding, computing stats): `src/app/helpers.rs` or `src/app/filters.rs`
- App-level OS shell ops: `src/app/shell.rs`
- Free-standing math/hash on render side: `crates/render-shared/src/lib.rs` (`hash_transform`, `name_hash` already there)

**New tests:**
- Per-crate unit tests: `#[cfg(test)] mod tests { ... }` at the bottom of the relevant `.rs` file
- Headless integration: extend `src/cli_test.rs` (invoked via `dirstat-rs test <name>`)

## Special Directories

**`crates/pt-megakernel/src/{adaptive,pathguide,restir}/`:**
- Purpose: Self-contained PT feature submodules; each has Rust glue + WGSL shader(s)
- Generated: No
- Committed: Yes

**`crates/bvh-gpu/src/bvh_gpu/`:**
- Purpose: Holds all WGSL shaders for the LBVH pipeline; `lib.rs` is one level up
- Generated: No
- Committed: Yes

**`crates/render-3d/shaders/`:**
- Purpose: WGSL for raster pipelines (object-id, outline, skybox); separate from `src/` because they're loaded by multiple Rust modules
- Generated: No
- Committed: Yes

**`data/`:**
- Purpose: Runtime asset bundle (HDR map default, README screenshots)
- Generated: No
- Committed: Yes

**`docs/`:**
- Purpose: Background literature (PDFs) for GPU BVH algorithms
- Generated: No
- Committed: Yes

**`.planning/codebase/`:**
- Purpose: Generated codebase-mapper output (this file lives here)
- Generated: Yes (by `/gsd-map-codebase`)
- Committed: Optional (depends on workflow)

---

*Structure analysis: 2026-05-09*
