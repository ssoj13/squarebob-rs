# Architecture

**Analysis Date:** 2026-04-20

## Pattern

**Desktop GUI app** with a **single main binary** (`src/main.rs`) hosting one **`app::App`** (`src/app/mod.rs`) implementing **`eframe::App`**.

Core loop:

1. **Input** — egui `Ui` at top level; `run_frame` coordinates panels, central dock, shortcuts (`src/app/mod.rs`).
2. **Scan pipeline** — Background worker threads send **`ScanMsg`** over **`crossbeam-channel`** into `App`; UI applies **`DirEntry`** tree and progress (`src/scanner.rs`, `src/app/mod.rs`).
3. **Display tree** — Raw scan tree filtered via **`filters`** into a cached display tree (`src/app/filters.rs`, `rebuild_display_tree` paths in `src/app/mod.rs`).
4. **Layout** — **`treemap`** crate computes per-frame or incremental layouts from `DirEntry` and viewport (`crates/treemap/`).
5. **Render** — Branches:
   - **2D CPU** — `renderer` CPU path (`src/renderer.rs` delegating to treemap CPU renderer).
   - **2D GPU** — `treemap` wgpu + textures registered with egui.
   - **3D** — **`render_3d::Renderer3D`** writes to an egui-managed texture; path tracing uses **`pt-*`** / **`bvh-gpu`** stack (`crates/render-3d/`, workspace PT crates).

## Layers

| Layer | Responsibility | Key modules |
|-------|----------------|-------------|
| **CLI / bootstrap** | Args, logging, wgpu/eframe init, viewport options | `src/main.rs` |
| **Application shell** | Tabs, toolbar, status, presets, persistence, events | `src/app/mod.rs`, `src/app/toolbar.rs`, `src/app/status_bar.rs`, `src/app/state.rs`, `src/app/presets.rs` |
| **Dock UI** | Panel layout (`egui_dock`) | `src/app/dock.rs` |
| **Views** | Treemap interaction, tree list, extensions, settings forms | `src/app/treemap_view.rs`, `src/app/tree_panel.rs`, `src/app/ext_panel.rs`, `src/app/settings/` |
| **Domain data** | Tree model, serde | `crates/dirstat-core` |
| **Scan / cache** | FS + optional NTFS, disk cache | `src/scanner.rs`, `src/scanner_ntfs.rs`, `src/cache.rs` |
| **Events** | Lightweight event bus for UI decoupling | `src/events.rs` |

## Data Flow (Simplified)

```
CLI → App::new → optional cache load → start_scan (thread) → ScanMsg::* → App state (tree, progress)
tree + filters → display_tree → treemap layout → renderer (2D/3D) → egui texture / paint
```

## Abstractions

- **`DirEntry`** — Single tree node type shared by layout and render (`crates/dirstat-core/src/lib.rs`).
- **`RenderMode` / `RenderBackend`** — Switches 2D/3D and CPU/GPU (`src/renderer.rs`, `src/main.rs`).
- **`path_key`** — Stable hex key from scan path for filenames (`src/path_key.rs`).
- **GPU stack** — Separated into **`render-core`**, **`render-shared`**, **`render-3d`**, with PT/BVH in dedicated crates for testability and compile time.

## Entry Points

- **`src/main.rs`** — `fn main`, `eframe::run_native`, Wgpu setup for custom rendering.
- **Crate libs** — Workspace crates expose library APIs consumed by the binary (e.g. `treemap`, `render-3d`).

---

*Architecture analysis: 2026-04-20*
